//! Backend-neutral renderer types: what the app reads from a renderer (`GpuState`) and the
//! values it exchanges with one, whichever backend draws (D3D12 `gpu`, Vulkan `vulkan::gpu`).
pub mod plan;
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Constants {
    pub inverse_vp: [f32; 16],
    pub current_vp: [f32; 16],
    pub previous_vp: [f32; 16],
    pub world_view: [f32; 16],
    pub eye: [f32; 4],
    pub dimensions: [u32; 4],
    pub sampling: [u32; 4],
    /// x: suns at the start of the emitter table (track 7); y: pixel filter (0 box, 1 Gaussian,
    /// 2 Blackman-Harris), z: its width in pixels; w: texture anisotropy (1–16).
    pub lights: [f32; 4],
    pub light_color_aspect: [f32; 4],
    pub background: [f32; 4],
    pub display: [f32; 4],
    pub modes: [u32; 4],
    pub options: [f32; 4],
    pub jitter: [f32; 4],
    pub filter: [u32; 4],
    pub scene_info: [u32; 4],
    pub environment: [f32; 4],
    pub hdr: [f32; 4],
    pub tools: [f32; 4],
    pub lens: [f32; 4],
    pub camera_model: [f32; 4],
    pub look: [f32; 4],
    pub lift: [f32; 4],
    pub grade_gamma: [f32; 4],
    pub gain: [f32; 4],
    pub environment_map: [f32; 4],
    /// Track 12 fog: density, phase g, top height, half extent on x / z.
    pub fog: [f32; 4],
    /// Fog albedo (rgb).
    pub fog_color: [f32; 4],
    /// Track 14, read only by the path-recording trace: render pixel x, y, paths, capacity.
    pub path_record: [u32; 4],
    /// Track 14: light path expression slots in use (0 = off), the slot the LPE view shows.
    pub lpe: [u32; 4],
    /// Track 15 ReSTIR DI: flags (bit 0 on, bit 1 history valid), candidates, spatial
    /// neighbours, history cap.
    pub restir: [u32; 4],
    /// Track 15: the render pixels a dispatch covers (x0, y0, x1, y1).
    pub region: [u32; 4],
    /// Track 15 (integrators.hlsl): photons this frame, photon capacity, splat offset in `aux`
    /// (bytes), flags (bit 0 light tracing).
    pub integrator: [u32; 4],
    /// Gather radius, image-plane area at distance 1 times the region's share.
    pub photon: [f32; 4],
    /// The scene's bounding sphere (centre, radius).
    pub bounds: [f32; 4],
    /// Track 16 lens system: surfaces (0 = off), dispersion, scene units per mm, -.
    pub real_lens: [f32; 4],
    /// Track 16 glare pass: mode, region, flags, image size in the grid (w | h << 16).
    pub glare: [u32; 4],
    /// Glare strength (0 = off), threshold, aperture diameter in grid texels, -.
    pub glare_settings: [f32; 4],
    /// The HDRI's split sun (`environment::Sun`): world direction, cosine of the half angle
    /// (0 = none); radiance times the HDRI strength, the share of environment samples.
    pub env_sun: [f32; 4],
    pub env_sun_color: [f32; 4],
    /// Ground projection height (0 = off), the floor's height, shadow catcher (1 / 0), -.
    pub env_ground: [f32; 4],
    /// Heterogeneous fog: noise amount (0 = uniform), feature size, -, -.
    pub fog_noise: [f32; 4],
}

/// What `constants` needs beyond the settings and `GpuState`: the camera, this frame's sample
/// count and flags, and backend state that isn't public.
pub struct FrameInputs {
    pub view: glam::Mat4,
    pub proj: glam::Mat4,
    pub eye: glam::Vec3,
    pub previous_vp: glam::Mat4,
    /// Samples this frame.
    pub n: u32,
    pub reset: bool,
    pub flags: u32,
    pub jitter: [f32; 2],
    pub sequence_base: u32,
    pub lut_size: u32,
    pub present_count: u32,
    pub env_scale: f32,
    pub env_inv_norm: f32,
    pub env_sun: Option<(crate::environment::Sun, f32)>,
    /// Environment strength, rotation (radians) and visibility.
    pub env: (f32, f32, bool),
    pub lens_surfaces: u32,
    pub glare_grid: u32,
    /// ReSTIR word: on, history valid << 1, layers << 8.
    pub restir: u32,
}

/// The `Constants` block of one frame (common.hlsli `cbuffer Frame`), the same for every
/// backend. The trace's region, integrator, photon, bounds, glare and path-record fields stay
/// for the passes to set.
pub fn constants(s: &crate::scene::Settings, st: &GpuState, x: &FrameInputs) -> Constants {
    let (view, proj, eye) = (x.view, x.proj, x.eye);
    let vp = proj * view;
    let (n, reset, flags, jitter) = (x.n, x.reset, x.flags, x.jitter);
    let (env_strength, env_rotation, env_visible) = x.env;
    let lc = s.light_color();
    Constants {
        inverse_vp: vp.inverse().to_cols_array(),
        current_vp: vp.to_cols_array(),
        previous_vp: x.previous_vp.to_cols_array(),
        world_view: view.to_cols_array(),
        eye: [eye.x, eye.y, eye.z, 1.],
        dimensions: [st.rw, st.rh, st.width, st.height],
        sampling: [st.samples, n, s.bounces, st.frame],
        lights: [
            st.sun_count as f32,
            s.pixel_filter as f32,
            s.filter_width,
            s.anisotropy as f32,
        ],
        light_color_aspect: [lc[0], lc[1], lc[2], s.light_aspect],
        background: [s.background[0], s.background[1], s.background[2], 0.],
        display: [s.exposure, s.gamma, s.direct, s.indirect],
        modes: [
            s.view,
            s.tonemap,
            (s.denoise && !s.battle) as u32,
            st.rr_active as u32,
        ],
        options: [
            s.firefly,
            s.denoise_strength,
            s.compare as u32 as f32,
            s.split,
        ],
        jitter: [
            jitter[0],
            jitter[1],
            s.sobol as u32 as f32,
            s.glass_shadow as u32 as f32,
        ],
        filter: [
            0,
            s.temporal as u32,
            reset as u32,
            st.reference_valid as u32,
        ],
        scene_info: [
            st.emitter_count as u32,
            s.objects.len() as u32,
            x.sequence_base,
            flags,
        ],
        // View-space ray scale (tan of the half field of view) for the denoiser.
        // w: the medium the camera is in (underwater), a stack entry or 0.
        environment: [
            s.sky,
            1. / proj.x_axis.x,
            1. / proj.y_axis.y,
            crate::geometry::camera_medium(s, eye) as f32,
        ],
        hdr: [
            st.hdr_active as u32 as f32,
            st.hdr_levels[0],
            st.hdr_levels[1],
            (st.hdr_active && st.hdr10) as u32 as f32,
        ],
        tools: [
            st.zebras as u32 as f32,
            st.scopes_on as u32 as f32,
            s.bounces.min(8) as f32,
            256.,
        ],
        lens: [
            s.aperture,
            s.focus_distance,
            s.aperture_blades as f32,
            s.blade_rotation.to_radians(),
        ],
        camera_model: [
            s.projection as f32,
            s.cat_eye,
            s.tilt.to_radians(),
            s.distance * (s.fov.to_radians() * 0.5).tan(),
        ],
        look: [s.halation, s.grain, s.vignette, x.lut_size as f32],
        lift: [
            s.lift[0],
            s.lift[1],
            s.lift[2],
            [1., 2., 3.][s.grain_size.min(2) as usize],
        ],
        grade_gamma: [
            s.grade_gamma[0],
            s.grade_gamma[1],
            s.grade_gamma[2],
            s.lut_strength,
        ],
        gain: [
            s.gain[0],
            s.gain[1],
            s.gain[2],
            (x.present_count % 4096) as f32,
        ],
        environment_map: [
            env_strength * x.env_scale,
            env_rotation,
            if x.env_inv_norm > 0. {
                1. + if env_visible { 2. } else { 0. }
            } else {
                0.
            },
            x.env_inv_norm * x.env_scale,
        ],
        fog: [s.fog.density, s.fog.phase, s.fog.height, s.fog.extent],
        // w: the extinction tint (`lighting::pack_tint`; 0 = grey).
        fog_color: [
            s.fog.color[0],
            s.fog.color[1],
            s.fog.color[2],
            crate::lighting::pack_tint(s.fog.extinction),
        ],
        region: [0, 0, st.rw, st.rh],
        integrator: [0; 4],
        photon: [0.; 4],
        bounds: [0.; 4],
        real_lens: [
            x.lens_surfaces as f32,
            s.dispersion,
            0.001 / s.lens_scale,
            0.,
        ],
        glare: [0, 0, 0, x.glare_grid],
        // A smaller aperture in the grid spreads the pattern wider.
        glare_settings: [s.glare, s.glare_threshold, 256. - 232. * s.glare_size, 0.],
        env_sun: match x.env_sun.filter(|_| x.env_inv_norm > 0.) {
            Some((sun, _)) => {
                let d = sun.direction(env_rotation);
                [d[0], d[1], d[2], sun.om]
            }
            None => [0.; 4],
        },
        env_sun_color: match x.env_sun.filter(|_| x.env_inv_norm > 0.) {
            Some((sun, share)) => {
                let r = sun.radiance.map(|v| v * env_strength);
                [r[0], r[1], r[2], share]
            }
            None => [0.; 4],
        },
        // The floor quad sits at y = −1 (geometry::static_geometry).
        // Ground projection only for a loaded HDRI (the physical sky replaces it).
        env_ground: [
            if s.physical_sky.on { 0. } else { s.hdri_ground },
            -1.,
            s.shadow_catcher as u32 as f32,
            0.,
        ],
        fog_noise: [s.fog.noise, s.fog.noise_size, 0., 0.],
        path_record: [0; 4],
        lpe: [st.lpe_slots, s.lpe_slot, 0, 0],
        restir: [
            x.restir,
            s.restir_candidates.clamp(1, 32),
            s.restir_neighbors.min(8),
            s.restir_history.min(64),
        ],
    }
}

pub const TEXTURES: usize = 29;
/// Constants are written once per pass at this stride (trace at 0, filter passes 1-7, display 8).
pub const CB_STRIDE: u64 = 1024;
/// Constant slots: trace 0, filter 1–7, display 8, battle quadrants 9–12, glare passes from 13.
pub const CB_SLOTS: u64 = 13 + 10;

/// Storage texture formats (UAV u0 to u28), per backend mapped to DXGI / Vulkan formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexFormat {
    Rgba32F,
    R32F,
    Rg32F,
    Rgba8,
    Rgb10A2,
    Rgba16F,
    R32U,
}

/// Format and size of texture `i` (docs/claude/data-layouts.md "UAV textures"): output-size
/// textures (10, 13, 14, 18, 26), the fixed scopes counters (28), render size for the rest.
pub fn texture_layout(
    i: usize,
    hdr10: bool,
    output: [u32; 2],
    render: [u32; 2],
) -> (TexFormat, [u32; 2]) {
    let format = match i {
        5 | 7 | 17 | 19 | 20 | 25 => TexFormat::R32F,
        6 => TexFormat::Rg32F,
        10 => TexFormat::Rgba8,
        // HDR10 writes PQ code values that copy straight into the backbuffer.
        26 if hdr10 => TexFormat::Rgb10A2,
        26 => TexFormat::Rgba16F,
        27 => TexFormat::Rg32F,
        28 => TexFormat::R32U,
        _ => TexFormat::Rgba32F,
    };
    let size = if [10, 13, 14, 18, 26].contains(&i) {
        output
    } else if i == 28 {
        SCOPES
    } else {
        render
    };
    (format, size)
}
