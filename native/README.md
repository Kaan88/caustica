# Cornell RTX — Rust / Direct3D 12

A native Windows port of the Cornell box path tracer. The renderer and application are Rust; path tracing and postprocessing are HLSL. A small C++ C-ABI bridge calls NVIDIA's NGX SDK. The original WebGL application remains in the parent folder.

Startup defaults favor image quality: **native-resolution DLAA with Ray Reconstruction and 4 samples per frame**. Select Quality/Balanced/Performance explicitly when you want a lower internal resolution.

## Run

Double-click **`../launch-native.cmd`**, or run `target/release/cornell-rtx.exe`. The launch script builds the application if the release executable is missing.

For a clean checkout:

```powershell
cd native
powershell -NoProfile -ExecutionPolicy Bypass -File tools/setup.ps1
cargo run --release --locked
```

Build requirements: Windows x64, Rust MSVC (Rust 1.88 or newer), Visual Studio C++ Build Tools, Git, and a Windows SDK containing DXC. `build.rs` discovers DXC in the Windows Kits directory; override with `DXC_PATH` if necessary. Runtime requirements: D3D12 feature level 12.1 and DXR tier 1.1. DLSS RR additionally requires a supported NVIDIA RTX GPU/driver. Capability detection reports actual NGX initialization, creation and evaluation results; the native denoiser remains available when RR is unavailable. `--require-rr` makes missing or failed RR a fatal error for verification.

Setup fetches NVIDIA's official DLSS **310.9.16** SDK pinned to commit `9981c6350dc58c0ce21f024fd65e31e851c31689`. Cargo copies `nvngx_dlss.dll` and `nvngx_dlssd.dll` next to the executable. No DLL downloads occur during application startup. NVIDIA's SDK and runtime retain their own license, available in `vendor/dlss/LICENSE.txt`; the bridge uses a stable local-development project GUID. Replace it with your registered project identity before distribution.

## Rendering

- **Hardware ray tracing:** triangle BLAS and TLAS, built with `ID3D12Device5`; HLSL `RayQuery` traversal for every primary, bounce and shadow ray. This uses DXR tier 1.1 inline ray tracing, including the GPU's acceleration-structure traversal hardware. No software intersection loop or WebGL renderer is involved.
- **Path tracing:** progressive FP32 HDR accumulation, per-sample Welford luminance variance, next-event estimation over every emissive triangle (power-weighted, so area lights and emissive meshes share one sampler), balance-heuristic MIS, Russian roulette, and an optional firefly clamp.
- **Scenes:** up to 256 objects and 32 extra area lights, plus the main ceiling light. Eleven shapes: box, sphere, cylinder, cone, torus, trefoil knot, Utah teapot (tessellated from Martin Newell's Bézier patches), Stanford bunny (the 69,451-triangle scan), octahedron, chess pawn and thin ring. Each object has yaw/pitch/roll, nonuniform scale, and either rests on the floor or floats at a set height. The stage can be the Cornell enclosure or an open floor of any size with sky light. Lights can be hidden from the camera while still lighting the scene and appearing in reflections.
- **Materials:** Lambertian/GGX reflection, dielectric Fresnel, refraction, total internal reflection, Beer–Lambert absorption, metallic color, roughness, reflectivity, IOR, transmission and emission, with optional emission-only (unlit) surfaces. Procedural checker, stripe and marble patterns, and a 12-entry material library (porcelain, chrome, gold, copper, glasses, jade, lacquer, marble, emitters).
- **NVIDIA DLSS Ray Reconstruction:** NGX feature `RayReconstruction`, created/evaluated through the official DLSSD helpers. It consumes fresh noisy HDR samples, diffuse/specular albedo, world normals plus packed roughness, linear view depth, current-to-previous pixel motion, primary-surface specular hit distance and camera matrices. RR receives neither progressively accumulated color nor the native denoiser's output. Jitter uses a 64-phase Halton sequence, with deterministic guide surfaces for each frame. Camera motion preserves neural history; geometry/material/light edits reset it.
- **DLSS quality:** DLAA at 100% internal resolution, Quality, Balanced, Performance and Ultra Performance, plus custom render scale. Models: NVIDIA default, Transformer D, Transformer E, and RR2/F. RR2/F is the default. Quality/model changes recreate the NGX feature with completed GPU work.
- **Native denoising (SVGF-style):** raw samples are integrated over time in albedo-demodulated space, then filtered by five à-trous passes guided by object id, normal, plane distance and a Gaussian-prefiltered luminance variance. On a camera move, the previous integration is reprojected bilinearly; each tap is checked by object id, normal and depth. While the camera is still, that history stays frozen and fades out over 256 fresh samples, so samples are never counted twice. Mirror-like pixels keep at most 2 samples of history. The history never feeds the unbiased progressive accumulation. Native rendering offers padded 2D Owen-scrambled Sobol sampling or independent random sampling. RR deliberately uses independent random path samples, following NVIDIA's noise guidance.
- **Output:** eight views (beauty, normals, diffuse albedo, depth, direct, indirect, specular, convergence), six tone maps, exposure/gamma, direct/indirect relighting and a frozen linear-HDR A/B reference.
- **Native controls:** egui rendered directly into the D3D12 swapchain, 24 scene presets, randomization, camera controls, validated JSON save/load, GPU timestamp timing, pause/convergence and resize support. A separate **Scene objects** window lists, searches, adds, duplicates, deletes, hides and focuses objects and edits their transform and material; click an object in the viewport to select it. The Lighting section edits the main light and up to 32 extra area lights. Material and light edits update the sampling tables without rebuilding the acceleration structures.
- **Presets:** 0–8 are the Cornell-box studies (classic, reflective spheres, sphere and box, mirror + glass, mirror, glass, warm, dramatic, emissive spheres). 9–23 are open studios: material matrix, weekend sphere field, Whitted checkerboard, Utah teapot studio, Stanford bunny studio, glass laboratory, RGB shadow study, sphere pyramid, mirror corridor, knot gallery, column courtyard, chess still life, neon sculpture, orbital sculpture, and an empty stage to build on. Switching between Cornell-box presets keeps your camera; any other switch uses the preset's own view. Randomize picks any preset except the empty stage: inside the Cornell box it reshapes and recolors the blocks, elsewhere it keeps the layout and recolors diffuse and glass objects.
- **Captures:** PNG and linear RGB32F OpenEXR. The high-resolution button renders fresh paths at the export resolution (up to 4096 on the longest side), with a chosen sample budget; it then restarts the interactive render at its previous resolution. CLI exports can render up to 8192×8192, subject to available GPU memory.

## Controls

| Input | Action |
|---|---|
| Left click | Select the object under the cursor |
| Left drag | Orbit |
| Right drag / Shift-drag | Pan |
| Wheel | Dolly |
| F | Frame all objects |
| Delete | Delete the selected object |
| Space | Pause/resume |
| R | Reset accumulation |
| A | Auto orbit |
| D | Toggle native denoiser (RR has a separate toggle) |
| C | Capture A/B reference |
| 1–8 | Select view |
| F12 | Save `cornell.png` |
| Tab | Hide/show controls |
| Escape | Close |

Save/load uses `settings.json` in the working directory. PNG/EXR buttons write `cornell.png` / `cornell.exr`; high-resolution export writes `cornell-high-res.png`. These are native settings; the browser JSON schema is separate. All material colors are linear RGB. Lighting temperature is normalized to unit luminance, then multiplied by the gel and intensity.

## Offline rendering and verification

From `native/`:

```powershell
# 4K output, RR quality, 256 frames; omit --headless to watch the render.
.\target\release\cornell-rtx.exe --headless --width 3840 --height 2160 --scale 0.666667 --frames 256 --require-rr --capture cornell-4k.png

# Native-resolution DLAA and linear HDR output.
.\target\release\cornell-rtx.exe --headless --width 1920 --height 1080 --scale 1 --frames 128 --require-rr --capture cornell.exr

# Unfiltered progressive DXR reference: 1024 frames × 2 spp = 2048 spp.
.\target\release\cornell-rtx.exe --headless --width 960 --height 720 --scale 1 --frames 1024 --samples 2048 --no-rr --raw --capture reference.exr

# Preset indices 0–23 follow the UI order (--help lists them). Native settings can be loaded
# from a file; --set then overrides single top-level settings with JSON values.
.\target\release\cornell-rtx.exe --headless --frames 64 --preset 12 --set bounces=20 --set exposure=1.3 --capture teapots.png
.\target\release\cornell-rtx.exe --settings settings.json

# Full verification; uses target/validation so an open development build can stay running.
powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify.ps1
```

Other switches: `--spp N`, `--samples N`, `--no-vsync`, `--debug`, `--rr`, `--capture-ui`, `--self-test`, `--help`. `--preset` or `--settings` (give only one) is applied before every other switch, so the others always override it, in any order. `--headless` uses a hidden Win32 window and a real hardware D3D12 device; it requires `--frames N`. `--capture-ui` includes the controls in PNG captures. `--debug` requires Windows Graphics Tools/debug layers and fails verification on D3D12 warnings/errors.

`--capture-sequence directory` captures the last 32 frames of a finite run for temporal image analysis. `--orbit-test` moves the camera by 0.25° per frame after a 64-frame warmup. The image-based RR regression renders a noise-free scene, emission-only walls and black emission-only blocks, so paths end at the first hit, to isolate reconstruction shimmer from path-tracing noise:

```powershell
python -m pip install numpy Pillow
python tools/rr_stability.py
```

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

The 180-frame integration self-test covers all 24 presets including the empty stage, scene edits, camera motion, pause/resume, accumulation reset rules, resize, changing render scale, native raw/filter paths, RR D/E/F switching, all AOVs, settings round-trip, A/B and high-resolution PNG export. It also edits a live scene: removing every light, adding all eleven shapes, making one emissive, hiding it, adding, hiding and disabling an area light, deleting an object, picking in the viewport, framing and randomizing. Every frame it checks that the GPU's triangle and emitter counts match the current settings. `verify.ps1` additionally checks formatting, unit tests, Clippy, required RR creation/evaluation, DLAA and HDR EXR. It needs an RTX GPU; use unit tests and native CLI runs for non-NVIDIA DXR devices.

## Implementation map

| File | Responsibility |
|---|---|
| `src/main.rs` | Win32 window/event integration through winit, egui UI, controls, CLI, exports, edit tracking and integration self-test |
| `src/scene.rs` | Serializable settings, validation, Cornell-box presets, randomization, camera and light temperature |
| `src/presets.rs` | Studio presets 9–23, preset descriptions and the material library |
| `src/geometry.rs` | Cached shape meshes, scene assembly, framing, focus and viewport picking |
| `src/lighting.rs` | Packed GPU materials and the power-weighted emissive-triangle table |
| `src/editor.rs` | Scene objects window and area-light editor |
| `src/teapot_data.rs` | Utah teapot patches, generated by `tools/build_teapot_data.py` |
| `src/gpu.rs` | Device/capabilities, swapchain, root signature, PSOs, BLAS/TLAS, material and light buffers, resources/barriers/fences, guide buffers, timestamps and readbacks |
| `shaders/pathtrace.hlsl` | DXR transport, light sampling, material and pattern evaluation, guide generation and accumulation |
| `shaders/post.hlsl` | Native temporal/spatial filter, AOV display, compositing and tone mapping |
| `shaders/gui.hlsl` | D3D12 egui rendering |
| `bridge/ngx.cpp` | NGX capability/init/create/evaluate/release C ABI |

All frame submissions are fenced before upload buffers or descriptors can be reused. Resize/shutdown also wait for work queued by Present. Texture state transitions bracket NGX input reads, copies, UI output and readbacks. The executable embeds its compiled DXIL shaders, so DXC is needed only at build time.

## Current limits

- The path tracer retains the original approximation for rough glass and does not importance-sample refractive caustics. GGX uses a single-scattering model. Every shape is a triangle mesh; spheres use 64×32 tessellation with smooth normals. Sky light is not importance-sampled, so open scenes lit mainly by the sky converge more slowly. Procedural patterns are evaluated in world space, so they slide across an object that moves.
- The native sampler has Sobol scrambling but does not carry over the browser's generated blue-noise texture. The native denoiser follows SVGF but keeps nearest-neighbor upscaling when the render scale is below 1 without RR; at very low sample counts it can leave faint low-frequency blotches on flat walls.
- RR albedo guides use a material/Fresnel estimate; glass guides describe the primary glass surface. Neural reconstruction can leave artifacts in difficult glossy/refractive scenes, especially during disocclusion or at aggressive render scales.
- Direct/indirect sliders are exact AOV recomposition for raw native output. On filtered/RR output they apply the accumulated AOV ratio to reconstructed beauty; that operation is an approximation. Diagnostic AOVs remain progressive raw estimates.
- Memory scales with pixel count. Guides/history use FP32, and large output targets can allocate several GB. There is one frame in flight and no frame generation, Reflex, ReSTIR, texture importer, or mesh importer.
- High-resolution export currently runs synchronously and restarts interactive accumulation. The GUI is a movable overlay; Tab gives a clean viewport. A/B references are invalidated by resize.

NVIDIA integration reference: [official DLSS SDK and integration guides](https://github.com/NVIDIA/DLSS). D3D12 bindings: [Microsoft windows-rs](https://github.com/microsoft/windows-rs).

Mesh credits and terms are in [assets/CREDITS.md](assets/CREDITS.md). The Utah teapot data comes from three.js (MIT, license in `assets/THREE-LICENSE.txt`). The Stanford bunny may be redistributed for research and non-commercial use with credit; commercial use needs Stanford's permission.
