#![allow(unsafe_op_in_unsafe_fn)]
use crate::scene::{Settings, Upscaler, Vertex};
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Vec3};
use std::{
    ffi::c_void,
    mem::{ManuallyDrop, size_of},
    ptr::null_mut,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        Graphics::{
            Direct3D::*,
            Direct3D12::*,
            Dxgi::{Common::*, *},
        },
        System::Threading::*,
    },
    core::{Interface, PCSTR},
};

const TEXTURES: usize = 29;
/// Size of the scopes counter texture (u28).
pub const SCOPES: [u32; 2] = [256, 260];
/// Shader-visible heap: UAVs 0..TEXTURES, then one SRV slot per egui texture from 32.
const HEAP_SIZE: usize = 512;
const FIRST_GUI_SLOT: usize = 32;
/// Mesh texture maps (core track 5): SRVs t0.. in space 1, in the heap after `HEAP_SIZE`.
const MAX_MAPS: usize = 4096;
/// Still frames a temporal upscaler gets fresh jittered samples before they accumulate.
const SR_FRESH_FRAMES: u32 = 32;
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
struct GuiTexture {
    texture: Texture,
    slot: usize,
    /// CPU copy, so partial updates (the font atlas grows) can be patched in.
    pixels: Vec<u8>,
    size: [usize; 2],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Constants {
    inverse_vp: [f32; 16],
    current_vp: [f32; 16],
    previous_vp: [f32; 16],
    world_view: [f32; 16],
    eye: [f32; 4],
    dimensions: [u32; 4],
    sampling: [u32; 4],
    /// x: suns at the start of the emitter table (track 7); y: pixel filter (0 box, 1 Gaussian,
    /// 2 Blackman-Harris), z: its width in pixels; w: texture anisotropy (1–16).
    lights: [f32; 4],
    light_color_aspect: [f32; 4],
    background: [f32; 4],
    display: [f32; 4],
    modes: [u32; 4],
    options: [f32; 4],
    jitter: [f32; 4],
    filter: [u32; 4],
    scene_info: [u32; 4],
    environment: [f32; 4],
    hdr: [f32; 4],
    tools: [f32; 4],
    lens: [f32; 4],
    camera_model: [f32; 4],
    look: [f32; 4],
    lift: [f32; 4],
    grade_gamma: [f32; 4],
    gain: [f32; 4],
    environment_map: [f32; 4],
    /// Track 12 fog: density, phase g, top height, half extent on x / z.
    fog: [f32; 4],
    /// Fog albedo (rgb).
    fog_color: [f32; 4],
    /// Track 14, read only by the path-recording trace: render pixel x, y, paths, capacity.
    path_record: [u32; 4],
    /// Track 14: light path expression slots in use (0 = off), the slot the LPE view shows.
    lpe: [u32; 4],
    /// Track 15 ReSTIR DI: flags (bit 0 on, bit 1 history valid), candidates, spatial
    /// neighbours, history cap.
    restir: [u32; 4],
    /// Track 15: the render pixels a dispatch covers (x0, y0, x1, y1).
    region: [u32; 4],
    /// Track 15 (integrators.hlsl): photons this frame, photon capacity, splat offset in `aux`
    /// (bytes), flags (bit 0 light tracing).
    integrator: [u32; 4],
    /// Gather radius, image-plane area at distance 1 times the region's share.
    photon: [f32; 4],
    /// The scene's bounding sphere (centre, radius).
    bounds: [f32; 4],
    /// Track 16 lens system: surfaces (0 = off), dispersion, scene units per mm, -.
    real_lens: [f32; 4],
    /// Track 16 glare pass: mode, region, flags, image size in the grid (w | h << 16).
    glare: [u32; 4],
    /// Glare strength (0 = off), threshold, aperture diameter in grid texels, -.
    glare_settings: [f32; 4],
}
/// Timestamps per frame: start, after the trace, after reconstruction, after display, after
/// the UI.
const TIMESTAMPS: u32 = 5 + 8;
/// Battle mode's quadrant timestamps (start and end of each) follow the frame's five.
const BATTLE_QUERY: u32 = 5;
/// Battle mode quadrants (track 15): top left, top right, bottom left, bottom right.
pub const BATTLE_NAMES: [&str; 4] = ["Path tracing", "Path + ReSTIR DI", "BDPT", "Photon mapping"];
/// integrators.hlsl: the photon counter and grid heads before the photons.
const PHOTON_HASH: u64 = 1 << 20;
const PHOTON_BASE: u64 = 16 + 4 * PHOTON_HASH;
/// Deposits the photon buffer holds per photon (integrators.hlsl walks up to 32 bounces; Russian
/// roulette keeps the mean count of deposits far below this, the rest are dropped).
const PHOTON_BOUNCES: u32 = 10;
/// Per-quadrant state of battle mode: samples, GPU time spent (ms), measured ms per sample
/// (0 = not measured yet) and the fractional samples carried to the next frame.
#[derive(Clone, Copy, Default)]
pub struct BattleQuadrant {
    pub samples: u32,
    pub ms: f64,
    pub cost: f64,
    carry: f64,
    /// Samples dispatched this frame (0 = skipped).
    last: u32,
    /// Photon mapping's work per sample this frame: its photon count's multiple (1 elsewhere).
    scale: f64,
}
/// Constants are written once per pass at this stride (trace at 0, filter passes 1-7, display 8).
const CB_STRIDE: u64 = 1024;
/// Constant slots: trace 0, filter 1–7, display 8, battle quadrants 9–12, glare passes from 13.
const CB_SLOTS: u64 = 13 + 10;
const GLARE_SLOT: u64 = 13;
/// The glare grid (post.hlsl GN) and the image's longer side in it (the rest is padding, so the
/// circular convolution does not wrap highlights around the frame).
const GLARE_N: u64 = 512;
const GLARE_IMAGE: f32 = 384.;
const _: () = assert!(size_of::<Constants>() as u64 <= CB_STRIDE);
/// Path visualizer records per request, the count record included (48 B each).
const PATH_CAPACITY: usize = 16384;
/// ReSTIR light-tile samples per frame (pathtrace.hlsl: 128 tiles of 1024).
const RESTIR_TILES: u64 = 128 * 1024;
unsafe extern "C" {
    fn caustica_ngx_release(ngx: *mut c_void);
    fn caustica_ngx_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn caustica_ngx_create(
        ngx: *mut c_void,
        cmd: *mut c_void,
        w: u32,
        h: u32,
        ow: u32,
        oh: u32,
        preset: u32,
    ) -> u32;
    fn caustica_ngx_eval(
        ngx: *mut c_void,
        cmd: *mut c_void,
        resources: *const *mut c_void,
        w: u32,
        h: u32,
        jx: f32,
        jy: f32,
        reset: i32,
        dt: f32,
        view: *const f32,
        proj: *const f32,
    ) -> u32;
    fn caustica_ngx_destroy(ngx: *mut c_void);
    fn caustica_ngx_caps(ngx: *mut c_void) -> u32;
    fn caustica_ngx_create_sr(
        ngx: *mut c_void,
        cmd: *mut c_void,
        w: u32,
        h: u32,
        ow: u32,
        oh: u32,
        preset: u32,
    ) -> u32;
    fn caustica_ngx_eval_sr(
        ngx: *mut c_void,
        cmd: *mut c_void,
        color: *mut c_void,
        depth: *mut c_void,
        motion: *mut c_void,
        output: *mut c_void,
        w: u32,
        h: u32,
        jx: f32,
        jy: f32,
        reset: i32,
        dt: f32,
    ) -> u32;
    fn caustica_fsr_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn caustica_fsr_release(fsr: *mut c_void);
    fn caustica_fsr_destroy(fsr: *mut c_void);
    fn caustica_fsr_create(fsr: *mut c_void, w: u32, h: u32, ow: u32, oh: u32) -> u32;
    fn caustica_fsr_eval(
        fsr: *mut c_void,
        cmd: *mut c_void,
        color: *mut c_void,
        depth: *mut c_void,
        motion: *mut c_void,
        output: *mut c_void,
        w: u32,
        h: u32,
        ow: u32,
        oh: u32,
        jx: f32,
        jy: f32,
        reset: i32,
        dt: f32,
        fov_y: f32,
        near: f32,
        far: f32,
    ) -> u32;
    fn caustica_xess_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn caustica_xess_destroy(xess: *mut c_void);
    fn caustica_xess_create(
        xess: *mut c_void,
        w: u32,
        ow: u32,
        oh: u32,
        velocity_scale: f32,
    ) -> u32;
    fn caustica_xess_eval(
        xess: *mut c_void,
        cmd: *mut c_void,
        color: *mut c_void,
        depth: *mut c_void,
        motion: *mut c_void,
        output: *mut c_void,
        w: u32,
        h: u32,
        jx: f32,
        jy: f32,
        reset: i32,
    ) -> u32;
}
fn ngx_ok(code: u32) -> bool {
    code & 0xfff00000 != 0xbad00000
}
pub struct Texture {
    pub resource: ID3D12Resource,
    pub format: DXGI_FORMAT,
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
pub struct Gpu {
    pub device: ID3D12Device5,
    queue: ID3D12CommandQueue,
    swap: IDXGISwapChain3,
    allocator: ID3D12CommandAllocator,
    pub list: ID3D12GraphicsCommandList4,
    fence: ID3D12Fence,
    event: HANDLE,
    fence_value: u64,
    heap: ID3D12DescriptorHeap,
    rtv: ID3D12DescriptorHeap,
    root: ID3D12RootSignature,
    trace: ID3D12PipelineState,
    /// The path tracer with texture-map code (core track 5) and diffraction gratings (track 11),
    /// used while any material has maps or a grating.
    trace_maps: ID3D12PipelineState,
    /// `trace` and `trace_maps` compiled with LEAN (track 20a (b)): no ReSTIR, fog or volume
    /// media, SDF procedurals or layered-BSDF extras, which frees registers; used while
    /// `lean_scene` holds and the frame has neither fog nor ReSTIR.
    trace_lean: ID3D12PipelineState,
    trace_maps_lean: ID3D12PipelineState,
    /// Spectral rendering (`Settings::spectral`): the full tracer with the map code, carrying
    /// wavelengths (engine-notes "Spectral rendering").
    trace_spectral: ID3D12PipelineState,
    /// `trace_spectral` with LEAN, picked under the same rule as `trace_lean`.
    trace_spectral_lean: ID3D12PipelineState,
    /// Track 20: the four tracers above as SER ray generation pipelines, in the order trace,
    /// trace_maps, trace_lean, trace_maps_lean; None without SER (or CAUSTICA_NO_SER=1).
    /// The SER tracers: the four `trace` variants (index textured + 2 × LEAN), then LPE, then
    /// spectral and spectral LEAN.
    ser: Option<[SerPipeline; 7]>,
    /// Path visualizer (track 14): the trace variant that re-traces one pixel into
    /// `path_buffer` (bound at u0 space 2 for every dispatch), read back through `path_readback`.
    trace_paths: ID3D12PipelineState,
    path_buffer: ID3D12Resource,
    path_readback: ID3D12Resource,
    /// One-shot: record paths through this output pixel in the next `render` (pixel, paths).
    pub path_request: Option<([u32; 2], u32)>,
    /// The records of the last request (the count record stripped), until the app takes them.
    pub path_result: Option<Vec<crate::paths::Record>>,
    /// Light path expressions (track 14): the trace variant that fills the AOVs, the DFA table
    /// (t8, an upload buffer, rewritten when the expressions change), the slots in use and the
    /// AOV buffer (u1 space 2: render pixels x slots float4s, allocated while any slot is set).
    trace_lpe: ID3D12PipelineState,
    lpe_table: ID3D12Resource,
    lpe_uploaded: Option<Vec<String>>,
    pub lpe_slots: u32,
    lpe_aov: Option<ID3D12Resource>,
    lpe_aov_len: u64,
    /// Track 16: the lens system's buffer (t9, upload), what it was built for, its surfaces.
    lens_buffer: ID3D12Resource,
    lens_params: Option<crate::lens::Params>,
    lens_surfaces: u32,
    /// Track 16 glare: the FFT pass, its buffer (u4 space 2: two regions of 3 × GLARE_N²
    /// complex values) and the aperture its kernel was built for (blades, rotation, diameter).
    glare: ID3D12PipelineState,
    glare_buffer: ID3D12Resource,
    glare_kernel: Option<[f32; 3]>,
    /// ReSTIR DI (track 15): the reservoir passes before the trace and their buffer (u2 space
    /// 2: two 32-byte reservoirs per render pixel and layer, allocated while ReSTIR is on); the
    /// history is valid from the frame after a pass until a full reset.
    restir_layers: u32,
    restir_presample: ID3D12PipelineState,
    restir_initial: ID3D12PipelineState,
    restir_spatial: ID3D12PipelineState,
    reservoirs: Option<ID3D12Resource>,
    reservoirs_len: u64,
    restir_history: bool,
    restir_ran: bool,
    /// Track 15 integrators (integrators.hlsl): BDPT and its splat resolve, photon mapping's
    /// grid clear, photon trace and gather. `aux` (u3 space 2) holds the photon grid at 0 and
    /// the BDPT splats at `splat_offset`.
    bdpt: ID3D12PipelineState,
    bdpt_resolve: ID3D12PipelineState,
    photon_clear: ID3D12PipelineState,
    photon_trace: ID3D12PipelineState,
    /// `photon_trace` with spectral photons (`Settings::spectral`).
    photon_trace_spectral: ID3D12PipelineState,
    photon_gather: ID3D12PipelineState,
    aux: Option<ID3D12Resource>,
    aux_len: u64,
    splat_offset: u64,
    /// Photon mapping's radius schedule since the last reset: frames done and r² / r0².
    photon_frames: u32,
    photon_scale: f32,
    /// The scene's bounding sphere for sun and sky photons, measured when photon mapping runs
    /// after a rebuild (`bounds_valid`).
    bounds: [f32; 4],
    bounds_valid: bool,
    /// Battle mode (track 15), reset with the accumulation.
    pub battle: [BattleQuadrant; 4],
    textured: bool,
    /// No material with a coat, anisotropy, sheen, thin film or extinction, and no visible SDF
    /// object: the LEAN trace variants render this scene unchanged (`integrate` adds the
    /// frame's fog and ReSTIR).
    lean_scene: bool,
    filter: ID3D12PipelineState,
    display: ID3D12PipelineState,
    gui: ID3D12PipelineState,
    gui_hdr: ID3D12PipelineState,
    gui_hdr10: ID3D12PipelineState,
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
    /// Film-look LUT (u29, a 3D texture kept across resizes) and its edge length, 0 = none.
    lut: Texture,
    lut_size: u32,
    /// HDRI environment (u30, RGBA32F) and its sampling CDFs (u31, R32F, one more row than the
    /// map); 1×1 placeholders when none is loaded. `env_inv_norm` 0 = none (see environment.rs).
    env: Texture,
    env_cdf: Texture,
    env_inv_norm: f32,
    /// The map is stored as RGBA16F divided by this (≥ 1), so a sun brighter than half-float's
    /// 65504 keeps its value; strength and the pdf normalization are multiplied back.
    env_scale: f32,
    /// Frames presented, for the grain's per-frame noise.
    present_count: u32,
    /// Look tools (roadmap 5b): zebras, scopes (read back into `scopes` each frame) and a
    /// light-meter probe at an output pixel (read back into `probe_result`: linear beauty and
    /// diffuse albedo).
    pub zebras: bool,
    pub scopes_on: bool,
    pub scopes: Vec<u32>,
    pub probe: Option<[u32; 2]>,
    pub probe_result: Option<Probe>,
    readback: Option<ID3D12Resource>,
    scope_zero: Option<ID3D12Resource>,
    /// The swapchain accepted the color space for its current format.
    color_space_ok: bool,
    buffers: Vec<ID3D12Resource>,
    textures: Vec<Texture>,
    constants: ID3D12Resource,
    /// Object-space vertices of every mesh in use (t1), and the static world-space vertices:
    /// floor, room and lights (t6), with their BLASes (core track 4).
    mesh_vertices: ID3D12Resource,
    static_vertices: ID3D12Resource,
    static_cache: Vec<Vertex>,
    /// Emissive procedural objects' photon records: count and first record (`photon.zw`).
    sdf_emitters: [u32; 2],
    /// The static triangles' opacity groups, in `static_cache` order (rebuild_scene).
    static_groups: Vec<StaticGroup>,
    /// Track 13: one AABB, the unit cube, shared by every procedural (`Sdf`) object.
    sdf_blas: Option<ID3D12Resource>,
    meshes: std::collections::HashMap<String, MeshBlas>,
    /// One record per TLAS instance (t7, indexed by InstanceID).
    instances: Vec<InstanceRecord>,
    instance_table: ID3D12Resource,
    material_buffer: ID3D12Resource,
    emitter_buffer: ID3D12Resource,
    emitter_lookup: ID3D12Resource,
    pub triangle_count: usize,
    pub emitter_count: usize,
    /// Suns lead the emitter table (`lighting::analytic`).
    pub sun_count: u32,
    tlas: Option<ID3D12Resource>,
    /// egui textures (font atlas, scene thumbnails), each with its own heap slot (32 and up).
    gui_textures: std::collections::HashMap<egui::TextureId, GuiTexture>,
    free_slots: Vec<usize>,
    /// Mesh texture maps on the GPU, keyed by the `Arc<Map>` address (the Arc is kept, so the
    /// address stays unique), and the free map slots.
    maps: std::collections::HashMap<usize, MapTexture>,
    free_maps: Vec<u32>,
    /// When set, each frame's backbuffer (with UI) is copied here before Present, because a
    /// FLIP_DISCARD buffer's contents are undefined once presented. Kept in COPY_DEST.
    pub keep_ui_frame: bool,
    ui_frame: Option<ID3D12Resource>,
    ngx: *mut c_void,
    fsr: *mut c_void,
    xess: *mut c_void,
    /// Temporal upscaler for the native path (RR off, render scale below 1).
    sr_created: Option<Upscaler>,
    sr_failed: Vec<Upscaler>,
    sr_reset: bool,
    /// Frames drawn with a temporal upscaler since the last restart (see `SR_FRESH_FRAMES`).
    sr_fresh: u32,
    pub upscale_status: String,
    pub rr_status: String,
    rr_created: bool,
    rr_failed: bool,
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
    /// Why each reconstruction backend could not start (empty when it did): RR, DLSS SR, FSR,
    /// XeSS. `capabilities()` adds failures found later, while rendering.
    init_reasons: [String; 4],
    previous_vp: Mat4,
    rr_reset: bool,
    restarts: u32,
    sequence_base: u32,
    /// History (15, 23) holds earlier samples warped into the current view.
    history_valid: bool,
    /// Textures 21/22 hold the integration of the last traced frame, and 16/17/20 its guides.
    integrated_valid: bool,
    /// The camera moved since the last traced frame (accumulation restarted, history kept).
    camera_moved: bool,
    pub reference_valid: bool,
    query: ID3D12QueryHeap,
    timing: ID3D12Resource,
}
/// An object instance's mask and whether it still needs the shader per candidate (track 20a
/// (a)). Rays leave out `MASK_HIDDEN` on camera rays and `MASK_CLEAR_GLASS` on shadow rays
/// when glass doesn't block (pathtrace.hlsl); every other ray includes all of 0xff. An object
/// under one rule gets just that bit and goes opaque; under both no single bit says it, so it
/// stays non-opaque with the shader's tests (as do glass objects below 0.5 transmission,
/// opaque now: the shader committed every candidate of theirs anyway).
fn instance_mask(m: &crate::lighting::PackedMaterial) -> (u32, bool) {
    let hidden = (m.lobes[3] as u32 & 2) != 0;
    let clear = m.lobes[2] >= 0.5;
    match (hidden, clear) {
        (true, true) => (0xff, true),
        (true, false) => (MASK_HIDDEN, false),
        (false, true) => (MASK_CLEAR_GLASS, false),
        (false, false) => (0xff, false),
    }
}
/// Instance mask bits (pathtrace.hlsl MASK_HIDDEN, MASK_CLEAR_GLASS).
const MASK_HIDDEN: u32 = 2;
const MASK_CLEAR_GLASS: u32 = 4;
/// What the device offers for Shader Execution Reordering (track 20): Shader Model 6.9 (with
/// the Agility SDK, src/main.rs) and raytracing tier 1.2; `reorders` says whether
/// `MaybeReorderThread` does anything on this GPU (D3D12_OPTIONS22).
#[derive(Clone, Copy, Debug, Default)]
pub struct SerSupport {
    pub shader_model: u32,
    pub tier: u32,
    pub reorders: bool,
}
impl SerSupport {
    pub fn available(&self) -> bool {
        self.shader_model >= 0x69 && self.tier >= 12
    }
}
unsafe fn ser_support(device: &ID3D12Device5) -> SerSupport {
    unsafe {
        let mut sm = D3D12_FEATURE_DATA_SHADER_MODEL {
            HighestShaderModel: D3D_SHADER_MODEL(0x69),
        };
        let shader_model = if device
            .CheckFeatureSupport(
                D3D12_FEATURE_SHADER_MODEL,
                &mut sm as *mut _ as _,
                size_of_val(&sm) as u32,
            )
            .is_ok()
        {
            sm.HighestShaderModel.0 as u32
        } else {
            0x60
        };
        let mut o5 = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
        let _ = device.CheckFeatureSupport(
            D3D12_FEATURE_D3D12_OPTIONS5,
            &mut o5 as *mut _ as _,
            size_of_val(&o5) as u32,
        );
        // D3D12_FEATURE_D3D12_OPTIONS22 (65): ShaderExecutionReorderingActuallyReorders first.
        let mut o22 = [0u32; 4];
        let reorders = device
            .CheckFeatureSupport(D3D12_FEATURE(65), o22.as_mut_ptr() as _, 16)
            .is_ok()
            && o22[0] != 0;
        SerSupport {
            shader_model,
            tier: o5.RaytracingTier.0 as u32,
            reorders,
        }
    }
}
impl Gpu {
    pub unsafe fn new(
        hwnd: HWND,
        width: u32,
        height: u32,
        s: &Settings,
        debug: bool,
    ) -> Result<Self> {
        if debug {
            let mut d: Option<ID3D12Debug> = None;
            D3D12GetDebugInterface(&mut d)?;
            d.unwrap().EnableDebugLayer();
            if let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
                q.SetBreakOnSeverity(
                    DXGI_DEBUG_ALL,
                    DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR,
                    false,
                )?;
                q.SetBreakOnSeverity(
                    DXGI_DEBUG_ALL,
                    DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
                    false,
                )?;
            }
        }
        let factory: IDXGIFactory6 = CreateDXGIFactory2(if debug {
            DXGI_CREATE_FACTORY_DEBUG
        } else {
            DXGI_CREATE_FACTORY_FLAGS(0)
        })?;
        if debug && let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
            q.SetBreakOnSeverity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR,
                false,
            )?;
            q.SetBreakOnSeverity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
                false,
            )?;
        }
        let mut selected = None;
        for i in 0..32 {
            let Ok(adapter) = factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(
                i,
                DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
            ) else {
                break;
            };
            let mut device = None;
            if D3D12CreateDevice::<_, ID3D12Device5>(&adapter, D3D_FEATURE_LEVEL_12_1, &mut device)
                .is_err()
            {
                continue;
            }
            let dev = device.unwrap();
            let mut opt = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
            dev.CheckFeatureSupport(
                D3D12_FEATURE_D3D12_OPTIONS5,
                &mut opt as *mut _ as _,
                size_of_val(&opt) as u32,
            )?;
            if opt.RaytracingTier.0 < D3D12_RAYTRACING_TIER_1_1.0 {
                continue;
            }
            let desc = adapter.GetDesc1()?;
            let end = desc.Description.iter().position(|v| *v == 0).unwrap_or(128);
            selected = Some((dev, String::from_utf16_lossy(&desc.Description[..end])));
            break;
        }
        let (device,adapter_name)=selected.context("No D3D12 adapter with DXR tier 1.1. Update the GPU driver and use a ray-tracing-capable GPU.")?;
        let ser = ser_support(&device);
        OMM.store(
            ser.tier >= 12 && std::env::var_os("CAUSTICA_NO_OMM").is_none_or(|v| v != "1"),
            std::sync::atomic::Ordering::Relaxed,
        );
        eprintln!(
            "D3D12: shader model {}.{}, raytracing tier {}.{}; SER {}",
            ser.shader_model >> 4,
            ser.shader_model & 15,
            ser.tier / 10,
            ser.tier % 10,
            match (ser.available(), ser.reorders) {
                (false, _) => "unavailable",
                (true, false) => "accepted, no reordering",
                (true, true) => "reorders",
            }
        );
        let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let swap: IDXGISwapChain3 = factory
            .CreateSwapChainForHwnd(&queue, hwnd, &desc, None, None)?
            .cast()?;
        factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
        let allocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
        let list: ID3D12GraphicsCommandList4 =
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)?;
        list.Close()?;
        let fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
        let event = CreateEventW(None, false, false, None)?;
        let heap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: (HEAP_SIZE + MAX_MAPS) as u32,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;
        let rtv = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: 2,
            ..Default::default()
        })?;
        let ranges = [
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
                // The per-resize textures, the LUT (u29), the environment and its CDFs (u30, u31).
                NumDescriptors: TEXTURES as u32 + 3,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 2,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
            // Texture maps: only slots a material points at are read, so unused ones may stay
            // empty (root signature 1.0 descriptors are volatile).
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: MAX_MAPS as u32,
                BaseShaderRegister: 0,
                RegisterSpace: 1,
                OffsetInDescriptorsFromTableStart: 0,
            },
        ];
        let params = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 1,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[0],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[1],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
            root_srv(3),
            root_srv(4),
            root_srv(5),
            root_srv(6),
            root_srv(7),
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[2],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 14: the path visualizer's records (u0 space 2, the recording trace only).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Light path expressions: the AOVs (u1 space 2) and the DFA table (t8).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 1,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            root_srv(8),
            // ReSTIR DI: the reservoirs (u2 space 2).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 2,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 15 integrators: BDPT splats and the photon grid (u3 space 2, raw).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 3,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 16: the lens system (t9).
            root_srv(9),
            // Track 16: glare (u4 space 2).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 4,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
        ];
        let gui_sampler = D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            MaxLOD: f32::MAX,
            ShaderRegister: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            ..Default::default()
        };
        // Texture maps repeat (glTF's default wrap mode) and are filtered anisotropically; the
        // shader limits the ratio to Settings::anisotropy through the gradients it passes.
        let samplers = [
            gui_sampler,
            D3D12_STATIC_SAMPLER_DESC {
                Filter: D3D12_FILTER_ANISOTROPIC,
                MaxAnisotropy: 16,
                AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                ShaderRegister: 1,
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                ..gui_sampler
            },
        ];
        let rd = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: params.len() as _,
            pParameters: params.as_ptr(),
            NumStaticSamplers: samplers.len() as _,
            pStaticSamplers: samplers.as_ptr(),
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };
        let mut blob = None;
        let mut errors = None;
        D3D12SerializeRootSignature(
            &rd,
            D3D_ROOT_SIGNATURE_VERSION_1,
            &mut blob,
            Some(&mut errors),
        )?;
        let blob = blob.unwrap();
        let root: ID3D12RootSignature = device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(blob.GetBufferPointer() as _, blob.GetBufferSize()),
        )?;
        let compute = |code: &[u8]| create_compute_pipeline(&device, &root, code);
        let trace = compute(include_bytes!(concat!(env!("OUT_DIR"), "/trace.dxil")))?;
        let trace_maps = compute(include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps.dxil")))?;
        let trace_lean = compute(include_bytes!(concat!(env!("OUT_DIR"), "/trace_lean.dxil")))?;
        let trace_maps_lean = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trace_maps_lean.dxil"
        )))?;
        let trace_spectral = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trace_spectral.dxil"
        )))?;
        let trace_spectral_lean = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trace_spectral_lean.dxil"
        )))?;
        let ser_pipelines = make_ser(
            &device,
            &root,
            ser,
            [
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lean_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps_lean_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lpe_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral_lean_ser.dxil")),
            ],
        );
        let trace_paths = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trace_paths.dxil"
        )))?;
        let path_size = (PATH_CAPACITY * size_of::<crate::paths::Record>()) as u64;
        let path_buffer = buffer(
            &device,
            path_size,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            // Buffers start in COMMON and are promoted to UAV by the first dispatch.
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let path_readback = buffer(
            &device,
            path_size,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let trace_lpe = compute(include_bytes!(concat!(env!("OUT_DIR"), "/trace_lpe.dxil")))?;
        let lpe_table = buffer(
            &device,
            (crate::lpe::SLOTS * crate::lpe::STATES * crate::lpe::SYMBOLS * 4) as u64,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let glare = compute(include_bytes!(concat!(env!("OUT_DIR"), "/glare.dxil")))?;
        let glare_buffer = buffer(
            &device,
            2 * 3 * GLARE_N * GLARE_N * 8,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let lens_buffer = buffer(
            &device,
            (crate::lens::FLOAT4S * 16) as u64,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let restir_presample = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/restir_presample.dxil"
        )))?;
        let restir_initial = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/restir_initial.dxil"
        )))?;
        let restir_spatial = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/restir_spatial.dxil"
        )))?;
        let bdpt = compute(include_bytes!(concat!(env!("OUT_DIR"), "/bdpt.dxil")))?;
        let bdpt_resolve = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/bdpt_resolve.dxil"
        )))?;
        let photon_clear = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/photon_clear.dxil"
        )))?;
        let photon_trace = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/photon_trace.dxil"
        )))?;
        let photon_trace_spectral = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/photon_trace_spectral.dxil"
        )))?;
        let photon_gather = compute(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/photon_gather.dxil"
        )))?;
        let filter = compute(include_bytes!(concat!(env!("OUT_DIR"), "/filter.dxil")))?;
        let display = compute(include_bytes!(concat!(env!("OUT_DIR"), "/display.dxil")))?;
        let (vs, ps): (&[u8], &[u8]) = (
            include_bytes!(concat!(env!("OUT_DIR"), "/vs_main.dxil")),
            include_bytes!(concat!(env!("OUT_DIR"), "/ps_main.dxil")),
        );
        let gui = create_gui_pipeline(&device, &root, DXGI_FORMAT_R8G8B8A8_UNORM, vs, ps)?;
        let gui_hdr = create_gui_pipeline(&device, &root, DXGI_FORMAT_R16G16B16A16_FLOAT, vs, ps)?;
        let gui_hdr10 = create_gui_pipeline(&device, &root, DXGI_FORMAT_R10G10B10A2_UNORM, vs, ps)?;
        let lut = texture_3d(&device, 2)?;
        let env = env_texture(&device, 1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)?;
        let env_cdf = env_texture(&device, 1, 2, DXGI_FORMAT_R32_FLOAT)?;
        let constants = buffer(
            &device,
            CB_STRIDE * CB_SLOTS,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let geometry = buffer(
            &device,
            256,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let mut query = None;
        device.CreateQueryHeap(
            &D3D12_QUERY_HEAP_DESC {
                Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
                Count: TIMESTAMPS,
                NodeMask: 0,
            },
            &mut query,
        )?;
        let query = query.unwrap();
        let timing = buffer(
            &device,
            8 * TIMESTAMPS as u64,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let mut ngx = null_mut();
        let code = caustica_ngx_init(device.as_raw(), &mut ngx);
        let rr_status = if ngx_ok(code) {
            "DLSS Ray Reconstruction available".into()
        } else {
            format!("NGX unavailable (0x{code:08x}); using native denoiser")
        };
        let mut fsr = null_mut();
        let fsr_code = caustica_fsr_init(device.as_raw(), &mut fsr);
        let mut xess = null_mut();
        let xess_code = caustica_xess_init(device.as_raw(), &mut xess);
        let init_reasons = {
            let nvidia = adapter_name.to_ascii_lowercase().contains("nvidia");
            let caps = if ngx.is_null() {
                0
            } else {
                caustica_ngx_caps(ngx)
            };
            let ngx_reason = |bit: u32, what: &str| {
                if !ngx_ok(code) && !nvidia {
                    format!("needs an NVIDIA RTX GPU (this GPU: {adapter_name})")
                } else if !ngx_ok(code) {
                    format!(
                        "NGX did not start (0x{code:08x}); update the driver, and check that nvngx_dlss*.dll is next to the exe"
                    )
                } else if caps & bit == 0 {
                    format!("this GPU or driver does not offer {what}")
                } else {
                    String::new()
                }
            };
            [
                ngx_reason(1, "Ray Reconstruction"),
                ngx_reason(2, "DLSS Super Resolution"),
                match fsr_code {
                    _ if !fsr.is_null() => String::new(),
                    1 => "amd_fidelityfx_loader_dx12.dll is missing next to the exe".into(),
                    c => format!("the FidelityFX loader did not start ({c})"),
                },
                match xess_code {
                    _ if !xess.is_null() => String::new(),
                    1 => "libxess.dll is missing next to the exe".into(),
                    2 => "libxess.dll lacks a needed function (an old version?)".into(),
                    c => format!("XeSS did not start (result {})", c & 0xff),
                },
            ]
        };
        eprintln!(
            "GPU: {adapter_name}; {rr_status}; DLSS SR {}; FSR {}; XeSS {}",
            if !ngx.is_null() && caustica_ngx_caps(ngx) & 2 != 0 {
                "available".to_string()
            } else {
                "unavailable".into()
            },
            if fsr.is_null() {
                format!("unavailable ({fsr_code})")
            } else {
                "available".into()
            },
            if xess.is_null() {
                format!("unavailable (0x{xess_code:x})")
            } else {
                "available".into()
            }
        );
        let mut gpu = Self {
            device,
            queue,
            swap,
            allocator,
            list,
            fence,
            event,
            fence_value: 0,
            heap,
            rtv,
            root,
            trace,
            trace_maps,
            trace_lean,
            trace_maps_lean,
            trace_spectral,
            trace_spectral_lean,
            ser: ser_pipelines,
            trace_paths,
            path_buffer,
            path_readback,
            path_request: None,
            path_result: None,
            trace_lpe,
            lpe_table,
            lpe_uploaded: None,
            lens_buffer,
            lens_params: None,
            lens_surfaces: 0,
            glare,
            glare_buffer,
            glare_kernel: None,
            lpe_slots: 0,
            lpe_aov: None,
            lpe_aov_len: 0,
            restir_presample,
            restir_initial,
            restir_spatial,
            reservoirs: None,
            reservoirs_len: 0,
            restir_history: false,
            restir_layers: 0,
            restir_ran: false,
            bdpt,
            bdpt_resolve,
            photon_clear,
            photon_trace,
            photon_trace_spectral,
            photon_gather,
            aux: None,
            aux_len: 0,
            splat_offset: 0,
            photon_frames: 0,
            photon_scale: 1.,
            bounds: [0., 0., 0., 2.],
            bounds_valid: false,
            battle: [BattleQuadrant::default(); 4],
            textured: false,
            lean_scene: false,
            filter,
            display,
            gui,
            gui_hdr,
            gui_hdr10,
            hdr_active: false,
            hdr10: false,
            hdr10_refused: false,
            hdr_levels: [200., 1000.],
            painting: false,
            lut,
            lut_size: 0,
            env,
            env_cdf,
            env_inv_norm: 0.,
            env_scale: 1.,
            present_count: 0,
            zebras: false,
            scopes_on: false,
            scopes: vec![],
            probe: None,
            probe_result: None,
            readback: None,
            scope_zero: None,
            color_space_ok: true,
            buffers: vec![],
            textures: vec![],
            constants,
            material_buffer: geometry.clone(),
            emitter_buffer: geometry.clone(),
            emitter_lookup: geometry.clone(),
            triangle_count: 0,
            emitter_count: 0,
            sun_count: 0,
            mesh_vertices: geometry.clone(),
            static_vertices: geometry.clone(),
            instance_table: geometry,
            static_cache: vec![],
            static_groups: vec![],
            sdf_emitters: [0; 2],
            sdf_blas: None,
            meshes: Default::default(),
            instances: vec![],
            tlas: None,
            gui_textures: Default::default(),
            free_slots: (FIRST_GUI_SLOT..HEAP_SIZE).rev().collect(),
            maps: Default::default(),
            free_maps: (0..MAX_MAPS as u32).rev().collect(),
            keep_ui_frame: false,
            ui_frame: None,
            ngx,
            rr_status,
            fsr,
            xess,
            sr_created: None,
            sr_failed: vec![],
            sr_reset: true,
            sr_fresh: 0,
            upscale_status: String::new(),
            rr_created: false,
            rr_failed: false,
            rr_active: false,
            width,
            height,
            swap_size: [width, height],
            rw: 0,
            rh: 0,
            samples: 0,
            frame: 0,
            gpu_ms: 0.,
            fence_wait_ms: 0.,
            pass_ms: [0.; 4],
            adapter_name,
            init_reasons,
            previous_vp: Mat4::IDENTITY,
            rr_reset: true,
            restarts: 0,
            sequence_base: 0,
            history_valid: false,
            integrated_valid: false,
            camera_moved: false,
            reference_valid: false,
            query,
            timing,
        };
        gpu.resize(width, height, s)?;
        gpu.rebuild_scene(s)?;
        Ok(gpu)
    }
    pub unsafe fn begin(&self) -> Result<()> {
        self.allocator.Reset()?;
        self.list.Reset(&self.allocator, None)?;
        Ok(())
    }
    pub unsafe fn submit(&mut self) -> Result<()> {
        self.list.Close()?;
        self.queue.ExecuteCommandLists(&[Some(self.list.cast()?)]);
        self.fence_value += 1;
        self.queue.Signal(&self.fence, self.fence_value)?;
        if self.fence.GetCompletedValue() < self.fence_value {
            let start = std::time::Instant::now();
            self.fence
                .SetEventOnCompletion(self.fence_value, self.event)?;
            ensure!(
                WaitForSingleObject(self.event, 30_000)
                    == windows::Win32::Foundation::WAIT_OBJECT_0,
                "GPU fence timeout; device removed: {:?}",
                self.device.GetDeviceRemovedReason()
            );
            self.fence_wait_ms += start.elapsed().as_secs_f64() * 1000.;
        }
        Ok(())
    }
    pub fn reset(&mut self, reset_history: bool) {
        self.samples = 0;
        self.sr_fresh = 0;
        // Each restart takes a fresh, 8192-aligned Sobol block, so a moving camera
        // never reuses the previous frame's sample points.
        self.restarts = self.restarts.wrapping_add(1);
        self.sequence_base = self.restarts.wrapping_mul(0x9E37_79B1) & !8191;
        self.photon_frames = 0;
        self.photon_scale = 1.;
        for q in &mut self.battle {
            *q = BattleQuadrant {
                cost: q.cost,
                ..Default::default()
            };
        }
        if reset_history {
            self.rr_reset = true;
            self.history_valid = false;
            self.restir_history = false;
            self.camera_moved = false;
        } else {
            self.camera_moved = true;
        }
    }
    /// Releases every neural/temporal upscaler so the next frame recreates the one in use.
    pub unsafe fn reconfigure_upscalers(&mut self) -> Result<()> {
        self.wait_idle()?;
        self.release_upscalers();
        self.reset(true);
        Ok(())
    }
    unsafe fn release_upscalers(&mut self) {
        caustica_ngx_release(self.ngx);
        caustica_fsr_release(self.fsr);
        self.rr_created = false;
        self.rr_failed = false;
        self.sr_created = None;
        self.sr_failed.clear();
    }
    /// What each reconstruction backend can do here, with the reason when it can't: the
    /// startup checks plus failures met while rendering (UI plan 5.2).
    pub fn capabilities(&self) -> Capabilities {
        let backend = |ok: bool, i: usize, failed: bool| Backend {
            available: ok && !failed,
            reason: if failed && ok {
                "failed while rendering (see the status line); tried again after a restart".into()
            } else {
                self.init_reasons[i].clone()
            },
        };
        let sr = |u: Upscaler, i: usize| {
            backend(self.upscaler_available(u), i, self.sr_failed.contains(&u))
        };
        Capabilities {
            rr: backend(self.rr_available(), 0, self.rr_failed),
            dlss_sr: sr(Upscaler::Dlss, 1),
            fsr: sr(Upscaler::Fsr, 2),
            xess: sr(Upscaler::Xess, 3),
        }
    }
    /// NGX loaded and reports Ray Reconstruction (caps bit 0).
    pub fn rr_available(&self) -> bool {
        !self.ngx.is_null() && unsafe { caustica_ngx_caps(self.ngx) } & 1 != 0
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        match kind {
            Upscaler::Dlss => {
                !self.ngx.is_null() && unsafe { caustica_ngx_caps(self.ngx) } & 2 != 0
            }
            Upscaler::Fsr => !self.fsr.is_null(),
            Upscaler::Xess => !self.xess.is_null(),
            Upscaler::Bilinear | Upscaler::Bicubic => true,
        }
    }
    /// Creates the selected temporal upscaler if needed. Returns it when it can run this frame.
    unsafe fn prepare_upscaler(&mut self, s: &Settings) -> Option<Upscaler> {
        let kind = s.upscaler;
        if !s.perspective() {
            self.upscale_status = "Upscaling needs a perspective camera; bicubic".into();
            return None;
        }
        if self.painting {
            self.upscale_status = "Light painting accumulates at render resolution".into();
            return None;
        }
        if s.battle {
            self.upscale_status = "Battle mode compares raw samples".into();
            return None;
        }
        // At 100% only DLAA / Native AA (a temporal upscaler, when asked for) runs.
        if self.rr_active || (self.rw >= self.width && !(s.native_aa && kind.temporal())) {
            self.upscale_status.clear();
            return None;
        }
        if !kind.temporal() {
            self.upscale_status = format!("{} upscaling", kind.label());
            return None;
        }
        if !self.upscaler_available(kind) || self.sr_failed.contains(&kind) {
            if !self.sr_failed.contains(&kind) {
                self.upscale_status = format!("{} unavailable; bicubic upscaling", kind.label());
            }
            return None;
        }
        if self.sr_created == Some(kind) {
            return Some(kind);
        }
        let (w, h, ow, oh) = (self.rw, self.rh, self.width, self.height);
        let (ok, code) = match kind {
            Upscaler::Dlss => {
                let code = caustica_ngx_create_sr(
                    self.ngx,
                    self.list.as_raw(),
                    w,
                    h,
                    ow,
                    oh,
                    s.dlss_preset,
                );
                (ngx_ok(code), code)
            }
            Upscaler::Fsr => {
                let code = caustica_fsr_create(self.fsr, w, h, ow, oh);
                (code == 0, code)
            }
            _ => {
                let code = caustica_xess_create(self.xess, w, ow, oh, 1.);
                (code == 0, code)
            }
        };
        if ok {
            self.sr_created = Some(kind);
            self.sr_reset = true;
            self.upscale_status = match kind {
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
            Some(kind)
        } else {
            self.sr_failed.push(kind);
            self.upscale_status = format!(
                "{} failed to start (0x{code:08x}); bicubic upscaling",
                kind.label()
            );
            eprintln!("{}", self.upscale_status);
            None
        }
    }
    unsafe fn wait_idle(&mut self) -> Result<()> {
        // Present queues work after the command-list fence. Include it before resizing or shutdown.
        self.fence_value += 1;
        self.queue.Signal(&self.fence, self.fence_value)?;
        self.fence
            .SetEventOnCompletion(self.fence_value, self.event)?;
        ensure!(
            WaitForSingleObject(self.event, 30_000) == windows::Win32::Foundation::WAIT_OBJECT_0,
            "GPU idle timeout"
        );
        Ok(())
    }
    fn swap_format(&self) -> DXGI_FORMAT {
        if self.hdr_active && self.hdr10 {
            DXGI_FORMAT_R10G10B10A2_UNORM
        } else if self.hdr_active {
            DXGI_FORMAT_R16G16B16A16_FLOAT
        } else {
            DXGI_FORMAT_R8G8B8A8_UNORM
        }
    }
    /// Switches the swapchain between 8-bit SDR, RGBA16F scRGB and (with `hdr10`) R10G10B10A2
    /// HDR10. Recreates the render-size textures like a resize. HDR10 falls back to scRGB when
    /// refused; returns false when the swapchain can't present scRGB either.
    pub unsafe fn set_hdr(&mut self, on: bool, hdr10: bool, s: &Settings) -> Result<bool> {
        if on == self.hdr_active && (!on || hdr10 == self.hdr10) {
            return Ok(true);
        }
        // Color-space support depends on the buffer format, so switch first, then check.
        self.hdr_active = on;
        self.hdr10 = on && hdr10;
        let [w, h] = self.swap_size;
        self.resize(w, h, s)?;
        if on && hdr10 && !self.color_space_ok {
            // HDR10 refused: try scRGB before giving up on HDR.
            self.hdr10_refused = true;
            self.hdr10 = false;
            self.resize(w, h, s)?;
        }
        if on && !self.color_space_ok {
            self.hdr_active = false;
            self.hdr10 = false;
            self.resize(w, h, s)?;
            return Ok(false);
        }
        Ok(true)
    }
    /// Zeroes the scope counters (u28) with a copy from a zero buffer.
    unsafe fn clear_scopes(&mut self) -> Result<()> {
        let t = self.textures[28].resource.clone();
        let (layout, bytes) = footprint(&self.device, &t);
        if self.scope_zero.is_none() {
            let zero = buffer(
                &self.device,
                bytes,
                D3D12_HEAP_TYPE_UPLOAD,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_GENERIC_READ,
            )?;
            write_buffer(&zero, 0, &vec![0u8; bytes as usize])?;
            self.scope_zero = Some(zero);
        }
        transition(
            &self.list,
            &t,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(self.scope_zero.clone()),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        Ok(())
    }
    /// Readback buffer layout: scopes at 0, then the probe texels (linear output, albedo).
    fn readback_layout(&self) -> (u64, u64, u64) {
        let scopes = SCOPES[0] as u64 * 4 * SCOPES[1] as u64;
        let probe_linear = scopes.next_multiple_of(512);
        (scopes, probe_linear, probe_linear + 512)
    }
    /// Uploads a film-look LUT (`size`³ RGB, red fastest, as in .cube files); `None` clears it.
    pub unsafe fn set_lut(&mut self, lut: Option<(u32, &[[f32; 3]])>) -> Result<()> {
        self.wait_idle()?;
        let Some((size, data)) = lut else {
            self.lut_size = 0;
            return Ok(());
        };
        ensure!(
            data.len() == (size * size * size) as usize,
            "LUT size mismatch"
        );
        let t = texture_3d(&self.device, size)?;
        let (layout, bytes) = footprint(&self.device, &t.resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let row = layout.Footprint.RowPitch as u64;
        for z in 0..size as u64 {
            for y in 0..size as u64 {
                let start = ((z * size as u64 + y) * size as u64) as usize;
                let texels: Vec<f32> = data[start..start + size as usize]
                    .iter()
                    .flat_map(|c| [c[0], c[1], c[2], 1.])
                    .collect();
                write_buffer(
                    &staging,
                    (z * size as u64 + y) * row,
                    bytemuck::cast_slice(&texels),
                )?;
            }
        }
        self.begin()?;
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t.resource);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        self.lut = t;
        self.lut_size = size;
        self.lut_view();
        Ok(())
    }
    /// Texture maps on the GPU (core track 5).
    pub fn map_count(&self) -> usize {
        self.maps.len()
    }
    pub fn environment_loaded(&self) -> bool {
        self.env_inv_norm > 0.
    }
    /// Uploads an HDRI and its sampling tables (environment.rs); `None` clears it.
    pub unsafe fn set_environment(
        &mut self,
        env: Option<&crate::environment::EnvMap>,
    ) -> Result<()> {
        self.wait_idle()?;
        let Some(env) = env else {
            self.env_inv_norm = 0.;
            return Ok(());
        };
        let (w, h) = (env.width as u32, env.height as u32);
        // Half floats: an 8k map is 256 MB instead of 512 MB.
        let peak = env
            .texels
            .iter()
            .flat_map(|t| t[..3].iter().copied())
            .fold(0f32, f32::max);
        let scale = (peak / 60000.).max(1.);
        let halves: Vec<u16> = env
            .texels
            .iter()
            .flat_map(|t| [t[0] / scale, t[1] / scale, t[2] / scale, 1.])
            .map(f32_to_half)
            .collect();
        let texels = env_texture(&self.device, w, h, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
        // The CDF rows, then their cutpoints (environment::build).
        let cdf = env_texture(&self.device, w, 2 * (h + 1), DXGI_FORMAT_R32_FLOAT)?;
        self.upload_texture(&texels, bytemuck::cast_slice(&halves), w as usize * 8)?;
        self.upload_texture(&cdf, bytemuck::cast_slice(&env.cdf), w as usize * 4)?;
        self.env = texels;
        self.env_cdf = cdf;
        self.env_inv_norm = env.inv_norm;
        self.env_scale = scale;
        self.env_views();
        Ok(())
    }
    /// Copies tightly packed rows (`row_bytes` each) into a UAV texture, through a staging
    /// buffer laid out by the copy footprint.
    unsafe fn upload_texture(&mut self, t: &Texture, data: &[u8], row_bytes: usize) -> Result<()> {
        let (layout, bytes) = footprint(&self.device, &t.resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let pitch = layout.Footprint.RowPitch as u64;
        for (y, row) in data.chunks_exact(row_bytes).enumerate() {
            write_buffer(&staging, y as u64 * pitch, row)?;
        }
        self.begin()?;
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t.resource);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()
    }
    /// UAVs of the environment (heap slot 30) and its CDFs (31).
    unsafe fn env_views(&self) {
        for (t, slot) in [(&self.env, TEXTURES + 1), (&self.env_cdf, TEXTURES + 2)] {
            let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: t.format,
                ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
                Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_UAV::default(),
                },
            };
            self.device
                .CreateUnorderedAccessView(&t.resource, None, Some(&desc), self.cpu(slot));
        }
    }
    /// The LUT's UAV at heap slot 29 (after the per-resize textures).
    unsafe fn lut_view(&self) {
        let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
            Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                Texture3D: D3D12_TEX3D_UAV {
                    MipSlice: 0,
                    FirstWSlice: 0,
                    WSize: u32::MAX,
                },
            },
        };
        self.device.CreateUnorderedAccessView(
            &self.lut.resource,
            None,
            Some(&desc),
            self.cpu(TEXTURES),
        );
    }
    /// Copies the scope counters and the probe texels into the readback buffer.
    unsafe fn queue_readbacks(&mut self, probe: Option<[u32; 2]>) -> Result<()> {
        let (_, probe_linear, probe_albedo) = self.readback_layout();
        if self.readback.is_none() {
            self.readback = Some(buffer(
                &self.device,
                probe_albedo + 1024,
                D3D12_HEAP_TYPE_READBACK,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            )?);
        }
        let readback = self.readback.clone().unwrap();
        let copy = |index: usize, rect: [u32; 4], offset: u64, format: DXGI_FORMAT, row: u32| {
            let t = &self.textures[index].resource;
            transition(
                &self.list,
                t,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            let mut src = location(t);
            let mut dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(readback.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: offset,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: format,
                            Width: rect[2] - rect[0],
                            Height: rect[3] - rect[1],
                            Depth: 1,
                            RowPitch: row,
                        },
                    },
                },
            };
            let area = D3D12_BOX {
                left: rect[0],
                top: rect[1],
                front: 0,
                right: rect[2],
                bottom: rect[3],
                back: 1,
            };
            self.list
                .CopyTextureRegion(&dst, 0, 0, 0, &src, Some(&area));
            ManuallyDrop::drop(&mut src.pResource);
            ManuallyDrop::drop(&mut dst.pResource);
            transition(
                &self.list,
                t,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        };
        if self.scopes_on {
            copy(
                28,
                [0, 0, SCOPES[0], SCOPES[1]],
                0,
                DXGI_FORMAT_R32_UINT,
                SCOPES[0] * 4,
            );
        }
        if let Some([x, y]) = probe {
            copy(
                18,
                [x, y, x + 1, y + 1],
                probe_linear,
                DXGI_FORMAT_R32G32B32A32_FLOAT,
                256,
            );
            let rx = (x * self.rw / self.width).min(self.rw - 1);
            let ry = (y * self.rh / self.height).min(self.rh - 1);
            copy(
                3,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo,
                DXGI_FORMAT_R32G32B32A32_FLOAT,
                256,
            );
            copy(
                5,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo + 512,
                DXGI_FORMAT_R32_FLOAT,
                256,
            );
            copy(
                19,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo + 768,
                DXGI_FORMAT_R32_FLOAT,
                256,
            );
        }
        Ok(())
    }
    unsafe fn finish_readbacks(&mut self, probe: Option<[u32; 2]>) -> Result<()> {
        self.probe_result = None;
        let Some(readback) = &self.readback else {
            return Ok(());
        };
        if !self.scopes_on && probe.is_none() {
            return Ok(());
        }
        let (scopes, probe_linear, probe_albedo) = self.readback_layout();
        let mut p = null_mut();
        readback.Map(0, None, Some(&mut p))?;
        let base = p as *const u8;
        if self.scopes_on {
            self.scopes =
                std::slice::from_raw_parts(base as *const u32, (scopes / 4) as usize).to_vec();
        }
        if probe.is_some() {
            let texel = |offset: u64| {
                let f = std::slice::from_raw_parts(base.add(offset as usize) as *const f32, 4);
                [f[0], f[1], f[2], f[3]]
            };
            self.probe_result = Some(Probe {
                linear: texel(probe_linear),
                albedo: texel(probe_albedo),
                depth: texel(probe_albedo + 512)[0],
                id: texel(probe_albedo + 768)[0] as u32,
            });
        }
        readback.Unmap(0, None);
        Ok(())
    }
    /// Swaps in freshly compiled shaders (hot reload). Every pipeline is built before any is
    /// replaced, so a failure keeps the old ones. Restarts the accumulation.
    pub unsafe fn reload_shaders(&mut self, sh: &crate::hotreload::Shaders) -> Result<()> {
        let trace = create_compute_pipeline(&self.device, &self.root, &sh.trace)?;
        let trace_maps = create_compute_pipeline(&self.device, &self.root, &sh.trace_maps)?;
        let trace_lean = create_compute_pipeline(&self.device, &self.root, &sh.trace_lean)?;
        let trace_maps_lean =
            create_compute_pipeline(&self.device, &self.root, &sh.trace_maps_lean)?;
        let trace_spectral = create_compute_pipeline(&self.device, &self.root, &sh.trace_spectral)?;
        let trace_spectral_lean =
            create_compute_pipeline(&self.device, &self.root, &sh.trace_spectral_lean)?;
        let trace_paths = create_compute_pipeline(&self.device, &self.root, &sh.trace_paths)?;
        let trace_lpe = create_compute_pipeline(&self.device, &self.root, &sh.trace_lpe)?;
        let restir_presample =
            create_compute_pipeline(&self.device, &self.root, &sh.restir_presample)?;
        let restir_initial = create_compute_pipeline(&self.device, &self.root, &sh.restir_initial)?;
        let restir_spatial = create_compute_pipeline(&self.device, &self.root, &sh.restir_spatial)?;
        let bdpt = create_compute_pipeline(&self.device, &self.root, &sh.bdpt)?;
        let bdpt_resolve = create_compute_pipeline(&self.device, &self.root, &sh.bdpt_resolve)?;
        let photon_clear = create_compute_pipeline(&self.device, &self.root, &sh.photon_clear)?;
        let photon_trace = create_compute_pipeline(&self.device, &self.root, &sh.photon_trace)?;
        let photon_trace_spectral =
            create_compute_pipeline(&self.device, &self.root, &sh.photon_trace_spectral)?;
        let photon_gather = create_compute_pipeline(&self.device, &self.root, &sh.photon_gather)?;
        let filter = create_compute_pipeline(&self.device, &self.root, &sh.filter)?;
        let display = create_compute_pipeline(&self.device, &self.root, &sh.display)?;
        let glare = create_compute_pipeline(&self.device, &self.root, &sh.glare)?;
        let gui = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            &sh.vs,
            &sh.ps,
        )?;
        let gui_hdr = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            &sh.vs,
            &sh.ps,
        )?;
        let gui_hdr10 = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R10G10B10A2_UNORM,
            &sh.vs,
            &sh.ps,
        )?;
        self.wait_idle()?;
        (self.trace_lean, self.trace_maps_lean) = (trace_lean, trace_maps_lean);
        self.trace_spectral = trace_spectral;
        self.trace_spectral_lean = trace_spectral_lean;
        self.photon_trace_spectral = photon_trace_spectral;
        if self.ser.is_some() {
            self.ser = make_ser(
                &self.device,
                &self.root,
                ser_support(&self.device),
                [
                    &sh.trace_ser,
                    &sh.trace_maps_ser,
                    &sh.trace_lean_ser,
                    &sh.trace_maps_lean_ser,
                    &sh.trace_lpe_ser,
                    &sh.trace_spectral_ser,
                    &sh.trace_spectral_lean_ser,
                ],
            );
        }
        (
            self.trace,
            self.trace_maps,
            self.trace_paths,
            self.trace_lpe,
            self.restir_presample,
            self.restir_initial,
            self.restir_spatial,
            (
                self.bdpt,
                self.bdpt_resolve,
                self.photon_clear,
                self.photon_trace,
                self.photon_gather,
            ),
            self.filter,
            (self.display, self.glare),
            self.gui,
            self.gui_hdr,
        ) = (
            trace,
            trace_maps,
            trace_paths,
            trace_lpe,
            restir_presample,
            restir_initial,
            restir_spatial,
            (
                bdpt,
                bdpt_resolve,
                photon_clear,
                photon_trace,
                photon_gather,
            ),
            filter,
            (display, glare),
            gui,
            gui_hdr,
        );
        self.gui_hdr10 = gui_hdr10;

        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    /// Resizes the swapchain and the output to the same size.
    pub unsafe fn resize(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        self.resize_swap(w, h)?;
        self.resize_output(w, h, s)
    }
    /// Resizes only the swapchain. While the output has another size (a video export), each
    /// frame clears the backbuffer instead of copying the image into it, then draws the UI.
    pub unsafe fn resize_swap(&mut self, w: u32, h: u32) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid viewport size"
        );
        // The previous submit has completed. Reset bindings before releasing swap-chain resources.
        self.wait_idle()?;
        self.begin()?;
        self.list.Close()?;
        self.buffers.clear();
        self.ui_frame = None;
        let resized = self
            .swap
            .ResizeBuffers(2, w, h, self.swap_format(), DXGI_SWAP_CHAIN_FLAG(0))
            .and_then(|_| {
                let space = if self.hdr_active && self.hdr10 {
                    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
                } else if self.hdr_active {
                    DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
                } else {
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709
                };
                let support = self.swap.CheckColorSpaceSupport(space)?;
                self.color_space_ok =
                    support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32 != 0;
                if self.color_space_ok {
                    self.swap.SetColorSpace1(space)?;
                }
                Ok(())
            });
        if let Err(e) = resized {
            if let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
                for i in 0..q.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                    let mut len = 0;
                    let _ = q.GetMessage(DXGI_DEBUG_ALL, i, None, &mut len);
                    let mut bytes = vec![0u64; len.div_ceil(8)];
                    let p = bytes.as_mut_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                    if q.GetMessage(DXGI_DEBUG_ALL, i, Some(p), &mut len).is_ok() {
                        eprintln!(
                            "DXGI: {}",
                            String::from_utf8_lossy(std::slice::from_raw_parts(
                                (*p).pDescription as _,
                                (*p).DescriptionByteLength
                            ))
                        );
                    }
                }
            }
            return Err(e.into());
        }
        let step = self
            .device
            .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
            as usize;
        let start = self.rtv.GetCPUDescriptorHandleForHeapStart();
        for i in 0..2 {
            let b: ID3D12Resource = self.swap.GetBuffer(i)?;
            self.device.CreateRenderTargetView(
                &b,
                None,
                D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: start.ptr + i as usize * step,
                },
            );
            self.buffers.push(b);
        }
        self.swap_size = [w, h];
        Ok(())
    }
    /// Re-creates the output and render-size textures; the swapchain keeps its size.
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid output size"
        );
        self.wait_idle()?;
        self.begin()?;
        self.list.Close()?;
        self.release_upscalers();
        self.textures.clear();
        self.width = w;
        self.height = h;
        self.rw = ((w as f32 * s.scale).round() as u32).max(1);
        self.rh = ((h as f32 * s.scale).round() as u32).max(1);
        for i in 0..TEXTURES {
            let format = match i {
                5 | 7 | 17 | 19 | 20 | 25 => DXGI_FORMAT_R32_FLOAT,
                6 => DXGI_FORMAT_R32G32_FLOAT,
                10 => DXGI_FORMAT_R8G8B8A8_UNORM,
                // HDR10 writes PQ code values that copy straight into the backbuffer.
                26 if self.hdr_active && self.hdr10 => DXGI_FORMAT_R10G10B10A2_UNORM,
                26 => DXGI_FORMAT_R16G16B16A16_FLOAT,
                27 => DXGI_FORMAT_R32G32_FLOAT,
                28 => DXGI_FORMAT_R32_UINT,
                _ => DXGI_FORMAT_R32G32B32A32_FLOAT,
            };
            let (tw, th) = if [10, 13, 14, 18, 26].contains(&i) {
                (w, h)
            } else if i == 28 {
                (SCOPES[0], SCOPES[1])
            } else {
                (self.rw, self.rh)
            };
            let t = texture(
                &self.device,
                tw,
                th,
                format,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )?;
            let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: format,
                ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
                Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_UAV::default(),
                },
            };
            self.device
                .CreateUnorderedAccessView(&t.resource, None, Some(&desc), self.cpu(i));
            self.textures.push(t);
        }
        self.lut_view();
        self.env_views();
        self.rr_active = false;
        self.reference_valid = false;
        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    fn cpu(&self, index: usize) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        unsafe {
            D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + index
                        * self.device.GetDescriptorHandleIncrementSize(
                            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        ) as usize,
            }
        }
    }
    fn gpu(&self, index: usize) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        unsafe {
            D3D12_GPU_DESCRIPTOR_HANDLE {
                ptr: self.heap.GetGPUDescriptorHandleForHeapStart().ptr
                    + index as u64
                        * self.device.GetDescriptorHandleIncrementSize(
                            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        ) as u64,
            }
        }
    }
    pub unsafe fn rebuild_scene(&mut self, s: &Settings) -> Result<()> {
        let packed = crate::lighting::materials(s);
        let needs_any_hit = |m: u32| {
            let m = &packed[m as usize];
            (m.lobes[3] as u32 & 2) != 0 || m.lobes[2] > 0.
        };
        self.begin()?;
        let mut keep: Vec<ID3D12Resource> = vec![];
        // Static geometry (floor, room, lights) in world space, rebuilt only when it changes.
        // Its triangles are grouped by `instance_mask` like objects (track 20a (a)): lights
        // hidden from the camera and clear glass become opaque instances with their mask bit,
        // so only triangles under both rules still run the shader per candidate. Each group is
        // its own BLAS and instance; the instance record's vertex_base is the group's first
        // triangle in `static_vertices`.
        let soup = crate::geometry::static_geometry(s);
        // Measured by photon mapping when it next runs: it walks every object's vertices, which
        // made each frame of a timeline export (a rebuild per frame) 10 ms slower.
        self.bounds_valid = false;
        type Group<'a> = ((u32, bool), Vec<&'a [Vertex; 3]>);
        let mut groups: Vec<Group> = vec![];
        for t in soup.as_chunks::<3>().0 {
            let key = instance_mask(&packed[t[0].material as usize]);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(t),
                None => groups.push((key, vec![t])),
            }
        }
        // Opaque 0xff first, so plain scenes keep one static instance at InstanceID 0.
        groups.sort_by_key(|g| g.0 != (0xff, false));
        let layout: Vec<StaticGroup> = {
            let mut first = 0;
            groups
                .iter()
                .map(|g| {
                    let group = StaticGroup {
                        mask: g.0.0,
                        any_hit: g.0.1,
                        first,
                        count: g.1.len() as u32,
                        blas: None,
                    };
                    first += group.count;
                    group
                })
                .collect()
        };
        let statics: Vec<Vertex> = groups
            .into_iter()
            .flat_map(|g| g.1)
            .flatten()
            .copied()
            .collect();
        let same_layout = layout.len() == self.static_groups.len()
            && layout.iter().zip(&self.static_groups).all(|(a, b)| {
                (a.mask, a.any_hit, a.first, a.count) == (b.mask, b.any_hit, b.first, b.count)
            });
        if self.static_groups.is_empty()
            || !same_layout
            || bytemuck::cast_slice::<Vertex, u8>(&statics)
                != bytemuck::cast_slice::<Vertex, u8>(&self.static_cache)
        {
            // An empty room still needs a vertex buffer to bind.
            let bytes: &[u8] = if statics.is_empty() {
                &[0; size_of::<Vertex>()]
            } else {
                bytemuck::cast_slice(&statics)
            };
            self.static_vertices = self.gpu_buffer(bytes, &mut keep)?;
            let base = self.static_vertices.GetGPUVirtualAddress();
            let mut layout = layout;
            for g in &mut layout {
                let flags = if g.any_hit {
                    D3D12_RAYTRACING_GEOMETRY_FLAG_NONE
                } else {
                    D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE
                };
                let address = base + (g.first as usize * 3 * size_of::<Vertex>()) as u64;
                g.blas = Some(
                    self.build_blas(&[triangles(address, g.count as usize, flags)], &mut keep)?,
                );
            }
            self.static_groups = layout;
            self.static_cache = statics;
        }
        // One BLAS per distinct mesh in object space (core track 4). They are rebuilt only when
        // the set of meshes changes; the shared vertex buffer then holds every mesh in use.
        // Track 20a (a): an alpha-tested object whose material needs no other per-candidate
        // test gets a BLAS with opacity micromaps, keyed by mesh, base map and cutoff.
        let omm_of = |i: usize, o: &crate::scene::Object, key: &str| {
            if !omm_on() || !o.material.texture.is_empty() {
                return None;
            }
            let maps = crate::mesh::maps(o)?;
            let base = maps.base.filter(|_| maps.cutoff > 0.)?;
            (instance_mask(&packed[5 + i]) == (0xff, false)).then(|| {
                let omm_key = format!(
                    "{key}#omm{:x}/{}",
                    std::sync::Arc::as_ptr(&base) as usize,
                    maps.cutoff.to_bits()
                );
                (omm_key, base, maps.cutoff)
            })
        };
        let visible: Vec<VisibleMesh> = s
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.visible && o.sdf.is_none())
            .map(|(i, o)| {
                let (key, source) = crate::geometry::object_source(o);
                let omm = omm_of(i, o, &key);
                VisibleMesh {
                    index: i,
                    blas_key: omm.as_ref().map_or_else(|| key.clone(), |m| m.0.clone()),
                    key,
                    source,
                    omm: omm.map(|m| (m.1, m.2)),
                }
            })
            .collect();
        let mut wanted: Vec<&str> = visible.iter().map(|v| v.blas_key.as_str()).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut have: Vec<&str> = self.meshes.keys().map(String::as_str).collect();
        have.sort_unstable();
        if wanted != have {
            let mut local: Vec<Vertex> = vec![];
            let mut bases = std::collections::HashMap::new();
            for VisibleMesh {
                index: i,
                key,
                source,
                ..
            } in &visible
            {
                if bases.contains_key(key) {
                    continue;
                }
                bases.insert(key.clone(), (local.len() as u32, source.len() / 3));
                let uv = crate::mesh::uvs(&s.objects[*i]).filter(|u| u.len() == source.len());
                local.extend(source.iter().enumerate().map(|(k, (p, n))| Vertex {
                    position: p.to_array(),
                    material: 0,
                    normal: n.to_array(),
                    pad: 0,
                    uv: uv.as_ref().map_or([0.; 2], |u| u[k]),
                }));
            }
            if local.is_empty() {
                local.push(bytemuck::Zeroable::zeroed());
            }
            self.mesh_vertices = self.gpu_buffer(bytemuck::cast_slice(&local), &mut keep)?;
            let base = self.mesh_vertices.GetGPUVirtualAddress();
            let mut meshes = std::collections::HashMap::new();
            for v in &visible {
                if meshes.contains_key(&v.blas_key) {
                    continue;
                }
                let (first, count) = bases[&v.key];
                let address = base + first as u64 * size_of::<Vertex>() as u64;
                // A BLAS holds its own copy of the triangles, so ones already built are reused.
                let (blas, omm) = match self.meshes.remove(&v.blas_key) {
                    Some(old) => (old.blas, old._omm),
                    None => match &v.omm {
                        Some((map, cutoff)) => {
                            let uvs: Vec<[f32; 2]> = local
                                [first as usize..first as usize + count * 3]
                                .iter()
                                .map(|v| v.uv)
                                .collect();
                            self.omm_blas(address, count, &uvs, map, *cutoff, &mut keep)?
                        }
                        None => (
                            self.build_blas(
                                &[triangles(
                                    address,
                                    count,
                                    D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
                                )],
                                &mut keep,
                            )?,
                            None,
                        ),
                    },
                };
                meshes.insert(
                    v.blas_key.clone(),
                    MeshBlas {
                        blas,
                        first,
                        count,
                        _omm: omm,
                        _map: v.omm.as_ref().map(|(m, _)| m.clone()),
                    },
                );
            }
            self.meshes = meshes;
        }
        // Instances: the static BLAS (InstanceID 0), then one per visible object. Flags decide
        // opacity per object: glass runs the any-hit loop so shadow rays may pass it.
        let cull = D3D12_RAYTRACING_INSTANCE_FLAG_TRIANGLE_CULL_DISABLE.0 as u32;
        let mut descs: Vec<u8> = vec![];
        let mut table: Vec<InstanceRecord> = vec![];
        let identity = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
        let mut push =
            |transform: &[f32; 12], id: u32, flags: u32, blas: &ID3D12Resource, mask: u32| {
                descs.extend_from_slice(bytemuck::cast_slice(transform));
                descs.extend_from_slice(&(id | mask << 24).to_le_bytes());
                descs.extend_from_slice(&(flags << 24).to_le_bytes());
                descs.extend_from_slice(&blas.GetGPUVirtualAddress().to_le_bytes());
            };
        // Static instances first: emitter_base 0, so emitter_lookup takes the static triangle
        // index (vertex_base + primitive) directly.
        for g in &self.static_groups {
            let opacity = if g.any_hit {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
            } else {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
            } as u32;
            push(
                &identity,
                table.len() as u32,
                cull | opacity,
                g.blas.as_ref().unwrap(),
                g.mask,
            );
            table.push(InstanceRecord {
                vertex_base: g.first,
                material: u32::MAX,
                emitter_base: 0,
                object: u32::MAX,
            });
        }
        let mut triangle_count = self.static_cache.len() / 3;
        for VisibleMesh {
            index: i,
            blas_key,
            omm,
            ..
        } in &visible
        {
            let mesh = &self.meshes[blas_key];
            let material = 5 + *i as u32;
            // Masked maps (alpha cutoff) are tested in the any-hit loop as well.
            let o = &s.objects[*i];
            let masked = o.material.texture.is_empty()
                && crate::mesh::maps(o).is_some_and(|m| m.cutoff > 0.);
            // Track 20a (a): an object's two skip rules (hidden from camera rays, glass that
            // shadow rays pass) become instance masks, which the rays leave out (pathtrace.hlsl
            // MASK_*), so the RT cores commit its hits without running the shader per candidate.
            // Only alpha-tested objects, and ones under both rules, stay non-opaque.
            let (mask, any_hit) = instance_mask(&packed[material as usize]);
            // With micromaps the geometry's own non-opaque flag stands, so the RT cores read
            // them (a FORCE flag would override them).
            let opacity = if omm.is_some() {
                0
            } else if any_hit || masked {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
            } else {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
            } as u32;
            push(
                &crate::geometry::object_transform(&s.objects[*i]),
                table.len() as u32,
                cull | opacity,
                &mesh.blas,
                if masked { 0xff } else { mask },
            );
            table.push(InstanceRecord {
                vertex_base: mesh.first,
                material,
                emitter_base: u32::MAX,
                object: *i as u32,
            });
            triangle_count += mesh.count;
        }
        // Procedural objects (track 13): instances of the unit-cube AABB; the shader intersects
        // the surface (kind in vertex_base). Glass and hidden ones are non-opaque like meshes,
        // though the shader sees every procedural candidate either way.
        if s.objects.iter().any(|o| o.visible && !o.sdf.is_none()) {
            if self.sdf_blas.is_none() {
                let aabb = self.gpu_buffer(
                    bytemuck::cast_slice(&[-1f32, -1., -1., 1., 1., 1.]),
                    &mut keep,
                )?;
                let desc = D3D12_RAYTRACING_GEOMETRY_DESC {
                    Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
                    Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
                    Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
                        AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
                            AABBCount: 1,
                            AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                                StartAddress: aabb.GetGPUVirtualAddress(),
                                StrideInBytes: 24,
                            },
                        },
                    },
                };
                self.sdf_blas = Some(self.build_blas(&[desc], &mut keep)?);
                keep.push(aabb);
            }
            let blas = self.sdf_blas.clone().unwrap();
            for (i, o) in s.objects.iter().enumerate() {
                if !o.visible || o.sdf.is_none() {
                    continue;
                }
                let material = 5 + i as u32;
                let opacity = if needs_any_hit(material) {
                    D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
                } else {
                    D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
                } as u32;
                push(
                    &crate::geometry::object_transform(o),
                    table.len() as u32,
                    cull | opacity,
                    &blas,
                    0xff,
                );
                table.push(InstanceRecord {
                    vertex_base: o.sdf.id(o.phase),
                    material,
                    emitter_base: u32::MAX,
                    object: i as u32,
                });
            }
        }
        let instances = upload(&self.device, &descs)?;
        let ti = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
            NumDescs: table.len() as u32,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                InstanceDescs: instances.GetGPUVirtualAddress(),
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device
            .GetRaytracingAccelerationStructurePrebuildInfo(&ti, &mut info);
        let tlas = buffer(
            &self.device,
            info.ResultDataMaxSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
        )?;
        let ts = buffer(
            &self.device,
            info.ScratchDataSizeInBytes.max(256),
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        // Each BLAS build above ends with a UAV barrier.
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: tlas.GetGPUVirtualAddress(),
                Inputs: ti,
                ScratchAccelerationStructureData: ts.GetGPUVirtualAddress(),
                ..Default::default()
            },
            None,
        );
        uav(&self.list, Some(&tlas));
        self.submit()?;
        drop((keep, instances, ts));
        self.tlas = Some(tlas);
        self.instances = table;
        self.triangle_count = triangle_count;
        self.update_materials(s)?;
        if !self.painting {
            self.reset(true);
        }
        Ok(())
    }
    /// A DEFAULT-heap buffer filled through a staging copy recorded on the open command list
    /// (the staging buffer goes into `keep` until the list has run).
    unsafe fn gpu_buffer(
        &mut self,
        bytes: &[u8],
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<ID3D12Resource> {
        let staging = upload(&self.device, bytes)?;
        // Buffers start in COMMON; the copy promotes it to COPY_DEST.
        let b = buffer(
            &self.device,
            bytes.len() as u64,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.list
            .CopyBufferRegion(&b, 0, &staging, 0, bytes.len() as u64);
        transition(
            &self.list,
            &b,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
        keep.push(staging);
        Ok(b)
    }
    /// Track 20a (a): a BLAS whose triangles link to opacity micromaps built from the base
    /// map's alpha (`omm::build`), with the micromap array it needs kept alive beside it.
    /// Falls back to a plain BLAS when there is nothing to build.
    unsafe fn omm_blas(
        &mut self,
        address: u64,
        count: usize,
        uvs: &[[f32; 2]],
        map: &crate::mesh::Map,
        cutoff: f32,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<(ID3D12Resource, Option<ID3D12Resource>)> {
        let plain = triangles(address, count, D3D12_RAYTRACING_GEOMETRY_FLAG_NONE);
        let Some(mut maps) = crate::omm::build(uvs, map.width, map.height, &map.rgba, cutoff)
            .filter(|m| m.index.len() == count)
        else {
            return Ok((self.build_blas(&[plain], keep)?, None));
        };
        // Every triangle settled by a special index still needs an array to link: one unused
        // level-1 micromap.
        if maps.is_trivial() {
            maps.data.extend([0; 4]);
            maps.descs.push((0, 1));
        }
        let data = self.gpu_buffer(&maps.data, keep)?;
        let descs: Vec<u32> = maps
            .descs
            .iter()
            .flat_map(|&(offset, level)| [offset, level | OMM_FORMAT_4_STATE << 16])
            .collect();
        let descs = self.gpu_buffer(bytemuck::cast_slice(&descs), keep)?;
        let index = self.gpu_buffer(bytemuck::cast_slice(&maps.index), keep)?;
        let histogram: Vec<OmmHistogramEntry> = maps
            .histogram()
            .into_iter()
            .map(|(level, count)| OmmHistogramEntry {
                count,
                level,
                format: OMM_FORMAT_4_STATE,
            })
            .collect();
        let array_desc = OmmArrayDesc {
            histogram_entries: histogram.len() as u32,
            histogram: histogram.as_ptr(),
            input: data.GetGPUVirtualAddress(),
            descs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                StartAddress: descs.GetGPUVirtualAddress(),
                StrideInBytes: 8,
            },
        };
        let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE(OMM_ARRAY_TYPE),
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
            NumDescs: 1,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                pGeometryDescs: &array_desc as *const OmmArrayDesc as *const _,
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device
            .GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
        ensure!(
            info.ResultDataMaxSizeInBytes > 0,
            "the driver reports no size for an opacity micromap array"
        );
        let array = buffer(
            &self.device,
            info.ResultDataMaxSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
        )?;
        let scratch = buffer(
            &self.device,
            info.ScratchDataSizeInBytes.max(256),
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: array.GetGPUVirtualAddress(),
                Inputs: inputs,
                ScratchAccelerationStructureData: scratch.GetGPUVirtualAddress(),
                ..Default::default()
            },
            None,
        );
        uav(&self.list, Some(&array));
        keep.extend([data, descs, scratch]);
        // The triangles, linked to the array: per triangle a micromap or a special index.
        let tri = plain.Anonymous.Triangles;
        let linkage = OmmLinkage {
            index: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                StartAddress: index.GetGPUVirtualAddress(),
                StrideInBytes: 4,
            },
            index_format: DXGI_FORMAT_R32_UINT,
            base_location: 0,
            array: array.GetGPUVirtualAddress(),
        };
        let mut desc = D3D12_RAYTRACING_GEOMETRY_DESC {
            Type: D3D12_RAYTRACING_GEOMETRY_TYPE(OMM_GEOMETRY_TYPE),
            Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
            Anonymous: std::mem::zeroed(),
        };
        std::ptr::write(
            &mut desc.Anonymous as *mut _ as *mut OmmTriangles,
            OmmTriangles {
                triangles: &tri,
                linkage: &linkage,
            },
        );
        let blas = self.build_blas(&[desc], keep)?;
        keep.push(index);
        Ok((blas, Some(array)))
    }
    /// Frame timestamp k (0 start, 1 after the trace, 2 after reconstruction, 3 after display,
    /// 4 after the UI) and named events between them, so Nsight and PIX split their metrics per
    /// pass (track 20a step 1).
    unsafe fn stamp(&self, k: u32) {
        const PASSES: [&str; 4] = ["trace", "reconstruction", "display", "ui"];
        if k > 0 {
            self.list.EndEvent();
        }
        self.list
            .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, k);
        if let Some(name) = PASSES.get(k as usize) {
            // Metadata 0: a UTF-16 string (WINPIX_EVENT_UNICODE_VERSION; 1 would be ANSI).
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            self.list
                .BeginEvent(0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32);
        }
    }
    /// Records a BLAS build on the open command list (scratch goes into `keep`).
    unsafe fn build_blas(
        &mut self,
        descs: &[D3D12_RAYTRACING_GEOMETRY_DESC],
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<ID3D12Resource> {
        let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
            NumDescs: descs.len() as u32,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                pGeometryDescs: descs.as_ptr(),
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device
            .GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
        let blas = buffer(
            &self.device,
            info.ResultDataMaxSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
        )?;
        let scratch = buffer(
            &self.device,
            info.ScratchDataSizeInBytes.max(256),
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: blas.GetGPUVirtualAddress(),
                Inputs: inputs,
                ScratchAccelerationStructureData: scratch.GetGPUVirtualAddress(),
                ..Default::default()
            },
            None,
        );
        uav(&self.list, Some(&blas));
        keep.push(scratch);
        Ok(blas)
    }
    /// Materials, the emitter table and the instance table. Emitters stay world-space
    /// triangles: the static ones, then each emissive object's (its lookup range is
    /// `emitter_base` in the instance table; other objects have none).
    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        let materials = crate::lighting::materials(s);
        let mut world = self.static_cache.clone();
        for record in self.instances.iter_mut().filter(|r| r.material != u32::MAX) {
            let m = &materials[record.material as usize];
            let emission =
                m.color_rough[0].max(m.color_rough[1]).max(m.color_rough[2]) * m.optical[1];
            // Procedural objects are not light-sampled: their emission is found by BSDF hits.
            record.emitter_base =
                if emission > 0. && s.objects[record.object as usize].sdf.is_none() {
                    let base = (world.len() / 3) as u32;
                    world.extend(crate::geometry::object_geometry(
                        &s.objects[record.object as usize],
                        record.material,
                    ));
                    base
                } else {
                    u32::MAX
                };
        }
        let analytic = crate::lighting::analytic(s);
        let (emitters, lookup) = crate::lighting::emitter_table(&world, &materials, &analytic);
        self.sun_count = crate::lighting::sun_count(&emitters);
        let mut materials = materials;
        self.attach_maps(s, &mut materials)?;
        // Only real material records: the portal matrices after them hold a translation in optical.w.
        self.textured = materials
            .iter()
            .take(5 + s.objects.len() + s.lights.len())
            .any(|m| m.lobes[3] as u32 & 16 != 0 || m.optical[3] < 0.);
        // The LEAN variants' material gates (pathtrace.hlsl lean_strip), over every record: a
        // portal matrix can only turn them off.
        self.lean_scene = !s.objects.iter().any(|o| o.visible && !o.sdf.is_none())
            && materials.iter().all(|m| {
                m.layers[0] <= 0.
                    && m.layers[3] <= 0.
                    && m.sheen[..3].iter().all(|v| *v <= 0.)
                    && m.surface_scale[2] <= 0.
                    && m.medium[3] <= 0.
            });
        self.emitter_count = emitters.len();
        self.material_buffer = upload(&self.device, bytemuck::cast_slice(&materials))?;
        // The alias table for ReSTIR follows the emitters (`emitter_count` stays the table),
        // then the CDF's cutpoints for `select_index`.
        let mut table = emitters.clone();
        table.extend(crate::lighting::alias_records(&emitters));
        table.extend(crate::lighting::guide_records(&emitters));
        // Emissive procedural objects for photon mapping (`sdf_emitter_records`), last.
        let sdf = sdf_emitter_records(s, &self.instances, &materials);
        self.sdf_emitters = [(sdf.len() / 2) as u32, table.len() as u32];
        table.extend(sdf);
        self.emitter_buffer = upload(&self.device, bytemuck::cast_slice(&table))?;
        self.emitter_lookup = upload(&self.device, bytemuck::cast_slice(&lookup))?;
        self.instance_table = upload(&self.device, bytemuck::cast_slice(&self.instances))?;
        // Light painting keeps the exposure, but reservoirs made for the old lights would be
        // reused with stale weights.
        self.restir_history = false;
        if !self.painting {
            self.reset(true);
        }
        Ok(())
    }
    unsafe fn bind(&self, offset: u64) {
        self.list.SetDescriptorHeaps(&[Some(self.heap.clone())]);
        self.list.SetComputeRootSignature(&self.root);
        self.list
            .SetComputeRootConstantBufferView(0, self.constants.GetGPUVirtualAddress() + offset);
        self.list.SetComputeRootShaderResourceView(
            1,
            self.tlas.as_ref().unwrap().GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(2, self.mesh_vertices.GetGPUVirtualAddress());
        self.list.SetComputeRootDescriptorTable(3, self.gpu(0));
        self.list
            .SetComputeRootShaderResourceView(5, self.material_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(6, self.emitter_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(7, self.emitter_lookup.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(8, self.static_vertices.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(9, self.instance_table.GetGPUVirtualAddress());
        self.list
            .SetComputeRootDescriptorTable(10, self.gpu(HEAP_SIZE));
        self.list
            .SetComputeRootUnorderedAccessView(11, self.path_buffer.GetGPUVirtualAddress());
        // Without AOVs no shader reads u1 space 2 (display checks the slot count).
        self.list.SetComputeRootUnorderedAccessView(
            12,
            self.lpe_aov
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(13, self.lpe_table.GetGPUVirtualAddress());
        // Without ReSTIR no shader reads u2 space 2 (restir.x is 0).
        self.list.SetComputeRootUnorderedAccessView(
            14,
            self.reservoirs
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        // Only the BDPT and photon passes read u3 space 2, and they run only with a buffer.
        self.list.SetComputeRootUnorderedAccessView(
            15,
            self.aux
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(16, self.lens_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootUnorderedAccessView(17, self.glare_buffer.GetGPUVirtualAddress());
    }
    /// Sizes `aux` (u3 space 2) for the integrators in use: the photon grid and photons at 0
    /// (photon mapping), the light-tracing splats after them (BDPT, one rgb float triple per
    /// render pixel, zeroed here and by bdpt_resolve after every frame).
    unsafe fn prepare_aux(&mut self, s: &Settings, keep: &mut Vec<ID3D12Resource>) -> Result<()> {
        let photons = s.integrator == 2 || s.battle;
        let splats = s.integrator == 1 || s.battle;
        let photon_bytes = if photons {
            PHOTON_BASE + s.photons as u64 * s.bounces.clamp(1, PHOTON_BOUNCES) as u64 * 32
        } else {
            0
        };
        let splat_bytes = if splats {
            self.rw as u64 * self.rh as u64 * 12
        } else {
            0
        };
        let len = photon_bytes + splat_bytes;
        if len == self.aux_len && photon_bytes == self.splat_offset {
            return Ok(());
        }
        self.splat_offset = photon_bytes;
        self.aux_len = len;
        if len == 0 {
            self.aux = None;
            return Ok(());
        }
        let aux = buffer(
            &self.device,
            len,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        if splat_bytes > 0 {
            let zero = upload(&self.device, &vec![0u8; splat_bytes as usize])?;
            self.list
                .CopyBufferRegion(&aux, photon_bytes, &zero, 0, splat_bytes);
            transition(
                &self.list,
                &aux,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
            keep.push(zero);
        }
        if let Some(old) = self.aux.replace(aux) {
            keep.push(old);
        }
        Ok(())
    }
    /// Runs one integrator over `region` with the constants `c` (written at `offset`): ReSTIR's
    /// passes and the path tracer (0), BDPT and its splat resolve (1), or photon mapping's
    /// grid clear, photon trace and gather (2). `photon_boost` multiplies the photon count
    /// (battle mode's photon quadrant spends its time share on photons).
    #[allow(clippy::too_many_arguments)]
    unsafe fn integrate(
        &mut self,
        s: &Settings,
        c: &Constants,
        offset: u64,
        integrator: u32,
        restir: bool,
        region: [u32; 4],
        n: u32,
        photon_boost: f32,
    ) -> Result<()> {
        let mut c = *c;
        if integrator == 2 && !self.bounds_valid {
            // The bounding sphere sun and sky photons start from. The floor (material 3) is
            // left out: it often reaches far past the scene.
            let (mut lo, mut hi) = s.objects_bounds();
            for v in self.static_cache.iter().filter(|v| v.material != 3) {
                lo = lo.min(Vec3::from(v.position));
                hi = hi.max(Vec3::from(v.position));
            }
            // Fog scatters light toward the camera wherever it is: the sphere covers the fog
            // box too, or multiple scattering outside the objects' sphere would be missing
            // (underwater: 42% dark).
            if s.fog.density > 0. && lo.is_finite() {
                let f = &s.fog;
                lo = lo.min(Vec3::new(-f.extent, -1., -f.extent));
                hi = hi.max(Vec3::new(f.extent, f.height, f.extent));
            }
            self.bounds = if lo.is_finite() {
                let c = (lo + hi) * 0.5;
                [c.x, c.y, c.z, ((hi - lo).length() * 0.5).max(0.5)]
            } else {
                [0., 0., 0., 2.]
            };
            self.bounds_valid = true;
        }
        c.bounds = self.bounds;
        let (w, h) = (region[2] - region[0], region[3] - region[1]);
        let share = (w * h) as f32 / (self.rw * self.rh).max(1) as f32;
        c.region = region;
        c.sampling[1] = n;
        if !restir {
            c.restir[0] = 0;
        }
        let photons = ((s.photons as f32 * share * photon_boost) as u32).clamp(1024, s.photons);
        let capacity = (self.splat_offset.saturating_sub(PHOTON_BASE) / 32) as u32;
        // Light tracing needs a pinhole perspective camera and samples spread over each pixel.
        let light_tracing = s.perspective() && s.aperture <= 0. && c.scene_info[3] & 1 == 0;
        c.integrator = [
            photons,
            capacity,
            self.splat_offset as u32,
            light_tracing as u32,
        ];
        c.photon = [
            s.photon_radius * self.bounds[3] * self.photon_scale.sqrt(),
            4. * c.environment[1] * c.environment[2] * share,
            self.sdf_emitters[0] as f32,
            self.sdf_emitters[1] as f32,
        ];
        write_buffer(&self.constants, offset, bytemuck::bytes_of(&c))?;
        self.bind(offset);
        let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
        match integrator {
            1 => {
                for pass in [&self.bdpt, &self.bdpt_resolve] {
                    self.list.SetPipelineState(pass);
                    self.list.Dispatch(gx, gy, 1);
                    uav(&self.list, None);
                }
            }
            2 => {
                self.list.SetPipelineState(&self.photon_clear);
                self.list.Dispatch((PHOTON_HASH / 64) as u32, 1, 1);
                uav(&self.list, None);
                self.list.SetPipelineState(if s.spectral {
                    &self.photon_trace_spectral
                } else {
                    &self.photon_trace
                });
                self.list.Dispatch(photons.div_ceil(64), 1, 1);
                uav(&self.list, None);
                self.list.SetPipelineState(&self.photon_gather);
                self.list.Dispatch(gx, gy, 1);
                uav(&self.list, None);
                // Knaus and Zwicker's radius schedule, alpha = 2/3.
                self.photon_frames += 1;
                let i = self.photon_frames as f32;
                self.photon_scale *= (i + 2. / 3.) / (i + 1.);
            }
            _ => {
                if restir {
                    // ReSTIR DI: initial candidates and the temporal merge, then the spatial
                    // merge into the final reservoirs the trace reads.
                    self.list.SetPipelineState(&self.restir_presample);
                    self.list.Dispatch((RESTIR_TILES / 64) as u32, 1, 1);
                    uav(&self.list, None);
                    for pass in [&self.restir_initial, &self.restir_spatial] {
                        self.list.SetPipelineState(pass);
                        self.list.Dispatch(gx, gy, self.restir_layers);
                        uav(&self.list, None);
                    }
                }
                // LEAN exactly when the shader's fog and ReSTIR flags are off for this frame.
                let lean = self.lean_scene && c.fog[0] <= 0. && c.restir[0] & 1 == 0;
                let variant = self.textured as usize + 2 * lean as usize;
                // Light path expressions stay RGB (their tracer has no spectral twin).
                let spectral = s.spectral && self.lpe_slots == 0;
                if let Some(ser) = self.ser.as_ref() {
                    // Track 20: the same threads as the compute dispatch, through
                    // DispatchRays so the shader may reorder them (MaybeReorderThread).
                    let p = &ser[if self.lpe_slots > 0 {
                        4
                    } else if spectral {
                        5 + lean as usize
                    } else {
                        variant
                    }];
                    self.list.SetPipelineState1(&p.state);
                    self.list.DispatchRays(&D3D12_DISPATCH_RAYS_DESC {
                        RayGenerationShaderRecord: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                            StartAddress: p.table.GetGPUVirtualAddress(),
                            SizeInBytes: D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64,
                        },
                        Width: gx * 8,
                        Height: gy * 8,
                        Depth: 1,
                        ..Default::default()
                    });
                } else {
                    self.list.SetPipelineState(if self.lpe_slots > 0 {
                        &self.trace_lpe
                    } else if spectral && lean {
                        &self.trace_spectral_lean
                    } else if spectral {
                        &self.trace_spectral
                    } else {
                        [
                            &self.trace,
                            &self.trace_maps,
                            &self.trace_lean,
                            &self.trace_maps_lean,
                        ][variant]
                    });
                    self.list.Dispatch(gx, gy, 1);
                }
                uav(&self.list, None);
            }
        }
        Ok(())
    }
    /// Battle mode (track 15): the four quadrants, each with its own integrator and sample
    /// count, at equal GPU time: the slowest takes `spp` samples per frame and the others as
    /// many as fit in its time (measured per quadrant with timestamps, at most 64).
    unsafe fn battle_frame(&mut self, s: &Settings, c: &Constants, restir: bool) -> Result<()> {
        let (hw, hh) = (self.rw / 2, self.rh / 2);
        let regions = [
            [0, 0, hw, hh],
            [hw, 0, self.rw, hh],
            [0, hh, hw, self.rh],
            [hw, hh, self.rw, self.rh],
        ];
        let slowest = self.battle.iter().map(|q| q.cost).fold(0., f64::max);
        let budget = slowest * s.spp as f64;
        for (i, region) in regions.into_iter().enumerate() {
            let q = &mut self.battle[i];
            let want = if q.cost > 0. && budget > 0. {
                budget / q.cost + q.carry
            } else {
                s.spp as f64
            };
            // Photon mapping spends its share on photons, not on gathers: each frame's gathers
            // share one photon map, so one sample per frame keeps its samples independent (and
            // its sample cap as far away as the others').
            let photons = i == 3;
            let spp = if photons {
                1
            } else {
                (want.floor() as u32).clamp(1, 64)
            };
            q.scale = if photons { want.clamp(1., 4.) } else { 1. };
            q.carry = if photons {
                0.
            } else {
                (want - spp as f64).clamp(0., 1.)
            };
            let n = spp.min(s.max_samples.saturating_sub(q.samples));
            q.last = n;
            let samples = q.samples;
            let query = BATTLE_QUERY + 2 * i as u32;
            self.list
                .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, query);
            if n > 0 {
                let mut qc = *c;
                qc.sampling[0] = samples;
                let offset = (9 + i as u64) * CB_STRIDE;
                let scale = self.battle[i].scale as f32;
                self.integrate(
                    s,
                    &qc,
                    offset,
                    [0, 0, 1, 2][i],
                    i == 1 && restir,
                    region,
                    n,
                    scale,
                )?;
            }
            self.list
                .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, query + 1);
        }
        if restir && self.battle[1].last > 0 {
            self.restir_history = true;
        }
        Ok(())
    }
    /// Reservoir layers of the last frame's ReSTIR passes (0 when they did not run).
    pub fn restir_layers(&self) -> u32 {
        if self.restir_ran {
            self.restir_layers
        } else {
            0
        }
    }
    /// Sizes the ReSTIR reservoir buffer while ReSTIR is on: one layer (two reservoirs per
    /// render pixel) for each of the frame's first 4 samples, which keeps them independent;
    /// later samples use plain light sampling. A new buffer has no history. True when the
    /// passes run this frame.
    unsafe fn prepare_restir(&mut self, s: &Settings) -> Result<bool> {
        // Battle mode's second quadrant is path tracing with ReSTIR.
        let wanted = (s.restir && s.integrator == 0) || s.battle;
        let on = wanted && self.emitter_count > 0;
        // At most 1 GiB of reservoirs (4K: two layers).
        let pixels = (self.rw as u64 * self.rh as u64).max(1);
        self.restir_layers = s
            .spp
            .clamp(1, 4)
            .min(((1 << 30) / (pixels * 64)).max(1) as u32);
        // Plus the light tiles (pathtrace.hlsl RESTIR_TILES × RESTIR_TILE, 32 B each).
        let len = if wanted {
            self.rw as u64 * self.rh as u64 * self.restir_layers as u64 * 64 + RESTIR_TILES * 32
        } else {
            0
        };
        if len == 0 {
            self.reservoirs = None;
            self.reservoirs_len = 0;
            self.restir_history = false;
        } else if len != self.reservoirs_len {
            self.reservoirs = Some(buffer(
                &self.device,
                len,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON,
            )?);
            self.reservoirs_len = len;
            self.restir_history = false;
        }
        Ok(on)
    }
    /// The image's size in the glare grid, w | h << 16 (the longer side GLARE_IMAGE).
    fn glare_grid(&self) -> u32 {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let (w, h) = if aspect >= 1. {
            (GLARE_IMAGE, GLARE_IMAGE / aspect)
        } else {
            (GLARE_IMAGE * aspect, GLARE_IMAGE)
        };
        (w.round().max(1.) as u32) | (h.round().max(1.) as u32) << 16
    }
    /// Track 16 glare: rebuilds the kernel's spectrum when the aperture changes (its shape into
    /// region 0, FFT, the per-wavelength pattern into region 1, FFT), then convolves this frame's
    /// highlights (region 0: highlights, row FFT, column FFT × kernel × inverse, inverse rows).
    /// The display pass reads region 0.
    unsafe fn run_glare(&mut self, s: &Settings, c: &mut Constants) -> Result<()> {
        let key = [
            s.aperture_blades as f32,
            s.blade_rotation,
            c.glare_settings[2],
        ];
        let mut passes = Vec::new();
        if self.glare_kernel != Some(key) {
            passes.extend([
                [1, 0, 0],
                [2, 0, 0],
                [3, 0, 0],
                [4, 0, 0],
                [2, 1, 0],
                [3, 1, 0],
            ]);
            self.glare_kernel = Some(key);
        }
        passes.extend([[0, 0, 0], [2, 0, 0], [3, 0, 2], [2, 0, 1]]);
        for (i, p) in passes.iter().enumerate() {
            c.glare[..3].copy_from_slice(p);
            let offset = (GLARE_SLOT + i as u64) * CB_STRIDE;
            write_buffer(&self.constants, offset, bytemuck::bytes_of(c))?;
            self.bind(offset);
            self.list.SetPipelineState(&self.glare);
            self.list.Dispatch(GLARE_N as u32, 1, 1);
            uav(&self.list, None);
        }
        c.glare[..3].copy_from_slice(&[0; 3]);
        Ok(())
    }
    /// Builds the lens system's buffer (track 16) when the lens, its stop, focus, field of view,
    /// aspect or dispersion change. The GPU is idle between frames (one frame in flight).
    unsafe fn prepare_lens(&mut self, s: &Settings) -> Result<()> {
        if !s.real_lens() {
            self.lens_surfaces = 0;
            return Ok(());
        }
        let p = crate::lens::Params {
            lens: s.lens_system,
            f_number: s.f_number,
            focus_mm: s.focus_distance * s.lens_scale * 1000.,
            fov: s.fov.to_radians(),
            aspect: self.rw as f32 / self.rh.max(1) as f32,
            dispersion: s.dispersion,
        };
        if self.lens_params.as_ref() != Some(&p) || self.lens_surfaces == 0 {
            let (data, n) = crate::lens::pack(&p);
            write_buffer(&self.lens_buffer, 0, bytemuck::cast_slice(&data))?;
            self.lens_surfaces = n as u32;
            self.lens_params = Some(p);
        }
        Ok(())
    }
    /// Uploads the light path expressions' table when they change, and sizes the AOV buffer
    /// (a new buffer restarts the accumulation, as its contents are undefined).
    unsafe fn prepare_lpe(&mut self, s: &Settings) -> Result<()> {
        if self.lpe_uploaded.as_ref() != Some(&s.lpe) {
            let (table, slots) = crate::lpe::table(&s.lpe);
            write_buffer(&self.lpe_table, 0, bytemuck::cast_slice(&table))?;
            self.lpe_slots = slots;
            self.lpe_uploaded = Some(s.lpe.clone());
        }
        let len = self.rw as u64 * self.rh as u64 * self.lpe_slots as u64 * 16;
        if len == 0 {
            self.lpe_aov = None;
            self.lpe_aov_len = 0;
        } else if len != self.lpe_aov_len {
            self.lpe_aov = Some(buffer(
                &self.device,
                len,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON,
            )?);
            self.lpe_aov_len = len;
            self.reset(false);
        }
        Ok(())
    }
    /// One light path expression's AOV at render resolution, as RGB floats (self-test, tools).
    pub unsafe fn read_lpe(&mut self, slot: u32) -> Result<Vec<f32>> {
        anyhow::ensure!(slot < self.lpe_slots, "LPE slot {slot} is not in use");
        let aov = self.lpe_aov.clone().context("No LPE buffer")?;
        let staging = buffer(
            &self.device,
            self.lpe_aov_len,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        // A buffer used as a UAV decays to COMMON after each submit; COPY_SOURCE is implicit.
        self.list.CopyResource(&staging, &aov);
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let all =
            std::slice::from_raw_parts(p as *const [f32; 4], (self.lpe_aov_len / 16) as usize);
        let out = all
            .iter()
            .skip(slot as usize)
            .step_by(self.lpe_slots as usize)
            .flat_map(|v| [v[0], v[1], v[2]])
            .collect();
        staging.Unmap(0, None);
        Ok(out)
    }
    /// Points each visible object's material at its texture maps (core track 5), uploading
    /// maps that are new and releasing ones no object uses any more.
    unsafe fn attach_maps(
        &mut self,
        s: &Settings,
        materials: &mut [crate::lighting::PackedMaterial],
    ) -> Result<()> {
        let mut used = std::collections::HashSet::new();
        // (material record, maps): visible objects (a texture set wins over the mesh's own
        // maps), then the walls (records 0-2) and the floor (3).
        let set = |m: &crate::scene::Material| {
            (!m.texture.is_empty())
                .then(|| crate::texture::load(&m.texture).ok())
                .flatten()
        };
        let mut wanted: Vec<(usize, crate::mesh::Maps)> = vec![];
        for record in self.instances.iter().filter(|r| r.material != u32::MAX) {
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
                let key = std::sync::Arc::as_ptr(map) as usize;
                used.insert(key);
                if !self.maps.contains_key(&key) {
                    let texture = self.upload_map(map)?;
                    self.maps.insert(key, texture);
                }
                *slot = self.maps[&key].slot + 1;
            }
            let m = &mut materials[record];
            m.maps = [slots[0], slots[1], slots[2], maps.cutoff.to_bits()];
            // Flag bits the shader tests before any map work: 16 has maps, 8 masked.
            let masked = slots[0] != 0 && maps.cutoff > 0.;
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
        let stale: Vec<usize> = self
            .maps
            .keys()
            .filter(|k| !used.contains(k))
            .copied()
            .collect();
        if !stale.is_empty() {
            // Frames in flight may still read them.
            self.wait_idle()?;
            for key in stale {
                let old = self.maps.remove(&key).unwrap();
                self.free_maps.push(old.slot);
            }
        }
        Ok(())
    }
    /// One map as an RGBA8 texture (sRGB view for colour maps) with an SRV in its slot.
    unsafe fn upload_map(&mut self, map: &std::sync::Arc<crate::mesh::Map>) -> Result<MapTexture> {
        let slot = self
            .free_maps
            .pop()
            .context("Out of texture map slots (4096)")?;
        let levels = 1 + map.mips.len();
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: map.width as u64,
            Height: map.height,
            DepthOrArraySize: 1,
            MipLevels: levels as u16,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            ..Default::default()
        };
        let mut resource: Option<ID3D12Resource> = None;
        self.device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut resource,
        )?;
        let t = Texture {
            resource: resource.unwrap(),
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        // Every level through one staging buffer, laid out by the copy footprints.
        let mut layouts = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); levels];
        let mut total = 0;
        self.device.GetCopyableFootprints(
            &desc,
            0,
            levels as u32,
            0,
            Some(layouts.as_mut_ptr()),
            None,
            None,
            Some(&mut total),
        );
        let staging = buffer(
            &self.device,
            total,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let mut mapped = null_mut();
        staging.Map(0, Some(&D3D12_RANGE::default()), Some(&mut mapped))?;
        for (layout, data) in layouts
            .iter()
            .zip(std::iter::once(&map.rgba).chain(&map.mips))
        {
            let row = layout.Footprint.Width as usize * 4;
            for (y, src) in data.chunks_exact(row).enumerate() {
                let at = layout.Offset as usize + y * layout.Footprint.RowPitch as usize;
                std::ptr::copy_nonoverlapping(src.as_ptr(), (mapped as *mut u8).add(at), row);
            }
        }
        staging.Unmap(0, None);
        self.begin()?;
        for (level, layout) in layouts.iter().enumerate() {
            let mut dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(t.resource.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    SubresourceIndex: level as u32,
                },
            };
            let mut src = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(staging.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: *layout,
                },
            };
            self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            ManuallyDrop::drop(&mut src.pResource);
            ManuallyDrop::drop(&mut dst.pResource);
        }
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
        self.submit()?;
        drop(staging);
        let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: if map.srgb {
                DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
            } else {
                DXGI_FORMAT_R8G8B8A8_UNORM
            },
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV {
                    MipLevels: levels as u32,
                    ..Default::default()
                },
            },
        };
        self.device.CreateShaderResourceView(
            &t.resource,
            Some(&desc),
            self.cpu(HEAP_SIZE + slot as usize),
        );
        Ok(MapTexture {
            _texture: t,
            slot,
            _map: map.clone(),
        })
    }
    pub unsafe fn render(
        &mut self,
        s: &Settings,
        draw: bool,
        dt: f32,
        primitives: &[egui::ClippedPrimitive],
        delta: &egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<()> {
        self.begin()?;
        let mut keep = Vec::new();
        self.update_gui_textures(delta, &mut keep)?;
        let (view, proj, eye) = s.camera(self.width as f32 / self.height as f32);
        let vp = proj * view;
        let reset = self.rr_reset;
        // RR and the temporal upscalers need a perspective camera (motion vectors, guides).
        self.rr_active = s.rr && !self.ngx.is_null() && !self.rr_failed && s.perspective() && !self.painting
                // Battle mode compares raw samples.
                && !s.battle;
        if !s.rr && !self.ngx.is_null() {
            self.rr_status = "DLSS RR ready (disabled)".into();
        }
        if self.rr_active && self.rr_created {
            self.rr_status = "DLSS Ray Reconstruction active".into();
        }
        if self.rr_active && !self.rr_created {
            let code = caustica_ngx_create(
                self.ngx,
                self.list.as_raw(),
                self.rw,
                self.rh,
                self.width,
                self.height,
                s.rr_preset,
            );
            if ngx_ok(code) {
                self.rr_created = true;
                self.rr_status = "DLSS Ray Reconstruction active".into();
            } else {
                self.rr_active = false;
                self.rr_failed = true;
                self.rr_status =
                    format!("RR creation failed: 0x{code:08x}; native denoiser active");
            }
        }
        self.prepare_lpe(s)?;
        self.prepare_lens(s)?;
        let restir = self.prepare_restir(s)?;
        let sr = self.prepare_upscaler(s);
        // With a temporal upscaler each frame is fresh jittered samples, as with RR: the native
        // denoiser's history and the upscaler do the accumulation, so detail is resolved above
        // the render resolution. (Feeding an accumulated image measured worse: the upscalers
        // assume every input frame sits at its own jitter.)
        // Once the view has been still for SR_FRESH_FRAMES the upscaler's short history has
        // done what it can, so the samples accumulate from then on and converge to the cap.
        let accumulating = sr.is_some() && self.sr_fresh >= SR_FRESH_FRAMES;
        if sr.is_some() && draw && !accumulating {
            if self.samples > 0 {
                let fresh = self.sr_fresh;
                self.reset(false);
                self.sr_fresh = fresh;
            }
            self.sr_fresh += 1;
        }
        let jittered = self.rr_active || sr.is_some();
        let jitter = if jittered {
            [
                halton(self.frame % 64 + 1, 2) - 0.5,
                halton(self.frame % 64 + 1, 3) - 0.5,
            ]
        } else {
            [0., 0.]
        };
        // Display flags: bit 0 jittered primary rays, bit 1 show the upscaled texture (14),
        // bits 2-3 the spatial upscale filter (1 bilinear, 2 bicubic).
        let spatial = if s.upscaler == Upscaler::Bilinear {
            1
        } else {
            2
        };
        let flags = jittered as u32
            | ((self.rr_active || sr.is_some()) as u32) << 1
            | spatial << 2
            | s.mis_mode.min(2) << 4;
        let n = if draw {
            s.spp.min(s.max_samples.saturating_sub(self.samples))
        } else {
            0
        };
        let lc = s.light_color();
        // The physical sky (track 9) is baked at strength 1 with north at −z.
        let (env_strength, env_rotation, env_visible) = if s.physical_sky.on {
            (1., s.physical_sky.rotation(), s.physical_sky.visible)
        } else {
            (
                s.hdri_strength,
                s.hdri_rotation.to_radians(),
                s.hdri_visible,
            )
        };
        let mut c = Constants {
            inverse_vp: vp.inverse().to_cols_array(),
            current_vp: vp.to_cols_array(),
            previous_vp: if reset { vp } else { self.previous_vp }.to_cols_array(),
            world_view: view.to_cols_array(),
            eye: [eye.x, eye.y, eye.z, 1.],
            dimensions: [self.rw, self.rh, self.width, self.height],
            sampling: [self.samples, n, s.bounces, self.frame],
            lights: [
                self.sun_count as f32,
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
                self.rr_active as u32,
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
                self.reference_valid as u32,
            ],
            scene_info: [
                self.emitter_count as u32,
                s.objects.len() as u32,
                self.sequence_base,
                flags,
            ],
            // View-space ray scale (tan of the half field of view) for the denoiser.
            environment: [s.sky, 1. / proj.x_axis.x, 1. / proj.y_axis.y, 0.],
            hdr: [
                self.hdr_active as u32 as f32,
                self.hdr_levels[0],
                self.hdr_levels[1],
                (self.hdr_active && self.hdr10) as u32 as f32,
            ],
            tools: [
                self.zebras as u32 as f32,
                self.scopes_on as u32 as f32,
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
            look: [s.halation, s.grain, s.vignette, self.lut_size as f32],
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
                (self.present_count % 4096) as f32,
            ],
            environment_map: [
                env_strength * self.env_scale,
                env_rotation,
                if self.env_inv_norm > 0. {
                    1. + if env_visible { 2. } else { 0. }
                } else {
                    0.
                },
                self.env_inv_norm * self.env_scale,
            ],
            fog: [s.fog.density, s.fog.phase, s.fog.height, s.fog.extent],
            fog_color: [s.fog.color[0], s.fog.color[1], s.fog.color[2], 0.],
            region: [0, 0, self.rw, self.rh],
            integrator: [0; 4],
            photon: [0.; 4],
            bounds: [0.; 4],
            real_lens: [
                self.lens_surfaces as f32,
                s.dispersion,
                0.001 / s.lens_scale,
                0.,
            ],
            glare: [0, 0, 0, self.glare_grid()],
            // A smaller aperture in the grid spreads the pattern wider.
            glare_settings: [s.glare, s.glare_threshold, 256. - 232. * s.glare_size, 0.],
            path_record: [0; 4],
            lpe: [self.lpe_slots, s.lpe_slot, 0, 0],
            restir: [
                restir as u32 | (self.restir_history as u32) << 1 | self.restir_layers << 8,
                s.restir_candidates.clamp(1, 32),
                s.restir_neighbors.min(8),
                s.restir_history.min(64),
            ],
        };
        // Path visualizer: the output pixel maps to a render pixel as the light meter's does.
        let path_request = self.path_request.take();
        if let Some(([x, y], count)) = path_request {
            c.path_record = [
                (x * self.rw / self.width.max(1)).min(self.rw.saturating_sub(1)),
                (y * self.rh / self.height.max(1)).min(self.rh.saturating_sub(1)),
                count,
                PATH_CAPACITY as u32,
            ];
        }
        // eye.w scales the UI to paper white in HDR (gui.hlsl); the trace ignores it.
        // Negative: HDR10, so gui.hlsl encodes PQ / BT.2020 instead of scRGB.
        if self.hdr_active {
            c.eye[3] = self.hdr_levels[0] / 80. * if self.hdr10 { -1. } else { 1. };
        }
        write_buffer(&self.constants, 0, bytemuck::bytes_of(&c))?;
        self.stamp(0);
        if n == 0 {
            self.stamp(1);
        }
        if path_request.is_some() {
            // Writes only the path records, so it may run before this frame's trace.
            self.bind(0);
            self.list.SetPipelineState(&self.trace_paths);
            self.list.Dispatch(1, 1, 1);
            uav(&self.list, None);
            transition(
                &self.list,
                &self.path_buffer,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            self.list
                .CopyResource(&self.path_readback, &self.path_buffer);
            transition(
                &self.list,
                &self.path_buffer,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        }
        self.prepare_aux(s, &mut keep)?;
        self.restir_ran = n > 0 && restir && (s.battle || s.integrator == 0);
        if n > 0 {
            if s.battle {
                self.battle_frame(s, &c, restir)?;
            } else {
                let path_restir = restir && s.integrator == 0;
                self.integrate(
                    s,
                    &c,
                    0,
                    s.integrator,
                    path_restir,
                    [0, 0, self.rw, self.rh],
                    n,
                    1.,
                )?;
                if path_restir {
                    self.restir_history = true;
                }
            }
            self.stamp(1);
            if self.rr_active {
                let inputs = [0, 2, 3, 4, 5, 6, 7];
                for i in inputs {
                    transition(
                        &self.list,
                        &self.textures[i].resource,
                        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    );
                }
                let resources: Vec<_> = self.textures.iter().map(|t| t.resource.as_raw()).collect();
                let code = caustica_ngx_eval(
                    self.ngx,
                    self.list.as_raw(),
                    resources.as_ptr(),
                    self.rw,
                    self.rh,
                    jitter[0],
                    jitter[1],
                    (reset || !s.temporal) as i32,
                    dt * 1000.,
                    view.to_cols_array().as_ptr(),
                    proj.to_cols_array().as_ptr(),
                );
                for i in inputs {
                    transition(
                        &self.list,
                        &self.textures[i].resource,
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    );
                }
                uav(&self.list, None);
                if !ngx_ok(code) {
                    self.rr_active = false;
                    self.rr_failed = true;
                    self.rr_status = format!("RR evaluation failed: 0x{code:08x}");
                    eprintln!("{}", self.rr_status);
                }
            }
            self.previous_vp = vp;
            self.rr_reset = false;
        }
        if !self.rr_active && s.denoise && !s.battle {
            // Pass 0 warps the last integrated frame into this view after a camera move; a
            // still camera keeps that history frozen. Pass 1 integrates new samples (skipped
            // while paused). Passes 2–6 are the à-trous filter.
            let traced = n > 0;
            let reproject = traced && self.camera_moved && s.temporal && self.integrated_valid;
            if traced && self.camera_moved {
                self.history_valid = reproject;
            }
            if !s.temporal {
                self.history_valid = false;
            }
            let first = if reproject {
                0
            } else if traced || !self.integrated_valid {
                1
            } else {
                2
            };
            for i in first..7 {
                c.filter[0] = i;
                c.filter[2] = !self.history_valid as u32;
                let offset = (i as u64 + 1) * CB_STRIDE;
                write_buffer(&self.constants, offset, bytemuck::bytes_of(&c))?;
                self.bind(offset);
                self.list.SetPipelineState(&self.filter);
                self.list
                    .Dispatch(self.rw.div_ceil(8), self.rh.div_ceil(8), 1);
                uav(&self.list, None);
                if i == 1 && traced {
                    // The next reprojection checks history against these guides.
                    for (from, to) in [(2, 16), (5, 17), (19, 20)] {
                        copy_texture(
                            &self.list,
                            &self.textures[from].resource,
                            &self.textures[to].resource,
                        );
                    }
                }
            }
            self.integrated_valid = true;
        } else if n > 0 {
            self.history_valid = false;
            self.integrated_valid = false;
        }
        if n > 0 {
            self.camera_moved = false;
        }
        if let Some(kind) = sr
            && (n > 0 || self.sr_reset)
        {
            // An accumulated input is the mean over every jitter phase, so it sits at the pixel centre.
            let jitter = if accumulating { [0., 0.] } else { jitter };
            self.run_upscaler(kind, s, &mut c, jitter, (self.sr_reset || reset) as i32, dt);
        }
        self.stamp(2);
        c.modes[3] = self.rr_active as u32;
        c.filter[0] = 0;
        if s.glare > 0. {
            self.run_glare(s, &mut c)?;
        }
        write_buffer(&self.constants, 8 * CB_STRIDE, bytemuck::bytes_of(&c))?;
        if self.scopes_on {
            self.clear_scopes()?;
        }
        self.bind(8 * CB_STRIDE);
        self.list.SetPipelineState(&self.display);
        self.list
            .Dispatch(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        uav(&self.list, None);
        let probe = self
            .probe
            .filter(|p| p[0] < self.width && p[1] < self.height);
        if self.scopes_on || probe.is_some() {
            self.queue_readbacks(probe)?;
        }
        self.stamp(3);
        let index = self.swap.GetCurrentBackBufferIndex() as usize;
        let back = &self.buffers[index];
        let shown = if self.hdr_active { 26 } else { 10 };
        let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.rtv.GetCPUDescriptorHandleForHeapStart().ptr
                + index
                    * self
                        .device
                        .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                        as usize,
        };
        if self.swap_size == [self.width, self.height] {
            transition(
                &self.list,
                &self.textures[shown].resource,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            self.list.CopyResource(back, &self.textures[shown].resource);
            transition(
                &self.list,
                &self.textures[shown].resource,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
        } else {
            // A video export renders at its own size: the window shows the UI over a plain
            // background (the progress window has its own preview).
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            let grey = if self.hdr_active { 0.02 } else { 0.1 };
            self.list
                .ClearRenderTargetView(handle, &[grey, grey, grey, 1.], None);
        }
        self.list.OMSetRenderTargets(1, Some(&handle), false, None);
        self.draw_gui(primitives, pixels_per_point, &mut keep)?;
        self.stamp(4);
        if !(s.battle && n > 0) {
            // Every query the resolve reads must have been ended this frame.
            for i in BATTLE_QUERY..TIMESTAMPS {
                self.list
                    .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, i);
            }
        }

        self.list.ResolveQueryData(
            &self.query,
            D3D12_QUERY_TYPE_TIMESTAMP,
            0,
            TIMESTAMPS,
            &self.timing,
            0,
        );
        // Freed textures go after the draw that may still use them this frame; one frame in
        // flight means the slot is free again by the next one.
        for id in &delta.free {
            if let Some(old) = self.gui_textures.remove(id) {
                self.free_slots.push(old.slot);
                keep.push(old.texture.resource);
            }
        }
        if self.keep_ui_frame {
            if self.ui_frame.is_none() {
                self.ui_frame = Some(
                    texture(
                        &self.device,
                        self.swap_size[0],
                        self.swap_size[1],
                        self.swap_format(),
                        D3D12_RESOURCE_FLAG_NONE,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                    )?
                    .resource,
                );
            }
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            self.list
                .CopyResource(self.ui_frame.as_ref().unwrap(), back);
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            );
        } else {
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );
        }
        self.submit()?;
        self.swap.Present(s.vsync as u32, DXGI_PRESENT(0)).ok()?;
        self.finish_readbacks(probe)?;
        if path_request.is_some() {
            let mut p = null_mut();
            self.path_readback.Map(0, None, Some(&mut p))?;
            let all = std::slice::from_raw_parts(p as *const crate::paths::Record, PATH_CAPACITY);
            let count = (all[0].info as usize).min(PATH_CAPACITY - 1);
            self.path_result = Some(all[1..=count].to_vec());
            self.path_readback.Unmap(0, None);
        }
        self.present_count = self.present_count.wrapping_add(1);
        let mut data = null_mut();
        let count = TIMESTAMPS as usize;
        self.timing.Map(
            0,
            Some(&D3D12_RANGE {
                Begin: 0,
                End: 8 * count,
            }),
            Some(&mut data),
        )?;
        let times = std::slice::from_raw_parts(data as *const u64, count);
        let ms = 1000. / self.queue.GetTimestampFrequency()? as f64;
        let span = |a: usize, b: usize| times[b].saturating_sub(times[a]) as f64 * ms;
        // Without the UI, as before: trace to the end of display.
        self.gpu_ms = span(0, 3);
        self.pass_ms = [span(0, 1), span(1, 2), span(2, 3), span(3, 4)];
        if s.battle {
            for (i, q) in self.battle.iter_mut().enumerate() {
                if q.last == 0 {
                    continue;
                }
                let ms = span(
                    BATTLE_QUERY as usize + 2 * i,
                    BATTLE_QUERY as usize + 2 * i + 1,
                );
                let per = ms / (q.last as f64 * q.scale.max(1.));
                q.cost = if q.cost > 0. {
                    0.8 * q.cost + 0.2 * per
                } else {
                    per
                };
                q.ms += ms;
                q.samples += q.last;
                q.last = 0;
            }
            self.samples = self.battle.iter().map(|q| q.samples).min().unwrap_or(0);
        } else {
            self.samples += n;
        }
        self.timing.Unmap(0, None);
        if n > 0 {
            self.frame = self.frame.wrapping_add(1);
        }
        Ok(())
    }
    /// Upscales the native color (denoised or raw) with device depth and low-res pixel motion
    /// into texture 14. On failure the display falls back to bicubic.
    unsafe fn run_upscaler(
        &mut self,
        kind: Upscaler,
        s: &Settings,
        c: &mut Constants,
        jitter: [f32; 2],
        history_reset: i32,
        dt: f32,
    ) {
        let color = if s.denoise { 8 } else { 1 };
        let inputs = [color, 25, 6];
        for i in inputs {
            transition(
                &self.list,
                &self.textures[i].resource,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
            );
        }
        let r = |i: usize| self.textures[i].resource.as_raw();
        let (cmd, w, h) = (self.list.as_raw(), self.rw, self.rh);
        let code = match kind {
            Upscaler::Dlss => {
                let code = caustica_ngx_eval_sr(
                    self.ngx,
                    cmd,
                    r(color),
                    r(25),
                    r(6),
                    r(14),
                    w,
                    h,
                    jitter[0],
                    jitter[1],
                    history_reset,
                    dt * 1000.,
                );
                if ngx_ok(code) { 0 } else { code }
            }
            Upscaler::Fsr => caustica_fsr_eval(
                self.fsr,
                cmd,
                r(color),
                r(25),
                r(6),
                r(14),
                w,
                h,
                self.width,
                self.height,
                jitter[0],
                jitter[1],
                history_reset,
                dt * 1000.,
                s.fov.to_radians(),
                0.01,
                500.,
            ),
            _ => caustica_xess_eval(
                self.xess,
                cmd,
                r(color),
                r(25),
                r(6),
                r(14),
                w,
                h,
                jitter[0],
                jitter[1],
                history_reset,
            ),
        };
        for i in inputs {
            transition(
                &self.list,
                &self.textures[i].resource,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        }
        uav(&self.list, None);
        self.sr_reset = false;
        if code != 0 {
            self.sr_failed.push(kind);
            self.sr_created = None;
            c.scene_info[3] &= !2;
            self.upscale_status =
                format!("{} failed (0x{code:08x}); bicubic upscaling", kind.label());
            eprintln!("{}", self.upscale_status);
        }
    }
    unsafe fn update_gui_textures(
        &mut self,
        delta: &egui::TexturesDelta,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        for (id, change) in &delta.set {
            let size = change.image.size();
            let pixels: Vec<egui::Color32> = match &change.image {
                egui::ImageData::Color(i) => i.pixels.clone(),
                egui::ImageData::Font(i) => i.srgba_pixels(None).collect(),
            };
            let (mut data, mut full, slot) = match self.gui_textures.remove(id) {
                Some(old) => {
                    keep.push(old.texture.resource);
                    (old.pixels, old.size, old.slot)
                }
                None => (
                    vec![],
                    [0, 0],
                    self.free_slots.pop().context("Out of egui texture slots")?,
                ),
            };
            if let Some(pos) = change.pos {
                for y in 0..size[1] {
                    let start = ((pos[1] + y) * full[0] + pos[0]) * 4;
                    for x in 0..size[0] {
                        data[start + x * 4..start + x * 4 + 4]
                            .copy_from_slice(&pixels[y * size[0] + x].to_array());
                    }
                }
            } else {
                full = size;
                data = pixels.iter().flat_map(|p| p.to_array()).collect();
            }
            let t = texture(
                &self.device,
                full[0] as _,
                full[1] as _,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            )?;
            let (layout, bytes) = footprint(&self.device, &t.resource);
            let staging = buffer(
                &self.device,
                bytes,
                D3D12_HEAP_TYPE_UPLOAD,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_GENERIC_READ,
            )?;
            for y in 0..full[1] {
                let row = &data[y * full[0] * 4..(y + 1) * full[0] * 4];
                write_buffer(&staging, y as u64 * layout.Footprint.RowPitch as u64, row)?;
            }
            let mut dst = location(&t.resource);
            let mut src = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(staging.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: layout,
                },
            };
            self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            ManuallyDrop::drop(&mut src.pResource);
            ManuallyDrop::drop(&mut dst.pResource);
            transition(
                &self.list,
                &t.resource,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
            let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: t.format,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MipLevels: 1,
                        ..Default::default()
                    },
                },
            };
            self.device
                .CreateShaderResourceView(&t.resource, Some(&srv), self.cpu(slot));
            self.gui_textures.insert(
                *id,
                GuiTexture {
                    texture: t,
                    slot,
                    pixels: data,
                    size: full,
                },
            );
            keep.push(staging);
        }
        Ok(())
    }
    unsafe fn draw_gui(
        &self,
        primitives: &[egui::ClippedPrimitive],
        ppp: f32,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        if self.gui_textures.is_empty() {
            return Ok(());
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct GuiVertex {
            position: [f32; 2],
            uv: [f32; 2],
            color: [u8; 4],
        }
        self.list.SetDescriptorHeaps(&[Some(self.heap.clone())]);
        self.list.SetGraphicsRootSignature(&self.root);
        self.list
            .SetGraphicsRootConstantBufferView(0, self.constants.GetGPUVirtualAddress());
        self.list
            .SetPipelineState(if self.hdr_active && self.hdr10 {
                &self.gui_hdr10
            } else if self.hdr_active {
                &self.gui_hdr
            } else {
                &self.gui
            });
        self.list
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.list.RSSetViewports(&[D3D12_VIEWPORT {
            Width: self.swap_size[0] as f32,
            Height: self.swap_size[1] as f32,
            MaxDepth: 1.,
            ..Default::default()
        }]);
        // One vertex and one index upload per frame; each mesh draws from its offsets.
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        // gui.hlsl maps positions by the output size in the frame constants; during a video
        // export that differs from the swapchain, so positions are scaled to match.
        let fit = [
            ppp * self.width as f32 / self.swap_size[0] as f32,
            ppp * self.height as f32 / self.swap_size[1] as f32,
        ];
        for primitive in primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive
                && !mesh.indices.is_empty()
                && let Some(texture) = self.gui_textures.get(&mesh.texture_id)
            {
                draws.push((
                    primitive.clip_rect,
                    indices.len() as u32,
                    mesh.indices.len() as u32,
                    vertices.len() as i32,
                    texture.slot,
                ));
                indices.extend_from_slice(&mesh.indices);
                vertices.extend(mesh.vertices.iter().map(|v| GuiVertex {
                    position: [v.pos.x * fit[0], v.pos.y * fit[1]],
                    uv: [v.uv.x, v.uv.y],
                    color: v.color.to_array(),
                }));
            }
        }
        if draws.is_empty() {
            return Ok(());
        }
        let vb = upload(&self.device, bytemuck::cast_slice(&vertices))?;
        let ib = upload(&self.device, bytemuck::cast_slice(&indices))?;
        self.list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: vb.GetGPUVirtualAddress(),
                SizeInBytes: (vertices.len() * size_of::<GuiVertex>()) as _,
                StrideInBytes: size_of::<GuiVertex>() as _,
            }]),
        );
        self.list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: ib.GetGPUVirtualAddress(),
            SizeInBytes: (indices.len() * 4) as _,
            Format: DXGI_FORMAT_R32_UINT,
        }));
        let mut bound = usize::MAX;
        for (r, first, count, base, slot) in draws {
            let [sw, sh] = self.swap_size.map(|v| v as f32);
            let left = (r.min.x * ppp).clamp(0., sw) as i32;
            let top = (r.min.y * ppp).clamp(0., sh) as i32;
            let right = (r.max.x * ppp).clamp(left as f32, sw) as i32;
            let bottom = (r.max.y * ppp).clamp(top as f32, sh) as i32;
            // Fully clipped meshes (e.g. a window outside a small swapchain) draw nothing,
            // and the debug layer warns about an empty scissor.
            if right <= left || bottom <= top {
                continue;
            }
            self.list
                .RSSetScissorRects(&[windows::Win32::Foundation::RECT {
                    left,
                    top,
                    right,
                    bottom,
                }]);
            if slot != bound {
                self.list.SetGraphicsRootDescriptorTable(4, self.gpu(slot));
                bound = slot;
            }
            self.list.DrawIndexedInstanced(count, 1, first, base, 0);
        }
        keep.push(vb);
        keep.push(ib);
        Ok(())
    }
    pub unsafe fn capture_reference(&mut self) -> Result<()> {
        self.begin()?;
        copy_texture(
            &self.list,
            &self.textures[18].resource,
            &self.textures[13].resource,
        );
        self.submit()?;
        self.reference_valid = true;
        Ok(())
    }
    pub unsafe fn capture(
        &mut self,
        path: &std::path::Path,
        hdr: bool,
        include_ui: bool,
    ) -> Result<()> {
        let index = if hdr { 18 } else { 10 };
        let resource = if include_ui {
            self.ui_frame
                .clone()
                .context("UI capture needs keep_ui_frame set before rendering")?
        } else {
            self.textures[index].resource.clone()
        };
        let state = if include_ui {
            D3D12_RESOURCE_STATE_COPY_DEST
        } else {
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS
        };
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            state,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            state,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        // A UI capture in HDR mode copies the RGBA16F scRGB backbuffer (HDR10: R10G10B10A2 PQ).
        let ui_hdr = include_ui && self.hdr_active;
        let ui_hdr10 = ui_hdr && self.hdr10;
        // The UI frame has the swapchain's size, the output textures the output's.
        let (width, height) = if include_ui {
            (self.swap_size[0], self.swap_size[1])
        } else {
            (self.width, self.height)
        };
        let row_size = width as usize
            * if hdr {
                16
            } else if ui_hdr && !ui_hdr10 {
                8
            } else {
                4
            };
        let mut packed = Vec::with_capacity(row_size * height as usize);
        for y in 0..height as usize {
            packed.extend_from_slice(std::slice::from_raw_parts(
                (p as *const u8).add(y * layout.Footprint.RowPitch as usize),
                row_size,
            ));
        }
        staging.Unmap(0, None);
        if ui_hdr {
            // Back to SDR: paper white becomes 1.0, then sRGB-encode.
            let scale = 80. / self.hdr_levels[0];
            let scrgb: Vec<f32> = if ui_hdr10 {
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
            packed = scrgb
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
                .collect();
        }
        if hdr {
            let floats: Vec<f32> = packed
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let rgb: Vec<f32> = floats
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2]])
                .collect();
            image::Rgb32FImage::from_raw(width, height, rgb)
                .unwrap()
                .save(path)?;
        } else {
            image::save_buffer(path, &packed, width, height, image::ColorType::Rgba8)?;
        }
        Ok(())
    }
    /// Reads an output-resolution float texture (18 linear RGBA32F, 26 scRGB RGBA16F) as RGBA f32.
    /// Reads the RGBA8 output (texture 10) as tightly packed rows.
    pub unsafe fn read_rgba8(&mut self, index: usize) -> Result<Vec<u8>> {
        ensure!(
            self.textures[index].format == DXGI_FORMAT_R8G8B8A8_UNORM,
            "Texture {index} is not RGBA8"
        );
        let resource = self.textures[index].resource.clone();
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let row = self.width as usize * 4;
        let mut out = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            out.extend_from_slice(std::slice::from_raw_parts(
                (p as *const u8).add(y * layout.Footprint.RowPitch as usize),
                row,
            ));
        }
        staging.Unmap(0, None);
        Ok(out)
    }
    pub unsafe fn read_floats(&mut self, index: usize) -> Result<Vec<f32>> {
        let resource = self.textures[index].resource.clone();
        let half = self.textures[index].format == DXGI_FORMAT_R16G16B16A16_FLOAT;
        let packed10 = self.textures[index].format == DXGI_FORMAT_R10G10B10A2_UNORM;
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for y in 0..self.height as usize {
            let row = (p as *const u8).add(y * layout.Footprint.RowPitch as usize);
            if packed10 {
                for x in 0..self.width as usize {
                    let word = (row.add(x * 4) as *const u32).read_unaligned();
                    out.extend_from_slice(&crate::display::hdr10_to_scrgb(word));
                }
                continue;
            }
            for i in 0..self.width as usize * 4 {
                out.push(if half {
                    half_to_f32((row.add(i * 2) as *const u16).read_unaligned())
                } else {
                    (row.add(i * 4) as *const f32).read_unaligned()
                });
            }
        }
        staging.Unmap(0, None);
        Ok(out)
    }
    pub unsafe fn debug_messages(&self) -> Vec<String> {
        let mut result = vec![];
        if let Ok(q) = self.device.cast::<ID3D12InfoQueue>() {
            for i in 0..q.GetNumStoredMessagesAllowedByRetrievalFilter() {
                let mut len = 0;
                let _ = q.GetMessage(i, None, &mut len);
                let mut bytes = vec![0u64; len.div_ceil(8)];
                let p = bytes.as_mut_ptr() as *mut D3D12_MESSAGE;
                if q.GetMessage(i, Some(p), &mut len).is_ok() {
                    let msg = &*p;
                    let text: String = String::from_utf8_lossy(std::slice::from_raw_parts(
                        msg.pDescription,
                        msg.DescriptionByteLength.saturating_sub(1),
                    ))
                    .into();
                    // The Agility SDK's layer (track 20) flags every CheckFeatureSupport as
                    // superseded by ID3D12Device16::CheckFeatureSupport1, which the windows
                    // crate doesn't bind; the old call still answers correctly.
                    // It also notes every buffer created in a state other than COMMON, which
                    // the upscaler SDKs (DLSS, FSR, XeSS) do for theirs; the runtime uses COMMON.
                    let superseded = text
                        .contains("superseded by ID3D12Device16::CheckFeatureSupport1")
                        || (text.contains("Ignoring InitialState")
                            && text.contains("Buffers are effectively created in state"));
                    if msg.Severity.0 <= D3D12_MESSAGE_SEVERITY_WARNING.0 && !superseded {
                        result.push(text);
                    }
                }
            }
            q.ClearStoredMessages();
        }
        result
    }
}
impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.wait_idle();
            caustica_ngx_destroy(self.ngx);
            caustica_fsr_destroy(self.fsr);
            caustica_xess_destroy(self.xess);
            let _ = CloseHandle(self.event);
        }
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
fn half_to_f32(h: u16) -> f32 {
    let (sign, exponent, mantissa) = ((h >> 15) as u32, ((h >> 10) & 31) as i32, (h & 1023) as f32);
    let magnitude = match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1. + mantissa / 1024.) * 2f32.powi(exponent - 15),
    };
    if sign == 1 { -magnitude } else { magnitude }
}
fn bytecode(code: &[u8]) -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: code.as_ptr() as _,
        BytecodeLength: code.len(),
    }
}
/// A mesh's BLAS and its range in the shared object-space vertex buffer.
/// A texture map on the GPU (`Gpu::maps`), holding its source so the key stays unique.
struct MapTexture {
    _texture: Texture,
    slot: u32,
    _map: std::sync::Arc<crate::mesh::Map>,
}
struct MeshBlas {
    blas: ID3D12Resource,
    first: u32,
    count: usize,
    /// The opacity micromap array the BLAS links to (alpha-tested meshes, `omm_blas`).
    _omm: Option<ID3D12Resource>,
    /// The alpha map the micromap was made from: the BLAS key holds its address, so the map
    /// is kept alive while the key is in use (a freed map's address could come back).
    _map: Option<std::sync::Arc<crate::mesh::Map>>,
}
/// Opacity micromaps in use (track 20a (a)): set at device creation when the device has
/// raytracing tier 1.2, unless `CAUSTICA_NO_OMM=1` (A/B checks: the image must not change).
static OMM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn omm_on() -> bool {
    OMM.load(std::sync::atomic::Ordering::Relaxed)
}
// D3D12 opacity micromap structs (Agility SDK 1.620 d3d12.h), not bound by the windows crate.
const OMM_GEOMETRY_TYPE: i32 = 2; // D3D12_RAYTRACING_GEOMETRY_TYPE_OMM_TRIANGLES
const OMM_ARRAY_TYPE: i32 = 2; // D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_OPACITY_MICROMAP_ARRAY
const OMM_FORMAT_4_STATE: u32 = 2; // D3D12_RAYTRACING_OPACITY_MICROMAP_FORMAT_OC1_4_STATE
#[repr(C)]
struct OmmHistogramEntry {
    count: u32,
    level: u32,
    format: u32,
}
#[repr(C)]
struct OmmArrayDesc {
    histogram_entries: u32,
    histogram: *const OmmHistogramEntry,
    input: u64,
    descs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE,
}
#[repr(C)]
struct OmmLinkage {
    index: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE,
    index_format: DXGI_FORMAT,
    base_location: u32,
    array: u64,
}
#[repr(C)]
struct OmmTriangles {
    triangles: *const D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC,
    linkage: *const OmmLinkage,
}
/// Track 15: photon records for emissive procedural (SDF) objects, which the emitter table
/// leaves out (light sampling can't pick points on them). Two records each: the world bounding
/// sphere of the object's unit cube (centre, radius), then its instance's index and the running
/// sum of the selection probabilities (by emission times sphere area), then the world-to-object
/// matrix's three rows. integrators.hlsl `sdf_photon` emits from them by random lines.
fn sdf_emitter_records(
    s: &Settings,
    instances: &[InstanceRecord],
    materials: &[crate::lighting::PackedMaterial],
) -> Vec<crate::lighting::Emitter> {
    let mut found = vec![];
    for (index, record) in instances.iter().enumerate() {
        if record.material == u32::MAX {
            continue;
        }
        let o = &s.objects[record.object as usize];
        let m = &materials[record.material as usize];
        let emission = m.color_rough[0].max(m.color_rough[1]).max(m.color_rough[2]) * m.optical[1];
        if o.sdf.is_none() || emission <= 0. {
            continue;
        }
        let t = crate::geometry::object_transform(o);
        let o2w = Mat4::from_cols_array(&[
            t[0], t[4], t[8], 0., t[1], t[5], t[9], 0., t[2], t[6], t[10], 0., t[3], t[7], t[11],
            1.,
        ]);
        let centre = o2w.transform_point3(Vec3::ZERO);
        let radius = (0..8)
            .map(|k| {
                let corner = Vec3::new(
                    if k & 1 == 0 { -1. } else { 1. },
                    if k & 2 == 0 { -1. } else { 1. },
                    if k & 4 == 0 { -1. } else { 1. },
                );
                (o2w.transform_point3(corner) - centre).length()
            })
            .fold(0f32, f32::max);
        if radius <= 0. || !radius.is_finite() {
            continue;
        }
        found.push((
            index as u32,
            centre,
            radius,
            emission * radius * radius,
            o2w.inverse(),
        ));
    }
    let total: f32 = found.iter().map(|f| f.3).sum();
    let mut sum = 0.;
    let mut out = vec![];
    for (i, (index, centre, radius, power, w2o)) in found.iter().enumerate() {
        sum += power / total;
        let cdf = if i + 1 == found.len() { 1. } else { sum };
        let row = |r: usize| w2o.row(r).to_array();
        out.push(crate::lighting::Emitter {
            position_area: [centre.x, centre.y, centre.z, *radius],
            edge1_cdf: [f32::from_bits(*index), cdf, 0., 0.],
            edge2_probability: row(0),
            emission_material: row(1),
        });
        out.push(crate::lighting::Emitter {
            position_area: row(2),
            edge1_cdf: [0.; 4],
            edge2_probability: [0.; 4],
            emission_material: [0.; 4],
        });
    }
    out
}
/// A visible mesh object in `rebuild_scene`: its mesh key (vertex data), the key of the BLAS it
/// uses (the mesh key, or a micromap variant), and for that variant the base map and cutoff.
struct VisibleMesh {
    index: usize,
    key: String,
    blas_key: String,
    source: crate::geometry::Source,
    omm: Option<(std::sync::Arc<crate::mesh::Map>, f32)>,
}
/// A run of static triangles that share an instance mask and opacity (`instance_mask`).
struct StaticGroup {
    mask: u32,
    any_hit: bool,
    first: u32,
    count: u32,
    blas: Option<ID3D12Resource>,
}
/// Mirrors `Instance` in common.hlsli. `material` u32::MAX marks a static instance, whose
/// vertices carry their own material ids; its `vertex_base` is its first static triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceRecord {
    vertex_base: u32,
    material: u32,
    emitter_base: u32,
    object: u32,
}
/// A triangle geometry of `count` triangles at `address` (32-byte vertices, position first).
fn triangles(
    address: u64,
    count: usize,
    flags: D3D12_RAYTRACING_GEOMETRY_FLAGS,
) -> D3D12_RAYTRACING_GEOMETRY_DESC {
    D3D12_RAYTRACING_GEOMETRY_DESC {
        Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
        Flags: flags,
        Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
            Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
                VertexFormat: DXGI_FORMAT_R32G32B32_FLOAT,
                VertexCount: (count * 3) as u32,
                VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                    StartAddress: address,
                    StrideInBytes: size_of::<Vertex>() as u64,
                },
                ..Default::default()
            },
        },
    }
}
/// A UAV texture that lives across resizes (the environment and its CDFs).
unsafe fn env_texture(
    device: &ID3D12Device5,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
) -> Result<Texture> {
    texture(
        device,
        w,
        h,
        format,
        D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    )
}
/// A cubic RGBA32F UAV texture (the film-look LUT).
unsafe fn texture_3d(device: &ID3D12Device5, size: u32) -> Result<Texture> {
    let mut resource: Option<ID3D12Resource> = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE3D,
            Width: size as u64,
            Height: size,
            DepthOrArraySize: size as u16,
            MipLevels: 1,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            ..Default::default()
        },
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        None,
        &mut resource,
    )?;
    Ok(Texture {
        resource: resource.context("3D texture")?,
        format: DXGI_FORMAT_R32G32B32A32_FLOAT,
    })
}
unsafe fn create_compute_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    code: &[u8],
) -> Result<ID3D12PipelineState> {
    let mut d = D3D12_COMPUTE_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root.clone())),
        CS: bytecode(code),
        // Opacity micromaps for the cs_6_5 tracers' RayQuery too (track 20a (a)):
        // D3D12_PIPELINE_STATE_FLAG_RAYQUERY_ALLOW_OPACITY_MICROMAPS_ON_PRE_SM69_SHADERS.
        Flags: if omm_on() {
            D3D12_PIPELINE_STATE_FLAGS(0x80)
        } else {
            D3D12_PIPELINE_STATE_FLAG_NONE
        },
        ..Default::default()
    };
    let p = device.CreateComputePipelineState(&d);
    ManuallyDrop::drop(&mut d.pRootSignature);
    Ok(p?)
}
/// A SER tracer (track 20): a ray tracing state object with just the `trace_rg` ray
/// generation shader of a DXIL library (it traces with inline RayQuery, so no hit groups),
/// on the compute root signature as its global one, and the one-record shader table.
pub struct SerPipeline {
    state: ID3D12StateObject,
    table: ID3D12Resource,
}
unsafe fn create_ser_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    code: &[u8],
) -> Result<SerPipeline> {
    unsafe {
        let library = D3D12_DXIL_LIBRARY_DESC {
            DXILLibrary: bytecode(code),
            ..Default::default()
        };
        let mut global = D3D12_GLOBAL_ROOT_SIGNATURE {
            pGlobalRootSignature: ManuallyDrop::new(Some(root.clone())),
        };
        // Inline ray queries only: no TraceRay, so the smallest payload and recursion.
        let shader = D3D12_RAYTRACING_SHADER_CONFIG {
            MaxPayloadSizeInBytes: 4,
            MaxAttributeSizeInBytes: 8,
        };
        let pipeline = D3D12_RAYTRACING_PIPELINE_CONFIG {
            MaxTraceRecursionDepth: 1,
        };
        let sub = |t: D3D12_STATE_SUBOBJECT_TYPE, d: *const std::ffi::c_void| {
            D3D12_STATE_SUBOBJECT { Type: t, pDesc: d }
        };
        let subobjects = [
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
                &library as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
                &global as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
                &shader as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
                &pipeline as *const _ as _,
            ),
        ];
        let desc = D3D12_STATE_OBJECT_DESC {
            Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
            NumSubobjects: subobjects.len() as u32,
            pSubobjects: subobjects.as_ptr(),
        };
        let state = device.CreateStateObject::<ID3D12StateObject>(&desc);
        ManuallyDrop::drop(&mut global.pGlobalRootSignature);
        let state = state?;
        let props: ID3D12StateObjectProperties = state.cast()?;
        let id = props.GetShaderIdentifier(windows::core::w!("trace_rg"));
        anyhow::ensure!(!id.is_null(), "trace_rg has no shader identifier");
        let id = std::slice::from_raw_parts(
            id as *const u8,
            D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize,
        );
        // Upload heaps start 64 KiB aligned, past the 64-byte shader table alignment.
        let table = upload(device, id)?;
        Ok(SerPipeline { state, table })
    }
}
/// The SER tracers, when the device supports SER and CAUSTICA_NO_SER isn't set (A/B
/// measurements); a creation failure falls back to the compute tracers with a message.
unsafe fn make_ser(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    support: SerSupport,
    code: [&[u8]; 7],
) -> Option<[SerPipeline; 7]> {
    if !support.available() || std::env::var_os("CAUSTICA_NO_SER").is_some() {
        return None;
    }
    let made: Result<Vec<SerPipeline>> = code
        .iter()
        .map(|c| unsafe { create_ser_pipeline(device, root, c) })
        .collect();
    match made {
        Ok(v) => v.try_into().ok(),
        Err(e) => {
            eprintln!("SER tracers unavailable, using the compute tracers: {e:#}");
            None
        }
    }
}
unsafe fn create_gui_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    format: DXGI_FORMAT,
    vs: &[u8],
    ps: &[u8],
) -> Result<ID3D12PipelineState> {
    let input = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"POSITION".as_ptr() as _),
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"TEXCOORD".as_ptr() as _),
            Format: DXGI_FORMAT_R32G32_FLOAT,
            AlignedByteOffset: 8,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"COLOR".as_ptr() as _),
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            AlignedByteOffset: 16,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
    ];
    let blend = D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D12_BLEND_ONE,
        DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ONE,
        DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        ..Default::default()
    };
    let mut d = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root.clone())),
        VS: bytecode(vs),
        PS: bytecode(ps),
        BlendState: D3D12_BLEND_DESC {
            RenderTarget: [blend; 8],
            ..Default::default()
        },
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        },
        DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
            DepthEnable: false.into(),
            DepthFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            ..Default::default()
        },
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input.as_ptr(),
            NumElements: input.len() as _,
        },
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        RTVFormats: [
            format,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
        ],
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        ..Default::default()
    };
    let result = device.CreateGraphicsPipelineState(&d);
    ManuallyDrop::drop(&mut d.pRootSignature);
    Ok(result?)
}
unsafe fn buffer(
    device: &ID3D12Device5,
    size: u64,
    heap: D3D12_HEAP_TYPE,
    flags: D3D12_RESOURCE_FLAGS,
    state: D3D12_RESOURCE_STATES,
) -> Result<ID3D12Resource> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size.max(256).next_multiple_of(256),
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: flags,
        ..Default::default()
    };
    let mut r = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: heap,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &desc,
        // Default-heap buffers start in COMMON whatever is asked (the Agility SDK's layer
        // warns about any other state, track 20) and promote to UAV / shader-resource use on
        // their own; upload, readback and acceleration-structure states still apply.
        if heap == D3D12_HEAP_TYPE_DEFAULT
            && (state == D3D12_RESOURCE_STATE_UNORDERED_ACCESS
                || state == D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE)
        {
            D3D12_RESOURCE_STATE_COMMON
        } else {
            state
        },
        None,
        &mut r,
    )?;
    Ok(r.unwrap())
}
unsafe fn texture(
    device: &ID3D12Device5,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
    flags: D3D12_RESOURCE_FLAGS,
    state: D3D12_RESOURCE_STATES,
) -> Result<Texture> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: w as u64,
        Height: h,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Flags: flags,
        ..Default::default()
    };
    let mut r = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &desc,
        state,
        None,
        &mut r,
    )?;
    Ok(Texture {
        resource: r.unwrap(),
        format,
    })
}
unsafe fn upload(device: &ID3D12Device5, bytes: &[u8]) -> Result<ID3D12Resource> {
    let r = buffer(
        device,
        bytes.len() as u64,
        D3D12_HEAP_TYPE_UPLOAD,
        D3D12_RESOURCE_FLAG_NONE,
        D3D12_RESOURCE_STATE_GENERIC_READ,
    )?;
    write_buffer(&r, 0, bytes)?;
    Ok(r)
}
unsafe fn write_buffer(r: &ID3D12Resource, offset: u64, bytes: &[u8]) -> Result<()> {
    let mut p = null_mut();
    r.Map(0, Some(&D3D12_RANGE::default()), Some(&mut p))?;
    std::ptr::copy_nonoverlapping(
        bytes.as_ptr(),
        (p as *mut u8).add(offset as usize),
        bytes.len(),
    );
    r.Unmap(0, None);
    Ok(())
}
unsafe fn transition(
    cmd: &ID3D12GraphicsCommandList4,
    r: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    let mut b = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(r.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
        ..Default::default()
    };
    cmd.ResourceBarrier(std::slice::from_ref(&b));
    ManuallyDrop::drop(&mut (*b.Anonymous.Transition).pResource);
}
unsafe fn uav(cmd: &ID3D12GraphicsCommandList4, r: Option<&ID3D12Resource>) {
    let mut b = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            UAV: ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                pResource: ManuallyDrop::new(r.cloned()),
            }),
        },
        ..Default::default()
    };
    cmd.ResourceBarrier(std::slice::from_ref(&b));
    ManuallyDrop::drop(&mut (*b.Anonymous.UAV).pResource);
}
unsafe fn copy_texture(
    cmd: &ID3D12GraphicsCommandList4,
    from: &ID3D12Resource,
    to: &ID3D12Resource,
) {
    transition(
        cmd,
        from,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    transition(
        cmd,
        to,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_COPY_DEST,
    );
    cmd.CopyResource(to, from);
    transition(
        cmd,
        from,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    );
    transition(
        cmd,
        to,
        D3D12_RESOURCE_STATE_COPY_DEST,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    );
}
unsafe fn location(r: &ID3D12Resource) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(r.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: 0,
        },
    }
}
unsafe fn footprint(
    device: &ID3D12Device5,
    r: &ID3D12Resource,
) -> (D3D12_PLACED_SUBRESOURCE_FOOTPRINT, u64) {
    let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
    let mut bytes = 0;
    device.GetCopyableFootprints(
        &r.GetDesc(),
        0,
        1,
        0,
        Some(&mut layout),
        None,
        None,
        Some(&mut bytes),
    );
    (layout, bytes)
}
fn halton(mut index: u32, base: u32) -> f32 {
    let (mut f, mut result) = (1., 0.);
    while index > 0 {
        f /= base as f32;
        result += f * (index % base) as f32;
        index /= base;
    }
    result
}

fn root_srv(register: u32) -> D3D12_ROOT_PARAMETER {
    D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            Descriptor: D3D12_ROOT_DESCRIPTOR {
                ShaderRegister: register,
                RegisterSpace: 0,
            },
        },
        ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_floats_round_trip() {
        for v in [0., 1., 0.1, 0.333_333, 1e-5, 6.1e-5, 1000.5, 65504., -2.5] {
            let back = half_to_f32(f32_to_half(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} → {back}");
        }
        assert_eq!(f32_to_half(1.), 0x3c00);
        assert_eq!(f32_to_half(1e6), 0x7c00, "past the range is infinity");
        assert_eq!(f32_to_half(65520.), 0x7c00, "rounds up past 65504");
    }
}
