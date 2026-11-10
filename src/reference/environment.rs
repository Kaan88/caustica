//! The environment: an HDRI or the physical sky's baked map, importance-sampled, with its split
//! sun and the ground projection. The texels and the sun come from `crate::environment`; the
//! sampling tables are built here, in f64, from the texels alone.
use super::sampling::{Rng, basis};
use crate::environment::{EnvMap, luminance};
use crate::scene::Settings;
use anyhow::Result;
use glam::Vec3;
use std::f32::consts::PI;
use std::f64::consts::PI as PI64;
use std::sync::Arc;

pub(super) struct Env {
    map: Arc<EnvMap>,
    strength: f32,
    /// Radians: the map's φ = atan2(x, −z) − rotation.
    rotation: f32,
    /// Seen by camera rays; hidden, they see the background colour.
    pub(super) visible: bool,
    /// Ground projection: the capture point's height above the floor (0 = off).
    pub(super) ground: f32,
    /// Row choice by luminance · sin θ (cumulative, ending at 1), then the choice within each row
    /// (`width` per row, cumulative, ending at 1).
    rows: Vec<f64>,
    columns: Vec<f64>,
    /// Converts luminance · sin θ_row / sin θ into a solid-angle pdf.
    norm: f32,
    /// The split sun: direction, 1 − cos of its half angle, radiance, share of the samples.
    sun: Option<(Vec3, f32, Vec3, f32)>,
    /// The physical sky's catalogue stars (night sky N3) and this frame's rotation, scale,
    /// share of sky samples and camera cone (`Scene::new` sets the cone).
    pub(super) stars: Option<(&'static crate::stars::Table, crate::stars::Frame)>,
}

/// The floor's height (`geometry::static_geometry`); ground projection and the shadow catcher
/// are about this plane.
pub(super) const FLOOR: f32 = -1.;

impl Env {
    /// The scene's environment, if it has one.
    pub(super) fn load(s: &Settings) -> Result<Option<Env>> {
        if s.physical_sky.on {
            // Baked at strength 1 with north at −z (as `Gpu::frame` sets it).
            let sky = &s.physical_sky;
            let map = crate::sky::bake_cached(sky)?;
            let stars = crate::stars::Table::get()
                .filter(|_| map.stars)
                .map(|t| (t, crate::stars::Frame::new(sky, map.star_share, 0.)));
            let mut env = Self::new(map, 1., sky.rotation(), sky.visible, 0.);
            env.stars = stars;
            return Ok(Some(env));
        }
        if s.hdri.is_empty() {
            return Ok(None);
        }
        let map = Arc::new(crate::environment::load(&s.hdri, s.hdri_sun)?);
        Ok(Some(Self::new(
            map,
            s.hdri_strength,
            s.hdri_rotation.to_radians(),
            s.hdri_visible,
            s.hdri_ground,
        )))
    }
    pub(super) fn new(
        map: Arc<EnvMap>,
        strength: f32,
        rotation: f32,
        visible: bool,
        ground: f32,
    ) -> Env {
        let (w, h) = (map.width, map.height);
        let mut columns = vec![0f64; w * h];
        let mut rows = vec![0f64; h];
        let mut total = 0f64;
        for y in 0..h {
            let row = &mut columns[y * w..(y + 1) * w];
            let mut sum = 0f64;
            for (x, c) in row.iter_mut().enumerate() {
                let t = map.texels[y * w + x];
                sum += luminance([t[0], t[1], t[2]]) as f64;
                *c = sum;
            }
            for c in row.iter_mut() {
                *c = if sum > 0. { *c / sum } else { 0. };
            }
            row[w - 1] = 1.;
            total += sum * (PI64 * (y as f64 + 0.5) / h as f64).sin();
            rows[y] = total;
        }
        rows.iter_mut().for_each(|r| *r /= total);
        rows[h - 1] = 1.;
        // A texel spans π/h in θ and 2π/w in φ: its pdf over dθ dφ is p / (2π²/(w h)).
        let norm = ((w * h) as f64 / (2. * PI64 * PI64 * total)) as f32;
        let sun = map.sun.map(|sun| {
            let d = Vec3::from(sun.direction(rotation));
            (
                d,
                sun.om,
                Vec3::from(sun.radiance) * strength,
                map.sun_share,
            )
        });
        Env {
            map,
            strength,
            rotation,
            visible,
            ground,
            rows,
            columns,
            norm,
            sun,
            stars: None,
        }
    }
    /// The share of sky light samples the map (and its split sun) keeps; the stars take the
    /// rest (`env_keep` in the shader).
    pub(super) fn keep(&self) -> f32 {
        1. - self.stars.map_or(0., |(_, f)| f.share)
    }
    /// What camera rays and pure-delta chains see of the stars along `d`.
    pub(super) fn stars_seen(&self, d: Vec3) -> Vec3 {
        self.stars.map_or(Vec3::ZERO, |(t, f)| f.seen(t, d))
    }
    fn texel(&self, x: usize, y: usize) -> Vec3 {
        let t = self.map.texels[y * self.map.width + x];
        Vec3::new(t[0], t[1], t[2])
    }
    /// (u, v) in [0, 1) × [0, 1] of a world direction.
    fn uv(&self, d: Vec3) -> (f32, f32) {
        let theta = d.y.clamp(-1., 1.).acos();
        let phi = d.x.atan2(-d.z) - self.rotation;
        ((phi / (2. * PI) + 0.5).rem_euclid(1.), theta / PI)
    }
    fn in_sun(&self, d: Vec3) -> bool {
        // Within the disc by sin² (a cross product), which keeps its precision near the centre.
        self.sun.is_some_and(|(c, om, _, _)| {
            d.dot(c) > 0. && d.cross(c).length_squared() <= om * (2. - om) * 1.000001
        })
    }
    /// The radiance from direction `d`: the map filtered bilinearly (sampling keeps the
    /// per-texel pdf; the MIS weights only need the pdf samples were drawn from), plus the sun.
    pub(super) fn radiance(&self, d: Vec3) -> Vec3 {
        let (w, h) = (self.map.width as i64, self.map.height as i64);
        let (u, v) = self.uv(d);
        let (fx, fy) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |dx: i64, dy: i64| {
            let x = (x0 as i64 + dx).rem_euclid(w) as usize;
            let y = (y0 as i64 + dy).clamp(0, h - 1) as usize;
            self.texel(x, y)
        };
        let c = at(0, 0)
            .lerp(at(1, 0), tx)
            .lerp(at(0, 1).lerp(at(1, 1), tx), ty);
        let sun = match self.sun {
            Some((_, _, l, _)) if self.in_sun(d) => l,
            _ => Vec3::ZERO,
        };
        c * self.strength + sun
    }
    /// The map's own pdf (the nearest texel's).
    fn map_pdf(&self, d: Vec3) -> f32 {
        let (w, h) = (self.map.width, self.map.height);
        let (u, v) = self.uv(d);
        let x = ((u * w as f32) as usize).min(w - 1);
        let y = ((v * h as f32) as usize).min(h - 1);
        let sin_row = (PI * (y as f32 + 0.5) / h as f32).sin();
        let sin = (1. - d.y * d.y).max(0.).sqrt();
        luminance(self.texel(x, y).to_array()) * self.norm * sin_row / sin.max(1e-6)
    }
    /// The solid-angle pdf of `sample` drawing `d`.
    pub(super) fn pdf(&self, d: Vec3) -> f32 {
        self.mix(self.map_pdf(d), d)
    }
    /// The map's pdf `p` at `d` mixed with the sun's disc by its share.
    fn mix(&self, p: f32, d: Vec3) -> f32 {
        match self.sun {
            Some((_, om, _, share)) => {
                (1. - share) * p
                    + if self.in_sun(d) {
                        share / (2. * PI * om)
                    } else {
                        0.
                    }
            }
            None => p,
        }
    }
    /// A direction by the map's luminance · sin θ (uniform within the texel) or, by its share, in
    /// the sun's disc; and its pdf.
    pub(super) fn sample(&self, r: &mut Rng) -> (Vec3, f32) {
        if let Some((c, om, _, share)) = self.sun
            && r.f() < share
        {
            let x = r.f() * om;
            let s = (x * (2. - x)).sqrt();
            let a = 2. * PI * r.f();
            let d = basis(c, Vec3::new(s * a.cos(), s * a.sin(), 1. - x)).normalize();
            return (d, self.pdf(d));
        }
        let (w, h) = (self.map.width, self.map.height);
        let pick = |cdf: &[f64], u: f64| {
            let i = cdf.partition_point(|c| *c <= u).min(cdf.len() - 1);
            let lo = if i > 0 { cdf[i - 1] } else { 0. };
            (i, ((u - lo) / (cdf[i] - lo).max(1e-300)).clamp(0., 1.))
        };
        let (y, fy) = pick(&self.rows, r.f() as f64);
        let (x, fx) = pick(&self.columns[y * w..(y + 1) * w], r.f() as f64);
        let theta = PI * (y as f32 + fy as f32) / h as f32;
        let phi = 2. * PI * ((x as f32 + fx as f32) / w as f32 - 0.5) + self.rotation;
        let d = Vec3::new(
            theta.sin() * phi.sin(),
            theta.cos(),
            -theta.sin() * phi.cos(),
        );
        // The pdf of the texel drawn from, not of the one `d` looks up: near the poles cos θ in
        // f32 can put `d` a row off, beside a star a thousand times brighter or darker.
        let sin_row = (PI * (y as f32 + 0.5) / h as f32).sin();
        let p =
            luminance(self.texel(x, y).to_array()) * self.norm * sin_row / theta.sin().max(1e-6);
        (d, self.mix(p, d))
    }
    /// Ground projection: the direction from the capture point (0, floor + height, 0) to `p`.
    pub(super) fn ground_dir(&self, p: Vec3) -> Vec3 {
        (p - Vec3::new(0., FLOOR + self.ground, 0.)).normalize()
    }
    /// The direction a camera-seen background is looked up in: below the horizon, with ground
    /// projection, toward where the ray meets the floor's plane.
    pub(super) fn background_dir(&self, o: Vec3, d: Vec3) -> Vec3 {
        if self.ground <= 0. || d.y >= 0. || o.y <= FLOOR {
            return d;
        }
        self.ground_dir(o + d * ((FLOOR - o.y) / d.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The pdf integrates to 1 and samples land where it is large: the mean of 1/pdf over
    /// samples is the sphere's 4π.
    #[test]
    fn samples_match_the_pdf() {
        let (w, h) = (64, 32);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let hot = if (x, y) == (40, 9) { 500. } else { 0. };
                [0.2 + hot, 0.3 + y as f32 * 0.01, 0.5]
            })
            .collect();
        let env = Env::new(
            Arc::new(crate::environment::build(w, h, rgb).unwrap()),
            1.,
            0.7,
            true,
            0.,
        );
        let mut r = Rng::new(1, 2);
        let n = 200_000;
        let (mut inv, mut off) = (0f64, 0);
        for _ in 0..n {
            let (d, p) = env.sample(&mut r);
            // A direction on a texel's edge can look up its neighbour.
            off += ((env.pdf(d) - p).abs() > 1e-3 * p) as u32;
            inv += 1. / p as f64;
        }
        assert!(off < n / 1000, "{off}");
        let mean = inv / n as f64;
        assert!((mean / (4. * PI64) - 1.).abs() < 0.02, "{mean}");
        // The pdf agrees with the map's own normalization.
        assert!((env.norm / env.map.inv_norm - 1.).abs() < 1e-3);
    }
    /// With a split sun, the disc takes its share of samples and the mixture pdf still
    /// integrates to 1.
    #[test]
    fn the_sun_takes_its_share() {
        let (w, h) = (256, 128);
        let mut rgb = vec![[0.5f32; 3]; w * h];
        for (x, y) in [(64, 32), (65, 32), (64, 33), (65, 33)] {
            rgb[y * w + x] = [5000., 4000., 3000.];
        }
        let sun = crate::environment::split_sun(w, h, &mut rgb).unwrap();
        let plain = crate::environment::build(w, h, rgb).unwrap();
        let disc = luminance(sun.radiance) * 2. * PI * sun.om;
        let map = EnvMap {
            sun_share: disc / (disc + 1. / plain.inv_norm),
            sun: Some(sun),
            ..plain
        };
        let env = Env::new(Arc::new(map), 1., 0., true, 0.);
        let mut r = Rng::new(3, 4);
        let n = 100_000;
        let (mut inv, mut inside) = (0f64, 0);
        for _ in 0..n {
            let (d, p) = env.sample(&mut r);
            inv += 1. / p as f64;
            inside += env.in_sun(d) as u32;
        }
        assert!((inv / n as f64 / (4. * PI64) - 1.).abs() < 0.03);
        let share = env.sun.unwrap().3;
        assert!((inside as f32 / n as f32 - share).abs() < 0.01);
    }
}
