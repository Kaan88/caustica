# Architecture: source map, frame pipeline, rendering

Moved out of `CLAUDE.md` on 2026-11-02 (unchanged text). Paths are relative to the repo root. Where this file says "see X", X is a section of `docs/engine-notes.md`.

## Source map

| File | Key places |
|---|---|
| `src/main.rs` | `App` state. `App::new` is the CLI parser (`--set key=json`, `--ui-*`). `redraw()` runs each frame: self-test script, HDR/monitor sync, egui (`ui()` lives in `ui.rs`), save/load, dirty keys, render, self-test asserts, actions, export. `window_event()` handles input and the single-key toggles (each one toasts). |
| `src/scene.rs` | `Settings`, `Material`, `Object`, `AreaLight`, `Shape` (11) and `Pattern` (4). `PRESETS` holds 24 names. `randomize()` (l.318). `preset()` (l.352) builds 0–8 inline and sends 9+ to `presets::apply`; it keeps the camera only when moving between Cornell presets 0–8. `validate()` (l.456) bounds every field. `camera()` returns `look_at_rh` and `perspective_rh` with near 0.01 and far 500. `light_color()` converts Kelvin to linear RGB at unit luminance, times tint and intensity. |
| `src/presets.rs` | 12-entry material library (`material(i)`), `studio()` base stage, `apply()` for presets 9–23, `description()`. |
| `src/geometry.rs` | Unit meshes cached in a `LazyLock`. `object_geometry()` applies scale (size·0.35), YXZ Euler rotation and floor grounding (floor at y = −1). `geometry()` builds one triangle soup. Also frame, focus and CPU pick (Möller–Trumbore over every object, every click). |
| `src/lighting.rs` | `PackedMaterial` (176 B) and `Emitter` (64 B), `materials()` ordering, `analytic()` and `emitter_table()` (suns, spheres, then triangles; power-weighted CDF plus a per-triangle lookup, 1-based, 0 = not an emitter; see "Analytic lights"). |
| `src/display.rs` | `Monitor::query(hwnd)`: the DXGI output holding most of the window, `GetDesc1` (HDR on, peak/full-frame/min nits) and the SDR white level via `DisplayConfigGetDeviceInfo`; `current()` = `IDXGIFactory1::IsCurrent`. Also `hdr_map` (CPU copy of the shader transform, for tests) and `write_hdr_png` (16-bit PQ/BT.2020 with a cICP chunk). |
| `src/ui.rs` | UI plan phase 1 (2026-10-30). `UiState` (tab, advanced, panel_hidden, selection, toasts, export size/samples): never in `Settings`. `impl App { fn ui }` draws top bar, status bar, right `SidePanel` with Scene / Render / Look / Export tabs, toasts. Helpers `row` (2-column grid row, label carries the tooltip), `more` (inline when advanced, else a "More" header), `theme`. `UiState::delete_selected`, `toast`, `error`. |
| `src/gizmo.rs` | Roadmap 3b (2026-10-30). `View` (camera in physical pixels: project, ray, world-per-pixel), `frame()` (pivot + rotation of the selection; lights are `flat`), `Transform` (mode, constraint, typed value, snap, original settings; `apply` restores the target then transforms; `restore` cancels), `layout`/`hit`/`draw`, `draw_lights`, `pick` (objects via `pick_object_hit` plus light quads). Unit tests cover typed values, arrow drags, screen-angle rotation, Blender's axis toggling and light scaling. |
| `src/scenes.rs` | Roadmap 4 (2026-10-30). `folder()` (`./scenes`, next to the exe, or `exe/../../scenes`), `list()` (packs by `pack.json` order; builtin first, user last), `read()` (wrapper `{format, title, description, tags, credits, settings}` or bare `Settings`; validated; title becomes `scene_name`), `find()` (path, stem or title), `load_into()` (uses `Settings::keep_output` so render choices survive), `save_user()`. Test `builtin_scenes_are_valid`. |
| `src/hotreload.rs` | Roadmap 5a. `HotReload::new()` (debug builds, not the self-test) watches `CORNELL_SHADERS` (build.rs bakes the source path and the DXC path `CORNELL_DXC`); `poll()` every 0.5 s on the newest mtime; `compile()` runs all 17 entries of `ENTRIES` (keep in sync with build.rs) with build.rs's flags. `Gpu::reload_shaders` builds all PSOs before swapping, so an error keeps the old ones; prints `SHADERS RELOADED` / `SHADER ERROR`. Tested by editing post.hlsl under a running instance (valid, broken, restored). |
| `src/paths.rs` | Track 14 path visualizer: `Record` (the shader's `PathVertex`), `parse`, `draw`, `window`, `PathView` (in `UiState`). See engine-notes "Path visualizer". |
| `src/lpe.rs` | Track 14 light path expressions: parser, NFA → minimal DFA (`compile`), the GPU `table`, `PRESETS`. See engine-notes "Light path expressions". |
| BDPT, photon mapping, battle mode | Track 15 part 2: `shaders/integrators.hlsl`, `Gpu::integrate` / `battle_frame` / `prepare_aux`, `Settings::integrator` / `photons` / `photon_radius` / `battle`, the `region` constant. See engine-notes "BDPT and photon mapping". |
| ReSTIR DI | Track 15: `restir_presample` / `restir_initial` / `restir_spatial` in pathtrace.hlsl (before `trace`), `lighting::alias_records`, `Gpu::prepare_restir`, `Settings::restir*`. See engine-notes "ReSTIR DI". |
| `src/lens.rs` | Track 16 real lenses: `LENSES` (prescriptions), `INFO` (focal length, wide-open f-number), `pack` (focus, film size, exit-pupil bands for the shader). Glare is in post.hlsl `glare_pass` + `Gpu::run_glare`. See engine-notes "Real lenses and glare". |
| `src/physics.rs` | Track 17 physics sandbox: `Sim` (rapier3d bodies from `Settings::objects`, written back each step), grab / explode tools and `App::step_physics`. See engine-notes "Physics sandbox". |
| `src/audio.rs` | Track 19 audio-reactive lights: loopback capture (`Audio`), FFT bands (`bands`), `apply` scales the frame copy's lights. See engine-notes "Audio-reactive lights". |
| `src/script.rs` | Track 19 Rhai scripting: `--script` runs on its own thread; each call is a `Request` the app answers between frames (`App::poll_script`). `set` applies a JSON-pointer edit through `validate()`. See engine-notes "Scripting". |
| `src/laser.rs` | Track 17 laser puzzle: CPU beam trace (`trace`), `Laser` state (segments, solved, moves) and its overlay. See engine-notes "Laser puzzle". |
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

## Rendering behaviour worth remembering

- **Guide ray** (per pixel, before the spp loop): writes the RR guides.
  - Jittered with Halton(2,3) over 64 phases when RR is on; unjittered otherwise.
  - Specular hit distance comes from a deterministic mirror-reflection ray, not the sampled lobe. This was the RR stability fix.
- **Transport:**
  - NEE picks an emissive triangle by power. MIS uses the balance heuristic on both the NEE and the BSDF-hit side. With ReSTIR DI on, the first hit's light sample comes from the reservoirs instead (weight 1, BSDF hits on table emitters from that vertex count 0).
  - The BSDF is Lambert + GGX (Smith G1·G1 form) as a mixture chosen by `spec_probability`, under an optional sheen and clear coat (track 11, see "Layered materials"); the GGX lobes are energy-compensated from tables in `layers.hlsli` and the base can be anisotropic (see "Energy compensation and anisotropy").
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
