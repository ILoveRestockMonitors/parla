//! Bounded field inspection. One COM worker; a stuck provider never creates
//! another worker. Plain data snapshots can safely cross inference threads.
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationTextRange,
    TextPatternRangeEndpoint_End as End, TextPatternRangeEndpoint_Start as Start,
    TextUnit_Character, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldContext {
    pub text_before: Option<String>,
    pub selected_text: Option<String>,
    pub text_after: Option<String>,
    pub runtime_id: Option<Vec<i32>>,
    pub password: bool,
    pub available: bool,
    pub inspection_failed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub hwnd: isize,
    pub focus: isize,
    pub input_epoch: Option<u64>,
    pub context: FieldContext,
}
impl TargetSnapshot {
    pub fn capture(timeout_ms: u64) -> Self {
        let (hwnd, focus) = native_focus();
        let input_epoch = crate::hotkey::windows::input_epoch();
        let mut context = probe(timeout_ms);
        // A cold/busy accessibility provider may miss the first deadline.
        // Retry observation only; never replace the captured native target.
        if context.inspection_failed && native_focus() == (hwnd, focus) {
            context = probe(timeout_ms);
        }
        if native_focus() != (hwnd, focus) {
            return Self {
                hwnd: 0,
                focus: 0,
                input_epoch,
                context: FieldContext::default(),
            };
        }
        Self {
            hwnd,
            focus,
            input_epoch,
            context,
        }
    }
    pub fn same_field(&self, other: &Self) -> bool {
        self.hwnd != 0
            && self.hwnd == other.hwnd
            && self.focus == other.focus
            && !self.context.password
            && !other.context.password
            && match (&self.context.runtime_id, &other.context.runtime_id) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            }
    }
    pub fn unchanged(&self, other: &Self) -> bool {
        self.same_field(other)
            && !self.context.inspection_failed
            && !other.context.inspection_failed
            && self.context.available == other.context.available
            && (!self.context.available
                || (self.context.text_before == other.context.text_before
                    && self.context.selected_text == other.context.selected_text
                    && self.context.text_after == other.context.text_after))
    }
    pub fn matches_current(&self, timeout_ms: u64) -> bool {
        self.unchanged(&Self::capture(timeout_ms))
    }
    pub fn insertion_mismatch(&self, other: &Self) -> Option<&'static str> {
        if self.hwnd == 0 || self.hwnd != other.hwnd || self.focus != other.focus {
            return Some("target window or control changed");
        }
        if self.context.password || other.context.password {
            return Some("password field cannot receive dictation");
        }
        if self.context.inspection_failed || other.context.inspection_failed {
            return Some("field inspection timed out or failed");
        }
        match (&self.context.runtime_id, &other.context.runtime_id) {
            (Some(a), Some(b)) if a != b => return Some("target field changed"),
            (Some(_), None) | (None, Some(_)) => {
                return Some("field identity temporarily unavailable")
            }
            _ => {}
        }
        if self.context.available != other.context.available {
            return Some("caret inspection temporarily unavailable");
        }
        if !self.unchanged(other) {
            return Some("caret, selection, or surrounding text changed");
        }
        None
    }
    pub fn verify_for_insertion(&self, timeout_ms: u64) -> Result<(), String> {
        self.verify_observations(|| Self::capture(timeout_ms), || self.native_still_focused())
    }
    fn verify_observations(
        &self,
        mut observe: impl FnMut() -> Self,
        still_focused: impl Fn() -> bool,
    ) -> Result<(), String> {
        for attempt in 0..2 {
            let current = observe();
            let Some(reason) = self.insertion_mismatch(&current) else {
                return Ok(());
            };
            let transient = reason == "field inspection timed out or failed"
                || reason == "field identity temporarily unavailable"
                || reason == "caret inspection temporarily unavailable";
            if transient && self.native_fallback_matches(&current) && still_focused() {
                return Ok(());
            }
            if attempt == 0 && transient && !self.context.inspection_failed && still_focused() {
                continue;
            }
            return Err(format!(
                "{reason}; text saved for recovery — use Copy final"
            ));
        }
        unreachable!()
    }
    pub fn native_still_focused(&self) -> bool {
        native_focus() == (self.hwnd, self.focus)
    }
    fn native_fallback_matches(&self, other: &Self) -> bool {
        self.hwnd != 0
            && self.focus != 0
            && self.hwnd == other.hwnd
            && self.focus == other.focus
            && !self.context.password
            && !other.context.password
            && matches!((self.input_epoch, other.input_epoch), (Some(a), Some(b)) if a == b)
            && match (&self.context.runtime_id, &other.context.runtime_id) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
    pub fn input_still_unchanged(&self) -> bool {
        match self.input_epoch {
            Some(epoch) => crate::hotkey::windows::input_epoch() == Some(epoch),
            None => true, // No fallback was authorized without the monitor.
        }
    }
}
fn native_focus() -> (isize, isize) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let tid = GetWindowThreadProcessId(hwnd, None);
        let focus = if GetGUIThreadInfo(tid, &mut info).is_ok() {
            info.hwndFocus.0 as isize
        } else {
            0
        };
        (hwnd.0 as isize, focus)
    }
}
enum Request {
    Probe {
        deadline: Instant,
        reply: mpsc::SyncSender<FieldContext>,
    },
    Select {
        deadline: Instant,
        target: TargetSnapshot,
        text: String,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
}
fn worker() -> &'static mpsc::SyncSender<Request> {
    static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(1);
        let _ = std::thread::Builder::new()
            .name("parla-uia".into())
            .spawn(move || unsafe {
                if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                    return;
                }
                let Ok(automation) = CoCreateInstance::<_, IUIAutomation>(
                    &CUIAutomation,
                    None,
                    CLSCTX_INPROC_SERVER,
                ) else {
                    return;
                };
                while let Ok(request) = rx.recv() {
                    match request {
                        Request::Probe { deadline, reply } if Instant::now() < deadline => {
                            let _ = reply.try_send(
                                probe_inner(&automation).unwrap_or_else(|_| failed_probe()),
                            );
                        }
                        Request::Select {
                            deadline,
                            target,
                            text,
                            reply,
                        } if Instant::now() < deadline => {
                            let _ =
                                reply.try_send(select_inner(&automation, &target, &text, deadline));
                        }
                        _ => {}
                    }
                }
            });
        tx
    })
}
fn probe(timeout_ms: u64) -> FieldContext {
    let timeout = Duration::from_millis(timeout_ms.min(500));
    let (tx, rx) = mpsc::sync_channel(1);
    if worker()
        .try_send(Request::Probe {
            deadline: Instant::now() + timeout,
            reply: tx,
        })
        .is_err()
    {
        return failed_probe();
    }
    rx.recv_timeout(timeout).unwrap_or_else(|_| failed_probe())
}
fn failed_probe() -> FieldContext {
    FieldContext {
        inspection_failed: true,
        ..Default::default()
    }
}
unsafe fn probe_inner(automation: &IUIAutomation) -> windows::core::Result<FieldContext> {
    let focused = automation.GetFocusedElement()?;
    let mut result = FieldContext::default();
    result.password = focused.CurrentIsPassword()?.as_bool();
    if result.password {
        return Ok(result);
    }
    if let Ok(array) = focused.GetRuntimeId() {
        let id = (|| -> windows::core::Result<Vec<i32>> {
            let low = SafeArrayGetLBound(array, 1)?;
            let high = SafeArrayGetUBound(array, 1)?;
            let mut id = Vec::new();
            if i64::from(high) - i64::from(low) > 64 {
                return Ok(id);
            }
            for index in low..=high {
                let mut value = 0i32;
                SafeArrayGetElement(array, &index, (&mut value as *mut i32).cast())?;
                id.push(value);
            }
            Ok(id)
        })();
        let _ = SafeArrayDestroy(array);
        result.runtime_id = id.ok().filter(|v| !v.is_empty());
    }
    let Ok(pattern) = focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    else {
        return Ok(result);
    };
    let selection = pattern.GetSelection()?;
    if selection.Length()? != 1 {
        return Ok(result);
    }
    let range = selection.GetElement(0)?;
    // Chromium ranges can move outside a contenteditable even when the text
    // pattern belongs to that editor. Compare only the editor's own text:
    // changing messages, attachment labels and typing indicators are not edits.
    let document = pattern.DocumentRange()?;
    if !range_within(&range, &document)? {
        return Ok(failed_probe());
    }
    let selected = range.GetText(65537)?.to_string();
    if selected.len() > 65536 {
        return Ok(result);
    }
    result.selected_text = Some(selected);
    let before = range.Clone()?;
    before.MoveEndpointByRange(End, &range, Start)?;
    before.MoveEndpointByUnit(Start, TextUnit_Character, -512)?;
    clamp_to_document(&before, &document)?;
    result.text_before = Some(before.GetText(2048)?.to_string());
    let after = range.Clone()?;
    after.MoveEndpointByRange(Start, &range, End)?;
    after.MoveEndpointByUnit(End, TextUnit_Character, 160)?;
    clamp_to_document(&after, &document)?;
    result.text_after = Some(after.GetText(640)?.to_string());
    result.available = true;
    Ok(result)
}
unsafe fn range_within(
    range: &IUIAutomationTextRange,
    document: &IUIAutomationTextRange,
) -> windows::core::Result<bool> {
    Ok(range.CompareEndpoints(Start, document, Start)? >= 0
        && range.CompareEndpoints(End, document, End)? <= 0)
}
unsafe fn clamp_to_document(
    range: &IUIAutomationTextRange,
    document: &IUIAutomationTextRange,
) -> windows::core::Result<()> {
    if range.CompareEndpoints(Start, document, Start)? < 0 {
        range.MoveEndpointByRange(Start, document, Start)?;
    }
    if range.CompareEndpoints(End, document, End)? > 0 {
        range.MoveEndpointByRange(End, document, End)?;
    }
    Ok(())
}
pub fn select_inserted_text(target: &TargetSnapshot, text: &str) -> Result<(), String> {
    let timeout = Duration::from_millis(400);
    let (tx, rx) = mpsc::sync_channel(1);
    worker()
        .try_send(Request::Select {
            deadline: Instant::now() + timeout,
            target: target.clone(),
            text: text.into(),
            reply: tx,
        })
        .map_err(|_| "field inspection busy".to_string())?;
    rx.recv_timeout(timeout)
        .map_err(|_| "field inspection timed out; command not confirmed".to_string())?
}
unsafe fn select_inner(
    automation: &IUIAutomation,
    target: &TargetSnapshot,
    text: &str,
    deadline: Instant,
) -> Result<(), String> {
    if !target.context.available || text.is_empty() || !target.native_still_focused() {
        return Err("cannot verify previous insertion".into());
    }
    let context = probe_inner(automation).map_err(|e| e.to_string())?;
    let current = TargetSnapshot {
        hwnd: target.hwnd,
        focus: target.focus,
        input_epoch: crate::hotkey::windows::input_epoch(),
        context,
    };
    if !target.unchanged(&current) {
        return Err("field changed since insertion".into());
    }
    let focused = automation.GetFocusedElement().map_err(|e| e.to_string())?;
    let pattern: IUIAutomationTextPattern = focused
        .GetCurrentPatternAs(UIA_TextPatternId)
        .map_err(|e| e.to_string())?;
    let range = pattern
        .GetSelection()
        .and_then(|a| a.GetElement(0))
        .map_err(|e| e.to_string())?;
    let document = pattern.DocumentRange().map_err(|e| e.to_string())?;
    if !range_within(&range, &document).map_err(|e| e.to_string())? {
        return Err("selection is outside the original editor".into());
    }
    if !range.GetText(1).map_err(|e| e.to_string())?.is_empty() {
        return Err("selection changed".into());
    }
    // Providers differ in whether supplementary Unicode characters consume
    // one or two movement units. Neither count authorizes a mutation: the
    // candidate range must contain exactly the original payload.
    for count in [text.chars().count(), text.encode_utf16().count()] {
        if Instant::now() >= deadline || !target.native_still_focused() {
            return Err("field selection expired".into());
        }
        let trial = range.Clone().map_err(|e| e.to_string())?;
        let count = i32::try_from(count).map_err(|_| "insertion too long")?;
        trial
            .MoveEndpointByUnit(Start, TextUnit_Character, -count)
            .map_err(|e| e.to_string())?;
        if !range_within(&trial, &document).map_err(|e| e.to_string())? {
            continue;
        }
        if trial.GetText(-1).map_err(|e| e.to_string())?.to_string() != text {
            continue;
        }
        if Instant::now() >= deadline || !target.native_still_focused() {
            return Err("field selection expired".into());
        }
        return trial.Select().map_err(|e| e.to_string());
    }
    Err("inserted range no longer matches".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> TargetSnapshot {
        TargetSnapshot {
            hwnd: 1,
            focus: 2,
            input_epoch: None,
            context: FieldContext {
                runtime_id: Some(vec![42]),
                available: true,
                text_before: Some("hello ".into()),
                selected_text: Some(String::new()),
                text_after: Some(String::new()),
                ..Default::default()
            },
        }
    }
    #[test]
    fn same_window_different_control_or_caret_refused() {
        let a = target();
        let mut b = a.clone();
        b.context.runtime_id = Some(vec![43]);
        assert!(!a.unchanged(&b));
        b = a.clone();
        b.context.text_before = Some("manual edit".into());
        assert!(!a.unchanged(&b));
        b = a.clone();
        b.context.password = true;
        assert!(!a.unchanged(&b));
        b = a.clone();
        b.context.available = false;
        assert!(!a.unchanged(&b));
        assert!(a.unchanged(&a));
    }
    #[test]
    fn transient_probe_recovers_without_changing_target() {
        let a = target();
        let mut missing = a.clone();
        missing.context = failed_probe();
        let mut observations = vec![missing, a.clone()].into_iter();
        assert!(a
            .verify_observations(|| observations.next().unwrap(), || true)
            .is_ok());
        assert!(observations.next().is_none());
    }
    #[test]
    fn inspection_failure_can_type_with_stable_native_target_and_input_guard() {
        let mut a = target();
        a.input_epoch = Some(7);
        let mut b = a.clone();
        b.context = failed_probe();
        assert!(a.verify_observations(|| b.clone(), || true).is_ok());
        // The initial inspection may fail too: native focus and input continuity
        // are independent of the accessibility worker.
        a.context = failed_probe();
        assert!(a.verify_observations(|| b.clone(), || true).is_ok());
        b.context = target().context;
        assert!(a.verify_observations(|| b.clone(), || true).is_ok());
    }
    #[test]
    fn native_fallback_refuses_activity_focus_password_or_unknown_monitor() {
        let mut a = target();
        a.input_epoch = Some(7);
        let mut b = a.clone();
        b.context = failed_probe();
        b.input_epoch = Some(8);
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
        b.input_epoch = None;
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
        b.input_epoch = Some(7);
        b.focus = 3;
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
        b.focus = a.focus;
        b.hwnd = 3;
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
        b.hwnd = a.hwnd;
        b.context.password = true;
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
        b.context.password = false;
        assert!(a.verify_observations(|| b.clone(), || false).is_err());
        b.context = target().context;
        b.context.text_before = Some("changed".into());
        assert!(a.verify_observations(|| b.clone(), || true).is_err());
    }
    #[test]
    fn intermittent_caret_availability_recovers() {
        let a = target();
        let mut missing = a.clone();
        missing.context.available = false;
        let mut observations = vec![missing, a.clone()].into_iter();
        assert!(a
            .verify_observations(|| observations.next().unwrap(), || true)
            .is_ok());
    }
    #[test]
    fn real_changes_never_retry_into_a_new_target() {
        let a = target();
        let mut changed = a.clone();
        changed.context.text_before = Some("edited".into());
        let mut observations = vec![changed, a.clone()].into_iter();
        assert!(a
            .verify_observations(|| observations.next().unwrap(), || true)
            .unwrap_err()
            .contains("surrounding text changed"));
        assert!(observations.next().is_some());
        let mut moved = a.clone();
        moved.focus += 1;
        moved.context = failed_probe();
        let mut observations = vec![moved, a.clone()].into_iter();
        assert!(a
            .verify_observations(|| observations.next().unwrap(), || true)
            .is_err());
        assert!(observations.next().is_some());
    }
    #[test]
    fn persistent_failure_and_missing_start_snapshot_remain_recoverable() {
        let a = target();
        let mut failed = a.clone();
        failed.context = failed_probe();
        let mut calls = 0;
        assert!(a
            .verify_observations(
                || {
                    calls += 1;
                    failed.clone()
                },
                || true
            )
            .is_err());
        assert_eq!(calls, 2);
        assert!(!failed.unchanged(&failed));
        assert!(failed.verify_observations(|| a.clone(), || true).is_err());
    }
    #[test]
    #[ignore = "reads the foreground application's accessibility provider; run manually"]
    fn live_field_inspection_diagnostic() {
        let first = TargetSnapshot::capture(120);
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(50));
            let next = TargetSnapshot::capture(150);
            // Never log field contents, window titles, or runtime identifiers.
            eprintln!("field check: {:?}; initial available={}, failed={}; current available={}, failed={}",
                first.insertion_mismatch(&next), first.context.available, first.context.inspection_failed,
                next.context.available, next.context.inspection_failed);
        }
    }
    #[test]
    #[ignore = "read-only live editor range regression; set PARLA_TEST_HWND to a verified window handle"]
    fn live_editor_ranges_stay_inside_field() {
        unsafe {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::Accessibility::{
                TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_EditControlTypeId,
            };
            let hwnd: isize = std::env::var("PARLA_TEST_HWND").unwrap().parse().unwrap();
            CoInitializeEx(None, COINIT_MULTITHREADED).unwrap();
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
            let root = automation.ElementFromHandle(HWND(hwnd)).unwrap();
            let condition = automation
                .CreatePropertyCondition(
                    UIA_ControlTypePropertyId,
                    &windows::core::VARIANT::from(UIA_EditControlTypeId.0),
                )
                .unwrap();
            let edits = root.FindAll(TreeScope_Descendants, &condition).unwrap();
            assert_eq!(
                edits.Length().unwrap(),
                1,
                "select a window with exactly one editable control"
            );
            let edit = edits.GetElement(0).unwrap();
            assert!(!edit.CurrentIsPassword().unwrap().as_bool());
            let pattern: IUIAutomationTextPattern =
                edit.GetCurrentPatternAs(UIA_TextPatternId).unwrap();
            let document = pattern.DocumentRange().unwrap();
            let selection = pattern.GetSelection().unwrap();
            assert_eq!(selection.Length().unwrap(), 1);
            let caret = selection.GetElement(0).unwrap();
            assert!(range_within(&caret, &document).unwrap());
            let before = caret.Clone().unwrap();
            before.MoveEndpointByRange(End, &caret, Start).unwrap();
            before
                .MoveEndpointByUnit(Start, TextUnit_Character, -512)
                .unwrap();
            let after = caret.Clone().unwrap();
            after.MoveEndpointByRange(Start, &caret, End).unwrap();
            after
                .MoveEndpointByUnit(End, TextUnit_Character, 160)
                .unwrap();
            let escaped = !range_within(&before, &document).unwrap()
                || !range_within(&after, &document).unwrap();
            eprintln!("Old ranges escaped field: {escaped}");
            if std::env::var_os("PARLA_EXPECT_ESCAPE").is_some() {
                assert!(escaped, "fixture did not reproduce the original bug");
            }
            clamp_to_document(&before, &document).unwrap();
            clamp_to_document(&after, &document).unwrap();
            assert!(range_within(&before, &document).unwrap());
            assert!(range_within(&after, &document).unwrap());
            assert_eq!(before.CompareEndpoints(End, &caret, Start).unwrap(), 0);
            assert_eq!(after.CompareEndpoints(Start, &caret, End).unwrap(), 0);
            // Reconstruct the field for this short-draft regression. This tests
            // no draft text was dropped and no surrounding page text was read.
            let whole = document.GetText(4096).unwrap().to_string();
            assert!(
                whole.chars().count() < 160,
                "use a short draft for this test"
            );
            let combined = format!(
                "{}{}{}",
                before.GetText(4096).unwrap(),
                caret.GetText(4096).unwrap(),
                after.GetText(4096).unwrap()
            );
            assert!(
                combined == whole,
                "bounded context did not reconstruct the draft"
            );
            eprintln!("Bounded ranges retain the complete draft and preserve caret endpoints.");
        }
    }
}
