use crate::audio::capture::AudioClip;
use crate::dictionary::Entry;
use crate::store::settings::Settings;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Stop,
    Cancel,
    Retry,
    Purge,
    Restore,
    Learn { alias: String, canonical: String },
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LastResult {
    pub id: u64,
    pub raw: Option<String>,
    pub normalized: Option<String>,
    pub final_text: Option<String>,
    pub error: Option<String>,
    pub stage_ms: serde_json::Value,
    pub insertion: String,
    pub backend: String,
}
#[derive(Clone)]
pub struct ReplayClip {
    pub clip: Arc<AudioClip>,
    pub settings: Settings,
    pub entries: Vec<Entry>,
    pub expires: Instant,
}
pub struct RuntimeState {
    pub recording: bool,
    pub mode: String,
    pub recording_started: Option<Instant>,
    pub queued: usize,
    pub processing: bool,
    pub settings: Settings,
    pub last: LastResult,
    pub audio: Option<ReplayClip>,
    pub error: Option<String>,
    pub device: String,
    pub hook_ready: bool,
    commands: VecDeque<Command>,
}
impl RuntimeState {
    fn expire_at(&mut self, now: Instant) {
        if !self.settings.retain_audio_for_retry
            || self.audio.as_ref().is_some_and(|a| now >= a.expires)
        {
            self.audio = None;
        }
    }
    fn new(settings: Settings) -> Self {
        Self {
            recording: false,
            mode: "starting".into(),
            recording_started: None,
            queued: 0,
            processing: false,
            settings,
            last: LastResult::default(),
            audio: None,
            error: None,
            device: String::new(),
            hook_ready: false,
            commands: VecDeque::new(),
        }
    }
}
static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
fn state() -> &'static Mutex<RuntimeState> {
    STATE.get_or_init(|| Mutex::new(RuntimeState::new(Settings::load())))
}
pub fn init(settings: Settings) {
    let _ = STATE.set(Mutex::new(RuntimeState::new(settings)));
}
pub fn update(f: impl FnOnce(&mut RuntimeState)) {
    if let Ok(mut s) = state().lock() {
        f(&mut s);
    }
}
pub fn enqueue(command: Command) -> bool {
    let Ok(mut s) = state().lock() else {
        return false;
    };
    if s.commands.len() >= 8 {
        return false;
    }
    s.commands.push_back(command);
    true
}
pub fn take_command() -> Option<Command> {
    state().lock().ok()?.commands.pop_front()
}
pub fn purge() {
    update(|s| {
        s.audio = None;
        s.last = LastResult::default();
    });
}
pub fn expire_audio() {
    update(|s| s.expire_at(Instant::now()));
}
pub fn replay() -> Option<ReplayClip> {
    expire_audio();
    state().lock().ok()?.audio.clone()
}
pub fn remember_audio(clip: Arc<AudioClip>, settings: Settings, entries: Vec<Entry>) {
    update(|s| {
        s.audio = if settings.retain_audio_for_retry && s.settings.retain_audio_for_retry {
            Some(ReplayClip {
                clip,
                expires: Instant::now() + Duration::from_secs(settings.retry_audio_seconds as u64),
                settings,
                entries,
            })
        } else {
            None
        };
    });
}
pub fn snapshot() -> serde_json::Value {
    expire_audio();
    let Ok(s) = state().lock() else {
        return serde_json::json!({"mode":"unavailable"});
    };
    serde_json::json!({
        "recording": s.recording, "mode": s.mode,
        "recording_elapsed_ms": s.recording_started.map(|t| t.elapsed().as_millis()).unwrap_or(0),
        "queued": s.queued, "processing": s.processing, "effective_settings": s.settings,
        "last_result": s.last, "audio_retry_available": s.audio.is_some(), "error": s.error,
        "device": s.device, "hook_ready": s.hook_ready,
        "input_monitor_ready": crate::hotkey::windows::input_epoch().is_some(),
        "build_id": concat!(env!("CARGO_PKG_VERSION"), "-terminal-insertion-20260916")
    })
}

/// HUD polling does not serialize or copy transcript contents.
pub fn hud_snapshot() -> serde_json::Value {
    let Ok(s) = state().lock() else {
        return serde_json::json!({});
    };
    serde_json::json!({"recording":s.recording,"processing":s.processing || s.queued>0,"mode":s.mode,
        "recording_elapsed_ms":s.recording_started.map(|t|t.elapsed().as_millis()).unwrap_or(0),
        "error":s.error,"effective_settings":{"hud_enabled":s.settings.hud_enabled,"toggle_chord":s.settings.toggle_chord}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_retains_full_clip_until_expiry_or_opt_out() {
        let now = Instant::now();
        let mut settings = Settings::default();
        settings.retain_audio_for_retry = true;
        settings.retry_audio_seconds = 1;
        let mut state = RuntimeState::new(settings.clone());
        let clip = Arc::new(AudioClip {
            samples: vec![0.25; 32000],
            sample_rate: 16000,
            started: now - Duration::from_secs(2),
            ended: now,
        });
        let replay = ReplayClip {
            clip,
            settings,
            entries: vec![],
            expires: now + Duration::from_secs(1),
        };
        state.audio = Some(replay.clone());
        state.expire_at(now);
        assert_eq!(state.audio.as_ref().unwrap().clip.samples.len(), 32000);
        state.expire_at(now + Duration::from_secs(1));
        assert!(state.audio.is_none());
        state.audio = Some(replay);
        state.settings.retain_audio_for_retry = false;
        state.expire_at(now);
        assert!(state.audio.is_none());
    }
}
