# Path-Traced Cornell Box — WebGL2 GPU Path Tracer

**This is the original WebGL2 version.** The native Rust / D3D12 RTX renderer lives in the [repo root](../README.md).

A self-contained, real-time interactive path tracer of the Cornell box, running entirely
on the GPU in WebGL2 / GLSL ES 3.00. No libraries, no build step, no network access.

```
cornell/
├── index.html   UI shell, dark theme, HUD
├── shaders.js   GLSL sources (trace / SVGF denoise / motion history / display)
├── app.js       WebGL2 engine, schema-driven UI, interaction
└── tools/       headless-Chrome (CDP) probe scripts from development; not needed to run
```

## Running it locally

Option A — just open the file:

- Double-click `index.html`, or drag it into Chrome / Edge / Firefox.
- Everything is classic scripts, so `file://` works (a few browsers disable some
  features over `file://`; if rendering fails, use option B).

Option B — serve the folder (recommended):

```bash
cd cornell
python -m http.server 8080
# then open http://localhost:8080/index.html
```

Requires a browser with WebGL2 and float render targets
(`EXT_color_buffer_float`, or `EXT_color_buffer_half_float` as fallback).
Chrome, Edge, Firefox and Safari 15.4+ all qualify. Hardware GPU strongly recommended:
software rasterizers (SwiftShader) run, but slowly.

## What it actually does

The trace pass is a fullscreen-triangle WebGL2 pass that shoots `spp` samples per pixel
per frame, up to `bounces` path segments each:

- **Monte Carlo transport with progressive accumulation.** Each frame renders a batch of
  samples and blends them into the running average
  `mix(prev, batch, n / (k + n))`. Accumulation runs until *Max accumulated samples*,
  then the status reads *Converged*. Every sample gets a fresh sub-pixel offset, so edges
  anti-alias as samples accumulate.
- **Low-discrepancy sampling** (*Rendering → Sampling pattern*, default *Sobol + blue
  noise*). Every random decision on a path (sub-pixel offset, lobe choice, light sample,
  bounce direction, roulette) draws from its own 2D slot of an Owen-scrambled Sobol
  sequence (Burley 2020's hash-based scrambling), so each pixel's samples are stratified
  instead of clumping. All pixels share the sequence, shifted per pixel by a 64×64
  blue-noise mask (generated at startup by void-and-cluster), which pushes the leftover
  error toward fine, even grain. *Random (white noise)* restores the old PCG sampler.
- **Next-event estimation from the area light with MIS.** The rectangular light is
  sampled directly at every diffuse hit (balance heuristic with the analytic light pdf),
  plus cosine-weighted hemisphere sampling. The BSDF-side weight is applied only to
  emission the bounce ray actually reaches, never to the rest of the path. This is what
  makes soft shadows, penumbrae and colour bleeding converge quickly.
- **Materials.** Each surface is a mixture of a Lambertian lobe, an importance-sampled
  GGX lobe and a dielectric lobe, picked per bounce by Metallic / Reflectivity /
  Transmission. Specular F0 comes from the IOR (0.04 at 1.5) and is raised toward 1 by
  Reflectivity; metals use their base colour. Dielectrics use Schlick Fresnel (with the
  transmitted angle when leaving the glass), refraction, total internal reflection and
  Beer–Lambert absorption: a transmissive object's base colour is its transmittance over
  one unit of distance. Roughness on glass is an approximate frosted-glass perturbation.
  Emissive geometry is supported (Object emission strength).
- **Light colour.** Colour temperature gives a blackbody colour normalised to unit
  luminance, so Light intensity means the same at every temperature; the Light colour
  picker then filters it like a gel.
- **Russian roulette** after the 4th bounce with throughput-compensated survival.
  An optional firefly clamp (*Rendering → Firefly clamp*, off by default) caps each
  sample at a multiple of the pixel's running mean. It removes speckles sooner but biases
  the image darker, so leave it off for reference renders.
- **Variance tracking.** Online (Chan) variance of luminance is accumulated in the
  beauty buffer's alpha channel. The *Convergence* view shows the relative standard
  error of each pixel's running mean on a log heat scale (blue ≤ 0.1 %, red ≥ 30 %), so
  it cools down as samples accumulate and shows where noise remains.
- **Denoise** is an SVGF-style filter (Schied et al. 2017, spatiotemporal
  variance-guided filtering). The displayed estimate is divided by albedo, so only
  lighting is filtered and textures stay sharp, then blurred by five à-trous passes
  (5×5 taps at spacing 1, 2, 4, 8, 16). Each tap is weighted by whether it lies on the
  same plane, faces the same way, and has a similar brightness, judged against the
  pixel's own estimated noise. The variance comes from the sample statistics, or from
  the neighbourhood while a pixel has fewer than 4 samples. The brightness tolerance
  follows each pixel's sample count, history included: loose while noisy, tight as it
  converges. So the filter smooths hard at first and leaves a converged image almost
  untouched. *Denoise strength* scales that tolerance (0.5 = as tuned, 0 = off).
- **Reusing samples while the camera moves** (*Camera → Reuse samples while moving*, on
  by default). A camera move still restarts the accumulation, but the previous image is
  reprojected into the new view through each pixel's surface point and blended in as
  history: up to 24 samples' worth, fading to nothing over the next 256 fresh samples.
  History is only taken from the same object, at a matching depth and normal, so
  disoccluded areas start clean. Mirror- and glass-like pixels keep at most about 2
  samples of it, because reflections don't move with the surface. The history only
  feeds the display and the denoiser, never the accumulation, so the converged image is
  unbiased.
- **AOVs**, traced in separate MRT channels:
  - *Direct light*: emitters seen by the camera, plus light reaching the first visible
    surface straight from an emitter (bounced at most once).
  - *Indirect light*: beauty − direct, i.e. everything that bounced more than once.
  - *Specular reflections*: everything seen through a specular or refractive first hit.
  - Normals, albedo, depth and convergence.
- **Direct / indirect light intensity** sliders recombine those two passes at display
  time (compositing, like light-path passes in an offline renderer). At 1 / 1 the image
  is exactly the beauty render, and changing them never restarts accumulation.
- **Tonemap / gamma / exposure** applied only in the display pass
  (none, Reinhard, Reinhard-white, ACES, Uncharted-2 filmic, contrast), so accumulation
  stays linear HDR.
- **A/B comparison** freezes the Beauty view exactly as shown (denoise and light
  sliders included) in linear HDR; exposure, tone mapping and gamma stay live on both
  sides. The comparison shows in the Beauty view.
- **High-res export** re-runs the display pass at an offscreen resolution (×2…×4,
  capped at 4096 px) with bilinear resampling of the accumulated HDR buffer.

## Controls

| Action | How |
|---|---|
| Orbit | Left-drag / touch-drag |
| Pan | Right-drag / Shift-drag / two-finger drag (moves along the screen axes) |
| Zoom | Wheel / pinch |
| Pause / Resume | Button or `Space` |
| Reset accumulation | Button or `R` |
| Auto-rotate | Button or `A` |
| Capture reference (before/after compare) | Button or `C` |
| Denoise toggle | Button or `D` (SVGF-style filter; *Denoise strength* scales it) |
| Sampling pattern | *Rendering → Sampling pattern*: Sobol + blue noise, or random |
| Reuse samples while moving | *Camera* panel toggle (on by default) |
| View modes | Dropdown or `1`–`8` |
| Screenshot | Button (PNG of the viewport) |
| Export high-res PNG | Button (display pass re-rendered at export scale) |
| Save / load settings | Buttons (JSON file; values are checked on load, unknown or invalid entries are skipped) |
| Presets | Scene preset dropdown: layouts (Classic, Two reflective spheres, Sphere and box, Mirror + glass) and looks (Mirror, Glass, Warm lighting, Dramatic lighting, Emissive spheres). Each starts from the defaults and keeps only camera and output settings |
| Randomize / Reset all | Buttons |

Keyboard shortcuts are ignored while Ctrl / Cmd / Alt is held and while you type in a
field or a dropdown. Space or Enter on a focused button presses that button.

Every slider is live: changing a lighting / material / geometry / camera / trace
parameter resets the accumulation automatically, so you never see a blend of old and new
scenes. While paused, a change renders one preview batch and then holds.
Exposure, gamma, tonemap, view mode, direct/indirect scaling, denoise and comparison are
display-side only and do **not** reset accumulation. When the render is paused or
converged and nothing changes, the app stops redrawing and the HUD time reads *idle*.

The HUD shows **GPU time** per frame when the browser exposes
`EXT_disjoint_timer_query_webgl2`, and CPU submit time otherwise (the label says which).

## Honest notes and limitations

- **Camera motion restarts the accumulation.** Orbiting, zooming or auto-rotate starts
  a fresh running average each frame, so auto-rotate never reaches “Converged”. With
  *Reuse samples while moving* on, the reprojected history keeps the image smooth in the
  meantime. Pixels that straddle two objects (silhouettes, the rim of the light) get no
  history and stay noisy until the camera stops. Newly revealed areas start from scratch
  as well.
- **High-res export is not supersampling.** It resamples the accumulated HDR buffer
  bilinearly and tone-maps at the export resolution. Sample density is unchanged; the
  image is larger and cleaner-looking, but detail comes from the accumulation resolution.
- **Transmissive objects shadow by default.** The *Glass casts shadows* checkbox controls
  whether glass blocks direct light in next-event estimation. Turning it off brightens
  the region behind glass (a common no-caustics approximation); refraction itself is
  always traced in mirror paths.
- **Caustics are not sampled.** Light focused through glass onto the floor arrives only
  when a diffuse ray happens to hit the glass — so glass projects a soft, slightly noisy
  patch rather than a crisp caustic.
- **Rough specular loses some energy at grazing angles.** GGX samples that reflect below
  the surface are terminated (single-scattering microfacet model), so very rough metals
  are slightly darker at their rims than a multiple-scattering model would make them.
- **Shadowed faces converge slowly.** Faces not seen by the light are pure indirect
  light; without the denoiser they stay visibly noisy at low sample counts.
- **The denoiser has no firefly handling.** Rare, very bright samples (light bounced
  off a mirror onto a wall) survive as speckles until enough samples average them out.
  At low sample counts it can leave faint low-frequency blotches on flat walls, which
  is typical for SVGF.
- **Heavy settings can time out the GPU.** One frame traces Samples per frame × Max
  bounces paths per pixel in a single draw. On a weak GPU, 16 × 16 at full resolution
  can exceed the driver's watchdog; the context is then restored automatically and the
  render restarts. Lower those settings if it keeps happening.
- **Memory.** The renderer uses 15 RGBA32F textures at the internal resolution
  (accumulation, denoiser, history and reference; 240 bytes per pixel), roughly 380 MB
  at 1600×1000 internal pixels.
- **Half-float fallback.** On GPUs without float32 render targets the buffers are
  RGBA16F, whose precision stalls the running mean after roughly a thousand samples.
- **Objects rest on the floor.** Object Y is snapped to the floor by design so nothing
  floats when you change size or stretch. Camera distance auto-fits through the open
  front face on load, camera reset, and preset changes.
- **Resolution slider** is a percentage of the CSS viewport size, device-pixel-ratio
  capped at 1.25.
- The scene is the classic open-front Cornell box: interior walls at x = ±1, floor
  y = −1, ceiling y = +1, back wall z = −1, front open at z = +1 (the walls are thin
  boxes). The area light is a quad on the ceiling; it is never an occluder.
- **GPUs with only 4 draw buffers** (the WebGL2 minimum) run without the specular AOV,
  so the Specular reflections view is black there. Moving-camera history on mirrors
  then isn't shortened, so reflections can smear slightly while orbiting.

## Measured noise

Classic preset at 672×509 on an RTX 4070 Ti SUPER. Error is the RMS difference from a
16,384-sample reference, on luminance compressed with l/(1+l), ×1000 (lower is better).

| Samples | Random | Sobol + blue noise | Old bilateral denoise | SVGF (Sobol) |
|---|---|---|---|---|
| 4 | 50.5 | 46.1 | – | 8.8 |
| 16 | 27.6 | 21.8 | 23.9 | 6.2 |
| 64 | 14.3 | 10.0 | 5.3 | 3.9 |
| 256 | 7.2 | 4.8 | 4.6 | 2.4 |
| 1024 | 3.7 | 2.4 | – | 1.5 |

Sobol + blue noise reaches the same error as random with 2–2.4× fewer samples from 64
spp on, for about 0.3 ms more per frame (1.23 vs 0.93 ms at 2 spp). SVGF adds about
0.6 ms.

While orbiting 30° over 60 frames at 2 spp, measured on surfaces (97% of pixels;
object edges excluded):

| Moving camera | Neither | Reuse | SVGF | Reuse + SVGF |
|---|---|---|---|---|
| Error | 39.0 | 8.5 | 4.3 | 2.5 |

Once the camera stops, the image converges to the same error as a render that never
moved: the history is gone after 256 samples.
