//! Commit one complete clipboard paste to the captured field. Never retry
//! after sending input: an unconfirmed paste may already have reached the app.
use crate::context::target::{self, TargetSnapshot};
use std::sync::Mutex;
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

static COMMIT_LOCK: Mutex<()> = Mutex::new(());
#[derive(Debug, Clone)]
pub struct CommitReceipt {
    pub before: TargetSnapshot,
    pub after: TargetSnapshot,
    pub payload: String,
    pub verified: bool,
}
pub fn modifiers_released() -> bool {
    [0x10, 0x11, 0x12, 0x5b, 0x5c, 0x20]
        .iter()
        .all(|&vk| unsafe { GetAsyncKeyState(vk) >= 0 })
}
fn event(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0x5041524c,
            },
        },
    }
}
fn paste_events() -> [INPUT; 4] {
    [
        event(0x11, 0, KEYBD_EVENT_FLAGS(0)),
        event(0x56, 0, KEYBD_EVENT_FLAGS(0)),
        event(0x56, 0, KEYEVENTF_KEYUP),
        event(0x11, 0, KEYEVENTF_KEYUP),
    ]
}
fn send_paste(target: &TargetSnapshot, clipboard_sequence: u32) -> Result<(), String> {
    if !target.native_still_focused() || !modifiers_released() {
        return Err("focus or modifiers changed before paste".into());
    }
    if unsafe { GetClipboardSequenceNumber() } != clipboard_sequence {
        return Err("clipboard changed before paste; use Copy final".into());
    }
    let events = paste_events();
    let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
    if sent != events.len() as u32 {
        // Release only the synthetic shortcut keys; never resend the paste.
        if sent > 0 {
            let release = [
                event(0x56, 0, KEYEVENTF_KEYUP),
                event(0x11, 0, KEYEVENTF_KEYUP),
            ];
            unsafe {
                SendInput(&release, std::mem::size_of::<INPUT>() as i32);
            }
        }
        return Err(format!(
            "paste input incomplete ({sent}/4 events); automatic retry disabled"
        ));
    }
    Ok(())
}
fn verify_receipt(before: &TargetSnapshot, after: &TargetSnapshot, text: &str) -> bool {
    if !before.same_field(after)
        || !before.context.available
        || !after.context.available
        || after.context.selected_text.as_deref() != Some("")
        || before.context.text_after != after.context.text_after
    {
        return false;
    }
    let expected = format!(
        "{}{text}",
        before.context.text_before.as_deref().unwrap_or_default()
    );
    let suffix = crate::context::tail_chars(&expected, 120);
    after
        .context
        .text_before
        .as_deref()
        .is_some_and(|actual| actual.ends_with(&suffix))
}
pub fn commit(target: &TargetSnapshot, text: &str) -> Result<CommitReceipt, String> {
    let _guard = COMMIT_LOCK
        .lock()
        .map_err(|_| "insertion lock unavailable")?;
    commit_locked(target, text)
}
fn commit_locked(target: &TargetSnapshot, text: &str) -> Result<CommitReceipt, String> {
    if text.is_empty() || text.len() > 256 * 1024 {
        return Err("empty or oversized candidate".into());
    }
    // Keep the full transcript available even if a preflight check blocks
    // insertion. Deliberately retain it, including after an unconfirmed paste;
    // restoring old clipboard data too early races asynchronous editors.
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| format!("clipboard unavailable: {e}; use Copy final"))?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|e| format!("clipboard busy: {e}; use Copy final"))?;
    let sequence = unsafe { GetClipboardSequenceNumber() };
    if clipboard.get_text().ok().as_deref() != Some(text)
        || unsafe { GetClipboardSequenceNumber() } != sequence {
        return Err("clipboard changed while preparing paste; use Copy final".into());
    }
    let preflight = (|| {
        if !modifiers_released() {
            return Err("dictation shortcut or modifier still held; text saved for recovery — use Copy final".into());
        }
        let current = target.verify_for_insertion(150)?;
        send_paste(&current, sequence)?;
        Ok(current)
    })();
    let current = preflight.map_err(|e: String| {
        if unsafe { GetClipboardSequenceNumber() } == sequence {
            format!("Clipboard ready — if text is missing, press Ctrl+V. {e}")
        } else {
            e
        }
    })?;
    let target = &current;
    let mut after = TargetSnapshot::capture(100);
    let mut verified = verify_receipt(target, &after, text);
    // SendInput acceptance is not read-back confirmation. Give asynchronous
    // editors a short, bounded opportunity to expose the committed range.
    for delay in [20, 40, 80, 120, 200] {
        if verified || !target.context.available || !target.native_still_focused() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(delay));
        after = TargetSnapshot::capture(100);
        verified = verify_receipt(target, &after, text);
    }
    Ok(CommitReceipt {
        before: target.clone(),
        after,
        payload: text.into(),
        verified,
    })
}
pub fn replace(
    receipt: &CommitReceipt,
    replacement: &str,
) -> Result<Option<CommitReceipt>, String> {
    let _guard = COMMIT_LOCK
        .lock()
        .map_err(|_| "insertion lock unavailable")?;
    if !receipt.verified || !modifiers_released() {
        return Err(
            "previous insertion is not safely replaceable; copy recovered text instead".into(),
        );
    }
    target::select_inserted_text(&receipt.after, &receipt.payload)?;
    let selected = TargetSnapshot::capture(150);
    if !receipt.after.same_field(&selected)
        || selected.context.selected_text.as_deref() != Some(&receipt.payload)
    {
        return Err("selection could not be verified".into());
    }
    if replacement.is_empty() {
        if !selected.native_still_focused() || !modifiers_released() {
            return Err("field changed before deletion".into());
        }
        let events = [
            event(0x2e, 0, KEYBD_EVENT_FLAGS(0)),
            event(0x2e, 0, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
        if sent != 2 {
            return Err("delete was not confirmed; automatic retry disabled".into());
        }
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let after = TargetSnapshot::capture(100);
            if verify_receipt(&selected, &after, "") {
                return Ok(None);
            }
            if !selected.native_still_focused() {
                break;
            }
        }
        Err("Deletion could not be verified; automatic retry disabled.".into())
    } else {
        commit_locked(&selected, replacement).map(Some)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::target::FieldContext;
    #[test]
    fn receipt_requires_exact_suffix_and_same_field() {
        let before = TargetSnapshot {
            hwnd: 1,
            focus: 2,
            context: FieldContext {
                available: true,
                text_before: Some("Before ".into()),
                text_after: Some("after".into()),
                selected_text: Some("old".into()),
                ..Default::default()
            },
        };
        let mut after = before.clone();
        after.context.selected_text = Some(String::new());
        after.context.text_before = Some("Before Claude ".into());
        assert!(verify_receipt(&before, &after, "Claude "));
        after.context.text_before = Some("Before claw ed ".into());
        assert!(!verify_receipt(&before, &after, "Claude "));
        after.context.text_before = Some("Before Claude ".into());
        after.focus = 3;
        assert!(!verify_receipt(&before, &after, "Claude "));
    }
}
