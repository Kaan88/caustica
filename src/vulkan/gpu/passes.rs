//! Per-frame passes and the scene resources they need, as D3D12's gpu/passes.rs: aux buffers,
//! integrators, battle mode, ReSTIR, glare, lens, LPE, texture maps, environment, LUT.
use super::*;
use glam::Vec3;

impl Gpu {
    /// Binds set 0-2 with the constants at `offset`, then one dispatch and a full barrier.
    pub(super) unsafe fn dispatch(&self, pipe: &str, offset: u64, groups: [u32; 3]) {
        let (d, cmd) = (&self.device, self.frame.cmd);
        unsafe {
            self.layout.bind(d, cmd, offset as u32);
            d.device
                .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipes[pipe]);
            d.device.cmd_dispatch(cmd, groups[0], groups[1], groups[2]);
            full_barrier(&d.device, cmd);
        }
    }

    /// Points a space-2 binding at `buffer`, or the path buffer while there is none.
    pub(super) unsafe fn bind_space2(&self, binding: u32, buffer: Option<&Buffer>) {
        let b = buffer.unwrap_or(&self.path_buffer);
        unsafe { self.layout.buffer(&self.device, 2, binding, b.handle) };
    }

    /// A zeroed device-local storage buffer.
    pub(super) unsafe fn storage(&self, len: u64) -> Result<Buffer> {
        unsafe {
            let d = &self.device;
            let b = d.buffer(len, vk::BufferUsageFlags::STORAGE_BUFFER, false)?;
            d.one_shot(|cmd| {
                d.device
                    .cmd_fill_buffer(cmd, b.handle, 0, vk::WHOLE_SIZE, 0)
            })?;
            Ok(b)
        }
    }

    /// Sizes `aux` (u3 space 2) for the integrators in use (D3D12's `prepare_aux`).
    pub(super) unsafe fn prepare_aux(&mut self, s: &Settings) -> Result<()> {
        let photons = s.integrator == 2 || s.battle;
        let splats = s.integrator == 1 || s.battle;
        let photon_bytes = if photons {
            PHOTON_BASE + s.photons as u64 * s.bounces.clamp(1, PHOTON_BOUNCES) as u64 * 32
        } else {
            0
        };
        let splat_bytes = if splats {
            self.st.rw as u64 * self.st.rh as u64 * 12
        } else {
            0
        };
        let len = photon_bytes + splat_bytes;
        if len == self.aux_len && photon_bytes == self.splat_offset {
            return Ok(());
        }
        self.splat_offset = photon_bytes;
        self.aux_len = len;
        unsafe {
            if let Some(old) = self.aux.take() {
                self.device.free_buffer(old);
            }
            if len > 0 {
                self.aux = Some(self.storage(len)?);
            }
            self.bind_space2(3, self.aux.as_ref());
        }
        Ok(())
    }

    /// One integrator over `region` with the constants `c` at `offset` (D3D12's `integrate`).
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn integrate(
        &mut self,
        s: &Settings,
        c: &Constants,
        offset: u64,
        integrator: u32,
        restir: bool,
        region: [u32; 4],
        n: u32,
        photon_boost: f32,
    ) -> Result<()> {
        let mut c = *c;
        if integrator == 2 && !self.bounds_valid {
            let (mut lo, mut hi) = s.objects_bounds();
            for v in self.static_cache.iter().filter(|v| v.material != 3) {
                lo = lo.min(Vec3::from(v.position));
                hi = hi.max(Vec3::from(v.position));
            }
            if s.fog.density > 0. && lo.is_finite() {
                let f = &s.fog;
                lo = lo.min(Vec3::new(-f.extent, -1., -f.extent));
                hi = hi.max(Vec3::new(f.extent, f.height, f.extent));
            }
            self.bounds = if lo.is_finite() {
                let c = (lo + hi) * 0.5;
                [c.x, c.y, c.z, ((hi - lo).length() * 0.5).max(0.5)]
            } else {
                [0., 0., 0., 2.]
            };
            self.bounds_valid = true;
        }
        c.bounds = self.bounds;
        let (w, h) = (region[2] - region[0], region[3] - region[1]);
        let share = (w * h) as f32 / (self.st.rw * self.st.rh).max(1) as f32;
        c.region = region;
        c.sampling[1] = n;
        if !restir {
            c.restir[0] = 0;
        }
        let photons = ((s.photons as f32 * share * photon_boost) as u32).clamp(1024, s.photons);
        let capacity = (self.splat_offset.saturating_sub(PHOTON_BASE) / 32) as u32;
        let light_tracing = s.perspective() && s.aperture <= 0. && c.scene_info[3] & 1 == 0;
        c.integrator = [
            photons,
            capacity,
            self.splat_offset as u32,
            light_tracing as u32,
        ];
        c.photon = [
            s.photon_radius * self.bounds[3] * self.photon_scale.sqrt(),
            4. * c.environment[1] * c.environment[2] * share,
            self.sdf_emitters[0] as f32,
            self.sdf_emitters[1] as f32,
        ];
        unsafe {
            self.device
                .write(&self.constants, offset, bytemuck::bytes_of(&c))?
        };
        let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
        unsafe {
            match integrator {
                1 => {
                    let bdpt = if s.spectral { "bdpt_spectral" } else { "bdpt" };
                    for pass in [bdpt, "bdpt_resolve"] {
                        self.dispatch(pass, offset, [gx, gy, 1]);
                    }
                }
                2 => {
                    self.dispatch("photon_clear", offset, [(PHOTON_HASH / 64) as u32, 1, 1]);
                    let trace = if s.spectral {
                        "photon_trace_spectral"
                    } else {
                        "photon_trace"
                    };
                    self.dispatch(trace, offset, [photons.div_ceil(64), 1, 1]);
                    let gather = if s.spectral {
                        "photon_gather_spectral"
                    } else {
                        "photon_gather"
                    };
                    self.dispatch(gather, offset, [gx, gy, 1]);
                    // Knaus and Zwicker's radius schedule, alpha = 2/3.
                    self.photon_frames += 1;
                    let i = self.photon_frames as f32;
                    self.photon_scale *= (i + 2. / 3.) / (i + 1.);
                }
                _ => {
                    if restir {
                        self.dispatch(
                            "restir_presample",
                            offset,
                            [(RESTIR_TILES / 64) as u32, 1, 1],
                        );
                        for pass in ["restir_initial", "restir_spatial"] {
                            self.dispatch(pass, offset, [gx, gy, self.restir_layers]);
                        }
                    }
                    // LEAN exactly when the shader's fog and ReSTIR flags are off this frame.
                    let lean = self.lean_scene && c.fog[0] <= 0. && c.restir[0] & 1 == 0;
                    let spectral = s.spectral && self.st.lpe_slots == 0;
                    let pipe = if self.st.lpe_slots > 0 {
                        "trace_lpe"
                    } else if spectral && lean {
                        "trace_spectral_lean"
                    } else if spectral {
                        "trace_spectral"
                    } else {
                        ["trace", "trace_maps", "trace_lean", "trace_maps_lean"]
                            [self.textured as usize + 2 * lean as usize]
                    };
                    self.dispatch(pipe, offset, [gx, gy, 1]);
                }
            }
        }
        Ok(())
    }

    /// Battle mode's four quadrants at equal GPU time (D3D12's `battle_frame`).
    pub(super) unsafe fn battle_frame(
        &mut self,
        s: &Settings,
        c: &Constants,
        restir: bool,
    ) -> Result<()> {
        let (rw, rh) = (self.st.rw, self.st.rh);
        let (hw, hh) = (rw / 2, rh / 2);
        let regions = [
            [0, 0, hw, hh],
            [hw, 0, rw, hh],
            [0, hh, hw, rh],
            [hw, hh, rw, rh],
        ];
        let slowest = self.st.battle.iter().map(|q| q.cost).fold(0., f64::max);
        let budget = slowest * s.spp as f64;
        for (i, region) in regions.into_iter().enumerate() {
            let q = &mut self.st.battle[i];
            let want = if q.cost > 0. && budget > 0. {
                budget / q.cost + q.carry
            } else {
                s.spp as f64
            };
            let photons = i == 3;
            let spp = if photons {
                1
            } else {
                (want.floor() as u32).clamp(1, 64)
            };
            q.scale = if photons { want.clamp(1., 4.) } else { 1. };
            q.carry = if photons {
                0.
            } else {
                (want - spp as f64).clamp(0., 1.)
            };
            let n = spp.min(s.max_samples.saturating_sub(q.samples));
            q.last = n;
            let samples = q.samples;
            let query = BATTLE_QUERY + 2 * i as u32;
            unsafe { self.stamp(query) };
            if n > 0 {
                let mut qc = *c;
                qc.sampling[0] = samples;
                let offset = (9 + i as u64) * CB_STRIDE;
                let scale = self.st.battle[i].scale as f32;
                unsafe {
                    self.integrate(
                        s,
                        &qc,
                        offset,
                        [0, 0, 1, 2][i],
                        i == 1 && restir,
                        region,
                        n,
                        scale,
                    )?
                };
            }
            unsafe { self.stamp(query + 1) };
        }
        if restir && self.st.battle[1].last > 0 {
            self.restir_history = true;
        }
        Ok(())
    }

    /// Reservoir layers of the last frame's ReSTIR passes (0 when they did not run).
    pub fn restir_layers(&self) -> u32 {
        if self.restir_ran {
            self.restir_layers
        } else {
            0
        }
    }

    /// Sizes the ReSTIR reservoirs (D3D12's `prepare_restir`). True when the passes run.
    pub(super) unsafe fn prepare_restir(&mut self, s: &Settings) -> Result<bool> {
        let wanted = (s.restir && s.integrator == 0) || s.battle;
        let on = wanted && self.st.emitter_count > 0;
        let pixels = (self.st.rw as u64 * self.st.rh as u64).max(1);
        self.restir_layers = s
            .spp
            .clamp(1, 4)
            .min(((1 << 30) / (pixels * 64)).max(1) as u32);
        let len = if wanted {
            pixels * self.restir_layers as u64 * 64 + RESTIR_TILES * 32
        } else {
            0
        };
        if len != self.reservoirs_len {
            unsafe {
                if let Some(old) = self.reservoirs.take() {
                    self.device.free_buffer(old);
                }
                if len > 0 {
                    self.reservoirs = Some(self.storage(len)?);
                }
                self.bind_space2(2, self.reservoirs.as_ref());
            }
            self.reservoirs_len = len;
            self.restir_history = false;
        }
        Ok(on)
    }

    /// The image's size in the glare grid, w | h << 16 (the longer side GLARE_IMAGE).
    pub(super) fn glare_grid(&self) -> u32 {
        let aspect = self.st.width as f32 / self.st.height.max(1) as f32;
        let (w, h) = if aspect >= 1. {
            (GLARE_IMAGE, GLARE_IMAGE / aspect)
        } else {
            (GLARE_IMAGE * aspect, GLARE_IMAGE)
        };
        (w.round().max(1.) as u32) | (h.round().max(1.) as u32) << 16
    }

    /// Track 16 glare (D3D12's `run_glare`).
    pub(super) unsafe fn run_glare(&mut self, s: &Settings, c: &mut Constants) -> Result<()> {
        let key = [
            s.aperture_blades as f32,
            s.blade_rotation,
            c.glare_settings[2],
        ];
        let mut passes = Vec::new();
        if self.glare_kernel != Some(key) {
            passes.extend([
                [1, 0, 0],
                [2, 0, 0],
                [3, 0, 0],
                [4, 0, 0],
                [2, 1, 0],
                [3, 1, 0],
            ]);
            self.glare_kernel = Some(key);
        }
        passes.extend([[0, 0, 0], [2, 0, 0], [3, 0, 2], [2, 0, 1]]);
        for (i, p) in passes.iter().enumerate() {
            c.glare[..3].copy_from_slice(p);
            let offset = (GLARE_SLOT + i as u64) * CB_STRIDE;
            unsafe {
                self.device
                    .write(&self.constants, offset, bytemuck::bytes_of(c))?;
                self.dispatch("glare", offset, [GLARE_N as u32, 1, 1]);
            }
        }
        c.glare[..3].copy_from_slice(&[0; 3]);
        Ok(())
    }

    /// Rebuilds the lens system's buffer when it changes (D3D12's `prepare_lens`).
    pub(super) unsafe fn prepare_lens(&mut self, s: &Settings) -> Result<()> {
        if !s.real_lens() {
            self.lens_surfaces = 0;
            return Ok(());
        }
        let p = crate::lens::Params {
            lens: s.lens_system,
            f_number: s.f_number,
            focus_mm: s.focus_distance * s.lens_scale * 1000.,
            fov: s.fov.to_radians(),
            aspect: self.st.rw as f32 / self.st.rh.max(1) as f32,
            dispersion: s.dispersion,
        };
        if self.lens_params.as_ref() != Some(&p) || self.lens_surfaces == 0 {
            let (data, n) = crate::lens::pack(&p);
            unsafe {
                self.device
                    .write(&self.lens_buffer, 0, bytemuck::cast_slice(&data))?
            };
            self.lens_surfaces = n as u32;
            self.lens_params = Some(p);
        }
        Ok(())
    }

    /// The light path expressions' table and AOV buffer (D3D12's `prepare_lpe`).
    pub(super) unsafe fn prepare_lpe(&mut self, s: &Settings) -> Result<()> {
        let names = if s.lpe.iter().any(|e| e.contains('\'')) {
            crate::lpe::material_names(s)
        } else {
            vec![]
        };
        if self
            .lpe_uploaded
            .as_ref()
            .is_none_or(|(e, n)| *e != s.lpe || *n != names)
        {
            let (table, slots, labels) = crate::lpe::table(&s.lpe);
            let classes = crate::lpe::classes(&labels, &names);
            unsafe {
                let d = &self.device;
                d.write(&self.lpe_table, 0, bytemuck::cast_slice(&table))?;
                d.write(
                    &self.lpe_table,
                    (table.len() * 4) as u64,
                    bytemuck::cast_slice(&classes),
                )?;
            }
            if self.lpe_uploaded.is_some() && slots > 0 {
                self.reset(false);
            }
            self.st.lpe_slots = slots;
            self.lpe_uploaded = Some((s.lpe.clone(), names));
        }
        let len = self.st.rw as u64 * self.st.rh as u64 * self.st.lpe_slots as u64 * 16;
        if len != self.lpe_aov_len {
            unsafe {
                if let Some(old) = self.lpe_aov.take() {
                    self.device.free_buffer(old);
                }
                if len > 0 {
                    self.lpe_aov = Some(self.storage(len)?);
                }
                self.bind_space2(1, self.lpe_aov.as_ref());
            }
            self.lpe_aov_len = len;
            if len > 0 {
                self.reset(false);
            }
        }
        Ok(())
    }

    /// One light path expression's AOV at render resolution, as RGB floats.
    pub unsafe fn read_lpe(&mut self, slot: u32) -> Result<Vec<f32>> {
        ensure!(slot < self.st.lpe_slots, "LPE slot {slot} is not in use");
        let aov = self.lpe_aov.as_ref().context("No LPE buffer")?;
        unsafe {
            let d = &self.device;
            d.device.device_wait_idle()?;
            let staging = d.buffer(self.lpe_aov_len, vk::BufferUsageFlags::TRANSFER_DST, true)?;
            let region = [vk::BufferCopy::default().size(self.lpe_aov_len)];
            d.one_shot(|cmd| {
                d.device
                    .cmd_copy_buffer(cmd, aov.handle, staging.handle, &region)
            })?;
            let all = std::slice::from_raw_parts(
                staging.mapped as *const [f32; 4],
                (self.lpe_aov_len / 16) as usize,
            );
            let out = all
                .iter()
                .skip(slot as usize)
                .step_by(self.st.lpe_slots as usize)
                .flat_map(|v| [v[0], v[1], v[2]])
                .collect();
            d.free_buffer(staging);
            Ok(out)
        }
    }

    /// Texture maps (core track 5): `render::attach_maps` patches the materials, this uploads
    /// new maps into free `maps[]` slots and releases unused ones.
    pub(super) unsafe fn attach_maps(
        &mut self,
        s: &Settings,
        materials: &mut [crate::lighting::PackedMaterial],
    ) -> Result<()> {
        let instances = std::mem::take(&mut self.instances);
        let used = crate::render::attach_maps(s, &instances, materials, &mut |map| {
            let key = std::sync::Arc::as_ptr(map) as usize;
            if !self.maps.contains_key(&key) {
                let texture = unsafe { self.upload_map(map)? };
                self.maps.insert(key, texture);
            }
            Ok(self.maps[&key].1)
        });
        self.instances = instances;
        let used = used?;
        let stale: Vec<usize> = self
            .maps
            .keys()
            .filter(|k| !used.contains(k))
            .copied()
            .collect();
        for key in stale {
            let (image, slot, _) = self.maps.remove(&key).unwrap();
            unsafe { self.device.free_image(image) };
            self.free_maps.push(slot);
        }
        Ok(())
    }

    /// One map with its mips (sRGB for colour maps) into a free `maps[]` slot.
    unsafe fn upload_map(&mut self, map: &std::sync::Arc<crate::mesh::Map>) -> Result<MapImage> {
        let slot = self
            .free_maps
            .pop()
            .context("Out of texture map slots (4096)")?;
        let levels = 1 + map.mips.len() as u32;
        let format = if map.srgb {
            vk::Format::R8G8B8A8_SRGB
        } else {
            vk::Format::R8G8B8A8_UNORM
        };
        let (w, h) = (map.width, map.height);
        unsafe {
            let d = &self.device;
            let image = d.image_levels([w, h, 1], format, vk::ImageUsageFlags::SAMPLED, levels)?;
            for (level, data) in std::iter::once(&map.rgba).chain(&map.mips).enumerate() {
                let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                d.fill_image(&image, level as u32, [lw, lh, 1], data)?;
            }
            self.layout.map(d, slot, image.view);
            Ok((image, slot, map.clone()))
        }
    }

    /// Uploads a film-look LUT (`size`³ RGB, red fastest); `None` clears it.
    pub unsafe fn set_lut(&mut self, lut: Option<(u32, &[[f32; 3]])>) -> Result<()> {
        unsafe { self.device.device.device_wait_idle()? };
        let Some((size, data)) = lut else {
            self.lut_size = 0;
            return Ok(());
        };
        ensure!(
            data.len() == (size * size * size) as usize,
            "LUT size mismatch"
        );
        let texels: Vec<f32> = data.iter().flat_map(|c| [c[0], c[1], c[2], 1.]).collect();
        unsafe {
            let d = &self.device;
            let image = d.image(
                [size, size, size],
                vk::Format::R32G32B32A32_SFLOAT,
                vk::ImageUsageFlags::STORAGE,
            )?;
            d.fill_image(&image, 0, [size, size, size], bytemuck::cast_slice(&texels))?;
            self.layout.image(d, 29, image.view);
            d.free_image(std::mem::replace(&mut self.fixed[0], image));
        }
        self.lut_size = size;
        Ok(())
    }

    /// Texture maps on the GPU (core track 5).
    pub fn map_count(&self) -> usize {
        self.maps.len()
    }
    pub fn environment_loaded(&self) -> bool {
        self.env_inv_norm > 0.
    }
    /// The catalogue's stars are on the GPU and on in this sky (night sky N3).
    pub fn stars_loaded(&self) -> bool {
        self.env_stars.is_some() && self.star_buffer.is_some()
    }

    /// Uploads an HDRI and its sampling tables (environment.rs); `None` clears it.
    pub unsafe fn set_environment(
        &mut self,
        env: Option<&crate::environment::EnvMap>,
    ) -> Result<()> {
        unsafe { self.device.device.device_wait_idle()? };
        let Some(env) = env else {
            self.env_inv_norm = 0.;
            self.env_sun = None;
            self.env_stars = None;
            return Ok(());
        };
        self.env_sun = env.sun.map(|s| (s, env.sun_share));
        self.env_stars = None;
        if let Some(table) = crate::stars::Table::get().filter(|_| env.stars) {
            if self.star_buffer.is_none() {
                let words = table.gpu_words();
                unsafe {
                    let b = self.device.upload(
                        bytemuck::cast_slice(&words),
                        vk::BufferUsageFlags::STORAGE_BUFFER,
                    )?;
                    self.layout.buffer(&self.device, 0, 118, b.handle);
                    self.star_buffer = Some(b);
                }
            }
            self.env_stars = Some(env.star_share);
        }
        let (w, h) = (env.width as u32, env.height as u32);
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
        unsafe {
            let d = &self.device;
            let storage = vk::ImageUsageFlags::STORAGE;
            let texels = d.image([w, h, 1], vk::Format::R16G16B16A16_SFLOAT, storage)?;
            let cdf = d.image([w, 2 * (h + 1), 1], vk::Format::R32_SFLOAT, storage)?;
            d.fill_image(&texels, 0, [w, h, 1], bytemuck::cast_slice(&halves))?;
            d.fill_image(&cdf, 0, [w, 2 * (h + 1), 1], bytemuck::cast_slice(&env.cdf))?;
            self.layout.image(d, 30, texels.view);
            self.layout.image(d, 31, cdf.view);
            d.free_image(std::mem::replace(&mut self.fixed[1], texels));
            d.free_image(std::mem::replace(&mut self.fixed[2], cdf));
        }
        self.env_inv_norm = env.inv_norm;
        self.env_scale = scale;
        Ok(())
    }

    /// Puts a linear RGBA image of the output size (the CPU reference) where the A/B compare
    /// reads its reference (texture 13).
    pub unsafe fn upload_reference(&mut self, rgba: &[f32]) -> Result<()> {
        let (w, h) = (self.st.width, self.st.height);
        ensure!(
            rgba.len() == w as usize * h as usize * 4,
            "reference image size"
        );
        unsafe {
            let d = &self.device;
            d.device.device_wait_idle()?;
            d.fill_image(&self.textures[13], 0, [w, h, 1], bytemuck::cast_slice(rgba))?;
        }
        self.st.reference_valid = true;
        Ok(())
    }

    pub unsafe fn capture_reference(&mut self) -> Result<()> {
        unsafe {
            let d = &self.device;
            d.device.device_wait_idle()?;
            let (a, b) = (&self.textures[18], &self.textures[13]);
            d.one_shot(|cmd| copy_image(&d.device, cmd, a, b))?;
        }
        self.st.reference_valid = true;
        Ok(())
    }
}

/// Copies the overlap of two GENERAL-layout images.
pub(super) unsafe fn copy_image(d: &ash::Device, cmd: vk::CommandBuffer, a: &Image, b: &Image) {
    let layers = vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    };
    let region = vk::ImageCopy::default()
        .src_subresource(layers)
        .dst_subresource(layers)
        .extent(vk::Extent3D {
            width: a.size[0].min(b.size[0]),
            height: a.size[1].min(b.size[1]),
            depth: 1,
        });
    unsafe {
        d.cmd_copy_image(
            cmd,
            a.handle,
            vk::ImageLayout::GENERAL,
            b.handle,
            vk::ImageLayout::GENERAL,
            &[region],
        )
    };
}
