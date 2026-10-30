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
const MAX_WIDTH: usize = 4096;

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
}

pub fn luminance(c: [f32; 3]) -> f32 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(0.)
}

/// Loads an equirectangular map; relative paths are relative to the scenes folder.
pub fn load(path: &str) -> Result<EnvMap> {
    let full = crate::mesh::resolve(path);
    let image = image::open(&full)
        .with_context(|| format!("Reading {}", full.display()))?
        .into_rgb32f();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let texels: Vec<[f32; 3]> = image
        .pixels()
        .map(|p| p.0.map(|v| if v.is_finite() { v.max(0.) } else { 0. }))
        .collect();
    build(w, h, texels)
}

pub fn build(mut w: usize, mut h: usize, mut rgb: Vec<[f32; 3]>) -> Result<EnvMap> {
    ensure!(
        w >= 2 && h >= 1 && rgb.len() == w * h,
        "Bad environment size {w}×{h}"
    );
    while w > MAX_WIDTH && h >= 2 {
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
    let mut cdf = vec![0f32; w * (h + 1)];
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
    Ok(EnvMap {
        width: w,
        height: h,
        texels: rgb.into_iter().map(|c| [c[0], c[1], c[2], 1.]).collect(),
        cdf,
        inv_norm: ((w * h) as f64 / (2. * (PI as f64).powi(2) * total)) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let env = build(8192, 4096, vec![[1.; 3]; 8192 * 4096]).unwrap();
        assert_eq!((env.width, env.height), (4096, 2048));
        assert!(build(4, 2, vec![[0.; 3]; 8]).is_err());
    }
}
