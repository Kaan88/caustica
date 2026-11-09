//! Light sampling: the environment by its share, else emissive triangles by power.
use super::bsdf::{emitted, material_at};
use super::bvh::Tri;
use super::sampling::{Rng, basis};
use super::scene::{Light, Scene};
use glam::Vec3;
use std::f32::consts::PI;

/// How far a shadow ray goes toward a point `dist` away on an emitter whose normal it meets at
/// cosine `c`. The ray's origin is offset by 0.0003, so the ray can run up to that much nearer the
/// emitter's plane than the sampled line and cross it 0.0003 / |c| before the point: the stop is
/// twice that off the plane, measured along the normal. Near-tangent rays and rays to nearby
/// points cap it, so it can't skip a real occluder.
pub(super) fn shadow_limit(dist: f32, c: f32) -> f32 {
    (dist - (0.5 * dist).min(0.0006 / c.abs().max(0.02))).max(0.0002)
}

/// 1 − cos of the half angle a sphere of radius `r` covers from `d2` away (squared), 0 from
/// inside; written so it keeps its precision for far spheres.
pub(super) fn sphere_om(d2: f32, r: f32) -> f32 {
    let s2 = r * r / d2;
    if d2 > r * r {
        s2 / (1. + (1. - s2).sqrt())
    } else {
        0.
    }
}

/// A direction uniform in the cone around `axis` of solid angle 2π `om`, and its pdf.
pub(super) fn cone_sample(axis: Vec3, om: f32, r: &mut Rng) -> (Vec3, f32) {
    let x = r.f() * om;
    let s = (x * (2. - x)).max(0.).sqrt();
    let a = 2. * PI * r.f();
    (
        basis(axis, Vec3::new(s * a.cos(), s * a.sin(), 1. - x)),
        1. / (2. * PI * om),
    )
}

impl Scene<'_> {
    pub(super) fn area(t: &Tri) -> f32 {
        (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5
    }
    /// The probability of picking entry `k` of the light table, the environment's share taken.
    fn pick(&self, k: usize) -> f32 {
        (1. - self.p_env) * (self.cdf[k] - if k > 0 { self.cdf[k - 1] } else { 0. })
    }
    /// The solid-angle pdf of light sampling reaching emitter triangle `tri` at `at` from `from`
    /// (a sphere light's mesh: the sphere's cone, 0 from inside it).
    pub(super) fn light_pdf(&self, tri: usize, from: Vec3, at: Vec3) -> f32 {
        let Some(k) = self.tri_light.get(tri).and_then(|e| e.checked_sub(1)) else {
            return 0.;
        };
        let k = k as usize;
        if let Light::Sphere { c, r, .. } = self.lights[k] {
            let om = sphere_om((c - from).length_squared(), r);
            return if om > 0. {
                self.pick(k) / (2. * PI * om)
            } else {
                0.
            };
        }
        let p = self.pick(k);
        let t = &self.bvh.tris[tri];
        let ng = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize();
        let d = at - from;
        let c = ng.dot(d.normalize()).abs();
        p * d.length_squared() / (Self::area(t) * c).max(1e-12)
    }
    /// One light sample: direction, radiance arriving, solid-angle pdf, shadow-ray length.
    pub(super) fn sample_light(&self, x: Vec3, r: &mut Rng) -> Option<(Vec3, Vec3, f32, f32)> {
        if let Some(env) = &self.env
            && r.f() < self.p_env
        {
            let (ld, pe) = env.sample(r);
            return (pe > 0.).then(|| (ld, env.radiance(ld), self.p_env * pe, 1e4));
        }
        if self.lights.is_empty() {
            return None;
        }
        let u = r.f();
        let k = self
            .cdf
            .partition_point(|c| *c <= u)
            .min(self.lights.len() - 1);
        let tri = match self.lights[k] {
            Light::Tri(tri) => tri,
            // A direction in the cone the sphere covers, and a shadow ray up to its true surface
            // (the inscribed mesh lies beyond it). From inside, no sample.
            Light::Sphere { c, r: radius, le } => {
                let c = c - x;
                let d2 = c.length_squared();
                let om = sphere_om(d2, radius);
                if om <= 0. {
                    return None;
                }
                let (ld, pc) = cone_sample(c / d2.sqrt(), om, r);
                let b = c.dot(ld);
                let limit = b - (radius * radius - (d2 - b * b)).max(0.).sqrt();
                return Some((ld, le, self.pick(k) * pc, (limit - 0.0006).max(0.0002)));
            }
        };
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
