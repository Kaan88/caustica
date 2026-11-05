//! The one pipeline layout every pass uses, Vulkan's root signature (docs/claude/data-layouts.md
//! "Root signature"; build.rs shifts t to 100+, b to 200+, s to 300+, and spaces are sets).
//! `tools/spirv_bindings.py` lists what the shaders declare; this must hold their union.
//!
//! - set 0: u0-u31 storage images (0-31), t0 the TLAS (100), t1 and t3-t17 storage buffers
//!   (101, 103-109), t2 the egui texture (102), b0 the frame constants (200, a dynamic uniform
//!   buffer: one `CB_STRIDE` slot per pass), s0 / s1 immutable samplers (300, 301).
//! - set 1: `maps[]`, up to `MAX_MAPS` sampled images (binding 100, variable count).
//! - set 2: space 2's raw buffers u0-u4 (0-4).
//!
//! Every binding is partially bound: a pass may leave what it doesn't read unwritten.
use super::Device;
use anyhow::Result;
use ash::vk;

pub const MAX_MAPS: u32 = 4096;
pub const SET0_BUFFERS: [u32; 16] = [
    101, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
];

pub struct Layout {
    pub sets: [vk::DescriptorSetLayout; 3],
    pub pipeline: vk::PipelineLayout,
    samplers: [vk::Sampler; 2],
    pool: vk::DescriptorPool,
    pub descriptors: [vk::DescriptorSet; 3],
}

impl Layout {
    pub unsafe fn new(d: &Device) -> Result<Self> {
        unsafe {
            let dev = &d.device;
            let sampler = |mode: vk::SamplerAddressMode, mips: bool| {
                dev.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                        .address_mode_u(mode)
                        .address_mode_v(mode)
                        .address_mode_w(mode)
                        .max_lod(if mips { vk::LOD_CLAMP_NONE } else { 0. })
                        // Texture maps filter anisotropically, as D3D12's map sampler
                        // (16x); the shader limits the ratio to Settings::anisotropy.
                        .anisotropy_enable(mips && d.anisotropy)
                        .max_anisotropy(if mips && d.anisotropy { 16. } else { 1. }),
                    None,
                )
            };
            let samplers = [
                sampler(vk::SamplerAddressMode::CLAMP_TO_EDGE, false)?,
                sampler(vk::SamplerAddressMode::REPEAT, true)?,
            ];
            let all = vk::ShaderStageFlags::ALL;
            let b = |binding: u32, ty: vk::DescriptorType| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(ty)
                    .descriptor_count(1)
                    .stage_flags(all)
            };
            let mut set0: Vec<_> = (0..32)
                .map(|i| b(i, vk::DescriptorType::STORAGE_IMAGE))
                .collect();
            set0.push(b(100, vk::DescriptorType::ACCELERATION_STRUCTURE_KHR));
            set0.extend(SET0_BUFFERS.map(|i| b(i, vk::DescriptorType::STORAGE_BUFFER)));
            set0.push(b(102, vk::DescriptorType::SAMPLED_IMAGE));
            set0.push(b(200, vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC));
            set0.push(b(300, vk::DescriptorType::SAMPLER).immutable_samplers(&samplers[..1]));
            set0.push(b(301, vk::DescriptorType::SAMPLER).immutable_samplers(&samplers[1..]));
            let set1 = [vk::DescriptorSetLayoutBinding::default()
                .binding(100)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(MAX_MAPS)
                .stage_flags(all)];
            let set2: Vec<_> = (0..5)
                .map(|i| b(i, vk::DescriptorType::STORAGE_BUFFER))
                .collect();
            let partial = vk::DescriptorBindingFlags::PARTIALLY_BOUND;
            let make = |bindings: &[vk::DescriptorSetLayoutBinding],
                        flags: &[vk::DescriptorBindingFlags]| {
                let mut f =
                    vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(flags);
                dev.create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default()
                        .bindings(bindings)
                        .push_next(&mut f),
                    None,
                )
            };
            let sets = [
                make(&set0, &vec![partial; set0.len()])?,
                make(
                    &set1,
                    &[partial | vk::DescriptorBindingFlags::VARIABLE_DESCRIPTOR_COUNT],
                )?,
                make(&set2, &[partial; 5])?,
            ];
            let pipeline = dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&sets),
                None,
            )?;
            let sizes = [
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::STORAGE_IMAGE,
                    descriptor_count: 32,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                    descriptor_count: 1,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::STORAGE_BUFFER,
                    descriptor_count: SET0_BUFFERS.len() as u32 + 5,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::SAMPLED_IMAGE,
                    descriptor_count: 1 + MAX_MAPS,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
                    descriptor_count: 1,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::SAMPLER,
                    descriptor_count: 2,
                },
            ];
            let pool = dev.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(3)
                    .pool_sizes(&sizes),
                None,
            )?;
            let counts = [1, MAX_MAPS, 1];
            let mut variable = vk::DescriptorSetVariableDescriptorCountAllocateInfo::default()
                .descriptor_counts(&counts);
            let v = dev.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&sets)
                    .push_next(&mut variable),
            )?;
            Ok(Self {
                sets,
                pipeline,
                samplers,
                pool,
                descriptors: [v[0], v[1], v[2]],
            })
        }
    }

    pub unsafe fn compute(
        &self,
        d: &Device,
        spv: &[u8],
        entry: &std::ffi::CStr,
        flags: vk::PipelineCreateFlags,
    ) -> Result<vk::Pipeline> {
        unsafe {
            let code = ash::util::read_spv(&mut std::io::Cursor::new(spv))?;
            let module = d
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?;
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(entry);
            let result = d.device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[vk::ComputePipelineCreateInfo::default()
                    .flags(flags)
                    .stage(stage)
                    .layout(self.pipeline)],
                None,
            );
            d.device.destroy_shader_module(module, None);
            Ok(result.map_err(|(_, e)| e)?[0])
        }
    }

    pub unsafe fn image(&self, d: &Device, binding: u32, view: vk::ImageView) {
        let info = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::GENERAL)];
        let ty = if binding < 32 {
            vk::DescriptorType::STORAGE_IMAGE
        } else {
            vk::DescriptorType::SAMPLED_IMAGE
        };
        let w = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptors[0])
            .dst_binding(binding)
            .descriptor_type(ty)
            .image_info(&info);
        unsafe { d.device.update_descriptor_sets(&[w], &[]) };
    }

    /// Texture map `slot` (set 1, `maps[slot]`).
    pub unsafe fn map(&self, d: &Device, slot: u32, view: vk::ImageView) {
        let info = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::GENERAL)];
        let w = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptors[1])
            .dst_binding(100)
            .dst_array_element(slot)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&info);
        unsafe { d.device.update_descriptor_sets(&[w], &[]) };
    }

    /// A storage buffer in set 0 (t registers) or set 2 (space 2 u registers).
    pub unsafe fn buffer(&self, d: &Device, set: usize, binding: u32, buffer: vk::Buffer) {
        let info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer)
            .range(vk::WHOLE_SIZE)];
        let w = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptors[set])
            .dst_binding(binding)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&info);
        unsafe { d.device.update_descriptor_sets(&[w], &[]) };
    }

    pub unsafe fn constants(&self, d: &Device, buffer: vk::Buffer, stride: u64) {
        let info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer)
            .range(stride)];
        let w = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptors[0])
            .dst_binding(200)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
            .buffer_info(&info);
        unsafe { d.device.update_descriptor_sets(&[w], &[]) };
    }

    pub unsafe fn accel(&self, d: &Device, accel: vk::AccelerationStructureKHR) {
        let handles = [accel];
        let mut a = vk::WriteDescriptorSetAccelerationStructureKHR::default()
            .acceleration_structures(&handles);
        let mut w = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptors[0])
            .dst_binding(100)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .push_next(&mut a);
        w.descriptor_count = 1;
        unsafe { d.device.update_descriptor_sets(&[w], &[]) };
    }

    /// Binds the three sets for compute with the constants at `offset`.
    pub unsafe fn bind(&self, d: &Device, cmd: vk::CommandBuffer, offset: u32) {
        unsafe {
            d.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline,
                0,
                &self.descriptors,
                &[offset],
            )
        };
    }

    pub unsafe fn destroy(&self, d: &Device) {
        unsafe {
            d.device.destroy_descriptor_pool(self.pool, None);
            d.device.destroy_pipeline_layout(self.pipeline, None);
            for s in self.sets {
                d.device.destroy_descriptor_set_layout(s, None);
            }
            for s in self.samplers {
                d.device.destroy_sampler(s, None);
            }
        }
    }
}
