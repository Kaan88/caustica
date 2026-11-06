//! Vulkan backend (port in progress, see engine-notes "Vulkan backend: design").
//!
//! Phase 2: instance, a device with ray query and acceleration structures, a swap chain, and a
//! clear + present loop. `caustica --vulkan-smoke [--headless] [--frames N] [--debug]
//! [--no-vsync]` runs it on its own (`--vulkan` selects the renderer, `gpu`). Headless clears an
//! offscreen image and reads it back, so it also runs on a GPU-less machine (lavapipe).
use anyhow::{Context, Result, anyhow, bail, ensure};
use ash::{ext, khr, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::time::Instant;

pub mod gpu;
mod gui;
pub mod layout;
mod progress;
pub mod res;

static MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "system" fn on_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    let text = unsafe { (*data).message_as_c_str() }
        .map(|m| m.to_string_lossy().into_owned())
        .unwrap_or_default();
    // NGX loads its networks through NVIDIA's private CuModule chain, which the layer can't know.
    let vendor = text.contains("VUID-VkCuModuleCreateInfoNVX-pNext-pNext");
    if !vendor
        && severity.intersects(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
    {
        eprintln!("vulkan: {text}");
        if let Ok(mut m) = MESSAGES.lock() {
            m.push(text);
        }
    }
    vk::FALSE
}

/// Validation warnings and errors since the last call (empty without `--debug`).
pub fn messages() -> Vec<String> {
    MESSAGES
        .lock()
        .map(|mut m| std::mem::take(&mut *m))
        .unwrap_or_default()
}

/// Instance, the chosen physical device and a logical device with one graphics + compute queue.
pub struct Device {
    #[allow(dead_code)] // keeps the loader library loaded
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    debug: Option<(ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub surface_fn: khr::surface::Instance,
    pub phys: vk::PhysicalDevice,
    pub device: ash::Device,
    pub family: u32,
    pub queue: vk::Queue,
    pub name: String,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    /// samplerAnisotropy is on (the texture-map sampler filters anisotropically).
    pub anisotropy: bool,
    /// `VK_EXT_hdr_metadata` is enabled.
    pub hdr_metadata: bool,
}

const DEVICE_EXTENSIONS: [&CStr; 3] = [
    khr::acceleration_structure::NAME,
    khr::ray_query::NAME,
    khr::deferred_host_operations::NAME,
];

impl Device {
    /// With `window`, also creates its surface and picks a queue that can present to it.
    pub unsafe fn new(
        window: Option<(RawDisplayHandle, RawWindowHandle)>,
        debug: bool,
    ) -> Result<(Self, Option<vk::SurfaceKHR>)> {
        unsafe {
            let entry = ash::Entry::load().context("Vulkan loader not found")?;
            let app = vk::ApplicationInfo::default()
                .application_name(c"Caustica")
                .engine_name(c"Caustica")
                .api_version(vk::API_VERSION_1_3);
            let mut extensions: Vec<*const c_char> = match window {
                Some((display, _)) => ash_window::enumerate_required_extensions(display)?.to_vec(),
                None => vec![khr::surface::NAME.as_ptr()],
            };
            // HDR swap chains (scRGB, HDR10) need the extra colour spaces, where offered.
            if window.is_some()
                && entry
                    .enumerate_instance_extension_properties(None)?
                    .iter()
                    .any(|e| e.extension_name_as_c_str() == Ok(ext::swapchain_colorspace::NAME))
            {
                extensions.push(ext::swapchain_colorspace::NAME.as_ptr());
            }
            // What NGX and XeSS need, where the loader offers it (they report unavailable otherwise).
            let offered = entry.enumerate_instance_extension_properties(None)?;
            for e in gpu::upscale::required_extensions(
                false,
                vk::Instance::null(),
                vk::PhysicalDevice::null(),
            ) {
                let known = extensions.iter().any(|&p| CStr::from_ptr(p) == e);
                if !known && offered.iter().any(|o| o.extension_name_as_c_str() == Ok(e)) {
                    extensions.push(e.as_ptr());
                }
            }
            let validation = c"VK_LAYER_KHRONOS_validation";
            let mut layers = Vec::new();
            if debug {
                let have = entry
                    .enumerate_instance_layer_properties()?
                    .iter()
                    .any(|l| l.layer_name_as_c_str() == Ok(validation));
                if have {
                    layers.push(validation.as_ptr());
                } else {
                    eprintln!("vulkan: validation layer not installed, running without it");
                }
                extensions.push(ext::debug_utils::NAME.as_ptr());
            }
            let instance = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&app)
                        .enabled_layer_names(&layers)
                        .enabled_extension_names(&extensions),
                    None,
                )
                .context("vkCreateInstance")?;
            let debug = if debug {
                let fns = ext::debug_utils::Instance::new(&entry, &instance);
                type S = vk::DebugUtilsMessageSeverityFlagsEXT;
                type T = vk::DebugUtilsMessageTypeFlagsEXT;
                let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                    .message_severity(S::WARNING | S::ERROR)
                    .message_type(T::GENERAL | T::VALIDATION | T::PERFORMANCE)
                    .pfn_user_callback(Some(on_message));
                let messenger = fns.create_debug_utils_messenger(&info, None)?;
                Some((fns, messenger))
            } else {
                None
            };
            let surface_fn = khr::surface::Instance::new(&entry, &instance);
            let surface = match window {
                Some((display, handle)) => Some(ash_window::create_surface(
                    &entry, &instance, display, handle, None,
                )?),
                None => None,
            };
            // On failure the surface, messenger and instance go too: Menu → Renderer falls back
            // to D3D12 on the same window, which a live surface would keep bound.
            let made = (|| -> Result<_> {
                let (phys, family, name) = pick(&instance, &surface_fn, surface)?;

                let mut extensions: Vec<*const c_char> =
                    DEVICE_EXTENSIONS.iter().map(|e| e.as_ptr()).collect();
                if surface.is_some() {
                    extensions.push(khr::swapchain::NAME.as_ptr());
                }
                let offered = instance.enumerate_device_extension_properties(phys)?;
                // Tells a compositor the HDR output's range, so it needn't assume 10000 nits.
                let hdr_metadata = surface.is_some()
                    && offered
                        .iter()
                        .any(|o| o.extension_name_as_c_str() == Ok(ext::hdr_metadata::NAME));
                if hdr_metadata {
                    extensions.push(ext::hdr_metadata::NAME.as_ptr());
                }
                for e in gpu::upscale::required_extensions(true, instance.handle(), phys) {
                    let known = extensions.iter().any(|&p| CStr::from_ptr(p) == e)
                        // NGX lists it, but it may not be enabled beside the core feature below.
                        || e == vk::EXT_BUFFER_DEVICE_ADDRESS_NAME;
                    if !known && offered.iter().any(|o| o.extension_name_as_c_str() == Ok(e)) {
                        extensions.push(e.as_ptr());
                    }
                }
                // Small types the upscalers' shaders use, where the device has them.
                let mut has11 = vk::PhysicalDeviceVulkan11Features::default();
                let mut has12 = vk::PhysicalDeviceVulkan12Features::default();
                let mut has = vk::PhysicalDeviceFeatures2::default()
                    .push_next(&mut has11)
                    .push_next(&mut has12);
                instance.get_physical_device_features2(phys, &mut has);
                let has_int16 = has.features.shader_int16;
                let anisotropy = has.features.sampler_anisotropy != 0;
                // The egui pass's fragment shader writes the object-id image (u19).
                ensure!(
                    has.features.fragment_stores_and_atomics != 0,
                    "the GPU lacks fragmentStoresAndAtomics (the UI pass writes object ids)"
                );
                let mut f11 = vk::PhysicalDeviceVulkan11Features::default()
                    .storage_buffer16_bit_access(has11.storage_buffer16_bit_access != 0);
                let mut f12 = vk::PhysicalDeviceVulkan12Features::default()
                    .buffer_device_address(true)
                    .descriptor_indexing(true)
                    .runtime_descriptor_array(true)
                    .shader_sampled_image_array_non_uniform_indexing(true)
                    .descriptor_binding_partially_bound(true)
                    .descriptor_binding_variable_descriptor_count(true)
                    .scalar_block_layout(true)
                    .timeline_semaphore(true)
                    .shader_float16(has12.shader_float16 != 0)
                    .shader_int8(has12.shader_int8 != 0)
                    .storage_buffer8_bit_access(has12.storage_buffer8_bit_access != 0);
                let mut f13 = vk::PhysicalDeviceVulkan13Features::default()
                    .synchronization2(true)
                    .dynamic_rendering(true)
                    .maintenance4(true);
                let mut fas = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
                    .acceleration_structure(true);
                let mut frq = vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
                let core = vk::PhysicalDeviceFeatures::default()
                    .shader_int64(true)
                    .shader_int16(has_int16 != 0)
                    .sampler_anisotropy(anisotropy)
                    .fragment_stores_and_atomics(true)
                    .shader_storage_image_extended_formats(true)
                    .shader_storage_image_read_without_format(true)
                    .shader_storage_image_write_without_format(true);
                let priority = [1.0];
                let queues = [vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(family)
                    .queue_priorities(&priority)];
                let device = instance
                    .create_device(
                        phys,
                        &vk::DeviceCreateInfo::default()
                            .queue_create_infos(&queues)
                            .enabled_extension_names(&extensions)
                            .enabled_features(&core)
                            .push_next(&mut f11)
                            .push_next(&mut f12)
                            .push_next(&mut f13)
                            .push_next(&mut fas)
                            .push_next(&mut frq),
                        None,
                    )
                    .context("vkCreateDevice")?;
                Ok((phys, family, name, device, anisotropy, hdr_metadata))
            })();
            let (phys, family, name, device, anisotropy, hdr_metadata) = match made {
                Ok(m) => m,
                Err(e) => {
                    if let Some(s) = surface {
                        surface_fn.destroy_surface(s, None);
                    }
                    if let Some((fns, messenger)) = &debug {
                        fns.destroy_debug_utils_messenger(*messenger, None);
                    }
                    instance.destroy_instance(None);
                    return Err(e);
                }
            };
            let queue = device.get_device_queue(family, 0);
            let memory = instance.get_physical_device_memory_properties(phys);
            Ok((
                Self {
                    entry,
                    instance,
                    debug,
                    surface_fn,
                    phys,
                    device,
                    family,
                    queue,
                    name,
                    memory,
                    anisotropy,
                    hdr_metadata,
                },
                surface,
            ))
        }
    }

    pub fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        (0..self.memory.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0
                    && self.memory.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| anyhow!("no memory type {flags:?} in {bits:#b}"))
    }

    unsafe fn alloc(
        &self,
        req: vk::MemoryRequirements,
        flags: vk::MemoryPropertyFlags,
    ) -> Result<vk::DeviceMemory> {
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(self.memory_type(req.memory_type_bits, flags)?);
        Ok(unsafe { self.device.allocate_memory(&info, None)? })
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            if let Some((fns, messenger)) = self.debug.take() {
                fns.destroy_debug_utils_messenger(messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// The best device with everything the renderer needs: discrete over integrated over the rest
/// (lavapipe), and a graphics + compute queue that presents to `surface`.
unsafe fn pick(
    instance: &ash::Instance,
    surface_fn: &khr::surface::Instance,
    surface: Option<vk::SurfaceKHR>,
) -> Result<(vk::PhysicalDevice, u32, String)> {
    let mut best: Option<(u32, vk::PhysicalDevice, u32, String)> = None;
    let mut rejected = Vec::new();
    for phys in unsafe { instance.enumerate_physical_devices()? } {
        let props = unsafe { instance.get_physical_device_properties(phys) };
        let name = props
            .device_name_as_c_str()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match unsafe { suitable(instance, surface_fn, surface, phys, &props) } {
            Ok(family) => {
                let score = match props.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                    _ => 1,
                };
                if best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, phys, family, name));
                }
            }
            Err(why) => rejected.push(format!("{name}: {why}")),
        }
    }
    best.map(|(_, p, f, n)| (p, f, n))
        .ok_or_else(|| anyhow!("no Vulkan device with ray query: {}", rejected.join("; ")))
}

unsafe fn suitable(
    instance: &ash::Instance,
    surface_fn: &khr::surface::Instance,
    surface: Option<vk::SurfaceKHR>,
    phys: vk::PhysicalDevice,
    props: &vk::PhysicalDeviceProperties,
) -> Result<u32> {
    unsafe {
        if props.api_version < vk::API_VERSION_1_3 {
            bail!("Vulkan 1.3 needed");
        }
        let have = instance.enumerate_device_extension_properties(phys)?;
        for e in DEVICE_EXTENSIONS
            .iter()
            .chain(surface.map(|_| &khr::swapchain::NAME))
        {
            if !have.iter().any(|h| h.extension_name_as_c_str() == Ok(*e)) {
                bail!("{} missing", e.to_string_lossy());
            }
        }
        let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut fas = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        let mut frq = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        {
            let mut f2 = vk::PhysicalDeviceFeatures2::default()
                .push_next(&mut f12)
                .push_next(&mut f13)
                .push_next(&mut fas)
                .push_next(&mut frq);
            instance.get_physical_device_features2(phys, &mut f2);
        }
        let core = instance.get_physical_device_features(phys);
        let missing: Vec<&str> = [
            (core.shader_int64, "shaderInt64"),
            (
                core.shader_storage_image_extended_formats,
                "storageImageExtendedFormats",
            ),
            (
                core.shader_storage_image_read_without_format,
                "storageImageReadWithoutFormat",
            ),
            (
                core.shader_storage_image_write_without_format,
                "storageImageWriteWithoutFormat",
            ),
            (f12.buffer_device_address, "bufferDeviceAddress"),
            (f12.descriptor_indexing, "descriptorIndexing"),
            (f12.runtime_descriptor_array, "runtimeDescriptorArray"),
            (
                f12.shader_sampled_image_array_non_uniform_indexing,
                "nonUniformIndexing",
            ),
            (f12.descriptor_binding_partially_bound, "partiallyBound"),
            (
                f12.descriptor_binding_variable_descriptor_count,
                "variableDescriptorCount",
            ),
            (f12.scalar_block_layout, "scalarBlockLayout"),
            (f12.timeline_semaphore, "timelineSemaphore"),
            (f13.synchronization2, "synchronization2"),
            (f13.dynamic_rendering, "dynamicRendering"),
            (f13.maintenance4, "maintenance4"),
            (fas.acceleration_structure, "accelerationStructure"),
            (frq.ray_query, "rayQuery"),
        ]
        .into_iter()
        .filter(|(on, _)| *on == vk::FALSE)
        .map(|(_, n)| n)
        .collect();
        if !missing.is_empty() {
            bail!("features missing: {}", missing.join(", "));
        }
        let families = instance.get_physical_device_queue_family_properties(phys);
        let want = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE;
        for (i, f) in families.iter().enumerate() {
            let presents = match surface {
                Some(s) => surface_fn.get_physical_device_surface_support(phys, i as u32, s)?,
                None => true,
            };
            if f.queue_flags.contains(want) && presents {
                return Ok(i as u32);
            }
        }
        bail!("no graphics + compute queue that presents")
    }
}

pub struct Swapchain {
    fns: khr::swapchain::Device,
    pub surface: vk::SurfaceKHR,
    pub handle: vk::SwapchainKHR,
    pub images: Vec<vk::Image>,
    /// One view per image, for the egui pass's colour attachment.
    pub views: Vec<vk::ImageView>,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    /// One per image: signalled by the submit that renders it, waited on by its present.
    release: Vec<vk::Semaphore>,
    vsync: bool,
    /// The wanted output: 0 SDR, 1 scRGB (RGBA16F, extended linear sRGB), 2 HDR10 (PQ).
    pub hdr: u8,
    /// Whether the surface offered the wanted mode's format and colour space (else SDR).
    pub hdr_ok: bool,
    /// The peak last sent through `VK_EXT_hdr_metadata`; 0 after a rebuild.
    metadata_peak: f32,
}

impl Swapchain {
    pub unsafe fn new(
        d: &Device,
        surface: vk::SurfaceKHR,
        size: [u32; 2],
        vsync: bool,
    ) -> Result<Self> {
        let mut s = Self {
            fns: khr::swapchain::Device::new(&d.instance, &d.device),
            surface,
            handle: vk::SwapchainKHR::null(),
            images: Vec::new(),
            views: Vec::new(),
            extent: vk::Extent2D::default(),
            format: vk::Format::UNDEFINED,
            release: Vec::new(),
            vsync,
            hdr: 0,
            hdr_ok: true,
            metadata_peak: 0.,
        };
        unsafe { s.recreate(d, size)? };
        Ok(s)
    }

    /// Describes the HDR output to the compositor when its peak (in signal nits) changes.
    pub unsafe fn set_hdr_metadata(&mut self, d: &Device, peak: f32, hdr10: bool) {
        if !d.hdr_metadata || self.hdr == 0 || self.metadata_peak == peak {
            return;
        }
        self.metadata_peak = peak;
        let xy = |x, y| vk::XYColorEXT { x, y };
        let [r, g, b] = if hdr10 {
            [xy(0.708, 0.292), xy(0.170, 0.797), xy(0.131, 0.046)]
        } else {
            [xy(0.640, 0.330), xy(0.300, 0.600), xy(0.150, 0.060)]
        };
        let metadata = vk::HdrMetadataEXT::default()
            .display_primary_red(r)
            .display_primary_green(g)
            .display_primary_blue(b)
            .white_point(xy(0.3127, 0.3290))
            .max_luminance(peak)
            .max_content_light_level(peak);
        unsafe {
            ext::hdr_metadata::Device::new(&d.instance, &d.device)
                .set_hdr_metadata(&[self.handle], &[metadata]);
        }
    }

    pub unsafe fn recreate(&mut self, d: &Device, size: [u32; 2]) -> Result<()> {
        unsafe {
            d.device.device_wait_idle()?;
            let caps = d
                .surface_fn
                .get_physical_device_surface_capabilities(d.phys, self.surface)?;
            let formats = d
                .surface_fn
                .get_physical_device_surface_formats(d.phys, self.surface)?;
            let modes = d
                .surface_fn
                .get_physical_device_surface_present_modes(d.phys, self.surface)?;
            let wanted = match self.hdr {
                1 => Some((
                    vk::Format::R16G16B16A16_SFLOAT,
                    vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT,
                )),
                2 => Some((
                    vk::Format::A2B10G10R10_UNORM_PACK32,
                    vk::ColorSpaceKHR::HDR10_ST2084_EXT,
                )),
                _ => None,
            };
            let hdr = wanted
                .and_then(|(f, c)| formats.iter().find(|s| s.format == f && s.color_space == c));
            self.hdr_ok = self.hdr == 0 || hdr.is_some();
            let format = hdr
                .or_else(|| {
                    [vk::Format::B8G8R8A8_UNORM, vk::Format::R8G8B8A8_UNORM]
                        .into_iter()
                        .find_map(|f| {
                            formats.iter().find(|s| {
                                s.format == f && s.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                            })
                        })
                })
                .or(formats.first())
                .copied()
                .context("surface has no formats")?;
            let mode = if self.vsync {
                vk::PresentModeKHR::FIFO
            } else {
                [vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE]
                    .into_iter()
                    .find(|m| modes.contains(m))
                    .unwrap_or(vk::PresentModeKHR::FIFO)
            };
            let extent = if caps.current_extent.width != u32::MAX {
                caps.current_extent
            } else {
                vk::Extent2D {
                    width: size[0].clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: size[1]
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            };
            let mut count = caps.min_image_count.max(2) + 1;
            if caps.max_image_count > 0 {
                count = count.min(caps.max_image_count);
            }
            let usage = vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::COLOR_ATTACHMENT;
            if !caps.supported_usage_flags.contains(usage) {
                bail!("swap chain images can't be transfer destinations");
            }
            let old = self.handle;
            self.handle = self.fns.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.surface)
                    .min_image_count(count)
                    .image_format(format.format)
                    .image_color_space(format.color_space)
                    .image_extent(extent)
                    .image_array_layers(1)
                    .image_usage(usage)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(mode)
                    .clipped(true)
                    .old_swapchain(old),
                None,
            )?;
            for v in self.views.drain(..) {
                d.device.destroy_image_view(v, None);
            }
            if old != vk::SwapchainKHR::null() {
                self.fns.destroy_swapchain(old, None);
            }
            self.images = self.fns.get_swapchain_images(self.handle)?;
            for &image in &self.images {
                self.views.push(
                    d.device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(format.format)
                            .subresource_range(res::COLOR),
                        None,
                    )?,
                );
            }
            self.extent = extent;
            self.format = format.format;
            self.metadata_peak = 0.;
            for s in self.release.drain(..) {
                d.device.destroy_semaphore(s, None);
            }
            for _ in &self.images {
                self.release.push(
                    d.device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
                );
            }
            Ok(())
        }
    }

    unsafe fn destroy(&mut self, d: &Device) {
        unsafe {
            for s in self.release.drain(..) {
                d.device.destroy_semaphore(s, None);
            }
            for v in self.views.drain(..) {
                d.device.destroy_image_view(v, None);
            }
            self.fns.destroy_swapchain(self.handle, None);
            d.surface_fn.destroy_surface(self.surface, None);
        }
    }
}

/// Command pool, one command buffer, its fence and the acquire semaphore (one frame in flight,
/// as on D3D12).
struct Frame {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    acquire: vk::Semaphore,
}

impl Frame {
    unsafe fn new(d: &Device) -> Result<Self> {
        unsafe {
            let pool = d.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(d.family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            let cmd = d.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(1),
            )?[0];
            let fence = d.device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?;
            let acquire = d
                .device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
            Ok(Self {
                pool,
                cmd,
                fence,
                acquire,
            })
        }
    }

    unsafe fn begin(&self, d: &Device) -> Result<()> {
        unsafe {
            d.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            d.device.reset_fences(&[self.fence])?;
            d.device
                .reset_command_buffer(self.cmd, vk::CommandBufferResetFlags::empty())?;
            d.device.begin_command_buffer(
                self.cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            Ok(())
        }
    }

    unsafe fn destroy(&self, d: &Device) {
        unsafe {
            d.device.destroy_semaphore(self.acquire, None);
            d.device.destroy_fence(self.fence, None);
            d.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// Clears `image` (whole, color) and leaves it in `to` for `next_stage`/`next_access`.
unsafe fn record_clear(
    d: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    color: [f32; 4],
    to: vk::ImageLayout,
    next_stage: vk::PipelineStageFlags2,
    next_access: vk::AccessFlags2,
) {
    let range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let to_dst = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(image)
        .subresource_range(range)];
    let after = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .dst_stage_mask(next_stage)
        .dst_access_mask(next_access)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(to)
        .image(image)
        .subresource_range(range)];
    unsafe {
        d.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&to_dst),
        );
        d.cmd_clear_color_image(
            cmd,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &vk::ClearColorValue { float32: color },
            &[range],
        );
        d.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&after),
        );
    }
}

/// `--vulkan`: the phase-2 smoke run.
pub fn run(args: &[String]) -> Result<()> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
    };
    let frames = value("--frames")
        .map(|v| v.parse::<u32>())
        .transpose()
        .context("--frames")?;
    let size = [
        value("--width")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1280),
        value("--height")
            .and_then(|v| v.parse().ok())
            .unwrap_or(720),
    ];
    if flag("--headless") {
        unsafe { headless(frames.unwrap_or(1), size, flag("--debug")) }
    } else {
        let event_loop = winit::event_loop::EventLoop::new()?;
        let mut app = Windowed {
            frames,
            size,
            debug: flag("--debug"),
            vsync: !flag("--no-vsync"),
            live: None,
            shown: 0,
            start: Instant::now(),
            error: None,
        };
        event_loop.run_app(&mut app)?;
        match app.error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

fn color(t: f32) -> [f32; 4] {
    [
        0.5 + 0.5 * t.sin(),
        0.5 + 0.5 * (t + 2.1).sin(),
        0.5 + 0.5 * (t + 4.2).sin(),
        1.0,
    ]
}

/// Clears an offscreen image `frames` times, reads the last one back and checks its color.
unsafe fn headless(frames: u32, size: [u32; 2], debug: bool) -> Result<()> {
    unsafe {
        let (d, _) = Device::new(None, debug)?;
        let frame = Frame::new(&d)?;
        let format = vk::Format::R8G8B8A8_UNORM;
        let image = d.device.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: size[0],
                    height: size[1],
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC),
            None,
        )?;
        let image_memory = d.alloc(
            d.device.get_image_memory_requirements(image),
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        d.device.bind_image_memory(image, image_memory, 0)?;
        let bytes = size[0] as u64 * size[1] as u64 * 4;
        let buffer = d.device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(bytes)
                .usage(vk::BufferUsageFlags::TRANSFER_DST),
            None,
        )?;
        let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let buffer_memory = d.alloc(d.device.get_buffer_memory_requirements(buffer), host)?;
        d.device.bind_buffer_memory(buffer, buffer_memory, 0)?;

        let started = Instant::now();
        let mut last = [0.0; 4];
        for i in 0..frames {
            last = color(i as f32 * 0.1);
            frame.begin(&d)?;
            record_clear(
                &d.device,
                frame.cmd,
                image,
                last,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::TRANSFER,
                vk::AccessFlags2::TRANSFER_READ,
            );
            if i + 1 == frames {
                let region = vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: size[0],
                        height: size[1],
                        depth: 1,
                    });
                d.device.cmd_copy_image_to_buffer(
                    frame.cmd,
                    image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    buffer,
                    &[region],
                );
                let to_host = [vk::MemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::HOST)
                    .dst_access_mask(vk::AccessFlags2::HOST_READ)];
                d.device.cmd_pipeline_barrier2(
                    frame.cmd,
                    &vk::DependencyInfo::default().memory_barriers(&to_host),
                );
            }
            d.device.end_command_buffer(frame.cmd)?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(frame.cmd)];
            d.device.queue_submit2(
                d.queue,
                &[vk::SubmitInfo2::default().command_buffer_infos(&cmds)],
                frame.fence,
            )?;
        }
        d.device.wait_for_fences(&[frame.fence], true, u64::MAX)?;
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let ptr = d
            .device
            .map_memory(buffer_memory, 0, bytes, vk::MemoryMapFlags::empty())?
            as *const u8;
        let texels = std::slice::from_raw_parts(ptr, bytes as usize);
        let want: Vec<i32> = last.iter().map(|c| (c * 255.0).round() as i32).collect();
        let bad = texels
            .as_chunks::<4>()
            .0
            .iter()
            .position(|p| p.iter().zip(&want).any(|(&a, &b)| (a as i32 - b).abs() > 1));
        let first = texels[..4].to_vec();
        d.device.unmap_memory(buffer_memory);
        d.device.destroy_buffer(buffer, None);
        d.device.free_memory(buffer_memory, None);
        d.device.destroy_image(image, None);
        d.device.free_memory(image_memory, None);
        frame.destroy(&d);
        if let Some(i) = bad {
            bail!("readback differs at texel {i}: got {first:?}, want {want:?}");
        }
        let warnings = messages();
        if !warnings.is_empty() {
            bail!("{} validation messages", warnings.len());
        }
        println!(
            "VULKAN_OK {} frames={frames} {}x{} ms={ms:.1} pixel={first:?}",
            d.name, size[0], size[1]
        );
        Ok(())
    }
}

struct Live {
    swap: Swapchain,
    frame: Frame,
    // Dropped last: the device outlives the swap chain and frame resources (see Drop).
    device: Device,
    window: winit::window::Window,
}

impl Drop for Live {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device.device_wait_idle();
            self.frame.destroy(&self.device);
            self.swap.destroy(&self.device);
        }
    }
}

struct Windowed {
    frames: Option<u32>,
    size: [u32; 2],
    debug: bool,
    vsync: bool,
    live: Option<Live>,
    shown: u32,
    start: Instant,
    error: Option<anyhow::Error>,
}

impl Windowed {
    unsafe fn open(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) -> Result<()> {
        let attrs = winit::window::Window::default_attributes()
            .with_title("Caustica (Vulkan)")
            .with_inner_size(winit::dpi::PhysicalSize::new(self.size[0], self.size[1]));
        let window = event_loop.create_window(attrs)?;
        let raw = (
            window.display_handle()?.as_raw(),
            window.window_handle()?.as_raw(),
        );
        let (device, surface) = unsafe { Device::new(Some(raw), self.debug)? };
        let surface = surface.context("no surface")?;
        let inner = window.inner_size();
        let swap =
            unsafe { Swapchain::new(&device, surface, [inner.width, inner.height], self.vsync)? };
        let frame = unsafe { Frame::new(&device)? };
        eprintln!(
            "vulkan: {} {:?} {}x{}",
            device.name, swap.format, swap.extent.width, swap.extent.height
        );
        self.live = Some(Live {
            swap,
            frame,
            device,
            window,
        });
        Ok(())
    }

    /// Returns false when the frame was skipped (minimized or the swap chain was rebuilt).
    unsafe fn draw(&mut self) -> Result<bool> {
        let live = self.live.as_mut().context("no window")?;
        let inner = live.window.inner_size();
        if inner.width == 0 || inner.height == 0 {
            return Ok(false);
        }
        let size = [inner.width, inner.height];
        let (d, swap, frame) = (&live.device, &mut live.swap, &live.frame);
        unsafe {
            if [swap.extent.width, swap.extent.height] != size {
                swap.recreate(d, size)?;
            }
            d.device.wait_for_fences(&[frame.fence], true, u64::MAX)?;
            let index = match swap.fns.acquire_next_image(
                swap.handle,
                u64::MAX,
                frame.acquire,
                vk::Fence::null(),
            ) {
                Ok((i, _)) => i,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    swap.recreate(d, size)?;
                    return Ok(false);
                }
                Err(e) => return Err(e.into()),
            };
            frame.begin(d)?;
            record_clear(
                &d.device,
                frame.cmd,
                swap.images[index as usize],
                color(self.start.elapsed().as_secs_f32()),
                vk::ImageLayout::PRESENT_SRC_KHR,
                vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                vk::AccessFlags2::NONE,
            );
            d.device.end_command_buffer(frame.cmd)?;
            let release = swap.release[index as usize];
            let waits = [vk::SemaphoreSubmitInfo::default()
                .semaphore(frame.acquire)
                .stage_mask(vk::PipelineStageFlags2::TRANSFER)];
            let signals = [vk::SemaphoreSubmitInfo::default()
                .semaphore(release)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(frame.cmd)];
            d.device.queue_submit2(
                d.queue,
                &[vk::SubmitInfo2::default()
                    .wait_semaphore_infos(&waits)
                    .command_buffer_infos(&cmds)
                    .signal_semaphore_infos(&signals)],
                frame.fence,
            )?;
            let present_waits = [release];
            let swapchains = [swap.handle];
            let indices = [index];
            match swap.fns.queue_present(
                d.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&present_waits)
                    .swapchains(&swapchains)
                    .image_indices(&indices),
            ) {
                Ok(false) => {}
                Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => swap.recreate(d, size)?,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(true)
    }
}

impl winit::application::ApplicationHandler for Windowed {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        if let Err(e) = unsafe { self.open(event_loop) } {
            self.error = Some(e);
            event_loop.exit();
            return;
        }
        if let Some(l) = &self.live {
            l.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::WindowEvent;
        match event {
            WindowEvent::CloseRequested => {
                self.live = None;
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                match unsafe { self.draw() } {
                    Ok(true) => self.shown += 1,
                    Ok(false) => {}
                    Err(e) => {
                        self.error = Some(e);
                        self.live = None;
                        event_loop.exit();
                        return;
                    }
                }
                if self.frames.is_some_and(|n| self.shown >= n) {
                    let name = self
                        .live
                        .as_ref()
                        .map(|l| l.device.name.clone())
                        .unwrap_or_default();
                    let warnings = messages();
                    self.live = None;
                    if warnings.is_empty() {
                        println!("VULKAN_OK {name} frames={} presented", self.shown);
                    } else {
                        self.error = Some(anyhow!("{} validation messages", warnings.len()));
                    }
                    event_loop.exit();
                } else if let Some(l) = &self.live {
                    l.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}
