//! Procedural (SDF) objects: kinds as `scene::Sdf` (`Sdf::id`: kind in the low 8 bits, the
//! detail or phase in thousandths above), each inside the unit cube of its object space. Rays
//! are carried into object space unnormalized, so a hit's t is the world t.
//!
//! The exact sphere, cylinder and cone are solved in closed form; the torus, Mandelbulb, Menger
//! sponge and metaballs sphere-trace their distance field (`field`) from where the ray enters the cube, or the unit sphere for the round
//! ones. The GPU's rules are the same; the code is not a copy of it.
use glam::{Mat3, Mat4, Vec2, Vec3, Vec3Swizzles};

pub(super) const SPHERE: u32 = 1;
pub(super) const TORUS: u32 = 2;
pub(super) const MANDELBULB: u32 = 3;
pub(super) const MENGER: u32 = 4;
pub(super) const METABALLS: u32 = 5;
pub(super) const CYLINDER: u32 = 6;
pub(super) const CONE: u32 = 7;

/// One visible procedural object of the settings.
pub(super) struct SdfObject {
    pub(super) raw: u32,
    pub(super) material: u32,
    pub(super) w2o: Mat4,
    /// The inverse transpose's linear part, for normals.
    normal_matrix: Mat3,
}

impl SdfObject {
    pub(super) fn new(raw: u32, material: u32, o2w: Mat4) -> Self {
        let w2o = o2w.inverse();
        Self {
            raw,
            material,
            w2o,
            normal_matrix: Mat3::from_mat4(w2o).transpose(),
        }
    }
    /// The objects of `s` the GPU builds procedural instances for, materials 5 + index.
    pub(super) fn of(s: &crate::scene::Settings) -> Vec<Self> {
        s.objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.visible && !o.sdf.is_none())
            .map(|(i, o)| {
                let t = crate::geometry::object_transform(o);
                let o2w = Mat4::from_cols_array(&[
                    t[0], t[4], t[8], 0., t[1], t[5], t[9], 0., t[2], t[6], t[10], 0., t[3], t[7],
                    t[11], 1.,
                ]);
                Self::new(o.sdf.id(o.phase, o.detail), 5 + i as u32, o2w)
            })
            .collect()
    }
    /// The nearest crossing of the world ray o + t d in (tmin, tmax).
    pub(super) fn hit(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<f32> {
        hit(
            self.raw,
            self.w2o.transform_point3(o),
            self.w2o.transform_vector3(d),
            tmin,
            tmax,
        )
    }
    /// The unit world normal at the hit `t` along o + t d (not yet turned toward the ray).
    pub(super) fn normal(&self, o: Vec3, d: Vec3, t: f32) -> Vec3 {
        let p = self.w2o.transform_point3(o) + self.w2o.transform_vector3(d) * t;
        (self.normal_matrix * normal(self.raw, p)).normalize_or(Vec3::Y)
    }
}

fn kind(raw: u32) -> u32 {
    raw & 255
}

/// The parameter in thousandths above the kind (`Object::detail` or the phase).
fn param(raw: u32) -> f32 {
    (raw >> 8) as f32 * 0.001
}

/// The detail, or `default` when it is 0.
fn detail(raw: u32, default: f32) -> f32 {
    Some(param(raw)).filter(|v| *v > 0.).unwrap_or(default)
}

/// The distance field of `raw` at object-space `p`; unknown kinds are the unit sphere.
pub(super) fn field(raw: u32, p: Vec3) -> f32 {
    match kind(raw) {
        TORUS => {
            // Tube radius r about the circle of radius 1 − r in the xz plane.
            let r = detail(raw, 0.28);
            Vec2::new(p.xz().length() - (1. - r), p.y).length() - r
        }
        MANDELBULB => mandelbulb(p, detail(raw, 8.)),
        MENGER => menger(p, (detail(raw, 5.) as i32).clamp(1, 6)),
        METABALLS => metaballs(p, param(raw)),
        CYLINDER => {
            // Radial and axial excess over the radius-1, half-height-1 cylinder.
            let e = Vec2::new(p.xz().length() - 1., p.y.abs() - 1.);
            e.max(Vec2::ZERO).length() + e.max_element().min(0.)
        }
        CONE => {
            // In the (radius, y) half-plane: the base disc's segment from (0, −1) to (1, −1) and
            // the slanted side from (1, −1) to (0, 1).
            let q = Vec2::new(p.xz().length(), p.y);
            let base = q - Vec2::new(q.x.min(1.), -1.);
            let a = Vec2::new(1., -1.);
            let e = Vec2::new(-1., 2.);
            let side = q - (a + e * ((q - a).dot(e) / e.dot(e)).clamp(0., 1.));
            let inside = q.y > -1. && q.x < 0.5 * (1. - q.y) && q.y < 1.;
            let d = base.length().min(side.length());
            if inside { -d } else { d }
        }
        _ => p.length() - 1.,
    }
}

/// The bulb of power `n`, scaled by 1.15 to fill the cube, by the distance estimate
/// ½ ln r · r / |dz/dc|. Power 8 uses Quilez's polynomial form of the same map (as the GPU does,
/// whose image it must match); other powers the spherical-coordinate one, y up.
fn mandelbulb(p: Vec3, n: f32) -> f32 {
    const SCALE: f32 = 1.15;
    let c = p * SCALE;
    let mut w = c;
    if n != 8. {
        let (mut dr, mut r) = (1f32, w.length());
        for _ in 0..6 {
            if r >= 2. {
                break;
            }
            let theta = (w.y / r.max(1e-12)).clamp(-1., 1.).acos() * n;
            let phi = w.z.atan2(w.x) * n;
            dr = n * r.powf(n - 1.) * dr + 1.;
            let (st, ct) = theta.sin_cos();
            let (sp, cp) = phi.sin_cos();
            w = c + r.powf(n) * Vec3::new(st * cp, ct, st * sp);
            r = w.length();
        }
        return 0.5 * r.max(1e-8).ln() * r / dr / SCALE;
    }
    let (mut m, mut dz) = (w.length_squared(), 1f32);
    for _ in 0..6 {
        if m >= 4. {
            break;
        }
        // |w|⁷ · 8 = 8 m³ √m.
        dz = 8. * m.powi(3) * m.sqrt() * dz + 1.;
        let Vec3 { x, y, z } = w;
        let (x2, y2, z2) = (x * x, y * y, z * z);
        let (x4, y4, z4) = (x2 * x2, y2 * y2, z2 * z2);
        let k3 = x2 + z2;
        let k2 = 1. / k3.powi(7).sqrt();
        let k1 = x4 + y4 + z4 - 6. * y2 * z2 - 6. * x2 * y2 + 2. * z2 * x2;
        let k4 = x2 - y2 + z2;
        w = c + Vec3::new(
            64. * x * y * z * (x2 - z2) * k4 * (x4 - 6. * x2 * z2 + z4) * k1 * k2,
            -16. * y2 * k3 * k4 * k4 + k1 * k1,
            -8. * y
                * k4
                * (x4 * x4 - 28. * x4 * x2 * z2 + 70. * x4 * z4 - 28. * x2 * z2 * z4 + z4 * z4)
                * k1
                * k2,
        );
        m = w.length_squared();
    }
    0.25 * m.max(1e-16).ln() * m.sqrt() / dz / SCALE
}

/// The unit cube with, per level, the three axis crosses of a 3× finer grid removed.
fn menger(p: Vec3, levels: i32) -> f32 {
    let q = p.abs() - Vec3::ONE;
    let mut d = q.max(Vec3::ZERO).length() + q.max_element().min(0.);
    let mut scale = 1.;
    for _ in 0..levels {
        // The position inside this level's cell, in [−1, 1).
        let a = (p * scale).rem_euclid(Vec3::splat(2.)) - Vec3::ONE;
        scale *= 3.;
        let r = (Vec3::ONE - 3. * a.abs()).abs();
        let cross = r.x.max(r.y).min(r.y.max(r.z)).min(r.z.max(r.x)) - 1.;
        d = d.max(cross / scale);
    }
    d
}

/// The lava lamp's wax at phase `ph`: in q = (x, 3y, z) a pool, a cap and four blobs on
/// Lissajous paths, blended by a polynomial smooth minimum of width 0.5. Stretching y by 3
/// makes d / 3 a bound.
fn metaballs(p: Vec3, ph: f32) -> f32 {
    let q = Vec3::new(p.x, 3. * p.y, p.z);
    let pool = (q - Vec3::new(0., -3.3, 0.)).length() - 0.95;
    let mut d = pool.min((q - Vec3::new(0., 3.4, 0.)).length() - 0.7);
    for i in 0..4 {
        let fi = i as f32;
        let a = ph * (0.5 + 0.17 * fi) + 1.7 * fi;
        let r = 0.42 + 0.1 * (i & 1) as f32;
        let c = Vec3::new(
            0.4 * (1.3 * a + fi).sin(),
            2.3 * a.sin(),
            0.4 * (0.9 * a + 2. * fi).cos(),
        );
        let e = (q - c).length() - r;
        let h = (0.5 + (e - d)).clamp(0., 1.);
        d = e + (d - e) * h - 0.5 * h * (1. - h);
    }
    d / 3.
}

/// The object-space normal (any length) at a surface point `p`.
pub(super) fn normal(raw: u32, p: Vec3) -> Vec3 {
    let rho = p.xz().length();
    match kind(raw) {
        SPHERE => p,
        // The cap or the side, whichever p lies nearer.
        CYLINDER if (p.y.abs() - 1.).abs() < (rho - 1.).abs() => Vec3::new(0., p.y.signum(), 0.),
        CYLINDER => Vec3::new(p.x, 0., p.z),
        // The side's distance is its radial gap times cos(atan(1/2)) = 2/√5.
        CONE if (p.y + 1.).abs() < (rho - 0.5 * (1. - p.y)).abs() * 0.894 => Vec3::NEG_Y,
        CONE => {
            let r = rho.max(1e-8);
            Vec3::new(p.x / r, 0.5, p.z / r)
        }
        _ => {
            // The field's gradient by central differences on a tetrahedron's corners.
            let e = 1e-4;
            [
                Vec3::new(1., -1., -1.),
                Vec3::new(-1., -1., 1.),
                Vec3::new(-1., 1., -1.),
                Vec3::ONE,
            ]
            .iter()
            .map(|k| *k * field(raw, p + *k * e))
            .sum()
        }
    }
}

/// The nearest surface crossing of the object-space ray o + t d in (tmin, tmax); from inside
/// (refraction) that is the exit.
pub(super) fn hit(raw: u32, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<f32> {
    // Every shape lies in the unit cube: a ray that misses it, in range, misses the shape.
    let inv = Vec3::select(d.abs().cmpgt(Vec3::splat(1e-12)), d, Vec3::splat(1e-12)).recip();
    let (a, b) = ((Vec3::NEG_ONE - o) * inv, (Vec3::ONE - o) * inv);
    let t0 = tmin.max(a.min(b).max_element());
    let t1 = tmax.min(a.max(b).min_element());
    if t0 > t1 {
        return None;
    }
    let inside = |t: f32| t > tmin && t < tmax;
    match kind(raw) {
        SPHERE => {
            // |o + t d|² = 1.
            let (qa, qb) = (d.dot(d), o.dot(d));
            let disc = qb * qb - qa * (o.dot(o) - 1.);
            if disc < 0. {
                return None;
            }
            let r = disc.sqrt();
            [(-qb - r) / qa, (-qb + r) / qa]
                .into_iter()
                .find(|t| *t > tmin)
                .filter(|t| inside(*t))
        }
        k @ (CYLINDER | CONE) => {
            let cone = k == CONE;
            let mut roots = vec![];
            // The side: radius² = 1 (cylinder) or ((1 − y) / 2)² (cone), for y in [−1, 1].
            let (oxz, dxz) = (o.xz(), d.xz());
            let (qa, qb, qc) = if cone {
                let (h, dh) = (1. - o.y, -d.y);
                (
                    dxz.dot(dxz) - 0.25 * dh * dh,
                    2. * oxz.dot(dxz) - 0.5 * h * dh,
                    oxz.dot(oxz) - 0.25 * h * h,
                )
            } else {
                (dxz.dot(dxz), 2. * oxz.dot(dxz), oxz.dot(oxz) - 1.)
            };
            let disc = qb * qb - 4. * qa * qc;
            if qa.abs() > 1e-12 && disc >= 0. {
                let r = disc.sqrt();
                for t in [(-qb - r) / (2. * qa), (-qb + r) / (2. * qa)] {
                    if (o.y + d.y * t).abs() <= 1. {
                        roots.push(t);
                    }
                }
            }
            // The caps (the cone has only its base): planes y = ±1 inside the unit disc.
            if d.y.abs() >= 1e-12 {
                for y in if cone { &[-1.][..] } else { &[-1., 1.][..] } {
                    let t = (y - o.y) / d.y;
                    if (oxz + dxz * t).length_squared() <= 1. {
                        roots.push(t);
                    }
                }
            }
            roots.into_iter().filter(|t| inside(*t)).reduce(f32::min)
        }
        k => {
            // Sphere tracing along the unit object direction, in object units.
            let len = d.length();
            let u = d / len;
            let (mut s, mut e) = (t0 * len, t1 * len);
            if k != 4 && k != 5 {
                // Round shapes start at their bounding sphere instead of the cube's corners.
                let bb = o.dot(u);
                let disc = bb * bb - o.dot(o) + 1.;
                if disc < 0. {
                    return None;
                }
                s = s.max(-bb - disc.sqrt());
                e = e.min(-bb + disc.sqrt());
            }
            // From inside, march the negated field to the exit.
            let side = if field(raw, o + u * s) < 0. { -1. } else { 1. };
            for _ in 0..128 {
                if s >= e {
                    break;
                }
                let h = side * field(raw, o + u * s);
                // 1e-4 world units.
                if h < 1e-4 * len {
                    let t = s / len;
                    return (t > tmin).then_some(t);
                }
                s += if k == 3 { 0.9 * h } else { h };
            }
            None
        }
    }
}
