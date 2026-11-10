//! The one pipeline layout every pass uses, Vulkan's root signature (docs/claude/data-layouts.md
//! "Root signature"; build.rs shifts t to 100+, b to 200+, s to 300+, and spaces are sets).
//! `tools/spirv_bindings.py` lists what the shaders declare; this must hold their union.
//!
//! - set 0: u0-u31 storage images (0-31), t0 the TLAS (100), t1 and t3-t17 storage buffers
//!   (101, 103-109; t18 the star table, 118), t2 the egui texture (102), b0 the frame constants (200, a dynamic uniform
//!   buffer: one `CB_STRIDE` slot per pass), s0 / s1 immutable samplers (300, 301).
//! - set 1: `maps[]`, up to `MAX_MAPS` sampled images (binding 100, variable count).
//! - set 2: space 2's raw buffers u0-u4 (0-4).
//!
//! Every binding is partially bound: a pass may leave what it doesn't read unwritten.
use super::Device;
use anyhow::Result;
use ash::vk;

pub const MAX_MAPS: u32 = 4096;
pub const SET0_BUFFERS: [u32; 17] = [
    101, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
];

/// Our wrapper around the driver's cache blob: magic, version, length and FNV-1a hash, so a
/// truncated or foreign file is dropped before the driver sees it.
const CACHE_MAGIC: &[u8; 4] = b"CVPC";
const CACHE_VERSION: u32 = 1;
const CACHE_HEADER: usize = 20;

/// One file per shader set (`key`, a hash of every SPIR-V module), so hot reloads and old
/// builds don't pile pipelines into one ever-growing blob. None on a `--thumbnails-out` run.
fn cache_path(key: u64) -> Option<std::path::PathBuf> {
    if crate::dirs::cache_overridden() {
        return None;
    }
    crate::dirs::cache().map(|c| c.join(format!("{CACHE_PREFIX}{key:016x}.bin")))
}
const CACHE_PREFIX: &str = "vulkan-pipelines-";
/// Files kept for other shader sets (another build run beside this one): the newest few.
const CACHE_KEEP: usize = 3;

/// Removes the oldest cache files beyond `CACHE_KEEP`, and temp files left by a crash.
fn prune_caches(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files = vec![];
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with(CACHE_PREFIX) && name != "vulkan-pipelines.bin" {
            continue;
        }
        let time = e.metadata().and_then(|m| m.modified()).ok();
        if name.ends_with(".bin") && name != "vulkan-pipelines.bin" {
            files.push((time, e.path()));
        } else if time
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|a| a.as_secs() > 3600)
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in files.into_iter().skip(CACHE_KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

/// The cache key for a set of SPIR-V modules.
pub fn shader_key<'a>(modules: impl IntoIterator<Item = &'a [u8]>) -> u64 {
    modules.into_iter().fold(0, |h, m| {
        fnv1a(m) ^ h.rotate_left(5).wrapping_mul(0x0100_0000_01b3)
    })
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

fn wrap(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CACHE_HEADER + data.len());
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&fnv1a(data).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// The driver's blob inside `file` if our wrapper is intact and the blob's own header
/// (VK_PIPELINE_CACHE_HEADER_VERSION_ONE) names this vendor, device and cache UUID: a new
/// driver or GPU changes the UUID, and the file is then ignored and rewritten.
fn unwrap<'a>(file: &'a [u8], vendor: u32, device: u32, uuid: &[u8; 16]) -> Option<&'a [u8]> {
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    if file.len() < CACHE_HEADER || &file[..4] != CACHE_MAGIC || u32_at(file, 4) != CACHE_VERSION {
        return None;
    }
    let data = &file[CACHE_HEADER..];
    let hash = u64::from_le_bytes(file[12..20].try_into().unwrap());
    if data.len() != u32_at(file, 8) as usize || fnv1a(data) != hash || data.len() < 32 {
        return None;
    }
    let ok = u32_at(data, 0) >= 32
        && u32_at(data, 4) == vk::PipelineCacheHeaderVersion::ONE.as_raw() as u32
        && u32_at(data, 8) == vendor
        && u32_at(data, 12) == device
        && data[16..32] == uuid[..];
    ok.then_some(data)
}

/// An empty cache when there is no file or it doesn't match; null if even that fails, which
/// only costs the compile time.
unsafe fn load_cache(d: &Device, key: u64) -> vk::PipelineCache {
    unsafe {
        let props = d.instance.get_physical_device_properties(d.phys);
        let file = cache_path(key)
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_default();
        let data = unwrap(
            &file,
            props.vendor_id,
            props.device_id,
            &props.pipeline_cache_uuid,
        );
        let info = vk::PipelineCacheCreateInfo::default().initial_data(data.unwrap_or(&[]));
        d.device
            .create_pipeline_cache(&info, None)
            .or_else(|_| {
                d.device
                    .create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None)
            })
            .unwrap_or_else(|e| {
                eprintln!("vulkan: pipeline cache: {e}");
                vk::PipelineCache::null()
            })
    }
}

pub struct Layout {
    pub sets: [vk::DescriptorSetLayout; 3],
    pub pipeline: vk::PipelineLayout,
    samplers: [vk::Sampler; 2],
    pool: vk::DescriptorPool,
    pub descriptors: [vk::DescriptorSet; 3],
    /// Compiled pipelines kept across runs (`load_cache` / `save_cache`), so a restart skips
    /// the 13-17 s compile even when the driver's own disk cache is full or off.
    cache: vk::PipelineCache,
    cache_key: u64,
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
            // The TLAS: a display-only device has no such descriptor type, and the shaders it
            // runs (display, filter, glare, UI) don't declare it.
            if d.ray_tracing {
                set0.push(b(100, vk::DescriptorType::ACCELERATION_STRUCTURE_KHR));
            }
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
            let mut sizes = vec![
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::STORAGE_IMAGE,
                    descriptor_count: 32,
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
            if d.ray_tracing {
                sizes.push(vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                    descriptor_count: 1,
                });
            }
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
                cache: vk::PipelineCache::null(),
                cache_key: 0,
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
                self.cache,
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

    /// Loads the cache for this shader set; call before building pipelines.
    pub unsafe fn open_cache(&mut self, d: &Device, key: u64) {
        unsafe {
            d.device.destroy_pipeline_cache(self.cache, None);
            self.cache = load_cache(d, key);
        }
        self.cache_key = key;
    }

    /// Writes the pipeline cache to disk after the pipelines are built (startup, shader reload).
    /// Failures only log: the next start compiles again.
    pub unsafe fn save_cache(&self, d: &Device) {
        if self.cache == vk::PipelineCache::null() {
            return;
        }
        let Some(path) = cache_path(self.cache_key) else {
            return;
        };
        let data = match unsafe { d.device.get_pipeline_cache_data(self.cache) } {
            Ok(data) => data,
            Err(e) => return eprintln!("vulkan: pipeline cache: {e}"),
        };
        // Per process: a thumbnail render can save beside the app.
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(path.parent().unwrap_or(&path))
            .and_then(|()| std::fs::write(&tmp, wrap(&data)))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("vulkan: pipeline cache {}: {e}", path.display());
        }
        if let Some(dir) = path.parent() {
            prune_caches(dir);
        }
    }

    pub unsafe fn destroy(&self, d: &Device) {
        unsafe {
            d.device.destroy_pipeline_cache(self.cache, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(vendor: u32, device: u32, uuid: [u8; 16]) -> Vec<u8> {
        let mut b = vec![];
        for v in [32u32, 1, vendor, device] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&uuid);
        b.extend_from_slice(b"driver data");
        b
    }

    #[test]
    fn a_saved_cache_reads_back_only_on_the_same_device_and_driver() {
        let data = blob(0x10de, 0x2705, [7; 16]);
        let file = wrap(&data);
        assert_eq!(unwrap(&file, 0x10de, 0x2705, &[7; 16]), Some(&data[..]));
        assert_eq!(unwrap(&file, 0x10de, 0x2705, &[8; 16]), None, "new driver");
        assert_eq!(
            unwrap(&file, 0x1002, 0x2705, &[7; 16]),
            None,
            "other vendor"
        );
        assert_eq!(unwrap(&file, 0x10de, 0x2704, &[7; 16]), None, "other GPU");
    }

    #[test]
    fn a_damaged_cache_file_is_ignored() {
        let file = wrap(&blob(1, 2, [3; 16]));
        assert_eq!(
            unwrap(&file[..file.len() - 1], 1, 2, &[3; 16]),
            None,
            "truncated"
        );
        let mut flipped = file.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert_eq!(unwrap(&flipped, 1, 2, &[3; 16]), None, "corrupt");
        assert_eq!(unwrap(&[], 1, 2, &[3; 16]), None, "empty");
        assert_eq!(
            unwrap(&wrap(b"short"), 1, 2, &[3; 16]),
            None,
            "no driver header"
        );
    }

    #[test]
    fn the_cache_key_follows_every_shader() {
        let key = shader_key([&b"trace"[..], b"display"]);
        assert_eq!(key, shader_key([&b"trace"[..], b"display"]));
        assert_ne!(key, shader_key([&b"trace"[..], b"displaz"]));
        assert_ne!(key, shader_key([&b"display"[..], b"trace"]));
    }

    #[test]
    fn pruning_keeps_the_newest_caches_and_nothing_else_is_touched() {
        let dir = std::env::temp_dir().join(format!("caustica-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        for i in 0..5u64 {
            let f = std::fs::File::create(dir.join(format!("{CACHE_PREFIX}{i:016x}.bin"))).unwrap();
            f.set_modified(old + std::time::Duration::from_secs(i))
                .unwrap();
        }
        let stale = std::fs::File::create(dir.join(format!("{CACHE_PREFIX}9.1.tmp"))).unwrap();
        stale.set_modified(old).unwrap();
        std::fs::write(dir.join("scene--0123456789abcdef.png"), b"").unwrap();
        prune_caches(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        std::fs::remove_dir_all(&dir).unwrap();
        let newest = |i: u64| format!("{CACHE_PREFIX}{i:016x}.bin");
        assert_eq!(
            left,
            [
                "scene--0123456789abcdef.png".to_string(),
                newest(2),
                newest(3),
                newest(4)
            ]
        );
    }
}
