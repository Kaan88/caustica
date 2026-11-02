# Caustica roadmap

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
- **Ray Reconstruction:** keep the current Default, D, E and F list, and check the SDK headers for newer RR presets when updating the SDK. (Checked 2026-11-02 against the vendored `nvsdk_ngx_defs_dlssd.h`: G–O all revert to default, so nothing new.)
- **Implementation:**
  - New `Settings.dlss_preset: u32` (0, 10–13).
  - In `bridge/ngx.cpp` `caustica_ngx_create_sr`, set `NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_{DLAA,Quality,Balanced,Performance,UltraPerformance,UltraQuality}` on `sr_params` before creating the feature, the same way RR sets its hints.
  - Add it to the `main.rs` reconfigure trigger list, and add a combo next to the Upscaler choice, shown when DLSS SR is selected.
  - Self-test: cycle the presets once.

### 2. Native HDR output, with settings taken from Windows — done 2026-10-30

As built: scRGB swapchain, BT.2390 roll-off on the max channel, Auto/SDR/forced modes, Windows peak and SDR white with manual overrides, egui at paper white, and HDR PNG (PQ / BT.2020, cICP). Added 2026-11-02: the HDR10 swapchain (Display → *Swapchain*, `hdr10`: R10G10B10A2, PQ / BT.2020, falls back to scRGB when refused) and HDR JPEG XR screenshots (scRGB half floats, lossless, through WIC; `--capture *.jxr`). Still open: egui blends on PQ code values in HDR10 (anti-aliased text edges are slightly off); no mastering metadata (`SetHDRMetaData`) is sent in either mode. Details are in docs/engine-notes.md, "Native HDR".

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

Not done from phase 1's list (they belong to ui-plan.md phases 2–5, core track 6): Quality bundles with a `Capabilities` struct, per-pass GPU timing in the status bar, persisted UI prefs (advanced toggle, UI size), modified-row dots, file pickers for settings / LUT / reference EXR (meshes and HDRIs have one), and async high-res export (it blocks the UI today). Done since: undo / redo, viewport handles and hover (3b), the scene browser with thumbnails (4), Quality bundles, GPU timing, prefs and the async still (core 6). Done 2026-11-02: `gpu::Capabilities` (per backend: available, or the reason: not an NVIDIA GPU, NGX error code, missing DLL, failed while rendering) behind the RR checkbox, the Upscaler combo and the Quality bundles; modified-row dots on every slider (`crate::slider`: a dot when the value differs from the scene as loaded, click to revert); *Browse…* for the LUT and the reference EXR, and *Save settings as…* / *Load settings…* beside the quick settings.json buttons. Still open: dots only on sliders (checkboxes and combos have none); the status bar does not turn amber with the fallback when a backend fails mid-session (the Render tab says so).

### 3b. Blender-style transform gizmos (owner request, 2026-10-30) — done 2026-10-30

As built: move / rotate / scale gizmos for objects, area lights and the main light (no main-light rotation, since it is always horizontal), G / R / S modal transforms with X / Y / Z, Shift+axis planes, typed values, Ctrl snap, Shift fine and cancel, Global / Local orientation, light outlines and light picking, undo / redo. Drag cost is in docs/engine-notes.md, "Gizmos".

**Not done yet (owner asked to keep this list, 2026-10-30).** Done 2026-11-02, see engine-notes "Multi-select, pivots and snapping":
- ~~**Snapping to other objects**~~: a *Snap* combo in the top bar (Increment / Surface / Vertex / Edge) for Ctrl during a move; surface snapping rests the moved group on the surface along its normal. Still open: no rotation aligned to the surface normal; no snapping of rotate or scale to targets; no edge-centre or volume targets; lights are not snap targets; the CPU ray cast tests every triangle of every object each pointer move (not measured on 20M-triangle files).
- ~~**Pivot options**~~: median point, active element, individual origins and the 3D cursor (Shift+right-click places it on the surface under the pointer). Still open: the cursor shows only while it is the pivot; no "cursor to selected" / "selection to cursor".
- ~~**Multi-select**~~: Shift+click (viewport and outliner), box select (B, or a drag with the Select tool; Shift adds), Ctrl+A / Alt+A (A stays auto orbit), and G / R / S, gizmos, duplicate, hide and delete on the whole selection. Still open: box select tests each item's origin, not its outline; the inspector edits the active item only; the timeline keys (I) and physics grabs use the active item only; lights get no hover outline.
- ~~**Alt-drag duplicate**~~: Alt+drag on a gizmo handle copies the selection in place and drags the copies (Shift+D already duplicated then moved); cancelling either removes the copies. Still open (review 2026-11-02): with physics on, a gizmo transform pauses the simulation (fixed: holding an arrow still used to drop the object), but a mouse grab active when G starts stays pending, and the first transform after enabling physics adds an extra undo step; self-test 237–238 relies on wall-clock frame time to show the old drop; the snap ray cast rebuilds every object's triangles on each pointer move; HDR10 once refused stays refused until a restart (no retry after a display change), and the R10G10B10A2 typed UAV store is not checked with CheckFeatureSupport.
- ~~**Smaller gaps**~~: a free or plane move takes a typed vector (1 Tab 2 Tab 3, Blender style).
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

As built: views 8–11 (bounces, BVH cost, variance heat maps, false colour), zebras, scopes binned with atomics in the display pass (no extra pass; cleared by a copy), light meter (one-texel readback of linear output and albedo), convergence graph in egui (reference from an EXR or the current image, log–log with a 1/√n guide), lifetime stats and 8 achievements in `%APPDATA%\Caustica\stats.json`.

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

As built: thin lens with round or 3–12-blade apertures, blade rotation, cat-eye (blocked lens samples contribute nothing, so corners also darken), Ctrl+click focus with an eased pull; camera models orthographic, 360° equirect, little planet and tilt-shift (non-perspective models switch RR, temporal upscalers, picking and the gizmo off); tone maps AgX (Wrensch's polynomial fit) and **Khronos PBR Neutral instead of Tony McMapface** (Tony needs its 48³ LUT; load it as a .cube through the LUT slot instead); halation, grain (three sizes), vignette, lift / gamma / gain wheels and a .cube LUT loader. Not done: grain matched to named film stocks, and equirect metadata for VR viewers. Both done 2026-11-02: a *Film stock* combo (Look → Film) with seven stocks setting grain, grain size and halation (matched by eye; the grain is still monochrome and no stock's colour response is modelled; true per-stock grain needs colour grain and measured spectra), and GPano XMP on PNG and JPEG saves while the camera is 360° equirect (quick saves, the still, image sequences, `--capture` and script captures; not UI captures). Still open: spherical metadata in equirect videos (ffmpeg's mov muxer writes `sv3d` only from stream side data, not from a CLI flag; Google's spatial-media tool can inject it afterwards); `--capture` and script captures still take no JPEG.

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
   - Coplanar faces (a glass object's base on the floor) z-fight differently than before; see docs/engine-notes.md "Instancing".
5. Textures. — done 2026-10-30: imported meshes sample their base colour, normal and metallic-roughness maps (UVs in the vertex buffer, a bindless table of up to 4096 maps, mipmaps chosen by ray cones, alpha MASK cutouts in the any-hit loops), and any material (shapes, meshes, floor, walls) can take a box-mapped texture set from an ambientCG or Poly Haven folder. Material values scale the maps (value / average), so old scenes and the sliders keep working. Untextured scenes run a shader variant without the map code (unchanged speed); the textured chess set costs 16% more than its flat averages did. 8 CC0 sets in `tools/fetch_assets.ps1` and a `scenes/materials` pack.
   - Not done: emissive maps (emission stays the average colour: light sampling has no texture lookup; would need per-emitter-triangle UVs); glTF `KHR_texture_transform`, TEXCOORD_1, stored tangents (per-triangle tangents from UV derivatives instead of MikkTSpace), occlusion maps (a path tracer does not need them), clearcoat / sheen maps; BLEND alpha is treated as MASK at 0.5 (no stochastic transparency); texture sets ignore Opacity, AO and Displacement maps.
   - Not done: box mapping has hard seams on curved shapes (a blended triplanar would need three lookups); a texture set on a mesh is box-mapped too (no option to use the mesh's UVs); built-in shapes have no UVs.
   - Not done: maps are uncompressed RGBA8 (a 4k map is 85 MB with mips; BC7 would be a quarter); sRGB maps are mip-filtered in sRGB; the ray-cone widening for rough bounces is a heuristic (0.5·α or 0.5 per bounce), not the lobe's real spread; the alpha test samples LOD 0.
   - Not done: the material editor picks a file (no folder picker, no previews); downloading sets from inside the app belongs to the asset browser (item 6); texture painting and the lightmap baker (feature tracks) can build on the UVs now.
6. UI phases 2–5. — done 2026-10-30 (phase 5 in part). Phase 2: right-click menus (viewport and outliner), Ctrl+D / Shift+D duplicate, H / Alt+H hide, Home, Page Up / Down, F frames the selection, double-click frames, middle-drag pan, an F1 shortcut sheet, selection and hover outlines. Phases 3–5: favourite and recent scenes, Ctrl+S / Ctrl+Shift+S / Ctrl+O, `--preset` names, persisted preferences (UI size, advanced, first-run hint), Quality bundles in the top bar, per-pass GPU time and time-to-cap in the status bar, the high-resolution still as a non-blocking job up to 8192 (PNG or EXR), Show in folder toasts, a draggable A/B split, a wrapping top bar.
   - Not done: Alt-drag duplicate (Shift+D instead); outlines for lights and silhouette outlines from the id buffer (outlines are projected boxes and vanish while a corner is behind the camera); the preset registry refactor (slugs only); ~~a capabilities struct with per-backend reasons~~ (done 2026-11-02, item 3); the "what the denoiser is doing" line; export file name templates; lens shift to centre the subject in the free viewport (touches the camera and RR, needs `tools/rr_stability.py`); material ball and light thumbnails.

### Known issues (not scheduled)

- **Owner notes 2026-11-02 (to review after track 17):**
  - The stained-glass / transparency scene: BDPT with the physical sky may not work. The glass shadow also changes the colour of plain path tracing in this scene.
  - Stained glass shows little colour with path tracing (maybe a limitation: coloured caustics through glass are hard for a path tracer). It works well with photon mapping, and better with BDPT, but BDPT fails with the physical sky.
  - *Checked 2026-11-02, not reproduced:* with the physical sky on (10:00, and the sun alone at 07:00 and 17:00), path tracing and BDPT agree headless (mean colour within 0.2%, 480×270, 800 spp, raw), and with RR both look the same. The coloured light on the floor is a caustic of a small, distant light through glass: path tracing's shadow rays pass glass untinted (`glass_shadow` tints them, an approximation), so only BSDF paths that happen to hit the sun carry it; that is a known path-tracing limit, and BDPT / photon mapping are the right tools. Next: a viewer repro (camera, time of day, integrator, denoiser) from the owner.
  - Done 2026-11-02: the Newton's cradle scene: give it real physics (track 17 sandbox), by changing the same scene.
  - The GPU shows full load but low power draw. Find out whether that is normal (latency or occupancy bound, the CPU waiting on the fence with one frame in flight, small dispatches) or a performance problem.
  - *Measured 2026-11-02 (the earlier "GPU idles" lead was wrong: those runs had vsync on):* RENDER_OK now prints `fence_wait_ms_mean`. With `--no-vsync` the CPU waits on the fence for 82–96% of each frame (stained-glass-hall: 480×270 1.24 of 1.50 ms, 1080p 15.3 of 15.8 ms, 1080p RR 12.5 of 13.2 ms), so the CPU is not the bottleneck and a second frame in flight would gain at most a few percent. Under load (1080p, 4 spp, no RR) the RTX 4070 Ti SUPER draws 169 W of its 285 W limit at 98% utilisation and 2850 MHz, P2, no power, thermal or clock limits active. So the GPU is busy but the work is latency-bound: incoherent rays and scattered memory reads in one large compute kernel (`trace`, inline RayQuery, 8×8 groups), where warps wait on BVH traversal and memory rather than doing arithmetic. That is normal for path tracing (games' path tracers go higher partly through shader execution reordering). Options, none tried: (1) profile with Nsight Graphics GPU Trace for occupancy, registers per thread and the stall reasons, before changing anything; (2) cut register pressure (split the uber-kernel: native-only and photon / BDPT paths compiled out of the plain path-tracing variant, as `trace` already does for texture maps); (3) wavefront path tracing (one kernel per bounce stage with queues sorted by material); (4) move tracing to a DXR pipeline with shader execution reordering (SM 6.9 `MaybeReorderThread`, Ada supports it; inline RayQuery in compute cannot reorder), the largest change and the largest likely gain (commonly 20–40% on divergent paths).
  - *Audio, fixed 2026-11-02:* "Audio capture stopped (output device changed?)" right after switching audio on. cpal reports a default-device change notification as a stream error but keeps the stream running, and some drivers and audio tools send that notification as soon as a capture opens; every error switched audio off. Now the capture is reopened on the current default device (at most once a second) and only a failed reopen is an error.
  - Done 2026-11-02: physics objects went to sleep too easily (raised rapier's sleep thresholds, see track 17). Still open from this note: the laser as a physical beam visible only in fog or dust (the *Beam in image* rods glow by themselves).
  - Done 2026-11-02: volumes in BDPT and photon mapping, and the material cases the sweep found (engine-notes "Media in BDPT and photon mapping").
  - *Sweep 2026-11-02* (`tools/integrator_sweep.py`: every built-in scene, 240×135, 400 spp raw, mean colour against path tracing). Before the media work 35 of 160 comparisons were off by more than 5%, after it 13 (numbers per scene in engine-notes "Media in BDPT and photon mapping"). Fixed: all media scenes, birthday-cake (emitters kept photons), turrell, light-temperature-row, mirror-room, shape-material-matrix, the glass scenes. Still open:
    - optics-bench 77% (BDPT and photon mapping agree with each other: path tracing converges slowly on the L S+ D caustic of the hidden 3° spot). Confirm with a long path-traced reference.
    - sky-garden-dusk 11–12% and sunlit-room-day-cycle 10% darker in both, physical sky at low sun. Noon matches (0.0%). Lead: path tracing itself disagrees at dusk (MIS 0.0987, light sampling only 0.1009, BSDF sampling only 0.0859 in red), and the mean changes with `sun_size` (0.080 vs 0.076 at 4×) although the sun's irradiance should not: the sun / sky split between the env map and the sun's sampling looks inconsistent at low sun.
    - kusama-infinity-room photon 13% darker (BDPT matches): likely the gather radius's boundary bias at the mirror walls; an adaptive radius would show it.
    - lava-lamp photon 27% (procedural SDF emitters emit no photons), bigger-on-the-inside 8% (portals unsupported in both).
    - cornell-caustic BDPT 5.1% and veach-ajar photon 5.0%: at the threshold, probably noise at 400 spp.

- *Fixed 2026-11-02:* `--self-test` failed once with "The timeline export did not finish": the export's encoder thread can lag the two frames the check allowed. The self-test now waits on its export checks (frames 145, 149, 276) while an export still runs, up to 600 frames.

- **Native denoiser:** faint low-frequency blotches on flat walls at very low spp, typical of SVGF (found in the 2026-10-29 review, not started).
- **Gamepad:** untested, no controller here (5d).
- **Reported 2026-10-31, fixed 2026-11-01:**
  - *Accumulation stalls with a render scale below 1:* by design a temporal upscaler got fresh 4 spp every frame, so the count never grew and noise was capped by the upscaler's short history. Now, after 32 still frames, the samples accumulate to the cap and the upscaler gets zero jitter. Measured at 0.5 scale, 256 spp against a 4096 spp reference: raw RMSE×1000 17.7 → 10.9; denoised 5×5-blurred error unchanged (5.2) but plain error 9.1 → 11.9, from edges: a static input loses the upscaler's sub-pixel edge detail. Not done: keep that detail (e.g. accumulate per jitter phase, or blend the fresh-phase output at edges).
  - *Layer sweeps showed no difference:* the values do reach the shader; the dark surround (sky 0.05, one thin light) left the coat only a small highlight. The scene now uses sky 0.5 and a square light.
  - *Switching scenes changed quality:* `denoise` was not kept by `Settings::keep_output` or `preset()`; now it is.
  - *Walk mode mouse look:* yaw sign flipped in `Walk::look`.
- **Hidden lights occlude:** see core 3 above.
- **Light sampling missed ~0.5%, fixed 2026-11-02:** NEE shadow rays ended a fixed 0.6 mm short of the light along the ray, so at grazing angles they still hit the emitter's own face (the lead from BDPT's `connected()`). `nee_limit` now stops 0.6 mm off the surface along its normal. night-skyline at 480×270, 4096 spp: light sampling only vs MIS −0.57% → +0.06%. (Found 2026-11-01 with ReSTIR: emissive buildings lit by their neighbours.)
- **BSDF-only vs MIS offset, fixed 2026-11-02 by the same change:** on the tea set, BSDF sampling only was 1.6% brighter than MIS; after `nee_limit` they agree to 0.005% (320×240, 8192 spp): MIS was the side losing light.
- **Ultrareview of track 15 part 2 (2026-11-02, owner: fix later):**
  - *Fixed 2026-11-02:* both confirmed items (the battle timestamps are read before the Unmap, video exports clear `battle`); `preset()` keeps `battle`; the CAS loop retries 1024 times; ReSTIR reservoirs only for path tracing or battle; a photon cell chain longer than the 4096 walked is counted and the walked part scaled up; the battle graph and the normal convergence graph share `error_sums`. The splat zeroing through an upload buffer stays: it runs only when the buffer is (re)allocated, not per frame. Ultrareview closed.
  - *Confirmed:* `Gpu` battle mode reads the timestamp slice after `self.timing.Unmap` (move the per-quadrant loop before the Unmap); `video::Job::start` does not clear `battle`, so an export with battle mode on renders four-integrator quadrants and takes several times longer (set `v.battle = false` next to `v.compare`).
  - *Plausible, unchecked:* `photon_gather` walks at most 4096 photons per cell but divides by the full photon count, so large gather radii or photon counts come out dark (count per cell and rescale, or bound chain length); `preset()` drops `battle` while `keep_output` keeps it; BDPT's `atomic_add_float` gives up after 64 compare-exchange attempts, losing splats on hot pixels; `prepare_restir` allocates reservoirs for BDPT / photon mapping, which never read them (gate on `integrator == 0 || battle`).
  - *Cleanups:* `prepare_aux` zeroes the splats with a CPU vector and upload buffer (use a GPU clear); the battle convergence graph duplicates the RMSE code of the normal graph.

### Feature tracks

These follow the core track. Each lists what it needs; within those limits the order is a suggestion. The letters refer to the engine features in scene-research.md section 4. Items 7–12 schedule the ones the core track left out; the rest of the features depend on them.

#### 7. Analytic lights (F) — done 2026-10-30

As built: extra lights have a *Kind*: rectangle, disk, sphere, spot (a disk with a cone and a soft edge) and sun (angular diameter, 0.53° by default). Spheres and suns are sampled uniformly over the solid angle they cover, disks and spots by area; all share the power-weighted emitter table with MIS, and were checked unbiased (MIS, NEE-only and BSDF-only agree). Spheres, disks and spots are real geometry (picking, RR guides, reflections); suns are found on misses. See docs/engine-notes.md "Analytic lights".

- Not done: glTF `KHR_lights_punctual` import (mesh import ignores lights); true delta point and directional lights (by design: the smallest sphere is 0.05 across, the smallest sun 0.05°); IES profiles, gobos / cookies and light linking; the main ceiling light stays a rectangle; a sun is not split out of an HDRI automatically (core 3 not-done); a sphere's camera and BSDF hits see its 64 × 32 inscribed mesh while light sampling sees the true sphere (under 0.5% of its solid angle, at the silhouette); a sun's weight in the light-selection CDF uses a nominal 50-unit receiving area, so a sun beside very bright small lights can be under-sampled; suns are tested per miss in a loop (fine for a few, not for 32); spheres and suns do not get a shape-specific gizmo (spheres scale their diameter, suns ignore size); no scene files use the new kinds yet; flashlight mode (below) is not built.

- Unlocks **flashlight mode**: a spotlight attached to the camera. With walk mode (5d) and a dark Sponza, it's an instant horror game.

#### 8. Animation and timeline (K) — done 2026-10-31

As built: keyframes on any float in the scene (camera, object transforms, lights, materials, exposure, sky, HDRI rotation) as tracks on JSON paths, stored in the scene file; smooth (monotone cubic, no overshoot), linear, ease and hold per key; a timeline strip (play / scrub / step, the selection's keys, interpolation, length, fps, shutter); I keys the selection or the camera, Shift+I every edited value, Alt+I deletes; the video dialog's *Timeline* motion (and `--turntable-timeline`) renders the animation, with motion blur from a shutter angle (one moment per pass, TLAS / materials rebuilt, accumulation kept, RR off); `scenes/builtin/pool-break-1984` as the showcase. See docs/engine-notes.md "Animation and timeline".

- Not done: a curve editor (keys are edited by value at the playhead only, no tangent handles or dragging keys in time); keys on booleans and choices (light on / off, shapes, tone map; flickering candles need a hold-keyed intensity instead); per-object timing tools (offset, scale, copy keys); camera paths along a spline with look-at targets; recording a walk or a gizmo drag into keys (auto-key); Shift+I with no keys yet keys nothing (an edit counts only against the last time the animation applied); keyed playback serializes the whole scene each frame (fine for hundreds of objects, not measured at 4096); motion blur only in exports (the viewport plays sharp) and only on the non-RR path; each pass is one moment, so deformation and HDRI-rotation blur come only from many passes; audio; Newton's cradle and day-cycle time-lapses (track 9) are not built as scenes yet.


- **Keyframes:** camera, object transforms, light intensity and color, and material parameters. Use a timeline strip in the UI, with linear, smooth and hold interpolation. Keyframes are stored in the scene file.
- **Video export:** step the timeline per frame, render N spp, and pipe the result to ffmpeg (5d). The export dialog itself (resolution, fps, codecs, progress, HDR) comes first, as item 6.
- **Motion blur:** shutter open/close settings.
  - DXR has no motion BVH. Pick one shutter time per spp batch, set the instance transforms for that time and rebuild the TLAS.
  - This is cheap only after instancing (core 4), where the TLAS is rebuilt and the BLAS stays.
  - Recreate Cook's 1984 pool-ball scene as the showcase.
- **Unlocks:** Newton's cradle, flickering candles, day-cycle time-lapses (item 9) and turntables with keyframed lights.

#### 9. Physical sky and day cycle — done 2026-10-31

As built: *Physical sky* in the Stage section (time of day, date, latitude, north, turbidity, sun intensity and size, ground albedo, seen by camera, *Key a day*). Instead of Hosek–Wilkie (fitted coefficient tables, weak at twilight) the sky is a ray-marched Earth atmosphere after Bruneton / Hillaire: Rayleigh, Mie and ozone, a transmittance table, single scattering plus Hillaire's 2020 isotropic multiple-scattering term, earth shadow, a ground below the horizon lit by sun and sky. It is baked on the CPU (1024 × 512, all cores) into the environment map and reuses the HDRI path (CDFs, MIS); the sun is not in the map but an analytic sun light (track 7) coloured by the atmosphere's transmittance, so its shadows stay sharp. Sun and sky share one unit (the sun above the atmosphere lights a white surface to *Sun intensity*). Every field is keyable (track 8), timeline exports re-bake per moment; `scenes/builtin` gained *Sunlit room · day cycle* (a keyed 06:30–19:30 time-lapse through an east window) and *Sky garden at dusk*. See docs/engine-notes.md "Physical sky".

- Done 2026-11-02: stars (a hashed field above the horizon, dimmed by the air), the moon (phase from the date, a disc in the map, and after astronomical twilight the sky is the moonlit one), clock time (longitude, time zone and the equation of time), the observer's altitude (0–20 km), and the bake on a worker thread with a cache of the last 12 maps (windowed runs; headless and exports bake in place), so dragging the time no longer stalls the window. The constant night floor dropped 100× so the night sky shows. **Owner, 2026-11-02: revisit the night sky later; the owner does not like the current implementation** (stars and moon are physically scaled, so they need Exposure 10 plus Sun intensity 1000 to show; Exposure tops out at 10×; no moon-position readout). See engine-notes "Physical sky: night, clock time, altitude". Not done: clouds; aerial perspective on scene geometry (volumes, track 12); single wavelength-triplet RGB, not spectral; the moon's declination is the sun's (real orbit tilt ±5° ignored) and it has no surface texture or earthshine; stars are one map texel each (blocky when zoomed, and the debug build's half-size map shows larger ones); the ozone and aerosol profiles are fixed; timeline exports still bake per moment (cached when moments repeat); the first look at a new time still waits for its bake (the previous sky stays on screen meanwhile).

Original plan:

- **Sky model:** Hosek–Wilkie, or the Prague model for more accurate sunsets. Bake it into the environment map on change and reuse the HDRI CDF path.
- **Controls:** time of day, latitude and turbidity. With item 8, a time-lapse through a window.

#### 10. Nested dielectrics (H) — done 2026-10-31

As built: priority-based nesting after Schmidt and Budge (2002). Each path carries a stack of up to 4 transmissive media in one 64-bit word; a material's *Priority* (0–7, advanced material rows) decides who owns an overlap, the latest entered wins ties, and a surface that does not bound the current medium is a false hit (passed straight through, no bounce, MIS state kept). Refraction uses the IORs on both sides (water–glass 1.33 / 1.52, ice–water, an air bubble's 1.0 in water with its total internal reflection), absorption follows the current medium, and dielectrics switched from Schlick to the exact Fresnel term. `gen:glass/<seed>` lathes a tumbler and its water (reaching into the glass wall) as two parts; `scenes/builtin/glass-of-water` adds ice (3), air bubbles (4), a straw and bars behind. Three more scenes (2026-10-31) use priorities as constructive solid geometry, a solid glass shape hollowed by a higher-ranked liquid or air shape: `layered-cocktail` (three liquids and an air headspace in a highball, whisky over an ice ball), `paperweights` (coloured glass of a higher index and bubbles in clear glass) and `aquarium` (a glass box hollowed by water and air, with fish, plants, gravel, marbles and bubbles). Checked: an inner box of the water's IOR ranked above it, and an IOR-1.5 box ranked below it, both vanish to noise level (RMSE×1000 7.8 and 6.4 against a 16k reference; the same box at equal priority: 23.4). The Fresnel switch moves old glass scenes slightly (glass-stack: RMSE×1000 15.4 between old and new, mean luminance +0.2%). Cost (release, 1440×900, GPU ms, two runs each, old → new): with RR, glass-stack 5.11 → 5.31, cornell-caustic 5.20 → 5.38, gem-display 4.75 → 4.73, veach-mis (no glass) 4.10 → 4.14; native denoiser, glass-stack 5.59 → 6.12, cornell-caustic 4.87 → 5.64, gem-display 4.93 → 5.39, veach-mis 3.85 → 4.01; `glass-of-water` itself 11.1 RR / 13.1 native. A first version (a `uint4` stack and a `continue` of the bounce loop) cost glass scenes about 1 ms with RR as well. See docs/engine-notes.md, "Nested dielectrics".

- Not done: the native path still pays 10–15% on glass scenes (0.5–0.8 ms; RR is within noise), probably register pressure from the stack across the native-only code; the camera always starts in air (no camera medium, so a camera inside the water tank or an aquarium sees through the wrong IOR; underwater mode belongs to track 12); entering vs leaving is still decided from the interpolated shading normal, as before, but a misclassified grazing hit on a thin smooth part now leaves a wrong medium for the rest of the path (the geometric normal would be sturdier; needs the triangle's vertices in `intersect`); a material with partial transmission counts as a medium for false hits on every hit, not only when its dielectric lobe is chosen; NEE shadow rays still pass glass without refraction or absorption (no caustics through NEE, as before); stacks deeper than 4 drop the push; the snow globe is not built, and the aquarium's water is clear (murky water wants track 12) and its surface flat (no ripples, so no caustics on the gravel); a flat-bottomed solid glass resting on the floor shows the floor under it fully lit, because shadow rays pass glass (small under the glasses and bottles of the new scenes; the millefiori paperweight became a low dome because a wide disc was dominated by it); no glTF `KHR_materials_volume` thickness or attenuation import (attenuation colour would map to absorption).

Original plan: unlocks the glass of water, ice in a drink, the **snow globe** and the **aquarium** (with item 12 for murky water).

#### 11. Layered and exotic materials (I) — done: clear coat, sheen, energy compensation and anisotropy 2026-10-31; thin film and diffraction grating 2026-11-01

As built (part 1): glTF's clear coat and sheen layers. A coat (GGX, IOR 1.5, its own roughness, its own sampled lobe) over a Charlie sheen (Estevez–Kulla visibility) over the base; each layer dims what lies below by the energy it takes from the view direction, with the sheen's albedo in a generated constant table (`tools/fit_layers.py`). Material sliders, glTF import of both extensions' factors, and `scenes/builtin/layered-materials`; later the same day `layer-sweeps` (one parameter per row, 0 to 1), `layered-furnace` (the white furnace with coats and sheens), `velvet-lounge` (velvet sofa, lacquer table, coated floor) and `perfume-vanity` (nested perfume bottles with a sheen atomiser and curtain, coated pearls and box). Checked in the white furnace (every layered sphere matches the bare white base, 0.95–0.98, none above 1) and with MIS against light-sampling-only (agree to noise). Cost on unlayered scenes: none to 3–5% (tea-set, gem-display with RR). See docs/engine-notes.md, "Layered materials".

- Not done (part 1): the layers drop reciprocity (view-only dimming, as glTF); the coat has no tint, thickness absorption or normal map of its own, and refracts nothing (the base sees light undisturbed); glTF clearcoat / sheen texture maps and the coat normal map are not read; the RR guides (diffuse / specular albedo) ignore both layers; the rough coat and rough metals still lose energy (single-scatter GGX, below); layers on glass (transmission) do nothing; the sheen is sampled with cosine directions only, so a strong, thin sheen at low roughness is noisy.
As built (part 2): GGX energy compensation (Turquin's one-table form of Kulla–Conty) for the base, the coat and the layers under a coat, and anisotropic GGX for brushed and spun metal with visible-normal sampling; tables generated by `tools/fit_layers.py`, including a 4-D one for the anisotropic albedo; `scenes/builtin/brushed-metal` and brushed spheres in `layered-furnace`. Checked: the white furnace's rough metals 0.31–0.90 → 0.99–1.00, the brushed spheres 0.99, rough-coat rims gone; light-sampling-only vs MIS on brushed-metal agree (9×9-blurred RMSE×1000 0.11). Cost: 0–7% (RR) on the reference scenes, some of it longer paths from the kept energy; next-event estimation now evaluates the BSDF once for all strategies. See docs/engine-notes.md, "Energy compensation and anisotropy".

- Not done (part 2): the compensation is the non-reciprocal view-side factor (no multiple-scattering lobe, no colour saturation of multiply scattered metal); the RR guides and the native demodulation's specular albedo ignore it and the anisotropy (rough metals' guides under-state their reflectance); the anisotropic table has 12 nv steps, coarse near grazing; anisotropy has no direction map (glTF's `anisotropyTexture`) and turns from the object's x axis, not the UV tangent; meshes without objects' axes (the room, the floor) brush along world x; rare bright pixels remain where a strongly anisotropic metal sphere meets the floor; rough glass is not compensated.
As built (part 3, 2026-11-01): thin-film interference in glTF's KHR_materials_iridescence form (Belcour–Barla's RGB closed form, two Airy orders) on the base specular and on dielectrics, so soap bubbles, oil slicks and anodized titanium work; packed into the free `surface_scale.zw`; sliders and glTF import; `scenes/builtin/thin-film`. Checked in the white furnace and with MIS against light-sampling-only. See docs/engine-notes.md, "Thin film".

- Not done (part 3): the film's thickness is uniform per material (glTF's `iridescenceThicknessTexture` and the thickness range are not read, and there is no procedural variation, so a bubble has none of its swirling bands); no film on the clear coat (glTF puts it on the base, as here); a rough non-metal base under a strong film loses up to ~6% in some channels (the diffuse takes 1 − F(vh) while a rough GGX lobe returns less than F; a proper fix weights the diffuse by 1 − E_spec and changes old renders); only two Airy orders (very thick films, past ~1500 nm, wash out towards their mean as they should, but not exactly); the film is not spectral.
As built (part 4, 2026-11-01): diffraction gratings (CD, DVD, Blu-ray): orders ±1 and ±2 as the base GGX lobe translated across the grooves on the projected disk, importance-sampled, with one importance-sampled wavelength per grating hit weighted into RGB; evanescent orders give their energy back to the mirror reflection; grooves follow the brush direction and *Spun*; packed into the sign of `optical.w`; the code lives only in the texture-map shader variant, picked when a material has a grating. `scenes/builtin/optical-discs`. Checked in the white furnace and with MIS against light-sampling-only. See docs/engine-notes.md, "Diffraction grating".

- Not done (part 4): the RGB approximation (one wavelength per hit, three Gaussian channel curves; spectral rendering would make it exact, future goals); fixed order efficiencies (0.35 / 0.15, not from the groove profile, and no blaze); orders use the isotropic lobe; a rough order lobe partly past the horizon loses that part and the mirror's energy compensation slightly over-boosts orders moved inward (±10% colour at grazing rims in the furnace); no grating on glass (transmission orders) or on the coat; the RR guides and the native specular albedo ignore the orders; no straight-groove example scene (a ruled plate was tried and caught no orders from the lamps); the texture-map variant now carries the grating code (textured scenes 2–10% slower) and grating scenes the map code; no glTF import (no extension exists).
- Track 11 is complete.

Original list:

- Clearcoat and sheen first. Then anisotropy for brushed metal.
- **Thin-film interference:** soap bubbles, oil slicks, anodized titanium and beetle shells. Belcour and Barla (2017) give an RGB form that doesn't need spectral rendering.
- **Diffraction grating (CD/DVD):** an anisotropic lobe with a per-channel angular offset as an RGB approximation. It becomes exact after spectral rendering (future goals).
- **GGX energy compensation:** add Kulla–Conty here (a small LUT), since every layered lobe loses energy.

#### 12. Volumes (J) — done 2026-11-01

As built: homogeneous media inside transmissive objects (density, scatter colour, HG phase, on top of the Beer–Lambert colour) through the nested-dielectric stack, and global fog in a floor-to-height box; free-flight sampling, NEE with transmittance at scatter points, MIS with the phase function; four scenes (`cornell-fog`, `stage-beams`, `scattering-materials`, `underwater`). See engine-notes "Volumes".

- Not done: heterogeneous media (VDB / noise density, smoke plumes); spectral (per-channel) extinction, so chromatic media only colour by albedo and Beer absorption; true subsurface (diffusion / random walk tuned for skin) beyond brute-force scattering; equiangular sampling for lights inside fog (beams of small lights converge slowly; multiple scattering into a beam shows as speckles); the native denoiser leaves haze noisy (RR is fine); fog inside glass, fog subtracted where shadow rays cross glass, and fog on segments skipped by nested-dielectric false hits are ignored; NEE out of a scattering object ignores refraction at its surface and, with glass shadows on, is blocked; guides (RR, denoiser) see the first surface, not the medium; fog is not keyable as a whole in the timeline UI (its floats are paths, untested); Vermeer room, The Weather Project and the jack-o'-lantern are not built.

- **Unlocks:**
  - Visible laser beams (item 17).
  - God rays and the **Vermeer room**.
  - Olafur Eliasson's ***The Weather Project***: a sun disc, fog and a mirrored ceiling.
  - Subsurface jade, milk and a glowing **jack-o'-lantern**.
  - **Underwater mode**: the whole room filled with a medium.

#### 13. Procedural primitives and SDFs — done 2026-11-01 (first part)

As built: per-object procedural surfaces in one shared AABB BLAS, intersected inside the existing `Proceed()` loops: an exact (analytic) sphere, an exact sphere-traced torus, the Mandelbulb and a level-5 Menger sponge, with SDF-gradient normals; a *Surface* combo in the Scene tab; `sdf-gallery`. See engine-notes "Procedural surfaces".

- Done 2026-11-02: *Metaballs* (kind 5) for the lava lamp, animated by a keyable `Object::phase` carried in the instance's `vertex_base` high bits. Not done: smooth CSG between objects, exact cylinders and cones, parameters per object (Mandelbulb power, Menger level, torus radii), user SDF expressions; procedural emitters are not light-sampled; texture maps and box mapping on procedural surfaces (they render the flat colour); anisotropy / gratings use a generic tangent, not the brush direction; normals use a fixed object-space step (noisy on small fractals); the Menger march can give up at grazing angles (pinholes); walk-mode collision and sky occlusion see no procedural objects (`geometry()` skips them); RR / denoiser guides are exact but fractal detail below a pixel shimmers under RR (not measured); the fractals are slow (Mandelbulb +8 ms at 1440×900) — a step budget tied to the pixel footprint, or a baked distance grid, would help.

#### 14. Path visualizer and light path expressions — done 2026-11-01

As built (part 1): click a pixel with the window open (P), or Alt+click, and a recording variant of `trace` re-traces its paths with that frame's samples; they draw in 3D over the viewport (by bounce, throughput or event, light samples yellow / blocked red), and the window lists the paths (bounces, radiance, why each ended) with a vertex scrubber and table (event, throughput, what each vertex adds). See engine-notes "Path visualizer".

Part 2, light path expressions: up to 4 expressions (Look tab, `Settings::lpe`) compile to DFAs that the `trace_lpe` variant runs per path, each into its own AOV (View → *Light path (LPE)*, EXR capture); light samples split exactly into diffuse and glossy, continued paths take a hashed label with a share / probability weight, so every AOV is unbiased and a partition sums to the beauty. See engine-notes "Light path expressions". Not done: object and material labels ("light off the red wall"); glossy transmission as its own event; a multi-layer EXR with every slot; denoised AOVs; more than 4 slots.

- Not done (visualizer): an animated photon along a path; shadow rays don't show their pass-throughs (glass shadows, nested-dielectric false hits) or portals; no export of the recorded paths; only the perspective camera draws them.

Was:

- **Path visualizer:**
  - Click a pixel. A small dispatch re-traces that pixel's paths with the same sample indices and writes the vertices, throughput and event types (diffuse, specular, refract, NEE, portal) to a buffer.
  - The paths draw as 3D lines over the viewport, colored by bounce or throughput, with a scrubber per path.
  - Needs a line overlay. It fits with UI phase 2's viewport work.
- **Light path expressions (LPE):**
  - Regex-filtered AOVs, such as `LS+DE` for caustics only, or "only light that bounced off the red wall".
  - A small DFA runs per path in `trace`, writing into a few configurable AOV slots.
  - It generalizes today's direct, indirect and spec AOVs.

#### 15. More integrators and integrator battle mode

ReSTIR DI done 2026-11-01 (part 1): first-hit light sampling through per-pixel reservoirs with temporal and spatial reuse, unbiased (generalized RIS, pairwise MIS in the spatial pass), one reservoir chain per sample for the first 4 samples; Render → Advanced → *ReSTIR DI*, and the `fairy-lights` scene (300 bulbs). Direct light at 1 spp: 6.7× (fairy-lights) and 11× (candle-shelf) less variance on flat regions. See engine-notes "ReSTIR DI".

- Not done (ReSTIR): visibility reuse; ReSTIR GI / PT for later bounces; the target ignores coat, sheen, anisotropy and transmission; history is dropped on every non-camera change (animation); fixed spatial radius; a reservoir debug view; accumulation with a static camera converges more slowly with temporal reuse on.

BDPT, photon mapping and battle mode done 2026-11-01 (part 2): Render → *Integrator* (path tracing, bidirectional, photon mapping) and *Battle mode*, the scenes `veach-ajar-bdpt` and `cornell-caustic-photons`. Unbiased BDPT with balance-heuristic MIS over every strategy, light tracing included; photon mapping as probabilistic progressive photon mapping (Knaus–Zwicker), which serves as the caustics mode; battle mode splits the image into four quadrants at equal GPU time, with per-quadrant error curves in the convergence graph. On cornell-caustic at equal samples (RMSE×1000 against a 65k-spp reference): path 23.6, BDPT 6.8, photon mapping 5.1 (BDPT costs 6.8× and photon mapping 2.9× a path sample there). See engine-notes "BDPT and photon mapping".

- Done 2026-11-02: homogeneous media (fog and object media, nested dielectrics with priorities) in both, volume photons, caps raised to 24 BDPT bounces and 32 photon bounces. Not done (BDPT / photon mapping): portals; the MIS weights leave out the free-flight density (σt·Tr) of medium vertices and the transmittance of surface vertices, a small bias in dense media; chromatic (per-channel) extinction; connections across invisible IOR-1 boundaries; a separate radius for the volume gather; `mis_mode` is ignored; a hybrid caustics mode that keeps path tracing and takes only L S+ D paths from a caustic photon map; photon emission from emissive texture maps and procedural emitters; an adaptive gather radius; light tracing with depth of field, non-perspective cameras or RR's jitter (the camera counts as a delta vertex then); the direct / indirect AOVs (the beauty goes in both); BDPT costs 7–15× a path sample (local-memory vertex arrays), so plain path tracing still wins at equal time on veach-ajar (RMSE×1000 4.5 vs 5.9 at 160×90); battle mode stops each quadrant at the sample cap, so the time stops being equal once the fastest reaches it; the native denoiser gets only the guide ray's albedo from BDPT and photon mapping (fringes, mirror ghosting; RR is fine); the error and variance views are wrong across battle quadrants; dragging the photon slider reallocates the photon buffer every frame.

- **BDPT:** good for light behind a door, the Veach "door" scene.
- **SPPM / photon mapping:** a **caustics mode**. A light-tracing compute pass deposits photons into a hash grid, and camera paths gather from it. Real caustics under glass spheres and on the glass-lab table.
- **ReSTIR DI:** thousands of emitters at 1 spp, for a night city or a dense Kusama room. It slots in beside `select_emitter` and reuses the motion vectors and object IDs.
- **Battle mode:** a 2×2 split of path, BDPT, SPPM and path with ReSTIR at equal time, with the live convergence graph (5b) per quadrant. It generalizes the A/B slider.

#### 16. Real lenses and physical glare — done 2026-11-02

Done: four lens prescriptions (Double Gauss 50 mm f/2, Nakamura wide angle 22 mm, Muller fisheye 10 mm, an uncorrected singlet) traced per camera ray with exit-pupil sampling, f-number, scene scale, aperture blades on the stop and optional dispersion (Render → Lens); FFT diffraction glare from the aperture shape, per wavelength (Look → Film → Glare); the `lens-fairy-lights` scene. See engine-notes "Real lenses and glare".

- Not done: RR, the temporal upscalers, click picking and the gizmo with a lens system (motion vectors and picking assume a pinhole; a distortion-aware projection would fix them); real Abbe numbers (the prescriptions carry only n_d, so dispersion uses estimates and is off by default, as it undoes the designs' achromatic pairs); a Zeiss Planar and a telephoto prescription; lens systems with tilt-shift and the other camera models, and BDPT light tracing through a lens; the paraxial focus misses best focus on the singlet's strong spherical aberration; glare is additive (the highlight's own energy is not removed), on a fixed 512² grid (384 px of image, so spikes are soft at 4K and wrap past 128 grid cells), and has no dust, scratches or obstruction (spider vanes) in the aperture; lens flare ghosts (reflections between elements).

- **Lens systems:** trace through real lens prescriptions (Kolb 1995): a Double Gauss, a Zeiss Planar or a fisheye. Distortion, vignetting, chromatic aberration and bokeh then come from the glass elements themselves.
- **Glare:** an FFT of the aperture shape (the same shape as the 5c bokeh) gives the diffraction starburst on bright emitters. Convolve in `display` at a reduced resolution.

#### 17. Physics sandbox, optics bench and laser puzzle

Physics sandbox core done 2026-11-02 (part 1): rapier3d rigid bodies for the objects (box, ball and convex-hull colliders, the floor and Cornell walls), Scene → *Physics* (gravity, speed / slow motion, bounce, friction), grab-and-throw with the mouse, E for an explosion, and the `physics` generator family (dominoes, brick wall, pyramid, Jenga, ball pit) with five curated scenes. See engine-notes "Physics sandbox".

Optics bench done 2026-11-02 (part 2): convex lens, concave lens and prism shapes, a beam-splitter material (all three integrators), spot lights cone-sampled in light subpaths, and the `optics-bench` scene. See engine-notes "Optics bench".

Laser puzzle done 2026-11-02 (part 3): a CPU-traced laser beam (mirrors, splitters, glass) drawn over the viewport, a target object, a move counter with par, Scene → *Laser puzzle*, and three levels in a `scenes/puzzles` pack. See engine-notes "Laser puzzle".

- Done 2026-11-02: locked objects (`Object::locked`, the gizmo refuses them; the three levels lock their targets), moves count only object transforms (not lights or the camera), several targets (`Puzzle::targets`, all must be reached), *Beam in image* (the beam as thin unlit emissive rods in the frame's copy, so it lights the floor and shows in captures), and a *Levels* menu with the best score per level (`Prefs::puzzle_best`). Not done (laser puzzle): coloured filters and colour-matched targets; prisms that split colours (needs spectral rendering); a level using several targets (the feature exists, no level yet); the rods are not in video exports (the export renders its own copy); fog-visible beams need volumes in the beam; the beam is retraced from uncached triangles of every object on any change, which with the physics sandbox running costs a retrace per frame (several ms on big meshes).

- Not done (optics bench): dispersion in the prism (needs spectral rendering: the prism only bends white light); beams visible in haze through glass (needs volumes in BDPT / photon mapping); lens shapes with set focal lengths (only size and stretch); cylindrical lenses, mirrors with curvature, gratings as objects.

- Done 2026-11-02: wind with gusts and a gravity direction (Scene → Physics), N / Shift+N drop a ball / box under the pointer, the wheel moves a held body nearer or farther, Freeze / Unfreeze the selection, per-object mass (`Object::mass`, a density multiplier), *Record* (keys every moving body's pose into the timeline at its fps, so a timeline export renders the simulation), concave colliders (V-HACD convex decomposition for tori, rings, knots, teapots, bunnies, concave lenses and meshes up to 20k triangles, cached by shape and scale), colour and name edits keep the simulation and its velocities (poses, shapes and body settings are compared), bodies sleep later (owner note: thresholds 0.01 / 0.08 rad/s after 1.5 s instead of rapier's 0.05 / 0.5 after 0.5 s), and the `wrecking-ball` and `castle` scenes. Done 2026-11-02 (engine-notes "Physics: incremental edits, joints, cloth, exports"): edits sync body by body (spawning, Freeze, gizmo and inspector edits keep the other bodies' velocities), keyframed objects are kinematic bodies, ball joints (`Object::links`), cloth (`Object::cloth`, Scene inspector), the simulation runs in timeline video exports, Record clears older keys and thins its keys out, `hull_points` keeps extreme vertices, card house / chain / rope bridge / cloth playgrounds and a three-turn domino spiral, self-test frames 211–213. Not done (physics): cloth is one-way (bodies don't feel it) and does not collide with itself or other cloths; links are set in the scene file only (no joint tool in the viewport); edits that touch a joint, delete several objects or change what is keyframed still restart the simulation; motion blur is off in exports with physics; a keyframed (kinematic) body jumps to each frame's pose in one simulation step, so its pushes are a little strong; Record keys bodies only (no cloth shapes); the card house settles by up to 2.5 cm at the top; the wrecking ball still hangs from one joint (a chain of links would look better); V-HACD takes about a second on a big mesh the first time (cached after).

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

#### 17b. GitHub builds and releases (owner request, 2026-11-02) — workflows added 2026-11-02

- Done 2026-11-02 (owner request): an installed copy keeps what it writes out of Program Files: settings, user scenes, captures and installed assets in `%APPDATA%\Caustica`, thumbnails and the asset cache in `%LOCALAPPDATA%\Caustica`; a `portable` file beside the exe keeps everything beside it (snapshots in builds/ get one), and a build under `target/` keeps the development layout. Files from an older install are copied over once (never overwritten); the uninstaller removes the cache and asks before deleting the user data (engine-notes "Data folders"). Not done: no setting to move the data folder; a file left beside an old install's exe after the migration is not cleaned up.

Added `.github/workflows/ci.yml` (every push to main and every PR: `tools/setup.ps1 -SkipBuild` for the SDKs with `vendor/` cached, then fmt, clippy, unit tests and a release build; no GPU on the runner, so no self-test) and `release.yml` (a `v*` tag, or a manual run: release build, a zip of the exe, its DLLs, ffmpeg, `scenes/` and the README, uploaded as an artifact and, for tags, attached to a GitHub release with generated notes). No tag has been pushed yet. **Changed 2026-11-02 (owner: releases automatic, versioned from now on):** `release.yml` is gone; `ci.yml` releases every push to main that passes fmt, clippy, tests and the release build. Version: the Cargo.toml version if its tag does not exist yet and is above the highest `v<major>.<minor>.*` tag, else the next patch (Cargo.toml stays at 0.1.0, so pushes go 0.1.0, 0.1.1, …; bump it to start a new minor or major). The exe carries it (`CAUSTICA_VERSION` from build.rs: window title, Menu, `--version`; local builds say `-dev`, PR builds `-ci.<sha>`); the release is `v<version>` with the zip and the commit subjects since the previous tag. **Changed again the same day (owner):** only a pushed `v*` tag releases (the version is the tag); pushes to main are checked only, and untagged builds say `<Cargo.toml version>-ci.<sha>`. Tags go on finished features or fixes that warrant them. v0.1.0 came from the last automatic run. **Also 2026-11-02 (owner):** the project is renamed Caustica (crate and exe `caustica`, window title, docs, scene credits; `%APPDATA%\CornellRTX` moves to `%APPDATA%\Caustica` on first start; the Cornell box scenes and the `cornell_box` setting keep their names), the WebGL app (`webgl/`) and every mention of it are gone, the exe has an icon (`assets/caustica.ico`, drawn by `tools/make_icon.py` as a traced coffee-cup caustic; build.rs embeds it with version info through rc.exe, and the window and taskbar use it), and releases carry an Inno Setup installer (`installer/caustica.iss`: per-user, Start menu, optional desktop icon, uninstaller, launch after install; built in CI with Inno Setup from Chocolatey, never locally). The GitHub repo is `Kaan88/caustica` (renamed the same day). Still open: the local folder is still called `cornell` (owner's choice); the installer is unsigned (SmartScreen warns on first run) and was first built on the runner, not tried on a clean machine; uninstalling leaves files the app wrote beside itself (settings, captures, downloads, thumbnails). Not done: the self-test in CI (needs a GPU runner); DXC is the runner's Windows SDK copy, not pinned; downloaded assets are not in the zip.

- GitHub Actions on a Windows runner: `cargo fmt --check`, clippy, tests and a release build on every push (DXC from the Windows SDK or a pinned download; the vendor SDKs that cannot be fetched in CI need a stub or a cache, see docs/claude/setup.md).
- Releases: a tag builds the exe with its DLLs, shaders and `scenes/` into a zip and attaches it to a GitHub release with notes from the roadmap entry.
- Open question: the NGX project GUID in `bridge/ngx.cpp` is a development one; decide before any public release.

#### 18. Content using the features above

- **Art and history:**
  - Edward Hopper's *Nighthawks*: mesh import plus item 7.
  - The real Cornell box: measured spectral data and a split view against the photograph. Exact after spectral rendering.
  - Cook's pool balls (item 8).
- Done 2026-11-02: the Newton's cradle swings with the physics sandbox (owner note): fixed frame, balls hung on their strings by joints, elastic contacts. See engine-notes "Physics sandbox".
- Done 2026-11-02: `snow-globe` (glass around water, a lit cottage, 160 snowflakes), `hourglass` (glass cones, sand pile and stream) and `birthday-cake` (seven candles with keyframed flicker) as built-in scenes. Not done: the jack-o'-lantern (needs carving: CSG or a mesh), an hourglass with flowing sand (needs particles), the measured Cornell box (spectral).
- Done 2026-11-02: `lava-lamp` (a new *Metaballs* procedural surface, track 13, whose blobs move with a keyable `Object::phase`; the scene keys it over a 20 s timeline) and `nighthawks` (a corner diner after Hopper, from boxes and cylinders: glass front, fluorescent tubes, three customers and a counterman). Not done: the figures are cylinders with ball heads (no posed meshes), the diner's front is straight with one corner pane (the painting's curved glass wants a curved-glass shape), metaballs have no per-object blob count or colour, and a phase change rebuilds the whole scene (phase is in `geometry_key`; uploading only the instance table would do); the wax is a single emissive colour.
- **Miniatures:**
  - Snow globe (10), aquarium (10, 12) and lava lamp (13).
  - Newton's cradle (8) and an hourglass (mesh import).
  - Jack-o'-lantern (12) and a birthday cake with flickering candles (7, 8).

#### 19. Integrations and whimsy

- **Text-to-scene with Claude:**
  - Needs the scene files (item 4) and ideally the generators (core 1).
  - The prompt carries the `Settings` schema, the shape, pattern and material-library lists, and a few example files. The model returns JSON, which goes through `validate()`; on failure, retry with the error.
  - **Art critic mode:** send the render back and ask for concrete lighting and composition changes as `key=json` edits.
  - Done 2026-11-02: Scene → "Describe a scene (Claude)" and `--describe "text"`: `claude-opus-5-5` over raw HTTP (ureq), `src/scene.rs` and two example files as the cached system prompt, replies through `scenes::parse` + `validate()`, up to two repairs with the error sent back. Tested with mocked replies and a live 401; no generated scene seen yet (no API key on this machine). See engine-notes "Text-to-scene". Not done: a smaller prompt for small local models (the system prompt is roughly 16k tokens; owner: later); token-by-token streaming (the status line shows the stage and elapsed time, the text arrives whole); the Codex CLI backend has not been run (Codex is not installed here; built from its documented flags); no live test against a real OpenAI key or a running local server (mock servers only); an edit or an applied critique replaces the whole scene, so manual changes made while the request ran are lost (Ctrl+Z restores them).
  - Done 2026-11-02 (owner request): a *Backend* picker in the Describe a scene section: saved backends (`Prefs::ai_profiles`, provider, model, effort, endpoint; never keys) switch in one click, *Save* / *Forget* manage the list. Not done: new backends are still set up in Menu > Text-to-scene model; a key typed for one backend is not kept per profile.
  - Done 2026-11-02 (owner request): an *opencode CLI* backend (`opencode run`) that uses the providers and logins set up in opencode; *Fetch* lists `opencode models`, so the model is picked inside Caustica. Not done: per-model reasoning variants (typed into the name as `#variant`); the reply arrives whole (no streaming of the event stream).
  - Done 2026-11-02 (owner request, completing the item): *Edit this scene* (the current scene file in the prompt, `--describe-edit`), *Critique the render* (art critic mode: the render as a PNG plus the scene, notes and an improved scene to apply), the generators and the material library in the prompt, model and reasoning pickers under the text box with *Fetch* for server model lists, status with elapsed time and Cancel, and ChatGPT accounts through the Codex CLI. Live through the Claude CLI: an edit (hourglass at night) and a critique.
  - Done 2026-11-02 (owner request): backends beyond the API key, picked in Menu → Text-to-scene model: the Claude Code CLI (`claude -p`, the CLI's login, the default; first real scene generated with it), the Claude API, ChatGPT (OpenAI API) and a custom OpenAI- or Anthropic-compatible endpoint for local models (Ollama, LM Studio, llama.cpp). Keys typed in the menu are session only.
- **Scripting console** (`rhai`): script animations, generators and batch renders, for example rendering every preset at 4,096 spp. Add a `--script file` CLI flag.
  - Done 2026-11-02: `--script file.rhai` (headless or windowed) with `load`, `set` (nested `/` paths, validated), `frames`, `reset`, `capture`, `samples`, `scenes`; `examples/scripts/batch.rhai`. See engine-notes "Scripting". Not done: an in-app console (a text box running lines against the live scene), access to generators and keyframes from scripts, reading settings back (`get`), a script that runs per frame (procedural animation). Done 2026-11-02: `get(key)` returns a setting or `/` path as JSON text.
- **Audio-reactive lights:** WASAPI loopback capture (`cpal`) and an FFT drive emitter intensities per frequency band. Runs in per-frame RR mode, since every frame changes the lights.
  - Done 2026-11-02: Scene → Audio (`Settings::audio`): WASAPI loopback through cpal (an input stream on the default output device), a 2048-sample FFT (rustfft) into bass/mid/treble with automatic gain and smoothing, scaling the main light (bass), area light i and emitter i (band i mod 3) in the frame's copy of the settings. See engine-notes "Audio-reactive lights". Done 2026-11-02: the `audio-equalizer` demo scene (twelve bars and three area lights by band). Not done: per-light band choice, a beat detector (strobe on kicks), colour shifts per band, non-f32 output formats (24-bit integer shared mode), video export synced to an audio file.
- Removed 2026-11-02 (owner decision, not planned): the live wallpaper (rendering behind the desktop icons with the WorkerW trick) and the Blender live link (an add-on streaming objects, materials and the camera over a socket).

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
