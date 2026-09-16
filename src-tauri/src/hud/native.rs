//! Per-pixel alpha capsule. Shapes are anti-aliased in physical pixels; GDI
//! supplies grayscale glyph coverage, composited into premultiplied BGRA.
//! Only this HUD thread opts into per-monitor DPI awareness.
use serde_json::Value;
use std::{
    mem::size_of,
    path::Path,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HANDLE, HWND, POINT, RECT, SIZE},
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

const WIDTH: f32 = 392.0;
const HEIGHT: f32 = 92.0;
const MINT: [u8; 3] = [159, 226, 204];

#[derive(Clone, PartialEq, Eq)]
struct Content {
    recording: bool,
    hint: String,
    timer: String,
    label: String,
}
impl Content {
    fn from_snapshot(snapshot: &Value) -> Option<Self> {
        let label = super::status_label(snapshot)?;
        let recording = snapshot["recording"].as_bool().unwrap_or(false);
        let seconds = snapshot["recording_elapsed_ms"].as_u64().unwrap_or(0) / 1000;
        let hint = if !recording {
            "Turning your voice into text".into()
        } else if snapshot["mode"].as_str() == Some("hold") {
            "Release keys to finish".into()
        } else {
            format!(
                "{} to finish",
                snapshot["effective_settings"]["toggle_chord"]
                    .as_str()
                    .unwrap_or("Ctrl+Space")
            )
        };
        Some(Self {
            recording,
            hint,
            timer: format!("{:02}:{:02}", seconds / 60, seconds % 60),
            label,
        })
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: usize,
    height: usize,
    scale: f32,
}
impl Surface {
    fn new(scale: f32) -> windows::core::Result<Self> {
        let width = (WIDTH * scale).round() as usize;
        let height = (HEIGHT * scale).round() as usize;
        unsafe {
            let dc = CreateCompatibleDC(HDC(0));
            if dc.0 == 0 {
                return Err(windows::core::Error::from_win32());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, HANDLE(0), 0)
            {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
            let previous = SelectObject(dc, bitmap);
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
                scale,
            })
        }
    }
    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, self.width * self.height * 4) }
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.width * self.height * 4) }
    }
    fn blend(&mut self, x: usize, y: usize, rgb: [u8; 3], alpha: f32) {
        let offset = (y * self.width + x) * 4;
        let pixel = &mut self.bytes_mut()[offset..offset + 4];
        let a = alpha.clamp(0.0, 1.0);
        for (channel, color) in [rgb[2], rgb[1], rgb[0]].iter().enumerate() {
            pixel[channel] = (*color as f32 * a + pixel[channel] as f32 * (1.0 - a)).round() as u8;
        }
        pixel[3] = (255.0 * a + pixel[3] as f32 * (1.0 - a)).round() as u8;
    }
    fn rounded(&mut self, bounds: [f32; 4], radius: f32, rgb: [u8; 3], opacity: f32) {
        let [left, top, width, height] = bounds;
        let min_x = ((left * self.scale).floor() as i32 - 1).max(0) as usize;
        let min_y = ((top * self.scale).floor() as i32 - 1).max(0) as usize;
        let max_x = (((left + width) * self.scale).ceil() as usize + 1).min(self.width);
        let max_y = (((top + height) * self.scale).ceil() as usize + 1).min(self.height);
        for y in min_y..max_y {
            for x in min_x..max_x {
                let px = (x as f32 + 0.5) / self.scale;
                let py = (y as f32 + 0.5) / self.scale;
                let distance = rounded_distance(px, py, bounds, radius);
                self.blend(
                    x,
                    y,
                    rgb,
                    opacity * (0.5 - distance * self.scale).clamp(0.0, 1.0),
                );
            }
        }
    }
    fn text(
        &mut self,
        mask: &mut Surface,
        text: &str,
        bounds: [f32; 4],
        size: f32,
        weight: i32,
        rgb: [u8; 3],
        right: bool,
    ) -> windows::core::Result<()> {
        mask.bytes_mut().fill(0);
        unsafe {
            let font = CreateFontW(
                -(size * mask.scale).round() as i32,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                ANTIALIASED_QUALITY.0 as u32,
                DEFAULT_PITCH.0 as u32,
                w!("Segoe UI"),
            );
            if font.0 == 0 {
                return Err(windows::core::Error::from_win32());
            }
            let previous = SelectObject(mask.dc, font);
            SetBkMode(mask.dc, TRANSPARENT);
            SetTextColor(mask.dc, COLORREF(0x00ffffff));
            let [x, y, width, height] = bounds;
            let mut rect = RECT {
                left: (x * mask.scale).round() as i32,
                top: (y * mask.scale).round() as i32,
                right: ((x + width) * mask.scale).round() as i32,
                bottom: ((y + height) * mask.scale).round() as i32,
            };
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                mask.dc,
                &mut wide,
                &mut rect,
                DT_SINGLELINE
                    | DT_VCENTER
                    | DT_NOPREFIX
                    | DT_END_ELLIPSIS
                    | if right { DT_RIGHT } else { DT_LEFT },
            );
            // DIB memory must not be read while GDI still has a queued draw.
            let _ = GdiFlush();
            SelectObject(mask.dc, previous);
            let _ = DeleteObject(font);
        }
        for y in 0..self.height {
            for x in 0..self.width {
                // Render glyphs at twice the physical resolution. This avoids
                // harsh font hinting at the small 100% HUD size without colored
                // ClearType fringes on a transparent layered window.
                let offset = (y * 2 * mask.width + x * 2) * 4;
                let pixels = mask.bytes();
                let coverage = (pixels[offset] as f32
                    + pixels[offset + 4] as f32
                    + pixels[offset + mask.width * 4] as f32
                    + pixels[offset + mask.width * 4 + 4] as f32)
                    / (4.0 * 255.0);
                if coverage > 0.0 {
                    self.blend(x, y, rgb, coverage);
                }
            }
        }
        Ok(())
    }
    fn base(&mut self, content: &Content) -> windows::core::Result<()> {
        self.bytes_mut().fill(0);
        // Broad, quiet shadow; the window corners remain truly transparent.
        for y in 0..self.height {
            for x in 0..self.width {
                let px = (x as f32 + 0.5) / self.scale;
                let py = (y as f32 + 0.5) / self.scale;
                let distance = rounded_distance(px, py, [16.0, 18.0, 360.0, 60.0], 30.0).max(0.0);
                let shadow = 0.16 * (-(distance / 6.0).powi(2)).exp();
                self.blend(x, y, [0, 0, 0], shadow);
            }
        }
        self.rounded([16.0, 12.0, 360.0, 60.0], 30.0, [80, 86, 94], 0.96);
        self.rounded([16.8, 12.8, 358.4, 58.4], 29.2, [26, 29, 34], 1.0);
        self.rounded([29.0, 24.0, 36.0, 36.0], 18.0, [38, 52, 50], 1.0);
        let mut mask = Surface::new(self.scale * 2.0)?;
        let text_width = if content.recording { 216.0 } else { 274.0 };
        self.text(
            &mut mask,
            if content.recording {
                "Listening"
            } else {
                "Processing"
            },
            [79.0, 23.0, text_width, 20.0],
            14.0,
            600,
            [243, 245, 247],
            false,
        )?;
        self.text(
            &mut mask,
            &content.hint,
            [79.0, 44.0, text_width, 16.0],
            11.0,
            400,
            [161, 169, 179],
            false,
        )?;
        if content.recording {
            self.rounded([306.0, 30.0, 1.0, 24.0], 0.5, [67, 73, 81], 0.7);
            self.text(
                &mut mask,
                &content.timer,
                [315.0, 31.0, 43.0, 21.0],
                13.0,
                500,
                [188, 196, 206],
                true,
            )?;
        }
        Ok(())
    }
    fn indicator(&mut self, recording: bool, seconds: f32, animate: bool) {
        if recording {
            // A gently breathing status symbol, not a simulated audio meter.
            let opacity = if animate {
                0.82 + 0.18 * (seconds * 2.4).sin()
            } else {
                1.0
            };
            for (index, height) in [8.0, 15.0, 21.0, 15.0, 8.0].iter().enumerate() {
                self.rounded(
                    [35.5 + index as f32 * 5.0, 42.0 - height / 2.0, 3.0, *height],
                    1.5,
                    MINT,
                    opacity,
                );
            }
        } else {
            for index in 0..3 {
                let opacity = if animate {
                    0.38 + 0.62 * ((seconds * 4.5 - index as f32 * 0.8).sin() * 0.5 + 0.5)
                } else {
                    0.8
                };
                self.rounded(
                    [37.0 + index as f32 * 8.0, 39.5, 4.0, 4.0],
                    2.0,
                    MINT,
                    opacity,
                );
            }
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}
fn rounded_distance(x: f32, y: f32, bounds: [f32; 4], radius: f32) -> f32 {
    let [left, top, width, height] = bounds;
    let dx = (x - left - width / 2.0).abs() - width / 2.0 + radius;
    let dy = (y - top - height / 2.0).abs() - height / 2.0 + radius;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

struct Overlay(HWND);
impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

pub fn run() -> windows::core::Result<()> {
    run_with_snapshot(|_| Some(crate::runtime::hud_snapshot()))
}

fn run_with_snapshot(mut snapshot: impl FnMut(HWND) -> Option<Value>) -> windows::core::Result<()> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            w!("STATIC"),
            w!("Parla"),
            WS_POPUP,
            0,
            0,
            WIDTH as i32,
            HEIGHT as i32,
            None,
            None,
            None,
            None,
        );
        if hwnd.0 == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let _window = Overlay(hwnd);
        let mut surface = Surface::new(1.0)?;
        let mut base = Vec::new();
        let mut previous_content: Option<Content> = None;
        let mut opacity = 0.0f32;
        let mut shown = false;
        let mut point = POINT::default();
        let started = Instant::now();
        let mut previous_frame = Instant::now();
        let mut animate = true;
        let mut message = MSG::default();
        loop {
            while PeekMessageW(&mut message, HWND(0), 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let Some(snapshot) = snapshot(hwnd) else {
                return Ok(());
            };
            let content = Content::from_snapshot(&snapshot);
            let visible = content.is_some();
            let now = Instant::now();
            let dt = (now - previous_frame).as_secs_f32().min(0.05);
            previous_frame = now;
            if visible && !shown {
                let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                GetMonitorInfoW(monitor, &mut info).ok()?;
                // Move onto the active monitor first, then query that window's DPI.
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    info.rcWork.left,
                    info.rcWork.top,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
                let scale = (GetDpiForWindow(hwnd).max(96) as f32 / 96.0).clamp(1.0, 4.0);
                if (scale - surface.scale).abs() > 0.01 {
                    surface = Surface::new(scale)?;
                }
                point = POINT {
                    x: info.rcWork.left
                        + (info.rcWork.right - info.rcWork.left - surface.width as i32) / 2,
                    y: info.rcWork.bottom - surface.height as i32 - (12.0 * scale).round() as i32,
                };
                let mut enabled = windows::Win32::Foundation::BOOL(1);
                let _ = SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    Some((&mut enabled as *mut windows::Win32::Foundation::BOOL).cast()),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
                animate = enabled.as_bool();
                previous_content = None;
                shown = true;
            }
            // Disabling the HUD takes effect immediately, even during a fade.
            if snapshot["effective_settings"]["hud_enabled"].as_bool() == Some(false) {
                opacity = 0.0;
            } else if !animate {
                opacity = if visible { 1.0 } else { 0.0 };
            } else {
                opacity = (opacity + if visible { dt / 0.16 } else { -dt / 0.20 }).clamp(0.0, 1.0);
            }
            if let Some(current) = content {
                if previous_content.as_ref() != Some(&current) {
                    surface.base(&current)?;
                    base = surface.bytes().to_vec();
                    let label: Vec<u16> = current.label.encode_utf16().chain(Some(0)).collect();
                    let _ = SetWindowTextW(hwnd, PCWSTR(label.as_ptr()));
                    previous_content = Some(current);
                }
            }
            if shown && opacity > 0.0 {
                surface.bytes_mut().copy_from_slice(&base);
                if let Some(content) = &previous_content {
                    surface.indicator(content.recording, started.elapsed().as_secs_f32(), animate);
                }
                let eased = opacity * opacity * (3.0 - 2.0 * opacity);
                let position = POINT {
                    x: point.x,
                    y: point.y + ((1.0 - eased) * 4.0 * surface.scale).round() as i32,
                };
                let size = SIZE {
                    cx: surface.width as i32,
                    cy: surface.height as i32,
                };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: (255.0 * eased).round() as u8,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                UpdateLayeredWindow(
                    hwnd,
                    HDC(0),
                    Some(&position),
                    Some(&size),
                    surface.dc,
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                )?;
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            } else if shown {
                let _ = ShowWindow(hwnd, SW_HIDE);
                shown = false;
                previous_content = None;
            }
            std::thread::sleep(Duration::from_millis(if shown && animate {
                33
            } else {
                100
            }));
        }
    }
}

/// Export the exact native renderer without starting hooks, recording, playback,
/// or model services. PPM composites make visual review independent of desktop.
pub fn write_previews(directory: &Path) -> std::io::Result<()> {
    use std::io::Write;
    for scale in [1.0, 1.5, 2.0] {
        for recording in [true, false] {
            let snapshot = serde_json::json!({"recording":recording,"processing":!recording,
                "recording_elapsed_ms":83000,"effective_settings":{"hud_enabled":true,"toggle_chord":"Ctrl+Space"}});
            let content = Content::from_snapshot(&snapshot).unwrap();
            let mut frame = Surface::new(scale).map_err(std::io::Error::other)?;
            frame.base(&content).map_err(std::io::Error::other)?;
            frame.indicator(recording, 0.5, true);
            for (theme, bg) in [("light", [240u8, 242, 245]), ("dark", [18u8, 20, 24])] {
                let mut image = Vec::new();
                write!(&mut image, "P6\n{} {}\n255\n", frame.width, frame.height)?;
                for pixel in frame.bytes().chunks_exact(4) {
                    for (channel, index) in [2, 1, 0].iter().enumerate() {
                        image.push(
                            (pixel[*index] as f32
                                + bg[channel] as f32 * (1.0 - pixel[3] as f32 / 255.0))
                                .round()
                                .min(255.0) as u8,
                        );
                    }
                }
                let state = if recording { "listening" } else { "processing" };
                std::fs::write(
                    directory.join(format!(
                        "hud-{state}-{theme}-{}.ppm",
                        (scale * 100.0) as u32
                    )),
                    image,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "briefly displays only the native HUD; no microphone, hooks, or input"]
    fn live_hud_never_takes_focus_and_hides_after_idle() {
        let mut frame = 0;
        run_with_snapshot(|hwnd| {
            frame += 1;
            unsafe {
                assert_ne!(GetForegroundWindow(), hwnd, "HUD must not activate");
                let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                let required = WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED;
                assert_eq!(style & required.0, required.0);
                if frame == 6 { assert!(IsWindowVisible(hwnd).as_bool()); }
                if frame == 24 {
                    assert!(!IsWindowVisible(hwnd).as_bool(), "idle HUD must finish fading out");
                    return None;
                }
            }
            Some(serde_json::json!({"recording":frame < 8,"processing":frame >= 8 && frame < 12,
                "recording_elapsed_ms":83000,"effective_settings":{"hud_enabled":true,"toggle_chord":"Ctrl+Space"}}))
        }).unwrap();
    }

    #[test]
    fn native_frames_have_transparent_corners_and_valid_premultiplied_alpha() {
        for scale in [1.0, 1.5, 2.0] {
            let snapshot = serde_json::json!({"recording":true,"recording_elapsed_ms":83000,
                "effective_settings":{"hud_enabled":true,"toggle_chord":"Ctrl+Shift+Space"}});
            let mut frame = Surface::new(scale).unwrap();
            frame
                .base(&Content::from_snapshot(&snapshot).unwrap())
                .unwrap();
            frame.indicator(true, 0.0, true);
            assert_eq!(&frame.bytes()[..4], &[0, 0, 0, 0]);
            assert!(frame
                .bytes()
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
            assert_eq!(
                frame.bytes()
                    [((42.0 * scale) as usize * frame.width + (196.0 * scale) as usize) * 4 + 3],
                255
            );
        }
    }
}
