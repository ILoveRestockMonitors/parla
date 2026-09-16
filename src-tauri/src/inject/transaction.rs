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

#[derive(Debug, PartialEq, Eq)]
enum TextStep {
    Unicode(Vec<u16>),
    LineBreak,
}

/// Keep each line-break chord and each UTF-16 surrogate pair in one batch.
/// Electron editors can ignore Unicode LF packets; Shift+Enter invokes their
/// normal soft-line-break path instead of their unmodified Enter/send action.
fn text_steps(text: &str) -> Vec<TextStep> {
    let mut steps = Vec::new();
    let mut units = Vec::with_capacity(64);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' || c == '\n' {
            if !units.is_empty() {
                steps.push(TextStep::Unicode(std::mem::take(&mut units)));
            }
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            steps.push(TextStep::LineBreak);
        } else {
            if units.len() + c.len_utf16() > 64 {
                steps.push(TextStep::Unicode(std::mem::take(&mut units)));
            }
            units.extend(c.encode_utf16(&mut [0; 2]).iter().copied());
        }
    }
    if !units.is_empty() {
        steps.push(TextStep::Unicode(units));
    }
    steps
}

fn step_events(step: &TextStep) -> Vec<INPUT> {
    match step {
        TextStep::LineBreak => vec![
            event(0xa0, 0, KEYBD_EVENT_FLAGS(0)), // left Shift down
            event(0x0d, 0, KEYBD_EVENT_FLAGS(0)), // Enter down
            event(0x0d, 0, KEYEVENTF_KEYUP),
            event(0xa0, 0, KEYEVENTF_KEYUP),
        ],
        TextStep::Unicode(units) => units
            .iter()
            .flat_map(|&unit| {
                [
                    event(0, unit, KEYEVENTF_UNICODE),
                    event(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                ]
            })
            .collect(),
    }
}

fn send_step(step: &TextStep, mut send: impl FnMut(&[INPUT]) -> u32) -> Result<(), String> {
    let events = step_events(step);
    let sent = send(&events);
    if sent != events.len() as u32 {
        // A partial chord must not leave Shift or Enter held. Cleanup contains
        // key-UP events only; never retry content or an Enter key-down.
        if *step == TextStep::LineBreak && sent > 0 && sent < 4 {
            let mut releases = Vec::new();
            if sent == 2 {
                releases.push(event(0x0d, 0, KEYEVENTF_KEYUP));
            }
            releases.push(event(0xa0, 0, KEYEVENTF_KEYUP));
            let released = send(&releases);
            if released != releases.len() as u32 {
                return Err(
                    "partial line break; key release was not confirmed; automatic retry disabled"
                        .into(),
                );
            }
        }
        return Err(format!(
            "partial insertion ({sent}/{} input events in batch); automatic retry disabled",
            events.len()
        ));
    }
    Ok(())
}

fn send_text(target: &TargetSnapshot, text: &str) -> Result<(), String> {
    let mut accepted_units = 0;
    for step in text_steps(text) {
        if !target.native_still_focused()
            || !target.input_still_unchanged()
            || !modifiers_released()
        {
            return Err(format!(
                "insertion interrupted after at most {} UTF-16 units; recovered text is available",
                accepted_units
            ));
        }
        send_step(&step, |events| unsafe {
            SendInput(events, std::mem::size_of::<INPUT>() as i32)
        })?;
        match step {
            TextStep::Unicode(units) => accepted_units += units.len(),
            TextStep::LineBreak => {
                accepted_units += 1;
                // Allow the queued Shift-up to settle before the next guard.
                // Never suppress external input or release a user's modifiers.
                for _ in 0..10 {
                    if modifiers_released() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        }
    }
    Ok(())
}

/// Make the candidate available for manual recovery while leaving automatic
/// Unicode insertion independent of clipboard availability.
fn copy_candidate_to_clipboard(text: &str) {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return;
    };
    let _ = clipboard.set_text(text.to_owned());
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
    // Re-resolve the captured target for retries, restore and spoken commands.
    // Browsers must never receive Enter chords, including Shift+Enter. If the
    // process cannot be identified, conservatively flatten this insertion too.
    let app = crate::context::windows::exe_for_window(target.hwnd);
    let payload = match app.as_deref() {
        Some(exe) => crate::formatter::layout::for_app(text, Some(exe)),
        None => crate::formatter::layout::single_line(text),
    };
    let text = payload.as_ref();
    // Preserve the normal commit text for recovery, but never let a locked or
    // unavailable clipboard prevent the original automatic insertion path.
    copy_candidate_to_clipboard(text);
    if !modifiers_released() {
        return Err(
            "dictation shortcut or modifier still held; text saved for recovery — use Copy final"
                .into(),
        );
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
    fn browser_grocery_input_contains_text_only_and_no_enter_events() {
        for app in ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"] {
            for source in [
                "I need these in order: Cheese pizza, eggs, broccoli, potatoes.",
                "I need these in order:\n• Cheese pizza\n• Eggs\n• Broccoli\n• Potatoes",
                "Cheese pizza\r\neggs\rbroccoli\tpotatoes\u{2028}🛒",
            ] {
                let payload = crate::formatter::layout::for_app(source, Some(app));
                let mut output = Vec::new();
                for step in text_steps(&payload) {
                    assert!(matches!(step, TextStep::Unicode(_)));
                    send_step(&step, |events| {
                        for input in events {
                            let key = unsafe { input.Anonymous.ki };
                            assert_eq!(key.wVk.0, 0, "browser received a shortcut key");
                            assert!(key.dwFlags.contains(KEYEVENTF_UNICODE));
                            assert!(![9, 10, 13, 0x85, 0x2028, 0x2029].contains(&key.wScan));
                            if !key.dwFlags.contains(KEYEVENTF_KEYUP) {
                                output.push(key.wScan);
                            }
                        }
                        events.len() as u32
                    })
                    .unwrap();
                }
                assert_eq!(String::from_utf16(&output).unwrap(), payload);
                for item in ["pizza", "eggs", "broccoli", "potatoes"] {
                    assert!(payload.to_lowercase().contains(item));
                }
            }
        }
    }
    #[test]
    fn newline_plan_preserves_crlf_blank_lines_and_surrogate_pairs() {
        let source = format!(
            "{}🛒\r\n• Cheese pizza\n\n• Eggs\r• Broccoli\n• Potatoes",
            "a".repeat(63)
        );
        let steps = text_steps(&source);
        let mut restored = String::new();
        for step in &steps {
            match step {
                TextStep::Unicode(units) => {
                    assert!(units.len() <= 64);
                    restored.push_str(
                        &String::from_utf16(units).expect("surrogate pair split across batches"),
                    );
                }
                TextStep::LineBreak => restored.push('\n'),
            }
        }
        assert_eq!(restored, source.replace("\r\n", "\n").replace('\r', "\n"));
        assert_eq!(
            steps
                .iter()
                .filter(|step| **step == TextStep::LineBreak)
                .count(),
            5
        );
        assert!(text_steps("").is_empty());
    }

    #[test]
    fn grocery_list_uses_four_shift_enter_chords_without_bare_enter() {
        let source = "I need these in order:\n• Cheese pizza\n• Eggs\n• Broccoli\n• Potatoes";
        let mut shift = false;
        let mut enters = 0;
        let mut output = Vec::new();
        for step in text_steps(source) {
            let events = step_events(&step);
            for input in events {
                let key = unsafe { input.Anonymous.ki };
                assert_eq!(key.dwExtraInfo, 0x5041524c);
                let up = key.dwFlags.contains(KEYEVENTF_KEYUP);
                match key.wVk.0 {
                    0xa0 => shift = !up,
                    0x0d => {
                        assert!(shift, "bare Enter can submit a chat");
                        if !up {
                            enters += 1;
                            output.push(10);
                        }
                    }
                    0 => {
                        assert!(!shift, "Shift must be released before text");
                        assert!(key.dwFlags.contains(KEYEVENTF_UNICODE));
                        assert!(
                            ![10, 13].contains(&key.wScan),
                            "Unicode newlines are ignored by some editors"
                        );
                        if !up {
                            output.push(key.wScan);
                        }
                    }
                    _ => panic!("unexpected shortcut"),
                }
            }
            assert!(!shift, "a line-break chord must fit in one SendInput call");
        }
        assert_eq!(enters, 4);
        assert_eq!(String::from_utf16(&output).unwrap(), source);
    }

    #[test]
    fn partial_line_break_only_releases_keys_never_retries_enter() {
        for accepted in 0..4 {
            let mut calls = Vec::new();
            let result = send_step(&TextStep::LineBreak, |events| {
                calls.push(
                    events
                        .iter()
                        .map(|input| {
                            let k = unsafe { input.Anonymous.ki };
                            (k.wVk.0, k.dwFlags.0)
                        })
                        .collect::<Vec<_>>(),
                );
                if calls.len() == 1 {
                    accepted
                } else {
                    events.len() as u32
                }
            });
            assert!(result.is_err());
            if accepted == 0 {
                assert_eq!(calls.len(), 1);
            } else {
                assert_eq!(calls.len(), 2);
                assert!(calls[1]
                    .iter()
                    .all(|(_, flags)| *flags == KEYEVENTF_KEYUP.0));
                assert_eq!(calls[1].last().unwrap().0, 0xa0);
                assert_eq!(
                    calls[1].iter().filter(|(vk, _)| *vk == 0x0d).count(),
                    usize::from(accepted == 2)
                );
            }
        }
    }

    #[test]
    fn failed_text_or_release_is_reported_without_content_retry() {
        let mut calls = 0;
        assert!(send_step(&TextStep::Unicode(vec![65, 66]), |_| {
            calls += 1;
            1
        })
        .is_err());
        assert_eq!(calls, 1);
        calls = 0;
        let error = send_step(&TextStep::LineBreak, |_| {
            calls += 1;
            if calls == 1 {
                2
            } else {
                0
            }
        })
        .unwrap_err();
        assert!(error.contains("release was not confirmed"));
        assert_eq!(calls, 2);
    }

    #[test]
    fn receipt_requires_exact_suffix_and_same_field() {
        let before = TargetSnapshot {
            hwnd: 1,
            focus: 2,
            input_epoch: None,
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
