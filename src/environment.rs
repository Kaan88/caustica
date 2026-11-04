//! HDRI environments (roadmap core track 3): an equirectangular .hdr / .exr lights the scene
//! and is importance-sampled. The GPU gets the texels (u30) and the sampling tables (u31): one
//! conditional CDF per row, then a last row holding the marginal CDF over rows.
//!
//! Mapping (mirrored in pathtrace.hlsl): u = φ / 2π + 0.5 with φ = atan2(x, −z) − rotation,
//! v = θ / π with θ = acos(y). Texels are looked up nearest, so the sampling pdf matches the
//! radiance exactly: pdf(ω) = lum · inv_norm · sin θ_row / sin θ.
use anyhow::{Context, Result, ensure};
use std::f32::consts::PI;

/// Largest width kept; larger maps are box-filtered down by halves.
const MAX_WIDTH: usize = 8192;

pub struct EnvMap {
    pub width: usize,
    pub height: usize,
    /// Linear RGB, alpha 1, row-major from the top (θ = 0).
    pub texels: Vec<[f32; 4]>,
    /// `width` × (`height` + 1): conditional CDFs (each row ends at 1), then the marginal CDF
    /// over rows in the first `height` entries of the last row.
    pub cdf: Vec<f32>,
    /// width · height / (2π² Σ lum · sin θ_row).
    pub inv_norm: f32,
    /// The sun split out of the map (`load` with `split_sun`), and the share of environment
    /// samples it takes (its power over the total).
    pub sun: Option<Sun>,
    pub sun_share: f32,
}
/// A sun taken out of an HDRI: a disc of uniform radiance around a direction in map space
/// (θ from +y, φ as the map's u: φ = 2π (u − 0.5), at rotation 0).
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub theta: f32,
    pub phi: f32,
    /// 1 − the cosine of the disc's half angle (its solid angle over 2π), kept in this form: a
    /// sun a few texels wide in an 8k map has 1 − cos near f32's resolution at 1.
    pub om: f32,
    /// Linear RGB radiance over the disc.
    pub radiance: [f32; 3],
}
impl Sun {
    /// The world direction toward the sun with the map turned by `rotation` radians (the
    /// shader's `env_texel` subtracts it from φ).
    pub fn direction(&self, rotation: f32) -> [f32; 3] {
        let (st, ct) = self.theta.sin_cos();
        let (sp, cp) = (self.phi + rotation).sin_cos();
        [st * sp, ct, -st * cp]
    }
}
/// Takes a compact sun out of the texels (HDRI *Split sun*): the 4-connected region around the
/// brightest texel that is above 1/50 of it, if it covers under 0.02 sr and holds at least 5%
/// of the map's power. Its excess over the ring around it becomes a disc of the same power and
/// solid angle at its power-weighted centre, and its texels take the ring's mean (so the map
/// keeps the sky and loses the spike). None leaves the texels as they were.
pub fn split_sun(w: usize, h: usize, rgb: &mut [[f32; 3]]) -> Option<Sun> {
    let lum: Vec<f32> = rgb.iter().map(|c| luminance(*c)).collect();
    let (peak_at, &peak) = lum.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if peak <= 0. {
        return None;
    }
    let omega = |y: usize| {
        (2. * PI / w as f32) * (PI / h as f32) * (PI * (y as f32 + 0.5) / h as f32).sin()
    };
    let total: f64 = (0..w * h)
        .map(|i| lum[i] as f64 * omega(i / w) as f64)
        .sum();
    let threshold = peak / 50.;
    let mut inside = vec![false; w * h];
    let mut region = vec![peak_at];
    inside[peak_at] = true;
    let mut ring = std::collections::HashSet::new();
    let mut k = 0;
    let mut solid = 0f32;
    while k < region.len() {
        let i = region[k];
        k += 1;
        solid += omega(i / w);
        if solid > 0.02 {
            return None;
        }
        let (x, y) = (i % w, i / w);
        let mut next = vec![(x + 1) % w + y * w, (x + w - 1) % w + y * w];
        if y > 0 {
            next.push(i - w);
        }
        if y + 1 < h {
            next.push(i + w);
        }
        for j in next {
            if inside[j] {
                continue;
            }
            if lum[j] > threshold {
                inside[j] = true;
                region.push(j);
            } else {
                ring.insert(j);
            }
        }
    }
    let ring: Vec<usize> = ring.into_iter().filter(|&j| !inside[j]).collect();
    let background: [f32; 3] = if ring.is_empty() {
        [0.; 3]
    } else {
        std::array::from_fn(|c| ring.iter().map(|&j| rgb[j][c]).sum::<f32>() / ring.len() as f32)
    };
    let (mut power, mut centre) = ([0f64; 3], [0f64; 3]);
    let mut area = 0f64;
    for &i in &region {
        let (x, y) = (i % w, i / w);
        let o = omega(y) as f64;
        area += o;
        let excess: [f32; 3] = std::array::from_fn(|c| (rgb[i][c] - background[c]).max(0.));
        let l = luminance(excess) as f64 * o;
        let (theta, phi) = (
            PI * (y as f32 + 0.5) / h as f32,
            2. * PI * ((x as f32 + 0.5) / w as f32 - 0.5),
        );
        let d = [
            theta.sin() * phi.sin(),
            theta.cos(),
            -theta.sin() * phi.cos(),
        ];
        for c in 0..3 {
            power[c] += excess[c] as f64 * o;
            centre[c] += d[c] as f64 * l;
        }
    }
    let sun_lum = luminance(power.map(|v| v as f32)) as f64;
    if sun_lum < 0.05 * total {
        return None;
    }
    let len = centre.iter().map(|v| v * v).sum::<f64>().sqrt();
    if len <= 0. {
        return None;
    }
    let d = centre.map(|v| (v / len) as f32);
    for &i in &region {
        rgb[i] = background;
    }
    Some(Sun {
        theta: d[1].clamp(-1., 1.).acos(),
        phi: d[0].atan2(-d[2]),
        om: (area / (2. * std::f64::consts::PI)) as f32,
        radiance: power.map(|v| (v / area) as f32),
    })
}

pub fn luminance(c: [f32; 3]) -> f32 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(0.)
}

/// Loads an equirectangular map; relative paths are relative to the scenes folder.
pub fn load(path: &str, sun: bool) -> Result<EnvMap> {
    let full = crate::mesh::resolve(path);
    let image = image::open(&full)
        .with_context(|| format!("Reading {}", full.display()))?
        .into_rgb32f();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let texels: Vec<[f32; 3]> = image
        .pixels()
        .map(|p| p.0.map(|v| if v.is_finite() { v.max(0.) } else { 0. }))
        .collect();
    if !sun {
        return build(w, h, texels);
    }
    let mut texels = texels;
    let found = split_sun(w, h, &mut texels);
    let mut map = build(w, h, texels)?;
    if let Some(sun) = found {
        // Shares by power: the map's is 1 / inv_norm (∫ lum dΩ), the disc's lum · Ω.
        let disc = luminance(sun.radiance) * 2. * PI * sun.om;
        map.sun_share = disc / (disc + 1. / map.inv_norm);
        map.sun = Some(sun);
    }
    Ok(map)
}

pub fn build(w: usize, h: usize, rgb: Vec<[f32; 3]>) -> Result<EnvMap> {
    build_capped(w, h, rgb, MAX_WIDTH)
}
/// `build` with a width cap (tests use a small one).
fn build_capped(
    mut w: usize,
    mut h: usize,
    mut rgb: Vec<[f32; 3]>,
    max_width: usize,
) -> Result<EnvMap> {
    ensure!(
        w >= 2 && h >= 1 && rgb.len() == w * h,
        "Bad environment size {w}×{h}"
    );
    while w > max_width && h >= 2 {
        let (nw, nh) = (w / 2, h / 2);
        rgb = (0..nw * nh)
            .map(|i| {
                let (x, y) = (i % nw * 2, i / nw * 2);
                let at = |dx: usize, dy: usize| rgb[(y + dy) * w + x + dx];
                std::array::from_fn(|k| {
                    (at(0, 0)[k] + at(1, 0)[k] + at(0, 1)[k] + at(1, 1)[k]) * 0.25
                })
            })
            .collect();
        (w, h) = (nw, nh);
    }
    // The marginal CDF (h entries) is stored in a row of width w, and the doubled CDF texture
    // must stay within D3D12's 16384 rows: equirectangular maps are 2:1 anyway.
    ensure!(
        h <= w,
        "Environment map {w}×{h} is taller than wide (expected an equirectangular 2:1 map)"
    );
    // Rows 0..h: each row's conditional CDF; row h: the marginal CDF over rows; rows h + 1 ..
    // 2h + 1: the same CDFs' cutpoints (`lighting::cutpoints`, K = guide_buckets(w) of them,
    // stored as f32 indices, exact below 2^24), read by the shader's `cdf_search`.
    let mut cdf = vec![0f32; w * 2 * (h + 1)];
    let mut rows = vec![0f64; h];
    for (y, row_sum) in rows.iter_mut().enumerate() {
        let sin = (PI * (y as f32 + 0.5) / h as f32).sin() as f64;
        let row = &mut cdf[y * w..(y + 1) * w];
        let mut sum = 0f64;
        for (x, c) in row.iter_mut().enumerate() {
            sum += luminance(rgb[y * w + x]) as f64;
            *c = sum as f32;
        }
        if sum > 0. {
            row.iter_mut().for_each(|c| *c = (*c as f64 / sum) as f32);
        } else {
            // A black row is never chosen; a uniform CDF keeps the search well defined.
            row.iter_mut()
                .enumerate()
                .for_each(|(x, c)| *c = (x + 1) as f32 / w as f32);
        }
        row[w - 1] = 1.;
        *row_sum = sum * sin;
    }
    let total: f64 = rows.iter().sum();
    ensure!(total > 0., "The environment map is black");
    let mut running = 0f64;
    for (y, s) in rows.iter().enumerate() {
        running += s;
        cdf[h * w + y] = (running / total) as f32;
    }
    cdf[h * w + h - 1] = 1.;
    let k = crate::lighting::guide_buckets(w);
    for row in 0..=h {
        let count = if row < h { w } else { h };
        let guide = crate::lighting::cutpoints(count, k, |i| cdf[row * w + i]);
        let base = (h + 1 + row) * w;
        for (c, g) in cdf[base..base + k].iter_mut().zip(guide) {
            *c = g as f32;
        }
    }
    Ok(EnvMap {
        width: w,
        height: h,
        texels: rgb.into_iter().map(|c| [c[0], c[1], c[2], 1.]).collect(),
        cdf,
        inv_norm: ((w * h) as f64 / (2. * (PI as f64).powi(2) * total)) as f32,
        sun: None,
        sun_share: 0.,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A bright 2×2 spot on a grey sky splits into a disc of its excess power, centred on it,
    /// and the spot's texels take the sky's value; a map without a spike keeps its texels.
    #[test]
    fn a_compact_sun_splits_out() {
        let (w, h) = (256, 128);
        let mut rgb = vec![[0.5f32; 3]; w * h];
        for (x, y) in [(64, 32), (65, 32), (64, 33), (65, 33)] {
            rgb[y * w + x] = [5000., 4000., 3000.];
        }
        let sun = split_sun(w, h, &mut rgb).expect("the spot is a sun");
        assert!(
            rgb.iter().all(|c| *c == [0.5; 3]),
            "the spot was filled with the sky"
        );
        // Map centre (u 65/256, v 33/128): φ = 2π (u − 0.5), θ = π v.
        assert!(
            (sun.phi - 2. * PI * (65. / 256. - 0.5)).abs() < 0.02,
            "{sun:?}"
        );
        assert!((sun.theta - PI * 33. / 128.).abs() < 0.02, "{sun:?}");
        let disc = 2. * PI * sun.om;
        let texel = |y: usize| {
            (2. * PI / w as f32) * (PI / h as f32) * (PI * (y as f32 + 0.5) / h as f32).sin()
        };
        let expect = 4999.5 * 2. * (texel(32) + texel(33));
        assert!((sun.radiance[0] * disc / expect - 1.).abs() < 1e-3);
        let mut flat = vec![[0.5f32; 3]; w * h];
        assert!(split_sun(w, h, &mut flat).is_none());
    }
    /// The pdf integrates to 1 over the sphere, so it can weight samples.
    #[test]
    fn the_pdf_is_normalized() {
        let (w, h) = (64, 32);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let sun = if (x, y) == (40, 9) { 5000. } else { 0. };
                [0.2 + sun, 0.3 + y as f32 * 0.01, 0.5]
            })
            .collect();
        let env = build(w, h, rgb).unwrap();
        // ∫ pdf dω over texels: pdf is lum·inv_norm·sinθ_row/sinθ, dω = sinθ dθ dφ, so each
        // texel contributes lum·inv_norm·sinθ_row·(π/h)(2π/w).
        let integral: f32 = (0..w * h)
            .map(|i| {
                let y = i / w;
                let sin = (PI * (y as f32 + 0.5) / h as f32).sin();
                luminance(env.texels[i][..3].try_into().unwrap())
                    * env.inv_norm
                    * sin
                    * (PI / h as f32)
                    * (2. * PI / w as f32)
            })
            .sum();
        assert!((integral - 1.).abs() < 1e-3, "{integral}");
        assert_eq!(env.cdf[h * w + h - 1], 1.);
        assert!((0..h).all(|y| env.cdf[y * w + w - 1] == 1.));
        // The sun's row takes most of the marginal mass.
        let marginal =
            |y: usize| env.cdf[h * w + y] - if y == 0 { 0. } else { env.cdf[h * w + y - 1] };
        assert!(marginal(9) > 0.5);
    }
    #[test]
    fn big_maps_are_halved_and_black_maps_fail() {
        let env = build_capped(256, 128, vec![[1.; 3]; 256 * 128], 64).unwrap();
        assert_eq!((env.width, env.height), (64, 32));
        assert!(build(4, 2, vec![[0.; 3]; 8]).is_err());
    }
}
