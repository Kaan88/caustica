#![allow(unsafe_op_in_unsafe_fn)]
use crate::render::plan::{self, InstanceRecord, VisibleMesh, mesh_uvs};
pub use crate::render::*;
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

/// Shader-visible heap: UAVs 0..TEXTURES, then one SRV slot per egui texture from 32.
const HEAP_SIZE: usize = 512;
const FIRST_GUI_SLOT: usize = 32;
/// Mesh texture maps (core track 5): SRVs t0.. in space 1, in the heap after `HEAP_SIZE`.
const MAX_MAPS: usize = 4096;
struct GuiTexture {
    texture: Texture,
    slot: usize,
    /// CPU copy, so partial updates (the font atlas grows) can be patched in.
    pixels: Vec<u8>,
    size: [usize; 2],
}
const _: () = assert!(size_of::<Constants>() as u64 <= CB_STRIDE);
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
    /// Light path expressions (track 14): the trace variant that fills the AOVs, the DFA table
    /// (t8, an upload buffer, rewritten when the expressions change), the slots in use and the
    /// AOV buffer (u1 space 2: render pixels x slots float4s, allocated while any slot is set).
    trace_lpe: ID3D12PipelineState,
    lpe_table: ID3D12Resource,
    /// The expressions and (with a label in use) the material names the table was built from.
    lpe_uploaded: Option<(Vec<String>, Vec<String>)>,
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
    photon_gather_spectral: ID3D12PipelineState,
    bdpt_spectral: ID3D12PipelineState,
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
    /// Film-look LUT (u29, a 3D texture kept across resizes) and its edge length, 0 = none.
    lut: Texture,
    lut_size: u32,
    /// HDRI environment (u30, RGBA32F) and its sampling CDFs (u31, R32F, one more row than the
    /// map); 1×1 placeholders when none is loaded. `env_inv_norm` 0 = none (see environment.rs).
    env: Texture,
    env_cdf: Texture,
    env_inv_norm: f32,
    env_sun: Option<(crate::environment::Sun, f32)>,
    /// The map is stored as RGBA16F divided by this (≥ 1), so a sun brighter than half-float's
    /// 65504 keeps its value; strength and the pdf normalization are multiplied back.
    env_scale: f32,
    /// Frames presented, for the grain's per-frame noise.
    present_count: u32,
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
    /// Each mesh's range in `mesh_vertices` (first vertex, triangles) by mesh key, including
    /// meshes no longer shown; the vertices written and the buffer's size, in vertices
    /// (`upload_mesh_vertices`).
    vertex_ranges: std::collections::HashMap<String, (u32, usize)>,
    vertex_used: u64,
    vertex_capacity: u64,
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
    tlas: Option<ID3D12Resource>,
    /// The TLAS's scratch and instance buffers and its instances' fixed parts, for refits.
    tlas_cache: Option<TlasCache>,
    /// BLASes built on the open list awaiting compaction, with their compacted-size buffers.
    compact_queue: Vec<(ID3D12Resource, ID3D12Resource)>,
    /// egui textures (font atlas, scene thumbnails), each with its own heap slot (32 and up).
    gui_textures: std::collections::HashMap<egui::TextureId, GuiTexture>,
    free_slots: Vec<usize>,
    /// Mesh texture maps on the GPU, keyed by the `Arc<Map>` address (the Arc is kept, so the
    /// address stays unique), and the free map slots.
    maps: std::collections::HashMap<usize, MapTexture>,
    free_maps: Vec<u32>,
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
    rr_created: bool,
    rr_failed: bool,
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
    query: ID3D12QueryHeap,
    timing: ID3D12Resource,
    /// The backend-neutral state the app reads (`Deref`s to it).
    pub st: GpuState,
}
impl std::ops::Deref for Gpu {
    type Target = GpuState;
    fn deref(&self) -> &GpuState {
        &self.st
    }
}
impl std::ops::DerefMut for Gpu {
    fn deref_mut(&mut self) -> &mut GpuState {
        &mut self.st
    }
}
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
mod capture;
mod device;
mod frame;
mod init;
mod passes;
mod scene;
mod util;
use util::*;
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
/// A visible mesh object in `rebuild_scene`: its mesh key (vertex data), the key of the BLAS it
/// uses (the mesh key, or a micromap variant), and for that variant the base map and cutoff.
/// What a TLAS refit reuses (`rebuild_scene`): the scratch buffer (sized for a build or an
/// update), the instance descriptions' upload buffer, each instance's id, mask, flags and BLAS
/// address (bytes 48–64 of its description; a refit needs them unchanged), and the refits
/// since the last full build.
struct TlasCache {
    scratch: ID3D12Resource,
    instances: ID3D12Resource,
    tails: Vec<u8>,
    refits: u32,
}
/// An object's per-vertex UVs, or zeros when it has none or they don't match its `n` vertices.
/// A run of static triangles that share an instance mask and opacity (`instance_mask`).
struct StaticGroup {
    mask: u32,
    any_hit: bool,
    first: u32,
    count: u32,
    blas: Option<ID3D12Resource>,
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
