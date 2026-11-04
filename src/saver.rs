//! Windows screensaver mode (roadmap 5d). Windows starts a `.scr` with `/s` (run full screen),
//! `/c` or `/c:HWND` (settings), or `/p HWND` (draw a preview into the Screen Saver dialog).
use crate::platform::HWND;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::Foundation::RECT;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::*;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

pub enum Mode {
    Normal,
    Show,
    Preview(HWND),
}
/// Reads the screensaver switches; any other command line is the normal app.
pub fn mode(args: &[String]) -> Mode {
    let Some(first) = args.first().map(|a| a.to_ascii_lowercase()) else {
        return Mode::Normal;
    };
    if first.starts_with("/s") || first.starts_with("-s") {
        return Mode::Show;
    }
    if first.starts_with("/p") || first.starts_with("-p") {
        let handle = first
            .split_once(':')
            .map(|(_, h)| h.to_string())
            .or_else(|| args.get(1).cloned())
            .and_then(|h| h.trim().parse::<usize>().ok());
        if let Some(h) = handle {
            return Mode::Preview(HWND(h as *mut _));
        }
    }
    Mode::Normal
}
/// Paints a scene thumbnail into the preview window until Windows closes it.
#[cfg(not(windows))]
pub fn preview(_parent: HWND) {}
#[cfg(windows)]
pub fn preview(parent: HWND) {
    let thumbnail = crate::scenes::list()
        .iter()
        .filter_map(|e| e.thumbnail())
        .find(|p| p.is_file())
        .and_then(|p| image::open(p).ok())
        .map(|i| i.to_rgba8());
    let started = Instant::now();
    unsafe {
        while IsWindow(Some(parent)).as_bool() && started.elapsed() < Duration::from_secs(3600) {
            let mut r = RECT::default();
            if GetClientRect(parent, &mut r).is_ok() {
                let dc = GetDC(Some(parent));
                match &thumbnail {
                    Some(img) => {
                        // GDI wants BGRA, bottom-up unless the height is negative.
                        let bgra: Vec<u8> =
                            img.pixels().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
                        let info = BITMAPINFO {
                            bmiHeader: BITMAPINFOHEADER {
                                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                                biWidth: img.width() as i32,
                                biHeight: -(img.height() as i32),
                                biPlanes: 1,
                                biBitCount: 32,
                                biCompression: BI_RGB.0,
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        SetStretchBltMode(dc, HALFTONE);
                        StretchDIBits(
                            dc,
                            0,
                            0,
                            r.right - r.left,
                            r.bottom - r.top,
                            0,
                            0,
                            img.width() as i32,
                            img.height() as i32,
                            Some(bgra.as_ptr() as _),
                            &info,
                            DIB_RGB_COLORS,
                            SRCCOPY,
                        );
                    }
                    None => {
                        let brush =
                            CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x0010_1010));
                        FillRect(dc, &r, brush);
                        let _ = DeleteObject(brush.into());
                    }
                }
                ReleaseDC(Some(parent), dc);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
/// Show mode: cycles the scenes, orbiting slowly, and ends on any input.
pub struct Saver {
    pub next_switch: Instant,
    pub index: usize,
    pub first_cursor: Option<[f64; 2]>,
    pub started: Instant,
}
impl Saver {
    pub const SCENE_SECONDS: u64 = 45;
    pub fn new() -> Self {
        Self {
            next_switch: Instant::now(),
            index: 0,
            first_cursor: None,
            started: Instant::now(),
        }
    }
    /// True once the pointer moved more than a few pixels from where it first appeared.
    pub fn moved(&mut self, cursor: [f64; 2]) -> bool {
        match self.first_cursor {
            None => {
                self.first_cursor = Some(cursor);
                false
            }
            Some(c) => (cursor[0] - c[0]).abs() + (cursor[1] - c[1]).abs() > 12.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screensaver_switches_parse() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(mode(&a(&["/S"])), Mode::Show));
        assert!(matches!(mode(&a(&["/p", "1234"])), Mode::Preview(h) if h.0 as usize == 1234));
        assert!(matches!(mode(&a(&["/p:77"])), Mode::Preview(h) if h.0 as usize == 77));
        assert!(matches!(mode(&a(&["/c:5"])), Mode::Normal));
        assert!(matches!(mode(&a(&["--preset", "3"])), Mode::Normal));
    }
}
