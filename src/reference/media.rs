//! The nested-dielectric stack and what it absorbs.
use super::bsdf::rgb;
use super::scene::Scene;
use super::spectral::Waves;
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
    pub(super) fn transmittance(&self, media: &[(u32, u32)], t: f32, w: &Waves) -> Vec3 {
        let Some(top) = top(media) else {
            return Vec3::ONE;
        };
        let m = &self.materials[top.1 as usize];
        let c = w.up_material(m, rgb(m.color_rough)).max(Vec3::splat(0.001));
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
    /// The uniform fog's extinction per channel: the density times its tint, read as the GPU
    /// reads it (`lighting::pack_tint`: 8 bits per channel, relative to the largest).
    pub(super) fn fog_sigma(&self, w: &Waves) -> Vec3 {
        w.sigma(
            self.s.fog.density * w.up(tint_of(crate::lighting::pack_tint(self.s.fog.extinction))),
        )
    }
    /// The fog's transmittance along o + d t over [start, limit], per channel: exact when
    /// uniform, an unbiased ratio-tracking estimate when `Fog::noise` makes it heterogeneous.
    pub(super) fn fog_transmittance(
        &self,
        o: Vec3,
        d: Vec3,
        start: f32,
        limit: f32,
        w: &Waves,
        r: &mut super::sampling::Rng,
    ) -> Vec3 {
        let Some((t0, t1)) = self.fog_interval(o, d, limit) else {
            return Vec3::ONE;
        };
        let t0 = t0.max(start);
        if t1 <= t0 {
            return Vec3::ONE;
        }
        let f = &self.s.fog;
        if f.noise <= 0. {
            return (-self.fog_sigma(w) * (t1 - t0)).exp();
        }
        ratio_tracking(self.fog_sigma(w), f.noise, f.noise_size, o, d, t0, t1, r)
    }
    /// The scattering medium of the innermost dielectric when it has one (`Material::density`):
    /// its extinction, scattering albedo and Henyey-Greenstein g.
    pub(super) fn object_medium(
        &self,
        media: &[(u32, u32)],
        w: &Waves,
    ) -> Option<(Vec3, Vec3, f32)> {
        let m = &self.materials[top(media)?.1 as usize];
        (m.medium[3] > 0.).then(|| (extinction(m, w), w.up(rgb(m.medium)), m.phase[0]))
    }
    /// What a shadow ray from o toward a light `limit` away lets through of the media. Inside a
    /// scattering object: its extinction and its colour's absorption up to the nearest surface,
    /// refraction there ignored as glass shadows ignore it, then the fog past that point (glass
    /// crossed on the way not subtracted from it). Inside clear glass: nothing is lost. Outside
    /// every object: the fog.
    pub(super) fn shadow_transmittance(
        &self,
        media: &[(u32, u32)],
        o: Vec3,
        d: Vec3,
        limit: f32,
        w: &Waves,
        r: &mut super::sampling::Rng,
    ) -> Vec3 {
        let (mut tr, mut start) = (Vec3::ONE, 0.);
        if !media.is_empty() {
            let Some((sigma, _, _)) = self.object_medium(media, w) else {
                return Vec3::ONE;
            };
            start = self
                .intersect(
                    o,
                    d,
                    limit,
                    &self.accept(super::scene::Ray::Bounce, o, d),
                    false,
                )
                .map_or(limit, |h| h.t);
            tr = (-sigma * start).exp() * self.transmittance(media, start, w);
        }
        tr * self.fog_transmittance(o, d, start, limit, w, r)
    }
    /// A free flight through the fog over [t0, t1): `flight` when uniform, delta tracking
    /// otherwise.
    pub(super) fn fog_flight(
        &self,
        o: Vec3,
        d: Vec3,
        t0: f32,
        t1: f32,
        w: &Waves,
        r: &mut super::sampling::Rng,
    ) -> (f32, Vec3) {
        let f = &self.s.fog;
        if f.noise <= 0. {
            return flight(self.fog_sigma(w), t0, t1, r);
        }
        delta_tracking(self.fog_sigma(w), f.noise, f.noise_size, o, d, t0, t1, r)
    }
}

/// The integer hash the noise lattice is built on (the GPU's `hash`, a lowbias32 variant).
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^ x >> 16
}

/// The value at an integer lattice point, in [0, 1).
fn lattice(x: i32, y: i32, z: i32) -> f32 {
    let k = (x as u32).wrapping_mul(73856093)
        ^ (y as u32).wrapping_mul(19349663)
        ^ (z as u32).wrapping_mul(83492791);
    hash(k) as f32 * (1. / 4294967296.)
}

/// Value noise: the lattice values blended trilinearly with smoothstep weights.
fn value_noise(p: Vec3) -> f32 {
    let c = p.floor();
    let u = p - c;
    let u = u * u * (3. - 2. * u);
    let (x, y, z) = (c.x as i32, c.y as i32, c.z as i32);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    // Along x on each of the four edges, then y, then z.
    let edge = |j: i32, k: i32| mix(lattice(x, y + j, z + k), lattice(x + 1, y + j, z + k), u.x);
    let near = mix(edge(0, 0), edge(1, 0), u.y);
    let far = mix(edge(0, 1), edge(1, 1), u.y);
    mix(near, far, u.z)
}

/// The fog's relative density at p (1 on average): four octaves of value noise, normalised to
/// [0, 1], mapped to 1 + noise·(2 fBm − 1) and clamped at 0.
pub(super) fn fog_density(p: Vec3, noise: f32, size: f32) -> f32 {
    let mut q = p / size;
    let (mut sum, mut weight, mut amp) = (0., 0., 0.5);
    for _ in 0..4 {
        sum += amp * value_noise(q);
        weight += amp;
        q = q * 2.03 + Vec3::new(17.1, -3.7, 9.3);
        amp *= 0.5;
    }
    (1. + noise * (2. * sum / weight - 1.)).max(0.)
}

/// The majorant: the density never exceeds 1 + noise, so this bounds every channel.
fn majorant(sigma: Vec3, noise: f32) -> f32 {
    sigma.max_element() * (1. + noise)
}

/// Delta (spectral) tracking over [t0, t1) of a medium of extinction sigma·density: tentative
/// collisions at the majorant's rate, each real with the probability the path's weighted
/// share of the extinction gives, the weight correcting for the channels not sampled by it
/// (1 when grey). Returns t1 when it gets through.
#[allow(clippy::too_many_arguments)]
pub(super) fn delta_tracking(
    sigma: Vec3,
    noise: f32,
    size: f32,
    o: Vec3,
    d: Vec3,
    t0: f32,
    t1: f32,
    r: &mut super::sampling::Rng,
) -> (f32, Vec3) {
    let mu = majorant(sigma, noise);
    let mut w = Vec3::ONE;
    if mu <= 0. {
        return (t1, w);
    }
    let mut t = t0;
    for _ in 0..1024 {
        t -= (1. - r.f()).ln() / mu;
        if t >= t1 {
            return (t, w);
        }
        let real = sigma * fog_density(o + d * t, noise, size);
        let null = Vec3::splat(mu) - real;
        let (a, b) = (w.dot(real), w.dot(null));
        let p = a / (a + b).max(1e-30);
        if r.f() < p {
            return (t, w * real / (mu * p));
        }
        w *= null / (mu * (1. - p));
    }
    (t1, w)
}

/// Ratio tracking over [t0, t1]: the product of the null-collision probabilities at tentative
/// collisions drawn at the majorant's rate; 0 once every channel is below 1e-4.
#[allow(clippy::too_many_arguments)]
pub(super) fn ratio_tracking(
    sigma: Vec3,
    noise: f32,
    size: f32,
    o: Vec3,
    d: Vec3,
    t0: f32,
    t1: f32,
    r: &mut super::sampling::Rng,
) -> Vec3 {
    let mu = majorant(sigma, noise);
    let mut tr = Vec3::ONE;
    if mu <= 0. {
        return tr;
    }
    let mut t = t0;
    for _ in 0..1024 {
        t -= (1. - r.f()).ln() / mu;
        if t >= t1 {
            break;
        }
        tr *= Vec3::ONE - sigma * fog_density(o + d * t, noise, size) / mu;
        if tr.max_element() < 1e-4 {
            return Vec3::ZERO;
        }
    }
    tr
}

/// A material's extinction per channel (`extinction_of`): its density times its packed tint,
/// where phase.w holds one (not on blackbodies and the measured light, whose phase.w is a scale),
/// uplifted at the path's wavelengths (`Waves::sigma`).
pub(super) fn extinction(m: &crate::lighting::PackedMaterial, w: &Waves) -> Vec3 {
    let spectrum = m.phase[2];
    w.sigma(
        m.medium[3]
            * if spectrum < 0. || spectrum == 4. {
                Vec3::ONE
            } else {
                w.up(tint_of(m.phase[3]))
            },
    )
}

/// A packed extinction tint (`lighting::pack_tint`) as a colour; 0 is grey.
pub(super) fn tint_of(t: f32) -> Vec3 {
    let p = t as u32;
    if p == 0 {
        return Vec3::ONE;
    }
    Vec3::new((p & 255) as f32, (p >> 8 & 255) as f32, (p >> 16) as f32) / 255.
}

/// A free-flight distance from t0 in a medium of extinction `sigma`, and the weight w the path
/// takes on with it. A grey medium samples its transmittance exactly (w = 1). A chromatic one
/// picks a channel uniformly to sample by and weights with the one-sample MIS over the three
/// channels' densities: w = T·σ / mean(T·σ) for a scatter (ts < t1), T / mean(T) past t1.
pub(super) fn flight(sigma: Vec3, t0: f32, t1: f32, r: &mut super::sampling::Rng) -> (f32, Vec3) {
    if sigma.x == sigma.y && sigma.x == sigma.z {
        return (t0 - (1. - r.f()).ln() / sigma.x, Vec3::ONE);
    }
    let s = sigma[((r.f() * 3.) as usize).min(2)];
    let ts = t0 - (1. - r.f()).ln() / s.max(1e-12);
    let tr = (-sigma * (ts.min(t1) - t0)).exp();
    let f = if ts < t1 { tr * sigma } else { tr };
    (ts, f / (f.element_sum() / 3.).max(1e-30))
}
