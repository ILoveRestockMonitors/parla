//! Portable capture uses native field/window identity plus an input epoch.
//! Text is deliberately not read and replacement is unavailable until exact
//! accessible range checks exist on the platform. One bounded probe worker.
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldContext {
    pub text_before: Option<String>,
    pub selected_text: Option<String>,
    pub text_after: Option<String>,
    pub runtime_id: Option<Vec<i32>>,
    pub password: bool,
    pub available: bool,
    pub inspection_failed: bool,
    pub terminal: bool,
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
        let input_epoch = crate::hotkey::input_epoch();
        let timeout = Duration::from_millis(timeout_ms.clamp(1, 500));
        let (tx, rx) = mpsc::sync_channel(1);
        let result = if worker().try_send((Instant::now() + timeout, tx)).is_ok() {
            rx.recv_timeout(timeout).ok()
        } else {
            None
        };
        let mut result = result.unwrap_or_else(Self::unavailable);
        result.input_epoch = input_epoch;
        // Activity during inspection invalidates the whole observation.
        if input_epoch != crate::hotkey::input_epoch() {
            result.context.inspection_failed = true;
        }
        result
    }
    pub(crate) fn unavailable() -> Self {
        Self {
            hwnd: 0,
            focus: 0,
            input_epoch: None,
            context: FieldContext {
                inspection_failed: true,
                ..Default::default()
            },
        }
    }
    pub fn same_field(&self, other: &Self) -> bool {
        self.hwnd != 0
            && self.focus != 0
            && self.hwnd == other.hwnd
            && self.focus == other.focus
            && !self.context.password
            && !other.context.password
            && !self.context.inspection_failed
            && !other.context.inspection_failed
            && self.context.runtime_id.is_some()
            && self.context.runtime_id == other.context.runtime_id
    }
    pub fn unchanged(&self, other: &Self) -> bool {
        self.same_field(other)
            && matches!((self.input_epoch, other.input_epoch), (Some(a), Some(b)) if a == b)
    }
    pub fn matches_current(&self, timeout_ms: u64) -> bool {
        self.unchanged(&Self::capture(timeout_ms))
    }
    pub fn verify_for_insertion(&self, timeout_ms: u64) -> Result<(), String> {
        if self.matches_current(timeout_ms) {
            Ok(())
        } else {
            Err("Destination changed, field inspection unavailable, or intervening input detected; use Copy final and paste manually.".into())
        }
    }
    pub fn native_still_focused(&self) -> bool {
        self.same_field(&Self::capture(100))
    }
    pub fn input_still_unchanged(&self) -> bool {
        matches!(self.input_epoch, Some(epoch) if crate::hotkey::input_epoch() == Some(epoch))
    }
}
type Request = (Instant, mpsc::SyncSender<TargetSnapshot>);
fn worker() -> &'static mpsc::SyncSender<Request> {
    static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Request>(1);
        let _ = std::thread::Builder::new()
            .name("parla-focus".into())
            .spawn(move || {
                while let Ok((deadline, reply)) = rx.recv() {
                    if Instant::now() < deadline {
                        let _ = reply.try_send(super::native::capture());
                    }
                }
            });
        tx
    })
}
pub fn select_inserted_text(_: &TargetSnapshot, _: &str) -> Result<(), String> {
    Err("Verified selection replacement is not implemented on this platform; use Copy raw.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> TargetSnapshot {
        TargetSnapshot {
            hwnd: 11,
            focus: 12,
            input_epoch: Some(3),
            context: FieldContext {
                runtime_id: Some(vec![11, 12]),
                ..Default::default()
            },
        }
    }
    #[test]
    fn native_guard_refuses_unknown_password_activity_or_changed_focus() {
        let a = fixture();
        assert!(a.unchanged(&a));
        let mutations: &[fn(&mut TargetSnapshot)] = &[
            |b: &mut TargetSnapshot| b.hwnd = 0,
            |b: &mut TargetSnapshot| b.focus += 1,
            |b: &mut TargetSnapshot| b.input_epoch = None,
            |b: &mut TargetSnapshot| b.input_epoch = Some(4),
            |b: &mut TargetSnapshot| b.context.password = true,
            |b: &mut TargetSnapshot| b.context.inspection_failed = true,
            |b: &mut TargetSnapshot| b.context.runtime_id = None,
        ];
        for mutate in mutations {
            let mut b = a.clone();
            mutate(&mut b);
            assert!(!a.unchanged(&b));
        }
        assert!(!TargetSnapshot::unavailable().unchanged(&TargetSnapshot::unavailable()));
    }
}
