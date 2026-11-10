//! Backend-neutral renderer types: what the app reads from a renderer (`GpuState`) and the
//! values it exchanges with one, whichever backend draws (D3D12 `gpu`, Vulkan `vulkan::gpu`).
pub mod plan;
pub mod progress;
use crate::scene::{Denoiser, Upscaler};
/// Size of the scopes counter texture (u28).
pub const SCOPES: [u32; 2] = [256, 260];
/// u28's two rows under the scopes (which clear and read back only `SCOPES`): the exposure
/// meter's histogram (row 260: weighted counts in 0-127, in-bin steps in 128-255) and its state
/// (row 261: the group counter at 0, then from column 8 the `meter_state` words). post.hlsl
/// mirrors the rows and the magic.
// No Rust reader (only the state row is read back); it names the row post.hlsl writes.
#[allow(dead_code)]
pub const METER_ROW: u32 = 260;
pub const METER_STATE_ROW: u32 = 261;
pub const METER_MAGIC: u32 = 0x4d45_5452;
/// u28's full size.
pub const SCOPES_TEXTURE: [u32; 2] = [SCOPES[0], SCOPES[1] + 2];
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

/// An LPE layer's EXR name: `lpe<slot>_` and the expression with everything but letters and
/// digits as `_` (a dot would nest layers), at most 48 characters.
pub(crate) fn lpe_layer_name(slot: u32, expr: Option<&String>) -> String {
    let mut name = format!("lpe{}", slot + 1);
    if let Some(e) = expr {
        let mut last = '_';
        let mut tail = String::new();
        for c in e.chars() {
            let c = if c.is_ascii_alphanumeric() { c } else { '_' };
            if c != '_' || last != '_' {
                tail.push(c);
            }
            last = c;
        }
        let tail = tail.trim_matches('_');
        if !tail.is_empty() {
            name = format!("{name}_{tail}");
        }
    }
    name.chars().take(48).collect()
}
/// The AOV layers both backends write to a multi-layer EXR (before LPE layers), from
/// `read(index)` = a float texture's (width, height, channels, samples).
/// The numbers of a `static const` table in shader source (`name[...] = { ... };`), in order;
/// the CPU reference and the Vulkan table buffers read the shaders' own tables.
pub fn hlsl_table(src: &str, name: &str) -> Vec<f32> {
    let start = src
        .find(&format!(" {name}["))
        .unwrap_or_else(|| panic!("{name} in the shader source"));
    let body = &src[start..];
    let body = &body[body.find('{').unwrap() + 1..body.find("};").unwrap()];
    let body = body.replace("float4", " ").replace("float3", " ");
    body.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
        .filter(|t| !t.is_empty() && t.chars().any(|c| c.is_ascii_digit()))
        .filter_map(|t| t.trim_end_matches('.').parse().ok())
        .collect()
}

pub fn aov_layers(
    mut read: impl FnMut(usize) -> anyhow::Result<(u32, u32, usize, Vec<f32>)>,
) -> anyhow::Result<Vec<AovLayer>> {
    let mut layers = vec![];
    let mut add = |name: &str,
                   (w, h, n, data): (u32, u32, usize, Vec<f32>),
                   pick: &[(usize, &'static str)]| {
        let data = data
            .chunks_exact(n)
            .flat_map(|t| pick.iter().map(|(c, _)| t[*c]).collect::<Vec<_>>())
            .collect();
        layers.push(AovLayer {
            name: name.into(),
            width: w,
            height: h,
            channels: pick.iter().map(|(_, c)| *c).collect(),
            data,
        });
    };
    const RGB: [(usize, &str); 3] = [(0, "R"), (1, "G"), (2, "B")];
    add(
        "beauty",
        read(18)?,
        &[(0, "R"), (1, "G"), (2, "B"), (3, "A")],
    );
    add("diffuse_albedo", read(3)?, &RGB);
    add("specular_albedo", read(4)?, &RGB);
    let normal = read(2)?;
    add("normal", normal.clone(), &[(0, "X"), (1, "Y"), (2, "Z")]);
    add("roughness", normal, &[(3, "Y")]);
    add("depth", read(5)?, &[(0, "Z")]);
    add("motion", read(6)?, &[(0, "X"), (1, "Y")]);
    add("direct", read(11)?, &RGB);
    add("specular", read(12)?, &RGB);
    add("object_id", read(19)?, &[(0, "Y")]);
    Ok(layers)
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
    /// The backend behind a denoiser; `None` for the ones that always run (Off, Native).
    pub fn denoiser(&self, d: Denoiser) -> Option<&Backend> {
        match d {
            Denoiser::Rr => Some(&self.rr),
            Denoiser::Native | Denoiser::Off => None,
        }
    }
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
    /// The swapchain offered no HDR format (a Vulkan surface without scRGB, or a DXGI colour
    /// space refused): HDR stays off and is not asked for again until the display changes. Each
    /// try resizes the output, so asking every frame reset accumulation every frame.
    pub hdr_refused: bool,
    /// Paper white and panel peak in nits for the HDR transform (set by the app each frame).
    pub hdr_levels: [f32; 2],
    /// Wayland: the compositor's reference white in nits, 0 elsewhere. It treats an HDR surface
    /// as relative, showing a signal of 203 nits at its reference, so `hdr_signal` rescales.
    pub hdr_reference: f32,
    /// Keep the exposure: scene edits keep the accumulated sums, and RR and temporal upscaling
    /// are off. Set by the motion-blur video export for the passes of one frame; progressive path only.
    pub keep_exposure: bool,
    /// Look tools (roadmap 5b): zebras, scopes (read back into `scopes` each frame) and a
    /// light-meter probe at an output pixel (read back into `probe_result`: linear beauty and
    /// diffuse albedo).
    pub zebras: bool,
    pub scopes_on: bool,
    pub scopes: Vec<u32>,
    pub probe: Option<[u32; 2]>,
    pub probe_result: Option<Probe>,
    /// The exposure meter's step for the next `render` (E3), set by the app.
    pub meter: MeterStep,
    /// The meter's state after the last frame (read back every frame; `None` before a reading).
    pub metered: Option<crate::reference::display::Adaptation>,
    /// The pre-exposure the reconstruction inputs were last scaled by (E5; 0 before the first).
    pub pre_exposure: f32,
    pub triangle_count: usize,
    pub emitter_count: usize,
    /// Suns lead the emitter table (`lighting::analytic`).
    pub sun_count: u32,
    /// When set, each frame's backbuffer (with UI) is copied here before Present, because a
    /// FLIP_DISCARD buffer's contents are undefined once presented. Kept in COPY_DEST.
    pub keep_ui_frame: bool,
    /// A `--headless` run: no swap chain image is acquired or presented (the compositor would
    /// throttle the presents to the monitor rate); the display pass and the UI draw into an
    /// offscreen image of the swap chain's format and size, so captures are the same.
    pub headless: bool,
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
    /// The device traces. Without it the GPU only displays (and draws the UI): the CPU
    /// renderer fills the view, and `render` traces nothing.
    pub ray_tracing: bool,
    /// What the denoiser did on the last frame, in words (the status bar, Advanced on).
    pub denoiser_activity: &'static str,
    pub reference_valid: bool,
}
impl GpuState {
    /// Paper white and peak as the HDR output encodes them: `hdr_levels`, relative to the
    /// reference white on Wayland (`hdr_reference`). PQ ends at 10000 nits, so at a low reference
    /// the peak stops there and the transform rolls off to it instead of clipping.
    pub fn hdr_signal(&self) -> [f32; 2] {
        let scale = if self.hdr_reference > 0. {
            203. / self.hdr_reference
        } else {
            1.
        };
        let [white, peak] = self.hdr_levels;
        [white * scale, peak * scale].map(|nits| nits.min(10000.))
    }
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
    /// Night sky N3 (stars.rs `Frame::rows`): the rows of the J2000-to-world rotation with,
    /// in w, the irradiance scale, the stars' share of sky light samples (0 = no light
    /// sampling) and the camera cone's chord; then the zenith optical depth, w 1 while stars
    /// are on (0 = off).
    pub star_x: [f32; 4],
    pub star_y: [f32; 4],
    pub star_z: [f32; 4],
    pub star_tau: [f32; 4],
    /// The limiting magnitude's kept star count and their CDF mass (stars.rs `Frame::rows`).
    pub star_limit: [f32; 4],
    /// x the specular bounce budget (`Settings::specular_bounces`, 0 = legacy), y z w unused.
    pub bounce_split: [u32; 4],
    /// Exposure meter (E3): dt (simulated seconds), ev_bias (stops), exposure_speed (1/s), the Eyes
    /// look's strength (E4, 0 = off).
    pub meter: [f32; 4],
    /// Flags (bit 0 auto exposure, bit 1 this dispatch is the meter pass, bit 2 commit), the
    /// `MeterOp` code, the meter mode (0 centre-weighted, 1 spot, 2 histogram), -.
    pub meter_ctrl: [u32; 4],
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
    /// The environment's stars (`EnvMap::stars`): their share of sky samples, `None` = off.
    pub env_stars: Option<f32>,
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
    let stars = match x.env_stars {
        Some(share) if s.physical_sky.on && x.env_inv_norm > 0. => {
            let f = crate::stars::Frame::new(
                &s.physical_sky,
                share,
                crate::stars::chord(s.fov, st.height, s.physical_sky.star_size),
            );
            // A limit that keeps no star, before the sky is rebuilt: off, never a zero count
            // beside the old share (the shader's pick would read before the table).
            if f.count == 0 { [[0.; 4]; 5] } else { f.rows() }
        }
        _ => [[0.; 4]; 5],
    };
    Constants {
        inverse_vp: vp.inverse().to_cols_array(),
        current_vp: vp.to_cols_array(),
        previous_vp: x.previous_vp.to_cols_array(),
        world_view: view.to_cols_array(),
        eye: [eye.x, eye.y, eye.z, 0.],
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
            (s.native_filter() && !s.battle) as u32,
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
            st.hdr_signal()[0],
            st.hdr_signal()[1],
            (st.hdr_active && st.hdr10) as u32 as f32,
        ],
        tools: [
            st.zebras as u32 as f32,
            st.scopes_on as u32 as f32,
            (s.bounces + s.specular_bounces).min(8) as f32,
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
            // Accuracy F1: 1 reads textures at mip 0.
            s.lod0_textures as u32 as f32,
        ],
        // z: accuracy B1, 1 = gentle Russian roulette (start 8, floor 0.2).
        fog_noise: [
            s.fog.noise,
            s.fog.noise_size,
            s.gentle_roulette as u32 as f32,
            0.,
        ],
        star_x: stars[0],
        star_y: stars[1],
        star_z: stars[2],
        star_tau: stars[3],
        star_limit: stars[4],
        bounce_split: [s.specular_bounces, 0, 0, 0],
        path_record: [0; 4],
        lpe: [st.lpe_slots, s.lpe_slot, 0, 0],
        restir: [
            x.restir,
            s.restir_candidates.clamp(1, 32),
            s.restir_neighbors.min(8),
            s.restir_history.min(64),
        ],
        meter: [
            st.meter.dt,
            s.ev_bias,
            s.exposure_speed,
            if s.eyes { s.eyes_strength } else { 0. },
        ],
        meter_ctrl: [
            s.auto_exposure as u32 | (st.meter.commit as u32) << 2,
            st.meter.op.code(),
            s.meter_mode as u32,
            0,
        ],
    }
}
/// The meter's step for the next `render` (set by the app each frame; `MeterPlan` decides it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterStep {
    pub op: crate::reference::display::MeterOp,
    /// Start this step's adaptation from where the last one ended (false for the later passes
    /// of one video frame, which redo the frame's step on a better image).
    pub commit: bool,
    /// Simulated seconds this step covers.
    pub dt: f32,
}
impl MeterStep {
    /// Whether `render` runs the meter pass (auto exposure, and something to do).
    pub fn runs(&self, s: &crate::scene::Settings) -> bool {
        s.auto_exposure && self.op != crate::reference::display::MeterOp::Freeze
    }
    /// The meter pass's dispatch: one thread per `METER_STEP`² cell, 8 x 8 per group; a hold
    /// needs one thread.
    pub fn groups(&self, width: u32, height: u32) -> [u32; 3] {
        use crate::reference::display::{METER_STEP, MeterOp};
        if self.op == MeterOp::Hold {
            return [1, 1, 1];
        }
        [
            width.div_ceil(METER_STEP).div_ceil(8),
            height.div_ceil(METER_STEP).div_ceil(8),
            1,
        ]
    }
    /// The meter pass's copy of the display constants.
    pub fn pass_constants(c: &Constants) -> Constants {
        let mut m = *c;
        m.meter_ctrl[0] |= 2;
        m
    }
}
/// The adaptation state read back from u28 row `METER_STATE_ROW` (eight words from column 8):
/// magic, base, current, target, scene log2 luminance. `None` before the first reading.
pub fn meter_state(words: &[u32]) -> Option<crate::reference::display::Adaptation> {
    (words.len() >= 5 && words[0] == METER_MAGIC).then(|| crate::reference::display::Adaptation {
        valid: true,
        base: f32::from_bits(words[1]),
        current: f32::from_bits(words[2]),
        target: f32::from_bits(words[3]),
        scene_log2: f32::from_bits(words[4]),
    })
}

pub const TEXTURES: usize = 29;
/// Constants are written once per pass at this stride (trace at 0, filter passes 1-7, display 8).
pub const CB_STRIDE: u64 = 1024;
/// Constant slots: trace 0, filter 1–7, display 8, battle quadrants 9–12, glare passes 13–22,
/// the exposure meter pass 23, the pre-exposure scale passes 24–26 (`PRE_EXPOSURE_SLOT`).
pub const CB_SLOTS: u64 = 13 + 10 + 1 + 3;
pub const METER_SLOT: u64 = 23;
/// Pre-exposure (E5): slot 24 scales the reconstruction input by P, 25 restores it, 26 brings the
/// reconstruction (texture 14) back to scene units.
pub const PRE_EXPOSURE_SLOT: u64 = 24;
/// Pre-exposure keeps P while the displayed exposure stays within this many stops of it, so P
/// (and the reconstruction's history scale) changes rarely.
pub const PRE_EXPOSURE_HYSTERESIS: f32 = 1.;
/// P stays within 2^±this.
pub const PRE_EXPOSURE_RANGE: f32 = 40.;
/// The pre-exposure P (E5) for this frame: a power of two near the previous frame's displayed
/// exposure (one frame in flight, so the read-back meter state is the last frame's), with
/// hysteresis around the current `p` (0 = none yet). A power of two makes the scale and its
/// inverse exact in fp32, so the textures it touches come back bit-identical.
pub fn pre_exposure(p: f32, exposure: f32) -> f32 {
    // `CAUSTICA_PRE_EXPOSURE=0` turns it off (P = 1), for before/after measurements.
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OFF.get_or_init(|| std::env::var("CAUSTICA_PRE_EXPOSURE").is_ok_and(|v| v == "0")) {
        return 1.;
    }
    if !(exposure.is_finite() && exposure > 0.) {
        return if p > 0. { p } else { 1. };
    }
    let stops = exposure.log2();
    if p > 0. && (stops - p.log2()).abs() <= PRE_EXPOSURE_HYSTERESIS {
        return p;
    }
    stops
        .round()
        .clamp(-PRE_EXPOSURE_RANGE, PRE_EXPOSURE_RANGE)
        .exp2()
}
/// The scale pass's constants: display's entry with `meter_ctrl.x` bit 3, the texture in `y`
/// (0 noisy, 1 accumulation, 8 ping, 14 reconstruction) and the factor's bits in `w`.
pub fn scale_constants(c: &Constants, texture: u32, factor: f32) -> Constants {
    let mut m = *c;
    m.meter_ctrl = [8, texture, 0, factor.to_bits()];
    m
}
/// Still frames a temporal upscaler gets fresh jittered samples before they accumulate.
pub const SR_FRESH_FRAMES: u32 = 32;
/// Timestamps per frame: start, after the trace, after reconstruction, after display, after
/// the UI.
pub const TIMESTAMPS: u32 = 5 + 8;
/// Battle mode's quadrant timestamps (start and end of each) follow the frame's five.
pub const BATTLE_QUERY: u32 = 5;
/// integrators.hlsl: the photon counter and grid heads before the photons.
pub const PHOTON_HASH: u64 = 1 << 20;
pub const PHOTON_BASE: u64 = 16 + 4 * PHOTON_HASH;
/// Deposits the photon buffer holds per photon (integrators.hlsl walks up to 32 bounces; Russian
/// roulette keeps the mean count of deposits far below this, the rest are dropped).
pub const PHOTON_BOUNCES: u32 = 10;
pub const GLARE_SLOT: u64 = 13;
/// The glare grid (post.hlsl GN) and the image's longer side in it (the rest is padding, so the
/// circular convolution does not wrap highlights around the frame).
pub const GLARE_N: u64 = 512;
pub const GLARE_IMAGE: f32 = 384.;
/// Path visualizer records per request, the count record included (48 B each).
pub const PATH_CAPACITY: usize = 16384;
/// ReSTIR light-tile samples per frame (pathtrace.hlsl: 128 tiles of 1024).
pub const RESTIR_TILES: u64 = 128 * 1024;

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
        SCOPES_TEXTURE
    } else {
        render
    };
    (format, size)
}

/// Points each visible object's material at its texture maps (core track 5): `slot_of` gives a
/// map's slot, uploading it when new. Returns the maps in use, by `Arc` address.
pub fn attach_maps(
    s: &crate::scene::Settings,
    instances: &[plan::InstanceRecord],
    materials: &mut [crate::lighting::PackedMaterial],
    slot_of: &mut dyn FnMut(&std::sync::Arc<crate::mesh::Map>) -> anyhow::Result<u32>,
) -> anyhow::Result<std::collections::HashSet<usize>> {
    let mut used = std::collections::HashSet::new();
    // (material record, maps): visible objects (a texture set wins over the mesh's own
    // maps), then the walls (records 0-2) and the floor (3).
    let set = |m: &crate::scene::Material| {
        (!m.texture.is_empty())
            .then(|| crate::texture::load(&m.texture).ok())
            .flatten()
    };
    let mut wanted: Vec<(usize, crate::mesh::Maps)> = vec![];
    for record in instances.iter().filter(|r| r.material != u32::MAX) {
        let o = &s.objects[record.object as usize];
        let maps = if o.material.texture.is_empty() {
            crate::mesh::maps(o)
        } else {
            set(&o.material)
        };
        wanted.extend(maps.map(|m| (record.material as usize, m)));
    }
    if let Some(maps) = set(&s.walls) {
        wanted.extend((0..3).map(|i| (i, maps.clone())));
    }
    wanted.extend(set(&s.floor).map(|m| (3, m)));
    for (record, maps) in wanted {
        let mut slots = [0u32; 3];
        for (slot, map) in slots
            .iter_mut()
            .zip([&maps.base, &maps.normal, &maps.metal_rough])
        {
            let Some(map) = map else { continue };
            used.insert(std::sync::Arc::as_ptr(map) as usize);
            *slot = slot_of(map)? + 1;
        }
        // An emissive map (flag 64, pathtrace.hlsl `emitted_color`) on an opaque, unpatterned
        // emitter: secondary = (slot + 1, the mean emitted colour over its brightest
        // channel), medium.xyz = 1 / the map's mean.
        let emissive = match &maps.emissive {
            Some(map)
                if materials[record].lobes[2] <= 0.
                    && materials[record].optical[2] == 0.
                    && materials[record].optical[1] > 0. =>
            {
                used.insert(std::sync::Arc::as_ptr(map) as usize);
                let slot = slot_of(map)?;
                let mean: [f32; 3] =
                    std::array::from_fn(|k| maps.emissive_factor[k] * map.average[k]);
                let peak = mean[0].max(mean[1]).max(mean[2]);
                (peak > 0.).then(|| (slot + 1, mean.map(|v| v / peak), map))
            }
            _ => None,
        };
        let m = &mut materials[record];
        if let Some((slot, colour, map)) = emissive {
            m.secondary = [slot as f32, colour[0], colour[1], colour[2]];
            for k in 0..3 {
                m.medium[k] = if map.average[k] > 1e-4 {
                    1. / map.average[k]
                } else {
                    0.
                };
            }
            m.lobes[3] = (m.lobes[3] as u32 | 16 | 64) as f32;
        }
        m.maps = [slots[0], slots[1], slots[2], maps.cutoff.to_bits()];
        // Flag bits the shader tests before any map work: 16 has maps, 8 masked.
        let masked = slots[0] != 0 && maps.cutoff != 0.;
        m.lobes[3] = (m.lobes[3] as u32 | 16 | if masked { 8 } else { 0 }) as f32;
        // Scale by value / average, so the map's mean is the slider value. A channel
        // that averages black (no metal anywhere) leaves the value unscaled.
        let ratio = |value: f32, average: f32| {
            if average > 1e-3 { value / average } else { 0. }
        };
        if let Some(b) = &maps.base {
            for k in 0..3 {
                m.base_scale[k] = ratio(m.color_rough[k], b.average[k]);
            }
        }
        if let Some(mr) = &maps.metal_rough {
            // z, w: the thin film (lighting::pack).
            m.surface_scale[0] = ratio(m.color_rough[3], mr.average[1]);
            m.surface_scale[1] = ratio(m.lobes[0], mr.average[2]);
        }
    }
    Ok(used)
}
/// A UI frame captured in HDR (scRGB RGBA16F, or HDR10 A2B10G10R10 PQ with `hdr10`) back to
/// sRGB RGBA8: paper white (`paper_nits`) becomes 1.0, then sRGB-encode.
pub fn hdr_ui_to_srgb8(packed: &[u8], hdr10: bool, paper_nits: f32) -> Vec<u8> {
    let scale = 80. / paper_nits;
    let scrgb: Vec<f32> = if hdr10 {
        packed
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|b| crate::display::hdr10_to_scrgb(u32::from_le_bytes(*b)))
            .collect()
    } else {
        packed
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| half_to_f32(u16::from_le_bytes(*b)))
            .collect()
    };
    scrgb
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if i % 4 == 3 {
                (v.clamp(0., 1.) * 255.).round() as u8
            } else {
                let v = (v * scale).clamp(0., 1.);
                let e = if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1. / 2.4) - 0.055
                };
                (e * 255.).round() as u8
            }
        })
        .collect()
}
/// The radical inverse of `index` in `base` (upscaler jitter).
pub fn halton(mut index: u32, base: u32) -> f32 {
    let (mut f, mut result) = (1., 0.);
    while index > 0 {
        f /= base as f32;
        result += f * (index % base) as f32;
        index /= base;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_exposure_is_a_power_of_two_with_hysteresis() {
        // Night: exposure 3e7 is 2^24.8, so P = 2^25; the inverse undoes it exactly.
        let p = pre_exposure(0., 3e7);
        assert_eq!(p, 2f32.powi(25));
        let x = 1.2345e-9f32;
        assert_eq!(x * p * (1. / p), x);
        // Within a stop P holds; past it P moves.
        assert_eq!(pre_exposure(p, 2f32.powf(25.9)), p);
        assert_eq!(pre_exposure(p, 2f32.powf(23.6)), 2f32.powi(24));
        // Nonsense keeps P (1 before any); the range is clamped.
        assert_eq!(pre_exposure(0., f32::NAN), 1.);
        assert_eq!(pre_exposure(p, 0.), p);
        assert_eq!(pre_exposure(0., 1e30), 2f32.powi(40));
        let m = scale_constants(&bytemuck::Zeroable::zeroed(), 14, 0.5);
        assert_eq!(m.meter_ctrl, [8, 14, 0, 0.5f32.to_bits()]);
    }
    #[test]
    fn lpe_layers_are_named_by_their_expression() {
        let e = "C D'red wall' .* L".to_string();
        assert_eq!(super::lpe_layer_name(0, Some(&e)), "lpe1_C_D_red_wall_L");
        assert_eq!(super::lpe_layer_name(2, None), "lpe3");
    }

    /// gui.hlsl reads a positive eye.w as HDR paper white and linearises egui's sRGB colours for
    /// it. On SDR's UNORM targets nothing re-encodes them, so the whole UI would draw too dark;
    /// a machine with HDR on hides this, since only SDR output keeps the value set here.
    #[test]
    fn sdr_constants_leave_the_gui_in_srgb() {
        let c = constants(
            &crate::scene::Settings::default(),
            &GpuState::default(),
            &FrameInputs {
                view: glam::Mat4::IDENTITY,
                proj: glam::Mat4::IDENTITY,
                eye: glam::Vec3::ONE,
                previous_vp: glam::Mat4::IDENTITY,
                n: 1,
                reset: false,
                flags: 0,
                jitter: [0.; 2],
                sequence_base: 0,
                lut_size: 0,
                present_count: 0,
                env_scale: 1.,
                env_inv_norm: 1.,
                env_sun: None,
                env_stars: None,
                env: (1., 0., true),
                lens_surfaces: 0,
                glare_grid: 0,
                restir: 0,
            },
        );
        assert_eq!(c.eye, [1., 1., 1., 0.]);
    }
}
