# Setup: stack, vendor SDKs, defaults, machine

Moved out of `CLAUDE.md` on 2026-11-02 (unchanged text). Paths are relative to the repo root. Where this file says "see X", X is a section of `docs/engine-notes.md`.

## What it is

- Caustica: a Windows-only path tracer that began as a Cornell box renderer.
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
- **Public since 2026-11-06:** the code is MIT; what ships keeps its own terms (`THIRD-PARTY-NOTICES.md`, ground rules in `docs/roadmap.md`).
  - Near term: DLSS presets, native HDR, UI phase 1, Blender-style gizmos and scene files (42 scenes) are done (2026-10-30); next are the quick wins (hot reload, debug views, depth of field, portals).
  - Medium term: the core engine track (generators → mesh import → HDRI → instancing → textures → UI), then feature tracks 7–20 (lights, animation, volumes, SDFs, path visualizer, integrators, physics, SER).
  - Future: Vulkan, software fallback / CPU reference, spectral, curved space, frame generation, neural radiance cache, splats, VR.
- **Git:** the repo root is the local folder `cornell/` (it kept the old name), with a private GitHub remote, `Kaan88/caustica`. `main` is the verified state. Commit and push only when the user asks.

### Machine (2026-10-29)

- Hardware and drivers: RTX 4070 Ti SUPER 16 GB, driver 616.64, Windows 11.
- Toolchain: Rust 1.99.1 MSVC, DXC from Windows SDK 10.0.26100, Python 3.13 with numpy and Pillow (for `tools/rr_stability.py`).
