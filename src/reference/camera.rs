//! Camera rays: the pixel filter's offset and the ray through a film point.
use super::sampling::{Rng, refract};
use super::scene::Scene;
use super::spectral::Waves;
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

/// Whether `q` lies in the stop of radius `r`: a disk, or the regular polygon of `blades` sides
/// rotated by `rotation` that `aperture_point` samples, `r` being its corners' radius.
pub(super) fn inside_stop(q: Vec2, r: f32, blades: u32, rotation: f32) -> bool {
    if blades <= 2 {
        return q.length_squared() <= r * r;
    }
    // Fold the angle into one edge's sector, measured from the edge's normal.
    let sector = 2. * PI / blades as f32;
    let a = (q.y.atan2(q.x) - rotation).rem_euclid(sector) - 0.5 * sector;
    q.length() * a.cos() <= r * (0.5 * sector).cos()
}

/// What a lens trace refracts by: an RGB channel's indices, or (spectral) one wavelength's.
#[derive(Clone, Copy)]
pub(super) enum Channel {
    Rgb(usize),
    Nm(f32),
}

/// A real lens system (`lens::pack`'s buffer) in millimetres, the film at z = 0 and the lens
/// toward -z, and what places it in the world.
pub(super) struct RealLens {
    data: Vec<[f32; 4]>,
    surfaces: usize,
    /// World units per millimetre.
    scale: f32,
    dispersion: bool,
}

impl RealLens {
    pub(super) fn new(s: &Settings, aspect: f32) -> Self {
        let (data, surfaces) = crate::lens::pack(&crate::lens::Params {
            lens: s.lens_system,
            f_number: s.f_number,
            focus_mm: s.focus_distance * s.lens_scale * 1000.,
            fov: s.fov.to_radians(),
            aspect,
            dispersion: s.dispersion,
        });
        RealLens {
            data,
            surfaces,
            scale: 0.001 / s.lens_scale,
            dispersion: s.dispersion > 0.,
        }
    }
    /// A trace through a round stop, for tests.
    #[cfg(test)]
    pub(super) fn trace_channel(&self, o: Vec3, d: Vec3, c: Channel) -> Option<(Vec3, Vec3)> {
        self.trace(o, d, c, 0, 0.)
    }
    /// Surface i's index on its film side in channel c. `pack` puts the three on one Cauchy
    /// curve n(λ) = A + B/λ² (green at 550 nm, red 610, blue 465), so red and blue give B and
    /// green A at 550: any wavelength reads off that curve.
    fn ior(&self, i: usize, c: Channel) -> f32 {
        let n = self.data[3 + 2 * i];
        match c {
            Channel::Rgb(c) => n[c],
            Channel::Nm(nm) => {
                let l = nm * 1e-3;
                let b = (n[2] - n[0]) / (1. / (0.465 * 0.465) - 1. / (0.61 * 0.61));
                n[1] + b * (1. / (l * l) - 1. / (0.55 * 0.55))
            }
        }
    }
    /// From the film out of the front element, refracting in channel c; None when a surface is
    /// missed, its rim or the stop blocks the ray, or it reflects totally.
    fn trace(
        &self,
        mut o: Vec3,
        mut d: Vec3,
        c: Channel,
        blades: u32,
        rot: f32,
    ) -> Option<(Vec3, Vec3)> {
        for i in (0..self.surfaces).rev() {
            let [radius, z, aperture, _] = self.data[2 + 2 * i];
            if radius == 0. {
                let t = (z - o.z) / d.z;
                if t.is_nan() || t <= 0. {
                    return None;
                }
                o += t * d;
                if !inside_stop(o.truncate(), aperture, blades, rot) {
                    return None;
                }
                continue;
            }
            // The sphere's centre sits `radius` past the vertex; the ray meets the cap on the
            // vertex's side, the near root when it travels toward the centre's side.
            let oc = o - Vec3::new(0., 0., z + radius);
            let b = oc.dot(d);
            let disc = b * b - (oc.length_squared() - radius * radius);
            if disc < 0. {
                return None;
            }
            let t = if (d.z > 0.) != (radius < 0.) {
                -b - disc.sqrt()
            } else {
                -b + disc.sqrt()
            };
            if t.is_nan() || t <= 0. {
                return None;
            }
            o += t * d;
            if o.truncate().length_squared() > aperture * aperture {
                return None;
            }
            let n = (oc + t * d).normalize();
            let outside = if i > 0 { self.ior(i - 1, c) } else { 1. };
            d = refract(
                d,
                if n.dot(d) > 0. { -n } else { n },
                self.ior(i, c) / outside,
            );
            if d == Vec3::ZERO {
                return None;
            }
        }
        Some((o, d))
    }
}

impl Scene<'_> {
    /// The camera ray through film point `pixel`, origin, direction and weight, for each
    /// projection (`camera_ray`), or None for a sample the cat-eye or a lens system's barrel
    /// cuts off (it still counts, as black). Only the perspective and tilt-shift projections
    /// take the thin lens, and a lens system replaces it in perspective; the pinhole draws no
    /// random numbers. A dispersive lens system on a spectral path (`w`) traces its hero wavelength.
    pub(super) fn camera_ray(
        &self,
        pixel: Vec2,
        w: &mut Waves,
        r: &mut Rng,
    ) -> Option<(Vec3, Vec3, Vec3)> {
        let f = &self.frame;
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let ndc = Vec2::new(ndc.x, -ndc.y);
        if let Some(lens) = &self.lens {
            return self.lens_ray(lens, ndc, w, r);
        }
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
                    return Some((self.eye, d, Vec3::ONE));
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
        .map(|(o, d)| (o, d, Vec3::ONE))
    }
    /// A lens system's ray: the film point for `ndc` (the image is inverted), a uniform point in
    /// the exit-pupil bounds of its band turned to the point's angle, traced out of the front
    /// element. Weight cos⁴ at the film × the bounds' area × `pack`'s normalisation, so the
    /// frame's centre gets 1; with dispersion each sample takes one channel at weight 3, or on a
    /// spectral path the hero wavelength (no random number drawn).
    fn lens_ray(
        &self,
        lens: &RealLens,
        ndc: Vec2,
        waves: &mut Waves,
        r: &mut Rng,
    ) -> Option<(Vec3, Vec3, Vec3)> {
        let f = &self.frame;
        let c = if !lens.dispersion {
            Channel::Rgb(1)
        } else if waves.on {
            Channel::Nm(waves.l.x)
        } else {
            Channel::Rgb(((r.f() * 3.) as usize).min(2))
        };
        let [half_w, half_h, bands, norm] = lens.data[0];
        let [rear_z, _, front_z, covered] = lens.data[1];
        let film = -ndc * Vec2::new(half_w, half_h);
        let rf = film.length();
        let band = ((rf / covered * bands) as usize).min(bands as usize - 1);
        let [x0, x1, y1, area] = lens.data[2 + 2 * crate::lens::MAX_SURFACES + band];
        if area <= 0. {
            return None;
        }
        let q = Vec2::new(x0 + (x1 - x0) * r.f(), (2. * r.f() - 1.) * y1);
        let q = if rf > 1e-6 { (film / rf).rotate(q) } else { q };
        let o = film.extend(0.);
        let d = (q.extend(rear_z) - o).normalize();
        let cos4 = d.z.powi(4);
        let (lo, ld) = lens.trace(o, d, c, f.blades, f.rotation)?;
        let o =
            self.eye + (f.right * lo.x + f.up * lo.y + f.forward * (front_z - lo.z)) * lens.scale;
        let d = (f.right * ld.x + f.up * ld.y - f.forward * ld.z).normalize();
        let mut w = Vec3::splat(cos4 * area * norm);
        match c {
            Channel::Rgb(c) if lens.dispersion => w *= Vec3::AXES[c] * 3.,
            Channel::Nm(_) => waves.hero(&mut w),
            _ => {}
        }
        Some((o, d, w))
    }
    pub(super) fn camera_direction(&self, pixel: Vec2) -> Vec3 {
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let p = self.inverse_vp * Vec4::new(ndc.x, -ndc.y, 1., 1.);
        (p.truncate() / p.w - self.eye).normalize()
    }
}
