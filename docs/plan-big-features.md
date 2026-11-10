# Plan for the remaining big features (2026-11-10)

Scoped by seven read-only Sonnet scouts against the parity tree; their full output (files, line numbers, risks) is in the workflow journal `wf_76dff41a-db9`. The roadmap (`docs/roadmap.md`) stays the list; this file is the order and the staffing.

## Model tiers (owner rule, 2026-11-10)

"works the same sonnet if possible, opus low otherwise. check with higher efforts only when needed."

| Tier | Used for |
|---|---|
| **sonnet** | UI, Rust plumbing, Settings fields, removals, scene files, docs, tests, tools, CPU ports of math that already exists |
| **opus-low** | shader and light-transport math, new integrator code, GPU resources on both backends, threading/lock changes |
| **opus-high** | only a check that needs it: review of new transport math, or diagnosing an oracle failure. Skipped when the oracle passes. |
| haiku `verifier` / sonnet `reviewer` | as in CLAUDE.md: long runs, and the review before every commit |

Every lane runs in its own worktree off the latest `cpu-playable`, heavy commands through `target/safe.sh`, one heavy job at a time.

## Order

### Wave 0: now, while the CPU parity job finishes (new files only, no collision)
| Step | Tier | Size |
|---|---|---|
| Night sky N1: `tools/prep_sky_data.py` reduces HYG to `stars.bin` (about 1-2 MB) and a star-free 2k Milky Way map; THIRD-PARTY-NOTICES lines | sonnet | M |
| Night sky N2: `src/astro.rs` (sidereal time, precession, RA/Dec to local), calendar date in `PhysicalSky`, unit tests (Polaris altitude = latitude, Sirius transit, sun RA agrees with `sun_local()`) | sonnet | M |
| Spectral audit: `bdpt_spectral` and `photon_gather_spectral` already exist; reconcile notes and roadmap | sonnet | S |
| Accuracy audit table: each GPU shortcut, where, expected bias. Finding: the CPU copies the shortcuts (rough glass lerp, roulette), so the oracle can't see them | sonnet | S |

### Wave 1: after the parity tree is merged and committed to `cpu-playable`
Four lanes in parallel; file collisions noted. (CPU AOVs, LPE, BDPT, photons and battle mode are in the CPU parity run itself, 2026-11-10.)

1. **Cleanup and UI** (sonnet), one lane, in this order because they share `main.rs`, `ui.rs`, `chrome.rs`:
   - Remove the screensaver (S) and light painting (S+M). Keep the engine's keep-the-sums path that motion-blur video export and CPU timeline export use; rename it `keep_exposure`. Free self-test frames 240-242.
   - Denoiser enum `{Off, Native, Rr, ...}` replacing the `rr` / `denoise` bools, with serde migration (M). Do it before anything else adds Settings fields.
   - UI review batches A (chrome, overlays), B (Stage and Object inspectors), C (Look/Render/Export tabs), E (scene browser, mesh import thread; opus-low review of the lock change), then D (`main.rs` input), each M.
2. **Night sky** (N3-N8):
   - N4 Milky Way, airglow, light pollution in the bake; random stars gone (opus-low, M). Visible result first.
   - N2b (from wave 0): `sun_local()` uses Cooper's declination and an approximate equation of time, up to 1.1° off the exact chain the stars use. Move the sun (and check the moon) onto `astro.rs` so sun, moon and stars agree; physical-sky scenes shift by up to about 1° (sonnet, S).
   - N3 stars as point lights (owner, 2026-11-10: starlight lights surfaces too, so the NEE branch stays): separate power table for NEE, cone test through a sphere grid for camera and mirror chains, both backends, CPU twin (opus-low, L). N3b review (opus-high, S) only if the oracle disagrees.
   - N6 exposure, auto (owner, 2026-11-10: **on by default everywhere**, not only at night, with the Eyes look; every scene retested and camera work where needed, per the investigation in wave 0) (histogram, adaptation speed) and manual EV / ISO-shutter-f (opus-low, L); N6b pre-exposure so night radiance doesn't underflow fp16 denoiser inputs (opus-low, M, required for DLSS).
   - N7 Eyes look (scotopic, Purkinje) and the camera preset B (opus-low, M); N5 tweaks as Settings (sonnet, M); N8 scenes, docs, self-test, oracle (sonnet, S).
   - Collides with: `sky.rs`, Constants/cbuffer, `stage.rs`, `look_tab.rs` (UI batch B/C: land the UI batches touching those first or rebase).
3. **First-person avatar** (sonnet, with two opus-low checks): general `Object.camera_hidden` flag on the existing hidden instance mask (the CPU already honours it), `avatar::objects(pose)` boxes injected per frame like the flashlight, flashlight/head shadow fix and TLAS update cost checks (opus-low). About a week. Collides lightly with `main.rs` (rebase after UI batch D).
4. **Coplanar glass, option B (owner's choice, 2026-11-10):** a grounded-dielectric offset in geometry that both renderers read, so user scenes agree too (opus-low, L). Lands before a new full-oracle baseline. Asked at start: automatic for transmissive grounded objects or a per-object toggle, and the gap size (5-10 mm). Shares `geometry.rs` and physics rest height with the avatar lane: rebase one on the other.

### Auto exposure on by default (investigated 2026-11-10)
Findings: `exposure` is one display-only scalar (0..10, already outside `transport_key`); there is no camera ISO/shutter/f-number and no absolute unit. Radiance 1.0 = a white surface under a sun of intensity 1. Proposed calibration: **1.0 = 32,000 cd/m2** (noon sun about 100 klux), one constant shared by `post.hlsl` and `reference/display.rs`. Scotopic below 3e-7, mesopic below 1e-4 in scene units. 98 built-ins: exposure 0.35..9.0, 40+ at 1.0, 11 use the physical sky, none an HDRI. Hand-lit "night" scenes (nighthawks, jack-o-lantern, lighthouse-fog) are photopic at that scale, so the Eyes look would only trigger on physical-sky nights unless their lights are dimmed.
| Step | Tier | Size |
|---|---|---|
| E1. `SCENE_CD_M2` and luminance helpers, notes section | sonnet | S |
| E2. Settings: `auto_exposure`, `ev_bias` (keyable), speed, meter mode, `eyes`; exposure range widened; `transport_key`, `keep_output`, `preset()`; old scene files load as manual so authored looks survive, new scenes and defaults are auto | sonnet | M |
| E3. GPU histogram meter (centre-weighted, top 2-5% and bottom 1% ignored) into a persistent buffer, adaptation on simulated time (video and timeline deterministic), settle instantly for stills, `--capture` and converged images; CPU twin | opus-low | L |
| E4. Eyes look: scotopic desaturation and Purkinje shift between about 3 and 0.01 cd/m2, GPU and CPU | opus-low | M |
| E5. Pre-exposure for RR / SR / XeSS (night radiance underflows fp16; the bridges hard-code `InPreExposure=1`); rerun `tools/rr_stability.py` | opus-low (opus-high review only if RR shifts) | M |
| E6. UI, remote and d-pad act on the bias in auto; false colour relative to auto | sonnet | M |
| E7. Tests: self-test, oracle and PNG comparisons pin `auto_exposure=false`; meter determinism tests | sonnet | M |
| E8. Built-ins regenerated with `ev_bias = log2(old exposure)`, then retuned by eye; furnaces and measured-cornell stay manual/linear | sonnet | L |
Retest first: dark-side (9.0), jack-o-lantern, lighthouse-fog, nighthawks, colonnade-night, night-skyline, optics-bench, stage-beams, disco-floor, street-bokeh, gem-display, sunlit-room-day-cycle (keyed exposure becomes bias keys), vermeer-room, underwater, the dusk scenes, overcast-still-life, sun-greenhouse, veach-mis, panorama-gallery. Look for: bright emitters crushing the rest, black backgrounds lifted into noise, pumping while moving, night going scotopic, the day cycle staying monotonic.
Owner question at start: dim the hand-lit night scenes' lights to real levels (so they go scotopic) or keep them photopic.

### Wave 2
- **Accuracy mode** (audit: `docs/claude/accuracy-audit.md`; start with B7, an unexplained 0.2-0.6% NEE darkening in the default render, opus-high M; then a CPU "truth" switch so the oracle can see shortcuts the CPU copies): exact GGX BTDF on the CPU (opus-low M, opus-high S review), measure each shortcut (sonnet M), GPU port behind a variant so defaults don't change (opus-low L), Accurate bundle with LOD-0 textures (sonnet M, opus-low S). Collides with spectral part 2 in the lobe code: run before it.
- **OIDN 2 for stills** (sonnet S+M+M, CPU device, both backends through readback). Needs a download approval.
- **Volumetric clouds** (C1-C8): after the night sky's pre-exposure lands. Path tracer only on day one.

### Wave 3
- **Curved space** (opus-low, about 1.5 weeks for camera lensing): one gradient-index RK4 march covers the black hole (isotropic Schwarzschild), the hyperbolic and the spherical room, inside a bubble so other rays cost nothing. GI through the bubble judged after seeing the noise.
- **Spectral part 2** (audit: `docs/claude/spectral-audit.md`; BDPT is already spectral, photon gather camera side too, photons deposit RGB): finish BDPT and photon camera side (opus-low L+M, opus-high S review), LPE and path visualizer (opus-low), blackbody leftovers (sonnet M), fluorescence/black light (opus-low L).

### Wave 4: spikes first, then decide
- **Gaussian splats** (about 2 weeks for the stochastic version): AABB BLAS reusing the SDF path, .ply loader (sonnet), shader and CPU twin (opus-low).
- **Polarization**, minimal: CPU-first Stokes for smooth lobes, a polarizing filter, a GPU smooth-lobe variant. After spectral part 2.
- **Neural radiance cache:** cooperative-vector feasibility spike (opus-low M) before any build.
- **NRD:** worth it only for good denoising on non-NVIDIA GPUs; the diffuse/specular split is the big cost.
- **Reflex:** through `VK_NV_low_latency2` on Vulkan (Windows and Linux) and NVAPI/Streamline on D3D12; small. Frame Generation dropped.

### Windows session (whenever the owner is on Windows)
D3D12 build of everything above and the self-update checks (UAC relaunch, `SHA256SUMS`, installer).

## Dropped by the owner (2026-11-10)
- OptiX denoiser, the D3D12 display-only device for GPUs without ray tracing, DLSS Frame Generation.
- Kept although the scouts suggested dropping: four wavelengths and the Jakob-Hanika uplift (spectral part 2, low priority).

## Questions asked when each lane starts
- UI: O for auto orbit and Alt+D for the denoiser; the five Look bundle names; a grouped top bar or just correct wrapping.
- Night sky: real calendar date and default year; Milky Way at 2k or 4k; ship in the installer or as an asset download; planets now or later.
- Avatar: fly mode, collider or visual only, head shadows, default on or off, a public "visible to camera" checkbox.
- Coplanar glass B: automatic or per-object, gap size.
- Accuracy: CPU exact BTDF as the CPU default; roulette off or gentler in Accurate; 64 bounces vs TDR.
- Clouds: procedural only or imported volumes; fog and clouds together; wind.
- Curved space: camera lensing only, or GI through the bubble; redshift model.
- Splats: self-luminous or relit; demo capture.
