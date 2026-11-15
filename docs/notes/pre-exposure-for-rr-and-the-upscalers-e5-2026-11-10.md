## Pre-exposure for RR and the upscalers (E5, 2026-11-10)

Night radiance (1e-9..1e-5 scene units) underflows the fp16 inside DLSS RR, DLSS SR, FSR and XeSS (our textures are RGBA32F; the libraries are not). Each frame takes a pre-exposure P from the previous frame's displayed exposure (`GpuState::metered` read back with one frame in flight, `Adaptation::exposure`, so the manual exposure when auto is off), and:

- `render::pre_exposure`: P is a power of two, rounded from log2 of the exposure, kept while the exposure stays within `PRE_EXPOSURE_HYSTERESIS` = 1 stop of it, clamped to 2^±40. Powers of two make the scale and its inverse exact in fp32, so the input textures come back bit-identical.
- Scale passes: the `display` entry with `meter_ctrl.x` bit 3 (`scale_constants`: texture in `y`, factor bits in `w`; rgb only, so accumulation's M2 alpha is untouched), constant slots 24–26 (`PRE_EXPOSURE_SLOT`, `CB_SLOTS` 27). Before the reconstruction the colour input (texture 0 for RR, 8 or 1 for the upscalers) is multiplied by P; after it the input is divided back and texture 14 is divided by P. Every later reader (display, meter, scopes, light meter / probe, captures, EXR via `linear_output`, oracle readbacks) therefore sees scene units, and nothing runs at P = 1. Both backends (`Gpu::pre_expose` in `gpu/frame.rs` and `vulkan/gpu/upscale.rs`).
- Bridges: `InPreExposure = P` for DLSSD (ngx.cpp, vk.cpp) and DLSS SR, `preExposure = P` for FSR, so their histories rescale when P steps. XeSS has no pre-exposure input (its `exposureScale` multiplies the colour), so it stays 1 and only the data is scaled: a P step can show for a few frames in XeSS history (open). `InExposureScale` stays 1 and the SR auto-exposure flags stay.
- The NGX jitter rule is unchanged. `CAUSTICA_PRE_EXPOSURE=0` forces P = 1 for before/after measurements.
- Defaults chosen without the owner (asked not to ask), all changeable: power-of-two rounding, 1-stop hysteresis, the ±40 stop clamp, the previous frame's displayed (biased) rather than metered exposure, keeping XeSS at exposureScale 1.
- Measured (Vulkan, RTX, 640x480, `CAUSTICA_PRE_EXPOSURE=0` as before):
  - `tools/rr_stability.py` default (Cornell, manual exposure, P = 1): edge temporal std 0.00700 and mean frame difference 0.000158 before and after, identical (nothing runs at P = 1). PASS.
  - Same with `auto_exposure` on: edge std 0.00855 to 0.00846, frame difference 0.000475 to 0.000184. PASS both.
  - starlight-field, auto exposure, RR at scale 0.667, 96 frames, last 32 scored against a 200-star reference (the brightest 200 star pixels): 0 stars vanished (ratio < 0.25) and 0 flicker (temporal CV > 0.5) both ways; median brightness ratio 0.826 to 0.889, median temporal CV 0.136 to 0.059, mean frame difference 0.00059 to 0.00029. Stars survive RR; pre-exposure halves their flicker. Open: the D3D12 side is written but not compiled on Linux; FSR is D3D12-only so untested here; XeSS on a P step not measured.


