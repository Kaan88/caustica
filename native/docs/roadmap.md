# Cornell RTX roadmap

The single ordered plan. Details live in the two research documents:
[ui-plan.md](ui-plan.md) (UI redesign) and [scene-research.md](scene-research.md) (scenes).
Written 2026-10-30, and extended the same day with the feature brainstorm: quick wins (near-term item 5), feature tracks 7–20 (medium term) and the big ideas in future goals. Items 1–4 and 3b are done (2026-10-30); the rest is not started.

**Shape of the plan:**
- **Near term:** polish and plumbing, then cheap fun.
- **Medium term:** the core engine track first, then feature tracks that build on it.
- **Future goals:** large or research-grade items.

Each item lists its prerequisites by number.

## Ground rules from the owner

- **Personal project, never published.** License terms don't constrain anything: use any model, scene, HDRI or texture, including research-only, NC, ND and SA assets, Sponza, the Stanford scans and Disney data. Assets may be downloaded into the project, or into a local `assets/` folder, as convenient. The licensing cautions in scene-research.md are kept only as background.
- Credits stay in `assets/CREDITS.md` out of courtesy, not obligation.

## Near term

### 1. DLSS preset selection (small) — done 2026-10-30

Let the user choose the DLSS model presets the SDK offers, for both DLSS features.

- **Super Resolution** (DLSS 310.9.16 presets, from `nvsdk_ngx_defs.h`):

  | Preset | NVIDIA's description |
  |---|---|
  | Default | Lets NVIDIA pick per quality mode: K for DLAA, Quality and Balanced; M for Performance; L for Ultra Performance |
  | J | Transformer. Slightly less ghosting than K, more flicker |
  | K | Transformer. Best quality, higher cost |
  | L | Ultra Performance default |
  | M | Performance default |

  A–D were removed from the SDK; E and F are deprecated, and G, H, I, N and O revert to default. Offer only Default, J, K, L and M.
- **Ray Reconstruction:** keep the current Default, D, E and F list, and check the SDK headers for newer RR presets when updating the SDK.
- **Implementation:**
  - New `Settings.dlss_preset: u32` (0, 10–13).
  - In `bridge/ngx.cpp` `cornell_ngx_create_sr`, set `NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_{DLAA,Quality,Balanced,Performance,UltraPerformance,UltraQuality}` on `sr_params` before creating the feature, the same way RR sets its hints.
  - Add it to the `main.rs` reconfigure trigger list, and add a combo next to the Upscaler choice, shown when DLSS SR is selected.
  - Self-test: cycle the presets once.

### 2. Native HDR output, with settings taken from Windows — done 2026-10-30

As built: scRGB swapchain, BT.2390 roll-off on the max channel, Auto/SDR/forced modes, Windows peak and SDR white with manual overrides, egui at paper white, and HDR PNG (PQ / BT.2020, cICP). Not done: the optional HDR10 swapchain, and JPEG XR. Details are in CLAUDE.md, "Native HDR".

Goal: on an HDR monitor with Windows HDR on, show real HDR, using Windows' own calibration and SDR-brightness setting. With HDR off, keep today's SDR path.

- **Detecting the display:**
  - Get the `IDXGIOutput6` the window is mostly on (`IDXGISwapChain::GetContainingOutput`).
  - `GetDesc1()` gives `ColorSpace` (`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020` means HDR is on), `MinLuminance`, `MaxLuminance`, `MaxFullFrameLuminance` and the primaries. These come from Windows HDR Calibration when the user has run it.
  - SDR content brightness (the Windows slider) comes from `DisplayConfigGetDeviceInfo` with `DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL`. The value is in units of 80 nits / 1000.
  - Re-query on `WM_DISPLAYCHANGE`, on window moves between monitors (winit `Moved` / `ScaleFactorChanged`), and on DXGI factory invalidation (`IDXGIFactory1::IsCurrent() == false`).
- **Swapchain:** `R16G16B16A16_FLOAT` with `SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709)` (scRGB, where 1.0 = 80 nits). This is simpler than HDR10/PQ, and Windows does the final conversion. Optionally offer HDR10 (`R10G10B10A2` + G2084/P2020) for comparison.
- **Display transform:**
  - Replace the SDR tone map in HDR mode with: scene-linear × exposure → a display-referred curve that leaves highlights up to the panel's `MaxLuminance` untouched → soft roll-off near the peak (BT.2390 EETF or similar). Write scRGB with 1.0 = 80 nits, so paper white = SDR white level.
  - Keep the existing tone maps as "SDR look" options.
  - The output texture (10) becomes RGBA16F in HDR mode. The copy to the back buffer still works because the formats match.
- **UI:** egui draws in sRGB. In HDR mode scale its vertex colors to the SDR white level (linearize sRGB, multiply by white/80), so the panel matches Windows' SDR brightness instead of glowing.
- **Settings:**
  - Display mode: Auto (follow Windows) / SDR.
  - Peak brightness and paper white: "from Windows" by default, with a manual override.
  - Show the detected values in the panel ("HDR: 1015 nits peak, SDR white 240 nits").
- **Captures:**
  - EXR stays scene-linear.
  - Add an HDR screenshot format: 16-bit PNG with a PQ/BT.2020 `cICP` chunk, or JPEG XR as Windows' Snipping Tool does. An 8-bit PNG from an HDR frame needs its own SDR tone map pass.
- **Upscalers:** unaffected. They run on linear HDR before the display transform.
- **Testing:**
  - Windows can turn HDR on without an HDR monitor on some setups, and the WARP adapter can't present HDR.
  - Make the transform testable headless by rendering to an RGBA16F target and checking known values: an 80-nit gray maps to 1.0, and the peak is clamped to `MaxLuminance`/80.

### 3. UI phase 1 (from ui-plan.md section 8) — done 2026-10-30

Regroup and relabel only. Include the DLSS preset combo and the HDR status line from items 1–2.

### 3b. Blender-style transform gizmos (owner request, 2026-10-30) — done 2026-10-30

As built: move / rotate / scale gizmos for objects, area lights and the main light (no main-light rotation, since it is always horizontal), G / R / S modal transforms with X / Y / Z, Shift+axis planes, typed values, Ctrl snap, Shift fine and cancel, Global / Local orientation, light outlines and light picking, undo / redo. Drag cost is in CLAUDE.md, "Gizmos".

**Not done yet (owner asked to keep this list, 2026-10-30):**
- **Snapping to other objects:** surface, vertex, edge and face snapping, as Blender's snap menu offers. Today Ctrl only snaps to a 0.1-unit grid, 15° and 0.1×.
- **Pivot options:** median point, 3D cursor, individual origins and active element. Today the pivot is always the object's own origin.
- **Multi-select:** Shift-click to add, box select, select all (A), and transforming several items together.
- **Alt-drag duplicate:** Blender's Shift+D duplicate-then-move as well.
- **Smaller gaps:** free moves show one vector line in the header, with no per-axis typed vector entry.
- **Cheap drags:** about 20 ms per frame on the bunny and chess scenes needs instancing (core track 4).

Clicking an object (in the viewport or the outliner) shows transform handles on it, as in Blender. This replaces the plain drag handles of ui-plan.md phase 2 and lands together with that phase's undo stack.

- **Gizmos**, drawn at the object's pivot with a constant screen size:
  - **Move:** X / Y / Z arrows (red, green, blue) and the three plane squares (XY, XZ, YZ) for two-axis moves; a center circle moves in the view plane.
  - **Rotate:** three axis rings plus a white view-aligned outer ring (trackball-free, like Blender's default).
  - **Scale:** axis lines ending in cubes, plus the center circle for uniform scale.
  - Hovered handles highlight; the active axis draws as an infinite line while dragging.
- **Blender keys** as well as the handles:
  - G (grab/move), R (rotate), S (scale) start a modal transform from the mouse position.
  - X / Y / Z then constrain to an axis, Shift+X/Y/Z to the plane excluding it, pressing the axis again toggles global ↔ local.
  - Typing a number sets the exact value (G X 0.5 Enter), Ctrl snaps (grid 0.1 units, 15°, 0.1×), Shift gives fine control.
  - Left click or Enter confirms; right click or Esc cancels and restores the original transform.
  - A header shows the live values ("Move X 0.52 units"), like Blender's header text.
  - W / E / R-style tool switching is also offered as top-bar buttons: Select, Move, Rotate, Scale. Orientation: Global / Local.
- **Also for lights:** area lights and the main light get move and rotate gizmos and a scale gizmo that edits their width / depth (light quads become drawn and pickable).
- **Mapping to `Settings`:** move edits `position[x, z]` and `elevation` (y); rotate edits `rotation` (yaw) and `tilt` (pitch, roll); scale edits `size` (uniform), `scale_xz` (X, Z) and `stretch` (Y). Values clamp to `validate()` ranges. Grounded objects keep resting on the floor unless Y is moved explicitly.
- **Implementation:**
  - Draw with the egui painter on a background layer, projecting through `Settings::camera` as the self-test pick check does. Hit-test handles in screen space before orbit/pick in `window_event`.
  - Axis drags: project the axis onto the screen and use the mouse delta along it; plane drags: intersect the mouse ray with the plane; rotation: the angle around the projected pivot.
  - Every drag frame changes `geometry_key` and rebuilds the BLAS. Measure on chess and bunny; if it hitches, update every other frame or move objects by TLAS instance transforms (core track 4).
  - Undo: push one snapshot per completed transform (ui-plan.md 7.7).
  - Self-test: scripted G/R/S transforms with axis constraints and typed values, asserting the resulting fields, plus a cancel that restores them.

### 4. Scenes step 1 (from scene-research.md section 5) — done 2026-10-30

- Add a `scenes/` folder of JSON scene files and a Scenes window.
- Write the 42 buildable presets as files.

As built: `scenes/builtin` (42 scenes, generated by `tools/build_scenes.py`), a Scenes window with search and tags, `--scene`, saving to `scenes/user`, `tools/scene_sheet.py` contact sheets, and every scene loaded by the self-test. Thumbnails (rendered in the background, cached by file hash) and pack licence labels were added the same day.

Because licensing doesn't matter, the import steps that follow can start with the best-known scenes, not the CC0-safe ones: Sponza, San Miguel, Bistro, Bitterli's whole set (including the ND/NC ones), the Stanford dragon, Buddha and Lucy, and Disney's Moana island if memory allows.

### 5. Quick wins (small, independent, any order)

Each is an evening to two days and touches little of the architecture. They come after item 3 so their controls land in the new UI groups, and the scene entries after item 4 so they ship as files. Start with 5a: hot reload makes all later shader work faster. Every item adds self-test frames on unused frame numbers.

#### 5a. Developer tooling

- **Shader hot reload** (debug builds only):
  - Watch `shaders/` with the `notify` crate. On save, compile with `dxcompiler.dll` at runtime (`DxcCreateInstance`, via `hassle-rs` or a thin FFI) using the same entries and profiles as `build.rs`, then rebuild the affected PSOs.
  - On a compile error, keep the old PSO and show the DXC message in a toast.
  - Release keeps the embedded DXIL.
- **Perf history:** a script runs a fixed benchmark set (`--headless --no-vsync --frames 200` over presets 0, 13, 17 and 21, native and RR). It appends the `RENDER_OK` means with the commit hash to `captures/perf-history.csv` and plots the history, so slowdowns show up per commit.

#### 5b. Seeing the invisible: debug views and meters

Each is a new `modes.x` view in `display` or a small compute pass:

- **Heat maps:**
  - Bounce count, written from `trace` into a spare channel or a new R32F UAV.
  - **BVH cost:** count `Proceed()` iterations per pixel. This shows the cost of all-non-opaque geometry and measures the opaque split later.
  - Per-pixel variance from the Welford M2 in `accumulation.a`.
- **Cinematographer's scopes:** histogram, luma waveform, vectorscope, false-color exposure and zebras.
  - A compute pass reads `linear_output` (18), bins into a small UAV and reads back once per frame. egui draws the plots.
- **Light meter:** hover to read illuminance (lux) and luminance (nits) at the cursor from a one-texel readback, with the same scale as the HDR work in item 2 (1.0 = 80 nits).
- **Live convergence graph:** load a reference EXR (`tools/exr.py` conventions) and plot RMSE ×1000 on compressed luminance l/(1+l) against samples in egui. This turns `motion_error.py` into a live panel.
- **Lifetime stats:**
  - Persist totals in a small JSON next to the settings: rays traced, GPU hours, renders saved and the highest sample count reached.
  - Add a few achievements, such as the first caustic, the first 65k-spp render or every preset visited.

#### 5c. Camera and look

- **Thin-lens depth of field** (the native camera is a pinhole today):
  - Aperture and focus-distance settings.
  - **Aperture shapes:** round, 5–9 blades, or a custom polygon or star, sampled by triangle fan. Optical vignetting gives cat-eye bokeh near the frame edges.
  - **Click-to-focus:** read linear depth (u5) under the cursor and animate a focus pull.
  - **RR:** the guide ray goes through the lens center. Rerun `tools/rr_stability.py` with the aperture at 0 to confirm no change.
- **Camera models:** orthographic, 360° equirect (a panorama export that opens in VR viewers), little-planet stereographic and tilt-shift.
  - Only ray generation in `trace` changes.
  - Motion vectors assume perspective, so non-perspective models switch to the native denoiser: RR and the temporal upscalers off.
- **Film look stack in `display`:**
  - Tone maps: AgX and Tony McMapface as modes 6 and 7. `validate()` allows `tonemap < 6` today.
  - Effects: halation, grain matched to film stocks and vignette.
  - Grading: a `.cube` 3D LUT loader and lift/gamma/gain wheels.
  - In HDR mode these apply before the display transform from item 2.

#### 5d. Toys

- **Light painting** (long-exposure photography): hold a key, drag a light, and the accumulation keeps every position.
  - An `App` flag, not a `Settings` field, makes light-only edits skip `reset()`. `rebuild_scene` and `update_materials` still run.
  - It forces the progressive path while active: RR and temporal upscalers off, because those reset every frame. The sample cap is lifted.
  - The accumulation is a running mean, so the trail is exposure-normalized, as a real long exposure is.
- **Portals:** paired quads that teleport rays. This gives infinite corridors, Escher rooms and a box that's bigger inside.
  - In the bounce loop, on a portal hit, move origin and direction by the pair's relative transform and continue without counting a bounce.
  - NEE can't see lights through a portal. Emission reached after a portal crossing takes MIS weight 1, because NEE never samples it. Correct, but noisier.
  - A new material flag bit (bit2) plus a pair index in `optical`.
- **Walk mode:** WASD plus mouselook with capsule collision against the triangle soup, for big interiors.
- **Gamepad** via `gilrs`: orbit, zoom, a light-intensity trigger and preset cycling.
- **Windows screensaver:** the same exe, recognized when started as `.scr`.
  - `/s`: borderless fullscreen; slowly orbit and cycle presets (or the scene files from item 4); quit on input.
  - `/c`: open the settings panel.
  - `/p <HWND>`: winit can't easily render into a foreign HWND, so draw a pre-rendered thumbnail with plain Win32 GDI.
  - Install by right-clicking the `.scr` in place, so the NGX, FSR and XeSS DLLs stay next to it.
- **Video export:** pipe frames straight to `ffmpeg` (`-f rawvideo -pix_fmt rgba` on stdin) when it is on `PATH`, with optional 10-bit HEVC for HDR. A turntable at N spp per frame works today; keyframes come with the animation track below.
- **Phone remote control:**
  - An `axum` server with a websocket serves a page of sliders.
  - Edits come in as `key=json` and go through the same path as `--set`, so `validate()` guards them.
  - Off by default. It binds to the LAN with a random token shown in the UI as a QR code.

#### 5e. Scenes buildable today (as scene files)

- **Kusama, *Infinity Mirror Room*:** a mirror cube with a few hundred tiny emissive spheres, within the 256-object cap.
- **Dan Flavin installations:** emissive colored cylinders in a white room; pure color bleeding.
- **James Turrell Ganzfeld room:** rounded corners from stacked tori and cylinders, with one colored emitter.
- **Veach MIS test scene:** 4 glossy plates × 4 light sizes, which shows off the MIS. Add a toggle that shows NEE-only and BSDF-only.
- **Seed of the day:** `randomize()` seeded from today's date. It moves to the generators once they exist.

## Medium term

### Core track

Order is from scene-research.md section 5, unchanged:
1. Generators.
2. Mesh import (OBJ, then glTF) with an opaque/non-opaque geometry split.
3. HDRI environments with importance sampling.
4. Multiple BLAS + TLAS instancing past 256 objects.
5. Textures.
6. UI phases 2–5.

### Feature tracks

These follow the core track. Each lists what it needs; within those limits the order is a suggestion. The letters refer to the engine features in scene-research.md section 4. Items 7–12 schedule the ones the core track left out; the rest of the features depend on them.

#### 7. Analytic lights (F)

Point/sphere, spot and directional sun with an angular radius, as in scene-research.md. Prefer small spheres and disks over true delta lights, for RR and the native denoiser.

- Unlocks **flashlight mode**: a spotlight attached to the camera. With walk mode (5d) and a dark Sponza, it's an instant horror game.

#### 8. Animation and timeline (K)

- **Keyframes:** camera, object transforms, light intensity and color, and material parameters. Use a timeline strip in the UI, with linear, smooth and hold interpolation. Keyframes are stored in the scene file.
- **Video export:** step the timeline per frame, render N spp, and pipe the result to ffmpeg (5d).
- **Motion blur:** shutter open/close settings.
  - DXR has no motion BVH. Pick one shutter time per spp batch, set the instance transforms for that time and rebuild the TLAS.
  - This is cheap only after instancing (core 4), where the TLAS is rebuilt and the BLAS stays.
  - Recreate Cook's 1984 pool-ball scene as the showcase.
- **Unlocks:** Newton's cradle, flickering candles, day-cycle time-lapses (item 9) and turntables with keyframed lights.

#### 9. Physical sky and day cycle

Needs the sun from item 7 and the importance-sampled environment from core 3.

- **Sky model:** Hosek–Wilkie, or the Prague model for more accurate sunsets. Bake it into the environment map on change and reuse the HDRI CDF path.
- **Controls:** time of day, latitude and turbidity. With item 8, a time-lapse through a window.

#### 10. Nested dielectrics (H)

Unlocks the glass of water, ice in a drink, the **snow globe** and the **aquarium** (with item 12 for murky water).

#### 11. Layered and exotic materials (I)

- Clearcoat and sheen first. Then anisotropy for brushed metal.
- **Thin-film interference:** soap bubbles, oil slicks, anodized titanium and beetle shells. Belcour and Barla (2017) give an RGB form that doesn't need spectral rendering.
- **Diffraction grating (CD/DVD):** an anisotropic lobe with a per-channel angular offset as an RGB approximation. It becomes exact after spectral rendering (future goals).
- **GGX energy compensation:** add Kulla–Conty here (a small LUT), since every layered lobe loses energy.

#### 12. Volumes (J)

Homogeneous media inside meshes, plus global fog.

- **Unlocks:**
  - Visible laser beams (item 17).
  - God rays and the **Vermeer room**.
  - Olafur Eliasson's ***The Weather Project***: a sun disc, fog and a mirrored ceiling.
  - Subsurface jade, milk and a glowing **jack-o'-lantern**.
  - **Underwater mode**: the whole room filled with a medium.
- RR handles volumes poorly, so compare it against the native denoiser per scene.

#### 13. Procedural primitives and SDFs

Needs instancing (core 4): procedural AABBs go in their own BLAS.

- **Intersection:** inline `RayQuery` reports `CANDIDATE_PROCEDURAL_PRIMITIVE` in the existing `Proceed()` loop. The shader intersects or ray-marches there and calls `CommitProceduralPrimitiveHit(t)`.
- **Exact analytic shapes:** spheres, tori and cylinders, with no tessellation facets. Today a sphere costs 4,096 triangles.
- **SDFs by sphere tracing:** the Mandelbulb, a Menger sponge at any level, metaballs (a **lava lamp**) and smooth CSG subtraction. Normals come from the SDF gradient.
- This replaces scene-research.md's "SDF → marching cubes → mesh" route for anything that doesn't need to be a mesh.

#### 14. Path visualizer and light path expressions

- **Path visualizer:**
  - Click a pixel. A small dispatch re-traces that pixel's paths with the same sample indices and writes the vertices, throughput and event types (diffuse, specular, refract, NEE, portal) to a buffer.
  - The paths draw as 3D lines over the viewport, colored by bounce or throughput, with a scrubber per path.
  - Needs a line overlay. It fits with UI phase 2's viewport work.
- **Light path expressions (LPE):**
  - Regex-filtered AOVs, such as `LS+DE` for caustics only, or "only light that bounced off the red wall".
  - A small DFA runs per path in `trace`, writing into a few configurable AOV slots.
  - It generalizes today's direct, indirect and spec AOVs.

#### 15. More integrators and integrator battle mode

- **BDPT:** good for light behind a door, the Veach "door" scene.
- **SPPM / photon mapping:** a **caustics mode**. A light-tracing compute pass deposits photons into a hash grid, and camera paths gather from it. Real caustics under glass spheres and on the glass-lab table.
- **ReSTIR DI:** thousands of emitters at 1 spp, for a night city or a dense Kusama room. It slots in beside `select_emitter` and reuses the motion vectors and object IDs.
- **Battle mode:** a 2×2 split of path, BDPT, SPPM and path with ReSTIR at equal time, with the live convergence graph (5b) per quadrant. It generalizes the A/B slider.

#### 16. Real lenses and physical glare

After thin-lens depth of field (5c).

- **Lens systems:** trace through real lens prescriptions (Kolb 1995): a Double Gauss, a Zeiss Planar or a fisheye. Distortion, vignetting, chromatic aberration and bokeh then come from the glass elements themselves.
- **Glare:** an FFT of the aperture shape (the same shape as the 5c bokeh) gives the diffraction starburst on bright emitters. Convolve in `display` at a reduced resolution.

#### 17. Physics sandbox, optics bench and laser puzzle

- **Physics sandbox** (`rapier3d`):
  - Drop balls, throw objects with the mouse, and knock over Jenga towers and domino chains.
  - Needs instancing (core 4), so each step only rebuilds the TLAS. While the simulation runs, the renderer stays in per-frame RR mode.
- **Optics bench:**
  - New shapes: bi-convex and plano-concave lenses, a prism, and a beam splitter (a partial-mirror material).
  - It works as a physics-class simulator, and becomes truly colorful after spectral rendering.
- **Laser puzzle game:**
  - Gameplay: a laser (a thin emitter, visible in fog from item 12, or a cheap emissive cylinder before that) plus placeable mirrors, prisms and splitters. Route it to a target in N moves.
  - Win detection: a **sensor material** that adds the radiance hitting it into a counter buffer, read back once per frame.
  - Levels are scene files.

#### 18. Content using the features above

- **Art and history:**
  - Edward Hopper's *Nighthawks*: mesh import plus item 7.
  - The real Cornell box: measured spectral data and a split view against the photograph. Exact after spectral rendering.
  - Cook's pool balls (item 8).
- **Miniatures:**
  - Snow globe (10), aquarium (10, 12) and lava lamp (13).
  - Newton's cradle (8) and an hourglass (mesh import).
  - Jack-o'-lantern (12) and a birthday cake with flickering candles (7, 8).

#### 19. Integrations and whimsy

- **Text-to-scene with Claude:**
  - Needs the scene files (item 4) and ideally the generators (core 1).
  - The prompt carries the `Settings` schema, the shape, pattern and material-library lists, and a few example files. The model returns JSON, which goes through `validate()`; on failure, retry with the error.
  - **Art critic mode:** send the render back and ask for concrete lighting and composition changes as `key=json` edits.
- **Scripting console** (`rhai`): script animations, generators and batch renders, for example rendering every preset at 4,096 spp. Add a `--script file` CLI flag.
- **Audio-reactive lights:** WASAPI loopback capture (`cpal`) and an FFT drive emitter intensities per frequency band. Runs in per-frame RR mode, since every frame changes the lights.
- **Live wallpaper:** render behind the desktop icons with the WorkerW trick, at low spp. Pause when a fullscreen app runs or on battery.
- **Blender live link:** a Blender add-on streams objects, transforms, materials and the camera over a socket. Needs mesh import and instancing.
- **Web viewer parity:** the WebGL app loads native scene files (only the features it supports), so scenes can be shared with anyone who has a browser.

#### 20. Shader Execution Reordering (DXR 1.2)

- **Payoff:** Ada supports SER. Expect the biggest gains on divergent glass and mirror scenes; the mirror corridor (17) is the benchmark.
- **Cost:**
  - `MaybeReorderThread` is only allowed in ray generation shaders. `trace` has to move from a compute shader to a raygen shader, with a minimal RT pipeline and shader table, keeping inline `RayQuery` inside it.
  - Needs the D3D12 Agility SDK and Shader Model 6.9.
- **Vulkan:** check it before committing. `VK_NV_ray_tracing_invocation_reorder` has the same raygen-only restriction.

## Future goals

### Vulkan backend

- **Why:**
  - Portability: Linux, and AMD/Intel drivers where Vulkan RT is stronger.
  - The upscaler SDKs already ship Vulkan paths: NGX (`nvsdk_ngx_vk.h`, `nvsdk_ngx_helpers_vk.h`, `nvsdk_ngx_helpers_dlssd_vk.h`, all in `vendor/dlss/include`), XeSS (`xess_vk.h`) and FSR (the FidelityFX API has a VK backend).
- **Approach:**
  - Split `gpu.rs` into a backend-neutral `Renderer` and a backend trait: resources/textures, buffers, BLAS/TLAS build, compute dispatch, barriers, timestamp queries, swapchain present, readback and the upscaler hooks. Keep the D3D12 backend as the reference implementation.
  - Vulkan via the `ash` crate.
  - Ray tracing: `VK_KHR_acceleration_structure` + `VK_KHR_ray_query`, so it mirrors the inline `RayQuery` design; no pipelines or SBT.
  - Shaders: compile the same HLSL to SPIR-V with `dxc -spirv` (needs `[[vk::binding]]` annotations or `-fvk-*-shift` flags for the register layout).
  - Keep one source of truth for the `Constants` layout.
- **Risks and size:** the largest item on this list; budget weeks. Barriers and layouts, descriptor management and the NGX Vulkan device-extension requirements are the hard parts.
- **Do after:** HDR (it touches swapchain code in both backends) and before instancing grows `gpu.rs` further. Doing the split earlier is cheaper.

### Software fallback and "perfect screenshot" renderer

Two related goals:

1. **Fallback when there's no DXR GPU.** The cheapest route is Microsoft's WARP adapter, which runs the same D3D12 + DXR 1.1 code on the CPU. First verify that the current WARP build supports raytracing tier 1.1; recent Agility SDK versions advertise it. If it does, adapter selection (`Gpu::new`) just falls back to `IDXGIFactory4::EnumWarpAdapter` when no hardware adapter has tier 1.1. It will be slow, roughly seconds per frame, but correct, and the UI keeps working. Upscalers are unavailable there; bicubic applies.
2. **CPU reference renderer for ground-truth screenshots.** A separate Rust path tracer over the same `Settings` and geometry:
   - Same materials, emitters and sampling as `pathtrace.hlsl`.
   - BVH via Embree (`embree4` bindings) or a Rust BVH (`bvh` / `obvhs`), parallelized with `rayon`.
   - f64 accumulation, no denoiser, no clamps; optionally more bounces and spectral rendering.
   - Runs in the background with progress (`--reference N spp`), or headless from the CLI, and writes EXR/PNG.
   - Doubles as the oracle for GPU correctness tests: today the references are the GPU's own 16,384-spp renders, so a GPU bug would be in the reference too. A CPU renderer gives an independent check for `motion_error.py` and `upscale_quality.py`.
   - Keeping two path tracers in sync is the cost. Share material/sampling code where possible (a Rust module with the BSDF/emitter math mirrored in HLSL), and add a self-test that compares a low-sample GPU and CPU render statistically.
   - **Ordering with spectral rendering:** decide on spectral (below) before writing this. If the GPU goes spectral, the CPU reference should be spectral from day one rather than ported later.

### Spectral rendering

The biggest single jump in physical realism.

- **Approach:**
  - Hero-wavelength sampling (Wilkie et al. 2014): 4 wavelengths per path, with throughput as a 4-vector.
  - Convert to XYZ, then to linear RGB, at accumulation. Everything downstream stays RGB: AOVs, filter, RR, upscalers and display.
  - RGB albedos are uplifted to spectra (Jakob and Hanika 2019, a precomputed coefficient table). Kelvin lights become exact blackbody spectra.
- **Unlocks:**
  - **Dispersion:** IOR per wavelength (Cauchy or Sellmeier). A prism throws a rainbow on the wall; ship a *Dark Side of the Moon* preset. Diamonds sparkle, and the optics bench (17) becomes colorful.
  - Exact thin film and diffraction gratings (11).
  - The measured Cornell box spectra (18).
  - **Fluorescence:** UV black lights make "invisible" paint glow; add a black-light mode to the Neon preset. A material re-emits absorbed short wavelengths at longer ones (a re-radiation matrix per material).
- **Cost:**
  - `pathtrace.hlsl` changes almost everywhere.
  - Color noise rises at low spp, so measure RR and the native filter on it.
  - Keep an RGB build path for comparison.

### Polarization

- Mueller-matrix and Stokes-vector transport, after spectral. Fresnel becomes polarization-aware.
- **Showcase:** a virtual polarizing filter you can rotate. Glass reflections vanish at Brewster's angle, you get a "sunglasses view", and stress patterns appear in clear plastic.
- Rare in hobby renderers. Research-grade, with high effort.

### Curved space: black holes and non-Euclidean rooms

- **Gravitational lensing:**
  - Inside a black hole's sphere of influence, integrate the null geodesic in small steps (Schwarzschild metric, RK4). Each step is a short `RayQuery` segment.
  - Outside it, rays are straight as today.
  - The showcase is a tiny black hole in the Cornell box, with the walls bending around it, Interstellar-style. Add an accretion disk as an emissive volume (12).
- **Non-Euclidean rooms:**
  - Hyperbolic or spherical space by ray marching in the curved metric.
  - Portals (5d) cover the cheap "impossible room" cases first.
- **RR:** motion vectors and guides from the curved first segment are approximate, so expect to use the native denoiser.

### DLSS Frame Generation and Reflex

- Integrate through Streamline rather than raw NGX, since FG needs its own swapchain hooks.
- Doubles the presented frame rate in interactive use. Not for accumulated stills.
- Do it after native HDR (item 2), because both touch the swapchain. Streamline also has a Vulkan path.

### Neural radiance cache

- A tiny MLP trained online on path suffixes. Paths terminate early into the cache after the first diffuse bounce.
- Options: NVIDIA's NRC SDK, or our own with cooperative-vector matrix ops in HLSL (Shader Model 6.9).
- The payoff is far less noise at the same cost in heavy interiors (Sponza, San Miguel). Measure with the convergence graph (5b) and the CPU reference.

### 3D Gaussian splats

- Ray-traced splats alongside triangles (3DGRT-style): splat ellipsoids are wrapped in proxy geometry, and hits are blended in depth order inside the `Proceed()` loop.
- The showcase is a real captured object or room inside the path-traced Cornell box, receiving and casting light approximately.
- Needs procedural or proxy geometry (13) and instancing.

### VR via OpenXR

- Path-traced stereo at a low render resolution, with RR per eye.
- **Hard parts:**
  - Stereo motion vectors and two RR feature instances.
  - Frame pacing: 90 Hz leaves about 11 ms for both eyes.
- Realistic in the classic Cornell scenes on the 4070 Ti SUPER. Big scenes need the neural radiance cache or ReSTIR.

### Lightmap baker and texture painting

- **Baker:** bake path-traced GI into UV lightmaps and export glTF with the lightmaps, for use in game engines. Needs textures and UVs (core 5).
- **Texture painting:** paint directly on surfaces in the viewport. Strokes are projected through the object-id and UV buffers, and the render updates live.
  - Until textures exist, "paint mode" just recolors the clicked object's material, an S-sized quick win on top of picking.
