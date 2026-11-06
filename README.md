# Caustica — Rust / Direct3D 12

Caustica is a Windows path tracer on hardware ray tracing (it began as a Cornell box renderer). The renderer and application are Rust; path tracing and postprocessing are HLSL. A small C++ C-ABI bridge calls NVIDIA's NGX SDK.

Startup defaults favor image quality: **native-resolution DLAA with Ray Reconstruction and 4 samples per frame**. Select Quality/Balanced/Performance explicitly when you want a lower internal resolution.

## Run

Double-click **`launch.cmd`**, or run `target/release/caustica.exe`. The launch script builds the application if the release executable is missing.

For a clean checkout:

```powershell
cd native
powershell -NoProfile -ExecutionPolicy Bypass -File tools/setup.ps1
cargo run --release --locked
```

Build requirements: Windows x64, Rust MSVC (Rust 1.88 or newer), Visual Studio C++ Build Tools, Git, and a Windows SDK containing DXC. `build.rs` discovers DXC in the Windows Kits directory; override with `DXC_PATH` if necessary. Runtime requirements: D3D12 feature level 12.1 and DXR tier 1.1. DLSS RR additionally requires a supported NVIDIA RTX GPU/driver. Capability detection reports actual NGX initialization, creation and evaluation results; the native denoiser remains available when RR is unavailable. `--require-rr` makes missing or failed RR a fatal error for verification.

Setup fetches three vendor SDKs, each pinned to a commit and sparse-checked-out into `vendor/` (not committed):

| SDK | Version | Used for | License |
|---|---|---|---|
| [NVIDIA DLSS](https://github.com/NVIDIA/DLSS) | 310.9.16 | Ray Reconstruction, DLSS Super Resolution | `vendor/dlss/LICENSE.txt` |
| [AMD FSR SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) | 2.3.0 | FSR upscaling through the FidelityFX API | binary redistribution, `vendor/fsr/Kits/FidelityFX/docs/license.md` |
| [Intel XeSS SDK](https://github.com/intel/xess) | 3.0.2 | XeSS Super Resolution | Intel Simplified Software License, `vendor/xess/LICENSE.txt` |

Cargo copies the runtime DLLs (`nvngx_dlss.dll`, `nvngx_dlssd.dll`, `amd_fidelityfx_loader_dx12.dll`, `amd_fidelityfx_upscaler_dx12.dll`, `libxess.dll`) next to the executable. The FSR and XeSS DLLs are loaded at runtime, so the app still starts if one is missing; that upscaler is then shown as unavailable. No DLL downloads occur during application startup. On Linux, `sh tools/setup.sh` fetches the DLSS SDK's Linux files and Cargo copies `libnvidia-ngx-dlss.so.*` and `libnvidia-ngx-dlssd.so.*` beside the binary (the AppImage bundles them), so RR and DLSS SR work on NVIDIA GPUs there; Intel ships XeSS for Windows only, and FSR has no Vulkan build in the vendored SDK. The NGX bridge uses a stable local-development project GUID; replace it with your registered project identity before distribution.

## Releases and versions

Releases are built and uploaded locally: after `git tag v0.10.0 && git push origin v0.10.0`, `tools/release.ps1 -Version 0.10.0 -AppImage -Upload` makes the GitHub release `vX.Y.Z` with an installer (`caustica-vX.Y.Z-setup.exe`: per-user, no admin prompt, Start menu entry, uninstaller; updates install over the previous version) and a zip of the app, both with the models, HDRIs and textures the shipped scenes use (`tools/fetch_assets.ps1`, about 870 MB before compression), a Linux AppImage (`Caustica-X.Y.Z-x86_64.AppImage`: Vulkan, the shipped scenes without the asset packs, settings and captures under `~/.config/caustica`; built on a Linux machine by `tools/appimage.sh`), plus the commit list since the previous release. Without `-Upload` it only builds into `dist/`; `tools/make_installer.ps1 [-Source <release folder>]` builds the installer alone (Inno Setup 6.3+ or 7, assets fetched). The CI workflow (format, Clippy, tests, Windows and Linux builds) runs only when started by hand. Releases are tagged when a feature is complete or a fix warrants one; `Cargo.toml` carries the same version. The version shows in the window title, at the top of the Menu and with `--version`; local builds say `-dev`.

The app looks for a newer release at startup (Menu → *Check for updates at startup* turns it off) and can download, verify and install it: through the setup on Windows, by replacing the AppImage on Linux. Other copies get a link to the release page. Third-party terms are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Rendering

- **Hardware ray tracing:** triangle BLAS and TLAS, built with `ID3D12Device5`; HLSL `RayQuery` traversal for every primary, bounce and shadow ray. This uses DXR tier 1.1 inline ray tracing, including the GPU's acceleration-structure traversal hardware. No software intersection loop is involved.
- **Path tracing:** progressive FP32 HDR accumulation, per-sample Welford luminance variance, next-event estimation over every emissive triangle, sphere light and sun (power-weighted, so all lights and emissive meshes share one sampler; spheres and suns are sampled by the solid angle they cover), balance-heuristic MIS, Russian roulette, and an optional firefly clamp.
- **Scenes:** up to 4096 objects (TLAS instances of shared meshes, so moving one only rebuilds the top level) and 32 extra area lights, plus the main ceiling light. Eleven shapes: box, sphere, cylinder, cone, torus, trefoil knot, Utah teapot (tessellated from Martin Newell's Bézier patches), Stanford bunny (the 69,451-triangle scan), octahedron, chess pawn and thin ring. Each object has yaw/pitch/roll, nonuniform scale, and either rests on the floor or floats at a set height. The stage can be the Cornell enclosure or an open floor of any size with sky light. Lights can be hidden from the camera while still lighting the scene and appearing in reflections.
- **Materials:** Lambertian/GGX reflection, dielectric Fresnel, refraction, total internal reflection, Beer–Lambert absorption, metallic color, roughness, reflectivity, IOR, transmission and emission, with optional emission-only (unlit) surfaces. Procedural checker, stripe and marble patterns, and a 12-entry material library (porcelain, chrome, gold, copper, glasses, jade, lacquer, marble, emitters).
- **NVIDIA DLSS Ray Reconstruction:** NGX feature `RayReconstruction`, created/evaluated through the official DLSSD helpers. It consumes fresh noisy HDR samples, diffuse/specular albedo, world normals plus packed roughness, linear view depth, current-to-previous pixel motion, primary-surface specular hit distance and camera matrices. RR receives neither progressively accumulated color nor the native denoiser's output. Jitter uses a 64-phase Halton sequence, with deterministic guide surfaces for each frame. Camera motion preserves neural history; geometry/material/light edits reset it.
- **Quality mode and upscalers:** Native (100%), Quality (67%), Balanced (58%), Performance (50%) and Ultra Performance (33%), plus a custom render scale.
  - With **Ray Reconstruction** on, RR denoises and upscales in one step. Models: NVIDIA default, Transformer D, Transformer E and RR2/F (default).
  - With RR off at 100%, Quality mode offers **Native** (progressive accumulation, the unbiased path) or the selected temporal upscaler as anti-aliasing: **DLAA**, **FSR Native AA** or **XeSS Native AA**.
  - With RR off and a mode below 100%, the native denoiser runs at render resolution and an **upscaler** brings it to the window: **NVIDIA DLSS Super Resolution** (default), **AMD FSR** (FidelityFX API; FSR 3.1, or FSR 4 on supported AMD GPUs), **Intel XeSS** (runs on any recent DX12 GPU), or the spatial **bicubic** (Catmull-Rom, clamped to stop ringing) and **bilinear** filters.
  - Without RR the native denoiser keeps edges anti-aliased: it demodulates by the albedo averaged over each pixel's own samples and leaves pixels whose samples hit several surfaces mostly unfiltered. The progressive path spreads samples with a Blackman-Harris pixel filter (1.5 px) by default.
  - The three temporal upscalers get fresh jittered samples every frame (Halton, 64 phases), device depth and render-resolution motion vectors. The native denoiser's history and the upscaler then do the accumulation, as in a real-time renderer, so detail resolves above the render resolution. Motionless pixels keep a longer denoiser history, so a still image keeps getting cleaner. An unavailable upscaler falls back to bicubic and says so in the panel.
  - DLSS Super Resolution has its own **preset** choice: NVIDIA default (K for DLAA/Quality/Balanced, M for Performance, L for Ultra Performance), or J, K, L or M forced for every mode.
  - Changing the model, preset, quality mode or upscaler recreates the feature with completed GPU work.
- **Native denoising (SVGF-style):** raw samples are integrated over time in albedo-demodulated space, then filtered by five à-trous passes guided by object id, normal, plane distance and a Gaussian-prefiltered luminance variance. On a camera move, the previous integration is reprojected bilinearly; each tap is checked by object id, normal and depth. While the camera is still, that history stays frozen and fades out over 256 fresh samples, so samples are never counted twice. Mirror-like pixels keep at most 2 samples of history. The history never feeds the unbiased progressive accumulation. Native rendering offers padded 2D Owen-scrambled Sobol sampling or independent random sampling. RR deliberately uses independent random path samples, following NVIDIA's noise guidance.
- **HDR displays:** with Windows HDR on, the app outputs real HDR (an FP16 scRGB swapchain) and uses Windows' own values: the panel peak from Windows HDR Calibration and the SDR content brightness as paper white. The tone map applies in HDR too: midtones look exactly as in SDR at paper white, and the curve's highlights expand into the panel's extra range. Linear shows true scene brightness with a BT.2390 roll-off near the peak. Output can be Auto, SDR or forced HDR, and peak and paper white can be overridden. The controls are drawn at the SDR brightness so they match the rest of the desktop. **HDR PNG** saves what the HDR screen shows as a 16-bit PQ / BT.2020 PNG tagged for HDR viewers. PNG and EXR captures are unchanged.
- **Output:** twelve views (beauty, normals, diffuse albedo, depth, direct, indirect, specular, convergence, and heat maps of bounces, BVH cost and variance, plus false colour), six tone maps, exposure/gamma, direct/indirect relighting and a frozen linear-HDR A/B reference.
- **Native controls:** egui rendered directly into the D3D12 swapchain, 24 scene presets, randomization, camera controls, validated JSON save/load, GPU timestamp timing, pause/convergence and resize support. A separate **Scene objects** window lists, searches, adds, duplicates, deletes, hides and focuses objects and edits their transform and material; click an object in the viewport to select it. The Lighting section edits the main light and up to 32 extra area lights. Material and light edits update the sampling tables without rebuilding the acceleration structures.
- **Light kinds** (*+ Light* in the Scene tab, or *Kind* in a light's inspector): **rectangle** (the softbox), **disk** (an ellipse), **sphere** (a bulb that shines all round), **spot** (a disk whose light fades outside a cone: *Spread* and *Blend*) and **sun** (a distant disk of a set angular diameter, 0.53° like the real sun by default, for crisp shadows; it also shows in the sky and in reflections). Disks, spots and spheres keep their brightness when resized: their intensity is that of a 1 × 1 panel. A sun's intensity is the brightness of a white surface facing it.
- **Presets:** 0–8 are the Cornell-box studies (classic, reflective spheres, sphere and box, mirror + glass, mirror, glass, warm, dramatic, emissive spheres). 9–23 are open studios: material matrix, weekend sphere field, Whitted checkerboard, Utah teapot studio, Stanford bunny studio, glass laboratory, RGB shadow study, sphere pyramid, mirror corridor, knot gallery, column courtyard, chess still life, neon sculpture, orbital sculpture, and an empty stage to build on. Switching between Cornell-box presets keeps your camera; any other switch uses the preset's own view. Randomize picks any preset except the empty stage: inside the Cornell box it reshapes and recolors the blocks, elsewhere it keeps the layout and recolors diffuse and glass objects.
- **Lens and camera** (Scene tab → Camera): thin-lens depth of field with aperture, focus distance, round or bladed bokeh and cat-eye vignetting; **Ctrl+click** focuses on what is under the pointer with a short focus pull. Camera models: perspective, orthographic, 360° panorama (equirectangular), little planet and tilt-shift (a tilted plane of focus for the miniature look). Non-perspective models render with the native denoiser.
- **Film look** (Look tab): tone maps include AgX and Khronos PBR Neutral; halation, film grain (fine / medium / coarse) and vignette; lift / gamma / gain colour wheels; and any `.cube` 3D LUT with a strength slider. In HDR the look applies before the HDR expansion.
- **Light sampling** (Render tab → Advanced): MIS, or light sampling (NEE) only, or BSDF sampling only, to see what MIS combines; try it on *Veach MIS plates*. **ReSTIR DI** (same place) picks each pixel's first-bounce light sample through reservoirs reused across neighbouring pixels and frames: unbiased, and far less noise with many lights at a few samples per pixel; try it on *Fairy lights* (300 bulbs, on by default there). **Integrator** (Render tab) switches between path tracing, bidirectional path tracing (BDPT: paths from the lights joined to camera paths, for light through gaps and caustics seen directly) and photon mapping (fast caustics, slightly blurred at first); both handle fog and media (water, smoke-filled glass) too; try *Cornell caustic, photon mapping* and *Door ajar, bidirectional*. **Spectral** (Render tab, path tracing and photon mapping) traces wavelengths of light instead of red, green and blue: glass with **Dispersion** (material editor, an Abbe number) splits white light into a rainbow, and thin films and gratings get their exact colours; try *Dark side of the moon*. **Battle mode** splits the image into four quadrants (path, path + ReSTIR, BDPT, photon mapping) rendered at equal GPU time; with a reference loaded, the convergence graph plots each one's error over time. **Today** in the top bar loads the seed of the day: a random scene that is the same for everyone on the same day.
- **Toys:**
  - **Light painting** (L or *Paint light* in the top bar): moving or changing lights keeps one long exposure, so moving lights leave trails. It runs progressively, with RR and temporal upscalers off.
  - **Portals:** in an object's inspector, *Portal to* pairs it with another object. Rays that hit one continue from the matching point of the other, from either side, so corridors and impossible rooms are possible. Light sampling does not see through portals.
  - **Walk mode** (` or *Walk*): a first-person walk with gravity. WASD to move, Space to jump, C or Ctrl to crouch, Shift to run, drag to look, the wheel to make yourself bigger or smaller, B for head bob, Esc to stop. You step up small ledges and stairs, follow slopes and stop at walls. F switches to flying (Q / E down / up), which is also where you start when there is nothing to stand on below the camera.
  - **Video** (Export tab → *Export video…*): an orbit around the target (angle, elevation, distance, linear or eased) rendered frame by frame at any size up to 8192 (independent of the window), any frame rate and length, to a sample count or a time limit per frame. Formats: H.264, H.265, HDR10 HEVC (10-bit PQ, with HDR output on), ProRes, or a PNG / linear EXR sequence. The window stays responsive with a preview, progress, time left and *Cancel*. From the command line: `--turntable out.mp4 --turntable-frames N --turntable-spp N --turntable-format h264|h265|hdr10|prores|png|exr`, plus `--turntable-timeline` to follow the scene's keyframes (with motion blur when its shutter is open).
  - **Screensaver:** `powershell -File tools/make_screensaver.ps1` builds `builds/screensaver/Caustica.scr`; right-click it and choose *Install*. It cycles every preset and scene with a slow orbit.
  - **Gamepad:** sticks orbit and zoom, the triggers set light intensity, the bumpers switch scenes, A pauses, B restarts, the D-pad sets exposure, Start hides the panel.
  - **Phone remote** (Menu → *Phone remote*): serves a page on the local network with a QR code. It has sliders for exposure, light, orbit, zoom and aperture, scene buttons and the tone map. It is off by default, and every request needs a random per-session token.
- **Look tools** (Look tab → Tools): zebras on clipped areas; scopes (RGB and luma histogram, luma waveform, vectorscope of the displayed image); a light meter that shows luminance (nits, 1.0 = 80 nits as in HDR) and illuminance (lux, assuming a Lambertian surface) under the cursor; and a convergence graph that plots RMSE against a reference EXR, or against the current image once converged, as samples grow. The **path visualizer** (P, or *Path visualizer* in Tools) re-traces the paths through a pixel you click (Alt+click works any time) with that frame's samples and draws them in 3D, coloured by bounce, throughput or event, with the light samples (yellow: reached the light, red: blocked); orbit around them, pick one path to step through its vertices, and see what each vertex adds. **Light path expressions** (Look tab): up to four regular expressions over a path's events, each rendered into its own image (View → *Light path (LPE)*, raw samples, also what an EXR capture saves): `C D L` direct diffuse, `C D [ST]+ [LB]` caustics, `C [ST]+ .* [LB]` reflections and refractions, `C .* B` light from the sky. Events from the camera: D diffuse, G glossy, S specular, T transmission, V volume, then L a light or B the background; `.` any, `[DG]`, `[^S]`, `*`, `+`, `?`, `|`, `( )`. From the command line: `--set 'lpe=["C D L"]' --set view=12 --set lpe_slot=0`. Menu → *Statistics and achievements* shows lifetime totals (paths traced, GPU time, images saved, record sample count, presets and scenes visited) and eight achievements.
- **Captures:** PNG, JPEG (with a quality setting), linear RGB32F OpenEXR, and a 16-bit PQ / BT.2020 HDR PNG while HDR output is on. *Render high-resolution image* (Export tab) renders fresh paths at up to 8192 on the longest side with a chosen sample budget, in any of those formats, while the window stays responsive (progress, Cancel). Saved files get a toast with *Show in folder*.
- **Quality** (top bar): Best / Balanced / Fast / Fastest (DLSS Ray Reconstruction at 100 / 67 / 50 / 33%), Native denoiser, Native upscaled (with the best temporal upscaler available) and Reference (no denoiser). It picks the Render tab's settings in one go and shows *Custom quality* when they match no bundle. The status bar shows the time left to the sample cap and, on hover (or inline with advanced settings), GPU time per pass.
- **Scene files and preferences:** Menu → *Open scene…* (Ctrl+O), *Save scene* (Ctrl+S), *Save scene as…* (Ctrl+Shift+S); the Scenes window lists Favorites and Recent. Favorites, recent scenes, the UI size (Ctrl +/−) and *Show advanced settings* are remembered in `%APPDATA%\Caustica\prefs.json`. `--preset` takes a name such as `mirror-corridor`. While comparing A/B, drag the split line.

## Controls

The window has a **top bar** (Menu, scene picker grouped into Cornell box / Studios / Build your own with ◀ ▶, Randomize, view, Pause, Restart, Frame, the gizmo tools Select / Move / Rotate / Scale with Global or Local axes, Hide panel), a **side panel** with four tabs and a **status bar** (state, samples and progress, GPU time, the denoiser and upscaler in words, render and output resolution, HDR).

- **Scene:** an outliner (Stage, Camera, Main light, area lights, objects with visibility toggles) with add buttons, and an inspector for the selection.
- **Render:** DLSS Ray Reconstruction, RR model, quality mode, upscaler and DLSS preset, native denoiser, pixel filter (Box, Gaussian or Blackman-Harris, with a width) for the native path, sampling, texture filtering (anisotropic, 1–16×).
- **Look:** view, exposure in EV, tone map, display output and HDR levels, A/B compare.

Selecting an object or light shows a **Blender-style gizmo**: red / green / blue arrows and plane squares to move, rings to rotate, axis handles and a center circle to scale. Lights are outlined in the viewport even when hidden from the camera, and can be clicked. The main light moves and scales; area lights also rotate.
- **Export:** PNG, JPEG, HDR EXR, HDR PNG, high-resolution render (same formats), scene file.

Rarely used rows sit under **More**; Menu → *Show advanced settings* shows them all inline. Labels name what a control does and show units; hover a label for a one-line explanation. Short confirmations (saved files, key toggles, errors) appear as toasts at the bottom right.

| Input | Action |
|---|---|
| Left click | Select the object or light under the cursor (opens it in the Scene tab); empty space deselects. The selection is outlined, and so (faintly) is the object under the pointer |
| Double-click | Frame the object under the cursor |
| Right click | Select what is under the cursor and open a menu: frame, move, rotate, scale, duplicate, hide, delete, frame the scene, reset the camera, show all hidden (outliner rows have the same menu) |
| Drag a gizmo handle | Move along an arrow or plane, rotate on a ring, scale on an axis or (center) evenly |
| G / R / S | Move / rotate / scale the selection with the mouse, as in Blender: then X / Y / Z for an axis (again: local axis, again: free), Shift+X/Y/Z for a plane, type a value, Ctrl snaps, Shift is fine control; click or Enter confirms, right click or Esc cancels |
| Ctrl+Z / Ctrl+Y | Undo / redo (Ctrl+Shift+Z also redoes) |
| Left drag | Orbit |
| Right or middle drag / Shift-drag | Pan |
| Wheel | Dolly |
| F | Frame the selected object, or all objects when nothing is selected |
| Home | Reset the camera to the scene's own |
| Page Up / Page Down | Previous / next scene |
| Ctrl+D / Shift+D | Duplicate the selection / duplicate it and move the copy |
| H / Alt+H | Hide or show the selection / show all hidden objects |
| F1 | Sheet of every shortcut |
| Ctrl+S / Ctrl+Shift+S / Ctrl+O | Save the scene / save it as / open one |
| Delete | Delete the selected object or area light |
| Escape | Close the menu, else deselect (the app closes only from the window) |
| Space | Pause/resume |
| R | Restart accumulation (with an object or light selected, R rotates) |
| A | Auto orbit |
| D | Toggle the native denoiser |
| C | Freeze the A/B reference |
| 1–8 | Select view |
| F12 | Save `caustica.png` |
| I / Shift+I / Alt+I | Key the selection at the playhead (the camera when nothing is selected) / key every value edited since the playhead last moved / delete the selection's keys there |
| K | Play or stop the timeline |
| Shift+Left / Right | Step the playhead one frame |
| Tab | Hide the panel, then everything, then show all again |
| Ctrl +/− | UI size |

### Scene files

`scenes/` holds scene packs: one folder per pack with a `pack.json` (name and scene order) and one JSON file per scene. A scene file wraps the native settings with a title, description and tags; fields it leaves out take the defaults, so files only list what they change. `scenes/builtin` ships **96 scenes**: Cornell variants (mirror room, measured box, bunny, caustic, fog, dark room, water tank), validation and lighting studies (white furnace, Veach MIS plates, the Veach ajar room under BDPT, penumbra ladder, light temperatures, IES wall, glossy tiles), material showcases (shape × material matrix, gems, jewellery on velvet, diamond fire, scattering media, glass stack, bunny museum under IES spots, tea set, layered materials), light kinds (a sun through a greenhouse, bare sphere-light bulbs, a flashlight crypt, a blackbody forge), camera models (street bokeh through a Double Gauss lens, a tilt-shift village, a little planet, a 360° gallery, an isometric workshop), volumes (stage smoke, a lighthouse in noisy fog, cornell fog), the film look (a neon diner with halation, grain and a LUT), physics (festival banners in the wind, a swinging lantern chain, Newton's cradle, a wrecking ball, dominoes), architecture (Stonehenge at dusk, the torii corridor on a camera path, spiral staircase, colonnade, night skyline, desert monolith and zen garden under the physical sky, disco floor, mirror maze, stained-glass hall), art rooms (after Kusama, Flavin, Turrell; portal corridors) and still lifes (the 1984 pool break, the orrery run by a script, the bowling strike with motion blur, abacus with bokeh, atom, armillary, snowmen, candles, birthday cake, forest, pipe organ).

- **Scenes** in the top bar opens the browser: a grid of thumbnails grouped by pack, search, tag filters, a larger preview with description, tags, licence and credits, double-click to load. Each pack shows its licence as a label (amber when it is non-commercial); `pack.json` may give `license`, `commercial_use`, `attribution` and `source`. Loading keeps your render choices (resolution, RR, upscaler, presets, VSync, HDR). With a scene file loaded, ◀ ▶ step through the files.
- **Save to scenes/user** in the same window writes the current scene with a title and description (see *Where files go* below).
- **Generators:** the Scenes window's *Generate* section builds a scene from a family and a seed: studio still life, material grid, city blocks (day or night), temple, stairs (straight, L, U, spiral), stacking (cairn, Jenga, house of cards, sphere pyramid), Menger sponge, Sierpinski tetrahedron, sphereflake, kaleidoscope, Truchet maze, and four built from procedural meshes: Mandelbulb, terrain island, deep (level 3–4) Menger sponge and L-system trees, and two that show instancing off: a forest of up to 1800 trees and a crowd of up to 2900 copies of one shape. *Next seed* and *Random* step through variations, and a generated scene can be saved as a tiny scene file (family + seed). `scenes/generated` ships 96 curated seeds.
- **Thumbnails** are rendered in the background the first time the browser opens, by running the exe headless for each scene (320×200), and cached in the thumbnail folder (see *Where files go*) under the file's hash, so an edited scene gets a new one. *Re-render thumbnails* redoes them all; `--make-thumbnails` renders the missing ones from the command line.
- **Meshes:** *+ Mesh…* in the Scene tab imports an OBJ (MTL colour, emission, transparency), a glTF/GLB (node transforms and PBR material: base colour, roughness, metallic, emission, transmission, IOR, volume absorption; its punctual lights come along), a PLY or an STL; tick *Z-up* first for scans and CAD files. The mesh is scaled into the same box as the built-in shapes, so size, stretch and rotation work as usual. Scene files store the path, relative to `scenes/` when the file lives there; a missing file falls back to the object's shape and the inspector says so. A file with several materials becomes one object per material, lined up; textures are not sampled yet, so each part takes its texture's average colour. `scenes/meshes` has 9 scenes built on test models: run `tools/fetch_assets.ps1` once to download them (pinned, checksummed) into `scenes/models/`.
- **Sliders:** double-click any slider to reset it to the value the scene was loaded with.
- **Light outlines:** the rectangles drawn over lights are off by default; *Show outline* under each light's *On* switches them on (the selected light always shows its own).
- **Assets:** *Assets* in the top bar browses Poly Haven (HDRIs, textures, models), ambientCG (materials) and the Khronos glTF samples with thumbnails and search, downloads at a chosen resolution into `hdri`, `textures` or `models` of the user scenes folder (each file's SHA-256 pinned in its `assets.json`; see *Where files go*), and then uses it in one click: as the HDRI, as a texture on the selected object, floor or walls, or added to the scene. *Other sources* lists places to download meshes by hand (Stanford, McGuire, Bitterli, ORCA, Smithsonian, Sketchfab). From the command line: `--fetch-asset polyhaven/hdri/meadow_2@2k`.
- **Textures:** imported glTF and OBJ meshes show their base colour, normal and metallic-roughness maps (alpha-masked leaves too), with mipmaps. Any material, including the floor and walls, can take a texture set: *Texture → Load…* in the material editor, pick any map of an ambientCG or Poly Haven set (Color, NormalGL, Roughness, Metalness, or diff / nor_gl / arm) and the rest load with it, box-mapped at *Texture scale* repeats per unit. Colour, roughness and metal start at the maps' averages and scale them. `scenes/materials` has 5 scenes; `tools/fetch_assets.ps1` fetches 8 CC0 sets into `scenes/textures/`.
- **HDRI environments:** *Stage → HDRI → Load…* lights the scene with an equirectangular .hdr or .exr instead of the sky light, importance-sampled (with MIS against the BSDF and the scene's area lights), with strength, rotation and *Seen by camera* (off: the camera sees the background colour while the HDRI still lights). `scenes/environments` has 14 scenes on Poly Haven HDRIs; `tools/fetch_assets.ps1` fetches 19 of them (4k, and 8k for three skies) into `scenes/hdri/`.
- **Animation:** *Timeline* in the top bar (or the first key) shows a strip above the status bar: transport, the playhead (drag to scrub, snapped to frames unless Shift is held), every key in the scene and the selection's keys, *Key*, *Delete key*, the interpolation of the keys at the playhead (smooth, linear, ease in and out, hold), length, frame rate, shutter and *Clear*. Any value stored as a decimal number can be keyed: camera, object transforms, light position, size, colour and intensity, material values, exposure, sky and HDRI rotation. Edits at a still playhead stay until it moves, so they can be keyed (I keys what changed, Shift+I keys every edited value). Keys are saved in the scene file. *Export video…* → *Timeline* renders the animation at its frame rate; a shutter above 0° adds motion blur (each pass is one moment inside the open shutter; Ray Reconstruction is off for it). `scenes/builtin/pool-break-1984` is the showcase, after Cook, Porter and Carpenter's 1984 pool balls.
- `--scene NAME` loads a scene by file name, title or path on the command line; `--help` lists them.
- **Where files go** (`src/dirs.rs`): an installed copy never writes beside the exe, so Program Files works and uninstalling leaves nothing there. Prefs, stats, *Save settings.json*, saved images, videos and captures go to `%APPDATA%\Caustica` (`captures\` for images and videos), user scenes and downloaded assets to `%APPDATA%\Caustica\scenes`, and the caches (scene thumbnails, asset catalogs and previews) to `%LOCALAPPDATA%\Caustica\thumbnails`. The first start copies what an older build left beside the exe (user scenes, downloads, settings.json, captures). A file named `portable` beside the exe keeps everything beside it instead (build snapshots get one). Builds run from a cargo `target` folder use the repo's `scenes/` and the working directory as before, so what the asset browser downloads there stays in the repo and an installed copy doesn't see it. The uninstaller removes the caches and asks before deleting `%APPDATA%\Caustica`.
- `tools/build_scenes.py` generates the built-in pack (edit scenes there, then rerun it); `tools/scene_sheet.py` renders every scene into a labelled contact sheet in `captures/scenes/`.

Save/load uses `settings.json` in the working directory. The image buttons write `caustica.png` / `.jpg` / `.exr` / `.hdr.png`; high-resolution export writes `caustica-high-res` with the chosen format's extension. These are native settings; the browser JSON schema is separate. All material colors are linear RGB. Lighting temperature is normalized to unit luminance, then multiplied by the gel and intensity.

## Offline rendering and verification

From the repo root:

```powershell
# 4K output, RR quality, 256 frames; omit --headless to watch the render.
.\target\release\caustica.exe --headless --width 3840 --height 2160 --scale 0.666667 --frames 256 --require-rr --capture cornell-4k.png

# Native-resolution DLAA and linear HDR output.
.\target\release\caustica.exe --headless --width 1920 --height 1080 --scale 1 --frames 128 --require-rr --capture caustica.exr

# Unfiltered progressive DXR reference: 1024 frames × 2 spp = 2048 spp.
.\target\release\caustica.exe --headless --width 960 --height 720 --scale 1 --frames 1024 --samples 2048 --no-rr --raw --capture reference.exr

# Preset indices 0–23 follow the UI order (--help lists them). Native settings can be loaded
# from a file; --set then overrides single top-level settings with JSON values.
.\target\release\caustica.exe --headless --frames 64 --preset 12 --set bounces=20 --set exposure=1.3 --capture teapots.png
.\target\release\caustica.exe --settings settings.json

# Full verification; uses target/validation so an open development build can stay running.
powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify.ps1
```

**Renderer:** Menu → Renderer picks D3D12 or Vulkan for the next start (saved with the preferences; *Restart now* relaunches with the same command line). If Vulkan was chosen there and can't start, the app falls back to D3D12 and the menu says why. `--vulkan` and `--d3d12` override the choice for one run; headless runs and `--self-test` use only the switches. Linux always uses Vulkan.

Other switches: `--spp N`, `--samples N`, `--no-vsync`, `--debug`, `--rr`, `--capture-ui`, `--self-test`, `--help`. `--preset` or `--settings` (give only one) is applied before every other switch, so the others always override it, in any order. `--headless` uses a hidden Win32 window and a real hardware D3D12 device; it requires `--frames N`. `--capture-ui` includes the controls in PNG captures; `--ui-tab scene|render|look|export`, `--ui-advanced`, `--ui-tool select|move|rotate|scale` and `--ui-select stage|camera|main-light|light:N|object:N` set up what they show. `--drag-test` moves object 0 every frame to measure the acceleration-structure rebuild a gizmo drag causes. `--debug` requires Windows Graphics Tools/debug layers and fails verification on D3D12 warnings/errors.

`--capture-sequence directory` captures the last 32 frames of a finite run for temporal image analysis. `--orbit-test` moves the camera by 0.25° per frame after a 64-frame warmup. The image-based RR regression renders a noise-free scene, emission-only walls and black emission-only blocks, so paths end at the first hit, to isolate reconstruction shimmer from path-tracing noise:

```powershell
python -m pip install numpy Pillow
python tools/rr_stability.py
```

`tools/upscale_quality.py` compares the upscalers at 50% render scale (Classic preset, 1280×720 output) against a 16,384-sample full-resolution reference. Values are RMSE ×1000 of compressed luminance with a 4-pixel rim around the light excluded: DLSS and XeSS blend that extreme HDR edge differently from a linear filter, and those 0.8% of pixels would otherwise dominate their score.

| 50% render scale | Still (160 frames) | Camera orbit (2 spp/frame) |
|---|---|---|
| Native 100%, for comparison | 1.05 | 4.06 |
| DLSS Ray Reconstruction | **1.90** | **2.60** |
| Intel XeSS | 2.27 | 4.70 |
| AMD FSR | 2.28 | 4.25 |
| DLSS Super Resolution | 2.62 | 3.82 |
| Bicubic | 4.15 | 5.91 |
| Bilinear | 4.70 | 6.01 |

At half resolution in each axis, RR beats full-resolution rendering in motion, and DLSS SR does too. On a still camera the temporal upscalers come within about 2× of full resolution while tracing a quarter of the rays.

`tools/exr.py` reads the EXR captures into numpy arrays (`exr.read(path)` returns an H×W×3 float32 array) for numerical comparisons.

`tools/motion_error.py` measures the native denoiser against a 16,384-sample reference of the Classic preset at 800×600. The error is RMSE ×1000 of compressed luminance l/(1+l); "blurred" applies a 5×5 blur first, which exposes bias and smearing.

| Camera orbit, 2 spp per frame (last of 124 frames) | Error | Blurred | Around the light |
|---|---|---|---|
| Raw samples | 48.0 | 11.6 | 21.2 |
| Previous filter + temporal | 20.7 | 16.9 | 36.9 |
| SVGF-style filter + temporal | **9.6** | **4.4** | **7.0** |

| Still camera | 4 spp | 16 spp | 64 spp | 256 spp | 1024 spp |
|---|---|---|---|---|---|
| Raw | 35.8 | 18.3 | 9.3 | 4.7 | 2.4 |
| Previous filter | 14.1 | 13.0 | 10.6 | 5.3 | 4.8 |
| SVGF-style filter | **10.0** | **6.3** | **4.1** | **2.4** | **1.6** |

The previous filter fed its own blurred output back as history, so it got worse than raw samples above about 64 spp. At 1440×900 and 4 spp on an RTX 4070 Ti SUPER, the Classic preset takes 1.6 ms raw, 3.1 ms with the native denoiser and 3.9 ms with DLSS RR (means over 200 frames).

The RR bridge converts ray-sampling offsets to NGX image displacements with opposite signs on both axes. Specular hit distances come from a stable primary reflection ray rather than a randomly selected BSDF lobe; this prevents false reflection-motion changes on stationary surfaces.

The integration self-test (285 scripted frames, then one per scene file: 450 frames) covers all 24 presets including the empty stage, scene edits, camera motion, pause/resume, accumulation reset rules, resize, changing render scale, native raw/filter paths, RR D/E/F switching, every upscaler and DLSS SR preset, HDR output checked pixel by pixel against a CPU copy of the transform, all AOVs, settings round-trip, A/B, every image format and the high-resolution still, keyframes and a motion-blurred timeline export, draws every tab and inspector with the advanced rows shown, draws each gizmo, runs a keyed G X 0.5 Enter and its undo, and then loads every scene file in `scenes/`. It also edits a live scene: removing every light, adding all eleven shapes, making one emissive, hiding it, adding, hiding and disabling an area light, deleting an object, picking in the viewport, framing and randomizing. Every frame it checks that the GPU's triangle and emitter counts match the current settings. `verify.ps1` additionally checks formatting, unit tests, Clippy, required RR creation/evaluation, DLAA and HDR EXR. It needs an RTX GPU; use unit tests and native CLI runs for non-NVIDIA DXR devices.

## Developer tools

- **Shader hot reload** (debug builds): save a file in `shaders/` and the running app recompiles every entry with the build's DXC and swaps the pipelines; a compile error shows the DXC message and keeps the old shaders. Release builds keep the embedded DXIL.
- `tools/perf_history.py`: benchmarks presets 0, 13, 17 and 21 with RR and with the native denoiser (1440×900, 4 spp, 200 frames), appends the means with the commit hash to `captures/perf-history.csv` and plots `captures/perf-history.png`.
- `tools/snapshot_build.ps1 -Name NAME -Notes TEXT`: copies the release exe, DLLs and scene packs into `builds/<date>-NAME/` with a `NOTES.md`, so older builds stay runnable side by side (`builds/` is not committed).

## Implementation map

| File | Responsibility |
|---|---|
| `src/main.rs` | Win32 window/event integration through winit, egui UI, controls, CLI, exports, edit tracking and integration self-test |
| `src/scene.rs` | Serializable settings, validation, Cornell-box presets, randomization, camera and light temperature |
| `src/presets.rs` | Studio presets 9–23, preset descriptions and the material library |
| `src/geometry.rs` | Cached shape meshes, scene assembly, framing, focus and viewport picking |
| `src/lighting.rs` | Packed GPU materials and the power-weighted light table (emissive triangles, spheres, suns) |
| `src/display.rs` | Windows display query (HDR state, calibrated luminance, SDR white level), HDR transform reference and HDR PNG writer |
| `src/hotreload.rs` | Debug-build shader hot reload: polls `shaders/`, recompiles with DXC |
| `src/look.rs` | .cube LUT reader and the colour-wheel widget |
| `src/walk.rs`, `src/video.rs`, `src/saver.rs`, `src/remote.rs` | Walk mode, video export, screensaver mode, phone remote |
| `src/video_ui.rs` | Video export dialog and progress window |
| `src/stats.rs` | Lifetime statistics and achievements (`%APPDATA%\Caustica\stats.json`) |
| `src/generators.rs` | Seeded scene generators (15 families) |
| `src/environment.rs` | HDRI loading and its importance-sampling tables |
| `src/procedural.rs` | Procedural meshes for generators: Mandelbulb, terrain, Menger sponge, trees |
| `src/mesh.rs` | OBJ and glTF/GLB import, cached per path, normalised to the shape box |
| `src/meshfmt.rs` | PLY and STL readers |
| `src/dirs.rs` | Where files go: installed / portable / development modes, per-user folders, first-start migration |
| `src/scenes.rs` | Scene files: pack listing over the shipped and user folders, loading (wrapper or bare settings, validated), saving to `scenes/user` |
| `src/gizmo.rs` | Transform gizmos: projection, handle layout and hit tests, drag and modal G / R / S math, light outlines and picking |
| `src/ui.rs` | Top bar, side panel tabs, status bar, toasts, theme; UI state kept out of `Settings` |
| `src/editor.rs` | Object, area-light and material inspectors |
| `src/teapot_data.rs` | Utah teapot patches, generated by `tools/build_teapot_data.py` |
| `src/gpu.rs` | Device/capabilities, swapchain, root signature, PSOs, BLAS/TLAS, material and light buffers, resources/barriers/fences, guide buffers, timestamps and readbacks |
| `shaders/pathtrace.hlsl` | DXR transport, light sampling, material and pattern evaluation, guide generation and accumulation |
| `shaders/post.hlsl` | Native temporal/spatial filter, AOV display, compositing and tone mapping |
| `shaders/gui.hlsl` | D3D12 egui rendering |
| `bridge/ngx.cpp` | NGX C ABI: capabilities, Ray Reconstruction and DLSS Super Resolution create/evaluate/release |
| `bridge/fsr.cpp` | AMD FSR through the FidelityFX API, loaded at runtime |
| `bridge/xess.cpp` | Intel XeSS Super Resolution, loaded at runtime |

All frame submissions are fenced before upload buffers or descriptors can be reused. Resize/shutdown also wait for work queued by Present. Texture state transitions bracket NGX input reads, copies, UI output and readbacks. The executable embeds its compiled DXIL shaders, so DXC is needed only at build time.

## Current limits

- The path tracer retains the original approximation for rough glass and does not importance-sample refractive caustics. GGX uses a single-scattering model. Every shape is a triangle mesh; spheres use 64×32 tessellation with smooth normals. The built-in sky gradient is not importance-sampled (HDRIs are). Procedural patterns are evaluated in world space, so they slide across an object that moves.
- The native sampler has Sobol scrambling but has no blue-noise texture. The native denoiser follows SVGF but at very low sample counts it can leave faint low-frequency blotches on flat walls.
- RR albedo guides use a material/Fresnel estimate; glass guides describe the primary glass surface. Neural reconstruction can leave artifacts in difficult glossy/refractive scenes, especially during disocclusion or at aggressive render scales.
- Direct/indirect sliders are exact AOV recomposition for raw native output. On filtered/RR output they apply the accumulated AOV ratio to reconstructed beauty; that operation is an approximation. Diagnostic AOVs remain progressive raw estimates.
- Memory scales with pixel count. Guides/history use FP32, and large output targets can allocate several GB. There is one frame in flight and no frame generation or Reflex; ReSTIR covers only first-bounce direct light. Imported meshes are part of the one scene BLAS, so moving one rebuilds it all.
- High-resolution export currently runs synchronously and restarts interactive accumulation. The side panel covers the right of the image (the render still fills the window); Tab gives a clean viewport. A/B references are invalidated by resize.

NVIDIA integration reference: [official DLSS SDK and integration guides](https://github.com/NVIDIA/DLSS). D3D12 bindings: [Microsoft windows-rs](https://github.com/microsoft/windows-rs).

Mesh credits and terms are in [assets/CREDITS.md](assets/CREDITS.md). The Utah teapot data comes from three.js (MIT, license in `assets/THREE-LICENSE.txt`). The Stanford bunny may be redistributed for research and non-commercial use with credit; commercial use needs Stanford's permission.
