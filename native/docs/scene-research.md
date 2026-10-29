# Scene research and plan

Written 2026-10-29. This is research and a plan; no code has been changed.
Paths are relative to `native/`.

## 0. What the engine can and cannot do today

These facts come from reading the source, not the docs, and they set every limit below.

| Area | Current state | Consequence for scenes |
|---|---|---|
| Geometry | One BLAS made from a single **non-indexed triangle soup** (`geometry::geometry`). Each vertex is 32 B, so a triangle is 96 B. There is one TLAS instance with an identity transform. | Every copy of a mesh is stored again. A 1M-triangle scene is about 96 MB of vertices plus the BLAS. Any edit rebuilds everything. |
| Vertex buffer location | `upload()` creates the vertex, material and emitter buffers in an **UPLOAD heap** (system memory). The shader reads vertices from there on every hit. | Fine for 100k triangles. For millions, this costs PCIe bandwidth. Move the buffers to a DEFAULT heap first. |
| Opacity | The geometry flag is `NONE` (non-opaque), so **every candidate triangle goes through the shader's `Proceed()` loop**. This is how hidden lights and glass shadows work. | Large meshes pay an any-hit cost on every candidate. Split the geometry into an opaque descriptor and a non-opaque one before importing big scenes. |
| Shapes | 11 unit meshes: box, sphere (64×32), cylinder, cone, torus, knot, teapot, bunny (69,451 tris), octahedron, pawn, ring. They are cached in a `LazyLock`. | — |
| Transforms | Position x/z, elevation, `grounded` (floor at y = −1), size·0.35, stretch (y), `scale_xz`, yaw, and pitch/roll (`tilt`). Validation limits: position ±40, size ≤ 30, scale ≤ 20. | You cannot place an object below the floor or use an arbitrary matrix, but most layouts are possible. |
| Objects / lights | `MAX_OBJECTS` 256, `MAX_LIGHTS` 32, plus the main ceiling quad. Each object is its own material (ids 5+i). | The 256 cap is the main limit for procedural scenes. |
| Materials | Lambert + GGX (single scattering), metallic, reflectivity, smooth or approximate rough dielectric, Beer–Lambert absorption, emission ≤ 20, unlit flag, hidden-from-camera flag (lights only). | No textures, clearcoat, sheen, thin film, SSS or nested-dielectric priority. |
| Patterns | Checker, stripes and marble, evaluated in **world space**. Checker uses x and z only. Stripes run along x+z. | Patterns can create emissive windows: emission is `color × emission` after the pattern blend, so a black secondary color makes dark bands. Patterns slide when an object moves. |
| Lights | Rectangular area lights (one-sided, rotatable, can be hidden) and any emissive mesh. Power-weighted NEE over every emissive triangle, with MIS. | No point, spot or directional lights. Sun-like light means a large, far-away quad. |
| Environment | `background` (constant, 0–1) plus `sky × gradient`. Neither is importance-sampled. No HDRI. | Open scenes are lit mostly by area lights. The sky is ambient fill. |
| Stage | Cornell enclosure (2×2×2) or an open floor quad of half-size 0.5–40 at y = −1. The floor is always present. | A room has to be built from boxes. |
| Picking | CPU Möller–Trumbore over every triangle of every object, regenerating each mesh on every click. | Becomes slow with imported meshes. The GPU already writes `object_id` (u19), so reading back one pixel would replace it. |
| egui textures | `update_atlas` errors with "Only the egui font atlas is supported". | A thumbnail gallery needs user-texture support in the egui path in `gpu.rs`. |
| Dependencies | `image` with `png` and `exr` features, `serde_json`, `glam`. | Adding `hdr` to `image`, plus `tobj` or `gltf`, is a small change. |

---

## 1. Classic test scenes and asset sources

Licenses were checked on the source pages on 2026-10-29 where they were reachable. "Verify" means the page did not state the license or could not be fetched, so check it before shipping. **Usable** means usable once mesh import exists. Nothing in this table loads today except the Stanford bunny and the Utah teapot, which are already built in.

### 1a. Light-transport test scenes

| Scene | What it tests | Source | License | Size | Usable here |
|---|---|---|---|---|---|
| Cornell box (original measured) | Color bleeding, soft shadows, diffuse interreflection. It is the reference for comparing against measured data. | Cornell PCG, <https://www.graphics.cornell.edu/online/box/data.html>. Bitterli's CC0 version: <https://benedikt-bitterli.me/resources/> | Cornell data: academic, no formal license (verify). Bitterli version: **CC0** | 30–40 tris | **Yes, today.** Preset 0 already is one. The measured spectral data could tune the wall colors. |
| Jensen's Cornell box (mirror and glass spheres) | Caustic under a glass sphere; reflected and refracted light. | Henrik Wann Jensen, *Realistic Image Synthesis Using Photon Mapping* (2001) | Scene description only, so free to recreate | trivial | **Yes, today** (preset 3 is close). The caustic is slow: there is no caustic importance sampling, so it takes thousands of spp. That makes it a good stress test for RR. |
| Veach MIS (glossy plates, four spherical lights) | MIS of BSDF sampling against light sampling, across roughness and light size. | Veach thesis (1997), ch. 9. Bitterli **"Veach, MIS"** | **CC0** | < 10k tris | **Yes, today, as an approximation.** Four tilted thin boxes as metallic plates (roughness 0.01 → 0.3) and four emissive spheres of sizes 0.03 → 1 with intensity ∝ 1/r². The spheres are emissive meshes, so MIS applies to them. This is the best single check of the MIS code. |
| Veach bidirectional room ("Bidir") | Light reaching the scene indirectly, through a lamp pointing at the wall. A caustic from a glass egg. | Bitterli "Veach, Bidir Room" | **CC0** | ~30k tris | Partly today, using boxes, a hidden light facing the wall and a glass sphere. Fully, with OBJ import. |
| Veach "Ajar" door | Almost all light enters through a crack in a door. Unidirectional path tracing struggles. | Bitterli "Veach, Ajar" | **CC0** | ~60k tris (verify) | Today, as a box-built room with a gap and the light outside. It will look noisy, which is the point. It is the best scene for showing what RR can and cannot recover. |
| Water caustic / volumetric caustic | Caustic through a wavy water surface; caustic in fog. | Bitterli | **CC0** | small | Needs mesh import. The volumetric variant also needs media. |
| Glass of Water | Nested dielectrics (glass–water and water–air interfaces), caustics. | Bitterli, by aXel | **CC0** | ~100k tris (verify) | Needs import **and** nested-dielectric (IOR priority) handling to look right. |
| Material Test Ball / Mitsuba material ball | Material appearance on a standard shape. | Bitterli (CC0). Mitsuba ball: Wenzel Jakob (license on the Mitsuba GitHub; verify) | CC0 / verify | ~100k | Needs import. The CC0 Bitterli ball is the safe choice. |
| White furnace | Energy conservation: a white object under uniform white light must disappear. | Standard technique, no asset | n/a | trivial | **Yes, today.** Set `sky = 0`, `background = [1,1,1]`, all albedos 1, many bounces. It will show the energy lost by single-scattering GGX on rough metals. |
| LTE Orb | Material comparison on a calibrated orb. | pbrt-v4-scenes, by Yasutoshi Mori | **CC-BY 2.0** | ~500k (verify) | Needs import. |
| Crown | Gold, gems, many glass layers. | pbrt-v4-scenes, by Martin Lubich | Not stated (verify) | ~4M | Needs import, instancing and texture support. The license is unclear, so do not redistribute. |
| Transparent Machines | Stacked glass shapes, up to 64 bounces. | pbrt-v4-scenes, by beeple | beeple's own terms (verify) | moderate | Needs import. It is also possible today with primitives as a "glass stack" preset. |

### 1b. Interiors and architecture

| Scene | What it tests | Source | License | Size | Usable here |
|---|---|---|---|---|---|
| Crytek Sponza (2010 update) | Atrium lit through openings; alpha-tested foliage; textured architecture. | McGuire archive <https://casual-effects.com/data/>; Khronos glTF-Sample-Assets `Models/Sponza` | Khronos copy: **CRYENGINE Limited License Agreement** (`LicenseRef-CRYENGINE-Agreement`), not CC. Some mirrors say CC-BY 3.0. Use the license that ships with the copy you download. | 262,267 tris | After import + textures + alpha masks. Fits in one BLAS easily. |
| Intel Sponza (2022) | Modern high-detail Sponza with PBR textures, curtains, ivy. | Intel Graphics Research Samples | Believed **CC-BY 4.0** (verify on download) | ~20M+ tris with all add-ons (verify) | Needs instancing, textures, DEFAULT-heap buffers and indexed geometry. |
| Dabrovic Sponza / Sibenik cathedral | Older, untextured-friendly architecture. | McGuire archive | Verify per model | Sponza ~66k; Sibenik ~75k | Good **first import targets**: low triangle counts and they look fine with flat colors. |
| San Miguel | Courtyard, heavy foliage, many materials. | McGuire archive; pbrt-v4-scenes | McGuire lists CC-BY 3.0, author Guillermo M. Leal Llaguno (verify) | ~10M tris in the McGuire OBJ; pbrt: ~2.5M unique, 10.7M instanced | Needs instancing, textures and alpha. A late target. |
| Amazon Lumberyard Bistro (exterior/interior) | Street scene, many emissive lamps (many-light sampling), glass. | NVIDIA ORCA <https://developer.nvidia.com/orca>; McGuire archive; pbrt-v4-scenes | **CC-BY 4.0** (as stated by pbrt-v4-scenes) | Exterior ~2.8M tris, interior ~1M (verify) | Needs import, textures, instancing and DEFAULT heap. The interior is a good showcase for many emissive meshes. |
| Breakfast Room, Living Room, Fireplace Room, Grey & White Room, Wooden Staircase, Modern Hall, Country Kitchen, White Room, Japanese Classroom | Indirect-dominated interiors; sunlight through windows. | Bitterli Rendering Resources <https://benedikt-bitterli.me/resources/> (most authors: Wig42, Jay-Artist, NewSee2l035, NovaZeeke) | **CC BY 3.0**, attribution required | 50k–1M | After import. Many need textures; several look acceptable with flat albedos. **The best interior set**: the licenses are clear, and the files are pbrt-v4, Mitsuba 3 and Tungsten, with plain meshes. |
| Contemporary Bathroom, Bedroom | Glossy and glass-heavy interiors. | Bitterli (Mareck; SlykDrako) | **CC0** | ~100k–500k | After import. CC0, so they can ship in a scene pack. |
| Salle de bain | Bathroom, mirrors. | Bitterli (nacimus) | **CC BY 3.0** | — | After import. |
| Blender Classroom | Sun through windows, many chairs (instancing). | <https://www.blender.org/download/demo-files/> | **CC0** | ~1M+ (verify) | Hard. It is a `.blend` with Cycles node materials; export to glTF in Blender and map materials by hand. Bitterli's "Japanese Classroom" (CC BY 3.0) is easier. |
| Blender Barbershop | Cycles benchmark: hair, SSS, many lights. | Blender demo files | **CC-BY** | ~5M+ | Not realistic here (hair, SSS, node shaders). |
| Barcelona Pavilion | Glass and marble architecture, day and night. | Blender demo files (CC-BY); pbrt-v4-scenes (Hamza Cheggour, **CC-BY 2.0**) | CC-BY | ~1M (verify) | After import + textures. The night version is a hard multi-layer glass test. |
| Kroken, Watercolor | Modern interiors. | pbrt-v4-scenes (Angelo Ferretti) | **CC-BY-ND 2.0**: no derivatives | — | Render only, unmodified. Do not repackage converted files in a scene pack. |
| Conference room | Office with many small lights. | McGuire archive (Anat Grynberg, Greg Ward) | Verify | ~331k | After import. |
| Rungholt / Lost Empire | Minecraft-style voxel city: huge counts of instanced cubes. | McGuire archive (kescha / Morgan McGuire) | Verify | Rungholt ~6.7M tris | Needs instancing. A procedural voxel city (section 3) gives the same look with no license question. |

### 1c. Scanned and classic models

| Model | Source | License (exact terms as published) | Triangles | Usable here |
|---|---|---|---|---|
| Stanford bunny | Stanford 3D Scanning Repository <https://graphics.stanford.edu/data/3Dscanrep/> | Stanford terms: free for **research** use and **free redistribution** with credit to the Stanford Computer Graphics Laboratory; images may appear in scholarly publications; **no commercial use or inclusion in a product for sale without Stanford's permission**. | 69,451 | **Already in the project** under those terms (`assets/CREDITS.md`). |
| Stanford dragon | same | same Stanford terms | 871,414 (VRIP); Wikipedia lists 1,132,830 for another reconstruction | After import. Fits in one BLAS (~84 MB as soup; ~25 MB indexed). |
| Happy Buddha | same | same Stanford terms. Stanford also asks for respectful use given the statue's religious nature (verify the wording on the page). | 1,087,716 | After import. Do not use in jokey scenes. |
| Armadillo | same | same Stanford terms | 345,944 | After import. |
| Lucy | same | same Stanford terms | ~28M (14M vertices) | Too big as soup (2.7 GB). Needs indexed geometry, DEFAULT heap, and in practice a decimated version. |
| Asian dragon, Thai statue | same (XYZ RGB scans) | same Stanford terms | 7.2M; 10M | Decimate first. |
| Spot, Bob, Blub | Keenan Crane's model repository <https://www.cs.cmu.edu/~kmcrane/Projects/ModelRepository/> | **CC0** | 5,856 (Spot) | Ideal: small, CC0, and safe for a public scene pack. |
| Suzanne | Blender | **GPL-2.0-or-later** (inherited from Blender) | 500 faces | Avoid bundling, because of the GPL. Users can load it themselves. |
| Utah teapot | Newell | Free; commercial use allowed. The project uses three.js data under MIT. | patches | Already in. |
| Nefertiti bust scan | Nefertiti Hack | **CC BY-SA 4.0** | ~2M | Share-alike: a derivative scene file would have to be CC BY-SA. Keep it out of the main pack. |
| Common test-model mirror | <https://github.com/alecjacobson/common-3d-test-models> | Per model (follows the originals) | — | Convenient OBJ downloads. The license is always the original's. |

### 1d. Asset libraries

| Library | URL | License | Notes for this engine |
|---|---|---|---|
| Bitterli Rendering Resources | <https://benedikt-bitterli.me/resources/> | Per scene: CC0 or **CC BY 3.0**. Exceptions: Cem Yuksel hair is **CC BY-NC**, and Lego Digger is **CC BY-NC 3.0**. | 32 scenes in pbrt-v4, Mitsuba 3 and Tungsten. Meshes are plain OBJ/PLY with simple materials (diffuse, conductor, dielectric, rough variants). **This is the best first import target**, because its materials map almost one-to-one onto ours. |
| McGuire Computer Graphics Archive | <https://casual-effects.com/data/> | **Per model.** Many are CC-BY 3.0; some are author-specific or research-only. Morgan McGuire asks for citation of the archive. The page is JavaScript-rendered, so read each model's license note on download. | OBJ + MTL, which suits a `tobj` importer. Sponza, San Miguel, Bistro, Breakfast Room, Conference, Sibenik, Rungholt, Stanford models. |
| Khronos glTF-Sample-Assets | <https://github.com/KhronosGroup/glTF-Sample-Assets> | **Per model**, in each folder's README and `REUSE.toml`. Examples: FlightHelmet, SheenChair, Lantern, Corset, WaterBottle, ToyCar, TransmissionTest, GlassVaseFlowers, TransmissionThinwallTestGrid and AntiqueCamera are **CC0** (AntiqueCamera's logo is trademark-restricted). ABeautifulGame, StainedGlassLamp, IridescenceLamp, AnisotropyBarnLamp, ChronographWatch, CarConcept, CommercialRefrigerator, PotOfCoals, SpecularTest, TransmissionRoughnessTest and EmissiveStrengthTest are **CC BY 4.0**, and some also carry Khronos trademark-logo terms. DamagedHelmet is CC BY 4.0, but an older version is CC BY-NC. DragonAttenuation uses the **Stanford** dragon (Stanford terms) with a CC0 cloth. Sponza is **CRYENGINE license**. | glTF is the cleanest import route: PBR metal-rough maps to our material fairly directly. The transmission and volume test models are especially relevant. **ABeautifulGame** (a chess set, CC BY 4.0) is a natural successor to preset 20. |
| Poly Haven | <https://polyhaven.com> | **CC0** for all HDRIs, textures and models; commercial use and redistribution allowed; no attribution needed. Site content (logos, renders, text) is not CC0, and scraping is not allowed, so download by hand or through their official API under its terms. | HDRIs need environment-map support; models need glTF import plus textures. This is the safest source for a public scene pack. |
| Blender demo files | <https://www.blender.org/download/demo-files/> | Per file: **CC0**, **CC-BY** or **CC-BY-SA**. Classroom is CC0, Barbershop CC-BY, Pavillon CC-BY. Junk Shop, Lone Monk and Spring list no license on that page (verify). | `.blend` with Cycles nodes; export them through Blender. |
| pbrt-v4-scenes | <https://github.com/mmp/pbrt-v4-scenes> | Per scene (CC0, CC-BY 2.0/4.0, **CC-BY-ND 2.0**, beeple terms, unstated). | Large, well-lit scenes. Converting them needs a pbrt-v4 parser, which is heavy, so prefer the glTF/OBJ versions when they exist. |
| NVIDIA ORCA | <https://developer.nvidia.com/orca> | The page does not state licenses (Bistro is CC-BY 4.0 per other sources; UE4 Sun Temple is believed CC BY-NC-SA). The SDK is legacy. | FBX. Use the Bistro OBJ/glTF mirrors instead. |
| Disney Moana Island | <https://www.disneyanimation.com/resources/moana-island-scene/> | Disney's own license agreement (License.txt on the page). It is meant for developing, testing and benchmarking rendering algorithms. **Read it before any redistribution.** | Base download 45 GB (93 GB unpacked); billions of instanced primitives; Ptex textures. **Too big for this project.** It is mentioned only as the upper reference point. |

### Licensing cautions

- **Stanford models** (bunny, dragon, Buddha, armadillo, Lucy, Thai statue, Asian dragon) allow research use and free redistribution with credit, but **not commercial use or inclusion in a product for sale without Stanford's permission**. The bunny is already bundled under these terms. If Cornell RTX is ever sold or bundled commercially, the bunny must be removed or licensed. Keep all Stanford models in a separate `research` pack, flagged in the UI.
- **CC BY / CC BY 3.0 / 4.0**: commercial use is fine, but the attribution must ship with the scene. The pack manifest (section 5) should carry author, source and license fields, and the gallery should display them.
- **CC BY-NC** (Yuksel hair, Lego Digger, old DamagedHelmet, likely Sun Temple): no commercial use. Keep these out of default packs.
- **CC BY-ND** (Kroken, Watercolor): you may render them, but converted or modified files must not be redistributed. Load them from the user's own download.
- **CC BY-SA** (Nefertiti, Disney Cloud) and **GPL** (Suzanne): share-alike terms carry over to derived scene files. Keep them out.
- **CRYENGINE license** (Khronos Sponza): this is not a CC license. Read the agreement shipped in the Khronos repo before redistributing.
- **Trademarks**: Khronos, glTF, UX3D and DGG logos inside some glTF samples are trademark-licensed separately. Do not use them in promotional renders.
- **Safest default**: build public scene packs from **CC0 only** (Poly Haven, Keenan Crane, Bitterli's CC0 scenes, the CC0 Khronos samples) plus scenes this project generates itself. Add CC-BY scenes with visible credits. Do not download assets automatically at runtime; the project already avoids runtime downloads for DLSS.

---

## 2. Scenes buildable now (no engine changes)

Each of these needs only `Settings`, objects, materials, patterns and area lights. All fit under 256 objects and 32 lights. They could be added as presets in `presets.rs`, or better, as JSON files once the scene folder exists (section 5), since `--settings` already loads them.

**Cornell-box variants (keep `cornell_room = true`)**

1. **Mirror room**: walls metallic with roughness 0, with a matte sphere inside. Infinite interreflection that tests the bounce limit and RR on recursive mirrors.
2. **Measured Cornell**: wall colors and light from the original Cornell measurement data, with the two blocks at the published coordinates. A reference to compare against.
3. **Cornell bunny / Cornell teapot**: the classic box with a glass or jade bunny in place of the tall block. A familiar baseline with a complex mesh.
4. **Cornell caustic**: one small, bright light and a glass sphere on the floor. A caustic stress test showing current limits.
5. **Cornell dark room**: the main light off, with one emissive torus as the only light source. Emissive-mesh NEE in a closed room.
6. **Cornell water-tank**: a large glass box filling the lower half with blue absorption, plus objects inside. Beer–Lambert absorption over long paths. Nested media are not handled, so keep the objects outside or opaque.

**Validation and lighting studies (open stage)**

7. **White furnace**: `sky 0`, `background 1`, every albedo 1, a sphere row from rough to smooth metal. Energy loss shows up as dark objects.
8. **Veach MIS approximation**: four tilted chrome plates of rising roughness and four emissive spheres of rising size at equal power. A MIS check.
9. **Veach ajar room**: a box-built room, door slightly open, light outside. Hard indirect lighting that shows the noise floor and what RR does with it.
10. **Penumbra ladder**: a row of five posts, each under its own hidden area light of growing size (0.1 → 2). Soft-shadow width against light size.
11. **Light-temperature row**: seven matte busts (pawns), each lit by a light colored from 2000 K to 10000 K. White balance and color.
12. **Sky-only overcast still life**: no area lights, `sky = 4`. Ambient-only convergence; shows the cost of not importance-sampling the sky.
13. **Glossy floor tiles**: thin boxes as floor tiles, roughness 0 → 1 in a strip, with a bright object behind. Rough reflections smearing along the strip.

**Material showcases**

14. **Shape × material matrix**: all 11 shapes × 12 library materials (132 objects). A catalog of everything the engine has.
15. **Gem display**: octahedra with IOR 2.4 (diamond), 1.77 (sapphire, blue absorption) and 1.57 (emerald), on a black floor under small lights. Strong internal reflection and sparkle, without dispersion.
16. **Jewellery box**: gold and silver rings (Ring and Torus shapes) and pearl spheres (porcelain, low roughness). Metals in close interreflection.
17. **Ice cubes**: frosted glass boxes at random rotations on dark slate. Rough transmission.
18. **Glass stack ("transparent machines")**: stacked glass boxes, cones and tori at 32 bounces. Deep transmission chains.
19. **Bunny museum**: six bunnies on box plinths (glass, gold, jade, marble, chrome, rubber), each under its own hidden spot-like quad, in a dark room. Material comparison on a scan.
20. **Teapot tea set**: teapot, cylinder cups with ring handles, a box tray in lacquer. A domestic still life.

**Architecture and environments**

21. **Stonehenge at dusk**: rough stones (boxes) in a ring with lintels, a 2800 K sun as a large distant quad low on the horizon, `sky` low. Long shadows and warm–cool contrast.
22. **Torii corridor**: 12 red gates (cylinders + boxes) receding, with a sunset light at the end. Repetition, perspective and color bleeding.
23. **Spiral staircase**: 64 box steps rotated around a central cylinder, lit from the top. Rotation and self-shadowing.
24. **Colonnade at night**: columns, with emissive spheres as braziers between them. Many emissive meshes driving NEE.
25. **Night skyline**: about 150 box towers with emissive checker or stripe windows (pattern × emission), on a dark floor with a faint sky. Hundreds of emissive triangles.
26. **Desert monolith**: an orange floor, a matte black slab, strong sun, high `sky`. Minimalism and hard shadow.
27. **Zen garden**: a floor with a fine stripe pattern (raked sand), rough octahedron rocks, a single low sun. Patterns at grazing light.
28. **Disco floor**: an emissive checker floor (emission on the floor material), a chrome sphere, dark sky. An emissive floor as a light source.
29. **Mirror maze**: chrome slabs in a grid with a glowing sphere in the center. Long specular paths and RR reflection guides.
30. **Stained-glass hall**: a wall of colored glass panes (thin boxes) with a sun behind. The colors appear in glass reflections only: shadow rays pass glass untinted (`glass_shadow`), so there are no colored shadows. Useful for showing that limitation.

**Playful still lifes**

31. **Newton's cradle**: five chrome spheres floating on thin cylinders from a box frame. Mirror interreflection between spheres.
32. **Domino spiral**: 200 thin boxes along an Archimedean spiral. Many instances of a simple shape near the object cap.
33. **Orrery / solar system**: an emissive sun sphere, lacquered planets, a ring around a Saturn. One central emissive sphere lighting the whole scene.
34. **Atom**: a nucleus of colored spheres plus three tilted glowing rings. Emissive rings and tilted geometry.
35. **Armillary sphere**: five gold rings at different tilts around a glass sphere. Tilt transforms and metal on glass.
36. **Snowmen**: stacked white spheres, cone noses, a cool sky and blue fill. Subtle white-on-white shading.
37. **Pool table**: a green box table, 16 colored lacquer balls, a cylinder cue, a hanging softbox. Glossy dielectric balls.
38. **Candle shelf**: cylinders with small warm emissive spheres as flames, dark room. Many small lights (MIS stress).
39. **Bowling lane**: a stretched glossy box lane, ten pawns scaled as pins, a ball. Long glossy reflections.
40. **Stylised forest**: 120 cone + cylinder trees, seeded, with a sun quad. Shadow complexity near the object limit.
41. **Abacus**: torus beads on cylinder rods in a box frame. Many small curved objects.
42. **Pipe organ**: 40 chrome cylinders of rising height in front of a warm wall. Anisotropic-looking reflections from cylinders.

---

## 3. Procedural generators (effectively unlimited scenes)

A generator is a function `fn(seed: u64, params) -> Settings`. With 2^64 seeds and a few parameters, each generator becomes a family of scenes. The `randomize()` function in `scene.rs` already uses an xorshift RNG and could share a helper. Generators can live in a new `src/generators.rs` behind a "Generate" UI (a family dropdown, a seed field, "Next seed"), and a scene file can say `{"generator": "city", "seed": 42}`.

Budget rules for today's engine:
- **Objects**: hard cap 256. Generators must count objects and stop or merge.
- **Triangles**: there is no hard cap, but with the soup, the upload heap and any-hit on every candidate, keep a scene under about 2–3M triangles. A sphere is 4,096 tris, a torus 5,120 and a bunny 69k, so 256 spheres is about 1M tris.
- **Instancing** (section 4, feature B) removes both limits. Repeated shapes then cost one BLAS each, and the TLAS can hold millions of instances.

| Generator | Output | Fits today? | Needs |
|---|---|---|---|
| **Studio still life** | 3–12 objects picked from shapes and the material library, with a rule-of-thirds layout, a key/fill/rim light rig with random temperatures, and a random camera within limits. | Yes | Nothing. This is the cheapest way to "countless scenes". |
| **Material grid / swatch board** | N×M of one shape, sweeping two parameters (roughness × metallic, IOR × absorption, emission × size). | Yes, up to 16×16 | Nothing |
| **City blocks** | Grid streets; each lot holds 1–4 boxes with height from noise; window emission from patterns; sun or night mode. | Up to about 250 buildings | Instancing for bigger cities. Object-space patterns so windows stay aligned to each building. |
| **Voxel world (Rungholt-like)** | A heightfield of cubes. | No (thousands of cubes) | **Greedy meshing into one mesh object** (feature A), or instancing. |
| **Columns / temple / arcade** | Parametric classical layout: column count, spacing, order (cylinder with box capital), lintels, steps. | Yes | Nothing |
| **Stairs and architecture** | Straight, L, U and spiral stairs, rooms from wall slabs with door and window holes. Rooms are rectangular, so walls are just split into boxes. | Yes | Nothing |
| **Physics-free stacking** | Cairns, Jenga towers, card houses, sphere pyramids. Each piece rests on the top of the one below; the height is computed analytically, no simulation. | Yes | Nothing |
| **Menger sponge** | Level 2 = 400 cubes, level 3 = 8,000 cubes. | Level 2 as objects; level 3+ no | As **one generated mesh** with hidden interior faces removed (level 4 is about 1.2M tris). Needs feature A (runtime mesh objects). |
| **Sierpinski tetrahedron / pyramid** | Level n = 4^n tetrahedra. | Level 3 (64) or 4 (256) using octahedra or cones as stand-ins | One merged mesh for higher levels. A proper tetrahedron shape would help (easy to add). |
| **Mandelbulb / Julia sets** | Signed distance field → marching cubes → mesh. 256³ gives about 1–3M tris. | No | Feature A. The meshing runs on the CPU at load and is cached to disk. |
| **L-system trees** | Branches as cylinders (or swept tubes), leaves as small quads. | Small trees as ≤ 256 cylinders | Merged-mesh output: one tree ≈ 10k–100k tris. Forests need instancing. Leaves need alpha or textured cards for realism. |
| **Terrain heightfield** | fBm or ridged noise grid, 512² ≈ 520k tris, colored by height or slope. | No | Feature A, plus per-vertex colors or textures for the snow line. A water plane can be a glass box. |
| **CSG-like arrangements** | Unions are free (overlapping objects). Real subtraction (a box with holes) needs mesh booleans or SDF meshing. | Unions only | Use SDF → marching cubes (as for the Mandelbulb) for subtraction and blending. This avoids a boolean library. |
| **Recursive spheres (sphereflake)** | Haines' sphereflake: level 3 ≈ 820 spheres, level 4 ≈ 7,381. | Level 2 (91) | Instancing for level 4+. This is also one of Haines' SPD benchmark scenes, so it has history. |
| **Kaleidoscope / radial symmetry** | N-fold rotational copies of a random motif. | Yes | Nothing |
| **Mosaic / Truchet floor** | Tiled thin boxes or quarter-tori with random rotations. | ≤ 256 tiles | Merged mesh for dense tilings. |

Implementation notes:
- Generators must produce `Settings` that pass `validate()`. Add a unit test (like `randomized_scenes_are_valid`) that runs each generator for 200 seeds.
- Keep generator output deterministic. The seed and parameters are stored in the scene file, not the generated object list, so files stay small and regenerate exactly.
- For heavy meshes (Mandelbulb, terrain, Menger level 4), cache the generated mesh as a binary file keyed by a hash of (generator, version, params), next to the user's scene pack.

---

## 4. Engine features that unlock the most scenes

Ranked by (number of new scenes) ÷ (effort). The effort assumes the current code structure.

| Rank | Feature | Unlocks | Effort | What to touch |
|---|---|---|---|---|
| **A** | **Mesh objects from data**: `Shape::Mesh` with a mesh id, loaded from OBJ (crate `tobj`, MIT) or generated at runtime, plus a mesh cache. | Every procedural generator in section 3; all Bitterli, McGuire and Stanford models; Spot/Bob/Blub. | **Low–medium** | `scene.rs` (a new `Object` field such as `mesh: Option<String>`; Shape enum and labels; `validate`), `geometry.rs` (`MESHES` becomes a cache keyed by name/path; `object_geometry`), `editor.rs` (mesh picker), `Cargo.toml` (`tobj`), `main.rs` (self-test, dirty `geometry_key` includes the mesh path), `assets/CREDITS.md`. Per-object material stays as is. Multi-material OBJ files can split into one `Object` per material group at import. |
| **B** | **Real scene structure**: indexed geometry, **one BLAS per unique mesh**, a TLAS instance per object (transform in the instance descriptor), **buffers in a DEFAULT heap**, and **opaque vs non-opaque split**. | Scenes with thousands to millions of repeated objects (forests, cities, Rungholt, sphereflake 4+, San Miguel); edits rebuild only the TLAS; memory drops 3–4×; big speedups on large scenes. Also lifts the 256 limit. | **Medium–high** (the biggest change) | `gpu.rs` `rebuild_scene` (multiple BLAS, instance array, compaction, DEFAULT-heap uploads with a copy queue), `common.hlsli` (per-instance table: vertex/index offset, material, emitter offset), `pathtrace.hlsl` `intersect`/`occluded` (use `CommittedInstanceID()` / `CandidateInstanceID()` and indices; transform normals by the object-to-world matrix; set opaque flags per instance, using `InstanceMask` for hidden lights), `lighting.rs` (emitter table per instance; the emitter lookup indexed by instance + primitive), `geometry.rs` (mesh-space data; picking through `object_id` readback instead of the CPU loop), `scene.rs` (raise `MAX_OBJECTS`, separate materials from objects so many instances share one). Patterns can move to **object space** here, fixing the sliding. |
| **C** | **HDRI environment lighting with importance sampling** (equirect `.hdr`/`.exr`, a 2D marginal/conditional CDF, MIS with BSDF sampling, rotation and intensity). | Every open-stage scene becomes photographic; Poly Haven gives hundreds of CC0 HDRIs, so every existing preset × every HDRI is a new scene. Fixes the "sky not importance-sampled" limit. | **Medium** | `Cargo.toml` (`image` feature `hdr`; `exr` is already on), `scene.rs` (`environment: Option<String>`, rotation, intensity), `gpu.rs` (a texture SRV and two CDF buffers; descriptor heap has 64 slots), `common.hlsli`, `pathtrace.hlsl` (`environment_radiance` samples the map; add an env-sampling branch in NEE alongside `select_emitter`, with MIS on BSDF-sampled misses). RR guides are unaffected. |
| **D** | **Image textures + UVs** (base color, roughness/metal, normal, emission, alpha mask). | Sponza, Bistro, Bitterli interiors at full quality; glTF samples; Poly Haven models; textured emitters (TV screens, signs); alpha-tested foliage. | **Medium–high** | `scene.rs` `Vertex` (add UV and tangent; the vertex grows from 32 B to 48 B, so do it together with B's indexed layout), `lighting.rs` `PackedMaterial` (texture indices), `gpu.rs` (bindless SRV range in the root signature, a larger descriptor heap, mip generation, a sampler), `pathtrace.hlsl` (`SampleLevel` with ray-cone LOD; alpha test inside the `Proceed()` loop for non-opaque cutouts), `post.hlsl` unchanged. Emissive textures also need the emitter table to weight triangles by average texel emission. |
| **E** | **glTF 2.0 import** (crate `gltf`, MIT/Apache). | All Khronos samples, Poly Haven models, Sketchfab/Blender exports; cameras and punctual lights come along. | **Medium**, after A and D | A new `src/import.rs`: metal-rough → `Material`; `KHR_materials_transmission`/`volume`/`ior`/`emissive_strength` map onto existing fields; `clearcoat`/`sheen`/`iridescence` are ignored until F. Node hierarchy → instances (needs B). |
| **F** | **Analytic lights**: point/sphere, spot (cone + falloff), directional sun with an angular radius. | Theatrical lighting, product shots, sunlight with crisp shadows; glTF punctual lights. | **Low–medium** | `scene.rs` (light type and cone angles in `AreaLight`, or a new `Light` enum), `lighting.rs` (a light list next to the emitter table; selection CDF shared by power), `pathtrace.hlsl` (NEE branch; delta lights need no MIS weight; sun via a cone-sampled direction). The UI in `editor.rs`. Prefer a *small sphere/disk* over true delta lights, since RR and the native denoiser handle finite sources better. |
| **G** | **Scene file folder + gallery** (section 5) | Turns everything above into browsable content. | **Low** (folder + list), **medium** (thumbnails) | `main.rs` (a scenes window, `--scene path`), `scene.rs` (load relative asset paths), `gpu.rs` (egui user textures; today `update_atlas` rejects anything but the font atlas). |
| **H** | **Nested dielectrics** (IOR priority stack, Schmidt & Budge 2002). | Glass of water, liquid in glass, ice in a drink, gems in settings, aquariums. | **Low–medium** | `pathtrace.hlsl` (a small per-path interface stack replacing the single `absorption` variable), `scene.rs`/`lighting.rs` (a priority field per material). |
| **I** | **Clearcoat, sheen, thin-film/iridescence, anisotropy** | Car paint, fabrics, soap bubbles, brushed metal; glTF extensions. | **Low each** for clearcoat and sheen; medium for thin film and anisotropy | `scene.rs` `Material`, `lighting.rs` (`PackedMaterial` grows from 64 B to 80–96 B), `pathtrace.hlsl` `eval_bsdf` and sampling, `editor.rs`. Also fixes GGX energy loss if multiple-scattering compensation is added (Kulla–Conty style, a small LUT). |
| **J** | **Volumes / participating media** (a homogeneous medium inside a mesh, plus global fog; later NanoVDB grids). | Fog, god rays, subsurface (jade, milk, skin), smoke; Bitterli volumetric caustic; Disney cloud (CC BY-SA). | **Medium** for homogeneous; **high** for heterogeneous grids | `pathtrace.hlsl` (free-flight sampling, a phase function, NEE through media), `scene.rs`/`lighting.rs` (scattering coefficient, absorption, anisotropy g). RR handles volumes poorly, so expect artifacts; the native denoiser may be better here. |
| **K** | **Animation / turntable** (keyframed camera and objects, image-sequence export) | Every scene becomes a video; uses `--capture-sequence`. | **Low** for the camera; medium for objects (rebuilds per frame until B) | `main.rs`, `scene.rs`. |
| **L** | **Caustic help** (path guiding, manifold NEE, or ReSTIR) | Water caustics, glass on tables. | **High** | Research-grade. Out of scope for "more scenes". |

Two smaller items, which are cheap and remove obvious friction:
- **Opaque flag for non-glass objects** (part of B, but can ship alone). Put opaque and non-opaque triangles into two geometry descriptors in the one BLAS, and set `D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE` on the first. The shader's `Proceed()` loop then only runs for glass and hidden lights. This is a speedup for every scene, measurable with the existing benchmarks.
- **GPU picking** via the existing `object_id` texture (u19). Read back one texel. This removes the O(total triangles) CPU loop before imported meshes make it slow.

---

## 5. Recommended roadmap: 24 → hundreds of scenes

The order maximizes scene count early and keeps each step shippable and testable with the existing self-test and `verify.ps1`.

**Step 1: scene files and the folder (days, no renderer changes).**
- The native `Settings` JSON already is a complete scene description, and `--settings` loads it. Add a `scenes/` folder and a "Scenes" window that lists it. Write the 42 scenes in section 2 as JSON files instead of Rust code; keep the 24 built-ins in code.
- Add `--scene path` (an alias of `--settings` that resolves relative asset paths against the file's folder).
- **Result: about 66 scenes.**

**Step 2: generators (about 1 week).**
- `src/generators.rs` with studio still life, material grid, city, temple, stairs, stacking, kaleidoscope, sphereflake level 2 and Menger level 2. Each is seeded.
- A scene file can be `{"generator": "...", "seed": N, "params": {...}}`.
- **Result: unlimited seeded variants, with about 10 curated seeds per family listed in the gallery.**

**Step 3: mesh objects + opaque split + GPU picking (feature A plus the two small items; about 1 week).**
- OBJ import and runtime meshes. Unlocks Mandelbulb, terrain, Menger 3–4, L-system trees, and the Bitterli/McGuire/Stanford/Keenan Crane models with flat materials.
- **Result: about 30 imported scenes (Bitterli's 32 are the main source), plus mesh-based generators.**

**Step 4: HDRI environments (feature C; about 1 week).**
- Ship 5–10 Poly Haven CC0 HDRIs in a pack. Every studio scene × every HDRI is a variant, so the gallery can offer an "environment" dropdown instead of listing all combinations.
- **Result: open scenes look photographic; several hundred combinations.**

**Step 5: instancing and big scenes (feature B; 2–3 weeks).**
- Lift the 256-object cap. City, forest, voxel world, sphereflake 4–5, and large imports.
- Patterns move to object space.

**Step 6: textures + glTF (features D and E; 2–3 weeks).**
- Sponza, Bistro, Khronos samples, Poly Haven models.

**Step 7 onward**, as appetite allows: analytic lights (F), nested dielectrics (H), clearcoat and sheen (I), volumes (J), turntables (K).

### Scene pack folder format

```
scenes/
  builtin/                       # the 42 hand-made scenes from section 2
    pack.json
    veach-mis.json
    stonehenge-dusk.json
    ...
  generated/
    pack.json
    city-night-0042.json         # {"generator":"city","seed":42,"params":{...}}
  bitterli-cc0/                  # user-downloaded, not committed if large
    pack.json
    meshes/bathroom/*.obj
    textures/...
    contemporary-bathroom.json
  research/                      # Stanford models: separate, flagged non-commercial
    pack.json
  thumbnails/                    # generated cache, git-ignored
```

`pack.json`:

```json
{
  "name": "Bitterli CC0 scenes",
  "version": 1,
  "license": "CC0-1.0",
  "commercial_use": true,
  "attribution": "Scenes by Mareck, SlykDrako, ... via Benedikt Bitterli's Rendering Resources",
  "source": "https://benedikt-bitterli.me/resources/",
  "scenes": ["contemporary-bathroom.json", "bedroom.json"]
}
```

Scene file: a small wrapper around the existing `Settings` JSON. `Settings` uses `#[serde(default)]` and does not deny unknown fields, so new optional fields inside `settings` stay compatible both ways. The loader unwraps `settings`, and a bare `Settings` file (what Save writes today) is still accepted.

```json
{
  "format": 1,
  "title": "Stonehenge at dusk",
  "description": "Box stones, a low 2800 K sun quad, long shadows.",
  "tags": ["architecture", "sunset", "open-stage"],
  "license": "CC0-1.0",
  "credits": "Cornell RTX",
  "environment": null,
  "settings": { "...": "the current Settings object, unchanged" }
}
```

Later fields (one feature at a time): `objects[].mesh` (a relative path), `environment` (an HDRI path, rotation, intensity), `generator` (name, seed, params), `materials` (a shared table, needed with instancing).

Rules:
- All asset paths are relative to the scene file. Reject absolute paths and `..` outside the pack, because packs may come from other people.
- Every scene goes through `validate()`. Imported meshes get triangle and file-size limits (for example 20M tris and 2 GB) with a clear error.
- Every pack has a license field. The gallery shows it, and packs marked `"commercial_use": false` show a badge.
- Large third-party assets are **not committed**: add `native/scenes/*/meshes` and `textures` to `.gitignore`, and give a `tools/fetch-scenes.ps1` script that downloads from the original URLs and checks SHA-256 hashes, like `tools/setup.ps1` does for DLSS.

### Gallery / browser

- An egui window: tags and pack filter, search, a thumbnail grid, and a details panel (description, triangle count, light count, license, credits). Double-click loads the scene.
- **Thumbnails**: render headlessly with the existing CLI (`--headless --frames 64 --width 320 --height 200 --settings f.json --capture thumb.png`) from a `tools/thumbnails.ps1` batch script; cache them in `scenes/thumbnails/` keyed by a hash of the file. This needs egui user-texture support in `gpu.rs` (today only the font atlas works). A cheaper first version is a list without images.
- **Self-test**: extend the frame script to load every file in `scenes/builtin` and assert triangle and emitter counts, as it already does for the 24 presets.
- **Regression gallery**: the same thumbnail script can render a contact sheet of all scenes before and after a renderer change. This is a quick visual diff, complementing `rr_stability.py` and `motion_error.py`.

### Suggested first ten files to write (step 1)

These ten give the most variety for the effort and exercise different parts of the renderer:
1. White furnace
2. Veach MIS approximation
3. Veach ajar room
4. Stonehenge at dusk
5. Night skyline
6. Gem display
7. Glass stack
8. Bunny museum
9. Mirror room
10. Orrery

---

## Sources

- Bitterli, Rendering Resources: <https://benedikt-bitterli.me/resources/>
- McGuire Computer Graphics Archive: <https://casual-effects.com/data/> (licenses are per model on download)
- Khronos glTF-Sample-Assets, model list with licenses: <https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/Models.md>
- Poly Haven license: <https://polyhaven.com/license>
- Blender demo files: <https://www.blender.org/download/demo-files/>
- pbrt-v4 scenes: <https://github.com/mmp/pbrt-v4-scenes>
- NVIDIA ORCA: <https://developer.nvidia.com/orca>
- Disney Moana Island Scene: <https://www.disneyanimation.com/resources/moana-island-scene/>
- Stanford 3D Scanning Repository: <https://graphics.stanford.edu/data/3Dscanrep/> (unreachable on 2026-10-29; terms summarized from `assets/CREDITS.md` and common knowledge. Re-check before relying on them.)
- Wikipedia, List of common 3D test models: <https://en.wikipedia.org/wiki/List_of_common_3D_test_models>
- Keenan Crane's 3D model repository: <https://www.cs.cmu.edu/~kmcrane/Projects/ModelRepository/>
- Common test-model mirror: <https://github.com/alecjacobson/common-3d-test-models>
