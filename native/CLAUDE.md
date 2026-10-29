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
- The DLSS SDK is 310.9.16, pinned to commit `9981c63…`. `tools/setup.ps1` sparse-clones it into `vendor/dlss`.
- Startup defaults: RR on, RR2/F preset, DLAA (scale 1), 4 spp per frame, 6 bounces, sample cap 4096, ACES tonemap.
- **Git:** the repo root is `cornell/` (WebGL app plus `native/`), with a private GitHub remote. Commit and push only when the user asks.
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
  - Asserts that `vendor/dlss` exists.
  - Compiles 5 shader entries: `trace`, `filter` and `display` as cs_6_5; `vs_main` and `ps_main` as 6_0.
  - Compiles the bridge and links `nvsdk_ngx_d`.
  - **Copies `nvngx_dlss.dll` and `nvngx_dlssd.dll` next to the exe.** A running instance locks them and the build fails. Use a separate `CARGO_TARGET_DIR`, and never kill the user's running app.
- Headless runs need a real GPU and use a hidden window:
  - Command: `cornell-rtx.exe --headless --frames N [--width W --height H --scale s --spp n --samples cap --preset 0..23 --settings f.json --rr|--no-rr|--require-rr --raw --debug] --capture out.png|out.exr`.
  - Success prints `RENDER_OK frames= samples= gpu_ms= rr= size=`.
  - Argument order matters: `--preset` re-initializes settings, and `--settings` replaces them.
- Other test modes:
  - `--self-test`: 180 scripted frames; implies headless and `--debug`. It fails on any D3D12 warning.
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
| `src/main.rs` | `App` state. `App::new` is the CLI parser, including `--set key=json` (l.191). `redraw()` (l.283) runs each frame: self-test script, egui, save/load, dirty keys, render, self-test asserts, actions, export. `ui()` (l.644) is the main panel; it syncs `self.preset` from `scene_name`. `window_event()` (l.771) handles input. |
| `src/scene.rs` | `Settings`, `Material`, `Object`, `AreaLight`, `Shape` (11) and `Pattern` (4). `PRESETS` holds 24 names. `randomize()` (l.318). `preset()` (l.352) builds 0–8 inline and sends 9+ to `presets::apply`; it keeps the camera only when moving between Cornell presets 0–8. `validate()` (l.456) bounds every field. `camera()` returns `look_at_rh` and `perspective_rh` with near 0.01 and far 500. `light_color()` converts Kelvin to linear RGB at unit luminance, times tint and intensity. |
| `src/presets.rs` | 12-entry material library (`material(i)`), `studio()` base stage, `apply()` for presets 9–23, `description()`. |
| `src/geometry.rs` | Unit meshes cached in a `LazyLock`. `object_geometry()` applies scale (size·0.35), YXZ Euler rotation and floor grounding (floor at y = −1). `geometry()` builds one triangle soup. Also frame, focus and CPU pick (Möller–Trumbore over every object, every click). |
| `src/lighting.rs` | `PackedMaterial` and `Emitter` (64 B each), `materials()` ordering, `emitter_table()` (power-weighted CDF plus a per-triangle lookup, 1-based, 0 = not an emitter). |
| `src/editor.rs` | Object window (list, add, duplicate, delete, focus, transform, material), area-light editor, `material_ui`. |
| `src/gpu.rs` | Everything D3D12. Highlights: `new()`, `resize()` (l.496, allocates the 19 UAV textures), `rebuild_scene()` (l.611), `update_materials()`, `render()` (l.749), `capture()`, `capture_reference()`, egui atlas and draw. |
| `src/teapot_data.rs` | Generated by `tools/build_teapot_data.py` from `assets/TeapotGeometry.source.js`. Do not hand-edit. |
| `shaders/common.hlsli` | `cbuffer Frame`, which **must mirror `Constants` in gpu.rs field for field**. Also the resource bindings. |
| `shaders/pathtrace.hlsl` | `trace` (l.56): guide/AOV ray, then the spp loop. |
| `shaders/post.hlsl` | `filter` (5 à-trous passes plus temporal) and `display` (views, tonemap, A/B). |
| `shaders/gui.hlsl` | Declares a partial cbuffer (4 matrices, eye, then `dimensions`) that relies on that offset in `Constants`. |
| `bridge/ngx.cpp` | Init, create and eval for NGX DLSSD; C ABI. Contains a development project GUID that must be replaced before distribution. |

## Frame pipeline (`Gpu::render`)

1. Upload `Constants` to offset 0, then `trace` dispatch (8×8 groups, render resolution).
2. Then one of:
   - **If RR is active:** transition inputs 0, 2–7 to SRV, call `NGX_D3D12_EVALUATE_DLSSD_EXT` into texture 14, transition back.
   - **Otherwise, if denoise is on:** 5 `filter` passes, constants at offset 1024·(i+1). Then copy ping→history (8→15), normal (2→16) and depth (5→17), only when samples were traced.
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
| 3 | Table: UAV u0–u18 (heap slots 0–18) |
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
| 15 | history color | |
| 16 | history normal | |
| 17 | history depth | R32F |
| 18 | linear_output (HDR beauty, used for EXR) | **output resolution** |

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
  - The sequence index is `samples + sample_id`, with a per-pixel scramble fixed for the life of the app.
- **Native filter** (`post.hlsl`):
  - B3 5×5 à-trous over 5 passes, with weights from normal (pow 64), depth and luminance (σ from M2). Albedo demodulation uses diffuse + specular albedo.
  - Temporal blend in pass 0 only:
    - Nearest-pixel reprojection through `motion`.
    - Accepts a history pixel when normal dot > 0.95 and depth is within max(2.5 cm, 3%).
    - Clamps history to ±3σ.
    - History weight runs from 0.85 (rough) to 0.1 (glossy) and fades out by 256 spp.
  - At render scale < 1 without RR, display upscales by nearest neighbour.
- **RR (`bridge/ngx.cpp`):**
  - Settings: DLUnified denoise mode, packed roughness, linear depth, IsHDR and MVLowRes flags, MV scale 1.
  - **NGX jitter offset = −ray jitter on both axes. Do not change it without rerunning `tools/rr_stability.py`.** Measured result: 94% less edge shimmer (`captures/rr-stability/regression/metrics.json`).
  - Quality mode is derived from the ratio of render width to output width.
  - The preset hint is set for every quality mode. Preset ids: 0 default, 4 D, 5 E, 6 F.
- **Exports:**
  - PNG reads texture 10 (no UI) or the last presented backbuffer (`--capture-ui`).
  - EXR reads texture 18.
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

**Disk use.** `target/` is about 3.5 GB and `vendor/` about 0.3 GB. The root `.gitignore` already lists `native/target`, `native/vendor` and `native/captures`.

## Open review findings

These come from reading the code; check each before fixing it. They are listed in priority order.

1. **Light ghosting during camera motion (likely, not reproduced).**
   - The native temporal check uses only normal and depth (`post.hlsl:15`).
   - The WebGL version had the same flaw: the light quad is 3.5 cm below the ceiling and faces the same way, so its glow ghosted onto the ceiling while moving.
   - The WebGL fix stored the first-hit object id and rejected history on a mismatch.
2. **Fixed per-pixel Sobol pattern (likely).** After each camera move, the sequence restarts at index 0 with the same per-pixel scramble, so the noise pattern stays fixed on screen while the image moves. The WebGL version offset the sequence base on every move (`seqBase`).
3. **Pan uses yaw only** (`main.rs:823`), so panning at steep pitch is not screen-aligned. The WebGL version was fixed to pan along screen axes.
4. **Missing glyphs.** The egui default font lacks "Ⅱ" and "→": the Pause button and the "rw × rh → w × h" label show boxes.
5. **Delete key** deletes the selected object even when the object window is closed.
6. **Windows stay off-screen after a resize.** egui windows keep their position when the window shrinks, so the Scene objects window can end up partly off-screen. This is visible in the self-test capture after the 800×600 resize.
7. **Performance, cheap wins:**
   - Shadow rays use closest-hit search over non-opaque geometry. They could accept the first hit when there is no glass and no hidden-light filtering.
   - egui uploads a new committed buffer per mesh per frame.
   - All three dirty keys are reserialized every frame, and `material_key` is computed twice.
8. **Undefined swapchain read.** `--capture-ui` reads a FLIP_DISCARD backbuffer after Present. Its contents are formally undefined, but it works on this driver.
9. **Courtyard marble (19).** The world-space marble pattern reads as large black blotches on the columns. Tune `pattern_scale` or the marble function if anyone cares.

## Measurement conventions (carried over from the WebGL work)

- Error metric: RMSE ×1000 on compressed luminance l/(1+l) against a high-spp reference. The WebGL reference was 16,384 spp.
- Also track a 5×5-blurred error, which catches ghosting that plain RMSE hides.
- Compare against an **independent** reference: a white-noise reference when testing Sobol.
- Use numpy on EXR captures. imageio has no EXR backend here, and OpenEXR and cv2 are not installed.
  - The exe writes **tiled (64×64), RLE-compressed** RGB float EXRs.
  - Use `tools/exr.py`: `exr.read(path)` returns an H×W×3 float32 array. It handles scanline or tiled files with NONE or RLE compression.
  - It was used to confirm that direct + indirect recompose the beauty to 2e-7 relative error (presets 0, 3, 8, 21; raw native path).
- Report in a table, with the sample count where each method reaches equal error.
