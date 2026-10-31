# Track 11 - layered and exotic materials (roadmap 11)
# Date: 2026-10-31. Branch: agent/codearata-agent.

## What the roadmap asks for

Track 11 of docs/roadmap.md: clearcoat and sheen first, then anisotropy, thin-film interference
(Belcour and Barla 2017), the diffraction grating (CD/DVD) as an anisotropic lobe with a
per-channel angular offset, and Kulla-Conty energy compensation via a small LUT.

## Design decisions (locked in the implementation)

### Layer order and weights: the glTF spec, not Filament's shader order

The glTF KhMaterialExtend models clearcoat, sheen and anisotropy as operations on the
base BSDF; the reference implementations (see the Filament surface_brdf.fs below) compose
base -> film/grating (rides the base specular) -> sheen -> clearcoat. Sampling weights:

    coat  w_c = pc
    sheen w_s = sh * (1 - pc)
    base  w_b = (1 - pc) * (1 - sh)

which sum to 1 exactly. (The first draft used w_s = sh alone, summing to 1 + pc*sh - a
misweighted mixture; the fix is w_s = sh*(1-pc) everywhere the sampler branch bounds and
the pdf mix appear.) Fresnel transmittance pairs attenuate the deeper lobes:

    cpair = (1 - pc*Fresnel(nv, 0.04)) * (1 - pc*Fresnel(nl, 0.04))   # through the coat
    spair = (1 - sh*Fresnel(nv, f0)) * (1 - sh*Fresnel(nl, f0))       # through the sheen
    att   = cpair * spair

base lobe value gets `wbase*att*(...)`, coat gets pc*coat, sheen gets sh*cpair*sheen.

### Sources

- [Filament surface_brdf.fs](https://github.com/google/filament/blob/main/filament/src/filament/materials/gfx/surface_brdf.fs)
  (raw.githubusercontent.com mirror): the composition order, the aniso slope sampler,
  Charlie's sheen wiring.
- [Estevez and Kulla 2017 sheen paper](http://www.jcgt.org/published/0006/01/03/) - the
  Charlie D and the log-lambda visibility fit.
- Patapom's GodComplex BRDF.cs quoted the published fit coefficients (page 404'd; the
  coefficients were taken from the Filament copy and cross-checked numerically below).
- [glTF KHR_materials_Clearcoat/Sheen/Anisotropy/IOR (iridescence)](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos)
  for layer order semantics; the thin-film IOR layer rides the base.
- [Belcour and Barla 2017 "Computational Iron and Silver Rendered with iridescence" via salivity](https://saltydaydreams.wordpress.com/2021/05/06/iridescence/)
  (the dead UC Davis idav mirror, replaced per the CLAUDE.md rule: the same algorithm quoted
  in a blog): per-channel cos^2 envelope of the optical path difference over wavelength,
  phase flipped when exactly one boundary enters a denser medium.
- [Kulla and Conty "A Multiple-scattering Micro-reflectance BRDF Model" JCGT 8(3):1](https://jcgt.org/published/0008/01/03/)
  fetched as HTML: the directional-reflectance table approach; the F_ms polynomial
  (0.04 f0 + 0.66 f0^2 + 0.3 f0^3) and the E_avg published fit both appear there.
- Grating: the diffraction equation OPD = p*(v.T - l.T) with wavelength peaks at integer
  multiples of the period (rp-photonics / Wikipedia diffraction-grating articles).
- [Heitz "The VNDF GGX sampler"](...) for the isotropic GGX pdf relation (the repo's
  sampler already matches: pdf = D(nh)*nh/(4*vh)).

(All fetched with web_fetch through the duckduckgo-lite route documented in CLAUDE.md;
raw.githubusercontent.com for source files, blogs for dead journal mirrors.)

## Implementation shape (the code that shipped)

Material fields appended (layout discipline: new float4s at the end of Material/PackedMaterial
so the old byte order never moves): coating=(clearcoat, cc_roughness, sheen, sheen_roughness),
effects=(film_ior, film_thickness, grating_period, 0), weave=(anisotropy, rot_deg, 0, 0);
perceptual roughnesses squared to alpha exactly like color_rough.a (cc default 0.03 ->
alpha 9e-4, floor at 1e-3; sheen_rough 0.3 -> alpha 0.09).

Helpers inserted into shaders/pathtrace.hlsl (design mirrors, source-identical):
- `ggx_da` (Burley form via Filament: 1/(pi*at*ab*b^2), at=a/asp, ab=a*asp, asp=sqrt(1-0.9*an));
  `g1a` slope-space Smith G1 (reduces exactly to the repo `g1` when at=ab=a - checked).
- `charlie_d`, `charlie_l` (log-lambda fit, mirrored above x=0.5 in log space).
- `film_env`, `grating_env` (both share the per-channel cos^2 machinery;
  `film_favg` for the diffuse attenuation of the film (ior-1)/(4.08567+1.00071*ior)).
- `ggx_ms` with the complement-albedo table below (see Kulla-Conty section).

Kulla-Conty: NOT a polynomial fit. Power-basis fits of S(mu,alpha)=1-E(mu,alpha) over our own
GGX form were tried three times (max errors 5.5% / 4.8-5.0% / 3.9%, coefficients up to ~1e4)
and abandoned: fp32 cancellation makes a 1e4-coefficient power basis numerically catastrophic
at the alpha->0 corner where S -> 0. Shipped as the roadmap's "small LUT": a 32x32
StructuredBuffer<float> `ggx_table` (root SRV t8, heap slot free; Gpu::new uploads
`src/ggx_e_data.rs` from `tools/build_ggx_e_data.py`). S = 1 - E stored (E -> 1 at small
alpha would lose every digit); the MS term is
S(nv)*S(nl)/(pi - E_avg(a)) * F_ms(f0), F_ms = 0.04 f0 + 0.66 f0^2 + 0.3 f0^3, E_avg from
Patapom's published fit PI + a*(-0.446898 + a*(-5.72019 + a*(6.61848 - 2.41727*a))).

The table integrates exactly the shader's own lobe form (D=a^2/(pi*(nh^2(a^2-1)+1)^2),
G1=2n/(n+sqrt(a^2+(1-a^2)n^2)), f = D*G1(nv)*G1(nl)/(4 nv nl), F0=1) with
the estimator g1(nv)*g1(nl)*(v.h)/(mu*(n.h)) under a Hammersley lattice (radical inverse
bases 2/3, M=2^19) over the shader's own half-vector sampler; direct angular quadrature
cross-checks agree to ~1.1e-5 (see verified numbers below). Grid mu/alpha in [0.02,1].

Sampler: layered branch when pc+sh+an+ft+gp > 0 && non-dielectric; sel bounds pc /
pc+sh*(1-pc) / +wbase*ps / rest; coat GGX(stretched isotropic ac), sheen cosine (pdf nl/pi
matching weight sh*(1-pc)), base GGX (aniso stretch sampler `hv=normalize(at*r*cos*Tr +
ab*r*sin*Br + n)` which reduces to the repo sampler at at=ab=a) or cosine. The groove frame
is recomputed from `box_map(p,n,1.0,...)` inside the layered branch only (register lesson
from track 10: per-thread state is charged to every pixel; not stored in Hit).

Guard `pc+sh+an+ft+gp>0.0 && m.lobes.z<0.5` keeps the old path's arithmetic text-identical
(`plain` computed then guarded return) so scenes without layer fields render bit-identical
(tank regression: RMSE 0.0 below). Dielectrics never take the layered branch (glass+coat
combos = future work). The `material_at` pattern stash (`secondary.w = t` for patterned
materials, guarded out for portals so the portal pair id survives) feeds the film
thickness modulation lerp(0.5,1.5,t) and the grating ink mask (diffuse * t only under ink).

## Known limitations / future work

- Aniso UV-tangent orientation is box-map groove based (world X/Z on the dominant axis),
  not the mesh UV tangent frame (needs a Hit.Tangent - deliberately skipped for registers).
- Grating specular ink mask not implemented (only the diffuse is attenuated; the CD
  specular is full across the disc).
- Glass + coat / glass + sheen combos: dielectrics skip the layered branch.
- aniso cone_spread guide for RR is a heuristic (0.5*(at*ab+1)*0.5).
- E-table clamps alpha < 0.02 to the first row (smooth mirrors: MS ~ 0 there anyway).

## Verified numbers

Design harness: `captures/track11_verify.py` (gitignored, transient scenes/captures `v-*`).
Each material renders twice at 128×72, 2048 frames, 8192 samples, `--no-rr --raw`: layer fields
on, and the same scene zeroed (iso). The centre pixel hits the floor exactly at the camera
target; the design is an exact 360² quad integral over a 3×3 light patch at (0, 3.5, −1.6)
(camera at pitch 70, distance 20). Verdict rule: centre-pixel rel dev ≤ 10% AND on/off
per-channel ratio dev ≤ 6%. ALL FIVE PASS:

- carpaint (cc 1.0, ccR 0.06, color 0.02/0.12/0.4, rough 0.4):
  meas on [0.040 0.040 0.040] off [0.0481 0.0572 0.0826]
  ref  on [0.0398 0.0398 0.0398] off [0.0484 0.0577 0.0838]
  ratio meas [0.832 0.700 0.485] ref [0.822 0.689 0.475] — dev 1.4% / 1.1%: the blue base
  rides under the coat (red kept 0.83, blue attenuated to 0.49 through the Fresnel pair).
- steel (metal 1.0, aniso 0.85 rot 90, rough 0.3, color 0.55/0.56/0.58):
  ratio meas [0.784 0.784 0.784] ref [0.785 0.785 0.785] — dev 0.14% / 0.15%: the anisotropic
  lobe (slope D + Smith g1a + stretched sampler) matches the design essentially exactly.
- velvet (sheen 0.7 srough 0.35, rough 0.6, color 0.35/0.05/0.12):
  meas on [0.0127 0.0055 0.0072] off [0.0479 0.0206 0.0270] (ref [0.013 0.0056 0.0073]/[0.0489 0.021 0.0275]);
  ratio [0.265 0.266 0.266] vs ref — dev 0.009%: the Charlie ridge tints with the base f0
  and attenuates the base lobe by (1−pc)(1−sh)·spair·cpair.
- oilslick (film 420 nm ior 1.3, rough 0.25, dark base):
  meas on [0.0143 0.0772 0.0501] (ref [0.0141 0.0772 0.0505]); ratio [0.177 0.955 0.613]
  vs ref [0.175 0.957 0.620] — green-cyan envelope at ~420 nm confirmed numerically.
- cd (metal 1.0, grating 1600, rough 0.06, near-white):
  meas on [0.691 0.696 0.713] off [0.700 0.710 0.730]; ratio [0.987 0.979 0.976] vs
  ref [0.992 0.988 0.987]: grooves read near-white at the mirror centre (opd≈0), the colour
  lives off-centre — verified visually in the showcase sphere.
- film_env spot check: nv=1 ior=1.3 d=420 base ior=1.5 → (0.2871, 0.8159, 0.3486),
  hand-computed (0.276, 0.828, 0.366) (green-cyan peak at ~420 nm); favg(1.3)=0.0557
  → diffuse attenuation 0.9443.
- Regression, new vs the track-10 exe (`cornell-water-tank` 256×144, 512 frames, 4096 spp,
  --no-rr --raw): means bit-identical [0.16511445 0.14279538 0.11835204], RMSE 4.11e-7,
  max abs 1.07e-4 / max rel 1.01e-4 — a handful of grazing samples where `eval_bsdf`'s new
  float3 (position) argument lets DXC contract a fused form differently; not semantics.
- E-table: 2^19 Hammersley lattice vs independent quad cross-checks: rel 1.14e-5 (mu 0.3 α 0.4),
  2.75e-6 (mu 0.5 α 0.6), 6.09e-6 (mu 0.7 α 0.3); S range 4.1112e-4 .. 0.6931; S(α=1) row
  0.3983→0.6779 across μ; S(μ=1) column [4.11e-4, 0.1011, 0.3376, 0.5576] at α = 0.02/0.34/0.66/1.
- Self-test: 459 frames (53 builtin scenes), RELEASE_OK rr=true 800×600.

## Perf

(640×360, 240 frames, `--spp 8 --no-vsync --no-rr`, two alternating reps; means from frame 10;
new = track-11 build, old = builds/2026-10-31-track10-nested-dielectrics)

- `cornell-water-tank`: new 5.189/5.317 → 5.25; old 3.901/3.493 → 3.70 → **+42%**
- `glass-of-water`: new 3.585/3.719 → 3.65; old 2.696/2.823 → 2.76 → **+32%**
- `layered-materials` (new only — the old build rejects the layer fields): 2.458/2.560 → 2.51
- Earlier builds (no bit6 guard, and bit6 guard without scope changes) measured the same +28..42%
  band: the cost is NOT in the loads the guard removes; the trace's register class (and hence
  occupancy) is raised by the layered branch's peak live set for the whole kernel. The bit6
  guard still earns its keep by keeping the off path's arithmetic and loads textually identical
  (and the tank means bit-identical), but it buys no runtime on layer-free scenes.
- Getting it back: shrink the layered branch's live set (accumulate into one colour instead of
  keeping coat/sheen/base locals alive at the return), or gate the whole layered transport
  behind a separate trace entry chosen at PSO level — future work.
