//! Commit a complete candidate to its captured field. Partial SendInput is
//! never followed by a full clipboard paste, which would duplicate text.
use crate::context::target::{self, TargetSnapshot};
use std::sync::Mutex;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
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
fn send_text(target: &TargetSnapshot, text: &str) -> Result<(), String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    for (batch_index, batch) in units.chunks(64).enumerate() {
        if !target.native_still_focused() || !modifiers_released() {
            return Err(format!(
                "insertion interrupted after at most {} UTF-16 units; recovered text is available",
                batch_index * 64
            ));
        }
        let mut events = Vec::with_capacity(batch.len() * 2);
        for &unit in batch {
            events.push(event(0, unit, KEYEVENTF_UNICODE));
            events.push(event(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
        if sent != events.len() as u32 {
            return Err(format!(
                "partial insertion ({sent}/{} input events in batch); automatic retry disabled",
                events.len()
            ));
        }
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
    if !modifiers_released() {
        return Err("dictation shortcut or modifier still held; text saved for recovery — use Copy final".into());
    }
    target.verify_for_insertion(150)?;
    send_text(target, text)?;
    let mut after = TargetSnapshot::capture(100);
    let mut verified = verify_receipt(target, &after, text);
    // SendInput acceptance is not read-back confirmation. Give asynchronous
    // editors a short, bounded opportunity to expose the committed range.
    for _ in 0..2 {
        if verified || !target.native_still_focused() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
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
