// WH_KEYBOARD_LL hold-to-talk on CONFIGURABLE chord(s).
//
// Contract (plan R8 + Phase-0 findings):
// - The hook PROCEDURE only applies a tiny pure state machine, pushes events,
//   and returns immediately (<LowLevelHooksTimeout ~300 ms); all pipeline work
//   happens on the pipeline thread via poll_events().
// - Chords come from Settings.ptt_chords (parse via store::settings), injected
//   through set_chords() BEFORE the hook installs. Until then (and whenever
//   unset/unlocked) the built-in default ["ctrl+win"] applies, so behavior
//   never regresses to dead.
// - Suppression rule (Start-menu / shortcut safety): a key event is swallowed
//   ONLY if that virtual key participates in a configured chord's trigger
//   group AND the chord's remaining required groups are already held. Plain
//   taps of modifiers outside every configured chord pass through untouched,
//   so Ctrl+C/V and bare Win keep working.
// - Known v1 limitation, unchanged: trigger-key-first ordering (e.g. pressing
//   Win before Ctrl on the default chord) lets the Start menu open before
//   suppression arms; documented, UX guidance handled elsewhere.
use super::{HotkeyManager, TriggerEvent, WindowsLlHook};
use crate::store::settings::{default_chord, HotkeyChord};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT, WH_KEYBOARD_LL};

static EVENTS: OnceLock<Mutex<Vec<(TriggerEvent, Instant)>>> = OnceLock::new();
static HOOK: Mutex<usize> = Mutex::new(0);
static STATE: Mutex<Option<HookState>> = Mutex::new(None);

struct HookState {
    chords: Vec<HotkeyChord>,
    toggle: HotkeyChord,
    held: HashSet<u32>,
    active_chord: Option<usize>,
    toggle_consumed_vk: Option<u32>,
    consumed_hold_vks: HashSet<u32>,
}

fn events_cell() -> &'static Mutex<Vec<(TriggerEvent, Instant)>> {
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Inject the configured chords. Called once at startup, before the hook
/// thread spawns. Safe no-op if the state mutex is poisoned (hook then runs
/// on the built-in default chord).
pub fn set_chords(chords: Vec<HotkeyChord>) {
    if let Ok(mut guard) = STATE.lock() {
        *guard = Some(HookState {
            chords,
            toggle: crate::store::settings::Settings::default().toggle_hotkey_chord(),
            held: HashSet::new(),
            active_chord: None,
            toggle_consumed_vk: None,
            consumed_hold_vks: HashSet::new(),
        });
    }
}
pub fn set_hotkeys(chords: Vec<HotkeyChord>, toggle: HotkeyChord) {
    if let Ok(mut guard) = STATE.lock() {
        *guard = Some(HookState {
            chords,
            toggle,
            held: HashSet::new(),
            active_chord: None,
            toggle_consumed_vk: None,
            consumed_hold_vks: HashSet::new(),
        });
    }
}

/// Pure per-event transition. Returns (events to enqueue, swallow-this-key).
/// Kept free of WinAPI so it unit-tests without a keyboard.
fn on_key(state: &mut HookState, vk: u32, down: bool) -> (Vec<TriggerEvent>, bool) {
    on_key_flags(state, vk, down, false)
}

fn on_key_flags(
    state: &mut HookState,
    vk: u32,
    down: bool,
    injected: bool,
) -> (Vec<TriggerEvent>, bool) {
    let mut evs = Vec::new();
    let mut swallow_toggle_release = false;
    let mut swallow_hold_release = false;
    if injected {
        return (evs, false);
    }
    if down {
        if state.held.insert(vk) {
            if chord_held(&state.toggle, &state.held)
                && state.toggle.sets.last().map_or(false, |g| g.contains(&vk))
                && no_extra_modifiers(&state.toggle, &state.held)
            {
                state.toggle_consumed_vk = Some(vk);
                evs.push(TriggerEvent::Toggle);
            } else if state.active_chord.is_none() {
                if let Some(i) = state.chords.iter().position(|c| {
                    chord_held(c, &state.held)
                        && c.sets.last().is_some_and(|g| g.contains(&vk))
                        && chord_other_groups_held(c, &state.held, vk)
                }) {
                    state.active_chord = Some(i);
                    state.consumed_hold_vks.insert(vk);
                    evs.push(TriggerEvent::PttStart);
                }
            }
        }
    } else {
        let was_held = state.held.remove(&vk);
        if was_held {
            let swallow_hold = state.consumed_hold_vks.contains(&vk);
            swallow_hold_release = swallow_hold;
            swallow_toggle_release = state.toggle_consumed_vk == Some(vk);
            if swallow_toggle_release {
                state.toggle_consumed_vk = None;
            }
            if let Some(i) = state.active_chord {
                let c = &state.chords[i];
                if c.sets.iter().any(|g| g.contains(&vk)) && !chord_held(c, &state.held) {
                    state.active_chord = None;
                    evs.push(TriggerEvent::PttEnd);
                }
            }
            if swallow_hold {
                state.consumed_hold_vks.remove(&vk);
            }
        }
    }

    // Swallow policy: suppress ONLY keys that are the trigger of a now-or-
    // previously-satisfied chord (Win-in-Chord => no Start menu; rctrl-only
    // => no context menu) — never bystander keys.
    let swallow = swallow_toggle_release
        || swallow_hold_release
        || state.consumed_hold_vks.contains(&vk)
        || state.toggle_consumed_vk == Some(vk);
    (evs, swallow)
}

/// Every declared group of the chord has at least one representative key held.
fn chord_held(c: &HotkeyChord, held: &HashSet<u32>) -> bool {
    c.sets.iter().all(|g| g.iter().any(|v| held.contains(v)))
}

fn no_extra_modifiers(chord: &HotkeyChord, held: &HashSet<u32>) -> bool {
    const MODIFIERS: &[u32] = &[
        0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
    ];
    held.iter()
        .filter(|key| MODIFIERS.contains(key))
        .all(|key| chord.sets.iter().any(|group| group.contains(key)))
}

/// All groups EXCEPT the one containing `exclude` are satisfied. Used to
/// decide whether suppressing the just-pressed trigger key is warranted.
fn chord_other_groups_held(c: &HotkeyChord, held: &HashSet<u32>, exclude: u32) -> bool {
    c.sets
        .iter()
        .all(|g| g.contains(&exclude) || g.iter().any(|v| held.contains(v)))
}

/// The LL hook itself: state-machine step + queue push, immediate return.
unsafe extern "system" fn ll_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const HC_ACTION: i32 = 0;
    if code >= HC_ACTION {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;
        // WM_KEYDOWN=0x0100 WM_KEYDOWN=syschar=0x0104; *_UP likewise 0x0101/0x0105
        let down = matches!(wparam.0, 0x0100 | 0x0104);
        let up = matches!(wparam.0, 0x0101 | 0x0105);

        if down || up {
            let mut forward_with_default = true;
            if let Ok(mut guard) = STATE.lock() {
                let st = guard.get_or_insert_with(|| HookState {
                    chords: vec![default_chord()],
                    held: HashSet::new(),
                    toggle: crate::store::settings::Settings::default().toggle_hotkey_chord(),
                    active_chord: None,
                    toggle_consumed_vk: None,
                    consumed_hold_vks: HashSet::new(),
                });
                let flags = (*kb).flags;
                let (evs, swallow) = on_key_flags(st, vk, down, flags.0 & 0x10 != 0);
                for ev in evs {
                    push(ev);
                }
                if swallow {
                    forward_with_default = false;
                }
            }
            // Poisoned mutex => fail OPEN: forward to the system rather than
            // eat someone's keys.
            if !forward_with_default {
                return LRESULT(1); // swallowed
            }
        }
    }
    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
}

fn push(ev: TriggerEvent) {
    if let Ok(mut q) = events_cell().lock() {
        if q.len() >= 64 {
            q.clear();
            q.push((TriggerEvent::Cancel, Instant::now()));
            return;
        }
        q.push((ev, Instant::now()));
    }
}

/// Poll the trigger-event queue from any thread.
pub fn poll_events() -> Vec<TriggerEvent> {
    match events_cell().lock() {
        Ok(mut q) => std::mem::take(&mut *q)
            .into_iter()
            .map(|(e, _)| e)
            .collect(),
        Err(_) => Vec::new(),
    }
}
pub fn poll_timed_events() -> Vec<(TriggerEvent, Instant)> {
    match events_cell().lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => Vec::new(),
    }
}

impl HotkeyManager for WindowsLlHook {
    fn start(&mut self) -> Result<(), String> {
        use windows::Win32::UI::WindowsAndMessaging::SetWindowsHookExW;
        // WH_KEYBOARD_LL requires hmod = NULL per MSDN (dwThreadId == 0).
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_hook_proc), None, 0) }
            .map_err(|e| format!("SetWindowsHookExW failed: {e}"))?;
        *HOOK.lock().map_err(|_| "hook mutex poisoned")? = hook.0 as usize;
        // NOTE: a live LL hook needs a message pump on its installing thread.
        // main.rs spawns this module's thread with a GetMessageW pump.
        self.installed = true;
        Ok(())
    }

    fn stop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx;
        let hook = HOOK.lock().map(|g| *g).unwrap_or(0);
        if hook != 0 {
            unsafe {
                let _ = UnhookWindowsHookEx(windows::Win32::UI::WindowsAndMessaging::HHOOK(
                    hook as isize,
                ));
            }
            *HOOK.lock().unwrap() = 0;
        }
        self.installed = false;
    }

    fn take_events(&mut self) -> Vec<TriggerEvent> {
        poll_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::settings::parse_chord;

    const VK_CONTROL: u32 = 0x11;
    const VK_LWIN: u32 = 0x5B;
    const VK_RCTRL: u32 = 0xA3;
    const VK_A: u32 = 0x41; // bystander letter

    fn machine(chords: &[&str]) -> HookState {
        HookState {
            chords: chords.iter().map(|s| parse_chord(s).unwrap()).collect(),
            toggle: parse_chord("ctrl+space").unwrap(),
            held: HashSet::new(),
            active_chord: None,
            toggle_consumed_vk: None,
            consumed_hold_vks: HashSet::new(),
        }
    }

    fn drive(st: &mut HookState, seq: &[(u32, bool)]) -> (Vec<TriggerEvent>, Vec<bool>) {
        let mut evs = Vec::new();
        let mut swallows = Vec::new();
        for &(vk, down) in seq {
            let (e, sw) = on_key(st, vk, down);
            evs.extend(e);
            swallows.push(sw);
        }
        (evs, swallows)
    }

    #[test]
    fn default_ctrl_then_win_is_ptt() {
        let mut st = machine(&["ctrl+win"]);
        let (evs, sw) = drive(
            &mut st,
            &[
                (VK_CONTROL, true),
                (VK_LWIN, true),
                (VK_LWIN, false),
                (VK_CONTROL, false),
            ],
        );
        assert_eq!(evs, vec![TriggerEvent::PttStart, TriggerEvent::PttEnd]);
        // Win-down swallowed (start-menu guard); its UP too; plain ctrl taps not
        assert_eq!(sw, vec![false, true, true, false]);
    }

    #[test]
    fn second_configured_chord_fires_independently() {
        let mut st = machine(&["ctrl+win", "alt+shift"]);
        let (evs, _) = drive(
            &mut st,
            &[(0x12, true), (0x10, true), (0x10, false)], // alt, shift, shift-up
        );
        assert_eq!(evs.len(), 2);
        assert!(matches!(evs[0], TriggerEvent::PttStart));
        assert!(matches!(evs[1], TriggerEvent::PttEnd));
    }

    #[test]
    fn dedicated_side_key_single_button() {
        let mut st = machine(&["rctrl"]);
        let (evs, sw) = drive(&mut st, &[(VK_RCTRL, true), (VK_A, true)]);
        assert_eq!(evs, vec![TriggerEvent::PttStart]);
        assert_eq!(sw, vec![true, false]); // rctrl suppressed, letter passes
    }

    #[test]
    fn bystander_modifier_taps_untouched() {
        // Configured ONLY for ctrl+win: bare alt tap must neither trigger nor
        // be swallowed (system shortcuts stay intact).
        let mut st = machine(&["ctrl+win"]);
        let (evs, sw) = drive(&mut st, &[(0x12, true), (0x12, false)]);
        assert!(evs.is_empty());
        assert_eq!(sw, vec![false, false]);
    }

    #[test]
    fn declared_trigger_must_be_pressed_last() {
        let mut st = machine(&["ctrl+win"]);
        let (evs, _) = drive(&mut st, &[(VK_LWIN, true), (VK_CONTROL, true)]);
        assert!(evs.is_empty());
    }

    #[test]
    fn releasing_ctrl_first_consumes_only_original_trigger_up() {
        for trigger in [VK_LWIN, 0x20] {
            let mut st = machine(&["ctrl+win"]);
            let (_, sw) = drive(
                &mut st,
                &[
                    (VK_CONTROL, true),
                    (trigger, true),
                    (VK_CONTROL, false),
                    (trigger, false),
                ],
            );
            assert_eq!(sw, vec![false, true, false, true]);
        }
    }

    #[test]
    fn injected_shortcuts_are_ignored() {
        let mut st = machine(&["ctrl+win"]);
        assert!(on_key_flags(&mut st, VK_CONTROL, true, true).0.is_empty());
        assert!(on_key_flags(&mut st, 0x20, true, true).0.is_empty());
        assert!(st.held.is_empty());
    }

    #[test]
    fn toggle_is_one_event_per_physical_space_press_and_consumes_both_edges() {
        let mut st = machine(&["ctrl+win"]);
        let (evs, sw) = drive(
            &mut st,
            &[
                (VK_CONTROL, true),
                (0x20, true),
                (0x20, true),
                (0x20, false),
                (VK_CONTROL, false),
            ],
        );
        assert_eq!(evs, vec![TriggerEvent::Toggle]);
        assert_eq!(sw, vec![false, true, true, true, false]);
    }

    #[test]
    fn toggle_does_not_steal_ctrl_shift_space_or_ctrl_alt_space() {
        for extra in [0x10, 0xA0, 0x12, 0xA4] {
            let mut st = machine(&["ctrl+win"]);
            let (events, swallowed) = drive(
                &mut st,
                &[
                    (VK_CONTROL, true),
                    (extra, true),
                    (0x20, true),
                    (0x20, false),
                    (extra, false),
                    (VK_CONTROL, false),
                ],
            );
            assert!(events.is_empty());
            assert!(swallowed.iter().all(|v| !*v));
        }
    }

    #[test]
    fn unrelated_release_does_not_end_active_hold() {
        let mut st = machine(&["ctrl+win"]);
        let (evs, _) = drive(
            &mut st,
            &[
                (VK_CONTROL, true),
                (VK_LWIN, true),
                (VK_A, true),
                (VK_A, false),
                (VK_LWIN, false),
            ],
        );
        assert_eq!(evs, vec![TriggerEvent::PttStart, TriggerEvent::PttEnd]);
    }
}
