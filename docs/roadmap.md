# Cornell RTX roadmap

The single ordered plan. Details live in the two research documents:
[ui-plan.md](ui-plan.md) (UI redesign) and [scene-research.md](scene-research.md) (scenes).
Written 2026-10-30, and extended the same day with the feature brainstorm: quick wins (near-term item 5), feature tracks 7–20 (medium term) and the big ideas in future goals. The near term (items 1–5, 3b) and core tracks 1–4 (generators, mesh import, HDRI, instancing) are done (2026-10-30).

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

Not done from phase 1's list (they belong to ui-plan.md phases 2–5, core track 6): Quality bundles with a `Capabilities` struct, per-pass GPU timing in the status bar, persisted UI prefs (advanced toggle, UI size), modified-row dots, file pickers for settings / LUT / reference EXR (meshes and HDRIs have one), and async high-res export (it blocks the UI today). Done since: undo / redo, viewport handles and hover (3b), the scene browser with thumbnails (4).

### 3b. Blender-style transform gizmos (owner request, 2026-10-30) — done 2026-10-30

As built: move / rotate / scale gizmos for objects, area lights and the main light (no main-light rotation, since it is always horizontal), G / R / S modal transforms with X / Y / Z, Shift+axis planes, typed values, Ctrl snap, Shift fine and cancel, Global / Local orientation, light outlines and light picking, undo / redo. Drag cost is in CLAUDE.md, "Gizmos".

**Not done yet (owner asked to keep this list, 2026-10-30):**
- **Snapping to other objects:** surface, vertex, edge and face snapping, as Blender's snap menu offers. Today Ctrl only snaps to a 0.1-unit grid, 15° and 0.1×.
- **Pivot options:** median point, 3D cursor, individual origins and active element. Today the pivot is always the object's own origin.
- **Multi-select:** Shift-click to add, box select, select all (A), and transforming several items together.
- **Alt-drag duplicate:** Blender's Shift+D duplicate-then-move as well.
- **Smaller gaps:** free moves show one vector line in the header, with no per-axis typed vector entry.
- ~~Light outlines always on~~: hidden by default since item 6.
- ~~Cheap drags~~: done with instancing (core track 4): bunny 26 → 5.5 ms, chess 24 → 6.1 ms per drag frame.

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

#### 5a. Developer tooling — done 2026-10-30

As built: hot reload spawns the build's own `dxc.exe` (path baked in by build.rs) instead of loading `dxcompiler.dll`, polling `shaders/` every 0.5 s, so no `notify` or `hassle-rs` dependency. `tools/perf_history.py` writes `captures/perf-history.csv` and `.png`. Also added: `tools/snapshot_build.ps1` copies a runnable build into `builds/<date>-<name>/`.

- **Shader hot reload** (debug builds only):
  - Watch `shaders/` with the `notify` crate. On save, compile with `dxcompiler.dll` at runtime (`DxcCreateInstance`, via `hassle-rs` or a thin FFI) using the same entries and profiles as `build.rs`, then rebuild the affected PSOs.
  - On a compile error, keep the old PSO and show the DXC message in a toast.
  - Release keeps the embedded DXIL.
- **Perf history:** a script runs a fixed benchmark set (`--headless --no-vsync --frames 200` over presets 0, 13, 17 and 21, native and RR). It appends the `RENDER_OK` means with the commit hash to `captures/perf-history.csv` and plots the history, so slowdowns show up per commit.

#### 5b. Seeing the invisible: debug views and meters — done 2026-10-30

As built: views 8–11 (bounces, BVH cost, variance heat maps, false colour), zebras, scopes binned with atomics in the display pass (no extra pass; cleared by a copy), light meter (one-texel readback of linear output and albedo), convergence graph in egui (reference from an EXR or the current image, log–log with a 1/√n guide), lifetime stats and 8 achievements in `%APPDATA%\CornellRTX\stats.json`.

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

#### 5c. Camera and look — done 2026-10-30

As built: thin lens with round or 3–12-blade apertures, blade rotation, cat-eye (blocked lens samples contribute nothing, so corners also darken), Ctrl+click focus with an eased pull; camera models orthographic, 360° equirect, little planet and tilt-shift (non-perspective models switch RR, temporal upscalers, picking and the gizmo off); tone maps AgX (Wrensch's polynomial fit) and **Khronos PBR Neutral instead of Tony McMapface** (Tony needs its 48³ LUT; load it as a .cube through the LUT slot instead); halation, grain (three sizes), vignette, lift / gamma / gain wheels and a .cube LUT loader. Not done: grain matched to named film stocks, and equirect metadata for VR viewers.

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

#### 5d. Toys — done 2026-10-30

As built: light painting (L), portals (any object pairs with another; two-sided), walk mode (`), turntable video (ffmpeg when on PATH, else PNG frames), screensaver (/s, /p, /c; `tools/make_screensaver.ps1`), gamepad through gilrs, phone remote. Deviations: the remote is a std-only HTTP server with fetch polling (no axum/websocket), and the HDR 10-bit HEVC video option was not done (done with the export dialog, item 6). Untested here: the gamepad (no controller attached). The ffmpeg path was tested on 2026-10-30 once `tools/setup.ps1` pinned ffmpeg (an 8-frame turntable to MP4); the PNG fallback, the remote round trip and the screensaver run were tested earlier.

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

#### 5e. Scenes buildable today (as scene files) — done 2026-10-30

As built: `kusama-infinity-room`, `flavin-fluorescent`, `turrell-ganzfeld`, plus two portal scenes, `infinite-corridor` and `bigger-on-the-inside` (47 built-in scenes). The Veach toggle is a render setting, *Light sampling* (MIS / NEE only / BSDF only), shown on the existing `veach-mis` scene. Seed of the day is the top bar's *Today* button.

- **Kusama, *Infinity Mirror Room*:** a mirror cube with a few hundred tiny emissive spheres, within the 256-object cap of the time (4096 since instancing).
- **Dan Flavin installations:** emissive colored cylinders in a white room; pure color bleeding.
- **James Turrell Ganzfeld room:** rounded corners from stacked tori and cylinders, with one colored emitter.
- **Veach MIS test scene:** 4 glossy plates × 4 light sizes, which shows off the MIS. Add a toggle that shows NEE-only and BSDF-only.
- **Seed of the day:** `randomize()` seeded from today's date. It moves to the generators once they exist.

### 6. Owner requests (2026-10-30, after core track 4)

Small to medium items the owner asked for, in rough order of size. Each also points at the item it extends.

- ~~**Double-click resets a slider**~~ — done 2026-10-30 (the scene's value as loaded; UI values to their defaults). Was: reset to its default (the `Settings::default()` value, or the scene file's value when one is loaded). All sliders go through `crate::slider`, so one wrapper change covers them; the colour wheels (5c) already reset on double-click. Small; belongs with UI phase 2 (core 6).
- ~~**Light rectangles hidden by default**~~ — done 2026-10-30 ("Show outline" per light, off by default; hidden outlines are not clickable in the viewport). Was: the outlines `gizmo::draw_lights` paints over every area light and the main light are for grabbing them, so they should be off unless asked for. Add a "Show outlines" toggle to the existing light settings (the Lights section of the Scene tab, next to each light's *Seen by camera*), plus one for the main light; the selected light still shows its outline and gizmo. Small; extends 3b.
- ~~**More and better HDRIs**~~ — done 2026-10-30: 19 maps at 4k / 8k (two dropped after a 1:1 check), bilinear radiance, RGBA16F storage, four new environment scenes. Still open: more interiors and night maps that pass the check, and the in-app browser below. Was: fetch 4k (and some 8k) maps instead of 2k, add more of them (interiors, night, overcast, golden hour), and drop or replace the ones that look pixelated as backgrounds. Likely cause besides resolution: the camera sees the map through the same nearest-texel lookup the importance sampling uses (core 3), which is blocky at 2k; use a bilinear lookup for camera rays (and possibly for lighting too, with the pdf kept per texel). Check each map at 1440×900 before keeping it. Half-float storage (core 3 not-done) matters at 8k (8k RGBA32F = 512 MB). Small to medium; extends core 3.
- ~~**Better video export**~~ — done 2026-10-30: *Export video…* opens a dialog with size independent of the window (presets up to 8K, ≤ 8192 per side), fps, length ↔ frames, samples per frame and a time limit per frame, H.264 / H.265 (CRF), HDR10 HEVC (10-bit PQ BT.2020 from the HDR output), ProRes (profile), PNG or linear EXR sequences, orbit angle / elevation / distance / easing; it renders a pass at a time with the UI responsive (preview, progress, time left, Cancel) while the swapchain keeps the window's size. Still open: tiling past 8192 (the D3D12 texture limit); camera paths and keyframes (track 8); HDR10 only while the display is in HDR (texture 26 is written only then) and with max-CLL but no mastering-display metadata; no 10-bit SDR output or alpha; EXR sequences hold the beauty only (no AOVs); the scene can't be edited during an export (no background export of a frozen copy while working on); the viewport shows no preview, only the progress window; no audio; the high-resolution still still blocks the window (it could become a one-frame job); no resume after a cancel. Was: today the turntable is a fixed-yaw orbit at the window's size through ffmpeg H.264 CRF 16 (5d). Wanted: an export dialog with resolution independent of the window (up to the high-res still limit and beyond with tiling), fps, duration or frame count, samples per frame or a time budget, codec and quality (H.264 / H.265 / ProRes / PNG or EXR sequence), 10-bit HEVC for HDR (5d not-done), orbit options (angle, elevation, radius, easing), a progress bar with cancel that does not block the UI (shares the async export work of UI phase 1's not-done list), and later camera paths and keyframes (track 8). Medium.
- ~~**Mesh (and HDRI) sources with a built-in browser**~~ — done 2026-10-30: the Assets window browses Poly Haven (HDRIs, textures, models), ambientCG and the Khronos glTF samples (cached catalogs, thumbnails, search, resolution choice), downloads on a background thread with progress and cancel into `scenes/hdri`, `scenes/textures`, `scenes/models` with each file's SHA-256 pinned in `scenes/assets.json`, and uses the result in one click; *Other sources* lists the rest with licences and links. Still open: Sketchfab (needs an API token and OAuth for downloads), Smithsonian's open-access API, and the McGuire / Bitterli / ORCA archives (zips of whole scenes in other formats) have no in-app download; no 3D preview before downloading (thumbnails only); no list or delete of downloaded assets in the app; HDRIs come as .hdr only (no EXR); Poly Haven models are taken as glTF only; catalogs refresh only on *Refresh*; the thumbnail cache is never pruned; downloads are not resumable. Was: a list of sources the owner can use, plus an in-app browser to search, preview and download into `scenes/models` and `scenes/hdri`. Sources: Poly Haven (models, HDRIs and textures; CC0, public JSON API), the Khronos glTF sample assets, the Stanford 3D Scanning Repository and common-3d-test-models (scans), Morgan McGuire's Computer Graphics Archive (Sponza, San Miguel, Bistro and more), Benedikt Bitterli's rendering resources (scenes), NVIDIA ORCA (Bistro, Emerald Square, Zero Day), Sketchfab's downloadable models (API key; licence per model), Smithsonian 3D (open access) and ambientCG (CC0 materials, for core 5). The browser reuses the Scenes window's grid and thumbnail cache, pins every download by SHA-256 like `tools/fetch_assets.ps1`, shows the licence and credit, and imports with *+ Mesh…* / *Load HDRI* in one click. Medium to large; extends core 2 and 3, and the textures track (core 5) once materials download too.

Second batch (2026-10-30, after UI phases 3–5). The bigger ones extend a feature track or future goal, as noted:

- **Tree leaves are plain squares.** Step (1) done 2026-10-30: each leaf is a pointed oval of 8 single-sided triangles folded along the midrib (the shader already turns normals toward the ray, so the old back faces were redundant), and a third of the leaves go into a third mesh part, *Leaves (tint)*, which the `tree` and `forest` generators colour separately (the forest's counts became 400–1350 to fit three objects per tree). Still open: step (2) below, per-leaf colour (only two colours per tree; vertex colours don't exist), and the imported-foliage check. Was: The L-system tree generator (`procedural::tree`) emits each leaf as a flat double-sided quad with no texture, so there is no transparency problem to fix: there is no leaf shape at all. Fix in two steps. (1) Small: give the leaf cards UVs and a procedural leaf-shaped alpha mask (an ellipse or a lobed outline, generated in code and registered as a texture), so the existing alpha test from core 5 cuts them out; or build each leaf as a 6–8 vertex polygon instead of a quad, which needs no texture. Add a leaf colour variation per card. (2) Medium: thin-leaf transmission (a diffuse transmission lobe, so sunlit leaves glow from behind) as a material option, which also suits paper and lampshades; it fits track 11. Also check the imported foliage path: glTF BLEND alpha is treated as MASK at 0.5 (core 5 not-done), so soft-edged leaf textures look hard-edged; stochastic transparency would fix that. Small to medium; extends core 1 and 5.
- ~~**Real walking.**~~ — done 2026-10-30: gravity, a body of spheres above step height, a ground ray that climbs ledges and stairs up to 0.12 and follows slopes to 50°, jump (Space), crouch (C / Ctrl, blocked under low ceilings), sprint, head bob (B), body size on the wheel, and F for the old flying camera (on by itself when nothing is below). Still open: no moving platforms or physics objects to stand on; the body is a column of spheres, not a true capsule, so very thin rails can slip between them at large body sizes; collision tests every triangle's box each frame (fine for the chess set, not measured on 20M-triangle files); the flashlight (track 7) is not built. Was: Walk mode (5d) is a flying camera with sphere collision. Make it a first-person walk: gravity, a capsule at standing height that follows the floor, step-up over small ledges and stairs, slopes, jump, crouch, sprint, optional head bob, and a fly toggle that keeps today's behaviour. Pairs with the flashlight (track 7) and big interiors (Sponza, San Miguel). Small to medium; extends 5d.
- **Physics with a stacking generator and tools:** see track 17, extended below (stacking generators, wind, explosions, grab and drag, NVIDIA-PhysX-demo-style scenes). Large.
- **Even more accurate rendering, with a software mode if real time can't do it:** see "Accuracy mode and reference renderer" under future goals. Large.
- **A better denoiser than RR** (low priority): see "Other denoisers" under future goals. Medium to large.

Third batch (2026-10-31, owner requests):

- ~~**File format choice for the high-resolution export.**~~ — done 2026-10-31: one `video::ImageFormat` (PNG, JPEG with a quality slider, HDR EXR as linear 32-bit float, HDR PNG as 16-bit PQ / BT.2020 while HDR output is on) and one writer (`video::write_image`) behind *Save the current image* (one button per format, F12 = PNG), the high-resolution still (a *Format* combo) and the PNG / EXR image sequences. Still open: no 16-bit tone-mapped PNG (the tone-mapped image is RGBA8 on the GPU), no half-float or multi-layer (AOV) EXR, no TIFF / WebP / AVIF; JPEG sequences in the video dialog; `--capture` still goes through `Gpu::capture` (PNG / EXR / `.hdr.png` only). Was: The high-res still has no format selector, unlike the normal export. Add the same choices (PNG, JPEG, EXR, and whatever the normal export offers, with quality / bit depth where they apply) to the high-res dialog and share the code path so the two stay in step. Small; extends the high-res still (core 6 / UI phase 1).
- ~~**Anti-aliasing when DLSS is off and the native unbiased path is used.**~~ — done 2026-10-31, in three parts, measured with the new `tools/native_aa.py` (640×480, 4 spp per frame, edge-pixel RMSE ×1000 against 16k-spp references with the same filter; still camera / end of `--orbit-test`). (1) The native denoiser divided by and multiplied back the albedo of the single unjittered guide ray, so an edge pixel's mixed colour was snapped to one surface: without RR the albedo guides (u3, u4) are now the mean first-hit albedo of the samples themselves. (2) The à-trous object-id stop still works from the pixel centre, so the trace also accumulates coverage (the share of samples whose first hit is the guide's surface, in u3.a) and the last pass gives pixels below full coverage their unfiltered share back. (3) A *Pixel filter* for the progressive path (filter importance sampling, weight 1 per sample, per axis so Sobol stratification holds): Box (the old one), Gaussian, Blackman-Harris (new default), with a width (default 1.5 px, as Cycles). Native denoiser, edge error main → now: unlit edge scene still 48.6 → 26.8, orbit 42.5 → 27.2; Classic still 83.8 → 51.1, orbit 70.3 → 49.5 (box filter alone with (1)+(2): 28.1, 31.5, 66.0, 52.1); whole-image error dropped too (Classic orbit 8.7 → 6.4). Raw samples with the box filter are bit-identical to before. Still open: a native TAA with jittered guides (SVGF's usual partner) for camera motion; supersampling above 100% render scale; an SMAA / FXAA pass; filtering happens in linear light, so the 12× HDR light edge still steps after tone mapping (the reference too); a negative-lobed filter (Mitchell, Lanczos) needs weighted samples; RR and the temporal upscalers keep their own jitter (the filter does not apply). Was: With RR / DLAA off there is no edge filtering beyond the pixel jitter that accumulates over time, so moving views and low sample counts show stair-steps. Options to evaluate: sub-pixel jitter with a proper reconstruction filter (Blackman-Harris / Mitchell) in the accumulation, a native TAA that reuses the native denoiser's history, supersampling via render scale above 100%, and an edge-aware post pass (SMAA / FXAA) as a cheap fallback. Measure edge shimmer with the `tools/rr_stability.py` method. Medium; extends the native denoiser.
- ~~**Anisotropic filtering for textures.**~~ — done 2026-10-31: the ray-cone footprint at a hit is now an ellipse in uv (across the ray: the cone's width; along it: stretched by the grazing angle, capped at 20×), mapped through the world-space dP/du, dP/dv (box-mapped sets and mesh UVs alike) and passed to `SampleGrad` on an anisotropic sampler (16×); *Texture filtering* (Render tab, 1–16×, default 16×) clamps the ratio by widening the short axis, and 1× is the old isotropic lookup. Secondary bounces already read wider mips through the cone spread. Checked on the showroom's wood floor at pitch −4° (sharp to the horizon at 16×, smeared at 1×) and the ASWF chess set; cost within run-to-run noise (showroom 4.66–4.85 ms vs 4.69–4.85, chess 5.97–6.10 vs 6.03–6.20, 1440×900 RR). Still open: the alpha test still samples LOD 0; the cone spread per rough bounce is still the 0.5·α heuristic; mips are still box-filtered in sRGB. Was: Textured scenes blur or shimmer at grazing angles (floors, walls). Add anisotropic sampling to the texture lookups (ray differentials or cone footprint along the path to pick the mip and the ellipse axis, 4x to 16x taps as a setting), and make sure mipmaps are used on secondary bounces too. Check on the textured floors of `scenes/materials` at grazing views. Small to medium; extends core 5 (textures).

## Medium term

### Core track

Order is from scene-research.md section 5, unchanged:
1. Generators. — done 2026-10-30: 11 families in `src/generators.rs`, a Generate section in the Scenes window, and a `scenes/generated` pack of 90 curated seeds. The mesh-based generators (Mandelbulb, terrain, Menger 3–4, L-system trees) followed mesh import, as `gen:` procedural meshes.
2. Mesh import (OBJ, then glTF) with an opaque/non-opaque geometry split. — done 2026-10-30: `src/mesh.rs` reads OBJ (tobj, MTL diffuse) and glTF/GLB (node transforms, PBR base colour, roughness, metallic, emissive strength, transmission, IOR) into the same −1..1 box as the built-in shapes. *+ Mesh…* in the Scene tab opens a file picker; `Object.mesh` keeps the path (relative to `scenes/` when inside it). The BLAS has two geometries: lights hidden from the camera and glass first (any-hit loop), everything else flagged opaque. Vertices moved to a DEFAULT-heap buffer. Clicks pick on the GPU (object-id readback), so big meshes cost nothing to click.
   - Follow-ups done the same day: `tools/fetch_assets.ps1` (11 pinned models) and a `scenes/meshes` pack; multi-material files split into one object per material; texture averages as colours; the mesh-based generators; the opaque split measured 3–8% faster.
   - Still open: moving a split file's parts together (multi-select from 3b). Texture and normal maps came with track 5.
   - Owner request: a list of mesh sources and an in-app browser that downloads into `scenes/models` (item 6).
   - Importer gaps: glTF clearcoat, sheen, volume attenuation and `KHR_texture_transform` are ignored (the toy car's paint and the dragon's attenuation colour are approximated by hand in the scenes); OBJ reads only the MTL diffuse colour and shininess (no Ke emission, Ks, d / Tr transparency); no up-axis option, so Z-up scans (Lucy) need a −90° tilt; no PLY or STL.
   - Grounding a mesh scans every vertex of the whole file on each `object_frame` call; not measured on 20M-triangle files.
3. HDRI environments with importance sampling. — done 2026-10-30: .hdr / .exr equirect maps (`src/environment.rs`), luminance·sinθ CDFs on the GPU, NEE shared with area lights and MIS on misses, strength / rotation / seen-by-camera, 5 Poly Haven HDRIs in `tools/fetch_assets.ps1` and a `scenes/environments` pack. Checked unbiased (MIS, NEE-only and BSDF-only agree).
   - Not done: a separate sun light for very small suns, ground projection / shadow catcher, MIS compensation. (Half-float storage and bilinear lookup came with item 6.)
   - Owner request: more HDRIs at 4k / 8k, replace the pixelated-looking ones, bilinear background lookup, and an in-app browser (item 6).
   - Decide: lights hidden from the camera still block light and cast shadows (their quads stay in the BLAS for shadow rays). The studio stage's hidden rim light shadowed the HDRI sun, so the environment scenes start with no lights. Either keep it (physical) or make hidden lights invisible to shadow rays too.
4. Multiple BLAS + TLAS instancing past 256 objects. — done 2026-10-30: one object-space BLAS per distinct mesh, a static BLAS for floor / room / lights, a TLAS instance per object, an instance table for vertices, materials and emitters; 4096 objects; `forest` and `crowd` generators. Drags rebuild only the TLAS (chess set 141 → 7 ms per drag frame).
   - Not done: TLAS refit (`ALLOW_UPDATE`) instead of a full rebuild per edit; BLAS compaction; the mesh vertex buffer is re-uploaded whole when the set of meshes changes (adding one mesh to the chess scene re-sends 19 MB); a light edit still rebuilds the static BLAS (small).
   - Cost: simple scenes render up to 7% slower (two-level traversal; Classic RR 3.67 → 3.94 ms). Idea: flatten scenes with few objects into one BLAS, or put the floor and walls in the opaque range of a shared instance.
   - The Menger generator still merges cells into runs to fit 256 objects; it could use one instance per cube now. The object list UI draws every row (no virtual scrolling) at 4096 objects. `frame_objects` past 256 objects uses mesh bounding boxes (loose for rotated objects).
   - Coplanar faces (a glass object's base on the floor) z-fight differently than before; see CLAUDE.md "Instancing".
5. Textures. — done 2026-10-30: imported meshes sample their base colour, normal and metallic-roughness maps (UVs in the vertex buffer, a bindless table of up to 4096 maps, mipmaps chosen by ray cones, alpha MASK cutouts in the any-hit loops), and any material (shapes, meshes, floor, walls) can take a box-mapped texture set from an ambientCG or Poly Haven folder. Material values scale the maps (value / average), so old scenes and the sliders keep working. Untextured scenes run a shader variant without the map code (unchanged speed); the textured chess set costs 16% more than its flat averages did. 8 CC0 sets in `tools/fetch_assets.ps1` and a `scenes/materials` pack.
   - Not done: emissive maps (emission stays the average colour: light sampling has no texture lookup; would need per-emitter-triangle UVs); glTF `KHR_texture_transform`, TEXCOORD_1, stored tangents (per-triangle tangents from UV derivatives instead of MikkTSpace), occlusion maps (a path tracer does not need them), clearcoat / sheen maps; BLEND alpha is treated as MASK at 0.5 (no stochastic transparency); texture sets ignore Opacity, AO and Displacement maps.
   - Not done: box mapping has hard seams on curved shapes (a blended triplanar would need three lookups); a texture set on a mesh is box-mapped too (no option to use the mesh's UVs); built-in shapes have no UVs.
   - Not done: maps are uncompressed RGBA8 (a 4k map is 85 MB with mips; BC7 would be a quarter); sRGB maps are mip-filtered in sRGB; the ray-cone widening for rough bounces is a heuristic (0.5·α or 0.5 per bounce), not the lobe's real spread; the alpha test samples LOD 0.
   - Not done: the material editor picks a file (no folder picker, no previews); downloading sets from inside the app belongs to the asset browser (item 6); texture painting and the lightmap baker (feature tracks) can build on the UVs now.
6. UI phases 2–5. — done 2026-10-30 (phase 5 in part). Phase 2: right-click menus (viewport and outliner), Ctrl+D / Shift+D duplicate, H / Alt+H hide, Home, Page Up / Down, F frames the selection, double-click frames, middle-drag pan, an F1 shortcut sheet, selection and hover outlines. Phases 3–5: favourite and recent scenes, Ctrl+S / Ctrl+Shift+S / Ctrl+O, `--preset` names, persisted preferences (UI size, advanced, first-run hint), Quality bundles in the top bar, per-pass GPU time and time-to-cap in the status bar, the high-resolution still as a non-blocking job up to 8192 (PNG or EXR), Show in folder toasts, a draggable A/B split, a wrapping top bar.
   - Not done: Alt-drag duplicate (Shift+D instead); outlines for lights and silhouette outlines from the id buffer (outlines are projected boxes and vanish while a corner is behind the camera); the preset registry refactor (slugs only); a capabilities struct with per-backend reasons; the "what the denoiser is doing" line; export file name templates; lens shift to centre the subject in the free viewport (touches the camera and RR, needs `tools/rr_stability.py`); material ball and light thumbnails.

### Known issues (not scheduled)

- **Native denoiser:** faint low-frequency blotches on flat walls at very low spp, typical of SVGF (found in the 2026-10-29 review, not started).
- **Gamepad:** untested, no controller here (5d).
- **Hidden lights occlude:** see core 3 above.

### Feature tracks

These follow the core track. Each lists what it needs; within those limits the order is a suggestion. The letters refer to the engine features in scene-research.md section 4. Items 7–12 schedule the ones the core track left out; the rest of the features depend on them.

#### 7. Analytic lights (F) — done 2026-10-30

As built: extra lights have a *Kind*: rectangle, disk, sphere, spot (a disk with a cone and a soft edge) and sun (angular diameter, 0.53° by default). Spheres and suns are sampled uniformly over the solid angle they cover, disks and spots by area; all share the power-weighted emitter table with MIS, and were checked unbiased (MIS, NEE-only and BSDF-only agree). Spheres, disks and spots are real geometry (picking, RR guides, reflections); suns are found on misses. See CLAUDE.md "Analytic lights".

- Not done: glTF `KHR_lights_punctual` import (mesh import ignores lights); true delta point and directional lights (by design: the smallest sphere is 0.05 across, the smallest sun 0.05°); IES profiles, gobos / cookies and light linking; the main ceiling light stays a rectangle; a sun is not split out of an HDRI automatically (core 3 not-done); a sphere's camera and BSDF hits see its 64 × 32 inscribed mesh while light sampling sees the true sphere (under 0.5% of its solid angle, at the silhouette); a sun's weight in the light-selection CDF uses a nominal 50-unit receiving area, so a sun beside very bright small lights can be under-sampled; suns are tested per miss in a loop (fine for a few, not for 32); spheres and suns do not get a shape-specific gizmo (spheres scale their diameter, suns ignore size); no scene files use the new kinds yet; flashlight mode (below) is not built.

- Unlocks **flashlight mode**: a spotlight attached to the camera. With walk mode (5d) and a dark Sponza, it's an instant horror game.

#### 8. Animation and timeline (K) — done 2026-10-31

As built: keyframes on any float in the scene (camera, object transforms, lights, materials, exposure, sky, HDRI rotation) as tracks on JSON paths, stored in the scene file; smooth (monotone cubic, no overshoot), linear, ease and hold per key; a timeline strip (play / scrub / step, the selection's keys, interpolation, length, fps, shutter); I keys the selection or the camera, Shift+I every edited value, Alt+I deletes; the video dialog's *Timeline* motion (and `--turntable-timeline`) renders the animation, with motion blur from a shutter angle (one moment per pass, TLAS / materials rebuilt, accumulation kept, RR off); `scenes/builtin/pool-break-1984` as the showcase. See CLAUDE.md "Animation and timeline".

- Not done: a curve editor (keys are edited by value at the playhead only, no tangent handles or dragging keys in time); keys on booleans and choices (light on / off, shapes, tone map; flickering candles need a hold-keyed intensity instead); per-object timing tools (offset, scale, copy keys); camera paths along a spline with look-at targets; recording a walk or a gizmo drag into keys (auto-key); Shift+I with no keys yet keys nothing (an edit counts only against the last time the animation applied); keyed playback serializes the whole scene each frame (fine for hundreds of objects, not measured at 4096); motion blur only in exports (the viewport plays sharp) and only on the non-RR path; each pass is one moment, so deformation and HDRI-rotation blur come only from many passes; audio; Newton's cradle and day-cycle time-lapses (track 9) are not built as scenes yet.


- **Keyframes:** camera, object transforms, light intensity and color, and material parameters. Use a timeline strip in the UI, with linear, smooth and hold interpolation. Keyframes are stored in the scene file.
- **Video export:** step the timeline per frame, render N spp, and pipe the result to ffmpeg (5d). The export dialog itself (resolution, fps, codecs, progress, HDR) comes first, as item 6.
- **Motion blur:** shutter open/close settings.
  - DXR has no motion BVH. Pick one shutter time per spp batch, set the instance transforms for that time and rebuild the TLAS.
  - This is cheap only after instancing (core 4), where the TLAS is rebuilt and the BLAS stays.
  - Recreate Cook's 1984 pool-ball scene as the showcase.
- **Unlocks:** Newton's cradle, flickering candles, day-cycle time-lapses (item 9) and turntables with keyframed lights.

#### 9. Physical sky and day cycle — done 2026-10-31

- **Model:** Hosek–Wilkie "An Analytic Model for Full Spectral Sky-Dome Radiance" (2012) plus the 2013 solar-radiance fit; their datasets live in `assets/sky/` (3-clause BSD data, credited in `assets/CREDITS.md`; the layout decode and the paper notes are in `docs/research/track-9-physical-sky.md`). CPU bake at 2048×1024 into the existing `EnvMap` → `Gpu::set_environment` path, so CDF importance sampling, bilinear radiance and the background all come free from the HDRI track. The sun is baked as an exact 0.53° disc; the datasets' solar fit is a handful of times the sky radiance, so the disc carries `SOLAR_GAIN` 8000 to give a real sun-patch-to-pool ratio (~6× measured through the showcase window). Below the horizon the day model's lower half is scaled by the floor's albedo, and night blends (elevation-smoothstep over −7°..+1°) to a plain hemisphere pair (`NIGHT_ZENITH`/`NIGHT_HORIZON` × 0.4 below the horizon).
- **Sun position** (exact ecliptic form, chosen over the NOAA approximation whose constants could not be verified online): λ = 90° + (day−172.4)·360/365.25 anchored at the June solstice; δ = asin(sin ε sin λ); α = atan2(cos ε sin λ, cos λ); the hour angle carries the equation of the equinoxes (λ−α wrapped, ±10 min; the ±8 min equation of center from orbital eccentricity is omitted — noted as an artistic choice). elevation = asin(sin φ sin δ + cos φ cos δ cos h); azimuth from north via atan2(east, north), east = −cos δ sin h, north = sin δ cos φ − cos δ cos h sin φ. World dir: north −Z, east +X.
- **Settings:** sky_model (0/1), sky_time (0..24 h), sky_latitude (±85°), sky_day (1..365), sky_turbidity (1..10), sky_strength (0..100, default 1; the model reads nits-like so showcases use ~0.05). All in `transport_key`; a separate `sky_bake_key` in `redraw()` re-bakes on change (floor colour included since it is the model's ground albedo); the physical sky replaces the HDRI map when on, and hdri_rotation/hdri_visible still apply to the baked map. UI: Physical sky row with an Off/Hosek–Wilkie toggle plus five slider rows.
- **Bake perf:** 2048×1024 in parallel row strips (`thread::scope` + `chunks_mut`, procedural.rs pattern): ~110 ms on this machine's cores, so a scrubbed day cycle re-bakes each keyframe and stays responsive.
- **Scene:** `morning-window-room` (sealed white room, east window with muntins, pedestal + glass marble + copper ball), sky_time keyed 9.6 → 13.5 for the day-cycle sweep; the sun patch and its crisp mullion shadows crawl across the floor.
- **Tests:** six `sky.rs` unit tests (clock-following sun, warm-at-low solar ratio, blue zenith, exact disc texels in the bake, night never black, deterministic strips). Self-test 285–288: noon elevation/azimuth vs clock, environment asserted loaded, night elevation < −5°, morning solar warmth ratio; scene files now start at 289 (455 frames total).
- **Checked by render:** open-air desert test showed crisp long sun shadows with the disc lit; the sealed window room's pool measured numerically (linear EXR ~3.5 vs ~1.2 ambient near the window). Frame at gain 8000: the warm pool + mullion shadow read clearly.
- **Not done:** spectral output (the model is per-channel quintics — spectral would want the 2012 paper's own spectral tables); Prague/Hosek-Wilkie 2015 night-sky dataset (a hand-blended night hemisphere is used); sun elevation below −7° clamps to full night; no separate analytic sun light paired with the sky (light-sampling modes still work through the map's CDF); the disc is not split out, so at huge distances the disc is a bilinear blob; no in-shader sky (CPU bake only); day-cycle time-lapse exists via keyed sky_time (pool-break pattern) but no built-in time-lapse showcase scene beyond morning-window-room.

#### 10. Nested dielectrics (H) — done 2026-10-31

Unlocks the glass of water, ice in a drink, the **snow globe** and the **aquarium** (with item 12 for murky water).

- **Interior list per path** (the roadmap term is "IOR stack") in the dielectric branch of `trace`: `med_id[MED_DEPTH 8]` and `med_n`, the volumes the path is inside in the order entered; only the material id is stored per slot, and slot 0 is the ambient air (a sentinel that no material id matches). The current medium is the list top, so every boundary reads the *pair* of media it touches: `eta = inc/tra` with `inc` the incident medium's optical index and `tra` the transmit medium's, Schlick base `((inc-tra)/(inc+tra))²` written `r0*r0` (a plain `pow(base, 2)` would take a negative base when leaving denser media than entering — NaN), Beer–Lambert `-log(max(colour, 0.001))` of the medium being entered, and the outer medium's absorption looked back up by id on the pop, so glass past a liquid absorbs again on the far side. `throughput *= eta*eta` belongs to the transmission event only, so a total internal reflection off a glass's inner wall leaves the medium alone — impossible in the old air-relative model, and the point of ice (1.31) in water (1.333).
- **Push / pop / pass-through:** a front-facing hit (`h.front`, from `dot(d, h.n) < 0` in `intersect` before the normal is flipped) of a transmission surface not on the list pushes it; a back-facing hit pops through the topmost entry with the hit's material id; a surface of a volume the path never entered passes through with no medium change. Volumes are modelled **inset**, never piercing each other (hairline clearances asserted in the generator), so the liquid's surface doubles as the container's inner wall. No new data: the list is built from the material ids the ray query already returns, so `PackedMaterial` and the opaque/any-hit split are untouched. Depth cap `MED_DEPTH` 8; the list is walked with unrolled constant-index loops because DXC has no dynamically indexed local memory.
- **Checked numerically:** a head-on stack (air → glass 0.05 → water 0.30 → glass 0.25 → air → an unlit card at radiance 4, 96×72, 4096 raw native samples) matches the nested Fresnel × Beer–Lambert product within ~1% (2.433/3.096/2.989 measured vs 2.454/3.111/3.022), the pre-change build matches its own air-on-both-sides model within ~1%, and the measured old/new ratio (1.178, 1.059, 1.101) matches the analytic (1.192, 1.063, 1.112). Single-surface glass is unchanged: `cornell-water-tank` means agree to 1e-7 and RMSE 8.4e-05 (a few grazing samples where the `pow(x,2)` → `x*x` reassociation crosses the reflect/refract threshold). Cost (`--no-rr --raw --no-vsync`, 640×360, 300 frames, this build run alternating with the pre-change build): `glass-stack` 0.723/0.707 ms against 0.674/0.669 and `cornell-water-tank` 0.919/0.920 against 0.858/0.861, so +5 to +7% on glass scenes and a scene with one boundary costs the same. **Storing ids only is what keeps it there**: caching each entry's optical index and Beer–Lambert colour next to it (40 registers of list state) dropped the trace's occupancy and measured 2.3 ms on the tank (+46%), while reading those values back from `materials[]` by id costs nothing; the ids-only form renders bit-identically (RMSE 0.0) to the cached form on both test scenes. Unit test `nested_showcases_keep_the_volume_chain`; self-test 458 frames (scene files start at 289, the built-in pack is now 52).
- **Showcases:** `glass-of-water` (thick tumbler, water to the brim, three submerged ice cubes), `snow-globe` (water-filled globe with trees, snow beads and gold baubles seated inside the liquid) and `aquarium` (brim-full tank with gravel, a plant and fish), all rendered with RR at 960×540 and checked by eye.
- **Not done:** the general priority-based interior list — Schmidt and Budge, *Simple Nested Dielectrics in Ray Traced Images* (Journal of Graphics Tools 7(2), 1–8, 2002), the same list RenderMan/Arnold/Houdini/Takua/RTXPT use with a bounded rejected-hit counter (`kMaxRejectedDielectricHits` 4/16) and a continuation query — so overlapping or face-to-face volumes stay pass-through and `glass-stack`'s touching slabs double-count Fresnel at their seams (RMSE 0.14 there); an *inside test* so a path that starts inside a dielectric (a camera or a light in the glass) is seeded with its volume; attenuation along shadow and NEE rays (ratio tracking, Novak/Selle/Jarosz 2014) instead of the boolean `jitter.w` glass-shadows switch; an ambient medium other than air ("room filled with water" is track 12); thin-wall dielectrics and an authoring shortcut for interiors; CSG, so a snow globe is modelled water-full rather than half-filled.

#### 11. Layered and exotic materials (I) — done 2026-10-31

Car paint, brushed steel, velvet, oil slicks and CD grooves now exist; Kulla–Conty puts the lost energy back into GGX lobes.

- **Nine fields appended, layout discipline:** `Material`/`PackedMaterial` grew three trailing float4s (112 B → 160 B): `coating` (clearcoat amount, coat roughness — kept perceptual and squared to alpha like `color_rough.a`, floored at 1e-3 — sheen amount, sheen roughness), `effects` (film IOR, film thickness in nm, grating period in nm, 0; 1600 nm ≈ a CD) and `weave` (anisotropy, groove rotation in degrees, 0, 0). Serde defaults keep old scene JSON byte-identical, `validate()` bounds the nine fields ("Invalid layered material"), and the Scene tab's "material layers" grid (nine rows) exposes them.
- **glTF's layer order** (base → film/grating riding the base specular → sheen → coat) and **mixture weights summing to exactly 1** in the sampler: coat `pc`, sheen `sh·(1−pc)`, base `(1−pc)(1−sh)` split by the existing `spec_probability` into cosine `·(1−ps)` and GGX `·ps`. An early draft weighted sheen with bare `sh`, summing to 1+pc·ps — a misweighted mixture; the `sh·(1−pc)` form landed everywhere, and the sampler's `sel` bounds and the `pdf` line agree with the eval term by term.
- **Transmittance pairs** attenuate deeper lobes: coat `pc` unattenuated, sheen `sh·cpair`, base `wbase·cpair·spair`, with `cpair` the (nv, nl) Fresnel pair at the coat's f0 0.04 and `spair` the pair at the sheen's f0 (`f0_of(m)`).
- **Anisotropy:** Burley's GGX slope form (`1/(π·at·ab·b²)`, `asp=√max(0.01, 1−0.9·an)`, `at=a/asp`, `ab=a·asp`, reducing term-wise to the repo's isotropic `ggx_d`/`g1` at `at=ab=a`) plus slope-space Smith `g1a`; the base GGX sampler draws the stretched hemisphere `at·r·cosφ·T + ab·r·sinφ·B + n`. The groove frame (T, B) comes from `box_map` in world space rotated by `weave.y` and is recomputed **inside both branches that need it** — deliberately never stored in `Hit` (track 10's register lesson: per-thread state is charged to every pixel, so cost is measured, not guessed). UV-tangent anisotropy stays future work.
- **Thin film** (Belcour and Barla 2017, the RGB form glTF's `KHR_materials_iridescence` adopted): per-channel cos² envelope of OPD = 2·ior·d·cosθt at λ ≈ 650/510/475 nm; the phase flips by ½ when exactly one boundary enters a denser medium (XOR of `ior>1` and `base_ior>ior`); the film rides the base specular (incidence is the macro normal's cosine), its thickness is modulated ×[0.5,1.5] by the pattern tone (marble veins read a thicker film) and its diffuse attenuates by the mean reflectance `(ior−1)/(4.08567+1.00071·ior)`.
- **Diffraction grating (CD/DVD):** the same per-channel machinery with OPD = p·(v·T − l·T); the diffuse is multiplied by the pattern's ink tone — `material_at` stashes it in `m.secondary.w` behind the portal-bit guard (portals keep the pair id there). Only the diffuse is ink-masked for now; the specular ink mask is future work.
- **Kulla–Conty MS as the roadmap's small LUT:** a static 32×32 `StructuredBuffer<float> ggx_table` (t8, root slot 11, uploaded once in `Gpu::new` from `src/ggx_e_data.rs`, generated by `tools/build_ggx_e_data.py`: a 2^19-point Hammersley lattice integrates the shader's own D·G1·G1/(4·nv·nl) lobe and stores the **complement** S(μ,α)=1−E(μ,α) bilinear over α, μ ∈ [0.02, 1]; lattice-vs-quad cross-checks agree to ~1e-5, S range 4.11e-4..0.6931). The MS term is `S(nv,a)·S(nl,a)·F_ms(f0)/(π−E_avg(a))` with `F_ms=0.04·f0+0.66·f0²+0.3·f0³` and the published E_avg quintic fit; applied to the coat lobe (f0=0.04) and — inside the layered guard — to the base lobe (anisotropic uses the geometric-mean α √(at·ab) for the lookup). Power-basis polynomial fits were tried and rejected (max error 3.9–5.5% with coefficients to ±10⁴ — fp32 cancellation at the α→0 corner).
- **bit6 guard:** `lighting::pack` sets flag bit6 when a layer **amount** (clearcoat/sheen/anisotropy/film/grating) is non-zero — the default roughnesses alone must not — and eval, sampler, guide AOV and first-hit demodulation branch on that bit test; float4 reads sit inside the branches and the off-path keeps the old text verbatim. Honest finding: `cornell-water-tank` at 256×144 4096 spp new-vs-track-10 renders with **bit-identical means and RMSE 4.1e-7** (a handful of grazing samples at 1.07e-4 abs / 1.01e-4 rel — codegen contraction from `eval_bsdf`'s new float3 argument, not semantics), but 640×360 240-frame alternating medians cost **tank +42% (5.25 vs 3.70 ms) and glass-of-water +32% (3.65 vs 2.76 ms)** on layer-free scenes: the trace's register class is set by the layered branch's live set for the whole kernel, and no guard style changed that (+28% on the first unguarded build). Getting it back needs a smaller layered branch (accumulator form) or a separate trace entry — future work; dielectrics skip the branch entirely (glass + coat stays future).
- **Checked numerically:** the design harness (`captures/track11_verify.py`, a numpy mirror of the BSDF design) renders five transient verification scenes (layer fields on, and the same scene zeroed) at 128×72 2048-frame `--no-rr --raw` and compares the centre pixel — which hits the floor exactly at the camera target — against an exact 360² quad integral of the design over the light patch: **all five PASS** — car paint (coat), brushed steel (aniso rot 90), velvet (sheen), oil slick (film 420 nm ior 1.3), CD (grating 1600 nm) — centre-pixel relative deviation ≤2.1% and on/off per-channel ratio deviation ≤1.1% (velvet's 0.009%), the ratio carrying the env/attenuation check with geometry and lighting cancelled. The film spot check: `film_env(nv=1, ior=1.3, d=420, base 1.5)` = (0.2871, 0.8159, 0.3486) matching the hand-computed 420-nm green-cyan band (0.276, 0.828, 0.366), favg(1.3) = 0.0557 → diffuse attenuation 0.9443. Harness lesson recorded: camera keys are **top level** (`yaw/pitch/distance/target/fov` — a nested `"camera"` object is silently dropped by the serde defaults), and an untextured floor's colour comes from `base`, so the tint must equal the material colour. Self-test now 459 frames (built-in pack 53).
- **Showcases/library:** builtin `layered-materials` — six spheres (deep-blue car paint under the coat, brushed steel band, velvet, oil slick, CD, film over patterned marble) on a dark stage with warm key + cool rim (`captures/layered-materials.png` RR / `.exr` raw 640×360 512-frame 4096 spp, native mean 2.117 ms, RR 2.884 ms); material library 12–16 (Car paint, Brushed steel, Velvet, Oil slick, CD grooves).
- **Not done:** coat/sheen over dielectrics (glass skips the layered branch), UV-tangent-frame anisotropy, the grating's specular ink mask, the aniso sampler's exact pdf unvalidated against a measured integral, RR `cone_spread` guide for the anisotropic lobe (heuristic `0.25·(at·ab+1)`), α<0.02 table rows clamped (smooth mirrors, MS≈0 there anyway), occupancy control of the layered branch (see cost bullet), and the carried-forward track 10 items (priority-based nested list, inside test, shadow-ray attenuation).

#### 12. Volumes (J) — done 2026-10-31

Homogeneous media inside meshes, plus global fog — one ambient medium drives fog AND underwater;
free-flight race folded ahead of the miss test in the spp loop (pbrt VolPath structure, EGSR 2012
course), volume NEE reusing the existing emitter walk.

- **Append-only layout:** `PackedMaterial` grew `volume_a`/`volume_s` (160 → 192 B; σa, σs per
  metre); `Material` gained serde-defaulted `volume_absorb`/`volume_scatter` ("Invalid media
  material" 0..1e4) with Absorb/Scatter rows in the Scene tab's material grid, and `Settings`
  gained `medium_scatter`/`medium_absorb` + `medium_ior` (1..2.5, "Invalid medium"). The cbuffer
  pair appended after `environment_map`: `medium_s` (σs rgb) and `medium_a` (σa rgb, **w =
  medium_ior**) — the track-10 dielectric sentinel pair reads it, so glass bends against the
  water it sits in and the outer medium's absorption is restored on pop; the same pair drives
  fog and underwater (one mechanism).
- **One list, two mechanisms:** media membership REUSES the track-10 interior list (`has_volume(m)`
  = any media channel non-zero, containing medium = list top, ambient = the sentinel): no second
  stack, no new per-path state. A media boundary needs `transmission > 0` to toggle membership;
  its surface stays the material's normal BSDF (frosty volume surfaces stay future).
- **Race:** `vol = sbar > 0` (sbar = max channel of cur_s): `dfree = −ln(ξ)/sbar` raced against
  the surface hit (miss = the 500 far plane); on an event `throughput ×= e^{−ext·dfree}·(cur_s/sbar)`
  (arrival ÷ sampling survival, `ext = σa+σs−sbar` the conservative-race density), NEE at the event
  point via `vol_nee` (same power-weighted emitter walk, integrand `σs·(1/2π)`, shadow factor =
  **full extinction** `e^{−(σa+σs)·len}` — nothing is sampled along the shadow march; env at the
  500 nominal reach, suns at their 50), isotropic `vol_dir` continuation (pdf 1/2π) and a rerace
  from the event point. Surfaces under media: survival `e^{−ext·h.t}`, miss factors
  `e^{−ext·500/50}` ratio form, NEE arrival `e^{−(absorption+cur_s)·len}` — all guarded by
  `med = vol || ambient active` so media-free scenes keep the old arithmetic verbatim.
- **Verified numerically** (`captures/track12_verify.py` + `captures/probe_b.py`, 128×72 2048-frame
  `--no-rr --raw` transient scenes): Test A (open floor, one 3×3 rect light at intensity **1000**,
  ambient absorption only, `mis_mode=1` + direct/indirect recompose isolating bounce-0 NEE): centre
  on/off ratio vs the analytic quad integral × `e^{−σa·dist}` (the camera segment carries extinction
  too) — **exact all channels** (mixed K 0.3/0.11/0.075 max dev 0.0001, single-channel 0.2 probes
  dev 0.0000). Harness lesson: `beauty()` scales the recompose by `composed/max(raw,1e-4)` per
  channel — raw under 1e-4 skews (pre-existing display guard; this is what made the 0.3 channel
  look ~5× low at intensity 1, root-caused by matching every probe to raw·1e4 exactly). Test B
  (glass slab ior 1.05 with `volume_absorb (1.2,0.7,0.35)` σs=0 on the camera ray only): direct
  ratio **exactly 1.0000** — the single-medium shadow-march approximation documented, no leak —
  indirect (0.8322,0.9025,0.9466) vs exact `e^{−σa·chord}` (0.8308,0.9019,0.9463) within 0.2%.
  Real bug caught en route: Constants wrote `medium_a.w = 0.` instead of the IOR → sentinel r0 =
  −1 → glass always mirrored in media scenes (tank black mirror + a bogus 2× "speedup" in the
  first chain); one-line fix, fresh-exe re-verification after. Self-test 464 frames (58 builtin)
  exit 0, rerun after the final regeneration; media-off regression pairs (tank / layered-materials
  / glass-of-water new-vs-track11) **bit-identical**, RMSE 0.0 with identical means — stronger than
  the layered guard's 4.1e-7.
- **Single-medium model (documented, closes the track-10 glass-shadow carry only for media):**
  shadow attenuation = the current containing medium's extinction × the full segment length
  (segments crossing mesh-media boundaries keep one medium's σa; Test B's exact direct 1.0 IS this
  approximation measured), env/sun miss attenuation uses nominal reaches (far plane per bounce,
  sun 50 units), overlapping media do not additively Σσ; nested concentric media (milk in glass in
  water) read the innermost through the list top. HG phase η and heterogeneous ratio tracking stay
  future (homogeneous was the roadmap's own scope line).
- **Showcases/library:** five builtin showcases — `jade-and-milk` (jade glow + milk cylinder inside
  a glass tumbler, nested media), `jack-o-lantern` (warm flesh absorption, scatter, unlit candle
  inside — carving/windows are SDF work, track 13), `vermeer-window` (sealed dark room, one window,
  0.3/m suspended scatter → soft god ray across floorboards, table + two candle emitters),
  `weather-project` (20×14×6.5 hall, mirrored ceiling slab, unlit sun disc 18 + haze + five visitors),
  `cornell-underwater` (the whole room in water: Beer–Lambert distance fade, ceiling beam, glass
  block refracting against the water via the IOR pair — light and blocks at classic sizes after the
  p6/p7 occlusion diagnostic showed the oversized block was eating the light). Library grew to 18
  (17 = Milk; entry 6 Jade carries media fields).
- **Cost:** media-free `cornell-water-tank` pair in one alternating pass (640×360, 240 frames,
  native): **5.528 new vs 5.264 track-11 = +5.0%** — an earlier pass measured the same pair 2.249
  vs 2.058 (+9.3%); GPU power state moves absolutes ~2.4× between passes, so the honest claim is the
  +5..9% band: same register-class finding as track 11 (the race's live set sets the kernel-wide
  register budget; the guards buy media-off scenes nothing). Showcase native means (final retuned):
  weather 12.9, jade-and-milk 10.6, underwater 8.0, jack-o 6.3 ms.
- **Not done:** ambient medium not exposed in the UI (scene file / `--set` only), laser beams
  (item 17 — mechanism exists, no showcase shipped), per-scene RR-vs-native denoiser tuning
  (the roadmap's own line; the RR showcase captures were visually stable at 96 frames), `vol_nee`'s
  balance partner uses the phase pdf 1/2π (heuristic; exactness only proven where events cannot
  fire), HG phase, heterogeneous ratio tracking, overlapping-media Σ, boundary emission, carving
  (track 13's SDFs), shadow attenuation across mesh-media boundaries keeps one medium's extinction
  along the whole segment (Test B measures the direct AOV ratio exactly 1.0 — the crossing-boundary
  shadow march is future work), env/sun miss attenuation uses nominal reaches (far plane / sun 50
  units) instead of true per-medium segments, volume AOV (depth of first scatter / god-ray mask —
  the research notes' open question, lands with the debug-views track), and the carried track-10
  list items (priority-based nested list, inside test, shadow-ray attenuation for plain dielectrics).
- **UI volumetrics (later implementation — owner asked 2026-10-31):** the room-wide medium has no
  editor/UI rows today: fog/underwater density, extinction colour and IOR (`Settings.medium_scatter`,
  `medium_absorb`, `medium_ior`) adjust only through scene files or `--set` (the per-material Absorb/Scatter
  sliders exist; the ambient medium does not). Plan when implemented: a Render-tab "Fog" section —
  *Density* (σs per metre, scalar with an optional tint), *Absorption colour* (σa rgb), *Medium IOR* —
  writing the three Settings fields; they already sit in `transport_key`, so slider edits restart
  accumulation cleanly, and glass-against-water bending follows automatically from the IOR pair.

#### 13. Procedural primitives and SDFs — done 2026-10-31

Needs instancing (core 4): procedural AABBs go in their own BLAS. As built: an
`Object.procedural` shape spec (≤256 chars, parse-checked by `validate()` and in
`geometry_key`): `sphere/0.9`, `torus/0.55/0.24`, `cylinder/0.34/0.95`, `box`,
`bulb/8.5`, `menger/4`, `blob/11` — up to three atoms joined by `+` union, `-`
subtract and `*` intersect, each join with an optional `@k` smooth radius after the
atom's args (`menger/4-sphere/0.75/0/0.35/0@0.18`). `src/sdf.rs` parses, clamps and
bounds the chain and evaluates it on the CPU (picking, grounding, outlines); every
distinct spec gets its own single-AABB `PROCEDURAL_PRIMITIVE_AABBS` BLAS (cached beside
the mesh BLASes in `rebuild_scene`), instanced always FORCE_NON_OPAQUE — DXR auto-commits
the first candidate without running the loop, and the march needs the test — with `Instance.procedural`
carrying the record slot (+1), read from a new root SRV t9 (36 floats per slot: three
atoms as `(kind, p0..p2)`, `(p3..p5, op)`, `(k / safety, budget)`). Only the `trace_proc`
and `trace_maps_proc` PSO variants (`-D PROCEDURALS` in build.rs and hotreload.rs)
carry the march, so the plain variants keep their measured register budget. In the
candidate loop the shader marches the candidate ray in object space (the transform is
affine and the chain fits its AABB, so world t is the shared parameter; stepping is
`sd·safety/|od|` — 0.75 for smooth chains, 0.55 for bulbs). The bulb's DE needs a bounding-ball
branch outside |q| = 1.48 (its box is ±1.5, not ±1: the bailout ball sticks out of the unit box)
and divides the escape radius by the accumulated orbit derivative — the naive `(r−2)/(pw·r^(pw−1))`
collapses to ~1e-8 outside and the march stalls a few steps past the box) — commits with
`CommitProceduralPrimitiveHit(t)`, and reconstructs normals as a central-difference
gradient of the whole chain at the hit (analytic for spheres/tori/cylinders, fractals
included). CPU `geometry.rs::pick_object_hit` marches the same grammar so editor picks
agree. Four showcase scenes in the built-in pack (62 files now): `procedural-studio`
(exact chrome ball, torus bagel minus its axis cylinder, capped cylinder with a ball
finial), `golden-bulb`, `lava-lamp` (blob inside a hollow glass cylinder shell) and
`sponge-hall` (level-4 sponges, one smooth-cut by a sphere). Self-test frames 289–292
(bagel pick, smooth subtraction, bulb, blob + drop-back). "Menger at any level" is the
SDF loop (grammar 1–5), not voxel soup; the `gen:` marching-cubes meshes stay for
scenes that need real triangles.

- Not done: an emissive procedural surface is not in the NEE triangle table (`h.emitter`
  is 0 — it lights the path from BSDF hits only, so the lava lamp needs a helper light);
  the alpha-cutout map test is skipped for procedural candidates (committed maps are
  box-mapped when the material has a texture set, plain albedo otherwise); no per-parameter
  sliders in the inspector yet (the spec is a string); the selection outline is the
  chain AABB through the transform (loose for fractals); chains cap at three atoms;
  media membership against procedural glass is untested; dual-number/analytic fractal
  normals (gradient steps are good enough at 400–600 px); curved-space or non-affine
  instance transforms would break the object-space march by design.

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
  - **Owner additions (2026-10-30):**
    - **Stacking generators:** new generator families that build stable structures to knock down: brick walls, block pyramids, Jenga towers, card houses, domino spirals, a castle of mixed shapes, ball pits. They emit scene files like the other families, with a `physics` flag so they open with the simulation ready (sleeping bodies).
    - **Tools:** grab and drag (a spring joint to the cursor, like the Half-Life gravity gun), throw, an explosion (radial impulse at the clicked point, with an optional flash light), wind (a directional force field with gusts; also bends cloth), gravity strength and direction, slow motion and pause, spawn at the cursor, freeze / unfreeze a body.
    - **NVIDIA demo style showcases:** PhysX / Flex-type scenes such as a tower collapse, a wrecking ball, a chain or rope bridge, cloth draped over objects (cloth needs `rapier` plus a simple mass-spring or XPBD solver of our own), and a pile of thousands of small rigid bodies (needs `forest`-scale instancing, already 4096 objects).
    - Recording: the timeline of track 8 bakes a simulation into keyframes so a video export (item 6) can render it at full quality.
    - Dynamic convex hulls for imported meshes (rapier's `convex_decomposition`), boxes and spheres for the built-in shapes.
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

**Linux port notes** (collected while building Windows-only pieces; check each when porting):
- **ffmpeg:** Windows gets a pinned gyan.dev build through `tools/setup.ps1`, copied next to the exe. On Linux use the distro's `ffmpeg` from PATH (`video::ffmpeg()` already falls back to PATH); drop the `.exe` and the `CREATE_NO_WINDOW` creation flag (`std::os::windows::process::CommandExt`, also used in `hotreload.rs`, `scenes::render_thumbnails`).
- **Shaders:** `hotreload.rs` and build.rs call the Windows SDK's `dxc.exe`; on Linux use the DirectXShaderCompiler release binary (and SPIR-V output for Vulkan).
- **Windows-only modules:** `saver.rs` (screensaver switches and the GDI preview), `display.rs` (DXGI output description and `DisplayConfig` SDR white level; on Linux HDR comes from Wayland's colour-management protocol or not at all), the stats path (`%APPDATA%`; use `$XDG_DATA_HOME`).
- **Upscalers:** DLSS (NGX) and XeSS have Linux/Vulkan libraries; FSR through the FidelityFX API's Vulkan backend. The loaders in `bridge/` use `LoadLibrary`; use `dlopen`.
- **Gamepad, remote, scenes:** gilrs, the std HTTP remote and the scene files are portable as they are.

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

### Accuracy mode and reference renderer (owner request, 2026-10-30)

"Even more accurate rendering, with a software mode if it can't be done in real time." This groups the accuracy work into one goal, which the CPU reference above becomes part of:

1. **Audit the GPU path tracer's shortcuts** and list each with its error, measured against a high-spp reference: rough glass as a lerp toward a cosine sample (not a real rough dielectric BTDF with GGX microfacets, Walter 2007); Lambert + GGX without energy compensation (rough metals lose energy; add Kulla-Conty multiscatter); Russian roulette after bounce 3 and the 6-bounce default; emission flat on textured meshes (core 5 not-done); the ray-cone mip heuristic; RGB instead of spectral. Fix the ones that are cheap in real time.
2. **An "Accurate" quality bundle** (GPU, progressive, not real time): unlimited bounces up to 64 with roulette only on throughput, no clamps, full-resolution textures (LOD 0), the exact BTDFs from step 1, and no denoiser. It is the GPU's best image and the default for high-res stills.
3. **Software mode:** the CPU reference renderer above, plus spectral (below), runs where the GPU can't: as an offline renderer from the UI ("Render reference…") and the CLI, with progress, and as the oracle for the GPU's correctness. WARP covers machines without DXR.
- Measure each step with the convergence graph (5b) and RMSE against the CPU reference. Large; do steps 1–2 first.

### Other denoisers (owner request, low priority, 2026-10-30)

A better denoiser than DLSS RR, for both stills and interactive use:

- **Intel Open Image Denoise 2** (OIDN): the best offline quality for final stills, with albedo and normal AOVs (which we already write). It runs on the CPU or on NVIDIA GPUs through CUDA; share the textures with D3D12 through `ID3D12Device::CreateSharedHandle` and CUDA external memory. A "Denoise the still" option on high-res exports and the video export.
- **NVIDIA OptiX denoiser:** similar quality, also CUDA, has a temporal mode for video.
- **NVIDIA NRD** (ReBLUR / ReLAX, real time): replaces or sits beside the native SVGF filter, and fixes its low-frequency blotches at low spp (known issue). Needs separate diffuse and specular radiance with hit distances, which the AOVs nearly provide.
- Compare each against RR and the native filter with `tools/motion_error.py` and the convergence graph. Medium to large.

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

- **Baker:** bake path-traced GI into UV lightmaps and export glTF with the lightmaps, for use in game engines. Mesh UVs and textures exist since core 5; it still needs a second, non-overlapping UV set.
- **Texture painting:** paint directly on surfaces in the viewport. Strokes are projected through the object-id and UV buffers, and the render updates live.
  - Until textures exist, "paint mode" just recolors the clicked object's material, an S-sized quick win on top of picking.
