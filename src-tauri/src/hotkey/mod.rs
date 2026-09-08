// HotkeyManager trait + trigger state machine (idle -> recording -> finalizing).
// Windows implementation notes (WH_KEYBOARD_LL):
//   - Hook callback MUST return fast (< LowLevelHooksTimeout ~300 ms) or Windows
//     silently drops the hook. Bounce all real work to another thread/channel.
//   - RegisterHotKey is fire-on-press only: unusable for hold-to-talk.
//   - Default PTT chord per spec: Ctrl+Win. Never consume plain modifier taps
//     (breaks Ctrl+C/V in the foreground app).
pub mod windows;

pub enum TriggerEvent {
    PttStart,
    PttEnd,
    Toggle,
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

pub struct WindowsLlHook {
    pub(crate) installed: bool,
}

impl WindowsLlHook {
    pub fn new() -> Self {
        Self { installed: false }
    }
}

impl Default for WindowsLlHook {
    fn default() -> Self {
        Self::new()
    }
}
