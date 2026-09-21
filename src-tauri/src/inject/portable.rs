//! A guarded, single paste gesture. Never sends Return or retries a partial
//! gesture. Clipboard ownership stays alive for X11 manual recovery.
use crate::context::target::TargetSnapshot;
use enigo::{Direction, Enigo, Key, Keyboard};
use std::sync::Mutex;
static COMMIT_LOCK: Mutex<()> = Mutex::new(());
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
#[derive(Debug, Clone)]
pub struct CommitReceipt {
    pub before: TargetSnapshot,
    pub after: TargetSnapshot,
    pub payload: String,
    pub verified: bool,
}
pub fn modifiers_released() -> bool {
    // Wayland has no monitor; allow the pipeline to finish and expose recovery.
    crate::platform::is_wayland() || crate::hotkey::modifiers_released()
}
fn copy(text: &str) -> Result<(), String> {
    let mut clipboard = CLIPBOARD.lock().map_err(|_| "Clipboard lock unavailable")?;
    if clipboard.is_none() {
        *clipboard =
            Some(arboard::Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?);
    }
    clipboard
        .as_mut()
        .unwrap()
        .set_text(text)
        .map_err(|e| format!("Clipboard write: {e}"))
}
pub fn commit(target: &TargetSnapshot, text: &str) -> Result<CommitReceipt, String> {
    let _guard = COMMIT_LOCK
        .lock()
        .map_err(|_| "Insertion lock unavailable")?;
    if text.is_empty() || text.len() > 256 * 1024 {
        return Err("Empty or oversized candidate".into());
    }
    // Until exact editor capabilities can be identified, flatten ALL portable
    // destinations. Newlines pasted into terminals can execute commands.
    let payload = crate::formatter::layout::single_line(text).into_owned();
    let copied = copy(&payload);
    if crate::platform::is_wayland() {
        return Err("Wayland requires manual paste: use Copy final in the dashboard.".into());
    }
    copied?;
    if !modifiers_released() {
        return Err(
            "Release the dictation shortcut before insertion; use Copy final to recover.".into(),
        );
    }
    target.verify_for_insertion(150)?;
    let mut keyboard = Enigo::new(&enigo::Settings {
        release_keys_when_dropped: false,
        ..Default::default()
    })
    .map_err(|e| {
        format!(
            "Text insertion unavailable: {e}; check Accessibility permissions and use Copy final."
        )
    })?;
    // Enigo setup can take time. Revalidate immediately before the gesture.
    target.verify_for_insertion(100)?;
    if !modifiers_released() {
        return Err("Modifier pressed before insertion; use Copy final.".into());
    }
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;
    #[cfg(target_os = "linux")]
    let shift = target.context.terminal;
    #[cfg(not(target_os = "linux"))]
    let shift = false;
    let result = (|| {
        keyboard.key(modifier, Direction::Press)?;
        if shift {
            keyboard.key(Key::Shift, Direction::Press)?;
        }
        keyboard.key(Key::Unicode('v'), Direction::Click)
    })();
    let shift_release = if shift {
        keyboard.key(Key::Shift, Direction::Release)
    } else {
        Ok(())
    };
    let release = keyboard.key(modifier, Direction::Release);
    if result.is_err() || shift_release.is_err() || release.is_err() {
        return Err("Paste gesture was not confirmed; automatic retry disabled. Check the field before using Copy final.".into());
    }
    // Event acceptance does not prove that the editor accepted its clipboard.
    // No speculative read-back success and no document replacement/Restore.
    Ok(CommitReceipt {
        before: target.clone(),
        after: TargetSnapshot::capture(100),
        payload,
        verified: false,
    })
}
pub fn replace(_: &CommitReceipt, _: &str) -> Result<Option<CommitReceipt>, String> {
    Err(
        "Verified Restore is unavailable on this platform; use Copy raw and replace manually."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn portable_paste_never_contains_command_separating_controls() {
        let input = "Todo:\r\n• one\n• two\tthree\u{2028}four\u{2029}five";
        let text = crate::formatter::layout::single_line(input);
        assert!(!text
            .chars()
            .any(|c| c.is_control() || ['\u{2028}', '\u{2029}'].contains(&c)));
        for word in ["one", "two", "three", "four", "five"] {
            assert!(text.contains(word));
        }
    }
}
