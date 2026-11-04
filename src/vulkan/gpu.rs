//! The app-facing Vulkan renderer: the same API as the D3D12 `gpu::Gpu`.
//!
//! Phase 3 (engine-notes "Vulkan backend: design"): the storage textures, the scene (vertex
//! buffers, BLAS / TLAS, materials, emitters, instance table, all from `render::plan`), the
//! path-tracing `trace` pass and `display`, presented by a blit, and captures. The rest of the
//! passes arrive in phases 4 and 5; until then the calls that need them are no-ops or say that
//! Vulkan lacks them.
use super::layout::{Layout, SET0_BUFFERS};
use super::res::{Accel, Buffer, COLOR, Image, full_barrier, triangles};
use super::{Device, Frame, Swapchain};
use crate::render::plan;
pub use crate::render::*;
use crate::scene::{Settings, Upscaler, Vertex};
use anyhow::{Context, Result, bail, ensure};
use ash::{khr, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

const NOT_YET: &str = "not on Vulkan yet";

macro_rules! spv {
    ($name:literal) => {{
        #[cfg(spirv)]
        let code: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".spv"));
        #[cfg(not(spirv))]
        let code: &[u8] = &[];
        code
    }};
}

struct Pipelines {
    trace: vk::Pipeline,
    trace_spectral: vk::Pipeline,
    display: vk::Pipeline,
}

/// What `rebuild_scene` uploads; replaced whole on every rebuild for now (D3D12 keeps BLASes
/// and vertex ranges across rebuilds).
struct Scene {
    statics: Buffer,
    meshes: Buffer,
    aabb: Buffer,
    blases: Vec<Accel>,
    tlas: Accel,
    instance_descs: Buffer,
}

pub struct Gpu {
    pub st: GpuState,
    swap: Swapchain,
    frame: Frame,
    accel_fns: khr::acceleration_structure::Device,
    layout: Layout,
    pipes: Pipelines,
    textures: Vec<Image>,
    /// u29 LUT, u30 environment map, u31 its CDFs.
    fixed: Vec<Image>,
    constants: Buffer,
    /// Stands in for every buffer a pass doesn't read this frame (D3D12 binds the path buffer).
    placeholder: Buffer,
    scene: Option<Scene>,
    /// t3 materials, t4 emitters, t5 emitter lookup, t7 instance table.
    tables: Vec<Buffer>,
    static_cache: Vec<Vertex>,
    instances: Vec<plan::InstanceRecord>,
    lean_scene: bool,
    previous_vp: glam::Mat4,
    reset_pending: bool,
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
            self.free_scene();
            let d = &self.device;
            for b in self.tables.drain(..) {
                d.free_buffer(b);
            }
            for i in self.textures.drain(..).chain(self.fixed.drain(..)) {
                d.free_image(i);
            }
            // Not used again: Drop ends here.
            d.free_buffer(std::ptr::read(&self.constants));
            d.free_buffer(std::ptr::read(&self.placeholder));
            for p in [
                self.pipes.trace,
                self.pipes.trace_spectral,
                self.pipes.display,
            ] {
                d.device.destroy_pipeline(p, None);
            }
            self.layout.destroy(d);
            self.frame.destroy(d);
            self.swap.destroy(d);
        }
    }
}

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
            let swap = Swapchain::new(
                &device,
                surface.context("no surface")?,
                [width, height],
                true,
            )?;
            let frame = Frame::new(&device)?;
            let accel_fns =
                khr::acceleration_structure::Device::new(&device.instance, &device.device);
            let layout = Layout::new(&device)?;
            let pipes = Pipelines {
                trace: layout.compute(&device, spv!("trace"), c"trace")?,
                trace_spectral: layout.compute(&device, spv!("trace_spectral"), c"trace")?,
                display: layout.compute(&device, spv!("display"), c"display")?,
            };
            let constants = device.buffer(
                CB_STRIDE * CB_SLOTS,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                true,
            )?;
            layout.constants(&device, constants.handle, CB_STRIDE);
            let placeholder =
                device.buffer(1 << 16, vk::BufferUsageFlags::STORAGE_BUFFER, false)?;
            for b in SET0_BUFFERS {
                layout.buffer(&device, 0, b, placeholder.handle);
            }
            for b in 0..5 {
                layout.buffer(&device, 2, b, placeholder.handle);
            }
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
                upscale_status: NOT_YET.into(),
                rr_status: NOT_YET.into(),
                denoiser_activity: "",
                hdr_levels: [203., 1000.],
                ..Default::default()
            };
            let mut gpu = Self {
                st,
                swap,
                frame,
                accel_fns,
                layout,
                pipes,
                textures: vec![],
                fixed,
                constants,
                placeholder,
                scene: None,
                tables: vec![],
                static_cache: vec![],
                instances: vec![],
                lean_scene: false,
                previous_vp: glam::Mat4::IDENTITY,
                reset_pending: true,
                device,
            };
            gpu.st.swap_size = [width, height];
            gpu.resize_output(width, height, s)?;
            gpu.rebuild_scene(s)?;
            Ok(gpu)
        }
    }

    unsafe fn free_scene(&mut self) {
        if let Some(sc) = self.scene.take() {
            let d = &self.device;
            unsafe {
                d.free_accel(&self.accel_fns, sc.tlas);
                for b in sc.blases {
                    d.free_accel(&self.accel_fns, b);
                }
                for b in [sc.statics, sc.meshes, sc.aabb, sc.instance_descs] {
                    d.free_buffer(b);
                }
            }
        }
    }

    pub unsafe fn rebuild_scene(&mut self, s: &Settings) -> Result<()> {
        unsafe {
            self.device.device.device_wait_idle()?;
            self.free_scene();
            let d = &self.device;
            let packed = crate::lighting::materials(s);
            let (groups, statics) = plan::static_layout(s, &packed);
            let visible = plan::visible_meshes(s, &packed, false);
            // Every distinct mesh once, in the order first seen.
            let mut ranges = std::collections::HashMap::new();
            let mut vertices: Vec<Vertex> = vec![];
            for v in &visible {
                if ranges.contains_key(&v.blas_key) {
                    continue;
                }
                ranges.insert(
                    v.blas_key.clone(),
                    (vertices.len() as u32, v.source.len() / 3),
                );
                let uv = plan::mesh_uvs(&s.objects[v.index], v.source.len());
                vertices.extend(v.source.iter().zip(uv).map(|((p, n), uv)| Vertex {
                    position: p.to_array(),
                    material: 0,
                    normal: n.to_array(),
                    pad: 0,
                    uv,
                }));
            }
            let input = vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
            let statics_buf = d.upload(bytemuck::cast_slice(&statics), input)?;
            let meshes = d.upload(bytemuck::cast_slice(&vertices), input)?;
            let aabb = d.upload(bytemuck::cast_slice(&[-1f32, -1., -1., 1., 1., 1.]), input)?;
            let stride = size_of::<Vertex>() as u64;
            let blas_ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
            let mut blases = vec![];
            for g in &groups {
                let geometry = triangles(
                    statics_buf.address + g.first as u64 * 3 * stride,
                    g.count,
                    !g.any_hit,
                );
                blases.push(d.build_accel(&self.accel_fns, blas_ty, &[geometry], &[g.count])?);
            }
            let mut mesh_blas = std::collections::HashMap::new();
            for (key, &(first, count)) in &ranges {
                let geometry =
                    triangles(meshes.address + first as u64 * stride, count as u32, false);
                mesh_blas.insert(key.clone(), blases.len());
                blases.push(d.build_accel(
                    &self.accel_fns,
                    blas_ty,
                    &[geometry],
                    &[count as u32],
                )?);
            }
            let sdf_geometry = vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::AABBS)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default()
                        .data(vk::DeviceOrHostAddressConstKHR {
                            device_address: aabb.address,
                        })
                        .stride(24),
                });
            let sdf = blases.len();
            blases.push(d.build_accel(&self.accel_fns, blas_ty, &[sdf_geometry], &[1])?);

            let (instances, table, triangle_count) =
                plan::instances(s, &packed, &groups, &visible, &|k| ranges[k]);
            let descs: Vec<vk::AccelerationStructureInstanceKHR> = instances
                .iter()
                .enumerate()
                .map(|(id, inst)| {
                    let blas = match inst.blas {
                        plan::Blas::Static(i) => &blases[i],
                        plan::Blas::Mesh(k) => &blases[mesh_blas[k]],
                        plan::Blas::Sdf => &blases[sdf],
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
            let instance_descs = d.upload(desc_bytes, input)?;
            let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    instances: vk::AccelerationStructureGeometryInstancesDataKHR::default().data(
                        vk::DeviceOrHostAddressConstKHR {
                            device_address: instance_descs.address,
                        },
                    ),
                });
            let tlas = d.build_accel(
                &self.accel_fns,
                vk::AccelerationStructureTypeKHR::TOP_LEVEL,
                &[tlas_geometry],
                &[descs.len() as u32],
            )?;
            self.layout.accel(d, tlas.handle);
            self.layout.buffer(d, 0, 101, meshes.handle);
            self.layout.buffer(d, 0, 106, statics_buf.handle);
            self.scene = Some(Scene {
                statics: statics_buf,
                meshes,
                aabb,
                blases,
                tlas,
                instance_descs,
            });
            self.static_cache = statics;
            self.instances = table;
            self.st.triangle_count = triangle_count;
            self.update_materials(s)
        }
    }

    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        unsafe {
            self.device.device.device_wait_idle()?;
            let d = &self.device;
            let materials = crate::lighting::materials(s);
            let lights = plan::lights(s, &materials, &self.static_cache, &mut self.instances);
            self.st.sun_count = lights.sun_count;
            self.st.emitter_count = lights.emitter_count;
            self.lean_scene = lights.lean_scene;
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
        _dt: f32,
        _primitives: &[egui::ClippedPrimitive],
        _delta: &egui::TexturesDelta,
        _pixels_per_point: f32,
    ) -> Result<()> {
        unsafe {
            let d = &self.device;
            d.device
                .wait_for_fences(&[self.frame.fence], true, u64::MAX)?;
            let (view, proj, eye) = s.camera(self.st.width as f32 / self.st.height as f32);
            let vp = proj * view;
            let reset = std::mem::take(&mut self.reset_pending);
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
            let flags = spatial << 2
                | s.mis_mode.min(2) << 4
                | ((s.fog.equiangular && s.fog.density > 0.) as u32) << 6;
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
                    jitter: [0., 0.],
                    sequence_base: 0,
                    lut_size: 0,
                    present_count: self.st.frame,
                    env_scale: 1.,
                    env_inv_norm: 0.,
                    env_sun: None,
                    env,
                    lens_surfaces: 0,
                    glare_grid: 0,
                    restir: 0,
                },
            );
            // The native denoiser comes with phase 4: show the raw accumulation meanwhile.
            c.modes[2] = 0;
            c.region = [0, 0, self.st.rw, self.st.rh];
            c.integrator = [0; 4];
            let cmd = self.frame.cmd;
            self.frame.begin(d)?;
            if n > 0 {
                d.write(&self.constants, 0, bytemuck::bytes_of(&c))?;
                self.layout.bind(d, cmd, 0);
                let trace = if s.spectral {
                    self.pipes.trace_spectral
                } else {
                    self.pipes.trace
                };
                d.device
                    .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, trace);
                d.device
                    .cmd_dispatch(cmd, self.st.rw.div_ceil(8), self.st.rh.div_ceil(8), 1);
                full_barrier(&d.device, cmd);
            }
            d.write(&self.constants, 8 * CB_STRIDE, bytemuck::bytes_of(&c))?;
            self.layout.bind(d, cmd, (8 * CB_STRIDE) as u32);
            d.device
                .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipes.display);
            d.device.cmd_dispatch(
                cmd,
                self.st.width.div_ceil(8),
                self.st.height.div_ceil(8),
                1,
            );
            full_barrier(&d.device, cmd);
            self.previous_vp = vp;
            self.st.samples += n;
            self.st.frame += 1;
            self.present(draw)
        }
    }

    /// Ends the frame's command buffer: blits the output (texture 10) into the next swap chain
    /// image and presents it, or just submits when the window is minimized or not drawn.
    unsafe fn present(&mut self, draw: bool) -> Result<()> {
        unsafe {
            let d = &self.device;
            let (swap, frame) = (&mut self.swap, &self.frame);
            let cmd = frame.cmd;
            let shown = draw && !self.st.swap_size.contains(&0);
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
                let out = &self.textures[10];
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
                let to_present = barrier(
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                    vk::AccessFlags2::TRANSFER_WRITE,
                    vk::AccessFlags2::NONE,
                );
                d.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                );
            }
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

    pub fn removed_reason(&self) -> Option<String> {
        None
    }
    pub fn reset(&mut self, _reset_history: bool) {
        self.st.samples = 0;
        self.reset_pending = true;
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
            unsafe { self.swap.recreate(&self.device, [w, h])? };
        }
        Ok(())
    }
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid output size"
        );
        unsafe {
            let d = &self.device;
            d.device.device_wait_idle()?;
            for i in self.textures.drain(..) {
                d.free_image(i);
            }
            self.st.width = w;
            self.st.height = h;
            self.st.rw = ((w as f32 * s.scale).round() as u32).max(1);
            self.st.rh = ((h as f32 * s.scale).round() as u32).max(1);
            for i in 0..TEXTURES {
                let (format, size) = texture_layout(i, false, [w, h], [self.st.rw, self.st.rh]);
                let image = d.image(
                    [size[0], size[1], 1],
                    vk_format(format),
                    vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                )?;
                self.layout.image(d, i as u32, image.view);
                self.textures.push(image);
            }
        }
        self.reset(true);
        Ok(())
    }
    pub unsafe fn reload_shaders(&mut self, _sh: &crate::hotreload::Shaders) -> Result<()> {
        Ok(())
    }
    pub unsafe fn reconfigure_upscalers(&mut self) -> Result<()> {
        Ok(())
    }
    pub fn capabilities(&self) -> Capabilities {
        let no = || Backend {
            available: false,
            reason: NOT_YET.into(),
        };
        Capabilities {
            rr: no(),
            dlss_sr: no(),
            fsr: no(),
            xess: no(),
        }
    }
    pub fn upscaler_usable(&self, kind: Upscaler) -> bool {
        self.upscaler_available(kind)
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        matches!(kind, Upscaler::Bilinear | Upscaler::Bicubic)
    }
    pub unsafe fn set_hdr(&mut self, _on: bool, _hdr10: bool, _s: &Settings) -> Result<bool> {
        Ok(false)
    }
    pub unsafe fn set_lut(&mut self, _lut: Option<(u32, &[[f32; 3]])>) -> Result<()> {
        Ok(())
    }
    pub fn map_count(&self) -> usize {
        0
    }
    pub fn environment_loaded(&self) -> bool {
        false
    }
    pub unsafe fn set_environment(
        &mut self,
        _env: Option<&crate::environment::EnvMap>,
    ) -> Result<()> {
        Ok(())
    }
    pub fn restir_layers(&self) -> u32 {
        0
    }
    pub unsafe fn upload_reference(&mut self, _rgba: &[f32]) -> Result<()> {
        Ok(())
    }
    pub unsafe fn capture_reference(&mut self) -> Result<()> {
        Ok(())
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
        ensure!(!include_ui, "UI captures are {NOT_YET}");
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
    pub unsafe fn read_layers(&mut self) -> Result<Vec<AovLayer>> {
        bail!("readbacks are {NOT_YET}")
    }
    pub unsafe fn read_floats(&mut self, _index: usize) -> Result<Vec<f32>> {
        bail!("readbacks are {NOT_YET}")
    }
    pub unsafe fn read_lpe(&mut self, _slot: u32) -> Result<Vec<f32>> {
        bail!("LPE is {NOT_YET}")
    }
    pub unsafe fn debug_messages(&self) -> Vec<String> {
        super::messages()
    }
}
