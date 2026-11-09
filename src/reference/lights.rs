//! Light sampling: the environment by its share, else emissive procedural objects and emissive
//! triangles (half and half when there are both), each by power.
use super::bsdf::{emitted, material_at, rgb};
use super::bvh::Tri;
use super::sampling::{Rng, basis};
use super::scene::{Light, Ray, Scene};
use glam::{Mat4, Vec2, Vec3};
use std::f32::consts::PI;

/// An emissive procedural object as light sampling sees it: the sphere bounding its unit cube.
/// A sample picks a direction in the cone that sphere covers (every direction from inside it)
/// and counts only when the first surface that way is the object itself, so the bound only
/// shapes the noise, never the mean.
pub(super) struct SdfLight {
    /// Its index in `Scene::sdfs`.
    pub(super) object: usize,
    pub(super) centre: Vec3,
    pub(super) radius: f32,
    /// The power CDF over the records (the last entry exactly 1).
    pub(super) cdf: f32,
}
impl SdfLight {
    /// The records of the emissive objects, by power: the brightest channel's emission times the
    /// bounding sphere's cross-section, as the GPU weighs them (`sdf_emitter_records`).
    pub(super) fn of(
        sdfs: &[super::sdf::SdfObject],
        materials: &[crate::lighting::PackedMaterial],
    ) -> Vec<Self> {
        let mut out: Vec<(Self, f32)> = vec![];
        for (object, o) in sdfs.iter().enumerate() {
            let m = &materials[o.material as usize];
            let emission = rgb(m.color_rough).max_element() * m.optical[1];
            if emission <= 0. {
                continue;
            }
            let o2w: Mat4 = o.w2o.inverse();
            let centre = o2w.transform_point3(Vec3::ZERO);
            let radius = (0..8)
                .map(|k| {
                    let corner = Vec3::new(
                        if k & 1 == 0 { -1. } else { 1. },
                        if k & 2 == 0 { -1. } else { 1. },
                        if k & 4 == 0 { -1. } else { 1. },
                    );
                    o2w.transform_point3(corner).distance(centre)
                })
                .fold(0f32, f32::max);
            if radius > 0. && radius.is_finite() {
                let power = emission * radius * radius;
                out.push((
                    Self {
                        object,
                        centre,
                        radius,
                        cdf: 0.,
                    },
                    power,
                ));
            }
        }
        let total: f32 = out.iter().map(|o| o.1).sum();
        let mut sum = 0.;
        let n = out.len();
        out.into_iter()
            .enumerate()
            .map(|(i, (mut l, power))| {
                sum += power / total;
                l.cdf = if i + 1 == n { 1. } else { sum };
                l
            })
            .collect()
    }
}

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
    /// The probability of picking entry `k` of the light table, the environment's and the
    /// procedural emitters' shares taken.
    fn pick(&self, k: usize) -> f32 {
        (1. - self.p_env)
            * (1. - self.sdf_share)
            * (self.cdf[k] - if k > 0 { self.cdf[k - 1] } else { 0. })
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
    /// What the suns add along `d`, a ray that left the scene: each one whose disc holds `d`
    /// (tested by sin², from a cross product: a float cosine can't resolve a 0.05° sun), times
    /// `weight` of its light-sampling pdf. Hidden suns only from the first bounce on.
    pub(super) fn suns(&self, d: Vec3, bounce: u32, weight: impl Fn(f32) -> f32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        for (k, l) in self.lights.iter().enumerate() {
            let Light::Sun {
                d: c,
                om,
                le,
                hidden,
            } = *l
            else {
                continue;
            };
            let x = d.cross(c);
            if d.dot(c) <= 0. || x.dot(x) > om * (2. - om) || bounce == 0 && hidden {
                continue;
            }
            sum += le * weight(self.pick(k) / (2. * PI * om));
        }
        sum
    }
    /// One light sample: direction, radiance arriving, solid-angle pdf, shadow-ray length.
    pub(super) fn sample_light(&self, x: Vec3, r: &mut Rng) -> Option<(Vec3, Vec3, f32, f32)> {
        if let Some(env) = &self.env
            && r.f() < self.p_env
        {
            let (ld, pe) = env.sample(r);
            return (pe > 0.).then(|| (ld, env.radiance(ld), self.p_env * pe, 1e4));
        }
        if !self.sdf_lights.is_empty() && (self.lights.is_empty() || r.f() < 0.5) {
            return self.sample_sdf(x, r);
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
            // A direction in its disc; the shadow ray leaves the scene.
            Light::Sun { d, om, le, .. } => {
                let (ld, pc) = cone_sample(d, om, r);
                return Some((ld, le, self.pick(k) * pc, 1e4 - 0.0006));
            }
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
        let le = emitted(
            &m,
            self.emission_colour(&m, tri, Vec2::new(a, b)),
            front,
            (-ld).dot(n),
        );
        let pdf = self.light_pdf(tri, x, p);
        (pdf > 0. && le.max_element() > 0.).then_some((ld, le, pdf, shadow_limit(dist, ld.dot(n))))
    }
    /// The probability light sampling picks procedural emitter record `k`.
    fn sdf_prob(&self, k: usize) -> f32 {
        self.sdf_lights[k].cdf
            - if k > 0 {
                self.sdf_lights[k - 1].cdf
            } else {
                0.
            }
    }
    /// The solid-angle pdf of the direction toward record `k` from `x`: uniform in its cone, or
    /// over the whole sphere from inside its bound.
    fn sdf_dir_pdf(&self, k: usize, x: Vec3) -> f32 {
        let l = &self.sdf_lights[k];
        let om = sphere_om(l.centre.distance_squared(x), l.radius);
        if om > 0. {
            1. / (2. * PI * om)
        } else {
            1. / (4. * PI)
        }
    }
    /// The solid-angle pdf light sampling gives a direction from `from` that first meets
    /// procedural object `object` (0 when that object isn't a sampled emitter).
    pub(super) fn sdf_light_pdf(&self, object: usize, from: Vec3) -> f32 {
        let Some(k) = self.sdf_lights.iter().position(|l| l.object == object) else {
            return 0.;
        };
        (1. - self.p_env) * self.sdf_share * self.sdf_prob(k) * self.sdf_dir_pdf(k, from)
    }
    /// A light sample of an emissive procedural object: one by power, a direction in its bound's
    /// cone, kept when the nearest surface a shadow ray meets that way is the object.
    fn sample_sdf(&self, x: Vec3, r: &mut Rng) -> Option<(Vec3, Vec3, f32, f32)> {
        let u = r.f();
        let k = self
            .sdf_lights
            .partition_point(|l| l.cdf <= u)
            .min(self.sdf_lights.len() - 1);
        let l = &self.sdf_lights[k];
        let c = l.centre - x;
        let om = sphere_om(c.length_squared(), l.radius);
        let ld = if om > 0. {
            let x0 = r.f() * om;
            let s0 = (x0 * (2. - x0)).max(0.).sqrt();
            let a = 2. * PI * r.f();
            basis(
                c.normalize(),
                Vec3::new(s0 * a.cos(), s0 * a.sin(), 1. - x0),
            )
        } else {
            let z = 1. - 2. * r.f();
            let s0 = (1. - z * z).max(0.).sqrt();
            let a = 2. * PI * r.f();
            Vec3::new(s0 * a.cos(), z, s0 * a.sin())
        };
        let h = self.intersect(x, ld, 1e4, &self.accept(Ray::Shadow, x, ld), false)?;
        if h.sdf != Some(l.object) {
            return None;
        }
        let sf = self.surface(&h, x, ld);
        let m = material_at(&self.materials[sf.material as usize], sf.p);
        let le = emitted(&m, rgb(m.color_rough), sf.front, sf.emission_cos(ld));
        let pl = self.sdf_light_pdf(l.object, x);
        // The surface's own normal (`ng` faces the ray) sets the stop short of it.
        (pl > 0. && le.max_element() > 0.).then_some((ld, le, pl, shadow_limit(h.t, ld.dot(sf.ng))))
    }
}
