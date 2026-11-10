//! The egui pass on Vulkan (gui.hlsl, as D3D12's `draw_gui`): one indexed draw per mesh onto the
//! swap chain image after the output blit, with dynamic rendering.
//!
//! gui.hlsl reads set 0 bindings 19 (object ids, for the silhouette outlines), 102 (the egui
//! texture), 200 (the frame constants) and 300 (the sampler). The texture changes between draws,
//! which one descriptor set can't do inside a command buffer, so the pass has its own set layout
//! with just those four and one set per egui texture.
use super::Device;
use super::res::{Buffer, Image};
use anyhow::Result;
use ash::vk;
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GuiVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

struct Texture {
    image: Image,
    set: vk::DescriptorSet,
    pixels: Vec<u8>,
    size: [usize; 2],
}

pub struct Gui {
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pool: vk::DescriptorPool,
    sampler: vk::Sampler,
    pipeline: vk::Pipeline,
    format: vk::Format,
    shaders: (&'static [u8], &'static [u8]),
    textures: HashMap<egui::TextureId, Texture>,
    /// What the GPU may still read from the last frame: freed after the next fence wait.
    retired: Vec<Image>,
    buffers: Vec<Buffer>,
    object_id: vk::ImageView,
    constants: (vk::Buffer, u64),
}

const MAX_TEXTURES: u32 = 256;

impl Gui {
    pub unsafe fn new(
        d: &Device,
        format: vk::Format,
        constants: (vk::Buffer, u64),
        vs: &'static [u8],
        ps: &'static [u8],
    ) -> Result<Self> {
        unsafe {
            let dev = &d.device;
            let sampler = dev.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )?;
            let samplers = [sampler];
            let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
            let b = |binding, ty| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(ty)
                    .descriptor_count(1)
                    .stage_flags(stages)
            };
            let bindings = [
                b(19, vk::DescriptorType::STORAGE_IMAGE),
                b(102, vk::DescriptorType::SAMPLED_IMAGE),
                b(200, vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC),
                b(300, vk::DescriptorType::SAMPLER).immutable_samplers(&samplers),
            ];
            let set_layout = dev.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let layout = dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[set_layout]),
                None,
            )?;
            let size = |ty| vk::DescriptorPoolSize {
                ty,
                descriptor_count: MAX_TEXTURES,
            };
            let pool = dev.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .max_sets(MAX_TEXTURES)
                    .pool_sizes(&[
                        size(vk::DescriptorType::STORAGE_IMAGE),
                        size(vk::DescriptorType::SAMPLED_IMAGE),
                        size(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC),
                        size(vk::DescriptorType::SAMPLER),
                    ]),
                None,
            )?;
            let pipeline = make_pipeline(d, layout, format, vs, ps)?;
            Ok(Self {
                set_layout,
                layout,
                pool,
                sampler,
                pipeline,
                format,
                shaders: (vs, ps),
                textures: HashMap::new(),
                retired: vec![],
                buffers: vec![],
                object_id: vk::ImageView::null(),
                constants,
            })
        }
    }

    /// Rebuilds the pipeline when the swap chain's format changes (HDR on or off).
    pub unsafe fn set_format(&mut self, d: &Device, format: vk::Format) -> Result<()> {
        if format != self.format {
            unsafe {
                let pipeline =
                    make_pipeline(d, self.layout, format, self.shaders.0, self.shaders.1)?;
                d.device
                    .destroy_pipeline(std::mem::replace(&mut self.pipeline, pipeline), None);
            }
            self.format = format;
        }
        Ok(())
    }

    unsafe fn write_set(&self, d: &Device, set: vk::DescriptorSet, texture: vk::ImageView) {
        let ids = [vk::DescriptorImageInfo::default()
            .image_view(self.object_id)
            .image_layout(vk::ImageLayout::GENERAL)];
        let tex = [vk::DescriptorImageInfo::default()
            .image_view(texture)
            .image_layout(vk::ImageLayout::GENERAL)];
        let cb = [vk::DescriptorBufferInfo::default()
            .buffer(self.constants.0)
            .range(self.constants.1)];
        let mut writes = vec![
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(102)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&tex),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(200)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&cb),
        ];
        if self.object_id != vk::ImageView::null() {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(19)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .image_info(&ids),
            );
        }
        unsafe { d.device.update_descriptor_sets(&writes, &[]) };
    }

    /// The object-id texture (u19) after the render textures are recreated.
    pub unsafe fn set_object_id(&mut self, d: &Device, view: vk::ImageView) {
        self.object_id = view;
        for t in self.textures.values() {
            unsafe { self.write_set(d, t.set, t.image.view) };
        }
    }

    /// Frees what the last frame used and applies egui's texture changes. Call after the
    /// frame fence wait and before recording.
    pub unsafe fn update(&mut self, d: &Device, delta: &egui::TexturesDelta) -> Result<()> {
        unsafe {
            for i in self.retired.drain(..) {
                d.free_image(i);
            }
            for b in self.buffers.drain(..) {
                d.free_buffer(b);
            }
            for (id, changes) in &delta.set {
                for change in changes {
                    let size = change.image.size();
                    let pixels: Vec<egui::Color32> = match &change.image {
                        egui::ImageData::Color(i) => i.pixels.clone(),
                    };
                    let old = self.textures.remove(id);
                    let (mut data, mut full, set) = match old {
                        Some(t) => {
                            self.retired.push(t.image);
                            (t.pixels, t.size, t.set)
                        }
                        None => (
                            vec![],
                            [0, 0],
                            d.device.allocate_descriptor_sets(
                                &vk::DescriptorSetAllocateInfo::default()
                                    .descriptor_pool(self.pool)
                                    .set_layouts(&[self.set_layout]),
                            )?[0],
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
                    let image = d.image(
                        [full[0] as u32, full[1] as u32, 1],
                        vk::Format::R8G8B8A8_UNORM,
                        vk::ImageUsageFlags::SAMPLED,
                    )?;
                    let staging =
                        d.buffer(data.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
                    d.write(&staging, 0, &data)?;
                    let region = vk::BufferImageCopy::default()
                        .image_subresource(vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .image_extent(vk::Extent3D {
                            width: full[0] as u32,
                            height: full[1] as u32,
                            depth: 1,
                        });
                    d.one_shot(|cmd| {
                        d.device.cmd_copy_buffer_to_image(
                            cmd,
                            staging.handle,
                            image.handle,
                            vk::ImageLayout::GENERAL,
                            &[region],
                        )
                    })?;
                    d.free_buffer(staging);
                    self.write_set(d, set, image.view);
                    self.textures.insert(
                        *id,
                        Texture {
                            image,
                            set,
                            pixels: data,
                            size: full,
                        },
                    );
                }
            }
            Ok(())
        }
    }

    /// Frees egui's released textures; call after the frame's draw is recorded (one frame in
    /// flight: the images go with the next `update`).
    pub unsafe fn free(&mut self, d: &Device, delta: &egui::TexturesDelta) -> Result<()> {
        for id in &delta.free {
            if let Some(t) = self.textures.remove(id) {
                self.retired.push(t.image);
                unsafe { d.device.free_descriptor_sets(self.pool, &[t.set])? };
            }
        }
        Ok(())
    }

    /// Records the draws into `target` (a swap chain image view in COLOR_ATTACHMENT_OPTIMAL).
    /// `fit` scales egui points to the output size the constants carry (gui.hlsl), `offset` is
    /// the constants slot.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &mut self,
        d: &Device,
        cmd: vk::CommandBuffer,
        target: vk::ImageView,
        extent: vk::Extent2D,
        primitives: &[egui::ClippedPrimitive],
        ppp: f32,
        fit: [f32; 2],
        offset: u32,
    ) -> Result<()> {
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        for primitive in primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive
                && !mesh.indices.is_empty()
                && let Some(texture) = self.textures.get(&mesh.texture_id)
            {
                draws.push((
                    primitive.clip_rect,
                    indices.len() as u32,
                    mesh.indices.len() as u32,
                    vertices.len() as i32,
                    texture.set,
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
        unsafe {
            let dev = &d.device;
            let bytes: &[u8] = bytemuck::cast_slice(&vertices);
            let vb = d.buffer(
                bytes.len() as u64,
                vk::BufferUsageFlags::VERTEX_BUFFER,
                true,
            )?;
            d.write(&vb, 0, bytes)?;
            let bytes: &[u8] = bytemuck::cast_slice(&indices);
            let ib = d.buffer(bytes.len() as u64, vk::BufferUsageFlags::INDEX_BUFFER, true)?;
            d.write(&ib, 0, bytes)?;
            let attachment = [vk::RenderingAttachmentInfo::default()
                .image_view(target)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)];
            dev.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(extent.into())
                    .layer_count(1)
                    .color_attachments(&attachment),
            );
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            let [sw, sh] = [extent.width as f32, extent.height as f32];
            // Negative height: gui.hlsl's clip space is D3D's (y up).
            dev.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.,
                    y: sh,
                    width: sw,
                    height: -sh,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            dev.cmd_bind_vertex_buffers(cmd, 0, &[vb.handle], &[0]);
            dev.cmd_bind_index_buffer(cmd, ib.handle, 0, vk::IndexType::UINT32);
            let mut bound = vk::DescriptorSet::null();
            for (r, first, count, base, set) in draws {
                let left = (r.min.x * ppp).clamp(0., sw) as i32;
                let top = (r.min.y * ppp).clamp(0., sh) as i32;
                let right = (r.max.x * ppp).clamp(left as f32, sw) as i32;
                let bottom = (r.max.y * ppp).clamp(top as f32, sh) as i32;
                if right <= left || bottom <= top {
                    continue;
                }
                dev.cmd_set_scissor(
                    cmd,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D { x: left, y: top },
                        extent: vk::Extent2D {
                            width: (right - left) as u32,
                            height: (bottom - top) as u32,
                        },
                    }],
                );
                if set != bound {
                    dev.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.layout,
                        0,
                        &[set],
                        &[offset],
                    );
                    bound = set;
                }
                dev.cmd_draw_indexed(cmd, count, 1, first, base, 0);
            }
            dev.cmd_end_rendering(cmd);
            self.buffers.push(vb);
            self.buffers.push(ib);
        }
        Ok(())
    }

    pub unsafe fn destroy(&mut self, d: &Device) {
        unsafe {
            for (_, t) in self.textures.drain() {
                d.free_image(t.image);
            }
            for i in self.retired.drain(..) {
                d.free_image(i);
            }
            for b in self.buffers.drain(..) {
                d.free_buffer(b);
            }
            let dev = &d.device;
            dev.destroy_pipeline(self.pipeline, None);
            dev.destroy_descriptor_pool(self.pool, None);
            dev.destroy_pipeline_layout(self.layout, None);
            dev.destroy_descriptor_set_layout(self.set_layout, None);
            dev.destroy_sampler(self.sampler, None);
        }
    }
}

/// The egui graphics pipeline for a colour attachment of `format`.
unsafe fn make_pipeline(
    d: &Device,
    layout: vk::PipelineLayout,
    format: vk::Format,
    vs: &[u8],
    ps: &[u8],
) -> Result<vk::Pipeline> {
    unsafe {
        let dev = &d.device;
        let module = |spv: &[u8]| -> Result<vk::ShaderModule> {
            let code = ash::util::read_spv(&mut std::io::Cursor::new(spv))?;
            Ok(dev.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?)
        };
        let (vsm, psm) = (module(vs)?, module(ps)?);
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vsm)
                .name(c"vs_main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(psm)
                .name(c"ps_main"),
        ];
        let vbind = [vk::VertexInputBindingDescription::default()
            .stride(size_of::<GuiVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let attr = |location, format, offset| {
            vk::VertexInputAttributeDescription::default()
                .location(location)
                .format(format)
                .offset(offset)
        };
        let attrs = [
            attr(0, vk::Format::R32G32_SFLOAT, 0),
            attr(1, vk::Format::R32G32_SFLOAT, 8),
            attr(2, vk::Format::R8G8B8A8_UNORM, 16),
        ];
        let vertex = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vbind)
            .vertex_attribute_descriptions(&attrs);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // egui colours are premultiplied (as D3D12's gui blend state).
        let blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
        let dynamic = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);
        let formats = [format];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let result = dev.create_graphics_pipelines(
            vk::PipelineCache::null(),
            &[vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex)
                .input_assembly_state(&assembly)
                .viewport_state(&viewport)
                .rasterization_state(&raster)
                .multisample_state(&multisample)
                .color_blend_state(&blend)
                .dynamic_state(&dynamic)
                .layout(layout)
                .push_next(&mut rendering)],
            None,
        );
        dev.destroy_shader_module(vsm, None);
        dev.destroy_shader_module(psm, None);
        Ok(result.map_err(|(_, e)| e)?[0])
    }
}
