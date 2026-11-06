//! The material model, as pathtrace.hlsl evaluates it.
use crate::lighting::PackedMaterial;
use glam::{Vec2, Vec3};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------------------------
// The GGX directional albedo (layers.hlsli GGX_ALBEDO, 32 × 32 over cos θ and roughness).

pub(super) fn ggx_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let v = crate::render::hlsl_table(include_str!("../../shaders/layers.hlsli"), "GGX_ALBEDO");
        assert_eq!(v.len(), 1024, "GGX_ALBEDO has 1024 entries");
        v
    })
}
pub(super) fn ggx_albedo(nv: f32, roughness: f32) -> f32 {
    let t = ggx_table();
    let x = Vec2::new(nv.clamp(0., 1.), roughness.clamp(0., 1.)) * 31.;
    let (ix, iy) = ((x.x as usize).min(30), (x.y as usize).min(30));
    let (fx, fy) = (x.x - ix as f32, x.y - iy as f32);
    let s = |c: usize, r: usize| t[(iy + r) * 32 + ix + c];
    let lerp = |a: f32, b: f32, f: f32| a + (b - a) * f;
    lerp(lerp(s(0, 0), s(1, 0), fx), lerp(s(0, 1), s(1, 1), fx), fy)
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
/// `eval_bsdf` without layers, anisotropy, gratings or films: the value and the pdf of the
/// sampling below.
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
    let r = diff + f * d * gv * gl / (4. * nv * nl).max(1e-6) * (Vec3::ONE + f0 * (1. / ess - 1.));
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
/// The spot falloff (`spot`, flag 32): smoothstep between the cone's cosines.
pub(super) fn spot(m: &PackedMaterial, c: f32) -> f32 {
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
