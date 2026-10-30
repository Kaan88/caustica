# Cornell RTX (native): working notes for Claude

Reference for future sessions. Written 2026-10-29 after a full read of the source plus a build and render check.
`README.md` is the user-facing doc.
Paths here are relative to `native/` unless noted.

## What it is

- A Windows-only port of the WebGL Cornell box in the parent folder (`../index.html`, `../app.js`, `../shaders.js`).
- Stack:
  - Rust 2024 for the app; HLSL compute shaders compiled by DXC at build time and embedded as DXIL.
  - Direct3D 12 with DXR 1.1 inline `RayQuery`. There are no RT pipeline state objects or shader tables.
  - An egui 0.31 overlay drawn straight into the swapchain.
  - A small C++ bridge (`bridge/ngx.cpp`) to NVIDIA NGX for DLSS Ray Reconstruction.
- Vendor SDKs, each pinned and sparse-cloned by `tools/setup.ps1` (function `Get-Sdk`) into `vendor/` (gitignored):
  - `vendor/dlss`: NVIDIA DLSS 310.9.16, commit `9981c63…`.
  - `vendor/fsr`: AMD FSR SDK 2.3.0, commit `60f4ea8…`. FidelityFX API headers plus signed `amd_fidelityfx_loader_dx12.dll` and `amd_fidelityfx_upscaler_dx12.dll`.
  - `vendor/xess`: Intel XeSS SDK 3.0.2, commit `8fe81bd…`. Headers plus `libxess.dll`.
  - The FSR and XeSS licenses allow binary redistribution only, with no modification or reverse engineering.
- Startup defaults: RR on, RR2/F preset, DLAA (scale 1), 4 spp per frame, 6 bounces, sample cap 4096, ACES tonemap.
- **Personal project, never published:** license terms don't limit assets or SDK use. The ordered plan is `docs/roadmap.md`:
  - Near term: DLSS presets, native HDR, UI phase 1, Blender-style gizmos and scene files (42 scenes) are done (2026-10-30); next are the quick wins (hot reload, debug views, depth of field, light painting, portals, screensaver).
  - Medium term: the core engine track (generators → mesh import → HDRI → instancing → textures → UI), then feature tracks 7–20 (lights, animation, volumes, SDFs, path visualizer, integrators, physics, SER).
  - Future: Vulkan, software fallback / CPU reference, spectral, curved space, frame generation, neural radiance cache, splats, VR.
- **Git:** the repo root is `cornell/` (WebGL app plus `native/`), with a private GitHub remote. `main` is the verified state. Commit and push only when the user asks.
- The native settings JSON is not compatible with the browser's.

### Machine (2026-10-29)

- Hardware and drivers: RTX 4070 Ti SUPER 16 GB, driver 616.64, Windows 11.
- Toolchain: Rust 1.99.1 MSVC, DXC from Windows SDK 10.0.26100, Python 3.13 with numpy and Pillow (for `tools/rr_stability.py`).

## Build, run, verify

```powershell
cd native
$env:CARGO_TARGET_DIR="target/validation"   # keep checks away from target/release (the launcher's exe)
cargo fmt --check; cargo test --locked; cargo clippy --locked -- -D warnings; cargo build --locked
```

- `../launch-native.cmd` starts `target/release/cornell-rtx.exe` and only builds it if it is missing. It never rebuilds a stale exe.
- Release builds use `lto = "thin"` and `codegen-units = 1`, so they are slow.
- **Release is a GUI-subsystem exe with no console output.** Debug builds have a console.
  - `tools/verify.ps1` runs release via `Start-Process` with stdout and stderr redirected.
- `build.rs` does the following:
  - Asserts that `vendor/dlss`, `vendor/fsr` and `vendor/xess` exist.
  - Compiles 5 shader entries: `trace`, `filter` and `display` as cs_6_5; `vs_main` and `ps_main` as 6_0.
  - Compiles the bridge and links `nvsdk_ngx_d`.
  - **Copies `nvngx_dlss.dll` and `nvngx_dlssd.dll` next to the exe.** A running instance locks them and the build fails. Use a separate `CARGO_TARGET_DIR`, and never kill the user's running app.
- Headless runs need a real GPU and use a hidden window:
  - Command: `cornell-rtx.exe --headless --frames N [--width W --height H --scale s --spp n --samples cap --preset 0..23 --settings f.json --scene name --rr|--no-rr|--require-rr --raw --debug] --capture out.png|out.exr`.
  - Success prints `RENDER_OK frames= samples= gpu_ms= rr= size= gpu_ms_mean= frame_ms_mean=`. The means are taken from frame 10 on, so use ≥ 200 frames with `--no-vsync` for benchmarks.
  - `--preset`/`--settings` (only one) is applied before all other switches, so order doesn't matter.
    - **Old builds (including `main` before 2026-10-29's fixes) applied them in order**, so `--raw` and `--spp` given before `--preset` were silently reset. Put `--preset` first when benchmarking an old exe.
- Other test modes:
  - `--self-test`: 270 scripted frames (`SCENES_FROM`), then one frame per scene file (430 with the built-in, generated, mesh and environment packs); implies headless and `--debug`. It fails on any D3D12 warning.
    - Frames 10–125 cycle all 24 presets; frames 130–140 make live edits (lights, all shapes, emission, hide, delete, pick, frame, randomize).
    - Every frame it asserts that the GPU triangle and emitter counts match the settings.
    - The script lives in `redraw()`. Add new edits on unused frame numbers and assert after `render`.
  - `--debug`: needs the Windows Graphics Tools.
  - `--orbit-test`: moves the camera 0.25° per frame after frame 64.
  - `--capture-sequence dir`: captures the last 32 frames.
- Unbiased reference for noise measurement: `--no-rr --raw --samples N --frames N/spp --capture ref.exr`. Linear RGB32F at output resolution.
- `tools/rr_stability.py` is an RR shimmer regression. It uses emission-only walls and black emission-only blocks, so there is no Monte Carlo noise.
  - Run `python tools/rr_stability.py --exe <path>` to use a build other than `target/release`.
  - Thresholds: edge temporal std < 0.02 and mean frame difference < 0.0015.
  - Last result, 2026-10-29: 0.0071 and 0.00017. The 2026-10-26 run is kept as `captures/rr-stability/regression/metrics-2026-10-26.json`.
- `--set key=json` overrides one top-level setting after `--preset` or `--settings`. Unknown keys are rejected. Example: `--set direct=0 --set indirect=1`.
- Python on Windows needs Windows paths. Use `cygpath -w` when calling it from the Bash tool.

## Source map

| File | Key places |
|---|---|
| `src/main.rs` | `App` state. `App::new` is the CLI parser (`--set key=json`, `--ui-*`). `redraw()` runs each frame: self-test script, HDR/monitor sync, egui (`ui()` lives in `ui.rs`), save/load, dirty keys, render, self-test asserts, actions, export. `window_event()` handles input and the single-key toggles (each one toasts). |
| `src/scene.rs` | `Settings`, `Material`, `Object`, `AreaLight`, `Shape` (11) and `Pattern` (4). `PRESETS` holds 24 names. `randomize()` (l.318). `preset()` (l.352) builds 0–8 inline and sends 9+ to `presets::apply`; it keeps the camera only when moving between Cornell presets 0–8. `validate()` (l.456) bounds every field. `camera()` returns `look_at_rh` and `perspective_rh` with near 0.01 and far 500. `light_color()` converts Kelvin to linear RGB at unit luminance, times tint and intensity. |
| `src/presets.rs` | 12-entry material library (`material(i)`), `studio()` base stage, `apply()` for presets 9–23, `description()`. |
| `src/geometry.rs` | Unit meshes cached in a `LazyLock`. `object_geometry()` applies scale (size·0.35), YXZ Euler rotation and floor grounding (floor at y = −1). `geometry()` builds one triangle soup. Also frame, focus and CPU pick (Möller–Trumbore over every object, every click). |
| `src/lighting.rs` | `PackedMaterial` and `Emitter` (64 B each), `materials()` ordering, `emitter_table()` (power-weighted CDF plus a per-triangle lookup, 1-based, 0 = not an emitter). |
| `src/display.rs` | `Monitor::query(hwnd)`: the DXGI output holding most of the window, `GetDesc1` (HDR on, peak/full-frame/min nits) and the SDR white level via `DisplayConfigGetDeviceInfo`; `current()` = `IDXGIFactory1::IsCurrent`. Also `hdr_map` (CPU copy of the shader transform, for tests) and `write_hdr_png` (16-bit PQ/BT.2020 with a cICP chunk). |
| `src/ui.rs` | UI plan phase 1 (2026-10-30). `UiState` (tab, advanced, panel_hidden, selection, toasts, export size/samples): never in `Settings`. `impl App { fn ui }` draws top bar, status bar, right `SidePanel` with Scene / Render / Look / Export tabs, toasts. Helpers `row` (2-column grid row, label carries the tooltip), `more` (inline when advanced, else a "More" header), `theme`. `UiState::delete_selected`, `toast`, `error`. |
| `src/gizmo.rs` | Roadmap 3b (2026-10-30). `View` (camera in physical pixels: project, ray, world-per-pixel), `frame()` (pivot + rotation of the selection; lights are `flat`), `Transform` (mode, constraint, typed value, snap, original settings; `apply` restores the target then transforms; `restore` cancels), `layout`/`hit`/`draw`, `draw_lights`, `pick` (objects via `pick_object_hit` plus light quads). Unit tests cover typed values, arrow drags, screen-angle rotation, Blender's axis toggling and light scaling. |
| `src/scenes.rs` | Roadmap 4 (2026-10-30). `folder()` (`./scenes`, next to the exe, or `exe/../../scenes`), `list()` (packs by `pack.json` order; builtin first, user last), `read()` (wrapper `{format, title, description, tags, credits, settings}` or bare `Settings`; validated; title becomes `scene_name`), `find()` (path, stem or title), `load_into()` (uses `Settings::keep_output` so render choices survive), `save_user()`. Test `builtin_scenes_are_valid`. |
| `src/hotreload.rs` | Roadmap 5a. `HotReload::new()` (debug builds, not the self-test) watches `CORNELL_SHADERS` (build.rs bakes the source path and the DXC path `CORNELL_DXC`); `poll()` every 0.5 s on the newest mtime; `compile()` runs all 5 entries of `ENTRIES` (keep in sync with build.rs) with build.rs's flags. `Gpu::reload_shaders` builds all PSOs before swapping, so an error keeps the old ones; prints `SHADERS RELOADED` / `SHADER ERROR`. Tested by editing post.hlsl under a running instance (valid, broken, restored). |
| `src/look.rs` | Roadmap 5c: `read_cube` (.cube 3D LUT) and `color_wheel` (lift/gamma/gain widget: chroma from a dragged dot in an orthonormal RGB chroma plane, level from a slider, double-click resets). |
| `src/walk.rs`, `video.rs`, `saver.rs`, `remote.rs` | Roadmap 5d toys: walk mode, turntable video, screensaver mode, phone remote (see "Toys"). |
| `src/editor.rs` | Inspectors used by the Scene tab: `object_ui`, `light_ui`, `material_ui`, `shape_combo`, `new_object`. Ranges match `validate()`. |
| `src/gpu.rs` | Everything D3D12. Highlights: `new()`, `resize()` (l.496, allocates the 19 UAV textures), `rebuild_scene()` (l.611), `update_materials()`, `render()` (l.749), `capture()`, `capture_reference()`, egui atlas and draw. |
| `src/teapot_data.rs` | Generated by `tools/build_teapot_data.py` from `assets/TeapotGeometry.source.js`. Do not hand-edit. |
| `shaders/common.hlsli` | `cbuffer Frame`, which **must mirror `Constants` in gpu.rs field for field**. Also the resource bindings. |
| `shaders/pathtrace.hlsl` | `trace` (l.56): guide/AOV ray, then the spp loop. |
| `shaders/post.hlsl` | `filter` (5 à-trous passes plus temporal) and `display` (views, tonemap, A/B). |
| `shaders/gui.hlsl` | Declares a partial cbuffer (4 matrices, eye, then `dimensions`) that relies on that offset in `Constants`. |
| `bridge/ngx.cpp` | NGX C ABI: `cornell_ngx_caps` (bit 0 RR, bit 1 DLSS SR), RR create/eval (DLSSD), SR create/eval (DLSS) with its own parameter block. Contains a development project GUID that must be replaced before distribution. |
| `bridge/fsr.cpp` | FSR through the FidelityFX API; `LoadLibrary` of the loader DLL next to the exe. Context = upscale desc → DX12 backend desc → version desc (pNext chain). |
| `bridge/xess.cpp` | XeSS through `GetProcAddress` on `libxess.dll`; `xessD3D12Init` again on size change. |
| `docs/ui-plan.md`, `docs/scene-research.md` | Plans written by subagents on 2026-10-29: UI redesign in 5 phases; scene sources, licenses, 42 buildable presets, generators, and an engine-feature roadmap. |
| `tools/upscale_quality.py` | Upscaler comparison, still and orbit, against full-resolution references. |

## Frame pipeline (`Gpu::render`)

1. Upload `Constants` to offset 0, then `trace` dispatch (8×8 groups, render resolution).
2. Then one of:
   - **If RR is active:** transition inputs 0, 2–7 to SRV, call `NGX_D3D12_EVALUATE_DLSSD_EXT` into texture 14, transition back.
   - **Otherwise, if denoise is on:** filter passes 0–6, constants at offset `CB_STRIDE`·(i+1) (1024 B).
     - 0 reproject: only on a camera move.
     - 1 integrate: when samples were traced, or the integration is stale. It then copies guides 2→16, 5→17 and 19→20 for the next reprojection.
     - 2–6 à-trous.
     - Flags `camera_moved`, `history_valid` and `integrated_valid` in `Gpu` decide which passes run.
3. `display` (constants at offset 8·`CB_STRIDE`) writes `output_image` (10, RGBA8) and `linear_output` (18).
4. Copy 10 to the backbuffer, draw egui, Present.
5. **One frame in flight.** `submit()` blocks on the fence every frame, so per-frame upload buffers in `keep` survive until it signals. CPU writes into the shared upload CB are safe only because of that.

**Dirty tracking in `redraw()`.** Each frame, three serialized keys are compared with the previous frame's:

| Key | Covers | On change |
|---|---|---|
| `geometry_key` | Shapes and transforms, light placement, room and floor | `rebuild_scene` (new BLAS and TLAS) |
| `material_key` | The packed material bytes | `update_materials` (materials, emitters, lookup) |
| `transport_key` | All settings except display, camera-motion and name fields | Accumulation restart (see below) |

When `transport_key` changes, the accumulation restarts in one of two ways:

- **Camera-only change:** `reset(false)` sets samples to 0 but keeps RR and temporal history.
- **Any other change:** `reset(true)` also resets RR and the native temporal history.

A change to `rr` or `rr_preset` also calls `reconfigure_rr()`, which releases the NGX feature. `max_samples` and `spp` are in `transport_key`, so moving those sliders restarts accumulation.

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

**PackedMaterial** fields:

| Field | Contents |
|---|---|
| `color_rough` | RGB + roughness |
| `lobes` | metallic, reflectivity, transmission, flags. **bit0** = unlit (the path terminates on hit); **bit1** = hidden from camera rays (bounce 0 and the guide ray). |
| `optical` | IOR, emission, pattern id, one-sided emitter (1 for lights) |
| `secondary` | Pattern RGB + pattern scale. Patterns are evaluated in **world space**. |

**Emitter**:

| Field | Contents |
|---|---|
| `position_area` | Vertex a, triangle area |
| `edge1_cdf` | b−a, CDF |
| `edge2_probability` | c−a, selection probability |
| `emission_material` | Emitted RGB, material id |

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

There is one static linear-clamp sampler s0. The heap holds 512 descriptors: UAVs 0–31, egui textures from 32.

**Acceleration structures** (instanced since core track 4, see "Instancing"): a static BLAS (floor, room, lights; world space; two geometries, hidden lights non-opaque first, the rest opaque) as InstanceID 0, plus one BLAS per distinct object mesh in object space, instanced once per visible object with its transform. Glass objects are FORCE_NON_OPAQUE instances (their candidates run the `Proceed()` loop, so shadow rays can pass them), all other objects FORCE_OPAQUE. All use PREFER_FAST_TRACE.

**UAV textures.** Index = `u` register = heap slot. Formats are RGBA32F unless noted; resolution is render resolution unless noted.

| u | Purpose | Format / resolution |
|---|---|---|
| 0 | noisy (this frame's batch) | RR color input |
| 1 | accumulation (rgb mean, a = Welford M2 of luminance) | |
| 2 | normal_roughness | RR input |
| 3 | diffuse_albedo | RR input |
| 4 | specular_albedo | RR input |
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
| 31 | env_cdf: conditional CDF per row, then the marginal CDF in an extra row | R32F, **map width × (height + 1)** |

## Rendering behaviour worth remembering

- **Guide ray** (per pixel, before the spp loop): writes the RR guides.
  - Jittered with Halton(2,3) over 64 phases when RR is on; unjittered otherwise.
  - Specular hit distance comes from a deterministic mirror-reflection ray, not the sampled lobe. This was the RR stability fix.
- **Transport:**
  - NEE picks an emissive triangle by power. MIS uses the balance heuristic on both the NEE and the BSDF-hit side.
  - The BSDF is Lambert + GGX (Smith G1·G1 form) as a mixture chosen by `spec_probability`.
  - Dielectrics: stochastic Fresnel/TIR choice. Rough glass is an approximation (refracted direction lerped toward a cosine sample).
  - Beer–Lambert absorption inside glass. Russian roulette after bounce 3. The firefly clamp is off by default.
- **AOVs:**
  - direct = emission/environment at bounce ≤ 1, plus NEE at bounce 0.
  - spec = paths whose first bounce was specular or delta.
  - The direct/indirect sliders recompose the image from these.
- **Sampling:**
  - Native mode uses padded 2D Owen-scrambled Sobol. RR mode always uses white noise (NVIDIA's guidance).
  - There is no blue-noise mask, unlike the WebGL version.
  - The sequence index is `scene_info.z + samples + sample_id`. The base moves to a new 8192-aligned block on every reset, and the per-pixel scramble is fixed.
  - **Performance:**
    - Dimension pairs are computed together, and the second dimension uses a closed form (5-step superset transform). Before this, Sobol cost 6.5 ms vs 1.5 ms for white noise at 1440×900, 4 spp; now 1.7 ms.
    - The output is bit-identical to the old loop. Keep any change bit-identical, or re-measure convergence.
- **Native filter** (`post.hlsl`):
  - SVGF-style, with everything in albedo-demodulated light until the last pass:
    - **Pass 0, reproject** (camera moves only): bilinear warp of last frame's integration into this view. Each tap is checked against the object id, normal (> 0.9) and depth (3% + 1 cm). Coverage below 0.25 means no history.
    - **Pass 1, integrate:** blend the accumulation with history.
      - Weight h/(h+n): h is the history count, capped at 24 and lowered to 2 by the 3×3 specular share, then faded over 256 fresh samples.
      - Also integrates luminance moments. The variance is spatial (5×5) while fewer than 4 samples exist, otherwise from the moments.
    - **Passes 2–6, à-trous:** B3 kernel. Weights: object id (hard stop), normal pow 64, view-space plane distance, and luminance against 8·strength·√(3×3 Gaussian of variance).
      - Variance propagates with w², and pass 6 multiplies the albedo back in.
  - At render scale < 1 without RR, an upscaler takes over (see "Upscalers").
- **RR (`bridge/ngx.cpp`):**
  - Settings: DLUnified denoise mode, packed roughness, linear depth, IsHDR and MVLowRes flags, MV scale 1.
  - **NGX jitter offset = −ray jitter on both axes. Do not change it without rerunning `tools/rr_stability.py`.** Measured result: 94% less edge shimmer (`captures/rr-stability/regression/metrics.json`).
  - Quality mode is derived from the ratio of render width to output width.
  - The preset hint is set for every quality mode. Preset ids: 0 default, 4 D, 5 E, 6 F.
- **Exports:**
  - PNG reads texture 10 (no UI). With `--capture-ui`, `Gpu::keep_ui_frame` copies each backbuffer into `ui_frame` before Present, and the capture reads that.
  - EXR reads texture 18.
  - HDR PNG (`--capture name.hdr.png`, or the button in HDR mode) reads texture 26 through `Gpu::read_floats`.
  - A UI capture in HDR mode converts the FP16 backbuffer back to sRGB with paper white = 1.0.
  - High-resolution export resizes the swapchain to ≤ 4096, renders synchronously, then resizes back.

## Status (2026-10-29)

**The scene expansion is finished.** It was paused on 2026-10-26 and completed on 2026-10-29. It added:

- 24 presets and 11 shapes.
- Up to 256 objects (4096 since instancing) and 32 extra area lights.
- An object and light editor, viewport picking, procedural patterns and a material library.

The old `WORK_IN_PROGRESS.md` checkpoint was removed once this was done; this section replaces it.

**Done on 2026-10-29:**

- The self-test is extended to all presets and live edits (180 frames).
- `rr_stability.py` uses an unlit scene; it was rerun and passes.
- Randomize covers presets 0–22 and keeps studio layouts, recoloring only diffuse and glass objects.
- The preset camera carries over only between Cornell presets.
- The preset combo syncs from `scene_name`, so Load, `--settings` and Randomize all update it.
- The wheel and distance slider now reach 200, matching `validate()`.
- Editor slider ranges match `validate()`.
- The reflection guide ray runs only when RR is active.
- `direct` is zeroed alongside `radiance` on NaN samples.
- New `--set` flag; `--help` lists the presets.
- Chess (20) and Neon (21) cameras reframed. All 24 presets were checked at 1440×900.
- README brought up to date.

**Rendering at 1440×900** (DLAA RR, 4 spp, debug exe): 3.7–6.6 ms GPU per frame. The slowest are mirror corridor (17) at 6.6 ms, neon (21) at 5.3 ms and emissive spheres (8) at 5.1 ms.

**Git.** The project is a git repo (root `cornell/`), with a private GitHub remote. Commit only when the user asks.

**Disk use.** `target/` is about 3.5 GB and `vendor/` about 0.3 GB. The root `.gitignore` already lists `native/target`, `native/vendor` and `native/captures`.

## Upscalers (2026-10-29)

- **Setting:** `Settings.upscaler` (`Upscaler`: dlss default, fsr, xess, bicubic, bilinear). Used only when RR is off and `rw < width`.
  - It is in `transport_key`, so a change resets the render.
  - `main.rs` also calls `Gpu::reconfigure_upscalers()` when `rr`, `rr_preset`, `upscaler` or `dlss_preset` changes. This replaces `reconfigure_rr`.
- **`Gpu::prepare_upscaler`** creates the selected temporal upscaler lazily, at render size × output size.
  - A failure goes into `sr_failed`, and the display falls back to bicubic. `upscale_status` explains this in the UI.
  - `release_upscalers()` runs on resize and reconfigure.
- **Per-frame mode:** with a temporal upscaler active, `render()` calls `reset(false)` every drawn frame, so each frame is fresh jittered samples, as with RR. The native SVGF history (`camera_moved` → reproject) plus the upscaler do the accumulation. `run_upscaler` feeds color (ping when denoising, else accumulation), device depth (25) and motion (6), and writes texture 14.
- **Measured alternatives** (`tools/upscale_quality.py`, Classic, 1280×720, 50%):
  - Feeding the progressive accumulation (jittered or unjittered) measured worse: the upscalers assume each frame sits at its own jitter.
  - Still-camera quality now comes from a longer denoiser history: cap 256 where motion is 0, 24 otherwise (`post.hlsl` integrate).
- **Flags** in `scene_info.w`:
  - Bit 0: jittered primary rays. Both RR and temporal SR set it; trace reads it, no longer `modes.w`.
  - Bit 1: the display reads texture 14 (RR or SR output).
  - Bits 2–3: spatial filter (1 bilinear, 2 bicubic). Catmull-Rom, clamped to the 2×2 footprint against HDR ringing.
- **Jitter and motion conventions:**
  - All three take the same jitter sign as RR: the negated ray offset. Verified by probing all four sign combinations; the current one is best for each.
  - Motion is prev − curr in render pixels, scale 1. XeSS velocity scale is 1.
- **Results** (RMSE ×1000, light rim excluded; still / orbit):

| | Still | Orbit |
|---|---|---|
| Native 100% | 1.05 | 4.06 |
| RR | 1.90 | 2.60 |
| XeSS | 2.27 | 4.70 |
| FSR | 2.28 | 4.25 |
| DLSS SR | 2.62 | 3.82 |
| Bicubic | 4.15 | 5.91 |
| Bilinear | 4.70 | 6.01 |

  - With the rim included, DLSS SR and XeSS score about 9.5 on the still camera, because of how they blend the 12× HDR light edge.
- **Self-test:** frames 172–176 switch through every upscaler at scale 0.5 and assert that the available temporal ones start; frame 177 turns RR back on.
- **DLSS SR presets** (roadmap item 1, 2026-10-30): `Settings.dlss_preset` ∈ `scene::DLSS_PRESETS` (0 default, 10 J, 11 K, 12 L, 13 M), checked by `validate()` and kept across `preset()`.
  - `cornell_ngx_create_sr` sets all six `NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_*` hints on `sr_params`. A change reconfigures the upscalers (`main.rs` trigger list).
  - The UI combo shows only while DLSS SR is the active choice; `upscale_status` names the preset.
  - Self-test frames 180–184 cycle the presets and assert each starts; 185 restores default + RR.
  - Checked: at 50% (Performance), M renders identical to Default (NVIDIA picks M there), while J/K/L differ, so the hint reaches NGX.
  - SDK 310.9.16 has no RR presets beyond D/E/F.
- **egui scissor:** `draw_gui` skips meshes whose clipped scissor is empty. Off-screen windows after a self-test resize produced D3D12 warnings that failed the self-test, on `main` too.
- **Sliders:** all go through `crate::slider` (`SliderClamping::Edits`; since 2026-10-30 a `Slider` wrapper with double-click reset, see "Owner requests (item 6)"). egui's default clamps the stored value whenever a slider is drawn, which silently rewrote valid loaded values (found by the UI-plan agent). Self-test frames 178–179 guard this.

## Native HDR (roadmap item 2, 2026-10-30)

- **Settings** (display-only, not in `transport_key`, kept across `preset()`): `hdr_mode` 0 Auto (follow Windows), 1 SDR, 2 forced scRGB; `hdr_peak` and `hdr_white` in nits, 0 = from Windows.
- **Detection:** `App` holds a `display::Monitor`, re-queried on `Moved`/`ScaleFactorChanged` and when `current()` turns false (polled every 30 frames). This covers HDR toggles, calibration reruns and the SDR brightness slider. A change prints `DISPLAY DisplayInfo {…}`.
- **This machine:** Dell S3422DWG, Windows HDR on, 530 nits peak and full frame, SDR white 480 nits (slider at max). With white that close to the peak, the roll-off starts around 110 nits.
- **Swapchain:** `Gpu::set_hdr` flips `hdr_active` and calls `resize`, which uses `swap_format()` (RGBA16F or RGBA8) and sets G10/P709 or G22/P709.
  - **Color-space support depends on the current buffer format**, so check after `ResizeBuffers` (`color_space_ok`). Checking on the 8-bit swapchain always says no; if the check fails, it falls back to SDR.
- **Transform** (`post.hlsl` `hdr_map`), by tone map:
  - **Linear (0):** scene × exposure × paper white → BT.2390 EETF on the max channel in PQ space (knee = 1.5·PQ(peak) − 0.5) → min(peak) → /80. It keeps hue; the curve reaches the full peak only at 10,000 nits.
  - **Other tone maps (owner feedback: they had no effect in HDR):** t = `tone_curve` (the SDR curve, display light) ^ (2.2 / gamma). Below t = 0.5 the output is exactly t × paper white, the SDR brightness; above, u = (t − 0.5)/0.5 expands as 0.5 + 0.5·u/(1 − u + u/R), R = (r − 0.5)/0.5, r = peak/white: slope-continuous, reaching the peak at t = 1.
  - A first try, r·T(x/r), pushed midtones into the ACES toe (half as bright), and t/(1 − t + t/r) with no knee lifted ACES midtones by 27%.
  - Debug views and the A/B line use `sdr_in_hdr` (gamma-decoded × white).
  - `display::hdr_map` mirrors all of it (with `tone_curve`); unit test `hdr_tone_maps_keep_the_sdr_midtones_and_fill_the_headroom`.
  - `output_image` (10) stays SDR-tonemapped, so PNG, the sequence and high-res captures are unchanged.
  - The constants gain `hdr` = [on, white, peak, 0]; `eye.w` = white/80 tells `gui.hlsl` to linearize egui's premultiplied sRGB and scale it to paper white. There are two GUI PSOs (RGBA8 / RGBA16F).
- **Tests:** unit test `hdr_transform_keeps_midtones_and_respects_the_peak` (gray 0.4 at 200 nits → 1.0; clamp at peak/80; monotonic; hue kept) and `png_crc_matches_the_spec`.
  - Self-test 186–189: forced HDR with 1000/200; at 187 it reads back 18 and 26 and compares every pixel with `display::hdr_map` (worst relative error ~1e-3, the RGBA16F precision); 188 SDR; 189 Auto.
  - On an HDR monitor, Auto means the whole self-test runs in HDR.
  - The PQ PNG was decoded and matches linear EXR × white in BT.2020 to 0.1 nit.

## UI (phase 1 of docs/ui-plan.md, roadmap item 3, 2026-10-30)

- Layout: `TopBottomPanel` top bar and status bar, `SidePanel::right` (370 pt default, 320–560). Panel fill is 250/255 alpha over the render, which still fills the window (lens shift is phase 3).
- **Fixed slider width (130 pt)** inside `row`. Sizing sliders from `available_width` made the panel grow to its maximum on some tabs.
- Picking selects `Selection::Object`, switches to the Scene tab and shows the panel. Escape deselects (it no longer quits). Delete removes the selected object or area light. Tab cycles all → top bar only → nothing.
- Every single-key toggle and every save/load result is a toast; `self.status` is gone.
- **Render tab (reworked after owner feedback: the first version's "AI / Built-in / Off" split and "Upscaler: not needed at 100%" read wrong):** one Reconstruction section in the old menu's terms. "DLSS Ray Reconstruction" checkbox (disabled with `rr_status` when `Gpu::rr_available()` is false), RR model, Quality mode, Upscaler (disabled while RR is on), DLSS preset, "Native denoiser (D)", Render scale (advanced or off-list).
  - Quality mode entries come from `quality_modes`: with RR, DLAA (100%); without, Native (100%), plus DLAA / FSR Native AA / XeSS Native AA (100%) for a temporal upscaler, then 67 / 58 / 50 / 33%.
  - **`Settings.native_aa`** (default false): at 100% `prepare_upscaler` runs the temporal upscaler as anti-aliasing. Off by default so `--no-rr` at 100% stays the progressive path that unbiased references rely on. It is forced off for high-res export (DLAA restarts every frame) and is in the reconfigure trigger list. Self-test 196 checks that DLAA starts.
- Exposure is shown in EV (log2), written back only on change.
- egui's default fonts lack `→` and `●`: the glyph unit test caught both. The status dot is painted.
- CLI for captures: `--ui-tab`, `--ui-advanced`, `--ui-select`.
- Self-test: 178 opens the Render tab for the slider-clamp guard; 190–195 draw the object, light, main light, camera, Look and Export views with advanced rows, then delete the added light through `delete_selected`.
- Not done (later phases): undo, viewport handles and hover, scene browser with thumbnails, Quality bundles, per-pass GPU timing, persisted UI prefs, file pickers, async high-res export.

## Gizmos (roadmap item 3b, 2026-10-30)

- **State** in `UiState`: `tool` (Select / Move / Rotate / Scale, default Move), `local` (Global / Local), `hover` (handle under the pointer, from `overlay`), `transform` (running), `header`, and `undo` / `redo` stacks of `Settings` (≤ 100). `checkpoint` is called before deletes, adds, duplicates, reset-all and at the end of a transform (with the original).
- **Input** (`main.rs` `window_event`):
  - Left press on a hovered handle starts a handle transform (no orbit); release finishes it.
  - G / R / S start a modal transform when an object or light is selected and the pointer is over the viewport. R therefore only restarts when nothing is selected.
  - During a modal transform: left click confirms, right click cancels; keys go to `App::transform_key` (X / Y / Z, Shift for planes, digits, `.`, `-` toggles sign, Backspace, Enter, Esc).
  - Pointer deltas accumulate into `Transform::mouse`, ×0.1 with Shift. Ctrl snaps (0.1 units, 15°, 0.1×).
  - A short click picks objects and light quads (`gizmo::pick`); empty space deselects.
  - Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z, undo and redo.
- **Math:** an axis move uses the axis's on-screen direction (so the pivot follows the pointer, checked to 3 px); plane and free moves intersect pointer rays with the plane through the pivot; rotation is the screen angle around the projected pivot, with the sign flipped for axes facing away; scale is the ratio of pointer distances from the pivot. Rotations compose as quaternions and go back to YXZ Euler (objects: yaw = `rotation`, `tilt` = pitch / roll; area lights: `rotation` = [pitch, yaw, roll]). Per-axis scale uses local axes: X `scale_xz[0]`, Y `stretch`, Z `scale_xz[1]`, free = `size`. Main light: X scales width keeping depth, Z changes `light_aspect`; no rotation.
- **Drag cost** (release, 1440×900, `--drag-test`, frame ms): Classic 4.6 → 6.6, bunny 4.7 → 20, chess 4.9 → 22.
  - `geometry()` rebuilt every object (about 11 ms for 600k vertices), so it now caches each object's mesh per slot (`OBJECT_CACHE`, keyed by the `Object` and its material id). That saved about 4 ms.
  - The rest was new upload, BLAS and scratch allocations, plus the whole-scene BLAS build. Instancing (core track 4) fixed it: drags now rebuild only the TLAS (see "Instancing").
- Capture CLI: `--ui-tool`. Self-test 198–202: each tool drawn on object 0 and the main light, then a keyed G X 0.5 Enter (asserts +0.5 and that the transform ended), then undo (asserts the move is gone).
- Not done: see the list under roadmap item 3b (object snapping, pivot options, multi-select, Alt-drag / Shift+D duplicate, vector entry). Cheap drags came with instancing.

## Scene files (roadmap item 4, 2026-10-30)

- `scenes/builtin/` holds 42 files plus `pack.json`, **generated by `tools/build_scenes.py`**: edit scenes there, never the JSON.
  - Helpers mirror the presets: `obj`, `float_at` (non-grounded, y is the mesh center), `light` with `aim()` (pitch/yaw so the downward quad faces a target), `kelvin()`, `studio()` and `dark_stage()`.
  - `check()` mirrors `validate()` ranges and fails with a list. `obj()` moves excess `stretch` / `scale_xz` into `size` (same world size) and wraps rotation to ±180.
  - Files are sparse (omitted fields take `Settings::default()`). Objects are always written in full because `Object::default()` is the tall block (stretch 1.7, rotation 18).
- `tools/scene_sheet.py` renders every scene (480×300, 48 frames, RR) into `captures/scenes/sheet.png`. All 42 were checked by eye and retuned.
- **Engine fix found by the scenes:** a light that is off (main `light_on`, area `enabled`) used to keep its unlit quad in the BLAS: black in reflections, and a shadow caster. `geometry()` now skips those quads, `geometry_key` includes `light_on` / `enabled`, and gizmo outlines and picking skip them too.
- Scene title = `scene_name`. The top-bar picker shows it, and the Scene tab shows the file's description. ◀ ▶ step through files while a file scene is loaded.
- Scenes window (`UiState::show_scenes`): search, tag chips, list grouped by pack, details, Load, "Save to scenes/user".
- CLI: `--scene`, `--ui-scenes` (opens the window for captures).
- Self-test frames 204+: load each listed scene file and assert the title; the per-frame GPU count checks cover each scene's geometry.
- **Thumbnails and licences (2026-10-30, owner request):**
  - `gpu.rs` supports any egui texture. `gui_textures` maps `TextureId` → texture, heap slot (32..512, `free_slots`) and a CPU copy for partial updates. `update_gui_textures` replaces `update_atlas`; `delta.free` is applied after `draw_gui`; `draw_gui` binds root table 4 per mesh texture. The heap grew from 64 to 512 descriptors.
  - `scenes::Entry` has `hash` (FNV-1a of the file), `credits` and `license` (from `pack.json`: `license`, `commercial_use`, `attribution`, `source`). `Entry::thumbnail()` = `scenes/thumbnails/<pack dir>--<stem>-<hash>.png` (gitignored).
  - `scenes::render_thumbnails(entries)` runs this exe headless per scene on a thread (320×200, 32 frames, `CREATE_NO_WINDOW`, written as `.partial.png` then renamed, stale hashes removed) and reports each result on a channel. The Scenes window starts it for missing thumbnails on first open, never when `hidden`. `update_thumbnails` loads at most 4 PNGs per frame into egui textures. `--make-thumbnails` does the same from the CLI and exits.
  - Cards are painted at a fixed 160×100 (egui's `ImageButton` padding made the grid uneven). The selected scene shows a 256×160 preview, licence badge, tags, attribution and credits.
  - Self-test 203 loads a generated texture as entry 0's thumbnail and opens the window with it selected, so the scene-file frames draw the textured UI under the debug layer.
- Relative asset paths: done with mesh import and HDRIs (paths relative to `scenes/`, see `mesh::resolve`).

## ffmpeg (2026-10-30, owner allowed pulling the binary)

- `tools/setup.ps1` `Get-Ffmpeg '9.0.2' '<sha256>'` downloads gyan.dev's essentials build from its GitHub release mirror (`GyanD/codexffmpeg`). The SHA-256 is pinned (it matched gyan.dev's published checksum on 2026-10-30). Only `ffmpeg.exe` and `LICENSE` are kept, in `vendor/ffmpeg` (gitignored with `vendor/`). A `VERSION-<ver>` marker skips repeat downloads.
- build.rs copies it next to the exe when present (optional; builds work without it). `video::ffmpeg()` prefers that copy, then PATH. The snapshot and screensaver scripts copy it too.
- Linux port notes are listed in docs/roadmap.md under "Vulkan backend".

## Generators (core track 1, 2026-10-30)

- `generators.rs`: `generate(family, seed) -> Settings` for the 17 `FAMILIES` (the last four use procedural meshes, see "Mesh import"). RNG is xorshift64* seeded by seed and a hash of the family.
  - Helpers reuse `presets::{object, light, studio, material}` (now `pub(crate)`). `at()` places a mesh centre at a world y. `sun()` is a far quad aimed at the origin (warm 350, day 450, sky 0.2 / 0.55; brighter washed out). `frame()` is `frame_objects` × 0.78.
  - `finish()` names the scene "<family> · seed N", truncates to `MAX_OBJECTS` and runs `fit()` (excess stretch/scale into size, angles wrapped, as build_scenes.py does).
- New shape **Tetrahedron** (last in `Shape::ALL`; the mesh cache is indexed by enum order): base at y = −1, apex at +1, base circumradius √2.
- Menger level 2 merges filled cells into runs along x (fits under 256). The Sierpinski child centre is c + corner·scale/2. The sphereflake uses Haines' 6 + 3 directions, rotated per parent.
- Scene files with `"generator": {"family", "seed"}` regenerate in `scenes::read` (the title overrides the name). `scenes::save_generated` writes such a file to scenes/user. `tools/build_scenes.py` also writes `scenes/generated` (6 curated seeds × 15 families plus 3 each for forest and crowd; `GENERATED` table).
- Tests: every family × 24 seeds validates and is non-empty; generation is deterministic; Menger fits. The self-test sweeps every scene file, now 160 (430 frames).
- `tools/scene_sheet.py --pack generated` renders the pack. All 90 were checked; day cities and warm suns were dimmed, framing tightened.

## Scenes buildable today (roadmap 5e, 2026-10-30)

- `Settings.mis_mode` (0 MIS, 1 NEE only, 2 BSDF only) → `scene_info.w` bits 4–5. In the trace, mode 1 gives BSDF-found emission weight 0 (unless it came after a delta bounce) and NEE divides by `pl` alone; mode 2 skips NEE. Checked on veach-mis: the classic three images.
- `ui::seed_of_the_day()` (days since epoch × golden ratio) feeds `randomize` from the top bar's *Today* button.
- New generator scenes in `tools/build_scenes.py` use `room_shell()` (walls / ceiling / front as thin boxes) and `obj(portal=…)`:
  - `kusama-infinity-room`: 250 objects, emission 1.6, exposure 0.8; it blew out at emission 9.
  - `flavin-fluorescent` and `turrell-ganzfeld`: rounded seams are 1.6-scale cylinders in every corner and along the edges.
  - `infinite-corridor`: the end portals are paired, the near one rotated 180°.
  - `bigger-on-the-inside`: the box door is paired with the hall doorway at x = 30, which **must be rotated 180°** or the view leads back out of the hall.

## Mesh import (core track 2, 2026-10-30)

- `mesh.rs`: `Mesh = Vec<(Vec3, Vec3)>` (position, normal per vertex, 3 per triangle), at most 20M triangles and 2 GB files. A cache maps path → `Result<Arc<Imported>, String>` (errors are cached too). `resolve` makes relative paths relative to `scenes::folder()`; `store` is the inverse. `load`, `of(&Object)`, `import_object(path)` (size 2, material from the file).
  - OBJ via tobj (triangulate, single index; MTL diffuse). glTF via `gltf::Gltf::open` + `import_buffers`, following node transforms; material from PBR plus KHR emissive_strength / transmission / ior (each needs a Cargo feature). Smooth normals when missing; normalised into −1..1.
- `Object.mesh` (≤ 1024 chars) replaces `shape` when set. `geometry.rs` uses `mesh::of(o)` in `object_frame` / `object_geometry`; a missing file falls back to the shape. The inspector shows a Mesh row (file name, "missing", *Use a shape*); the Scene tab has *+ Mesh…* (rfd file dialog).
- **Opaque split** (`Gpu::rebuild_scene`): triangles whose material needs the any-hit loop (`lobes.w` bit 1 hidden, or transmission > 0) go first as geometry 0; the rest is geometry 1, flagged OPAQUE. `opaque_offset` (count of the first range) goes in `environment.w`; `intersect` adds it when `CommittedGeometryIndex() != 0`. `geometry_key` includes transmission > 0 and light visibility for that reason. Bit-identical renders on presets 0, 14 and 17 (RMSE 0.000).
- The vertex buffer is a DEFAULT-heap buffer created COMMON (creating it COPY_DEST warned), filled by a copy from a staging upload, then NON_PIXEL_SHADER_RESOURCE.
- **GPU picking:** a click stores `pick_point`; that frame's probe also copies u19 (object id) to `probe_albedo + 768`, so `Probe` has `id`. `gizmo::pick_from_probe` maps id − 1 (4 main light, 5..5+N objects, then area lights) and still tests light quads on the CPU, nearer than the GPU hit (lights hidden from the camera are not in u19). `gizmo::pick` (CPU, every triangle) stays for tests.
- Self-test 257–259: GPU pick of object 0's projected centre; 258 imports an OBJ pyramid from the temp folder (asserts 6 triangles) plus an object with a missing mesh file.
- Unit tests: OBJ quad, glTF triangle with an embedded base64 buffer, an OBJ with two materials split into parts.
- **Parts:** a file is read as one part per material (glTF material index, OBJ `material_id`), all normalised by the file's box. `Object.mesh` = `file#n` picks part n; plain `file` is every part merged (`Imported::whole`, built lazily). `mesh::ground(o)` grounds a part by the whole file, so the parts of one file stay together. Empty parts are kept so indices never shift (terrain bands can be empty). `import_objects` (used by *+ Mesh…*) makes one object per non-empty part, named `<stem> · <material>`.
- **glTF textures are averaged** (track 5 will sample them): base colour (sRGB → linear) and emissive multiply their factors; the metallic-roughness map's B and G multiply metallic and roughness (unaveraged, chess pieces came out fully metallic). 8-bit RGB/RGBA images only.
- `--mesh-info files…` prints each file's parts as JSON `Object`s plus triangle counts and exits. `tools/mesh_parts.json` is that output for the test models; `tools/build_scenes.py` reads it for the `scenes/meshes` pack (8 scenes: gold dragon in the Cornell box, jade Buddha, scan gallery, Spot and Suzanne, the ASWF chess set, glass dragon, toy car, duck and lantern).
- **Test models:** `tools/fetch_assets.ps1` (was fetch_models.ps1) downloads 11 models into `scenes/models/` (gitignored), pinned to a commit and SHA-256: 6 OBJ scans from alecjacobson/common-3d-test-models (xyzrgb dragon, happy Buddha, armadillo, Lucy, Spot, Suzanne) and 5 GLBs from KhronosGroup/glTF-Sample-Assets (DragonAttenuation, Duck, Lantern, ABeautifulGame, ToyCar). Lucy is Z-up (tilt −90). Without them the mesh scenes fall back to boxes. The snapshot script copies them with the scenes (about 95 MB).
- **Procedural meshes** (`procedural.rs`): `mesh: "gen:<kind>/<seed>[#part]"` builds in `mesh::read` and caches like a file (`resolve` leaves `gen:` paths alone). Kinds: `mandelbulb` (surface nets over the distance estimator, 240³ release / 80³ debug, sampled on all cores; power 5–9 and a phase twist per seed), `terrain` (384² fBm heightfield, ridged half the time, island falloff; parts sand / grass / rock / snow by height and slope, rock below normal.y 0.62), `menger` (voxels with exposed faces only; level 4 when seed % 3 == 0 in release, else 3), `tree` (recursive branching: 6-sided frusta, double-sided leaf cards; parts bark / leaves). The generator families `mandelbulb`, `terrain` (plus a sea box: stretch 0.065 puts it at height 0), `deep-menger` and `tree` (1–7 trees, summer or autumn) use them.
- **Perf of the opaque split** (2026-10-30, 1440×900, 300 frames, median of 3 alternating runs vs the core1 snapshot): Classic −7% RR / −8% native, bunny −7 / −8%, mirror corridor −3 / −3%, chess −7 / −4%, neon −3 / −4%.

## Owner requests (item 6, 2026-10-30)

- **Slider double-click reset:** `crate::slider` returns `Slider`, a wrapper around `egui::Slider::from_get_set` (as `Slider::new` builds it, plus `.integer()` for integral types) that forwards `suffix`, `logarithmic`, `smallest_positive`, `fixed_decimals`, `custom_formatter` and `custom_parser`. Its `Widget::ui` checks `slider_double_clicked` (hovered or pressed + `pointer.button_double_clicked`: a slider senses drags, so egui never reports it as double-clicked) and then writes the reference value.
  - The reference comes from `reset_source(live, reference)` registrations (address range of a live struct → same offset in a reference of the same type; `reset_value` picks the innermost range). `App::ui` registers the settings against `reset_baseline` (a clone taken whenever `scene_name` changes: the scene as loaded, preset, generated or randomized), each object and light against the baseline's same index (or the type default), and `UiState` against `ui_defaults`, and clears them after the frame. The exposure row (a local EV value) resets by hand. Hover text "Double-click to reset" on every slider.
  - Unit test `double_clicking_a_slider_resets_it` drives a real egui `Context` with pointer events: a rail click moves the value, a double-click resets it.
- **Light outlines:** `Settings::light_outline` and `AreaLight::outline` (default false; display only: removed from `transport_key`, per light too). `gizmo::draw_lights` skips lights whose outline is off unless selected; `pick_from_probe` only tests drawn outlines (hidden ones are picked in the Scene tab). "Show outline" rows sit under each light's "On". Self-test 266–268 asserts they default off, then draws all of them.
- **HDRIs:** radiance is bilinear (`env_bilinear`, wrap in u, clamp in v); the sampling pdf stays per texel, and MIS still agrees (studio and noon checks rerun: means 0.4486 / 0.4486 / 0.4485 and 0.5969 / 0.5970 / 0.5945, as before). The map is RGBA16F (`f32_to_half`, unit test `half_floats_round_trip`), divided by `env_scale` = max(1, peak / 60000) so suns past 65504 survive; strength and `inv_norm` are multiplied back in the constants. `MAX_WIDTH` 8192.
  - 21 Poly Haven maps were rendered as backgrounds at 1440×900 and checked at 1:1; satara_night (sensor noise) and moonless_golf (motion blur) were dropped, 19 kept (4k; 8k for kloofendal, venice_sunset and the_sky_is_on_fire). The old 2k files are gone. Before/after on venice_sunset: the 2k nearest-texel skyline was visibly blocky, 8k bilinear is smooth.
  - `scenes/environments` gained night on the Bund, teapot in the hallway, snow day and gold dragon at dusk (9 scenes).

## Instancing (core track 4, 2026-10-30)

- **Layout:** `Gpu::rebuild_scene` builds (1) the static BLAS from `geometry::static_geometry` (floor, room, main light, area lights; world space, split into non-opaque / opaque ranges as before, `opaque_offset` in `environment.w`), only when its vertices change; (2) one BLAS per distinct mesh key (`geometry::object_source`: `shape:<Kind>` or the `mesh` string), object-space, in `meshes`; (3) a TLAS of the static instance (InstanceID 0) and one instance per visible object (`geometry::object_transform`, DXR 3×4 row-major).
- **Mesh buffer:** every mesh in use is packed into one object-space vertex buffer (t1). When the set of mesh keys changes it is rebuilt and only new keys get a BLAS (a built BLAS holds its own copy of the triangles, so old ones are reused with new `first` offsets). The static vertices live in their own buffer (t6), so light edits don't touch the meshes.
- **Instance table** (t7, `InstanceRecord` ↔ `Instance` in common.hlsli): `vertex_base`, `material` (5 + object index; u32::MAX for the static instance, whose vertices carry ids), `emitter_base` (start of this instance's range in `emitter_lookup`, u32::MAX for non-emissive objects), `object`. Built in `rebuild_scene`, emitter bases filled and uploaded in `update_materials`.
- **Emitters** stay world-space triangles: static ones, then each emissive object's `object_geometry` (in instance order). The self-test's count check (full soup vs GPU) still holds.
- **Shader:** `candidate_material(instance, prim)` in both `Proceed()` loops; `intersect` reads the instance, then static vertices (+ `opaque_offset` for geometry 1) or `vertices[vertex_base + prim·3]` with the normal taken to world space as `mul(n, (float3x3)CommittedWorldToObject3x4())` (inverse transpose).
- `geometry::lowest_y` caches grounding per (mesh pointer, rotation, scale) because instancing asks for every object's frame on every rebuild (a chess part scans the whole 1.6M-vertex file).
- `frame_objects` is exact up to 256 objects; past that it uses each mesh's box corners through the instance transform (a 3600-object forest took minutes in the debug unit tests). The generator test runs 3 seeds for forest / crowd; unit tests take about 60 s in debug.
- `MAX_OBJECTS` = 4096. New generator families `forest` (400–1800 trees from 6 shared tree meshes, two objects each) and `crowd` (up to 54² copies of one shape).
- **Measured** (1440×900, release, median of 3 alternating runs vs the core3 snapshot): drag frame time bunny 26 → 5.5 ms, chess preset 24 → 6.1 ms, ASWF chess set 141 → 7 ms. Plain render GPU time: Classic +7% RR / +5% native, bunny +1 / +5%, chess +6 / +2%, mirror corridor −3 / −3%, neon −2 / +1% (two-level traversal costs a little in tiny scenes).
- **Render check** vs the core3 build (raw native, 512 and 4096 spp): RMSE ×1000 below 0.2 on presets 0, 2, 8, 13, 20 and the chess set; glass scenes differ by noise (the glass dragon 7.4 → 2.7 from 512 to 4096 spp). The glass lab (14) keeps a difference where glass objects' bottom faces lie exactly on the floor: coplanar faces z-fight, and the instance transform moves them by float rounding, so the other face wins in places. Neither build is more correct.

## HDRI environments (core track 3, 2026-10-30)

- **Settings:** `hdri` (path, ≤ 1024, relative to `scenes/` like meshes), `hdri_strength` (0–1000), `hdri_rotation` (±360°), `hdri_visible` (camera rays see it; off, bounce-0 misses return `background`). All transport fields. Stage section of the Scene tab: Load… (rfd, .hdr / .exr), Clear, strength, rotation, "Seen by camera".
- `environment.rs`: `load` (image crate, `hdr` and `exr` features; non-finite → 0) → `build(w, h, rgb)`: halves maps wider than 4096, then per row a conditional CDF of luminance, and a marginal CDF over rows weighted by sin θ of the row centre, in the extra last row. `inv_norm` = w·h / (2π² Σ lum·sinθ_row). A black map is an error.
- **GPU:** `Gpu::set_environment` uploads both through `upload_texture` (staging per footprint row) into textures that live across resizes (`env_views` at heap slots 30, 31, re-created in `resize`). Constants gain `environment_map` = [strength, rotation rad, flags (bit 0 loaded, bit 1 visible), inv_norm]. `environment_loaded()` for tests. `main.rs` reloads when `Settings::hdri` differs from `hdri_loaded`; a failure clears it with an error toast.
- **Shader:** sampling and the pdf work per texel; radiance is bilinear since item 6 (`env_bilinear`), u = φ/2π + 0.5 with φ = atan2(x, −z) − rotation, v = θ/π. `sample_env` binary-searches the marginal then the row, reusing each remainder for the position inside the texel; pdf = lum·inv_norm·sinθ_row / sinθ. NEE picks the HDRI with `p_env` = 0.5 when emissive triangles exist (1 otherwise); triangle NEE pdfs and BSDF-hit weights carry (1 − p_env). Misses after a non-delta bounce get the MIS weight against `p_env·env_pdf`. The light-sampling modes (`mis_mode`) work as for triangles. Without an HDRI nothing changes (renders match the previous build to 2e-7).
- **Checked:** spheres under the noon sky (a sun a few texels wide) and the photo studio, 2048–4096 spp, MIS vs NEE-only vs BSDF-only: means agree to 0.02% on the studio; on the noon sky NEE-only matches MIS to 0.27% per 20×20 block while BSDF-only is the expected firefly field. Unit tests: the pdf integrates to 1; big maps halve; black maps fail.
- **Assets:** `tools/fetch_assets.ps1` also fetches Poly Haven HDRIs (CC0, SHA-256 pinned) into `scenes/hdri/` (gitignored): 19 at 4k / 8k since item 6 (the first five were 2k). `scenes/environments` (9 scenes, `ENV_SCENES` in build_scenes.py; they start with no lights, since the studio's hidden rim light still casts shadows).
- Self-test 260–265: a generated 64×32 .hdr with a sun (written with the image crate), asserted loaded; hidden from the camera; mis modes 1 and 2; cleared and asserted unloaded. Scene files start at 270.
- Not done: the sun is not split out as a separate light (its shape is the map's texels), no ground projection / shadow catcher, no MIS compensation. (Maps are RGBA16F since item 6.)

## Toys (roadmap 5d, 2026-10-30)

- **Light painting:** `App::painting` → `Gpu::painting`. That makes `rr_active` false and `prepare_upscaler` return None, and stops `update_materials` and `rebuild_scene` from resetting. In `redraw` the light fields are removed from the transport-key diff, so light-only edits skip the reset. The sample cap is ignored while painting. Toggling it resets once.
- **Portals:**
  - `Object.portal` (index or −1; `validate` checks the pair). `lighting::materials` appends one record per portal holding the 3×4 rows of `portal_matrix(a, b)` = local(b) · diag(−1,1,−1) · local(a)⁻¹, where local = translate(pivot) · R · scale(0.35·size·(sx, stretch, sz)).
  - The object's own record gets flag bit 2 in `lobes.w`, a white colour, no emission and no pattern, and `secondary.w` = the matrix record's index.
  - Trace: on a portal hit, the map lands on the pair's far face, so a hit from inside (`!h.front`) just passes through. Up to 16 crossings per path, each with `bounce--`. `previous_delta = true` gives emission found after a crossing MIS weight 1.
  - The guide ray sees the portal surface itself, so RR guides are approximate there.
- **Walk mode** (`walk.rs`): moves the orbit camera's eye with distance 0.5, so looking turns in place (`place()` re-derives the target, clamped to ±20). Collision is a 0.22-unit sphere against `scene::geometry` triangles (AABB prefilter, two push-out passes, Ericson's closest point), rebuilt when `geom_key` changes. A dedicated `KeyboardInput` arm tracks held keys while walking. Unit test: walking into the Cornell back wall stops short of it.
- **Turntable** (`video.rs`): resizes with progressive settings and, per frame, sets the yaw, `reset`s and renders until `spp`. `Gpu::read_rgba8(10)` reads the frame, which goes to ffmpeg's stdin (`-f rawvideo -pix_fmt rgba … libx264 yuv420p crf 16`) or to `<name>/00001.png…`. `--turntable` triggers at rendered frame 2 of a headless run (frames default to 4).
- **Screensaver** (`saver.rs`): `run()` handles `/p HWND` (a GDI `StretchDIBits` of the first scene thumbnail every 0.5 s until `IsWindow` fails). `App::new` treats `/s` as show mode and ignores any other `/x` switch, so `/c` is the normal app.
  - Show mode: borderless fullscreen, hidden cursor, no UI. It cycles presets (except the empty stage) then scene files every 45 s with a slow orbit.
  - Input quits **after a 1.5 s grace period, ignoring modifier keys**: on this machine Windows sends a non-synthetic AltRight press when the fullscreen window takes focus.
- **Gamepad:** `gilrs` is polled in `remote_and_gamepad` (interactive runs only). Untested: no controller here.
- **Phone remote** (`remote.rs`): std `TcpListener` on 8642–8661 (`0.0.0.0` from the UI, `127.0.0.1` in tests). Every request needs `?t=<8-char token>`.
  - Endpoints: `GET /` (embedded page), `GET /state` (JSON of `KEYS`, refreshed every 0.5 s), `POST /set?k=key` with a JSON body, limited to `KEYS` plus `preset` (±1) and `restart`.
  - Edits apply through serde and `validate()`, like `--set`.
  - Turning it off flips an `AtomicBool`, after which the listener answers 410 (the port stays bound until exit).
  - QR code via the `qrcode` crate → egui texture.
- Self-test 240–256: painting keeps samples across a light move (241), portal records (244), walking moves the camera (252), a 3-frame turntable (253), a remote edit over HTTP applied (256). Scene files start at 260.

## Camera and look (roadmap 5c, 2026-10-30)

- **Constants grew past 512 B:** passes now sit at `CB_STRIDE` = 1024 (trace 0, filter 1–7, display 8; buffer 9 KB, compile-time assert). New blocks: `lens`, `camera_model`, `look`, `lift` (w = grain cell size), `grade_gamma` (w = LUT strength), `gain` (w = frame seed). Mirror any change in common.hlsli.
- **Trace:** `camera_ray(pixel, lens_sample, …)` handles every model. Guides use the pinhole ray (`lens_sample` false), so RR and the denoiser guides stay sharp.
  - Lens: `aperture_point()` gives a disk or an n-gon fan. The focal plane is at `focus_distance` along the view axis, or tilted about the camera right axis (model 4).
  - Cat-eye rejects lens points outside a circle shifted by the pixel's NDC (`valid=false` → zero throughput).
  - Ortho uses `camera_model.w` = distance·tan(fov/2) as the half height. Equirect is ±π by ±π/2. The little planet is a stereographic projection looking down.
  - Non-perspective guide depth is the hit distance.
- **`Settings::perspective()`** (models 0 and 4) gates RR (`rr_active`), the temporal upscalers (`prepare_upscaler`), click picking and the gizmo.
- **Display:**
  - `lens_and_film` adds halation and vignette on linear light before the tone map. Halation is 24 taps of `scene_at` (RR/SR output or the native image) above 1.0, tinted red-orange.
  - `film_look(t)` does lift/gamma/gain on the encoded value, then the LUT (manual trilinear over u29). It runs inside `tonemap` and in `hdr_map`'s tone-mapped branch (not the Linear HDR branch).
  - Grain is hashed per cell and frame, weighted to the midtones, after the tone map.
  - Tone maps 6 AgX and 7 PBR Neutral (`validate` allows < 8).
- **LUT:** u29 is a 3D RGBA32F texture (`texture_3d`) kept outside `textures`, so `resize` only re-creates its view (`lut_view`, heap slot 29). The root UAV range is `TEXTURES + 1`. `look::read_cube` parses .cube (3D, domain 0..1, size 2–128). `App` reloads it when `lut_path` changes.
- **Probe** is now `gpu::Probe { linear, albedo, depth, id }` (depth from u5, id from u19). Ctrl+left-click sets `focus_pending`; the next readback's depth becomes `focus_target`, eased with 1 − e^(−10·dt) per frame.
- Look fields (halation, grain, grain_size, vignette, lift, grade_gamma, gain, lut_path, lut_strength) are display-only, so they are excluded from `transport_key`.
- Self-test 216–239: aperture, blades and cat-eye; models 1–4 (218 asserts RR off); AgX, PBR Neutral; the film look plus a generated inverting LUT (224 asserts it loaded); Ctrl+click focus at the center (239 asserts it settled). Scene files start at 240.
- Visual check: `captures` at 640×400 of bokeh, ortho skyline, equirect Stonehenge, little-planet forest, tilt-shift skyline, AgX + halation/grain/vignette, PBR Neutral, all as intended.

## Debug views and meters (roadmap 5b, 2026-10-30)

- `Constants.tools` (mirrored in common.hlsli): zebras, scopes, heat scale for bounces (min(bounces, 8)), and BVH steps per sample at the top of the log map (256).
- Trace counts `proceed_steps` in both `RayQuery` loops and the bounces used, written to u27. Views 8–11 are in `display`: 8 is `(bounces − 1) / (max − 1)`; 9 is `log2(1 + steps/spp) / log2(257)`; 10 is the variance of the accumulation (log); 11 is false colour in EV around 0.18 of the exposed image.
- Scopes: `bin_scopes` atomics in `display` on the final SDR value. Before display, `clear_scopes` copies a zero upload buffer into u28; after display, `queue_readbacks` copies u28 and the light-meter texels (u18 at the output pixel, u3 at the render pixel) into one readback buffer. `finish_readbacks` maps it after Present into `Gpu::scopes` / `probe_result`.
- App: `g.zebras` / `scopes_on` / `probe` are set from `UiState` before `render`. The convergence graph reads u18 with `read_floats` at sample counts growing by 1.25× and computes RMSE ×1000 of l/(1+l) on the CPU (a 20 MB readback each time, so a hitch). The graph clears when samples drop.
- `stats.rs`: loaded only when the window is visible; saved every 30 s and on close. Achievements are checked each frame in `redraw`.
- Self-test 204–207 cycle views 8–11. 208 turns on zebras, scopes and the meter (cursor at the center). 209 asserts the luma histogram counts exactly width×height and the meter texels are finite. 210 sets the current image as the reference and restarts. 215 asserts ≥ 2 finite graph points and turns the tools off. Scene files start at frame 220.
- Capture CLI: `--ui-scopes` (scopes + zebras).

## Tooling (roadmap 5a, 2026-10-30)

- `tools/perf_history.py` appends to `captures/perf-history.csv` (date, commit, `+` if src/shaders/bridge are dirty, preset, mode, means) and plots the PNG with matplotlib. First row, commit 3c4c8a4+ (GPU ms, RR / native): Classic 3.81 / 3.13, bunny 3.96 / 3.90, mirror corridor 5.98 / 5.71, neon 4.52 / 4.27.
- **Build snapshots:** `tools/snapshot_build.ps1 -Name N -Notes T` copies exe + DLLs + scene packs (not thumbnails) into `builds/<date>-N/` with NOTES.md (gitignored). `scenes/models` and `scenes/hdri` are hard-linked, not copied (about 800 MB of pinned downloads). The owner asked for one per major item, so they can compare builds later. The exe finds `scenes/` next to itself.

## Review findings: fixed and merged (2026-10-29)

Done on branch `review-findings` and fast-forwarded into `main` (commit c72f163). The pre-fix state is commit 1fc032d. All results were measured.

**Native denoiser** (see "Rendering behaviour" for the design). Scores are ×1000, measured with `tools/motion_error.py`.

| Case | Before | After |
|---|---|---|
| Orbit, 2 spp: error / blurred / light band | 20.65 / 16.94 / 36.9 | 9.63 / 4.40 / 7.0 |
| Still camera, 16 spp | 12.96 | 6.31 |
| Still camera, 1024 spp | 4.79 (raw 2.43) | 1.56 |

- The old filter was biased: its blurred output was fed back as history, so it was worse than raw above 64 spp.
- The white specks came from per-pixel variance estimates that were near zero by chance. The 3×3 variance prefilter fixed them, so no separate firefly clamp was added.
- The light "ghost" was the spatial and temporal blur crossing from the light to the ceiling. The object-id stop fixed it.

**Sobol** decorrelates per reset and is now 4× cheaper. See "Rendering behaviour".

**Other fixes:**
- Pan follows the screen axes, and the target is clamped to ±20.
- The Pause glyph is ⏸, and the resolution label reads "Render … · output …".
- The `tests::ui_strings_have_glyphs` unit test checks every non-ASCII character in UI strings against egui's fonts.
- The Delete key only works while the Scene objects window is open.
- The UI capture is copied before Present.
- Marble veins are thinner and softer.
- The CLI is order-independent.
- RENDER_OK prints means.

**Performance:** 1440×900, 4 spp, mean of 200 frames, before → after on this branch.

| Case | Before | After |
|---|---|---|
| Classic, raw | 2.87 | 1.64 |
| Classic, native denoiser | 7.48 | 3.10 |
| Bunny (13), native | 7.53 | 3.86 |
| Mirror corridor (17), native | 10.04 | 5.75 |
| RR | 3.95 | 3.88 |
| With UI drawn, wall clock | 11.8 | 4.2 |

The gains come from:
- First-hit shadow rays.
- The Sobol pair cache.
- The packed filter guide with demodulated filtering.
- One egui upload per frame.
- Skipping dirty-key serialization when settings are unchanged.

**Not a bug:** windows going off-screen after a resize only happened because the self-test resizes the swapchain without resizing the window.

**Still open (not started):**
- Faint low-frequency blotches at very low spp, which is typical of SVGF.

## Measurement conventions (carried over from the WebGL work)

- Error metric: RMSE ×1000 on compressed luminance l/(1+l) against a high-spp reference. The WebGL reference was 16,384 spp.
- Also track a 5×5-blurred error, which catches ghosting that plain RMSE hides.
- Compare against an **independent** reference: a white-noise reference when testing Sobol.
- Use numpy on EXR captures. imageio has no EXR backend here, and OpenEXR and cv2 are not installed.
  - The exe writes **tiled (64×64), RLE-compressed** RGB float EXRs.
  - Use `tools/exr.py`: `exr.read(path)` returns an H×W×3 float32 array. It handles scanline or tiled files with NONE or RLE compression.
  - It was used to confirm that direct + indirect recompose the beauty to 2e-7 relative error (presets 0, 3, 8, 21; raw native path).
- Report in a table, with the sample count where each method reaches equal error.
