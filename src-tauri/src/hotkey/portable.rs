//! Passive OS event observation. No root/evdev access and no synthetic releases
//! of the user's keys. Chords must be reserved by the user in their desktop.
use super::TriggerEvent;
use crate::store::settings::HotkeyChord;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use std::time::Instant;

static EVENTS: Mutex<Vec<(TriggerEvent, Instant)>> = Mutex::new(Vec::new());
static STATE: Mutex<Option<State>> = Mutex::new(None);
static READY: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(0);
struct State {
    chords: Vec<HotkeyChord>,
    toggle: HotkeyChord,
    held: HashSet<u32>,
    active: Option<usize>,
}
impl State {
    fn key(&mut self, key: u32, down: bool) -> (Vec<TriggerEvent>, bool) {
        let mut events = Vec::new();
        let mut gesture = false;
        if down {
            let fresh = self.held.insert(key);
            let matches = |chord: &HotkeyChord| {
                chord.sets.last().is_some_and(|g| g.contains(&key))
                    && held(chord, &self.held)
                    && self
                        .held
                        .iter()
                        .filter(|k| modifier(**k))
                        .all(|k| chord.sets.iter().any(|g| g.contains(k)))
            };
            if matches(&self.toggle) {
                gesture = true;
                if fresh {
                    events.push(TriggerEvent::Toggle);
                }
            } else if fresh && self.active.is_none() {
                if let Some(index) = self.chords.iter().position(matches) {
                    self.active = Some(index);
                    gesture = true;
                    events.push(TriggerEvent::PttStart);
                }
            }
        } else {
            self.held.remove(&key);
            if let Some(index) = self.active {
                if !held(&self.chords[index], &self.held) {
                    self.active = None;
                    events.push(TriggerEvent::PttEnd);
                }
            }
        }
        (events, gesture)
    }
}
fn held(chord: &HotkeyChord, held: &HashSet<u32>) -> bool {
    chord
        .sets
        .iter()
        .all(|g| g.iter().any(|k| held.contains(k)))
}
fn modifier(key: u32) -> bool {
    matches!(key, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5)
}
pub fn set_hotkeys(chords: Vec<HotkeyChord>, toggle: HotkeyChord) {
    if let Ok(mut state) = STATE.lock() {
        *state = Some(State {
            chords,
            toggle,
            held: HashSet::new(),
            active: None,
        });
    }
}
pub fn input_epoch() -> Option<u64> {
    READY
        .load(Ordering::Acquire)
        .then(|| EPOCH.load(Ordering::Acquire))
}
pub fn modifiers_released() -> bool {
    STATE.lock().ok().is_some_and(|s| {
        s.as_ref()
            .is_some_and(|s| s.held.iter().all(|k| !modifier(*k) && *k != 0x20))
    })
}
pub(super) fn push(event: TriggerEvent) {
    if let Ok(mut queue) = EVENTS.lock() {
        if queue.len() >= 64 {
            queue.clear();
            queue.push((TriggerEvent::Cancel, Instant::now()));
        } else {
            queue.push((event, Instant::now()));
        }
    }
}
pub fn poll_timed_events() -> Vec<(TriggerEvent, Instant)> {
    EVENTS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}
fn vk(key: rdev::Key) -> u32 {
    use rdev::Key::*;
    match key {
        ControlLeft => 0xa2,
        ControlRight => 0xa3,
        MetaLeft => 0x5b,
        MetaRight => 0x5c,
        ShiftLeft => 0xa0,
        ShiftRight => 0xa1,
        Alt => 0xa4,
        AltGr => 0xa5,
        Space => 0x20,
        // No typed content is retained. One non-modifier invalidates the field.
        _ => 0xffff,
    }
}
pub fn listen() -> Result<(), String> {
    if crate::platform::is_wayland() {
        return Err("Wayland requires desktop-bound 'parla toggle' or dashboard Start/stop; paste the result manually.".into());
    }
    #[cfg(target_os = "macos")]
    if !crate::context::native::accessibility_trusted() {
        return Err("Grant Accessibility and Input Monitoring to Parla, then restart. Dashboard controls remain available.".into());
    }
    // A listener is only marked ready after it has delivered a real event.
    // A denied/dead listener must never authorize the insertion fallback.
    let result = rdev::listen(|event| {
        if !READY.swap(true, Ordering::AcqRel) {
            crate::runtime::update(|s| s.hook_ready = true);
        }
        let key = match event.event_type {
            rdev::EventType::KeyPress(k) => Some((vk(k), true)),
            rdev::EventType::KeyRelease(k) => Some((vk(k), false)),
            rdev::EventType::ButtonPress(_) => {
                EPOCH.fetch_add(1, Ordering::AcqRel);
                None
            }
            _ => None,
        };
        if let Some((key, down)) = key {
            let mut gesture = false;
            if let Ok(mut state) = STATE.lock() {
                if let Some(state) = state.as_mut() {
                    let (events, is_gesture) = state.key(key, down);
                    gesture = is_gesture;
                    for event in events {
                        push(event);
                    }
                }
            }
            if down && !modifier(key) && !gesture {
                EPOCH.fetch_add(1, Ordering::AcqRel);
            }
        }
    });
    READY.store(false, Ordering::Release);
    result.map_err(|e| {
        format!("{e:?}; check display, Accessibility/Input Monitoring permissions, then restart.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::settings::parse_chord;
    #[test]
    fn hold_release_and_toggle_repeat_are_distinct() {
        let mut s = State {
            chords: vec![parse_chord("ctrl+win").unwrap()],
            toggle: parse_chord("ctrl+alt+space").unwrap(),
            held: HashSet::new(),
            active: None,
        };
        assert!(s.key(0xa2, true).0.is_empty());
        assert_eq!(s.key(0x5b, true).0, vec![TriggerEvent::PttStart]);
        assert_eq!(s.key(0xa2, false).0, vec![TriggerEvent::PttEnd]);
        s.key(0x5b, false);
        s.key(0xa2, true);
        s.key(0xa4, true);
        assert_eq!(s.key(0x20, true).0, vec![TriggerEvent::Toggle]);
        assert!(s.key(0x20, true).0.is_empty());
        s.key(0x20, false);
        s.key(0xa0, true);
        assert!(
            s.key(0x20, true).0.is_empty(),
            "extra modifier must not trigger"
        );
    }
}
