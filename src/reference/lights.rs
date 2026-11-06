//! Light sampling: emissive triangles by power.
use super::bsdf::{emitted, material_at};
use super::bvh::Tri;
use super::sampling::Rng;
use super::scene::Scene;
use glam::Vec3;

/// How far a shadow ray goes toward a point `dist` away on an emitter whose normal it meets at
/// cosine `c`. The ray's origin is offset by 0.0003, so the ray can run up to that much nearer the
/// emitter's plane than the sampled line and cross it 0.0003 / |c| before the point: the stop is
/// twice that off the plane, measured along the normal. Near-tangent rays and rays to nearby
/// points cap it, so it can't skip a real occluder.
pub(super) fn shadow_limit(dist: f32, c: f32) -> f32 {
    (dist - (0.5 * dist).min(0.0006 / c.abs().max(0.02))).max(0.0002)
}

impl Scene<'_> {
    pub(super) fn area(t: &Tri) -> f32 {
        (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5
    }
    /// The solid-angle pdf of reaching emitter triangle `tri` at `at` from `from`.
    pub(super) fn light_pdf(&self, tri: usize, from: Vec3, at: Vec3) -> f32 {
        let Ok(k) = self.lights.binary_search(&tri) else {
            return 0.;
        };
        let p = self.cdf[k] - if k > 0 { self.cdf[k - 1] } else { 0. };
        let t = &self.bvh.tris[tri];
        let ng = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize();
        let d = at - from;
        let c = ng.dot(d.normalize()).abs();
        p * d.length_squared() / (Self::area(t) * c).max(1e-12)
    }
    /// One emitter point: direction, radiance arriving, solid-angle pdf, shadow-ray length.
    pub(super) fn sample_light(&self, x: Vec3, r: &mut Rng) -> Option<(Vec3, Vec3, f32, f32)> {
        if self.lights.is_empty() {
            return None;
        }
        let u = r.f();
        let k = self
            .cdf
            .partition_point(|c| *c <= u)
            .min(self.lights.len() - 1);
        let tri = self.lights[k];
        let t = &self.bvh.tris[tri];
        let (mut a, mut b) = (r.f(), r.f());
        if a + b > 1. {
            (a, b) = (1. - a, 1. - b);
        }
        let p = t.p[0] * (1. - a - b) + t.p[1] * a + t.p[2] * b;
        let d = p - x;
        let dist = d.length();
        if dist < 1e-6 {
            return None;
        }
        let ld = d / dist;
        let (_, n) = t.normals(a, b);
        let front = ld.dot(n) < 0.;
        let m = material_at(&self.materials[t.material as usize], p);
        // The spot's cosine is signed against the light's normal (`spot(lm, dot(ln, -ld))`).
        let le = emitted(&m, front, (-ld).dot(n));
        let pdf = self.light_pdf(tri, x, p);
        (pdf > 0. && le.max_element() > 0.).then_some((ld, le, pdf, shadow_limit(dist, ld.dot(n))))
    }
}
