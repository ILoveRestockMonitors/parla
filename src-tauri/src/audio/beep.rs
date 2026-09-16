//! Soft, overlapping sine chimes. Each voice fades fully to silence so neither
//! note boundaries nor the end of a cue introduce a click. No audio assets or
//! external services are needed; PlaySound buffers live for the process lifetime.
use std::f32::consts::{PI, TAU};
#[cfg(windows)]
use std::sync::OnceLock;

const RATE: u32 = 48_000;
#[cfg(windows)]
static CHIMES: OnceLock<[Vec<u8>; 3]> = OnceLock::new();

fn voice(samples: &mut [f32], hz: f32, offset_ms: usize, duration_ms: usize, gain: f32) {
    let offset = RATE as usize * offset_ms / 1000;
    let count = RATE as usize * duration_ms / 1000;
    for i in 0..count {
        let time = i as f32 / RATE as f32;
        let progress = i as f32 / (count - 1) as f32;
        // A 22 ms raised-cosine attack, warm decay, and a 70 ms soft release.
        let attack = (time / 0.022).min(1.0);
        let release = ((count - 1 - i) as f32 / (RATE as f32 * 0.070)).min(1.0);
        let envelope = (0.5 - 0.5 * (PI * attack).cos())
            * (0.5 - 0.5 * (PI * release).cos())
            * (-3.2 * progress).exp();
        let phase = TAU * hz * time;
        let tone = 0.96 * phase.sin() + 0.04 * (2.0 * phase).sin();
        samples[offset + i] += gain * envelope * tone;
    }
}

fn cue(notes: &[(f32, usize, usize, f32)]) -> Vec<i16> {
    let ms = notes.iter().map(|n| n.1 + n.2).max().unwrap_or(0);
    let mut samples = vec![0.0; RATE as usize * ms / 1000];
    for &(hz, offset, duration, gain) in notes {
        voice(&mut samples, hz, offset, duration, gain);
    }
    samples
        .iter()
        .map(|s| (s * i16::MAX as f32).round() as i16)
        .collect()
}

fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = pcm.len() * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

fn build_chimes() -> [Vec<u8>; 3] {
    [
        // An airy upward fifth opens recording; a lower third resolves it.
        wav_bytes(&cue(&[(440.0, 0, 220, 0.17), (659.25, 55, 230, 0.12)])),
        wav_bytes(&cue(&[(523.25, 0, 175, 0.14), (392.0, 45, 195, 0.11)])),
        // Distinct, but no harsh buzz on an error.
        wav_bytes(&cue(&[(329.63, 0, 190, 0.14), (261.63, 100, 210, 0.12)])),
    ]
}

pub fn write_previews(directory: &std::path::Path) -> std::io::Result<()> {
    for (name, bytes) in ["start.wav", "done.wav", "error.wav"]
        .iter()
        .zip(build_chimes())
    {
        std::fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(psnd: *const u8, hmod: isize, fdw: u32) -> i32;
}

#[derive(Clone, Copy)]
pub enum ChimeKind {
    Start,
    Done,
    Error,
}

/// Fire-and-forget: synthesis and playback never block the dictation pipeline.
pub fn play(kind: ChimeKind) {
    #[cfg(windows)]
    {
        std::thread::spawn(move || {
            let set = CHIMES.get_or_init(build_chimes);
            let idx = match kind {
                ChimeKind::Start => 0,
                ChimeKind::Done => 1,
                ChimeKind::Error => 2,
            };
            unsafe {
                // SND_ASYNC | SND_MEMORY | SND_NODEFAULT
                PlaySoundW(set[idx].as_ptr(), 0, 0x0001 | 0x0004 | 0x0002);
            }
        });
    }
    #[cfg(not(windows))]
    let _ = kind;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cues_are_quiet_click_free_and_fade_to_silence() {
        let chimes = build_chimes();
        assert_ne!(chimes[0], chimes[1]);
        for wav in chimes {
            let samples: Vec<i16> = wav[44..]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(samples.first(), Some(&0));
            assert_eq!(samples.last(), Some(&0));
            let peak = samples.iter().map(|&s| (s as i32).abs()).max().unwrap();
            assert!(
                peak > 2000 && peak < 8000,
                "cue must be audible without becoming piercing"
            );
            assert!(
                samples
                    .windows(2)
                    .all(|w| (w[1] as i32 - w[0] as i32).abs() < 600),
                "discontinuous samples click"
            );
            assert!(
                samples.iter().rev().take(240).all(|s| s.abs() < 20),
                "last 5 ms must settle to silence"
            );
            assert_eq!(
                u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                samples.len() * 2
            );
        }
    }
}
