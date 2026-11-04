//! What differs between Windows and Linux outside the GPU backends.
use std::process::Command;

#[cfg(windows)]
pub use windows::Win32::Foundation::HWND;
/// Stand-in so code that carries a window handle compiles off Windows; never constructed there.
#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub struct HWND(pub *mut std::ffi::c_void);

/// Keeps a console child process from flashing up a window (Windows `CREATE_NO_WINDOW`).
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Local wall-clock time as (year, month, day, hour, minute, second).
pub fn local_time() -> [u32; 6] {
    #[cfg(windows)]
    {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        [t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond].map(u32::from)
    }
    #[cfg(not(windows))]
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        [
            tm.tm_year as u32 + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        ]
    }
}

#[cfg(windows)]
pub use winit::platform::windows::WindowAttributesExtWindows;
/// The Windows-only window attributes the app sets, as no-ops elsewhere.
#[cfg(not(windows))]
pub trait WindowAttributesExtWindows {
    fn with_taskbar_icon(self, icon: Option<winit::window::Icon>) -> Self;
}
#[cfg(not(windows))]
impl WindowAttributesExtWindows for winit::window::WindowAttributes {
    fn with_taskbar_icon(self, _icon: Option<winit::window::Icon>) -> Self {
        self
    }
}
