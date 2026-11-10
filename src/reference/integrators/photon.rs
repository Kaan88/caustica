//! Progressive photon mapping (`Settings::integrator` 2), the GPU's estimator (`integrators.hlsl`,
//! Knaus and Zwicker's probabilistic form): every frame of `Settings::spp` samples gets its own
//! photon map of `Settings::photons` photons, gathered with a radius that shrinks from frame to
//! frame as r²·(i + 2/3)/(i + 1), so each frame is an independent, biased estimate and their mean
//! converges. The bias depends on the frame index, so the CPU keeps the GPU's frames: sample k
//! belongs to frame k / spp, whatever the passes.
//!
//! Photons start on the table's emitters (triangles and spheres by power, a sun from the disk of
//! the scene's bounding sphere facing it), on emissive procedural objects and on the sky, with the
//! GPU's shares, and walk up to min(bounces, 32) segments (Russian roulette from the third),
//! leaving a photon at every surface a gather would read (`gather_surface`) and every scatter in
//! a medium, from the second interaction on: the first is direct light, which the camera side
//! takes by next-event estimation. Camera paths walk through delta surfaces and near mirrors, and
//! at the first other surface take direct light (one light sample and one BSDF sample, balance
//! weighted) plus the photons within the radius and a slab a quarter radius thick around the
//! tangent plane; partly transmissive surfaces go on through the dielectric with its probability.
//! A scatter in a medium ends the path with direct light and the volume photons.
//!
//! The grid is exact (a map from cell to its photons, in emission order), where the GPU's is a
//! hash with chains it may sample systematically when they are long: both unbiased. Photons
//! beyond the GPU's buffer (photons × min(bounces, 10) deposits) are dropped as there. As on the
//! GPU, clear glass blocks shadow rays (`Scene::glass_blocks`) and portals, the shadow catcher,
//! ground projection, ReSTIR, `mis_mode` and the firefly clamp are the path tracer's alone; a
//! spectral scene gives each photon and each camera sample one wavelength from the start.
use crate::lighting::PackedMaterial;
use crate::reference::bsdf::{emitted, eval_bsdf, f0_of, flags, material_at, rgb};
use crate::reference::bvh::Hit;
use crate::reference::integrators::bdpt::{Kind, Step, Vertex};
use crate::reference::integrators::shading::sample_lobes;
use crate::reference::lights::{SdfLight, cone_sample};
use crate::reference::media::{hg_pdf, hg_sample, top};
use crate::reference::sampling::{Rng, basis};
use crate::reference::scene::{Light, Ray, Scene};
use crate::reference::spectral::Waves;
use crate::render::PHOTON_BOUNCES;
use glam::{IVec3, Vec3};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The most segments a photon or a camera path walks (`PHOTON_WALK`).
const WALK: u32 = 32;
/// The gather's slab around the tangent plane, in radii (`GATHER_SLAB`).
const SLAB: f32 = 0.25;
/// Photons a thread traces at a time.
const CHUNK: usize = 1024;

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Whether the gather reads photons at a surface of material `m` (as the camera sees it): not
/// all glass, not a beam splitter, not a near mirror (rough < 0.2 and a metal or a reflectance
/// over a half at normal incidence), whose sharp lobe the radius would blur.
pub(in crate::reference) fn gather_surface(m: &PackedMaterial) -> bool {
    m.lobes[2] < 1.
        && flags(m) & 4096 == 0
        && !(m.color_rough[3] < 0.2 && (m.lobes[0] > 0.5 || luminance(f0_of(m)) > 0.5))
}

/// The squared gather radius of frame `frame` (0 first) from the first one's, `r2`: Knaus and
/// Zwicker's schedule with alpha = 2/3, multiplied up in f32 as the GPU does.
pub(in crate::reference) fn radius2(r2: f32, frame: u32) -> f32 {
    let mut scale = 1f32;
    for i in 1..=frame {
        let i = i as f32;
        scale *= (i + 2. / 3.) / (i + 1.);
    }
    r2 * scale
}

/// A deposited photon: where, the direction it came from, its power (linear RGB, negative
/// components kept for spectral photons).
#[derive(Clone, Copy, Debug)]
struct Photon {
    p: Vec3,
    wi: Vec3,
    power: Vec3,
}

/// Photons by grid cell (cells one radius wide): each cell's photons are a run of `photons`.
#[derive(Default)]
struct Grid {
    photons: Vec<Photon>,
    cells: HashMap<IVec3, (u32, u32)>,
}
impl Grid {
    /// The grid of `list`, each cell's photons kept in their order.
    fn new(list: Vec<Photon>, r: f32) -> Self {
        let mut keyed: Vec<(IVec3, Photon)> = list.into_iter().map(|q| (cell(q.p, r), q)).collect();
        keyed.sort_by_key(|(c, _)| (c.x, c.y, c.z));
        let mut cells = HashMap::new();
        let mut start = 0;
        for i in 1..=keyed.len() {
            if i == keyed.len() || keyed[i].0 != keyed[start].0 {
                cells.insert(keyed[start].0, (start as u32, i as u32));
                start = i;
            }
        }
        Self {
            photons: keyed.into_iter().map(|(_, q)| q).collect(),
            cells,
        }
    }
    /// The photons of the 27 cells around `p`.
    fn near(&self, p: Vec3, r: f32) -> impl Iterator<Item = &Photon> {
        let c0 = cell(p, r);
        (0..27).flat_map(move |k| {
            let c = c0 + IVec3::new(k % 3, (k / 3) % 3, k / 9) - 1;
            let (a, b) = self.cells.get(&c).copied().unwrap_or((0, 0));
            self.photons[a as usize..b as usize].iter()
        })
    }
}
fn cell(p: Vec3, r: f32) -> IVec3 {
    (p / r).floor().as_ivec3()
}

/// One frame's photons, surface and volume apart, and the radius they are gathered with.
pub(in crate::reference) struct PhotonMap {
    pub(in crate::reference) frame: u32,
    r: f32,
    /// Photons emitted (every one counts, deposited or not).
    emitted: u32,
    surface: Grid,
    volume: Grid,
}

/// The sphere sun and sky photons start from: the visible objects' box and every triangle but
/// the floor's (material 3, which often reaches far past the scene), grown by the fog box when
/// there is fog; centre and radius (at least 0.5), else the origin and 2.
fn bounds(scene: &Scene) -> (Vec3, f32) {
    let s = scene.s;
    let (mut lo, mut hi) = s.objects_bounds();
    for t in scene.bvh.tris.iter().filter(|t| t.material != 3) {
        for p in t.p {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    if s.fog.density > 0. && lo.is_finite() {
        let f = &s.fog;
        lo = lo.min(Vec3::new(-f.extent, -1., -f.extent));
        hi = hi.max(Vec3::new(f.extent, f.height, f.extent));
    }
    if lo.is_finite() {
        ((lo + hi) * 0.5, ((hi - lo).length() * 0.5).max(0.5))
    } else {
        (Vec3::ZERO, 2.)
    }
}

/// Where a photon starts: origin, direction, power (at its wavelengths).
type Emission = (Vec3, Vec3, Vec3);

impl Scene<'_> {
    /// Frame `frame`'s photon map, traced on `threads` threads (None when `cancel` stopped it).
    /// Photon i draws from its own stream, so the map doesn't depend on the threads.
    pub(in crate::reference) fn photon_map(
        &self,
        frame: u32,
        threads: usize,
        cancel: &AtomicBool,
    ) -> Option<PhotonMap> {
        let (centre, radius) = bounds(self);
        let r0 = self.s.photon_radius * radius;
        let r = radius2(r0 * r0, frame).sqrt();
        let n = self.photons.max(1) as usize;
        let sdf = SdfLight::of(&self.sdfs, &self.materials);
        let chunks: Vec<std::sync::Mutex<Vec<(Photon, bool)>>> =
            (0..n.div_ceil(CHUNK)).map(|_| Default::default()).collect();
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let c = next.fetch_add(1, Ordering::Relaxed);
                        if c >= chunks.len() || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let mut out = vec![];
                        for i in c * CHUNK..((c + 1) * CHUNK).min(n) {
                            self.trace_photon(i as u32, frame, (centre, radius), &sdf, &mut out);
                        }
                        *chunks[c].lock().unwrap_or_else(|e| e.into_inner()) = out;
                    }
                });
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        // In emission order, cut at the GPU's capacity: its buffer holds the scene's whole count
        // of photons (`Settings::photons`), also when battle mode's quadrant traces a share.
        let capacity =
            self.s.photons.max(1) as usize * self.s.bounces.clamp(1, PHOTON_BOUNCES) as usize;
        let (mut surface, mut volume) = (vec![], vec![]);
        for (q, in_medium) in chunks
            .into_iter()
            .flat_map(|c| c.into_inner().unwrap_or_else(|e| e.into_inner()))
            .take(capacity)
        {
            if in_medium {
                volume.push(q);
            } else {
                surface.push(q);
            }
        }
        Some(PhotonMap {
            frame,
            r,
            emitted: n as u32,
            surface: Grid::new(surface, r),
            volume: Grid::new(volume, r),
        })
    }

    /// Where photon `i` starts, chosen as the GPU chooses: the sky with probability 1 (nothing
    /// else) or 1/2, else the table and the procedural emitters half and half when both exist.
    fn emit(
        &self,
        (centre, radius): (Vec3, f32),
        sdf: &[SdfLight],
        w: &Waves,
        r: &mut Rng,
    ) -> Option<Emission> {
        let s = self.s;
        let (tri, procedural) = (!self.lights.is_empty(), !sdf.is_empty());
        let sky = self.env.is_some() || Vec3::from(s.background).max_element() > 0. || s.sky > 0.;
        let p_env = match (sky, tri || procedural) {
            (false, _) => 0.,
            (true, true) => 0.5,
            (true, false) => 1.,
        };
        let p_tri = (1. - p_env)
            * if procedural {
                if tri { 0.5 } else { 0. }
            } else {
                1.
            };
        let disk = PI * radius * radius;
        // A point on the bounding sphere's disk facing `ld`, just outside it.
        let disk_origin = |ld: Vec3, r: &mut Rng| {
            let (q, a) = (radius * r.f().sqrt(), 2. * PI * r.f());
            centre + basis(ld, Vec3::new(q * a.cos(), q * a.sin(), radius * 1.01))
        };
        if r.f() < p_env {
            let (ld, pw, le) = match &self.env {
                Some(env) => {
                    let (ld, pw) = env.sample(r);
                    (ld, pw, env.radiance(ld))
                }
                None => {
                    let ld = uniform_sphere(r);
                    (ld, 1. / (4. * PI), self.plain_sky(ld))
                }
            };
            if pw <= 0. {
                return None;
            }
            return Some((disk_origin(ld, r), -ld, w.up(le) * disk / (p_env * pw)));
        }
        if procedural && (!tri || r.f() < 0.5) {
            return self.sdf_photon((1. - p_env) - p_tri, sdf, w, r);
        }
        if !tri {
            return None;
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
            let le = w.up_emitted(&self.materials[material as usize], le);
            let p = p_tri * self.entry_probability(k) * pc;
            return (p > 0.).then(|| (disk_origin(ld, r), -ld, le * disk / p));
        }
        let lp = self.light_point(k, r)?;
        let po = p_tri * self.entry_probability(k) / lp.area.max(1e-12);
        let two = Self::two_sided(&lp);
        let n = if two && r.f() < 0.5 { -lp.n } else { lp.n };
        let cone = Self::spot_cone(&lp);
        let (d, pd) = if cone != 0. {
            let c = 1. + (cone - 1.) * r.f();
            let s = (1. - c * c).max(0.).sqrt();
            let a = 2. * PI * r.f();
            (
                basis(n, Vec3::new(s * a.cos(), s * a.sin(), c)),
                1. / (2. * PI * (1. - cone)),
            )
        } else {
            let d = crate::reference::sampling::cosine_sample(n, r);
            (d, n.dot(d) / PI * if two { 0.5 } else { 1. })
        };
        if po <= 0. || pd <= 0. {
            return None;
        }
        let b = self.emission_toward(&lp, d, w) * n.dot(d) / (po * pd);
        Some((lp.p + n * 0.0003, d, b))
    }

    /// A photon from an emissive procedural object, which has no points to sample: a random
    /// line through the bounding sphere of one picked by power (a uniform point on the sphere,
    /// cosine-weighted inward) crosses its surface with density |cos| dA dω / (π A_sphere)
    /// (Cauchy-Crofton), a Lambertian emitter's. One of the line's k crossings is kept and
    /// stands for k · π L A_sphere / 2, leaving on the side its normal faces. `p`: the share
    /// of photons these emitters take.
    fn sdf_photon(&self, p: f32, sdf: &[SdfLight], w: &Waves, r: &mut Rng) -> Option<Emission> {
        let u = r.f();
        let j = sdf.partition_point(|l| l.cdf <= u).min(sdf.len() - 1);
        let q = sdf[j].cdf - if j > 0 { sdf[j - 1].cdf } else { 0. };
        let l = &sdf[j];
        let object = &self.sdfs[l.object];
        let out_n = uniform_sphere(r);
        let d = crate::reference::sampling::cosine_sample(-out_n, r);
        let x0 = l.centre + l.radius * out_n;
        let chord = 2. * l.radius * d.dot(-out_n);
        // Every crossing along the chord, each stepped past so it isn't found twice.
        let scale = object.w2o.transform_vector3(d).length();
        let mut hits = vec![];
        let mut t = 0.;
        while hits.len() < 16 {
            let Some(th) = object.hit(x0, d, t, chord) else {
                break;
            };
            hits.push(th);
            t = th + 1e-4;
            for _ in 0..32 {
                let at = object.w2o.transform_point3(x0 + d * t);
                if t >= chord || crate::reference::sdf::field(object.raw, at).abs() >= 2e-4 * scale
                {
                    break;
                }
                t += 2e-4;
            }
        }
        let k = hits.len();
        if k == 0 || q <= 0. || p <= 0. {
            return None;
        }
        let th = hits[((r.f() * k as f32) as usize).min(k - 1)];
        let nn = object.normal(x0, d, th);
        let w_out = if d.dot(nn) > 0. { d } else { -d };
        let at = x0 + d * th;
        let m = material_at(&self.materials[object.material as usize], at);
        let le = w.up_emitted(&m, emitted(&m, rgb(m.color_rough), true, 1.));
        let b = le * (2. * PI * PI * l.radius * l.radius) * k as f32 / (p * q);
        b.is_finite().then_some((at + nn * 0.0003, w_out, b))
    }

    /// Traces photon `i` of frame `frame`, appending its deposits (and whether each is in a
    /// medium) to `out`.
    fn trace_photon(
        &self,
        i: u32,
        frame: u32,
        sphere: (Vec3, f32),
        sdf: &[SdfLight],
        out: &mut Vec<(Photon, bool)>,
    ) {
        // Streams apart from the pixels' (their keys are pixel indices).
        let mut r = Rng::new((1 << 40) | i as u64, frame as u64);
        let mut w = self.waves(&r);
        let Some((mut o, mut d, mut b)) = self.emit(sphere, sdf, &w, &mut r) else {
            return;
        };
        w.hero(&mut b);
        let mut media = vec![];
        let mut deposit = |p: Vec3, wi: Vec3, b: Vec3, w: &Waves, volume: bool| {
            out.push((
                Photon {
                    p,
                    wi,
                    power: w.to_rgb(b),
                },
                volume,
            ));
        };
        let roulette = |k: u32, before: f32, b: &mut Vec3, r: &mut Rng| {
            if k >= 2 {
                let survival = (b.max_element() / before.max(1e-30)).clamp(0.05, 0.95);
                if r.f() > survival {
                    return false;
                }
                *b /= survival;
            }
            true
        };
        for k in 0..self.s.bounces.min(WALK) {
            match self.walk_step(&mut o, d, Ray::Bounce, &mut media, &mut b, &w, &mut r) {
                Step::Escape => return,
                Step::Scatter(p, albedo, g, _) => {
                    if k > 0 {
                        deposit(p, -d, b, &w, true);
                    }
                    let before = b.max_element();
                    b *= albedo;
                    let (next, _) = hg_sample(d, g, &mut r);
                    if !roulette(k, before, &mut b, &mut r) {
                        return;
                    }
                    (o, d) = (p, next);
                }
                Step::Surface(h, sf) => {
                    let x = self.surface_vertex(&h, &sf, d, b, 0., top(&media).copied(), &w);
                    if flags(&x.m) & 1 != 0 {
                        return;
                    }
                    if k > 0 && gather_surface(&self.vertex_material(&x, -d, &w)) {
                        deposit(x.p, -d, b, &w, false);
                    }
                    let before = b.max_element();
                    let Some(next) = self.sample_vertex(&x, d, false, &media, &mut b, &w, &mut r)
                    else {
                        return;
                    };
                    if let Some(after) = next.after {
                        media = after;
                    }
                    if !roulette(k, before, &mut b, &mut r) {
                        return;
                    }
                    d = next.next;
                    o = x.p + x.ng * if d.dot(x.ng) > 0. { 0.0003 } else { -0.0003 };
                }
            }
        }
    }

    /// One photon-mapping sample of the camera ray `o`, `d` (weight `weight`) with frame map
    /// `map`: linear RGB. `w`: the sample's wavelengths, kept to the hero from the start.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::reference) fn photon_gather(
        &self,
        mut o: Vec3,
        mut d: Vec3,
        weight: Vec3,
        map: &PhotonMap,
        w: &mut Waves,
        r: &mut Rng,
    ) -> Vec3 {
        w.hero = w.on;
        let w = &*w;
        let pe = self.bdpt_env_share();
        let mut beta = weight;
        let mut l = Vec3::ZERO;
        let mut media = self.camera_media.clone();
        for bounce in 0..self.s.bounces.min(WALK) {
            let kind = if bounce == 0 {
                Ray::Camera
            } else {
                Ray::Bounce
            };
            match self.walk_step(&mut o, d, kind, &mut media, &mut beta, w, r) {
                Step::Escape => {
                    l += beta * self.escape(d, bounce == 0, true, 0., w);
                    break;
                }
                Step::Scatter(p, albedo, g, sigma) => {
                    // A scatter: direct light plus the volume photons, and the path ends.
                    beta *= albedo;
                    let medium = top(&media).copied();
                    l += beta
                        * (self.volume_direct(p, d, g, medium, pe, w, r)
                            + map.gather_volume(p, d, g, sigma, w));
                    break;
                }
                Step::Surface(h, sf) => {
                    let x = self.surface_vertex(&h, &sf, d, beta, 0., top(&media).copied(), w);
                    // Reached through the camera, delta surfaces or near mirrors: nothing else
                    // finds this emission.
                    l += beta * x.le;
                    if flags(&x.m) & 1 != 0 {
                        break;
                    }
                    let m = self.vertex_material(&x, -d, w);
                    let next = if gather_surface(&m) {
                        l += beta
                            * (self.direct_light_at(&x, &m, -d, pe, w, r)
                                + map.gather(&x, &m, -d, w));
                        // The dielectric part of a partly transmissive surface goes on, with its
                        // probability (the rest was just gathered).
                        let z = m.lobes[2];
                        if z <= 0. || r.f() >= z {
                            break;
                        }
                        let s = self.dielectric(&media, &m, x.material, x.front, x.n, d, w, r);
                        beta *= s.weight;
                        if !s.reflected {
                            beta *= s.eta * s.eta;
                            media = s.after;
                        }
                        s.next
                    } else {
                        let Some(next) = self.sample_vertex(&x, d, true, &media, &mut beta, w, r)
                        else {
                            break;
                        };
                        if let Some(after) = next.after {
                            media = after;
                        }
                        next.next
                    };
                    d = next;
                    o = x.p + x.ng * if d.dot(x.ng) > 0. { 0.0003 } else { -0.0003 };
                }
            }
        }
        let l = if w.on {
            w.to_rgb(Vec3::new(3. * l.x, 0., 0.))
        } else {
            l
        };
        if l.is_finite() { l } else { Vec3::ZERO }
    }

    /// The light sample next-event estimation at a gather takes, as BDPT's: the environment map
    /// with probability `pe`, else the table (procedural emitters are found by hits alone):
    /// direction, radiance, solid-angle pdf and shadow length.
    fn gather_light(
        &self,
        x: Vec3,
        pe: f32,
        w: &Waves,
        r: &mut Rng,
    ) -> Option<(Vec3, Vec3, f32, f32)> {
        if let Some(env) = &self.env
            && r.f() < pe
        {
            let (ld, pdf) = env.sample(r);
            return (pdf > 0.).then(|| (ld, w.up(env.radiance(ld)), pe * pdf, 1e4));
        }
        let (ld, le, pdf, limit, _) = self.sample_table(x, w, r)?;
        Some((ld, le, pdf * self.table_scale(pe), limit))
    }
    /// The factor from the path tracer's light-table pdf to the gather's, whose table takes the
    /// share 1 − `pe` (no procedural emitters).
    fn table_scale(&self, pe: f32) -> f32 {
        let pt = (1. - self.p_env) * (1. - self.sdf_share);
        if pt > 0. { (1. - pe) / pt } else { 0. }
    }
    /// What a ray from `from` along `next` (sampled with density `pf`) finds, balance-weighted
    /// against `gather_light`: the sky and suns past the scene, or an emitter, through the media
    /// from `medium`.
    #[allow(clippy::too_many_arguments)]
    fn sampled_light(
        &self,
        from: Vec3,
        o: Vec3,
        next: Vec3,
        pf: f32,
        medium: Option<(u32, u32)>,
        pe: f32,
        w: &Waves,
        r: &mut Rng,
    ) -> Vec3 {
        let Some(h) = self.intersect(o, next, 1e4, &self.accept(Ray::Bounce, o, next), false)
        else {
            return self.escape(next, false, false, pf, w)
                * self.segment_tr(medium, from, next, 1e4, w, r);
        };
        let e = self.hit_emission(&h, o, next, w);
        if e.max_element() <= 0. {
            return Vec3::ZERO;
        }
        let weight = match h.sdf {
            None if self.tri_light.get(h.tri).is_some_and(|&k| k != 0) => {
                let pl = self.light_pdf(h.tri, from, o + next * h.t) * self.table_scale(pe);
                pf / (pf + pl)
            }
            _ => 1.,
        };
        e * weight * self.segment_tr(medium, from, next, h.t, w, r)
    }
    /// What the surface `h` (hit from `o` along `d`) emits back along the ray.
    fn hit_emission(&self, h: &Hit, o: Vec3, d: Vec3, w: &Waves) -> Vec3 {
        let sf = self.surface(h, o, d);
        let m = material_at(&self.materials[sf.material as usize], sf.p);
        emitted(
            &m,
            w.up_emission(&m, self.emission_colour(&m, h.tri, h.b)),
            sf.front,
            sf.emission_cos(d),
        )
    }
    /// Direct light at a gather point `x` (material `m` toward `v`): one light sample and one
    /// sample of the BSDF's other lobes (no transmission), balance-weighted.
    #[allow(clippy::too_many_arguments)]
    fn direct_light_at(
        &self,
        x: &Vertex,
        m: &PackedMaterial,
        v: Vec3,
        pe: f32,
        w: &Waves,
        r: &mut Rng,
    ) -> Vec3 {
        let mut l = Vec3::ZERO;
        let z = m.lobes[2];
        let o = x.p + x.ng * 0.0003;
        if let Some((ld, le, pl, limit)) = self.gather_light(x.p, pe, w, r)
            && x.n.dot(ld) > 0.
        {
            let (f, pdf) = eval_bsdf(m, x.n, x.tangent, v, ld, x.lambda);
            let pdf = pdf / (1. - z);
            if f.max_element() > 0. && !self.occluded(o, ld, limit) {
                let tr = self.segment_tr(x.medium, x.p, ld, limit, w, r);
                l += f * x.n.dot(ld) * le * tr / (pl + pdf).max(1e-8);
            }
        }
        if z < 1.
            && let Some((next, _)) = sample_lobes(m, x.n, x.tangent, -v, x.lambda, r)
            && x.n.dot(next) > 0.
        {
            let (f, pf) = eval_bsdf(m, x.n, x.tangent, v, next, x.lambda);
            let pf = pf / (1. - z);
            if pf > 0. {
                let b = f * x.n.dot(next) / pf;
                l += b * self.sampled_light(x.p, o, next, pf, x.medium, pe, w, r);
            }
        }
        l
    }
    /// Direct light at a scatter point: one light sample weighted by the phase function and one
    /// phase-function sample, balance-weighted, through the medium's transmittance.
    #[allow(clippy::too_many_arguments)]
    fn volume_direct(
        &self,
        x: Vec3,
        d: Vec3,
        g: f32,
        medium: Option<(u32, u32)>,
        pe: f32,
        w: &Waves,
        r: &mut Rng,
    ) -> Vec3 {
        let mut l = Vec3::ZERO;
        if let Some((ld, le, pl, limit)) = self.gather_light(x, pe, w, r) {
            let pp = hg_pdf(d.dot(ld), g);
            if !self.occluded(x, ld, limit) {
                l += pp * le * self.segment_tr(medium, x, ld, limit, w, r) / (pl + pp).max(1e-8);
            }
        }
        let (next, pf) = hg_sample(d, g, r);
        l + self.sampled_light(x, x, next, pf, medium, pe, w, r)
    }
}

impl PhotonMap {
    /// The radiance the surface photons near `x` reflect toward `v` (flux through the BSDF over
    /// the gather disk's area). No cosine: the photons' density on the surface holds it.
    fn gather(&self, x: &Vertex, m: &PackedMaterial, v: Vec3, w: &Waves) -> Vec3 {
        debug_assert_eq!(x.kind, Kind::Surface);
        let r = self.r;
        let mut sum = Vec3::ZERO;
        for q in self.surface.near(x.p, r) {
            let dq = q.p - x.p;
            if dq.length_squared() < r * r && dq.dot(x.n).abs() < SLAB * r && x.n.dot(q.wi) > 0. {
                sum += eval_bsdf(m, x.n, x.tangent, v, q.wi, x.lambda).0 * w.up(q.power);
            }
        }
        sum / (self.emitted.max(1) as f32 * PI * r * r)
    }
    /// The radiance the volume photons near `x` scatter toward −d (phase function times power
    /// over the sphere's volume), divided by the extinction there, the rate they were left at.
    fn gather_volume(&self, x: Vec3, d: Vec3, g: f32, sigma: Vec3, w: &Waves) -> Vec3 {
        let r = self.r;
        let mut sum = Vec3::ZERO;
        for q in self.volume.near(x, r) {
            if (q.p - x).length_squared() < r * r {
                sum += hg_pdf(q.wi.dot(d), g) * q.power;
            }
        }
        w.up(sum)
            / (self.emitted.max(1) as f32
                * (4. / 3.)
                * PI
                * r
                * r
                * r
                * sigma.max(Vec3::splat(1e-8)))
    }
}

fn uniform_sphere(r: &mut Rng) -> Vec3 {
    let z = 1. - 2. * r.f();
    let s = (1. - z * z).max(0.).sqrt();
    let a = 2. * PI * r.f();
    Vec3::new(s * a.cos(), z, s * a.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_follows_the_gpu_schedule() {
        // passes.rs: the scale starts at 1 and each frame multiplies (i + 2/3) / (i + 1) in.
        let mut scale = 1f32;
        for f in 0..300u32 {
            assert_eq!(radius2(1., f), scale, "frame {f}");
            let i = (f + 1) as f32;
            scale *= (i + 2. / 3.) / (i + 1.);
        }
        // r² falls as i^(−1/3).
        let ratio = radius2(1., 1000) / radius2(1., 125);
        assert!((ratio - 0.5).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn the_grid_finds_photons_in_neighbouring_cells_only() {
        let r = 0.1;
        let at = |x: f32| Photon {
            p: Vec3::new(x, 0., 0.),
            wi: Vec3::Y,
            power: Vec3::ONE,
        };
        let g = Grid::new(vec![at(0.05), at(0.15), at(0.25), at(-0.05), at(0.35)], r);
        let mut near: Vec<f32> = g.near(Vec3::new(0.12, 0., 0.), r).map(|q| q.p.x).collect();
        near.sort_by(f32::total_cmp);
        assert_eq!(near, vec![0.05, 0.15, 0.25]);
    }

    #[test]
    fn gather_surfaces() {
        let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
        m.optical[0] = 1.5;
        m.color_rough = [0.8, 0.8, 0.8, 0.5];
        assert!(gather_surface(&m));
        m.lobes[2] = 1.;
        assert!(!gather_surface(&m), "glass");
        m.lobes[2] = 0.;
        m.lobes[0] = 1.;
        m.color_rough[3] = 0.1;
        assert!(!gather_surface(&m), "a near mirror");
        m.color_rough[3] = 0.3;
        assert!(gather_surface(&m), "a rough metal");
    }
}
