## Grounded dielectric gap (2026-11-10)

Closes "Coplanar dielectric contacts" (section "CPU reference renderer"): where a dielectric face lies exactly on another face, both hits come at equal t and DXR's traversal and the CPU BVH break the tie differently, so neither renderer is wrong and the oracle fails. The owner chose a geometry offset that both renderers read (option B) over tie-break code in traversal.

- **Floor and support contacts, automatic:** `geometry::ground_lift` adds `DIELECTRIC_GAP` (4 mm, in a 2-unit room) to the pivot of a grounded object whose material transmits (`geometry::transmissive`: `transmission > 0`). It is applied in one place, `grounded_y`, which `object_frame` and `object_geometry` share; everything else (instance transforms, `object_corners`, the CPU reference via `geometry::geometry` and `object_transform` for SDFs, physics, gizmo, picking, snapping, lighting bounds) reads those, so nothing applies it twice. An elevated grounded glass (on a plinth) is lifted the same, which also separates it from its support. No per-object toggle.
- **Parts of one file rise together:** an imported multi-part file is grounded as a whole (`mesh::ground` measures the file), so lifting only its glass parts would split a bottle from its cap. `ground_lift` therefore lifts a grounded part when it is transmissive or when any part of its file imported as transmissive (`mesh::file_has_glass`, from the file's own part materials). Limit: a user edit that makes one part of an all-opaque file transmissive lifts that part alone. Test `the_parts_of_a_file_with_glass_rise_together`.
- **Physics:** a transmissive body's collider gets a rapier contact skin of `DIELECTRIC_GAP`, so a simulated glass settles the same 4 mm off the floor and off an opaque body as its grounded pose put it: no drop or jitter when the simulation starts. Rapier sums the two colliders' skins, so glass on glass would rest 8 mm apart; `GlassContacts` (a physics hook, on glass colliders only, marked by `user_data` 1) adds one gap back to such contacts' distance, giving 4 mm. `same_shape` includes the transmission bit, so crossing 0 rebuilds the body. Tests `a_grounded_glass_keeps_its_gap_when_simulated`, `glass_rests_one_gap_off_glass_and_off_opaque` (floor-glass, opaque-glass, glass-opaque and glass-glass stacks settle within 1 mm of the gap), `changing_transmission_rebuilds_the_body`.
- **Opaque on glass floats too, by design:** an opaque object resting on a grounded glass sits on top of the lifted glass, so it floats 4 mm as well (statically and simulated); nothing lifts it back.
- **Object-object contacts: no general rule.** Shrinking or separating nested dielectrics automatically would insert a thin film of the outer medium (an air gap between glass and water gives total internal reflection that the scene never had), and scenes rely on deliberate overlaps (priority CSG). So the built-in scenes were fixed in `tools/build_scenes.py` (`GAP`, same 4 mm): glass-stack's slabs and cone stack with a gap and start off the floor; stained-glass-hall's panes, which overlap in x and shared their front and back faces, step 6 mm in z per column; layered-cocktail's and aquarium's floor-standing vessels (built with `float_at`, not grounded) start at -1 + GAP, and liquids or air sharing a side wall where they overlap (Orange juice and Headspace, Tumbler headspace, the aquarium's air box) are narrowed by GAP. Hand-placed user objects touching exactly still tie; the rule for scenes is to overlap dielectrics (priority decides) or leave a gap, never to make faces coincide.
- **Oracle** (320 x 240, 2048 spp each side, `--time 0.7 --rgb`; before = the 2026-11-06 sweep, same settings):

| scene | before GPU/CPU (z, worst block) | after GPU/CPU (z, worst block) |
|---|---|---|
| cornell-water-tank | 1.0411 (z 88) fail | 1.0005 (z 1.38, block 1.0309 z 2.05) |
| stained-glass-hall | 1.0849 (z 37, block 1.4163) fail | 0.9999 (z -0.88, block 0.9988 z -2.47) |
| glass-stack | 0.9955 (z -17, block 0.9451) fail | 0.9998 (z -0.95, block 0.9990 z -2.05) |
| glass-of-water | 0.9973 (z -6) fail | 1.0000 (z 0.10, block 0.9946 z -3.10) |
| aquarium | 1.0058 (z 6) fail | 1.0000 (z -0.01, block 0.9868 z -2.34) |
| layered-cocktail | 1.0002 (block 0.9673, z -6) fail | 0.9998 (z -0.69, block 0.9814 z -3.64) |
| shape-material-matrix | pass, block 0.9835 (z -4.4) | 1.0002 (z 1.46, block 1.0012 z 3.03) |
| sdf-gallery (glass cone) | block 1.026 (z 9.3), 1024/512 spp | 0.9995 (z -3.03, block 0.9936 z -5.89) ok |

  The gap was not swept below 4 mm (1 cm was the earlier measurement); any non-zero gap removes the exact tie, so a smaller one should also pass but is untested. At normal view distance 4 mm under a 1.4-unit glass is not visible (crop in the lane's scratch folder).

