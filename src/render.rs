//! Backend-neutral renderer types: what the app reads from a renderer (`GpuState`) and the
//! values it exchanges with one, whichever backend draws (D3D12 `gpu`, Vulkan `vulkan::gpu`).
use crate::scene::Upscaler;
/// Size of the scopes counter texture (u28).
pub const SCOPES: [u32; 2] = [256, 260];
/// A light-meter / click-to-focus reading at one pixel: linear beauty, diffuse albedo and
/// linear view depth.
#[derive(Clone, Copy)]
pub struct Probe {
    pub linear: [f32; 4],
    pub albedo: [f32; 4],
    pub depth: f32,
    /// First-hit material id + 1 (u19), 0 for a miss: what a click selects.
    pub id: u32,
}
/// Battle mode quadrants (track 15): top left, top right, bottom left, bottom right.
pub const BATTLE_NAMES: [&str; 4] = ["Path tracing", "Path + ReSTIR DI", "BDPT", "Photon mapping"];
/// Per-quadrant state of battle mode: samples, GPU time spent (ms), measured ms per sample
/// (0 = not measured yet) and the fractional samples carried to the next frame.
#[derive(Clone, Copy, Default)]
pub struct BattleQuadrant {
    pub samples: u32,
    pub ms: f64,
    pub cost: f64,
    pub(crate) carry: f64,
    /// Samples dispatched this frame (0 = skipped).
    pub(crate) last: u32,
    /// Photon mapping's work per sample this frame: its photon count's multiple (1 elsewhere).
    pub(crate) scale: f64,
}
/// One layer of a multi-layer EXR (`Gpu::read_layers`): its name, size,
/// channel names, and interleaved samples.
pub struct AovLayer {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub channels: Vec<&'static str>,
    pub data: Vec<f32>,
}
/// One reconstruction backend: usable, or why not ("needs an NVIDIA RTX GPU", a missing DLL,
/// an init error code, a failure while rendering).
#[derive(Clone, Debug, Default)]
pub struct Backend {
    pub available: bool,
    pub reason: String,
}
/// Every reconstruction backend's availability, from `Gpu::capabilities`.
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub rr: Backend,
    pub dlss_sr: Backend,
    pub fsr: Backend,
    pub xess: Backend,
}
impl Capabilities {
    /// The backend behind a temporal upscaler; `None` for the spatial ones (always available).
    pub fn upscaler(&self, u: Upscaler) -> Option<&Backend> {
        match u {
            Upscaler::Dlss => Some(&self.dlss_sr),
            Upscaler::Fsr => Some(&self.fsr),
            Upscaler::Xess => Some(&self.xess),
            Upscaler::Bilinear | Upscaler::Bicubic => None,
        }
    }
}
/// What the app reads from a renderer, whichever backend draws (engine-notes "Vulkan backend:
/// design"). `Gpu` derefs to it, so `g.width` and `self.width` keep working.
#[derive(Default)]
pub struct GpuState {
    /// One-shot: record paths through this output pixel in the next `render` (pixel, paths).
    pub path_request: Option<([u32; 2], u32)>,
    /// The records of the last request (the count record stripped), until the app takes them.
    pub path_result: Option<Vec<crate::paths::Record>>,
    pub lpe_slots: u32,
    /// Battle mode (track 15), reset with the accumulation.
    pub battle: [BattleQuadrant; 4],
    /// The swapchain is RGBA16F scRGB (1.0 = 80 nits) and shows texture 26.
    pub hdr_active: bool,
    /// With `hdr_active`: the swapchain is HDR10 (R10G10B10A2, PQ / BT.2020) instead of scRGB.
    pub hdr10: bool,
    /// The swapchain refused HDR10 once this session; HDR then stays scRGB.
    pub hdr10_refused: bool,
    /// Paper white and panel peak in nits for the HDR transform (set by the app each frame).
    pub hdr_levels: [f32; 2],
    /// Light painting (roadmap 5d): progressive path only, and scene edits keep the accumulation.
    pub painting: bool,
    /// Look tools (roadmap 5b): zebras, scopes (read back into `scopes` each frame) and a
    /// light-meter probe at an output pixel (read back into `probe_result`: linear beauty and
    /// diffuse albedo).
    pub zebras: bool,
    pub scopes_on: bool,
    pub scopes: Vec<u32>,
    pub probe: Option<[u32; 2]>,
    pub probe_result: Option<Probe>,
    pub triangle_count: usize,
    pub emitter_count: usize,
    /// Suns lead the emitter table (`lighting::analytic`).
    pub sun_count: u32,
    /// When set, each frame's backbuffer (with UI) is copied here before Present, because a
    /// FLIP_DISCARD buffer's contents are undefined once presented. Kept in COPY_DEST.
    pub keep_ui_frame: bool,
    pub upscale_status: String,
    pub rr_status: String,
    pub rr_active: bool,
    /// Output size: the size of the output textures (10, 13, 14, 18, 26). Equal to the
    /// swapchain except during a video export (`resize_output`).
    pub width: u32,
    pub height: u32,
    /// Swapchain (window) size.
    pub swap_size: [u32; 2],
    pub rw: u32,
    pub rh: u32,
    pub samples: u32,
    pub frame: u32,
    pub gpu_ms: f64,
    /// Total time the CPU has spent waiting on the fence in `submit` (ms): the frame time
    /// minus this is the CPU's own share of each frame.
    pub fence_wait_ms: f64,
    /// GPU time of each pass this frame (ms): trace, reconstruction (native denoiser, RR or
    /// upscaler), display, UI.
    pub pass_ms: [f64; 4],
    pub adapter_name: String,
    /// What the denoiser did on the last frame, in words (the status bar, Advanced on).
    pub denoiser_activity: &'static str,
    pub reference_valid: bool,
}
/// Round-to-nearest-even f32 → IEEE half; values past 65504 become infinity.
pub fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if v.is_nan() {
        return sign | 0x7e00;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        // Subnormal: shift the implicit bit in, then round.
        let m = mantissa | 0x80_0000;
        let shift = (14 - exponent) as u32;
        let half = 1 << (shift - 1);
        let rounded = (m + half - 1 + ((m >> shift) & 1)) >> shift;
        return sign | rounded as u16;
    }
    let rounded = mantissa + 0xfff + ((mantissa >> 13) & 1);
    let (exponent, mantissa) = if rounded & 0x80_0000 != 0 {
        (exponent + 1, 0)
    } else {
        (exponent, rounded >> 13)
    };
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    sign | ((exponent as u16) << 10) | mantissa as u16
}
pub(crate) fn half_to_f32(h: u16) -> f32 {
    let (sign, exponent, mantissa) = ((h >> 15) as u32, ((h >> 10) & 31) as i32, (h & 1023) as f32);
    let magnitude = match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1. + mantissa / 1024.) * 2f32.powi(exponent - 15),
    };
    if sign == 1 { -magnitude } else { magnitude }
}
