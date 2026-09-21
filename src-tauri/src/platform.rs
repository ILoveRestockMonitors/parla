//! OS boundaries kept outside recognition and formatting.
use std::path::PathBuf;

pub fn initialize() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    crate::context::native::initialize()?;
    Ok(())
}

pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("PARLA_DATA_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    #[cfg(windows)]
    return PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Parla");
    #[cfg(target_os = "macos")]
    return PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/Parla");
    #[cfg(all(unix, not(target_os = "macos")))]
    return std::env::var_os("XDG_DATA_HOME")
        .filter(|p| PathBuf::from(p).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("parla");
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland")))
}

/// A held kernel lock, released even if the process crashes. The Windows name
/// intentionally stays compatible with previous installed versions.
pub struct SingleInstance {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    _file: std::fs::File,
}
impl SingleInstance {
    pub fn acquire() -> Result<Option<Self>, String> {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
            use windows::Win32::System::Threading::CreateMutexW;
            let name: Vec<u16> = "Local\\Parla.SingleInstance.v1\0".encode_utf16().collect();
            let handle = CreateMutexW(None, false, windows::core::PCWSTR(name.as_ptr()))
                .map_err(|e| e.to_string())?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self { handle }))
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .open(data_dir().join("instance.lock"))
                .map_err(|e| e.to_string())?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let e = std::io::Error::last_os_error();
                return if e.kind() == std::io::ErrorKind::WouldBlock {
                    Ok(None)
                } else {
                    Err(e.to_string())
                };
            }
            Ok(Some(Self { _file: file }))
        }
    }
}
#[cfg(windows)]
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

pub fn diagnostics() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    let note = "Allow Microphone, Accessibility and Input Monitoring for Parla in System Settings → Privacy & Security, then restart Parla. Shortcuts are observed globally and may also reach other apps; choose unused chords. Restore uses manual Copy raw. Status is shown in this dashboard; the floating HUD is Windows-only.";
    #[cfg(target_os = "linux")]
    let note = if is_wayland() {
        "Wayland: global shortcuts, target inspection and automatic insertion are unavailable. Use Start/stop here, or bind 'parla toggle' in your desktop's shortcut settings. Copy final and paste manually. Microphone access requires a working ALSA/PipeWire device. The floating HUD is Windows-only."
    } else {
        "X11: shortcuts are observed globally and may also reach other apps; choose unused chords. Automatic text insertion requires unchanged focused window and no typing/clicks. Password-field detection and verified Restore are unavailable on X11. Use Copy final for unsupported editors. The floating HUD is Windows-only."
    };
    #[cfg(windows)]
    let note = "Allow desktop microphone access in Windows Privacy settings. Elevated apps may require manual paste.";
    let mut value = serde_json::json!({"os":std::env::consts::OS,"wayland":is_wayland(),"settings_path":settings_path(),"guidance":note,"native_hud":cfg!(windows)});
    #[cfg(target_os = "macos")]
    {
        value["accessibility_granted"] = crate::context::native::accessibility_trusted().into();
        value["input_monitoring_granted"] =
            crate::context::native::input_monitoring_granted().into();
        value["microphone_permission"] = "Requested by macOS when capture opens; review Privacy & Security → Microphone if unavailable.".into();
    }
    let _ = &mut value;
    value
}
