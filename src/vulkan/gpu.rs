//! The app-facing Vulkan renderer: the same API as the D3D12 `gpu::Gpu`, mirroring its passes
//! (engine-notes "Vulkan backend: design"). `passes.rs` holds the per-frame passes and the
//! scene resources they need; the upscalers are D3D12-only for now.
use super::gui::Gui;
use super::layout::{Layout, SET0_BUFFERS};
use super::res::{Accel, Buffer, COLOR, Image, full_barrier, triangles};
use std::collections::HashMap;

mod passes;
pub(super) mod upscale;
use super::{Device, Frame, Swapchain};
use crate::render::plan;
pub use crate::render::*;
use crate::scene::{Settings, Upscaler, Vertex};
use anyhow::{Context, Result, bail, ensure};
use ash::{khr, vk};
use passes::copy_image;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

macro_rules! spv {
    ($name:literal) => {{
        #[cfg(spirv)]
        let code: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".spv"));
        #[cfg(not(spirv))]
        let code: &[u8] = &[];
        code
    }};
}

/// A texture map's image, its `maps[]` slot and the map (kept, so its address stays unique).
/// Bytes per texel of a swap chain format (RGBA16F scRGB is 8, the others 4).
fn texel_bytes(format: vk::Format) -> u64 {
    if format == vk::Format::R16G16B16A16_SFLOAT {
        8
    } else {
        4
    }
}

type MapImage = (Image, u32, std::sync::Arc<crate::mesh::Map>);

/// What `rebuild_scene` keeps between rebuilds, as D3D12: the static groups (rebuilt when
/// their triangles change), one compacted BLAS per distinct mesh (kept by key; the shared
/// vertex buffer is rewritten when the set changes), the procedural objects' unit-cube BLAS
/// (built once) and the TLAS (refit when only the transforms change).
#[derive(Default)]
struct Scene {
    statics: Option<Buffer>,
    groups: Vec<(plan::GroupKey, Accel)>,
    meshes: Option<Buffer>,
    /// BLAS, first vertex and triangle count by `blas_key`.
    mesh_blas: HashMap<String, (Accel, u32, usize)>,
    sdf: Option<(Accel, Buffer)>,
    tlas: Option<super::res::Tlas>,
}

pub struct Gpu {
    pub st: GpuState,
    swap: Swapchain,
    frame: Frame,
    accel_fns: khr::acceleration_structure::Device,
    layout: Layout,
    pipes: HashMap<&'static str, vk::Pipeline>,
    /// Space 2: u0 path records (host-visible: read after the fence), u1 LPE AOVs, u2 ReSTIR
    /// reservoirs, u3 photons and splats, u4 the glare grid; the path buffer stands in for the
    /// optional ones. t8 the LPE table and t9 the lens system (host-visible).
    path_buffer: Buffer,
    lpe_aov: Option<Buffer>,
    lpe_aov_len: u64,
    reservoirs: Option<Buffer>,
    reservoirs_len: u64,
    aux: Option<Buffer>,
    aux_len: u64,
    splat_offset: u64,
    glare_buffer: Buffer,
    lpe_table: Buffer,
    lens_buffer: Buffer,
    lpe_uploaded: Option<(Vec<String>, Vec<String>)>,
    lens_params: Option<crate::lens::Params>,
    lens_surfaces: u32,
    glare_kernel: Option<[f32; 3]>,
    restir_layers: u32,
    restir_history: bool,
    restir_ran: bool,
    battle_ran: bool,
    bounds: [f32; 4],
    bounds_valid: bool,
    photon_scale: f32,
    photon_frames: u32,
    sdf_emitters: [u32; 2],
    textured: bool,
    maps: HashMap<usize, MapImage>,
    free_maps: Vec<u32>,
    lut_size: u32,
    env_scale: f32,
    env_inv_norm: f32,
    env_sun: Option<(crate::environment::Sun, f32)>,
    restarts: u32,
    sequence_base: u32,
    present_count: u32,
    gui: Gui,
    /// Timestamps 0-4 (trace, reconstruction, display, UI), read after the next fence wait.
    queries: vk::QueryPool,
    tick_ms: f64,
    /// Scopes counters at 0, then the probe's texels (`PROBE` offsets); host-visible, filled by
    /// the frame's copies and read after the next fence wait.
    readback: Buffer,
    pending_probe: Option<[u32; 2]>,
    pending_scopes: bool,
    /// The last presented frame with its UI (`keep_ui_frame`), as tightly packed swap texels.
    ui_frame: Option<(Buffer, [u32; 2])>,
    textures: Vec<Image>,
    /// u29 LUT, u30 environment map, u31 its CDFs.
    fixed: Vec<Image>,
    constants: Buffer,
    /// Stands in for every buffer a pass doesn't read this frame (D3D12 binds the path buffer).
    placeholder: Buffer,
    scene: Scene,
    /// t3 materials, t4 emitters, t5 emitter lookup, t7 instance table.
    tables: Vec<Buffer>,
    /// t10-t17: the shaders' constant tables (layers.hlsli, spectral_tables.hlsli).
    shader_tables: Vec<Buffer>,
    static_cache: Vec<Vertex>,
    instances: Vec<plan::InstanceRecord>,
    lean_scene: bool,
    previous_vp: glam::Mat4,
    reset_pending: bool,
    /// The native denoiser's state, as on D3D12 (`Gpu::history_valid` and so on).
    history_valid: bool,
    integrated_valid: bool,
    camera_moved: bool,
    up: upscale::Upscalers,
    // Dropped last (see Drop).
    device: Device,
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

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device.device_wait_idle();
            self.up.destroy();
            self.free_scene();
            let d = &self.device;
            for b in self.tables.drain(..).chain(self.shader_tables.drain(..)) {
                d.free_buffer(b);
            }
            for i in self.textures.drain(..).chain(self.fixed.drain(..)) {
                d.free_image(i);
            }
            // Not used again: Drop ends here.
            d.free_buffer(std::ptr::read(&self.constants));
            d.free_buffer(std::ptr::read(&self.placeholder));
            for (_, p) in self.pipes.drain() {
                d.device.destroy_pipeline(p, None);
            }
            for (_, (image, _, _)) in self.maps.drain() {
                d.free_image(image);
            }
            for b in [self.lpe_aov.take(), self.reservoirs.take(), self.aux.take()]
                .into_iter()
                .flatten()
            {
                d.free_buffer(b);
            }
            for b in [
                &self.path_buffer,
                &self.glare_buffer,
                &self.lpe_table,
                &self.lens_buffer,
            ] {
                d.free_buffer(std::ptr::read(b));
            }
            self.layout.destroy(d);
            self.gui.destroy(d);
            d.device.destroy_query_pool(self.queries, None);
            d.free_buffer(std::ptr::read(&self.readback));
            if let Some((b, _)) = self.ui_frame.take() {
                d.free_buffer(b);
            }
            self.frame.destroy(d);
            self.swap.destroy(d);
        }
    }
}

/// Where the probe's four texels start in `Gpu::readback`, after the scopes counters.
const PROBE: u64 = (SCOPES[0] * SCOPES[1] * 4) as u64;

fn vk_format(f: TexFormat) -> vk::Format {
    match f {
        TexFormat::Rgba32F => vk::Format::R32G32B32A32_SFLOAT,
        TexFormat::R32F => vk::Format::R32_SFLOAT,
        TexFormat::Rg32F => vk::Format::R32G32_SFLOAT,
        TexFormat::Rgba8 => vk::Format::R8G8B8A8_UNORM,
        TexFormat::Rgb10A2 => vk::Format::A2B10G10R10_UNORM_PACK32,
        TexFormat::Rgba16F => vk::Format::R16G16B16A16_SFLOAT,
        TexFormat::R32U => vk::Format::R32_UINT,
    }
}

/// The swap chain while `Gpu::new` runs: destroyed with its surface if `new` returns early.
struct SwapGuard<'a>(&'a Device, Option<Swapchain>);

impl SwapGuard<'_> {
    fn into_inner(mut self) -> Swapchain {
        self.1.take().unwrap()
    }
}

impl std::ops::Deref for SwapGuard<'_> {
    type Target = Swapchain;
    fn deref(&self) -> &Swapchain {
        self.1.as_ref().unwrap()
    }
}

impl Drop for SwapGuard<'_> {
    fn drop(&mut self) {
        if let Some(mut swap) = self.1.take() {
            unsafe {
                let _ = self.0.device.device_wait_idle();
                swap.destroy(self.0);
            }
        }
    }
}

impl Gpu {
    pub unsafe fn new(
        window: &winit::window::Window,
        width: u32,
        height: u32,
        s: &Settings,
        debug: bool,
    ) -> Result<Self> {
        unsafe {
            ensure!(
                !spv!("trace").is_empty(),
                "this build has no SPIR-V shaders (no SPIR-V DXC at build time)"
            );
            let raw = (
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
            );
            let (device, surface) = Device::new(Some(raw), debug)?;
            let surface = surface.context("no surface")?;
            let swap = match Swapchain::new(&device, surface, [width, height], true) {
                Ok(swap) => swap,
                Err(e) => {
                    device.surface_fn.destroy_surface(surface, None);
                    return Err(e);
                }
            };
            // Until `Gpu` owns it, a failure below destroys the swap chain and the surface:
            // Menu → Renderer then falls back to D3D12 on the same window.
            let swap = SwapGuard(&device, Some(swap));
            let frame = Frame::new(&device)?;
            let accel_fns =
                khr::acceleration_structure::Device::new(&device.instance, &device.device);
            let layout = Layout::new(&device)?;
            // Compiled in parallel: NVIDIA's SPIR-V compiler takes seconds per tracer when its
            // cache is cold (a new driver or new shaders), about a minute in a row.
            let list = [
                ("trace", spv!("trace"), c"trace"),
                ("trace_maps", spv!("trace_maps"), c"trace"),
                ("trace_lean", spv!("trace_lean"), c"trace"),
                ("trace_maps_lean", spv!("trace_maps_lean"), c"trace"),
                ("trace_spectral", spv!("trace_spectral"), c"trace"),
                ("trace_spectral_lean", spv!("trace_spectral_lean"), c"trace"),
                ("trace_paths", spv!("trace_paths"), c"trace"),
                ("trace_lpe", spv!("trace_lpe"), c"trace"),
                (
                    "restir_presample",
                    spv!("restir_presample"),
                    c"restir_presample",
                ),
                ("restir_initial", spv!("restir_initial"), c"restir_initial"),
                ("restir_spatial", spv!("restir_spatial"), c"restir_spatial"),
                ("bdpt", spv!("bdpt"), c"bdpt"),
                ("bdpt_spectral", spv!("bdpt_spectral"), c"bdpt"),
                ("bdpt_resolve", spv!("bdpt_resolve"), c"bdpt_resolve"),
                ("photon_clear", spv!("photon_clear"), c"photon_clear"),
                ("photon_trace", spv!("photon_trace"), c"photon_trace"),
                (
                    "photon_trace_spectral",
                    spv!("photon_trace_spectral"),
                    c"photon_trace",
                ),
                ("photon_gather", spv!("photon_gather"), c"photon_gather"),
                (
                    "photon_gather_spectral",
                    spv!("photon_gather_spectral"),
                    c"photon_gather",
                ),
                ("filter", spv!("filter"), c"filter"),
                ("display", spv!("display"), c"display"),
                ("glare", spv!("glare"), c"glare_pass"),
            ];
            // NGX takes over a second to start, so it starts beside the pipelines. NGX and XeSS
            // get the device only, not our queue, so the progress frames' submits don't race them.
            struct Threaded(upscale::Upscalers);
            unsafe impl Send for Threaded {}
            let device_ref = &device;
            let done = std::sync::atomic::AtomicUsize::new(0);
            let (done_ref, swap_ref, frame_ref) = (&done, &swap, &frame);
            let (built, up): (Vec<(&str, Result<vk::Pipeline>)>, _) = std::thread::scope(|scope| {
                let device = device_ref;
                let up = scope.spawn(move || Threaded(upscale::Upscalers::new(device)));
                let jobs: Vec<_> = list
                    .iter()
                    .map(|&(name, code, entry)| {
                        let layout = &layout;
                        (
                            name,
                            scope.spawn(move || {
                                let pipe = layout.compute(
                                    device,
                                    code,
                                    entry,
                                    vk::PipelineCreateFlags::empty(),
                                );
                                done_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                pipe
                            }),
                        )
                    })
                    .collect();
                // A cold driver cache takes seconds: "Compiling shaders N/22" (render::progress).
                let (total, title) = (list.len(), window.title());
                let mut broken = false;
                let shown = crate::render::progress::watch(done_ref, total, |n| {
                    window.set_title(&format!("{title} · Compiling shaders {n}/{total}"));
                    if !broken
                        && let Err(e) = super::progress::draw(device, swap_ref, frame_ref, n, total)
                    {
                        // Only the title shows progress from here.
                        eprintln!("vulkan: progress frame: {e:#}");
                        broken = true;
                    }
                });
                if shown {
                    window.set_title(&title);
                }
                let built = jobs
                    .into_iter()
                    .map(|(name, job)| (name, job.join().expect("pipeline thread")))
                    .collect();
                (built, up.join().expect("upscaler thread").0)
            });
            let mut pipes = HashMap::new();
            for (name, pipe) in built {
                pipes.insert(name, pipe.with_context(|| format!("{name} pipeline"))?);
            }
            let constants = device.buffer(
                CB_STRIDE * CB_SLOTS,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                true,
            )?;
            layout.constants(&device, constants.handle, CB_STRIDE);
            let queries = device.device.create_query_pool(
                &vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::TIMESTAMP)
                    .query_count(TIMESTAMPS),
                None,
            )?;
            let tick_ms = device
                .instance
                .get_physical_device_properties(device.phys)
                .limits
                .timestamp_period as f64
                * 1e-6;
            let readback = device.buffer(PROBE + 64, vk::BufferUsageFlags::TRANSFER_DST, true)?;
            let gui = Gui::new(
                &device,
                swap.format,
                (constants.handle, CB_STRIDE),
                spv!("vs_main"),
                spv!("ps_main"),
            )?;
            let placeholder =
                device.buffer(1 << 16, vk::BufferUsageFlags::STORAGE_BUFFER, false)?;
            for b in SET0_BUFFERS {
                layout.buffer(&device, 0, b, placeholder.handle);
            }
            let host = |len: u64| device.buffer(len, vk::BufferUsageFlags::STORAGE_BUFFER, true);
            let path_buffer = host((PATH_CAPACITY * size_of::<crate::paths::Record>()) as u64)?;
            let lpe_table = host(
                ((crate::lpe::SLOTS * crate::lpe::STATES * crate::lpe::SYMBOLS
                    + crate::lpe::MATERIALS)
                    * 4) as u64,
            )?;
            let lens_buffer = host((crate::lens::FLOAT4S * 16) as u64)?;
            let glare_buffer = device.buffer(
                2 * 3 * GLARE_N * GLARE_N * 8,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                false,
            )?;
            for b in 0..4 {
                layout.buffer(&device, 2, b, path_buffer.handle);
            }
            layout.buffer(&device, 2, 4, glare_buffer.handle);
            layout.buffer(&device, 0, 108, lpe_table.handle);
            // The shaders' constant tables, read from buffers on Vulkan (layers.hlsli).
            let layers = include_str!("../../shaders/layers.hlsli");
            let spectral = include_str!("../../shaders/spectral_tables.hlsli");
            let mut shader_tables = vec![];
            for (binding, src, name) in [
                (110, layers, "SHEEN_ALBEDO"),
                (111, layers, "GGX_ALBEDO"),
                (112, layers, "COAT_ALBEDO"),
                (113, layers, "ANISO_ALBEDO"),
                (114, spectral, "SPECTRAL_BASIS"),
                (115, spectral, "SPECTRAL_OUT"),
                (116, spectral, "SPECTRAL_MEASURED"),
                (117, spectral, "SPECTRAL_D65"),
            ] {
                let data = crate::render::hlsl_table(src, name);
                let b = device.upload(
                    bytemuck::cast_slice(&data),
                    vk::BufferUsageFlags::STORAGE_BUFFER,
                )?;
                layout.buffer(&device, 0, binding, b.handle);
                shader_tables.push(b);
            }
            layout.buffer(&device, 0, 109, lens_buffer.handle);
            let storage = vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED;
            let fixed = vec![
                device.image([2, 2, 2], vk::Format::R32G32B32A32_SFLOAT, storage)?,
                device.image([1, 1, 1], vk::Format::R16G16B16A16_SFLOAT, storage)?,
                device.image([1, 2, 1], vk::Format::R32_SFLOAT, storage)?,
            ];
            for (i, img) in fixed.iter().enumerate() {
                layout.image(&device, 29 + i as u32, img.view);
            }
            let st = GpuState {
                adapter_name: device.name.clone(),
                denoiser_activity: "",
                hdr_levels: [203., 1000.],
                ..Default::default()
            };
            let swap = swap.into_inner();
            let mut gpu = Self {
                st,
                swap,
                frame,
                accel_fns,
                layout,
                pipes,
                path_buffer,
                lpe_aov: None,
                lpe_aov_len: 0,
                reservoirs: None,
                reservoirs_len: 0,
                aux: None,
                aux_len: 0,
                splat_offset: 0,
                glare_buffer,
                lpe_table,
                lens_buffer,
                lpe_uploaded: None,
                lens_params: None,
                lens_surfaces: 0,
                glare_kernel: None,
                restir_layers: 1,
                restir_history: false,
                restir_ran: false,
                battle_ran: false,
                bounds: [0., 0., 0., 2.],
                bounds_valid: false,
                photon_scale: 1.,
                photon_frames: 0,
                sdf_emitters: [0; 2],
                textured: false,
                maps: HashMap::new(),
                free_maps: (0..super::layout::MAX_MAPS).rev().collect(),
                lut_size: 0,
                env_scale: 1.,
                env_inv_norm: 0.,
                env_sun: None,
                restarts: 0,
                sequence_base: 0,
                present_count: 0,
                gui,
                shader_tables,
                queries,
                tick_ms,
                readback,
                pending_probe: None,
                pending_scopes: false,
                ui_frame: None,
                textures: vec![],
                fixed,
                constants,
                placeholder,
                scene: Scene::default(),
                tables: vec![],
                static_cache: vec![],
                instances: vec![],
                lean_scene: false,
                previous_vp: glam::Mat4::IDENTITY,
                reset_pending: true,
                history_valid: false,
                integrated_valid: false,
                camera_moved: false,
                up,
                device,
            };
            gpu.st.swap_size = [width, height];
            gpu.resize_output(width, height, s)?;
            gpu.rebuild_scene(s)?;
            Ok(gpu)
        }
    }

    unsafe fn free_scene(&mut self) {
        let sc = std::mem::take(&mut self.scene);
        let (d, fns) = (&self.device, &self.accel_fns);
        unsafe {
            for (_, a) in sc.groups {
                d.free_accel(fns, a);
            }
            for (_, (a, _, _)) in sc.mesh_blas {
                d.free_accel(fns, a);
            }
            if let Some((a, b)) = sc.sdf {
                d.free_accel(fns, a);
                d.free_buffer(b);
            }
            if let Some(t) = sc.tlas {
                d.free_tlas(fns, t);
            }
            for b in [sc.statics, sc.meshes].into_iter().flatten() {
                d.free_buffer(b);
            }
        }
    }

    pub unsafe fn rebuild_scene(&mut self, s: &Settings) -> Result<()> {
        unsafe {
            self.device.device.device_wait_idle()?;
            self.bounds_valid = false;
            let (d, fns, sc) = (&self.device, &self.accel_fns, &mut self.scene);
            let packed = crate::lighting::materials(s);
            let (groups, statics) = plan::static_layout(s, &packed);
            let input = vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
            let stride = size_of::<Vertex>() as u64;
            let blas_ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
            // Static geometry in world space, rebuilt only when it changes.
            let same_layout = groups.len() == sc.groups.len()
                && groups.iter().zip(&sc.groups).all(|(a, (b, _))| {
                    (a.mask, a.any_hit, a.first, a.count) == (b.mask, b.any_hit, b.first, b.count)
                });
            if sc.statics.is_none()
                || !same_layout
                || bytemuck::cast_slice::<Vertex, u8>(&statics)
                    != bytemuck::cast_slice::<Vertex, u8>(&self.static_cache)
            {
                for (_, a) in sc.groups.drain(..) {
                    d.free_accel(fns, a);
                }
                if let Some(b) = sc.statics.take() {
                    d.free_buffer(b);
                }
                // An empty room still needs a vertex buffer to bind.
                let bytes: &[u8] = if statics.is_empty() {
                    &[0; size_of::<Vertex>()]
                } else {
                    bytemuck::cast_slice(&statics)
                };
                let buffer = d.upload(bytes, input)?;
                let mut accels = vec![];
                for g in &groups {
                    let geometry = triangles(
                        buffer.address + g.first as u64 * 3 * stride,
                        g.count,
                        !g.any_hit,
                    );
                    accels.push(d.build_accel(fns, blas_ty, &[geometry], &[g.count], true)?);
                }
                let accels = d.compact_accels(fns, accels)?;
                sc.groups = groups.into_iter().zip(accels).collect();
                sc.statics = Some(buffer);
                self.static_cache = statics;
            }
            // One BLAS per distinct mesh in object space, rebuilt only when the set of meshes
            // changes; a BLAS holds its own copy of the triangles, so ones already built are reused.
            let visible = plan::visible_meshes(s, &packed, false);
            let mut wanted: Vec<&str> = visible.iter().map(|v| v.blas_key.as_str()).collect();
            wanted.sort_unstable();
            wanted.dedup();
            let mut have: Vec<&str> = sc.mesh_blas.keys().map(String::as_str).collect();
            have.sort_unstable();
            if wanted != have || sc.meshes.is_none() {
                // Every distinct mesh once, in the order first seen.
                let mut ranges = HashMap::new();
                let mut order = vec![];
                let mut vertices: Vec<Vertex> = vec![];
                for v in &visible {
                    if ranges.contains_key(&v.blas_key) {
                        continue;
                    }
                    ranges.insert(
                        v.blas_key.clone(),
                        (vertices.len() as u32, v.source.len() / 3),
                    );
                    order.push(v.blas_key.clone());
                    let uv = plan::mesh_uvs(&s.objects[v.index], v.source.len());
                    vertices.extend(v.source.iter().zip(uv).map(|((p, n), uv)| Vertex {
                        position: p.to_array(),
                        material: 0,
                        normal: n.to_array(),
                        pad: 0,
                        uv,
                    }));
                }
                if let Some(b) = sc.meshes.take() {
                    d.free_buffer(b);
                }
                let bytes: &[u8] = if vertices.is_empty() {
                    &[0; size_of::<Vertex>()]
                } else {
                    bytemuck::cast_slice(&vertices)
                };
                let meshes = d.upload(bytes, input)?;
                let mut old = std::mem::take(&mut sc.mesh_blas);
                let mut fresh = vec![];
                for key in order {
                    let (first, count) = ranges[&key];
                    if let Some((a, _, _)) = old.remove(&key) {
                        sc.mesh_blas.insert(key, (a, first, count));
                        continue;
                    }
                    let geometry =
                        triangles(meshes.address + first as u64 * stride, count as u32, false);
                    // A simulating cloth gets a new mesh each step: no compaction submit.
                    let compact = !key.contains("cloth:");
                    let a = d.build_accel(fns, blas_ty, &[geometry], &[count as u32], compact)?;
                    if compact {
                        fresh.push((key, a, first, count));
                    } else {
                        sc.mesh_blas.insert(key, (a, first, count));
                    }
                }
                for (_, (a, _, _)) in old {
                    d.free_accel(fns, a);
                }
                let (rest, accels): (Vec<_>, Vec<_>) = fresh
                    .into_iter()
                    .map(|(key, a, first, count)| ((key, first, count), a))
                    .unzip();
                for ((key, first, count), a) in rest.into_iter().zip(d.compact_accels(fns, accels)?)
                {
                    sc.mesh_blas.insert(key, (a, first, count));
                }
                sc.meshes = Some(meshes);
            }
            // The procedural objects' unit-cube AABB BLAS, built once.
            if sc.sdf.is_none() && s.objects.iter().any(|o| o.visible && !o.sdf.is_none()) {
                let aabb = d.upload(bytemuck::cast_slice(&[-1f32, -1., -1., 1., 1., 1.]), input)?;
                let geometry = vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::AABBS)
                    .geometry(vk::AccelerationStructureGeometryDataKHR {
                        aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default()
                            .data(vk::DeviceOrHostAddressConstKHR {
                                device_address: aabb.address,
                            })
                            .stride(24),
                    });
                let a = d.build_accel(fns, blas_ty, &[geometry], &[1], false)?;
                sc.sdf = Some((a, aabb));
            }

            let keys: Vec<plan::GroupKey> = sc
                .groups
                .iter()
                .map(|(g, _)| plan::GroupKey {
                    mask: g.mask,
                    any_hit: g.any_hit,
                    first: g.first,
                    count: g.count,
                })
                .collect();
            let (instances, table, triangle_count) =
                plan::instances(s, &packed, &keys, &visible, &|k| {
                    let m = &sc.mesh_blas[k];
                    (m.1, m.2)
                });
            let descs: Vec<vk::AccelerationStructureInstanceKHR> = instances
                .iter()
                .enumerate()
                .map(|(id, inst)| {
                    let blas = match inst.blas {
                        plan::Blas::Static(i) => &sc.groups[i].1,
                        plan::Blas::Mesh(k) => &sc.mesh_blas[k].0,
                        plan::Blas::Sdf => &sc.sdf.as_ref().expect("procedural BLAS").0,
                    };
                    let opacity = match inst.opacity {
                        plan::Opacity::ForceOpaque => vk::GeometryInstanceFlagsKHR::FORCE_OPAQUE,
                        plan::Opacity::ForceNonOpaque => {
                            vk::GeometryInstanceFlagsKHR::FORCE_NO_OPAQUE
                        }
                        plan::Opacity::Geometry => vk::GeometryInstanceFlagsKHR::empty(),
                    };
                    let flags =
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE | opacity;
                    vk::AccelerationStructureInstanceKHR {
                        transform: vk::TransformMatrixKHR {
                            matrix: inst.transform,
                        },
                        instance_custom_index_and_mask: vk::Packed24_8::new(
                            id as u32,
                            inst.mask as u8,
                        ),
                        instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                            0,
                            flags.as_raw() as u8,
                        ),
                        acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                            device_handle: blas.address,
                        },
                    }
                })
                .collect();
            let desc_bytes = std::slice::from_raw_parts(
                descs.as_ptr() as *const u8,
                descs.len() * size_of::<vk::AccelerationStructureInstanceKHR>(),
            );
            let tlas = d.build_tlas(fns, sc.tlas.take(), desc_bytes)?;
            self.layout
                .accel(d, tlas.accel.as_ref().expect("TLAS").handle);
            sc.tlas = Some(tlas);
            self.layout
                .buffer(d, 0, 101, sc.meshes.as_ref().unwrap().handle);
            self.layout
                .buffer(d, 0, 106, sc.statics.as_ref().unwrap().handle);
            self.instances = table;
            self.st.triangle_count = triangle_count;
            self.update_materials(s)
        }
    }

    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        unsafe {
            self.device.device.device_wait_idle()?;
            let mut materials = crate::lighting::materials(s);
            // Maps first: an emissive map sets the emitted colour the table is built from.
            self.attach_maps(s, &mut materials)?;
            let d = &self.device;
            let lights = plan::lights(s, &materials, &self.static_cache, &mut self.instances);
            self.st.sun_count = lights.sun_count;
            self.st.emitter_count = lights.emitter_count;
            self.lean_scene = lights.lean_scene;
            self.textured = lights.textured;
            self.sdf_emitters = lights.sdf_emitters;
            self.restir_history = false;
            for b in self.tables.drain(..) {
                d.free_buffer(b);
            }
            let usage = vk::BufferUsageFlags::STORAGE_BUFFER;
            self.tables = vec![
                d.upload(bytemuck::cast_slice(&materials), usage)?,
                d.upload(bytemuck::cast_slice(&lights.table), usage)?,
                d.upload(bytemuck::cast_slice(&lights.lookup), usage)?,
                d.upload(bytemuck::cast_slice(&self.instances), usage)?,
            ];
            for (b, binding) in self.tables.iter().zip([103, 104, 105, 107]) {
                self.layout.buffer(d, 0, binding, b.handle);
            }
            if !self.st.painting {
                self.reset(true);
            }
            Ok(())
        }
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
        unsafe {
            let d = &self.device;
            d.device
                .wait_for_fences(&[self.frame.fence], true, u64::MAX)?;
            self.gui.update(d, delta)?;
            self.decide_rr(s);
            let (view, proj, eye) = s.camera(self.st.width as f32 / self.st.height as f32);
            let vp = proj * view;
            let reset = self.reset_pending;
            // Every descriptor change before recording (a set can't change once bound).
            self.prepare_lpe(s)?;
            self.prepare_lens(s)?;
            let restir = self.prepare_restir(s)?;
            self.prepare_aux(s)?;
            let sr = self.decide_upscaler(s);
            let (jitter, accumulating) = self.upscale_jitter(sr, draw);
            let n = if draw {
                s.spp.min(s.max_samples.saturating_sub(self.st.samples))
            } else {
                0
            };
            let spatial = if s.upscaler == Upscaler::Bilinear {
                1
            } else {
                2
            };
            let jittered = self.st.rr_active || sr.is_some();
            let flags = jittered as u32
                | (jittered as u32) << 1
                | spatial << 2
                | s.mis_mode.min(2) << 4
                | ((s.fog.equiangular && s.fog.density > 0.) as u32) << 6
                // As on D3D12: when to write the object ids.
                | ((!jittered || (self.st.samples == 0 && self.up.sr_fresh <= 1)) as u32) << 7;
            let env = if s.physical_sky.on {
                (1., s.physical_sky.rotation(), s.physical_sky.visible)
            } else {
                (
                    s.hdri_strength,
                    s.hdri_rotation.to_radians(),
                    s.hdri_visible,
                )
            };
            let mut c = crate::render::constants(
                s,
                &self.st,
                &FrameInputs {
                    view,
                    proj,
                    eye,
                    previous_vp: if reset { vp } else { self.previous_vp },
                    n,
                    reset,
                    flags,
                    jitter,
                    sequence_base: self.sequence_base,
                    lut_size: self.lut_size,
                    present_count: self.present_count,
                    env_scale: self.env_scale,
                    env_inv_norm: self.env_inv_norm,
                    env_sun: self.env_sun,
                    env,
                    lens_surfaces: self.lens_surfaces,
                    glare_grid: self.glare_grid(),
                    restir: restir as u32
                        | (self.restir_history as u32) << 1
                        | self.restir_layers << 8,
                },
            );
            let (rw, rh) = (self.st.rw, self.st.rh);
            let path_request = self.st.path_request.take();
            if let Some(([x, y], count)) = path_request {
                c.path_record = [
                    (x * rw / self.st.width.max(1)).min(rw.saturating_sub(1)),
                    (y * rh / self.st.height.max(1)).min(rh.saturating_sub(1)),
                    count,
                    PATH_CAPACITY as u32,
                ];
            }
            // eye.w scales the UI to paper white in HDR (gui.hlsl); negative for HDR10.
            if self.st.hdr_active {
                c.eye[3] = self.st.hdr_levels[0] / 80. * if self.st.hdr10 { -1. } else { 1. };
            }
            let d = &self.device;
            d.write(&self.constants, 0, bytemuck::bytes_of(&c))?;
            let cmd = self.frame.cmd;
            self.frame.begin(d)?;
            d.device
                .cmd_reset_query_pool(cmd, self.queries, 0, TIMESTAMPS);
            self.stamp(0);
            // NGX and XeSS create their features in the command buffer.
            let sr = if self.create_upscalers(s, sr) {
                sr
            } else {
                // As if neither had been asked for: no texture 14, no jitter.
                c.scene_info[3] &= !3;
                c.jitter[0] = 0.;
                c.jitter[1] = 0.;
                self.device
                    .write(&self.constants, 0, bytemuck::bytes_of(&c))?;
                None
            };
            if path_request.is_some() {
                // Writes only the path records, so it may run before this frame's trace.
                self.dispatch("trace_paths", 0, [1, 1, 1]);
            }
            self.restir_ran = n > 0 && restir && (s.battle || s.integrator == 0);
            self.battle_ran = n > 0 && s.battle;
            if n > 0 {
                if s.battle {
                    self.battle_frame(s, &c, restir)?;
                } else {
                    let path_restir = restir && s.integrator == 0;
                    self.integrate(s, &c, 0, s.integrator, path_restir, [0, 0, rw, rh], n, 1.)?;
                    if path_restir {
                        self.restir_history = true;
                    }
                }
            }
            self.stamp(1);
            if n > 0 {
                if self.st.rr_active {
                    self.run_rr(s, jitter, reset, dt, view, proj);
                }
                self.reset_pending = false;
                self.previous_vp = vp;
            }
            if !self.st.rr_active && s.denoise && !s.battle {
                self.filter(s, &mut c, n > 0)?;
            } else {
                self.st.denoiser_activity = "";
                if n > 0 {
                    self.history_valid = false;
                    self.integrated_valid = false;
                }
            }
            if self.st.rr_active {
                self.st.denoiser_activity = if n == 0 {
                    "holding (no new samples)"
                } else if reset {
                    "history reset (scene edited or camera cut)"
                } else {
                    "accumulating over its history"
                };
            }
            if n > 0 {
                self.camera_moved = false;
            }
            if let Some(kind) = sr
                && (n > 0 || self.up.sr_reset)
            {
                // An accumulated input is the mean over every jitter phase, so it sits at the pixel centre.
                let jitter = if accumulating { [0., 0.] } else { jitter };
                let history = (self.up.sr_reset || reset) as i32;
                self.run_upscaler(kind, s, &mut c, jitter, history, dt);
            }
            self.stamp(2);
            c.modes[3] = self.st.rr_active as u32;
            c.filter[0] = 0;
            if s.glare > 0. {
                self.run_glare(s, &mut c)?;
            }
            let d = &self.device;
            if self.st.scopes_on {
                d.device.cmd_clear_color_image(
                    cmd,
                    self.textures[28].handle,
                    vk::ImageLayout::GENERAL,
                    &vk::ClearColorValue { uint32: [0; 4] },
                    &[COLOR],
                );
                full_barrier(&d.device, cmd);
            }
            d.write(&self.constants, 8 * CB_STRIDE, bytemuck::bytes_of(&c))?;
            self.dispatch(
                "display",
                8 * CB_STRIDE,
                [self.st.width.div_ceil(8), self.st.height.div_ceil(8), 1],
            );
            self.queue_readbacks();
            self.stamp(3);
            self.present(primitives, pixels_per_point)?;
            self.gui.free(&self.device, delta)?;
            // One frame in flight, as D3D12's submit: this frame's results are read now.
            self.device
                .device
                .wait_for_fences(&[self.frame.fence], true, u64::MAX)?;
            self.collect(s, n, path_request.is_some());
            self.present_count = self.present_count.wrapping_add(1);
            if n > 0 {
                self.st.frame = self.st.frame.wrapping_add(1);
            }
            Ok(())
        }
    }

    /// Ends the frame's command buffer: blits the output (texture 10) into the next swap chain
    /// image and presents it, or just submits when the window is minimized. Frames that trace
    /// nothing (paused, converged) still present: the UI must keep updating, as on D3D12.
    unsafe fn present(&mut self, primitives: &[egui::ClippedPrimitive], ppp: f32) -> Result<()> {
        unsafe {
            let d = &self.device;
            let (swap, frame) = (&mut self.swap, &self.frame);
            let cmd = frame.cmd;
            let shown = !self.st.swap_size.contains(&0);
            let index = if shown {
                match swap.fns.acquire_next_image(
                    swap.handle,
                    u64::MAX,
                    frame.acquire,
                    vk::Fence::null(),
                ) {
                    Ok((i, _)) => Some(i),
                    Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => None,
                    Err(e) => return Err(e.into()),
                }
            } else {
                None
            };
            if let Some(i) = index {
                let image = swap.images[i as usize];
                let barrier = |old, new, src, dst| {
                    [vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                        .src_access_mask(src)
                        .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                        .dst_access_mask(dst)
                        .old_layout(old)
                        .new_layout(new)
                        .image(image)
                        .subresource_range(COLOR)]
                };
                let to_dst = barrier(
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::AccessFlags2::NONE,
                    vk::AccessFlags2::TRANSFER_WRITE,
                );
                d.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_dst),
                );
                // In HDR the display pass writes the swap chain's format into texture 26.
                let out = &self.textures[if self.st.hdr_active { 26 } else { 10 }];
                let layers = vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                };
                let corner = |w: u32, h: u32| {
                    [
                        vk::Offset3D::default(),
                        vk::Offset3D {
                            x: w as i32,
                            y: h as i32,
                            z: 1,
                        },
                    ]
                };
                let region = vk::ImageBlit::default()
                    .src_subresource(layers)
                    .src_offsets(corner(out.size[0], out.size[1]))
                    .dst_subresource(layers)
                    .dst_offsets(corner(swap.extent.width, swap.extent.height));
                d.device.cmd_blit_image(
                    cmd,
                    out.handle,
                    vk::ImageLayout::GENERAL,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                    vk::Filter::LINEAR,
                );
                let to_draw = barrier(
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::AccessFlags2::TRANSFER_WRITE,
                    vk::AccessFlags2::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                );
                d.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
                );
                // gui.hlsl maps positions by the output size in the constants; scale egui
                // points to it when the swap chain differs (as D3D12's `draw_gui`).
                let fit = [
                    ppp * self.st.width as f32 / swap.extent.width as f32,
                    ppp * self.st.height as f32 / swap.extent.height as f32,
                ];
                self.gui.draw(
                    d,
                    cmd,
                    swap.views[i as usize],
                    swap.extent,
                    primitives,
                    ppp,
                    fit,
                    (8 * CB_STRIDE) as u32,
                )?;
                let mut last = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
                if self.st.keep_ui_frame {
                    // Keep the finished frame for `capture(include_ui)` (as D3D12's ui_frame).
                    let size = [swap.extent.width, swap.extent.height];
                    if self.ui_frame.as_ref().is_none_or(|(_, s)| *s != size) {
                        if let Some((b, _)) = self.ui_frame.take() {
                            d.free_buffer(b);
                        }
                        self.ui_frame = Some((
                            d.buffer(
                                size[0] as u64 * size[1] as u64 * texel_bytes(swap.format),
                                vk::BufferUsageFlags::TRANSFER_DST,
                                true,
                            )?,
                            size,
                        ));
                    }
                    let to_src = barrier(
                        last,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                        vk::AccessFlags2::TRANSFER_READ,
                    );
                    d.device.cmd_pipeline_barrier2(
                        cmd,
                        &vk::DependencyInfo::default().image_memory_barriers(&to_src),
                    );
                    let region = vk::BufferImageCopy::default()
                        .image_subresource(layers)
                        .image_extent(vk::Extent3D {
                            width: size[0],
                            height: size[1],
                            depth: 1,
                        });
                    d.device.cmd_copy_image_to_buffer(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        self.ui_frame.as_ref().unwrap().0.handle,
                        &[region],
                    );
                    last = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                }
                let to_present = barrier(
                    last,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE | vk::AccessFlags2::TRANSFER_READ,
                    vk::AccessFlags2::NONE,
                );
                d.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                );
            }
            d.device.cmd_write_timestamp2(
                cmd,
                vk::PipelineStageFlags2::ALL_COMMANDS,
                self.queries,
                4,
            );
            d.device.end_command_buffer(cmd)?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            let waits = [vk::SemaphoreSubmitInfo::default()
                .semaphore(frame.acquire)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let signals = index.map(|i| {
                [vk::SemaphoreSubmitInfo::default()
                    .semaphore(swap.release[i as usize])
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)]
            });
            let mut submit = vk::SubmitInfo2::default().command_buffer_infos(&cmds);
            if let Some(signals) = &signals {
                submit = submit
                    .wait_semaphore_infos(&waits)
                    .signal_semaphore_infos(signals);
            }
            d.device.queue_submit2(d.queue, &[submit], frame.fence)?;
            match index {
                Some(i) => {
                    let present_waits = [swap.release[i as usize]];
                    let swapchains = [swap.handle];
                    let indices = [i];
                    match swap.fns.queue_present(
                        d.queue,
                        &vk::PresentInfoKHR::default()
                            .wait_semaphores(&present_waits)
                            .swapchains(&swapchains)
                            .image_indices(&indices),
                    ) {
                        Ok(false) => {}
                        Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                            swap.recreate(d, self.st.swap_size)?
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                None if shown => swap.recreate(d, self.st.swap_size)?,
                None => {}
            }
            Ok(())
        }
    }

    unsafe fn stamp(&self, k: u32) {
        unsafe {
            self.device.device.cmd_write_timestamp2(
                self.frame.cmd,
                vk::PipelineStageFlags2::ALL_COMMANDS,
                self.queries,
                k,
            )
        };
    }

    /// Copies the scopes counters and the probe's texels into `readback` (as D3D12's
    /// `queue_readbacks`); `collect` reads them after the next fence wait.
    unsafe fn queue_readbacks(&mut self) {
        let probe = self
            .st
            .probe
            .filter(|p| p[0] < self.st.width && p[1] < self.st.height);
        self.pending_probe = probe;
        self.pending_scopes = self.st.scopes_on;
        let (d, cmd) = (&self.device.device, self.frame.cmd);
        let copy = |index: usize, x: u32, y: u32, w: u32, h: u32, offset: u64| {
            let region = vk::BufferImageCopy::default()
                .buffer_offset(offset)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D {
                    x: x as i32,
                    y: y as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: w,
                    height: h,
                    depth: 1,
                });
            unsafe {
                d.cmd_copy_image_to_buffer(
                    cmd,
                    self.textures[index].handle,
                    vk::ImageLayout::GENERAL,
                    self.readback.handle,
                    &[region],
                )
            };
        };
        if self.pending_scopes {
            copy(28, 0, 0, SCOPES[0], SCOPES[1], 0);
        }
        if let Some([x, y]) = probe {
            let rx = (x * self.st.rw / self.st.width).min(self.st.rw - 1);
            let ry = (y * self.st.rh / self.st.height).min(self.st.rh - 1);
            copy(18, x, y, 1, 1, PROBE);
            copy(3, rx, ry, 1, 1, PROBE + 16);
            copy(5, rx, ry, 1, 1, PROBE + 32);
            copy(19, rx, ry, 1, 1, PROBE + 48);
        }
    }

    /// This frame's timestamps (with battle mode's per-quadrant costs and samples), scopes,
    /// probe and path records; call after the frame fence wait.
    unsafe fn collect(&mut self, s: &Settings, n: u32, path: bool) {
        unsafe {
            let count = if self.battle_ran { TIMESTAMPS } else { 5 };
            let mut t = vec![0u64; count as usize];
            if self
                .device
                .device
                .get_query_pool_results(self.queries, 0, &mut t, vk::QueryResultFlags::TYPE_64)
                .is_ok()
            {
                let span = |a: usize, b: usize| t[b].saturating_sub(t[a]) as f64 * self.tick_ms;
                self.st.gpu_ms = span(0, 3);
                self.st.pass_ms = [span(0, 1), span(1, 2), span(2, 3), span(3, 4)];
                if self.battle_ran {
                    for (i, q) in self.st.battle.iter_mut().enumerate() {
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
                }
            }
            if s.battle {
                self.st.samples = self.st.battle.iter().map(|q| q.samples).min().unwrap_or(0);
            } else {
                self.st.samples += n;
            }
            let base = self.readback.mapped;
            if std::mem::take(&mut self.pending_scopes) {
                self.st.scopes = std::slice::from_raw_parts(
                    base as *const u32,
                    (SCOPES[0] * SCOPES[1]) as usize,
                )
                .to_vec();
            }
            self.st.probe_result = self.pending_probe.take().map(|_| {
                let texel = |offset: u64| {
                    let f = std::slice::from_raw_parts(base.add(offset as usize) as *const f32, 4);
                    [f[0], f[1], f[2], f[3]]
                };
                Probe {
                    linear: texel(PROBE),
                    albedo: texel(PROBE + 16),
                    depth: texel(PROBE + 32)[0],
                    id: texel(PROBE + 48)[0] as u32,
                }
            });
            if path {
                let all = std::slice::from_raw_parts(
                    self.path_buffer.mapped as *const crate::paths::Record,
                    PATH_CAPACITY,
                );
                let count = (all[0].info as usize).min(PATH_CAPACITY - 1);
                self.st.path_result = Some(all[1..=count].to_vec());
            }
        }
    }

    pub fn removed_reason(&self) -> Option<String> {
        None
    }
    pub fn reset(&mut self, reset_history: bool) {
        self.st.samples = 0;
        self.up.sr_fresh = 0;
        // Each restart takes a fresh, 8192-aligned Sobol block (as D3D12).
        self.restarts = self.restarts.wrapping_add(1);
        self.sequence_base = self.restarts.wrapping_mul(0x9E37_79B1) & !8191;
        self.photon_frames = 0;
        self.photon_scale = 1.;
        for q in &mut self.st.battle {
            *q = BattleQuadrant {
                cost: q.cost,
                ..Default::default()
            };
        }
        if reset_history {
            // As D3D12's rr_reset: motion vectors and RR history restart; a camera move keeps them.
            self.reset_pending = true;
            self.history_valid = false;
            self.restir_history = false;
            self.camera_moved = false;
        } else {
            self.camera_moved = true;
        }
    }

    /// The native denoiser (post.hlsl `filter`), as D3D12's `render`: pass 0 warps the last
    /// integrated frame into this view after a camera move, pass 1 integrates new samples,
    /// passes 2–6 are the à-trous filter.
    unsafe fn filter(&mut self, s: &Settings, c: &mut Constants, traced: bool) -> Result<()> {
        let reproject = traced && self.camera_moved && s.temporal && self.integrated_valid;
        if traced && self.camera_moved {
            self.history_valid = reproject;
        }
        if !s.temporal {
            self.history_valid = false;
        }
        self.st.denoiser_activity = if reproject {
            "reprojecting its history (camera moving)"
        } else if !traced {
            "filtering the held result (no new samples)"
        } else if self.history_valid {
            "integrating new samples over its history"
        } else if !s.temporal {
            "integrating new samples (temporal history off)"
        } else {
            "integrating from scratch (no usable history: an edit, a load or a resize)"
        };
        let first = if reproject {
            0
        } else if traced || !self.integrated_valid {
            1
        } else {
            2
        };
        let (d, cmd) = (&self.device, self.frame.cmd);
        for i in first..7 {
            c.filter[0] = i;
            c.filter[2] = !self.history_valid as u32;
            let offset = (i as u64 + 1) * CB_STRIDE;
            unsafe {
                d.write(&self.constants, offset, bytemuck::bytes_of(c))?;
                self.dispatch(
                    "filter",
                    offset,
                    [self.st.rw.div_ceil(8), self.st.rh.div_ceil(8), 1],
                );
                if i == 1 && traced {
                    // The next reprojection checks history against these guides.
                    for (from, to) in [(2, 16), (5, 17), (19, 20)] {
                        copy_image(&d.device, cmd, &self.textures[from], &self.textures[to]);
                    }
                    full_barrier(&d.device, cmd);
                }
            }
        }
        self.integrated_valid = true;
        Ok(())
    }
    pub unsafe fn resize(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        unsafe {
            self.resize_swap(w, h)?;
            self.resize_output(w, h, s)
        }
    }
    pub unsafe fn resize_swap(&mut self, w: u32, h: u32) -> Result<()> {
        self.st.swap_size = [w, h];
        if w > 0 && h > 0 {
            unsafe {
                self.swap.recreate(&self.device, [w, h])?;
                self.gui.set_format(&self.device, self.swap.format)?;
            }
        }
        Ok(())
    }
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid output size"
        );
        unsafe {
            self.device.device.device_wait_idle()?;
            // The upscalers are made for one size; the next frame recreates them.
            self.up.release();
            let d = &self.device;
            for i in self.textures.drain(..) {
                d.free_image(i);
            }
            self.st.width = w;
            self.st.height = h;
            self.st.rw = ((w as f32 * s.scale).round() as u32).max(1);
            self.st.rh = ((h as f32 * s.scale).round() as u32).max(1);
            for i in 0..TEXTURES {
                let hdr10 = self.st.hdr_active && self.st.hdr10;
                let (format, size) = texture_layout(i, hdr10, [w, h], [self.st.rw, self.st.rh]);
                let image = d.image(
                    [size[0], size[1], 1],
                    vk_format(format),
                    vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                )?;
                self.layout.image(d, i as u32, image.view);
                self.textures.push(image);
            }
        }
        unsafe { self.gui.set_object_id(&self.device, self.textures[19].view) };
        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    /// Shader hot reload (debug builds): the compute pipelines rebuilt from the SPIR-V that
    /// `hotreload` compiled. All are built before any is swapped, so an error keeps the old set.
    /// The UI pipelines stay (D3D12 reloads those too).
    pub unsafe fn reload_shaders(&mut self, sh: &crate::hotreload::Shaders) -> Result<()> {
        let list: [(&'static str, &[u8], &std::ffi::CStr); 22] = [
            ("trace", &sh.trace, c"trace"),
            ("trace_maps", &sh.trace_maps, c"trace"),
            ("trace_lean", &sh.trace_lean, c"trace"),
            ("trace_maps_lean", &sh.trace_maps_lean, c"trace"),
            ("trace_spectral", &sh.trace_spectral, c"trace"),
            ("trace_spectral_lean", &sh.trace_spectral_lean, c"trace"),
            ("trace_paths", &sh.trace_paths, c"trace"),
            ("trace_lpe", &sh.trace_lpe, c"trace"),
            (
                "restir_presample",
                &sh.restir_presample,
                c"restir_presample",
            ),
            ("restir_initial", &sh.restir_initial, c"restir_initial"),
            ("restir_spatial", &sh.restir_spatial, c"restir_spatial"),
            ("bdpt", &sh.bdpt, c"bdpt"),
            ("bdpt_spectral", &sh.bdpt_spectral, c"bdpt"),
            ("bdpt_resolve", &sh.bdpt_resolve, c"bdpt_resolve"),
            ("photon_clear", &sh.photon_clear, c"photon_clear"),
            ("photon_trace", &sh.photon_trace, c"photon_trace"),
            (
                "photon_trace_spectral",
                &sh.photon_trace_spectral,
                c"photon_trace",
            ),
            ("photon_gather", &sh.photon_gather, c"photon_gather"),
            (
                "photon_gather_spectral",
                &sh.photon_gather_spectral,
                c"photon_gather",
            ),
            ("filter", &sh.filter, c"filter"),
            ("display", &sh.display, c"display"),
            ("glare", &sh.glare, c"glare_pass"),
        ];
        unsafe {
            let mut built = vec![];
            for (name, code, entry) in list {
                match self.layout.compute(
                    &self.device,
                    code,
                    entry,
                    vk::PipelineCreateFlags::empty(),
                ) {
                    Ok(p) => built.push((name, p)),
                    Err(e) => {
                        for (_, p) in built {
                            self.device.device.destroy_pipeline(p, None);
                        }
                        return Err(e.context(format!("{name} pipeline")));
                    }
                }
            }
            self.device.device.device_wait_idle()?;
            for (name, p) in built {
                if let Some(old) = self.pipes.insert(name, p) {
                    self.device.device.destroy_pipeline(old, None);
                }
            }
        }
        self.reset(true);
        Ok(())
    }
    /// Switches the swap chain between SDR, scRGB and (with `hdr10`) HDR10, as D3D12's
    /// `set_hdr`: HDR10 falls back to scRGB when the surface lacks it; false when HDR is off
    /// because the surface offers neither.
    pub unsafe fn set_hdr(&mut self, on: bool, hdr10: bool, s: &Settings) -> Result<bool> {
        if on == self.st.hdr_active && (!on || hdr10 == self.st.hdr10) {
            return Ok(true);
        }
        let mut want = if !on {
            0
        } else if hdr10 {
            2
        } else {
            1
        };
        unsafe {
            loop {
                self.swap.hdr = want;
                if self.st.swap_size.contains(&0) {
                    break;
                }
                self.swap.recreate(&self.device, self.st.swap_size)?;
                if self.swap.hdr_ok || want == 0 {
                    break;
                }
                if want == 2 {
                    self.st.hdr10_refused = true;
                }
                want -= 1;
            }
            self.st.hdr_active = want > 0;
            self.st.hdr10 = want == 2;
            self.gui.set_format(&self.device, self.swap.format)?;
            let [w, h] = [self.st.width, self.st.height];
            self.resize_output(w, h, s)?;
        }
        self.st.hdr_refused = on && want == 0;
        Ok(!on || want > 0)
    }
    /// The raw bytes of texture `index`, tightly packed rows.
    unsafe fn read_image(&mut self, index: usize) -> Result<(u32, u32, Vec<u8>)> {
        unsafe {
            let d = &self.device;
            d.device.device_wait_idle()?;
            let img = &self.textures[index];
            let texel = match img.format {
                vk::Format::R32G32B32A32_SFLOAT => 16,
                vk::Format::R32G32_SFLOAT | vk::Format::R16G16B16A16_SFLOAT => 8,
                _ => 4,
            };
            let bytes = img.size[0] as u64 * img.size[1] as u64 * texel;
            let staging = d.buffer(bytes, vk::BufferUsageFlags::TRANSFER_DST, true)?;
            let region = vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D {
                    width: img.size[0],
                    height: img.size[1],
                    depth: 1,
                });
            d.one_shot(|cmd| {
                d.device.cmd_copy_image_to_buffer(
                    cmd,
                    img.handle,
                    vk::ImageLayout::GENERAL,
                    staging.handle,
                    &[region],
                )
            })?;
            let out = std::slice::from_raw_parts(staging.mapped, bytes as usize).to_vec();
            let size = (img.size[0], img.size[1]);
            d.free_buffer(staging);
            Ok((size.0, size.1, out))
        }
    }

    pub unsafe fn capture(
        &mut self,
        path: &std::path::Path,
        hdr: bool,
        include_ui: bool,
    ) -> Result<()> {
        if include_ui {
            let (buffer, [w, h]) = self
                .ui_frame
                .as_ref()
                .context("UI capture needs keep_ui_frame set before rendering")?;
            unsafe { self.device.device.device_wait_idle()? };
            let format = self.swap.format;
            let len = *w as usize * *h as usize * texel_bytes(format) as usize;
            ensure!(
                buffer.size as usize >= len,
                "UI frame from another swap chain format"
            );
            let raw = unsafe { std::slice::from_raw_parts(buffer.mapped, len) };
            let mut px = match format {
                vk::Format::R16G16B16A16_SFLOAT => {
                    crate::render::hdr_ui_to_srgb8(raw, false, self.st.hdr_levels[0])
                }
                vk::Format::A2B10G10R10_UNORM_PACK32 => {
                    crate::render::hdr_ui_to_srgb8(raw, true, self.st.hdr_levels[0])
                }
                _ => raw.to_vec(),
            };
            let bgra = format == vk::Format::B8G8R8A8_UNORM;
            for t in px.as_chunks_mut::<4>().0 {
                if bgra {
                    t.swap(0, 2);
                }
                t[3] = 255;
            }
            image::save_buffer(path, &px, *w, *h, image::ColorType::Rgba8)?;
            return Ok(());
        }
        // `name.layers.exr`: every AOV as a layer, as on D3D12.
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".layers.exr"))
        {
            let layers = unsafe { self.read_layers()? };
            return crate::video::write_layers(path, &layers);
        }
        let (w, h, bytes) = unsafe { self.read_image(if hdr { 18 } else { 10 })? };
        if hdr {
            let rgb: Vec<f32> = bytes
                .as_chunks::<16>()
                .0
                .iter()
                .flat_map(|t| {
                    (0..3).map(|k| f32::from_le_bytes(t[k * 4..k * 4 + 4].try_into().unwrap()))
                })
                .collect();
            image::Rgb32FImage::from_raw(w, h, rgb)
                .context("image size")?
                .save(path)?;
        } else {
            image::save_buffer(path, &bytes, w, h, image::ColorType::Rgba8)?;
        }
        Ok(())
    }
    pub unsafe fn read_rgba8(&mut self, index: usize) -> Result<Vec<u8>> {
        ensure!(
            self.textures[index].format == vk::Format::R8G8B8A8_UNORM,
            "Texture {index} is not RGBA8"
        );
        Ok(unsafe { self.read_image(index)? }.2)
    }
    /// A float texture as (width, height, channels, samples), as D3D12's `read_texture`.
    pub unsafe fn read_texture(&mut self, index: usize) -> Result<(u32, u32, usize, Vec<f32>)> {
        let (channels, half) = match self.textures[index].format {
            vk::Format::R32_SFLOAT => (1, false),
            vk::Format::R32G32_SFLOAT => (2, false),
            vk::Format::R32G32B32A32_SFLOAT => (4, false),
            vk::Format::R16G16B16A16_SFLOAT => (4, true),
            _ => bail!("Texture {index} is not a float texture"),
        };
        let (w, h, bytes) = unsafe { self.read_image(index)? };
        let data = if half {
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| crate::render::half_to_f32(u16::from_le_bytes(*b)))
                .collect()
        } else {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        };
        Ok((w, h, channels, data))
    }
    pub unsafe fn read_layers(&mut self) -> Result<Vec<AovLayer>> {
        crate::render::aov_layers(|i| unsafe { self.read_texture(i) })
    }
    /// Four floats per output pixel of a four-channel float texture, as D3D12's `read_floats`.
    pub unsafe fn read_floats(&mut self, index: usize) -> Result<Vec<f32>> {
        if self.textures[index].format == vk::Format::A2B10G10R10_UNORM_PACK32 {
            // The HDR10 output, back to scRGB as on D3D12.
            let (_, _, bytes) = unsafe { self.read_image(index)? };
            return Ok(bytes
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|b| crate::display::hdr10_to_scrgb(u32::from_le_bytes(*b)))
                .collect());
        }
        let (w, h, channels, data) = unsafe { self.read_texture(index)? };
        ensure!(channels == 4, "Texture {index} has {channels} channels");
        let (ow, oh) = (
            self.st.width.min(w) as usize,
            self.st.height.min(h) as usize,
        );
        Ok((0..oh)
            .flat_map(|y| data[y * w as usize * 4..][..ow * 4].iter().copied())
            .collect())
    }
    pub unsafe fn debug_messages(&self) -> Vec<String> {
        super::messages()
    }
}
