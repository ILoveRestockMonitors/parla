// Foreground process image name via Win32. Best-effort: on failure returns
// None and the pipeline sends app_category "other".
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

/// Returns the foreground executable's image name (e.g. "Code.exe"), if any.
pub fn foreground_exe() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0 == 0 {
            return None;
        }
        let mut title = [0u16; 512];
        let _ = GetWindowTextW(hwnd, &mut title);

        let mut pid = 0u32;
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let len = GetModuleFileNameExW(HANDLE(handle.0), None, &mut buf);
        let _ = windows::Win32::Foundation::CloseHandle(HANDLE(handle.0));
        if len == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().map(|s| s.to_string())
    }
}
