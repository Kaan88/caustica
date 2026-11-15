## Generators (core track 1, 2026-10-30)

- `generators.rs`: `generate(family, seed) -> Settings` for the 17 `FAMILIES` (the last four use procedural meshes, see "Mesh import"). RNG is xorshift64* seeded by seed and a hash of the family.
  - Helpers reuse `presets::{object, light, studio, material}` (now `pub(crate)`). `at()` places a mesh centre at a world y. `sun()` is a far quad aimed at the origin (warm 350, day 450, sky 0.2 / 0.55; brighter washed out). `frame()` is `frame_objects` × 0.78.
  - `finish()` names the scene "<family> · seed N", truncates to `MAX_OBJECTS` and runs `fit()` (excess stretch/scale into size, angles wrapped, as build_scenes.py does).
- New shape **Tetrahedron** (last in `Shape::ALL`; the mesh cache is indexed by enum order): base at y = −1, apex at +1, base circumradius √2.
- Menger level 2 merges filled cells into runs along x (fits under 256). The Sierpinski child centre is c + corner·scale/2. The sphereflake uses Haines' 6 + 3 directions, rotated per parent.
- Scene files with `"generator": {"family", "seed"}` regenerate in `scenes::read` (the title overrides the name). `scenes::save_generated` writes such a file to scenes/user. `tools/build_scenes.py` also writes `scenes/generated` (6 curated seeds × 15 families plus 3 each for forest and crowd; `GENERATED` table).
- Tests: every family × 24 seeds validates and is non-empty; generation is deterministic; Menger fits. The self-test sweeps every scene file, now 164 (437 frames).
- `tools/scene_sheet.py --pack generated` renders the pack. All 90 were checked; day cities and warm suns were dimmed, framing tightened.

