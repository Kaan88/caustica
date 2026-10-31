# Track 9 — Physical sky and day cycle: research notes (2026-10-31)

Saved for future reference. These notes preceded the implementation; the write-up of what was
built lives in CLAUDE.md and docs/roadmap.md.

## Candidate models

1. **Hosek–Wilkie 2012** — "An Analytic Model for Full Spectral Sky-Dome Radiance"
   (Lukas Hosek & Alexander Wilkie, ACM TOG 40? no — SIGGRAPH 2012, ToG 37(4)).
   A *fitted* model: quintic basis coefficients fitted to brute-force atmosphere simulations.
   Known weak points of Preetham (sunsets, high turbidity) are much improved. Drop-in for
   Preetham-style code. 3-clause BSD data. Chosen for this milestone.
2. **Prague model (2021)** — Vévoda, Wilkie, Hošek, Iser et al., "A Fitted Radiance and
   Attenuation Model for Realistic Atmospheres" (ACM TOG 40(4), SIGGRAPH 2021): post-sunset
   patterns, finite-distance attenuation, observer altitude, polarisation. Much larger data
   file; https://github.com/PetrVevoda/pragueskymodel. Keep for a later upgrade.
   Page: https://cgg.mff.cuni.cz/publications/skymodel-2021/

## Hosek–Wilkie details

- Official project page: https://cgg.mff.cuni.cz/projects/SkyDomeAp/ (the linked
  `HosekWilkie_SkylightModel_C_Source.1.4a.zip` returned 404 from the guessed URL on
  2026-10-31; the datasets below are the same data).
- Model data mirrored as raw files by phoekz/hw-skymodel (MIT wrapper around BSD data):
  https://github.com/phoekz/hw-skymodel — `src/rgb/params-r|params-g|params-b`
  (4320 bytes = 1080 f32 LE each) and `src/rgb/radiances-r|g|b` (480 bytes = 120 f32).
  Downloaded into `docs/research/skylight-2012/` (2026-10-31); copies are baked into the app
  from `assets/sky/` with `include_bytes!`.
- Dataset layout (from hw-skymodel/src/rgb/mod.rs, fetched 2026-10-31):
  - `params-<channel>`: 20 blocks of 54 f32 = (albedo half ∈ {0,1}) × (turbidity index 0..9,
    turbidity = index+1) × (6 quintic control points × 9 model parameters), stored
    quintic-major: index within block = j*9 + i.
  - `radiances-<channel>`: 20 blocks of 6 quintic controls for the solar radiance.
  - CPU per-frame: `t = (elevation_rad / (π/2))^(1/3)`; for turbidity τ with integer part
    n and fraction f, blend four blocks with weights
    s0=(1−a)(1−f), s1=(1−a)f, s2=a(1−f), s3=a·f (a = ground albedo per channel);
    each block's 9 parameters = quintic Bezier sum of its 6 controls at t.
  - Shader/CPU radiance, per channel c with solar radiance r_c and 9 parameters p0..p8:
    `radiance = r_c · (1 + p0·exp(p1/(|cos θ| + 0.01))) · (p2 + p3·exp(p4·γ) + p5·cos²γ
    + p6·mie + p7·|cos θ|^(1/2))`, where `mie = (1+cos²γ)/(1+ρ²−2ρ·cos γ)^1.5` with
    ρ = p8. (Verified against hw-skymodel/mod.rs; the `abs` on cos θ folds the ground
    hemisphere through.)
- Coordinates (their figure `coordinate-system.png`, verified by viewing the image 2026-10-31):
  **θ = view zenith angle** (0 = straight up), **γ = scattering angle between view direction
  and the sun direction**. `elevation` input = solar elevation above the horizon, radians,
  0..π/2; turbidity 1..10; albedo 0..1 per channel.
- License (saved in `docs/research/skylight-2012/`, quoted by hw-skymodel's `hosek-wilkie-license.txt`):
  3-clause BSD, "Copyright (c) 2012 - 2013, Lukas Hosek and Alexander Wilkie".
- The 2012 model is **daytime only** (solar elevation ≥ 0). It has no sun *disc* in the sky
  function; the 2013 follow-up ("Adding a Solar Radiance Function to the Hosek Skylight Model")
  gives the solar disc radiance; hw-skymodel's `radiances-*` data is that solar radiance, and
  the dome radiance is expressed in units of it (`radiance()` multiplies by r_c).
- Night: not modelled by 2012 HW. Plan: below-horizon blend to a procedural night hemisphere
  (zenith dark navy, horizon keeps a warm remnant near the sun's azimuth); moon and stars
  deliberately not done this milestone.

## Sun position from time of day

Standard NOAA-style approximation (e.g. the Wikipedia "Solar position algorithm" low-accuracy
section), using our units:
- B = 360/365.24 · (dayOfYear − 1) degrees; declination δ = asin(sin(23.44° · B))? — implement
  as δ = 23.44° · sin? no: δ = 23.44° × sin(B) is the classic low-accuracy form, accurate to a
  few degrees; good enough.
- Equation of time E = 9.87·sin(0.5B) − 7.53·sin(B) minutes; solar time = local time + E/60.
- Hour angle H = 15° · (solar time − 12); zenith: cos θz = sin φ sin δ + cos φ cos δ cos H,
  elevation = 90° − θz; azimuth from north: cos θa = (cos θz sin φ − cos δ)/(sin θz cos φ),
  az = θa, az = 360° − θa when H > 180°.
- World direction with our equirect convention (`environment.rs`: u = φ/2π + 0.5, φ = atan2(x, −z)):
  north = −Z, sun_dir = (sin az · cos e, sin e, −cos az · cos e).
- Solar disc: angular diameter 0.53° (mean), matching track 7's sun default.

## Decisions for the implementation

- Bake the sky on the CPU into the existing `environment::EnvMap` at 2048×1024 and push it
  through `Gpu::set_environment` — full reuse of the u30/u31 HDRI pipeline and its CDF
  importance sampling (the roadmap's own suggestion: "Bake it into the environment map on
  change and reuse the HDRI CDF path"). The sun disc is baked (hard edge, ~3 px at 2048);
  camera-ray background sees it like any HDRI.
- New `Settings` floats so the animation track (item 8) can key the day cycle for free:
  `sky_model` (0/1), `sky_time` (0..24 h), `sky_latitude` (−85..85°), `sky_day` (1..365),
  `sky_turbidity` (1..10). Ground albedo = floor material colour clamped per channel to 0..1
  — physical and free. `hdri_strength` doubles as the sky multiplier when the model is active.
- Environment reload in `main.rs` currently keys only on `Settings::hdri`; extend the key with
  the sky fields so a re-bake happens on any change (and reset accumulation like an HDRI swap).
- Showcase: a built-in scene file with a big window (thin-box room shell from
  `room_shell()` helpers) lit by the physical sky; a keyed day cycle proves track 8 + track 9
  integration (and pairs with the roadmap's "time-lapse through a window").
- Self-test: cycle `sky_time`, assert solar elevation/azimuth values and that the bake
  validated (map not black, sun texel count sane), plus per-frame emitter/triangle counts stay
  stable (the sky is not in the BLAS).

## Open questions deferred

- Whether the baked sun should also spawn an analytic `LightKind.Sun` automatically for crisp
  long shadows (baked-disc shadows are as sharp as any HDRI sun; kept as manual for now).
- Prague-model upgrade; night with moon and stars; `HDRI` + sky combined (currently exclusive).

## Implemented (2026-10-31, final — what actually shipped)

- The classic NOAA-style set above was NOT shipped: its constants (9.87/7.53) could not be
  verified online, and it carries phase errors near the solstices. Shipped the exact ecliptic
  form instead (in `src/sky.rs` `sun(day, latitude, time)`):
  - λ = 90° + (day − 172.4)·360/365.25 (anchored at the June solstice, so day 172.4 ⇒ λ = 90°);
  - ε = 23.44°, δ = asin(sin ε · sin λ), α = atan2(cos ε · sin λ, cos λ);
  - hour angle h = (time − 12)·15° + wrap(λ − α) — the difference λ−α IS the equation of the
    equinoxes (±10 min); the ±8 min equation of center from orbital eccentricity is omitted
    (artistic choice for a day cycle);
  - elevation = asin(sin φ sin δ + cos φ cos δ cos h); azimuth from north via atan2(east, north)
    with east = −cos δ sin h, north = sin δ cos φ − cos δ cos h sin φ (verified: noon ≈ 178°
    due south at lat 47 in June, rising east / setting west, night before sunrise);
  - world dir (north −Z, east +X): `Vec3(sin az · cos e, sin e, −cos az · cos e)`.
- **Solar-scale lesson (important for any future sky work):** the 2013 dataset solar radiance is
  only a handful of times the dome (turbidity 3, elev 45: solar_r ≈ 6.18 vs zenith dome ≈ 2.1)
  — right for how the sun looks, not physically consistent with a 0.53° disc. The disc therefore
  carries `SOLAR_GAIN` (final value 8000); a real room's sun-patch-to-sky-pool ratio measured
  ~3× through the showcase window with it. Re-tune gain ∝ (0.53/Ø)² if the disc ever changes.
- Verification trail: the sealed-window room renders looked ambiguous, so two checks settled it
  — (1) open-air desert-monolith with sky_model=1 shows crisp long disc-driven shadows;
  (2) a raw `--no-rr --raw` EXR sampled numerically (brute-force float alignment from the file
  end): sun pool ~3.5 vs ambient ~1.2 near the window. Lesson: prefer `tools/exr.py`
  (`exr.read`) for EXR numbers; the image crate's tiled writer defies casual hand-parsing.
- Shipped with 6 `sky.rs` unit tests, self-test 285–288 (289 scripted + 166 scene files = 455
  frames), the `morning-window-room` builtin showcase, UI sliders, and the CLAUDE.md
  "Physical sky and day cycle" section. `hdri_strength` stayed the HDRI's own multiplier;
  the bake key `sky_bake_key` (five sky fields + floor colour) is separate from the HDRI load
  check.
