# Caustica: working notes for Claude

`README.md` is the user-facing doc. Paths are relative to the repo root. Read these on demand; they hold the detail that used to be here:
- `docs/claude/data-layouts.md`: material IDs, PackedMaterial, Emitter, root signature, Vertex, acceleration structures, UAV slots. **Read before changing anything Rust and HLSL share.**
- `docs/claude/architecture.md`: source map, frame pipeline, dirty tracking, rendering behaviour.
- `docs/claude/testing.md`: headless switches, self-test frame map, test modes, RR stability, measurement conventions.
- `docs/claude/setup.md`: vendor SDKs, startup defaults, machine and toolchain.
- `docs/engine-notes.md`: per-feature design notes (`grep -n '^## '`); "see X" means a section there. `docs/roadmap.md`: the ordered plan.

## Delegation (owner rule, mandatory)

The owner wants routine work on cheaper models, without wasting tokens. Every subagent starts cold and costs roughly 90k+ tokens before it does anything, so spawn only when the work is worth that. This rule is the owner's explicit request for subagents, so it overrides the default of not spawning them.

- **Inline (main session):** any command with short, predictable output: `cargo fmt` (apply it, don't just check), `cargo clippy`, a filtered `cargo test <name>`, one headless render, grep, git. Design, shaders, GPU and engine code, perf analysis and fixes also stay here.
- **`verifier` agent (haiku, `.claude/agents/verifier.md`):** long or noisy runs: release builds, the full `cargo test`, `--self-test`, benchmarks, multi-render sweeps, `tools/*.py` measurements, `tools/verify.ps1`, snapshots. **Batch:** one spawn per checkpoint carrying every step, never a spawn per follow-up. Run in the background while design work continues. To follow up on a finished run, `SendMessage` the same agent (its context is kept) instead of spawning a new one.
- **`reviewer` agent (sonnet, `.claude/agents/reviewer.md`):** every review of a finished change before a commit. Check each finding before acting on it.
- **`lookup` agent (haiku, `.claude/agents/lookup.md`):** searches and lookups whose file dumps you don't need (where is X defined or used); a couple of greps run inline.

## Project

- Caustica, a Windows-only path tracer (it began as a Cornell box): Rust 2024, D3D12 with DXR 1.1 inline `RayQuery`, HLSL compiled by DXC in build.rs, egui 0.31, C++ bridges to NGX (DLSS RR/SR), FSR and XeSS.
- Personal project, never published: license terms don't limit assets or SDK use.
- Git: `main` is the verified state. Commit and push only when the user asks. CI checks every push; only a pushed tag `vX.Y.Z` publishes a release with an installer (`.github/workflows/ci.yml`, `installer/caustica.iss`). Tag when a feature is complete or a fix warrants it (say so, and bump `Cargo.toml` to the tagged version in the same push); don't tag routine commits. The icon is `assets/caustica.ico` from `tools/make_icon.py`, embedded by build.rs with rc.exe.
- Write each new feature's notes in `docs/engine-notes.md`. Record open items in `docs/roadmap.md`.

## Build and verify

```powershell
$env:CARGO_TARGET_DIR="target/validation"   # keep checks away from target/release (the launcher's exe)
cargo fmt; cargo test --locked; cargo clippy --locked -- -D warnings; cargo build --locked
```

- Never build into `target/release` for checks and never kill the user's running app: build.rs copies the DLSS DLLs next to the exe, and a running instance locks them.
- Release is a GUI-subsystem exe with no console: run it with Start-Process and redirected output (see `tools/verify.ps1`).
- Headless: `caustica.exe --headless --frames N [--width --height --preset --scene --set key=json --rr|--no-rr --raw] --capture out.png|out.exr`; success prints `RENDER_OK …`. Benchmarks need ≥ 200 frames and `--no-vsync`.
- `--self-test`: 300 scripted frames, then one per scene file; it fails on any D3D12 warning. Add edits on unused frame numbers (map in testing.md) and assert after `render`.
- Python on Windows needs Windows paths (`cygpath -w`). Read EXRs with `tools/exr.py`.

## Must stay in sync

- `Constants` (gpu.rs) ↔ `cbuffer Frame` (common.hlsli), field for field. gui.hlsl relies on the offset of `dimensions`.
- build.rs shader entries ↔ `hotreload::ENTRIES` (same order) ↔ `Shaders` and `Gpu::reload_shaders`.
- `D3D12SDKVersion` (main.rs) ↔ the Agility SDK version `tools/setup.ps1` fetches (`Get-Agility`); the SER tracers (`*_ser`, `lib_6_9`) ↔ their compute twins (`Gpu::ser`, same order).
- A new `Settings` field needs `validate()`, the UI ranges to match it, the `transport_key` exclusions if it is display-only, and `keep_output` / `preset()` if it is a viewer choice.
- The NGX jitter offset is −ray jitter. Don't change it without rerunning `tools/rr_stability.py`.
- Sobol sampling must stay bit-identical, or convergence must be re-measured.
- One frame in flight: CPU writes to upload buffers are safe only because `submit()` waits on the fence.
