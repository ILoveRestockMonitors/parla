// Modern PTT feedback: SYNTH chimes synthesized in memory, played via winmm
// PlaySound (SND_MEMORY|SND_ASYNC). Replaces both the kernel32 Beep and the
// earlier pure-sine version.
//
// Sound design: layered oscillator stack (saw-lite harmonic series + detuned
// second oscillator for width), quick pitch-drop pluck, fast attack / exp
// decay. Bright but polite. Buffers cached for process lifetime because
// SND_ASYNC requires the memory to stay valid.
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
const RATE: u32 = 22_050;

#[cfg(windows)]
static CHIMES: OnceLock<[Vec<u8>; 3]> = OnceLock::new();

/// One synth voice: harmonic series (saw-lite) + detuned twin + pluck glide.
#[cfg(windows)]
fn segment(samples: &mut Vec<i16>, f0: f32, f1: f32, ms: u32, vol: f32) {
    let n = (RATE as usize * ms as usize) / 1000;
    let attack = (RATE as usize * 3) / 1000; // 3 ms fast attack
    let mut ph1 = 0.0f32;
    let mut ph2 = 0.25f32; // offset so the detune twin doesn't null-cancel at t=0
    for i in 0..n {
        let t = i as f32 / n.max(1) as f32;
        // pluck: pitch starts ~2% sharp and settles within the first fifth
        let bend = 1.0 + 0.02 * (-t * 5.0).exp();
        let f = (f0 + (f1 - f0) * t) * bend;

        ph1 += core::f32::consts::TAU * f / RATE as f32;
        ph2 += core::f32::consts::TAU * (f * 1.004) / RATE as f32; // +7 cents twin

        // saw-lite stack: 1 + 1/2 + 1/3 + 1/4 (softened highs)
        let osc1 = ph1.sin()
            + 0.50 * (2.0 * ph1).sin()
            + 0.33 * (3.0 * ph1).sin()
            + 0.20 * (4.0 * ph1).sin();
        // twin carries mostly fundamental -> width without mud
        let osc2 = ph2.sin();
        let mut env = if i < attack {
            i as f32 / attack as f32
        } else {
            (-(t * 4.0)).exp().max(0.12) // exponential body decay, audible tail
        };

        // hard-limit the stacked waveform before scaling
        let raw = (osc1 * 0.22 + osc2 * 0.16).clamp(-0.9, 0.9);
        samples.push((raw * env * vol * 30_000.0) as i16);
        let _ = &mut env;
    }
}

#[cfg(windows)]
fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = pcm.len() * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
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

#[cfg(windows)]
fn build_chimes() -> [Vec<u8>; 3] {
    // START: rising fifth D5->A5, bright pluck then settle
    let mut start = Vec::new();
    segment(&mut start, 587.0, 880.0, 70, 0.9);
    segment(&mut start, 880.0, 880.0, 90, 0.8);
    // END: falling A5->D5 with a lower landing note (resolved feel)
    let mut end = Vec::new();
    segment(&mut end, 880.0, 587.0, 60, 0.85);
    segment(&mut end, 587.0, 494.0, 100, 0.75);
    // ERROR: low G3->E3 downglide, darker (quieter high partials dominate less)
    let mut err = Vec::new();
    segment(&mut err, 208.0, 165.0, 180, 0.85);
    [wav_bytes(&start), wav_bytes(&end), wav_bytes(&err)]
}

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(psnd: *const u8, hmod: isize, fdw: u32) -> i32;
}

#[cfg(windows)]
const SND_ASYNC: u32 = 0x0001;
#[cfg(windows)]
const SND_NODEFAULT: u32 = 0x0002;
#[cfg(windows)]
const SND_MEMORY: u32 = 0x0004;

#[derive(Clone, Copy)]
pub enum ChimeKind {
    Start,
    Done,
    Error,
}

/// Fire-and-forget: plays on its own thread, never blocks the pipeline.
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
                PlaySoundW(set[idx].as_ptr(), 0, SND_ASYNC | SND_MEMORY | SND_NODEFAULT);
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
    }
}
