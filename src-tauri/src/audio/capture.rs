use super::resample::resample;
use crate::store::settings::Settings;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub started: Instant,
    pub ended: Instant,
}
impl AudioClip {
    pub fn to_pcm16_16k(&self) -> Vec<i16> {
        resample(&self.samples, self.sample_rate, 16000)
    }
}
#[derive(Clone, Copy)]
struct Span {
    offset: usize,
    frames: usize,
    at: Instant,
}
// Keep stop-time copying bounded to 1 MiB (about 5.5 seconds at 48 kHz).
// Longer recordings transfer ownership as before: copying them solely to
// retain a reservation costs more than the next reservation saves.
const MAX_REUSE_COPY_SAMPLES: usize = 1024 * 1024 / std::mem::size_of::<f32>();
struct Shared {
    recording: bool,
    buf: Vec<f32>,
    channels: usize,
    rate: u32,
    started: Option<Instant>,
    error: Option<String>,
    max_samples: usize,
    endpoint: Option<Instant>,
    covered_endpoint: bool,
    carry_sum: f32,
    carry_count: usize,
    carry_at: Option<Instant>,
    spans: Vec<Span>,
}
impl Shared {
    fn clear_audio(&mut self) {
        // Keep the allocation, not the recording. In particular, a cancelled
        // or completed utterance must not linger in reusable idle storage.
        self.buf.fill(0.0);
        self.buf.clear();
        self.spans.clear();
        self.carry_sum = 0.0;
        self.carry_count = 0;
        self.carry_at = None;
    }
    fn prepare_buffer(&mut self) -> Result<(), String> {
        self.recording = false;
        self.clear_audio();
        // Short recordings retain this allocation for the next start. Taking
        // and shrinking at every stop used to discard the entire reservation.
        self.buf.try_reserve_exact(self.max_samples).map_err(|_| {
            "Not enough memory for the configured recording limit; reduce max_recording_seconds."
                .to_string()
        })
    }
    fn finish_samples(&mut self) -> Result<Vec<f32>, String> {
        self.recording = false;
        if self.buf.len() > MAX_REUSE_COPY_SAMPLES {
            let mut samples = std::mem::take(&mut self.buf);
            self.clear_audio();
            samples.shrink_to_fit();
            return Ok(samples);
        }
        let mut samples = Vec::new();
        if samples.try_reserve_exact(self.buf.len()).is_err() {
            self.clear_audio();
            return Err("Not enough memory to finish this recording.".into());
        }
        // Pending clips own only their actual frames, while the callback
        // buffer remains preallocated for the next recording. No allocation
        // or additional synchronization is introduced in the audio callback.
        samples.extend_from_slice(&self.buf);
        self.clear_audio();
        Ok(samples)
    }
    fn ingest(&mut self, input: impl Iterator<Item = f32>, packet_at: Instant) {
        let offset = self.buf.len();
        let mut first_at = None;
        let mut frame_index = 0u64;
        for sample in input {
            if !self.recording {
                break;
            }
            if self.carry_count == 0 {
                self.carry_at = Some(
                    packet_at + Duration::from_secs_f64(frame_index as f64 / self.rate as f64),
                );
            }
            self.carry_sum += if sample.is_finite() { sample } else { 0.0 };
            self.carry_count += 1;
            if self.carry_count != self.channels {
                continue;
            }
            let at = self.carry_at.take().unwrap_or(packet_at);
            let value = self.carry_sum / self.channels as f32;
            self.carry_sum = 0.0;
            self.carry_count = 0;
            frame_index += 1;
            if self.endpoint.is_some_and(|end| at > end) {
                self.covered_endpoint = true;
                break;
            }
            if self.started.is_none_or(|start| at >= start) {
                if self.buf.len() >= self.max_samples {
                    self.recording = false;
                    break;
                }
                first_at.get_or_insert(at);
                self.buf.push(value);
            }
        }
        if let Some(at) = first_at {
            // Bound metadata even for a broken driver sending tiny packets.
            if self.spans.len() >= 300_000 {
                self.error = Some("Audio packet limit reached.".into());
                self.recording = false;
            } else {
                self.spans.push(Span {
                    offset,
                    frames: self.buf.len() - offset,
                    at,
                });
            }
        }
        if self.buf.len() >= self.max_samples {
            self.recording = false;
        }
    }
    fn end_at(&mut self, end: Instant) {
        self.endpoint = Some(end);
        for span in &self.spans {
            let accepted = if end < span.at {
                0
            } else {
                ((end.duration_since(span.at).as_secs_f64() * self.rate as f64).floor() as usize
                    + 1)
                .min(span.frames)
            };
            if accepted < span.frames {
                let retained = span.offset + accepted;
                // A stop event may arrive after packets past its timestamp.
                // Erase that rejected tail before retaining the allocation.
                self.buf[retained..].fill(0.0);
                self.buf.truncate(retained);
                self.covered_endpoint = true;
                break;
            }
        }
    }
}
pub struct MicCapture {
    shared: Arc<Mutex<Shared>>,
    stream: cpal::Stream,
    device_name: String,
    dev_rate: u32,
    channels: usize,
}
impl MicCapture {
    pub fn open() -> Result<Self, String> {
        Self::open_with_settings(&Settings::default())
    }
    pub fn open_with_settings(settings: &Settings) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = settings
            .microphone_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().is_ok_and(|n| n.eq_ignore_ascii_case(name)))
                .ok_or_else(|| format!("Configured microphone {name:?} is unavailable."))?
        } else {
            host.default_input_device()
                .ok_or("No microphone input device found.")?
        };
        let name = device.name().unwrap_or_else(|_| "unknown".into());
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        if !(8000..=192000).contains(&rate) || !(1..=32).contains(&channels) {
            return Err("Unsupported microphone rate/channel count.".into());
        }
        let max_samples = rate as usize * settings.max_recording_seconds.clamp(1, 1200) as usize;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let shared = Arc::new(Mutex::new(Shared {
            recording: false,
            buf: Vec::new(),
            channels,
            rate,
            started: None,
            error: None,
            max_samples,
            endpoint: None,
            covered_endpoint: false,
            carry_sum: 0.0,
            carry_count: 0,
            carry_at: None,
            spans: Vec::with_capacity(300_000),
        }));
        let capture = shared.clone();
        let errors = shared.clone();
        let on_error = move |e: cpal::StreamError| {
            if let Ok(mut s) = errors.lock() {
                s.error = Some(e.to_string());
                s.recording = false;
            }
        };
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |samples: &[f32], info| {
                    let at = packet_time(info);
                    if let Ok(mut s) = capture.lock() {
                        if s.recording {
                            s.ingest(samples.iter().copied(), at);
                        }
                    }
                },
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |samples: &[i16], info| {
                    let at = packet_time(info);
                    if let Ok(mut s) = capture.lock() {
                        if s.recording {
                            s.ingest(samples.iter().map(|v| *v as f32 / 32768.0), at);
                        }
                    }
                },
                on_error,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |samples: &[u16], info| {
                    let at = packet_time(info);
                    if let Ok(mut s) = capture.lock() {
                        if s.recording {
                            s.ingest(samples.iter().map(|v| (*v as f32 - 32768.0) / 32768.0), at);
                        }
                    }
                },
                on_error,
                None,
            ),
            _ => return Err("Unsupported microphone sample format.".into()),
        }
        .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            stream,
            device_name: name,
            dev_rate: rate,
            channels,
        })
    }
    pub fn device_info(&self) -> String {
        format!(
            "{} @ {} Hz, {} channel(s) → 16000 Hz mono",
            self.device_name, self.dev_rate, self.channels
        )
    }
    pub fn start(&self) -> Result<(), String> {
        let mut s = self.shared.lock().map_err(|_| "Audio lock unavailable.")?;
        if let Some(error) = &s.error {
            return Err(format!("Microphone needs reopening: {error}"));
        }
        s.prepare_buffer()?;
        s.endpoint = None;
        s.covered_endpoint = false;
        s.recording = true;
        s.started = Some(Instant::now());
        drop(s);
        if let Err(e) = self.stream.play() {
            if let Ok(mut s) = self.shared.lock() {
                s.recording = false;
                s.clear_audio();
                s.error = Some(e.to_string());
            }
            return Err(e.to_string());
        }
        Ok(())
    }
    pub fn stop(&self) -> Result<Vec<i16>, String> {
        self.stop_at(Instant::now(), Duration::ZERO)
            .map(|c| c.to_pcm16_16k())
    }
    pub fn stop_at(&self, end: Instant, drain: Duration) -> Result<AudioClip, String> {
        let deadline = Instant::now() + drain.min(Duration::from_millis(500));
        self.shared
            .lock()
            .map_err(|_| "Audio lock unavailable.")?
            .end_at(end);
        loop {
            let done = {
                let s = self.shared.lock().map_err(|_| "Audio lock unavailable.")?;
                !s.recording || s.covered_endpoint || s.error.is_some()
            };
            if done || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let (samples, started, error) = {
            let mut s = self.shared.lock().map_err(|_| "Audio lock unavailable.")?;
            (
                s.finish_samples(),
                s.started.unwrap_or(end),
                s.error.clone(),
            )
        };
        if let Err(e) = self.stream.pause() {
            if let Ok(mut shared) = self.shared.lock() {
                shared.error = Some(e.to_string());
            }
        }
        let samples = samples?;
        if let Some(e) = error {
            if samples.is_empty() {
                return Err(e);
            }
        }
        Ok(AudioClip {
            samples,
            sample_rate: self.dev_rate,
            started,
            ended: end,
        })
    }
    pub fn last_error(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.error.clone())
    }
    pub fn is_recording(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.recording)
    }
}
fn packet_time(info: &cpal::InputCallbackInfo) -> Instant {
    let now = Instant::now();
    let stamp = info.timestamp();
    now.checked_sub(
        stamp
            .callback
            .duration_since(&stamp.capture)
            .unwrap_or_default(),
    )
    .unwrap_or(now)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn shared(start: Instant, channels: usize) -> Shared {
        Shared {
            recording: true,
            buf: vec![],
            channels,
            rate: 1000,
            started: Some(start),
            error: None,
            max_samples: 10,
            endpoint: None,
            covered_endpoint: false,
            carry_sum: 0.0,
            carry_count: 0,
            carry_at: None,
            spans: vec![],
        }
    }
    #[test]
    fn crossing_start_and_end_keep_exact_frames() {
        let t = Instant::now();
        let mut s = shared(t + Duration::from_millis(2), 1);
        s.ingest([1., 2., 3., 4., 5.].into_iter(), t);
        assert_eq!(s.buf, vec![3., 4., 5.]);
        s.end_at(t + Duration::from_millis(3));
        assert_eq!(s.buf, vec![3., 4.]);
        assert!(s.covered_endpoint);
    }
    #[test]
    fn pre_end_packet_arriving_after_stop_is_drained() {
        let t = Instant::now();
        let mut s = shared(t, 1);
        s.end_at(t + Duration::from_millis(2));
        s.ingest([1., 2., 3., 4.].into_iter(), t);
        assert_eq!(s.buf, vec![1., 2., 3.]);
        assert!(s.covered_endpoint);
    }
    #[test]
    fn stereo_split_frame_and_cap_count_mono_frames() {
        let t = Instant::now();
        let mut s = shared(t, 2);
        s.max_samples = 2;
        s.ingest([1.].into_iter(), t);
        s.ingest([-1., 0.5, 0.5, 1., 1.].into_iter(), t);
        assert_eq!(s.buf, vec![0., 0.5]);
        assert!(!s.recording);
        assert!(s.carry_count < 2);
    }
    #[test]
    fn repeated_recordings_reuse_storage_and_return_independent_tight_clips() {
        let t = Instant::now();
        let mut s = shared(t, 1);
        s.max_samples = 1000;
        s.prepare_buffer().unwrap();
        let pointer = s.buf.as_ptr();
        let capacity = s.buf.capacity();
        let mut previous = None;
        for round in 0..20 {
            s.prepare_buffer().unwrap();
            s.started = Some(t);
            s.endpoint = None;
            s.covered_endpoint = false;
            s.recording = true;
            let expected = vec![round as f32, 0.25, -0.5];
            s.ingest(expected.iter().copied(), t);
            let clip = s.finish_samples().unwrap();
            assert_eq!(clip, expected);
            assert!(clip.capacity() < capacity);
            assert_eq!(s.buf.as_ptr(), pointer);
            assert_eq!(s.buf.capacity(), capacity);
            assert!(s.buf.is_empty());
            assert!(!s.recording);
            if let Some(old_clip) = previous {
                assert_eq!(old_clip, vec![(round - 1) as f32, 0.25, -0.5]);
            }
            previous = Some(clip);
        }
    }
    #[test]
    fn completion_erases_used_storage_and_partial_channel_carry() {
        let t = Instant::now();
        let mut s = shared(t, 2);
        s.prepare_buffer().unwrap();
        s.recording = true;
        s.ingest([0.5, 0.5, -0.25, -0.25, 0.75].into_iter(), t);
        let pointer = s.buf.as_ptr();
        let initialized = s.buf.len();
        assert_eq!(s.carry_count, 1);
        assert_eq!(s.finish_samples().unwrap(), vec![0.5, -0.25]);
        assert_eq!(s.buf.as_ptr(), pointer);
        // SAFETY: finish_samples retains this allocation. These f32 slots
        // were initialized before clearing; inspecting them verifies erasure,
        // not uninitialized spare capacity. No mutation occurs during the read.
        let cleared = unsafe { std::slice::from_raw_parts(pointer, initialized) };
        assert!(cleared.iter().all(|sample| *sample == 0.0));
        assert_eq!(s.carry_sum, 0.0);
        assert_eq!(s.carry_count, 0);
        assert!(s.carry_at.is_none());
        assert!(s.spans.is_empty());
    }
    #[test]
    fn endpoint_erases_rejected_tail_before_buffer_reuse() {
        let t = Instant::now();
        let mut s = shared(t, 1);
        s.prepare_buffer().unwrap();
        s.recording = true;
        s.ingest([1., 2., 3., 4., 5.].into_iter(), t);
        let pointer = s.buf.as_ptr();
        let initialized = s.buf.len();
        s.end_at(t + Duration::from_millis(1));
        assert_eq!(s.buf, vec![1., 2.]);
        // SAFETY: end_at only overwrites/truncates, retaining the allocation
        // and initialization of all five original slots.
        let storage = unsafe { std::slice::from_raw_parts(pointer, initialized) };
        assert_eq!(storage, &[1., 2., 0., 0., 0.]);
        assert_eq!(s.finish_samples().unwrap(), vec![1., 2.]);
    }
    #[test]
    fn next_recording_excludes_idle_pre_start_and_prior_partial_frames() {
        let t = Instant::now();
        let mut s = shared(t, 2);
        s.prepare_buffer().unwrap();
        s.recording = true;
        s.ingest([0.75].into_iter(), t);
        assert!(s.finish_samples().unwrap().is_empty());
        s.ingest([1., 1.].into_iter(), t + Duration::from_secs(1));
        assert!(s.buf.is_empty());

        s.prepare_buffer().unwrap();
        s.started = Some(t + Duration::from_millis(2));
        s.endpoint = None;
        s.covered_endpoint = false;
        s.recording = true;
        s.ingest([1., 1., 2., 2., 3., 3., 4., 4.].into_iter(), t);
        s.end_at(t + Duration::from_millis(2));
        assert_eq!(s.finish_samples().unwrap(), vec![3.]);
    }
    #[test]
    fn reused_buffer_preserves_late_endpoint_drain_and_maximum_length() {
        let t = Instant::now();
        let mut s = shared(t, 1);
        s.max_samples = 3;
        for end_ms in [1, 5] {
            s.prepare_buffer().unwrap();
            s.started = Some(t);
            s.endpoint = None;
            s.covered_endpoint = false;
            s.recording = true;
            s.end_at(t + Duration::from_millis(end_ms));
            s.ingest([1., 2., 3., 4., 5.].into_iter(), t);
            assert!(s.buf.len() <= s.max_samples);
            let clip = s.finish_samples().unwrap();
            assert_eq!(
                clip,
                if end_ms == 1 {
                    vec![1., 2.]
                } else {
                    vec![1., 2., 3.]
                }
            );
        }
    }
    #[test]
    fn long_clips_transfer_storage_instead_of_adding_a_full_copy() {
        let t = Instant::now();
        let mut s = shared(t, 1);
        s.max_samples = MAX_REUSE_COPY_SAMPLES + 10;
        s.prepare_buffer().unwrap();
        s.buf.resize(MAX_REUSE_COPY_SAMPLES + 1, 0.25);
        s.carry_sum = 0.5;
        s.carry_count = 1;
        s.carry_at = Some(t);
        let clip = s.finish_samples().unwrap();
        assert_eq!(clip.len(), MAX_REUSE_COPY_SAMPLES + 1);
        assert!(clip.iter().all(|sample| *sample == 0.25));
        assert!(s.buf.is_empty());
        assert_eq!(s.buf.capacity(), 0);
        assert_eq!(s.carry_sum, 0.0);
        assert_eq!(s.carry_count, 0);
        assert!(s.carry_at.is_none());
    }
}
