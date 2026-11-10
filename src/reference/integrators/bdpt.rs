//! Bidirectional path tracing (`Settings::integrator` 1): Veach's estimator in PBRT-v3's form, the
//! GPU's model (`integrators.hlsl`) built from the path tracer's surface code (`shading`). Each
//! sample traces a camera subpath and a light subpath of up to `bounces` (at most 24) bounces and
//! joins them in every way (s light vertices, t camera vertices), each strategy weighted by the
//! balance heuristic over all of them. Light subpaths start on the table's triangles and spheres;
//! the sky, the environment map and the suns start none and keep the path tracer's two strategies
//! (hits and next-event estimation) with its MIS. Light tracing (t = 1) lands in other pixels:
//! those terms come back as splats, which `Progressive::pass` adds after the pass in row order.
//!
//! As on the GPU: clear glass blocks every connection and shadow ray (`Scene::glass_blocks`),
//! portals, the shadow catcher, ground projection, ReSTIR and `mis_mode` are the path tracer's
//! alone, there is no Russian roulette, and with a spectral scene a sample carries its hero
//! wavelength from the start. One difference, deliberate: a vertex can be connected whenever its
//! material has a non-delta part. The GPU skips a vertex that sampled its dielectric lobe, so on a
//! partly transmissive material (0 < transmission < 1) the strategies that end there count only
//! on the samples that took the other lobes, while the MIS weights assume them always: the GPU's
//! BDPT is dark there by that share (engine-notes "CPU reference renderer").
use crate::lighting::PackedMaterial;
use crate::reference::bsdf::{
    Wavelength, emitted, eval_bsdf, flags, grating, grating_wavelength, material_at, prepare_film,
    rgb,
};
use crate::reference::bvh::Hit;
use crate::reference::camera::filter_density;
use crate::reference::integrators::shading::{sample_lobes, spectral_material};
use crate::reference::lights::{cone_sample, shadow_limit};
use crate::reference::media::{
    extinction, flight, fog_density, hg_pdf, hg_sample, push, remove, top,
};
use crate::reference::sampling::{Rng, basis, cosine_sample, reflect};
use crate::reference::scene::{Light, Ray, Scene, Surface, mix};
use crate::reference::spectral::Waves;
use glam::{Vec2, Vec3, Vec4};
use std::f32::consts::PI;

/// The most bounces a BDPT sample takes (`BDPT_BOUNCES`).
pub(in crate::reference) const BOUNCES: u32 = 24;

/// A light-tracing term: the render pixel (row-major index) and what it adds to that pixel's sum.
pub(in crate::reference) type Splat = (u32, Vec3);

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    Camera,
    Light,
    Surface,
    Medium,
}

/// A point on a table emitter: its entry, where, its own normal (a triangle's face normal turned
/// to its shading side, a sphere's outward one), the material there and the emission colour.
#[derive(Clone, Copy)]
pub(super) struct LightPoint {
    pub(super) k: usize,
    pub(super) p: Vec3,
    pub(super) n: Vec3,
    pub(super) sphere: bool,
    pub(super) m: PackedMaterial,
    pub(super) colour: Vec3,
    pub(super) area: f32,
}

/// A subpath vertex. `beta`: the subpath's throughput arriving there; `pdf_fwd` / `pdf_rev`: the
/// area densities of reaching it from its own subpath's side / from the other side.
#[derive(Clone, Copy)]
pub(super) struct Vertex {
    pub(super) kind: Kind,
    pub(super) p: Vec3,
    /// The shading normal facing the arrival (a light's: the side it emitted from); zero for the
    /// camera and in a medium.
    pub(super) n: Vec3,
    /// The face normal facing the arrival: what offsets leave along.
    pub(super) ng: Vec3,
    pub(super) front: bool,
    /// It sampled a delta event (a dielectric, a beam splitter) to continue its subpath.
    pub(super) delta: bool,
    pub(super) beta: Vec3,
    pub(super) pdf_fwd: f32,
    pub(super) pdf_rev: f32,
    pub(super) material: u32,
    /// The material after its pattern and maps, before the film and the uplift (`material`).
    pub(super) m: PackedMaterial,
    pub(super) tangent: Vec3,
    pub(super) lambda: Wavelength,
    /// The light-table entry + 1 the surface belongs to (0: none).
    pub(super) emitter: u32,
    /// What the surface emits toward where its subpath came from.
    pub(super) le: Vec3,
    /// The innermost dielectric on the side it was reached from (None: air, so the fog box).
    pub(super) medium: Option<(u32, u32)>,
    /// A medium vertex's Henyey-Greenstein g.
    pub(super) g: f32,
    pub(super) light: Option<LightPoint>,
}
impl Vertex {
    fn blank(kind: Kind, p: Vec3, beta: Vec3, pdf_fwd: f32) -> Self {
        Vertex {
            kind,
            p,
            n: Vec3::ZERO,
            ng: Vec3::ZERO,
            front: true,
            delta: false,
            beta,
            pdf_fwd,
            pdf_rev: 0.,
            material: 0,
            m: bytemuck::Zeroable::zeroed(),
            tangent: Vec3::ZERO,
            lambda: Wavelength::NONE,
            emitter: 0,
            le: Vec3::ZERO,
            medium: None,
            g: 0.,
            light: None,
        }
    }
    /// Whether a strategy can end its subpath here: a scatter point, or a surface whose material
    /// has a part a connection can evaluate (not unlit, not all glass, not a beam splitter).
    fn connectible(&self) -> bool {
        match self.kind {
            Kind::Medium | Kind::Light => true,
            Kind::Camera => false,
            Kind::Surface => flags(&self.m) & (1 | 4096) == 0 && self.m.lobes[2] < 1.,
        }
    }
    /// The cosine a density or a geometry term takes here toward `w` (1 in a medium).
    fn cos(&self, w: Vec3) -> f32 {
        if self.kind == Kind::Medium {
            1.
        } else {
            self.n.dot(w).abs()
        }
    }
}

/// A solid-angle density at `from` as an area density at `at` (normal `n`; none in a medium).
fn area_pdf(pdf: f32, from: Vec3, at: Vec3, n: Option<Vec3>) -> f32 {
    let w = at - from;
    let d2 = w.length_squared().max(1e-12);
    pdf * n.map_or(1., |n| n.dot(w).abs() / d2.sqrt()) / d2
}
fn to_area(pdf: f32, from: Vec3, to: &Vertex) -> f32 {
    area_pdf(pdf, from, to.p, (to.kind != Kind::Medium).then_some(to.n))
}
fn remap0(x: f32) -> f32 {
    if x != 0. { x } else { 1. }
}
/// Adds one strategy's term: a non-finite one is dropped on its own, and each channel is kept
/// non-negative.
fn add_term(l: &mut Vec3, c: Vec3) {
    if c.is_finite() {
        *l += c.max(Vec3::ZERO);
    }
}

/// The densities one vertex contributes to `mis_weight`.
#[derive(Clone, Copy, Default, Debug)]
struct Pdfs {
    fwd: f32,
    rev: f32,
    delta: bool,
}
/// The balance-heuristic weight of strategy (s, t) from the ratios of each vertex's densities:
/// `cam` and `lig` hold the subpaths' first t and s vertices, and the connection's endpoints and
/// their neighbours take the reverse densities passed in (`qs_fwd1`: the forward density of a
/// light vertex sampled for s = 1). A ratio counts only where neither the vertex nor its
/// predecessor is delta; the connection's own endpoints never are.
#[allow(clippy::too_many_arguments)]
fn mis_weight(
    cam: &[Pdfs],
    lig: &[Pdfs],
    s: usize,
    t: usize,
    pt_rev: f32,
    ptm_rev: f32,
    qs_rev: f32,
    qsm_rev: f32,
    qs_fwd1: f32,
) -> f32 {
    if s + t == 2 {
        return 1.;
    }
    let mut sum = 0.;
    let mut ri = 1.;
    for i in (1..t).rev() {
        let rev = match t - 1 - i {
            0 => pt_rev,
            1 => ptm_rev,
            _ => cam[i].rev,
        };
        ri *= remap0(rev) / remap0(cam[i].fwd);
        if !(i != t - 1 && cam[i].delta) && !cam[i - 1].delta {
            sum += ri;
        }
    }
    ri = 1.;
    for j in (0..s).rev() {
        let rev = match s - 1 - j {
            0 => qs_rev,
            1 => qsm_rev,
            _ => lig[j].rev,
        };
        let fwd = if s == 1 && j == 0 {
            qs_fwd1
        } else {
            lig[j].fwd
        };
        ri *= remap0(rev) / remap0(fwd);
        if !(j != s - 1 && lig[j].delta) && !(j > 0 && lig[j - 1].delta) {
            sum += ri;
        }
    }
    if sum.is_finite() { 1. / (1. + sum) } else { 0. }
}

/// The share of the pixel filter's support (5 pixels at most, clipped to [r0, r1]) one splat at
/// film coordinate `f` picks, with probability w / W by `u`: the pixel and W (`pick_filtered`).
fn pick_filtered(s: &crate::scene::Settings, f: f32, r0: i32, r1: i32, u: f32) -> (i32, f32) {
    let h = 0.5 * s.filter_width;
    let lo = ((f - 0.5 - h).ceil() as i32).max(r0);
    let hi = ((f - 0.5 + h).floor() as i32).min(r1).min(lo + 4);
    let weights: Vec<f32> = (lo..=hi)
        .map(|x| filter_density(s, f - (x as f32 + 0.5)))
        .collect();
    let total: f32 = weights.iter().sum();
    let mut u = u * total;
    let mut at = lo;
    for (x, w) in (lo..=hi).zip(&weights) {
        at = x;
        if *w > 0. && u < *w {
            break;
        }
        u -= w;
    }
    (at, total)
}

/// A walk's next event (`Scene::walk_step`).
pub(super) enum Step {
    Escape,
    Surface(Hit, Surface),
    /// A scatter at a point, with the medium's albedo, g and extinction there (what photon
    /// mapping's volume gather divides by).
    Scatter(Vec3, Vec3, f32, Vec3),
}

/// A vertex's sampled continuation.
pub(super) struct Sampled {
    pub(super) next: Vec3,
    pub(super) pf: f32,
    pub(super) pr: f32,
    pub(super) delta: bool,
    /// The media past a refraction.
    pub(super) after: Option<Vec<(u32, u32)>>,
}

impl Scene<'_> {
    /// The share of next-event samples and light-path origins the environment takes: half beside
    /// the table, all without one (procedural emitters take none here).
    pub(super) fn bdpt_env_share(&self) -> f32 {
        match (&self.env, self.lights.is_empty()) {
            (None, _) => 0.,
            (Some(_), true) => 1.,
            (Some(_), false) => 0.5,
        }
    }
    /// The power CDF's probability of entry `k`.
    pub(super) fn entry_probability(&self, k: usize) -> f32 {
        self.cdf[k] - if k > 0 { self.cdf[k - 1] } else { 0. }
    }
    /// The area density of light-path origin `l`.
    fn origin_pdf(&self, l: &LightPoint, pe: f32) -> f32 {
        (1. - pe) * self.entry_probability(l.k) / l.area.max(1e-12)
    }
    /// A uniform point on table entry `k` (None for a sun).
    pub(super) fn light_point(&self, k: usize, r: &mut Rng) -> Option<LightPoint> {
        match self.lights[k] {
            Light::Sun { .. } => None,
            Light::Sphere {
                c,
                r: radius,
                le,
                material,
            } => {
                let z = 1. - 2. * r.f();
                let a = 2. * PI * r.f();
                let s = (1. - z * z).max(0.).sqrt();
                let n = Vec3::new(s * a.cos(), s * a.sin(), z);
                Some(LightPoint {
                    k,
                    p: c + radius * n,
                    n,
                    sphere: true,
                    m: self.materials[material as usize],
                    colour: le,
                    area: 4. * PI * radius * radius,
                })
            }
            Light::Tri(tri) => {
                let t = &self.bvh.tris[tri];
                let (mut a, mut b) = (r.f(), r.f());
                if a + b > 1. {
                    (a, b) = (1. - a, 1. - b);
                }
                let p = t.p[0] * (1. - a - b) + t.p[1] * a + t.p[2] * b;
                Some(self.tri_point(k, tri, p, Vec2::new(a, b)))
            }
        }
    }
    fn tri_point(&self, k: usize, tri: usize, p: Vec3, b: Vec2) -> LightPoint {
        let t = &self.bvh.tris[tri];
        let m = material_at(&self.materials[t.material as usize], p);
        LightPoint {
            k,
            p,
            n: t.normals(b.x, b.y).1,
            sphere: false,
            m,
            colour: self.emission_colour(&m, tri, b),
            area: Self::area(t),
        }
    }
    /// The light point a camera vertex on table entry `x.emitter` is (`hit_light`).
    fn hit_light(&self, x: &Vertex, hit: &Hit) -> LightPoint {
        let k = x.emitter as usize - 1;
        match self.lights[k] {
            Light::Sphere {
                c,
                r: radius,
                le,
                material,
            } => LightPoint {
                k,
                p: x.p,
                n: (x.p - c).normalize_or_zero(),
                sphere: true,
                m: self.materials[material as usize],
                colour: le,
                area: 4. * PI * radius * radius,
            },
            _ => self.tri_point(k, hit.tri, x.p, hit.b),
        }
    }
    pub(super) fn two_sided(l: &LightPoint) -> bool {
        !l.sphere && l.m.optical[3] < 0.5
    }
    /// A one-sided spot's cone cosine, out of which it emits nothing (0: no cone).
    pub(super) fn spot_cone(l: &LightPoint) -> f32 {
        if !l.sphere && flags(&l.m) & 32 != 0 && l.m.optical[3] >= 0.5 {
            l.m.secondary[0].clamp(-1., 0.9999)
        } else {
            0.
        }
    }
    /// The density of a light path leaving `l` along `w`.
    fn light_dir_pdf(l: &LightPoint, w: Vec3) -> f32 {
        let c = l.n.dot(w);
        let k = Self::spot_cone(l);
        if k != 0. {
            return if c >= k {
                1. / (2. * PI * (1. - k))
            } else {
                0.
            };
        }
        if Self::two_sided(l) {
            c.abs() / (2. * PI)
        } else {
            c.max(0.) / PI
        }
    }
    /// The radiance `l` sends along `w`, at the path's wavelengths.
    pub(super) fn emission_toward(&self, l: &LightPoint, w: Vec3, waves: &Waves) -> Vec3 {
        let c = l.n.dot(w);
        let raw = &self.materials[match self.lights[l.k] {
            Light::Tri(tri) => self.bvh.tris[tri].material,
            Light::Sphere { material, .. } | Light::Sun { material, .. } => material,
        } as usize];
        if l.sphere {
            return if c > 0. {
                waves.up_emitted(raw, l.colour)
            } else {
                Vec3::ZERO
            };
        }
        waves.up_emitted(raw, emitted(&l.m, l.colour, c > 0., c))
    }
    /// The material at `x` as seen from `v`: the film at that cosine (the indices of a single
    /// dielectric against air, as the GPU's vertices take them), then the spectral uplift.
    pub(super) fn vertex_material(&self, x: &Vertex, v: Vec3, w: &Waves) -> PackedMaterial {
        let mut m = x.m;
        let film = (m.surface_scale[2] > 0.).then(|| {
            let c = v.dot(x.n).clamp(0., 1.);
            let (outside, base) = match (m.lobes[2] > 0., x.front) {
                (false, _) => (1., 0.),
                (true, true) => (1., w.ior(&m)),
                (true, false) => (w.ior(&m), 1.),
            };
            prepare_film(&mut m, c, outside, base, None);
            (c, outside, base)
        });
        spectral_material(&mut m, film, w);
        m
    }
    /// f(v, l) at `x`, v toward the camera's side and l toward the light's, and the density of
    /// sampling l arriving from v; 0 below the surface.
    fn vertex_f(&self, x: &Vertex, v: Vec3, l: Vec3, w: &Waves) -> (Vec3, f32) {
        if x.kind == Kind::Medium {
            let p = hg_pdf((-v).dot(l), x.g);
            return (Vec3::splat(p), p);
        }
        if x.kind != Kind::Surface || !x.connectible() || x.n.dot(v) <= 0. || x.n.dot(l) <= 0. {
            return (Vec3::ZERO, 0.);
        }
        eval_bsdf(
            &self.vertex_material(x, v, w),
            x.n,
            x.tangent,
            v,
            l,
            x.lambda,
        )
    }
    fn vertex_pdf(&self, x: &Vertex, v: Vec3, l: Vec3, w: &Waves) -> f32 {
        self.vertex_f(x, v, l, w).1
    }
    /// The vertex a walk arriving along `d` makes at hit `h`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn surface_vertex(
        &self,
        h: &Hit,
        sf: &Surface,
        d: Vec3,
        beta: Vec3,
        pdf_fwd: f32,
        medium: Option<(u32, u32)>,
        w: &Waves,
    ) -> Vertex {
        let mut m = material_at(&self.materials[sf.material as usize], sf.p);
        // Emission before the maps, as the path tracer takes it.
        let le = emitted(
            &m,
            w.up_emission(&m, self.emission_colour(&m, h.tri, h.b)),
            sf.front,
            sf.emission_cos(d),
        );
        let mut n = sf.n;
        self.apply_maps(&mut m, h, &mut n);
        // A grating's wavelength: the hero's, or in RGB one fixed by the point, so connections
        // and the sample agree.
        let lambda = match grating(&m) {
            None => Wavelength::NONE,
            Some(_) if w.on => Wavelength {
                nm: w.l.x,
                weight: Vec3::ONE,
            },
            Some(_) => {
                let key = mix(sf.p.x.to_bits() ^ mix(sf.p.y.to_bits() ^ mix(sf.p.z.to_bits())));
                Wavelength::rgb(grating_wavelength(&mut Rng::new(key as u64, 0x6772_6174)))
            }
        };
        Vertex {
            n,
            ng: sf.ng,
            front: sf.front,
            material: sf.material,
            m,
            tangent: self.hit_tangent(sf, h),
            lambda,
            emitter: match h.sdf {
                Some(_) => 0,
                None => self.tri_light.get(h.tri).copied().unwrap_or(0),
            },
            le,
            medium,
            ..Vertex::blank(Kind::Surface, sf.p, beta, pdf_fwd)
        }
    }
    /// A false hit on nested dielectrics (`medium_top`): entering one ranked below the current
    /// medium, or leaving one that isn't it. Updates the stack.
    fn false_hit(&self, h: &Hit, sf: &Surface, media: &mut Vec<(u32, u32)>) -> bool {
        let m = &self.materials[h.material as usize];
        if m.lobes[2] <= 0. {
            return false;
        }
        let entry = ((flags(m) >> 8) & 7, h.material);
        let skip = if sf.front {
            top(media).is_some_and(|t| entry.0 < t.0)
        } else {
            media.contains(&entry) && top(media) != Some(&entry)
        };
        if skip {
            if sf.front {
                push(media, entry);
            } else {
                remove(media, entry);
            }
        }
        skip
    }
    /// A walk's next segment from `o` along `d` (`walk_step`): a free flight in the medium it is
    /// in (the innermost object's, else the fog box), false hits passed straight through (at most
    /// 32), absorption applied to `beta`. `o` ends at the last segment's start.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk_step(
        &self,
        o: &mut Vec3,
        d: Vec3,
        kind: Ray,
        media: &mut Vec<(u32, u32)>,
        beta: &mut Vec3,
        w: &Waves,
        r: &mut Rng,
    ) -> Step {
        for f in 0.. {
            let hit = self.intersect(*o, d, 1e4, &self.accept(kind, *o, d), false);
            let tmax = hit.as_ref().map_or(1e4, |h| h.t);
            let medium = if media.is_empty() {
                self.fog_interval(*o, d, tmax).map(|(t0, t1)| {
                    let (ts, fw) = self.fog_flight(*o, d, t0, t1, w, r);
                    (
                        ts,
                        t1,
                        fw,
                        w.up(Vec3::from(self.s.fog.color)),
                        self.s.fog.phase,
                        self.fog_sigma(w),
                    )
                })
            } else {
                self.object_medium(media, w).map(|(sigma, albedo, g)| {
                    let (ts, fw) = flight(sigma, 0., tmax, r);
                    (ts, tmax, fw, albedo, g, sigma)
                })
            };
            if let Some((ts, t1, fw, albedo, g, sigma)) = medium {
                *beta *= fw;
                if ts < t1 {
                    *beta *= self.transmittance(media, ts, w);
                    let p = *o + d * ts;
                    // Noisy fog's extinction where it scattered (none of the walk's draws).
                    let f = &self.s.fog;
                    let sigma = if media.is_empty() && f.noise > 0. {
                        sigma * fog_density(p, f.noise, f.noise_size)
                    } else {
                        sigma
                    };
                    return Step::Scatter(p, albedo, g, sigma);
                }
            }
            let Some(h) = hit else {
                return Step::Escape;
            };
            *beta *= self.transmittance(media, h.t, w);
            let sf = self.surface(&h, *o, d);
            if f >= 32 || !self.false_hit(&h, &sf, media) {
                return Step::Surface(h, sf);
            }
            *o = sf.p - sf.ng * 0.0003;
        }
        unreachable!()
    }
    /// Samples how a walk arriving along `d` goes on from `x`, as the path tracer samples it.
    /// Camera subpaths (`radiance`) carry a refraction's eta², light subpaths don't. `pf`: the
    /// density of `next` given −d; `pr`: of −d given `next` (both 0 for a delta event).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sample_vertex(
        &self,
        x: &Vertex,
        d: Vec3,
        radiance: bool,
        media: &[(u32, u32)],
        beta: &mut Vec3,
        w: &Waves,
        r: &mut Rng,
    ) -> Option<Sampled> {
        let sampled = |next, pf, pr, delta, after| {
            Some(Sampled {
                next,
                pf,
                pr,
                delta,
                after,
            })
        };
        if x.kind == Kind::Medium {
            let (next, p) = hg_sample(d, x.g, r);
            return sampled(next, p, p, false, None);
        }
        if flags(&x.m) & 1 != 0 {
            return None;
        }
        let m = self.vertex_material(x, -d, w);
        // A beam splitter: a front face tints and reflects with its reflectivity, else the walk
        // passes straight on.
        if flags(&m) & 4096 != 0 {
            let mut next = d;
            if x.front {
                *beta *= rgb(m.color_rough);
                if r.f() < m.lobes[1] {
                    next = reflect(d, x.n);
                }
            }
            return sampled(next, 0., 0., true, None);
        }
        if r.f() < m.lobes[2] {
            let s = self.dielectric(media, &m, x.material, x.front, x.n, d, w, r);
            *beta *= s.weight;
            if !s.reflected && radiance {
                *beta *= s.eta * s.eta;
            }
            let after = (!s.reflected).then_some(s.after);
            return sampled(s.next, 0., 0., true, after);
        }
        let (next, _) = sample_lobes(&m, x.n, x.tangent, d, x.lambda, r)?;
        if x.n.dot(next) <= 0. {
            return None;
        }
        let (f, pf, pr) = if radiance {
            let (f, pf) = eval_bsdf(&m, x.n, x.tangent, -d, next, x.lambda);
            (f, pf, eval_bsdf(&m, x.n, x.tangent, next, -d, x.lambda).1)
        } else {
            let (f, pr) = eval_bsdf(&m, x.n, x.tangent, next, -d, x.lambda);
            (f, eval_bsdf(&m, x.n, x.tangent, -d, next, x.lambda).1, pr)
        };
        if pf <= 0. {
            return None;
        }
        *beta *= f * x.n.dot(next) / pf;
        sampled(next, pf, pr, false, None)
    }
    /// What a ray leaving along `d` finds at infinity (`escape`): the sky or the environment (the
    /// camera's first segment may see the background instead) and the suns, the environment and
    /// the suns weighted against next-event estimation as the path tracer weights them.
    pub(super) fn escape(
        &self,
        d: Vec3,
        camera: bool,
        prev_delta: bool,
        prev_pdf: f32,
        w: &Waves,
    ) -> Vec3 {
        let pe = self.bdpt_env_share();
        let mut e = match &self.env {
            None => w.up(self.plain_sky(d)),
            Some(env) => {
                let seen = if camera && !env.visible {
                    Vec3::from(self.s.background)
                } else {
                    env.radiance(d)
                };
                let mut e = w.up(seen);
                if !camera && !prev_delta {
                    e *= prev_pdf / (prev_pdf + pe * env.pdf(d));
                }
                e
            }
        };
        // `suns` passes the path tracer's density of the sun's direction; BDPT picks the table
        // with probability 1 − pe and procedural emitters never.
        let table = (1. - self.p_env) * (1. - self.sdf_share);
        let scale = if table > 0. { (1. - pe) / table } else { 0. };
        e += self.suns(
            d,
            if camera { 0 } else { 1 },
            w,
            |pl| {
                if camera || prev_delta {
                    1.
                } else {
                    prev_pdf / (prev_pdf + pl * scale)
                }
            },
            |_, _| {},
        );
        e
    }
    /// What the media along a connection from `a` (in `medium`) let through over `len`: the
    /// object's extinction and Beer colour, else the fog box.
    pub(super) fn segment_tr(
        &self,
        medium: Option<(u32, u32)>,
        a: Vec3,
        d: Vec3,
        len: f32,
        w: &Waves,
        r: &mut Rng,
    ) -> Vec3 {
        match medium {
            Some(e) => {
                let m = &self.materials[e.1 as usize];
                (-extinction(m, w) * len).exp() * self.transmittance(&[e], len, w)
            }
            None => self.fog_transmittance(a, d, 0., len, w, r),
        }
    }
    /// Whether nothing lies between two vertices, each moved off its surface toward the other.
    fn connected(&self, a: Vec3, na: Vec3, b: Vec3, nb: Vec3) -> bool {
        let w = b - a;
        let a = a + na * if na.dot(w) > 0. { 0.0003 } else { -0.0003 };
        let b = b + nb * if nb.dot(w) < 0. { 0.0003 } else { -0.0003 };
        let e = b - a;
        let l = e.length();
        l > 0.
            && self
                .intersect(a, e / l, l, &self.accept(Ray::Shadow, a, e / l), true)
                .is_none()
    }
    pub(super) fn occluded(&self, o: Vec3, d: Vec3, limit: f32) -> bool {
        self.intersect(o, d, limit, &self.accept(Ray::Shadow, o, d), true)
            .is_some()
    }
    /// The film area light tracing spreads over: the region's share of the whole film's, since
    /// only the region's pixels start light paths (battle mode).
    fn film(&self) -> f32 {
        self.film_area * self.share
    }
    /// The pinhole's directional density over the film (`film`).
    fn camera_pdf_w(&self, w: Vec3) -> f32 {
        let c = w.dot(self.frame.forward);
        if c > 0. {
            1. / (self.film() * c * c * c)
        } else {
            0.
        }
    }
    /// The film coordinate (render pixels) point `q` projects to, inside the region.
    fn project(&self, q: Vec3) -> Option<Vec2> {
        let c = self.vp * Vec4::new(q.x, q.y, q.z, 1.);
        if c.w <= 0. {
            return None;
        }
        let size = Vec2::new(self.size[0] as f32, self.size[1] as f32);
        let f = Vec2::new(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5) * size;
        let [x0, y0, x1, y1] = self.region.map(|v| v as f32);
        (f.x >= x0 && f.y >= y0 && f.x < x1 && f.y < y1).then_some(f)
    }
    /// One light-tracing term at film point `f`, filtered as camera rays are: one pixel per axis,
    /// picked with probability w / W by a hash of the point and the sample, takes c · Wx · Wy.
    fn splat(&self, f: Vec2, c: Vec3, sample: u32, out: &mut Vec<Splat>) {
        if !c.is_finite() || c.max_element() <= 0. {
            return;
        }
        let key = mix(f.x.to_bits() ^ mix(f.y.to_bits() ^ sample));
        let u = |k: u32| (k >> 8) as f32 * (1. / 16_777_216.);
        let [x0, y0, x1, y1] = self.region.map(|v| v as i32);
        let (x, wx) = pick_filtered(self.s, f.x, x0, x1 - 1, u(key));
        let (y, wy) = pick_filtered(self.s, f.y, y0, y1 - 1, u(mix(key)));
        if wx * wy > 0. {
            out.push((y as u32 * self.size[0] + x as u32, c * wx * wy));
        }
    }

    /// One BDPT sample of the camera ray `o`, `d` (weight `weight`), sample index `sample`: the
    /// estimate for its own pixel as linear RGB, and the light-tracing terms for any pixel in
    /// `splats`. `w`: the sample's wavelengths, kept to the hero from the start when spectral.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::reference) fn bdpt(
        &self,
        mut o: Vec3,
        mut d: Vec3,
        weight: Vec3,
        sample: u32,
        w: &mut Waves,
        r: &mut Rng,
        splats: &mut Vec<Splat>,
    ) -> Vec3 {
        w.hero = w.on;
        // The hero wavelength's estimate (its weight 3 here) back to RGB; the RGB path keeps its
        // channels.
        let out = |w: &Waves, v: Vec3| {
            if w.on {
                w.to_rgb(Vec3::new(3. * v.x, 0., 0.))
            } else {
                v
            }
        };
        let depth = self.s.bounces.min(BOUNCES) as usize;
        let pe = self.bdpt_env_share();
        let lt = self.light_tracing;
        let mut l = Vec3::ZERO;

        // The camera subpath.
        let mut cam: Vec<Vertex> = Vec::with_capacity(depth + 2);
        let mut hits: Vec<Option<Hit>> = Vec::with_capacity(depth + 2);
        cam.push(Vertex {
            delta: !lt,
            ..Vertex::blank(Kind::Camera, o, weight, 0.)
        });
        hits.push(None);
        let mut beta = weight;
        let mut pdf_w = self.camera_pdf_w(d);
        let (mut prev_pdf, mut prev_delta) = (0f32, true);
        let mut media = self.camera_media.clone();
        loop {
            let kind = if cam.len() == 1 {
                Ray::Camera
            } else {
                Ray::Bounce
            };
            let from = cam[cam.len() - 1].p;
            let step = self.walk_step(&mut o, d, kind, &mut media, &mut beta, w, r);
            let (x, hit) = match step {
                Step::Escape => {
                    add_term(
                        &mut l,
                        beta * self.escape(d, cam.len() == 1, prev_delta, prev_pdf, w),
                    );
                    break;
                }
                Step::Scatter(p, albedo, g, _) => {
                    beta *= albedo;
                    let x = Vertex {
                        medium: top(&media).copied(),
                        g,
                        ..Vertex::blank(Kind::Medium, p, beta, area_pdf(pdf_w, from, p, None))
                    };
                    (x, None)
                }
                Step::Surface(h, sf) => {
                    let pdf = area_pdf(pdf_w, from, sf.p, Some(sf.n));
                    let x = self.surface_vertex(&h, &sf, d, beta, pdf, top(&media).copied(), w);
                    // Emission outside the light table (procedural emitters) is found by hits
                    // alone.
                    if x.emitter == 0 {
                        add_term(&mut l, beta * x.le);
                    }
                    (x, Some(h))
                }
            };
            cam.push(x);
            hits.push(hit);
            if cam.len() > depth + 1 || x.kind == Kind::Surface && flags(&x.m) & 1 != 0 {
                break;
            }
            let Some(next) = self.sample_vertex(&x, d, true, &media, &mut beta, w, r) else {
                break;
            };
            let t = cam.len();
            cam[t - 1].delta = next.delta;
            if t > 2 {
                let pr = to_area(next.pr, x.p, &cam[t - 2]);
                cam[t - 2].pdf_rev = pr;
            }
            if let Some(after) = next.after {
                media = after;
            }
            pdf_w = if next.delta { 0. } else { next.pf };
            prev_delta = next.delta;
            prev_pdf = next.pf;
            d = next.next;
            o = if x.kind == Kind::Medium {
                x.p
            } else {
                x.p + x.ng * if d.dot(x.ng) > 0. { 0.0003 } else { -0.0003 }
            };
        }

        // The light subpath, from a table emitter picked by power (none for the sky or a sun).
        let mut lig: Vec<Vertex> = Vec::with_capacity(depth + 1);
        if !self.lights.is_empty() && r.f() >= pe {
            let u = r.f();
            let k = self
                .cdf
                .partition_point(|c| *c <= u)
                .min(self.lights.len() - 1);
            if let Some(lp) = self.light_point(k, r) {
                self.light_subpath(lp, pe, depth, w, r, &mut lig);
            }
        }

        // Every strategy (s, t) with s + t − 2 bounces.
        let pdfs = |v: &[Vertex]| -> Vec<Pdfs> {
            v.iter()
                .map(|x| Pdfs {
                    fwd: x.pdf_fwd,
                    rev: x.pdf_rev,
                    delta: x.delta,
                })
                .collect()
        };
        let (cp, lpd) = (pdfs(&cam), pdfs(&lig));
        let ns = lig.len();
        for t in 1..=cam.len() {
            for s in 0..=ns.max(1) {
                if s + t < 2 || s + t - 2 > depth || s == 1 && t == 1 || s >= 2 && s > ns {
                    continue;
                }
                if s == 0 {
                    // The camera subpath ends on a table emitter.
                    let pt = &cam[t - 1];
                    if pt.emitter == 0 || pt.le.max_element() <= 0. {
                        continue;
                    }
                    let Some(hit) = hits[t - 1] else { continue };
                    let lp = self.hit_light(pt, &hit);
                    let wm = (cam[t - 2].p - pt.p).normalize();
                    let ptm_rev = if t > 2 {
                        to_area(Self::light_dir_pdf(&lp, wm), pt.p, &cam[t - 2])
                    } else {
                        0.
                    };
                    let mis = mis_weight(
                        &cp,
                        &lpd,
                        0,
                        t,
                        self.origin_pdf(&lp, pe),
                        ptm_rev,
                        0.,
                        0.,
                        0.,
                    );
                    add_term(&mut l, pt.beta * pt.le * mis);
                } else if t == 1 {
                    // Light tracing: the light subpath's last vertex seen by the camera.
                    if !lt || s > ns {
                        continue;
                    }
                    let qs = &lig[s - 1];
                    // A surface hidden from the camera is not seen by light tracing either.
                    if !qs.connectible() || (qs.kind == Kind::Surface && flags(&qs.m) & 2 != 0) {
                        continue;
                    }
                    let Some(film) = self.project(qs.p) else {
                        continue;
                    };
                    let wv = self.eye - qs.p;
                    let d2 = wv.length_squared();
                    let dist = d2.sqrt();
                    let wv = wv / dist;
                    let c = qs.cos(wv);
                    let ct = (-wv).dot(self.frame.forward);
                    if ct <= 0. {
                        continue;
                    }
                    let wq = (lig[s - 2].p - qs.p).normalize();
                    let (f, _) = self.vertex_f(qs, wv, wq, w);
                    let qsm_rev = to_area(self.vertex_pdf(qs, wv, wq, w), qs.p, &lig[s - 2]);
                    let tr = self.segment_tr(qs.medium, qs.p, wv, dist, w, r);
                    let c_term = qs.beta * f * c * tr / (d2 * self.film() * ct * ct * ct);
                    if c_term.max_element() <= 0. {
                        continue;
                    }
                    let q =
                        qs.p + qs.ng * if qs.ng.dot(wv) > 0. { 0.0003 } else { -0.0003 } - self.eye;
                    let lq = q.length();
                    if self
                        .intersect(
                            self.eye,
                            q / lq,
                            lq,
                            &self.accept(Ray::Camera, self.eye, q / lq),
                            true,
                        )
                        .is_some()
                    {
                        continue;
                    }
                    let qs_rev = self.camera_pdf_w(-wv) * c / d2;
                    let mis = mis_weight(&cp, &lpd, s, 1, 0., 0., qs_rev, qsm_rev, 0.);
                    self.splat(film, out(w, c_term * mis), sample, splats);
                } else if s == 1 {
                    // Next-event estimation: a fresh light choice per camera vertex.
                    let pt = &cam[t - 1];
                    if !pt.connectible() {
                        continue;
                    }
                    let wm = (cam[t - 2].p - pt.p).normalize();
                    let o = pt.p + pt.ng * 0.0003;
                    if let Some(env) = &self.env
                        && r.f() < pe
                    {
                        let (ld, pdf_e) = env.sample(r);
                        if pdf_e <= 0. {
                            continue;
                        }
                        let (f, pdf) = self.vertex_f(pt, wm, ld, w);
                        if f.max_element() > 0. && !self.occluded(o, ld, 1e4) {
                            let tr = self.segment_tr(pt.medium, pt.p, ld, 1e4, w, r);
                            let le = w.up(env.radiance(ld));
                            add_term(
                                &mut l,
                                pt.beta * f * pt.cos(ld) * tr * le / (pe * pdf_e + pdf),
                            );
                        }
                        continue;
                    }
                    if self.lights.is_empty() {
                        continue;
                    }
                    let u = r.f();
                    let k = self
                        .cdf
                        .partition_point(|c| *c <= u)
                        .min(self.lights.len() - 1);
                    if let Light::Sun {
                        d: axis,
                        om,
                        le,
                        material,
                        ..
                    } = self.lights[k]
                    {
                        let (ld, pc) = cone_sample(axis, om, r);
                        let (f, pdf) = self.vertex_f(pt, wm, ld, w);
                        if f.max_element() > 0. && !self.occluded(o, ld, 1e4) {
                            let tr = self.segment_tr(pt.medium, pt.p, ld, 1e4, w, r);
                            let le = w.up_emitted(&self.materials[material as usize], le);
                            let pl = (1. - pe) * self.entry_probability(k) * pc;
                            add_term(&mut l, pt.beta * f * pt.cos(ld) * tr * le / (pl + pdf));
                        }
                        continue;
                    }
                    let Some(lp) = self.light_point(k, r) else {
                        continue;
                    };
                    let po = self.origin_pdf(&lp, pe);
                    let ld = lp.p - pt.p;
                    let d2 = ld.length_squared().max(1e-12);
                    let len = d2.sqrt();
                    let ld = ld / len;
                    let c = lp.n.dot(-ld);
                    let g = if lp.sphere { c.max(0.) } else { c.abs() } / d2;
                    let le = self.emission_toward(&lp, -ld, w);
                    let (f, pdf) = self.vertex_f(pt, wm, ld, w);
                    let tr = self.segment_tr(pt.medium, pt.p, ld, len, w, r);
                    let c_term = pt.beta * f * pt.cos(ld) * g * le * tr / po.max(1e-30);
                    if c_term.max_element() <= 0. {
                        continue;
                    }
                    let visible = if lp.sphere {
                        !self.occluded(o, ld, shadow_limit(len, c))
                    } else {
                        self.connected(pt.p, pt.ng, lp.p, lp.n)
                    };
                    if !visible {
                        continue;
                    }
                    let pt_rev = Self::light_dir_pdf(&lp, -ld) * pt.cos(ld) / d2;
                    let ptm_rev = if t > 2 {
                        to_area(self.vertex_pdf(pt, ld, wm, w), pt.p, &cam[t - 2])
                    } else {
                        0.
                    };
                    let qs_rev = pdf * lp.n.dot(ld).abs() / d2;
                    let mis = mis_weight(&cp, &lpd, 1, t, pt_rev, ptm_rev, qs_rev, 0., po);
                    add_term(&mut l, c_term * mis);
                } else {
                    // Both subpaths joined by one edge.
                    let (qs, pt) = (&lig[s - 1], &cam[t - 1]);
                    if !qs.connectible() || !pt.connectible() {
                        continue;
                    }
                    let wc = qs.p - pt.p;
                    let d2 = wc.length_squared().max(1e-12);
                    let dist = d2.sqrt();
                    let wc = wc / dist;
                    let wm = (cam[t - 2].p - pt.p).normalize();
                    let wq = (lig[s - 2].p - qs.p).normalize();
                    let (fa, pa) = self.vertex_f(pt, wm, wc, w);
                    let (fb, _) = self.vertex_f(qs, -wc, wq, w);
                    let tr = self.segment_tr(pt.medium, pt.p, wc, dist, w, r);
                    let c_term = pt.beta * fa * qs.beta * fb * pt.cos(wc) * qs.cos(wc) * tr / d2;
                    if c_term.max_element() <= 0. || !self.connected(pt.p, pt.ng, qs.p, qs.ng) {
                        continue;
                    }
                    let pt_rev = self.vertex_pdf(qs, wq, -wc, w) * pt.cos(wc) / d2;
                    let ptm_rev = if t > 2 {
                        to_area(self.vertex_pdf(pt, wc, wm, w), pt.p, &cam[t - 2])
                    } else {
                        0.
                    };
                    let qs_rev = pa * qs.cos(wc) / d2;
                    let qsm_rev = to_area(self.vertex_pdf(qs, -wc, wq, w), qs.p, &lig[s - 2]);
                    let mis = mis_weight(&cp, &lpd, s, t, pt_rev, ptm_rev, qs_rev, qsm_rev, 0.);
                    add_term(&mut l, c_term * mis);
                }
            }
        }
        if l.is_finite() { out(w, l) } else { Vec3::ZERO }
    }

    /// The light subpath from `lp` into `lig`: its first direction cosine-weighted from the side
    /// it picks (either half of a two-sided panel) or uniform in a spot's cone.
    fn light_subpath(
        &self,
        lp: LightPoint,
        pe: f32,
        depth: usize,
        w: &Waves,
        r: &mut Rng,
        lig: &mut Vec<Vertex>,
    ) {
        let po = self.origin_pdf(&lp, pe);
        let two = Self::two_sided(&lp);
        let n = if two && r.f() < 0.5 { -lp.n } else { lp.n };
        let k = Self::spot_cone(&lp);
        let (mut d, pd) = if k != 0. {
            let c = 1. + (k - 1.) * r.f();
            let s = (1. - c * c).max(0.).sqrt();
            let a = 2. * PI * r.f();
            (
                basis(n, Vec3::new(s * a.cos(), s * a.sin(), c)),
                1. / (2. * PI * (1. - k)),
            )
        } else {
            let d = cosine_sample(n, r);
            (d, n.dot(d) / PI * if two { 0.5 } else { 1. })
        };
        let le = self.emission_toward(&lp, d, w);
        if po <= 0. || pd <= 0. || le.max_element() <= 0. {
            return;
        }
        lig.push(Vertex {
            n,
            ng: n,
            emitter: lp.k as u32 + 1,
            light: Some(lp),
            ..Vertex::blank(Kind::Light, lp.p, Vec3::splat(1. / po), po)
        });
        let mut beta = le * n.dot(d) / (po * pd);
        let mut o = lp.p + n * 0.0003;
        let mut pdf_w = pd;
        let mut media = vec![];
        loop {
            let from = lig[lig.len() - 1].p;
            let x = match self.walk_step(&mut o, d, Ray::Bounce, &mut media, &mut beta, w, r) {
                Step::Escape => break,
                Step::Scatter(p, albedo, g, _) => {
                    beta *= albedo;
                    Vertex {
                        medium: top(&media).copied(),
                        g,
                        ..Vertex::blank(Kind::Medium, p, beta, area_pdf(pdf_w, from, p, None))
                    }
                }
                Step::Surface(h, sf) => {
                    let pdf = area_pdf(pdf_w, from, sf.p, Some(sf.n));
                    self.surface_vertex(&h, &sf, d, beta, pdf, top(&media).copied(), w)
                }
            };
            lig.push(x);
            if lig.len() > depth || x.kind == Kind::Surface && flags(&x.m) & 1 != 0 {
                break;
            }
            let Some(next) = self.sample_vertex(&x, d, false, &media, &mut beta, w, r) else {
                break;
            };
            let s = lig.len();
            lig[s - 1].delta = next.delta;
            let pr = to_area(next.pr, x.p, &lig[s - 2]);
            lig[s - 2].pdf_rev = pr;
            if let Some(after) = next.after {
                media = after;
            }
            pdf_w = if next.delta { 0. } else { next.pf };
            d = next.next;
            o = if x.kind == Kind::Medium {
                x.p
            } else {
                x.p + x.ng * if d.dot(x.ng) > 0. { 0.0003 } else { -0.0003 }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// For one path of n vertices with arbitrary densities, the weights of every strategy that
    /// can make it sum to 1 and each is its density's share (the balance heuristic).
    #[test]
    fn mis_weights_of_one_path_sum_to_one() {
        let mut r = Rng::new(7, 3);
        for n in 3..8 {
            for _ in 0..20 {
                // a[i]: density of vertex i from the camera's side; b[i]: from the light's.
                let a: Vec<f32> = (0..n).map(|_| 0.1 + 4. * r.f()).collect();
                let b: Vec<f32> = (0..n).map(|_| 0.1 + 4. * r.f()).collect();
                let density = |s: usize| -> f32 {
                    let t = n - s;
                    (1..t).map(|i| a[i]).product::<f32>() * (t..n).map(|i| b[i]).product::<f32>()
                };
                let total: f32 = (0..n).map(density).sum();
                let mut sum = 0.;
                for s in 0..n {
                    let t = n - s;
                    let cam: Vec<Pdfs> = (0..t)
                        .map(|i| Pdfs {
                            fwd: a[i],
                            rev: b[i],
                            delta: false,
                        })
                        .collect();
                    let lig: Vec<Pdfs> = (0..s)
                        .map(|j| Pdfs {
                            fwd: b[n - 1 - j],
                            rev: a[n - 1 - j],
                            delta: false,
                        })
                        .collect();
                    let at = |v: &[f32], i: isize| {
                        if i >= 0 && (i as usize) < n {
                            v[i as usize]
                        } else {
                            0.
                        }
                    };
                    let (ti, si) = (t as isize, n as isize - s as isize);
                    let wgt = mis_weight(
                        &cam,
                        &lig,
                        s,
                        t,
                        at(&b, ti - 1),
                        at(&b, ti - 2),
                        at(&a, si),
                        at(&a, si + 1),
                        b[n - 1],
                    );
                    let expected = density(s) / total;
                    assert!(
                        (wgt - expected).abs() < 1e-4,
                        "n {n} s {s}: {wgt} vs {expected}"
                    );
                    sum += wgt;
                }
                assert!((sum - 1.).abs() < 1e-4, "n {n}: {sum}");
            }
        }
    }

    /// A delta vertex removes the strategies that would connect at it, and the rest still sum
    /// to 1.
    #[test]
    fn delta_vertices_drop_their_strategies() {
        let n = 5;
        // Vertex 2 is a mirror: the densities through it (vertex 3's from the camera's side,
        // vertex 1's from the light's) are 0, and the strategies that would connect at it are
        // gone.
        let a = [1., 2., 3., 0., 1.5];
        let b = [0.7, 0., 2.5, 0.9, 3.];
        let delta = [false, false, true, false, false];
        let mut sum = 0.;
        for s in 0..n {
            let t = n - s;
            // Strategies whose connection touches the delta vertex can't make the path.
            if t - 1 == 2 || n - s == 2 {
                continue;
            }
            let cam: Vec<Pdfs> = (0..t)
                .map(|i| Pdfs {
                    fwd: a[i],
                    rev: b[i],
                    delta: delta[i],
                })
                .collect();
            let lig: Vec<Pdfs> = (0..s)
                .map(|j| {
                    let i = n - 1 - j;
                    Pdfs {
                        fwd: b[i],
                        rev: a[i],
                        delta: delta[i],
                    }
                })
                .collect();
            let get = |v: &[f32], i: isize| {
                if i >= 0 && (i as usize) < n {
                    v[i as usize]
                } else {
                    0.
                }
            };
            let (ti, si) = (t as isize, n as isize - s as isize);
            sum += mis_weight(
                &cam,
                &lig,
                s,
                t,
                get(&b, ti - 1),
                get(&b, ti - 2),
                get(&a, si),
                get(&a, si + 1),
                b[n - 1],
            );
        }
        assert!((sum - 1.).abs() < 1e-4, "{sum}");
    }

    #[test]
    fn a_splat_picks_pixels_by_the_filter() {
        let mut s = crate::scene::Settings::default();
        for filter in 0..3 {
            s.pixel_filter = filter;
            s.filter_width = 2.;
            // Over many picks, pixel x gets W share in proportion to its density.
            let f = 10.3;
            let mut got = [0f32; 20];
            let n = 20000;
            for i in 0..n {
                let (x, w) = pick_filtered(&s, f, 0, 19, (i as f32 + 0.5) / n as f32);
                got[x as usize] += w / n as f32;
            }
            for (x, g) in got.iter().enumerate() {
                let want = filter_density(&s, f - (x as f32 + 0.5));
                assert!(
                    (g - want).abs() < 2e-3,
                    "filter {filter} x {x}: {g} vs {want}"
                );
            }
        }
    }
}
