## Merge of the six lanes and its fixes (2026-11-10)

Headless, OIDN, egui 0.37 and the Windows check tools went in first, then bounces, accuracy and exposure by a three-way git merge (local branch `merge-int`; each lane's state on `merge-in-*`). What only the combination needed:
- **Specular budget and glass:** a glass vertex (GPU lobes 3 and 4, the CPU's dielectric branch) counts against `specular_bounces` only at roughness <= 0.1, like glossy, whichever glass model traced it. Before, the lerp model counted rough glass at any roughness and the exact BTDF (`exact_glass`, non-delta with glass NEE) never did. Changes nothing at the default budget 0.
- **Spectral BDPT and photon mapping were 3x** with a dispersive lens (street-bokeh: 2.990x and 2.985x the path tracer): the camera took the hero wavelength's 3 and `hero_rgb` took it again. Now the camera only selects the channel when the hero is already on (`camera_ray`: `hero_only ? (1,0,0) : (3,0,0)`); `photon_gather` starts its hero before the camera ray, as `bdpt` did, and the CPU reference marks the hero taken before its camera ray for BDPT and photon (`progressive.rs`), so both sides take the 3 once, in the output. The path tracer is unchanged.
- **Auto exposure after a resize:** the meter texture is recreated empty, and the low-sample Hold had nothing to hold, so the display showed the manual exposure for about three frames. A Hold with no state now reads (`main.rs`, the meter op).
- **CPU OIDN stills meter the unfiltered mean** (`reference::run`, `video::write_cpu_image_exposed`), as the GPU meters before `denoise_capture` filters the file.
- **Merge build fixes:** egui 0.37's `egui_is_using_pointer` (the meter hold while dragging), the D3D12 `GpuState` initializer's exposure fields, and Windows-only dead code put under `cfg(test)`. The D3D12 side still needs a Windows run (docs/claude/windows-handoff.md).


