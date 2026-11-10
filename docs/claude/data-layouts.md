# Data layouts shared by Rust and HLSL

Moved out of `CLAUDE.md` on 2026-11-02 (unchanged text). Paths are relative to the repo root. Where this file says "see X", X is a section of `docs/engine-notes.md`.

## Data layouts (keep in sync on both sides)

**Material IDs** (vertex `material` field):

| ID | Meaning |
|---|---|
| 0 | Left wall |
| 1 | Right wall |
| 2 | Back wall and ceiling |
| 3 | Floor |
| 4 | Main light |
| 5 … 5+N−1 | Objects |
| 5+N … | Extra area lights |

**PackedMaterial** (176 B) fields:

| Field | Contents |
|---|---|
| `color_rough` | RGB + roughness |
| `lobes` | metallic, reflectivity, transmission, flags. **bit0** = unlit (the path terminates on hit); **bit1** = hidden from camera rays (bounce 0 and the guide ray; lights, and objects with `Object.camera_hidden`); **bit2** = portal; **bit3** = masked texture map (alpha test in the any-hit loops); **bit4** = has texture maps; **bit5** = spot light; **bits 8–10** = nested-dielectric priority (0–7); **bit11** = spun anisotropy; **bit12** = beam splitter (track 17); **bit13** (8192) = a light, or an object with `Object.casts_shadows` off, that casts no shadows: tested per candidate for triangles and for procedural/SDF candidates in `pathtrace.hlsl::intersect` and `occluded` (the instance mask bit `MASK_NO_SHADOW` (8) is only the fast path for triangle-only instances; SDF instances keep mask 0xff); **bit14** = IES profile (wave 2); **bit6** = emissive map (wave 2: `secondary` = map slot + 1 and the mean emitted colour over its peak, `medium.xyz` = 1 / the map's mean). Bits 3, 4 and 6 are set in `Gpu::attach_maps`. |
| `optical` | IOR, emission, pattern id, w: one-sided emitter (1 for lights); negative = a diffraction grating, −(strength ‰ × 16384 + spacing nm) (track 11) |
| `secondary` | Pattern RGB + pattern scale. Patterns are evaluated in **world space**. |
| `layers`, `sheen` | Track 11: coat, coat roughness, sheen roughness, anisotropy; sheen colour × strength, anisotropy rotation (radians). |
| `medium`, `phase` | Track 12: scattering albedo rgb + extinction (0 unless transmissive); HG g in x. See "Volumes". `phase.y`: the Abbe number for spectral dispersion (0 = none; transmissive only, engine-notes "Spectral rendering"). |
| `maps` (uint4) | Heap slot + 1 of the base colour, normal and metallic-roughness maps (0 = none), alpha cutoff bits (0 = opaque; negative = glTF BLEND, −(factor alpha), stochastic transparency). Filled by `Gpu::attach_maps`. |
| `base_scale` | Base colour value / map average (rgb); w = texture-set repeats per unit (box mapping; 0 = the mesh's own UVs), set by `lighting::pack`. |
| `surface_scale` | Roughness and metallic value / map average (x, y; 0 = leave the value unscaled); z, w the thin film (track 11): strength, thickness nm + 2048 × IOR in hundredths. |

**Emitter**:

| Field | Contents |
|---|---|
| `position_area` | Vertex a, triangle area |
| `edge1_cdf` | b−a, CDF |
| `edge2_probability` | c−a, selection probability |
| `emission_material` | Emitted RGB, material id |

After the triangle records and the guide records come the **SDF photon records** (since 2026-11-03, `gpu.rs` `sdf_emitter_records`; `photon.z` = count, `photon.w` = first record), two per emissive SDF object: A = `position_area` bounding sphere (centre, radius), `edge1_cdf` (instance index as bits, CDF over SDF emitters by power, 0, 0), `edge2_probability` / `emission_material` rows 0 and 1 of world-to-object; B = `position_area` row 2. Only photon emission (`sdf_photon`) reads them. With an emissive map anywhere, one **UV record** per emitter follows (index `photon.w + 2·photon.z + e`): position_area = uv of a, b; edge1_cdf.xy = uv of c (`emitted_color`).

The emitter buffer (t4) holds the `n` emitters, then the alias records (`lighting::alias_records`, ⌈n/8⌉ records, eight (threshold, alias) pairs each), then the CDF's cutpoints (`lighting::guide_records`, K = the largest power of two ≤ n, sixteen u32 per record) for `select_index`.

**Root signature:**

| Slot | Binding |
|---|---|
| 0 | CBV b0 |
| 1 | SRV t0 TLAS |
| 2 | SRV t1 mesh vertices (object space, every mesh in use) |
| 3 | Table: UAV u0–u31 (heap slots 0–31; u29 is the 3D LUT, u30 / u31 the HDRI and its CDFs) |
| 4 | Table: SRV t2, the egui texture of each GUI draw (heap slots 32–511, one per texture; pixel-visible only) |
| 5 | Root SRV t3 materials |
| 6 | Root SRV t4 emitters |
| 7 | Root SRV t5 emitter lookup |
| 8 | Root SRV t6 static vertices (floor, room, lights; world space) |
| 9 | Root SRV t7 instance table (`InstanceRecord` / `Instance`, by InstanceID) |
| 10 | Table: SRV t0.. space 1, texture maps (`maps[]`, heap slots 512–4607, `MAX_MAPS` 4096) |
| 11 | Root UAV u0 space 2, path visualizer records (`trace_paths` only; bound for every dispatch) |
| 12 | Root UAV u1 space 2, LPE AOVs (`trace_lpe` writes, `display` view 12 reads; the path buffer stands in while none) |
| 13 | Root SRV t8 LPE DFA table: `lpe::SLOTS` × `STATES` × `SYMBOLS` (64: label class × 8 + event) next-state words, then `lpe::MATERIALS` (8192) label classes by material id |
| 14 | Root UAV u2 space 2, ReSTIR DI reservoirs (bound for every dispatch; the path buffer stands in while ReSTIR is off) |
| 15 | Root UAV u3 space 2, raw: the photon grid and photons at 0, BDPT's light-tracing splats at `splat_offset` (the path buffer stands in while neither runs) |
| 16 | Root SRV t9 lens system (`lens::pack`, an upload buffer of `lens::FLOAT4S` float4s; track 16) |
| 17 | Root UAV u4 space 2 glare grid: two regions of 3 × 512² complex values (track 16) |
| 18 | Root SRV t18 star table (night sky N3, `stars::Table::gpu_words`, u32 words): header (count, grid rows 180, columns 360, offsets word, entries word, 0, 0, 0), 8 words per star (J2000 direction xyz, CDF, RGB irradiance over the sun's, probability), the grid's `rows·cols + 1` cell offsets, then the star indices. Stars are sorted brightest first; a limiting magnitude keeps a prefix, whose count and CDF mass travel in `Constants.star_limit` (x, y), not in this buffer. Uploaded once with the first starry sky; the lens buffer stands in until then. Vulkan: set 0 binding 118 (the placeholder until uploaded). |

Static samplers: s0 linear clamp (GUI, pixel-visible), s1 linear wrap with mips (texture maps). The heap holds 512 + 4096 descriptors: UAVs 0–31, egui textures 32–511, texture maps from 512.

**Vertex** (40 B): position, material, normal, pad, uv (meshes only; zero for shapes and static geometry). The BLAS vertex stride follows `size_of::<Vertex>()`.

**Acceleration structures** (instanced since core track 4, see "Instancing"; procedural objects are instances of one unit-cube AABB BLAS with `vertex_base` = the `Sdf` kind, see "Procedural surfaces"): the static geometry (floor, room, lights; world space), grouped by instance mask since 2026-11-03: one BLAS and one instance per group (`StaticGroup`, opaque 0xff first), each with `vertex_base` = its first triangle in `static_vertices` (`environment.w` is unused, written 0), plus one BLAS per distinct object mesh in object space, instanced once per visible object with its transform. Instance masks (track 20a (a), `gpu.rs` `instance_mask` ↔ pathtrace.hlsl `MASK_*`): an object hidden from camera rays has mask 2 (`MASK_HIDDEN`, left out of camera rays), one of glass with transmission ≥ 0.5 has mask 4 (`MASK_CLEAR_GLASS`, left out of shadow rays unless glass blocks or `jitter.w` > 0.5), both are FORCE_OPAQUE. Alpha-tested objects and objects under both rules are FORCE_NON_OPAQUE with mask 0xff (the `Proceed()` loop tests them), all other objects FORCE_OPAQUE with mask 0xff; SDF instances keep 0xff. Alpha-tested meshes with a base map get an opacity micromap array (`src/omm.rs`, 4-state, conservative) and no FORCE flag, so the hardware resolves known-opaque and known-transparent micro-triangles itself; RayQuery has `RAYQUERY_FLAG_ALLOW_OPACITY_MICROMAPS` in the SER tracers and compute PSOs get flag 0x80. All use PREFER_FAST_TRACE.

**UAV textures.** Index = `u` register = heap slot. Formats are RGBA32F unless noted; resolution is render resolution unless noted.

| u | Purpose | Format / resolution |
|---|---|---|
| 0 | noisy (this frame's batch) | RR color input |
| 1 | accumulation (rgb mean, a = Welford M2 of luminance) | |
| 2 | normal_roughness | RR input |
| 3 | diffuse_albedo (RR: the guide's; native: mean of the samples, a = coverage) | RR input |
| 4 | specular_albedo (as u3) | RR input |
| 5 | depth (linear view z) | R32F, RR input |
| 6 | motion (prev − curr, pixels) | RG32F, RR input |
| 7 | specular hit distance | R32F, RR input |
| 8 / 9 | ping / pong (à-trous; final result in ping) | |
| 10 | output_image (tonemapped) | RGBA8, **output resolution** |
| 11 | direct_light AOV | |
| 12 | specular_light AOV | |
| 13 | reference_image (A/B) | **output resolution** |
| 14 | reconstruction (RR output) | **output resolution** |
| 15 | history (reprojected integration, frozen while the camera is still) | |
| 16 | history normal | |
| 17 | history depth | R32F |
| 18 | linear_output (HDR beauty, used for EXR) | **output resolution** |
| 19 | object_id (first-hit material id + 1, 0 = miss) | R32F |
| 20 | history object id | R32F |
| 21 | integrated (demodulated color; a = effective samples) | |
| 22 | moments (E[l], E[l²] per sample of demodulated luminance; z = variance of the integrated mean) | |
| 23 | history_moments | |
| 24 | filter_guide (octahedral normal xy, depth, object id: one read per à-trous tap) | |
| 25 | device_depth (projected z/w, 0 near → 1 far) for DLSS SR / FSR / XeSS | R32F |
| 26 | hdr_output (scRGB, 1.0 = 80 nits; written only in HDR mode, copied to the HDR backbuffer) | RGBA16F, **output resolution** |
| 27 | trace_stats: this frame's mean bounces per sample, BVH steps (`Proceed()` calls, all rays) per pixel | RG32F |
| 28 | scopes: counters, rows 0–3 histogram R,G,B,L; 4–131 luma waveform; 132–259 vectorscope (128 wide) | R32_UINT, **256×260 fixed** |
| 30 | env_map: HDRI, linear RGB (kept across resizes; 1×1 when none) | RGBA32F, **map size** |
| 31 | env_cdf: conditional CDF per row, then the marginal CDF in an extra row; rows height + 1 on: the same CDFs' cutpoints for `cdf_search` (K = the largest power of two ≤ width, f32 indices) | R32F, **map width × 2(height + 1)** |
