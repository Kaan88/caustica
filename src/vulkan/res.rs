//! Vulkan resources: buffers (all with device addresses), images kept in GENERAL layout,
//! one-shot submits, and acceleration structures. One allocation per resource: the renderer
//! holds a few dozen.
use super::Device;
use anyhow::{Result, ensure};
use ash::{khr, vk};

pub struct Buffer {
    pub handle: vk::Buffer,
    memory: vk::DeviceMemory,
    pub size: u64,
    pub address: u64,
    /// Host-visible buffers stay mapped; null otherwise.
    pub mapped: *mut u8,
}

pub struct Image {
    pub handle: vk::Image,
    memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub size: [u32; 3],
}

pub struct Accel {
    pub handle: vk::AccelerationStructureKHR,
    pub buffer: Buffer,
    pub address: u64,
}

pub const COLOR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// Every write before it is visible to every later read or write (D3D12's `uav(None)` and
/// more): the renderer is one queue with one frame in flight, so a full barrier costs little.
pub unsafe fn full_barrier(d: &ash::Device, cmd: vk::CommandBuffer) {
    let b = [vk::MemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
        .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
        .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)];
    unsafe { d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().memory_barriers(&b)) };
}

impl Device {
    pub unsafe fn buffer(
        &self,
        size: u64,
        usage: vk::BufferUsageFlags,
        host: bool,
    ) -> Result<Buffer> {
        unsafe {
            let size = size.max(16);
            let usage = usage
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC;
            let handle = self.device.create_buffer(
                &vk::BufferCreateInfo::default().size(size).usage(usage),
                None,
            )?;
            let req = self.device.get_buffer_memory_requirements(handle);
            let flags = if host {
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
            } else {
                vk::MemoryPropertyFlags::DEVICE_LOCAL
            };
            let mut address_flag = vk::MemoryAllocateFlagsInfo::default()
                .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
            let memory = self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(self.memory_type(req.memory_type_bits, flags)?)
                    .push_next(&mut address_flag),
                None,
            )?;
            self.device.bind_buffer_memory(handle, memory, 0)?;
            let address = self
                .device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(handle));
            let mapped = if host {
                self.device
                    .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())?
                    as *mut u8
            } else {
                std::ptr::null_mut()
            };
            Ok(Buffer {
                handle,
                memory,
                size,
                address,
                mapped,
            })
        }
    }

    pub unsafe fn free_buffer(&self, b: Buffer) {
        unsafe {
            self.device.destroy_buffer(b.handle, None);
            self.device.free_memory(b.memory, None);
        }
    }

    /// Writes `bytes` at `offset` into a host-visible buffer.
    pub unsafe fn write(&self, b: &Buffer, offset: u64, bytes: &[u8]) -> Result<()> {
        ensure!(
            !b.mapped.is_null() && offset + bytes.len() as u64 <= b.size,
            "write past a buffer"
        );
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                b.mapped.add(offset as usize),
                bytes.len(),
            )
        };
        Ok(())
    }

    /// A device-local buffer holding `bytes`, copied through a staging buffer (waits).
    pub unsafe fn upload(&self, bytes: &[u8], usage: vk::BufferUsageFlags) -> Result<Buffer> {
        unsafe {
            let dst = self.buffer(bytes.len() as u64, usage, false)?;
            if bytes.is_empty() {
                return Ok(dst);
            }
            let staging =
                self.buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
            self.write(&staging, 0, bytes)?;
            let region = [vk::BufferCopy::default().size(bytes.len() as u64)];
            self.one_shot(|cmd| {
                self.device
                    .cmd_copy_buffer(cmd, staging.handle, dst.handle, &region)
            })?;
            self.free_buffer(staging);
            Ok(dst)
        }
    }

    /// Records `record` into a fresh command buffer, submits it and waits.
    pub unsafe fn one_shot(&self, record: impl FnOnce(vk::CommandBuffer)) -> Result<()> {
        unsafe {
            let pool = self.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(self.family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )?;
            let cmd = self.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(1),
            )?[0];
            self.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            record(cmd);
            full_barrier(&self.device, cmd);
            self.device.end_command_buffer(cmd)?;
            let fence = self
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            let result = self
                .device
                .queue_submit2(
                    self.queue,
                    &[vk::SubmitInfo2::default().command_buffer_infos(&cmds)],
                    fence,
                )
                .and_then(|_| self.device.wait_for_fences(&[fence], true, u64::MAX));
            self.device.destroy_fence(fence, None);
            self.device.destroy_command_pool(pool, None);
            result?;
            Ok(())
        }
    }

    /// A 2D (or 3D when size[2] > 1) image in GENERAL layout with a view.
    pub unsafe fn image(
        &self,
        size: [u32; 3],
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Image> {
        unsafe {
            let three = size[2] > 1;
            let handle = self.device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(if three {
                        vk::ImageType::TYPE_3D
                    } else {
                        vk::ImageType::TYPE_2D
                    })
                    .format(format)
                    .extent(vk::Extent3D {
                        width: size[0],
                        height: size[1],
                        depth: size[2],
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(
                        usage
                            | vk::ImageUsageFlags::TRANSFER_SRC
                            | vk::ImageUsageFlags::TRANSFER_DST,
                    ),
                None,
            )?;
            let req = self.device.get_image_memory_requirements(handle);
            let memory = self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(self.memory_type(
                        req.memory_type_bits,
                        vk::MemoryPropertyFlags::DEVICE_LOCAL,
                    )?),
                None,
            )?;
            self.device.bind_image_memory(handle, memory, 0)?;
            let view = self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(handle)
                    .view_type(if three {
                        vk::ImageViewType::TYPE_3D
                    } else {
                        vk::ImageViewType::TYPE_2D
                    })
                    .format(format)
                    .subresource_range(COLOR),
                None,
            )?;
            let to_general = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .image(handle)
                .subresource_range(COLOR)];
            self.one_shot(|cmd| {
                self.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_general),
                );
                // Start from zero, as D3D12's committed textures do.
                self.device.cmd_clear_color_image(
                    cmd,
                    handle,
                    vk::ImageLayout::GENERAL,
                    &vk::ClearColorValue { uint32: [0; 4] },
                    &[COLOR],
                );
            })?;
            Ok(Image {
                handle,
                memory,
                view,
                format,
                size,
            })
        }
    }

    pub unsafe fn free_image(&self, i: Image) {
        unsafe {
            self.device.destroy_image_view(i.view, None);
            self.device.destroy_image(i.handle, None);
            self.device.free_memory(i.memory, None);
        }
    }

    /// Builds a BLAS or TLAS from `geometries` (one primitive count each), PREFER_FAST_TRACE.
    pub unsafe fn build_accel(
        &self,
        fns: &khr::acceleration_structure::Device,
        ty: vk::AccelerationStructureTypeKHR,
        geometries: &[vk::AccelerationStructureGeometryKHR<'_>],
        counts: &[u32],
    ) -> Result<Accel> {
        unsafe {
            let mut info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(ty)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .geometries(geometries);
            let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
            fns.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                counts,
                &mut sizes,
            );
            let buffer = self.buffer(
                sizes.acceleration_structure_size,
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
                false,
            )?;
            let handle = fns.create_acceleration_structure(
                &vk::AccelerationStructureCreateInfoKHR::default()
                    .buffer(buffer.handle)
                    .size(sizes.acceleration_structure_size)
                    .ty(ty),
                None,
            )?;
            // Scratch addresses must be aligned (256 covers every driver's minimum).
            let scratch = self.buffer(
                sizes.build_scratch_size + 256,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                false,
            )?;
            info =
                info.dst_acceleration_structure(handle)
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: scratch.address.next_multiple_of(256),
                    });
            let ranges: Vec<vk::AccelerationStructureBuildRangeInfoKHR> = counts
                .iter()
                .map(|&n| vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(n))
                .collect();
            self.one_shot(|cmd| fns.cmd_build_acceleration_structures(cmd, &[info], &[&ranges]))?;
            self.free_buffer(scratch);
            let address = fns.get_acceleration_structure_device_address(
                &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                    .acceleration_structure(handle),
            );
            Ok(Accel {
                handle,
                buffer,
                address,
            })
        }
    }

    pub unsafe fn free_accel(&self, fns: &khr::acceleration_structure::Device, a: Accel) {
        unsafe {
            fns.destroy_acceleration_structure(a.handle, None);
            self.free_buffer(a.buffer);
        }
    }
}

/// A triangle geometry of `count` triangles at `address` (`Vertex` stride, position first).
pub fn triangles<'a>(
    address: u64,
    count: u32,
    opaque: bool,
) -> vk::AccelerationStructureGeometryKHR<'a> {
    vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
        .flags(if opaque {
            vk::GeometryFlagsKHR::OPAQUE
        } else {
            vk::GeometryFlagsKHR::empty()
        })
        .geometry(vk::AccelerationStructureGeometryDataKHR {
            triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: address,
                })
                .vertex_stride(size_of::<crate::scene::Vertex>() as u64)
                .max_vertex((count * 3).saturating_sub(1))
                .index_type(vk::IndexType::NONE_KHR),
        })
}
