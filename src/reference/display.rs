//! The display transform on the CPU, for images written with no device (`--reference out.png`,
//! CPU video and high-resolution stills): the same model as post.hlsl's beauty view (halation and
//! vignette on scene light, exposure, the tone map, the film look with its LUT, the output gamma,
//! then grain), written from its definitions rather than its code. The A/B split line, zebras
//! and HDR outputs are the screen's, not a file's. The software view on the GPU ignores the
//! direct / indirect sliders (they need the GPU's direct-light AOV), and so does this.
use crate::scene::Settings;
use anyhow::{Context, Result};
use glam::Vec3;

/// A film-look LUT: `size`³ RGB, red fastest (as in .cube files).
pub struct Lut {
    size: usize,
    data: Vec<[f32; 3]>,
}
impl Lut {
    /// The scene's LUT (`lut_path`, resolved as the app resolves it), or none.
    pub fn load(s: &Settings) -> Result<Option<Lut>> {
        if s.lut_path.is_empty() {
            return Ok(None);
        }
        let path = crate::mesh::resolve(&s.lut_path);
        let path = if path.exists() {
            path
        } else {
            s.lut_path.clone().into()
        };
        let text = std::fs::read_to_string(&path).with_context(|| format!("LUT {}", s.lut_path))?;
        let (size, data) = crate::look::read_cube(&text)?;
        Ok(Some(Lut {
            size: size as usize,
            data,
        }))
    }
    fn at(&self, x: usize, y: usize, z: usize) -> Vec3 {
        let n = self.size - 1;
        let [r, g, b] = self.data[(z.min(n) * self.size + y.min(n)) * self.size + x.min(n)];
        Vec3::new(r, g, b)
    }
    /// Trilinear lookup of an encoded colour in 0..1.
    fn sample(&self, e: Vec3) -> Vec3 {
        let x = e.clamp(Vec3::ZERO, Vec3::ONE) * (self.size - 1) as f32;
        let b = x.floor();
        let f = x - b;
        let [bx, by, bz] = [b.x as usize, b.y as usize, b.z as usize];
        let along_x = |y, z| self.at(bx, y, z).lerp(self.at(bx + 1, y, z), f.x);
        let near = along_x(by, bz).lerp(along_x(by + 1, bz), f.y);
        let far = along_x(by, bz + 1).lerp(along_x(by + 1, bz + 1), f.y);
        near.lerp(far, f.z)
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}
fn saturate(c: Vec3) -> Vec3 {
    c.clamp(Vec3::ZERO, Vec3::ONE)
}
fn powf(c: Vec3, e: f32) -> Vec3 {
    Vec3::new(c.x.powf(e), c.y.powf(e), c.z.powf(e))
}
fn smoothstep(c: Vec3) -> Vec3 {
    let t = saturate(c);
    t * t * (3. - 2. * t)
}
/// Hable's filmic curve (Uncharted 2).
fn filmic(x: f32) -> f32 {
    (x * (0.15 * x + 0.05) + 0.004) / (x * (0.15 * x + 0.5) + 0.06) - 0.066_666_67
}
fn aces(c: Vec3) -> Vec3 {
    saturate(c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14))
}
/// AgX: Benjamin Wrensch's polynomial fit of Troy Sobotka's base, from log2 exposure in
/// -12.47..4.03 stops; the matrices are its inset and outset (rows).
fn agx(c: Vec3) -> Vec3 {
    let inset = [
        [0.842479, 0.0784336, 0.0792237],
        [0.0423282, 0.878468, 0.0791661],
        [0.0423756, 0.0784336, 0.879142],
    ];
    let outset = [
        [1.19688, -0.0980209, -0.0990297],
        [-0.0528969, 1.15190, -0.0989612],
        [-0.0529716, -0.0980435, 1.15107],
    ];
    let mul = |m: [[f32; 3]; 3], v: Vec3| {
        Vec3::new(
            Vec3::from(m[0]).dot(v),
            Vec3::from(m[1]).dot(v),
            Vec3::from(m[2]).dot(v),
        )
    };
    let l = mul(inset, c).max(Vec3::splat(1e-10));
    let l = Vec3::new(l.x.log2(), l.y.log2(), l.z.log2())
        .clamp(Vec3::splat(-12.47393), Vec3::splat(4.026069));
    let x = (l + 12.47393) / 16.5;
    let (x2, x4) = (x * x, x * x * x * x);
    let p =
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - 0.00232;
    saturate(powf(mul(outset, p).max(Vec3::ZERO), 2.2))
}
/// Khronos PBR Neutral: base colours kept, highlights compressed toward white.
fn pbr_neutral(c: Vec3) -> Vec3 {
    let start = 0.8 - 0.04;
    let low = c.min_element();
    let c = c - if low < 0.08 {
        low - 6.25 * low * low
    } else {
        0.04
    };
    let peak = c.max_element();
    if peak < start {
        return c;
    }
    let d = 1. - start;
    let new_peak = 1. - d * d / (peak + d - start);
    let g = 1. - 1. / (0.15 * (peak - new_peak) + 1.);
    (c * (new_peak / peak)).lerp(Vec3::splat(new_peak), g)
}
/// Exposed scene light to display light (1 = white) by tone map `mode` (`Settings::tonemap`).
pub fn tone_map(c: Vec3, mode: u32) -> Vec3 {
    match mode {
        1 => saturate(c / (1. + c)),
        2 => saturate(c * (1. + c / 16.) / (1. + c)),
        3 => aces(c),
        4 => {
            saturate(Vec3::new(filmic(2. * c.x), filmic(2. * c.y), filmic(2. * c.z)) / filmic(11.2))
        }
        5 => smoothstep(aces(c)),
        6 => agx(c),
        7 => saturate(pbr_neutral(c)),
        _ => saturate(c),
    }
}
/// Lift, gamma and gain on the display-encoded value (1/2.2), then the LUT by its strength.
fn film_look(s: &Settings, lut: Option<&Lut>, t: Vec3) -> Vec3 {
    let e = powf(saturate(t), 1. / 2.2);
    let lift = Vec3::from(s.lift);
    let e = Vec3::from(s.gain) * (e + lift * (1. - e));
    let g = Vec3::from(s.grade_gamma).max(Vec3::splat(0.05));
    let mut e = saturate(e);
    e = Vec3::new(e.x.powf(1. / g.x), e.y.powf(1. / g.y), e.z.powf(1. / g.z));
    if let Some(lut) = lut.filter(|l| l.size > 1) {
        e = e.lerp(lut.sample(e), s.lut_strength);
    }
    powf(saturate(e), 2.2)
}
/// Film grain's per-cell noise in -0.5..0.5: cells of `size` pixels, `seed` per frame.
fn grain_noise(x: u32, y: u32, size: u32, seed: u32) -> f32 {
    let (cx, cy) = (x / size.max(1), y / size.max(1));
    let mut h = cx
        .wrapping_mul(1973)
        .wrapping_add(cy.wrapping_mul(9277))
        .wrapping_add(seed.wrapping_mul(26699));
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535. - 0.5
}

/// What a file of the display would hold: RGBA8 of linear RGBA `linear` (`size`, rows top first).
/// `seed` is the grain's frame number. Glare's diffraction (an FFT of the whole frame) is not
/// here yet, so a look with glare is refused rather than written without it.
pub fn display(s: &Settings, linear: &[f32], size: [u32; 2], seed: u32) -> Result<Vec<u8>> {
    anyhow::ensure!(
        s.glare <= 0.,
        "The CPU display transform has no glare yet (set glare=0, or write an EXR)"
    );
    let lut = Lut::load(s)?;
    let [w, h] = size.map(|v| v as usize);
    anyhow::ensure!(linear.len() == w * h * 4, "image size");
    let scene_at = |x: i32, y: i32| {
        let i = (y.clamp(0, h as i32 - 1) as usize * w + x.clamp(0, w as i32 - 1) as usize) * 4;
        Vec3::new(linear[i], linear[i + 1], linear[i + 2])
    };
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let mut c = scene_at(x as i32, y as i32);
        // Halation: light above white, from 24 taps on two rings, glowing red-orange.
        if s.halation > 0. {
            let r = h as f32 * 0.012;
            let mut glow = Vec3::ZERO;
            for i in 0..12 {
                let a = i as f32 * std::f32::consts::PI / 6.;
                for k in 1..=2 {
                    let o = Vec3::new(a.cos(), a.sin(), 0.) * r * k as f32;
                    // Truncated toward zero, as a float-to-int conversion is on the GPU.
                    glow += (scene_at(x as i32 + o.x as i32, y as i32 + o.y as i32) - 1.)
                        .max(Vec3::ZERO);
                }
            }
            c += glow / 24. * s.halation * Vec3::new(1., 0.3, 0.12);
        }
        if s.vignette > 0. {
            let qx = (x as f32 / w as f32 - 0.5) * (w as f32 / h as f32);
            let qy = y as f32 / h as f32 - 0.5;
            let v = 1. / (1. + (qx * qx + qy * qy) * 1.6);
            c *= 1. + (v * v - 1.) * s.vignette;
        }
        let exposed = (c * s.exposure).max(Vec3::ZERO);
        let t = tone_map(exposed, s.tonemap);
        let mut d = powf(film_look(s, lut.as_ref(), t), 1. / s.gamma.max(0.1));
        // Grain on the displayed value, strongest in the midtones.
        if s.grain > 0. {
            let size = [1, 2, 3][s.grain_size.min(2) as usize];
            let g = grain_noise(x as u32, y as u32, size, seed) * s.grain * 0.18;
            let l = luminance(saturate(d));
            d = saturate(d + g * (4. * l * (1. - l) + 0.15));
        }
        let q = |v: f32| (v.clamp(0., 1.) * 255. + 0.5) as u8;
        [q(d.x), q(d.y), q(d.z), 255]
    };
    let mut out = vec![0u8; w * h * 4];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = h.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk, band) in out.chunks_mut(rows * w * 4).enumerate() {
            let pixel = &pixel;
            scope.spawn(move || {
                for (i, p) in band.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let at = chunk * rows * w + i;
                    *p = pixel(at % w, at / w);
                }
            });
        }
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Settings {
        let mut s = Settings::default();
        (s.exposure, s.gamma, s.tonemap) = (1., 1., 0);
        (s.halation, s.vignette, s.grain, s.glare) = (0., 0., 0., 0.);
        (s.lift, s.grade_gamma, s.gain) = ([0.; 3], [1.; 3], [1.; 3]);
        s.lut_path.clear();
        s
    }

    #[test]
    fn tone_maps_at_known_points() {
        let v = |x| Vec3::splat(x);
        // Linear clips; Reinhard halves 1; extended Reinhard keeps 16 at white.
        assert_eq!(tone_map(v(2.), 0), v(1.));
        assert!((tone_map(v(1.), 1).x - 0.5).abs() < 1e-6);
        assert!((tone_map(v(16.), 2).x - 1.).abs() < 1e-5);
        // ACES (Narkowicz): 0 stays 0, high values clip.
        assert!(tone_map(v(0.), 3).x.abs() < 1e-6);
        assert_eq!(tone_map(v(100.), 3), v(1.));
        // Hable at its white point 11.2 (input doubled) is 1.
        assert!((tone_map(v(5.6), 4).x - 1.).abs() < 1e-5);
        // AgX maps middle grey near 0.18 display-linear's usual ~0.2 range, and is monotonic.
        let a = tone_map(v(0.18), 6).x;
        assert!(a > 0.1 && a < 0.35, "{a}");
        assert!(tone_map(v(1.), 6).x > a);
        // PBR Neutral leaves base colours under its start untouched (after the toe offset).
        assert!((tone_map(v(0.5), 7).x - 0.46).abs() < 1e-6);
        for mode in 0..8 {
            let mut last = -1.;
            for i in 0..64 {
                let y = tone_map(v(i as f32 * 0.25), mode).x;
                assert!((0. ..=1.).contains(&y) && y >= last - 1e-6, "mode {mode}");
                last = y;
            }
        }
    }

    #[test]
    fn plain_display_is_clamped_linear() {
        let s = plain();
        let image = [0.25f32, 0.5, 2., 1.];
        assert_eq!(
            display(&s, &image, [1, 1], 0).unwrap(),
            vec![64, 128, 255, 255]
        );
        let mut s = plain();
        s.gamma = 2.2;
        let out = display(&s, &[0.5, 0.5, 0.5, 1.], [1, 1], 0).unwrap();
        assert_eq!(out[0], (0.5f32.powf(1. / 2.2) * 255. + 0.5) as u8);
    }

    #[test]
    fn look_vignette_halation_and_grain() {
        // Lift raises black; gain scales the encoded value.
        let mut s = plain();
        s.lift = [0.2; 3];
        let black = display(&s, &[0., 0., 0., 1.], [1, 1], 0).unwrap();
        assert_eq!(black[0], (0.2f32.powf(2.2) * 255. + 0.5) as u8);
        // The vignette darkens corners, not the centre.
        let mut s = plain();
        s.vignette = 1.;
        let grey = [0.5f32, 0.5, 0.5, 1.].repeat(9 * 9);
        let out = display(&s, &grey, [9, 9], 0).unwrap();
        assert!(out[0] < out[(4 * 9 + 4) * 4] && out[(4 * 9 + 4) * 4] >= 120);
        // Halation adds red around a highlight.
        let mut s = plain();
        s.halation = 1.;
        let mut img = [0f32, 0., 0., 1.].repeat(100 * 100);
        for y in 45..55 {
            for x in 45..55 {
                img[(y * 100 + x) * 4..][..3].copy_from_slice(&[3., 3., 3.]);
            }
        }
        let out = display(&s, &img, [100, 100], 0).unwrap();
        let i = (50 * 100 + 55) * 4;
        assert!(out[i] > out[i + 2], "{:?}", &out[i..i + 4]);
        // Grain changes midtones, by frame.
        let mut s = plain();
        s.grain = 1.;
        let grey = [0.5f32, 0.5, 0.5, 1.].repeat(64);
        let a = display(&s, &grey, [8, 8], 0).unwrap();
        assert_ne!(a, display(&s, &grey, [8, 8], 1).unwrap());
        assert!(a.chunks(4).any(|p| p[0] != 128));
        // Glare is refused, not left out.
        let mut s = plain();
        s.glare = 0.5;
        assert!(display(&s, &grey, [8, 8], 0).is_err());
    }

    #[test]
    fn lut_identity_and_inverse() {
        let n = 4;
        let mut data = vec![];
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let v = |i: usize| i as f32 / (n - 1) as f32;
                    data.push([1. - v(r), 1. - v(g), 1. - v(b)]);
                }
            }
        }
        let lut = Lut { size: n, data };
        let c = lut.sample(Vec3::new(0.2, 0.5, 0.9));
        assert!(
            (c - Vec3::new(0.8, 0.5, 0.1)).abs().max_element() < 1e-5,
            "{c}"
        );
    }
}
