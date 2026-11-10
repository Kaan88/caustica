//! What the estimators share at a surface: the hit's brushing direction, the material's spectral
//! form, and sampling its lobes and its dielectric. The path tracer and BDPT both call these, so a
//! difference between them is in how they combine paths, never in how a surface scatters.
use crate::lighting::PackedMaterial;
use crate::reference::bsdf::{
    GRATING_ORDER, Wavelength, aniso_alpha, aniso_frame, coat_probability, flags,
    fresnel_dielectric, grating, grating_move, order_shares, prepare_film, rgb, spec_probability,
};
use crate::reference::bvh::Hit;
use crate::reference::media::{push, remove, top};
use crate::reference::sampling::{
    Rng, cosine_sample, ggx_sample, ggx_sample_aniso, reflect, refract,
};
use crate::reference::scene::{Scene, Surface};
use crate::reference::spectral::Waves;
use glam::Vec3;

/// A dielectric's sample (`Scene::dielectric`).
pub(in crate::reference) struct Dielectric {
    pub(in crate::reference) next: Vec3,
    pub(in crate::reference) reflected: bool,
    /// What a thin film's colour leaves of the choice's probability (1 without a film).
    pub(in crate::reference) weight: Vec3,
    /// n1 / n2 across the interface.
    pub(in crate::reference) eta: f32,
    /// The media stack past the surface, for a refraction.
    pub(in crate::reference) after: Vec<(u32, u32)>,
}

/// The colours as spectra at the path's wavelengths, and the thin film (`film`: the cosine and
/// both indices it was prepared with in RGB) exact at them over the uplifted f0. RGB: unchanged.
pub(in crate::reference) fn spectral_material(
    m: &mut PackedMaterial,
    film: Option<(f32, f32, f32)>,
    w: &Waves,
) {
    if !w.on {
        return;
    }
    let c = w.up_material(m, rgb(m.color_rough));
    m.color_rough[..3].copy_from_slice(&c.to_array());
    let sheen = w.up(rgb(m.sheen));
    m.sheen[..3].copy_from_slice(&sheen.to_array());
    if let Some((c, outside, base)) = film {
        prepare_film(m, c, outside, base, Some(w.l));
    }
}

/// A direction from the material's non-dielectric lobes for a ray arriving along `direction` at
/// normal `n`: the coat, the base specular (a grating's order moved across the grooves, or
/// anisotropic), else the diffuse cosine. Also whether a glossy lobe chose it. None when a
/// grating's order leaves past the horizon. The caller checks the hemisphere and evaluates the
/// BSDF for the density.
pub(in crate::reference) fn sample_lobes(
    m: &PackedMaterial,
    n: Vec3,
    tangent: Vec3,
    direction: Vec3,
    lambda: Wavelength,
    r: &mut Rng,
) -> Option<(Vec3, bool)> {
    let ps = spec_probability(m);
    let pc = coat_probability(m);
    if pc > 0. && r.f() < pc {
        let cr = m.layers[1];
        return Some((ggx_sample(n, direction, (cr * cr).max(0.001), r), true));
    }
    if r.f() >= ps {
        return Some((cosine_sample(n, r), false));
    }
    let rough = m.color_rough[3];
    let a = (rough * rough).max(0.001);
    // A grating's order (`order_shares`): the isotropic sample moved across the grooves.
    let mut order = None;
    if let Some((share, spacing)) = grating(m) {
        let (_, b) = aniso_frame(n, tangent, m.sheen[3]);
        let k = lambda.nm / spacing;
        let shares = order_shares(share, k, n, b, -direction);
        let u = r.f();
        let mut c = 0.;
        order = (0..4)
            .find(|&o| {
                c += shares[o];
                u < c
            })
            .map(|o| (GRATING_ORDER[o] * k, b));
    }
    let d = if let Some((shift, b)) = order {
        let d = ggx_sample(n, direction, a, r);
        grating_move(n, b, d, shift).filter(|_| n.dot(d) > 0.)?
    } else if m.layers[3] > 0. {
        let (t, b) = aniso_frame(n, tangent, m.sheen[3]);
        ggx_sample_aniso(n, t, b, direction, aniso_alpha(m), r)
    } else {
        ggx_sample(n, direction, a, r)
    };
    Some((d, true))
}

impl Scene<'_> {
    /// The tangent an anisotropic material or a grating uses at a hit, from the raw record as the
    /// GPU's hit reads it; a procedural hit's is fixed by its normal, as `intersect` gives it.
    pub(in crate::reference) fn hit_tangent(&self, sf: &Surface, h: &Hit) -> Vec3 {
        let tangent = self.tangent(sf.material, sf.p);
        if h.sdf.is_none() || tangent == Vec3::ZERO {
            return tangent;
        }
        let up = if sf.n.y.abs() < 0.99 {
            Vec3::Y
        } else {
            Vec3::X
        };
        sf.n.cross(up).normalize()
    }
    /// A sample of dielectric `m` (`material`, hit from `front` at shading normal `n` along
    /// `direction`) inside `media`: reflection by the Fresnel term (a film's mean colour when it
    /// has one) or refraction, then a rough surface's lerp toward a cosine sample. The eta² a
    /// radiance path takes and the media change are the caller's.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::reference) fn dielectric(
        &self,
        media: &[(u32, u32)],
        m: &PackedMaterial,
        material: u32,
        front: bool,
        n: Vec3,
        direction: Vec3,
        w: &Waves,
        r: &mut Rng,
    ) -> Dielectric {
        let (n1, n2, after) = self.interface(media, m, material, front, w);
        let eta = n1 / n2;
        let ci = (-direction).dot(n).clamp(0., 1.);
        let mut fr = fresnel_dielectric(ci, eta);
        // A film colours the reflection and, by what it leaves, the transmission: the choice
        // goes by the mean, the weight per channel.
        let film = m.surface_scale[2];
        let mut frc = Vec3::splat(fr);
        if film > 0. && fr < 1. {
            frc = frc.lerp(Vec3::from_slice(&m.base_scale[..3]), film);
            fr = (frc.element_sum() / 3.).clamp(1e-4, 1. - 1e-4);
        }
        let reflected = r.f() < fr;
        let weight = match (film > 0., reflected) {
            (false, _) => Vec3::ONE,
            (true, true) => frc / fr,
            (true, false) => (Vec3::ONE - frc) / (1. - fr),
        };
        let mut next = if reflected {
            reflect(direction, n)
        } else {
            refract(direction, n, eta)
        };
        let rough = m.color_rough[3];
        if rough > 0.001 {
            let pert = cosine_sample(if reflected { n } else { -n }, r);
            next = next.lerp(pert, rough * rough).normalize();
        }
        Dielectric {
            next,
            reflected,
            weight,
            eta,
            after,
        }
    }
    /// The IORs on the near and far side of dielectric `material` hit from `front`, and the
    /// media stack beyond it: entering, the current medium's and its own; leaving, its own and
    /// the one the stack falls back to (air when empty). Each at the path's hero wavelength once
    /// it keeps that alone (`Waves::ior`).
    pub(in crate::reference) fn interface(
        &self,
        media: &[(u32, u32)],
        m: &PackedMaterial,
        material: u32,
        front: bool,
        w: &Waves,
    ) -> (f32, f32, Vec<(u32, u32)>) {
        let entry = ((flags(m) >> 8) & 7, material);
        let ior = |e: Option<&(u32, u32)>| e.map_or(1., |e| w.ior(&self.materials[e.1 as usize]));
        let mut after = media.to_vec();
        if front {
            push(&mut after, entry);
            (ior(top(media)), w.ior(m), after)
        } else {
            remove(&mut after, entry);
            let n2 = ior(top(&after));
            (w.ior(m), n2, after)
        }
    }
}
