//! What a render records besides its light, for a multi-layer EXR (`name.layers.exr`) and the
//! light path expression view: per sample the direct light, the light of paths whose first
//! bounce was specular, the first hit's diffuse and specular albedo, and each light path
//! expression's share (`Record`); per pixel, from one ray through its centre, the normal, the
//! roughness, the depth and the object id (`Scene::aov_guide`). The same quantities as the GPU's
//! path tracer writes (pathtrace.hlsl `trace_main`, `write_guides`), so the layers compare one to
//! one.
//!
//! Light path expressions (`crate::lpe`) run the GPU's own DFA table over the path's events; what
//! is independent is where the events are taken: a light sample at a surface splits exactly into
//! the diffuse part of the BSDF and the rest, and a continued path draws one of the two from its
//! own random stream (the beauty's samples stay as they were) by the luminance shares at the
//! sampled direction, weighting what follows by share / probability per channel. Each layer is
//! unbiased, and a partition of all paths sums to the beauty in expectation.
use super::bsdf::{f0_of, material_at, spec_fresnel};
use super::sampling::Rng;
use super::scene::{Ray, Scene};
use crate::lighting::PackedMaterial;
use crate::lpe::{SLOTS, STATES, SYMBOLS};
use crate::scene::Settings;
use glam::{DVec3, DVec4, Vec2, Vec3, Vec4};

/// Path events (`lpe::EVENTS`): diffuse, glossy, specular, transmission, volume, light,
/// background; `NONE` adds no step before the light event.
pub(super) const D: u32 = 0;
pub(super) const G: u32 = 1;
pub(super) const S: u32 = 2;
pub(super) const T: u32 = 3;
pub(super) const V: u32 = 4;
pub(super) const L: u32 = 5;
pub(super) const B: u32 = 6;
pub(super) const NONE: u32 = 8;

/// The glossy event of a material: S below roughness 0.1, else G.
pub(super) fn glossy(m: &PackedMaterial) -> u32 {
    if m.color_rough[3] < 0.1 { S } else { G }
}

/// The scene's light path expressions as `lpe::table` compiles them.
pub(super) struct Lpe {
    table: Vec<u32>,
    /// Slots in use (trailing empty ones dropped).
    pub(super) slots: usize,
    /// Each material id's label class (0 none).
    classes: Vec<u32>,
}
impl Lpe {
    /// None when no expression is set.
    pub(super) fn new(s: &Settings) -> Option<Lpe> {
        let (table, slots, labels) = crate::lpe::table(&s.lpe);
        let classes = if labels.is_empty() {
            vec![]
        } else {
            crate::lpe::classes(&labels, &crate::lpe::material_names(s))
        };
        (slots > 0).then_some(Lpe {
            table,
            slots: slots as usize,
            classes,
        })
    }
    fn class(&self, material: u32) -> u32 {
        self.classes.get(material as usize).copied().unwrap_or(0)
    }
    /// The next state | accept << 8 from `state` on symbol (`class`, `event`).
    fn step(&self, slot: usize, state: u8, event: u32, class: u32) -> u32 {
        self.table[(slot * STATES + state as usize) * SYMBOLS + class as usize * 8 + event as usize]
    }
}

/// What one sample records besides its radiance. Off (`Record::off`), every method returns at
/// once, so a plain render pays nothing.
pub(super) struct Record<'a> {
    on: bool,
    dfa: Option<&'a Lpe>,
    /// Per slot, the DFA state after the path's events so far (`lpe::DEAD`: no match possible).
    state: [u8; SLOTS],
    /// The product of share / probability of the labels drawn so far.
    weight: Vec3,
    /// The class of the surface the path is at, and of the light the next light event is from.
    label: u32,
    light_label: u32,
    /// Each slot's light, summed over the samples recorded.
    pub(super) lpe: [Vec3; SLOTS],
    /// This sample's direct light: light samples from the first vertex, emission and background
    /// seen directly or after one bounce.
    pub(super) direct: Vec3,
    /// The first bounce was a delta event or a sample of a GGX or coat lobe.
    pub(super) first_spec: bool,
    /// The first hit's diffuse albedo, and whether it is the guide ray's surface (`coverage`).
    pub(super) diffuse_albedo: Vec4,
    pub(super) specular_albedo: Vec3,
    /// The guide ray's material (u32::MAX for none).
    guide: u32,
}
impl<'a> Record<'a> {
    pub(super) fn off() -> Self {
        Self::new(false, None, u32::MAX)
    }
    pub(super) fn new(on: bool, dfa: Option<&'a Lpe>, guide: u32) -> Self {
        Record {
            on,
            dfa,
            state: [0; SLOTS],
            weight: Vec3::ONE,
            label: 0,
            light_label: 0,
            lpe: [Vec3::ZERO; SLOTS],
            direct: Vec3::ZERO,
            first_spec: false,
            diffuse_albedo: Vec4::ZERO,
            specular_albedo: Vec3::ZERO,
            guide,
        }
    }
    /// A new sample of the same pixel: the LPE sums go on.
    pub(super) fn start(&mut self) {
        self.state = [0; SLOTS];
        self.weight = Vec3::ONE;
        self.label = 0;
        self.light_label = 0;
        self.direct = Vec3::ZERO;
        self.first_spec = false;
        self.diffuse_albedo = Vec4::ZERO;
        self.specular_albedo = Vec3::ZERO;
    }
    /// The path is at material `m`'s surface.
    pub(super) fn at(&mut self, material: u32) {
        if let Some(l) = self.dfa {
            self.label = l.class(material);
        }
    }
    /// The next light event comes from material `m` (None: the environment, which has no class).
    pub(super) fn light(&mut self, material: Option<u32>) {
        if let Some(l) = self.dfa {
            self.light_label = material.map_or(0, |m| l.class(m));
        }
    }
    /// The light event of the light sample just taken: L, or B for the environment.
    pub(super) fn light_event(material: Option<u32>) -> u32 {
        if material.is_some() { L } else { B }
    }
    /// Direct light: counted when `direct` (the GPU's bounce rules).
    pub(super) fn direct(&mut self, direct: bool, e: Vec3) {
        if self.on && direct {
            self.direct += e;
        }
    }
    /// The path takes `event`.
    pub(super) fn advance(&mut self, event: u32) {
        let Some(l) = self.dfa else {
            return;
        };
        let class = if event == V { 0 } else { self.label };
        for k in 0..SLOTS {
            let st = self.state[k];
            self.state[k] = if k < l.slots && st != crate::lpe::DEAD {
                l.step(k, st, event, class) as u8
            } else {
                crate::lpe::DEAD
            };
        }
    }
    /// Light `e` reaching the camera through the path so far, then `event` (`NONE`: none), then
    /// the light event `light` (L or B).
    pub(super) fn add(&mut self, event: u32, light: u32, e: Vec3) {
        let Some(l) = self.dfa else {
            return;
        };
        if !e.is_finite() {
            return;
        }
        let class = if event == V { 0 } else { self.label };
        let light_class = if light == B { 0 } else { self.light_label };
        for k in 0..l.slots {
            let mut st = self.state[k];
            if st == crate::lpe::DEAD {
                continue;
            }
            if event != NONE {
                st = l.step(k, st, event, class) as u8;
                if st == crate::lpe::DEAD {
                    continue;
                }
            }
            if l.step(k, st, light, light_class) >> 8 != 0 {
                self.lpe[k] += e * self.weight;
            }
        }
    }
    /// A light sample `e` at a surface whose BSDF value toward it was `f`, of which `diffuse`
    /// diffuse: D takes that share, the material's glossy event the rest.
    pub(super) fn add_split(&mut self, m: &PackedMaterial, light: u32, e: Vec3, f: Vec3, d: Vec3) {
        if self.dfa.is_none() {
            return;
        }
        let ed = e * d / f.max(Vec3::splat(1e-30));
        self.add(D, light, ed);
        self.add(glossy(m), light, e - ed);
    }
    /// The path continues by a BSDF sample of value `f`, `d` of it diffuse: one label drawn by
    /// the luminance shares with `u` (its own stream), the weight corrected per channel.
    pub(super) fn choose(&mut self, m: &PackedMaterial, f: Vec3, d: Vec3, u: f32) {
        if self.dfa.is_none() {
            return;
        }
        let share = (d / f.max(Vec3::splat(1e-30))).clamp(Vec3::ZERO, Vec3::ONE);
        let (lf, ld) = (luminance(f), luminance(d));
        let pd = if lf > 0. { (ld / lf).clamp(0., 1.) } else { 0. };
        if u < pd {
            self.weight *= share / pd;
            self.advance(D);
        } else {
            self.weight *= (Vec3::ONE - share) / (1. - pd);
            self.advance(glossy(m));
        }
    }
    /// The first hit: its albedos (RGB, before any spectral uplift) and whether it is the guide's.
    pub(super) fn first_hit(&mut self, material: u32, diffuse: Vec3, specular: Vec3) {
        if self.on {
            self.diffuse_albedo = diffuse.extend((material == self.guide) as u32 as f32);
            self.specular_albedo = specular;
        }
    }
    /// The first vertex scattered in a medium of `albedo`: no surface, no coverage.
    pub(super) fn first_medium(&mut self, albedo: Vec3) {
        if self.on {
            self.diffuse_albedo = albedo.extend(0.);
        }
    }
    /// The camera ray left the scene: covered when the guide's did too.
    pub(super) fn first_miss(&mut self) {
        if self.on {
            self.diffuse_albedo.w = (self.guide == u32::MAX) as u32 as f32;
        }
    }
    pub(super) fn on(&self) -> bool {
        self.on
    }
}
fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The diffuse albedo a first hit records: the base colour less metal and transmission.
pub(super) fn diffuse_albedo(m: &PackedMaterial) -> Vec3 {
    super::bsdf::rgb(m.color_rough) * (1. - m.lobes[0]) * (1. - m.lobes[2])
}
/// The specular albedo at cosine `c`: the specular Fresnel term.
pub(super) fn specular_albedo(m: &PackedMaterial, c: f32) -> Vec3 {
    spec_fresnel(m, c, f0_of(m))
}

/// A pixel's guide-ray layers (`write_guides`): the hit's shading normal, roughness, depth (along
/// the view axis for the perspective and tilt-shift projections, else the distance) and material.
#[derive(Clone, Copy)]
pub(super) struct GuideAov {
    pub(super) normal: Vec3,
    pub(super) roughness: f32,
    pub(super) depth: f32,
    /// The material id, u32::MAX for none.
    pub(super) material: u32,
}
impl Default for GuideAov {
    fn default() -> Self {
        GuideAov {
            normal: Vec3::ZERO,
            roughness: 1.,
            depth: 1e4,
            material: u32::MAX,
        }
    }
}
/// A pixel's sums over its samples.
#[derive(Clone, Copy, Default)]
pub(super) struct PixelAov {
    pub(super) direct: DVec3,
    pub(super) specular: DVec3,
    pub(super) diffuse_albedo: DVec4,
    pub(super) specular_albedo: DVec3,
    pub(super) lpe: [DVec3; SLOTS],
}
impl PixelAov {
    /// Adds one sample's record (its `radiance`, after the firefly clamp scaled it and its direct
    /// light by `scale`); the LPE sums are added by `take_lpe` once per pixel and pass.
    pub(super) fn add(&mut self, rec: &Record, radiance: Vec3, scale: f32) {
        self.direct += (rec.direct * scale).as_dvec3();
        if rec.first_spec {
            self.specular += radiance.as_dvec3();
        }
        self.diffuse_albedo += rec.diffuse_albedo.as_dvec4();
        self.specular_albedo += rec.specular_albedo.as_dvec3();
    }
    pub(super) fn take_lpe(&mut self, rec: &Record) {
        for (s, e) in self.lpe.iter_mut().zip(rec.lpe) {
            *s += e.as_dvec3();
        }
    }
}

impl Scene<'_> {
    /// The guide ray through pixel (x, y)'s centre, as `guide` takes it.
    pub(super) fn aov_guide(&self, x: usize, y: usize) -> GuideAov {
        let pixel = (y * self.size[0] as usize + x) as u64;
        let ray = self.camera_ray(
            Vec2::new(x as f32 + 0.5, y as f32 + 0.5),
            &mut super::spectral::Waves::rgb(),
            &mut Rng::new(pixel, u64::MAX),
        );
        let Some((o, d, _)) = ray else {
            return GuideAov::default();
        };
        let Some(h) = self.intersect(o, d, 1e4, &self.accept(Ray::Camera, o, d), false) else {
            return GuideAov::default();
        };
        let mut sf = self.surface(&h, o, d);
        let mut m = material_at(&self.materials[sf.material as usize], sf.p);
        self.apply_maps(&mut m, &h, &mut sf.n);
        let depth = match self.frame.projection {
            0 | 4 => (sf.p - self.eye).dot(self.frame.forward),
            _ => h.t,
        };
        GuideAov {
            normal: sf.n,
            roughness: m.color_rough[3],
            depth,
            material: sf.material,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lpe(exprs: &[&str]) -> Lpe {
        let mut s = Settings::default();
        s.lpe = exprs.iter().map(|e| e.to_string()).collect();
        Lpe::new(&s).unwrap()
    }
    #[test]
    fn events_reach_the_slots_that_accept_them() {
        let l = lpe(&["C D L", "C [GS] .* [LB]", "C .* B"]);
        let mut r = Record::new(true, Some(&l), u32::MAX);
        // A light sample at the first (diffuse) surface: D L.
        r.add(D, L, Vec3::ONE);
        assert_eq!(r.lpe[..3], [Vec3::ONE, Vec3::ZERO, Vec3::ZERO]);
        // Emission seen directly: just L, which no slot accepts.
        r.add(NONE, L, Vec3::ONE);
        assert_eq!(r.lpe[0], Vec3::ONE);
        // A specular bounce, then the background.
        r.advance(S);
        r.add(NONE, B, Vec3::splat(2.));
        assert_eq!(r.lpe[..3], [Vec3::ONE, Vec3::splat(2.), Vec3::splat(2.)]);
        // A new sample starts the DFA over; the sums stay.
        r.start();
        r.advance(D);
        r.add(NONE, B, Vec3::ONE);
        assert_eq!(r.lpe[..3], [Vec3::ONE, Vec3::splat(2.), Vec3::splat(3.)]);
        // Off, nothing is recorded.
        let mut off = Record::off();
        off.add(D, L, Vec3::ONE);
        assert_eq!(off.lpe[0], Vec3::ZERO);
    }
    #[test]
    fn a_split_light_sample_sums_to_its_value() {
        let l = lpe(&["C D L", "C G L", "C S L"]);
        let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
        m.color_rough[3] = 0.5;
        let mut r = Record::new(true, Some(&l), u32::MAX);
        let (f, d) = (Vec3::new(0.4, 0.3, 0.2), Vec3::new(0.1, 0.3, 0.0));
        r.add_split(&m, L, f * 2., f, d);
        assert!((r.lpe[0] - d * 2.).length() < 1e-6);
        assert!((r.lpe[0] + r.lpe[1] - f * 2.).length() < 1e-6);
        assert_eq!(r.lpe[2], Vec3::ZERO);
    }
    #[test]
    fn the_drawn_label_is_unbiased_per_channel() {
        // Over u uniform, E[weight after D] = share and E[weight after G] = 1 - share.
        let l = lpe(&["C D L", "C G L"]);
        let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
        m.color_rough[3] = 0.5;
        let (f, d) = (Vec3::new(0.5, 0.4, 0.3), Vec3::new(0.4, 0.1, 0.3));
        let n = 1000;
        let mut sum = [Vec3::ZERO; 2];
        for i in 0..n {
            let mut r = Record::new(true, Some(&l), u32::MAX);
            r.choose(&m, f, d, (i as f32 + 0.5) / n as f32);
            r.add(NONE, L, Vec3::ONE);
            sum[0] += r.lpe[0];
            sum[1] += r.lpe[1];
        }
        let share = d / f;
        assert!(
            (sum[0] / n as f32 - share).abs().max_element() < 2e-3,
            "{sum:?}"
        );
        assert!(
            (sum[1] / n as f32 - (Vec3::ONE - share))
                .abs()
                .max_element()
                < 2e-3
        );
    }
    #[test]
    fn labels_pick_out_their_material() {
        let mut s = Settings::default();
        s.lpe = vec!["C D'left wall' L".into()];
        let l = Lpe::new(&s).unwrap();
        let mut r = Record::new(true, Some(&l), u32::MAX);
        r.at(1);
        r.add(D, L, Vec3::ONE);
        r.at(0);
        r.add(D, L, Vec3::ONE);
        assert_eq!(r.lpe[0], Vec3::ONE);
    }
}
