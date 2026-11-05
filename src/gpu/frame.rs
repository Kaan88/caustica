//! The frame: trace, reconstruction, display, upscalers and the egui pass.
use super::*;

impl Gpu {
    pub unsafe fn render(
        &mut self,
        s: &Settings,
        draw: bool,
        dt: f32,
        primitives: &[egui::ClippedPrimitive],
        delta: &egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<()> {
        self.begin()?;
        let mut keep = Vec::new();
        self.update_gui_textures(delta, &mut keep)?;
        let (view, proj, eye) = s.camera(self.width as f32 / self.height as f32);
        let vp = proj * view;
        let reset = self.rr_reset;
        self.poll_ngx(false);
        // RR and the temporal upscalers need a perspective camera (motion vectors, guides).
        self.rr_active = s.rr && !self.ngx.is_null() && !self.rr_failed && s.perspective() && !self.painting
                // Battle mode compares raw samples.
                && !s.battle;
        if !s.rr && !self.ngx.is_null() {
            self.rr_status = "DLSS RR ready (disabled)".into();
        }
        if self.rr_active && self.rr_created {
            self.rr_status = "DLSS Ray Reconstruction active".into();
        }
        if self.rr_active && !self.rr_created {
            let code = caustica_ngx_create(
                self.ngx,
                self.list.as_raw(),
                self.rw,
                self.rh,
                self.width,
                self.height,
                s.rr_preset,
            );
            if ngx_ok(code) {
                self.rr_created = true;
                self.rr_status = "DLSS Ray Reconstruction active".into();
            } else {
                self.rr_active = false;
                self.rr_failed = true;
                self.rr_status =
                    format!("RR creation failed: 0x{code:08x}; native denoiser active");
            }
        }
        self.prepare_lpe(s)?;
        self.prepare_lens(s)?;
        let restir = self.prepare_restir(s)?;
        let sr = self.prepare_upscaler(s);
        // With a temporal upscaler each frame is fresh jittered samples, as with RR: the native
        // denoiser's history and the upscaler do the accumulation, so detail is resolved above
        // the render resolution. (Feeding an accumulated image measured worse: the upscalers
        // assume every input frame sits at its own jitter.)
        // Once the view has been still for SR_FRESH_FRAMES the upscaler's short history has
        // done what it can, so the samples accumulate from then on and converge to the cap.
        let accumulating = sr.is_some() && self.sr_fresh >= SR_FRESH_FRAMES;
        if sr.is_some() && draw && !accumulating {
            if self.samples > 0 {
                let fresh = self.sr_fresh;
                self.reset(false);
                self.sr_fresh = fresh;
            }
            self.sr_fresh += 1;
        }
        let jittered = self.rr_active || sr.is_some();
        let jitter = if jittered {
            [
                halton(self.frame % 64 + 1, 2) - 0.5,
                halton(self.frame % 64 + 1, 3) - 0.5,
            ]
        } else {
            [0., 0.]
        };
        // Display flags: bit 0 jittered primary rays, bit 1 show the upscaled texture (14),
        // bits 2-3 the spatial upscale filter (1 bilinear, 2 bicubic), bits 4-5 light sampling,
        // bit 6 equiangular sampling in fog, bit 7 write the object ids: unjittered, or the
        // first frame after a real restart (not the upscaler's per-frame soft ones), so edge
        // ids and the outlines don't follow the jitter.
        let spatial = if s.upscaler == Upscaler::Bilinear {
            1
        } else {
            2
        };
        let flags = jittered as u32
            | ((self.rr_active || sr.is_some()) as u32) << 1
            | spatial << 2
            | s.mis_mode.min(2) << 4
            | ((s.fog.equiangular && s.fog.density > 0.) as u32) << 6
            | ((!jittered || (self.samples == 0 && self.sr_fresh <= 1)) as u32) << 7;
        let n = if draw {
            s.spp.min(s.max_samples.saturating_sub(self.samples))
        } else {
            0
        };
        // The physical sky (track 9) is baked at strength 1 with north at −z.
        let (env_strength, env_rotation, env_visible) = if s.physical_sky.on {
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
            &crate::render::FrameInputs {
                view,
                proj,
                eye,
                previous_vp: if reset { vp } else { self.previous_vp },
                n,
                reset,
                flags,
                jitter,
                sequence_base: self.sequence_base,
                lut_size: self.lut_size,
                present_count: self.present_count,
                env_scale: self.env_scale,
                env_inv_norm: self.env_inv_norm,
                env_sun: self.env_sun,
                env: (env_strength, env_rotation, env_visible),
                lens_surfaces: self.lens_surfaces,
                glare_grid: self.glare_grid(),
                restir: restir as u32 | (self.restir_history as u32) << 1 | self.restir_layers << 8,
            },
        );
        // Path visualizer: the output pixel maps to a render pixel as the light meter's does.
        let path_request = self.path_request.take();
        if let Some(([x, y], count)) = path_request {
            c.path_record = [
                (x * self.rw / self.width.max(1)).min(self.rw.saturating_sub(1)),
                (y * self.rh / self.height.max(1)).min(self.rh.saturating_sub(1)),
                count,
                PATH_CAPACITY as u32,
            ];
        }
        // eye.w scales the UI to paper white in HDR (gui.hlsl); the trace ignores it.
        // Negative: HDR10, so gui.hlsl encodes PQ / BT.2020 instead of scRGB.
        if self.hdr_active {
            c.eye[3] = self.hdr_levels[0] / 80. * if self.hdr10 { -1. } else { 1. };
        }
        write_buffer(&self.constants, 0, bytemuck::bytes_of(&c))?;
        self.stamp(0);
        if n == 0 {
            self.stamp(1);
        }
        if path_request.is_some() {
            // Writes only the path records, so it may run before this frame's trace.
            self.bind(0);
            self.list.SetPipelineState(&self.trace_paths);
            self.list.Dispatch(1, 1, 1);
            uav(&self.list, None);
            transition(
                &self.list,
                &self.path_buffer,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            self.list
                .CopyResource(&self.path_readback, &self.path_buffer);
            transition(
                &self.list,
                &self.path_buffer,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        }
        self.prepare_aux(s, &mut keep)?;
        self.restir_ran = n > 0 && restir && (s.battle || s.integrator == 0);
        if n > 0 {
            if s.battle {
                self.battle_frame(s, &c, restir)?;
            } else {
                let path_restir = restir && s.integrator == 0;
                self.integrate(
                    s,
                    &c,
                    0,
                    s.integrator,
                    path_restir,
                    [0, 0, self.rw, self.rh],
                    n,
                    1.,
                )?;
                if path_restir {
                    self.restir_history = true;
                }
            }
            self.stamp(1);
            if self.rr_active {
                let inputs = [0, 2, 3, 4, 5, 6, 7];
                for i in inputs {
                    transition(
                        &self.list,
                        &self.textures[i].resource,
                        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    );
                }
                let resources: Vec<_> = self.textures.iter().map(|t| t.resource.as_raw()).collect();
                let code = caustica_ngx_eval(
                    self.ngx,
                    self.list.as_raw(),
                    resources.as_ptr(),
                    self.rw,
                    self.rh,
                    jitter[0],
                    jitter[1],
                    (reset || !s.temporal) as i32,
                    dt * 1000.,
                    view.to_cols_array().as_ptr(),
                    proj.to_cols_array().as_ptr(),
                );
                for i in inputs {
                    transition(
                        &self.list,
                        &self.textures[i].resource,
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    );
                }
                uav(&self.list, None);
                if !ngx_ok(code) {
                    self.rr_active = false;
                    self.rr_failed = true;
                    self.rr_status = format!("RR evaluation failed: 0x{code:08x}");
                    eprintln!("{}", self.rr_status);
                }
            }
            self.previous_vp = vp;
            self.rr_reset = false;
        }
        if !self.rr_active && s.denoise && !s.battle {
            // Pass 0 warps the last integrated frame into this view after a camera move; a
            // still camera keeps that history frozen. Pass 1 integrates new samples (skipped
            // while paused). Passes 2–6 are the à-trous filter.
            let traced = n > 0;
            let reproject = traced && self.camera_moved && s.temporal && self.integrated_valid;
            if traced && self.camera_moved {
                self.history_valid = reproject;
            }
            if !s.temporal {
                self.history_valid = false;
            }
            self.denoiser_activity = if reproject {
                "reprojecting its history (camera moving)"
            } else if !traced {
                "filtering the held result (no new samples)"
            } else if self.history_valid {
                "integrating new samples over its history"
            } else if !s.temporal {
                "integrating new samples (temporal history off)"
            } else {
                "integrating from scratch (no usable history: an edit, a load or a resize)"
            };
            let first = if reproject {
                0
            } else if traced || !self.integrated_valid {
                1
            } else {
                2
            };
            for i in first..7 {
                c.filter[0] = i;
                c.filter[2] = !self.history_valid as u32;
                let offset = (i as u64 + 1) * CB_STRIDE;
                write_buffer(&self.constants, offset, bytemuck::bytes_of(&c))?;
                self.bind(offset);
                self.list.SetPipelineState(&self.filter);
                self.list
                    .Dispatch(self.rw.div_ceil(8), self.rh.div_ceil(8), 1);
                uav(&self.list, None);
                if i == 1 && traced {
                    // The next reprojection checks history against these guides.
                    for (from, to) in [(2, 16), (5, 17), (19, 20)] {
                        copy_texture(
                            &self.list,
                            &self.textures[from].resource,
                            &self.textures[to].resource,
                        );
                    }
                }
            }
            self.integrated_valid = true;
        } else if n > 0 {
            self.history_valid = false;
            self.integrated_valid = false;
        }
        if self.rr_active {
            self.denoiser_activity = if n == 0 {
                "holding (no new samples)"
            } else if reset {
                "history reset (scene edited or camera cut)"
            } else {
                "accumulating over its history"
            };
        } else if !s.denoise || s.battle {
            self.denoiser_activity = "";
        }
        if n > 0 {
            self.camera_moved = false;
        }
        if let Some(kind) = sr
            && (n > 0 || self.sr_reset)
        {
            // An accumulated input is the mean over every jitter phase, so it sits at the pixel centre.
            let jitter = if accumulating { [0., 0.] } else { jitter };
            self.run_upscaler(kind, s, &mut c, jitter, (self.sr_reset || reset) as i32, dt);
        }
        self.stamp(2);
        c.modes[3] = self.rr_active as u32;
        c.filter[0] = 0;
        if s.glare > 0. {
            self.run_glare(s, &mut c)?;
        }
        write_buffer(&self.constants, 8 * CB_STRIDE, bytemuck::bytes_of(&c))?;
        if self.scopes_on {
            self.clear_scopes()?;
        }
        self.bind(8 * CB_STRIDE);
        self.list.SetPipelineState(&self.display);
        self.list
            .Dispatch(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        uav(&self.list, None);
        let probe = self
            .probe
            .filter(|p| p[0] < self.width && p[1] < self.height);
        if self.scopes_on || probe.is_some() {
            self.queue_readbacks(probe)?;
        }
        self.stamp(3);
        let index = self.swap.GetCurrentBackBufferIndex() as usize;
        let back = &self.buffers[index];
        let shown = if self.hdr_active { 26 } else { 10 };
        let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.rtv.GetCPUDescriptorHandleForHeapStart().ptr
                + index
                    * self
                        .device
                        .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                        as usize,
        };
        if self.swap_size == [self.width, self.height] {
            transition(
                &self.list,
                &self.textures[shown].resource,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            self.list.CopyResource(back, &self.textures[shown].resource);
            transition(
                &self.list,
                &self.textures[shown].resource,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
        } else {
            // A video export renders at its own size: the window shows the UI over a plain
            // background (the progress window has its own preview).
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            let grey = if self.hdr_active { 0.02 } else { 0.1 };
            self.list
                .ClearRenderTargetView(handle, &[grey, grey, grey, 1.], None);
        }
        self.list.OMSetRenderTargets(1, Some(&handle), false, None);
        self.draw_gui(primitives, pixels_per_point, &mut keep)?;
        self.stamp(4);
        if !(s.battle && n > 0) {
            // Every query the resolve reads must have been ended this frame.
            for i in BATTLE_QUERY..TIMESTAMPS {
                self.list
                    .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, i);
            }
        }

        self.list.ResolveQueryData(
            &self.query,
            D3D12_QUERY_TYPE_TIMESTAMP,
            0,
            TIMESTAMPS,
            &self.timing,
            0,
        );
        // Freed textures go after the draw that may still use them this frame; one frame in
        // flight means the slot is free again by the next one.
        for id in &delta.free {
            if let Some(old) = self.gui_textures.remove(id) {
                self.free_slots.push(old.slot);
                keep.push(old.texture.resource);
            }
        }
        if self.keep_ui_frame {
            if self.ui_frame.is_none() {
                self.ui_frame = Some(
                    texture(
                        &self.device,
                        self.swap_size[0],
                        self.swap_size[1],
                        self.swap_format(),
                        D3D12_RESOURCE_FLAG_NONE,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                    )?
                    .resource,
                );
            }
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            self.list
                .CopyResource(self.ui_frame.as_ref().unwrap(), back);
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            );
        } else {
            transition(
                &self.list,
                back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );
        }
        self.submit()?;
        self.swap.Present(s.vsync as u32, DXGI_PRESENT(0)).ok()?;
        self.finish_readbacks(probe)?;
        if path_request.is_some() {
            let mut p = null_mut();
            self.path_readback.Map(0, None, Some(&mut p))?;
            let all = std::slice::from_raw_parts(p as *const crate::paths::Record, PATH_CAPACITY);
            let count = (all[0].info as usize).min(PATH_CAPACITY - 1);
            self.path_result = Some(all[1..=count].to_vec());
            self.path_readback.Unmap(0, None);
        }
        self.present_count = self.present_count.wrapping_add(1);
        let mut data = null_mut();
        let count = TIMESTAMPS as usize;
        self.timing.Map(
            0,
            Some(&D3D12_RANGE {
                Begin: 0,
                End: 8 * count,
            }),
            Some(&mut data),
        )?;
        let times = std::slice::from_raw_parts(data as *const u64, count);
        let ms = 1000. / self.queue.GetTimestampFrequency()? as f64;
        let span = |a: usize, b: usize| times[b].saturating_sub(times[a]) as f64 * ms;
        // Without the UI, as before: trace to the end of display.
        self.gpu_ms = span(0, 3);
        self.pass_ms = [span(0, 1), span(1, 2), span(2, 3), span(3, 4)];
        if s.battle {
            for (i, q) in self.battle.iter_mut().enumerate() {
                if q.last == 0 {
                    continue;
                }
                let ms = span(
                    BATTLE_QUERY as usize + 2 * i,
                    BATTLE_QUERY as usize + 2 * i + 1,
                );
                let per = ms / (q.last as f64 * q.scale.max(1.));
                q.cost = if q.cost > 0. {
                    0.8 * q.cost + 0.2 * per
                } else {
                    per
                };
                q.ms += ms;
                q.samples += q.last;
                q.last = 0;
            }
            self.samples = self.battle.iter().map(|q| q.samples).min().unwrap_or(0);
        } else {
            self.samples += n;
        }
        self.timing.Unmap(0, None);
        if n > 0 {
            self.frame = self.frame.wrapping_add(1);
        }
        Ok(())
    }
    /// Upscales the native color (denoised or raw) with device depth and low-res pixel motion
    /// into texture 14. On failure the display falls back to bicubic.
    pub(super) unsafe fn run_upscaler(
        &mut self,
        kind: Upscaler,
        s: &Settings,
        c: &mut Constants,
        jitter: [f32; 2],
        history_reset: i32,
        dt: f32,
    ) {
        let color = if s.denoise { 8 } else { 1 };
        let inputs = [color, 25, 6];
        for i in inputs {
            transition(
                &self.list,
                &self.textures[i].resource,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
            );
        }
        let r = |i: usize| self.textures[i].resource.as_raw();
        let (cmd, w, h) = (self.list.as_raw(), self.rw, self.rh);
        let code = match kind {
            Upscaler::Dlss => {
                let code = caustica_ngx_eval_sr(
                    self.ngx,
                    cmd,
                    r(color),
                    r(25),
                    r(6),
                    r(14),
                    w,
                    h,
                    jitter[0],
                    jitter[1],
                    history_reset,
                    dt * 1000.,
                );
                if ngx_ok(code) { 0 } else { code }
            }
            Upscaler::Fsr => caustica_fsr_eval(
                self.fsr,
                cmd,
                r(color),
                r(25),
                r(6),
                r(14),
                w,
                h,
                self.width,
                self.height,
                jitter[0],
                jitter[1],
                history_reset,
                dt * 1000.,
                s.fov.to_radians(),
                0.01,
                500.,
            ),
            _ => caustica_xess_eval(
                self.xess,
                cmd,
                r(color),
                r(25),
                r(6),
                r(14),
                w,
                h,
                jitter[0],
                jitter[1],
                history_reset,
            ),
        };
        for i in inputs {
            transition(
                &self.list,
                &self.textures[i].resource,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
        }
        uav(&self.list, None);
        self.sr_reset = false;
        if code != 0 {
            self.sr_failed.push(kind);
            self.sr_created = None;
            c.scene_info[3] &= !2;
            self.upscale_status =
                format!("{} failed (0x{code:08x}); bicubic upscaling", kind.label());
            eprintln!("{}", self.upscale_status);
        }
    }
    pub(super) unsafe fn update_gui_textures(
        &mut self,
        delta: &egui::TexturesDelta,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        for (id, change) in &delta.set {
            let size = change.image.size();
            let pixels: Vec<egui::Color32> = match &change.image {
                egui::ImageData::Color(i) => i.pixels.clone(),
                egui::ImageData::Font(i) => i.srgba_pixels(None).collect(),
            };
            let (mut data, mut full, slot) = match self.gui_textures.remove(id) {
                Some(old) => {
                    keep.push(old.texture.resource);
                    (old.pixels, old.size, old.slot)
                }
                None => (
                    vec![],
                    [0, 0],
                    self.free_slots.pop().context("Out of egui texture slots")?,
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
            let t = texture(
                &self.device,
                full[0] as _,
                full[1] as _,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            )?;
            let (layout, bytes) = footprint(&self.device, &t.resource);
            let staging = buffer(
                &self.device,
                bytes,
                D3D12_HEAP_TYPE_UPLOAD,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_GENERIC_READ,
            )?;
            for y in 0..full[1] {
                let row = &data[y * full[0] * 4..(y + 1) * full[0] * 4];
                write_buffer(&staging, y as u64 * layout.Footprint.RowPitch as u64, row)?;
            }
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
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
            let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: t.format,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MipLevels: 1,
                        ..Default::default()
                    },
                },
            };
            self.device
                .CreateShaderResourceView(&t.resource, Some(&srv), self.cpu(slot));
            self.gui_textures.insert(
                *id,
                GuiTexture {
                    texture: t,
                    slot,
                    pixels: data,
                    size: full,
                },
            );
            keep.push(staging);
        }
        Ok(())
    }
    pub(super) unsafe fn draw_gui(
        &self,
        primitives: &[egui::ClippedPrimitive],
        ppp: f32,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        if self.gui_textures.is_empty() {
            return Ok(());
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct GuiVertex {
            position: [f32; 2],
            uv: [f32; 2],
            color: [u8; 4],
        }
        self.list.SetDescriptorHeaps(&[Some(self.heap.clone())]);
        self.list.SetGraphicsRootSignature(&self.root);
        self.list
            .SetGraphicsRootConstantBufferView(0, self.constants.GetGPUVirtualAddress());
        // The UAVs, for the silhouette outlines' object ids (gui.hlsl).
        self.list.SetGraphicsRootDescriptorTable(3, self.gpu(0));
        self.list
            .SetPipelineState(if self.hdr_active && self.hdr10 {
                &self.gui_hdr10
            } else if self.hdr_active {
                &self.gui_hdr
            } else {
                &self.gui
            });
        self.list
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.list.RSSetViewports(&[D3D12_VIEWPORT {
            Width: self.swap_size[0] as f32,
            Height: self.swap_size[1] as f32,
            MaxDepth: 1.,
            ..Default::default()
        }]);
        // One vertex and one index upload per frame; each mesh draws from its offsets.
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        // gui.hlsl maps positions by the output size in the frame constants; during a video
        // export that differs from the swapchain, so positions are scaled to match.
        let fit = [
            ppp * self.width as f32 / self.swap_size[0] as f32,
            ppp * self.height as f32 / self.swap_size[1] as f32,
        ];
        for primitive in primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive
                && !mesh.indices.is_empty()
                && let Some(texture) = self.gui_textures.get(&mesh.texture_id)
            {
                draws.push((
                    primitive.clip_rect,
                    indices.len() as u32,
                    mesh.indices.len() as u32,
                    vertices.len() as i32,
                    texture.slot,
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
        let vb = upload(&self.device, bytemuck::cast_slice(&vertices))?;
        let ib = upload(&self.device, bytemuck::cast_slice(&indices))?;
        self.list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: vb.GetGPUVirtualAddress(),
                SizeInBytes: (vertices.len() * size_of::<GuiVertex>()) as _,
                StrideInBytes: size_of::<GuiVertex>() as _,
            }]),
        );
        self.list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: ib.GetGPUVirtualAddress(),
            SizeInBytes: (indices.len() * 4) as _,
            Format: DXGI_FORMAT_R32_UINT,
        }));
        let mut bound = usize::MAX;
        for (r, first, count, base, slot) in draws {
            let [sw, sh] = self.swap_size.map(|v| v as f32);
            let left = (r.min.x * ppp).clamp(0., sw) as i32;
            let top = (r.min.y * ppp).clamp(0., sh) as i32;
            let right = (r.max.x * ppp).clamp(left as f32, sw) as i32;
            let bottom = (r.max.y * ppp).clamp(top as f32, sh) as i32;
            // Fully clipped meshes (e.g. a window outside a small swapchain) draw nothing,
            // and the debug layer warns about an empty scissor.
            if right <= left || bottom <= top {
                continue;
            }
            self.list
                .RSSetScissorRects(&[windows::Win32::Foundation::RECT {
                    left,
                    top,
                    right,
                    bottom,
                }]);
            if slot != bound {
                self.list.SetGraphicsRootDescriptorTable(4, self.gpu(slot));
                bound = slot;
            }
            self.list.DrawIndexedInstanced(count, 1, first, base, 0);
        }
        keep.push(vb);
        keep.push(ib);
        Ok(())
    }
}
