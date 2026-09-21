// serde settings with corrupt-config -> defaults fallback (spec store contract).
// Persistence: LOCALAPPDATA\Parla\settings.json (Phase 5).
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Push-to-talk chords. Each entry is "+"-joined modifier tokens from
    /// {ctrl, win, alt, shift}; e.g. ["ctrl+win"] or ["ctrl+win","ctrl+shift"].
    /// Multiple entries = any one of them triggers. Serde default keeps old
    /// settings.json (missing field) working. Invalid entries fall back to
    /// the default chord wholesale (spec failure mode).
    #[serde(default = "default_ptt_chords")]
    pub ptt_chords: Vec<String>,
    #[serde(default = "default_toggle_chord")]
    pub toggle_chord: String,
    #[serde(default)]
    pub asr_model_path: String,
    #[serde(default)]
    pub asr_server_exe: String,
    #[serde(default = "default_formatter_port")]
    pub formatter_port: u16,
    #[serde(default)]
    pub formatter_model: String,
    #[serde(default = "default_formatter_num_ctx")]
    pub formatter_num_ctx: u32,
    #[serde(default)]
    pub history_mode: String,
    #[serde(default = "default_chimes_enabled")]
    pub chimes_enabled: bool,
    /// ASR backend: "whisper" (ggml whisper-server :9292) | "parakeet"
    /// (sherpa-onnx TDT shim :9293). Unknown values fall back to whisper.
    #[serde(default)]
    pub asr_backend: String,
    /// Directory holding the Parakeet ONNX files (encoder.onnx, decoder.onnx,
    /// joiner.onnx, tokens.txt). Only used when asr_backend == "parakeet".
    #[serde(default = "default_parakeet_dir")]
    pub parakeet_model_dir: String,
    #[serde(default = "default_cleanup_mode")]
    pub cleanup_mode: String,
    /// Remove explicit stutters without paraphrasing or collapsing emphasis.
    #[serde(default = "default_stutter_correction")]
    pub stutter_correction: bool,
    #[serde(default = "default_max_recording_seconds")]
    pub max_recording_seconds: u32,
    #[serde(default = "default_max_pending_utterances")]
    pub max_pending_utterances: usize,
    #[serde(default)]
    pub retain_audio_for_retry: bool,
    #[serde(default = "default_retry_audio_seconds")]
    pub retry_audio_seconds: u32,
    #[serde(default)]
    pub microphone_name: Option<String>,
    #[serde(default = "default_min_speech_rms")]
    pub min_speech_rms: f32,
    #[serde(default = "default_capture_drain_ms")]
    pub capture_drain_ms: u64,
    #[serde(default = "default_hud_enabled")]
    pub hud_enabled: bool,
}

fn default_schema_version() -> u32 {
    2
}
fn default_formatter_port() -> u16 {
    11434
}
fn default_formatter_num_ctx() -> u32 {
    2048
}
fn default_chimes_enabled() -> bool {
    true
}
fn default_toggle_chord() -> String {
    if cfg!(windows) {
        "ctrl+space"
    } else {
        "ctrl+alt+space"
    }
    .into()
}
fn default_stutter_correction() -> bool {
    true
}
fn default_cleanup_mode() -> String {
    "faithful".into()
}
fn default_max_recording_seconds() -> u32 {
    1200
}
fn default_max_pending_utterances() -> usize {
    2
}
fn default_retry_audio_seconds() -> u32 {
    120
}
fn default_min_speech_rms() -> f32 {
    0.001
}
fn default_capture_drain_ms() -> u64 {
    80
}
fn default_hud_enabled() -> bool {
    true
}

fn default_parakeet_dir() -> String {
    #[cfg(windows)]
    return format!(
        "{}\\Temp\\parla-parakeet\\model",
        std::env::var("LOCALAPPDATA").unwrap_or_default()
    );
    #[cfg(not(windows))]
    return crate::platform::data_dir()
        .join("models/parakeet")
        .to_string_lossy()
        .into_owned();
}

impl Settings {
    /// Normalized backend selector; anything unparseable => whisper.
    pub fn asr_backend_kind(&self) -> AsrBackend {
        match self.asr_backend.to_ascii_lowercase().as_str() {
            "parakeet" => AsrBackend::Parakeet,
            _ => AsrBackend::Whisper, // default + unknown + empty all land here
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AsrBackend {
    Whisper,
    Parakeet,
}

fn default_ptt_chords() -> Vec<String> {
    vec!["ctrl+win".into()]
}

/// Built-in chord for the LL hook fallback path (state uninitialized).
pub fn default_chord() -> HotkeyChord {
    parse_chord("ctrl+win").expect("built-in default chord must parse")
}

/// One parsed chord: ordered key-sets. The LAST set is the trigger key
/// (pressed last to start talking); earlier sets must already be held.
/// Tokens (case-insensitive): ctrl|control, lctrl, rctrl, win|super,
/// lwin, rwin, alt|menu, lalt, ralt, shift, lshift, rshift.
/// A generic token accepts either physical side plus the shared VK;
/// side-specific tokens bind one VK. Unknown token => None.
pub fn parse_chord(s: &str) -> Option<HotkeyChord> {
    const GENERIC: &[&str] = &[
        "CTRL", "CONTROL", "WIN", "SUPER", "CMD", "COMMAND", "ALT", "OPTION", "MENU", "SHIFT",
    ];
    let mut sets: Vec<Vec<u32>> = Vec::new();
    let mut side_specific_only = true;
    for tok in s.split('+') {
        let t = tok.trim().to_ascii_uppercase();
        let generic_hit = GENERIC.contains(&t.as_str());
        let vks: &[u32] = match t.as_str() {
            "CTRL" | "CONTROL" => &[0x11, 0xA2, 0xA3],
            "LCTRL" => &[0xA2],
            "RCTRL" => &[0xA3],
            "WIN" | "SUPER" | "CMD" | "COMMAND" => &[0x5B, 0x5C],
            "LWIN" => &[0x5B],
            "RWIN" => &[0x5C],
            "ALT" | "OPTION" | "MENU" => &[0x12, 0xA4, 0xA5],
            "LALT" => &[0xA4],
            "RALT" => &[0xA5],
            "SHIFT" => &[0x10, 0xA0, 0xA1],
            "SPACE" => &[0x20],
            "LSHIFT" => &[0xA0],
            "RSHIFT" => &[0xA1],
            _ => return None,
        };
        if generic_hit {
            side_specific_only = false;
        }
        sets.push(vks.to_vec());
    }
    if sets.iter().enumerate().any(|(i, g)| {
        sets[..i]
            .iter()
            .any(|p| p.iter().any(|key| g.contains(key)))
    }) {
        return None;
    }
    // Chord needs >=2 distinct groups, OR a single side-specific key that
    // the user dedicates entirely to Parla (e.g. "rctrl"). A single generic
    // modifier (plain "ctrl") is forbidden: it would break Ctrl+C/V.
    if !(sets.len() >= 2 || (sets.len() == 1 && side_specific_only && sets[0] != vec![0x20])) {
        return None;
    }
    Some(HotkeyChord { sets })
}

/// Parsed chord consumed by the LL hook: `sets[k]` = virtual-key set of the
/// k-th declared token; the last entry is the trigger key.
pub struct HotkeyChord {
    pub sets: Vec<Vec<u32>>,
}

impl Settings {
    /// Parse `ptt_chords` into hook-ready chords. Any unparseable entry
    /// invalidates the whole list => default ["ctrl+win"] (spec: corrupt
    /// settings fall back to defaults, never half-applied).
    pub fn hotkey_chords(&self) -> Vec<HotkeyChord> {
        let mut out = Vec::new();
        for s in &self.ptt_chords {
            match parse_chord(s) {
                Some(c) => out.push(c),
                None => {
                    eprintln!("[parla] ptt_chords entry {s:?} invalid; using defaults");
                    return default_ptt_chords()
                        .iter()
                        .filter_map(|s| parse_chord(s))
                        .collect();
                }
            }
        }
        if out.is_empty() {
            return default_ptt_chords()
                .iter()
                .filter_map(|s| parse_chord(s))
                .collect();
        }
        out
    }
}

impl Default for Settings {
    fn default() -> Self {
        let la = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let bundle = crate::bundle::installed();
        Self {
            schema_version: 2,
            ptt_chords: vec!["ctrl+win".into()],
            toggle_chord: default_toggle_chord(),
            asr_model_path: bundle
                .as_ref()
                .map(|b| b.whisper_model.clone())
                .unwrap_or_else(|| {
                    if cfg!(windows) {
                        format!("{}\\Temp\\parla-models\\ggml-small.bin", la)
                    } else {
                        crate::platform::data_dir()
                            .join("models/ggml-small.bin")
                            .to_string_lossy()
                            .into_owned()
                    }
                }),
            asr_server_exe: bundle
                .as_ref()
                .map(|b| b.whisper.clone())
                .unwrap_or_else(|| {
                    if !cfg!(windows) {
                        return crate::platform::data_dir()
                            .join("runtime/whisper/whisper-server")
                            .to_string_lossy()
                            .into_owned();
                    }
                    format!(
                        "{}\\Temp\\parla-whisper\\bin-cublas\\Release\\whisper-server.exe",
                        la
                    )
                }),
            formatter_port: 11434,
            formatter_model: "qwen3.5:9b".into(),
            formatter_num_ctx: 2048,
            history_mode: "auto_delete_24h".into(),
            chimes_enabled: default_chimes_enabled(),
            asr_backend: if bundle.is_some() {
                "parakeet"
            } else {
                "whisper"
            }
            .into(),
            parakeet_model_dir: bundle
                .as_ref()
                .map(|b| b.parakeet_model.clone())
                .unwrap_or_else(default_parakeet_dir),
            cleanup_mode: default_cleanup_mode(),
            stutter_correction: default_stutter_correction(),
            max_recording_seconds: default_max_recording_seconds(),
            max_pending_utterances: default_max_pending_utterances(),
            retain_audio_for_retry: false,
            retry_audio_seconds: default_retry_audio_seconds(),
            microphone_name: None,
            min_speech_rms: default_min_speech_rms(),
            capture_drain_ms: default_capture_drain_ms(),
            hud_enabled: true,
        }
    }
}

fn settings_path() -> String {
    crate::platform::settings_path()
        .to_string_lossy()
        .into_owned()
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("settings path has no parent")?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = parent.join(format!(
        ".settings-{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let old: Vec<u16> = tmp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let new: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
        }
        if unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), 0x1 | 0x8) } == 0 {
            let e = std::io::Error::last_os_error();
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

impl Settings {
    /// Load from an explicit path; ANY parse failure returns defaults (spec
    /// failure mode). `load()` delegates to the standard config location.
    pub fn load_from(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(txt) => serde_json::from_str::<Self>(&txt)
                .map(|mut s| {
                    s.normalize();
                    if s.schema_version > 2 {
                        s.history_mode = "never".into();
                        s.retain_audio_for_retry = false;
                        s.cleanup_mode = "faithful".into();
                    }
                    s
                })
                .unwrap_or_else(|e| {
                    eprintln!("[parla] settings unreadable ({e}); using defaults");
                    let mut s = Self::default();
                    s.history_mode = "never".into();
                    s
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !Path::new(path).exists() => {
                Self::default()
            }
            Err(e) => {
                eprintln!("[parla] cannot read settings ({e}); history disabled");
                let mut settings = Self::default();
                settings.history_mode = "never".into();
                settings
            }
        }
    }

    pub fn load() -> Self {
        let mut settings = Self::load_from(&settings_path());
        if let Ok(path) = std::env::var("PARLA_MODEL") {
            if !path.trim().is_empty() {
                settings.asr_model_path = path;
            }
        }
        settings
    }

    pub fn save_to(&self, path: &str) -> Result<(), String> {
        if self.schema_version > 2 {
            return Err("settings schema is newer than this build".into());
        }
        let mut effective = self.clone();
        effective.normalize();
        let txt = serde_json::to_vec_pretty(&effective).map_err(|e| e.to_string())?;
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        atomic_write(p, &txt)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings_path())
    }
}

impl Settings {
    /// Clamp user supplied values so a malformed file cannot create unbounded
    /// capture/queue/resource use. This is also the schema migration boundary.
    pub fn normalize(&mut self) {
        if self.schema_version <= 2 {
            self.schema_version = 2;
        }
        if self.toggle_chord.trim().is_empty() || parse_chord(&self.toggle_chord).is_none() {
            self.toggle_chord = default_toggle_chord();
        }
        if self.cleanup_mode != "faithful" && self.cleanup_mode != "polished" {
            self.cleanup_mode = default_cleanup_mode();
        }
        self.max_recording_seconds = self.max_recording_seconds.clamp(1, 1200);
        self.max_pending_utterances = self.max_pending_utterances.clamp(1, 2);
        if self.asr_model_path.is_empty() {
            self.asr_model_path = Settings::default().asr_model_path;
        }
        if self.asr_server_exe.is_empty() {
            self.asr_server_exe = Settings::default().asr_server_exe;
        }
        if self.formatter_model.is_empty() {
            self.formatter_model = "qwen3.5:9b".into();
        }
        if self.history_mode.is_empty() {
            self.history_mode = "auto_delete_24h".into();
        }
        if !matches!(
            self.history_mode.as_str(),
            "store" | "auto_delete_24h" | "never"
        ) {
            self.history_mode = "never".into();
        }
        self.formatter_num_ctx = self.formatter_num_ctx.clamp(1024, 32768);
        if self.formatter_port == 0 {
            self.formatter_port = default_formatter_port();
        }
        self.retry_audio_seconds = self.retry_audio_seconds.clamp(1, 600);
        if !self.min_speech_rms.is_finite() {
            self.min_speech_rms = default_min_speech_rms();
        }
        self.min_speech_rms = self.min_speech_rms.clamp(0.0, 1.0);
        self.capture_drain_ms = self.capture_drain_ms.clamp(0, 500);
    }
    pub fn toggle_hotkey_chord(&self) -> HotkeyChord {
        parse_chord(&self.toggle_chord)
            .unwrap_or_else(|| parse_chord(&default_toggle_chord()).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hermetic: each test gets its own file under LOCALAPPDATA\Temp so
    // parallel test threads never touch each other or the live config.
    fn tmp_settings_path(tag: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "parla-test-settings-{}-{tag}.json",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn roundtrip_via_disk() {
        let path = tmp_settings_path("roundtrip");
        let mut s = Settings::default();
        s.formatter_num_ctx = 1024;
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.formatter_num_ctx, 1024);
        assert_eq!(loaded.formatter_model, "qwen3.5:9b");
        s.chimes_enabled = false;
        s.save_to(&path).unwrap();
        assert!(
            !Settings::load_from(&path).chimes_enabled,
            "Windows replacement must replace an existing file"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn future_schema_write_preserves_original() {
        let path = tmp_settings_path("future");
        let original = r#"{"schema_version":99,"custom":"preserve"}"#;
        std::fs::write(&path, original).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.schema_version, 99);
        assert_eq!(s.history_mode, "never");
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = tmp_settings_path("corrupt");
        std::fs::write(&path, "{ not json !!!").unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.formatter_model, Settings::default().formatter_model);
        assert_eq!(s.history_mode, "never");
        let _ = std::fs::remove_file(&path);
        let inaccessible = Settings::load_from(&std::env::temp_dir().to_string_lossy());
        assert_eq!(
            inaccessible.history_mode, "never",
            "I/O errors must not enable history"
        );
    }

    #[test]
    fn missing_ptt_chords_field_defaults() {
        // Old settings.json shape (pre-2026-08-23) had no ptt_chords key.
        let path = tmp_settings_path("legacy");
        std::fs::write(
            &path,
            r#"{"asr_model_path":"x","asr_server_exe":"y","formatter_port":11434,"formatter_model":"qwen3.5:9b","formatter_num_ctx":2048,"history_mode":"store","chimes_enabled":true}"#,
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.ptt_chords, vec!["ctrl+win".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_chord_tokens() {
        assert!(parse_chord("ctrl+command").is_some());
        assert!(parse_chord("control+option+space").is_some());
        assert!(parse_chord("cmd").is_none());
        // ctrl+win: ctrl held first, win is the trigger key
        let c = parse_chord("ctrl+win").unwrap();
        assert_eq!(c.sets.len(), 2);
        assert!(c.sets[0].contains(&0x11));
        assert!(c.sets[1].contains(&0x5B) || c.sets[1].contains(&0x5C));
        // order matters by declaration: win first => win held, ctrl triggers
        let c2 = parse_chord("Win+Ctrl").unwrap();
        assert!(c2.sets[0].contains(&0x5B) || c2.sets[0].contains(&0x5C));
        assert!(c2.sets[1].contains(&0x11));
        // aliases + case-insensitive
        assert!(parse_chord("Control+Super").is_some());
        // side-specific single dedicated key allowed
        let rc = parse_chord("rctrl").unwrap();
        assert_eq!(rc.sets, vec![vec![0xA3]]);
        // a lone GENERIC modifier is forbidden (would break Ctrl+C/V)
        assert!(parse_chord("ctrl").is_none());
        // junk / empty / bare letters rejected
        assert!(parse_chord("ctrl+f9").is_none());
        assert!(parse_chord("").is_none());
        assert!(parse_chord("space").is_none());
        assert!(parse_chord("ctrl+rctrl").is_none());
    }

    #[test]
    fn invalid_entry_falls_back_wholesale() {
        let mut s = Settings::default();
        s.ptt_chords = vec!["ctrl+shift".into(), "banana".into()];
        let chords = s.hotkey_chords();
        // wholesale fallback to default, not half-applied
        assert_eq!(chords.len(), 1);
        assert!(chords[0].sets[0].contains(&0x11)); // ctrl
        assert!(chords[0].sets[1].contains(&0x5B) || chords[0].sets[1].contains(&0x5C));
        // win trigger
    }

    #[test]
    fn asr_backend_defaults_and_normalizes() {
        assert_eq!(Settings::default().asr_backend_kind(), AsrBackend::Whisper);
        let mut s = Settings::default();
        s.asr_backend = "PARAKEET".into(); // case-insensitive
        assert_eq!(s.asr_backend_kind(), AsrBackend::Parakeet);
        s.asr_backend = "bananas".into(); // unknown => whisper
        assert_eq!(s.asr_backend_kind(), AsrBackend::Whisper);
        s.asr_backend = "".into();
        assert_eq!(s.asr_backend_kind(), AsrBackend::Whisper);
    }

    #[test]
    fn legacy_settings_file_lacks_asr_fields() {
        // Pre-2026-09-02 settings.json has no asr_backend/parakeet_model_dir.
        let path = tmp_settings_path("legacy-asr");
        std::fs::write(
            &path,
            r#"{"asr_model_path":"x","asr_server_exe":"y","formatter_port":11434,"formatter_model":"qwen3.5:9b","formatter_num_ctx":2048,"history_mode":"store","chimes_enabled":true,"ptt_chords":["ctrl+win"]}"#,
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.asr_backend_kind(), AsrBackend::Whisper);
        assert!(
            s.stutter_correction,
            "existing settings gain stutter correction by default"
        );
        assert_eq!(s.parakeet_model_dir, default_parakeet_dir());
        let _ = std::fs::remove_file(&path);
    }
}
