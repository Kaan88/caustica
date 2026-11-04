# Build, run, test and measure

Moved out of `CLAUDE.md` on 2026-11-02 (unchanged text). Paths are relative to the repo root. Where this file says "see X", X is a section of `docs/engine-notes.md`.

## Build, run, verify

```powershell
$env:CARGO_TARGET_DIR="target/validation"   # keep checks away from target/release (the launcher's exe)
cargo fmt --check; cargo test --locked; cargo clippy --locked -- -D warnings; cargo build --locked
```

- `launch.cmd` starts `target/release/caustica.exe` and only builds it if it is missing. It never rebuilds a stale exe.
- Release builds use `lto = "thin"` and `codegen-units = 1`, so they are slow.
- **Release is a GUI-subsystem exe with no console output.** Debug builds have a console.
  - `tools/verify.ps1` runs release via `Start-Process` with stdout and stderr redirected.
- `build.rs` does the following:
  - Asserts that `vendor/dlss`, `vendor/fsr` and `vendor/xess` exist.
  - Compiles 14 shader entries: `trace`, `restir_presample`, `restir_initial`, `restir_spatial`, `filter`, `display` and `glare` (entry `glare_pass`, track 16) as cs_6_5; `vs_main` and `ps_main` as 6_0. `trace` is built four times: plain, `trace_maps` (`-D TEXTURE_MAPS`), `trace_paths` (plus `-D PATH_RECORD`, the path visualizer) and `trace_lpe` (plus `-D LPE`, light path expressions). The three ReSTIR DI passes come from the same file with `-D TEXTURE_MAPS`. Track 15's integrators come from `integrators.hlsl` (which includes pathtrace.hlsl), also with `-D TEXTURE_MAPS`: `bdpt`, `bdpt_resolve`, `photon_clear`, `photon_trace`, `photon_gather`.
  - Compiles the bridge and links `nvsdk_ngx_d`.
  - **Copies `nvngx_dlss.dll` and `nvngx_dlssd.dll` next to the exe.** A running instance locks them and the build fails. Use a separate `CARGO_TARGET_DIR`, and never kill the user's running app.
- Headless runs need a real GPU and use a hidden window:
  - Command: `caustica.exe --headless --frames N [--width W --height H --scale s --spp n --samples cap --preset 0..23 --settings f.json --scene name --rr|--no-rr|--require-rr --raw --debug] --capture out.png|out.exr`.
  - Success prints `RENDER_OK frames= samples= gpu_ms= rr= size= gpu_ms_mean= frame_ms_mean= fence_wait_ms_mean= trace_ms_mean= reconstruct_ms_mean= display_ms_mean= ui_ms_mean=` (reconstruct is RR, or the native filter with `--no-rr`). `tools/perf_baseline.ps1` is the track 20a sweep. The means are taken from frame 10 on, so use ≥ 200 frames with `--no-vsync` for benchmarks.
  - `--preset`/`--settings` (only one) is applied before all other switches, so order doesn't matter.
    - **Old builds (including `main` before 2026-10-29's fixes) applied them in order**, so `--raw` and `--spp` given before `--preset` were silently reset. Put `--preset` first when benchmarking an old exe.
- Other test modes:
  - `--self-test`: 297 scripted frames (`SCENES_FROM`), then one frame per scene file (193 files in the built-in, generated, mesh, environment and materials packs, 488 frames in all); implies headless and `--debug`. It fails on any D3D12 warning.
    - Frames 10–125 cycle all 24 presets; frames 130–140 make live edits (lights, all shapes, emission, hide, delete, pick, frame, randomize); 285–287 run BDPT, photon mapping and battle mode on preset 3 and check the image and every quadrant. 286–289 run with `spectral` on (photon mapping through glass with dispersion, battle mode, the Double Gauss lens with dispersion); 290 turns it off again. 292–293 grab a ball with physics on (a gizmo handle under the pointer wins). 294–295 run BDPT spectral, then RGB (the spectral twin once hung the RGB one after it); 296 returns to the path tracer. 248–251 walk with the flashlight on and a hidden light that casts no shadows.
    - Every frame it asserts that the GPU triangle and emitter counts match the settings.
    - The script lives in `redraw()`. Add new edits on unused frame numbers and assert after `render`.
  - `--debug`: needs the Windows Graphics Tools.
  - `--orbit-test`: moves the camera 0.25° per frame after frame 64.
  - `--capture-sequence dir`: captures the last 32 frames.
- Unbiased reference for noise measurement: `--no-rr --raw --samples N --frames N/spp --capture ref.exr`. Linear RGB32F at output resolution.
- `tools/rr_stability.py` is an RR shimmer regression. It uses emission-only walls and black emission-only blocks, so there is no Monte Carlo noise.
  - Run `python tools/rr_stability.py --exe <path>` to use a build other than `target/release`.
  - Thresholds: edge temporal std < 0.02 and mean frame difference < 0.0015.
  - Last result, 2026-10-29: 0.0071 and 0.00017. The 2026-10-26 run is kept as `captures/rr-stability/regression/metrics-2026-10-26.json`.
- `--set key=json` overrides one top-level setting after `--preset` or `--settings`. Unknown keys are rejected. Example: `--set direct=0 --set indirect=1`.
- Python on Windows needs Windows paths. Use `cygpath -w` when calling it from the Bash tool.

## Measurement conventions

- Error metric: RMSE ×1000 on compressed luminance l/(1+l) against a high-spp reference. The reference used 16,384 spp.
- Also track a 5×5-blurred error, which catches ghosting that plain RMSE hides.
- Compare against an **independent** reference: a white-noise reference when testing Sobol.
- Use numpy on EXR captures. imageio has no EXR backend here, and OpenEXR and cv2 are not installed.
  - The exe writes **tiled (64×64), RLE-compressed** RGB float EXRs.
  - Use `tools/exr.py`: `exr.read(path)` returns an H×W×3 float32 array. It handles scanline or tiled files with NONE or RLE compression.
  - It was used to confirm that direct + indirect recompose the beauty to 2e-7 relative error (presets 0, 3, 8, 21; raw native path).
- Report in a table, with the sample count where each method reaches equal error.
