# Caustica roadmap

The current plan, rewritten 2026-11-04. The long history (what was built, measured and decided, item by item) lives in `docs/engine-notes.md` (`grep -n '^## '`) and in git; this file keeps only what is done at a glance, what comes next, and a triaged list of open tails.
Order: **the "Plan" section below (2026-11-12).** It replaces the 2026-11-04 order (correctness, code health, tails, Vulkan); the sections under "Now" keep the detail of each open item. Tails marked [drop] are closed and will not be worked unless the owner asks.
Research documents: [ui-plan.md](ui-plan.md) (UI), [scene-research.md](scene-research.md) (scenes).

## Ground rules from the owner

- **Public since 2026-11-06, under MIT for the code.** Models, textures, HDRIs and SDKs keep their own terms; `THIRD-PARTY-NOTICES.md` lists them, and anything new that ships goes in it.
- **Licence risks the owner has accepted (2026-11-06), and will act on if raised:** Sponza stays bundled, though the CRYENGINE agreement covers games built with CRYENGINE; the GPLv3 FFmpeg stays bundled without published sources; releases up to v0.10.1 stay up without notices; the About box shows the NVIDIA logo, plus on Windows the AMD FSR and Intel logos, though the DLSS licence only grants word marks and Intel's bars promotional use of its name. Research-only and NC assets such as the Stanford scans are fine only while the app stays free. If it is ever sold: those assets go, and NVIDIA must be notified first (DLSS licence, exhibit section 4).
- **Performance work never lowers quality** (owner rule, 2026-11-02). The defaults stay as they are (spp, bounces, sample cap, DLAA render scale, RR preset), and no "performance mode" or lower-quality option is added. Speed has to come from doing the same work more efficiently. A change that alters the image has to be justified as unbiased, or as equal under the convergence measurement. Exception (owner, 2026-11-10): the Realtime Mode (1 spp, separate specular bounces, ReSTIR, RR at an upscaling scale), kept apart from the quality bundles and restoring the scene's own sampling when left.

- **Who it is for (owner, 2026-11-12):** the owner alone, for fun. No work for an outside audience (README for strangers, shareable benchmark scores, slim updates, signing). Only the owner's NVIDIA RTX card matters: no AMD or Intel testing, and FSR and XeSS are not reasons to keep anything. BDPT and photon mapping have scenes of their own: keep and improve them. The release rhythm and size don't matter.
- **Ask before removing anything** (owner, 2026-11-12): features, files and docs alike. Approved so far: the four toys in Plan step 3.

## Plan (2026-11-12)

From the project review of 2026-11-12 and the owner's answers. Each step ends with verify, review and a snapshot; commit when the owner asks.

1. **UI skin pass (owner, 2026-11-12: egui "feels cheap no matter what, like crappy html buttons").** Stay on egui and replace the stock look with the app's own painted widgets; the layout B structure stays.
   - **Mockups first** (owner): boards for the skin on the design canvas (button, toggle, checkbox, slider, combo, text field, section header, tab strip, menu and popup, in rest, hover, pressed, disabled and focused states; then the Render tab and the top bar drawn with them). The owner picks before any code. **Owner picked B "Instrument", 2026-11-12, "with no cutbacks"** (every gradient, highlight, inset shadow, band and state as drawn; handoff in `docs/skin-handoff.md`, board sources in `docs/mockups/`). Drawn 2026-11-12: https://claude.ai/artifact/W3Fp5p8GoqgjoaSsV3xDxR (private to the owner), board A "Quiet" (flat, borderless, hover-revealed buttons, Inter) and board B "Instrument" (raised controls, recessed fields, section bands, IBM Plex); each has the app at 1280×720 with the Render tab, and a sheet of every control in five states.
   - **Widgets:** one module of painted controls with those states, used in place of `ui.button`, `Slider`, `ComboBox`, `Checkbox` and `TextEdit`; styled as board B.
   - **Motion:** 80-150 ms eases on hover, press, section open and tab change (`animate_bool`); nothing that keeps the UI repainting at rest.
   - **Type and rhythm:** a UI font that holds up at small sizes without hinting, two weights, three sizes; one row height and one spacing grid, labels and values on shared columns.
   - **Depth:** shadows and layering for menus, popups and the floating strips.
   - **Order:** restyle the Render tab first and compare it with the current one in the live app; if that fixes the feel, do the rest tab by tab. If it does not, reopen the toolkit question (a webview UI was weighed and set aside: compositing over the swap chain and re-plumbing every control is weeks). Known limit: text at 100 % scale stays a little softer than native text.
   - The self-test's UI frames and `--capture-ui` captures must keep passing; captures get re-baselined once at the end.
   - **Built 2026-11-12/13, the whole pass** (engine-notes "UI skin: Instrument"): `src/ui/skin.rs` (the rasteriser, the atlas, every widget), IBM Plex as the app's font, and the skin on everything: the six tabs, top bar, tool rail, tab strip, outliner, status bar, timeline, the Scenes and Assets browsers, menus, tooltips, toasts, the Ctrl+K palette and the windows. The old look and its code paths are gone. `--ui-skin-sheet` shows every control in every state. Snapshot `builds/2026-11-13-ui-skin-complete`. Not committed: the owner looks first.
   - **Owner's answers, 2026-11-12:** every on/off row is a checkbox; the "modified" dot sits in the gutter before the rail; a slider's unit is outside the number field (end of the label column).
   - **Open, for the owner's eye (a headless capture cannot show them):** menus and submenus open, hover and pressed looks in the live app, the Stats, Graph, Console and Remote windows (no capture switch), the colour picker in use, text weight on the HDR screen.
   - **Open, known gaps:** tooltip text is 12.5 points, the board's is 11.5 (egui 0.37 has no hook short of touching every tooltip call); the Scenes and Assets cards and the colour wheels follow the board's language but the board draws neither, so their layout is a guess; the status bar's right-hand text is clipped, not ellipsised, in a narrow window; spinners keep an idle viewport drawing every frame while their job runs (since idle repaints are honoured); the theme is not pinned to dark, so a light-mode Windows would get egui's light style; captures (`captures/validation-ui.png` and friends) not yet re-baselined; Linux not looked at.
   - **Open, technical:** the skin's translucent pixels are corrected for scRGB over the panel colour only (a popup's shadow over the image is approximate), and HDR10 is uncorrected; pressed content moves one pixel at 100 %, not half a point.
2. **Land UI layout B.** Built and committed 2026-11-12 (`cc9bf00`), drawn on Vulkan on Windows. Left: draw it on Linux, the owner's pass in the live app, and the tails under "UI and editor".
3. **Cut four toys (owner approved):** the achievements (the lifetime statistics stay, see below), the phone remote (`src/remote.rs`, the `qrcode` crate), audio-reactive lights (`src/audio.rs`, `cpal`, `rustfft`, the Lights tab's Audio section) and the laser puzzle (`src/laser.rs`, `Settings::puzzle`, the Puzzle window, the `scenes/puzzles` pack of 5 scenes). Also their palette entries, self-test frames, README and engine-notes text, and a Won't do line. Old scene files that carry `puzzle` or `audio` must still load. After steps 1 and 2, because it edits the same UI files. To confirm with the owner when it starts: that the 5 puzzle scenes go too. The optics bench and the beam-splitter material stay.
   - **Statistics stay and grow (owner, 2026-11-12):** `src/stats.rs` loses the achievements (groups, tiers, progress bars, toasts) and keeps the lifetime totals in a plain Statistics window. Redundant fields go (owner: "remove redundant statistics"): the ones that only fed achievements or the cut toys (`marks`, `achievements`, `puzzles_solved`, `puzzles_par`, `audio_seconds`), and, proposed for the owner to confirm, `dark_seconds` (flashlight time) and `hot_reloads` (a developer counter). Kept: `paths`, `gpu_seconds`, `images_saved`, `videos_exported`, `max_samples`, `presets_seen`, `scenes_seen`, `walked`, `physics_seconds`. To add, the owner picks: per-session figures beside the lifetime ones, time per integrator and per denoiser, rays per second now and best, samples and time per scene, most-used scenes, exports by format, and a reset. An old `stats.json` with achievement fields must still load.
4. **Guard rails** (2-3 days):
   - a test that reads `cbuffer Frame` from `shaders/common.hlsli` and compares names and offsets with `Constants`;
   - one generated shader entry list used by both build.rs and `hotreload::ENTRIES`;
   - a Settings-field test (every field is bounded by `validate()` and classed in `transport_key`);
   - a three-scene, low-spp oracle run in `tools/verify.ps1`.
5. **D3D12 phase-out, proof first** (below). D3D12 stays the default until both gates pass.
6. **Oracle green on the remaining backend**, path tracer first: texture mip bias, emissive maps, Sponza alpha, the roughness-0 metal gap, the ReSTIR excess (Now a and e).
7. **BDPT and photon mapping, improved:** portals, stars, the specular bounce budget, the −ng walk rule (B7 leftovers), then the open discrepancies (sunset-tank, optics-bench, the sdf-gallery cone, spectral photon +8% on street-bokeh, the white-furnace gather bias). This reopens the two "[drop] BDPT / photon" lines under "Rendering and integrators" where they overlap.
8. **Code health:** split `App::redraw`, `App::new` and `window_event` (`src/main.rs`) as pure moves; split `docs/engine-notes.md` into one file per section (asked first: see "Docs" below).
9. **Fun features, the owner picks one at a time:** stop at a noise target with a render queue; volumetric clouds; the hash-grid radiance cache for the Realtime preset; explain this pixel on the CPU; planets and the observatory scene. The parked list in `docs/plan-big-features.md` stays parked.

### D3D12 phase-out

Why: every GPU feature is written twice (`src/gpu/` 5.7k lines, `src/vulkan/` 6.2k) and development happens on Linux, where D3D12 can't run. Vulkan already runs the whole renderer on Windows with DLSS RR, DLSS SR and XeSS.

**Phase 0, make the comparison possible (2-3 days).** A `--backend d3d12|vulkan` pass-through in `perf_history.py`, `perf_baseline.ps1`, `oracle.py`, `upscale_quality.py`, `rr_stability.py`, `motion_error.py` and `verify.ps1` (none has one today); `tools/backend_compare.py`, which runs both backends from one exe over a fixed scene list and writes one table of image difference and timings; the Vulkan validation layer installed on the Windows machine. Scene list: perf-history presets 0, 13, 17 and 21, Sponza, stylised-forest, glass-of-water, fairy-lights, one BDPT, one photon, one spectral and one HDRI scene.

**Phase 1, feature gate (1-2 weeks).** Every row passes on Windows before the default changes:

| Area | Gap today | Proof |
|---|---|---|
| Self-test | passes on Vulkan with no validation layer installed | 484 frames clean with the layer on |
| Image parity | compared at port phases 3-6 only | raw EXR per scene within noise of D3D12; the oracle's default list passes on Vulkan |
| HDR on Windows | the surface offered no HDR format, checks skipped | scRGB or HDR10 output, and the pixel-by-pixel HDR self-check passes. **Check first: it may be a driver limit, and it can block the plan.** |
| Opacity micromaps | none on Vulkan; alpha uses the any-hit loop | `VK_EXT_opacity_micromap` ported, or the loop shown to cost nothing measurable on the alpha scenes |
| RR and upscalers | ported; pre-exposure unproven on Vulkan | `rr_stability.py` and `upscale_quality.py` within D3D12's numbers; the night-scene pre-exposure check |
| UI | layout B never drawn on Vulkan | hand pass; `--capture-ui` captures match D3D12 |
| Exports | not compared | every image format, the high-res still and video match |
| Windowing | not compared | resize, minimise, monitor change, HDR toggle, Alt-Tab, two DPI scales |
| Startup | 0.5 s warm with the pipeline cache | cold and warm start beside D3D12's |

FSR has no Vulkan path and is given up (owner: AMD doesn't matter).

**Phase 2, performance gate (3-5 days).** Same exe and scenes, 1440×900 and 2560×1440, at least 200 frames, `--no-vsync`, three runs, medians. Measured: `gpu_ms_mean`, `trace_ms_mean`, `reconstruct_ms_mean`, `frame_ms_mean`, the rebuild under `--drag-test`, startup and peak VRAM. The D3D12 side runs as shipped, SER tracers and micromaps on. Pass: no scene more than 3% slower in `gpu_ms_mean` (owner to confirm the number). If the SER tracers' 7-14% shows as a loss, try raygen tracers on Vulkan (a `VK_KHR_ray_tracing_pipeline` raygen shader that still traces with ray queries, as the D3D12 `*_ser` libraries do; the D3D12 gain came from the raygen form, reordering itself measured slower) and measure again. Unproven on Vulkan: size it before starting. No quality setting changes to pass the gate.

**Phase 3, Vulkan as the default on Windows.** `Prefs::vulkan` defaults to true; `--d3d12` and the Renderer menu stay. Baselines (self-test, perf history, the Sobol rule's reference) move to Vulkan. One release in this state, used by the owner for a while.

**Phase 4, removal, only after the owner says so.** `src/gpu/`, the D3D12 half of `renderer.rs`, the DXIL half of build.rs and hot reload, the SER libraries, `bridge/ngx.cpp`, `fsr.cpp` and `xess.cpp` (D3D12 parts), the Agility SDK and its notices line, `tools/check_dxil.py`, `check_windows.sh`, `docs/claude/d3d12-on-linux.md` and `windows-handoff.md`. `display.rs` (DXGI monitor query, JPEG XR) stays: Vulkan on Windows uses it. Tag before the removal so the last D3D12 build stays reachable.

### Docs, waiting for the owner's yes

Fixed on 2026-11-12 without removing anything: stale counts and contradictions in the README, the project line in `CLAUDE.md`, a note in `windows-handoff.md`. Proposed, not done:
- Split `docs/engine-notes.md` (486 KB) into `docs/notes/<section>.md` with an index.
- Move finished plans to `docs/archive/`: `ui-plan.md`, `ui-review.md`, `egui-upgrade-plan.md`, `scene-research.md`, and `plan-big-features.md` once its backlog is folded in here.
- Cut the README to what the app is, how to build and run it and its limits, moving the feature tour to `docs/features.md`.
- Trim this file's finished "[done]" lines under "Now" and "Open tails" (their detail is in engine-notes).

## Done

Details are in engine-notes under the section named in quotes.

- DLSS preset selection, 2026-10-30.
- Scene overhaul: 14 new built-in scenes, 4 HDRI and 1 material scene for unused features, 16 reworked, 6 redundant and 56 generated seeds removed, laser puzzle housings, scene-relative LUT paths, 2026-11-05 (all 123 renders checked by hand; lighthouse, dark-side, optics-bench and greenhouse retuned in v0.10.1): "Scene overhaul".
- Native HDR output (scRGB; HDR10 swapchain and HDR JPEG XR added 2026-11-02), 2026-10-30: "Native HDR".
- UI phases 1-5: regrouping, undo / redo, scene browser with thumbnails, Quality bundles, GPU timing, prefs, async high-res still, capabilities struct, modified-row dots, file pickers, 2026-10-30 to 2026-11-02.
- Blender-style gizmos, G / R / S, snapping, pivots, multi-select, Alt-drag duplicate, 2026-10-30 / 2026-11-02: "Gizmos", "Multi-select, pivots and snapping".
- Scenes as files (now 90+ built-in, generated and mesh packs), Scenes window, thumbnails, 2026-10-30.
- Core track 1-6: generators (11 families), mesh import (OBJ, glTF, PLY, STL), HDRI environments, instancing to 4096 objects, textures, UI phases, 2026-10-30: "Instancing".
- Developer tooling (shader hot reload, perf history), debug views, scopes, light meter, convergence graph, stats, 2026-10-30.
- Camera and look (thin lens, camera models, AgX, halation, grain, LUT, film stocks, GPano), 2026-10-30 / 2026-11-02.
- Toys (light painting, portals, walk mode, screensaver, gamepad, phone remote), 2026-10-30.
- Owner requests: slider reset, light outlines off, HDRIs (19 maps), video export dialog, Poly Haven / ambientCG / Khronos browser, real walking, image format choice, native-path anti-aliasing (pixel filters; "Native AA"), anisotropic texture filtering, 2026-10-30 / 2026-10-31.
- Analytic lights (rectangle, disk, sphere, spot, sun), 2026-10-30: "Analytic lights".
- Animation and timeline (motion blur, curve editor, camera splines, auto-key), 2026-10-31 / 2026-11-04: "Animation and timeline".
- Physical sky and day cycle (stars, moon, clock time, altitude), 2026-10-31 / 2026-11-02: "Physical sky".
- Nested dielectrics, 2026-10-31: "Nested dielectrics".
- Layered and exotic materials (clear coat, sheen, energy compensation, anisotropy, thin film, diffraction grating), 2026-10-31 / 2026-11-01: "Layered materials", "Energy compensation and anisotropy", "Thin film", "Diffraction grating".
- Volumes (homogeneous, heterogeneous noise fog, equiangular, camera medium), 2026-11-01 / 2026-11-04: "Volumes".
- Procedural primitives and SDFs (metaballs, exact cylinder / cone), 2026-11-01: "Procedural surfaces".
- Path visualizer and light path expressions (object and light labels), 2026-11-01 / 2026-11-04: "Path visualizer", "Light path expressions".
- Integrators: ReSTIR DI, BDPT, photon mapping, battle mode, volumes in both, 2026-11-01 / 2026-11-02: "ReSTIR DI", "BDPT and photon mapping", "Media in BDPT and photon mapping".
- Real lenses and diffraction glare, 2026-11-02: "Real lenses and glare".
- Physics sandbox (rapier, V-HACD, joints, cloth with two-way coupling), optics bench, laser puzzle, 2026-11-02 / 2026-11-04: "Physics sandbox", "Optics bench", "Laser puzzle".
- GitHub CI, tag-only releases with installer, rename to Caustica, installed-mode data folders, 2026-11-02 / 2026-11-03: "Data folders".
- Content scenes (snow globe, hourglass, birthday cake, lava lamp, Nighthawks, Vermeer room, Weather Project, jack-o'-lantern), 2026-11-02 / 2026-11-03.
- Integrations: text-to-scene (Claude, ChatGPT, local, opencode backends), rhai scripting and in-app console, audio-reactive lights, 2026-11-02 / 2026-11-04: "Text-to-scene", "Scripting", "Audio-reactive lights".
- GPU efficiency 20a (LEAN variants, instance masks, opacity micromaps, CDF guide tables, Nsight profiles), 2026-11-03: "GPU efficiency".
- Shader Execution Reordering (raygen tracers 7-14% faster; reordering itself measured slower, compiled out), 2026-11-03: "Shader Execution Reordering".
- Closing the medium term, waves 1-3 (importers, IES, spectral leftovers, emissive maps, shadow catcher, TLAS refit, BLAS compaction, multi-layer EXR, material thumbnails, silhouette outlines), 2026-11-03 / 2026-11-04.
- Spectral rendering part 1 (path tracer, photons, dispersion, thin film, gratings, lens), 2026-11-03: "Spectral rendering".
- CPU reference renderer (first version, UI entry), 2026-11-04: "CPU reference renderer".
- Startup time first part (warm 5.9 to 2.2 s), 2026-11-04: "Startup time".
- Flashlight beside the camera (0.3 right, 0.4 down, beam crossing the view axis 8 units ahead) with a reflector beam profile through the IES path, an IES default (`scenes/ies/flashlight.ies`) and its settings in its own outliner row, 2026-11-06: "Flashlight".
- Achievements reworked (`src/stats.rs`): 37 in six groups, tiers sized for a fast card (about 200 million paths a second, so "billions" is no longer seconds), progress bars, 2026-11-06.
- UI usability review, 2026-11-06 (`docs/ui-review.md`): 42 findings, fixed in the working tree and since, including the SDR UI gamma bug (the whole UI drawn too dark), undo of panel edits, unsaved-work warning and a restorable session; the rest are listed open in the file.
- CPU mode (`--cpu`, and the GPU / CPU switch in the menu, top bar and Render tab), playable 2026-11-09: the CPU render shown full view through the display pass, previews while moving, samples and time in the status bar, saves, a list of what the scene still loses: "CPU reference renderer". Parity is in progress (Now e).

## Now

Order and staffing of the big features: `docs/plan-big-features.md` (2026-11-10).

Ordered. Each step ends with verify, review, snapshot, then a commit when the owner asks.

### a. Correctness against the CPU reference — first pass done 2026-11-04

`tools/oracle.py` renders scenes on the GPU (raw) and with the CPU reference and tests agreement statistically (engine-notes "Oracle and the pixel-filter finding"). All 8 default scenes pass for path tracing (within 0.6%, |z| < 3.5). Findings:
- **The BDPT / photon "biases" were the pixel filter.** BDPT and photon mapping always use a box filter; the path tracer's default Blackman-Harris doesn't sum to 1 over the pixel grid, so image means at sharp bright lights moved by ±0.6% (Cornell, sign flipping with light size) up to 10% (a sub-pixel sun). With a 1-px box on both sides: Cornell BDPT 0.9999, photons 0.9993; measured-cornell 0.9996 / 0.9988 (closes "+0.9%"); sky-garden-dusk BDPT/PT 0.9956 and BSDF-only/MIS 1.002 (closes the dusk gap and "PT disagrees with itself"). The oracle and `integrator_sweep.py` now compare with matched box filters.
- **BSDF-only vs light sampling at the bounce limit:** BSDF-only renders 0.4% darker at 6 bounces (light sampling at the last vertex adds one path length BSDF sampling doesn't); equal at 32. A convention, not a bug.
- **shape-material-matrix's 0.93 blocks:** gone under the oracle's test (worst block 0.978, z −4.2). Explained 2026-11-06: the blocks are grounded glass, a coplanar contact (engine-notes "CPU reference renderer").
- [keep] **GPU ReSTIR small excess, untraced:** fairy-lights 1.0010 (z 5.7) at 2048 spp, 1.0005 (z 6.4) at 8192, and 1.0002 with it off; preset 0's worst block unsteady, down to 0.993 (z -10.7). Shrinks with samples, under the oracle's floor (engine-notes "CPU reference renderer").
- [done 2026-11-10] **Accuracy audit B7, NEE-only darker than MIS:** the 0.57% was the shadow stop fixed 2026-11-02; night-skyline's remainder is noise (0.9988, z −1.7 at 1M spp); light-temperature-row had a shading-normal visibility mismatch (BSDF only 0.07% dark, z −11) between the shadow ray (+ng) and the BSDF continuation (−ng when under the face), fixed in both renderers: all three strategies agree, the default render there +0.10% (engine-notes "Light sampling and BSDF sampling agree (accuracy audit B7)").
- [done 2026-11-10] **Accuracy A1, exact rough dielectric on the CPU:** truth `glass_btdf` is the GGX BTDF with NEE; GPU lerp bias measured (ice-cubes 0.977, blocks up to 48% behind rough glass). Open: multiple-scattering compensation, BDPT/photon versions, making it the CPU default, GPU port behind a variant.
- [done 2026-11-10] **Accuracy A1 on the GPU:** `Settings::exact_glass` (default off, Render tab "Exact rough glass") picks EXACT_BTDF variants (`trace_exact`, `trace_spectral_exact`, SER twins at `Gpu::ser` 7/8, `bdpt_exact`, `bdpt_spectral_exact`); default SPIR-V byte-identical, self-test ok. Oracle GPU exact vs CPU truth: ice-cubes 0.9988, frosted cubes (rough 0.5) 0.9997, BDPT 1.0004/0.9992, spectral 0.9992/0.9997. Cost +25% trace time (14.4 to 18.3 ms, 1280x720). The Accurate bundle sets `exact_glass` (done 2026-11-10); BDPT keeps the rough glass vertex delta (no connections through the glass lobe, vertex_f unchanged); photon mapping and LPE keep the lerp; no LEAN twin; D3D12 side not compiled here (Linux); battle mode untouched; no frosted builtin scene (test scene was a roughness-0.5 copy of ice-cubes).
- [done 2026-11-10] **Accuracy: the Accurate quality bundle:** Quality menu entry (denoiser off, scale 1, 64 bounces, exact glass, `gentle_roulette`, `lod0_textures`, glass shadows on, firefly 0, ReSTIR off); 64 bounces cost nothing measurable at 1440 x 900 (roulette ends the paths), LOD-0 textures erased the mip residual on `texture-showroom` / `texture-rock-garden` (1.0062 / 1.0027 to 0.9997 / 0.9995), GPU Accurate agrees with the CPU on eight scenes; convergence against default in engine-notes "Accurate quality bundle". Open: media / SDF cap variants (D1, D2, C3), Tier 2 rows, Accurate stays Custom after a scene load.
- [done 2026-11-10] **Accuracy plan step 2, CPU truth switch:** `scene::Truth` / `--truth` / Render tab / `oracle.py --cpu-truth`, seven rows with exact code and six hooks (A1-A4, C1, C2); measurement matrix in `docs/claude/accuracy-audit.md` (SDF caps +0.55% on `sdf-gallery`, fog caps and floors no effect, roulette unbiased, 6 bounces 0.5-46% dark by scene).
- [open] **Truth switch leftovers:** exact versions of the six hook rows (next accuracy step); `lighthouse-fog` fails the block test with truth off at 4096/2048 (block (2, 3) 1.0581, z 7.3; suspect the rotating spot's time); `bounces` is capped at 32 by `validate` (the audit asks 64); roulette cost per sample not timed.
- [open] **B7 leftovers:** BDPT and photon-mapping walks still leave from −ng under the face (`integrators.hlsl`, `bdpt.rs`, `photon.rs`); measure them on light-temperature-row against the path tracer and apply the same rule if they differ. The shared rule (the face's visibility) is a convention: pbrt's f = 0 across the face or a terminator offset (audit C2) are the alternatives.
- [done 2026-11-10] **Coplanar dielectric contacts:** a grounded transmissive object is lifted `geometry::DIELECTRIC_GAP` (4 mm) in geometry, which both renderers, physics (collider contact skin), picking and the gizmo read; object-object contacts have no safe general rule, so the built-in scenes separate them in tools/build_scenes.py. All seven scenes and sdf-gallery's cone now pass the oracle at 2048 spp (worst mean 1.0005; table in engine-notes "Grounded dielectric gap"). Open: user scenes with hand-placed dielectrics touching exactly still tie; the gap was not swept below 4 mm.

Still open:
- [keep] Integrator sweep rerun in full with matched filters (2026-11-04, 400 spp, 240 × 135, `captures/sweep-2026-11-04`): 15 of 174 comparisons beyond 5%. To trace, largest first: sunset-tank BDPT 112% / photon 57% (colour shifts, not noise-like), optics-bench 72% / 71% (slow PT caustic), birthday-cake photon 23%, underwater BDPT 17%, lava-lamp photon 11%, shape-material-matrix photon 11%, sunlit-room-day-cycle photon 10.5%, snow-globe photon 10%, bigger-on-the-inside 8% (portals), gem-display photon 7%. dark-side (553% / 214%) is near-black (means ~1e-4), not meaningful at this spp. Next: rerun the flagged scenes at 4096 spp before reading anything into them.
- [keep] optics-bench (beam splitters) and lava-lamp (SDF emitters) could not be judged by the CPU reference; both now render on it (2026-11-10, Now e), so they can be re-measured against the path tracer. bigger-on-the-inside is a known gap in BDPT / photon mapping (no portals).
- [keep] Photon mapping on the white furnace: 0.9945 (z −22) with box filters, a gather-radius bias (expected of progressive photon mapping at 1024 spp; check that it falls with more passes).
- [keep] GPU photon mapping is dark inside sdf-gallery's green glass cone (2026-11-10, 1024 spp: 0.075 against 0.096-0.099 from the CPU's photon mapping and both path tracers; oracle block (5, 5) 0.976, z -7.8). Not traced: the walk is BDPT's, which passes there, so look at the photon side (deposits or the gather) on a procedural dielectric. The CPU reference renders photon mapping now (engine-notes "CPU reference renderer"), so this and the white-furnace item can be judged by `tools/oracle.py --integrator 2 --cpu-integrator 2` at equal spp.
- [done 2026-11-09] Extend the reference to HDRIs, sphere and sun lights and the physical sky: see Now e.
- [done 2026-11-05, measured] give BDPT and photon mapping the path tracer's pixel filter, so all integrators render the same image by default. BDPT and photon camera rays take `filter_offset`; a light-tracing splat goes to one pixel of the filter's support picked with probability w / W, weighted by W (`splat_filtered`: unbiased, one atomic splat per sample; splatting the whole support hung the GPU on self-test frame 295). Box 1 px renders as before. At 1024 spp with the scenes' filters, BDPT / PT 0.998–1.001 and photon / PT 0.995–1.001 (cornell-caustic, veach-ajar, measured-cornell).

### b. Code health — done 2026-11-04

Pure code moves, the self-test passing unchanged: the self-test script and checks to `src/selftest.rs`; `impl Gpu` into `src/gpu/` (init, device, scene, passes, frame, capture, util; `gpu.rs` 6.1k → 0.9k lines); `ui.rs` into `src/ui/` (windows, overlay, chrome, ai, scene / render / look tabs, stage; 5.7k → 1.5k lines). Map in docs/claude/architecture.md. Open: `gpu/` still mixes D3D12 calls with renderer logic; the Vulkan backend needs a device-abstraction seam below it (see d).

### c. Tails triage

Work the [keep] items in "Open tails" below, grouped by area, smallest first. Anything left [drop] stays closed.

### Owner requests, to order with the owner (2026-11-06)

- [done 2026-11-10] Remove light painting and the screensaver mode from the app (owner: "so pointless"): `src/saver.rs`, `tools/make_screensaver.ps1`, the L key, *Paint light* button, `--` switches and self-test frames 240-242 (free now) are gone; the engine's keep-the-sums path stays for the motion-blur export as `Gpu::keep_exposure` (engine-notes "Keep-exposure mode").

- [done 2026-11-06] Rework and expand the achievements: some fell far too fast ("billions" took a couple of seconds). See Done.
- [done 2026-11-06] Offset the flashlight from the camera: it shot straight from the eye, so its shadows were barely visible. See Done.
- [done 2026-11-10, Vulkan measured, D3D12 and hand use not tried] Proper first-person view in walk mode (2026-11-07): see your own legs when looking down and your reflection in mirrors and glass. An avatar of a few blocks is enough, in the spirit of a blocky game character but our own design, posed from the walker's stride, visible to reflections and to the camera below the neck. Done as `avatar.rs` (engine-notes "Walk avatar"); open: measure `update_materials` on D3D12 and skip identical tables there too, a colour or skin choice, a body that lags the view and follows the movement direction (strafing slides the same stride now), foot-planting that matches the stride length (the legs are shorter than the walker's step), a collider, hands holding the flashlight.
- The UI review's findings are all fixed (`docs/ui-review.md`); what is left there are the per-batch notes of defaults taken, plus R's meaning (restart against modal rotate) for the owner to decide.

- [open] **NGX updater pile-up on Windows (2026-11-11):** DLSS init leaves the driver's `nvngx_update.exe` running, thousands after a day of headless runs; see `docs/claude/windows-handoff.md` "NGX updater pile-up".

### Self-update, open items

Built 2026-11-06 (engine-notes "Self-update"). Open:
- On real Windows, not yet run: the D3D12 renderer with these changes (the releases were built under Wine); an all-users update's relaunch, with its UAC prompt; the *Updated to* toast after a setup; `tools/release.ps1`'s `SHA256SUMS` and `make_installer.ps1`'s licence copy; Vulkan with DLSS after the extension change.
- Later, if wanted: resuming an interrupted download; slim updates without the 700 MB of assets; signing, since `SHA256SUMS` doesn't protect against a compromised account.

### d. Vulkan backend

Portability: Linux, and AMD / Intel drivers where Vulkan RT is stronger. The upscaler SDKs already ship Vulkan paths: NGX (`nvsdk_ngx_vk.h`, `nvsdk_ngx_helpers_vk.h`, `nvsdk_ngx_helpers_dlssd_vk.h`, all in `vendor/dlss/include`), XeSS (`xess_vk.h`) and FSR (the FidelityFX API has a VK backend).

- **Approach:**
  - Split `gpu.rs` (step b) into a backend-neutral `Renderer` and a backend trait: resources / textures, buffers, BLAS / TLAS build, compute dispatch, barriers, timestamp queries, swapchain present, readback and the upscaler hooks. Keep the D3D12 backend as the reference implementation.
  - Vulkan via the `ash` crate.
  - Ray tracing: `VK_KHR_acceleration_structure` + `VK_KHR_ray_query`, mirroring the inline `RayQuery` design; no pipelines or SBT. `VK_NV_ray_tracing_invocation_reorder` (raygen-only, like D3D12 SER) is the later route to reordering, which measured slower on D3D12.
  - Shaders: the same HLSL to SPIR-V with `dxc -spirv` (`[[vk::binding]]` annotations or `-fvk-*-shift` flags for the register layout). One source of truth for the `Constants` layout.
- **Shaders, checked 2026-11-04:** `tools/spirv_check.py` compiles all 24 compute / graphics entries of build.rs to SPIR-V (Vulkan 1.3) unchanged with the DXC release build (`vendor/dxc-spirv`, fetched by `tools/setup.ps1` `Get-DxcSpirv`; the Windows SDK's dxc has no SPIR-V codegen); the 7 SER libraries are left for a raygen port. Still to settle with the real descriptor layout: register spaces become descriptor sets one-to-one, and t / u / b registers of the same number in one space share a binding unless shifted (`-fvk-t-shift` etc.); the bindless texture array needs `VK_EXT_descriptor_indexing` (core in 1.2).
- **Progress (phases in engine-notes "Vulkan backend: design"):** phases 1–6 are done on Windows (2026-11-05): the whole renderer runs on `--vulkan` (denoiser, every integrator, ReSTIR, glare, lens, LPE, maps, environment, LUT, HDR, readbacks, scopes, timestamps, DLSS RR / SR and XeSS), matching D3D12 within noise; BLAS reuse, compaction and TLAS refit as on D3D12; the "Compiling shaders N/22" screen on both backends. Dropped by the owner (2026-11-10, see Won't do): FSR on Vulkan and XeSS on Linux. Status: DLSS RR / SR on Linux runs on NVIDIA (first run 2026-11-06, RTX 4070 Ti SUPER, driver 615.78; engine-notes "Linux on a real desktop"); shader hot reload on Vulkan (done 2026-11-05, verified by hand; it now also reloads while paused or converged, and compiles the entries in parallel, 84 s → 24 s: `hotreload` compiles SPIR-V with the SPIR-V DXC and build.rs flags, `reload_shaders` rebuilds the compute pipelines; the UI pipelines stay); the self-test under the validation layer (`--vulkan --self-test` passes on the RTX, which has no layer installed; with Windows HDR on, the RTX's Vulkan surface offers no HDR format, so its HDR checks are skipped with a note: HDR output on Vulkan is untested until a surface offers scRGB or HDR10; the VM covers headless runs only); pumping window messages during a cold shader compile (done 2026-11-05, untested: `progress::watch` peeks the queue with PM_NOREMOVE every 16 ms, which keeps Windows from marking the window as not responding).
- **Owner reports, open (2026-11-05):**
  - [done 2026-11-05, verified] Anisotropic texture filtering had no effect on Vulkan: the map sampler (`vulkan/layout.rs`) was plain linear. It now has 16x anisotropy, as D3D12's, with `samplerAnisotropy` enabled where the device offers it. Checked: a low view of texture-loft-corner is 31% sharper at 16x than 1x on Vulkan (D3D12 36%), and the backends' 16x images differ by 0.3%.
  - [done 2026-11-05, verified by hand] Pausing froze the UI on Vulkan: `present` skipped the swap chain on frames that trace nothing (paused, and also converged). It now presents every frame unless the window is minimized, as D3D12 does.
- **Risks and size:** the largest item on the list; budget weeks. Barriers and layouts, descriptor management and the NGX Vulkan device-extension requirements are the hard parts.
- **Linux port:** done 2026-11-06 on an RTX under KDE Wayland (engine-notes "Linux on a real desktop"): exit crashes, `--self-test`, DLSS RR / SR, HDR output and `display::Monitor` (through `wp_color_manager_v1`, not KWin's config file), shader hot reload. Open: highlights above the reference white, unmeasurable by screenshot; the `linux` CI job on GitHub runs once CI is back on.

### e. CPU renderer parity - complete (2026-11-10)
`reference::unsupported()` is empty: every scene and setting the GPU renders also renders on the CPU. Light painting is gone. What is left are GPU-side oracle failures, found by the CPU and not yet fixed:
- Texture mip-prefilter bias: texture-showroom, texture-rock-garden (worst block 1.094) and texture-brick-box.
- Emissive-map scenes: the checker scene and Lantern.
- Sponza with alpha, direct light only: 1.0161.
- Integrator 2 (photon mapping) against the CPU path tracer on mirror-room, bigger-on-the-inside and infinite-corridor; integrator 1 (BDPT) on diamond-fire and lava-lamp.
- Battle preset 3 (photon mapping with the default pixel filter): 1.0067, z 23.8.
- ReSTIR at one bounce on nighthawks (reuse correlation).
- A steady 0.36% spectral gap on blackbody-forge.
- Fireflies in the dark-side fog.

The old "Software fallback and WARP" goal is replaced by this: the CPU view is the fallback when there is no ray-tracing GPU. Design and log: `docs/claude/windows-handoff.md`, `docs/claude/cpu-parity-audit.json`, engine-notes "CPU reference renderer".

## Open tails

Verdicts: [keep] = correctness bug, owner request or report, blocks Vulkan or a release, or small with clear user value. [drop] = polish nobody asked for, research-grade, against an owner rule, or blocked on hardware / certificates we lack. Dropped items are listed again under "Won't do".

### Rendering and integrators
- [keep] Specular bounce budget (B1, 2026-11-10, engine-notes "Specular bounce budget"): default `specular_bounces` is 0 (legacy, baselines bit-identical); the owner decides whether to raise it. BDPT, photon mapping and ReSTIR GI still use `bounces` alone. Needs a Windows build and `--self-test` (D3D12 shares `Constants`; `bounce_split` is a new last field).
- [keep] Integrator disagreements (dusk, measured-cornell, optics-bench, lava-lamp, bigger-on-the-inside, sweep 55/86, 0.93 blocks): see Now a.
- [done 2026-11-05, self-tests pass, not tried by hand] `photon_gather` walks the newest 4096 deposits of a long chain, so its estimate is unbiased only if deposit order is uncorrelated with power; check or bound. A chain past 4096 is counted and walked again taking every k-th photon from a random start (systematic sampling, unbiased; `restart_chain`).
- [done 2026-11-04] ReSTIR reservoirs only for path tracing or battle mode (was already gated: `prepare_restir`).
- [keep] Stained glass with the physical sky: not reproduced 2026-11-02 (PT and BDPT agree within 0.2%); waits on a viewer repro (camera, time, integrator, denoiser) from the owner.
- [done 2026-11-05, self-tests pass, not tried by hand] Nested dielectrics: entering vs leaving is decided from the shading normal, so a misclassified grazing hit can leave a wrong medium; use the geometric normal. The side is the triangle face normal turned toward the shading normal; `Hit::ng` is now that face normal (offsets too).
- [done 2026-11-05, self-tests pass, not tried by hand] Sun's weight in the light-selection CDF uses a nominal 50-unit receiving area; a sun beside very bright small lights can be under-sampled. The receiving area is the scene bounds (floor, box, objects) projected along the sun (`sun_area`).
- [drop] BDPT / photon: portals, MIS free-flight bias in dense media, chromatic extinction, hybrid caustics mode, adaptive radius, DoF light tracing: research-grade, one integrator (PT) is the product.
- [drop] BDPT / photon: procedural and emissive-map emitters, equiangular sampling, catcher, labels: same.
- [drop] ReSTIR visibility reuse, ReSTIR GI / PT, target ignoring coat and transmission: research-grade.
- [drop] Upscaler static-input edge detail (accumulate per jitter phase): measured trade-off, no report.
- [drop] Native TAA with jittered guides, supersampling above 100%, SMAA / FXAA, Mitchell filter: RR and DLAA cover it.
- [drop] Native denoiser low-frequency blotches at very low spp (SVGF trait): RR is the main denoiser; NRD under "Other denoisers" is the real fix.
- [drop] Native path glass scenes 10-15% slower (stack registers), NEE shadow rays through glass without refraction, stack deeper than 4: known approximations.
- [drop] Wavefront split to retry SER; `CULL_NON_OPAQUE` / `SKIP_PROCEDURAL_PRIMITIVES` variants; compact normal / UV buffer (1-2%); shared-memory launch-block investigation; re-profile after guide tables: marginal gains.
- [drop] Light sampling: sphere lights see the 64x32 inscribed mesh (under 0.5% of solid angle at the silhouette), delta point / directional lights, gobos and light linking: by design.

### Materials and textures
- [done 2026-11-04] IES horizontal planes weigh their angular span (trapezoid rule; test `uneven_horizontal_planes_weigh_their_span`).
- [done 2026-11-05, verified by hand] A texture set on a mesh is box-mapped; add an option to use the mesh's UVs. `Material::texture_uv` ("Mesh UVs" in the material editor): the set by the mesh UVs at texture-scale repeats per UV unit (packed as a negative `base_scale.w`).
- [drop] Per-map `KHR_texture_transform` and UV sets, BLEND without a base map, TEXCOORD_1, stored tangents, glTF clearcoat / sheen / anisotropy maps: rare in the files used.
- [drop] BC7 textures: lossy, against the quality rule.
- [drop] Box-mapping seams on curved shapes (triplanar), AO / opacity / displacement maps: polish.
- [drop] Layered materials: reciprocity, coat tint and thickness, coat normal map, layers on glass, rough glass compensation, RR guides ignoring layers and anisotropy: research-grade extras.
- [drop] Thin film: thickness texture and swirl bands, no film on the coat, two Airy orders; gratings: blaze, glass orders, straight-groove scene.
- [drop] Blackbody and measured spectra replace the base colour; emissive maps at LOD 0; glTF emission clamped to 20: low impact.
- [drop] Tree leaves: per-leaf colour and thin-leaf transmission (also imported foliage check).
- [drop] Procedural surfaces: smooth CSG, user SDF expressions, texture maps, fractal speed, Menger pinholes, grounding by proxy mesh (a thin torus floats), walk collision.
- [drop] Lenses: Zeiss Planar, real glasses for the double Gauss and wide angles, RR / picking with a lens, lens flare ghosts, glare dust and spider vanes, 512x512 grid.
- [drop] Spectral leftovers listed under Future goals.

### Lights and sky
- [done 2026-11-10] Auto exposure E1-E2: `photometry.rs` (`SCENE_CD_M2`, luminance helpers), Settings `auto_exposure`, `ev_bias`, `exposure_speed`, `meter_mode`, `eyes`, `eyes_strength`, manual range 2^-8..2^30, old files load manual (engine-notes "Exposure settings and photometric units"). E3 meter and adaptation done 2026-11-10 (engine-notes "Exposure meter and adaptation (E3)"; open: the D3D12 side was written but not compiled on Linux, check it on Windows; bin_scopes-style live GPU timing of the meter pass unmeasured). E4 Eyes look done 2026-11-10 (engine-notes "The Eyes look (E4)"; open: oracle check on a night scene, thresholds/rod weights/tint are unconfirmed defaults). E5 pre-exposure done 2026-11-10 (engine-notes "Pre-exposure for RR and the upscalers (E5)"; open: D3D12 side uncompiled on Linux, FSR untested, XeSS has no pre-exposure input so a P step may show in its history). E6 UI done 2026-11-10 (engine-notes "Exposure UI and the night camera": Look tab rows, status bar EV and cd/m2 in Advanced, d-pad and remote on the bias, Naked eye / Night camera presets and `airglow_warmth`; open: `NIGHT_CAMERA_EV_BIAS` -1.5 is unconfirmed by eye, tint (1.08, 1.02, 0.70) likewise). E7 tests done 2026-11-10 (engine-notes "Tests pin the display (E7)": tools and self-test pin auto off, self-test frames 240 and 245 check the meter and the Eyes look). E7b review fixes done 2026-11-10 (engine-notes "Exposure review fixes (E7b)": exposed-unit halation/glare thresholds, auto-to-manual seed, meter hold on light edits, RENDER_OK meter fields, AE-L). Next: E8 built-ins.
- [open] Firefly clamp floor relative to exposure (E7b review): the 0.05 floor is scene units, so dark scenes are clamped harder; an exposure-relative floor would change transport and needs the oracle and convergence re-measured.
- [open] E7b meter hold and auto-to-manual seed: check by eye in the live app (Linux and Windows); the 1 s hold and the seed heuristic (exposure, bias and scene name unchanged) are defaults.
- [done 2026-11-10] Auto exposure E8, built-in scenes: 94 built-ins carry `auto_exposure`, `ev_bias` (log2(old exposure / settled metered exposure), `tools/scene_exposure.json` through `tools/build_scenes.py`) and `eyes`; furnaces and measured-cornell stay manual; the day cycle keys `ev_bias`; starlight-field is auto at sun intensity 1; new `clear-night`; `environment::half_scale` stores night sky maps scaled up so intensity 1 works (oracle 1.0000); thumbnails re-render (`RENDER_VERSION` 3). Engine-notes "Built-in scenes and auto exposure". Open: hand-lit night scenes keep photopic light levels (owner default; dimming them to real levels is a per-scene edit); the biases come from one Linux 640 x 360 sweep and the meter reads tiny-light scenes to about 0.3 EV; D3D12 side of `half_scale` not compiled on Linux.
- [keep] **Night sky redesign (owner, 2026-11-10):** plan in `docs/night-sky.md` and `docs/plan-big-features.md`. Done: data (N1), astronomy (N2, N2b), Milky Way / airglow / light pollution (N4), catalogue stars as point lights (N3, 2026-11-10, engine-notes "Night sky: stars as point lights"). N5 tweaks done (limiting magnitude, star size, one Night sky group, engine-notes "Night sky: tweaks"). Still to come: N6 and N7 are done (auto exposure, Eyes look; night scenes now show at sun intensity 1); N8 scenes: starlight-field is auto and clear-night is new (E8); the starry-observatory scene is still to do.
- [open] **Planets (N5, later):** Mercury to Saturn as point lights from `astro.rs` ephemerides, coloured and with magnitudes, through the same table as the stars; not started. Star size is a render-side cone, not a display-side point spread: a glow or diffraction spike for bright stars would be a post effect (not started).
- [open] **Stars, N3 leftovers (2026-11-10):** (a) metal mirrors are GGX at α 0.001 here, not delta, so stars reach them only by light sampling: a star's reflection in a roughness-0 metal converges slowly (water and glass, which are delta, use the cone and are sharp at once); a "near-delta" partition (cone for lobes at the α floor, their specular part left out of star NEE) would fix it. (b) BDPT and photon mapping leave the stars out (no light sampling, no cone): night scenes under them miss starlight. (c) The air uses Kasten-Young air mass on the zenith optical depth, not the sun's exact `march_transmittance` (both renderers the same; a few % off near the horizon). (d) Star picks are by catalogue power, not horizon-culled: about half the star samples land below the horizon and count zero. (e) D3D12 side (root SRV t18) written but not compiled here (Linux); build and self-test it on Windows. (f) A floor material with metallic 1, roughness 0 under the night sky disagrees GPU/CPU by 0.4% (z −45) with stars off too; and a roughness-0 metal box by 5%: pre-existing, not star-related, for the accuracy audit.
- [done 2026-11-05, self-tests pass, not tried by hand] HDR10: a refusal is now retried after a display change (done 2026-11-04); still open: the R10G10B10A2 typed UAV store is not checked with `CheckFeatureSupport`. `set_hdr` checks D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE for R10G10B10A2 and falls back to scRGB as for a refused colour space.
- [ ] egui 0.37 port (steps 1-3 done on Linux): check the D3D12 build (`update_gui_textures` loops over several deltas per texture), text (Skrifa, hinted) and translucent panels in scRGB HDR and HDR10/PQ on Windows.
- [drop] HDR10: egui blends on PQ code values (text edges slightly off), `SetHDRMetaData` mastering metadata.
- [drop] Sky: clouds, aerial perspective on geometry, moon declination and texture, fixed ozone / aerosol profiles, blocky stars when zoomed, first-look bake wait.
- [drop] HDRIs: more interiors and night maps, EXR downloads; shadow catcher on objects / caustics / finite dome.
- [drop] Suns tested per miss in a loop (fine for a few), per-shape gizmos for spheres and suns.

### Volumes
- [keep] Laser as a physical beam visible only in fog or dust (owner note; the *Beam in image* rods glow by themselves): beams need volumes in the beam. Sized 2026-11-05: needs beam single scattering (a laser as a line light whose in-scatter is integrated along camera rays in fog, plus photon beams for BDPT / photon mapping): a feature, not a tail.
- [drop] Heterogeneous object media, VDB grids, animated noise fog, equiangular in noisy fog.
- [drop] True subsurface (skin-tuned), native denoiser leaving haze noisy (RR is fine), fog inside glass and through glass shadows, medium-aware guides.

### UI and editor
- **Redesign B (spec `docs/ui-redesign-b.md`, engine-notes "UI layout B"). Built 2026-11-12:** tab split (Lights, Object, Material), tool rail, one-row top bar with pins and live controls, the new Menu, outliner tree over the tab strip, collapsible sections with previews, viewport strip, the export card and the viewport's export preview, the timeline toolbar, track rows and key marks on property rows, the Scenes browser (Sort, card size, My scenes) and the Assets browser (category segment, Downloaded, download strip), icons and "Icon only" for every pin. **Open tails:**
  - Key marks: a value the scene JSON leaves out at its default can't be keyed until it is changed (the Key button and `Animation::apply` skip such fields too); the flashlight's rows have no marks (the timeline can't key it either).
  - Export: a high-resolution still shows the starting view in the viewport until it ends (no progressive preview); on very fast exports the viewport's preview trails the frame counter by a few frames.
  - Assets: local files without a cached catalogue entry are not listed under Downloaded.
  - Pins: no icons in the `+` menu and the palette list; Undo, Redo and About are not pinnable commands; a pin inside `»` has no right-click menu.
  - Scenes: no switch or self-test frame for a sort order, card size or My scenes; in development and portable runs only the `user` pack counts as My scenes.
  - `ui::fold` cuts an over-long preview at the left with no ellipsis, so each preview builder keeps to a character budget; eliding in `fold` would be sturdier.
  - Not looked at in the live app by the owner yet: the click paths (key marks, sort, downloads, Icon only) were checked by captures and code reading.
  - Owner to confirm the section 9 defaults (engine-notes lists them).
  - Vulkan on Windows: the self-test passes and `--capture-ui` captures of every tab, both browsers and a running export were looked at (2026-11-12). Linux has not drawn the new layout yet, and no hand pass on Vulkan in the live app.
- [done 2026-11-05, verified by hand] Owner, 2026-11-05: silhouette outlines should appear only for the clicked selection, not on hover while the mouse moves. The hover probe and its outline are removed (`ui.hovered` is set only by the self-test now). In Sponza an object is under the pointer almost everywhere, so the hover outline never goes away, and moving the pointer out of the viewport doesn't clear it. Drop the hover outline (`ui.hovered` from the GPU probe, gui.hlsl), or put it behind an option that is off by default.
- [done 2026-11-04] The status bar shows a failed or unavailable upscaler as "Bicubic (X unavailable)" in amber, like RR's fallback.
- [done 2026-11-05, verified by hand] Gizmo with physics: a mouse grab active when G starts stays pending; the first transform after enabling physics adds an extra undo step. G / R / S drop a pending grab; the simulation start inside a transform checkpoints the transform's original, so undo is one step.
- [done 2026-11-04] The self-test steps physics at a fixed 1/60 s (frames 237–238 no longer depend on frame time).
- [done 2026-11-05, verified by hand] A per-frame script's moves don't make the object kinematic, so with physics on the simulation's pose wins. `script::set_paths` records what the script sets at t = 0; those objects are kinematic like keyframed ones (a script that moves something only later is not seen).
- [done 2026-11-04] LPE EXR layers are named after their expressions (`lpe1_C_D_red_wall_L`).
- [drop] Modified-row dots on checkboxes and combos, preset registry refactor, lens shift in the free viewport, light thumbnails.
- [drop] Snapping: rotation to surface normal, rotate / scale snapping, edge-centre targets, lights as targets, "cursor to selected"; box select by outline; inspector and timeline keys for the whole selection; light hover outlines.
- [drop] Video export: tiling past 8192, 10-bit SDR, alpha, AOV EXR sequences, background export of a frozen copy, viewport preview, audio, resume.
- [drop] Asset browser: Sketchfab, Smithsonian, ORCA zips, 3D preview, list / delete downloads, catalog auto-refresh, cache pruning, resumable downloads.
- [drop] Curve editor tangent handles, box select; look-at targets that follow objects; arc-length camera paths; key points on the drawn path.
- [drop] Console highlighting and completion, generators listed in the Generate section, `add` for lights.
- [drop] Silhouette outlines on lights, behind glass, coplanar speckle; thumbnail cache pruning and retry.
- [drop] Startup: cold-cache re-measure, "Loading..." text, detached NGX thread on a failed start (harmless).
- [done 2026-11-05, verified by hand] Startup: NGX init (~1.5 s) still bounds the start; finish it behind the first frames with the native denoiser until ready. The join moved off the startup path: frames start with the native denoiser and `poll_ngx` takes NGX up when its thread finishes; headless runs, the self-test and --require-rr wait (`wait_ngx`).

### Materials / scenes content
- [keep] Check the overhauled scenes by eye (contact sheet of fresh thumbnails) and tune exposure / light levels where they read too dark or blown; the new physics scenes (festival-banners, swinging-lantern) need a run to confirm the cloth and the chain settle as intended.
- [keep] starry-observatory scene, after the night-sky redesign (stars and moon at a usable exposure).
- [drop] Nighthawks figures and curved glass, hourglass with flowing sand, metaball blob count / colour, a phase change rebuilding the whole scene, no scene files using the new light kinds.

### Animation, physics and cloth
- [done 2026-11-05, self-tests pass, not tried by hand] The mesh `LOADED` cache keeps a shipped file after a same-named one appears in the user folder (until restart). Hits re-resolve the path and are used only when it resolves to the same file.
- [done 2026-11-05, self-tests pass, not tried by hand] A D3D12 error mid-rebuild can leave `compact_queue` / `tlas_cache` stale. `rebuild_scene` clears the compaction queue and the TLAS cache on an error.
- [drop] Cloth: extreme mass ratios jitter, friction reaction on the body, continuous self-collision, cloth against cloth, thin rails between particles, per-step vertex-buffer append, `Record` of cloth shapes.
- [drop] Physics: joint tool in the viewport, edits that restart the simulation, motion blur with physics, kinematic jump strength, card-house settling, single-joint wrecking ball, V-HACD first-use second.
- [drop] Walking: moving platforms, a true capsule, collision cost on 20M-triangle files.
- [drop] Keyed playback cost at 4096 objects, motion blur in the viewport, deformation blur, Shift+I with no keys.
- [drop] Laser puzzle: rods in video exports, retrace per frame with physics; audio non-f32 formats and audio-synced video export.

### Import and export
- [drop] Equirect video spherical metadata (ffmpeg limitation), `--capture` JPEG, 16-bit tone-mapped PNG, TIFF / WebP / AVIF, half-float EXR, AOV copy memory at 8K.

### Scripting and tools
- [drop] Text-to-scene: smaller prompt for local models, streaming, Codex and OpenAI live tests (not installed here), per-profile keys, whole-scene replace on edit.
- [drop] NGX project GUID: settled 2026-11-06. It is the app's own identifier, which the DLSS guide (5.2.1) prescribes when NVIDIA has issued no application ID; nothing to register.
- [drop] Gamepad untested: no controller here.

### Performance
- [drop] Forest drag cost in `rebuild_scene`, a light edit rebuilding the static BLAS, simple scenes up to 7% slower from two-level traversal: small; any fix must not change the image.
- [drop] Second frame in flight: measured, a few percent at most.

### Realtime preset (B2, 2026-11-10)
- [done 2026-11-10, Vulkan only, D3D12 untouched] Top bar Quality "Realtime 67%": spp 1, bounces 3, specular bounces 8, ReSTIR DI on, scale 0.67, RR (native filter without RR). At 2560x1368: 4.9-11.4 ms GPU against 12.2-30.4 ms at the scenes' own settings, RR 3.0-3.4 ms; RMSE x1000 3.3-19.9 against 1.0-2.9, mean bias -0.8 to -5.6% (bounce truncation, worst in glass-of-water and cornell-bunny); a moving camera adds less error than at the scene's own settings (glass-of-water orbit +4.2 against +6.9 RMSE). Full table in engine-notes "Realtime preset". Open: the menu keeps reading "Realtime 67%" after picking "Good 67%" (other bundles leave transport alone); bounces 4-5 as a gentler preset; a ghosting check on more than one scene; Windows build (`Bundle::apply` gained an `rr_ok` argument, self-test callers updated).

### Platform, CI and release
- [keep] The first tag with the asset-fetch step and the ~870 MB installer exercises them for the first time: check installer size, build time and `LZMANumBlockThreads`, and run the installer on a clean machine.
- [keep] Owner request (2026-11-05): pre-build the scene and material thumbnails at release time and ship them in the installer, zip and AppImage, so a fresh install doesn't spend minutes rendering them on first open. Seed the thumbnail cache from the bundled set, keyed so an edited scene still re-renders its own.
  - **Rule (owner, 2026-11-10):** a change to a scene must re-render or remove its thumbnail. Thumbnails are keyed by a hash of the scene file, so an edited scene file orphans its old thumbnail; whoever changes a scene regenerates its thumbnail (CPU thumbnails exist: `--make-thumbnails --cpu`) or deletes the stale one, and a shipped set must hold no thumbnail whose hash no longer matches. The key also mixes `scenes::RENDER_VERSION`: an engine change that alters images without touching the scene files (astronomy, glass lift, shader fixes) bumps it, which re-renders every thumbnail by the same rule.
- [drop] CI self-test: needs a GPU runner. Signed installer: needs a certificate (SmartScreen warns).
- [drop] DXC on the runner not pinned; setting to move the data folder; leftover files after uninstall; downloaded assets not in the zip; local folder still called `cornell` (owner's choice).

## Future goals

Vulkan moved to "Now".

### Accuracy mode (owner request, 2026-10-30)
Audit the GPU path tracer's shortcuts against the CPU reference (rough glass as a lerp toward a cosine sample instead of a GGX BTDF, Russian roulette after bounce 3 and the 6-bounce default, the ray-cone mip heuristic, RGB instead of spectral) and fix the cheap ones; then an "Accurate" quality bundle (up to 64 bounces, no clamps, LOD 0 textures, no denoiser) as the high-res still default. Software mode is the CPU reference plus spectral. Measure with the convergence graph and the oracle.

### Other denoisers (owner request, low priority)
Intel OIDN 2 (best offline quality for stills, CUDA sharing with D3D12), NVIDIA OptiX (temporal mode), NVIDIA NRD (ReBLUR / ReLAX; would also fix the native filter's blotches). Compare against RR with `tools/motion_error.py`.
Done 2026-11-10: OIDN for stills (engine-notes "OIDN for stills"): captures, Save image and the CPU view's converged image, behind `still_denoise`. Open: the GPU's live converged view (a filtered image shown through the software view's reference path, both backends), an EXR option, Windows checks (build.rs DLL copy, the load by path, installer contents), OIDN's temporal-free live use is not planned.
The `Denoiser` enum (engine-notes "Denoiser enum") has the room: a variant, a label, a `Capabilities` entry and a pipeline arm in frame.rs / the Vulkan `render`. Done 2026-11-10: enum, serde migration, Render-tab combo.

### Spectral rendering, part 2
Part 1 is done (path tracer, photons, dispersion, exact thin film, gratings, lens). Open: BDPT, the photon gather's camera side, LPE and the path visualizer stay RGB; exact blackbody Kelvin lights; fluorescence (black-light mode); Jakob-Hanika uplift; colour noise under RR and the native filter unmeasured; four wavelengths instead of three.

### Polarization
Mueller / Stokes transport after spectral; a rotatable polarizing filter (Brewster's-angle reflections, stress patterns in plastic). Research-grade.

### Curved space: black holes and non-Euclidean rooms
Null geodesics (Schwarzschild, RK4) in short `RayQuery` segments around a black hole in the Cornell box with an emissive accretion disk; hyperbolic or spherical rooms by ray marching. Expect the native denoiser (RR guides are approximate). Portals cover the cheap impossible rooms.

### Realtime preset (instead of a raster mode)
Owner question 2026-11-10: a game-style mode that rasterises everything and ray traces only where needed. Not worth it: measured with the play build headless at 2560x1368, 240 frames, `--no-vsync`, RR on (trace + reconstruct = frame, ms):

| Scene | Own settings | bounces=2, spp=1 |
|---|---|---|
| cornell-bunny | 6.1 + 6.5 = 13.2 | 1.1 + 6.6 = 8.3 |
| glass-of-water | 23.2 + 6.5 = 32.3 | 1.9 + 6.6 = 11.1 |
| nighthawks | 8.4 + 6.4 = 15.6 | 1.4 + 6.6 = 8.8 |
| stylised-forest | 7.3 + 6.5 = 14.6 | 2.0 + 6.0 = 8.5 |
| colonnade-night | 8.9 + 5.8 = 15.2 | 1.8 + 6.2 = 8.6 |

Rasterising the first hit could save at most part of the 1-2 ms trace; DLSS RR at about 6.5 ms is the fixed cost. Instead: a "Realtime" preset with 1 spp, 2-3 bounces, ReSTIR DI and RR at an upscaling render scale. Follow-up at 2560x1440, 1 spp, 3 bounces: RR ran at native scale above (default `scale` 1.0). At scale 1 / 0.67 / 0.5 the trace is glass 2.6 / 1.3 / 0.6 ms, nighthawks 1.7 / 0.7 / 0.4, colonnade 2.4 / 1.0 / 0.5, and RR 6.5 / 3.0 / 2.2 ms; the frame then stops at 6.94 ms (144 fps) even with `--no-vsync`: IMMEDIATE is picked, but headless runs still present to a hidden window and the compositor throttles them to the monitor rate (XWayland: 17 ms). Fix: headless runs skip acquire/present. Quality, 8 frames vs a 600-frame reference: colonnade-night fine (RMSE 0.008-0.012); glass-of-water black at 3 bounces (RMSE 0.20) and nighthawks' interior through glass darker (bias -0.024), since glass chains need about 8 bounces. So the preset needs a separate specular/transmission bounce limit, which the integrator lacks. Not checked: RR ghosting with a moving camera.


### Neural radiance cache
A tiny MLP trained online on path suffixes (NVIDIA's NRC SDK, or our own with cooperative vectors, SM 6.9); far less noise in heavy interiors (Sponza). Measure with the convergence graph and the CPU reference.

### 3D Gaussian splats
Ray-traced splats (3DGRT-style) in proxy geometry, blended in depth order inside the `Proceed()` loop; a captured object inside the path-traced Cornell box.

## Won't do

Closed by owner decision or by the triage above (the [drop] lines in "Open tails" hold the detail):

- Live wallpaper (WorkerW trick) and the Blender live link: removed 2026-11-02, not planned.
- BC7 textures (lossy, against the quality rule); any lower-quality or "performance" mode other than the Realtime Mode exception.
- A second frame in flight (measured: a few percent at most).
- SER reordering in its current form (measured 35-105% slower; a wavefront rewrite is deferred).
- Gamepad testing, CI self-test on a GPU runner, signed installer, NGX GUID change: blocked on hardware, certificates, or moot for a never-published project.
- True delta point and directional lights, light linking and gobos (by design).
- Native TAA / SMAA, SVGF blotch fix (RR is the product; NRD is the real fix), ReSTIR GI / PT, VDB files, skin-tuned subsurface.
- Removed by the owner 2026-11-10: light painting and the screensaver mode (taken out of the app the same day); VR via OpenXR; the lightmap baker and texture painting; FSR on Vulkan and XeSS on Linux (both blocked on vendor SDKs); moving the window between monitors (coded, untested, left as is).
- Dropped by the owner 2026-11-10 from the big-feature plan (`docs/plan-big-features.md`): the OptiX denoiser, the D3D12 display-only device for GPUs without ray tracing, DLSS Frame Generation, and Reflex (no point without frame generation).
- Every other [drop] in "Open tails".
