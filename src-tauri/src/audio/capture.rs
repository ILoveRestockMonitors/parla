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
                self.buf.truncate(span.offset + accepted);
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
        s.buf.clear();
        let maximum = s.max_samples;
        s.buf.try_reserve_exact(maximum).map_err(|_| {
            "Not enough memory for the configured recording limit; reduce max_recording_seconds."
        })?;
        s.spans.clear();
        s.carry_sum = 0.0;
        s.carry_count = 0;
        s.carry_at = None;
        s.endpoint = None;
        s.covered_endpoint = false;
        s.recording = true;
        s.started = Some(Instant::now());
        drop(s);
        if let Err(e) = self.stream.play() {
            if let Ok(mut s) = self.shared.lock() {
                s.recording = false;
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
        let (mut samples, started, error) = {
            let mut s = self.shared.lock().map_err(|_| "Audio lock unavailable.")?;
            s.recording = false;
            (
                std::mem::take(&mut s.buf),
                s.started.unwrap_or(end),
                s.error.clone(),
            )
        };
        if let Err(e) = self.stream.pause() {
            if let Ok(mut shared) = self.shared.lock() {
                shared.error = Some(e.to_string());
            }
        }
        // Short clips must not hold an entire 20-minute allocation while
        // waiting behind another job. Callback storage stays preallocated.
        samples.shrink_to_fit();
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
}
