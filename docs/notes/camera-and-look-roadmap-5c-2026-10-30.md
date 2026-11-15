## Camera and look (roadmap 5c, 2026-10-30)

- **Constants grew past 512 B:** passes now sit at `CB_STRIDE` = 1024 (trace 0, filter 1–7, display 8; buffer 9 KB, compile-time assert). New blocks: `lens`, `camera_model`, `look`, `lift` (w = grain cell size), `grade_gamma` (w = LUT strength), `gain` (w = frame seed). Mirror any change in common.hlsli.
- **Trace:** `camera_ray(pixel, lens_sample, …)` handles every model. Guides use the pinhole ray (`lens_sample` false), so RR and the denoiser guides stay sharp.
  - Lens: `aperture_point()` gives a disk or an n-gon fan. The focal plane is at `focus_distance` along the view axis, or tilted about the camera right axis (model 4).
  - Cat-eye rejects lens points outside a circle shifted by the pixel's NDC (`valid=false` → zero throughput).
  - Ortho uses `camera_model.w` = distance·tan(fov/2) as the half height. Equirect is ±π by ±π/2. The little planet is a stereographic projection looking down.
  - Non-perspective guide depth is the hit distance.
- **`Settings::perspective()`** (models 0 and 4) gates RR (`rr_active`), the temporal upscalers (`prepare_upscaler`), click picking and the gizmo.
- **Display:**
  - `lens_and_film` adds halation and vignette on linear light before the tone map. Halation is 24 taps of `scene_at` (RR/SR output or the native image) above 1.0, tinted red-orange.
  - `film_look(t)` does lift/gamma/gain on the encoded value, then the LUT (manual trilinear over u29). It runs inside `tonemap` and in `hdr_map`'s tone-mapped branch (not the Linear HDR branch).
  - Grain is hashed per cell and frame, weighted to the midtones, after the tone map.
  - Tone maps 6 AgX and 7 PBR Neutral (`validate` allows < 8).
- **LUT:** u29 is a 3D RGBA32F texture (`texture_3d`) kept outside `textures`, so `resize` only re-creates its view (`lut_view`, heap slot 29). The root UAV range is `TEXTURES + 1`. `look::read_cube` parses .cube (3D, domain 0..1, size 2–128). `App` reloads it when `lut_path` changes.
- **Probe** is now `gpu::Probe { linear, albedo, depth, id }` (depth from u5, id from u19). Ctrl+left-click sets `focus_pending`; the next readback's depth becomes `focus_target`, eased with 1 − e^(−10·dt) per frame.
- Look fields (halation, grain, grain_size, vignette, lift, grade_gamma, gain, lut_path, lut_strength) are display-only, so they are excluded from `transport_key`.
- Self-test 216–239: aperture, blades and cat-eye; models 1–4 (218 asserts RR off); AgX, PBR Neutral; the film look plus a generated inverting LUT (224 asserts it loaded); Ctrl+click focus at the center (239 asserts it settled). Scene files start at 240.
- Visual check: `captures` at 640×400 of bokeh, ortho skyline, equirect Stonehenge, little-planet forest, tilt-shift skyline, AgX + halation/grain/vignette, PBR Neutral, all as intended.

