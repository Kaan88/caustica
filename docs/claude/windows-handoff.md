# Continuing from Windows

**State (2026-11-10):** `main` = tag `v0.12.0`, released from Linux with **only the AppImage** (owner's decision). Everything since v0.11.1 is in it: the CPU renderer and software view, the night sky, the walk avatar and flashlight, the UI review fixes, and six lanes merged the same day (headless no-present, OIDN for stills, egui 0.37, specular bounces and the Realtime preset, the accuracy work with exact rough glass and the Accurate preset, auto exposure with the Eyes look). `origin/cpu-playable` holds the same tree as WIP snapshots, so there is nothing to merge from it. Release notes: the v0.12.0 page on GitHub. Per-feature design: `docs/engine-notes.md` (the last section, "Merge of the six lanes and its fixes", lists what the merge itself changed).

**Not done on any Windows machine:** no build, no D3D12 run, no DXIL run, no C++ bridge build, no installer. The Linux side only proves the following:
- `tools/check_windows.sh clippy` type-checks the `cfg(windows)` Rust (D3D12 included) with clippy `-D warnings`, with zero warnings at v0.12.0.
- `tools/check_dxil.py` compiles and validates all 37 DXIL entries with the Linux DXC.

Neither links, builds `bridge/*.cpp`, or runs anything. **Rule: no Windows installer or zip is uploaded until this checklist passes** (owner, 2026-11-10; the Linux-only v0.12.0 was the one exception). Tick an item by writing the date and the result next to it. Line numbers drift: `grep` the quoted name.

**2026-11-11, by the owner on Windows: install over 0.11.1, the update prompt with its UAC relaunch, and uninstall all worked (v0.12.0 setup).** After the uninstall no file was left in `%LOCALAPPDATA%\Programs\Caustica` (no DLLs), only the empty folders `scenes	humbnails`; `caustica.iss` now removes them when empty (not yet in a built setup). **Installed Windows copies are offered v0.12.0 now, and it has no setup exe for them.** `update::parse_release` finds no `caustica-v*-setup.exe`, so the asset is `None`. From the code, the window then offers "Open release page" instead of Update (`can_update` in `src/ui/windows.rs`). First check on Windows: an installed 0.11.1 copy shows that, and "Skip this version" works.

## First session on Windows, in order
1. `git fetch origin`, check out `main` (v0.12.0). Run `tools/setup.ps1` (fetches the SDKs, now including OIDN; see the OIDN items). The fetched `scenes/{models,hdri,textures}` and `vendor/` are not in git.
2. Build and check as `CLAUDE.md` says (`$env:CARGO_TARGET_DIR="target/validation"`; fmt, test, clippy, build), then `--self-test --debug` (it fails on any D3D12 warning). Expect 484 frames. (2026-11-11: passed, no D3D12 warnings.)
3. `tools/verify.ps1` (2026-11-11: PASS). Then `tools/rr_stability.py`, twice: as it stands (exposure pinned, P = 1), and once with the exposure at 2^20 (see "Exposure").
4. `python tools/oracle.py --exe target\validation\release\caustica.exe` (default list). Linux/Vulkan gives every scene ok, gem-display the loosest at 1.0127. Then the extra runs in "Accuracy" and "Spectral".
5. By hand, what hidden runs can't exercise: Tab and shortcuts, typing into sliders, undo of panel edits, the restore-last-session prompt, the flashlight row, the CPU view, the Renderer menu, auto exposure while walking from a bright room into a dark one, the Realtime and Accurate presets, and the Still denoiser row.
6. Re-baseline the D3D12 numbers: headless frame times change (no Present), the meter adds a pass and a readback per frame, and the trace loop bound is dynamic. Record `gpu_ms_mean`, `trace_ms_mean`, `frame_ms_mean` and `fence_wait_ms_mean` (`--no-vsync`, at least 200 frames) for v0.11.1 and v0.12.0 on the same scenes.
7. The release (last section).

## Checklist

### Build and ABI
- [x] 2026-11-11 Windows, RTX: test 399 ok, clippy clean, release build with no DXC warnings. `cargo build --locked` and `cargo clippy --locked -- -D warnings` on Windows. This is the first build of:
  - egui 0.37 and `egui-winit` 0.37 (`windows-sys 0.61` beside the project's `windows 0.62`)
  - the C++ bridges with `pre_exposure`
  - OIDN in `build.rs`
- [x] 2026-11-11 Windows, RTX: read both sides, order matches (NGX, NGX SR, FSR, NGX VK). **Bridge ABI: `pre_exposure` on every upscaler eval.**
  - C++ side: `bridge/ngx.cpp` `caustica_ngx_eval` (RR, `InPreExposure`) and `caustica_ngx_eval_sr`; `bridge/fsr.cpp` (`d.preExposure`); XeSS has no input.
  - Rust side: the declarations in `src/gpu.rs`.
  - The parameter order must match on both sides. `dt` and `pre_exposure` are both `f32`, so a swap still links and only shows in the image.
- [x] 2026-11-11 Windows, RTX: builds; CB_SLOTS = 27. `Constants` stays under `CB_STRIDE` (a `const` assert; it fails the build). `CB_SLOTS` is 27: glare 13-22, meter 23, pre-exposure 24-26. Nothing else should index past 22 (`grep CB_SLOTS`).
- [x] 2026-11-11 Windows, RTX: fields present, builds. **D3D12 `GpuState` initializer** (`src/gpu/init.rs`, `st: GpuState {`): the merge added `meter: Default::default()`, `metered: None`, `pre_exposure: 0.` by hand. Vulkan uses `..Default::default()`. Confirm it matches.

### Exposure (meter, Eyes look, pre-exposure)
Each frame on D3D12 now gets a new compute pass, a readback, and in-place scaling of the reconstruction inputs.
- [~] 2026-11-11 Windows, RTX: RR night (light 1.2e-6, auto): mean P/P=1 = 0.99997, within 2%; SR, FSR and XeSS not tried. **What NGX and FSR do with the pre-exposure.**
  - The app multiplies the colour input by P (a power of two), passes P to the SDK, and divides the output by P (`pre_expose`, `src/gpu/frame.rs`).
  - Assumption from the SDK headers, unproven: the output comes back pre-exposed.
  - Check with RR, DLSS SR, FSR and XeSS on a night scene: self-test frame 245, or `--preset 0 --set light_intensity=1.2e-6 --set auto_exposure=true`.
  - Compare `CAUSTICA_PRE_EXPOSURE=0` (P = 1) against the default. The mean luminance of the two EXRs should agree within 2%, with no banding or fp16 clipping.
  - XeSS gets the scaled colour with `exposureScale=1`.
- [~] 2026-11-11 Windows, RTX: `--self-test --debug` clean. Daylight auto is not bit-identical to `CAUSTICA_PRE_EXPOSURE=0` (mean 0.99965) although a rerun with the same settings is byte-identical: daylight auto meters exposure 3.86, so P = 4, not 1, and RR is not scale-invariant. Bit-identity holds only where P = 1; the expectation below was wrong, not the code. **RR path scales in place:**
  - Calls to check: `pre_expose(&c, 0, false)` before the RR transitions, P passed to `caustica_ngx_eval`, and `pre_expose(&c, 0, true)` after.
  - `--self-test --debug` must show no state or barrier complaints on textures 0 and 14.
  - Every later reader must see scene units: display, meter, scopes, EXR and `read_floats(18)`.
  - A daylight EXR with auto exposure must be bit-identical to one with `CAUSTICA_PRE_EXPOSURE=0`.
- [ ] **Upscaler path:** `run_upscaler` scales before and restores after. Texture 14 must be in UAV state for the restore. FSR, XeSS and DLSS SR must match `CAUSTICA_PRE_EXPOSURE=0` in daylight.
- [ ] **u28 (scopes) is now 256 × 262.** Rows 260 and 261 belong to the meter: histogram, group counter and state.
  - `clear_scopes` (`src/gpu/device.rs`) copies a box over the scopes rows only.
  - The readback buffer grew 256 bytes, and the state copy is the box `[8, 261, 16, 262]` at `probe_albedo + 1024`.
  - Check `--self-test --debug` for copy-footprint and box errors.
  - Turn the scopes on (`--ui-scopes`) while auto exposure runs: the scopes must draw and the meter must keep adapting.
- [ ] **Resize:** `resize_output` now sets `st.metered = None`, because u28 is recreated empty. After resizing the window, auto exposure must show no frame at the manual exposure.
- [ ] **Readback every frame:** `queue_readbacks` now runs unconditionally. Measure `frame_ms_mean` and `fence_wait_ms_mean` against v0.11.1; under 0.1 ms is expected.
- [ ] **Meter pass (DXIL):** `display` dispatched again with `meter_ctrl.x` bit 1, plus `meter_pass` and `meter_update` in `shaders/post.hlsl`.
  - It uses groupshared memory, `InterlockedAdd` and `InterlockedExchange` on `RWTexture2D<uint> scopes`, and a last-group-reduces pattern without `globallycoherent`.
  - Run it on NVIDIA, and on AMD or Intel if one is at hand. Watch for a meter that flickers between frames.
  - Self-test frame 240 asserts middle grey within 0.15 stop against the CPU histogram.
- [ ] **Scale pass (DXIL):** `scale_pass` (`meter_ctrl.x` bit 3) works through a `RWTexture2D<float4>` passed as a function parameter. Texture 14 is dispatched at output size, the other textures at render size.
- [ ] **Glare after the meter** (moved in `frame.rs` and `vulkan/gpu.rs`): glare now uses the same frame's exposure. Check a glare scene with auto exposure: street-bokeh, or stage-beams.
- [ ] **HDR self-check:** `check_hdr_output` now uses the meter's exposure. Check it on the scRGB and HDR10 swap chains.
- [x] 2026-11-11 Windows, RTX: self-test passes on D3D12. **Self-test frames 240, 245 and 246.** Frame 246 restores preset 0 after 245's dark scene. Every other frame pins auto exposure and Eyes off. Check that they pass on D3D12.
- [ ] **Video export with auto exposure:** H.264 and a PNG sequence, from the UI and from `--turntable`, which now sets `g.meter = job.meter_step()`. The first frame settles and later frames adapt smoothly. EXR stays in scene units.
- [x] 2026-11-11 Windows, RTX: as is 0.00695 / 0.000156; at exposure 2^20 (scratch copy of its scene, emission scaled 2^-20; P not confirmed active) 0.00708 / 0.000255. The script has no --set option. **`tools/rr_stability.py`** pins `auto_exposure=false`, `eyes=false` and `exposure=1`, so P = 1 and pre-exposure is not exercised.
  - Thresholds: edge temporal std < 0.02, mean frame difference < 0.0015. The last run gave 0.0071 and 0.00017.
  - Add a run at exposure 2^20 (`--set exposure=1048576`, with the light and background scaled down by 2^20) for P ≠ 1.
- [ ] **`--vulkan` on Windows with RR at night:** `pre_exposure` through `bridge/vk.cpp` (`ngxvk`).
- [ ] **Remote page and gamepad:** the remote's exposure slider and auto switch, ev_bias at ±16, and the d-pad stepping ev_bias in auto mode.

### Accuracy (exact rough glass, Accurate preset, ray origin)
- [ ] **Six new DXIL entries** (37 in all):
  - `trace_exact`, `trace_exact_ser`, `trace_spectral_exact`, `trace_spectral_exact_ser`, `bdpt_exact`, `bdpt_spectral_exact`
  - SER twins at `Gpu::ser` 7 and 8. The order must match the SER tracers.
  - `build.rs` entries ↔ `hotreload::ENTRIES` ↔ `Shaders` ↔ `Gpu::reload_shaders` must match one to one.
  - Check that `exact_glass=true` renders with and without SER, and that hot reload still works.
- [x] 2026-11-11 Windows, RTX: glass-of-water 1.0009, gem-display 0.9977 (worst block 1.0251), both ok. Default list all ok, worst mean veach-mis 1.0018. **Oracle with exact glass on D3D12:** `oracle.py --scenes glass-of-water,gem-display --set exact_glass=true`. Linux gives 1.0006 and 1.0127.
- [ ] **Ray origin on the +ng side for non-delta lobes** (`origin=h.p+h.ng*(!delta || ...)` in `trace_main`): this changes images near the terminator. The SER and non-SER tracers must give the same image, and the oracle must stay within its limits.
- [ ] **Accurate preset:** choose it, then switch to every other preset and back. The name must follow each time, and Realtime's `specular_bounces` must never ride along (`realtime_then_accurate_drops_the_specular_budget` covers the logic).
- [ ] `oracle.py` gained `--cpu-truth`, `--cpu-set` and `--gpu-set`. Run it once on Windows to prove the script.

### Bounces (specular budget, Realtime preset)
- [ ] **Dynamic loop bound** (`bounce<sampling.z+spec_used`, `bounce_split`):
  - Watch the DXC output for unroll warnings or register-pressure changes.
  - With the default `specular_bounces = 0`, images must be bit-identical to v0.11.1. Compare EXRs of presets 0, 3, 8 and 21, and compare `trace_ms_mean`.
- [ ] **Glass vertices count against the budget only at roughness ≤ 0.1**, in both the lerp and the exact model (`lobe>=3 ? m.color_rough.a<=0.1 : ...`). Check Realtime on a rough-glass scene.
- [ ] **Realtime on a DLSS machine:**
  - RR runs (`--require-rr`), and the image is stable: `rr_stability.py` with spp 1, bounces 3, specular_bounces 8.
  - With `--no-rr` it falls back to the native filter and still reads as Realtime.
  - Measure its frame time against Best on the glass scenes.

### Spectral
- [~] 2026-11-11 Windows, RTX: 320x180, 64 frames: BDPT/PT 1.0049, photon/PT 1.0203 (Linux 0.995 / 1.008); the 3x is gone, the photon 2% needs more samples before reading anything into it. **BDPT and photon mapping with a dispersive lens were 3× too bright; fixed.**
  - GPU: `camera_ray` uses `hero_only ? (1,0,0) : (3,0,0)`, and `photon_gather` now calls `begin_hero` before `camera_ray`.
  - CPU: `progressive.rs` sets `waves.hero = waves.on` before the camera ray when the sample goes to BDPT or photon mapping.
  - Linux, street-bokeh, 320×180, 64 frames: BDPT/PT 0.995 and photon/PT 1.008 with spectral on, against 2.990 and 2.985 before the fix.
  - Check on D3D12: `--set integrator=N --set spectral=true --scene street-bokeh --no-rr --raw --capture x.exr` for N = 0, 1, 2. Compare the means.
- [ ] **Known, open (predates the merge):** GPU spectral photon mapping reads about 8% high on street-bokeh against the CPU path tracer: 1.085, z 3.0, even with `dispersion=0`. Details in `docs/plan-big-features.md`. Check whether D3D12 shows the same.

### Headless (no Present)
- [ ] **`Present` is skipped when `headless`** (`src/gpu/frame.rs`). This covers `--headless`, `--make-thumbnails`, `--turntable` and the self-test.
  - The back buffer stays in PRESENT state, so `--self-test --debug` must be clean.
  - `--capture-ui` and the other capture paths must equal v0.11.1.
  - HDR headless captures must be unchanged.
  - The swap chain has no waitable object (`grep -i waitable src/gpu` is empty). Keep that true.
- [ ] **The back buffer index never advances in headless runs.** Nothing indexes per back buffer today; check that the exposure code doesn't.
- [~] 2026-11-11 Windows, RTX: `--vulkan --self-test` 484 frames ok (no validation layer installed). Vulkan twin on Windows: `--vulkan --headless --frames 8 --capture a.png` and `--vulkan --self-test`.

### egui 0.37
- [ ] **Font atlas is a colour image** (`update_gui_textures`: the `ImageData::Font` arm is gone, and the per-delta upload is now a nested loop).
  - Check text weight and brightness in SDR, scRGB and HDR10 (`shaders/gui.hlsl` assumes premultiplied sRGB).
  - A growing atlas must recreate the texture: show CJK or accented scene names and look for tofu.
- [ ] **Translucency changed in 0.32:** popups, shadows and selections are slightly darker. Check in all three output modes by eye.
- [ ] **Windows-only egui code:**
  - The `renderer_menu` in `src/ui/chrome.rs` is a plain `menu_button`. In 0.37 a click inside closes it, so check the D3D12/Vulkan switch, "Restart now" and "Retry Vulkan now".
  - `show_in_folder`.
- [ ] **Menus that hold checkboxes are `sticky_menu`:** the top-bar Menu, LPE Presets, Levels and "+ Add" (its Z-up box). Check that ticking keeps them open.
- [ ] **A/B compare split handle is `Order::Middle`:** dragging it must not orbit the camera.
- [ ] **The meter hold uses `egui_is_using_pointer`:** dragging a light or material slider holds auto exposure ("Exposure held").
- [ ] **Window behaviour:** clipboard (text fields and the Rhai console), IME, key release on focus loss (Alt-Tab while holding W), and DPI at 100, 125 and 150% on two monitors.
  - The side panel width must still save. The resize id `Id::new("side panel").with("__resize")` has to match egui 0.37's.
  - The top bar must wrap at 1280, 1920 and 2560.
- [ ] Re-baseline the UI captures (`--capture-ui`) and the docs images.

### OIDN (still denoiser)
- [ ] **`tools/setup.ps1` `Get-Oidn`:**
  - It downloads `oidn-2.5.1.x64.windows.zip` and checks SHA256 `f11f91bc072a5e3a564515724cb72ab8fcfbc445c84a84c197ff9b16cc01396f`, which hasn't been verified on Windows.
  - It expands to `vendor/oidn/windows/{bin,lib,include}`.
  - A second run is a no-op (`VERSION-win-2.5.1`).
  - The script is CRLF.
- [ ] **`build.rs` `oidn_runtime`:**
  - The four DLLs (`OpenImageDenoise.dll`, `_core`, `_device_cpu`, `tbb12.dll`) land beside the exe, including with `CARGO_TARGET_DIR=target/validation`.
  - A rebuild while the app runs does not fail.
  - A check-only build skips the copy.
- [~] 2026-11-11 Windows, RTX: `--oidn` capture denoised (307 ms, albedo + normal guides), no `OIDN skipped`; renamed-DLL case not tried. **`src/oidn.rs` loading:**
  - `--headless --frames 64 --oidn --capture a.png` comes out cleaner than without `--oidn`, with no `OIDN skipped` in the log.
  - With one DLL renamed, the Render tab row shows the reason and the capture is saved raw.
- [ ] **Guides on D3D12:** `save_still` and `denoise_capture` read albedo and normal via `Gpu::read_layers`. A failure denoises unguided with no message, so check with and without RR that texture and edges survive.
- [ ] **CPU view with OIDN:**
  - A pause mid-render shows the filtered image.
  - CPU stills are metered on the unfiltered mean (`write_cpu_image_exposed`).
  - The UI must not stutter while oneTBB runs.
- [~] 2026-11-11 Windows, RTX: `release.ps1 -Version 0.12.0` (no upload) builds setup.exe 735 MB (v0.10.1: 674) and zip 920 MB (853); the staged folder has the four DLLs and the three `licenses\OIDN-*.txt`; the unzipped copy run outside the repo renders `--oidn` (340 ms, guided). `caustica.iss` installs `{#SourceDir}\*` and uninstalls what it installed. Install, uninstall and the update over 0.11.1 not tried. Fixed on the way: `make_installer.ps1` ran the GUI exe with `&`, which doesn't wait, so the thumbnail check raced the renders; it now waits on the process alone (`Start-Process -Wait` hung on the driver's `nvngx_update.exe` children, see below). **Packages:**
  - `make_installer.ps1` stages the DLLs and `licenses\OIDN-*.txt`.
  - The installed app has them beside the exe, and uninstall removes them.
  - An in-place update over 0.11.1 replaces them.
  - The unzipped zip runs `--headless --oidn` with no `vendor/` folder.
  - Note in the release notes how much the installer grew.

### Pipeline cache (Vulkan)
- [ ] **`--vulkan` on NVIDIA Windows:**
  - The first start writes one `vulkan-pipelines-<hash>.bin` under `dirs::cache()`, and the second start takes about 0.5 s.
  - A changed shader or a driver update rewrites the file.
  - No stray `.tmp` is left after a crash or with two instances.
  - At most 3 files are kept.
- [x] 2026-11-11 Windows, RTX: 188 thumbnails, no cache file. **`--thumbnails-out` (the installer's thumbnail step) writes no cache file** (`dirs::cache_overridden`). Check that the bundled `thumbnails` folder has no `vulkan-pipelines-*.bin`.

## NGX updater pile-up (found 2026-11-11)
Each DLSS init makes the driver start `nvngx_update.exe -feature dlssd -bootstrap` (from `DriverStore\...
v_dispi.inf_*`), and in a long session of headless runs many of them never exit: 2963 after a day of self-tests, oracle and thumbnail runs, about 6 per run during the 188 thumbnail renders, each with a `conhost.exe`. A single run left none. The SDK headers have no switch for it (it follows `Init_with_ProjectID`). Effects: the machine slows down, and any wait on a process tree (`Start-Process -Wait`) never returns. After heavy runs: `Get-Process nvngx_update | Stop-Process -Force`. Open: whether a normal UI session leaves any, and whether `NVSDK_NGX_ENGINE_TYPE_CUSTOM` without a project ID or a driver update avoids it.

## The Windows release
v0.12.0 exists, holding only the AppImage and a `SHA256SUMS` that lists only the AppImage. `tools/release.ps1 -Upload` runs `gh release create`, which fails on an existing tag. Two ways:
- **Preferred: v0.12.1 with the fixes the checklist needs.**
  1. Bump `Cargo.toml` and `Cargo.lock`, commit to `main`, tag `v0.12.1`, push.
  2. Run `tools/release.ps1 -Version 0.12.1 -AppImage -Upload`. This builds the installer and zip, builds the AppImage on the Linux VM, writes `SHA256SUMS` over all of them, and creates the release.
  3. Write notes by hand (the script lists commit subjects) and edit the release.
- **Add Windows assets to v0.12.0** (only if nothing changed in code):
  1. `tools/release.ps1 -Version 0.12.0` without `-Upload` (builds `dist\`).
  2. `gh release download v0.12.0 -p "Caustica-*.AppImage" -D dist`.
  3. Write `SHA256SUMS` over the installer, the zip and the AppImage, in `sha256sum` format, LF, no BOM, as `release.ps1` does.
  4. `gh release upload v0.12.0 <setup.exe> <zip> dist\SHA256SUMS --clobber`.

Self-update refuses an update whose `SHA256SUMS` lacks its file.

## Next work after Windows
`docs/plan-big-features.md` holds the plan:
- "Backlog from the plan review", in batches of at most 3 non-overlapping lanes
- redesign B (`docs/ui-redesign-b.md`), including where the new controls go
- the hash-grid cache with the Realtime preset

`docs/ui-review.md` has one row partly open: R does different things depending on whether something is selected.

**CPU parity:** stages 0-8 of `docs/claude/cpu-parity-audit.json` are done, and `reference::unsupported` refuses nothing. Stage 9's display transform and exports are done (engine-notes "Display transform and exports with no device"). Still to prove on Windows: the CPU view as the fallback on a machine with no ray-tracing GPU (roadmap).

## Heavy runs
A run on Linux froze the desktop on 2026-11-06, cause not proven: 14 GB of RAM-backed `/tmp` plus parallel builds and GPU renders. Keep a few cores free, run one GPU process at a time at low resolution, and keep temporary files on disk. On Linux, `target/safe.sh` takes a build lock (cargo) or a GPU lock and checks free memory.
