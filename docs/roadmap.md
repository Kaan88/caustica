# Caustica roadmap

The current plan, rewritten 2026-11-04. The long history (what was built, measured and decided, item by item) lives in `docs/engine-notes.md` (`grep -n '^## '`) and in git; this file keeps only what is done at a glance, what comes next, and a triaged list of open tails.
Order: **the "Plan" section below (2026-11-12).** It replaces the 2026-11-04 order (correctness, code health, tails, Vulkan); the sections under "Now" keep the detail of each open item. Tails marked [drop] are closed and will not be worked unless the owner asks.
Research documents: [ui-plan.md](archive/ui-plan.md) (UI), [scene-research.md](archive/scene-research.md) (scenes).

## Ground rules from the owner

- **Public since 2026-11-06, under MIT for the code.** Models, textures, HDRIs and SDKs keep their own terms; `THIRD-PARTY-NOTICES.md` lists them, and anything new that ships goes in it.
- **Licence risks the owner has accepted (2026-11-06), and will act on if raised:** Sponza stays bundled, though the CRYENGINE agreement covers games built with CRYENGINE; the GPLv3 FFmpeg stays bundled without published sources; releases up to v0.10.1 stay up without notices; the About box shows the NVIDIA logo, plus on Windows the AMD FSR and Intel logos, though the DLSS licence only grants word marks and Intel's bars promotional use of its name. Research-only and NC assets such as the Stanford scans are fine only while the app stays free. If it is ever sold: those assets go, and NVIDIA must be notified first (DLSS licence, exhibit section 4).
- **Performance work never lowers quality** (owner rule, 2026-11-02). The defaults stay as they are (spp, bounces, sample cap, DLAA render scale, RR preset), and no "performance mode" or lower-quality option is added. Speed has to come from doing the same work more efficiently. A change that alters the image has to be justified as unbiased, or as equal under the convergence measurement. Exception (owner, 2026-11-10): the Realtime Mode (1 spp, separate specular bounces, ReSTIR, RR at an upscaling scale), kept apart from the quality bundles and restoring the scene's own sampling when left.

- **Who it is for (owner, 2026-11-12):** the owner alone, for fun. No work for an outside audience (README for strangers, shareable benchmark scores, slim updates, signing). Only the owner's NVIDIA RTX card matters: no AMD or Intel testing, and FSR and XeSS are not reasons to keep anything. BDPT and photon mapping have scenes of their own: keep and improve them. The release rhythm and size don't matter.
- **Ask before removing anything** (owner, 2026-11-12): features, files and docs alike. Approved so far: the four toys in Plan step 3.

## Plan (2026-11-12)

From the project review of 2026-11-12 and the owner's answers. Each step ends with verify, review and a snapshot; commit when the owner asks.

1. **[draft, implemented; to be checked properly later] UI skin pass (owner, 2026-11-12: egui "feels cheap no matter what, like crappy html buttons").** Committed (`c960155`) and shipped in v0.13.0, but a draft (owner, 2026-11-17): the owner's proper check in the live app and the open items below come later; the plan moves on meanwhile. Stay on egui and replace the stock look with the app's own painted widgets; the layout B structure stays.
   - **Mockups first** (owner): boards for the skin on the design canvas (button, toggle, checkbox, slider, combo, text field, section header, tab strip, menu and popup, in rest, hover, pressed, disabled and focused states; then the Render tab and the top bar drawn with them). The owner picks before any code. **Owner picked B "Instrument", 2026-11-12, "with no cutbacks"** (every gradient, highlight, inset shadow, band and state as drawn; handoff in `docs/skin-handoff.md`, board sources in `docs/mockups/`). Drawn 2026-11-12: https://claude.ai/artifact/W3Fp5p8GoqgjoaSsV3xDxR (private to the owner), board A "Quiet" (flat, borderless, hover-revealed buttons, Inter) and board B "Instrument" (raised controls, recessed fields, section bands, IBM Plex); each has the app at 1280×720 with the Render tab, and a sheet of every control in five states.
   - **Widgets:** one module of painted controls with those states, used in place of `ui.button`, `Slider`, `ComboBox`, `Checkbox` and `TextEdit`; styled as board B.
   - **Motion:** 80-150 ms eases on hover, press, section open and tab change (`animate_bool`); nothing that keeps the UI repainting at rest.
   - **Type and rhythm:** a UI font that holds up at small sizes without hinting, two weights, three sizes; one row height and one spacing grid, labels and values on shared columns.
   - **Depth:** shadows and layering for menus, popups and the floating strips.
   - **Order:** restyle the Render tab first and compare it with the current one in the live app; if that fixes the feel, do the rest tab by tab. If it does not, reopen the toolkit question (a webview UI was weighed and set aside: compositing over the swap chain and re-plumbing every control is weeks). Known limit: text at 100 % scale stays a little softer than native text.
   - The self-test's UI frames and `--capture-ui` captures must keep passing; captures get re-baselined once at the end.
   - **Built 2026-11-12/13, the whole pass** (engine-notes "UI skin: Instrument"): `src/ui/skin.rs` (the rasteriser, the atlas, every widget), IBM Plex as the app's font, and the skin on everything: the six tabs, top bar, tool rail, tab strip, outliner, status bar, timeline, the Scenes and Assets browsers, menus, tooltips, toasts, the Ctrl+K palette and the windows. The old look and its code paths are gone. `--ui-skin-sheet` shows every control in every state. Snapshot `builds/2026-11-13-ui-skin-complete`. Committed as `c960155`, released as v0.13.0.
   - **Owner's answers, 2026-11-12:** every on/off row is a checkbox; the "modified" dot sits in the gutter before the rail; a slider's unit is outside the number field (end of the label column).
   - **Open, for the owner's eye (a headless capture cannot show them):** menus and submenus open, hover and pressed looks in the live app, the Statistics, Graph and Console windows (no capture switch), the colour picker in use, text weight on the HDR screen.
   - **Open, known gaps:** tooltip text is 12.5 points, the board's is 11.5 (egui 0.37 has no hook short of touching every tooltip call); the Scenes and Assets cards and the colour wheels follow the board's language but the board draws neither, so their layout is a guess; the status bar's right-hand text is clipped, not ellipsised, in a narrow window; spinners keep an idle viewport drawing every frame while their job runs (since idle repaints are honoured); the theme is not pinned to dark, so a light-mode Windows would get egui's light style; captures (`captures/validation-ui.png` and friends) not yet re-baselined; Linux not looked at.
   - **Open, technical:** the skin's translucent pixels are corrected for scRGB over the panel colour only (a popup's shadow over the image is approximate), and HDR10 is uncorrected; pressed content moves one pixel at 100 %, not half a point.
2. **[done 2026-11-12] Land UI layout B.** Built and committed 2026-11-12 (`cc9bf00`), drawn on Vulkan on Windows. Tails, not blocking: draw it on Linux, the owner's pass in the live app (with step 1's check), and the items under "UI and editor".
3. **Cut four toys (owner approved). Done 2026-11-15:** the phone remote (`src/remote.rs`, the `qrcode` crate), audio-reactive lights (`src/audio.rs`, `cpal`, `rustfft`, the Lights tab's Audio section), the laser puzzle (`src/laser.rs`, `Settings::puzzle`, the Puzzle window, the `scenes/puzzles` pack of 5 scenes) and the achievements (groups, tiers, progress bars, toasts, the Achievements window) are gone, with their palette entries, self-test frames, README and notes text. Scene files that carry `puzzle` or `audio` still load (the keys are ignored), as do old `stats.json` files and prefs that pin the removed windows (unknown pins are skipped). The optics bench and the beam-splitter material stay. The `audio-equalizer` scene went too (owner, 2026-11-15); `lava-lamp` and `nighthawks` no longer carry an `audio` key.
   - **Statistics stay and grow (owner, 2026-11-12):** `src/stats.rs` loses the achievements (groups, tiers, progress bars, toasts) and keeps the lifetime totals in a plain Statistics window. Redundant fields go (owner: "remove redundant statistics"): the ones that only fed achievements or the cut toys (`marks`, `achievements`, `puzzles_solved`, `puzzles_par`, `audio_seconds`; all removed 2026-11-15, as were `dark_seconds` and `hot_reloads`), and, proposed for the owner to confirm, `dark_seconds` (flashlight time) and `hot_reloads` (a developer counter). Kept: `paths`, `gpu_seconds`, `images_saved`, `videos_exported`, `max_samples`, `presets_seen`, `scenes_seen`, `walked`, `physics_seconds`. To add, the owner picks: per-session figures beside the lifetime ones, time per integrator and per denoiser, rays per second now and best, samples and time per scene, most-used scenes, exports by format, and a reset. An old `stats.json` with achievement fields must still load.
4. **[done 2026-11-15, except the oracle run] Guard rails** (`e10ab78`: `src/guards.rs`, the one entry list in `src/shader_entries.rs`). Still open: the oracle run in `tools/verify.ps1`. Plan:
   - a test that reads `cbuffer Frame` from `shaders/common.hlsli` and compares names and offsets with `Constants`;
   - one generated shader entry list used by both build.rs and `hotreload::ENTRIES`;
   - a Settings-field test (every field is bounded by `validate()` and classed in `transport_key`);
   - a three-scene, low-spp oracle run in `tools/verify.ps1`.
5. **D3D12 phase-out, proof first** (below). D3D12 stays the default until both gates pass.
6. **Oracle green on the remaining backend**, path tracer first: texture mip bias, emissive maps, Sponza alpha, the roughness-0 metal gap, the ReSTIR excess (Now a and e).
7. **BDPT and photon mapping, improved:** portals, stars, the specular bounce budget, the −ng walk rule (B7 leftovers), then the open discrepancies (sunset-tank, optics-bench, the sdf-gallery cone, spectral photon +8% on street-bokeh, the white-furnace gather bias). This reopens the two "[drop] BDPT / photon" lines under "Rendering and integrators" where they overlap.
8. **Code health:** split `App::redraw`, `App::new` and `window_event` (`src/main.rs`) as pure moves; (the engine-notes split is done, see "Docs" below).
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

### Docs reorganised (done 2026-11-15)

Fixed on 2026-11-12 without removing anything: stale counts and contradictions in the README, the project line in `CLAUDE.md`, a note in `windows-handoff.md`. The owner approved four proposals and they were done on 2026-11-15, nothing removed, only moved:
- Done 2026-11-15: split `docs/engine-notes.md` (486 KB) into `docs/notes/<section>.md` with an index.
- Done 2026-11-15: moved finished plans to `docs/archive/`: `ui-plan.md`, `ui-review.md`, `egui-upgrade-plan.md`, `scene-research.md`, and `plan-big-features.md` once its backlog is folded in here.
- Done 2026-11-15: cut the README to what the app is, how to build and run it and its limits, moving the feature tour to `docs/features.md`.
- Done 2026-11-15: trimmed this file's finished "[done]" lines under "Now" and "Open tails" (their detail is in engine-notes).

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
- UI usability review, 2026-11-06 (`docs/archive/ui-review.md`): 42 findings, fixed in the working tree and since, including the SDR UI gamma bug (the whole UI drawn too dark), undo of panel edits, unsaved-work warning and a restorable session; the rest are listed open in the file.
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
- [open] **Truth switch leftovers:** exact versions of the six hook rows (next accuracy step); `lighthouse-fog` fails the block test with truth off at 4096/2048 (block (2, 3) 1.0581, z 7.3; suspect the rotating spot's time); `bounces` is capped at 32 by `validate` (the audit asks 64); roulette cost per sample not timed.
- [open] **B7 leftovers:** BDPT and photon-mapping walks still leave from −ng under the face (`integrators.hlsl`, `bdpt.rs`, `photon.rs`); measure them on light-temperature-row against the path tracer and apply the same rule if they differ. The shared rule (the face's visibility) is a convention: pbrt's f = 0 across the face or a terminator offset (audit C2) are the alternatives.

Still open:
- [keep] Integrator sweep rerun in full with matched filters (2026-11-04, 400 spp, 240 × 135, `captures/sweep-2026-11-04`): 15 of 174 comparisons beyond 5%. To trace, largest first: sunset-tank BDPT 112% / photon 57% (colour shifts, not noise-like), optics-bench 72% / 71% (slow PT caustic), birthday-cake photon 23%, underwater BDPT 17%, lava-lamp photon 11%, shape-material-matrix photon 11%, sunlit-room-day-cycle photon 10.5%, snow-globe photon 10%, bigger-on-the-inside 8% (portals), gem-display photon 7%. dark-side (553% / 214%) is near-black (means ~1e-4), not meaningful at this spp. Next: rerun the flagged scenes at 4096 spp before reading anything into them.
- [keep] optics-bench (beam splitters) and lava-lamp (SDF emitters) could not be judged by the CPU reference; both now render on it (2026-11-10, Now e), so they can be re-measured against the path tracer. bigger-on-the-inside is a known gap in BDPT / photon mapping (no portals).
- [keep] Photon mapping on the white furnace: 0.9945 (z −22) with box filters, a gather-radius bias (expected of progressive photon mapping at 1024 spp; check that it falls with more passes).
- [keep] GPU photon mapping is dark inside sdf-gallery's green glass cone (2026-11-10, 1024 spp: 0.075 against 0.096-0.099 from the CPU's photon mapping and both path tracers; oracle block (5, 5) 0.976, z -7.8). Not traced: the walk is BDPT's, which passes there, so look at the photon side (deposits or the gather) on a procedural dielectric. The CPU reference renders photon mapping now (engine-notes "CPU reference renderer"), so this and the white-furnace item can be judged by `tools/oracle.py --integrator 2 --cpu-integrator 2` at equal spp.

### b. Code health — done 2026-11-04

Pure code moves, the self-test passing unchanged: the self-test script and checks to `src/selftest.rs`; `impl Gpu` into `src/gpu/` (init, device, scene, passes, frame, capture, util; `gpu.rs` 6.1k → 0.9k lines); `ui.rs` into `src/ui/` (windows, overlay, chrome, ai, scene / render / look tabs, stage; 5.7k → 1.5k lines). Map in docs/claude/architecture.md. Open: `gpu/` still mixes D3D12 calls with renderer logic; the Vulkan backend needs a device-abstraction seam below it (see d).

### c. Tails triage

Work the [keep] items in "Open tails" below, grouped by area, smallest first. Anything left [drop] stays closed.

### Owner requests, to order with the owner (2026-11-06)


- The UI review's findings are all fixed (`docs/archive/ui-review.md`); what is left there are the per-batch notes of defaults taken, plus R's meaning (restart against modal rotate) for the owner to decide.

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
- [keep] Stained glass with the physical sky: not reproduced 2026-11-02 (PT and BDPT agree within 0.2%); waits on a viewer repro (camera, time, integrator, denoiser) from the owner.
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
- [open] Firefly clamp floor relative to exposure (E7b review): the 0.05 floor is scene units, so dark scenes are clamped harder; an exposure-relative floor would change transport and needs the oracle and convergence re-measured.
- [open] E7b meter hold and auto-to-manual seed: check by eye in the live app (Linux and Windows); the 1 s hold and the seed heuristic (exposure, bias and scene name unchanged) are defaults.
- [keep] **Night sky redesign (owner, 2026-11-10):** plan in `docs/night-sky.md` and `docs/plan-big-features.md`. Done: data (N1), astronomy (N2, N2b), Milky Way / airglow / light pollution (N4), catalogue stars as point lights (N3, 2026-11-10, engine-notes "Night sky: stars as point lights"). N5 tweaks done (limiting magnitude, star size, one Night sky group, engine-notes "Night sky: tweaks"). Still to come: N6 and N7 are done (auto exposure, Eyes look; night scenes now show at sun intensity 1); N8 scenes: starlight-field is auto and clear-night is new (E8); the starry-observatory scene is still to do.
- [open] **Planets (N5, later):** Mercury to Saturn as point lights from `astro.rs` ephemerides, coloured and with magnitudes, through the same table as the stars; not started. Star size is a render-side cone, not a display-side point spread: a glow or diffraction spike for bright stars would be a post effect (not started).
- [open] **Stars, N3 leftovers (2026-11-10):** (a) metal mirrors are GGX at α 0.001 here, not delta, so stars reach them only by light sampling: a star's reflection in a roughness-0 metal converges slowly (water and glass, which are delta, use the cone and are sharp at once); a "near-delta" partition (cone for lobes at the α floor, their specular part left out of star NEE) would fix it. (b) BDPT and photon mapping leave the stars out (no light sampling, no cone): night scenes under them miss starlight. (c) The air uses Kasten-Young air mass on the zenith optical depth, not the sun's exact `march_transmittance` (both renderers the same; a few % off near the horizon). (d) Star picks are by catalogue power, not horizon-culled: about half the star samples land below the horizon and count zero. (e) D3D12 side (root SRV t18) written but not compiled here (Linux); build and self-test it on Windows. (f) A floor material with metallic 1, roughness 0 under the night sky disagrees GPU/CPU by 0.4% (z −45) with stars off too; and a roughness-0 metal box by 5%: pre-existing, not star-related, for the accuracy audit.
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
- [drop] Modified-row dots on checkboxes and combos, preset registry refactor, lens shift in the free viewport, light thumbnails.
- [drop] Snapping: rotation to surface normal, rotate / scale snapping, edge-centre targets, lights as targets, "cursor to selected"; box select by outline; inspector and timeline keys for the whole selection; light hover outlines.
- [drop] Video export: tiling past 8192, 10-bit SDR, alpha, AOV EXR sequences, background export of a frozen copy, viewport preview, audio, resume.
- [drop] Asset browser: Sketchfab, Smithsonian, ORCA zips, 3D preview, list / delete downloads, catalog auto-refresh, cache pruning, resumable downloads.
- [drop] Curve editor tangent handles, box select; look-at targets that follow objects; arc-length camera paths; key points on the drawn path.
- [drop] Console highlighting and completion, generators listed in the Generate section, `add` for lights.
- [drop] Silhouette outlines on lights, behind glass, coplanar speckle; thumbnail cache pruning and retry.
- [drop] Startup: cold-cache re-measure, "Loading..." text, detached NGX thread on a failed start (harmless).

### Materials / scenes content
- [keep] Check the overhauled scenes by eye (contact sheet of fresh thumbnails) and tune exposure / light levels where they read too dark or blown; the new physics scenes (festival-banners, swinging-lantern) need a run to confirm the cloth and the chain settle as intended.
- [keep] starry-observatory scene, after the night-sky redesign (stars and moon at a usable exposure).
- [drop] Nighthawks figures and curved glass, hourglass with flowing sand, metaball blob count / colour, a phase change rebuilding the whole scene, no scene files using the new light kinds.

### Animation, physics and cloth
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

- The achievements (`src/stats.rs` keeps the lifetime statistics), the phone remote (`src/remote.rs`, `qrcode`), audio-reactive lights (`src/audio.rs`, `cpal`, `rustfft`) and the laser puzzle (`src/laser.rs`, the `scenes/puzzles` pack of 5 scenes): removed 2026-11-15 (Plan step 3). The optics bench and the beam-splitter material stay.
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
