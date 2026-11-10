//! Walk mode (roadmap 5d; a real first-person walk since item 6's second batch): WASD + mouse
//! look through the orbit camera. The body is a column of spheres above step height that the
//! scene's triangles push out, standing on a ray cast down from the hips: gravity, step-up over
//! ledges and stairs, slopes up to 50°, jump, crouch, sprint and head bob. Fly mode keeps the
//! old free camera (one sphere at the eye). The orbit camera stays the source of truth: walking
//! moves its eye, and looking turns it around the eye instead of the target.
use crate::scene::Settings;
use glam::Vec3;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Body sizes in scene units at body scale 1 (the Cornell box is about 5.5 m tall, so an eye
/// height of 0.62 is a person's), speeds per second.
const RADIUS: f32 = 0.15;
pub const EYE: f32 = 0.62;
pub const CROUCH_EYE: f32 = 0.36;
/// Ledges up to this high are stepped onto; the body spheres start above it.
const STEP: f32 = 0.12;
const SPEED: f32 = 1.2;
const FLY_SPEED: f32 = 1.6;
const GRAVITY: f32 = 4.;
/// Take-off speed: a jump of JUMP² / 2·GRAVITY ≈ 0.2 (half a metre).
const JUMP: f32 = 1.25;
/// Cosine of the steepest walkable slope (50°).
const WALKABLE: f32 = 0.64;
/// Below this the walker gives up falling and switches to fly mode.
const FLOOR_OF_THE_WORLD: f32 = -20.;

pub struct Walk {
    triangles: Vec<[Vec3; 3]>,
    /// The geometry key the triangles were built from.
    key: String,
    /// Free flight (the old walk mode): no gravity, Q / E down / up.
    pub fly: bool,
    pub bob: bool,
    /// Body scale (mouse wheel): sizes, speeds and jumps scale with it.
    pub scale: f32,
    feet: Vec3,
    /// The eye this walker last placed; a different camera (Home, a scene load) resets the feet.
    placed: Option<Vec3>,
    vy: f32,
    grounded: bool,
    /// 0 standing, 1 crouched, eased.
    crouch: f32,
    /// Head bob: stride phase and an amplitude that fades in and out.
    phase: f32,
    sway: f32,
    /// How much of the stride the avatar shows: 1 walking on the ground, eased to 0 standing,
    /// airborne or flying (no bob option involved).
    gait: f32,
    jump_held: bool,
    /// A note for the UI (a toast), taken by `message`.
    note: Option<&'static str>,
}
fn direction(s: &Settings) -> Vec3 {
    let (sy, cy) = s.yaw.to_radians().sin_cos();
    let (sp, cp) = s.pitch.to_radians().sin_cos();
    Vec3::new(sy * cp, sp, cy * cp)
}
fn eye(s: &Settings) -> Vec3 {
    Vec3::from(s.target) + direction(s) * s.distance
}
/// Puts the target in front of `eye` at the camera's distance, for the current yaw and pitch.
fn place(s: &mut Settings, eye: Vec3) {
    s.target = (eye - direction(s) * s.distance)
        .to_array()
        .map(|v| v.clamp(-20., 20.));
}
impl Walk {
    /// Starts walking from the current camera: a short orbit radius, so looking turns in place.
    /// The feet start an eye height below the camera, and gravity does the rest.
    pub fn enter(s: &mut Settings) -> Self {
        let e = eye(s);
        s.distance = 0.5;
        s.rotate = false;
        place(s, e);
        Self {
            triangles: vec![],
            key: String::new(),
            fly: false,
            bob: true,
            scale: 1.,
            feet: e - Vec3::Y * EYE,
            placed: None,
            vy: 0.,
            grounded: false,
            crouch: 0.,
            phase: 0.,
            sway: 0.,
            gait: 0.,
            jump_held: false,
            note: None,
        }
    }
    /// Mouse look: turn around the eye.
    pub fn look(s: &mut Settings, dx: f32, dy: f32) {
        let e = eye(s);
        s.yaw -= dx * 0.2;
        s.pitch = (s.pitch + dy * 0.2).clamp(-85., 85.);
        place(s, e);
    }
    /// Mouse wheel: a bigger or smaller body (0.2–5×).
    pub fn zoom(&mut self, steps: f32) {
        self.scale = (self.scale * (steps * 0.1).exp()).clamp(0.2, 5.);
        self.placed = None;
    }
    pub fn toggle_fly(&mut self) {
        self.fly = !self.fly;
        self.vy = 0.;
        self.grounded = false;
    }
    /// What the avatar is drawn from (`avatar::parts`).
    pub fn stance(&self, s: &Settings) -> crate::avatar::Stance {
        crate::avatar::Stance {
            feet: self.feet,
            eye_height: self.eye_height(),
            yaw: s.yaw,
            pitch: s.pitch,
            phase: self.phase,
            gait: self.gait,
            scale: self.scale,
            crouch: self.crouch,
        }
    }
    pub fn message(&mut self) -> Option<&'static str> {
        self.note.take()
    }
    fn eye_height(&self) -> f32 {
        (EYE + (CROUCH_EYE - EYE) * self.crouch) * self.scale
    }
    /// Moves for one frame from the held keys; returns true when the camera moved.
    pub fn update(
        &mut self,
        s: &mut Settings,
        keys: &HashSet<KeyCode>,
        dt: f32,
        run: bool,
        geometry_key: &str,
    ) -> bool {
        let dt = dt.min(0.05);
        let held = |k: KeyCode| keys.contains(&k);
        let axis =
            |plus: KeyCode, minus: KeyCode| held(plus) as i32 as f32 - held(minus) as i32 as f32;
        let (fwd, side) = (
            axis(KeyCode::KeyW, KeyCode::KeyS),
            axis(KeyCode::KeyD, KeyCode::KeyA),
        );
        let down = held(KeyCode::KeyQ)
            || held(KeyCode::KeyC)
            || held(KeyCode::ControlLeft)
            || held(KeyCode::ControlRight);
        let up = held(KeyCode::KeyE) || held(KeyCode::Space);
        // A still flyer has nothing to do; a walker may still be falling or crouching.
        if self.fly && fwd == 0. && side == 0. && !up && !down {
            self.placed = None;
            return false;
        }
        if self.key != geometry_key {
            let v = crate::scene::geometry(s);
            self.triangles = v
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| t.map(|p| Vec3::from(p.position)))
                .collect();
            self.key = geometry_key.to_string();
        }
        let start = eye(s);
        if self.placed.is_none_or(|p| (p - start).length() > 1e-4) {
            // Someone else moved the camera: stand under it, or fly when nothing is below.
            self.feet = start - Vec3::Y * self.eye_height();
            self.vy = 0.;
            self.grounded = false;
            if !self.fly && ground(&self.triangles, start, start.y - FLOOR_OF_THE_WORLD).is_none() {
                self.fly = true;
                self.note = Some("Nothing to stand on below: fly mode on (F to walk)");
            }
        }
        let k = self.scale;
        let view = -direction(s);
        let forward = Vec3::new(view.x, 0., view.z).normalize_or_zero();
        let right = forward.cross(Vec3::Y);
        let r = RADIUS * k;
        let new_eye = if self.fly {
            let vertical = up as i32 as f32 - down as i32 as f32;
            let velocity = (forward * fwd + right * side + Vec3::Y * vertical).normalize_or_zero()
                * FLY_SPEED
                * k
                * if run { 3. } else { 1. };
            self.gait = 0.;
            let mut e = start + velocity * dt;
            let near = self.near(e, e, r);
            for _ in 0..2 {
                e += push(&near, e, r);
            }
            self.feet = e - Vec3::Y * self.eye_height();
            e
        } else {
            // Crouch, but stand up only where the head fits.
            let want = down as i32 as f32;
            let stand_head = self.feet + Vec3::Y * (EYE * k);
            let blocked = want < self.crouch
                && overlaps(&self.near(stand_head, stand_head, r), stand_head, r * 0.9);
            if !blocked {
                self.crouch += (want - self.crouch) * (1. - (-12. * dt).exp());
                if (want - self.crouch).abs() < 1e-3 {
                    self.crouch = want;
                }
            }
            let speed = SPEED
                * k
                * if self.crouch > 0.5 {
                    0.45
                } else if run {
                    2.2
                } else {
                    1.
                };
            let step = (forward * fwd + right * side).normalize_or_zero() * speed * dt;
            if held(KeyCode::Space) && !self.jump_held && self.grounded && self.crouch < 0.5 {
                self.vy = JUMP * k;
                self.grounded = false;
            }
            self.jump_held = held(KeyCode::Space);
            if !self.grounded {
                self.vy -= GRAVITY * k * dt;
            }
            let from = self.feet;
            self.feet += step + Vec3::Y * (self.vy * dt);
            // The body: spheres from just above step height to the top of the head.
            let top = self.eye_height() + 0.06 * k;
            let low = STEP * k + r;
            let count = ((top - r - low) / r).ceil().max(1.) as usize;
            let centre = |feet: Vec3, i: usize| {
                feet + Vec3::Y * (low + (top - r - low).max(0.) * i as f32 / count as f32)
            };
            let near = self.near(
                from.min(self.feet) - Vec3::Y * (STEP * 2. * k),
                from.max(self.feet) + Vec3::Y * top,
                r,
            );
            for _ in 0..3 {
                let mut moved = Vec3::ZERO;
                for i in 0..=count {
                    let d = push(&near, centre(self.feet + moved, i), r);
                    moved += d;
                }
                if moved.y < 0. && self.vy > 0. {
                    self.vy = 0.; // Head against a ceiling.
                }
                // Standing, the body only slides sideways; the ground ray holds it up.
                if self.grounded {
                    moved.y = moved.y.min(0.);
                }
                self.feet += moved;
            }
            // The ground: a ray down from step height (falling, from where the feet were). Grounded,
            // it reaches a step below the feet too, so stairs and slopes down are followed
            // instead of fallen down.
            let above = STEP * k + (-self.vy * dt).max(0.);
            let reach = above + if self.grounded { STEP * k } else { 1e-3 };
            let origin = self.feet + Vec3::Y * above;
            match ground(&near, origin, reach) {
                Some(y) if self.vy <= 0. => {
                    self.feet.y = y;
                    self.vy = 0.;
                    self.grounded = true;
                }
                _ => self.grounded = false,
            }
            if self.feet.y < FLOOR_OF_THE_WORLD {
                self.feet.y = FLOOR_OF_THE_WORLD;
                self.fly = true;
                self.vy = 0.;
                self.note = Some("Nothing to stand on: fly mode on (F to walk)");
            }
            // Head bob: one dip per stride, faded in while walking on the ground.
            let walked = Vec3::new(self.feet.x - from.x, 0., self.feet.z - from.z).length();
            let target = (self.bob && self.grounded && walked > 1e-5) as i32 as f32;
            self.sway += (target - self.sway) * (1. - (-8. * dt).exp());
            if self.sway < 0.01 && target == 0. {
                self.sway = 0.;
            }
            let striding = (self.grounded && walked > 1e-5) as i32 as f32;
            self.gait += (striding - self.gait) * (1. - (-10. * dt).exp());
            if self.gait < 0.01 && striding == 0. {
                self.gait = 0.;
            }
            self.phase += walked / (0.42 * k) * std::f32::consts::PI;
            let bob = self.sway * 0.012 * k * (2. * self.phase).sin();
            self.feet + Vec3::Y * (self.eye_height() + bob)
        };
        let moved = (new_eye - start).length() > 1e-5;
        if moved {
            place(s, new_eye);
        }
        // The target is clamped to ±20, so the eye may have stopped short: follow it.
        let actual = eye(s);
        if moved {
            self.feet += actual - new_eye;
        }
        self.placed = Some(actual);
        moved
    }
    /// Triangles whose boxes touch the box from `lo` to `hi` grown by `r`.
    fn near(&self, lo: Vec3, hi: Vec3, r: f32) -> Vec<[Vec3; 3]> {
        let (lo, hi) = (lo - Vec3::splat(r), hi + Vec3::splat(r));
        self.triangles
            .iter()
            .filter(|t| {
                let (a, b) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
                a.cmple(hi).all() && b.cmpge(lo).all()
            })
            .copied()
            .collect()
    }
}
/// How far to move a sphere at `c` out of the triangles it overlaps (applied one by one, so
/// corners settle over a few calls).
fn push(triangles: &[[Vec3; 3]], c: Vec3, r: f32) -> Vec3 {
    let mut p = c;
    for t in triangles {
        let d = p - closest_point(p, t);
        let len = d.length();
        if len < r && len > 1e-6 {
            p += d / len * (r - len);
        }
    }
    p - c
}
fn overlaps(triangles: &[[Vec3; 3]], c: Vec3, r: f32) -> bool {
    triangles
        .iter()
        .any(|t| (c - closest_point(c, t)).length_squared() < r * r)
}
/// The highest walkable surface a ray from `origin` straight down meets within `reach`.
fn ground(triangles: &[[Vec3; 3]], origin: Vec3, reach: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in triangles {
        let n = (t[1] - t[0]).cross(t[2] - t[0]);
        let len = n.length();
        if len < 1e-12 || n.y.abs() / len < WALKABLE {
            continue;
        }
        // Möller–Trumbore with the direction −Y.
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = Vec3::NEG_Y.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = origin - t[0];
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = Vec3::NEG_Y.dot(q) / det;
        let d = e2.dot(q) / det;
        if u >= 0. && v >= 0. && u + v <= 1. && (0. ..=reach).contains(&d) {
            best = Some(best.map_or(d, |b| b.min(d)));
        }
    }
    best.map(|d| origin.y - d)
}
/// Closest point on a triangle (Ericson, Real-Time Collision Detection 5.1.5).
fn closest_point(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0. && d2 <= 0. {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0. && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0. && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1. / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// An axis-aligned box's 12 triangles.
    fn cuboid(lo: Vec3, hi: Vec3) -> Vec<[Vec3; 3]> {
        let c = |i: usize| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        };
        let faces = [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ];
        faces
            .iter()
            .flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
            .collect()
    }
    /// A walker standing at (0, 0, 0) facing −z on a big floor, with extra boxes.
    fn world(boxes: &[(Vec3, Vec3)]) -> (Settings, Walk) {
        let mut s = Settings {
            yaw: 0.,
            pitch: 0.,
            distance: 4.,
            target: [0., 0., 0.],
            ..Settings::default()
        };
        let mut w = Walk::enter(&mut s);
        w.triangles = cuboid(Vec3::new(-10., -1., -10.), Vec3::new(10., 0., 10.));
        for (lo, hi) in boxes {
            w.triangles.extend(cuboid(*lo, *hi));
        }
        w.key = "k".into();
        // Stand at the origin.
        place(&mut s, Vec3::Y * EYE);
        (s, w)
    }
    fn run(s: &mut Settings, w: &mut Walk, keys: &[KeyCode], seconds: f32) {
        let keys: HashSet<KeyCode> = keys.iter().copied().collect();
        for _ in 0..(seconds * 100.) as usize {
            w.update(s, &keys, 0.01, false, "k");
        }
    }
    #[test]
    fn walking_into_a_wall_stops_short_of_it() {
        let mut s = Settings::default();
        let mut w = Walk::enter(&mut s);
        let keys: HashSet<KeyCode> = [KeyCode::KeyW].into();
        // Face the back wall of the Cornell box (z = -1) and walk for five seconds.
        for _ in 0..500 {
            w.update(&mut s, &keys, 0.01, false, "k");
        }
        let e = eye(&s);
        assert!(e.z > -1. + RADIUS * 0.9, "walked through the wall: {e}");
        assert!(e.z < 0.5, "did not move: {e}");
        // The default camera is outside the box, over nothing: it flies.
        assert!(w.fly && (e.y - 1.).abs() < 1., "{e}");
        assert!(
            (closest_point(Vec3::new(0., 1., 0.), &[Vec3::ZERO, Vec3::X, Vec3::Z]) - Vec3::ZERO)
                .length()
                < 1e-6
        );
    }
    #[test]
    fn falls_steps_up_ledges_and_stops_at_walls() {
        // A low step at z < −1 and a tall block at z < −3.
        let (mut s, mut w) = world(&[
            (Vec3::new(-2., 0., -3.), Vec3::new(2., 0.1, -1.)),
            (Vec3::new(-2., 0., -6.), Vec3::new(2., 0.5, -3.)),
        ]);
        // Dropped from a height, the walker lands and stands at eye height.
        place(&mut s, Vec3::new(0., 2., 0.));
        run(&mut s, &mut w, &[], 2.);
        assert!((eye(&s).y - EYE).abs() < 1e-3, "{}", eye(&s));
        // Standing still does not move the camera (no accumulation restarts).
        let before = s.target;
        run(&mut s, &mut w, &[], 0.5);
        assert_eq!(before, s.target);
        // Onto the step, then stopped by the tall block.
        run(&mut s, &mut w, &[KeyCode::KeyW], 5.);
        let e = eye(&s);
        assert!((e.y - (0.1 + EYE)).abs() < 0.02, "not on the step: {e}");
        assert!(e.z > -3. + RADIUS * 0.9, "walked into the block: {e}");
    }
    #[test]
    fn jumps_land_and_crouching_lowers_the_eye() {
        let (mut s, mut w) = world(&[]);
        run(&mut s, &mut w, &[], 0.2);
        let mut peak: f32 = 0.;
        let keys: HashSet<KeyCode> = [KeyCode::Space].into();
        for _ in 0..150 {
            w.update(&mut s, &keys, 0.01, false, "k");
            peak = peak.max(eye(&s).y);
        }
        let height = JUMP * JUMP / (2. * GRAVITY);
        assert!(
            (peak - EYE - height).abs() < 0.03,
            "jumped {} not {height}",
            peak - EYE
        );
        // Holding Space does not jump again; it lands.
        assert!((eye(&s).y - EYE).abs() < 1e-3);
        run(&mut s, &mut w, &[KeyCode::KeyC], 1.);
        assert!((eye(&s).y - CROUCH_EYE).abs() < 1e-3);
        // Fly mode floats.
        w.toggle_fly();
        run(&mut s, &mut w, &[KeyCode::KeyE], 1.);
        assert!(eye(&s).y > CROUCH_EYE + 1.);
    }
}
