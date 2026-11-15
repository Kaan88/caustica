## Tooling (roadmap 5a, 2026-10-30)

- `tools/perf_history.py` appends to `captures/perf-history.csv` (date, commit, `+` if src/shaders/bridge are dirty, preset, mode, means) and plots the PNG with matplotlib. First row, commit 3c4c8a4+ (GPU ms, RR / native): Classic 3.81 / 3.13, bunny 3.96 / 3.90, mirror corridor 5.98 / 5.71, neon 4.52 / 4.27.
- **Build snapshots:** `tools/snapshot_build.ps1 -Name N -Notes T` copies exe + DLLs + scene packs (not thumbnails) into `builds/<date>-N/` with NOTES.md (gitignored). `scenes/models` and `scenes/hdri` are hard-linked, not copied (about 800 MB of pinned downloads). The owner asked for one per major item, so they can compare builds later. The exe finds `scenes/` next to itself.

