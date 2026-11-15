## The Eyes look (E4, 2026-11-10)

Scotopic vision for the display, off for old files and on in new scenes (`eyes`, `eyes_strength`, display-only, outside `transport_key`). GPU in `post.hlsl` (`eyes_mix`, `eyes`, applied to scene light right before the exposure in `tonemap` and both `hdr_map` paths); CPU twin `reference/display.rs` (`eyes_mix`, `eyes_look`, before the exposure in `display_exposed`). The strength travels in `Constants.meter.w` (0 when off), so no new cbuffer field.

- Adapted luminance: middle grey over the metered exposure without the bias (`metered_exposure()` on the GPU, `Adaptation::metered` / `still_exposures` on the CPU; the manual exposure when auto is off or before a reading), times `SCENE_CD_M2`. So the eye follows the meter's adaptation time constants, and the bias stays a look that doesn't change how dark the eye thinks it is.
- Mix: 0 at or above 3 cd/m², smoothstep in log10 luminance to 1 at 0.01 cd/m², times `eyes_strength`. At 0 the shader returns the input untouched, so a daylight image is bit-identical with the Eyes on or off (test `eyes_leave_daylight_bit_identical_and_desaturate_the_night`).
- Rod vision: scotopic luminance from linear Rec. 709 with weights (0.04, 0.53, 0.43) (a V'(λ) approximation: rods peak near 507 nm, red barely counts; they sum to 1 so white keeps its level), shown in a Purkinje tint (0.80, 1.00, 1.30) normalised to luminance 1. `c + (tint·rod − c)·mix`.
- Defaults chosen without the owner (asked not to ask), all changeable: the 3 / 0.01 cd/m² bounds, the rod weights, the tint, the smoothstep, using the metered rather than the displayed exposure. Not modelled: loss of acuity (scotopic blur) and rod noise.
- Open: GPU vs CPU agreement under `tools/oracle.py` on a night scene; the D3D12 side is not compiled on Linux (Vulkan uses the same HLSL).

