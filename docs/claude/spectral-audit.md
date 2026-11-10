# Spectral audit (wave 0, 2026-11-10)

What is spectral today, per integrator and per feature, read from the shaders and the Rust on branch `wave0`
(based on `cpu-playable`). Docs only: no source was changed and nothing was run, so every row is a code reading,
not a measurement. Findings that need a render to confirm are marked **unverified**.

Paths: `P` = `shaders/pathtrace.hlsl`, `I` = `shaders/integrators.hlsl`, `B` = `build.rs`,
`G` = `src/gpu/passes.rs`, `V` = `src/vulkan/gpu/passes.rs`, `E` = `docs/engine-notes.md`.

## 1. How a variant is chosen

- `Settings::spectral` is the only switch. Variants are the same shader compiled with `-D SPECTRAL`
  (`B:207-270`, `src/hotreload.rs:181-233`, `src/gpu/init.rs:360-374`).
- Path tracer: `trace_spectral`, `trace_spectral_lean` and their SER twins (`G:261-265`, `Gpu::ser` 5 and 6).
  **Forced back to RGB when any LPE slot is set** (`G:237`, `V:171`); the CPU does the same
  (`src/reference/scene.rs:427`).
- BDPT: `bdpt_spectral` (`G:186-189`, `V:134`). Photon mapping: `photon_trace_spectral` + `photon_gather_spectral`
  (`G:201-212`, `V:141-150`). The resolve pass `bdpt_resolve` is shared (it only averages).
- No other pass has a spectral build: ReSTIR presample/initial/spatial, LPE (`trace_lpe`, `B:85-91`), path
  visualizer (`trace_paths`, `B:75-81`), post (glare, halation, film look).
- The three wavelengths ride in the `float3` colour channels (`P:120-148`); one hero wavelength per sample in
  BDPT and the photon passes (`I:107-108`).

## 2. Table: feature -> spectral / RGB / partial

| Feature | State | Evidence |
|---|---|---|
| Path tracer, 3 wavelengths, hero termination | spectral | `P:120-148` (`begin_wavelengths`, `up`, `to_rgb`, `hero`); per-sample start `P:1423`; end `P:1663` (`to_rgb(radiance)`, no clamp `P:1668`) |
| PT colour entry points (material colour, sheen, emission, light samples, sky/HDRI, background, suns, media, splitters) | spectral | `P:1565, 1571, 1585, 1509, 1543, 1520, 1488, 1044, 1155, 1293`; media `P:575, 1245` |
| PT dispersion (Cauchy via Abbe) | spectral | `P:204-209` (`ior_of`), `hero()` at `P:1621` |
| PT gratings | spectral | hero at `P:1582`, `grating_weight()=1` when spectral `P:742`, `vertex_material` `I:92` |
| PT thin film | spectral, exact per wavelength | `P:601-602` (`waves`), `prepare_film_waves` `P:620-625`, call `P:1585`. Film IOR itself is constant (no dispersion of the film); a metal base's complex IOR is derived from the uplifted f0 (`P:622`), not measured n,k |
| PT real lens with dispersion | spectral | `lens_ior` `P:867-874`, hero weight `P:915-923` |
| PT chromatic media (extinction tint) | spectral | `tint_of` `P:185`, `flight` `P:190-199` takes the hero's sigma when `hero_only` |
| Measured spectra (Cornell codes 1-4) | spectral, but only 5 built-in spectra | `own_spectrum` `P:157-163`, `up_material` `P:175`, `up_emission` `P:177`, `up_emitted` `P:179-182`; table `src/spectrum.rs:8-13`. No user-loadable spectra |
| Blackbody materials (`Material::temperature` with emission) | spectral, exact Planck | `src/lighting.rs:82-85` (phase.z = -T), `P:160` |
| Blackbody extra lights (`Light::temperature`) | spectral, exact | `src/lighting.rs:211-218` passes `temperature` to `pack` -> same path as above |
| Blackbody main light (`light_temperature`) | spectral, exact **only while `light_tint` is white** | `src/lighting.rs:197-208 (gate at 203)`; a tinted main light uplifts its RGB (`up_emitted`) |
| Sun lights (`LightKind::Sun`) with Kelvin | spectral exact (same `pack`) | `src/lighting.rs:211-218` |
| Physical-sky sun | RGB uplifted (not a solar spectrum) | colour is `l / max` RGB, `src/lighting.rs:247-252`, then `up_emitted` `P:1520` |
| Physical sky, HDRI, background | RGB uplifted (inherent: the data is RGB) | `P:1509, 1543`, `I:200`; bake is an RGB map (`src/sky.rs:441`) |
| Photon emission | spectral | one wavelength, hero from emission: `I:566-567`; env/sun photons `I:550`, `I:567` (`up`/`up_emitted`) |
| Photon walk (surface, dispersion, grating, film, media) | spectral | `vertex_material` `I:87-100`, `sample_vertex` dispersion `I:142`, `walk_step` `I:71-84` |
| Photon deposit | **RGB** (collapses the spectrum) | `deposit` `I:497-503`: `b=to_rgb(b)`; volume flag in blue's lowest mantissa bit `I:501` |
| Photon gather, camera side | **spectral** (one wavelength per camera path), but gathers RGB photons | `begin_hero` `I:676`, `hero_rgb(L)` `I:699`; photons re-uplifted at the camera's wavelength: surface `I:607`, volume `I:634`. So the receiver's spectral response (film, measured albedo) acts on an RGB-smoothed photon power: **partial** |
| Photon gather camera-side glass (prism seen directly) | spectral | follows from `begin_hero` + `sample_vertex` `I:142`. E's "camera side is RGB, a prism seen directly does not split colours" (line 878) is stale |
| BDPT camera subpath | spectral, one wavelength | `begin_hero` `I:456`; `up(alb)` at media `I:306`; escape `I:200-207` |
| BDPT light subpath (emission, scatter) | spectral | `up_emitted` `I:336`, `up(alb)` `I:344` |
| BDPT connections s>=1, NEE, env, suns | spectral | `I:398, 405, 409` |
| BDPT t=1 light-tracing splat | spectral | `up_emitted` `I:385`, `vertex_f` -> `vertex_material` `I:87-100`, splat via `hero_rgb` `I:391`. Same wavelength as the sample's camera subpath |
| BDPT vertex material (film, grating, dispersion) | spectral | `vertex_material` `I:87-100` (grating wavelength = hero `I:92`; film `prepare_film_waves` `I:97-98`) |
| BDPT sample weights | **partial: always hero-only (1 wavelength)** even when nothing is dispersive | `begin_hero` sets `hero_only=spectral_on` `I:107`; the PT keeps three wavelengths until `hero()` is needed. So BDPT/photon colour noise is up to 3x the PT's |
| BDPT / photon gather with a dispersive real lens | **suspected bug, unverified** | `camera_ray` multiplies `camera_weight` by `(3,0,0)` when `spectral_on` (`P:923`) and the BDPT/gather then apply the 3 again in `hero_rgb` (`I:108`, `beta=camera_weight` `I:300`, `I:677`): 3x too bright. The PT is right (its hero weight lives only in the throughput) |
| ReSTIR DI reservoirs | RGB targets, spectral shading | presample/initial/spatial have no spectral build; `restir_nee` shades with `up_emitted` `P:1155`. Unbiased (target function only steers sampling) |
| LPE variant | **RGB** | no `SPECTRAL` build (`B:85-91`); forced at `G:237`, `V:171`; `lpe_sum` accumulates RGB `P:667-673, 1677-1678` |
| Path visualizer | **RGB** | `trace_paths` built with `PATH_RECORD` only (`B:75-81`), dispatched at `src/gpu/frame.rs:157` and `src/vulkan/gpu.rs:916` regardless of `spectral`; records show RGB throughput, no dispersion, no wavelength |
| Denoiser/RR guides (albedo, roughness...) | RGB by design | colours read before `up` `P:1585` |
| Output (`to_rgb`, D65 in the weights) | spectral | `P:143-146`; negative components kept in the mean `P:1668, 1681`; clamped only at display (`post.hlsl` `integrate`/`beauty`), see E:936 |
| Camera film / sensor | spectral (CIE 1931 2deg -> sRGB, D65 white) | `SPECTRAL_OUT` in `shaders/spectral_tables.hlsli`; no camera-specific sensor curves |
| Post: glare, halation, vignette, grain | RGB (post-process, after `to_rgb`) | `shaders/post.hlsl:217-226` (halation colour is a constant `(1,0.3,0.12)`); glare scales its pattern per RGB channel, `post.hlsl:227` |
| Vulkan backend | same set as D3D12 | `V:134-177`; `trace_paths`, LPE RGB there too; no LPE layers in the capture (E:937) |
| CPU reference | PT spectral; refuses BDPT/photons | `src/reference/spectral.rs`; `src/reference/mod.rs:54`. So the oracle cannot test spectral BDPT/photons; only PT-vs-BDPT agreement was measured (E:966, 1.009, same as RGB) |
| Fluorescence, Jakob-Hanika uplift, 4 wavelengths, polarization | not implemented | grep over `src`, `shaders`: no hits |

## 3. True remaining list for Spectral part 2

Tiers: sonnet = mechanical or doc work; opus-low = new GPU code with a known pattern; opus-high = design with
real risk. Sizes S/M/L as the plan uses them (S < 1 day, M 1-2, L 3+).

| # | Item | Why it is still open | Tier | Size |
|---|---|---|---|---|
| 1 | Doc/UI reconciliation (section 4) | Roadmap line 223 and several notes still say BDPT and the photon camera side are RGB; the Spectral tooltip says "BDPT stays RGB" | sonnet | S |
| 2 | Verify and fix the hero-weight double count with a dispersive real lens in `bdpt_spectral` / `photon_gather_spectral` | Section 2, `P:923` vs `I:108`. Check: lens system with `dispersion=1`, spectral on, BDPT vs PT luminance ratio (expect 1.00, suspected 3.0). Fix is one line (drop the `(3,0,0)` when the caller is already hero) | sonnet to run, opus-low to fix | S |
| 3 | Measure colour noise under RR and the native denoiser, spectral vs RGB (roadmap item "unmeasured") | Decides whether #4/#5 matter. Script on Classic and a coloured scene at equal spp, chroma variance of `noisy` and of the RR output, `tools/` | sonnet (verifier runs) | S |
| 4 | BDPT/photon: keep 3 wavelengths until a hero event (`hero()` lazily, as the PT), or gate `begin_hero` on a scene flag "has dispersion/grating/dispersive lens" | `I:107` forces one wavelength per sample, up to 3x the PT's colour noise in non-dispersive scenes. Needs care: MIS pdfs are wavelength-free, but a hero event inside one subpath must zero the other channels of every connection through it. Adding a flag means a `Constants`/`cbuffer Frame` field (keep in sync) | opus-low | M |
| 5 | LPE spectral twin | `trace_lpe + SPECTRAL`: `lpe_add` sums `e * lpe_weight` per slot, so each slot needs `to_rgb` at the end (`P:1677`) and the clamp rule of the beauty; new entry in `B`, `hotreload::ENTRIES` (same order), `Shaders`, `Gpu::reload_shaders`, a `lpe_ser` twin if SER is on (`Gpu::ser` index 4), Vulkan list, CPU twin (`scene.rs:427` stops disabling spectral for LPE). Register pressure of the LPE tracer is already high | opus-low | M-L |
| 6 | Path visualizer in spectral | Smallest useful version: record the sample's hero wavelength in record 13 (`end`) and colour the polyline by it; full version: `trace_paths` with `-D SPECTRAL`. Parser `src/paths.rs`, UI | sonnet | S-M |
| 7 | Exact blackbody for a tinted main light | `src/lighting.rs:197-208 (gate at 203)` gates on `light_tint == 1`. Exact form: blackbody(l) times the uplifted tint (a per-wavelength product, which is what a coloured filter is). Needs the tint carried to the shader (phase.w already holds the scale) | sonnet | S-M |
| 8 | Sun and sky spectra | Physical sky's sun is an RGB uplift. Optional: solar blackbody (5772 K) times an atmospheric transmittance; low value until the night-sky bake lands (stars are RGB points, fine) | opus-low | M |
| 9 | Photons carrying a spectrum through deposit | `deposit` collapses to RGB (`I:497-503`), so a film/measured receiver sees an RGB-smoothed power. Fix: keep the photon's wavelength in the 32-byte record (needs a slot; the volume flag already steals a mantissa bit) and let the gather weight by the photon's delta. Probably not worth it: only a coloured receiver under a prism differs | opus-high | L |
| 10 | Jakob-Hanika (2019) uplift | Mallett-Yuksel `up()` is linear (3x3 per sample, round trip 1.1e-4 under D65); JH gives better saturated colours but is a nonlinear 3D-table lookup, so the linear properties that photons/HDRI/emission rely on (E:875, 878) break and emission needs its own scale path. Tables: generator in `tools/spectral_tables.py`, CPU twin, SPIR-V `StructuredBuffer` path (E:1030 note). Do #3 first | opus-high | L |
| 11 | Fluorescence ("black-light mode") | Needs a wavelength-changing event (resample or shift the hero at the hit, rewrite `path_lambdas` and `uplift_rows`), a UV range (tables stop at 380 nm; extend to ~300 and the importance pdf), an excitation/emission field per material (`PackedMaterial` is full: read `docs/claude/data-layouts.md` first), a UV source, CPU twin | opus-high | L |
| 12 | Four wavelengths | Every colour in the tracer is a `float3`; going to 4 means `float4` everywhere (+registers, the +16-18% LEAN spectral cost of E:880 gets worse) or a second pass. Not worth it before #3 shows colour noise is a problem; BDPT/photon are hero-only and gain nothing | opus-low | L |
| 13 | Unclamped spectral output | The GPU capture clamps pixel means at 0 (E:936), so negative channels from hero-only rainbows add light; an unclamped EXR needs a raw accumulation readback. Maybe a soft gamut map instead of a clamp | opus-low | M |
| 14 | Spectral data: user-loadable spectra and measured n,k for metals | Not on the roadmap. Only 5 built-in spectra exist; metals derive their IOR from f0 | sonnet | M |

Recommended order: 1, 2, 3, then 4 and 6 (cheap, visible), 7, 5; 8-14 as demand appears. Items 10, 11 and 12 are the
real "Spectral part 2" research; 9 and 12 are probably better dropped.

Not spectral on purpose: glare/halation/grain (post-process on the displayed image), denoiser and DLSS guides,
ReSTIR's target function.

## 4. Proposed corrections (diff-style; do not apply here)

`docs/roadmap.md`

```
@@ roadmap.md:223 (Spectral rendering, part 2)
-Part 1 is done (path tracer, photons, dispersion, exact thin film, gratings, lens). Open: BDPT, the photon gather's camera side, LPE and the path visualizer stay RGB; exact blackbody Kelvin lights; fluorescence (black-light mode); Jakob-Hanika uplift; colour noise under RR and the native filter unmeasured; four wavelengths instead of three.
+Part 1 is done (path tracer, photons, BDPT, the photon gather's camera side, dispersion, exact thin film, gratings, lens, blackbody materials and lights, measured spectra). Open (docs/claude/spectral-audit.md): LPE and the path visualizer stay RGB; BDPT and the photon passes carry one wavelength per sample whatever the scene (3x the colour noise of the path tracer); a tinted main light and the physical sky's sun are RGB uplifts; fluorescence (black-light mode); Jakob-Hanika uplift; colour noise under RR and the native filter unmeasured; four wavelengths instead of three.
@@ roadmap.md:217 (Accuracy mode)
-... RGB instead of spectral) ... Software mode is the CPU reference plus spectral.
+(Spectral is on the CPU reference since 2026-11-10; the "RGB instead of spectral" shortcut no longer applies to the path tracer.)
@@ roadmap.md:139, 206
 (check: "[drop] Spectral leftovers" and "[drop] ... spectral" lines: reference now supports spectral; drop the word from the list at 206.)
```

`docs/engine-notes.md`, section "Spectral rendering" and its neighbours

```
@@ E:873 (Switch)
-LPE runs and the path visualizer stay RGB (no spectral twin), as does BDPT.
+LPE runs and the path visualizer stay RGB (no spectral twin: `G:237`, `trace_paths`). BDPT runs `bdpt_spectral` and photon mapping `photon_trace_spectral` + `photon_gather_spectral` (one wavelength per sample, `begin_hero`).
@@ E:878 (Spectral photons)
-the gather stays RGB, and since the uplift is linear the two sides agree. ... The camera side of photon mapping is RGB, so a prism seen directly in a photon-mapped scene does not split colours; its caustics do.
+the photon store is RGB (`deposit` calls `to_rgb`), and the gather re-uplifts it at the camera path's wavelength (`photon_gather_spectral`, one hero wavelength per camera path), so a prism seen directly splits colours as well as throwing its caustic. A receiver with a spectral response (film, measured albedo) sees the photon's RGB-smoothed power, not its original wavelength.
@@ E:402 (Thin film model)
-then XYZ -> linear Rec. 709. No spectral rendering.
+then XYZ -> linear Rec. 709. This is the RGB model; with Spectral on, `film_fresnel(..., waves=true)` evaluates the same Airy sum at each path wavelength exactly (no CMF fit).
@@ E:914 (Real lens systems)
-Spectral lens tracing (the hero wavelength, `lens_ior` c = 3) waits for the spectral group.
+Spectral lens tracing (the hero wavelength, `lens_ior` c = 3) is done (see "Spectral rendering").
@@ E:923 (Thin film in RGB)
-Gratings and the spectral film stay refused (spectral mode is).
+(Spectral mode, with its film and gratings, has rendered on the CPU since 2026-11-10: see the next entries.)
@@ E:924 (Diffraction gratings in RGB)
-(spectral stays refused as a whole)
+(spectral mode renders: gratings take the hero wavelength at weight 1, see the spectral entry)
@@ E:966 (Spectral leftovers)
-Blackbody lights (`kelvin`) and measured spectra ... Spectral BDPT (`bdpt_spectral`) and the photon gather's camera side (`photon_gather_spectral`) take one wavelength per sample from the start (`begin_hero` / `hero_rgb`).
+Add: exact for a material's temperature, extra lights' temperature, sun lights and the main light while `light_tint` is white (`lighting.rs:197-208`); a tinted main light, the physical sky's sun, the sky and HDRIs are RGB uplifts. "Spectral leftovers" in the roadmap are now: LPE, the path visualizer, tinted main light, fluorescence, Jakob-Hanika, four wavelengths (docs/claude/spectral-audit.md).
+Add (open, unverified): with a dispersive real lens, `camera_ray` weights the hero by 3 (`pathtrace.hlsl:923`) and `hero_rgb` weighs it by 3 again, so BDPT and the photon gather may be 3x too bright there.
@@ E:880 (Cost) -- add
+BDPT and the photon passes are hero-only always: one wavelength per sample, so their colour noise exceeds the path tracer's in scenes with no dispersion. Not measured.
```

Source text, not notes (listed so the same wave can fix them; not edited here)

```
@@ src/ui/render_tab.rs:291 (Spectral tooltip)
-Path tracing and photon mapping (there the photons carry wavelengths, so a prism throws a rainbow); BDPT stays RGB.
+Path tracing, BDPT and photon mapping (BDPT and photon mapping carry one wavelength per sample); light path expressions and the path visualizer stay RGB.
```

README.md:60 says "Spectral (Render tab, path tracing and photon mapping)": add BDPT to that list.
