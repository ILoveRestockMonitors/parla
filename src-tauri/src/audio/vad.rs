// VAD trim + level meter for the HUD. UNIMPLEMENTED(Phase 2).
//
// In PTT mode the key release is the endpoint (spec [FACT]) — VAD here only
// trims lead/trail silence before ASR and feeds the waveform, it never stops
// recording on its own. Candidate: vad-rs / Silero.
pub struct Vad;

impl Vad {
    /// Returns trimmed slice bounds (lead_silence, core, trail_silence).
    /// UNIMPLEMENTED(Phase 2).
    pub fn trim(&self, pcm: &[i16]) -> (usize, usize, usize) {
        (0, pcm.len(), 0)
    }

    /// RMS level 0..=1 for HUD metering.
    pub fn level(pcm: &[i16]) -> f32 {
        if pcm.is_empty() {
            return 0.0;
        }
        let sum: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum / pcm.len() as f64).sqrt() as f32 / i16::MAX as f32
    }
}
