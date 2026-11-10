//! Windows: the renderer the app drives, D3D12 (the reference) or Vulkan (`--vulkan`). Both
//! have the same API and deref to `GpuState`; this forwards every call to the one in use.
//! Linux has only Vulkan and uses `vulkan::gpu::Gpu` directly.
use crate::render::{AovLayer, Capabilities, GpuState};
use crate::scene::{Settings, Upscaler};
use anyhow::Result;

pub enum Renderer {
    D3d12(Box<crate::gpu::Gpu>),
    Vulkan(Box<crate::vulkan::gpu::Gpu>),
}

macro_rules! each {
    ($self:ident, $g:ident => $e:expr) => {
        match $self {
            Renderer::D3d12($g) => $e,
            Renderer::Vulkan($g) => $e,
        }
    };
}

impl std::ops::Deref for Renderer {
    type Target = GpuState;
    fn deref(&self) -> &GpuState {
        each!(self, g => &g.st)
    }
}
impl std::ops::DerefMut for Renderer {
    fn deref_mut(&mut self) -> &mut GpuState {
        each!(self, g => &mut g.st)
    }
}

impl Renderer {
    pub unsafe fn new(
        window: &winit::window::Window,
        width: u32,
        height: u32,
        s: &Settings,
        debug: bool,
        vulkan: bool,
    ) -> Result<Self> {
        unsafe {
            if vulkan {
                return Ok(Self::Vulkan(Box::new(crate::vulkan::gpu::Gpu::new(
                    window, width, height, s, debug,
                )?)));
            }
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let hwnd = match window.window_handle()?.as_raw() {
                RawWindowHandle::Win32(h) => windows::Win32::Foundation::HWND(h.hwnd.get() as _),
                _ => anyhow::bail!("Windows only"),
            };
            Ok(Self::D3d12(Box::new(crate::gpu::Gpu::new(
                hwnd, width, height, s, debug,
            )?)))
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
        unsafe { each!(self, g => g.render(s, draw, dt, primitives, delta, pixels_per_point)) }
    }
    pub unsafe fn upload_reference(&mut self, rgba: &[f32]) -> Result<()> {
        unsafe { each!(self, g => g.upload_reference(rgba)) }
    }
    pub unsafe fn capture_reference(&mut self) -> Result<()> {
        unsafe { each!(self, g => g.capture_reference()) }
    }
    pub unsafe fn capture(
        &mut self,
        path: &std::path::Path,
        hdr: bool,
        include_ui: bool,
    ) -> Result<()> {
        unsafe { each!(self, g => g.capture(path, hdr, include_ui)) }
    }
    pub unsafe fn read_rgba8(&mut self, index: usize) -> Result<Vec<u8>> {
        unsafe { each!(self, g => g.read_rgba8(index)) }
    }
    pub unsafe fn read_layers(&mut self) -> Result<Vec<AovLayer>> {
        unsafe { each!(self, g => g.read_layers()) }
    }
    pub unsafe fn read_floats(&mut self, index: usize) -> Result<Vec<f32>> {
        unsafe { each!(self, g => g.read_floats(index)) }
    }
    pub unsafe fn read_lpe(&mut self, slot: u32) -> Result<Vec<f32>> {
        unsafe { each!(self, g => g.read_lpe(slot)) }
    }
    pub unsafe fn debug_messages(&self) -> Vec<String> {
        unsafe { each!(self, g => g.debug_messages()) }
    }
    pub fn removed_reason(&self) -> Option<String> {
        each!(self, g => g.removed_reason())
    }
    pub fn reset(&mut self, reset_history: bool) {
        each!(self, g => g.reset(reset_history))
    }
    pub unsafe fn reconfigure_upscalers(&mut self) -> Result<()> {
        unsafe { each!(self, g => g.reconfigure_upscalers()) }
    }
    pub fn is_vulkan(&self) -> bool {
        matches!(self, Renderer::Vulkan(_))
    }
    /// Blocks until DLSS (NGX) has started, for runs that need it from the first frame.
    pub fn wait_ngx(&mut self) {
        match self {
            Renderer::D3d12(g) => g.poll_ngx(true),
            Renderer::Vulkan(g) => g.wait_ngx(),
        }
    }
    pub fn capabilities(&self) -> Capabilities {
        each!(self, g => g.capabilities())
    }
    pub fn upscaler_usable(&self, kind: Upscaler) -> bool {
        each!(self, g => g.upscaler_usable(kind))
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        each!(self, g => g.upscaler_available(kind))
    }
    pub unsafe fn set_hdr(&mut self, on: bool, hdr10: bool, s: &Settings) -> Result<bool> {
        unsafe { each!(self, g => g.set_hdr(on, hdr10, s)) }
    }
    pub unsafe fn set_lut(&mut self, lut: Option<(u32, &[[f32; 3]])>) -> Result<()> {
        unsafe { each!(self, g => g.set_lut(lut)) }
    }
    pub fn map_count(&self) -> usize {
        each!(self, g => g.map_count())
    }
    pub fn environment_loaded(&self) -> bool {
        each!(self, g => g.environment_loaded())
    }
    pub fn stars_loaded(&self) -> bool {
        each!(self, g => g.stars_loaded())
    }
    pub unsafe fn set_environment(
        &mut self,
        env: Option<&crate::environment::EnvMap>,
    ) -> Result<()> {
        unsafe { each!(self, g => g.set_environment(env)) }
    }
    pub unsafe fn reload_shaders(&mut self, sh: &crate::hotreload::Shaders) -> Result<()> {
        unsafe { each!(self, g => g.reload_shaders(sh)) }
    }
    pub unsafe fn resize(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        unsafe { each!(self, g => g.resize(w, h, s)) }
    }
    pub unsafe fn resize_swap(&mut self, w: u32, h: u32) -> Result<()> {
        unsafe { each!(self, g => g.resize_swap(w, h)) }
    }
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        unsafe { each!(self, g => g.resize_output(w, h, s)) }
    }
    pub fn restir_layers(&self) -> u32 {
        each!(self, g => g.restir_layers())
    }
    pub unsafe fn rebuild_scene(&mut self, s: &Settings) -> Result<()> {
        unsafe { each!(self, g => g.rebuild_scene(s)) }
    }
    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        unsafe { each!(self, g => g.update_materials(s)) }
    }
}
