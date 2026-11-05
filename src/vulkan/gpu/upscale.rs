//! DLSS Ray Reconstruction, DLSS Super Resolution and XeSS on Vulkan (bridge/vk.cpp), as
//! D3D12's device.rs / frame.rs. FSR has no Vulkan path here: the vendored SDK ships only the
//! DX12 loader (roadmap).

use super::super::res::full_barrier;
use super::Gpu;
use crate::render::{Backend, Capabilities, Constants, SR_FRESH_FRAMES};
use crate::scene::{Settings, Upscaler};
use ash::vk;
use std::ffi::c_void;
use std::ptr::null_mut;

/// An image as the bridge takes it (`VkImageRef`); GENERAL layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ImageRef {
    image: vk::Image,
    view: vk::ImageView,
    format: vk::Format,
    width: u32,
    height: u32,
}

#[cfg(upscalers)]
mod ffi {
    use super::ImageRef;
    use ash::vk;
    use std::ffi::c_void;
    unsafe extern "C" {
        pub fn caustica_vk_upscaler_extensions(
            device: i32,
            instance: vk::Instance,
            phys: vk::PhysicalDevice,
            out: *mut *const std::ffi::c_char,
            cap: u32,
        ) -> u32;
        pub fn caustica_ngxvk_init(
            instance: vk::Instance,
            phys: vk::PhysicalDevice,
            device: vk::Device,
            gipa: vk::PFN_vkGetInstanceProcAddr,
            gdpa: vk::PFN_vkGetDeviceProcAddr,
            out: *mut *mut c_void,
        ) -> u32;
        pub fn caustica_ngxvk_caps(n: *mut c_void) -> u32;
        pub fn caustica_ngxvk_release(n: *mut c_void);
        pub fn caustica_ngxvk_destroy(n: *mut c_void);
        pub fn caustica_ngxvk_create(
            n: *mut c_void,
            cmd: vk::CommandBuffer,
            w: u32,
            h: u32,
            ow: u32,
            oh: u32,
            preset: u32,
        ) -> u32;
        pub fn caustica_ngxvk_eval(
            n: *mut c_void,
            cmd: vk::CommandBuffer,
            images: *const ImageRef,
            w: u32,
            h: u32,
            jx: f32,
            jy: f32,
            reset: i32,
            dt: f32,
            view: *const f32,
            projection: *const f32,
        ) -> u32;
        pub fn caustica_ngxvk_create_sr(
            n: *mut c_void,
            cmd: vk::CommandBuffer,
            w: u32,
            h: u32,
            ow: u32,
            oh: u32,
            preset: u32,
        ) -> u32;
        pub fn caustica_ngxvk_eval_sr(
            n: *mut c_void,
            cmd: vk::CommandBuffer,
            color: *const ImageRef,
            depth: *const ImageRef,
            motion: *const ImageRef,
            output: *const ImageRef,
            w: u32,
            h: u32,
            jx: f32,
            jy: f32,
            reset: i32,
            dt: f32,
        ) -> u32;
        pub fn caustica_xessvk_init(
            instance: vk::Instance,
            phys: vk::PhysicalDevice,
            device: vk::Device,
            out: *mut *mut c_void,
        ) -> u32;
        pub fn caustica_xessvk_destroy(x: *mut c_void);
        pub fn caustica_xessvk_create(x: *mut c_void, w: u32, ow: u32, oh: u32, scale: f32) -> u32;
        pub fn caustica_xessvk_eval(
            x: *mut c_void,
            cmd: vk::CommandBuffer,
            color: *const ImageRef,
            depth: *const ImageRef,
            motion: *const ImageRef,
            output: *const ImageRef,
            w: u32,
            h: u32,
            jx: f32,
            jy: f32,
            reset: i32,
        ) -> u32;
    }
}

/// The instance (`device` false) or device extensions NGX and XeSS ask for; empty off Windows.
pub fn required_extensions(
    device: bool,
    instance: vk::Instance,
    phys: vk::PhysicalDevice,
) -> Vec<&'static std::ffi::CStr> {
    #[cfg(upscalers)]
    unsafe {
        let mut names = [std::ptr::null(); 64];
        let n = ffi::caustica_vk_upscaler_extensions(
            device as i32,
            instance,
            phys,
            names.as_mut_ptr(),
            names.len() as u32,
        );
        names[..n as usize]
            .iter()
            .map(|&p| std::ffi::CStr::from_ptr(p))
            .collect()
    }
    #[cfg(not(upscalers))]
    {
        let _ = (device, instance, phys);
        vec![]
    }
}

/// The NGX runtimes NGX looks for beside the exe (build.rs copies them there).
#[cfg(upscalers)]
const NGX_RUNTIME: &str = if cfg!(windows) {
    "nvngx_dlss*.dll"
} else {
    "libnvidia-ngx-dlss*.so.*"
};

fn ngx_ok(code: u32) -> bool {
    code & 0xfff00000 != 0xbad00000
}

/// Upscaler handles and the per-session state D3D12 keeps on `Gpu`.
pub(super) struct Upscalers {
    ngx: *mut c_void,
    xess: *mut c_void,
    /// Why RR, DLSS SR, FSR and XeSS can't run (empty when they can).
    reasons: [String; 4],
    pub rr_created: bool,
    pub rr_failed: bool,
    pub sr_created: Option<Upscaler>,
    pub sr_failed: Vec<Upscaler>,
    pub sr_reset: bool,
    pub sr_fresh: u32,
}

impl Upscalers {
    /// Starts NGX and XeSS on `d` (each is null when it can't run here).
    pub unsafe fn new(d: &super::Device) -> Self {
        let fsr = "FSR has no Vulkan path in this build (the SDK ships only the DX12 loader)";
        #[allow(unused_mut)]
        let mut up = Self {
            ngx: null_mut(),
            xess: null_mut(),
            reasons: [
                "this build has no DLSS SDK (sh tools/setup.sh)".into(),
                "this build has no DLSS SDK (sh tools/setup.sh)".into(),
                fsr.into(),
                "Intel ships XeSS for Windows only (no libxess.so)".into(),
            ],
            rr_created: false,
            rr_failed: false,
            sr_created: None,
            sr_failed: vec![],
            sr_reset: true,
            sr_fresh: 0,
        };
        #[cfg(upscalers)]
        unsafe {
            let (instance, phys, device) = (d.instance.handle(), d.phys, d.device.handle());
            let code = ffi::caustica_ngxvk_init(
                instance,
                phys,
                device,
                d.entry.static_fn().get_instance_proc_addr,
                d.instance.fp_v1_0().get_device_proc_addr,
                &mut up.ngx,
            );
            let caps = if up.ngx.is_null() {
                0
            } else {
                ffi::caustica_ngxvk_caps(up.ngx)
            };
            let nvidia = d.name.to_ascii_lowercase().contains("nvidia");
            let ngx_reason = |bit: u32, what: &str| {
                if !ngx_ok(code) && !nvidia {
                    format!("needs an NVIDIA RTX GPU (this GPU: {})", d.name)
                } else if !ngx_ok(code) || up.ngx.is_null() {
                    format!(
                        "NGX did not start on Vulkan (0x{code:08x}); update the driver, and check that {NGX_RUNTIME} is next to the exe"
                    )
                } else if caps & bit == 0 {
                    format!("this GPU or driver does not offer {what}")
                } else {
                    String::new()
                }
            };
            up.reasons[0] = ngx_reason(1, "Ray Reconstruction");
            up.reasons[1] = ngx_reason(2, "DLSS Super Resolution");
            let xcode = ffi::caustica_xessvk_init(instance, phys, device, &mut up.xess);
            up.reasons[3] = match xcode {
                _ if !up.xess.is_null() => String::new(),
                #[cfg(windows)]
                1 => "libxess.dll is missing next to the exe".into(),
                #[cfg(not(windows))]
                1 => "Intel ships XeSS for Windows only (no libxess.so)".into(),
                2 => "libxess has no Vulkan entry points".into(),
                c => format!("XeSS did not start on Vulkan (0x{c:x})"),
            };
        }
        #[cfg(not(upscalers))]
        let _ = d;
        up
    }
    pub fn ngx_caps(&self) -> u32 {
        #[cfg(upscalers)]
        if !self.ngx.is_null() {
            return unsafe { ffi::caustica_ngxvk_caps(self.ngx) };
        }
        0
    }
    /// Releases the features (not the libraries) so the next frame recreates them.
    pub unsafe fn release(&mut self) {
        #[cfg(upscalers)]
        unsafe {
            ffi::caustica_ngxvk_release(self.ngx)
        };
        self.rr_created = false;
        self.rr_failed = false;
        self.sr_created = None;
        self.sr_failed.clear();
    }
    pub unsafe fn destroy(&mut self) {
        #[cfg(upscalers)]
        unsafe {
            ffi::caustica_ngxvk_destroy(std::mem::replace(&mut self.ngx, null_mut()));
            ffi::caustica_xessvk_destroy(std::mem::replace(&mut self.xess, null_mut()));
        }
    }
}

impl Gpu {
    fn image_ref(&self, i: usize) -> ImageRef {
        let t = &self.textures[i];
        ImageRef {
            image: t.handle,
            view: t.view,
            format: t.format,
            width: t.size[0],
            height: t.size[1],
        }
    }
    pub(super) fn rr_available(&self) -> bool {
        self.up.ngx_caps() & 1 != 0
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        match kind {
            Upscaler::Dlss => self.up.ngx_caps() & 2 != 0,
            Upscaler::Fsr => false,
            Upscaler::Xess => !self.up.xess.is_null(),
            Upscaler::Bilinear | Upscaler::Bicubic => true,
        }
    }
    pub fn upscaler_usable(&self, kind: Upscaler) -> bool {
        !kind.temporal() || (self.upscaler_available(kind) && !self.up.sr_failed.contains(&kind))
    }
    /// D3D12 parity (`gpu::Gpu::poll_ngx`): here NGX starts with the upscalers, nothing waits.
    pub fn wait_ngx(&mut self) {}
    pub fn capabilities(&self) -> Capabilities {
        let backend = |ok: bool, i: usize, failed: bool| Backend {
            available: ok && !failed,
            reason: if failed && ok {
                "failed while rendering (see the status line); tried again after a restart".into()
            } else {
                self.up.reasons[i].clone()
            },
        };
        let sr = |u: Upscaler, i: usize| {
            backend(
                self.upscaler_available(u),
                i,
                self.up.sr_failed.contains(&u),
            )
        };
        Capabilities {
            rr: backend(self.rr_available(), 0, self.up.rr_failed),
            dlss_sr: sr(Upscaler::Dlss, 1),
            fsr: sr(Upscaler::Fsr, 2),
            xess: sr(Upscaler::Xess, 3),
        }
    }
    pub unsafe fn reconfigure_upscalers(&mut self) -> anyhow::Result<()> {
        unsafe {
            self.device.device.device_wait_idle()?;
            self.up.release();
        }
        self.reset(true);
        Ok(())
    }

    /// Before the constants: whether RR runs this frame (it is created after `frame.begin`).
    pub(super) fn decide_rr(&mut self, s: &Settings) {
        self.st.rr_active = s.rr
            && self.rr_available()
            && !self.up.rr_failed
            && s.perspective()
            && !self.st.painting
            && !s.battle;
        if !s.rr && self.rr_available() {
            self.st.rr_status = "DLSS RR ready (disabled)".into();
        } else if !self.rr_available() {
            self.st.rr_status = format!("DLSS RR unavailable: {}", self.up.reasons[0]);
        }
        if self.st.rr_active && self.up.rr_created {
            self.st.rr_status = "DLSS Ray Reconstruction active".into();
        }
    }

    /// Before the constants: the temporal upscaler that runs this frame, if any (as D3D12's
    /// `prepare_upscaler`, minus the creation, which needs the command buffer).
    pub(super) fn decide_upscaler(&mut self, s: &Settings) -> Option<Upscaler> {
        let kind = s.upscaler;
        let st = &mut self.st;
        if !s.perspective() {
            st.upscale_status = "Upscaling needs a perspective camera; bicubic".into();
            return None;
        }
        if st.painting {
            st.upscale_status = "Light painting accumulates at render resolution".into();
            return None;
        }
        if s.battle {
            st.upscale_status = "Battle mode compares raw samples".into();
            return None;
        }
        if st.rr_active || (st.rw >= st.width && !(s.native_aa && kind.temporal())) {
            st.upscale_status.clear();
            return None;
        }
        if !kind.temporal() {
            st.upscale_status = format!("{} upscaling", kind.label());
            return None;
        }
        if !self.upscaler_available(kind) || self.up.sr_failed.contains(&kind) {
            if !self.up.sr_failed.contains(&kind) {
                self.st.upscale_status = format!("{} unavailable; bicubic upscaling", kind.label());
            }
            return None;
        }
        Some(kind)
    }

    /// The jitter and the accumulation rule with a temporal upscaler (see D3D12's `render`).
    /// Returns (jitter, accumulating).
    pub(super) fn upscale_jitter(&mut self, sr: Option<Upscaler>, draw: bool) -> ([f32; 2], bool) {
        let accumulating = sr.is_some() && self.up.sr_fresh >= SR_FRESH_FRAMES;
        if sr.is_some() && draw && !accumulating {
            if self.st.samples > 0 {
                let fresh = self.up.sr_fresh;
                self.reset(false);
                self.up.sr_fresh = fresh;
            }
            self.up.sr_fresh += 1;
        }
        let jitter = if self.st.rr_active || sr.is_some() {
            let i = self.st.frame % 64 + 1;
            [
                crate::render::halton(i, 2) - 0.5,
                crate::render::halton(i, 3) - 0.5,
            ]
        } else {
            [0., 0.]
        };
        (jitter, accumulating)
    }

    /// After `frame.begin`: creates RR or the upscaler when they aren't yet. False when the
    /// one decided on failed to start (the frame then shows bicubic).
    pub(super) unsafe fn create_upscalers(&mut self, s: &Settings, sr: Option<Upscaler>) -> bool {
        #[cfg(not(upscalers))]
        {
            let _ = (s, sr);
            true
        }
        #[cfg(upscalers)]
        unsafe {
            let cmd = self.frame.cmd;
            let (w, h, ow, oh) = (self.st.rw, self.st.rh, self.st.width, self.st.height);
            if self.st.rr_active && !self.up.rr_created {
                let code = ffi::caustica_ngxvk_create(self.up.ngx, cmd, w, h, ow, oh, s.rr_preset);
                if ngx_ok(code) {
                    self.up.rr_created = true;
                    self.st.rr_status = "DLSS Ray Reconstruction active".into();
                } else {
                    self.st.rr_active = false;
                    self.up.rr_failed = true;
                    self.st.rr_status =
                        format!("RR creation failed: 0x{code:08x}; native denoiser active");
                    eprintln!("{}", self.st.rr_status);
                    return false;
                }
            }
            let Some(kind) = sr else { return true };
            if self.up.sr_created == Some(kind) {
                return true;
            }
            let (ok, code) = match kind {
                Upscaler::Dlss => {
                    let code = ffi::caustica_ngxvk_create_sr(
                        self.up.ngx,
                        cmd,
                        w,
                        h,
                        ow,
                        oh,
                        s.dlss_preset,
                    );
                    (ngx_ok(code), code)
                }
                _ => {
                    let code = ffi::caustica_xessvk_create(self.up.xess, w, ow, oh, 1.);
                    (code == 0, code)
                }
            };
            if ok {
                self.up.sr_created = Some(kind);
                self.up.sr_reset = true;
                self.st.upscale_status = match kind {
                    Upscaler::Dlss => format!(
                        "{} active · preset {}",
                        kind.label(),
                        crate::scene::DLSS_PRESETS
                            .iter()
                            .find(|(id, _)| *id == s.dlss_preset)
                            .map_or("default", |(_, l)| l)
                    ),
                    _ => format!("{} active", kind.label()),
                };
                true
            } else {
                self.up.sr_failed.push(kind);
                self.st.upscale_status = format!(
                    "{} failed to start (0x{code:08x}); bicubic upscaling",
                    kind.label()
                );
                eprintln!("{}", self.st.upscale_status);
                false
            }
        }
    }

    /// Ray Reconstruction into texture 14 (as D3D12's `render`).
    pub(super) unsafe fn run_rr(
        &mut self,
        s: &Settings,
        jitter: [f32; 2],
        reset: bool,
        dt: f32,
        view: glam::Mat4,
        proj: glam::Mat4,
    ) {
        #[cfg(not(upscalers))]
        let _ = (s, jitter, reset, dt, view, proj);
        #[cfg(upscalers)]
        unsafe {
            let images: Vec<ImageRef> = (0..self.textures.len())
                .map(|i| self.image_ref(i))
                .collect();
            let code = ffi::caustica_ngxvk_eval(
                self.up.ngx,
                self.frame.cmd,
                images.as_ptr(),
                self.st.rw,
                self.st.rh,
                jitter[0],
                jitter[1],
                (reset || !s.temporal) as i32,
                dt * 1000.,
                view.to_cols_array().as_ptr(),
                proj.to_cols_array().as_ptr(),
            );
            full_barrier(&self.device.device, self.frame.cmd);
            if !ngx_ok(code) {
                self.st.rr_active = false;
                self.up.rr_failed = true;
                self.st.rr_status = format!("RR evaluation failed: 0x{code:08x}");
                eprintln!("{}", self.st.rr_status);
            }
        }
    }

    /// Upscales the native colour (denoised or raw) with depth and motion into texture 14.
    pub(super) unsafe fn run_upscaler(
        &mut self,
        kind: Upscaler,
        s: &Settings,
        c: &mut Constants,
        jitter: [f32; 2],
        history_reset: i32,
        dt: f32,
    ) {
        #[cfg(not(upscalers))]
        let (code, _) = (0u32, (kind, s, &c, jitter, history_reset, dt));
        #[cfg(upscalers)]
        let code = unsafe {
            let color = self.image_ref(if s.denoise { 8 } else { 1 });
            let (depth, motion, out) = (self.image_ref(25), self.image_ref(6), self.image_ref(14));
            let (cmd, w, h) = (self.frame.cmd, self.st.rw, self.st.rh);
            let code = match kind {
                Upscaler::Dlss => {
                    let code = ffi::caustica_ngxvk_eval_sr(
                        self.up.ngx,
                        cmd,
                        &color,
                        &depth,
                        &motion,
                        &out,
                        w,
                        h,
                        jitter[0],
                        jitter[1],
                        history_reset,
                        dt * 1000.,
                    );
                    if ngx_ok(code) { 0 } else { code }
                }
                _ => ffi::caustica_xessvk_eval(
                    self.up.xess,
                    cmd,
                    &color,
                    &depth,
                    &motion,
                    &out,
                    w,
                    h,
                    jitter[0],
                    jitter[1],
                    history_reset,
                ),
            };
            full_barrier(&self.device.device, cmd);
            code
        };
        self.up.sr_reset = false;
        if code != 0 {
            self.up.sr_failed.push(kind);
            self.up.sr_created = None;
            c.scene_info[3] &= !2;
            self.st.upscale_status =
                format!("{} failed (0x{code:08x}); bicubic upscaling", kind.label());
            eprintln!("{}", self.st.upscale_status);
        }
    }
}
