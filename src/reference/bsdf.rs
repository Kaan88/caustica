//! The material model, as pathtrace.hlsl evaluates it.
use crate::lighting::PackedMaterial;
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
pub(super) fn spec_probability(m: &PackedMaterial) -> f32 {
    (0.25 + 0.5 * m.lobes[0] + 0.25 * m.lobes[1]).clamp(0.1, 0.95)
}
/// `eval_bsdf` without anisotropy, gratings or films: the value and the pdf of the sampling
/// below. A sheen sits on the base and a clear coat on both; each dims what lies below by the
/// albedo it takes from the view direction (glTF's layering), so the result stays one function
/// for light and BSDF samples alike.
pub(super) fn eval_bsdf(m: &PackedMaterial, n: Vec3, v: Vec3, l: Vec3) -> (Vec3, f32) {
    let nl = n.dot(l).clamp(0., 1.);
    let nv = n.dot(v).clamp(0., 1.);
    let h = (v + l).normalize_or_zero();
    let nh = n.dot(h).clamp(0., 1.);
    let vh = v.dot(h).clamp(0., 1.);
    let rough = m.color_rough[3];
    let a = (rough * rough).max(0.001);
    let d = ggx_d(nh, a);
    let f0 = f0_of(m);
    let f = fresnel(vh, f0);
    let ps = spec_probability(m);
    let (gv, gl) = (g1(nv, a), g1(nl, a));
    let ess = ggx_albedo(nv, rough);
    let spec_pdf = d * nh / (4. * vh).max(1e-6);
    let pdf = (1. - ps) * nl / PI + ps * spec_pdf;
    let diff = (Vec3::ONE - f) * (1. - m.lobes[0]) * rgb(m.color_rough) / PI;
    let mut r =
        diff + f * d * gv * gl / (4. * nv * nl).max(1e-6) * (Vec3::ONE + f0 * (1. / ess - 1.));
    let mut pdf = pdf;
    let sheen = rgb(m.sheen);
    if sheen.max_element() > 0. {
        let sr = m.layers[2];
        let below = 1. - sheen.max_element() * sheen_albedo(nv, sr).min(1.);
        r = r * below + sheen * sheen_lobe(sr, nh, nv, nl);
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
        r = r * (1. - coat * coat_albedo(nv, cr)) + Vec3::splat(coat * lobe);
        let pc = coat_probability(m);
        pdf = pdf * (1. - pc) + pc * dc * nh / (4. * vh).max(1e-6);
    }
    let keep = 1. - m.lobes[2];
    (keep * r, keep * pdf)
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
/// Emitted radiance leaving a surface toward a viewer at cosine `c` on side `front`.
pub(super) fn emitted(m: &PackedMaterial, front: bool, c: f32) -> Vec3 {
    if m.optical[1] <= 0. || (m.optical[3] > 0.5 && !front) {
        return Vec3::ZERO;
    }
    rgb(m.color_rough) * m.optical[1] * spot(m, c)
}
