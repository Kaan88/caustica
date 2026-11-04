//! The app-facing Vulkan renderer: the same API as the D3D12 `gpu::Gpu`, so the app builds
//! on Linux (phase 2b). It clears and presents; the passes arrive in phases 3 to 5, and until
//! then the calls that need them succeed as no-ops or report that Vulkan lacks them.
use super::{Device, Frame, Swapchain, record_clear};
pub use crate::render::*;
use crate::scene::{Settings, Upscaler};
use anyhow::{Context, Result, bail};
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub struct Gpu {
    pub st: GpuState,
    swap: Swapchain,
    frame: Frame,
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
            self.frame.destroy(&self.device);
            self.swap.destroy(&self.device);
        }
    }
}

const NOT_YET: &str = "not on Vulkan yet";

impl Gpu {
    pub unsafe fn new(
        window: &winit::window::Window,
        width: u32,
        height: u32,
        _s: &Settings,
        debug: bool,
    ) -> Result<Self> {
        unsafe {
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
            let st = GpuState {
                width,
                height,
                swap_size: [width, height],
                rw: width,
                rh: height,
                adapter_name: device.name.clone(),
                upscale_status: NOT_YET.into(),
                rr_status: NOT_YET.into(),
                denoiser_activity: "",
                hdr_levels: [203., 1000.],
                ..Default::default()
            };
            Ok(Self {
                st,
                swap,
                frame,
                device,
            })
        }
    }

    pub unsafe fn render(
        &mut self,
        _s: &Settings,
        draw: bool,
        _dt: f32,
        _primitives: &[egui::ClippedPrimitive],
        _delta: &egui::TexturesDelta,
        _pixels_per_point: f32,
    ) -> Result<()> {
        if !draw || self.swap_size.contains(&0) {
            return Ok(());
        }
        let (d, swap, frame) = (&self.device, &mut self.swap, &self.frame);
        unsafe {
            d.device.wait_for_fences(&[frame.fence], true, u64::MAX)?;
            let index = match swap.fns.acquire_next_image(
                swap.handle,
                u64::MAX,
                frame.acquire,
                vk::Fence::null(),
            ) {
                Ok((i, _)) => i,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    swap.recreate(d, self.st.swap_size)?;
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            };
            frame.begin(d)?;
            record_clear(
                &d.device,
                frame.cmd,
                swap.images[index as usize],
                [0.02, 0.02, 0.025, 1.0],
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
                Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    swap.recreate(d, self.st.swap_size)?
                }
                Err(e) => return Err(e.into()),
            }
        }
        self.st.frame += 1;
        self.st.samples += 1;
        Ok(())
    }

    pub unsafe fn begin(&self) -> Result<()> {
        Ok(())
    }
    pub unsafe fn submit(&mut self) -> Result<()> {
        unsafe { self.device.device.device_wait_idle()? };
        Ok(())
    }
    pub fn removed_reason(&self) -> Option<String> {
        None
    }
    pub fn reset(&mut self, _reset_history: bool) {
        self.st.samples = 0;
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
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, _s: &Settings) -> Result<()> {
        (self.st.width, self.st.height, self.st.rw, self.st.rh) = (w, h, w, h);
        self.st.samples = 0;
        Ok(())
    }
    pub unsafe fn rebuild_scene(&mut self, _s: &Settings) -> Result<()> {
        self.st.samples = 0;
        Ok(())
    }
    pub unsafe fn update_materials(&mut self, _s: &Settings) -> Result<()> {
        self.st.samples = 0;
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
    pub fn rr_available(&self) -> bool {
        false
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
    pub unsafe fn capture(
        &mut self,
        _path: &std::path::Path,
        _hdr: bool,
        _include_ui: bool,
    ) -> Result<()> {
        bail!("captures are {NOT_YET}")
    }
    pub unsafe fn read_rgba8(&mut self, _index: usize) -> Result<Vec<u8>> {
        bail!("readbacks are {NOT_YET}")
    }
    pub unsafe fn read_texture(&mut self, _index: usize) -> Result<(u32, u32, usize, Vec<f32>)> {
        bail!("readbacks are {NOT_YET}")
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
