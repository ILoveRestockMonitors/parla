//! Accessibility identity only: no surrounding text or window titles are read.
use super::target::{FieldContext, TargetSnapshot};
use std::ffi::{c_char, c_void, CStr};
type CFRef = *const c_void;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> CFRef;
    fn AXUIElementCopyAttributeValue(element: CFRef, attribute: CFRef, value: *mut CFRef) -> i32;
    fn AXUIElementGetPid(element: CFRef, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFRef, timeout: f32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn CGPreflightListenEventAccess() -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(allocator: CFRef, text: *const c_char, encoding: u32) -> CFRef;
    fn CFStringGetCString(string: CFRef, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
    fn CFRelease(value: CFRef);
    fn CFHash(value: CFRef) -> usize;
    fn CFGetTypeID(value: CFRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}
#[link(name = "proc")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
}
struct Owned(CFRef);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                CFRelease(self.0);
            }
        }
    }
}
fn attribute(element: CFRef, name: &'static [u8]) -> Option<Owned> {
    unsafe {
        let key = Owned(CFStringCreateWithCString(
            std::ptr::null(),
            name.as_ptr().cast(),
            0x08000100,
        ));
        if key.0.is_null() {
            return None;
        }
        let mut value = std::ptr::null();
        (AXUIElementCopyAttributeValue(element, key.0, &mut value) == 0 && !value.is_null())
            .then_some(Owned(value))
    }
}
fn string(value: &Owned) -> Option<String> {
    let mut bytes = [0i8; 256];
    unsafe {
        if CFGetTypeID(value.0) != CFStringGetTypeID() {
            return None;
        }
        CFStringGetCString(
            value.0,
            bytes.as_mut_ptr(),
            bytes.len() as isize,
            0x08000100,
        )
        .then(|| {
            CStr::from_ptr(bytes.as_ptr())
                .to_string_lossy()
                .into_owned()
        })
    }
}
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}
pub fn input_monitoring_granted() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}
pub fn exe_for_window(pid: isize) -> Option<String> {
    let mut bytes = [0u8; 4096];
    let len = unsafe { proc_pidpath(pid as i32, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if len <= 0 {
        return None;
    }
    let text = unsafe { CStr::from_ptr(bytes.as_ptr().cast()) }.to_string_lossy();
    text.rsplit('/').next().map(str::to_owned)
}
pub fn capture() -> TargetSnapshot {
    fn capture_inner() -> Option<TargetSnapshot> {
        if !accessibility_trusted() {
            return None;
        }
        unsafe {
            let system = Owned(AXUIElementCreateSystemWide());
            if system.0.is_null() {
                return None;
            }
            AXUIElementSetMessagingTimeout(system.0, 0.08);
            let app = attribute(system.0, b"AXFocusedApplication\0")?;
            if CFGetTypeID(app.0) != AXUIElementGetTypeID() {
                return None;
            }
            AXUIElementSetMessagingTimeout(app.0, 0.08);
            let mut pid = 0;
            if AXUIElementGetPid(app.0, &mut pid) != 0 || pid == 0 {
                return None;
            }
            let focused = attribute(app.0, b"AXFocusedUIElement\0")?;
            if CFGetTypeID(focused.0) != AXUIElementGetTypeID() {
                return None;
            }
            AXUIElementSetMessagingTimeout(focused.0, 0.08);
            let role = string(&attribute(focused.0, b"AXRole\0")?)?;
            let subrole = attribute(focused.0, b"AXSubrole\0").and_then(|v| string(&v));
            // A missing text-field subtype cannot establish that it isn't a
            // secure field. Unsupported providers fall back to manual paste.
            if role == "AXTextField" && subrole.is_none() {
                return None;
            }
            let password = subrole.as_deref() == Some("AXSecureTextField");
            let terminal =
                exe_for_window(pid as isize).is_some_and(|app| super::is_terminal_exe(&app));
            if !password
                && !terminal
                && !["AXTextField", "AXTextArea", "AXComboBox", "AXWebArea"]
                    .contains(&role.as_str())
            {
                return None;
            }
            let identity = CFHash(focused.0);
            Some(TargetSnapshot {
                hwnd: pid as isize,
                focus: identity as isize,
                input_epoch: None,
                context: FieldContext {
                    runtime_id: Some(vec![pid, identity as i32, (identity >> 32) as i32]),
                    password,
                    terminal,
                    ..Default::default()
                },
            })
        }
    }
    capture_inner().unwrap_or_else(TargetSnapshot::unavailable)
}
