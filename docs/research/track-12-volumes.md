# Track 12 — Volumes (J): design notes

Written 2026-10-31 (pre-implementation). Roadmap 12: homogeneous media inside meshes +
global fog; unlocks laser beams (17), god rays / Vermeer room, The Weather Project,
subsurface jade/milk/jack-o'-lantern, and underwater mode (whole room filled with a medium).
RR handles volumes poorly — compare per scene against the native denoiser (roadmap's own note).

## Sources (web research, fetched 2026-10-31)

- EGSR 2012 course "Importance Sampling Techniques for Path Tracing in Participating
  Media" (Kulla and Fajardo): [abstract/diglib](https://diglib.eg.org/items/a3eae150-9f7d-4edd-ba74-520c0a20212b);
  the course's own PDF mirror is at https://fpsunflower.github.io/ckulla/data/egsr_2012_volume_paper.pdf
  (web_fetch cannot decode PDFs; the same design is spelled out in the book chapters below).
  Take-away: free-flight distance sampling + NEE for volume scattering events with MIS +
  ratio/delta tracking for heterogeneous media. We ship HOMOGENEOUS media: transmittance
  along any segment is the exact exponential — no delta tracking needed (that is the
  roadmap's own scope line: "homogeneous media inside meshes").
- pbrt-v3 `integrators/volpath.cpp` (fetched full text,
  https://raw.githubusercontent.com/mmp/pbrt-v3/master/src/integrators/volpath.cpp): the loop
  samples the medium interaction BEFORE/alongside the surface interaction
  (`beta *= ray.medium->Sample(...)`; if a MediumInteraction came first:
  NEE from the scatter point `UniformSampleOneLight(mi, ...)` then
  `mi.phase->Sample_p(wo, &wi)` continuation) — exactly the structure to fold into our
  spp loop.
- pbr-book 3ed §15.3 "Volumetric Light Transport"
  (https://www.pbr-book.org/3ed-2018/Light_Transport_II_Volume_Rendering/Volumetric_Light_Transport):
  phase p and scatteringPdf coincide under perfect phase sampling; NEE uses VisibilityTester::Tr()
  (transmittance) instead of Unoccluded().
- pbr-book 4ed §14.2 "Volume Scattering Integrators"
  (https://pbr-book.org/4ed/Light_Transport_II_Volume_Rendering/Volume_Scattering_Integrators):
  SimpleVolPath (random-walk, ground truth) vs VolPath (improvements: sample distances but take
  surface scattering when the sampled distance is past the closest intersection — i.e. the
  free-flight-vs-surface race structure we need).
- Okdalto blog "Volume Rendering, the Path Tracing Version"
  (https://okdalto.github.io/en/volume-rendering-path-tracing/): ray integral
  C(t) = ∫ T(t)·σ(t)·[c(t)+L_ext(t)] dt, extinction σ = σa+σs, Beer–Lambert as the
  transmittance integrand; delta tracking (heterogeneous) — not needed for our homogeneous
  segments between boundary crossings.
- smallpt-vol (classic minimal volume path tracer): free flight d = −ln ξ / σ, absorption
  as transmittance — validated design by convergence (our numeric harness does the same test).

## Design

### Data (append-only layout discipline, both sides)

- `Material`/`PackedMaterial` gain two float4s at the end (160 B → 192 B):
  `volume_a` = (sigma_a per meter rgb, w unused), `volume_s` = (sigma_s per meter rgb, w
  unused). Scene-side serde-defaulted fields `volume_absorb: [f32;3]` and `volume_scatter:
  [f32;3]` (0 = no medium); `validate()` bounds channels to (0. ..= 1e4) "Invalid volume".
  Editor: "media" grid two rows (Absorption, Scattering, per meter); presets/showcase values
  tuned by eye against the e^{-sigma_a L} analytic (jade ~ (2.2,0.55,0.2)/m absorb +
  (0.32,0.34,0.35)/m scatter; milk absorb ~ (20.,27.,38.)? tune — milk absorbs over cm so
  per-meter values are large; water (0.34,0.095,0.06)? tune per showcase; fog/scattering-only
  media keep absorb small grey).
- Ambient medium (fog AND underwater, one mechanism): `Settings.medium_scatter [f32;3]`,
  `medium_absorb [f32;3]` (per meter, default 0); two float4s appended to `Constants`
  (gpu.rs) AND cbuffer `Frame` after `environment_map` (mirror field-for-field):
  `float4 medium_s; // sigma_s rgb (per meter), w unused` + `float4 medium_a; // sigma_a rgb`.
  In `transport_key` automatically (new top-level fields).
- A medium's boundary is its object's surface: any object whose material has
  `volume_scatter` (or absorb) non-zero contributes membership; the surface itself keeps its
  normal BSDF (jade = glossy dielectric-ish boundary, milk = rough transmission-ish, pumpkin
  skin = matte-ish + interior scatter) — scattering membership is orthogonal to the surface
  shader, so no new surface kind. Boundary pass-through for shadow/march: media boundary
  surfaces are never occluders for the transmittance march (light through milk is attenuated
  by sigma_a of the milk instead — see march below).

### Transport (fold into `trace`'s spp loop, guard: sigma_s sum > 0)

Membership stack reuses the track-10 interior list pattern (push/pop `med_id` on front/back
hits of volume-field materials); running per-path sums `sigma_a_tot` and `sigma_s_tot`
(+ `sigma_bar = max channel of sigma_s_tot`) are updated incrementally on push/pop (plus the
ambient medium seeded at path start). Guard for the machinery: `bool vol = sigma_bar>0` at
path start; off-path code stays verbatim (bit-identical arithmetic), though the trace's
register class is raised kernel-wide again (same measured finding as track 11 — see "Cost").

1. Free flight vs surface race (pbrt structure): after `Intersect`, when `vol`:
   d = −ln ξ / sigma_bar (single scatter point shared by all channels, conservative sigma_bar).
   If d < t_hit (or miss): scatter event at x+d:
   `throughput *= exp(-(sigma_a + sigma_s − sigma_bar)·d) * (sigma_s / sigma_bar)` per channel
   (reduces to e^{−sigma_a·d} when the medium's sigma_s is uniform — the smallpt/pbrt form).
   Bounce++ counter, NEE from the volume point (below), then continuation direction sampled
   isotropically (cos = 1−2u, az = 2πv, pdf = 1/2π). HG phase (η) is future work (needs the
   published closed-form sampler; iso + absorption already gives believable jade/milk/fog).
2. Volume NEE at a scatter point: mirror the existing surface NEE block with the scattering
   weight folded in: contribution gets `sigma_s/sigma_bar · (1/2π)` (scattering coefficient ×
   phase evaluated/pdf = 1 with exact iso sampling) and the MIS denominator becomes
   `pl + sigma_bar·exp(−sigma_bar·d_light)·(1/2π)` (balance heuristic against the
   free-flight continuation density — the EGSR course's homogeneous form; d_light = |y−x|).
   Transmittance to the sampled light point y = `T_vol(x→y)`: march along the segment,
   toggling membership at boundary crossings (any-hit query windows [t0, len], commit the
   nearest volume-field surface, push/pop by front/back with the existing entered/leaving
   convention, multiply e^{−sigma_a_tot·Δ} between crossings); opaque BSDF occluders zero it
   (glass keeps the existing "glass shadows" switch, absorption along shadow rays inside glass
   stays the carried-forward not-done item from track 10). Ambient-only media: one query,
   T = e^{−sigma_a_tot·len} exact (no crossings). Sun/sphere/env miss paths get the same
   T applied along their rays.
3. Segments that end at surfaces (scatter distance past the surface): absorption to the
   surface along the inside-medium stretch applies the same running sigma_a_tot factor; the
   existing per-transmission-event Beer–Lambert of NON-volume dielectrics is untouched, and
   volume-field materials set their transmission-event absorption to 0 in `pack` (their
   absorption always comes from the march) so nothing double-counts. Nested media (milk
   inside glass): sigma_a_tot = σa_milk + (glass handled by existing event absorption outside
   the march) — documented simplification: shadow/march transmittance uses only
   volume-field media; dielectric Beer–Lambert remains event-based as in track 10.
4. Russian roulette unchanged (bounce counter shared); guide/AOV rays unchanged (the guide
   writes surface hit info only — RR on volumes is poor per the roadmap note; showcases
   verify under the native denoiser).

### Showcases (builtin pack) + validation

- `glass-of-milk`: milk cylinder medium inside the glass tumbler (nested pair via the inset
  volume pattern from track 10), sun + sky.
- `jade-orbs`: jade ball + jade cube on a stage, single key light — absorption-vs-scatter
  rim glow; god-ray shaft optional.
- `jack-o-lantern`: pumpkin = medium sphere (orange absorb, scatter) + small warm sphere
  emitter inside the cavity; glow through flesh = scattering+absorption only, no emission
  (carving awaits track 13 SDFs — noted not-done).
- `vermeer-room`: sealed room, one tall window (emissive quad), ambient fog scatter →
  crisp diagonal shafts across the floor.
- `weather-project`: huge hall, sun disc high, mirrored ceiling (smooth metal plane),
  ambient fog → shafts + ceiling reflection.
- `cornell-underwater`: classic Cornell with ambient water medium (strong blue-green absorb
  + scatter): back wall fades with distance = absorption; shafts from the main light.
- Numeric harness (captures/, transient): (a) analytic transmittance: floor pixel straight
  under the light through a uniform ambient medium — expect radiance·e^{−sigma_a·d} within a
  few % (NEE is exact for homogeneous, so tight tolerance ~2–3%); (b) shaft geometry: inside
  vs outside the sun cone ratio; (c) milk/jade eyeball vs smallpt-style reference renders.
- Regression: layer/media-free scenes stay bit-identical means (guard verbatim); self-test
  counts update (builtin pack grows; frame total recomputed).
- Cost: scattering machinery registers raise the trace kernel-wide again (same measured
  finding as track 11's guard) — measure tank/glass/layered-media and record honestly.
- RR note per the roadmap line: compare RR vs native denoiser per scene (volumes flicker
  under RR's surface-centric guides; native default for volume showcases).

## Open questions resolved by decision

- Phase function: isotropic now (exact pdf; HG η = future, roadmap never demanded it).
- Heterogeneous (grids/noise fields): NOT this track (roadmap says homogeneous); ratio
  tracking documented as the future extension (okdalto/EGSR both cover it).
- Boundary emission ("frosty" volume surfaces): out (track 15/16 later).
- Volume AOV (depth of first scatter, "god ray mask"): future (debug views track 5b-ish).
- Shadow attenuation inside NON-volume glass (Beer–Lambert along shadow rays): carried
  forward with the track-10 not-done list (documented).

## What shipped (post-implementation addendum, 2026-10-31)

- Data layer as designed: PackedMaterial 160→192 B (`volume_a`/`volume_s` appended),
  Material serde fields + validate, editor Absorb/Scatter rows, Settings
  medium_scatter/medium_absorb/medium_ior, Constants/cbuffer pair appended after
  environment_map (`medium_a.w` = medium_ior), library 18 (17 Milk, 6 Jade carries
  media), builtin pack 53→58.
- Bug caught during verification (worth keeping as a caution): gpu.rs Constants literal
  wrote `medium_a.w = 0.` instead of `s.medium_ior` → the dielectric sentinel pair reads
  IOR 0 → r0 = (0−ior)/(0+ior) = −1 → fr = 1 → glass ALWAYS mirrored in media-on scenes
  (cornell-water-tank black mirror + a bogus 2× "speedup" in the first chain). One-line
  fix, rebuilt clean; all later numbers from the fixed exe.
- The Test A red-channel "anomaly" (0.3 channel ~5× low, stable across sample counts)
  ROOT-CAUSED: post.hlsl `beauty()`'s direct/indirect recompose scales by
  `composed/max(raw,1e-4)` per channel — raw beauty under 1e-4 saturates the guard and
  skews the recompose (pre-existing display behavior, not a media bug; every data point
  matched raw·1e4 exactly). Verification fixed by rect-light intensity 1000 (raw ~3e-2):
  all channels then exact (mixed K 0.3/0.11/0.075 max dev 0.0001; 0.2 probes 0.0000).
- Test B (in-mesh glass slab, volume_absorb (1.2,0.7,0.35), sigma_s 0): direct AOV ratio
  exactly 1.0000 (shadow ray never crosses the mesh — documents the single-medium
  shadow-march approximation, no leak); indirect (0.8322,0.9025,0.9466) vs e^{−σa·chord}
  (0.8308,0.9019,0.9463), dev ≤0.2% (bounce-2 re-entry mixture).
- Regression: tank/layered-materials/glass-of-water new-vs-track11 EXR pairs: RMSE 0.0
  with identical means (bit-identical) — the med-guard keeps media-off scenes exact.
- Cost: media-off tank pair in one alternating pass +5.0% (5.528 vs 5.264, 640×360
  240f native); an earlier pass measured the same pair +9.3% (2.249/2.058) — power
  state moves absolutes ~2.4× between passes; the +5..9% band is the honest claim,
  same register-class lesson as track 11. Showcase native means: weather 12.9,
  jade-and-milk 10.6, underwater 8.0, jack-o 6.3 ms.
- Showcase retunes after eyeballing the first captures: pumpkin skin vol_a (0.35,1.1,2.6)
  + scatter 4.5 (olive cast fixed); vermeer 220/1.4 → 55/0.9, darker base, scatter 0.3,
  added still_life table + 2 candle emitters (vermeer_room alone is just the shell — the
  first capture read as an empty blown cream room); underwater tall block 2.6 → 2.0 and
  sphere 2.3 → 1.9 (the oversized block occluded the light — p6/p7 wall diagnostics),
  light back to classic-ish [0,0.85,0]/1.3/13, exposure 1.6.
- Open for future sessions: ambient medium has no UI row (scene file / --set only);
  vol_nee balance denominator uses pl+ps with ps = 1/2π (phase pdf — the shipped
  isotropic sampler's pdf; heuristic partner, exactness only proven where events
  don't fire); HG phase η, heterogeneous ratio tracking, overlapping-media additive
  Σσ, carving/CSG for jack-o windows (track 13 SDFs), per-scene RR-vs-native tuning.
