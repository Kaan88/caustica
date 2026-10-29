# Cornell RTX roadmap

The single ordered plan. Details live in the two research documents:
[ui-plan.md](ui-plan.md) (UI redesign) and [scene-research.md](scene-research.md) (scenes).
Written 2026-10-30. Nothing here is implemented yet.

## Ground rules from the owner

- **Personal project, never published.** License terms don't constrain anything: use any model, scene, HDRI or texture, including research-only, NC, ND and SA assets, Sponza, the Stanford scans and Disney data. Assets may be downloaded into the project, or into a local `assets/` folder, as convenient. The licensing cautions in scene-research.md are kept only as background.
- Credits stay in `assets/CREDITS.md` out of courtesy, not obligation.

## Near term

### 1. DLSS preset selection (small)

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

### 2. Native HDR output, with settings taken from Windows

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

### 3. UI phase 1 (from ui-plan.md section 8)

Regroup and relabel only. Include the DLSS preset combo and the HDR status line from items 1–2.

### 4. Scenes step 1 (from scene-research.md section 5)

- Add a `scenes/` folder of JSON scene files and a Scenes window.
- Write the 42 buildable presets as files.

Because licensing doesn't matter, the import steps that follow can start with the best-known scenes, not the CC0-safe ones: Sponza, San Miguel, Bistro, Bitterli's whole set (including the ND/NC ones), the Stanford dragon, Buddha and Lucy, and Disney's Moana island if memory allows.

## Medium term

Order is from scene-research.md section 5, unchanged:
1. Generators.
2. Mesh import (OBJ, then glTF) with an opaque/non-opaque geometry split.
3. HDRI environments with importance sampling.
4. Multiple BLAS + TLAS instancing past 256 objects.
5. Textures.
6. UI phases 2–5.

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
