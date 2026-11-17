# D3D12 phase-out (2026-11-17)

Plan and gates: `docs/roadmap.md` "D3D12 phase-out". This note keeps what was built and measured for it.

## Phase 0: making the comparison possible

- **Backend switches on the tools.** `--backend d3d12|vulkan` on `oracle.py` (GPU side only; the CPU side has no renderer), `perf_history.py`, `rr_stability.py`, `motion_error.py` and `upscale_quality.py`; `-Backend` on `verify.ps1` and `perf_baseline.ps1`. Each adds `--d3d12` or `--vulkan` to every launch of the exe, the cached reference renders included. Without one, a tool runs the exe's own choice (headless runs and the self-test ignore Menu → Renderer, so that is D3D12 on Windows and Vulkan on Linux). Tools that cache references in their output folder (`motion_error`, `upscale_quality`, `rr_stability`) get a `-<backend>` suffix on the default folder, so one renderer never scores against the other's reference. `perf_history.py` tags the commit column `<hash>@<backend>` from the renderer RENDER_OK reports (so an untagged Linux run and an untagged Windows run never share a series), which keeps the CSV schema and plots the two as separate points. `upscale_quality.py` skips FSR on Vulkan (no Vulkan path; owner, 2026-11-10).
- **`backend=` on `RENDER_OK`** (`Renderer::is_vulkan`). A run that asked for Vulkan and fell back would otherwise look like a pass; `backend_compare.py` fails such a run.
- **`tools/backend_compare.py`**: each scene on both renderers from one exe, raw accumulation at scale 1, `--no-vsync`, the display pinned and the clock frozen (`--time 0`). The two EXRs go through the oracle's statistical test (`oracle.test`: image mean within z 4 or 0.2%, no 8 x 8 block off) with the ratio printed as D3D12 / Vulkan, plus the RMSE x1000 of compressed luminance; timings are the RENDER_OK means. `--rr` adds an RR run per backend for timings only, with `--require-rr` so a failed RR shows as nan, not as the native denoiser's time.
- **The Vulkan validation layer on Windows.** Khronos's stable releases carry Android binaries only, so it comes from the LunarG SDK: `setup.ps1 -VulkanLayer` downloads the signed installer (SHA-256 pinned to LunarG's published hash) and runs it silently with `copy_only=1` into `vendor/vulkan-sdk/<version>`, which leaves the registry, PATH and `VULKAN_SDK` alone. The loader finds it through `VK_ADD_LAYER_PATH`, which `verify.ps1` sets, except when the process is elevated: an elevated loader ignores layer-path variables and reads only `HKLM\SOFTWARE\Khronos\Vulkan\ExplicitLayers`. Claude Code runs elevated on this machine, so `setup.ps1 -RegisterVulkanLayer` adds one value there pointing at the repo's manifest (owner's choice, 2026-11-17); an explicit layer loads only when an app asks for it.

## A one-off device loss under the layer (not explained)

The first `--vulkan --debug` run with the layer (2026-11-17, release exe, 640 x 400, 16 frames, RR on) lost the device just after the shader compile, with no validation message before it. It has not happened since: the same command ran clean 10 times out of 10 the same day, five with the layer's handle wrapping on (`VK_KHRONOS_VALIDATION_UNIQUE_HANDLES=true`) and five with it off. A first guess, that the layer's handle wrapping broke NGX's direct Vulkan calls, was wrong: the setting tried for it (`handle_wrapping`) is not the layer's key (that is `unique_handles`, "Handle Wrapping" in the manifest), the layer ignored it, and the runs passed anyway, so the change was reverted. If it comes back, note whether it is the first run after a driver, layer or pipeline-cache change.

## Phase 1, first finding: the self-test under the layer

`verify.ps1 -Release -Backend vulkan` (2026-11-17) fails in the self-test at frame 174 (Chess still life) on validation messages. Three families:
1. `vkCreateShaderModule`: SPIR-V declares `DotProduct` / `DotProductInput4x8BitPacked` without `shaderIntegerDotProduct` enabled on the device.
2. `vkQueueSubmit2`: a command buffer expects an image in `SHADER_READ_ONLY_OPTIMAL` and it is in another layout.
3. `VK_DESCRIPTOR_TYPE_MUTABLE_EXT` used without `mutableDescriptorType` enabled.
The D3D12 `verify.ps1 -Release` passes (oracle included, 415 unit tests). These are Phase 1's first row ("484 frames clean with the layer on").

All three came from XeSS on Vulkan, not from the renderer's own code (which uses neither integer dot products nor mutable descriptors):
- `src/vulkan.rs` now enables `shaderIntegerDotProduct` and, when `VK_EXT_mutable_descriptor_type` is among the enabled extensions (XeSS asks for it), `mutableDescriptorType`, each where the device has it. `xessVKGetRequiredDeviceFeatures` would patch the chain itself, but it can't patch one that sets `pEnabledFeatures`, as ours does.
- `src/vulkan/gpu/upscale.rs` moves XeSS's color, depth and motion inputs to `SHADER_READ_ONLY_OPTIMAL` around `xessVKExecute` and back to `GENERAL` after: `xess_vk.h` requires that layout for its inputs (only the output is `GENERAL`), while the bridge comment said all four were `GENERAL`, which holds for NGX only.

After the fixes (2026-11-17): `verify.ps1 -Release -Backend vulkan` passes, self-test clean under the layer, oracle GPU/CPU 0.9996 / 1.0061 / 0.9861; `verify.ps1 -Release` (D3D12) passes unchanged.

## Upscaler quality on both renderers (2026-11-17)

`tools/upscale_quality.py`, release exe, Classic preset at 1280 x 720, render scale 0.5 (RMSE x1000 of compressed luminance / without the light rim / blurred / sharpness):

| case | D3D12 still | Vulkan still | D3D12 orbit | Vulkan orbit |
|---|---|---|---|---|
| native 100 % | 1.19 / 0.96 / 0.53 / 0.84 | 1.19 / 0.96 / 0.53 / 0.84 | 6.37 / 3.32 / 2.68 / 0.87 | 6.23 / 3.37 / 2.68 / 0.87 |
| bilinear | 9.10 / 5.09 / 4.69 / 0.79 | 9.09 / 5.08 / 4.68 / 0.79 | 14.50 / 5.80 / 7.74 / 0.80 | 14.35 / 5.78 / 7.54 / 0.81 |
| bicubic | 7.50 / 4.62 / 3.15 / 0.81 | 7.50 / 4.61 / 3.15 / 0.81 | 13.19 / 5.83 / 6.80 / 0.83 | 13.05 / 5.80 / 6.62 / 0.83 |
| DLSS SR | 6.29 / 4.06 / 3.09 / 0.81 | 6.22 / 3.97 / 3.06 / 0.81 | 5.68 / 3.90 / 3.42 / 0.85 | 5.64 / 3.85 / 3.40 / 0.85 |
| FSR | 11.07 / 5.57 / 4.04 / 0.95 | (no FSR on Vulkan) | 9.22 / 4.52 / 4.78 / 0.83 | |
| XeSS | 11.19 / 5.05 / 4.82 / 0.82 | 11.18 / 5.06 / 4.82 / 0.82 | 6.41 / 4.81 / 3.44 / 0.85 | 6.40 / 4.80 / 3.44 / 0.85 |
| DLSS RR | 3.44 / 2.16 / 1.73 / 0.83 | 3.79 / 2.18 / 1.91 / 0.83 | 4.91 / 2.83 / 2.35 / 0.84 | 4.91 / 2.84 / 2.35 / 0.84 |

The two renderers match within a few percent everywhere but DLSS RR on the still camera, 10 % worse on Vulkan (3.79 against 3.44; without the light rim it is 2.18 against 2.16, so most of the difference sits at the light's edge, where pre-exposure matters): the lead for the RR row of phase 1 ("pre-exposure unproven on Vulkan"). XeSS's still score being worse than bilinear is the same on both and older than the layout fix. These tables are no longer comparable with the README's older upscaler table (different code, references and metric revisions).


## First comparison table (2026-11-17)

`tools/backend_compare.py`, release exe, 960 x 600, 256 frames x 4 spp, RTX 4070 Ti SUPER. Every scene agrees within noise (ratio D3D12 / Vulkan, z, worst 8 x 8 block, RMSE x1000 of compressed luminance); timings are `gpu_ms_mean` / `trace_ms_mean` in ms, raw accumulation, one run each, so they are a first look, not the phase 2 gate:

| scene | D3D12/VK | z | worst block | RMSE | D3D12 gpu/trace | Vulkan gpu/trace |
|---|---|---|---|---|---|---|
| preset:0 | 1.0000 | -0.28 | 0.9929 | 3.02 | 1.53 / 1.43 | 1.35 / 1.28 |
| preset:13 | 0.9999 | -0.93 | 0.9991 | 4.65 | 1.55 / 1.47 | 1.42 / 1.34 |
| preset:17 | 1.0000 | -0.43 | 0.9972 | 8.17 | 2.47 / 2.38 | 2.00 / 1.94 |
| preset:21 | 0.9999 | -2.29 | 0.9998 | 1.32 | 1.51 / 1.44 | 1.41 / 1.35 |
| sponza | 0.9993 | -3.90 | 0.9967 | 0.86 | 7.14 / 7.07 | 10.44 / 10.37 |
| stylised-forest | 1.0000 | 1.62 | 0.9996 | 1.52 | 1.58 / 1.51 | 1.46 / 1.39 |
| glass-of-water | 0.9997 | -1.33 | 0.9950 | 18.53 | 4.95 / 4.88 | 4.66 / 4.60 |
| fairy-lights | 1.0005 | 6.73 | 1.0021 | 5.40 | 16.48 / 16.41 | 5.21 / 5.15 |
| veach-ajar (BDPT) | 1.0000 | -1.20 | 1.0003 | 0.29 | 75.59 / 75.53 | 107.12 / 107.06 |
| cornell-caustic (photon) | 1.0000 | 1.20 | 0.9999 | 0.09 | 3.52 / 3.44 | 3.83 / 3.77 |
| blackbody-forge (spectral) | 0.9999 | -0.87 | 0.9982 | 0.94 | 3.68 / 3.60 | 4.17 / 4.09 |
| hdri-noon-spheres | 0.9996 | -0.32 | 1.0000 | 10.31 | 1.74 / 1.67 | 8.40 / 8.32 |

fairy-lights' z 6.7 passes only through the 0.2 % floor (ratio 1.0005); worth a second look at more samples. Read as leads for phase 2: Vulkan is slower on Sponza (+46 %), veach-ajar BDPT (+42 %), the HDRI scene (4.8x) and the spectral scene (+13 %), and faster on fairy-lights (3.2x less GPU time, ReSTIR on there by default) and slightly faster on the simple presets. None of these causes is traced yet; the Sponza gap matches the missing opacity micromaps in the phase 1 table.
