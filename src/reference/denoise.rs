//! A denoiser for the CPU view: the spatial half of the native SVGF-style filter (post.hlsl's
//! `integrate` and five à-trous passes), on the mean the progressive render holds. It is for
//! looking at only: the sums stay unbiased and nothing saved as the raw mean goes through it.
//! The temporal half has nothing to do here: the view starts over on any camera move, and its
//! mean already holds the whole history.
//!
//! The guides are the GPU's: per pixel the first-hit object, normal and position of a ray through
//! the pixel centre, and from the samples themselves the mean first-hit albedo (diffuse plus
//! specular, the background's albedo being 0), the coverage (the share of samples whose first
//! hit is the guide's object) and the second moment of luminance. Light is filtered divided by
//! the albedo, so texture-like detail in the albedo stays sharp; edge pixels (coverage below 1)
//! keep their unfiltered share, which the surface stops can't separate.
use super::bsdf::{f0_of, fresnel, material_at, rgb};
use super::sampling::Rng;
use super::scene::{Ray, Scene};
use glam::{Vec2, Vec3};

pub(super) fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The pixel-centre ray's first hit: what the filter's surface stops compare.
#[derive(Clone, Copy, Default)]
pub(super) struct Guide {
    /// The material + 1; 0 for the background, which the filter leaves alone.
    pub(super) id: u32,
    pub(super) n: Vec3,
    pub(super) p: Vec3,
    /// One pixel's width at that distance, the scale of the plane-distance stop.
    pub(super) pixel: f32,
}
/// What the samples add per pixel besides their light.
#[derive(Clone, Copy, Default)]
pub(super) struct Moments {
    pub(super) albedo: Vec3,
    pub(super) covered: u32,
    /// Sum of squared luminance (of what went into the sums, after the firefly clamp).
    pub(super) m2: f64,
}

impl Scene<'_> {
    pub(super) fn guide(&self, x: usize, y: usize) -> Guide {
        // Through the pixel centre; a lens takes a fixed point of its own (the pinhole draws
        // nothing), and one that cuts the ray off leaves no guide.
        let pixel = (y * self.size[0] as usize + x) as u64;
        let ray = self.camera_ray(
            Vec2::new(x as f32 + 0.5, y as f32 + 0.5),
            &mut super::spectral::Waves::rgb(),
            &mut Rng::new(pixel, u64::MAX),
        );
        let Some((o, d, _)) = ray else {
            return Guide::default();
        };
        let Some(h) = self.intersect(o, d, 1e4, &self.accept(Ray::Camera, o, d), false) else {
            return Guide::default();
        };
        let sf = self.surface(&h, o, d);
        let f = &self.frame;
        let span = match f.projection {
            1 => f.ortho_half,
            // Perspective: by depth along the view axis; the wide projections by distance.
            0 => f.tan_half * (sf.p - self.eye).dot(f.forward),
            _ => f.tan_half * h.t,
        };
        Guide {
            id: sf.material + 1,
            n: sf.n,
            p: sf.p,
            pixel: 2. * span / self.size[1] as f32,
        }
    }
    /// A sample's first hit along (o, d): its albedo (diffuse plus specular at the view cosine,
    /// 0 for the background) and its material + 1 (0 for none).
    pub(super) fn first_albedo(&self, o: Vec3, d: Vec3) -> (Vec3, u32) {
        let Some(h) = self.intersect(o, d, 1e4, &self.accept(Ray::Camera, o, d), false) else {
            return (Vec3::ZERO, 0);
        };
        let sf = self.surface(&h, o, d);
        let m = material_at(&self.materials[sf.material as usize], sf.p);
        let diffuse = rgb(m.color_rough) * (1. - m.lobes[0]) * (1. - m.lobes[2]);
        let specular = fresnel(sf.n.dot(-d).abs(), f0_of(&m));
        (diffuse + specular, sf.material + 1)
    }
}

/// The filtered image of a mean of `count` samples: `mean`, `moments` and `guides` per pixel,
/// rows top first; `strength` is the setting's (0..1). Runs its passes over rows on `threads`.
pub(super) fn filter(
    mean: &[Vec3],
    moments: &[Moments],
    guides: &[Guide],
    count: u32,
    size: [usize; 2],
    strength: f32,
    threads: usize,
) -> Vec<Vec3> {
    let [w, h] = size;
    let n = count.max(1) as f32;
    let at = |x: isize, y: isize| -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h)
            .then(|| y as usize * w + x as usize)
    };
    let albedo = |i: usize| (moments[i].albedo / n).max(Vec3::splat(0.05));
    let coverage = |i: usize| moments[i].covered as f32 / n;
    // integrate: demodulated light and the variance of its luminance.
    let light = |i: usize| mean[i].max(Vec3::ZERO) / albedo(i);
    let mut cur: Vec<(Vec3, f32)> = vec![(Vec3::ZERO, 0.); w * h];
    super::progressive::for_rows(
        &mut cur,
        w,
        threads,
        false,
        &Default::default(),
        &|y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let c = light(i);
                let variance = if count < 4 {
                    // Too few samples for moments: the spread of same-surface neighbours instead.
                    let (mut s1, mut s2, mut k) = (0., 0., 0.);
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            if let Some(j) = at(x as isize + dx, y as isize + dy)
                                && guides[j].id == guides[i].id
                            {
                                let l = luminance(light(j));
                                (s1, s2, k) = (s1 + l, s2 + l * l, k + 1.);
                            }
                        }
                    }
                    (s2 / k - (s1 / k) * (s1 / k)).max(0.)
                } else {
                    // The raw second moment, scaled to demodulated light by the pixel's ratio.
                    let (ld, lr) = (luminance(c), luminance(mean[i].max(Vec3::ZERO)));
                    let s = if lr > 1e-6 {
                        ld / lr
                    } else {
                        1. / luminance(albedo(i))
                    };
                    let m2 = (moments[i].m2 / n as f64) as f32 * s * s;
                    (m2 - ld * ld).max(0.) / n
                };
                *out = (c, variance);
            }
        },
    );
    let integrated: Vec<Vec3> = cur.iter().map(|c| c.0).collect();
    let mut next = cur.clone();
    for pass in 0..5 {
        let step = 1isize << pass;
        let src = &cur;
        super::progressive::for_rows(
            &mut next,
            w,
            threads,
            false,
            &Default::default(),
            &|y, row| {
                for (x, out) in row.iter_mut().enumerate() {
                    let i = y * w + x;
                    let g = guides[i];
                    let center = src[i];
                    if g.id == 0 {
                        *out = center;
                        continue;
                    }
                    // The luminance stop from a 3x3 Gaussian of the variance.
                    let (mut gv, mut gw) = (0., 0.);
                    for dy in -1..=1isize {
                        for dx in -1..=1isize {
                            if let Some(j) = at(x as isize + dx, y as isize + dy)
                                && guides[j].id == g.id
                            {
                                let k = (2 - dx.abs()) as f32 * (2 - dy.abs()) as f32;
                                gv += k * src[j].1;
                                gw += k;
                            }
                        }
                    }
                    let stop = 8. * strength.max(0.01) * (gv / gw).sqrt() + 1e-4;
                    let l = luminance(center.0);
                    let (mut sum, mut total, mut variance) = (Vec3::ZERO, 0., 0.);
                    for dy in -2..=2isize {
                        for dx in -2..=2isize {
                            let Some(j) = at(x as isize + dx * step, y as isize + dy * step) else {
                                continue;
                            };
                            // Never mix surfaces, whatever their light.
                            let q = guides[j];
                            if q.id != g.id {
                                continue;
                            }
                            let tap = |d: isize| [6., 4., 1.][d.unsigned_abs()];
                            let offset = Vec2::new(dx as f32, dy as f32).length() * step as f32;
                            let plane =
                                (-g.n.dot(q.p - g.p).abs() / (2. * g.pixel * offset + 1e-4)).exp();
                            let c = src[j];
                            let wgt = tap(dx)
                                * tap(dy)
                                * g.n.dot(q.n).clamp(0., 1.).powi(64)
                                * plane
                                * (-(luminance(c.0) - l).abs() / stop).exp();
                            sum += c.0 * wgt;
                            total += wgt;
                            variance += wgt * wgt * c.1;
                        }
                    }
                    *out = if total > 1e-6 {
                        (sum / total, variance / (total * total))
                    } else {
                        center
                    };
                }
            },
        );
        std::mem::swap(&mut cur, &mut next);
    }
    // Edge pixels get their unfiltered share back, then the albedo returns.
    (0..w * h)
        .map(|i| {
            let c = if guides[i].id == 0 {
                integrated[i]
            } else {
                integrated[i].lerp(cur[i].0, coverage(i).clamp(0., 1.))
            };
            c * albedo(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::sampling::Rng;

    /// A flat wall split into two objects at x = w/2, the right one 4x as bright: noise falls,
    /// the means stay and the step stays sharp.
    #[test]
    fn filter_smooths_noise_and_keeps_object_edges() {
        let (w, h, count) = (32usize, 24usize, 8u32);
        let mut mean = vec![];
        let mut moments = vec![];
        let mut guides = vec![];
        let mut truth = vec![];
        for y in 0..h {
            for x in 0..w {
                let right = x >= w / 2;
                let level = if right { 0.8 } else { 0.2 };
                let mut r = Rng::new((y * w + x) as u64, 0);
                let (mut s, mut m2) = (0f32, 0f64);
                for _ in 0..count {
                    let v = level * 2. * r.f();
                    s += v;
                    m2 += (v * v) as f64;
                }
                mean.push(Vec3::splat(s / count as f32));
                truth.push(level);
                moments.push(Moments {
                    albedo: Vec3::splat(0.5 * count as f32),
                    covered: count,
                    m2,
                });
                guides.push(Guide {
                    id: if right { 2 } else { 1 },
                    n: Vec3::Z,
                    p: Vec3::new(x as f32, y as f32, 0.) * 0.01,
                    pixel: 0.01,
                });
            }
        }
        let out = filter(&mean, &moments, &guides, count, [w, h], 0.5, 4);
        let err = |img: &[Vec3]| -> f32 {
            img.iter()
                .zip(&truth)
                .map(|(c, t)| (c.x - t).powi(2))
                .sum::<f32>()
                / img.len() as f32
        };
        assert!(
            err(&out) < 0.2 * err(&mean),
            "{} vs {}",
            err(&out),
            err(&mean)
        );
        for y in 0..h {
            // The columns either side of the edge keep their own level.
            assert!((out[y * w + w / 2 - 1].x - 0.2).abs() < 0.08);
            assert!((out[y * w + w / 2].x - 0.8).abs() < 0.2);
        }
        // Background pixels pass through untouched.
        guides.iter_mut().for_each(|g| g.id = 0);
        let same = filter(&mean, &moments, &guides, count, [w, h], 0.5, 2);
        assert!(
            same.iter()
                .zip(&mean)
                .all(|(a, b)| (*a - *b).abs().max_element() < 1e-6)
        );
    }
}
