# Caustica — Rust / Direct3D 12 and Vulkan

Caustica is a path tracer on hardware ray tracing for Windows (Direct3D 12 or Vulkan) and Linux (Vulkan); it began as a Cornell box renderer. The renderer and application are Rust; path tracing and postprocessing are HLSL, compiled to DXIL and SPIR-V. Small C++ C-ABI bridges call NVIDIA's NGX SDK, AMD FSR and Intel XeSS.

Startup defaults favor image quality: **native-resolution DLAA with Ray Reconstruction and 4 samples per frame**. Select Quality/Balanced/Performance explicitly when you want a lower internal resolution.

## Run

Double-click **`launch.cmd`**, or run `target/release/caustica.exe`. The launch script builds the application if the release executable is missing.

For a clean checkout:

```powershell
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

The app looks for a newer release at startup (Menu → Settings → *Check for updates at startup* turns it off) and can download, verify and install it: through the setup on Windows, by replacing the AppImage on Linux. Other copies get a link to the release page. Third-party terms are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

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
.\target\release\caustica.exe --headless --frames 64 --preset 12 --set bounces=20 --set auto_exposure=false --set exposure=1.3 --capture teapots.png
.\target\release\caustica.exe --settings settings.json

# Full verification; uses target/validation so an open development build can stay running.
powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify.ps1
```

**Renderer:** Menu → Renderer picks D3D12 or Vulkan for the next start (saved with the preferences; *Restart now* relaunches with the same command line). If Vulkan was chosen there and can't start, the app falls back to D3D12 and the menu says why. `--vulkan` and `--d3d12` override the choice for one run; headless runs and `--self-test` use only the switches. Linux always uses Vulkan.


The feature tour, the controls, the switches and measurement notes, developer tools and the implementation map are in [docs/features.md](docs/features.md).

## Current limits

- By default the path tracer keeps the original approximation for rough glass (Render tab → Advanced → "Exact rough glass", `--set exact_glass=true`, renders it as the exact GGX BTDF with light sampling, about 25% slower on glass scenes) and does not importance-sample refractive caustics. GGX uses a single-scattering model. Every shape is a triangle mesh; spheres use 64×32 tessellation with smooth normals. The built-in sky gradient is not importance-sampled (HDRIs are). Procedural patterns are evaluated in world space, so they slide across an object that moves.
- The native sampler has Sobol scrambling but has no blue-noise texture. The native denoiser follows SVGF but at very low sample counts it can leave faint low-frequency blotches on flat walls.
- RR albedo guides use a material/Fresnel estimate; glass guides describe the primary glass surface. Neural reconstruction can leave artifacts in difficult glossy/refractive scenes, especially during disocclusion or at aggressive render scales.
- Direct/indirect sliders are exact AOV recomposition for raw native output. On filtered/RR output they apply the accumulated AOV ratio to reconstructed beauty; that operation is an approximation. Diagnostic AOVs remain progressive raw estimates.
- Memory scales with pixel count. Guides/history use FP32, and large output targets can allocate several GB. There is one frame in flight and no frame generation or Reflex; ReSTIR covers only first-bounce direct light. Imported meshes are part of the one scene BLAS, so moving one rebuilds it all.
- High-resolution export restarts interactive accumulation. The right column and the tool rail cover the edges of the image (the render still fills the window); Tab gives a clean viewport. A/B references are invalidated by resize.

NVIDIA integration reference: [official DLSS SDK and integration guides](https://github.com/NVIDIA/DLSS). D3D12 bindings: [Microsoft windows-rs](https://github.com/microsoft/windows-rs).

Mesh credits and terms are in [assets/CREDITS.md](assets/CREDITS.md). The Utah teapot data comes from three.js (MIT, license in `assets/THREE-LICENSE.txt`). The Stanford bunny may be redistributed for research and non-commercial use with credit; commercial use needs Stanford's permission.
