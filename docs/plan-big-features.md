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
   - Denoiser enum `{Off, Native, Rr, ...}` replacing the `rr` / `denoise` bools, with serde migration (M). DONE 2026-11-10 (engine-notes "Denoiser enum"). Do it before anything else adds Settings fields.
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
| E8. Built-ins regenerated with `ev_bias = log2(old exposure / settled metered exposure)` (the first formula, `log2(old exposure)`, double-counted the meter), then retuned by eye; furnaces and measured-cornell stay manual/linear. DONE 2026-11-10 (engine-notes "Built-in scenes and auto exposure") | sonnet | L |
Retest first: dark-side (9.0), jack-o-lantern, lighthouse-fog, nighthawks, colonnade-night, night-skyline, optics-bench, stage-beams, disco-floor, street-bokeh, gem-display, sunlit-room-day-cycle (keyed exposure becomes bias keys), vermeer-room, underwater, the dusk scenes, overcast-still-life, sun-greenhouse, veach-mis, panorama-gallery. Look for: bright emitters crushing the rest, black backgrounds lifted into noise, pumping while moving, night going scotopic, the day cycle staying monotonic.
Owner question at start: dim the hand-lit night scenes' lights to real levels (so they go scotopic) or keep them photopic.

### Integration of the six running lanes (from the plan review, 2026-11-10)
Running: exposure and accuracy (base a752d0f, with the top-bar fix and pipeline cache patched in), headless, bounces, egui and OIDN (base 80b14a1). Backups of each lane's diff: `/home/kaan/caustica-checkpoint-2026-11-06/lanes-*`.
- **Merge order:** headless, OIDN, egui, then bounces and accuracy back to back with one shared oracle and self-test run (they edit the same path-tracer loop, `pathtrace.hlsl` ~1470/1701-1708 and `reference/integrators/path.rs`), exposure last.
- **`cbuffer Frame` order:** exposure (`meter`, `meter_ctrl`) and bounces (`bounce_split`) both append; bounces merges first, so `bounce_split` comes before `meter`. Mirror in `Constants` and the Vulkan layout; add a size/offset test.
- **Exposure in two merges:** the engine (E1-E5, E7, E7b) with `auto_exposure` defaulting off, then the default flip with E6 and E8.
- **Follow-up fixes after the four-lane workflow ends:** bounces: count glass against the specular budget only at roughness ≤ 0.1 (the delta flag includes rough glass through the lerp shortcut); the Realtime bundle must restore the scene's sampling when left, and must not read as "Good 67%" after an edit. OIDN: guides from the sample-averaged first-hit albedo and normal (follow delta chains), `cleanAux` only once they converge.
- **Spectral BDPT/photon 3x (FIXED 2026-11-10 at the merge, engine-notes "Merge of the six lanes"; confirmed 2026-11-10, street-bokeh with a dispersive lens: BDPT 2.990x and photon 2.985x the path tracer, 1.0x with dispersion off):** `camera_ray` (`pathtrace.hlsl` ~925) multiplies `camera_weight` by 3 for hero-only and `hero_rgb` (`integrators.hlsl` ~108) applies it again. Fix at merge: `if(!hero_only)camera_weight*=float3(3,0,0);`, check the BDPT t=1 splat (~403), mark the spectral-audit row confirmed. Renders: `/home/kaan/caustica-tmp/now/spec3/`.
- **Open (found 2026-11-10, predates the merge): GPU spectral photon mapping reads about 8% high on street-bokeh** against the CPU path tracer (`oracle.py --scenes street-bokeh --integrator 2 --cpu-integrator 0 --set spectral=true --set dispersion=0`: 1.085, z 3.0; the d40b55c build gives the same 1.0854). The CPU photon gets 1.008 on the same comparison, GPU BDPT 0.995, RGB photon 1.012, and spectral photon on preset 0 0.9999; so it is the GPU photon pass in spectral mode with something in this scene (lamps, lens system), not the hero weight. Belongs with Spectral part 2 (photons deposit RGB).
- **Lane tools (lane-tools worktree):** `tools/lane_overlap.py` (rerun right before each merge; today 11 textual conflicts, 3 special groups), `docs/claude/d3d12-on-linux.md` (DXIL compiles here today, 31/31 entries validated; a Rust cross-check needs the `rust-std` for x86_64-pc-windows-msvc, owner approval pending), the per-lane D3D12 checklist in `docs/claude/windows-handoff.md`.
- **D3D12 debt:** a per-lane checklist with file:line in `docs/claude/windows-handoff.md`; no tag or release until a Windows session clears it.
- **Owner decisions on the review (2026-11-10):** agrees with the review's findings. NRD cut (Won't do). Neural radiance cache: undecided; a world-space hash-grid radiance cache (SHaRC-style, plain compute, CPU twin for the hashing) is agreed, possibly as part of the Realtime preset. Four-wavelength spectral (with the Jakob-Hanika uplift) and fluorescence: parked, to review again later. Volumetric clouds: kept (wave 2, after pre-exposure). Curved space: parked for a future review, with the review's design holes recorded (bubble boundary refraction, straight shadow rays across the bubble, explicit redshift model, analytic tests near the photon ring). Gaussian splats and polarization: parked for future review with curved space (the review's design notes kept: splats with triangle proxies and baked-lit/no-NEE, polarization needs polarization-aware rough reflection for the floor demo); instead, a `.caustica.zip` scene pack (M). Realtime preset: allowed as an explicit exception to the no-performance-mode rule, as a Mode (Realtime / Standard / Accurate) separate from the reconstruction bundles, restoring the scene's own sampling when left. Redesign B: full, as specified, with pinnable live widgets (EV bias, Mode, denoiser...), minus export Pause and tags. Auto exposure stays a scene setting (authored look and keyed bias travel with the scene; old files load manual).
- **Earlier text, superseded where it conflicts:** the suggested cuts (shrink redesign B to a first slice, export Pause, collections/tags, NRD, the NRC spike, four-wavelength spectral, fluorescence, possibly volumetric clouds) and the new-idea list from the review.

### Backlog from the plan review (owner agreed 2026-11-10), after the six lanes merge
At most three lanes at a time, chosen so their files don't overlap (review advice).
- **Now, no source collisions:** two locks in `safe.sh` (done 2026-11-10: build.lock 14 GB for cargo, gpu.lock 8 GB for the rest, `SAFE_EXCLUSIVE=1` for timing, `*.holder` files show who holds each); D3D12 type-check and DXIL compile on Linux (feasibility first); the D3D12 debt checklist; a lane-overlap preflight script; the suspected 3× spectral BDPT/photon brightness (one oracle run).
- **Fixes:** rename `keep_exposure` to `keep_sums`; the Vulkan GUI uploader re-uploads the whole atlas on a partial delta; embedded-scene and panorama tags miss the `save_buffer` paths; the specular bounce limit is ignored by BDPT and photon mapping (document or extend); Russian roulette counts only non-specular vertices when the split is on; OIDN guides; the Realtime bundle fixes.
- **Small features:** every saved image carries its scene (open a render to get the scene back); drag and drop onto the window; a crash-report bundle; copy image to clipboard; copy/paste materials and an eyedropper; live reload of the open scene file; AE-L (in E7b); palette entries for the new toggles; ask the AI for a Rhai script in the console; a shared Settings-field checklist test; an exposure-invariance test for transport; an analytic light-level sanity table; a built-in benchmark with a shareable score; scene from a reference photo.
- **Medium features:** the Realtime / Standard / Accurate Mode; a world-space hash-grid radiance cache (candidate for the Realtime Mode); the `.caustica.zip` scene pack; stop at a noise target (stills, headless, high-res export); an all-scene visual diff between two builds; an MCP server over the script API (`caustica --mcp`); camera shots on number keys; real lamp spectra (sodium, fluorescent, LED, mercury); a batch render queue; explain this pixel (CPU reference).
- **Redesign B:** full as specified, with pinnable live widgets, minus export Pause and tags; the lane controls get their rows in the redesign tables (replaces the extra egui inventory step); answer §9 with the review's proposals unless the owner objects.
- **Parked for future review:** neural radiance cache, four-wavelength spectral and Jakob-Hanika, fluorescence, curved space, Gaussian splats, polarization. **Won't do:** NRD.

### Between waves 1 and 2: egui 0.31 → 0.37
After the exposure and accuracy lanes merge and before any UI redesign: three steps, about three days, sites and risks in `docs/egui-upgrade-plan.md` (written 2026-11-10 by a side session; not started). Extra egui step (owner, 2026-11-10), after every lane that adds UI has merged on top of the 0.37 port (exposure E6, accuracy Accurate bundle, bounces `specular_bounces` and the Realtime bundle, OIDN still denoiser, and any other new control): (1) port those additions to the 0.37 APIs and the menu/popup behaviour; (2) list every control the lanes added, where it sits now, and a proposed place in today's layout and in redesign B, with screenshots at 1280/1920/2560; (3) show the owner only the placements that are a real choice, as mockups, and apply the answers. Sonnet for the port and inventory; the owner decides placement.

Then the UI redesign: layout B (tool rail plus inspector), the side session reports the owner chose it, spec in `docs/ui-redesign-b.md` (open questions in its section 9; not started).

### Wave 2
- **B1 specular bounces** DONE 2026-11-10 (engine-notes "Specular bounce budget"): `Settings::specular_bounces`, default 0 = legacy; GPU and CPU path tracers. Open: owner decides whether to raise the default (changes baselines); D3D12 build and self-test on Windows.
- **B2 Realtime preset** DONE 2026-11-10 (engine-notes "Realtime preset"): `Bundle::Realtime` in the Quality menu; measured on five scenes plus an orbit check. Open: gentler bounces 4-5 variant, more ghosting scenes, Windows build.
- **Accuracy mode** (audit: `docs/claude/accuracy-audit.md`; B7 done 2026-11-10: the darkening was the shadow stop fixed 2026-11-02 plus noise, and a shading-normal visibility mismatch found on the way is fixed in both renderers; CPU "truth" switch built 2026-11-10 with its measurement matrix, hooks left for the exact versions): exact GGX BTDF on the CPU (opus-low M, opus-high S review), measure each shortcut (sonnet M), GPU port behind a variant so defaults don't change (opus-low L), Accurate bundle with LOD-0 textures (sonnet M, opus-low S; done 2026-11-10, engine-notes "Accurate quality bundle"). Collides with spectral part 2 in the lobe code: run before it.
- **OIDN 2 for stills** (sonnet S+M+M, CPU device, both backends through readback). Download approved 2026-11-10; O1 DONE: OIDN 2.5.1 CPU device fetched into `vendor/oidn` (see docs/claude/setup.md), listed in THIRD-PARTY-NOTICES.md. Default, changeable: Windows ships `OpenImageDenoise.dll`, `_core.dll`, `_device_cpu.dll`, `tbb12.dll` only (no tbbbind NUMA helpers). O2 DONE 2026-11-10 (integration, measurements in engine-notes "OIDN for stills"): runtime loading, `still_denoise` setting and `--oidn`, PNG/JPEG captures and Save image on both backends through readback, the CPU renderer (`--reference` and the software view's converged image); EXR stays raw. Open: the GPU's live converged view; a Windows check of the DLL copy and the installer.
- **Volumetric clouds** (C1-C8): after the night sky's pre-exposure lands. Path tracer only on day one.

### Wave 3
- **Curved space** (opus-low, about 1.5 weeks for camera lensing): one gradient-index RK4 march covers the black hole (isotropic Schwarzschild), the hyperbolic and the spherical room, inside a bubble so other rays cost nothing. GI through the bubble judged after seeing the noise.
- **Spectral part 2** (audit: `docs/claude/spectral-audit.md`; BDPT is already spectral, photon gather camera side too, photons deposit RGB): finish BDPT and photon camera side (opus-low L+M, opus-high S review), LPE and path visualizer (opus-low), blackbody leftovers (sonnet M), fluorescence/black light (opus-low L).

### Wave 4: spikes first, then decide
- **Gaussian splats** (about 2 weeks for the stochastic version): AABB BLAS reusing the SDF path, .ply loader (sonnet), shader and CPU twin (opus-low).
- **Polarization**, minimal: CPU-first Stokes for smooth lobes, a polarizing filter, a GPU smooth-lobe variant. After spectral part 2.
- **Neural radiance cache:** cooperative-vector feasibility spike (opus-low M) before any build.
- **NRD:** worth it only for good denoising on non-NVIDIA GPUs; the diffuse/specular split is the big cost.
- **Realtime preset** (owner question 2026-11-10 on a game-style raster mode; raster rejected by measurement, see the roadmap): 1 spp, ReSTIR DI, RR at render scale 0.5-0.67 (the error barely moves with scale). Needs first:
  - a separate, higher bounce limit for specular and transmission chains (today one `bounces` limit covers both, `pathtrace.hlsl` ~1470): at 3 bounces glass-of-water renders black and nighthawks' interior through glass goes dark. Diffuse 2-3, specular about 8 (opus-low M, both renderers, oracle);
  - the 144 fps cap in headless benchmarks: `--no-vsync` does pick IMMEDIATE, but headless runs still present to a hidden window and the Wayland compositor throttles those presents to the monitor rate (6.94 ms at 144 Hz; XWayland 17 ms). Headless runs should skip acquire/present and render offscreen, since they capture through readback anyway (sonnet S). Until then compare `gpu_ms`, not `frame_ms`;
  - then the preset itself (sonnet S) and an RR ghosting check with a moving camera.

### Windows session (whenever the owner is on Windows)
D3D12 build of everything above and the self-update checks (UAC relaunch, `SHA256SUMS`, installer).

## Dropped by the owner (2026-11-10)
- OptiX denoiser, the D3D12 display-only device for GPUs without ray tracing, DLSS Frame Generation, and Reflex (owner: no point without frame generation).
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
