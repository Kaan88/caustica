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
#[cfg(target_os = "linux")]
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

/// The event loop. Wayland can't hide a window, so every hidden run (thumbnails, headless
/// renders, turntables) put an entry on the taskbar for its whole life. With `hidden` set it uses
/// X11 instead when a local X server answers (XWayland on a Wayland desktop), where a window that
/// is never mapped stays invisible. winit allows one event loop per process, so it only tries X11
/// when the server's socket answers.
#[cfg(target_os = "linux")]
pub fn event_loop(hidden: bool) -> anyhow::Result<winit::event_loop::EventLoop<()>> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let mut builder = winit::event_loop::EventLoop::builder();
    if hidden && local_x_server() {
        builder.with_x11();
    }
    Ok(builder.build()?)
}
#[cfg(not(target_os = "linux"))]
pub fn event_loop(_hidden: bool) -> anyhow::Result<winit::event_loop::EventLoop<()>> {
    Ok(winit::event_loop::EventLoop::new()?)
}
/// `DISPLAY` names a local server (`:N` or `:N.S`) whose socket accepts a connection (a socket
/// left behind by a server that exited refuses; a lazily started XWayland accepts). It can't see
/// a refused X authorisation, such as a run that inherited `DISPLAY` but not `XAUTHORITY`.
#[cfg(target_os = "linux")]
fn local_x_server() -> bool {
    let Some(display) = std::env::var_os("DISPLAY") else {
        return false;
    };
    let Some(number) = display.to_str().and_then(|d| d.strip_prefix(':')) else {
        return false;
    };
    let number = number.split('.').next().unwrap_or_default();
    !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{number}")).is_ok()
}
