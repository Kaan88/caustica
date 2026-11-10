//! The material model, as pathtrace.hlsl evaluates it.
use crate::lighting::PackedMaterial;
use crate::reference::sampling::Rng;
use glam::{Vec2, Vec3};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------------------------
// The GGX directional albedo (layers.hlsli GGX_ALBEDO, 32 × 32 over cos θ and roughness).

/// A fitted table from layers.hlsli (tools/fit_layers.py), `n` × `n` over cos θ and roughness.
fn layers_table(name: &str, n: usize) -> Vec<f32> {
    let v = crate::render::hlsl_table(include_str!("../../shaders/layers.hlsli"), name);
    assert_eq!(v.len(), n * n, "{name} has {} entries", n * n);
    v
}
pub(super) fn ggx_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| layers_table("GGX_ALBEDO", 32))
}
/// Bilinear over an `n` × `n` table, rows of roughness, columns of cos θ.
fn lookup(t: &[f32], n: usize, nv: f32, roughness: f32) -> f32 {
    let k = (n - 1) as f32;
    let x = Vec2::new(nv.clamp(0., 1.), roughness.clamp(0., 1.)) * k;
    let (ix, iy) = ((x.x as usize).min(n - 2), (x.y as usize).min(n - 2));
    let (fx, fy) = (x.x - ix as f32, x.y - iy as f32);
    let s = |c: usize, r: usize| t[(iy + r) * n + ix + c];
    let lerp = |a: f32, b: f32, f: f32| a + (b - a) * f;
    lerp(lerp(s(0, 0), s(1, 0), fx), lerp(s(0, 1), s(1, 1), fx), fy)
}
pub(super) fn ggx_albedo(nv: f32, roughness: f32) -> f32 {
    lookup(ggx_table(), 32, nv, roughness)
}
/// What a clear coat reflects from view cos θ (Fresnel of IOR 1.5 included): what it takes from
/// the layers below.
pub(super) fn coat_albedo(nv: f32, roughness: f32) -> f32 {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    lookup(
        TABLE.get_or_init(|| layers_table("COAT_ALBEDO", 32)),
        32,
        nv,
        roughness,
    )
}
/// The Charlie sheen's directional albedo.
pub(super) fn sheen_albedo(nv: f32, roughness: f32) -> f32 {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    lookup(
        TABLE.get_or_init(|| layers_table("SHEEN_ALBEDO", 16)),
        16,
        nv,
        roughness,
    )
}

/// The GGX lobe's directional albedo with anisotropy (`aniso_albedo`, 12 × 4 × 8 × 6 over cos θ,
/// the view's azimuth from T in quarter turns, roughness and strength): multilinear in all four.
pub(super) fn aniso_albedo(nv: f32, azimuth: f32, roughness: f32, strength: f32) -> f32 {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let v =
            crate::render::hlsl_table(include_str!("../../shaders/layers.hlsli"), "ANISO_ALBEDO");
        assert_eq!(v.len(), 12 * 4 * 8 * 6, "ANISO_ALBEDO has 2304 entries");
        v
    });
    const SIZE: [usize; 4] = [12, 4, 8, 6];
    let mut base = [0usize; 4];
    let mut frac = [0f32; 4];
    for (k, x) in [nv, azimuth, roughness, strength].into_iter().enumerate() {
        let x = x.clamp(0., 1.) * (SIZE[k] - 1) as f32;
        base[k] = (x as usize).min(SIZE[k] - 2);
        frac[k] = x - base[k] as f32;
    }
    let mut e = 0.;
    for corner in 0..16usize {
        let (mut index, mut weight, mut stride) = (0, 1., 1);
        for k in 0..4 {
            let up = (corner >> k) & 1;
            weight *= if up == 1 { frac[k] } else { 1. - frac[k] };
            index += (base[k] + up) * stride;
            stride *= SIZE[k];
        }
        e += weight * t[index];
    }
    e
}
/// The brushing frame (T, B) in the plane of `n`: the hit's tangent projected, turned by
/// `rotation` radians about n (`aniso_frame`).
pub(super) fn aniso_frame(n: Vec3, t: Vec3, rotation: f32) -> (Vec3, Vec3) {
    let p = t - n * n.dot(t);
    let t0 = if p.length_squared() > 1e-12 {
        p.normalize()
    } else {
        n.any_orthonormal_pair().0
    };
    let (s, c) = rotation.sin_cos();
    let t = t0 * c + n.cross(t0) * s;
    (t, n.cross(t))
}
/// Alpha along the grooves (T) and across them (B): the strength² widens it toward 1.
pub(super) fn aniso_alpha(m: &PackedMaterial) -> Vec2 {
    let a = (m.color_rough[3] * m.color_rough[3]).max(0.001);
    let w = m.layers[3];
    Vec2::new(a, a + (1. - a) * w * w)
}
/// Anisotropic GGX D for a half vector in the local frame (x on T, y on B, z on n).
pub(super) fn ggx_d_aniso(h: Vec3, a: Vec2) -> f32 {
    let s = Vec3::new(h.x / a.x, h.y / a.y, h.z);
    let k = s.length_squared();
    1. / (PI * a.x * a.y * k * k)
}
/// Smith G1 of the anisotropic GGX for a direction in the local frame.
pub(super) fn g1_aniso(w: Vec3, a: Vec2) -> f32 {
    let root = (w.x * w.x * a.x * a.x + w.y * w.y * a.y * a.y + w.z * w.z).sqrt();
    2. * w.z / (w.z + root).max(1e-6)
}

/// Below this roughness the sheen's distribution is too sharp for its fitted visibility.
const SHEEN_MIN_ROUGHNESS: f32 = 0.07;
/// Charlie sheen (Estevez and Kulla 2017): the inverted-Gaussian-like sin^(1/α) distribution
/// with their fitted Λ for the visibility, scaled down where its albedo would pass 1.
pub(super) fn sheen_lobe(roughness: f32, nh: f32, nv: f32, nl: f32) -> f32 {
    let a = roughness.max(SHEEN_MIN_ROUGHNESS).powi(2);
    let t = (1. - a) * (1. - a);
    let mix = |p: f32, q: f32| p + (q - p) * t;
    let l = |x: f32| {
        mix(21.5473, 25.3245) / (1. + mix(3.82987, 3.32435) * x.powf(mix(0.19823, 0.16801)))
            + mix(-1.97760, -1.27393) * x
            + mix(-4.32054, -4.85967)
    };
    let lambda = |c: f32| {
        if c < 0.5 {
            l(c).exp()
        } else {
            (2. * l(0.5) - l(1. - c)).exp()
        }
    };
    let d = (2. + 1. / a) * (1. - nh * nh).max(0.).powf(0.5 / a) / (2. * PI);
    let vis = (1. / ((1. + lambda(nv) + lambda(nl)) * 4. * (nv * nl).max(1e-6))).clamp(0., 1.);
    d * vis / sheen_albedo(nv, roughness).max(1.)
}
/// The share of BSDF samples the clear coat's own GGX lobe takes.
pub(super) fn coat_probability(m: &PackedMaterial) -> f32 {
    0.3 * m.layers[0]
}

// ---------------------------------------------------------------------------------------------
// Material model, as pathtrace.hlsl evaluates it.

pub(super) fn rgb(v: [f32; 4]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
pub(super) fn fresnel(c: f32, f0: Vec3) -> Vec3 {
    f0 + (Vec3::ONE - f0) * (1. - c.clamp(0., 1.)).powi(5)
}
/// Exact unpolarised Fresnel reflectance, eta = n1 / n2 (`fresnel_dielectric`).
pub(super) fn fresnel_dielectric(cosi: f32, eta: f32) -> f32 {
    let s2 = eta * eta * (1. - cosi * cosi).max(0.);
    if s2 >= 1. {
        return 1.;
    }
    let ct = (1. - s2).sqrt();
    let rs = (eta * cosi - ct) / (eta * cosi + ct);
    let rp = (cosi - eta * ct) / (cosi + eta * ct);
    0.5 * (rs * rs + rp * rp)
}
pub(super) fn ggx_d(nh: f32, a: f32) -> f32 {
    let d = nh * nh * (a * a - 1.) + 1.;
    a * a / (PI * d * d)
}
pub(super) fn g1(nv: f32, a: f32) -> f32 {
    2. * nv / (nv + (a * a + (1. - a * a) * nv * nv).sqrt()).max(1e-6)
}
pub(super) fn f0_of(m: &PackedMaterial) -> Vec3 {
    let f = ((m.optical[0] - 1.) / (m.optical[0] + 1.)).powi(2);
    Vec3::splat(f)
        .lerp(Vec3::ONE, m.lobes[1])
        .lerp(rgb(m.color_rough), m.lobes[0])
}
// ---------------------------------------------------------------------------------------------
// Thin film (glTF's KHR_materials_iridescence, Belcour and Barla 2017): an Airy sum over the
// first two orders of the light bouncing inside a film between the outside and the base, its
// spectral integral in closed form against Gaussian fits of the CIE XYZ matching functions.

/// Schlick's approximation for one reflectance at normal incidence.
fn schlick(f0: f32, c: f32) -> f32 {
    f0 + (1. - f0) * (1. - c.clamp(0., 1.)).powi(5)
}
/// The phase-shifted fringe `cos(2π·opd/λ + shift)` integrated against the matching functions
/// (the Khronos viewer's fit), in linear Rec. 709. `opd` is in nm.
pub(super) fn film_sensitivity(opd: f32, shift: Vec3) -> Vec3 {
    // One Gaussian per XYZ channel (value, centre frequency, variance), and a second lobe on X.
    const GAUSS: [(f32, f32, f32); 3] = [
        (5.4856e-13, 1.6810e6, 4.3278e9),
        (4.4201e-13, 1.7953e6, 9.3046e9),
        (5.2481e-13, 2.2084e6, 6.6121e9),
    ];
    let x = 2. * PI * opd * 1e-9;
    let term = |(val, pos, var): (f32, f32, f32), shift: f32| {
        val * (2. * PI * var).sqrt() * (pos * x + shift).cos() * (-x * x * var).exp()
    };
    let xyz = Vec3::new(
        term(GAUSS[0], shift.x) + term((9.7470e-14, 2.2399e6, 4.5282e9), shift.x),
        term(GAUSS[1], shift.y),
        term(GAUSS[2], shift.z),
    ) / 1.0685e-7;
    glam::Mat3::from_cols(
        Vec3::new(3.2404542, -0.969_266, 0.0556434),
        Vec3::new(-1.5371385, 1.8760108, -0.2040259),
        Vec3::new(-0.4985314, 0.0415560, 1.0572252),
    ) * xyz
}
/// The film's reflectance at view cosine `c1`: film IOR `eta2` and base IOR `eta3` (per
/// channel) relative to the outside, thickness in nm. With `waves` (spectral, nm per channel)
/// the Airy terms are exact at those wavelengths; without, the RGB fit (`film_sensitivity`).
pub(super) fn film_fresnel(
    c1: f32,
    eta2: f32,
    eta3: Vec3,
    thickness: f32,
    waves: Option<Vec3>,
) -> Vec3 {
    // A film thinner than ~30 nm fades into the outside medium, leaving the base's Fresnel.
    let fade = (thickness / 30.).clamp(0., 1.);
    let eta2 = 1. + (eta2 - 1.) * fade * fade * (3. - 2. * fade);
    let s2 = (1. - c1 * c1) / (eta2 * eta2);
    if s2 >= 1. {
        return Vec3::ONE;
    }
    let c2 = (1. - s2).sqrt();
    let r12 = schlick(((eta2 - 1.) / (eta2 + 1.)).powi(2), c1);
    let t121 = 1. - r12;
    // Reflection off a denser medium shifts the phase by half a wave.
    let half = |denser: bool| if denser { PI } else { 0. };
    let r23 = eta3.map(|e| schlick(((e - eta2) / (e + eta2)).powi(2), c2));
    let phi = eta3.map(|e| half(eta2 >= 1.) + half(e < eta2));
    let opd = 2. * eta2 * thickness * c2;
    let r123 = (r12 * r23).clamp(Vec3::splat(1e-5), Vec3::splat(0.9999));
    let rs = t121 * t121 * r23 / (Vec3::ONE - r123);
    let sq = r123.map(f32::sqrt);
    let mut sum = Vec3::splat(r12) + rs;
    let mut cm = rs - Vec3::splat(t121);
    for k in 1..=2 {
        let k = k as f32;
        cm *= sq;
        sum += cm
            * 2.
            * match waves {
                Some(l) => (k * (2. * PI * opd / l + phi)).map(f32::cos),
                None => film_sensitivity(k * opd, k * phi),
            };
    }
    sum.clamp(Vec3::ZERO, Vec3::ONE)
}
/// The film's thickness (nm) and IOR from `surface_scale.w` (lighting.rs `film_code`).
pub(super) fn film_decode(w: f32) -> (f32, f32) {
    let ior = (w / 2048.).floor();
    (w - ior * 2048., ior / 100.)
}
/// Evaluates the film once for the hit at view cosine `c` (as glTF's model does) into the hit's
/// copy of the material, `base_scale.rgb` (`prepare_film`). `outside` and `base` are the IORs on
/// both sides; base 0 is an opaque base, whose IOR per channel comes from its f0. `waves`: the
/// spectral path's wavelengths, on a material already uplifted (`prepare_film_waves`).
pub(super) fn prepare_film(
    m: &mut PackedMaterial,
    c: f32,
    outside: f32,
    base: f32,
    waves: Option<Vec3>,
) {
    if m.surface_scale[2] <= 0. {
        return;
    }
    let eta3 = if base > 0. {
        Vec3::splat(base / outside)
    } else {
        let q = f0_of(m)
            .clamp(Vec3::ZERO, Vec3::splat(0.9999))
            .map(f32::sqrt);
        (Vec3::ONE + q) / (Vec3::ONE - q)
    };
    let (thickness, ior) = film_decode(m.surface_scale[3]);
    let f = film_fresnel(c, ior / outside, eta3, thickness, waves);
    m.base_scale[..3].copy_from_slice(&f.to_array());
}
// Diffraction grating (a CD's or DVD's spiral): order o of wavelength λ leaves where the
// tangential part of the direction is the mirror's moved by o·λ/spacing across the grooves (B of
// `aniso_frame`). Moving on the projected disk keeps f·cos·dω, so each order is the isotropic GGX
// lobe translated: f_o(l) = f(l0), pdf_o(l) = pdf(l0)·cos l / cos l0, energy lost only past the
// horizon. Orders ±1 take 0.35 of the strength each, ±2 0.15 each.
pub(super) const GRATING_ORDER: [f32; 4] = [-1., 1., -2., 2.];
const GRATING_SHARE: [f32; 4] = [0.35, 0.35, 0.15, 0.15];
// Three Gaussian channel curves of equal area (nm), so a flat spectrum is white.
const GRATING_MU: Vec3 = Vec3::new(600., 545., 455.);
const GRATING_SIGMA: Vec3 = Vec3::new(40., 35., 25.);
/// The grating of `m`, strength and groove spacing (nm), from `optical.w` < 0
/// (`lighting::grating_code`); an IES light's record holds other data there.
pub(super) fn grating(m: &PackedMaterial) -> Option<(f32, f32)> {
    let c = -m.optical[3];
    if c <= 0. || m.lobes[3] as u32 & 16384 != 0 {
        return None;
    }
    let hi = (c / 16384.).floor();
    Some((hi / 1000., c - hi * 16384.))
}
/// The direction whose lobe a move of `shift` along B carries to `l`, or `None` past the horizon.
pub(super) fn grating_move(n: Vec3, b: Vec3, l: Vec3, shift: f32) -> Option<Vec3> {
    let lt = l - n * n.dot(l) + b * shift;
    let s = lt.length_squared();
    (s < 1.).then(|| lt + n * (1. - s).sqrt())
}
/// Each order's share at view `v` (k = λ/spacing): its weight × the strength when the mirror
/// direction's order leaves above the horizon, else 0 (an evanescent order's energy stays in
/// the mirror). Depends on v and λ only, so sampling and evaluation agree.
pub(super) fn order_shares(share: f32, k: f32, n: Vec3, b: Vec3, v: Vec3) -> [f32; 4] {
    let mirror = n * (2. * n.dot(v)) - v;
    std::array::from_fn(|o| {
        if grating_move(n, b, mirror, GRATING_ORDER[o] * k).is_some() {
            share * GRATING_SHARE[o]
        } else {
            0.
        }
    })
}
/// A grating hit's wavelength (nm): one of the three channel curves, then a Box-Muller normal
/// draw from it, clamped to 300–900.
pub(super) fn grating_wavelength(r: &mut Rng) -> f32 {
    let c = ((r.f() * 3.) as usize).min(2);
    let (u1, u2) = (1. - r.f(), r.f());
    let z = (-2. * u1.ln()).sqrt() * (2. * PI * u2).cos();
    (GRATING_MU[c] + GRATING_SIGMA[c] * z).clamp(300., 900.)
}
/// A grating hit's wavelength (nm, 0 without a grating) and the weight its orders take: RGB
/// draws one (`grating_wavelength`) and weighs it into the channels (`grating_weight`); the
/// spectral path takes its hero wavelength at weight 1.
#[derive(Clone, Copy)]
pub(super) struct Wavelength {
    pub(super) nm: f32,
    pub(super) weight: Vec3,
}
impl Wavelength {
    pub(super) const NONE: Self = Self {
        nm: 0.,
        weight: Vec3::ONE,
    };
    pub(super) fn rgb(nm: f32) -> Self {
        Self {
            nm,
            weight: grating_weight(nm),
        }
    }
}
/// The RGB weight of wavelength `lambda` drawn by `grating_wavelength`: 3·N_c(λ) / Σ N (at
/// most 3 per channel, mean white over the draw).
pub(super) fn grating_weight(lambda: f32) -> Vec3 {
    let x = (Vec3::splat(lambda) - GRATING_MU) / GRATING_SIGMA;
    let nc = (x * x * -0.5).exp() / GRATING_SIGMA;
    3. * nc / nc.element_sum().max(1e-30)
}
/// The base specular's Fresnel at cosine `c`, blended toward the prepared film by its strength.
pub(super) fn spec_fresnel(m: &PackedMaterial, c: f32, f0: Vec3) -> Vec3 {
    let f = fresnel(c, f0);
    let strength = m.surface_scale[2];
    if strength > 0. {
        f.lerp(rgb(m.base_scale), strength)
    } else {
        f
    }
}
pub(super) fn spec_probability(m: &PackedMaterial) -> f32 {
    (0.25 + 0.5 * m.lobes[0] + 0.25 * m.lobes[1]).clamp(0.1, 0.95)
}
/// `eval_bsdf` (a thin film is read from `base_scale`, `prepare_film`): the value and the pdf of
/// the path tracer's sampling. `lambda` is the hit's grating wavelength (`Wavelength`). `t` is the hit's tangent, read by a grating's grooves and by an
/// anisotropic base (layers.w > 0), whose pdf is that of
/// visible-normal sampling. A sheen sits on the base and a clear coat on both; each dims what lies below by the
/// albedo it takes from the view direction (glTF's layering), so the result stays one function
/// for light and BSDF samples alike.
pub(super) fn eval_bsdf(
    m: &PackedMaterial,
    n: Vec3,
    t: Vec3,
    v: Vec3,
    l: Vec3,
    lambda: Wavelength,
) -> (Vec3, f32) {
    let (f, pdf, _) = eval_bsdf_parts(m, n, t, v, l, lambda);
    (f, pdf)
}
/// `eval_bsdf` and the diffuse part of its value: the base's diffuse lobe as the sheen and the
/// coat above it leave it (light path expressions split a vertex into D and G or S by it).
pub(super) fn eval_bsdf_parts(
    m: &PackedMaterial,
    n: Vec3,
    t: Vec3,
    v: Vec3,
    l: Vec3,
    lambda: Wavelength,
) -> (Vec3, f32, Vec3) {
    let nl = n.dot(l).clamp(0., 1.);
    let nv = n.dot(v).clamp(0., 1.);
    let h = (v + l).normalize_or_zero();
    let nh = n.dot(h).clamp(0., 1.);
    let vh = v.dot(h).clamp(0., 1.);
    let rough = m.color_rough[3];
    let a = (rough * rough).max(0.001);
    let f0 = f0_of(m);
    let f = spec_fresnel(m, vh, f0);
    // The energy lost to single scattering follows the film's reflectance at normal incidence.
    let fc = if m.surface_scale[2] > 0. {
        spec_fresnel(m, 1., f0)
    } else {
        f0
    };
    let ps = spec_probability(m);
    let (d, gv, gl, ess, spec_pdf) = if m.layers[3] <= 0. {
        let d = ggx_d(nh, a);
        let spec_pdf = d * nh / (4. * vh).max(1e-6);
        (d, g1(nv, a), g1(nl, a), ggx_albedo(nv, rough), spec_pdf)
    } else {
        let (tt, bb) = aniso_frame(n, t, m.sheen[3]);
        let aa = aniso_alpha(m);
        let local = |w: Vec3, z: f32| Vec3::new(w.dot(tt), w.dot(bb), z);
        let d = ggx_d_aniso(local(h, nh), aa);
        let gv = g1_aniso(local(v, nv), aa);
        let (vt, vb) = (v.dot(tt).abs(), v.dot(bb).abs());
        let azimuth = if vt + vb > 1e-6 {
            vb.atan2(vt) * (2. / PI)
        } else {
            0.
        };
        let ess = aniso_albedo(nv, azimuth, rough, m.layers[3]);
        (
            d,
            gv,
            g1_aniso(local(l, nl), aa),
            ess,
            gv * d / (4. * nv).max(1e-6),
        )
    };
    let mut pdf = (1. - ps) * nl / PI + ps * spec_pdf;
    let diff = (Vec3::ONE - f) * (1. - m.lobes[0]) * rgb(m.color_rough) / PI;
    let compensation = Vec3::ONE + fc * (1. / ess - 1.);
    let mut r = diff + f * d * gv * gl / (4. * nv * nl).max(1e-6) * compensation;
    let mut diffuse = diff;
    if let Some((share, spacing)) = grating(m).filter(|_| lambda.nm > 0. && nl > 0.) {
        // Each order is the isotropic lobe evaluated where the move across the grooves takes l
        // back from; the mirror keeps what the orders that leave don't take.
        let (_, b) = aniso_frame(n, t, m.sheen[3]);
        let k = lambda.nm / spacing;
        let w = order_shares(share, k, n, b, v);
        let (mut orders, mut op) = (Vec3::ZERO, 0.);
        for (o, &wo) in GRATING_ORDER.iter().zip(&w) {
            let Some(l0) = grating_move(n, b, l, -o * k).filter(|_| wo > 0.) else {
                continue;
            };
            let nl0 = n.dot(l0).max(1e-6);
            let h0 = (v + l0).normalize_or_zero();
            let (nh0, vh0) = (n.dot(h0).clamp(0., 1.), v.dot(h0).clamp(0., 1.));
            let d0 = ggx_d(nh0, a);
            orders += wo * spec_fresnel(m, vh0, f0) * d0 * g1(nl0, a) / nl0;
            // pdf_o(l) = pdf(l0) cos l / cos l0: the move keeps projected solid angle.
            op += wo * d0 * nh0 / (4. * vh0).max(1e-6) * nl / nl0;
        }
        let g: f32 = w.iter().sum();
        r = diff
            + (r - diff) * (1. - g)
            + orders * (g1(nv, a) / (4. * nv).max(1e-6)) * compensation * lambda.weight;
        pdf = (1. - ps) * nl / PI + ps * ((1. - g) * spec_pdf + op);
    }
    let sheen = rgb(m.sheen);
    if sheen.max_element() > 0. {
        let sr = m.layers[2];
        let below = 1. - sheen.max_element() * sheen_albedo(nv, sr).min(1.);
        r = r * below + sheen * sheen_lobe(sr, nh, nv, nl);
        diffuse *= below;
    }
    let coat = m.layers[0];
    if coat > 0. {
        let cr = m.layers[1];
        let ac = (cr * cr).max(0.001);
        let dc = ggx_d(nh, ac);
        // Compensated in full (1/E): the coat is clear, so what multiple scattering keeps is white.
        let fc = 0.04 + 0.96 * (1. - vh).powi(5);
        let lobe =
            fc * dc * g1(nv, ac) * g1(nl, ac) / (4. * nv * nl).max(1e-6) / ggx_albedo(nv, cr);
        let below = 1. - coat * coat_albedo(nv, cr);
        r = r * below + Vec3::splat(coat * lobe);
        diffuse *= below;
        let pc = coat_probability(m);
        pdf = pdf * (1. - pc) + pc * dc * nh / (4. * vh).max(1e-6);
    }
    let keep = 1. - m.lobes[2];
    (keep * r, keep * pdf, keep * diffuse)
}
/// The pattern colour at p (`material_at`).
pub(super) fn material_at(m: &PackedMaterial, p: Vec3) -> PackedMaterial {
    let mut m = *m;
    let s = m.secondary[3];
    let t = match m.optical[2] as u32 {
        1 => ((p.x * s).floor() + (p.z * s).floor()).abs() % 2.,
        2 => {
            if ((p.x + p.z) * s * PI).sin() >= 0. {
                1.
            } else {
                0.
            }
        }
        3 => {
            let vein = ((p.x + p.z) * s * 0.6
                + 3. * (p.y * 3. + p.z * 2.).sin()
                + 1.5 * (p.x * 5. - p.z * 2.).sin()
                + 0.6 * (p.x * 13. + p.y * 11.).sin()
                + 0.3 * (p.z * 27. - p.y * 21.).sin())
            .sin();
            0.75 * (1. - vein.abs()).powi(6)
        }
        _ => 0.,
    };
    let c = rgb(m.color_rough).lerp(rgb(m.secondary), t);
    m.color_rough[..3].copy_from_slice(&c.to_array());
    m
}
pub(super) fn flags(m: &PackedMaterial) -> u32 {
    m.lobes[3] as u32
}
/// Sample `i` of a light's IES profile (flag 16384), from the fields `lighting::materials`
/// stores `ies::pack`'s 20 words in, in that order: three bytes per word, low first.
pub(super) fn ies_sample(m: &PackedMaterial, i: usize) -> f32 {
    let k = i / 3;
    let word = match k {
        0..4 => m.layers[k],
        4..8 => m.sheen[k - 4],
        8..12 => m.base_scale[k - 8],
        12..16 => m.surface_scale[k - 12],
        16..19 => m.medium[k - 16],
        _ => m.phase[0],
    };
    ((word as u32 >> (8 * (i % 3))) & 255) as f32 / 255.
}
/// The profile's relative intensity at cosine `c` from the light's normal: samples evenly
/// spaced in angle over 0–90°, interpolated linearly in angle; nothing behind the light.
pub(super) fn ies_profile(m: &PackedMaterial, c: f32) -> f32 {
    if c <= 0. {
        return 0.;
    }
    let last = crate::ies::SAMPLES - 1;
    let x = c.min(1.).acos() / std::f32::consts::FRAC_PI_2 * last as f32;
    let i = (x as usize).min(last - 1);
    let (a, b) = (ies_sample(m, i), ies_sample(m, i + 1));
    a + (b - a) * (x - i as f32)
}
/// A light's factor toward cosine `c` from its normal: its IES profile (flag 16384), which
/// replaces a spot's cone, else the spot falloff (flag 32): smoothstep between the cone's
/// cosines, else 1.
pub(super) fn spot(m: &PackedMaterial, c: f32) -> f32 {
    if flags(m) & 16384 != 0 {
        return ies_profile(m, c);
    }
    if flags(m) & 32 == 0 {
        return 1.;
    }
    let (e0, e1) = (m.secondary[0], m.secondary[1]);
    let t = ((c - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
/// Emitted radiance leaving a surface of colour `colour` (`Scene::emission_colour`: the
/// material's own, or its emissive map's texel) toward a viewer at cosine `c` on side `front`.
pub(super) fn emitted(m: &PackedMaterial, colour: Vec3, front: bool, c: f32) -> Vec3 {
    if m.optical[1] <= 0. || (m.optical[3] > 0.5 && !front) {
        return Vec3::ZERO;
    }
    colour * m.optical[1] * spot(m, c)
}
