# Cornell RTX (native): working notes for Claude

Reference for future sessions. Written 2026-10-29 after a full read of the source plus a build and render check.
`README.md` is the user-facing doc.
Paths here are relative to the repo root unless noted.

## Delegation (owner rule, mandatory)

The owner wants routine work on cheaper models, without wasting tokens. Every subagent starts cold and costs roughly 90k+ tokens before it does anything, so spawn only when the work is worth that. This rule is the owner's explicit request for subagents, so it overrides the default of not spawning them.

- **Inline (main session):** any command with short, predictable output: `cargo fmt` (apply it, don't just check), `cargo clippy`, a filtered `cargo test <name>`, one headless render, grep, git. Design, shaders, GPU and engine code, perf analysis and fixes also stay here.
- **`verifier` agent (haiku, `.claude/agents/verifier.md`):** long or noisy runs: release builds, the full `cargo test`, `--self-test`, benchmarks, multi-render sweeps, `tools/*.py` measurements, `tools/verify.ps1`, snapshots. **Batch:** one spawn per checkpoint carrying every step, never a spawn per follow-up. Run in the background while design work continues. To follow up on a finished run, `SendMessage` the same agent (its context is kept) instead of spawning a new one.
- **`reviewer` agent (sonnet, `.claude/agents/reviewer.md`):** every review of a finished change before a commit. Check each finding before acting on it.
- **`Explore` agent (haiku):** only for wide searches whose file dumps you don't need; a couple of greps run inline.

## What it is

- A Windows-only port of the WebGL Cornell box in `webgl/` (`index.html`, `app.js`, `shaders.js`).
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
- Startup defaults: RR on, RR2/F preset, DLAA (scale 1), 4 spp per frame, 6 bounces, sample cap 4096, ACES tonemap, Blackman-Harris 1.5 px pixel filter (native path), 16× texture anisotropy.
- **Personal project, never published:** license terms don't limit assets or SDK use. The ordered plan is `docs/roadmap.md`:
  - Near term: DLSS presets, native HDR, UI phase 1, Blender-style gizmos and scene files (42 scenes) are done (2026-10-30); next are the quick wins (hot reload, debug views, depth of field, light painting, portals, screensaver).
  - Medium term: the core engine track (generators → mesh import → HDRI → instancing → textures → UI), then feature tracks 7–20 (lights, animation, volumes, SDFs, path visualizer, integrators, physics, SER).
  - Future: Vulkan, software fallback / CPU reference, spectral, curved space, frame generation, neural radiance cache, splats, VR.
- **Git:** the repo root is `cornell/` (native app at the root, the original WebGL app in `webgl/`), with a private GitHub remote. `main` is the verified state. Commit and push only when the user asks.
- The native settings JSON is not compatible with the browser's.

### Machine (2026-10-29)

- Hardware and drivers: RTX 4070 Ti SUPER 16 GB, driver 616.64, Windows 11.
- Toolchain: Rust 1.99.1 MSVC, DXC from Windows SDK 10.0.26100, Python 3.13 with numpy and Pillow (for `tools/rr_stability.py`).

## Build, run, verify

```powershell
$env:CARGO_TARGET_DIR="target/validation"   # keep checks away from target/release (the launcher's exe)
cargo fmt --check; cargo test --locked; cargo clippy --locked -- -D warnings; cargo build --locked
```

- `launch.cmd` starts `target/release/cornell-rtx.exe` and only builds it if it is missing. It never rebuilds a stale exe.
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
  - `--self-test`: 285 scripted frames (`SCENES_FROM`), then one frame per scene file (453 with the built-in, generated, mesh, environment and materials packs); implies headless and `--debug`. It fails on any D3D12 warning.
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
| `src/lighting.rs` | `PackedMaterial` (144 B) and `Emitter` (64 B), `materials()` ordering, `analytic()` and `emitter_table()` (suns, spheres, then triangles; power-weighted CDF plus a per-triangle lookup, 1-based, 0 = not an emitter; see "Analytic lights"). |
| `src/display.rs` | `Monitor::query(hwnd)`: the DXGI output holding most of the window, `GetDesc1` (HDR on, peak/full-frame/min nits) and the SDR white level via `DisplayConfigGetDeviceInfo`; `current()` = `IDXGIFactory1::IsCurrent`. Also `hdr_map` (CPU copy of the shader transform, for tests) and `write_hdr_png` (16-bit PQ/BT.2020 with a cICP chunk). |
| `src/ui.rs` | UI plan phase 1 (2026-10-30). `UiState` (tab, advanced, panel_hidden, selection, toasts, export size/samples): never in `Settings`. `impl App { fn ui }` draws top bar, status bar, right `SidePanel` with Scene / Render / Look / Export tabs, toasts. Helpers `row` (2-column grid row, label carries the tooltip), `more` (inline when advanced, else a "More" header), `theme`. `UiState::delete_selected`, `toast`, `error`. |
| `src/gizmo.rs` | Roadmap 3b (2026-10-30). `View` (camera in physical pixels: project, ray, world-per-pixel), `frame()` (pivot + rotation of the selection; lights are `flat`), `Transform` (mode, constraint, typed value, snap, original settings; `apply` restores the target then transforms; `restore` cancels), `layout`/`hit`/`draw`, `draw_lights`, `pick` (objects via `pick_object_hit` plus light quads). Unit tests cover typed values, arrow drags, screen-angle rotation, Blender's axis toggling and light scaling. |
| `src/scenes.rs` | Roadmap 4 (2026-10-30). `folder()` (`./scenes`, next to the exe, or `exe/../../scenes`), `list()` (packs by `pack.json` order; builtin first, user last), `read()` (wrapper `{format, title, description, tags, credits, settings}` or bare `Settings`; validated; title becomes `scene_name`), `find()` (path, stem or title), `load_into()` (uses `Settings::keep_output` so render choices survive), `save_user()`. Test `builtin_scenes_are_valid`. |
| `src/hotreload.rs` | Roadmap 5a. `HotReload::new()` (debug builds, not the self-test) watches `CORNELL_SHADERS` (build.rs bakes the source path and the DXC path `CORNELL_DXC`); `poll()` every 0.5 s on the newest mtime; `compile()` runs all 5 entries of `ENTRIES` (keep in sync with build.rs) with build.rs's flags. `Gpu::reload_shaders` builds all PSOs before swapping, so an error keeps the old ones; prints `SHADERS RELOADED` / `SHADER ERROR`. Tested by editing post.hlsl under a running instance (valid, broken, restored). |
| `src/look.rs` | Roadmap 5c: `read_cube` (.cube 3D LUT) and `color_wheel` (lift/gamma/gain widget: chroma from a dragged dot in an orthonormal RGB chroma plane, level from a slider, double-click resets). |
| `src/walk.rs`, `video.rs`, `saver.rs`, `remote.rs` | Roadmap 5d toys: walk mode, video export (see "Video export"), screensaver mode, phone remote (see "Toys"). |
| `src/sky.rs` | Track 9 physical sky: `PhysicalSky` (`Settings::physical_sky`), solar position, atmosphere bake, the sun light, `environment_key` / `upload_environment`. See "Physical sky". |
| `src/anim.rs`, `timeline_ui.rs` | Track 8 keyframes and the timeline strip; see "Animation and timeline". |
| `src/video_ui.rs` | The video export dialog and the progress window shown during an export. |
| `src/prefs.rs` | UI preferences in %APPDATA%\CornellRTX\prefs.json: favourite and recent scenes, UI size, advanced toggle, first-run hint. |
| `src/editor.rs` | Inspectors used by the Scene tab: `object_ui`, `light_ui`, `material_ui`, `shape_combo`, `new_object`. Ranges match `validate()`. |
| `src/mesh.rs`, `procedural.rs`, `environment.rs`, `texture.rs` | Core tracks 2, 1, 3 and 5: mesh import (parts, UVs, texture maps), `gen:` procedural meshes, HDRI maps and their CDFs, texture sets. See their sections below. |
| `src/assets.rs`, `browser.rs` | Item 6 asset browser: online catalogs, pinned downloads, the Assets window. See "Asset browser". |
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
| `lobes` | metallic, reflectivity, transmission, flags. **bit0** = unlit (the path terminates on hit); **bit1** = hidden from camera rays (bounce 0 and the guide ray); **bit2** = portal; **bit3** = masked texture map (alpha test in the any-hit loops); **bit4** = has texture maps; **bit5** = spot light; **bits 8–10** = nested-dielectric priority (0–7). Bits 3 and 4 are set in `Gpu::attach_maps`. |
| `optical` | IOR, emission, pattern id, one-sided emitter (1 for lights) |
| `secondary` | Pattern RGB + pattern scale. Patterns are evaluated in **world space**. |
| `layers`, `sheen` | Track 11: coat, coat roughness, sheen roughness, −; sheen colour × strength, −. |
| `maps` (uint4) | Heap slot + 1 of the base colour, normal and metallic-roughness maps (0 = none), alpha cutoff bits (0 = opaque). Filled by `Gpu::attach_maps`. |
| `base_scale` | Base colour value / map average (rgb); w = texture-set repeats per unit (box mapping; 0 = the mesh's own UVs), set by `lighting::pack`. |
| `surface_scale` | Roughness and metallic value / map average (x, y; 0 = leave the value unscaled). |

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
| 10 | Table: SRV t0.. space 1, texture maps (`maps[]`, heap slots 512–4607, `MAX_MAPS` 4096) |

Static samplers: s0 linear clamp (GUI, pixel-visible), s1 linear wrap with mips (texture maps). The heap holds 512 + 4096 descriptors: UAVs 0–31, egui textures 32–511, texture maps from 512.

**Vertex** (40 B): position, material, normal, pad, uv (meshes only; zero for shapes and static geometry). The BLAS vertex stride follows `size_of::<Vertex>()`.

**Acceleration structures** (instanced since core track 4, see "Instancing"): a static BLAS (floor, room, lights; world space; two geometries, hidden lights non-opaque first, the rest opaque) as InstanceID 0, plus one BLAS per distinct object mesh in object space, instanced once per visible object with its transform. Glass objects are FORCE_NON_OPAQUE instances (their candidates run the `Proceed()` loop, so shadow rays can pass them), all other objects FORCE_OPAQUE. All use PREFER_FAST_TRACE.

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
| 31 | env_cdf: conditional CDF per row, then the marginal CDF in an extra row | R32F, **map width × (height + 1)** |

## Rendering behaviour worth remembering

- **Guide ray** (per pixel, before the spp loop): writes the RR guides.
  - Jittered with Halton(2,3) over 64 phases when RR is on; unjittered otherwise.
  - Specular hit distance comes from a deterministic mirror-reflection ray, not the sampled lobe. This was the RR stability fix.
- **Transport:**
  - NEE picks an emissive triangle by power. MIS uses the balance heuristic on both the NEE and the BSDF-hit side.
  - The BSDF is Lambert + GGX (Smith G1·G1 form) as a mixture chosen by `spec_probability`, under an optional sheen and clear coat (track 11, see "Layered materials").
  - Dielectrics: stochastic choice by the exact Fresnel term (TIR included). Rough glass is an approximation (refracted direction lerped toward a cosine sample).
  - Nested dielectrics (track 10): a per-path stack of up to 4 media (one 64-bit word) ranked by `Material::priority`; surfaces that don't bound the current medium are false hits (skipped, no bounce). See "Nested dielectrics".
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
  - High-resolution export and video export render at their own output size without touching the swapchain, as a `video::Job` (see "Video export" and "UI phases 3–5").

## Feature notes

Per-feature design notes (upscalers, HDR, UI phases, lights, gizmos, scene files, generators, mesh import, video export, asset browser, textures, animation, physical sky, instancing, HDRIs, toys, camera and look, debug views, tooling, the 2026-10-29 review) live in **`docs/engine-notes.md`**. Before changing an area, read its section there (`grep -n '^## ' docs/engine-notes.md`). Where this file says "see X", X is a section there. Write each new feature's notes there, and keep this file to what every session needs.

## Measurement conventions (carried over from the WebGL work)

- Error metric: RMSE ×1000 on compressed luminance l/(1+l) against a high-spp reference. The WebGL reference was 16,384 spp.
- Also track a 5×5-blurred error, which catches ghosting that plain RMSE hides.
- Compare against an **independent** reference: a white-noise reference when testing Sobol.
- Use numpy on EXR captures. imageio has no EXR backend here, and OpenEXR and cv2 are not installed.
  - The exe writes **tiled (64×64), RLE-compressed** RGB float EXRs.
  - Use `tools/exr.py`: `exr.read(path)` returns an H×W×3 float32 array. It handles scanline or tiled files with NONE or RLE compression.
  - It was used to confirm that direct + indirect recompose the beauty to 2e-7 relative error (presets 0, 3, 8, 21; raw native path).
- Report in a table, with the sample count where each method reaches equal error.
