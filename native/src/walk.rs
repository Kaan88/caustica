//! Walk mode (roadmap 5d): WASD + mouse look through the orbit camera, with sphere collision
//! against the scene's triangles. The orbit camera stays the source of truth: walking moves its
//! eye, and looking turns it around the eye instead of the target.
use crate::scene::Settings;
use glam::Vec3;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Collision radius and speeds in scene units (per second).
const RADIUS: f32 = 0.22;
const SPEED: f32 = 1.6;

pub struct Walk {
    triangles: Vec<[Vec3; 3]>,
    /// The geometry key the triangles were built from.
    key: String,
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
    pub fn enter(s: &mut Settings) -> Self {
        let e = eye(s);
        s.distance = 0.5;
        s.rotate = false;
        place(s, e);
        Self {
            triangles: vec![],
            key: String::new(),
        }
    }
    /// Mouse look: turn around the eye.
    pub fn look(s: &mut Settings, dx: f32, dy: f32) {
        let e = eye(s);
        s.yaw += dx * 0.2;
        s.pitch = (s.pitch + dy * 0.2).clamp(-85., 85.);
        place(s, e);
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
        let axis = |plus: KeyCode, minus: KeyCode| {
            keys.contains(&plus) as i32 as f32 - keys.contains(&minus) as i32 as f32
        };
        let (fwd, side, up) = (
            axis(KeyCode::KeyW, KeyCode::KeyS),
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyE, KeyCode::KeyQ),
        );
        if fwd == 0. && side == 0. && up == 0. {
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
        let view = -direction(s);
        let forward = Vec3::new(view.x, 0., view.z).normalize_or_zero();
        let right = forward.cross(Vec3::Y);
        let velocity = (forward * fwd + right * side + Vec3::Y * up).normalize_or_zero()
            * SPEED
            * if run { 3. } else { 1. };
        let mut e = eye(s) + velocity * dt.min(0.1);
        // Push the sphere out of every triangle it overlaps (two passes settle corners).
        for _ in 0..2 {
            for t in &self.triangles {
                let lo = t[0].min(t[1]).min(t[2]) - Vec3::splat(RADIUS);
                let hi = t[0].max(t[1]).max(t[2]) + Vec3::splat(RADIUS);
                if e.cmplt(lo).any() || e.cmpgt(hi).any() {
                    continue;
                }
                let c = closest_point(e, t);
                let d = e - c;
                let len = d.length();
                if len < RADIUS && len > 1e-6 {
                    e += d / len * (RADIUS - len);
                }
            }
        }
        place(s, e);
        true
    }
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
    #[test]
    fn walking_into_a_wall_stops_short_of_it() {
        let mut s = Settings::default();
        let mut w = Walk::enter(&mut s);
        let keys: HashSet<KeyCode> = [KeyCode::KeyW].into();
        // Face the back wall of the Cornell box (z = -1) and walk for two seconds.
        for _ in 0..500 {
            w.update(&mut s, &keys, 0.01, false, "k");
        }
        let e = eye(&s);
        assert!(e.z > -1. + RADIUS * 0.9, "walked through the wall: {e}");
        assert!(e.z < 0.5, "did not move: {e}");
        assert!(
            (closest_point(Vec3::new(0., 1., 0.), &[Vec3::ZERO, Vec3::X, Vec3::Z]) - Vec3::ZERO)
                .length()
                < 1e-6
        );
    }
}
