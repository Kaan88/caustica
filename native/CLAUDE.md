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
  - Near term: DLSS presets, native HDR, UI phase 1 and Blender-style gizmos are done (2026-10-30); next are scene files, then quick wins (hot reload, debug views, depth of field, light painting, portals, screensaver).
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
  - Command: `cornell-rtx.exe --headless --frames N [--width W --height H --scale s --spp n --samples cap --preset 0..23 --settings f.json --rr|--no-rr|--require-rr --raw --debug] --capture out.png|out.exr`.
  - Success prints `RENDER_OK frames= samples= gpu_ms= rr= size= gpu_ms_mean= frame_ms_mean=`. The means are taken from frame 10 on, so use ≥ 200 frames with `--no-vsync` for benchmarks.
  - `--preset`/`--settings` (only one) is applied before all other switches, so order doesn't matter.
    - **Old builds (including `main` before 2026-10-29's fixes) applied them in order**, so `--raw` and `--spp` given before `--preset` were silently reset. Put `--preset` first when benchmarking an old exe.
- Other test modes:
  - `--self-test`: 204 scripted frames; implies headless and `--debug`. It fails on any D3D12 warning.
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
   - **Otherwise, if denoise is on:** filter passes 0–6, constants at offset 512·(i+1).
     - 0 reproject: only on a camera move.
     - 1 integrate: when samples were traced, or the integration is stale. It then copies guides 2→16, 5→17 and 19→20 for the next reprojection.
     - 2–6 à-trous.
     - Flags `camera_moved`, `history_valid` and `integrated_valid` in `Gpu` decide which passes run.
3. `display` (constants at offset 6144) writes `output_image` (10, RGBA8) and `linear_output` (18).
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
| 2 | SRV t1 vertices |
| 3 | Table: UAV u0–u26 (heap slots 0–26) |
| 4 | Table: SRV t2, egui atlas (heap slot 32, pixel-visible only) |
| 5 | Root SRV t3 materials |
| 6 | Root SRV t4 emitters |
| 7 | Root SRV t5 emitter lookup |

There is one static linear-clamp sampler s0. The heap holds 64 descriptors.

**Acceleration structures.** There is one BLAS of the whole triangle soup and one TLAS instance with an identity transform. Both use PREFER_FAST_TRACE. Geometry is **non-opaque**, so every candidate runs through the `Proceed()` loop. That loop is how hidden lights and glass shadows are skipped.

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
- Up to 256 objects and 32 extra area lights.
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
- **Sliders:** all go through `crate::slider` (`SliderClamping::Edits`). egui's default clamps the stored value whenever a slider is drawn, which silently rewrote valid loaded values (found by the UI-plan agent). Self-test frames 178–179 guard this.

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
  - The rest is new upload, BLAS and scratch allocations, plus the whole-scene BLAS build. Instancing (core track 4) is the real fix.
- Capture CLI: `--ui-tool`. Self-test 198–202: each tool drawn on object 0 and the main light, then a keyed G X 0.5 Enter (asserts +0.5 and that the transform ended), then undo (asserts the move is gone).
- Not done: snapping to other objects, a numeric vector entry for free moves, pivot options, multi-select, and Alt-drag duplicate.

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
