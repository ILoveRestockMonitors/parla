//! Optional status HUD; never activates or takes keyboard focus.
use serde_json::Value;

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::write_previews;

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
        .spawn(|| {
            if let Err(error) = native::run() {
                eprintln!("[hud] {error}");
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
