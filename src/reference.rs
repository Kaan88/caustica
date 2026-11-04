//! CPU reference renderer (future goal: ground-truth screenshots). `--reference SPP --capture
//! out.exr` renders the scene on the CPU, with no GPU at all, into a linear EXR to compare with
//! the GPU's raw accumulation (`tools/reference_compare.py`).
//!
//! It is an independent implementation of the same model: the same triangles
//! (`geometry::geometry`), material records (`lighting::materials`), camera (`Settings::camera`),
//! pixel filters, BSDF (`eval_bsdf`'s isotropic core with the same energy compensation table,
//! parsed from layers.hlsli), dielectric branch and visibility rules (lights hidden from the
//! camera, shadows that skip clear glass and lights that cast none), but its own BVH, its own
//! light sampling (emissive triangles by power), its own random numbers (PCG per pixel and
//! sample) and its own MIS. So a disagreement points at the GPU's sampling, sample sequences,
//! acceleration structures or shader arithmetic, not at a different model. Scenes it cannot
//! render (`unsupported`) are refused with a list of what is missing.
use crate::lighting::PackedMaterial;
use crate::scene::{Settings, Vertex};
use anyhow::Result;
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// What a scene uses that the reference does not render; empty when it can.
pub fn unsupported(s: &Settings) -> Vec<String> {
    let mut out = vec![];
    let mut need = |c: bool, what: &str| {
        if c {
            out.push(what.to_string());
        }
    };
    need(!s.hdri.is_empty(), "an HDRI environment");
    need(s.physical_sky.on, "the physical sky");
    need(s.fog.density > 0., "fog");
    need(s.shadow_catcher, "the shadow catcher");
    need(s.spectral, "spectral rendering");
    need(s.firefly > 0., "the firefly clamp (biased; set firefly=0)");
    need(
        s.integrator != 0,
        "BDPT and photon mapping (the reference is a path tracer)",
    );
    need(
        s.projection != 0,
        "a camera projection other than perspective",
    );
    need(s.aperture > 0., "depth of field");
    need(s.lens_system != 0, "a real lens system");
    need(
        s.objects.iter().any(|o| o.visible && !o.sdf.is_none()),
        "procedural (SDF) objects",
    );
    need(
        s.lights.iter().any(|l| {
            l.enabled
                && matches!(
                    l.kind,
                    crate::scene::LightKind::Sphere | crate::scene::LightKind::Sun
                )
        }),
        "sphere and sun lights",
    );
    need(
        s.objects.iter().any(|o| {
            o.visible && (!o.material.texture.is_empty() || !o.mesh.is_empty() && mesh_has_maps(o))
        }),
        "texture maps",
    );
    for (what, bit) in [
        ("portals", 4u32),
        ("alpha cutouts", 8),
        ("beam splitters", 4096),
        ("IES profiles", 16384),
        ("emissive maps", 64),
    ] {
        need(
            crate::lighting::materials(s)
                .iter()
                .any(|m| m.lobes[3] as u32 & bit != 0),
            what,
        );
    }
    let mats = crate::lighting::materials(s);
    need(mats.iter().any(|m| m.layers[0] > 0.), "clear coats");
    need(mats.iter().any(|m| m.layers[3] > 0.), "anisotropy");
    need(
        mats.iter().any(|m| m.sheen[..3].iter().any(|c| *c > 0.)),
        "sheen",
    );
    need(mats.iter().any(|m| m.surface_scale[2] > 0.), "thin films");
    need(
        mats.iter().any(|m| m.optical[3] < 0.),
        "diffraction gratings",
    );
    need(
        mats.iter().any(|m| m.medium[3] > 0.),
        "participating media in objects",
    );
    // Measured spectra and blackbodies render as their RGB colour outside spectral mode, as on
    // the GPU.
    out
}
/// Imported meshes may bring their own texture maps (the glTF's), which the reference skips.
fn mesh_has_maps(o: &crate::scene::Object) -> bool {
    crate::mesh::load(&o.mesh).is_ok_and(|m| m.parts.iter().any(|p| !p.maps.is_empty()))
}

// ---------------------------------------------------------------------------------------------
// The GGX directional albedo (layers.hlsli GGX_ALBEDO, 32 × 32 over cos θ and roughness).

fn ggx_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let src = include_str!("../shaders/layers.hlsli");
        let start = src
            .find("GGX_ALBEDO[256]")
            .expect("GGX_ALBEDO in layers.hlsli");
        let body = &src[start..];
        let body = &body[body.find('{').unwrap() + 1..body.find("};").unwrap()];
        let body = body.replace("float4", " ");
        let v: Vec<f32> = body
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
            .filter(|t| !t.is_empty() && t.chars().any(|c| c.is_ascii_digit()))
            .filter_map(|t| t.trim_end_matches('.').parse().ok())
            .collect();
        assert_eq!(v.len(), 1024, "GGX_ALBEDO has 1024 entries");
        v
    })
}
fn ggx_albedo(nv: f32, roughness: f32) -> f32 {
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

fn rgb(v: [f32; 4]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
fn fresnel(c: f32, f0: Vec3) -> Vec3 {
    f0 + (Vec3::ONE - f0) * (1. - c.clamp(0., 1.)).powi(5)
}
/// Exact unpolarised Fresnel reflectance, eta = n1 / n2 (`fresnel_dielectric`).
fn fresnel_dielectric(cosi: f32, eta: f32) -> f32 {
    let s2 = eta * eta * (1. - cosi * cosi).max(0.);
    if s2 >= 1. {
        return 1.;
    }
    let ct = (1. - s2).sqrt();
    let rs = (eta * cosi - ct) / (eta * cosi + ct);
    let rp = (cosi - eta * ct) / (cosi + eta * ct);
    0.5 * (rs * rs + rp * rp)
}
fn ggx_d(nh: f32, a: f32) -> f32 {
    let d = nh * nh * (a * a - 1.) + 1.;
    a * a / (PI * d * d)
}
fn g1(nv: f32, a: f32) -> f32 {
    2. * nv / (nv + (a * a + (1. - a * a) * nv * nv).sqrt()).max(1e-6)
}
fn f0_of(m: &PackedMaterial) -> Vec3 {
    let f = ((m.optical[0] - 1.) / (m.optical[0] + 1.)).powi(2);
    Vec3::splat(f)
        .lerp(Vec3::ONE, m.lobes[1])
        .lerp(rgb(m.color_rough), m.lobes[0])
}
fn spec_probability(m: &PackedMaterial) -> f32 {
    (0.25 + 0.5 * m.lobes[0] + 0.25 * m.lobes[1]).clamp(0.1, 0.95)
}
/// `eval_bsdf` without layers, anisotropy, gratings or films: the value and the pdf of the
/// sampling below.
fn eval_bsdf(m: &PackedMaterial, n: Vec3, v: Vec3, l: Vec3) -> (Vec3, f32) {
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
fn material_at(m: &PackedMaterial, p: Vec3) -> PackedMaterial {
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
fn flags(m: &PackedMaterial) -> u32 {
    m.lobes[3] as u32
}
/// The spot falloff (`spot`, flag 32): smoothstep between the cone's cosines.
fn spot(m: &PackedMaterial, c: f32) -> f32 {
    if flags(m) & 32 == 0 {
        return 1.;
    }
    let (e0, e1) = (m.secondary[0], m.secondary[1]);
    let t = ((c - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
/// Emitted radiance leaving a surface toward a viewer at cosine `c` on side `front`.
fn emitted(m: &PackedMaterial, front: bool, c: f32) -> Vec3 {
    if m.optical[1] <= 0. || (m.optical[3] > 0.5 && !front) {
        return Vec3::ZERO;
    }
    rgb(m.color_rough) * m.optical[1] * spot(m, c)
}

// ---------------------------------------------------------------------------------------------
// Geometry: a binned-SAH BVH over the scene's triangles.

struct Tri {
    p: [Vec3; 3],
    n: [Vec3; 3],
    material: u32,
}
#[derive(Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle and count (count > 0); inner: the right child's index (left = +1).
    start: u32,
    count: u32,
}
struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
}
#[derive(Clone, Copy)]
struct Hit {
    t: f32,
    tri: usize,
    b: Vec2,
}
/// What a ray may hit: camera rays skip lights hidden from the camera; shadow rays skip lights
/// that cast no shadow and, unless glass casts shadows, clear glass.
#[derive(Clone, Copy, PartialEq)]
enum Ray {
    Camera,
    Bounce,
    Shadow,
}

impl Bvh {
    fn build(tris: Vec<Tri>) -> Self {
        let mut bvh = Self {
            nodes: vec![],
            tris: vec![],
        };
        let centers: Vec<Vec3> = tris
            .iter()
            .map(|t| (t.p[0] + t.p[1] + t.p[2]) / 3.)
            .collect();
        let mut order: Vec<usize> = (0..tris.len()).collect();
        if !order.is_empty() {
            bvh.split(&tris, &centers, &mut order[..], 0, 0);
        }
        let mut slots: Vec<Option<Tri>> = tris.into_iter().map(Some).collect();
        bvh.tris = order.iter().map(|i| slots[*i].take().unwrap()).collect();
        bvh
    }
    fn bounds(tris: &[Tri], idx: &[usize]) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in idx {
            for p in tris[*i].p {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        (lo, hi)
    }
    fn split(
        &mut self,
        tris: &[Tri],
        centers: &[Vec3],
        idx: &mut [usize],
        start: usize,
        depth: usize,
    ) -> usize {
        let (lo, hi) = Self::bounds(tris, idx);
        let me = self.nodes.len();
        self.nodes.push(Node {
            min: lo,
            max: hi,
            start: start as u32,
            count: idx.len() as u32,
        });
        if idx.len() <= 4 {
            return me;
        }
        let area = |lo: Vec3, hi: Vec3| {
            let e = (hi - lo).max(Vec3::ZERO);
            e.x * e.y + e.y * e.z + e.z * e.x
        };
        // Binned SAH over the centroids' extent, 16 bins per axis.
        const BINS: usize = 16;
        let (clo, chi) = idx.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(a, b), i| (a.min(centers[*i]), b.max(centers[*i])),
        );
        let mut best = (f32::MAX, 0usize, 0f32);
        for axis in 0..3 {
            let extent = chi[axis] - clo[axis];
            if extent <= 1e-9 {
                continue;
            }
            let mut bins = [(0usize, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); BINS];
            for i in idx.iter() {
                let b = (((centers[*i][axis] - clo[axis]) / extent * BINS as f32) as usize)
                    .min(BINS - 1);
                let e = &mut bins[b];
                e.0 += 1;
                for p in tris[*i].p {
                    e.1 = e.1.min(p);
                    e.2 = e.2.max(p);
                }
            }
            for cut in 1..BINS {
                let merge = |r: &[(usize, Vec3, Vec3)]| {
                    r.iter()
                        .fold((0, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |a, b| {
                            (a.0 + b.0, a.1.min(b.1), a.2.max(b.2))
                        })
                };
                let (l, r) = (merge(&bins[..cut]), merge(&bins[cut..]));
                if l.0 == 0 || r.0 == 0 {
                    continue;
                }
                let cost = l.0 as f32 * area(l.1, l.2) + r.0 as f32 * area(r.1, r.2);
                if cost < best.0 {
                    best = (cost, axis, clo[axis] + extent * cut as f32 / BINS as f32);
                }
            }
        }
        // Past depth 40 the list is halved (no SAH), so the tree stays inside the traversal stack.
        let mid = if best.0 < f32::MAX && depth < 40 {
            let (axis, at) = (best.1, best.2);
            let mut i = 0;
            for j in 0..idx.len() {
                if centers[idx[j]][axis] < at {
                    idx.swap(i, j);
                    i += 1;
                }
            }
            i
        } else {
            0
        };
        // No useful cut (all centroids together): split in the middle of the list.
        let mid = if mid == 0 || mid == idx.len() {
            idx.len() / 2
        } else {
            mid
        };
        let (left, right) = idx.split_at_mut(mid);
        self.nodes[me].count = 0;
        self.split(tris, centers, left, start, depth + 1);
        let r = self.split(tris, centers, right, start + mid, depth + 1);
        self.nodes[me].start = r as u32;
        me
    }
    fn slab(n: &Node, o: Vec3, inv: Vec3, tmax: f32) -> Option<f32> {
        let t0 = (n.min - o) * inv;
        let t1 = (n.max - o) * inv;
        let near = t0.min(t1).max_element().max(0.);
        let far = t0.max(t1).min_element().min(tmax);
        (near <= far).then_some(near)
    }
    /// Closest accepted hit in (1e-4, tmax): Möller–Trumbore, both sides.
    fn intersect(
        &self,
        o: Vec3,
        d: Vec3,
        tmax: f32,
        accept: &dyn Fn(u32) -> bool,
        any: bool,
    ) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = d.recip();
        let mut best: Option<Hit> = None;
        let mut limit = tmax;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if Self::slab(node, o, inv, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for i in node.start as usize..(node.start + node.count) as usize {
                    let t = &self.tris[i];
                    let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
                    let pv = d.cross(e2);
                    let det = e1.dot(pv);
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    let id = 1. / det;
                    let tv = o - t.p[0];
                    let u = tv.dot(pv) * id;
                    if !(0. ..=1.).contains(&u) {
                        continue;
                    }
                    let qv = tv.cross(e1);
                    let v = d.dot(qv) * id;
                    if v < 0. || u + v > 1. {
                        continue;
                    }
                    let tt = e2.dot(qv) * id;
                    if tt > 1e-4 && tt < limit && accept(t.material) {
                        limit = tt;
                        best = Some(Hit {
                            t: tt,
                            tri: i,
                            b: Vec2::new(u, v),
                        });
                        if any {
                            return best;
                        }
                    }
                }
            } else {
                let here = stack[top] as usize;
                let (l, r) = (here + 1, node.start as usize);
                // Nearer child last, so it pops first.
                let (dl, dr) = (
                    Self::slab(&self.nodes[l], o, inv, limit),
                    Self::slab(&self.nodes[r], o, inv, limit),
                );
                let mut push = |c: usize| {
                    if top < stack.len() {
                        stack[top] = c as u32;
                        top += 1;
                    }
                };
                match (dl, dr) {
                    (Some(a), Some(b)) if a < b => {
                        push(r);
                        push(l);
                    }
                    (Some(_), Some(_)) => {
                        push(l);
                        push(r);
                    }
                    (Some(_), None) => push(l),
                    (None, Some(_)) => push(r),
                    _ => {}
                }
            }
        }
        best
    }
}

// ---------------------------------------------------------------------------------------------
// Sampling.

/// PCG32 (O'Neill), seeded per pixel and sample.
struct Rng(u64);
impl Rng {
    fn new(pixel: u64, sample: u64) -> Self {
        // splitmix64 of both, so neighbouring pixels and samples start uncorrelated.
        let mix = |mut z: u64| {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut r = Self(mix(mix(pixel) ^ sample.wrapping_mul(0xd1b5_4a32_d192_ed03)));
        r.next();
        r
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        self.0 = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let x = (((old >> 18) ^ old) >> 27) as u32;
        x.rotate_right((old >> 59) as u32)
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 8) as f32 * (1. / 16_777_216.)
    }
}
fn basis(n: Vec3, l: Vec3) -> Vec3 {
    let (t, b) = n.any_orthonormal_pair();
    t * l.x + b * l.y + n * l.z
}
fn cosine_sample(n: Vec3, r: &mut Rng) -> Vec3 {
    let (q, a) = (r.f().sqrt(), 2. * PI * r.f());
    basis(
        n,
        Vec3::new(q * a.cos(), q * a.sin(), (1. - q * q).max(0.).sqrt()),
    )
}
fn reflect(d: Vec3, n: Vec3) -> Vec3 {
    d - 2. * d.dot(n) * n
}
fn refract(d: Vec3, n: Vec3, eta: f32) -> Vec3 {
    let c = -d.dot(n);
    let k = 1. - eta * eta * (1. - c * c);
    if k < 0. {
        return Vec3::ZERO;
    }
    eta * d + (eta * c - k.sqrt()) * n
}
/// GGX normal-distribution sample (`ggx_sample`), reflected.
fn ggx_sample(n: Vec3, d: Vec3, a: f32, r: &mut Rng) -> Vec3 {
    let (u, phi) = (r.f(), 2. * PI * r.f());
    let c = ((1. - u) / (1. + (a * a - 1.) * u)).sqrt();
    let s = (1. - c * c).max(0.).sqrt();
    reflect(d, basis(n, Vec3::new(s * phi.cos(), s * phi.sin(), c)))
}
fn erfinv(x: f32) -> f32 {
    let mut w = -((1. - x) * (1. + x)).max(1e-30).ln();
    let p = if w < 5. {
        w -= 2.5;
        [
            2.810_226_4e-8,
            3.432_739_4e-7,
            -3.523_387_7e-6,
            -4.391_506_5e-6,
            0.000_218_580_87,
            -0.001_253_725,
            -0.004_177_681_6,
            0.246_640_73,
            1.501_409_4,
        ]
        .iter()
        .fold(0., |p, c| c + p * w)
    } else {
        w = w.sqrt() - 3.;
        [
            -0.000_200_214_26,
            0.000_100_950_56,
            0.001_349_343_2,
            -0.003_673_428_4,
            0.005_739_507_7,
            -0.007_622_461,
            0.009_438_870_5,
            1.001_674,
            2.832_976_8,
        ]
        .iter()
        .fold(0., |p, c| c + p * w)
    };
    p * x
}
fn blackman_harris_inverse(u: f32) -> f32 {
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..16 {
        let t = 0.5 * (lo + hi);
        let c = (0.35875 * t - 0.48829 * (2. * PI * t).sin() / (2. * PI)
            + 0.14128 * (4. * PI * t).sin() / (4. * PI)
            - 0.01168 * (6. * PI * t).sin() / (6. * PI))
            / 0.35875;
        if c < u {
            lo = t;
        } else {
            hi = t;
        }
    }
    0.5 * (lo + hi)
}
/// The pixel filter's offset for uniform u (`filter_offset`).
fn filter_offset(s: &Settings, u: Vec2) -> Vec2 {
    let w = s.filter_width;
    match s.pixel_filter {
        1 => (Vec2::new(erfinv(2. * u.x - 1.), erfinv(2. * u.y - 1.))
            * (w / 6. * std::f32::consts::SQRT_2))
            .clamp(Vec2::splat(-0.5 * w), Vec2::splat(0.5 * w)),
        2 => (Vec2::new(blackman_harris_inverse(u.x), blackman_harris_inverse(u.y)) - 0.5) * w,
        _ => (u - 0.5) * w,
    }
}

// ---------------------------------------------------------------------------------------------
// The integrator.

/// The nested-dielectric stack (`medium_top`, `medium_push`, `medium_remove`): entries are
/// (priority, material), at most 4; the current medium is the highest priority, the later
/// entry among equals.
fn top(media: &[(u32, u32)]) -> Option<&(u32, u32)> {
    // max_by_key keeps the last of equal maxima.
    media.iter().max_by_key(|e| e.0)
}
fn push(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    // A full stack drops the push, as on the GPU.
    if media.len() < 4 {
        media.push(e);
    }
}
fn remove(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    if let Some(i) = media.iter().rposition(|x| *x == e) {
        media.remove(i);
    }
}

struct Scene<'a> {
    s: &'a Settings,
    bvh: Bvh,
    materials: Vec<PackedMaterial>,
    /// Emissive triangles (indices into the BVH's order), their power CDF and total.
    lights: Vec<usize>,
    cdf: Vec<f32>,
    eye: Vec3,
    inverse_vp: Mat4,
    /// The dielectric the camera is inside (`geometry::camera_medium`), as the GPU seeds it.
    camera_media: Vec<(u32, u32)>,
    size: [u32; 2],
}
struct Surface {
    p: Vec3,
    n: Vec3,
    front: bool,
    material: u32,
}
impl Scene<'_> {
    fn accept(&self, kind: Ray) -> impl Fn(u32) -> bool + '_ {
        move |id| {
            let m = &self.materials[id as usize];
            match kind {
                Ray::Camera => flags(m) & 2 == 0,
                Ray::Bounce => true,
                Ray::Shadow => flags(m) & 8192 == 0 && (self.s.glass_shadow || m.lobes[2] < 0.5),
            }
        }
    }
    fn surface(&self, h: &Hit, d: Vec3) -> Surface {
        let t = &self.bvh.tris[h.tri];
        let (u, v) = (h.b.x, h.b.y);
        let mut n = (t.n[0] * (1. - u - v) + t.n[1] * u + t.n[2] * v).normalize_or_zero();
        if n == Vec3::ZERO {
            n = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize();
        }
        let front = d.dot(n) < 0.;
        if !front {
            n = -n;
        }
        Surface {
            p: t.p[0] * (1. - u - v) + t.p[1] * u + t.p[2] * v,
            n,
            front,
            material: t.material,
        }
    }
    /// Beer-Lambert through `t` of the innermost dielectric: its colour is the transmittance
    /// per unit (`medium_absorption`).
    fn transmittance(&self, media: &[(u32, u32)], t: f32) -> Vec3 {
        let Some(top) = top(media) else {
            return Vec3::ONE;
        };
        let c = rgb(self.materials[top.1 as usize].color_rough).max(Vec3::splat(0.001));
        c.powf(t)
    }
    fn area(t: &Tri) -> f32 {
        (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5
    }
    /// The solid-angle pdf of reaching emitter triangle `tri` at `at` from `from`.
    fn light_pdf(&self, tri: usize, from: Vec3, at: Vec3) -> f32 {
        let Ok(k) = self.lights.binary_search(&tri) else {
            return 0.;
        };
        let p = self.cdf[k] - if k > 0 { self.cdf[k - 1] } else { 0. };
        let t = &self.bvh.tris[tri];
        let ng = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize();
        let d = at - from;
        let c = ng.dot(d.normalize()).abs();
        p * d.length_squared() / (Self::area(t) * c).max(1e-12)
    }
    /// One emitter point: direction, radiance arriving, solid-angle pdf, distance.
    fn sample_light(&self, x: Vec3, r: &mut Rng) -> Option<(Vec3, Vec3, f32, f32)> {
        if self.lights.is_empty() {
            return None;
        }
        let u = r.f();
        let k = self
            .cdf
            .partition_point(|c| *c <= u)
            .min(self.lights.len() - 1);
        let tri = self.lights[k];
        let t = &self.bvh.tris[tri];
        let (mut a, mut b) = (r.f(), r.f());
        if a + b > 1. {
            (a, b) = (1. - a, 1. - b);
        }
        let p = t.p[0] * (1. - a - b) + t.p[1] * a + t.p[2] * b;
        let d = p - x;
        let dist = d.length();
        if dist < 1e-6 {
            return None;
        }
        let ld = d / dist;
        let mut n = (t.n[0] * (1. - a - b) + t.n[1] * a + t.n[2] * b).normalize_or_zero();
        if n == Vec3::ZERO {
            n = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize();
        }
        let front = ld.dot(n) < 0.;
        let m = material_at(&self.materials[t.material as usize], p);
        // The spot's cosine is signed against the light's normal (`spot(lm, dot(ln, -ld))`).
        let le = emitted(&m, front, (-ld).dot(n));
        let pdf = self.light_pdf(tri, x, p);
        (pdf > 0. && le.max_element() > 0.).then_some((ld, le, pdf, dist))
    }
    fn radiance(&self, mut origin: Vec3, mut direction: Vec3, r: &mut Rng) -> Vec3 {
        let s = self.s;
        let mut throughput = Vec3::ONE;
        let mut radiance = Vec3::ZERO;
        let mut previous_pdf = 0f32;
        let mut previous_delta = true;
        let mut previous_position = origin;
        // The dielectrics the path is inside: (priority, material), innermost last.
        let mut media: Vec<(u32, u32)> = self.camera_media.clone();
        let mut false_hits = 0;
        let mut bounce = 0;
        while bounce < s.bounces {
            let kind = if bounce == 0 {
                Ray::Camera
            } else {
                Ray::Bounce
            };
            // False hits on nested dielectrics (`medium_top`): the ray goes on unbent.
            let hit = loop {
                let Some(h) = self
                    .bvh
                    .intersect(origin, direction, 1e4, &self.accept(kind), false)
                else {
                    break None;
                };
                let t = &self.bvh.tris[h.tri];
                let m = &self.materials[t.material as usize];
                if false_hits >= 32 || m.lobes[2] <= 0. {
                    break Some(h);
                }
                let sf = self.surface(&h, direction);
                let entry = ((flags(m) >> 8) & 7, t.material);
                let skip = if sf.front {
                    top(&media).is_some_and(|top| entry.0 < top.0)
                } else {
                    media.contains(&entry) && top(&media) != Some(&entry)
                };
                if !skip {
                    break Some(h);
                }
                // The segment up to the false hit lies in the medium before it.
                throughput *= self.transmittance(&media, h.t);
                if sf.front {
                    push(&mut media, entry);
                } else {
                    remove(&mut media, entry);
                }
                false_hits += 1;
                origin = sf.p + direction * 0.0003;
            };
            let Some(h) = hit else {
                // The sky gradient over the background colour (no HDRI).
                let up = (direction.y * 0.5 + 0.5).clamp(0., 1.);
                let sky = Vec3::from(s.background)
                    + s.sky * Vec3::new(0.8, 0.87, 1.0).lerp(Vec3::new(0.16, 0.35, 0.72), up);
                radiance += throughput * sky;
                break;
            };
            throughput *= self.transmittance(&media, h.t);
            let sf = self.surface(&h, direction);
            let m = material_at(&self.materials[sf.material as usize], sf.p);
            let le = emitted(&m, sf.front, (-direction).dot(sf.n));
            if le.max_element() > 0. {
                let weight = if bounce > 0 && !previous_delta {
                    let pl = self.light_pdf(h.tri, previous_position, sf.p);
                    previous_pdf / (previous_pdf + pl)
                } else {
                    1.
                };
                radiance += throughput * le * weight;
            }
            // Unlit: emission only.
            if flags(&m) & 1 != 0 {
                break;
            }
            let v = -direction;
            // Light sampling.
            if let Some((ld, le, pl, dist)) = self.sample_light(sf.p, r) {
                let nl = sf.n.dot(ld);
                if nl > 0. {
                    let (f, pdf) = eval_bsdf(&m, sf.n, v, ld);
                    if f.max_element() > 0.
                        && self
                            .bvh
                            .intersect(
                                sf.p + sf.n * 0.0003,
                                ld,
                                // Short of the light by more than the origin's offset, which
                                // moves toward it by up to 0.0003.
                                dist - 2e-3,
                                &self.accept(Ray::Shadow),
                                true,
                            )
                            .is_none()
                    {
                        radiance += throughput * f * nl * le / (pl + pdf).max(1e-8);
                    }
                }
            }
            // The next direction.
            let next;
            let mut delta = false;
            let mut bsdf_pdf = 0.;
            if r.f() < m.lobes[2] {
                let entry = ((flags(&m) >> 8) & 7, sf.material);
                let ior = |e: Option<&(u32, u32)>| {
                    e.map_or(1., |e| self.materials[e.1 as usize].optical[0])
                };
                let mut after = media.clone();
                if sf.front {
                    push(&mut after, entry);
                } else {
                    remove(&mut after, entry);
                }
                let (n1, n2) = if sf.front {
                    (ior(top(&media)), m.optical[0])
                } else {
                    (m.optical[0], ior(top(&after)))
                };
                let eta = n1 / n2;
                let ci = v.dot(sf.n).clamp(0., 1.);
                let fr = fresnel_dielectric(ci, eta);
                let reflected = r.f() < fr;
                let mut d = if reflected {
                    reflect(direction, sf.n)
                } else {
                    refract(direction, sf.n, eta)
                };
                if !reflected {
                    media = after;
                    throughput *= eta * eta;
                }
                let rough = m.color_rough[3];
                if rough > 0.001 {
                    let pert = cosine_sample(if reflected { sf.n } else { -sf.n }, r);
                    d = d.lerp(pert, rough * rough).normalize();
                }
                next = d;
                delta = true;
            } else {
                let ps = spec_probability(&m);
                let d = if r.f() < ps {
                    let rough = m.color_rough[3];
                    ggx_sample(sf.n, direction, (rough * rough).max(0.001), r)
                } else {
                    cosine_sample(sf.n, r)
                };
                if sf.n.dot(d) <= 0. {
                    break;
                }
                let (f, pdf) = eval_bsdf(&m, sf.n, v, d);
                throughput *= f * sf.n.dot(d).abs() / pdf.max(1e-7);
                bsdf_pdf = pdf;
                next = d;
            }
            previous_pdf = bsdf_pdf;
            previous_delta = delta;
            previous_position = sf.p;
            direction = next.normalize();
            origin = sf.p
                + sf.n
                    * if direction.dot(sf.n) > 0. {
                        0.0003
                    } else {
                        -0.0003
                    };
            if bounce >= 3 {
                let survival = throughput.max_element().clamp(0.05, 0.95);
                if r.f() > survival {
                    break;
                }
                throughput /= survival;
            }
            bounce += 1;
        }
        if radiance.is_finite() {
            radiance.max(Vec3::ZERO)
        } else {
            Vec3::ZERO
        }
    }
    fn camera_direction(&self, pixel: Vec2) -> Vec3 {
        let ndc = pixel / Vec2::new(self.size[0] as f32, self.size[1] as f32) * 2. - 1.;
        let p = self.inverse_vp * Vec4::new(ndc.x, -ndc.y, 1., 1.);
        (p.truncate() / p.w - self.eye).normalize()
    }
}

impl<'a> Scene<'a> {
    fn new(s: &'a Settings, size: [u32; 2]) -> Self {
        let materials = crate::lighting::materials(s);
        let vertices: Vec<Vertex> = crate::geometry::geometry(s);
        let tris: Vec<Tri> = vertices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| Tri {
                p: [0, 1, 2].map(|i| Vec3::from(v[i].position)),
                n: [0, 1, 2].map(|i| Vec3::from(v[i].normal)),
                material: v[0].material,
            })
            .filter(|t| (t.material as usize) < materials.len())
            .collect();
        let bvh = Bvh::build(tris);
        // Emissive triangles by power: area × luminance of the emission (pattern-free colour).
        let mut lights = vec![];
        let mut cdf = vec![];
        let mut total = 0.;
        for (i, t) in bvh.tris.iter().enumerate() {
            let m = &materials[t.material as usize];
            if m.optical[1] > 0. {
                let c = rgb(m.color_rough) * m.optical[1];
                let w = Scene::area(t) * (0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z).max(1e-6);
                total += w;
                lights.push(i);
                cdf.push(total);
            }
        }
        for c in &mut cdf {
            *c /= total;
        }
        let (view, proj, eye) = s.camera(size[0] as f32 / size[1] as f32);
        Scene {
            s,
            bvh,
            materials,
            lights,
            cdf,
            eye,
            inverse_vp: (proj * view).inverse(),
            camera_media: match crate::geometry::camera_medium(s, eye) {
                0 => vec![],
                e => vec![(e >> 13, (e & 0x1fff) - 1)],
            },
            size,
        }
    }
}

/// Renders `spp` samples per pixel on every core: linear RGBA, rows top first.
pub fn render(
    s: &Settings,
    size: [u32; 2],
    spp: u32,
    progress: &(dyn Fn(f32) + Sync),
) -> Result<Vec<f32>> {
    let mut p = Progressive::new(s, size)?;
    p.pass(spp, cores(), &AtomicBool::new(false), progress);
    Ok(p.image())
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// A render that grows by passes: per-pixel sums of samples `0..samples`, each seeded by pixel
/// and sample index, so any split into passes gives the same bits as one pass.
pub struct Progressive<'a> {
    scene: Scene<'a>,
    sums: Vec<Vec3>,
    pub samples: u32,
}
impl<'a> Progressive<'a> {
    pub fn new(s: &'a Settings, size: [u32; 2]) -> Result<Self> {
        let missing = unsupported(s);
        anyhow::ensure!(
            missing.is_empty(),
            "The CPU reference can't render this scene: {}",
            missing.join(", ")
        );
        Ok(Progressive {
            scene: Scene::new(s, size),
            sums: vec![Vec3::ZERO; size[0] as usize * size[1] as usize],
            samples: 0,
        })
    }
    /// Adds `spp` samples to every pixel on `threads` threads. Returns false when `cancel` stopped
    /// it partway; the sums are then mixed and the render should be dropped.
    pub fn pass(
        &mut self,
        spp: u32,
        threads: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(f32) + Sync),
    ) -> bool {
        let scene = &self.scene;
        let s = scene.s;
        let (w, h) = (scene.size[0] as usize, scene.size[1] as usize);
        let first = self.samples;
        let next_row = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let rows: Vec<std::sync::Mutex<&mut [Vec3]>> =
            self.sums.chunks_mut(w).map(std::sync::Mutex::new).collect();
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let y = next_row.fetch_add(1, Ordering::Relaxed);
                        if y >= h || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let mut row = rows[y].lock().unwrap_or_else(|e| e.into_inner());
                        for (x, sum) in row.iter_mut().enumerate() {
                            for k in first..first + spp {
                                let mut r = Rng::new((y * w + x) as u64, k as u64);
                                let off = filter_offset(s, Vec2::new(r.f(), r.f()));
                                let pixel = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) + off;
                                let d = scene.camera_direction(pixel);
                                *sum += scene.radiance(scene.eye, d, &mut r);
                            }
                        }
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if n.is_multiple_of((h / 20).max(1)) {
                            progress(n as f32 / h as f32);
                        }
                    }
                });
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        self.samples += spp;
        true
    }
    /// The mean so far: linear RGBA, rows top first.
    pub fn image(&self) -> Vec<f32> {
        let n = self.samples.max(1) as f32;
        self.sums
            .iter()
            .flat_map(|c| {
                let c = *c / n;
                [c.x, c.y, c.z, 1.]
            })
            .collect()
    }
}

/// The CPU reference rendering in the background for the viewport (Look → Compare). It renders
/// in passes of about half a second and publishes the mean after each; dropping it cancels the
/// render (the thread finishes its current row and exits on its own).
pub struct Live {
    /// The transport key and output size it renders; a change means a restart.
    pub key: String,
    pub size: [u32; 2],
    /// A different key and size waiting to settle (camera or slider drags) before a restart.
    pub pending: Option<(String, [u32; 2], std::time::Instant)>,
    shared: std::sync::Arc<Shared>,
}
#[derive(Default)]
struct Shared {
    cancel: AtomicBool,
    state: std::sync::Mutex<LiveState>,
}
#[derive(Default, Clone)]
pub struct LiveState {
    /// The latest mean, and whether it has not been shown yet.
    pub image: Option<std::sync::Arc<Vec<f32>>>,
    pub fresh: bool,
    pub samples: u32,
    pub cap: u32,
    pub seconds: f32,
    /// Why it can't render (unsupported features), shown instead of the image.
    pub error: Option<String>,
}
impl Live {
    pub fn start(s: Settings, size: [u32; 2], key: String) -> Live {
        let shared = std::sync::Arc::new(Shared::default());
        let cap = s.max_samples.max(1);
        lock(&shared.state).cap = cap;
        let job = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("cpu-reference".into())
            .spawn(move || {
                let start = std::time::Instant::now();
                let mut p = match Progressive::new(&s, size) {
                    Ok(p) => p,
                    Err(e) => {
                        lock(&job.state).error = Some(format!("{e:#}"));
                        return;
                    }
                };
                // One core stays free for the UI and the GPU's submissions.
                let threads = cores().saturating_sub(1).max(1);
                let mut spp = 1;
                while p.samples < cap && !job.cancel.load(Ordering::Relaxed) {
                    let t = std::time::Instant::now();
                    let n = spp.min(cap - p.samples);
                    if !p.pass(n, threads, &job.cancel, &|_| {}) {
                        return;
                    }
                    let per_sample = t.elapsed().as_secs_f32() / n as f32;
                    spp = ((0.5 / per_sample.max(1e-6)) as u32).clamp(1, 64);
                    let image = std::sync::Arc::new(p.image());
                    let mut st = lock(&job.state);
                    st.image = Some(image);
                    st.fresh = true;
                    st.samples = p.samples;
                    st.seconds = start.elapsed().as_secs_f32();
                }
            });
        if let Err(e) = spawned {
            lock(&shared.state).error = Some(format!("Starting the CPU render: {e}"));
        }
        Live {
            key,
            size,
            pending: None,
            shared,
        }
    }
    /// Progress without the image.
    pub fn state(&self) -> LiveState {
        let st = lock(&self.shared.state);
        LiveState {
            image: None,
            fresh: st.fresh,
            samples: st.samples,
            cap: st.cap,
            seconds: st.seconds,
            error: st.error.clone(),
        }
    }
    /// Still rendering (not finished, refused or failed).
    pub fn running(&self) -> bool {
        let st = lock(&self.shared.state);
        st.error.is_none() && st.samples < st.cap
    }
    /// The latest mean if it is new, or with `again` whenever there is one (the reference
    /// texture was recreated by a resize, an export or an HDR switch).
    pub fn image(&self, again: bool) -> Option<std::sync::Arc<Vec<f32>>> {
        let mut st = lock(&self.shared.state);
        if !(st.fresh || again) {
            return None;
        }
        st.fresh = false;
        st.image.clone()
    }
    /// Stops the render (idempotent); the thread finishes its row and exits.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `--reference SPP`: renders and writes a linear EXR (or a PNG of the clamped values).
pub fn run(s: &Settings, size: [u32; 2], spp: u32, path: &std::path::Path) -> Result<()> {
    let start = std::time::Instant::now();
    let rgba = render(s, size, spp, &|f| {
        println!("REFERENCE_PROGRESS {:.0}%", f * 100.)
    })?;
    let rgb: Vec<f32> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let image = image::Rgb32FImage::from_raw(size[0], size[1], rgb).context("image size")?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exr"))
    {
        image.save(path)?;
    } else {
        image::DynamicImage::ImageRgb32F(image)
            .to_rgb8()
            .save(path)?;
    }
    println!(
        "REFERENCE_OK spp={spp} size={}x{} seconds={:.1}",
        size[0],
        size[1],
        start.elapsed().as_secs_f32()
    );
    Ok(())
}
use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_albedo_table_parses_and_a_white_furnace_conserves() {
        assert_eq!(ggx_table().len(), 1024);
        // Smooth surfaces keep almost all light at normal incidence.
        assert!(ggx_albedo(1., 0.) > 0.95);
        // A white diffuse BSDF integrates to about 1 (cosine-weighted, Monte Carlo).
        let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
        m.color_rough = [1., 1., 1., 1.];
        m.optical[0] = 1.;
        let (n, v) = (Vec3::Y, Vec3::Y);
        let mut r = Rng::new(1, 2);
        let mut sum = Vec3::ZERO;
        let k = 20000;
        for _ in 0..k {
            let l = cosine_sample(n, &mut r);
            // Cosine samples: f cos / (cos / pi).
            let (f, _) = eval_bsdf(&m, n, v, l);
            sum += f * PI;
        }
        let mean = sum / k as f32;
        // The GPU's model: a white rough dielectric with f0 = 0 (IOR 1) keeps about all light.
        assert!((mean.x - 1.).abs() < 0.06, "{mean}");
    }
    #[test]
    fn the_bvh_finds_the_nearest_triangle() {
        let tri = |z: f32, m| Tri {
            p: [
                Vec3::new(-1., -1., z),
                Vec3::new(1., -1., z),
                Vec3::new(0., 1., z),
            ],
            n: [Vec3::Z; 3],
            material: m,
        };
        let bvh = Bvh::build((0..20).map(|i| tri(-(i as f32) - 1., i)).collect());
        let h = bvh
            .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|_| true, false)
            .unwrap();
        assert_eq!(bvh.tris[h.tri].material, 0);
        let h = bvh
            .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|m| m >= 5, false)
            .unwrap();
        assert_eq!(bvh.tris[h.tri].material, 5);
        assert!(
            bvh.intersect(Vec3::ZERO, Vec3::Z, 1e4, &|_| true, false)
                .is_none()
        );
    }
    /// The default Cornell box renders, finite and lit.
    #[test]
    fn the_cornell_box_renders() {
        let mut s = Settings::default();
        s.bounces = 4;
        assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
        let img = render(&s, [32, 24], 4, &|_| {}).unwrap();
        assert!(img.iter().all(|v| v.is_finite()));
        let mean: f32 = img.chunks_exact(4).map(|p| p[0] + p[1] + p[2]).sum::<f32>() / (32. * 24.);
        assert!(mean > 0.01, "{mean}");
    }
    /// Passes of any size give the same bits as one pass.
    #[test]
    fn passes_match_one_render() {
        let mut s = Settings::default();
        s.bounces = 3;
        let one = render(&s, [16, 12], 5, &|_| {}).unwrap();
        let mut p = Progressive::new(&s, [16, 12]).unwrap();
        let stop = AtomicBool::new(false);
        for n in [2, 1, 2] {
            assert!(p.pass(n, 3, &stop, &|_| {}));
        }
        assert_eq!(p.samples, 5);
        assert_eq!(p.image(), one);
    }
}
