// AsrEngine trait + shared types.
pub mod parakeet_local;
pub mod registry;
pub mod wav;
pub mod whisper_local;

pub use whisper_local::WhisperServerClient;

use crate::store::settings::{AsrBackend, Settings};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// Owns the optional local backend process. A single manager serializes
/// startup/recovery and retains the child so slow model loads cannot spawn
/// duplicate processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    NotStarted,
    Starting,
    Running,
    Exited,
    Failed,
}
fn should_spawn(state: ProcessState) -> bool {
    matches!(
        state,
        ProcessState::NotStarted | ProcessState::Exited | ProcessState::Failed
    )
}
fn slot_decision(
    _state: ProcessState,
    owned_child: bool,
    port_open: bool,
    healthy: bool,
    identity_match: bool,
) -> Result<bool, String> {
    if owned_child && !identity_match {
        return Err("owned backend identity changed; restart required".into());
    }
    if healthy {
        return Ok(false);
    }
    if port_open && !owned_child {
        return Err("backend port is occupied by an incompatible service".into());
    }
    // A previously reused external server may have exited. Its stale
    // Running label must never prevent a replacement when the port is free.
    Ok(!owned_child)
}
struct Slot {
    child: Option<std::process::Child>,
    state: ProcessState,
    identity: Option<String>,
}

/// Poll an owned child without waiting.  Clearing its identity is important:
/// a new model may be selected after the old process has actually exited.
fn poll_child(slot: &mut Slot, name: &str) -> Result<bool, String> {
    let Some(child) = slot.child.as_mut() else {
        return Ok(false);
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            slot.child = None;
            slot.identity = None;
            slot.state = ProcessState::Exited;
            let _ = (name, status);
            Ok(false)
        }
        Ok(None) => Ok(true),
        Err(e) => {
            slot.state = ProcessState::Failed;
            Err(format!("{name} process status: {e}"))
        }
    }
}
pub struct BackendManager {
    parakeet: Mutex<Slot>,
    whisper: Mutex<Slot>,
}
impl BackendManager {
    pub fn new() -> Self {
        let slot = || {
            Mutex::new(Slot {
                child: None,
                state: ProcessState::NotStarted,
                identity: None,
            })
        };
        Self {
            parakeet: slot(),
            whisper: slot(),
        }
    }
    pub fn ensure_engine(
        &self,
        settings: &Settings,
        backend: AsrBackend,
    ) -> Result<Box<dyn AsrEngine>, String> {
        match backend {
            AsrBackend::Whisper => {
                let port = 9292u16;
                let alive = || whisper_ready(port);
                let mut slot = self.whisper.lock().map_err(|_| "backend mutex poisoned")?;
                let _ = poll_child(&mut slot, "whisper")?;
                let identity = format!(
                    "whisper:{}",
                    std::env::var("PARLA_MODEL")
                        .unwrap_or_else(|_| settings.asr_model_path.clone())
                );
                if slot.child.is_some()
                    && slot.identity.as_deref().is_some_and(|old| old != identity)
                {
                    return Err(
                        "owned whisper process uses a different model; restart required".into(),
                    );
                }
                if !alive() {
                    let port_open = std::net::TcpStream::connect_timeout(
                        &([127, 0, 0, 1], port).into(),
                        std::time::Duration::from_millis(100),
                    )
                    .is_ok();
                    if slot.child.is_some() {
                        let _ = poll_child(&mut slot, "whisper")?;
                    }
                    if slot.child.is_none()
                        && slot_decision(slot.state, false, port_open, false, true)?
                    {
                        let exe = std::env::var("PARLA_WHISPER_SERVER")
                            .unwrap_or_else(|_| settings.asr_server_exe.clone());
                        let model = std::env::var("PARLA_MODEL")
                            .unwrap_or_else(|_| settings.asr_model_path.clone());
                        if exe.trim().is_empty() || model.trim().is_empty() {
                            return Err("whisper server/model path is not configured".into());
                        }
                        slot.state = ProcessState::Starting;
                        match spawn_whisper(&exe, &model, port) {
                            Ok(child) => slot.child = Some(child),
                            Err(e) => {
                                slot.state = ProcessState::Failed;
                                return Err(e);
                            }
                        }
                        slot.identity = Some(identity);
                    }
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                    while !alive() && std::time::Instant::now() < deadline {
                        if slot.child.is_some() && !poll_child(&mut slot, "whisper")? {
                            return Err("whisper process exited before becoming ready".into());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    if !alive() {
                        slot.state = ProcessState::Failed;
                        return Err("whisper server did not become ready within 20 s".into());
                    }
                    slot.state = ProcessState::Running;
                } else {
                    slot.state = ProcessState::Running;
                }
                Ok(Box::new(WhisperServerClient::new(port)))
            }
            AsrBackend::Parakeet => {
                let client = parakeet_local::ParakeetClient::from_settings(settings);
                if !client.weights_present() {
                    return Err(format!("parakeet weights missing in {}", client.model_dir));
                }
                client.ensure_shim_deployed()?;
                let identity = format!("parakeet:{}", client.model_dir);
                let mut slot = self.parakeet.lock().map_err(|_| "backend mutex poisoned")?;
                let _ = poll_child(&mut slot, "parakeet")?;
                if slot.child.is_some()
                    && slot.identity.as_deref().is_some_and(|old| old != identity)
                {
                    return Err(
                        "owned parakeet process uses a different model; restart required".into(),
                    );
                }
                if !client.alive() {
                    let port_open = std::net::TcpStream::connect_timeout(
                        &([127, 0, 0, 1], 9293).into(),
                        std::time::Duration::from_millis(100),
                    )
                    .is_ok();
                    if slot.child.is_some() {
                        if poll_child(&mut slot, "parakeet")? {
                            slot.state = ProcessState::Starting;
                        }
                    }
                    let owned = slot.child.is_some();
                    if slot.identity.as_deref().is_some_and(|old| old != identity) {
                        return Err("owned backend identity changed; restart required".into());
                    }
                    if slot.child.is_none()
                        && slot_decision(slot.state, owned, port_open, false, true)?
                    {
                        slot.state = ProcessState::Starting;
                        match spawn_shim(&client) {
                            Ok(child) => slot.child = Some(child),
                            Err(e) => {
                                slot.state = ProcessState::Failed;
                                return Err(e);
                            }
                        }
                        slot.identity = Some(identity);
                    }
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
                    while !client.alive() && std::time::Instant::now() < deadline {
                        if slot.child.is_some() && !poll_child(&mut slot, "parakeet")? {
                            return Err("parakeet process exited before becoming ready".into());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    if !client.alive() {
                        return Err("parakeet backend did not become ready before deadline".into());
                    }
                    slot.state = ProcessState::Running;
                }
                slot.state = ProcessState::Running;
                Ok(Box::new(client))
            }
        }
    }
    pub fn recover(
        &self,
        settings: &Settings,
        backend: AsrBackend,
    ) -> Result<Box<dyn AsrEngine>, String> {
        let slot = match backend {
            AsrBackend::Parakeet => &self.parakeet,
            AsrBackend::Whisper => &self.whisper,
        };
        if let Ok(mut s) = slot.lock() {
            if let Some(mut child) = s.child.take() {
                child
                    .kill()
                    .map_err(|e| format!("stop backend child: {e}"))?;
                child
                    .wait()
                    .map_err(|e| format!("wait backend child: {e}"))?;
            }
            s.identity = None;
            s.state = ProcessState::Exited;
        }
        self.ensure_engine(settings, backend)
    }
}
static MANAGER: OnceLock<BackendManager> = OnceLock::new();
/// The installed whisper.cpp health contract does not identify its model.
/// Only a child launched by this manager has a known configuration.
pub fn whisper_model_verification() -> &'static str {
    let Some(manager) = MANAGER.get() else {
        return "external_model_unverified";
    };
    match manager.whisper.try_lock() {
        Ok(slot) if slot.child.is_some() && slot.identity.is_some() => "owned_configuration",
        Ok(_) => "external_model_unverified",
        Err(_) => "checking",
    }
}
pub fn ensure_engine_for(
    settings: &Settings,
    backend: AsrBackend,
) -> Result<Box<dyn AsrEngine>, String> {
    MANAGER
        .get_or_init(BackendManager::new)
        .ensure_engine(settings, backend)
}

pub struct RawTranscript {
    pub text: String,
    pub confidence: Option<f32>,
    pub language: String,
}

/// Engine-agnostic speech-to-text. Implementations must be warm before first
/// use (warm at launch per spec design tells) and thread-safe.
pub trait AsrEngine: Send {
    /// Transcribe 16 kHz mono PCM. Hotwords bias recognition where supported
    /// (whisper initial_prompt; Deepgram keyterms).
    fn transcribe(
        &self,
        pcm: &[i16],
        language: &str,
        hotwords: &[String],
    ) -> Result<RawTranscript, String>;
    fn name(&self) -> &'static str;
}

/// Runtime-mutable backend selector, shared between the pipeline thread and
/// the dashboard control endpoint. 0 = whisper, 1 = parakeet.
static BACKEND: AtomicU8 = AtomicU8::new(0);

fn kind_to_u8(k: AsrBackend) -> u8 {
    match k {
        AsrBackend::Whisper => 0,
        AsrBackend::Parakeet => 1,
    }
}

pub fn set_backend(k: AsrBackend) {
    BACKEND.store(kind_to_u8(k), Ordering::SeqCst);
}

pub fn current_backend() -> AsrBackend {
    match BACKEND.load(Ordering::SeqCst) {
        1 => AsrBackend::Parakeet,
        _ => AsrBackend::Whisper,
    }
}

pub fn backend_name(k: AsrBackend) -> &'static str {
    match k {
        AsrBackend::Whisper => "whisper",
        AsrBackend::Parakeet => "parakeet",
    }
}

/// Pick the engine for the CURRENT backend at call time. whisper is always
/// constructible; parakeet lazily spawns its shim only when its weights exist.
/// Returns Err(explanation) when the selected backend is not usable.
pub fn ensure_engine(settings: &Settings) -> Result<Box<dyn AsrEngine>, String> {
    ensure_engine_for(settings, current_backend())
}

/// Spawn the python shim detached. NEVER from a tool-call shell in dev
/// (harness reaping) — this runs inside the engine process, parented to it,
/// hidden window, same recipe class as whisper-server. Shim stdout/stderr go
/// to a log file (NOT null) so cold-start failures are diagnosable.
fn spawn_shim(client: &parakeet_local::ParakeetClient) -> Result<std::process::Child, String> {
    let root = std::path::Path::new(&client.model_dir)
        .parent()
        .ok_or("parakeet dir has no parent")?
        .to_path_buf();
    let shim = root.join("shim.py");
    let log_path = root.join("shim.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("open shim log {log_path:?}: {e}"))?;
    let err_log = log
        .try_clone()
        .map_err(|e| format!("clone shim log handle: {e}"))?;
    let python = std::env::var("PARLA_PARAKEET_PYTHON").unwrap_or_else(|_| {
        let installed = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
            .join("Programs/Python/Python312/python.exe");
        if installed.is_file() {
            installed.to_string_lossy().into_owned()
        } else {
            "python".into()
        }
    });
    let mut cmd = std::process::Command::new(python);
    cmd.arg("-u")
        .arg(&shim)
        .arg("--models")
        .arg(&client.model_dir)
        .arg("--port")
        .arg("9293")
        .stdout(log)
        .stderr(err_log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.spawn().map_err(|e| format!("spawn parakeet shim: {e}"))
}

fn spawn_whisper(exe: &str, model: &str, port: u16) -> Result<std::process::Child, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["-m", model, "-l", "en", "--port", &port.to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn()
        .map_err(|e| format!("failed to spawn whisper server {exe}: {e}"))
}

pub fn whisper_ready(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    ureq::get(&url)
        .timeout(std::time::Duration::from_millis(250))
        .call()
        .ok()
        .and_then(|r| {
            use std::io::Read;
            let mut text = String::new();
            r.into_reader().take(4097).read_to_string(&mut text).ok()?;
            Some(text)
        })
        .filter(|s| s.len() <= 4096)
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
        .and_then(|v| {
            v.get("status")
                .and_then(|x| x.as_str())
                .map(|s| s == "ok" || s == "ready")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod supervisor_tests {
    use super::*;
    #[test]
    fn no_child_is_spawnable_and_starting_is_not() {
        assert!(should_spawn(ProcessState::NotStarted));
        assert!(should_spawn(ProcessState::Exited));
        assert!(!should_spawn(ProcessState::Starting));
        assert!(!should_spawn(ProcessState::Running));
    }

    #[test]
    fn healthy_owned_backend_still_rejects_model_change() {
        let result = slot_decision(ProcessState::Running, true, true, true, false);
        assert!(result.is_err());
    }

    #[test]
    fn occupied_port_without_owned_child_never_spawns() {
        let result = slot_decision(ProcessState::NotStarted, false, true, false, true);
        assert!(result.is_err());
    }

    #[test]
    fn starting_owned_backend_waits_without_spawning() {
        assert_eq!(
            slot_decision(ProcessState::Starting, true, true, false, true).unwrap(),
            false
        );
    }

    #[test]
    fn failed_spawn_is_retryable() {
        assert!(slot_decision(ProcessState::Failed, false, false, false, true).unwrap());
    }

    #[test]
    fn departed_external_backend_can_be_replaced() {
        assert!(slot_decision(ProcessState::Running, false, false, false, true).unwrap());
        assert!(!slot_decision(ProcessState::Failed, true, false, false, true).unwrap());
    }
}
