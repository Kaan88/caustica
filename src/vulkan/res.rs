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

/// The TLAS with what a refit reuses: its scratch (sized for build and update), the instance
/// buffer (host-visible) and the instance tails it was built over.
pub struct Tlas {
    pub accel: Option<Accel>,
    scratch: Option<Buffer>,
    instances: Buffer,
    tails: Vec<u8>,
    pub refits: u32,
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
            // Addresses feed only acceleration-structure builds, so a display-only device
            // (no bufferDeviceAddress) makes buffers without them.
            let usage =
                usage | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::TRANSFER_SRC;
            let usage = if self.ray_tracing {
                usage | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            } else {
                usage
            };
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
            let mut info = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(self.memory_type(req.memory_type_bits, flags)?);
            if self.ray_tracing {
                info = info.push_next(&mut address_flag);
            }
            let memory = self.device.allocate_memory(&info, None)?;
            self.device.bind_buffer_memory(handle, memory, 0)?;
            let address = if self.ray_tracing {
                self.device.get_buffer_device_address(
                    &vk::BufferDeviceAddressInfo::default().buffer(handle),
                )
            } else {
                0
            };
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
        unsafe { self.image_levels(size, format, usage, 1) }
    }

    /// `image` with `levels` mip levels (texture maps).
    pub unsafe fn image_levels(
        &self,
        size: [u32; 3],
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        levels: u32,
    ) -> Result<Image> {
        let range = vk::ImageSubresourceRange {
            level_count: levels,
            ..COLOR
        };
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
                    .mip_levels(levels)
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
                    .subresource_range(range),
                None,
            )?;
            let to_general = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .image(handle)
                .subresource_range(range)];
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
                    &[range],
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

    /// Copies tightly packed texels into mip `level` of `img` (GENERAL layout), through a
    /// staging buffer (waits).
    pub unsafe fn fill_image(
        &self,
        img: &Image,
        level: u32,
        size: [u32; 3],
        data: &[u8],
    ) -> Result<()> {
        unsafe {
            let staging =
                self.buffer(data.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
            self.write(&staging, 0, data)?;
            let region = vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: level,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D {
                    width: size[0],
                    height: size[1],
                    depth: size[2],
                });
            let result = self.one_shot(|cmd| {
                self.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.handle,
                    img.handle,
                    vk::ImageLayout::GENERAL,
                    &[region],
                )
            });
            self.free_buffer(staging);
            result
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
        compact: bool,
    ) -> Result<Accel> {
        unsafe {
            type F = vk::BuildAccelerationStructureFlagsKHR;
            let mut info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(ty)
                .flags(if compact {
                    F::PREFER_FAST_TRACE | F::ALLOW_COMPACTION
                } else {
                    F::PREFER_FAST_TRACE
                })
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

    /// BLAS compaction, as D3D12's `compact_blases`: the compacted sizes of `accels` (built
    /// with `compact`) in one query, then each copied into a structure of that size. Returns
    /// them in order; the originals are freed.
    pub unsafe fn compact_accels(
        &self,
        fns: &khr::acceleration_structure::Device,
        accels: Vec<Accel>,
    ) -> Result<Vec<Accel>> {
        if accels.is_empty() {
            return Ok(accels);
        }
        unsafe {
            let n = accels.len() as u32;
            let pool = self.device.create_query_pool(
                &vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR)
                    .query_count(n),
                None,
            )?;
            let handles: Vec<_> = accels.iter().map(|a| a.handle).collect();
            let built = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
                .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)];
            let queried = self.one_shot(|cmd| {
                self.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&built),
                );
                self.device.cmd_reset_query_pool(cmd, pool, 0, n);
                fns.cmd_write_acceleration_structures_properties(
                    cmd,
                    &handles,
                    vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR,
                    pool,
                    0,
                );
            });
            let mut sizes = vec![0u64; n as usize];
            let read = queried.and_then(|_| {
                Ok(self.device.get_query_pool_results(
                    pool,
                    0,
                    &mut sizes,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )?)
            });
            self.device.destroy_query_pool(pool, None);
            read?;
            let mut out = Vec::with_capacity(accels.len());
            for &size in &sizes {
                let buffer = self.buffer(
                    size,
                    vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
                    false,
                )?;
                let handle = fns.create_acceleration_structure(
                    &vk::AccelerationStructureCreateInfoKHR::default()
                        .buffer(buffer.handle)
                        .size(size)
                        .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL),
                    None,
                )?;
                out.push((handle, buffer));
            }
            self.one_shot(|cmd| {
                self.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&built),
                );
                for (a, (handle, _)) in accels.iter().zip(&out) {
                    fns.cmd_copy_acceleration_structure(
                        cmd,
                        &vk::CopyAccelerationStructureInfoKHR::default()
                            .src(a.handle)
                            .dst(*handle)
                            .mode(vk::CopyAccelerationStructureModeKHR::COMPACT),
                    );
                }
            })?;
            for a in accels {
                self.free_accel(fns, a);
            }
            Ok(out
                .into_iter()
                .map(|(handle, buffer)| Accel {
                    address: fns.get_acceleration_structure_device_address(
                        &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                            .acceleration_structure(handle),
                    ),
                    handle,
                    buffer,
                })
                .collect())
        }
    }

    /// The TLAS over `descs` (`VkAccelerationStructureInstanceKHR`s), as D3D12's: built with
    /// ALLOW_UPDATE and refit in place when only the transforms changed since `prev` (the same
    /// ids, masks, flags and BLASes: bytes 48–64 of each), with a full build every 64 refits so
    /// the tree doesn't degrade. One frame in flight makes rewriting the instances safe.
    pub unsafe fn build_tlas(
        &self,
        fns: &khr::acceleration_structure::Device,
        prev: Option<Tlas>,
        descs: &[u8],
    ) -> Result<Tlas> {
        unsafe {
            let tails: Vec<u8> = descs
                .chunks(64)
                .flat_map(|d| d[48..64].iter().copied())
                .collect();
            let count = (descs.len() / 64) as u32;
            let refit = prev
                .as_ref()
                .is_some_and(|p| p.tails == tails && p.refits < 64);
            let mut t = match prev {
                Some(p) if refit => p,
                other => {
                    if let Some(p) = other {
                        self.free_tlas(fns, p);
                    }
                    let instances = self.buffer(
                        descs.len() as u64,
                        vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                        true,
                    )?;
                    Tlas {
                        accel: None,
                        scratch: None,
                        instances,
                        tails,
                        refits: 0,
                    }
                }
            };
            self.write(&t.instances, 0, descs)?;
            let geometry = [vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    instances: vk::AccelerationStructureGeometryInstancesDataKHR::default().data(
                        vk::DeviceOrHostAddressConstKHR {
                            device_address: t.instances.address,
                        },
                    ),
                })];
            type F = vk::BuildAccelerationStructureFlagsKHR;
            let mut info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                .flags(F::PREFER_FAST_TRACE | F::ALLOW_UPDATE)
                .geometries(&geometry);
            if refit {
                let handle = t.accel.as_ref().unwrap().handle;
                info = info
                    .mode(vk::BuildAccelerationStructureModeKHR::UPDATE)
                    .src_acceleration_structure(handle)
                    .dst_acceleration_structure(handle);
                t.refits += 1;
            } else {
                let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
                fns.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &info,
                    &[count],
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
                        .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL),
                    None,
                )?;
                let address = fns.get_acceleration_structure_device_address(
                    &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                        .acceleration_structure(handle),
                );
                t.accel = Some(Accel {
                    handle,
                    buffer,
                    address,
                });
                t.scratch = Some(self.buffer(
                    sizes.build_scratch_size.max(sizes.update_scratch_size) + 256,
                    vk::BufferUsageFlags::STORAGE_BUFFER,
                    false,
                )?);
                info = info
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .dst_acceleration_structure(handle);
            }
            info = info.scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: t.scratch.as_ref().unwrap().address.next_multiple_of(256),
            });
            let ranges =
                [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(count)];
            // The BLASes were built in earlier submits; make them visible to this build.
            let built = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
                .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)];
            self.one_shot(|cmd| {
                self.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&built),
                );
                fns.cmd_build_acceleration_structures(cmd, &[info], &[&ranges]);
            })?;
            Ok(t)
        }
    }

    pub unsafe fn free_tlas(&self, fns: &khr::acceleration_structure::Device, t: Tlas) {
        unsafe {
            if let Some(a) = t.accel {
                self.free_accel(fns, a);
            }
            if let Some(b) = t.scratch {
                self.free_buffer(b);
            }
            self.free_buffer(t.instances);
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
