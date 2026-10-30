#![allow(unsafe_op_in_unsafe_fn)]
use crate::scene::{self, Settings, Upscaler, Vertex};
use anyhow::{Context, Result, ensure};
use glam::Mat4;
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
    light_position_size: [f32; 4],
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
}
/// Constants are written once per pass at this stride (trace at 0, filter passes 1-7, display 8).
const CB_STRIDE: u64 = 1024;
const _: () = assert!(size_of::<Constants>() as u64 <= CB_STRIDE);
unsafe extern "C" {
    fn cornell_ngx_release(ngx: *mut c_void);
    fn cornell_ngx_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn cornell_ngx_create(
        ngx: *mut c_void,
        cmd: *mut c_void,
        w: u32,
        h: u32,
        ow: u32,
        oh: u32,
        preset: u32,
    ) -> u32;
    fn cornell_ngx_eval(
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
    fn cornell_ngx_destroy(ngx: *mut c_void);
    fn cornell_ngx_caps(ngx: *mut c_void) -> u32;
    fn cornell_ngx_create_sr(
        ngx: *mut c_void,
        cmd: *mut c_void,
        w: u32,
        h: u32,
        ow: u32,
        oh: u32,
        preset: u32,
    ) -> u32;
    fn cornell_ngx_eval_sr(
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
    fn cornell_fsr_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn cornell_fsr_release(fsr: *mut c_void);
    fn cornell_fsr_destroy(fsr: *mut c_void);
    fn cornell_fsr_create(fsr: *mut c_void, w: u32, h: u32, ow: u32, oh: u32) -> u32;
    fn cornell_fsr_eval(
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
    fn cornell_xess_init(device: *mut c_void, result: *mut *mut c_void) -> u32;
    fn cornell_xess_destroy(xess: *mut c_void);
    fn cornell_xess_create(xess: *mut c_void, w: u32, ow: u32, oh: u32, velocity_scale: f32)
    -> u32;
    fn cornell_xess_eval(
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
    filter: ID3D12PipelineState,
    display: ID3D12PipelineState,
    gui: ID3D12PipelineState,
    gui_hdr: ID3D12PipelineState,
    /// The swapchain is RGBA16F scRGB (1.0 = 80 nits) and shows texture 26.
    pub hdr_active: bool,
    /// Paper white and panel peak in nits for the HDR transform (set by the app each frame).
    pub hdr_levels: [f32; 2],
    /// Triangles in the non-opaque BLAS range; the opaque range's primitive indices start here.
    pub opaque_offset: u32,
    /// Light painting (roadmap 5d): progressive path only, and scene edits keep the accumulation.
    pub painting: bool,
    /// Film-look LUT (u29, a 3D texture kept across resizes) and its edge length, 0 = none.
    lut: Texture,
    lut_size: u32,
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
    geometry: ID3D12Resource,
    material_buffer: ID3D12Resource,
    emitter_buffer: ID3D12Resource,
    emitter_lookup: ID3D12Resource,
    cached_vertices: Vec<Vertex>,
    pub triangle_count: usize,
    pub emitter_count: usize,
    blas: Option<ID3D12Resource>,
    tlas: Option<ID3D12Resource>,
    /// egui textures (font atlas, scene thumbnails), each with its own heap slot (32 and up).
    gui_textures: std::collections::HashMap<egui::TextureId, GuiTexture>,
    free_slots: Vec<usize>,
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
    pub upscale_status: String,
    pub rr_status: String,
    rr_created: bool,
    rr_failed: bool,
    pub rr_active: bool,
    pub width: u32,
    pub height: u32,
    pub rw: u32,
    pub rh: u32,
    pub samples: u32,
    pub frame: u32,
    pub gpu_ms: f64,
    pub adapter_name: String,
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
            NumDescriptors: HEAP_SIZE as u32,
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
                // The per-resize textures, then the LUT (u29).
                NumDescriptors: TEXTURES as u32 + 1,
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
        ];
        let sampler = D3D12_STATIC_SAMPLER_DESC {
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
        let rd = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: params.len() as _,
            pParameters: params.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
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
        let filter = compute(include_bytes!(concat!(env!("OUT_DIR"), "/filter.dxil")))?;
        let display = compute(include_bytes!(concat!(env!("OUT_DIR"), "/display.dxil")))?;
        let (vs, ps): (&[u8], &[u8]) = (
            include_bytes!(concat!(env!("OUT_DIR"), "/vs_main.dxil")),
            include_bytes!(concat!(env!("OUT_DIR"), "/ps_main.dxil")),
        );
        let gui = create_gui_pipeline(&device, &root, DXGI_FORMAT_R8G8B8A8_UNORM, vs, ps)?;
        let gui_hdr = create_gui_pipeline(&device, &root, DXGI_FORMAT_R16G16B16A16_FLOAT, vs, ps)?;
        let lut = texture_3d(&device, 2)?;
        let constants = buffer(
            &device,
            CB_STRIDE * 9,
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
                Count: 2,
                NodeMask: 0,
            },
            &mut query,
        )?;
        let query = query.unwrap();
        let timing = buffer(
            &device,
            16,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let mut ngx = null_mut();
        let code = cornell_ngx_init(device.as_raw(), &mut ngx);
        let rr_status = if ngx_ok(code) {
            "DLSS Ray Reconstruction available".into()
        } else {
            format!("NGX unavailable (0x{code:08x}); using native denoiser")
        };
        let mut fsr = null_mut();
        let fsr_code = cornell_fsr_init(device.as_raw(), &mut fsr);
        let mut xess = null_mut();
        let xess_code = cornell_xess_init(device.as_raw(), &mut xess);
        eprintln!(
            "GPU: {adapter_name}; {rr_status}; DLSS SR {}; FSR {}; XeSS {}",
            if !ngx.is_null() && cornell_ngx_caps(ngx) & 2 != 0 {
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
            filter,
            display,
            gui,
            gui_hdr,
            hdr_active: false,
            hdr_levels: [200., 1000.],
            opaque_offset: 0,
            painting: false,
            lut,
            lut_size: 0,
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
            cached_vertices: vec![],
            triangle_count: 0,
            emitter_count: 0,
            geometry,
            blas: None,
            tlas: None,
            gui_textures: Default::default(),
            free_slots: (FIRST_GUI_SLOT..HEAP_SIZE).rev().collect(),
            keep_ui_frame: false,
            ui_frame: None,
            ngx,
            rr_status,
            fsr,
            xess,
            sr_created: None,
            sr_failed: vec![],
            sr_reset: true,
            upscale_status: String::new(),
            rr_created: false,
            rr_failed: false,
            rr_active: false,
            width,
            height,
            rw: 0,
            rh: 0,
            samples: 0,
            frame: 0,
            gpu_ms: 0.,
            adapter_name,
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
            self.fence
                .SetEventOnCompletion(self.fence_value, self.event)?;
            ensure!(
                WaitForSingleObject(self.event, 30_000)
                    == windows::Win32::Foundation::WAIT_OBJECT_0,
                "GPU fence timeout; device removed: {:?}",
                self.device.GetDeviceRemovedReason()
            );
        }
        Ok(())
    }
    pub fn reset(&mut self, reset_history: bool) {
        self.samples = 0;
        // Each restart takes a fresh, 8192-aligned Sobol block, so a moving camera
        // never reuses the previous frame's sample points.
        self.restarts = self.restarts.wrapping_add(1);
        self.sequence_base = self.restarts.wrapping_mul(0x9E37_79B1) & !8191;
        if reset_history {
            self.rr_reset = true;
            self.history_valid = false;
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
        cornell_ngx_release(self.ngx);
        cornell_fsr_release(self.fsr);
        self.rr_created = false;
        self.rr_failed = false;
        self.sr_created = None;
        self.sr_failed.clear();
    }
    /// NGX loaded and reports Ray Reconstruction (caps bit 0).
    pub fn rr_available(&self) -> bool {
        !self.ngx.is_null() && unsafe { cornell_ngx_caps(self.ngx) } & 1 != 0
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        match kind {
            Upscaler::Dlss => !self.ngx.is_null() && unsafe { cornell_ngx_caps(self.ngx) } & 2 != 0,
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
                let code = cornell_ngx_create_sr(
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
                let code = cornell_fsr_create(self.fsr, w, h, ow, oh);
                (code == 0, code)
            }
            _ => {
                let code = cornell_xess_create(self.xess, w, ow, oh, 1.);
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
        if self.hdr_active {
            DXGI_FORMAT_R16G16B16A16_FLOAT
        } else {
            DXGI_FORMAT_R8G8B8A8_UNORM
        }
    }
    /// Switches the swapchain between 8-bit SDR and RGBA16F scRGB. Recreates the
    /// render-size textures like a resize. Returns false when the swapchain can't present scRGB.
    pub unsafe fn set_hdr(&mut self, on: bool, s: &Settings) -> Result<bool> {
        if on == self.hdr_active {
            return Ok(true);
        }
        // Color-space support depends on the buffer format, so switch first, then check.
        self.hdr_active = on;
        self.resize(self.width, self.height, s)?;
        if on && !self.color_space_ok {
            self.hdr_active = false;
            self.resize(self.width, self.height, s)?;
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
        let filter = create_compute_pipeline(&self.device, &self.root, &sh.filter)?;
        let display = create_compute_pipeline(&self.device, &self.root, &sh.display)?;
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
        self.wait_idle()?;
        (
            self.trace,
            self.filter,
            self.display,
            self.gui,
            self.gui_hdr,
        ) = (trace, filter, display, gui, gui_hdr);
        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    pub unsafe fn resize(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid viewport size"
        );
        // The previous submit has completed. Reset bindings before releasing swap-chain resources.
        self.wait_idle()?;
        self.begin()?;
        self.list.Close()?;
        self.release_upscalers();
        self.buffers.clear();
        let resized = self
            .swap
            .ResizeBuffers(2, w, h, self.swap_format(), DXGI_SWAP_CHAIN_FLAG(0))
            .and_then(|_| {
                let space = if self.hdr_active {
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
        self.width = w;
        self.height = h;
        self.rw = ((w as f32 * s.scale).round() as u32).max(1);
        self.rh = ((h as f32 * s.scale).round() as u32).max(1);
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
        self.textures.clear();
        self.ui_frame = None;
        for i in 0..TEXTURES {
            let format = match i {
                5 | 7 | 17 | 19 | 20 | 25 => DXGI_FORMAT_R32_FLOAT,
                6 => DXGI_FORMAT_R32G32_FLOAT,
                10 => DXGI_FORMAT_R8G8B8A8_UNORM,
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
        // Two BLAS ranges: triangles that need the any-hit loop first (lights hidden from the
        // camera, glass that shadow rays may pass), then everything else as opaque geometry,
        // which the hardware commits without running the Proceed() loop.
        let packed = crate::lighting::materials(s);
        let needs_any_hit = |v: &Vertex| {
            let m = &packed[v.material as usize];
            (m.lobes[3] as u32 & 2) != 0 || m.lobes[2] > 0.
        };
        let soup = scene::geometry(s);
        let (special, opaque): (Vec<&[Vertex; 3]>, Vec<&[Vertex; 3]>) = soup
            .as_chunks::<3>()
            .0
            .iter()
            .partition(|t| needs_any_hit(&t[0]));
        self.opaque_offset = special.len() as u32;
        let vertices: Vec<Vertex> = special
            .into_iter()
            .chain(opaque)
            .flatten()
            .copied()
            .collect();
        drop(soup);
        // Vertices live in GPU memory: hits read them, and big meshes would otherwise cross PCIe.
        let bytes: &[u8] = bytemuck::cast_slice(&vertices);
        let staging = upload(&self.device, bytes)?;
        self.geometry = buffer(
            &self.device,
            bytes.len() as u64,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_NONE,
            // Buffers start in COMMON; the copy promotes it to COPY_DEST.
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let stride = size_of::<Vertex>() as u64;
        let base = self.geometry.GetGPUVirtualAddress();
        let range = |first: usize, count: usize, flags| D3D12_RAYTRACING_GEOMETRY_DESC {
            Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
            Flags: flags,
            Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
                Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
                    VertexFormat: DXGI_FORMAT_R32G32B32_FLOAT,
                    VertexCount: (count * 3) as u32,
                    VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                        StartAddress: base + first as u64 * 3 * stride,
                        StrideInBytes: stride,
                    },
                    ..Default::default()
                },
            },
        };
        let (n0, total) = (self.opaque_offset as usize, vertices.len() / 3);
        let mut descs = vec![];
        if n0 > 0 {
            descs.push(range(0, n0, D3D12_RAYTRACING_GEOMETRY_FLAG_NONE));
        }
        if total > n0 {
            descs.push(range(n0, total - n0, D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE));
        }
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
            info.ScratchDataSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        // Explicit 64-byte DXR instance descriptor, including packed 24/8 bitfields.
        let mut instance = [0u8; 64];
        let transform: [f32; 12] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
        instance[..48].copy_from_slice(bytemuck::cast_slice(&transform));
        instance[48..52].copy_from_slice(&0xff000000u32.to_le_bytes());
        instance[52..56].copy_from_slice(
            &((D3D12_RAYTRACING_INSTANCE_FLAG_TRIANGLE_CULL_DISABLE.0 as u32) << 24).to_le_bytes(),
        );
        instance[56..64].copy_from_slice(&blas.GetGPUVirtualAddress().to_le_bytes());
        let instances = upload(&self.device, &instance)?;
        let ti = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
            NumDescs: 1,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                InstanceDescs: instances.GetGPUVirtualAddress(),
            },
        };
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
            info.ScratchDataSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.begin()?;
        self.list
            .CopyBufferRegion(&self.geometry, 0, &staging, 0, bytes.len() as u64);
        transition(
            &self.list,
            &self.geometry,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
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
        self.blas = Some(blas);
        self.tlas = Some(tlas);
        self.triangle_count = vertices.len() / 3;
        self.cached_vertices = vertices;
        self.update_materials(s)?;
        if !self.painting {
            self.reset(true);
        }
        Ok(())
    }
    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        let materials = crate::lighting::materials(s);
        let (emitters, lookup) = crate::lighting::emitter_table(&self.cached_vertices, &materials);
        self.emitter_count = emitters.len();
        self.material_buffer = upload(&self.device, bytemuck::cast_slice(&materials))?;
        self.emitter_buffer = upload(&self.device, bytemuck::cast_slice(&emitters))?;
        self.emitter_lookup = upload(&self.device, bytemuck::cast_slice(&lookup))?;
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
            .SetComputeRootShaderResourceView(2, self.geometry.GetGPUVirtualAddress());
        self.list.SetComputeRootDescriptorTable(3, self.gpu(0));
        self.list
            .SetComputeRootShaderResourceView(5, self.material_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(6, self.emitter_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(7, self.emitter_lookup.GetGPUVirtualAddress());
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
        self.rr_active =
            s.rr && !self.ngx.is_null() && !self.rr_failed && s.perspective() && !self.painting;
        if !s.rr && !self.ngx.is_null() {
            self.rr_status = "DLSS RR ready (disabled)".into();
        }
        if self.rr_active && self.rr_created {
            self.rr_status = "DLSS Ray Reconstruction active".into();
        }
        if self.rr_active && !self.rr_created {
            let code = cornell_ngx_create(
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
        let sr = self.prepare_upscaler(s);
        // With a temporal upscaler each frame is fresh jittered samples, as with RR: the native
        // denoiser's history and the upscaler do the accumulation, so detail is resolved above
        // the render resolution. (Feeding an accumulated image measured worse: the upscalers
        // assume every input frame sits at its own jitter.)
        if sr.is_some() && draw && self.samples > 0 {
            self.reset(false);
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
        let mut c = Constants {
            inverse_vp: vp.inverse().to_cols_array(),
            current_vp: vp.to_cols_array(),
            previous_vp: if reset { vp } else { self.previous_vp }.to_cols_array(),
            world_view: view.to_cols_array(),
            eye: [eye.x, eye.y, eye.z, 1.],
            dimensions: [self.rw, self.rh, self.width, self.height],
            sampling: [self.samples, n, s.bounces, self.frame],
            light_position_size: [
                s.light_position[0],
                s.light_position[1],
                s.light_position[2],
                s.light_size,
            ],
            light_color_aspect: [lc[0], lc[1], lc[2], s.light_aspect],
            background: [s.background[0], s.background[1], s.background[2], 0.],
            display: [s.exposure, s.gamma, s.direct, s.indirect],
            modes: [s.view, s.tonemap, s.denoise as u32, self.rr_active as u32],
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
            environment: [
                s.sky,
                1. / proj.x_axis.x,
                1. / proj.y_axis.y,
                self.opaque_offset as f32,
            ],
            hdr: [
                self.hdr_active as u32 as f32,
                self.hdr_levels[0],
                self.hdr_levels[1],
                0.,
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
        };
        // eye.w scales the UI to paper white in HDR (gui.hlsl); the trace ignores it.
        if self.hdr_active {
            c.eye[3] = self.hdr_levels[0] / 80.;
        }
        write_buffer(&self.constants, 0, bytemuck::bytes_of(&c))?;
        self.list
            .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, 0);
        if n > 0 {
            self.bind(0);
            self.list.SetPipelineState(&self.trace);
            self.list
                .Dispatch(self.rw.div_ceil(8), self.rh.div_ceil(8), 1);
            uav(&self.list, None);
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
                let code = cornell_ngx_eval(
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
        if !self.rr_active && s.denoise {
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
            self.run_upscaler(kind, s, &mut c, jitter, (self.sr_reset || reset) as i32, dt);
        }
        c.modes[3] = self.rr_active as u32;
        c.filter[0] = 0;
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
        self.list
            .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, 1);
        self.list.ResolveQueryData(
            &self.query,
            D3D12_QUERY_TYPE_TIMESTAMP,
            0,
            2,
            &self.timing,
            0,
        );
        let index = self.swap.GetCurrentBackBufferIndex() as usize;
        let back = &self.buffers[index];
        let shown = if self.hdr_active { 26 } else { 10 };
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
        let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.rtv.GetCPUDescriptorHandleForHeapStart().ptr
                + index
                    * self
                        .device
                        .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                        as usize,
        };
        self.list.OMSetRenderTargets(1, Some(&handle), false, None);
        self.draw_gui(primitives, pixels_per_point, &mut keep)?;
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
                        self.width,
                        self.height,
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
        self.present_count = self.present_count.wrapping_add(1);
        let mut data = null_mut();
        self.timing
            .Map(0, Some(&D3D12_RANGE { Begin: 0, End: 16 }), Some(&mut data))?;
        let times = std::slice::from_raw_parts(data as *const u64, 2);
        let frequency = self.queue.GetTimestampFrequency()?;
        self.gpu_ms = (times[1] - times[0]) as f64 / frequency as f64 * 1000.;
        self.timing.Unmap(0, None);
        self.samples += n;
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
                let code = cornell_ngx_eval_sr(
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
            Upscaler::Fsr => cornell_fsr_eval(
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
            _ => cornell_xess_eval(
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
        self.list.SetPipelineState(if self.hdr_active {
            &self.gui_hdr
        } else {
            &self.gui
        });
        self.list
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.list.RSSetViewports(&[D3D12_VIEWPORT {
            Width: self.width as f32,
            Height: self.height as f32,
            MaxDepth: 1.,
            ..Default::default()
        }]);
        // One vertex and one index upload per frame; each mesh draws from its offsets.
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
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
                    position: [v.pos.x * ppp, v.pos.y * ppp],
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
            let left = (r.min.x * ppp).clamp(0., self.width as f32) as i32;
            let top = (r.min.y * ppp).clamp(0., self.height as f32) as i32;
            let right = (r.max.x * ppp).clamp(left as f32, self.width as f32) as i32;
            let bottom = (r.max.y * ppp).clamp(top as f32, self.height as f32) as i32;
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
        // A UI capture in HDR mode copies the RGBA16F scRGB backbuffer.
        let ui_hdr = include_ui && self.hdr_active;
        let row_size = self.width as usize
            * if hdr {
                16
            } else if ui_hdr {
                8
            } else {
                4
            };
        let mut packed = Vec::with_capacity(row_size * self.height as usize);
        for y in 0..self.height as usize {
            packed.extend_from_slice(std::slice::from_raw_parts(
                (p as *const u8).add(y * layout.Footprint.RowPitch as usize),
                row_size,
            ));
        }
        staging.Unmap(0, None);
        if ui_hdr {
            // Back to SDR: paper white becomes 1.0, then sRGB-encode.
            let scale = 80. / self.hdr_levels[0];
            packed = packed
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let v = half_to_f32(u16::from_le_bytes(*b));
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
            image::Rgb32FImage::from_raw(self.width, self.height, rgb)
                .unwrap()
                .save(path)?;
        } else {
            image::save_buffer(
                path,
                &packed,
                self.width,
                self.height,
                image::ColorType::Rgba8,
            )?;
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
                    if msg.Severity.0 <= D3D12_MESSAGE_SEVERITY_WARNING.0 {
                        result.push(
                            String::from_utf8_lossy(std::slice::from_raw_parts(
                                msg.pDescription,
                                msg.DescriptionByteLength.saturating_sub(1),
                            ))
                            .into(),
                        );
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
            cornell_ngx_destroy(self.ngx);
            cornell_fsr_destroy(self.fsr);
            cornell_xess_destroy(self.xess);
            let _ = CloseHandle(self.event);
        }
    }
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
        ..Default::default()
    };
    let p = device.CreateComputePipelineState(&d);
    ManuallyDrop::drop(&mut d.pRootSignature);
    Ok(p?)
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
        state,
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
