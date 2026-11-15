## Upscalers (2026-10-29)

- **Setting:** `Settings.upscaler` (`Upscaler`: dlss default, fsr, xess, bicubic, bilinear). Used only when RR is off and `rw < width`.
  - It is in `transport_key`, so a change resets the render.
  - `main.rs` also calls `Gpu::reconfigure_upscalers()` when `rr`, `rr_preset`, `upscaler` or `dlss_preset` changes. This replaces `reconfigure_rr`.
- **`Gpu::prepare_upscaler`** creates the selected temporal upscaler lazily, at render size × output size.
  - A failure goes into `sr_failed`, and the display falls back to bicubic. `upscale_status` explains this in the UI.
  - `release_upscalers()` runs on resize and reconfigure.
- **Per-frame mode:** with a temporal upscaler active, `render()` calls `reset(false)` every drawn frame, so each frame is fresh jittered samples, as with RR. The native SVGF history (`camera_moved` → reproject) plus the upscaler do the accumulation. `run_upscaler` feeds color (ping when denoising, else accumulation), device depth (25) and motion (6), and writes texture 14.
- **Measured alternatives** (`tools/upscale_quality.py`, Classic, 1280×720, 50%):
  - Feeding the progressive accumulation (jittered or unjittered) measured worse: the upscalers assume each frame sits at its own jitter.
  - Still-camera quality now comes from a longer denoiser history: cap 256 where motion is 0, 24 otherwise (`post.hlsl` integrate).
- **Flags** in `scene_info.w`:
  - Bit 0: jittered primary rays. Both RR and temporal SR set it; trace reads it, no longer `modes.w`.
  - Bit 1: the display reads texture 14 (RR or SR output).
  - Bits 2–3: spatial filter (1 bilinear, 2 bicubic). Catmull-Rom, clamped to the 2×2 footprint against HDR ringing.
- **Jitter and motion conventions:**
  - All three take the same jitter sign as RR: the negated ray offset. Verified by probing all four sign combinations; the current one is best for each.
  - Motion is prev − curr in render pixels, scale 1. XeSS velocity scale is 1.
- **Results** (RMSE ×1000, light rim excluded; still / orbit):

| | Still | Orbit |
|---|---|---|
| Native 100% | 1.05 | 4.06 |
| RR | 1.90 | 2.60 |
| XeSS | 2.27 | 4.70 |
| FSR | 2.28 | 4.25 |
| DLSS SR | 2.62 | 3.82 |
| Bicubic | 4.15 | 5.91 |
| Bilinear | 4.70 | 6.01 |

  - With the rim included, DLSS SR and XeSS score about 9.5 on the still camera, because of how they blend the 12× HDR light edge.
- **Self-test:** frames 172–176 switch through every upscaler at scale 0.5 and assert that the available temporal ones start; frame 177 turns RR back on.
- **DLSS SR presets** (roadmap item 1, 2026-10-30): `Settings.dlss_preset` ∈ `scene::DLSS_PRESETS` (0 default, 10 J, 11 K, 12 L, 13 M), checked by `validate()` and kept across `preset()`.
  - `caustica_ngx_create_sr` sets all six `NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_*` hints on `sr_params`. A change reconfigures the upscalers (`main.rs` trigger list).
  - The UI combo shows only while DLSS SR is the active choice; `upscale_status` names the preset.
  - Self-test frames 180–184 cycle the presets and assert each starts; 185 restores default + RR.
  - Checked: at 50% (Performance), M renders identical to Default (NVIDIA picks M there), while J/K/L differ, so the hint reaches NGX.
  - SDK 310.9.16 has no RR presets beyond D/E/F.
- **egui scissor:** `draw_gui` skips meshes whose clipped scissor is empty. Off-screen windows after a self-test resize produced D3D12 warnings that failed the self-test, on `main` too.
- **Sliders:** all go through `crate::slider` (`SliderClamping::Edits`; since 2026-10-30 a `Slider` wrapper with double-click reset, see "Owner requests (item 6)"). egui's default clamps the stored value whenever a slider is drawn, which silently rewrote valid loaded values (found by the UI-plan agent). Self-test frames 178–179 guard this.

