//! Camera rays: the pixel filter's offset and the ray through a film point.
use super::sampling::Rng;
use super::scene::Scene;
use crate::scene::Settings;
use glam::{Vec2, Vec3, Vec4};
use std::f32::consts::PI;

pub(super) fn erfinv(x: f32) -> f32 {
    let mut w = -((1. - x) * (1. + x)).max(1e-30).ln();
    let p = if w < 5. {
        w -= 2.5;
        [
            2.810_226_4e-8,
            3.432_739_4e-7,
            -3.523_387_7e-6,
            -4.391_506_5e-6,
            0.000_218_580_87,
            -0.001_253_725,
            -0.004_177_681_6,
            0.246_640_73,
            1.501_409_4,
        ]
        .iter()
        .fold(0., |p, c| c + p * w)
    } else {
        w = w.sqrt() - 3.;
        [
            -0.000_200_214_26,
            0.000_100_950_56,
            0.001_349_343_2,
            -0.003_673_428_4,
            0.005_739_507_7,
            -0.007_622_461,
            0.009_438_870_5,
            1.001_674,
            2.832_976_8,
        ]
        .iter()
        .fold(0., |p, c| c + p * w)
    };
    p * x
}
pub(super) fn blackman_harris_inverse(u: f32) -> f32 {
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..16 {
        let t = 0.5 * (lo + hi);
        let c = (0.35875 * t - 0.48829 * (2. * PI * t).sin() / (2. * PI)
            + 0.14128 * (4. * PI * t).sin() / (4. * PI)
            - 0.01168 * (6. * PI * t).sin() / (6. * PI))
            / 0.35875;
        if c < u {
            lo = t;
        } else {
            hi = t;
        }
    }
    0.5 * (lo + hi)
}
/// The pixel filter's offset for uniform u (`filter_offset`).
pub(super) fn filter_offset(s: &Settings, u: Vec2) -> Vec2 {
    let w = s.filter_width;
    match s.pixel_filter {
        1 => (Vec2::new(erfinv(2. * u.x - 1.), erfinv(2. * u.y - 1.))
            * (w / 6. * std::f32::consts::SQRT_2))
            .clamp(Vec2::splat(-0.5 * w), Vec2::splat(0.5 * w)),
        2 => (Vec2::new(blackman_harris_inverse(u.x), blackman_harris_inverse(u.y)) - 0.5) * w,
        _ => (u - 0.5) * w,
    }
}

/// The camera's frame as the GPU reads it: world_view's rows (right, up, -forward), the
/// aspect, the tangent of the half field of view and the orthographic half height.
#[derive(Clone, Copy, Default)]
pub(super) struct Frame {
    pub(super) right: Vec3,
    pub(super) up: Vec3,
    pub(super) forward: Vec3,
    pub(super) aspect: f32,
    pub(super) tan_half: f32,
    pub(super) ortho_half: f32,
    pub(super) projection: u32,
    /// The thin lens: aperture radius (0 = pinhole), focus distance, blades (0 to 2 = round),
    /// blade rotation and tilt (radians) and the cat-eye strength.
    pub(super) aperture: f32,
    pub(super) focus: f32,
    pub(super) blades: u32,
    pub(super) rotation: f32,
    pub(super) cat_eye: f32,
    pub(super) tilt: f32,
}

/// A point in the unit aperture for uniform numbers drawn from `r`: the disk, or a regular
/// polygon of `blades` sides rotated by `rotation`, uniform over its area either way.
pub(super) fn aperture_point(blades: u32, rotation: f32, r: &mut Rng) -> Vec2 {
    if blades <= 2 {
        let (q, a) = (r.f().sqrt(), 2. * PI * r.f());
        return q * Vec2::from_angle(a);
    }
    // Pick one of the equal triangles from the centre, then a uniform point in it.
    let n = blades as f32;
    let k = (r.f() * n).floor();
    let edge = |i: f32| Vec2::from_angle(rotation + 2. * PI * i / n);
    let (mut u, mut v) = (r.f(), r.f());
    if u + v > 1. {
        (u, v) = (1. - u, 1. - v);
    }
    u * edge(k) + v * edge(k + 1.)
}

impl Scene<'_> {
    /// The camera ray through film point `pixel`, origin and direction, for each projection
    /// (`camera_ray`), or None for a sample the cat-eye cuts off (it still counts, as black).
    /// Only the perspective and tilt-shift projections take the thin lens; the pinhole draws no
    /// random numbers. Lens systems are refused in `unsupported`.
    pub(super) fn camera_ray(&self, pixel: Vec2, r: &mut Rng) -> Option<(Vec3, Vec3)> {
        let f = &self.frame;
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let ndc = Vec2::new(ndc.x, -ndc.y);
        Some(match f.projection {
            1 => (
                self.eye + (f.right * ndc.x * f.aspect + f.up * ndc.y) * f.ortho_half,
                f.forward,
            ),
            2 => {
                let (phi, theta) = (ndc.x * PI, ndc.y * PI * 0.5);
                let d = f.right * phi.sin() * theta.cos()
                    + f.up * theta.sin()
                    + f.forward * phi.cos() * theta.cos();
                (self.eye, d.normalize())
            }
            3 => {
                // Stereographic, looking straight down: the ground in the middle, the sky around.
                let fh = (Vec3::new(f.forward.x, 0., f.forward.z) + 1e-5).normalize();
                let rh = fh.cross(Vec3::Y).normalize();
                let q = Vec2::new(ndc.x * f.aspect, ndc.y) * f.tan_half * 2.;
                let (qq, s) = (q.dot(q), 1. + q.dot(q));
                let d = rh * (2. * q.x / s) + fh * (2. * q.y / s) - Vec3::Y * ((1. - qq) / s);
                (self.eye, d.normalize())
            }
            _ => {
                let d = self.camera_direction(pixel);
                if f.aperture <= 0. {
                    return Some((self.eye, d));
                }
                // Everything on the focal plane (tilted about the right axis for tilt-shift)
                // stays sharp: every lens point aims at where the pinhole ray meets it.
                let n = if f.projection == 4 {
                    (f.forward * f.tilt.cos() + f.up * f.tilt.sin()).normalize()
                } else {
                    f.forward
                };
                let focal = self.eye + d * (f.focus * f.forward.dot(n) / d.dot(n).max(1e-4));
                let l = aperture_point(f.blades, f.rotation, r);
                // Cat-eye: the barrel cuts the aperture with a unit circle shifted toward the
                // frame's edge.
                let shift = ndc * Vec2::new(f.aspect, 1.) * (f.cat_eye * 0.7);
                if f.cat_eye > 0. && l.distance(shift) > 1. {
                    return None;
                }
                let o = self.eye + (f.right * l.x + f.up * l.y) * f.aperture;
                (o, (focal - o).normalize())
            }
        })
    }
    pub(super) fn camera_direction(&self, pixel: Vec2) -> Vec3 {
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let p = self.inverse_vp * Vec4::new(ndc.x, -ndc.y, 1., 1.);
        (p.truncate() / p.w - self.eye).normalize()
    }
}
