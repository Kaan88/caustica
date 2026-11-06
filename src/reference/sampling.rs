//! Random numbers and direction sampling.
use glam::Vec3;
use std::f32::consts::PI;

/// PCG32 (O'Neill), seeded per pixel and sample.
pub(super) struct Rng(u64);
impl Rng {
    pub(super) fn new(pixel: u64, sample: u64) -> Self {
        // splitmix64 of both, so neighbouring pixels and samples start uncorrelated.
        let mix = |mut z: u64| {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut r = Self(mix(mix(pixel) ^ sample.wrapping_mul(0xd1b5_4a32_d192_ed03)));
        r.next();
        r
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        self.0 = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let x = (((old >> 18) ^ old) >> 27) as u32;
        x.rotate_right((old >> 59) as u32)
    }
    pub(super) fn f(&mut self) -> f32 {
        (self.next() >> 8) as f32 * (1. / 16_777_216.)
    }
}
pub(super) fn basis(n: Vec3, l: Vec3) -> Vec3 {
    let (t, b) = n.any_orthonormal_pair();
    t * l.x + b * l.y + n * l.z
}
pub(super) fn cosine_sample(n: Vec3, r: &mut Rng) -> Vec3 {
    let (q, a) = (r.f().sqrt(), 2. * PI * r.f());
    basis(
        n,
        Vec3::new(q * a.cos(), q * a.sin(), (1. - q * q).max(0.).sqrt()),
    )
}
pub(super) fn reflect(d: Vec3, n: Vec3) -> Vec3 {
    d - 2. * d.dot(n) * n
}
pub(super) fn refract(d: Vec3, n: Vec3, eta: f32) -> Vec3 {
    let c = -d.dot(n);
    let k = 1. - eta * eta * (1. - c * c);
    if k < 0. {
        return Vec3::ZERO;
    }
    eta * d + (eta * c - k.sqrt()) * n
}
/// GGX normal-distribution sample (`ggx_sample`), reflected.
pub(super) fn ggx_sample(n: Vec3, d: Vec3, a: f32, r: &mut Rng) -> Vec3 {
    let (u, phi) = (r.f(), 2. * PI * r.f());
    let c = ((1. - u) / (1. + (a * a - 1.) * u)).sqrt();
    let s = (1. - c * c).max(0.).sqrt();
    reflect(d, basis(n, Vec3::new(s * phi.cos(), s * phi.sin(), c)))
}
