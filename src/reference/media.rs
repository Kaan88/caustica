//! The nested-dielectric stack and what it absorbs.
use super::bsdf::rgb;
use super::scene::Scene;
use glam::Vec3;

/// The nested-dielectric stack (`medium_top`, `medium_push`, `medium_remove`): entries are
/// (priority, material), at most 4; the current medium is the highest priority, the later
/// entry among equals.
pub(super) fn top(media: &[(u32, u32)]) -> Option<&(u32, u32)> {
    // max_by_key keeps the last of equal maxima.
    media.iter().max_by_key(|e| e.0)
}
pub(super) fn push(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    // A full stack drops the push, as on the GPU.
    if media.len() < 4 {
        media.push(e);
    }
}
pub(super) fn remove(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    if let Some(i) = media.iter().rposition(|x| *x == e) {
        media.remove(i);
    }
}

impl Scene<'_> {
    /// Beer-Lambert through `t` of the innermost dielectric: its colour is the transmittance
    /// per unit (`medium_absorption`).
    pub(super) fn transmittance(&self, media: &[(u32, u32)], t: f32) -> Vec3 {
        let Some(top) = top(media) else {
            return Vec3::ONE;
        };
        let c = rgb(self.materials[top.1 as usize].color_rough).max(Vec3::splat(0.001));
        c.powf(t)
    }
}

/// Henyey-Greenstein: the density of scattering by cos θ = c from the incoming direction.
pub(super) fn hg_pdf(c: f32, g: f32) -> f32 {
    let d = (1. + g * g - 2. * g * c).max(1e-6);
    (1. - g * g) / (4. * std::f32::consts::PI * d * d.sqrt())
}

/// A direction scattered from `d` by Henyey-Greenstein (inverting its cdf in cos θ), and its pdf.
pub(super) fn hg_sample(d: Vec3, g: f32, r: &mut super::sampling::Rng) -> (Vec3, f32) {
    let u = r.f();
    let c = if g.abs() < 1e-3 {
        1. - 2. * u
    } else {
        let s = (1. - g * g) / (1. - g + 2. * g * u);
        (1. + g * g - s * s) / (2. * g)
    }
    .clamp(-1., 1.);
    let sn = (1. - c * c).max(0.).sqrt();
    let a = 2. * std::f32::consts::PI * r.f();
    let (t, b) = d.any_orthonormal_pair();
    ((t * a.cos() + b * a.sin()) * sn + d * c, hg_pdf(c, g))
}

impl Scene<'_> {
    /// Where the fog box (from below the floor to `Fog::height`, ±`Fog::extent` on x and z)
    /// covers o + d t for t in [0, tmax]; None when the fog is off or missed.
    pub(super) fn fog_interval(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<(f32, f32)> {
        let f = &self.s.fog;
        if f.density <= 0. {
            return None;
        }
        let (mut t0, mut t1) = (0f32, tmax);
        let lo = Vec3::new(-f.extent, -1., -f.extent);
        let hi = Vec3::new(f.extent, f.height, f.extent);
        for k in 0..3 {
            if d[k].abs() < 1e-9 {
                if o[k] < lo[k] || o[k] > hi[k] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((lo[k] - o[k]) / d[k], (hi[k] - o[k]) / d[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        (t1 > t0).then_some((t0, t1))
    }
    /// The (grey, uniform) fog's transmittance along o + d t up to `limit`.
    pub(super) fn fog_transmittance(&self, o: Vec3, d: Vec3, limit: f32) -> f32 {
        self.fog_interval(o, d, limit)
            .map_or(1., |(t0, t1)| (-self.s.fog.density * (t1 - t0)).exp())
    }
}
