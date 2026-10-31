# Track 10 — Nested dielectrics: research notes (2026-10-31)

Saved for future reference. These notes preceded the implementation; the write-up of what was
built lives in CLAUDE.md ("Nested dielectrics") and docs/roadmap.md (track 10).

`web_search` was unavailable this session (no search API key), so sources were reached with
`web_fetch` through a DuckDuckGo Lite query page. Fetched 2026-10-31.

## The problem

A path that crosses several glass/water boundaries must bend rays and attenuate by the *pair*
of media on either side of each boundary, not by "air on both sides". Getting it wrong shows
up as:

- wrong refraction inside a liquid (the ice cube in a drink does not bend the tea behind it),
- hidden submerged objects (ice under the liquid surface invisible),
- a bright ring where a liquid surface meets the glass wall (a physically impossible
  "double refraction" event).

## Candidate techniques

1. **Interface tracking (an IOR stack)** — the oldest and simplest. Keep the list of dielectric
   volumes the path has entered and not left; entering pushes, exiting pops; the incident
   medium is the top of the list and the transmit medium is the surface hit (or the other way
   round for a back-facing hit). Correct for *properly nested* closed manifold volumes, wrong
   where volumes overlap (liquid surface intersecting the glass wall, ice poking through the
   liquid surface).
2. **Priority-based nested dielectrics — Schmidt & Budge 2002, "Simple Nested Dielectrics in
   Ray Traced Images", Journal of Graphics Tools 7(2), 1-8.** The industry standard. Each
   volume carries a *nested priority* (lower number = higher priority; ambient air is
   infinitely low priority). The interior list is the set of surfaces entered but not exited,
   and each hit is classified:
   - hit priority higher or equal to the best on the list -> **true hit**: shade, and take
     the incident IOR from the list and the transmit IOR from the surface (front side) or
     vice versa (back side, transmit from the next-best entry, or air if the list has one);
   - hit priority lower than the best on the list -> **false hit**: no intersection at all,
     continue the ray through the surface, but still update list membership (add when entering,
     remove when exiting);
   - a true hit that reflects does not update the list;
   - a surface with no priority set is always a true hit and never joins the list, so
     non-manifold geometry (cracks inside ice, hair, leaves) embedded inside glass works.
   Overlapping modelling then behaves like a boolean: the higher-priority volume cuts its
   space out of the lower-priority one, so the double events of (1) never happen.
   Used by RenderMan, Arnold, Houdini/mantra, Maxwell, Takua, RTXPT.
3. **pbrt-v4 "medium boundaries"** (not fetched; the repo path guessed 404): intersect *all*
   dielectric boundaries along the segment, sort them, and walk the resulting chain of media.
   Same idea as (2) but decided by geometry instead of an author priority; needs an
   IntersectAll traversal.

### Sources (fetched 2026-10-31)

- Yining Karl Li, "Nested Dielectrics" (2019-05-21), the clearest write-up of both, with the
  worked event lists for glass + liquid + ice:
  https://blog.yiningkarlli.com/2019/05/nested-dielectrics.html
  (its figures: ice cubes in tea in a glass; the failure images of pure interface tracking;
  Bitterli's Glass of Water re-rendered with red liquid so the liquid/glass interface reads.)
- NVIDIA RTXPT, DXR HLSL, same API family as this renderer:
  https://github.com/NVIDIA-RTX/RTXPT/blob/main/Rtxpt/Shaders/PathTracer/PathTracerNestedDielectrics.hlsli
  `ComputeOutsideIoR` (outside IOR from the interior list, stepping one entry down when
  exiting the current top material) and `HandleNestedDielectrics` (reject false hits and
  restart the ray just past them, with a `kMaxRejectedDielectricHits` = 4 (quality 1) / 16
  (quality 2) counter, then terminate or fall back to the non-nested case).
- RenderMan wiki "Nested Dielectrics" (REN25/REN26 pages; the REN26 page came back empty over
  fetch, the REN25 mirror is quoted by the search result): intersection priority plus the IOR
  set, "models are built volumetrically with outward-facing normals", overlapping glass +
  liquid with a priority on the glass.
- Reference list from that post: Novak, Selle & Jarosz 2014, "Residual Ratio Tracking for
  Estimating Attenuation in Participating Media" (ACM ToG 33(6)) — the unbiased alternative
  to ray-marched attenuation, and Takua uses the nested-dielectric system for transmittance
  along bidirectional connections.
- Wikipedia "Refractive index" (relative vs absolute IOR; Snell; TIR; Fresnel; reflectivity):
  https://en.wikipedia.org/wiki/Refractive_index
- Schmidt & Budge PDF mirrors found (researchgate / academia / escholarship / tandfonline,
  original link http://graphics.idav.ucdavis.edu/~bcbudge/deep/research/nested_dielectrics.pdf
  was unreachable); the algorithm is quoted in full from the Yining post instead.
- Arnold user guide "Nested Dielectrics" (IOR between two media, bubbles) and the Blender
  merge request "Cycles: Implement volume stack priority and nested IOR support"
  https://projects.blender.org/blender/blender/pulls/118478 — Cycles still needs a
  priority/masking system for overlapping volumes; the idea thread
  https://blenderartists.org/t/adding-ior-to-volume-shaders-for-better-nested-dielectrics-in-cycles/1613969.

## Typical IORs (for materials and showcase scenes)

air 1.0 (reference medium), water 1.333, ice ~1.27-1.31, window / crown glass 1.5-1.52,
acrylic ~1.45, glycerin 1.46, diamond 2.418. `Settings::Material.ior` is bounded 1..2.5 by
`validate()`, so all of these except diamond already fit; the water/ice/glass chain is
expressible today with no new setting.

## Decision for this codebase

Closest-hit `RayQuery` (`shaders/pathtrace.hlsl::intersect`) commits one hit per segment and
has no false-hit rejection machinery, and our objects are whole closed meshes (built-in shapes,
imported GLB/OBJ, `gen:` procedural), so the volumes can simply be modelled *inset inside each
other* instead of overlapping. That makes (1) enough for this milestone, and it needs:

- **no new data**: the IORs and absorption colours are already per material (`optical.x` IOR,
  `color_rough.rgb` Beer-Lambert tint), and entering/exiting is already known —
  `intersect` sets `h.front = dot(d, h.n) < 0` and flips `h.n` to face the ray, so a
  front-facing hit on a closed outward-normal mesh is "entering";
- the existing opaque/non-opaque split already keeps every transmission > 0 object in the
  any-hit range, so all nested boundaries are visited in the right order;
- **bit-identical existing renders**: with an empty stack, `eta = 1/ior` entering and
  `eta = ior` exiting, `f0 = ((1-ior)/(1+ior))^2` and `absorption = -log(color)` entering /
  0 exiting are exactly the old expressions, so presets 0/14/17 and the mesh packs stay
  unchanged;
- stack depth capped (12 volumes) and an exit that finds no matching entry treated as air, so
  a non-manifold or overlapping model degrades instead of corrupting the path.

Upgrades to keep in mind later, in rough order of value:

1. **Priority-based nested dielectrics (2)**: needs a per-object nested-priority value (a spare
   `surface_scale.z`/`w` channel, or a new PackedMaterial field), false-hit rejection with a
   bounded rejected-hit counter and a continuation RayQuery, and a UI row + scene field. This
   is what fixes ice cubes that pierce the liquid surface and liquid that touches glass.
2. **Attenuation along shadow/NEE rays through media**: `occluded()` is boolean today and there
   is a `jitter.w` "glass shadows" switch (dielectrics can be ignored by shadow rays). Correct
   glass-of-water shading wants Beer-Lambert along those rays, ideally ratio tracking (Novak
   2014); that is also the entry point for track 12 volumes.
3. Volumes/murky water is track 12; thin-film and layered BSDFs are track 11 and will want the
   same IOR-stack plumbing for a clearcoat over a dielectric.

## Modelled as (the convention this implementation assumes)

- the outer glass wall and the liquid volume are two closed meshes with the liquid inset by a
  hairline (a fraction of a millimetre at scene scale ~2 units), so liquid/glass reads as
  glass -> liquid with no coincident surfaces;
- solids inside the drink (ice, a straw, a lemon wedge) stay strictly inside the liquid volume
  while submerged, or strictly outside it; nothing pierces the liquid surface;
- a snow globe is a glass dome (sphere) over a base with its contents inside the dome's air,
  which needs no nesting at all beyond the dome; an aquarium is a glass box with a water box
  inset inside it and pebbles/fish inside the water.

## Verified on 2026-10-31 (the as-built numbers, kept so a later session can re-measure)

`captures/nested-slab.json`, `captures/nested_check.py` and `captures/reg_check.py` are the
transient scene + scripts (captures/ is gitignored); the shipped check for the same physical
model is the analytic comparison below.

- **The chain reads the pair, not air:** the head-on stack (air → glass 0.05 → water 0.30 →
  glass 0.25 → air → an unlit card at radiance 4) measured 2.433/3.096/2.989 against the nested
  Fresnel × Beer–Lambert analytic 2.454/3.111/3.022 (≤1.1%), while the pre-change build measured
  2.867/3.279/3.291 against its own air-on-both-sides analytic 2.901/3.292/3.323 (≤1.2%). The
  measured old/new ratio (1.178, 1.059, 1.101) lands on the analytic (1.192, 1.063, 1.112): the
  old build travels the far 0.25 of glass with *no* absorption (18% brighter in red) and uses
  air/water Fresnel 0.0204 at the liquid wall instead of glass/water 0.0043.
- **One boundary is untouched:** `cornell-water-tank` (one closed glass volume, nothing inside)
  agrees with the pre-change build to 1e-7 on the per-pixel means, RMSE 8.4e-05 — a handful of
  grazing samples where the Schlick base written `r0*r0` instead of `pow(base, 2)` crosses the
  stochastic reflect/refract threshold.
- **`glass-stack` (six glass slab volumes touching face to face) changes, RMSE 0.14:** a
  coincident pair pops to air and pushes back, so the seam gets two Fresnel coin flips where the
  physical answer is one glass/glass interface. That is exactly what the priority-based list
  with false-hit rejection fixes, and it stays on the not-done list.
- **Register lesson:** an interior list that caches per-slot derived state (id + optical index +
  absorption = 40 registers) measured +46% on the tank and +15% on `glass-stack` (640×360, raw
  native, alternating runs); the ids-only list (8 registers, values read back from `materials[]`
  by id) costs +5 to +7% on glass scenes and nothing on a scene with one boundary, and renders
  bit-identically (RMSE 0.0) to the cached form. This kernel is register-bound, so list state is
  charged to every pixel, including pixels that never touch a dielectric.
- **Two scan traps found while building this:** the loop that finds the list entry carrying the
  hit's material (`under`) and the loop that reads the entry below it (`under - 1`) must be
  *separate* unrolled passes — folding them into one pass reads a half-updated `under`, which
  silently turned every pop into an exit into air (the nested slab came out 166% too bright and
  the eta chain stopped telescoping); and `pow()` of a Fresnel base below one is NaN, which is
  why the base is squared by multiplication.
- Self-test 458 frames (scene files start at 289, the built-in pack is now 52); `cargo test` 58
  green including `nested_showcases_keep_the_volume_chain`; clippy clean at `-D warnings`.
