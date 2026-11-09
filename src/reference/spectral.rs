//! Spectral mode (`Settings::spectral`, engine-notes "Spectral rendering"): a path carries three
//! wavelengths in its Vec3 channels instead of red, green and blue, so every per-channel product
//! of the path tracer is per wavelength. RGB colours become spectra where they enter (`up`, the
//! Mallett-Yuksel basis), a material's own spectrum (measured or blackbody) is read directly,
//! and the sample goes back to linear sRGB at its end (`to_rgb`). An event whose direction
//! depends on the wavelength (dispersion, a grating's orders, a dispersive lens) keeps the first
//! wavelength alone at three times the weight (`hero`). The GPU's model, implemented again:
//! the wavelengths come from the path's own random stream (`Rng::fork`), not a Weyl sequence.
use crate::lighting::PackedMaterial;
use crate::spectral_tables::{COUNT, D65, LO, MEASURED, OUT, STEP};
use glam::Vec3;
use std::sync::OnceLock;

/// The tables' upper end (nm).
const HI: f32 = LO + STEP * (COUNT - 1) as f32;
/// The wavelength importance's sharpness (`WAVE_A`): pdf ∝ sech²(a(λ − 538)).
const A: f32 = 0.0072;

/// The basis spectra per grid entry, parsed from the shader's table as the Vulkan backend does.
fn basis() -> &'static [[f32; 3]] {
    static T: OnceLock<Vec<[f32; 3]>> = OnceLock::new();
    T.get_or_init(|| {
        let v = crate::render::hlsl_table(
            include_str!("../../shaders/spectral_tables.hlsli"),
            "SPECTRAL_BASIS",
        );
        assert_eq!(v.len(), 3 * COUNT, "SPECTRAL_BASIS size");
        v.as_chunks::<3>().0.to_vec()
    })
}
/// Linear interpolation of a per-entry table at `l` nm, clamped to its range.
fn table<const N: usize>(t: &[[f32; N]], l: f32) -> [f32; N] {
    let x = ((l - LO) / STEP).clamp(0., COUNT as f32 - 1.001);
    let i = x as usize;
    let f = x - i as f32;
    std::array::from_fn(|k| t[i][k] + (t[i + 1][k] - t[i][k]) * f)
}
fn t_of(l: f32) -> f32 {
    (A * (l - 538.)).tanh()
}
/// The wavelength for u in [0, 1): the importance's inverse CDF in closed form.
pub(super) fn wave_sample(u: f32) -> f32 {
    let t = t_of(LO) + (t_of(HI) - t_of(LO)) * u;
    538. + 0.5 * ((1. + t) / (1. - t)).ln() / A
}
pub(super) fn wave_pdf(l: f32) -> f32 {
    let c = (A * (l - 538.)).cosh();
    A / ((t_of(HI) - t_of(LO)) * c * c)
}

/// A path's wavelengths (nm, `l`), the basis at each (`rows`), and whether only the first one
/// is left. Off (`rgb`): the channels are red, green and blue and every method is the identity.
#[derive(Clone, Copy)]
pub(super) struct Waves {
    pub(super) on: bool,
    pub(super) l: Vec3,
    rows: [Vec3; 3],
    pub(super) hero: bool,
}
impl Waves {
    pub(super) fn rgb() -> Self {
        Self {
            on: false,
            l: Vec3::splat(550.),
            rows: [Vec3::X, Vec3::Y, Vec3::Z],
            hero: false,
        }
    }
    /// Three wavelengths stratified over the importance from one number.
    pub(super) fn new(u: f32) -> Self {
        let l = Vec3::from_array(std::array::from_fn(|i| {
            wave_sample((u + i as f32 / 3.).fract())
        }));
        Self {
            on: true,
            l,
            rows: [0, 1, 2].map(|i| Vec3::from(table(basis(), l[i]))),
            hero: false,
        }
    }
    /// An RGB colour as the spectrum at the path's wavelengths.
    pub(super) fn up(&self, c: Vec3) -> Vec3 {
        if !self.on {
            return c;
        }
        Vec3::new(
            self.rows[0].dot(c),
            self.rows[1].dot(c),
            self.rows[2].dot(c),
        )
    }
    /// A sample's spectral radiance as linear sRGB: (1/3) Σ out(λi) v_i / p(λi). After `hero`
    /// the others are 0, so the same sum is the hero's estimate.
    pub(super) fn to_rgb(self, v: Vec3) -> Vec3 {
        if !self.on {
            return v;
        }
        (0..3)
            .map(|i| Vec3::from(table(&OUT, self.l[i])) * (v[i] / (3. * wave_pdf(self.l[i]))))
            .sum()
    }
    /// Keeps the first wavelength alone from here on, at 3 times its weight (once).
    pub(super) fn hero(&mut self, throughput: &mut Vec3) {
        if self.on && !self.hero {
            *throughput = Vec3::new(3. * throughput.x, 0., 0.);
            self.hero = true;
        }
    }
    /// An extinction as the sampling sees it: after `hero` the first wavelength's in all three,
    /// so distance sampling and its weights follow the one that is left.
    pub(super) fn sigma(&self, s: Vec3) -> Vec3 {
        if self.hero { Vec3::splat(s.x) } else { s }
    }
    /// A material's own spectrum (`phase.z`, `lighting::spectrum_fields`): > 0 a measured one
    /// (1–3 the Cornell walls, 4 its light times phase.w), < 0 a blackbody at −phase.z K, Planck
    /// relative to 560 nm over D65, times phase.w.
    fn own(&self, m: &PackedMaterial) -> Vec3 {
        let code = m.phase[2];
        Vec3::from_array(std::array::from_fn(|i| {
            let l = self.l[i];
            if code < 0. {
                let (t, r) = (-code, 560. / l);
                let [d65] = table(&D65.map(|d| [d]), l);
                r.powi(5) * ((1.438_777e7 / (560. * t)).exp() - 1.)
                    / ((1.438_777e7 / (l * t)).exp() - 1.)
                    / d65
                    * m.phase[3]
            } else {
                let s = table(&MEASURED, l);
                match code as u32 {
                    c @ 1..=3 => s[c as usize - 1],
                    _ => s[3] * m.phase[3],
                }
            }
        }))
    }
    /// A material's colour `c` as the path carries it: its own spectrum when it has one.
    pub(super) fn up_material(&self, m: &PackedMaterial, c: Vec3) -> Vec3 {
        if self.on && m.phase[2] != 0. {
            self.own(m)
        } else {
            self.up(c)
        }
    }
    /// Emitted RGB `x` of material `m` (its colour times scalars): its own spectrum scaled by the
    /// same factor, else the uplift.
    pub(super) fn up_emitted(&self, m: &PackedMaterial, x: Vec3) -> Vec3 {
        if self.on && m.phase[2] != 0. {
            let c = Vec3::from_slice(&m.color_rough[..3]).element_sum();
            return self.own(m) * (x.element_sum() / c.max(1e-12));
        }
        self.up(x)
    }
    /// Whether `m` disperses: spectral, with an Abbe number in phase.y.
    pub(super) fn dispersive(&self, m: &PackedMaterial) -> bool {
        self.on && m.phase[1] > 0.
    }
    /// `m`'s index of refraction: n_d (optical.x), or once the path keeps the hero alone,
    /// Cauchy's n(λ) = A + B/λ² through n_d at 587.6 nm with n_F − n_C = (n_d − 1)/V.
    pub(super) fn ior(&self, m: &PackedMaterial) -> f32 {
        let nd = m.optical[0];
        if !self.dispersive(m) || !self.hero {
            return nd;
        }
        let l = self.l.x * 1e-3;
        let b = (nd - 1.) / (m.phase[1] * (1. / (0.4861 * 0.4861) - 1. / (0.6563 * 0.6563)));
        nd + b * (1. / (l * l) - 1. / (0.5876 * 0.5876))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_inverts_the_pdf() {
        // The CDF's derivative is the pdf, and the samples span the tables.
        let du = 1e-3;
        for k in 1..999 {
            let u = k as f32 * du;
            let (a, b) = (wave_sample(u - du / 2.), wave_sample(u + du / 2.));
            let pdf = du / (b - a);
            let want = wave_pdf(wave_sample(u));
            assert!((pdf - want).abs() < 2e-3 * want, "u {u}: {pdf} vs {want}");
        }
        assert!((wave_sample(0.) - LO).abs() < 0.05 && (wave_sample(0.9999999) - HI).abs() < 0.5);
    }

    /// Up then out over many wavelength triples is the identity on RGB: the basis sums to a
    /// flat spectrum, which integrates to white. Within 3e-3: the product of two interpolated
    /// tables is not the tables' grid product (green's primary is off by 2.1e-3).
    #[test]
    fn uplift_round_trips() {
        let n = 20000;
        for c in [
            Vec3::ONE,
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(0.8, 0.3, 0.1),
            Vec3::new(0.05, 0.5, 0.9),
        ] {
            let mean: Vec3 = (0..n)
                .map(|k| {
                    let w = Waves::new((k as f32 + 0.5) / n as f32);
                    w.to_rgb(w.up(c))
                })
                .sum::<Vec3>()
                / n as f32;
            assert!((mean - c).abs().max_element() < 3e-3, "{c} -> {mean}");
        }
    }

    #[test]
    fn hero_keeps_the_first_wavelength() {
        let mut w = Waves::new(0.3);
        let mut t = Vec3::new(0.5, 0.2, 0.7);
        w.hero(&mut t);
        assert_eq!(t, Vec3::new(1.5, 0., 0.));
        w.hero(&mut t);
        assert_eq!(t, Vec3::new(1.5, 0., 0.));
        assert_eq!(w.sigma(Vec3::new(1., 2., 3.)), Vec3::ONE);
        let mut rgb = Waves::rgb();
        let mut t = Vec3::ONE;
        rgb.hero(&mut t);
        assert_eq!(t, Vec3::ONE);
    }

    #[test]
    fn cauchy_dispersion() {
        let mut m = <PackedMaterial as bytemuck::Zeroable>::zeroed();
        (m.optical[0], m.phase[1]) = (1.5168, 64.17);
        let at = |m: &PackedMaterial, nm: f32| {
            let mut w = Waves::new(0.);
            w.l.x = nm;
            let mut t = Vec3::ONE;
            w.hero(&mut t);
            w.ior(m)
        };
        assert!((at(&m, 587.6) - 1.5168).abs() < 1e-5);
        assert!(((at(&m, 486.1) - at(&m, 656.3)) - 0.5168 / 64.17).abs() < 1e-4);
        // Before the hero, and on a material without an Abbe number: n_d.
        assert_eq!(Waves::new(0.).ior(&m), 1.5168);
        m.phase[1] = 0.;
        assert_eq!(at(&m, 450.), 1.5168);
    }

    /// The measured and blackbody spectra integrate to the RGB colour the packed record holds.
    #[test]
    fn own_spectra_match_their_colours() {
        let n = 20000;
        for (code, w) in [(1., 1.), (3., 1.), (4., 2.), (-3000., 0.)] {
            let mut m = <PackedMaterial as bytemuck::Zeroable>::zeroed();
            let colour = if code < 0. {
                let (c, s) = crate::spectrum::blackbody_peak(-code);
                m.phase[3] = s;
                Vec3::from(c)
            } else {
                m.phase[3] = w;
                Vec3::from(crate::spectrum::measured(code as u32)) * if code == 4. { w } else { 1. }
            };
            m.phase[2] = code;
            let mean: Vec3 = (0..n)
                .map(|k| {
                    let wv = Waves::new((k as f32 + 0.5) / n as f32);
                    wv.to_rgb(wv.up_material(&m, Vec3::ZERO))
                })
                .sum::<Vec3>()
                / n as f32;
            assert!(
                (mean - colour).abs().max_element() < 3e-3 * colour.max_element().max(1.),
                "{code}: {colour} vs {mean}"
            );
        }
    }
}
