## Metaballs and track 18–19 leftovers (2026-11-02)

- *Metaballs* (`Sdf::Metaballs`, shader kind 5): a bottom pool, a top cap and four blobs on Lissajous paths joined by a polynomial smooth minimum. The field is built for an object 3× taller than wide (it works in q = (x, 3y, z) and returns d/3, a valid bound), so the `lava-lamp` wax object is a box of that aspect. The blob positions follow `Object::phase` (keyable like any float; validate keeps 0..10000). The phase travels in the high 24 bits of the procedural instance's `vertex_base` (thousandths), the kind in the low 8; `sdf`, `sdf_normal` and `sdf_hit` take that raw word. A phase change is a geometry-key change, so playback rebuilds the instance table (the BLASes stay cached).
- Scenes: `lava-lamp`, `nighthawks`, `audio-equalizer` (Scene → Audio on; object i and area light i follow band i mod 3, so the bars are ordered bass, mid, treble).
- Scripting: `get(key)` (the setting's JSON text). Text-to-scene: `Prefs::ai_profiles` and the *Backend* row in Describe a scene.

