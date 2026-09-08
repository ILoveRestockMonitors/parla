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
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, TextPatternRangeEndpoint_End as End,
    TextPatternRangeEndpoint_Start as Start, TextUnit_Character, UIA_TextPatternId,
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
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub hwnd: isize,
    pub focus: isize,
    pub context: FieldContext,
}
impl TargetSnapshot {
    pub fn capture(timeout_ms: u64) -> Self {
        let (hwnd, focus) = native_focus();
        let context = probe(timeout_ms);
        if native_focus() != (hwnd, focus) {
            return Self {
                hwnd: 0,
                focus: 0,
                context: FieldContext::default(),
            };
        }
        Self {
            hwnd,
            focus,
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
            && self.context.available == other.context.available
            && (!self.context.available
                || (self.context.text_before == other.context.text_before
                    && self.context.selected_text == other.context.selected_text
                    && self.context.text_after == other.context.text_after))
    }
    pub fn matches_current(&self, timeout_ms: u64) -> bool {
        self.unchanged(&Self::capture(timeout_ms))
    }
    pub fn native_still_focused(&self) -> bool {
        native_focus() == (self.hwnd, self.focus)
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
                            let _ = reply.try_send(probe_inner(&automation).unwrap_or_default());
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
        return FieldContext::default();
    }
    rx.recv_timeout(timeout).unwrap_or_default()
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
    let selected = range.GetText(65537)?.to_string();
    if selected.len() > 65536 {
        return Ok(result);
    }
    result.selected_text = Some(selected);
    let before = range.Clone()?;
    before.MoveEndpointByRange(End, &range, Start)?;
    before.MoveEndpointByUnit(Start, TextUnit_Character, -512)?;
    result.text_before = Some(before.GetText(2048)?.to_string());
    let after = range.Clone()?;
    after.MoveEndpointByRange(Start, &range, End)?;
    after.MoveEndpointByUnit(End, TextUnit_Character, 160)?;
    result.text_after = Some(after.GetText(640)?.to_string());
    result.available = true;
    Ok(result)
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
}
