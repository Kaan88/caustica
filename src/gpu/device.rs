//! Command submission, upscaler setup, HDR, LUT and environment textures, readbacks, shader reload and resizing.
use super::*;

impl Gpu {
    pub unsafe fn begin(&self) -> Result<()> {
        self.allocator.Reset()?;
        self.list.Reset(&self.allocator, None)?;
        Ok(())
    }
    pub unsafe fn submit(&mut self) -> Result<()> {
        self.list.Close()?;
        self.queue.ExecuteCommandLists(&[Some(self.list.cast()?)]);
        self.fence_value += 1;
        self.queue.Signal(&self.fence, self.fence_value)?;
        if self.fence.GetCompletedValue() < self.fence_value {
            let start = std::time::Instant::now();
            self.fence
                .SetEventOnCompletion(self.fence_value, self.event)?;
            ensure!(
                WaitForSingleObject(self.event, 30_000)
                    == windows::Win32::Foundation::WAIT_OBJECT_0,
                "GPU fence timeout; device removed: {:?}",
                self.device.GetDeviceRemovedReason()
            );
            self.fence_wait_ms += start.elapsed().as_secs_f64() * 1000.;
        }
        Ok(())
    }
    /// Why the device was removed, if it was (a hang, a page fault, a driver reset).
    pub fn removed_reason(&self) -> Option<String> {
        let r = unsafe { self.device.GetDeviceRemovedReason() };
        r.is_err()
            .then(|| format!("{r:?}{}", unsafe { self.dred_report() }))
    }
    /// DRED's account of a removal (debug runs only): per command list, the operations after
    /// the last one the GPU finished; then a page fault's address and the allocations near it.
    pub(super) unsafe fn dred_report(&self) -> String {
        let Ok(dred) = self.device.cast::<ID3D12DeviceRemovedExtendedData>() else {
            return String::new();
        };
        let mut out = String::new();
        if let Ok(b) = dred.GetAutoBreadcrumbsOutput() {
            let mut node = b.pHeadAutoBreadcrumbNode;
            while !node.is_null() {
                let n = &*node;
                let done = if n.pLastBreadcrumbValue.is_null() {
                    0
                } else {
                    *n.pLastBreadcrumbValue
                };
                if done < n.BreadcrumbCount && !n.pCommandHistory.is_null() {
                    let ops =
                        std::slice::from_raw_parts(n.pCommandHistory, n.BreadcrumbCount as usize);
                    let dispatches = ops[..done as usize]
                        .iter()
                        .filter(|o| **o == D3D12_AUTO_BREADCRUMB_OP_DISPATCH)
                        .count();
                    let next: Vec<i32> = ops[done as usize..].iter().take(8).map(|o| o.0).collect();
                    out += &format!(
                        "; DRED: list {done}/{} ops done ({dispatches} dispatches), next ops {next:?}",
                        n.BreadcrumbCount
                    );
                }
                node = n.pNext;
            }
        }
        if let Ok(p) = dred.GetPageFaultAllocationOutput()
            && p.PageFaultVA != 0
        {
            out += &format!("; page fault at {:#x}", p.PageFaultVA);
            for (label, mut node) in [
                ("live", p.pHeadExistingAllocationNode),
                ("freed", p.pHeadRecentFreedAllocationNode),
            ] {
                let mut k = 0;
                while !node.is_null() && k < 4 {
                    let n = &*node;
                    let name = if n.ObjectNameW.is_null() {
                        String::new()
                    } else {
                        n.ObjectNameW.to_string().unwrap_or_default()
                    };
                    out += &format!(" {label}: {name:?} type {}", n.AllocationType.0);
                    node = n.pNext;
                    k += 1;
                }
            }
        }
        out
    }
    pub fn reset(&mut self, reset_history: bool) {
        self.samples = 0;
        self.sr_fresh = 0;
        // Each restart takes a fresh, 8192-aligned Sobol block, so a moving camera
        // never reuses the previous frame's sample points.
        self.restarts = self.restarts.wrapping_add(1);
        self.sequence_base = self.restarts.wrapping_mul(0x9E37_79B1) & !8191;
        self.photon_frames = 0;
        self.photon_scale = 1.;
        for q in &mut self.battle {
            *q = BattleQuadrant {
                cost: q.cost,
                ..Default::default()
            };
        }
        if reset_history {
            self.rr_reset = true;
            self.history_valid = false;
            self.restir_history = false;
            self.camera_moved = false;
        } else {
            self.camera_moved = true;
        }
    }
    /// Releases every neural/temporal upscaler so the next frame recreates the one in use.
    pub unsafe fn reconfigure_upscalers(&mut self) -> Result<()> {
        self.wait_idle()?;
        self.release_upscalers();
        self.reset(true);
        Ok(())
    }
    pub(super) unsafe fn release_upscalers(&mut self) {
        caustica_ngx_release(self.ngx);
        caustica_fsr_release(self.fsr);
        self.rr_created = false;
        self.rr_failed = false;
        self.sr_created = None;
        self.sr_failed.clear();
    }
    /// What each reconstruction backend can do here, with the reason when it can't: the
    /// startup checks plus failures met while rendering (UI plan 5.2).
    pub fn capabilities(&self) -> Capabilities {
        let backend = |ok: bool, i: usize, failed: bool| Backend {
            available: ok && !failed,
            reason: if failed && ok {
                "failed while rendering (see the status line); tried again after a restart".into()
            } else {
                self.init_reasons[i].clone()
            },
        };
        let sr = |u: Upscaler, i: usize| {
            backend(self.upscaler_available(u), i, self.sr_failed.contains(&u))
        };
        Capabilities {
            rr: backend(self.rr_available(), 0, self.rr_failed),
            dlss_sr: sr(Upscaler::Dlss, 1),
            fsr: sr(Upscaler::Fsr, 2),
            xess: sr(Upscaler::Xess, 3),
        }
    }
    /// Takes NGX up once its start-up thread is done (`wait`: blocks until it is). Called at the
    /// start of each frame; until then RR and DLSS SR are unavailable ("starting").
    pub fn poll_ngx(&mut self, wait: bool) {
        if !self
            .ngx_pending
            .as_ref()
            .is_some_and(|t| wait || t.is_finished())
        {
            return;
        }
        let (ngx, code) = match self.ngx_pending.take().unwrap().join() {
            Ok(t) => t.0,
            Err(e) => std::panic::resume_unwind(e),
        };
        self.ngx = ngx;
        let (status, [rr, sr]) = unsafe { ngx_status(ngx, code, &self.st.adapter_name) };
        eprintln!("NGX: {status}");
        self.st.rr_status = status;
        self.init_reasons[0] = rr;
        self.init_reasons[1] = sr;
    }
    /// NGX loaded and reports Ray Reconstruction (caps bit 0).
    pub fn rr_available(&self) -> bool {
        !self.ngx.is_null() && unsafe { caustica_ngx_caps(self.ngx) } & 1 != 0
    }
    /// Whether `kind` runs: available and not failed this session (else bicubic stands in).
    pub fn upscaler_usable(&self, kind: Upscaler) -> bool {
        !kind.temporal() || (self.upscaler_available(kind) && !self.sr_failed.contains(&kind))
    }
    pub fn upscaler_available(&self, kind: Upscaler) -> bool {
        match kind {
            Upscaler::Dlss => {
                !self.ngx.is_null() && unsafe { caustica_ngx_caps(self.ngx) } & 2 != 0
            }
            Upscaler::Fsr => !self.fsr.is_null(),
            Upscaler::Xess => !self.xess.is_null(),
            Upscaler::Bilinear | Upscaler::Bicubic => true,
        }
    }
    /// Creates the selected temporal upscaler if needed. Returns it when it can run this frame.
    pub(super) unsafe fn prepare_upscaler(&mut self, s: &Settings) -> Option<Upscaler> {
        let kind = s.upscaler;
        if !s.perspective() {
            self.upscale_status = "Upscaling needs a perspective camera; bicubic".into();
            return None;
        }
        if self.painting {
            self.upscale_status = "Light painting accumulates at render resolution".into();
            return None;
        }
        if s.battle {
            self.upscale_status = "Battle mode compares raw samples".into();
            return None;
        }
        // At 100% only DLAA / Native AA (a temporal upscaler, when asked for) runs.
        if self.rr_active || (self.rw >= self.width && !(s.native_aa && kind.temporal())) {
            self.upscale_status.clear();
            return None;
        }
        if !kind.temporal() {
            self.upscale_status = format!("{} upscaling", kind.label());
            return None;
        }
        if !self.upscaler_available(kind) || self.sr_failed.contains(&kind) {
            if !self.sr_failed.contains(&kind) {
                self.upscale_status = format!("{} unavailable; bicubic upscaling", kind.label());
            }
            return None;
        }
        if self.sr_created == Some(kind) {
            return Some(kind);
        }
        let (w, h, ow, oh) = (self.rw, self.rh, self.width, self.height);
        let (ok, code) = match kind {
            Upscaler::Dlss => {
                let code = caustica_ngx_create_sr(
                    self.ngx,
                    self.list.as_raw(),
                    w,
                    h,
                    ow,
                    oh,
                    s.dlss_preset,
                );
                (ngx_ok(code), code)
            }
            Upscaler::Fsr => {
                let code = caustica_fsr_create(self.fsr, w, h, ow, oh);
                (code == 0, code)
            }
            _ => {
                let code = caustica_xess_create(self.xess, w, ow, oh, 1.);
                (code == 0, code)
            }
        };
        if ok {
            self.sr_created = Some(kind);
            self.sr_reset = true;
            self.upscale_status = match kind {
                Upscaler::Dlss => format!(
                    "{} active · preset {}",
                    kind.label(),
                    crate::scene::DLSS_PRESETS
                        .iter()
                        .find(|(id, _)| *id == s.dlss_preset)
                        .map_or("default", |(_, l)| l)
                ),
                _ => format!("{} active", kind.label()),
            };
            Some(kind)
        } else {
            self.sr_failed.push(kind);
            self.upscale_status = format!(
                "{} failed to start (0x{code:08x}); bicubic upscaling",
                kind.label()
            );
            eprintln!("{}", self.upscale_status);
            None
        }
    }
    pub(super) unsafe fn wait_idle(&mut self) -> Result<()> {
        // Present queues work after the command-list fence. Include it before resizing or shutdown.
        self.fence_value += 1;
        self.queue.Signal(&self.fence, self.fence_value)?;
        self.fence
            .SetEventOnCompletion(self.fence_value, self.event)?;
        ensure!(
            WaitForSingleObject(self.event, 30_000) == windows::Win32::Foundation::WAIT_OBJECT_0,
            "GPU idle timeout"
        );
        Ok(())
    }
    pub(super) fn swap_format(&self) -> DXGI_FORMAT {
        if self.hdr_active && self.hdr10 {
            DXGI_FORMAT_R10G10B10A2_UNORM
        } else if self.hdr_active {
            DXGI_FORMAT_R16G16B16A16_FLOAT
        } else {
            DXGI_FORMAT_R8G8B8A8_UNORM
        }
    }
    /// Switches the swapchain between 8-bit SDR, RGBA16F scRGB and (with `hdr10`) R10G10B10A2
    /// HDR10. Recreates the render-size textures like a resize. HDR10 falls back to scRGB when
    /// refused; returns false when the swapchain can't present scRGB either.
    pub unsafe fn set_hdr(&mut self, on: bool, hdr10: bool, s: &Settings) -> Result<bool> {
        if on == self.hdr_active && (!on || hdr10 == self.hdr10) {
            return Ok(true);
        }
        // The display pass writes the backbuffer as a typed UAV: an adapter without typed stores
        // to R10G10B10A2 can't run HDR10 (scRGB then, as for a refused colour space).
        let hdr10 = hdr10 && {
            let ok = typed_store(&self.device, DXGI_FORMAT_R10G10B10A2_UNORM);
            self.hdr10_refused |= !ok;
            ok
        };
        // Color-space support depends on the buffer format, so switch first, then check.
        self.hdr_active = on;
        self.hdr10 = on && hdr10;
        let [w, h] = self.swap_size;
        self.resize(w, h, s)?;
        if on && hdr10 && !self.color_space_ok {
            // HDR10 refused: try scRGB before giving up on HDR.
            self.hdr10_refused = true;
            self.hdr10 = false;
            self.resize(w, h, s)?;
        }
        if on && !self.color_space_ok {
            self.hdr_refused = true;
            self.hdr_active = false;
            self.hdr10 = false;
            self.resize(w, h, s)?;
            return Ok(false);
        }
        Ok(true)
    }
    /// Zeroes the scope counters (u28) with a copy from a zero buffer.
    pub(super) unsafe fn clear_scopes(&mut self) -> Result<()> {
        let t = self.textures[28].resource.clone();
        let (layout, bytes) = footprint(&self.device, &t);
        if self.scope_zero.is_none() {
            let zero = buffer(
                &self.device,
                bytes,
                D3D12_HEAP_TYPE_UPLOAD,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_GENERIC_READ,
            )?;
            write_buffer(&zero, 0, &vec![0u8; bytes as usize])?;
            self.scope_zero = Some(zero);
        }
        transition(
            &self.list,
            &t,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(self.scope_zero.clone()),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        Ok(())
    }
    /// Readback buffer layout: scopes at 0, then the probe texels (linear output, albedo).
    pub(super) fn readback_layout(&self) -> (u64, u64, u64) {
        let scopes = SCOPES[0] as u64 * 4 * SCOPES[1] as u64;
        let probe_linear = scopes.next_multiple_of(512);
        (scopes, probe_linear, probe_linear + 512)
    }
    /// Uploads a film-look LUT (`size`³ RGB, red fastest, as in .cube files); `None` clears it.
    pub unsafe fn set_lut(&mut self, lut: Option<(u32, &[[f32; 3]])>) -> Result<()> {
        self.wait_idle()?;
        let Some((size, data)) = lut else {
            self.lut_size = 0;
            return Ok(());
        };
        ensure!(
            data.len() == (size * size * size) as usize,
            "LUT size mismatch"
        );
        let t = texture_3d(&self.device, size)?;
        let (layout, bytes) = footprint(&self.device, &t.resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let row = layout.Footprint.RowPitch as u64;
        for z in 0..size as u64 {
            for y in 0..size as u64 {
                let start = ((z * size as u64 + y) * size as u64) as usize;
                let texels: Vec<f32> = data[start..start + size as usize]
                    .iter()
                    .flat_map(|c| [c[0], c[1], c[2], 1.])
                    .collect();
                write_buffer(
                    &staging,
                    (z * size as u64 + y) * row,
                    bytemuck::cast_slice(&texels),
                )?;
            }
        }
        self.begin()?;
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t.resource);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        self.lut = t;
        self.lut_size = size;
        self.lut_view();
        Ok(())
    }
    /// Texture maps on the GPU (core track 5).
    pub fn map_count(&self) -> usize {
        self.maps.len()
    }
    pub fn environment_loaded(&self) -> bool {
        self.env_inv_norm > 0.
    }
    /// Uploads an HDRI and its sampling tables (environment.rs); `None` clears it.
    pub unsafe fn set_environment(
        &mut self,
        env: Option<&crate::environment::EnvMap>,
    ) -> Result<()> {
        self.wait_idle()?;
        let Some(env) = env else {
            self.env_inv_norm = 0.;
            self.env_sun = None;
            return Ok(());
        };
        self.env_sun = env.sun.map(|s| (s, env.sun_share));
        let (w, h) = (env.width as u32, env.height as u32);
        // Half floats: an 8k map is 256 MB instead of 512 MB.
        let peak = env
            .texels
            .iter()
            .flat_map(|t| t[..3].iter().copied())
            .fold(0f32, f32::max);
        let scale = (peak / 60000.).max(1.);
        let halves: Vec<u16> = env
            .texels
            .iter()
            .flat_map(|t| [t[0] / scale, t[1] / scale, t[2] / scale, 1.])
            .map(f32_to_half)
            .collect();
        let texels = env_texture(&self.device, w, h, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
        // The CDF rows, then their cutpoints (environment::build).
        let cdf = env_texture(&self.device, w, 2 * (h + 1), DXGI_FORMAT_R32_FLOAT)?;
        self.upload_texture(&texels, bytemuck::cast_slice(&halves), w as usize * 8)?;
        self.upload_texture(&cdf, bytemuck::cast_slice(&env.cdf), w as usize * 4)?;
        self.env = texels;
        self.env_cdf = cdf;
        self.env_inv_norm = env.inv_norm;
        self.env_scale = scale;
        self.env_views();
        Ok(())
    }
    /// Copies tightly packed rows (`row_bytes` each) into a UAV texture, through a staging
    /// buffer laid out by the copy footprint.
    pub(super) unsafe fn upload_texture(
        &mut self,
        t: &Texture,
        data: &[u8],
        row_bytes: usize,
    ) -> Result<()> {
        let (layout, bytes) = footprint(&self.device, &t.resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let pitch = layout.Footprint.RowPitch as u64;
        for (y, row) in data.chunks_exact(row_bytes).enumerate() {
            write_buffer(&staging, y as u64 * pitch, row)?;
        }
        self.begin()?;
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        let mut dst = location(&t.resource);
        let mut src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()
    }
    /// UAVs of the environment (heap slot 30) and its CDFs (31).
    pub(super) unsafe fn env_views(&self) {
        for (t, slot) in [(&self.env, TEXTURES + 1), (&self.env_cdf, TEXTURES + 2)] {
            let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: t.format,
                ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
                Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_UAV::default(),
                },
            };
            self.device
                .CreateUnorderedAccessView(&t.resource, None, Some(&desc), self.cpu(slot));
        }
    }
    /// The LUT's UAV at heap slot 29 (after the per-resize textures).
    pub(super) unsafe fn lut_view(&self) {
        let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
            Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                Texture3D: D3D12_TEX3D_UAV {
                    MipSlice: 0,
                    FirstWSlice: 0,
                    WSize: u32::MAX,
                },
            },
        };
        self.device.CreateUnorderedAccessView(
            &self.lut.resource,
            None,
            Some(&desc),
            self.cpu(TEXTURES),
        );
    }
    /// Copies the scope counters and the probe texels into the readback buffer.
    pub(super) unsafe fn queue_readbacks(&mut self, probe: Option<[u32; 2]>) -> Result<()> {
        let (_, probe_linear, probe_albedo) = self.readback_layout();
        if self.readback.is_none() {
            self.readback = Some(buffer(
                &self.device,
                probe_albedo + 1024,
                D3D12_HEAP_TYPE_READBACK,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            )?);
        }
        let readback = self.readback.clone().unwrap();
        let copy = |index: usize, rect: [u32; 4], offset: u64, format: DXGI_FORMAT, row: u32| {
            let t = &self.textures[index].resource;
            transition(
                &self.list,
                t,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            let mut src = location(t);
            let mut dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(readback.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: offset,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: format,
                            Width: rect[2] - rect[0],
                            Height: rect[3] - rect[1],
                            Depth: 1,
                            RowPitch: row,
                        },
                    },
                },
            };
            let area = D3D12_BOX {
                left: rect[0],
                top: rect[1],
                front: 0,
                right: rect[2],
                bottom: rect[3],
                back: 1,
            };
            self.list
                .CopyTextureRegion(&dst, 0, 0, 0, &src, Some(&area));
            ManuallyDrop::drop(&mut src.pResource);
            ManuallyDrop::drop(&mut dst.pResource);
            transition(
                &self.list,
                t,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        };
        if self.scopes_on {
            copy(
                28,
                [0, 0, SCOPES[0], SCOPES[1]],
                0,
                DXGI_FORMAT_R32_UINT,
                SCOPES[0] * 4,
            );
        }
        if let Some([x, y]) = probe {
            copy(
                18,
                [x, y, x + 1, y + 1],
                probe_linear,
                DXGI_FORMAT_R32G32B32A32_FLOAT,
                256,
            );
            let rx = (x * self.rw / self.width).min(self.rw - 1);
            let ry = (y * self.rh / self.height).min(self.rh - 1);
            copy(
                3,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo,
                DXGI_FORMAT_R32G32B32A32_FLOAT,
                256,
            );
            copy(
                5,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo + 512,
                DXGI_FORMAT_R32_FLOAT,
                256,
            );
            copy(
                19,
                [rx, ry, rx + 1, ry + 1],
                probe_albedo + 768,
                DXGI_FORMAT_R32_FLOAT,
                256,
            );
        }
        Ok(())
    }
    pub(super) unsafe fn finish_readbacks(&mut self, probe: Option<[u32; 2]>) -> Result<()> {
        self.probe_result = None;
        let Some(readback) = &self.readback else {
            return Ok(());
        };
        if !self.scopes_on && probe.is_none() {
            return Ok(());
        }
        let (scopes, probe_linear, probe_albedo) = self.readback_layout();
        let mut p = null_mut();
        readback.Map(0, None, Some(&mut p))?;
        let base = p as *const u8;
        if self.scopes_on {
            self.st.scopes =
                std::slice::from_raw_parts(base as *const u32, (scopes / 4) as usize).to_vec();
        }
        if probe.is_some() {
            let texel = |offset: u64| {
                let f = std::slice::from_raw_parts(base.add(offset as usize) as *const f32, 4);
                [f[0], f[1], f[2], f[3]]
            };
            self.st.probe_result = Some(Probe {
                linear: texel(probe_linear),
                albedo: texel(probe_albedo),
                depth: texel(probe_albedo + 512)[0],
                id: texel(probe_albedo + 768)[0] as u32,
            });
        }
        readback.Unmap(0, None);
        Ok(())
    }
    /// Swaps in freshly compiled shaders (hot reload). Every pipeline is built before any is
    /// replaced, so a failure keeps the old ones. Restarts the accumulation.
    pub unsafe fn reload_shaders(&mut self, sh: &crate::hotreload::Shaders) -> Result<()> {
        let trace = create_compute_pipeline(&self.device, &self.root, &sh.trace)?;
        let trace_maps = create_compute_pipeline(&self.device, &self.root, &sh.trace_maps)?;
        let trace_lean = create_compute_pipeline(&self.device, &self.root, &sh.trace_lean)?;
        let trace_maps_lean =
            create_compute_pipeline(&self.device, &self.root, &sh.trace_maps_lean)?;
        let trace_spectral = create_compute_pipeline(&self.device, &self.root, &sh.trace_spectral)?;
        let trace_spectral_lean =
            create_compute_pipeline(&self.device, &self.root, &sh.trace_spectral_lean)?;
        let trace_paths = create_compute_pipeline(&self.device, &self.root, &sh.trace_paths)?;
        let trace_lpe = create_compute_pipeline(&self.device, &self.root, &sh.trace_lpe)?;
        let restir_presample =
            create_compute_pipeline(&self.device, &self.root, &sh.restir_presample)?;
        let restir_initial = create_compute_pipeline(&self.device, &self.root, &sh.restir_initial)?;
        let restir_spatial = create_compute_pipeline(&self.device, &self.root, &sh.restir_spatial)?;
        let bdpt = create_compute_pipeline(&self.device, &self.root, &sh.bdpt)?;
        let bdpt_resolve = create_compute_pipeline(&self.device, &self.root, &sh.bdpt_resolve)?;
        let photon_clear = create_compute_pipeline(&self.device, &self.root, &sh.photon_clear)?;
        let photon_trace = create_compute_pipeline(&self.device, &self.root, &sh.photon_trace)?;
        let photon_trace_spectral =
            create_compute_pipeline(&self.device, &self.root, &sh.photon_trace_spectral)?;
        let photon_gather_spectral =
            create_compute_pipeline(&self.device, &self.root, &sh.photon_gather_spectral)?;
        let bdpt_spectral = create_compute_pipeline(&self.device, &self.root, &sh.bdpt_spectral)?;
        let photon_gather = create_compute_pipeline(&self.device, &self.root, &sh.photon_gather)?;
        let filter = create_compute_pipeline(&self.device, &self.root, &sh.filter)?;
        let display = create_compute_pipeline(&self.device, &self.root, &sh.display)?;
        let glare = create_compute_pipeline(&self.device, &self.root, &sh.glare)?;
        let gui = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            &sh.vs,
            &sh.ps,
        )?;
        let gui_hdr = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            &sh.vs,
            &sh.ps,
        )?;
        let gui_hdr10 = create_gui_pipeline(
            &self.device,
            &self.root,
            DXGI_FORMAT_R10G10B10A2_UNORM,
            &sh.vs,
            &sh.ps,
        )?;
        self.wait_idle()?;
        (self.trace_lean, self.trace_maps_lean) = (trace_lean, trace_maps_lean);
        self.trace_spectral = trace_spectral;
        self.trace_spectral_lean = trace_spectral_lean;
        self.photon_trace_spectral = photon_trace_spectral;
        self.photon_gather_spectral = photon_gather_spectral;
        self.bdpt_spectral = bdpt_spectral;
        if self.ser.is_some() {
            self.ser = make_ser(
                &self.device,
                &self.root,
                ser_support(&self.device),
                [
                    &sh.trace_ser,
                    &sh.trace_maps_ser,
                    &sh.trace_lean_ser,
                    &sh.trace_maps_lean_ser,
                    &sh.trace_lpe_ser,
                    &sh.trace_spectral_ser,
                    &sh.trace_spectral_lean_ser,
                ],
            );
        }
        (
            self.trace,
            self.trace_maps,
            self.trace_paths,
            self.trace_lpe,
            self.restir_presample,
            self.restir_initial,
            self.restir_spatial,
            (
                self.bdpt,
                self.bdpt_resolve,
                self.photon_clear,
                self.photon_trace,
                self.photon_gather,
            ),
            self.filter,
            (self.display, self.glare),
            self.gui,
            self.gui_hdr,
        ) = (
            trace,
            trace_maps,
            trace_paths,
            trace_lpe,
            restir_presample,
            restir_initial,
            restir_spatial,
            (
                bdpt,
                bdpt_resolve,
                photon_clear,
                photon_trace,
                photon_gather,
            ),
            filter,
            (display, glare),
            gui,
            gui_hdr,
        );
        self.gui_hdr10 = gui_hdr10;

        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    /// Resizes the swapchain and the output to the same size.
    pub unsafe fn resize(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        self.resize_swap(w, h)?;
        self.resize_output(w, h, s)
    }
    /// Resizes only the swapchain. While the output has another size (a video export), each
    /// frame clears the backbuffer instead of copying the image into it, then draws the UI.
    pub unsafe fn resize_swap(&mut self, w: u32, h: u32) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid viewport size"
        );
        // The previous submit has completed. Reset bindings before releasing swap-chain resources.
        self.wait_idle()?;
        self.begin()?;
        self.list.Close()?;
        self.buffers.clear();
        self.ui_frame = None;
        let resized = self
            .swap
            .ResizeBuffers(2, w, h, self.swap_format(), DXGI_SWAP_CHAIN_FLAG(0))
            .and_then(|_| {
                let space = if self.hdr_active && self.hdr10 {
                    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
                } else if self.hdr_active {
                    DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
                } else {
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709
                };
                let support = self.swap.CheckColorSpaceSupport(space)?;
                self.color_space_ok =
                    support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32 != 0;
                if self.color_space_ok {
                    self.swap.SetColorSpace1(space)?;
                }
                Ok(())
            });
        if let Err(e) = resized {
            if let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
                for i in 0..q.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                    let mut len = 0;
                    let _ = q.GetMessage(DXGI_DEBUG_ALL, i, None, &mut len);
                    let mut bytes = vec![0u64; len.div_ceil(8)];
                    let p = bytes.as_mut_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                    if q.GetMessage(DXGI_DEBUG_ALL, i, Some(p), &mut len).is_ok() {
                        eprintln!(
                            "DXGI: {}",
                            String::from_utf8_lossy(std::slice::from_raw_parts(
                                (*p).pDescription as _,
                                (*p).DescriptionByteLength
                            ))
                        );
                    }
                }
            }
            return Err(e.into());
        }
        let step = self
            .device
            .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
            as usize;
        let start = self.rtv.GetCPUDescriptorHandleForHeapStart();
        for i in 0..2 {
            let b: ID3D12Resource = self.swap.GetBuffer(i)?;
            self.device.CreateRenderTargetView(
                &b,
                None,
                D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: start.ptr + i as usize * step,
                },
            );
            self.buffers.push(b);
        }
        self.swap_size = [w, h];
        Ok(())
    }
    /// Re-creates the output and render-size textures; the swapchain keeps its size.
    pub unsafe fn resize_output(&mut self, w: u32, h: u32, s: &Settings) -> Result<()> {
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "Invalid output size"
        );
        self.wait_idle()?;
        self.begin()?;
        self.list.Close()?;
        self.release_upscalers();
        self.textures.clear();
        self.width = w;
        self.height = h;
        self.rw = ((w as f32 * s.scale).round() as u32).max(1);
        self.rh = ((h as f32 * s.scale).round() as u32).max(1);
        for i in 0..TEXTURES {
            let (format, [tw, th]) = crate::render::texture_layout(
                i,
                self.hdr_active && self.hdr10,
                [w, h],
                [self.rw, self.rh],
            );
            let format = match format {
                TexFormat::Rgba32F => DXGI_FORMAT_R32G32B32A32_FLOAT,
                TexFormat::R32F => DXGI_FORMAT_R32_FLOAT,
                TexFormat::Rg32F => DXGI_FORMAT_R32G32_FLOAT,
                TexFormat::Rgba8 => DXGI_FORMAT_R8G8B8A8_UNORM,
                TexFormat::Rgb10A2 => DXGI_FORMAT_R10G10B10A2_UNORM,
                TexFormat::Rgba16F => DXGI_FORMAT_R16G16B16A16_FLOAT,
                TexFormat::R32U => DXGI_FORMAT_R32_UINT,
            };
            let t = texture(
                &self.device,
                tw,
                th,
                format,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )?;
            let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: format,
                ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
                Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_UAV::default(),
                },
            };
            self.device
                .CreateUnorderedAccessView(&t.resource, None, Some(&desc), self.cpu(i));
            self.textures.push(t);
        }
        self.lut_view();
        self.env_views();
        self.rr_active = false;
        self.reference_valid = false;
        self.integrated_valid = false;
        self.reset(true);
        Ok(())
    }
    pub(super) fn cpu(&self, index: usize) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        unsafe {
            D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + index
                        * self.device.GetDescriptorHandleIncrementSize(
                            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        ) as usize,
            }
        }
    }
    pub(super) fn gpu(&self, index: usize) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        unsafe {
            D3D12_GPU_DESCRIPTOR_HANDLE {
                ptr: self.heap.GetGPUDescriptorHandleForHeapStart().ptr
                    + index as u64
                        * self.device.GetDescriptorHandleIncrementSize(
                            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        ) as u64,
            }
        }
    }
}
/// Whether `format` supports typed UAV stores on this adapter (D3D12_FORMAT_SUPPORT2).
unsafe fn typed_store(device: &ID3D12Device5, format: DXGI_FORMAT) -> bool {
    let mut f = D3D12_FEATURE_DATA_FORMAT_SUPPORT {
        Format: format,
        ..Default::default()
    };
    unsafe {
        device
            .CheckFeatureSupport(
                D3D12_FEATURE_FORMAT_SUPPORT,
                &mut f as *mut _ as _,
                size_of_val(&f) as u32,
            )
            .is_ok()
            && (f.Support2.0 & D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE.0) != 0
    }
}
