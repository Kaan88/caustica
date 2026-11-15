## Physics and puzzle additions (track 17, 2026-11-02)

Removed 2026-11-15 (roadmap Plan step 3). (The puzzle parts only; the physics notes stay.)

- Wind: a force `wind · gust · mass` set each step (`reset_forces` first), gust = 1 + gusts · (0.6 sin 1.3t + 0.4 sin(3.7t + 1)) on simulated time. Gravity is `down · gravity`.
- `same_bodies` replaces object equality in `Sim::matches`: material and name edits keep the simulation.
- Concave shapes: `decomposed()` runs V-HACD (resolution 48, at most 24 hulls) on the local, scaled mesh soup; cached by kind, mesh path and scale bits.
- Record: `record_physics` keys `position/0`, `position/1`, `elevation`, `rotation`, `tilt/0`, `tilt/1` of every dynamic body on each frame boundary of simulated time and stretches the animation's length.
- Sleep: activation thresholds 0.01 (normalized linear) and 0.08 rad/s, 1.5 s before sleeping.
- Laser: `beam_objects` turns the traced segments into unlit emissive cylinders (radius 0.015, emission 30 × power, at most 20 units each) in the frame's copy only, so picking, physics and the trace never see them.
- Scenes: `wrecking-ball` (one ball joint from the beam, mass 40×), `castle` (154 bodies; walls stop short of the corner towers so nothing starts interpenetrating).

