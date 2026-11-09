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

/// Opens a web page (or on Linux a folder) with whatever the desktop uses for it.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    let mut command = {
        // Not `cmd /c start`, which would read & and | in the URL as commands.
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(windows))]
    let mut command = Command::new("xdg-open");
    let _ = command.arg(url).spawn();
}

/// Lowers the calling thread's priority, for long CPU work behind a window.
pub fn background_thread() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    // On Linux "process" 0 is the calling thread alone.
    #[cfg(not(windows))]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
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

/// The app id / window class desktop rules match ("caustica"): the Wayland
/// xdg app id and the X11 WM_CLASS. Other platforms have no such name.
pub const APP_ID: &str = "caustica";

/// Tags a window with [`APP_ID`] for whichever Linux backend winit picks.
#[cfg(target_os = "linux")]
pub fn with_app_id(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
    let attrs = WindowAttributesExtWayland::with_name(attrs, APP_ID, APP_ID);
    WindowAttributesExtX11::with_name(attrs, APP_ID, APP_ID)
}
#[cfg(not(target_os = "linux"))]
pub fn with_app_id(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    attrs
}
