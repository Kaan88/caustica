# Caustica roadmap

The current plan, rewritten 2026-11-04. The long history (what was built, measured and decided, item by item) lives in `docs/engine-notes.md` (`grep -n '^## '`) and in git; this file keeps only what is done at a glance, what comes next, and a triaged list of open tails.
Order: **correctness against the CPU reference, then code health, then the tails marked [keep], then the Vulkan backend.** Tails marked [drop] are closed and will not be worked unless the owner asks.
Research documents: [ui-plan.md](ui-plan.md) (UI), [scene-research.md](scene-research.md) (scenes).

## Ground rules from the owner

- **Personal project, never published.** License terms don't constrain anything: use any model, scene, HDRI or texture, including research-only, NC, ND and SA assets, Sponza, the Stanford scans and Disney data. Assets may be downloaded into the project, or into a local `assets/` folder, as convenient. The licensing cautions in scene-research.md are kept only as background.
- Credits stay in `assets/CREDITS.md` out of courtesy, not obligation.
- **Performance work never lowers quality** (owner rule, 2026-11-02). The defaults stay as they are (spp, bounces, sample cap, DLAA render scale, RR preset), and no "performance mode" or lower-quality option is added. Speed has to come from doing the same work more efficiently. A change that alters the image has to be justified as unbiased, or as equal under the convergence measurement.

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

## Now

Ordered. Each step ends with verify, review, snapshot, then a commit when the owner asks.

### a. Correctness against the CPU reference — first pass done 2026-11-04

`tools/oracle.py` renders scenes on the GPU (raw) and with the CPU reference and tests agreement statistically (engine-notes "Oracle and the pixel-filter finding"). All 8 default scenes pass for path tracing (within 0.6%, |z| < 3.5). Findings:
- **The BDPT / photon "biases" were the pixel filter.** BDPT and photon mapping always use a box filter; the path tracer's default Blackman-Harris doesn't sum to 1 over the pixel grid, so image means at sharp bright lights moved by ±0.6% (Cornell, sign flipping with light size) up to 10% (a sub-pixel sun). With a 1-px box on both sides: Cornell BDPT 0.9999, photons 0.9993; measured-cornell 0.9996 / 0.9988 (closes "+0.9%"); sky-garden-dusk BDPT/PT 0.9956 and BSDF-only/MIS 1.002 (closes the dusk gap and "PT disagrees with itself"). The oracle and `integrator_sweep.py` now compare with matched box filters.
- **BSDF-only vs light sampling at the bounce limit:** BSDF-only renders 0.4% darker at 6 bounces (light sampling at the last vertex adds one path length BSDF sampling doesn't); equal at 32. A convention, not a bug.
- **shape-material-matrix's 0.93 blocks:** gone under the oracle's test (worst block 0.978, z −4.2). Closed.

Still open:
- [keep] Integrator sweep rerun in full with matched filters (2026-11-04, 400 spp, 240 × 135, `captures/sweep-2026-11-04`): 15 of 174 comparisons beyond 5%. To trace, largest first: sunset-tank BDPT 112% / photon 57% (colour shifts, not noise-like), optics-bench 72% / 71% (slow PT caustic), birthday-cake photon 23%, underwater BDPT 17%, lava-lamp photon 11%, shape-material-matrix photon 11%, sunlit-room-day-cycle photon 10.5%, snow-globe photon 10%, bigger-on-the-inside 8% (portals), gem-display photon 7%. dark-side (553% / 214%) is near-black (means ~1e-4), not meaningful at this spp. Next: rerun the flagged scenes at 4096 spp before reading anything into them.
- [keep] optics-bench (beam splitters) and lava-lamp (SDF emitters) can't be judged by the CPU reference; bigger-on-the-inside is a known gap (no portals in BDPT / photon mapping).
- [keep] Photon mapping on the white furnace: 0.9945 (z −22) with box filters, a gather-radius bias (expected of progressive photon mapping at 1024 spp; check that it falls with more passes).
- [keep] Extend the reference to HDRIs, sphere and sun lights, the physical sky only if a new question needs them.
- [done 2026-11-05, measured] give BDPT and photon mapping the path tracer's pixel filter, so all integrators render the same image by default. BDPT and photon camera rays take `filter_offset`; a light-tracing splat goes to one pixel of the filter's support picked with probability w / W, weighted by W (`splat_filtered`: unbiased, one atomic splat per sample; splatting the whole support hung the GPU on self-test frame 295). Box 1 px renders as before. At 1024 spp with the scenes' filters, BDPT / PT 0.998–1.001 and photon / PT 0.995–1.001 (cornell-caustic, veach-ajar, measured-cornell).

### b. Code health — done 2026-11-04

Pure code moves, the self-test passing unchanged: the self-test script and checks to `src/selftest.rs`; `impl Gpu` into `src/gpu/` (init, device, scene, passes, frame, capture, util; `gpu.rs` 6.1k → 0.9k lines); `ui.rs` into `src/ui/` (windows, overlay, chrome, ai, scene / render / look tabs, stage; 5.7k → 1.5k lines). Map in docs/claude/architecture.md. Open: `gpu/` still mixes D3D12 calls with renderer logic; the Vulkan backend needs a device-abstraction seam below it (see d).

### c. Tails triage

Work the [keep] items in "Open tails" below, grouped by area, smallest first. Anything left [drop] stays closed.

### Owner requests, to order with the owner (2026-11-06)

- Rework and expand the achievements: some fall far too fast ("billions" takes a couple of seconds).
- Offset the flashlight from the camera: it shoots straight from the eye, so its shadows are barely visible.

### Self-update, open items

Built 2026-11-06 (engine-notes "Self-update"). Open:
- The repo went public on 2026-11-06 with these licence questions open, for the owner: Sponza's CRYENGINE terms, FFmpeg's licence for the bundled build, Stanford's non-commercial terms, the licence of the app's own code (the repo has no `LICENSE`), and the releases up to v0.10.1, which ship no notices.
- The Windows path has never run (the list is in the handover report); the first release with `SHA256SUMS` must be installed by hand.
- Later, if wanted: resuming an interrupted download; slim updates without the 700 MB of assets; signing, since `SHA256SUMS` doesn't protect against a compromised account.

### d. Vulkan backend

Portability: Linux, and AMD / Intel drivers where Vulkan RT is stronger. The upscaler SDKs already ship Vulkan paths: NGX (`nvsdk_ngx_vk.h`, `nvsdk_ngx_helpers_vk.h`, `nvsdk_ngx_helpers_dlssd_vk.h`, all in `vendor/dlss/include`), XeSS (`xess_vk.h`) and FSR (the FidelityFX API has a VK backend).

- **Approach:**
  - Split `gpu.rs` (step b) into a backend-neutral `Renderer` and a backend trait: resources / textures, buffers, BLAS / TLAS build, compute dispatch, barriers, timestamp queries, swapchain present, readback and the upscaler hooks. Keep the D3D12 backend as the reference implementation.
  - Vulkan via the `ash` crate.
  - Ray tracing: `VK_KHR_acceleration_structure` + `VK_KHR_ray_query`, mirroring the inline `RayQuery` design; no pipelines or SBT. `VK_NV_ray_tracing_invocation_reorder` (raygen-only, like D3D12 SER) is the later route to reordering, which measured slower on D3D12.
  - Shaders: the same HLSL to SPIR-V with `dxc -spirv` (`[[vk::binding]]` annotations or `-fvk-*-shift` flags for the register layout). One source of truth for the `Constants` layout.
- **Shaders, checked 2026-11-04:** `tools/spirv_check.py` compiles all 24 compute / graphics entries of build.rs to SPIR-V (Vulkan 1.3) unchanged with the DXC release build (`vendor/dxc-spirv`, fetched by `tools/setup.ps1` `Get-DxcSpirv`; the Windows SDK's dxc has no SPIR-V codegen); the 7 SER libraries are left for a raygen port. Still to settle with the real descriptor layout: register spaces become descriptor sets one-to-one, and t / u / b registers of the same number in one space share a binding unless shifted (`-fvk-t-shift` etc.); the bindless texture array needs `VK_EXT_descriptor_indexing` (core in 1.2).
- **Progress (phases in engine-notes "Vulkan backend: design"):** phases 1–6 are done on Windows (2026-11-05): the whole renderer runs on `--vulkan` (denoiser, every integrator, ReSTIR, glare, lens, LPE, maps, environment, LUT, HDR, readbacks, scopes, timestamps, DLSS RR / SR and XeSS), matching D3D12 within noise; BLAS reuse, compaction and TLAS refit as on D3D12; the "Compiling shaders N/22" screen on both backends. Open: FSR on Vulkan (the vendored FidelityFX SDK has only the DX12 loader: blocked until a Vulkan build of the SDK is vendored); XeSS on Linux (Intel ships no `libxess.so` up to SDK 3.0.2: blocked; `bridge/vk.cpp` would `dlopen` one placed beside the binary); DLSS RR / SR on Linux runs on NVIDIA (first run 2026-11-06, RTX 4070 Ti SUPER, driver 615.78; engine-notes "Linux on a real desktop"); shader hot reload on Vulkan (done 2026-11-05, verified by hand; it now also reloads while paused or converged, and compiles the entries in parallel, 84 s → 24 s: `hotreload` compiles SPIR-V with the SPIR-V DXC and build.rs flags, `reload_shaders` rebuilds the compute pipelines; the UI pipelines stay); the self-test under the validation layer (`--vulkan --self-test` passes on the RTX, which has no layer installed; with Windows HDR on, the RTX's Vulkan surface offers no HDR format, so its HDR checks are skipped with a note: HDR output on Vulkan is untested until a surface offers scRGB or HDR10; the VM covers headless runs only); pumping window messages during a cold shader compile (done 2026-11-05, untested: `progress::watch` peeks the queue with PM_NOREMOVE every 16 ms, which keeps Windows from marking the window as not responding).
- **Owner reports, open (2026-11-05):**
  - [done 2026-11-05, verified] Anisotropic texture filtering had no effect on Vulkan: the map sampler (`vulkan/layout.rs`) was plain linear. It now has 16x anisotropy, as D3D12's, with `samplerAnisotropy` enabled where the device offers it. Checked: a low view of texture-loft-corner is 31% sharper at 16x than 1x on Vulkan (D3D12 36%), and the backends' 16x images differ by 0.3%.
  - [done 2026-11-05, verified by hand] Pausing froze the UI on Vulkan: `present` skipped the swap chain on frames that trace nothing (paused, and also converged). It now presents every frame unless the window is minimized, as D3D12 does.
- **Risks and size:** the largest item on the list; budget weeks. Barriers and layouts, descriptor management and the NGX Vulkan device-extension requirements are the hard parts.
- **Linux port:** done 2026-11-06 on an RTX under KDE Wayland (engine-notes "Linux on a real desktop"): exit crashes, `--self-test`, DLSS RR / SR, HDR output and `display::Monitor` (through `wp_color_manager_v1`, not KWin's config file), shader hot reload. Open: moving the window between monitors (coded; the owner has one monitor); highlights above the reference white, unmeasurable by screenshot; the `linux` CI job on GitHub runs once CI is back on.

## Open tails

Verdicts: [keep] = correctness bug, owner request or report, blocks Vulkan or a release, or small with clear user value. [drop] = polish nobody asked for, research-grade, against an owner rule, or blocked on hardware / certificates we lack. Dropped items are listed again under "Won't do".

### Rendering and integrators
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
- [keep] **Night sky redesign (owner):** stars and moon are physically scaled so they need Exposure 10 plus Sun intensity 1000 to show; Exposure tops out at 10x; no moon-position readout; the owner does not like the current look. Revisit with the owner (engine-notes "Physical sky: night, clock time, altitude").
- [done 2026-11-05, self-tests pass, not tried by hand] HDR10: a refusal is now retried after a display change (done 2026-11-04); still open: the R10G10B10A2 typed UAV store is not checked with `CheckFeatureSupport`. `set_hdr` checks D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE for R10G10B10A2 and falls back to scRGB as for a refused colour space.
- [drop] HDR10: egui blends on PQ code values (text edges slightly off), `SetHDRMetaData` mastering metadata.
- [drop] Sky: clouds, aerial perspective on geometry, moon declination and texture, fixed ozone / aerosol profiles, blocky stars when zoomed, first-look bake wait.
- [drop] HDRIs: more interiors and night maps, EXR downloads; shadow catcher on objects / caustics / finite dome.
- [drop] Suns tested per miss in a loop (fine for a few), per-shape gizmos for spheres and suns.

### Volumes
- [keep] Laser as a physical beam visible only in fog or dust (owner note; the *Beam in image* rods glow by themselves): beams need volumes in the beam. Sized 2026-11-05: needs beam single scattering (a laser as a line light whose in-scatter is integrated along camera rays in fog, plus photon beams for BDPT / photon mapping): a feature, not a tail.
- [drop] Heterogeneous object media, VDB grids, animated noise fog, equiangular in noisy fog.
- [drop] True subsurface (skin-tuned), native denoiser leaving haze noisy (RR is fine), fog inside glass and through glass shadows, medium-aware guides.

### UI and editor
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
- [drop] NGX project GUID is a development one: moot, the project is never published.
- [drop] Gamepad untested: no controller here.

### Performance
- [drop] Forest drag cost in `rebuild_scene`, a light edit rebuilding the static BLAS, simple scenes up to 7% slower from two-level traversal: small; any fix must not change the image.
- [drop] Second frame in flight: measured, a few percent at most.

### Platform, CI and release
- [keep] The first tag with the asset-fetch step and the ~870 MB installer exercises them for the first time: check installer size, build time and `LZMANumBlockThreads`, and run the installer on a clean machine.
- [keep] Owner request (2026-11-05): pre-build the scene and material thumbnails at release time and ship them in the installer, zip and AppImage, so a fresh install doesn't spend minutes rendering them on first open. Seed the thumbnail cache from the bundled set, keyed so an edited scene still re-renders its own.
- [drop] CI self-test: needs a GPU runner. Signed installer: needs a certificate (SmartScreen warns).
- [drop] DXC on the runner not pinned; setting to move the data folder; leftover files after uninstall; downloaded assets not in the zip; local folder still called `cornell` (owner's choice).

### CPU reference
- [keep] Statistical GPU-vs-CPU check and the shape-material-matrix blocks: see Now a.
- [keep] Reference support for HDRIs, the physical sky, sphere and sun lights, only as far as the oracle needs.
- [drop] Reference support for fog, SDF objects, textures, layers, object media, portals, splitters, IES, depth of field, lenses, other projections, spectral; f64 accumulation.
- [drop] UI: render with no GPU device at all, save the CPU image from the UI, full-view toggle, convergence-graph reference.

## Future goals

Vulkan moved to "Now".

### Software fallback and WARP
Fallback when there is no DXR GPU: Microsoft's WARP adapter runs the same D3D12 + DXR 1.1 code on the CPU. First verify the current WARP build advertises raytracing tier 1.1; then `Gpu::new` falls back to `EnumWarpAdapter` when no hardware adapter has it. Slow (seconds per frame) but correct; upscalers unavailable, bicubic applies.

### Accuracy mode (owner request, 2026-10-30)
Audit the GPU path tracer's shortcuts against the CPU reference (rough glass as a lerp toward a cosine sample instead of a GGX BTDF, Russian roulette after bounce 3 and the 6-bounce default, the ray-cone mip heuristic, RGB instead of spectral) and fix the cheap ones; then an "Accurate" quality bundle (up to 64 bounces, no clamps, LOD 0 textures, no denoiser) as the high-res still default. Software mode is the CPU reference plus spectral. Measure with the convergence graph and the oracle.

### Other denoisers (owner request, low priority)
Intel OIDN 2 (best offline quality for stills, CUDA sharing with D3D12), NVIDIA OptiX (temporal mode), NVIDIA NRD (ReBLUR / ReLAX; would also fix the native filter's blotches). Compare against RR with `tools/motion_error.py`.

### Spectral rendering, part 2
Part 1 is done (path tracer, photons, dispersion, exact thin film, gratings, lens). Open: BDPT, the photon gather's camera side, LPE and the path visualizer stay RGB; exact blackbody Kelvin lights; fluorescence (black-light mode); Jakob-Hanika uplift; colour noise under RR and the native filter unmeasured; four wavelengths instead of three.

### Polarization
Mueller / Stokes transport after spectral; a rotatable polarizing filter (Brewster's-angle reflections, stress patterns in plastic). Research-grade.

### Curved space: black holes and non-Euclidean rooms
Null geodesics (Schwarzschild, RK4) in short `RayQuery` segments around a black hole in the Cornell box with an emissive accretion disk; hyperbolic or spherical rooms by ray marching. Expect the native denoiser (RR guides are approximate). Portals cover the cheap impossible rooms.

### DLSS Frame Generation and Reflex
Through Streamline (FG needs its own swapchain hooks); doubles the presented rate in interactive use, not for stills. Done after HDR, which is now in; Streamline also has a Vulkan path.

### Neural radiance cache
A tiny MLP trained online on path suffixes (NVIDIA's NRC SDK, or our own with cooperative vectors, SM 6.9); far less noise in heavy interiors (Sponza). Measure with the convergence graph and the CPU reference.

### 3D Gaussian splats
Ray-traced splats (3DGRT-style) in proxy geometry, blended in depth order inside the `Proceed()` loop; a captured object inside the path-traced Cornell box.

### VR via OpenXR
Path-traced stereo at low render resolution with RR per eye (stereo motion vectors, two RR instances, 11 ms for both eyes at 90 Hz). Realistic in classic Cornell scenes; big scenes need the neural radiance cache.

### Lightmap baker and texture painting
Bake path-traced GI into UV lightmaps (needs a second, non-overlapping UV set) and export glTF; paint on surfaces through the object-id and UV buffers, with a live render.

## Won't do

Closed by owner decision or by the triage above (the [drop] lines in "Open tails" hold the detail):

- Live wallpaper (WorkerW trick) and the Blender live link: removed 2026-11-02, not planned.
- BC7 textures (lossy, against the quality rule); any lower-quality or "performance" mode.
- A second frame in flight (measured: a few percent at most).
- SER reordering in its current form (measured 35-105% slower; a wavefront rewrite is deferred).
- Gamepad testing, CI self-test on a GPU runner, signed installer, NGX GUID change: blocked on hardware, certificates, or moot for a never-published project.
- True delta point and directional lights, light linking and gobos (by design).
- Native TAA / SMAA, SVGF blotch fix (RR is the product; NRD is the real fix), ReSTIR GI / PT, VDB files, skin-tuned subsurface.
- Every other [drop] in "Open tails".
