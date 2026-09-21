// HotkeyManager trait + trigger state machine (idle -> recording -> finalizing).
// Windows implementation notes (WH_KEYBOARD_LL):
//   - Hook callback MUST return fast (< LowLevelHooksTimeout ~300 ms) or Windows
//     silently drops the hook. Bounce all real work to another thread/channel.
//   - RegisterHotKey is fire-on-press only: unusable for hold-to-talk.
//   - Default PTT chord per spec: Ctrl+Win. Never consume plain modifier taps
//     (breaks Ctrl+C/V in the foreground app).
#[cfg(not(windows))]
mod portable;
#[cfg(windows)]
pub mod windows;
#[cfg(not(windows))]
pub use portable::modifiers_released;
#[cfg(not(windows))]
pub use portable::{input_epoch, poll_timed_events, set_hotkeys};
#[cfg(windows)]
pub use windows::{input_epoch, poll_timed_events, set_hotkeys};

/// Explicit controls work even when the OS does not expose a global hook.
pub fn request_toggle() -> bool {
    #[cfg(windows)]
    windows::push(TriggerEvent::ManualToggle);
    #[cfg(not(windows))]
    portable::push(TriggerEvent::ManualToggle);
    true
}

pub fn spawn_listener() {
    std::thread::spawn(|| {
        #[cfg(windows)]
        {
            let mut hook = WindowsLlHook::new();
            if let Err(e) = hook.start() {
                crate::runtime::update(|s| {
                    s.error = Some(format!("Keyboard shortcut unavailable: {e}"))
                });
                return;
            }
            crate::runtime::update(|s| s.hook_ready = true);
            unsafe {
                use ::windows::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
                };
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            hook.stop();
        }
        #[cfg(not(windows))]
        if let Err(e) = portable::listen() {
            crate::runtime::update(|s| {
                s.error = Some(format!("Keyboard shortcut unavailable: {e}"))
            });
        }
        crate::runtime::update(|s| s.hook_ready = false);
    });
}

pub enum TriggerEvent {
    PttStart,
    PttEnd,
    Toggle,
    ManualToggle,
    Cancel,
}

impl PartialEq for TriggerEvent {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl std::fmt::Debug for TriggerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TriggerEvent::PttStart => "PttStart",
            TriggerEvent::PttEnd => "PttEnd",
            TriggerEvent::Toggle => "Toggle",
            TriggerEvent::ManualToggle => "ManualToggle",
            TriggerEvent::Cancel => "Cancel",
        };
        write!(f, "{name}")
    }
}

pub enum TriggerState {
    Idle,
    Recording,
    Finalizing,
}

pub trait HotkeyManager: Send {
    /// Install hooks and begin emitting events.
    fn start(&mut self) -> Result<(), String>;
    /// Remove hooks.
    fn stop(&mut self);
    /// Drain queued events (non-blocking). Called from the pipeline loop.
    fn take_events(&mut self) -> Vec<TriggerEvent>;
}

#[cfg(windows)]
pub struct WindowsLlHook {
    pub(crate) installed: bool,
}

#[cfg(windows)]
impl WindowsLlHook {
    pub fn new() -> Self {
        Self { installed: false }
    }
}

#[cfg(windows)]
impl Default for WindowsLlHook {
    fn default() -> Self {
        Self::new()
    }
}
