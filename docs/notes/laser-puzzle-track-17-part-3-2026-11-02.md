## Laser puzzle (track 17 part 3, 2026-11-02)

Removed 2026-11-15 (roadmap Plan step 3).

- `src/laser.rs`, `Settings::puzzle` (`enabled`, `origin`, `direction`, `target` object index or −1, `par`, `color`; out of `transport_key`, omitted at defaults). Scene → *Laser puzzle* edits it and shows "Solved / Not solved · N moves · par P".
- The beam is traced on the CPU, not rendered: geometric optics on every visible object's world triangles (Möller–Trumbore, interpolated vertex normals). Mirrors (metallic > 0.5, roughness < 0.2) reflect; beam splitters fork it (front faces, power × reflectivity and × 1 − reflectivity; back faces pass); materials with transmission > 0.5 refract by Snell with total internal reflection (no Fresnel split); anything else stops it. At most 64 segments; branches below 1% power end. Retraced only when the objects or the puzzle change (a JSON key).
- Drawn as viewport lines over the image (with the gizmo layer, perspective cameras only); width and opacity follow √power. The roadmap's GPU "sensor material" was not needed: the CPU trace makes win detection exact, free of readback and independent of the integrator and sample count.
- Moves: each finished transform that changed the settings counts one (`finish_transform`); the count resets when `scene_name` changes. Solving toasts "Solved in N moves (par P)".
- Levels: `scenes/puzzles` pack, `laser-1..3` (one mirror turn; a periscope; a return past a beam splitter with a mirror to move). `laser::tests` checks each starts unsolved and that its intended solution solves it within par.

