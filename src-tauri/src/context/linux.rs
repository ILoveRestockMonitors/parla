//! Read-only X11 focus and process identity. Wayland fails closed.
use super::target::{FieldContext, TargetSnapshot};
use libc::{c_char, c_int, c_long, c_uchar, c_ulong, c_void};
use std::ffi::CString;
use std::sync::OnceLock;
type Display = c_void;
#[repr(C)]
struct XErrorEvent {
    kind: c_int,
    display: *mut Display,
    resource: c_ulong,
    serial: c_ulong,
    error_code: c_uchar,
    request_code: c_uchar,
    minor_code: c_uchar,
}
type ErrorHandler = unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int;
static PREVIOUS_ERROR_HANDLER: OnceLock<Option<ErrorHandler>> = OnceLock::new();
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
#[link(name = "X11")]
extern "C" {
    fn XInitThreads() -> c_int;
    fn XSetErrorHandler(handler: Option<ErrorHandler>) -> Option<ErrorHandler>;
    fn XOpenDisplay(name: *const c_char) -> *mut Display;
    fn XCloseDisplay(display: *mut Display) -> c_int;
    fn XDefaultRootWindow(display: *mut Display) -> c_ulong;
    fn XGetInputFocus(display: *mut Display, focus: *mut c_ulong, revert: *mut c_int) -> c_int;
    fn XInternAtom(display: *mut Display, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XGetWindowProperty(
        display: *mut Display,
        window: c_ulong,
        property: c_ulong,
        offset: c_long,
        length: c_long,
        delete: c_int,
        req_type: c_ulong,
        actual_type: *mut c_ulong,
        format: *mut c_int,
        items: *mut c_ulong,
        remaining: *mut c_ulong,
        value: *mut *mut c_uchar,
    ) -> c_int;
    fn XFree(data: *mut c_void) -> c_int;
}

/// Must precede rdev, Enigo/libxdo and any read-only display connections.
pub fn initialize() -> Result<(), String> {
    INITIALIZED
        .get_or_init(|| unsafe {
            if XInitThreads() == 0 {
                return Err(
                    "X11 thread initialization failed; global desktop access is unavailable."
                        .into(),
                );
            }
            let previous = XSetErrorHandler(Some(error_handler));
            let _ = PREVIOUS_ERROR_HANDLER.set(previous);
            Ok(())
        })
        .clone()
}

unsafe extern "C" fn error_handler(display: *mut Display, error: *mut XErrorEvent) -> c_int {
    // A destination may disappear between focus and property requests. Xlib's
    // default BadWindow handler exits the process; let the request return its
    // failure instead. Preserve the previous policy for unrelated X errors.
    if !error.is_null() && (*error).error_code == 3 {
        return 0;
    }
    if let Some(Some(previous)) = PREVIOUS_ERROR_HANDLER.get() {
        return previous(display, error);
    }
    0
}
struct Connection(*mut Display);
impl Connection {
    fn open() -> Option<Self> {
        if crate::platform::is_wayland() {
            return None;
        }
        initialize().ok()?;
        let display = unsafe { XOpenDisplay(std::ptr::null()) };
        (!display.is_null()).then_some(Self(display))
    }
    fn property(&self, window: c_ulong, name: &str) -> Option<c_ulong> {
        unsafe {
            let name = CString::new(name).ok()?;
            let atom = XInternAtom(self.0, name.as_ptr(), 1);
            if atom == 0 || window == 0 {
                return None;
            }
            let (mut kind, mut format, mut count, mut remaining) = (0, 0, 0, 0);
            let mut data = std::ptr::null_mut();
            let status = XGetWindowProperty(
                self.0,
                window,
                atom,
                0,
                1,
                0,
                0,
                &mut kind,
                &mut format,
                &mut count,
                &mut remaining,
                &mut data,
            );
            let value = if status == 0 && format == 32 && count == 1 && !data.is_null() {
                Some(*(data as *const c_ulong))
            } else {
                None
            };
            if !data.is_null() {
                XFree(data.cast());
            }
            value
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            XCloseDisplay(self.0);
        }
    }
}
pub fn exe_for_window(window: isize) -> Option<String> {
    let display = Connection::open()?;
    let pid = display.property(window as c_ulong, "_NET_WM_PID")?;
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    path.file_name().map(|s| s.to_string_lossy().into_owned())
}
pub fn capture() -> TargetSnapshot {
    let Some(display) = Connection::open() else {
        return TargetSnapshot::unavailable();
    };
    unsafe {
        let (mut focus, mut revert) = (0, 0);
        XGetInputFocus(display.0, &mut focus, &mut revert);
        let window = display
            .property(XDefaultRootWindow(display.0), "_NET_ACTIVE_WINDOW")
            .unwrap_or(focus);
        // None=0 and PointerRoot=1 do not identify an editable destination.
        if focus <= 1 || window <= 1 {
            return TargetSnapshot::unavailable();
        }
        let terminal =
            exe_for_window(window as isize).is_some_and(|app| super::is_terminal_exe(&app));
        TargetSnapshot {
            hwnd: window as isize,
            focus: focus as isize,
            input_epoch: None,
            context: FieldContext {
                runtime_id: Some(vec![window as i32, focus as i32]),
                terminal,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destroyed_window_error_is_nonfatal_without_a_display() {
        let mut error = XErrorEvent {
            kind: 0,
            display: std::ptr::null_mut(),
            resource: 42,
            serial: 1,
            error_code: 3,
            request_code: 20,
            minor_code: 0,
        };
        assert_eq!(
            unsafe { error_handler(std::ptr::null_mut(), &mut error) },
            0
        );
    }
}
