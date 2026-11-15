## Gizmos (roadmap item 3b, 2026-10-30)

- **State** in `UiState`: `tool` (Select / Move / Rotate / Scale, default Move), `local` (Global / Local), `hover` (handle under the pointer, from `overlay`), `transform` (running), `header`, and `undo` / `redo` stacks of `Settings` (≤ 100). `checkpoint` is called before deletes, adds, duplicates and at the end of a transform (with the original); `checkpoint_scene` before anything that replaces the scene. See "Undo and the last session".
- **Input** (`main.rs` `window_event`):
  - Left press on a hovered handle starts a handle transform (no orbit); release finishes it.
  - G / R / S start a modal transform when an object or light is selected and the pointer is over the viewport. R therefore only restarts when nothing is selected.
  - During a modal transform: left click confirms, right click cancels; keys go to `App::transform_key` (X / Y / Z, Shift for planes, digits, `.`, `-` toggles sign, Backspace, Enter, Esc).
  - Pointer deltas accumulate into `Transform::mouse`, ×0.1 with Shift. Ctrl snaps (0.1 units, 15°, 0.1×).
  - A short click picks objects and light quads (`gizmo::pick`); empty space deselects.
  - Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z, undo and redo.
- **Math:** an axis move uses the axis's on-screen direction (so the pivot follows the pointer, checked to 3 px); plane and free moves intersect pointer rays with the plane through the pivot; rotation is the screen angle around the projected pivot, with the sign flipped for axes facing away; scale is the ratio of pointer distances from the pivot. Rotations compose as quaternions and go back to YXZ Euler (objects: yaw = `rotation`, `tilt` = pitch / roll; area lights: `rotation` = [pitch, yaw, roll]). Per-axis scale uses local axes: X `scale_xz[0]`, Y `stretch`, Z `scale_xz[1]`, free = `size`. Main light: X scales width keeping depth, Z changes `light_aspect`; no rotation.
- **Drag cost** (release, 1440×900, `--drag-test`, frame ms): Classic 4.6 → 6.6, bunny 4.7 → 20, chess 4.9 → 22.
  - `geometry()` rebuilt every object (about 11 ms for 600k vertices), so it now caches each object's mesh per slot (`OBJECT_CACHE`, keyed by the `Object` and its material id). That saved about 4 ms.
  - The rest was new upload, BLAS and scratch allocations, plus the whole-scene BLAS build. Instancing (core track 4) fixed it: drags now rebuild only the TLAS (see "Instancing").
- Capture CLI: `--ui-tool`. Self-test 198–202: each tool drawn on object 0 and the main light, then a keyed G X 0.5 Enter (asserts +0.5 and that the transform ended), then undo (asserts the move is gone).
- Not done: see the list under roadmap item 3b. Object snapping, pivots, multi-select, Alt-drag and vector entry came on 2026-11-02 ("Multi-select, pivots and snapping"); cheap drags came with instancing.

