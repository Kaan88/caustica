# Continuing from Windows

Branch `cpu-playable`. Snapshot of work in progress, unreviewed for the CPU renderer and CPU mode, not `main`: `main` stays at the last verified state until this has run on real Windows.

## In this branch
- Flashlight beside the camera, with a beam profile (`scenes/ies/flashlight.ies`) and settings in its outliner row.
- Achievements reworked (`src/stats.rs`).
- 24 of the 42 UI review findings fixed; `docs/ui-review.md` lists the 18 still open.
- CPU renderer split into `src/reference/`, with its refusal holes closed.
- CPU mode (`--cpu`, and a switch in the UI): written, but the work stopped during its first test on 2026-11-07, before its independent check. Start here: run it, finish what is missing against the "playable" brief below, and have it checked.

The playable brief: a clear switch that shows the CPU render full view through the normal display pass, the camera and edits restarting it with a fast preview while moving, samples and time in the status bar, saving with the normal buttons, a plain message listing what the scene uses that the CPU can't render yet, and CPU threads that don't starve the UI.

## First steps on Windows
1. `git fetch origin`, `git switch cpu-playable`. `vendor` and the fetched `scenes` folders are not in git; the checkout's own copies are used.
2. Build and check as `CLAUDE.md` says (`target/validation`), then run `--self-test`. **None of this has run on real Windows or D3D12 yet.**
3. Try by hand what hidden runs can't exercise: Tab and other shortcuts, typing into sliders, undo of panel edits, the restore-last-session prompt, the flashlight row, and the CPU mode.

## Remaining CPU parity work
`docs/claude/cpu-parity-audit.json` holds a spec per feature (`groups[].items[]`, `critic.missed`, `critic.order`); its line numbers have drifted. Done: stages 0 and 1. Next, in order:
2. Foundations: hit frame, two-level BVH, emitter semantics, MisMode, camera dispatcher.
3. Firefly clamp, projections, depth of field, sphere and sun lights, IES, flashlight, portals, splitters, real lenses.
4. Textures and maps, coat, sheen, anisotropy, thin film, gratings.
5. HDRI, physical sky, shadow catcher.
6. Fog, object media, nested dielectrics; SDF objects.
7. Spectral.
8. AOVs and LPE, BDPT, photon mapping, battle mode.
9. Export, denoiser, thumbnails, no-ray-tracing machines.
Each stage: remove the refusal in `reference::unsupported`, measure with `tools/oracle.py`, add a line to engine-notes "CPU reference renderer". The owner's decision on coplanar dielectric contacts is still open.

## Heavy runs
A run on Linux froze the desktop on 2026-11-06, cause not proven: 14 GB of RAM-backed `/tmp` plus parallel builds and GPU renders. Keep a few cores free, run one GPU process at a time, at low resolution, and keep temporary files on disk.
