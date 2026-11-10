//! Per-frame passes before the trace: aux buffers, integrators, battle mode, ReSTIR, glare, lens, LPE, texture maps.
use super::*;

impl Gpu {
    pub(super) unsafe fn bind(&self, offset: u64) {
        self.list.SetDescriptorHeaps(&[Some(self.heap.clone())]);
        self.list.SetComputeRootSignature(&self.root);
        self.list
            .SetComputeRootConstantBufferView(0, self.constants.GetGPUVirtualAddress() + offset);
        self.list.SetComputeRootShaderResourceView(
            1,
            self.tlas.as_ref().unwrap().GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(2, self.mesh_vertices.GetGPUVirtualAddress());
        self.list.SetComputeRootDescriptorTable(3, self.gpu(0));
        self.list
            .SetComputeRootShaderResourceView(5, self.material_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(6, self.emitter_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(7, self.emitter_lookup.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(8, self.static_vertices.GetGPUVirtualAddress());
        self.list
            .SetComputeRootShaderResourceView(9, self.instance_table.GetGPUVirtualAddress());
        self.list
            .SetComputeRootDescriptorTable(10, self.gpu(HEAP_SIZE));
        self.list
            .SetComputeRootUnorderedAccessView(11, self.path_buffer.GetGPUVirtualAddress());
        // Without AOVs no shader reads u1 space 2 (display checks the slot count).
        self.list.SetComputeRootUnorderedAccessView(
            12,
            self.lpe_aov
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(13, self.lpe_table.GetGPUVirtualAddress());
        // Without ReSTIR no shader reads u2 space 2 (restir.x is 0).
        self.list.SetComputeRootUnorderedAccessView(
            14,
            self.reservoirs
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        // Only the BDPT and photon passes read u3 space 2, and they run only with a buffer.
        self.list.SetComputeRootUnorderedAccessView(
            15,
            self.aux
                .as_ref()
                .unwrap_or(&self.path_buffer)
                .GetGPUVirtualAddress(),
        );
        self.list
            .SetComputeRootShaderResourceView(16, self.lens_buffer.GetGPUVirtualAddress());
        self.list
            .SetComputeRootUnorderedAccessView(17, self.glare_buffer.GetGPUVirtualAddress());
        // Without stars no shader reads t18 (star_tau.w is 0).
        self.list.SetComputeRootShaderResourceView(
            18,
            self.star_buffer
                .as_ref()
                .unwrap_or(&self.lens_buffer)
                .GetGPUVirtualAddress(),
        );
    }
    /// Sizes `aux` (u3 space 2) for the integrators in use: the photon grid and photons at 0
    /// (photon mapping), the light-tracing splats after them (BDPT, one rgb float triple per
    /// render pixel, zeroed here and by bdpt_resolve after every frame).
    pub(super) unsafe fn prepare_aux(
        &mut self,
        s: &Settings,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        let photons = s.integrator == 2 || s.battle;
        let splats = s.integrator == 1 || s.battle;
        let photon_bytes = if photons {
            PHOTON_BASE + s.photons as u64 * s.bounces.clamp(1, PHOTON_BOUNCES) as u64 * 32
        } else {
            0
        };
        let splat_bytes = if splats {
            self.rw as u64 * self.rh as u64 * 12
        } else {
            0
        };
        let len = photon_bytes + splat_bytes;
        if len == self.aux_len && photon_bytes == self.splat_offset {
            return Ok(());
        }
        self.splat_offset = photon_bytes;
        self.aux_len = len;
        if len == 0 {
            self.aux = None;
            return Ok(());
        }
        let aux = buffer(
            &self.device,
            len,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        if splat_bytes > 0 {
            let zero = upload(&self.device, &vec![0u8; splat_bytes as usize])?;
            self.list
                .CopyBufferRegion(&aux, photon_bytes, &zero, 0, splat_bytes);
            transition(
                &self.list,
                &aux,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );
            keep.push(zero);
        }
        if let Some(old) = self.aux.replace(aux) {
            keep.push(old);
        }
        Ok(())
    }
    /// Runs one integrator over `region` with the constants `c` (written at `offset`): ReSTIR's
    /// passes and the path tracer (0), BDPT and its splat resolve (1), or photon mapping's
    /// grid clear, photon trace and gather (2). `photon_boost` multiplies the photon count
    /// (battle mode's photon quadrant spends its time share on photons).
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
            // The bounding sphere sun and sky photons start from. The floor (material 3) is
            // left out: it often reaches far past the scene.
            let (mut lo, mut hi) = s.objects_bounds();
            for v in self.static_cache.iter().filter(|v| v.material != 3) {
                lo = lo.min(Vec3::from(v.position));
                hi = hi.max(Vec3::from(v.position));
            }
            // Fog scatters light toward the camera wherever it is: the sphere covers the fog
            // box too, or multiple scattering outside the objects' sphere would be missing
            // (underwater: 42% dark).
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
        let share = (w * h) as f32 / (self.rw * self.rh).max(1) as f32;
        c.region = region;
        c.sampling[1] = n;
        if !restir {
            c.restir[0] = 0;
        }
        let photons = ((s.photons as f32 * share * photon_boost) as u32).clamp(1024, s.photons);
        let capacity = (self.splat_offset.saturating_sub(PHOTON_BASE) / 32) as u32;
        // Light tracing needs a pinhole perspective camera and samples spread over each pixel.
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
        write_buffer(&self.constants, offset, bytemuck::bytes_of(&c))?;
        self.bind(offset);
        let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
        match integrator {
            1 => {
                let bdpt = if s.spectral {
                    &self.bdpt_spectral
                } else {
                    &self.bdpt
                };
                for pass in [bdpt, &self.bdpt_resolve] {
                    self.list.SetPipelineState(pass);
                    self.list.Dispatch(gx, gy, 1);
                    uav(&self.list, None);
                }
            }
            2 => {
                self.list.SetPipelineState(&self.photon_clear);
                self.list.Dispatch((PHOTON_HASH / 64) as u32, 1, 1);
                uav(&self.list, None);
                self.list.SetPipelineState(if s.spectral {
                    &self.photon_trace_spectral
                } else {
                    &self.photon_trace
                });
                self.list.Dispatch(photons.div_ceil(64), 1, 1);
                uav(&self.list, None);
                self.list.SetPipelineState(if s.spectral {
                    &self.photon_gather_spectral
                } else {
                    &self.photon_gather
                });
                self.list.Dispatch(gx, gy, 1);
                uav(&self.list, None);
                // Knaus and Zwicker's radius schedule, alpha = 2/3.
                self.photon_frames += 1;
                let i = self.photon_frames as f32;
                self.photon_scale *= (i + 2. / 3.) / (i + 1.);
            }
            _ => {
                if restir {
                    // ReSTIR DI: initial candidates and the temporal merge, then the spatial
                    // merge into the final reservoirs the trace reads.
                    self.list.SetPipelineState(&self.restir_presample);
                    self.list.Dispatch((RESTIR_TILES / 64) as u32, 1, 1);
                    uav(&self.list, None);
                    for pass in [&self.restir_initial, &self.restir_spatial] {
                        self.list.SetPipelineState(pass);
                        self.list.Dispatch(gx, gy, self.restir_layers);
                        uav(&self.list, None);
                    }
                }
                // LEAN exactly when the shader's fog and ReSTIR flags are off for this frame.
                let lean = self.lean_scene && c.fog[0] <= 0. && c.restir[0] & 1 == 0;
                let variant = self.textured as usize + 2 * lean as usize;
                // Light path expressions stay RGB (their tracer has no spectral twin).
                let spectral = s.spectral && self.lpe_slots == 0;
                if let Some(ser) = self.ser.as_ref() {
                    // Track 20: the same threads as the compute dispatch, through
                    // DispatchRays so the shader may reorder them (MaybeReorderThread).
                    let p = &ser[if self.lpe_slots > 0 {
                        4
                    } else if spectral {
                        5 + lean as usize
                    } else {
                        variant
                    }];
                    self.list.SetPipelineState1(&p.state);
                    self.list.DispatchRays(&D3D12_DISPATCH_RAYS_DESC {
                        RayGenerationShaderRecord: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                            StartAddress: p.table.GetGPUVirtualAddress(),
                            SizeInBytes: D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64,
                        },
                        Width: gx * 8,
                        Height: gy * 8,
                        Depth: 1,
                        ..Default::default()
                    });
                } else {
                    self.list.SetPipelineState(if self.lpe_slots > 0 {
                        &self.trace_lpe
                    } else if spectral && lean {
                        &self.trace_spectral_lean
                    } else if spectral {
                        &self.trace_spectral
                    } else {
                        [
                            &self.trace,
                            &self.trace_maps,
                            &self.trace_lean,
                            &self.trace_maps_lean,
                        ][variant]
                    });
                    self.list.Dispatch(gx, gy, 1);
                }
                uav(&self.list, None);
            }
        }
        Ok(())
    }
    /// Battle mode (track 15): the four quadrants, each with its own integrator and sample
    /// count, at equal GPU time: the slowest takes `spp` samples per frame and the others as
    /// many as fit in its time (measured per quadrant with timestamps, at most 64).
    pub(super) unsafe fn battle_frame(
        &mut self,
        s: &Settings,
        c: &Constants,
        restir: bool,
    ) -> Result<()> {
        let (hw, hh) = (self.rw / 2, self.rh / 2);
        let regions = [
            [0, 0, hw, hh],
            [hw, 0, self.rw, hh],
            [0, hh, hw, self.rh],
            [hw, hh, self.rw, self.rh],
        ];
        let slowest = self.battle.iter().map(|q| q.cost).fold(0., f64::max);
        let budget = slowest * s.spp as f64;
        for (i, region) in regions.into_iter().enumerate() {
            let q = &mut self.battle[i];
            let want = if q.cost > 0. && budget > 0. {
                budget / q.cost + q.carry
            } else {
                s.spp as f64
            };
            // Photon mapping spends its share on photons, not on gathers: each frame's gathers
            // share one photon map, so one sample per frame keeps its samples independent (and
            // its sample cap as far away as the others').
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
            self.list
                .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, query);
            if n > 0 {
                let mut qc = *c;
                qc.sampling[0] = samples;
                let offset = (9 + i as u64) * CB_STRIDE;
                let scale = self.battle[i].scale as f32;
                self.integrate(
                    s,
                    &qc,
                    offset,
                    [0, 0, 1, 2][i],
                    i == 1 && restir,
                    region,
                    n,
                    scale,
                )?;
            }
            self.list
                .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, query + 1);
        }
        if restir && self.battle[1].last > 0 {
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
    /// Sizes the ReSTIR reservoir buffer while ReSTIR is on: one layer (two reservoirs per
    /// render pixel) for each of the frame's first 4 samples, which keeps them independent;
    /// later samples use plain light sampling. A new buffer has no history. True when the
    /// passes run this frame.
    pub(super) unsafe fn prepare_restir(&mut self, s: &Settings) -> Result<bool> {
        // Battle mode's second quadrant is path tracing with ReSTIR.
        let wanted = (s.restir && s.integrator == 0) || s.battle;
        let on = wanted && self.emitter_count > 0;
        // At most 1 GiB of reservoirs (4K: two layers).
        let pixels = (self.rw as u64 * self.rh as u64).max(1);
        self.restir_layers = s
            .spp
            .clamp(1, 4)
            .min(((1 << 30) / (pixels * 64)).max(1) as u32);
        // Plus the light tiles (pathtrace.hlsl RESTIR_TILES × RESTIR_TILE, 32 B each).
        let len = if wanted {
            self.rw as u64 * self.rh as u64 * self.restir_layers as u64 * 64 + RESTIR_TILES * 32
        } else {
            0
        };
        if len == 0 {
            self.reservoirs = None;
            self.reservoirs_len = 0;
            self.restir_history = false;
        } else if len != self.reservoirs_len {
            self.reservoirs = Some(buffer(
                &self.device,
                len,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON,
            )?);
            self.reservoirs_len = len;
            self.restir_history = false;
        }
        Ok(on)
    }
    /// The image's size in the glare grid, w | h << 16 (the longer side GLARE_IMAGE).
    pub(super) fn glare_grid(&self) -> u32 {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let (w, h) = if aspect >= 1. {
            (GLARE_IMAGE, GLARE_IMAGE / aspect)
        } else {
            (GLARE_IMAGE * aspect, GLARE_IMAGE)
        };
        (w.round().max(1.) as u32) | (h.round().max(1.) as u32) << 16
    }
    /// Track 16 glare: rebuilds the kernel's spectrum when the aperture changes (its shape into
    /// region 0, FFT, the per-wavelength pattern into region 1, FFT), then convolves this frame's
    /// highlights (region 0: highlights, row FFT, column FFT × kernel × inverse, inverse rows).
    /// The display pass reads region 0.
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
            write_buffer(&self.constants, offset, bytemuck::bytes_of(c))?;
            self.bind(offset);
            self.list.SetPipelineState(&self.glare);
            self.list.Dispatch(GLARE_N as u32, 1, 1);
            uav(&self.list, None);
        }
        c.glare[..3].copy_from_slice(&[0; 3]);
        Ok(())
    }
    /// Builds the lens system's buffer (track 16) when the lens, its stop, focus, field of view,
    /// aspect or dispersion change. The GPU is idle between frames (one frame in flight).
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
            aspect: self.rw as f32 / self.rh.max(1) as f32,
            dispersion: s.dispersion,
        };
        if self.lens_params.as_ref() != Some(&p) || self.lens_surfaces == 0 {
            let (data, n) = crate::lens::pack(&p);
            write_buffer(&self.lens_buffer, 0, bytemuck::cast_slice(&data))?;
            self.lens_surfaces = n as u32;
            self.lens_params = Some(p);
        }
        Ok(())
    }
    /// Uploads the light path expressions' table when they change, and sizes the AOV buffer
    /// (a new buffer restarts the accumulation, as its contents are undefined).
    pub(super) unsafe fn prepare_lpe(&mut self, s: &Settings) -> Result<()> {
        // Labels match names, which the accumulation's transport key leaves out: a rename that
        // changes a label's surfaces resets here.
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
            write_buffer(&self.lpe_table, 0, bytemuck::cast_slice(&table))?;
            let classes = crate::lpe::classes(&labels, &names);
            write_buffer(
                &self.lpe_table,
                (table.len() * 4) as u64,
                bytemuck::cast_slice(&classes),
            )?;
            if self.lpe_uploaded.is_some() && slots > 0 {
                self.reset(false);
            }
            self.lpe_slots = slots;
            self.lpe_uploaded = Some((s.lpe.clone(), names));
        }
        let len = self.rw as u64 * self.rh as u64 * self.lpe_slots as u64 * 16;
        if len == 0 {
            self.lpe_aov = None;
            self.lpe_aov_len = 0;
        } else if len != self.lpe_aov_len {
            self.lpe_aov = Some(buffer(
                &self.device,
                len,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON,
            )?);
            self.lpe_aov_len = len;
            self.reset(false);
        }
        Ok(())
    }
    /// One light path expression's AOV at render resolution, as RGB floats (self-test, tools).
    pub unsafe fn read_lpe(&mut self, slot: u32) -> Result<Vec<f32>> {
        anyhow::ensure!(slot < self.lpe_slots, "LPE slot {slot} is not in use");
        let aov = self.lpe_aov.clone().context("No LPE buffer")?;
        let staging = buffer(
            &self.device,
            self.lpe_aov_len,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        // A buffer used as a UAV decays to COMMON after each submit; COPY_SOURCE is implicit.
        self.list.CopyResource(&staging, &aov);
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let all =
            std::slice::from_raw_parts(p as *const [f32; 4], (self.lpe_aov_len / 16) as usize);
        let out = all
            .iter()
            .skip(slot as usize)
            .step_by(self.lpe_slots as usize)
            .flat_map(|v| [v[0], v[1], v[2]])
            .collect();
        staging.Unmap(0, None);
        Ok(out)
    }
    /// Points each visible object's material at its texture maps (core track 5), uploading
    /// maps that are new and releasing ones no object uses any more.
    pub(super) unsafe fn attach_maps(
        &mut self,
        s: &Settings,
        materials: &mut [crate::lighting::PackedMaterial],
    ) -> Result<()> {
        let instances = std::mem::take(&mut self.instances);
        let used = crate::render::attach_maps(s, &instances, materials, &mut |map| {
            let key = std::sync::Arc::as_ptr(map) as usize;
            if !self.maps.contains_key(&key) {
                let texture = self.upload_map(map)?;
                self.maps.insert(key, texture);
            }
            Ok(self.maps[&key].slot)
        });
        self.instances = instances;
        let used = used?;
        let stale: Vec<usize> = self
            .maps
            .keys()
            .filter(|k| !used.contains(k))
            .copied()
            .collect();
        if !stale.is_empty() {
            // Frames in flight may still read them.
            self.wait_idle()?;
            for key in stale {
                let old = self.maps.remove(&key).unwrap();
                self.free_maps.push(old.slot);
            }
        }
        Ok(())
    }
    /// One map as an RGBA8 texture (sRGB view for colour maps) with an SRV in its slot.
    pub(super) unsafe fn upload_map(
        &mut self,
        map: &std::sync::Arc<crate::mesh::Map>,
    ) -> Result<MapTexture> {
        let slot = self
            .free_maps
            .pop()
            .context("Out of texture map slots (4096)")?;
        let levels = 1 + map.mips.len();
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: map.width as u64,
            Height: map.height,
            DepthOrArraySize: 1,
            MipLevels: levels as u16,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            ..Default::default()
        };
        let mut resource: Option<ID3D12Resource> = None;
        self.device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut resource,
        )?;
        let t = Texture {
            resource: resource.unwrap(),
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        // Every level through one staging buffer, laid out by the copy footprints.
        let mut layouts = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); levels];
        let mut total = 0;
        self.device.GetCopyableFootprints(
            &desc,
            0,
            levels as u32,
            0,
            Some(layouts.as_mut_ptr()),
            None,
            None,
            Some(&mut total),
        );
        let staging = buffer(
            &self.device,
            total,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let mut mapped = null_mut();
        staging.Map(0, Some(&D3D12_RANGE::default()), Some(&mut mapped))?;
        for (layout, data) in layouts
            .iter()
            .zip(std::iter::once(&map.rgba).chain(&map.mips))
        {
            let row = layout.Footprint.Width as usize * 4;
            for (y, src) in data.chunks_exact(row).enumerate() {
                let at = layout.Offset as usize + y * layout.Footprint.RowPitch as usize;
                std::ptr::copy_nonoverlapping(src.as_ptr(), (mapped as *mut u8).add(at), row);
            }
        }
        staging.Unmap(0, None);
        self.begin()?;
        for (level, layout) in layouts.iter().enumerate() {
            let mut dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(t.resource.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    SubresourceIndex: level as u32,
                },
            };
            let mut src = D3D12_TEXTURE_COPY_LOCATION {
                pResource: ManuallyDrop::new(Some(staging.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: *layout,
                },
            };
            self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            ManuallyDrop::drop(&mut src.pResource);
            ManuallyDrop::drop(&mut dst.pResource);
        }
        transition(
            &self.list,
            &t.resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
        self.submit()?;
        drop(staging);
        let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: if map.srgb {
                DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
            } else {
                DXGI_FORMAT_R8G8B8A8_UNORM
            },
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV {
                    MipLevels: levels as u32,
                    ..Default::default()
                },
            },
        };
        self.device.CreateShaderResourceView(
            &t.resource,
            Some(&desc),
            self.cpu(HEAP_SIZE + slot as usize),
        );
        Ok(MapTexture {
            _texture: t,
            slot,
            _map: map.clone(),
        })
    }
}
