//! Optional status HUD; never activates or takes keyboard focus.
use serde_json::Value;

pub fn status_label(snapshot: &Value) -> Option<String> {
    let settings = snapshot.get("effective_settings")?;
    if settings.get("hud_enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let recording = snapshot
        .get("recording")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let processing = snapshot
        .get("processing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if recording {
        let seconds = snapshot
            .get("recording_elapsed_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            / 1000;
        let hint = if snapshot.get("mode").and_then(Value::as_str) == Some("hold") {
            "Release keys to stop".to_string()
        } else {
            let chord = settings
                .get("toggle_chord")
                .and_then(Value::as_str)
                .unwrap_or("Ctrl+Space");
            format!("{chord} to stop")
        };
        return Some(format!(
            "Parla • Recording {:02}:{:02} • {hint}",
            seconds / 60,
            seconds % 60
        ));
    }
    if processing {
        return Some("Parla • Processing…".into());
    }
    None
}

#[cfg(windows)]
pub fn spawn() {
    std::thread::Builder::new()
        .name("parla-hud".into())
        .spawn(|| unsafe {
            use windows::core::{w, PCWSTR};
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::*;
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Parla"),
                WINDOW_STYLE(WS_POPUP.0 | WS_BORDER.0),
                0,
                0,
                430,
                60,
                None,
                None,
                None,
                None,
            );
            if hwnd.0 == 0 {
                return;
            }
            let x = (GetSystemMetrics(SM_CXSCREEN) - 430) / 2;
            let y = GetSystemMetrics(SM_CYSCREEN) - 100;
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, 430, 60, SWP_NOACTIVATE);
            let mut msg = MSG::default();
            loop {
                while PeekMessageW(&mut msg, HWND(0), 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        let _ = DestroyWindow(hwnd);
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let label = status_label(&crate::runtime::hud_snapshot());
                match label {
                    Some(text) => {
                        let wide: Vec<u16> =
                            text.encode_utf16().chain(std::iter::once(0)).collect();
                        let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                    None => {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .ok();
}
#[cfg(not(windows))]
pub fn spawn() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_recording_and_hides_idle() {
        let v = serde_json::json!({"recording":true,"recording_elapsed_ms":83000,"effective_settings":{"hud_enabled":true}});
        assert_eq!(
            status_label(&v).unwrap(),
            "Parla • Recording 01:23 • Ctrl+Space to stop"
        );
        let idle = serde_json::json!({"recording":false,"processing":false,"error":"insertion failed","effective_settings":{"hud_enabled":true}});
        assert!(status_label(&idle).is_none());
    }
}
