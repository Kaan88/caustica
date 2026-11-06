//! Camera rays: the pixel filter's offset and the ray through a film point.
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

impl Scene<'_> {
    pub(super) fn camera_direction(&self, pixel: Vec2) -> Vec3 {
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let p = self.inverse_vp * Vec4::new(ndc.x, -ndc.y, 1., 1.);
        (p.truncate() / p.w - self.eye).normalize()
    }
}
