## Scenes buildable today (roadmap 5e, 2026-10-30)

- `Settings.mis_mode` (0 MIS, 1 NEE only, 2 BSDF only) → `scene_info.w` bits 4–5. In the trace, mode 1 gives BSDF-found emission weight 0 (unless it came after a delta bounce) and NEE divides by `pl` alone; mode 2 skips NEE. Checked on veach-mis: the classic three images.
- `ui::seed_of_the_day()` (days since epoch × golden ratio) feeds `randomize` from the top bar's *Today* button.
- New generator scenes in `tools/build_scenes.py` use `room_shell()` (walls / ceiling / front as thin boxes) and `obj(portal=…)`:
  - `kusama-infinity-room`: 250 objects, emission 1.6, exposure 0.8; it blew out at emission 9.
  - `flavin-fluorescent` and `turrell-ganzfeld`: rounded seams are 1.6-scale cylinders in every corner and along the edges.
  - `infinite-corridor`: the end portals are paired, the near one rotated 180°.
  - `bigger-on-the-inside`: the box door is paired with the hall doorway at x = 30, which **must be rotated 180°** or the view leads back out of the hall.

