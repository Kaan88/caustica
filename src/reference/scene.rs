//! What a render reads: the triangles, materials, emitters and camera of the settings, and the
//! visibility and hit-frame rules.
use super::bsdf::flags;
use super::bvh::{Bvh, Hit, Tri};
use crate::lighting::PackedMaterial;
use crate::scene::{Settings, Vertex};
use glam::{Mat4, Vec2, Vec3};

/// What a ray may hit: camera rays skip lights hidden from the camera; shadow rays skip lights
/// that cast no shadow and, unless glass casts shadows, clear glass.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Ray {
    Camera,
    Bounce,
    Shadow,
}

pub(super) struct Scene<'a> {
    pub(super) s: &'a Settings,
    pub(super) bvh: Bvh,
    /// The procedural objects, which the BVH doesn't hold.
    pub(super) sdfs: Vec<super::sdf::SdfObject>,
    pub(super) materials: Vec<PackedMaterial>,
    /// The emitters (suns and spheres first, then emissive triangles) and their power CDF.
    pub(super) lights: Vec<Light>,
    pub(super) cdf: Vec<f32>,
    /// Per BVH triangle, its entry in `lights` + 1, or 0: a sphere light's mesh maps to the
    /// sphere's entry, as `lighting::emitter_table`'s lookup does.
    pub(super) tri_light: Vec<u32>,
    /// Emissive procedural objects, and the share of non-sky light samples they take.
    pub(super) sdf_lights: Vec<super::lights::SdfLight>,
    pub(super) sdf_share: f32,
    pub(super) eye: Vec3,
    pub(super) inverse_vp: Mat4,
    /// BDPT's light tracing (`integrators::bdpt`): the view-projection a point projects through,
    /// the film's area at unit distance (4 tan²) and whether the camera is a pinhole perspective
    /// one, which light paths can reach.
    pub(super) vp: Mat4,
    pub(super) film_area: f32,
    pub(super) light_tracing: bool,
    pub(super) frame: super::camera::Frame,
    /// The lens system, when the camera has one (`Settings::real_lens`).
    pub(super) lens: Option<super::camera::RealLens>,
    /// The dielectric the camera is inside (`geometry::camera_medium`), as the GPU seeds it.
    pub(super) camera_media: Vec<(u32, u32)>,
    pub(super) size: [u32; 2],
    /// The HDRI or the physical sky's map, and the share of light samples it takes.
    pub(super) env: Option<super::environment::Env>,
    pub(super) p_env: f32,
    /// The texture maps and the frames that box-map them.
    pub(super) textures: super::textures::Textures,
    /// Spectral mode (`spectral`): on unless light path expressions force the path tracer to RGB,
    /// as on the GPU (`passes.rs`).
    pub(super) spectral: bool,
    /// Shadow rays stop at clear glass whatever `glass_shadow` says: BDPT's connections and
    /// photon mapping's next-event estimation see it as solid, as on the GPU (`glass_blocks`).
    pub(super) glass_blocks: bool,
    /// The integrator (`Settings::integrator`, or battle mode's quadrant's) and the pixels it
    /// renders (x0, y0, x1, y1): BDPT's light tracing splats only inside them, its film counts
    /// their share of the image, and photon mapping traces that share of the photons
    /// (`set_quadrant`).
    pub(super) integrator: u32,
    pub(super) region: [u32; 4],
    pub(super) share: f32,
    pub(super) photons: u32,
}
/// An entry of the light table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Light {
    /// An emissive triangle (index into the BVH's order).
    Tri(usize),
    /// A sphere light: centre, radius and flat emission. Sampled in the cone it covers; rays
    /// meet its inscribed mesh (`geometry::geometry`), which is two-sided and unlit.
    Sphere {
        c: Vec3,
        r: f32,
        le: Vec3,
        material: u32,
    },
    /// A sun: the unit direction towards it, 1 − cos of its angular radius, its radiance, and
    /// whether the camera can't see it (flag 2). It has no geometry: rays that miss the scene
    /// inside its disc meet it.
    Sun {
        d: Vec3,
        om: f32,
        le: Vec3,
        hidden: bool,
        material: u32,
    },
}
pub(super) struct Surface {
    pub(super) p: Vec3,
    /// The shading normal, facing the ray: what the BSDF and the cosines see.
    pub(super) n: Vec3,
    /// The face normal, facing the ray: what ray offsets leave along.
    pub(super) ng: Vec3,
    /// The side the ray came from, the triangle's, not the interpolated normal's: at grazing hits
    /// on smooth meshes those disagree, and a wrong side pushes or pops the wrong medium.
    pub(super) front: bool,
    pub(super) material: u32,
}
impl Surface {
    /// Triangle `t` at barycentrics (u, v), hit along `d`.
    pub(super) fn on(t: &Tri, u: f32, v: f32, d: Vec3) -> Surface {
        let (mut n, mut ng) = t.normals(u, v);
        let front = d.dot(ng) < 0.;
        if !front {
            ng = -ng;
        }
        if d.dot(n) > 0. {
            n = -n;
        }
        Surface {
            p: t.p[0] * (1. - u - v) + t.p[1] * u + t.p[2] * v,
            n,
            ng,
            front,
            material: t.material,
        }
    }
    /// The cosine toward a viewer along `-d` against the emitter's own normal (the triangle's face
    /// normal, as light sampling's `normals` orients it), signed: a spot's cone points out of the
    /// front, so a two-sided spot seen from behind is outside it, as light sampling sees it.
    pub(super) fn emission_cos(&self, d: Vec3) -> f32 {
        let c = (-d).dot(self.ng);
        if self.front { c } else { -c }
    }
}
impl Scene<'_> {
    /// The tangent an anisotropic material brushes along, or a grating's grooves run along, at
    /// `p` (the hit's `tangent`): world X for the walls, the object's x axis for an object, or
    /// for spun brushing (flag 2048) the circle about the object's y axis. Zero (unused) for
    /// other materials.
    pub(super) fn tangent(&self, material: u32, p: Vec3) -> Vec3 {
        let m = &self.materials[material as usize];
        if m.layers[3] <= 0. && crate::reference::bsdf::grating(m).is_none() {
            return Vec3::ZERO;
        }
        let (ax, ay, o) = (material as usize)
            .checked_sub(5)
            .and_then(|i| self.textures.object_axes(i))
            .unwrap_or((Vec3::X, Vec3::Y, Vec3::ZERO));
        if flags(m) & 2048 != 0 {
            ay.cross(p - o)
        } else {
            ax
        }
    }
    /// The candidate filter of a ray from `o` along `d`: alpha holes first (`alpha_hole`), then
    /// the kind's rule. A hole is no hit at all: it takes no bounce and no false hit.
    pub(super) fn accept(
        &self,
        kind: Ray,
        o: Vec3,
        d: Vec3,
    ) -> impl Fn(u32, usize, Vec2) -> bool + '_ {
        // A blend decision per ray, stable across that ray's candidates: a hash of the ray
        // itself, so scenes without blending keep their random streams bit for bit.
        let key = mix(o
            .to_array()
            .into_iter()
            .chain(d.to_array())
            .fold(0, |k, c| mix(k ^ c.to_bits())));
        move |id, tri, b| {
            let m = &self.materials[id as usize];
            if flags(m) & 8 != 0 && self.alpha_hole(m, tri, b, key) {
                return false;
            }
            match kind {
                Ray::Camera => flags(m) & 2 == 0,
                Ray::Bounce => true,
                Ray::Shadow => {
                    flags(m) & 8192 == 0
                        && (self.s.glass_shadow || self.glass_blocks || m.lobes[2] < 0.5)
                }
            }
        }
    }
    /// Whether triangle `tri` at barycentrics `b` is a hole in its masked material `m` (flag 8):
    /// the base map's level-0 alpha at the raw vertex UVs (a texture set has no cutoff, so box
    /// mapping never reaches this). A cutoff c >= 0 is glTF MASK, a hole where alpha < c (c is
    /// already divided by the factor's alpha). A negative one is BLEND, −(the factor's alpha):
    /// stochastic transparency, opaque with probability alpha × −c, decided by a hash of the
    /// ray's `key`, the triangle and the barycentrics, not a random draw.
    pub(super) fn alpha_hole(&self, m: &PackedMaterial, tri: usize, b: Vec2, key: u32) -> bool {
        let Some(map) = (m.maps[0] as usize)
            .checked_sub(1)
            .and_then(|i| self.textures.maps.get(i))
        else {
            return false;
        };
        // A procedural object has no UVs and no cutout, as on the GPU.
        let Some(t) = self.bvh.tris.get(tri) else {
            return false;
        };
        let uv = t.uv[0] * (1. - b.x - b.y) + t.uv[1] * b.x + t.uv[2] * b.y;
        let a = super::textures::bilinear(map, uv).w;
        let c = f32::from_bits(m.maps[3]);
        if c >= 0. {
            return a < c;
        }
        let k = mix(key ^ mix((tri as u32).wrapping_mul(0x85eb_ca6b)) ^ b.x.to_bits());
        a * -c <= (k >> 8) as f32 * (1. / 16_777_216.)
    }
    /// The nearest hit along o + t d in (1e-4, tmax) whose material `accept` takes, triangles
    /// and procedural objects alike; with `any`, the first one found.
    pub(super) fn intersect(
        &self,
        o: Vec3,
        d: Vec3,
        tmax: f32,
        accept: &dyn Fn(u32, usize, Vec2) -> bool,
        any: bool,
    ) -> Option<Hit> {
        let mut best = self.bvh.intersect(o, d, tmax, accept, any);
        if any && best.is_some() {
            return best;
        }
        for (i, sdf) in self.sdfs.iter().enumerate() {
            let limit = best.map_or(tmax, |h| h.t);
            if !accept(sdf.material, usize::MAX, Vec2::ZERO) {
                continue;
            }
            if let Some(t) = sdf.hit(o, d, 1e-4, limit) {
                best = Some(Hit {
                    t,
                    tri: usize::MAX,
                    b: Vec2::ZERO,
                    material: sdf.material,
                    sdf: Some(i),
                });
                if any {
                    break;
                }
            }
        }
        best
    }
    /// The surface at hit `h` of the ray from `o` along `d`.
    pub(super) fn surface(&self, h: &Hit, o: Vec3, d: Vec3) -> Surface {
        let Some(i) = h.sdf else {
            return Surface::on(&self.bvh.tris[h.tri], h.b.x, h.b.y, d);
        };
        // One normal for shading and offsets; the side is the gradient's.
        let mut n = self.sdfs[i].normal(o, d, h.t);
        let front = d.dot(n) < 0.;
        if !front {
            n = -n;
        }
        Surface {
            p: o + d * h.t,
            n,
            ng: n,
            front,
            material: h.material,
        }
    }
}

/// A 32-bit integer hash (lowbias32), for decisions that must not draw from the sampler.
pub(super) fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

// truth hooks `offsets` (C1: the 0.0003 ray offsets in integrators/path.rs, lights.rs, bvh.rs) and
// `shadow_terminator` (C2: Hanika's offset and below-face rejection, at those same offset sites) are
// written by the next accuracy step; the flags report pending until then.
/// An object's unit world normal at a vertex, back to the inverse-transpose image of its unit
/// object-space normal. A mesh's normals interpolate in object space, then transform (the GPU's
/// instances); under non-uniform scale that differs from interpolating the unit world normals, so
/// a squashed ring would shade differently. With M = R S, |M^-T n| = 1 / |S R^T N|.
pub(super) fn interpolation_normal(s: &Settings, material: u32, n: Vec3) -> Vec3 {
    let Some(o) = (material as usize)
        .checked_sub(5)
        .and_then(|i| s.objects.get(i))
    else {
        return n;
    };
    let (scale, r) = crate::geometry::scale_rotation(o);
    n / (scale * glam::Mat3::from_mat4(r).transpose().mul_vec3(n))
        .length()
        .max(1e-12)
}

/// Copies each object's mesh UVs into its vertices (`geometry` leaves them zero), where its
/// material maps them: flag 16 without box mapping (base_scale.w <= 0), or an emissive map
/// (flag 64, read on mesh UVs even with box mapping, as the GPU's emitter UV records) or an alpha
/// cutout (flag 8, `alpha_hole`), as the
/// GPU's instances carry them (`plan::mesh_uvs`, in the mesh source's order). Objects follow the static geometry
/// in order, invisible ones empty and procedural ones absent, as `geometry::geometry` lays them.
fn mesh_uvs(s: &Settings, materials: &[PackedMaterial], vertices: &mut [Vertex]) {
    let mapped = |i: usize| {
        materials.get(5 + i).is_some_and(|m| {
            flags(m) & 16 != 0 && m.base_scale[3] <= 0. || flags(m) & (8 | 64) != 0
        })
    };
    if !(0..s.objects.len()).any(mapped) {
        return;
    }
    let mut at = crate::geometry::static_geometry(s).len();
    for (i, o) in s.objects.iter().enumerate() {
        if !o.sdf.is_none() || !o.visible {
            continue;
        }
        let n = crate::geometry::object_source(o).1.len();
        if mapped(i)
            && let Some(run) = vertices.get_mut(at..at + n)
        {
            for (v, uv) in run.iter_mut().zip(crate::render::plan::mesh_uvs(o, n)) {
                v.uv = uv;
            }
        }
        at += n;
    }
}

impl<'a> Scene<'a> {
    pub(super) fn new(s: &'a Settings, size: [u32; 2]) -> Self {
        let mut materials = crate::lighting::materials(s);
        let textures = super::textures::Textures::new(s, &mut materials);
        let mut vertices: Vec<Vertex> = crate::geometry::geometry(s);
        mesh_uvs(s, &materials, &mut vertices);
        let tris: Vec<Tri> = vertices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| Tri {
                p: [0, 1, 2].map(|i| Vec3::from(v[i].position)),
                n: [0, 1, 2]
                    .map(|i| interpolation_normal(s, v[i].material, Vec3::from(v[i].normal))),
                uv: [0, 1, 2].map(|i| Vec2::from(v[i].uv)),
                material: v[0].material,
            })
            .filter(|t| (t.material as usize) < materials.len())
            .collect();
        let bvh = Bvh::build(tris);
        let sdfs = super::sdf::SdfObject::of(s);
        // Suns and sphere lights first, as `emitter_table`: a sphere by its surface's power
        // (4πr² × luminance), a sun by its irradiance (L·π·sin² of its radius) over the area of
        // the scene it lights (`Analytic::area`).
        let lum = |c: Vec3| 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
        let mut lights = vec![];
        let mut weights = vec![];
        let mut sphere_entry = std::collections::HashMap::new();
        for a in crate::lighting::analytic(s) {
            if a.kind == crate::scene::LightKind::Sun {
                let m = &materials[a.material as usize];
                let le = crate::lighting::emitted(m);
                let om = a.extent;
                let w = lum(le) * std::f32::consts::PI * om * (2. - om) * a.area;
                if w > 0. {
                    lights.push(Light::Sun {
                        d: a.at.normalize(),
                        om,
                        le,
                        hidden: m.lobes[3] as u32 & 2 != 0,
                        material: a.material,
                    });
                    weights.push(w);
                }
                continue;
            }
            if a.kind != crate::scene::LightKind::Sphere {
                continue;
            }
            // Its mesh is never a triangle light, sampled or not.
            sphere_entry.insert(a.material, 0);
            let le = crate::lighting::emitted(&materials[a.material as usize]);
            let w = 4. * std::f32::consts::PI * a.extent * a.extent * lum(le);
            if w > 0. {
                sphere_entry.insert(a.material, lights.len() as u32 + 1);
                lights.push(Light::Sphere {
                    c: a.at,
                    r: a.extent,
                    le,
                    material: a.material,
                });
                weights.push(w);
            }
        }
        // Emissive triangles by power: area × luminance of the emission (pattern-free colour;
        // an emissive map's mean, as the GPU's table), uniform over each triangle.
        let mut tri_light = vec![0; bvh.tris.len()];
        for (i, t) in bvh.tris.iter().enumerate() {
            if let Some(e) = sphere_entry.get(&t.material) {
                tri_light[i] = *e;
                continue;
            }
            let m = &materials[t.material as usize];
            if m.optical[1] > 0. {
                let c = crate::lighting::emitted(m);
                weights.push(Scene::area(t) * lum(c).max(1e-6));
                lights.push(Light::Tri(i));
                tri_light[i] = lights.len() as u32;
            }
        }
        let total: f32 = weights.iter().sum();
        let mut cdf = vec![];
        let mut acc = 0.;
        for w in weights {
            acc += w;
            cdf.push(acc / total);
        }
        // A map that fails to load leaves the plain sky; `Progressive::new` refuses that.
        let mut env = super::environment::Env::load(s).unwrap_or_else(|e| {
            eprintln!("CPU reference: {e:#}");
            None
        });
        // The stars' camera cone: half an output pixel, as the GPU's constants (`render.rs`).
        if let Some((_, f)) = env.as_mut().and_then(|e| e.stars.as_mut()) {
            f.chord = crate::stars::chord(s.fov, size[1], s.physical_sky.star_size);
        }
        // Procedural emitters take light samples only alongside a light-sampling strategy (with
        // BSDF sampling alone, their hits count in full); with triangles too, half of them.
        let sdf_lights = if s.mis_mode.min(2) == 2 {
            vec![]
        } else {
            super::lights::SdfLight::of(&sdfs, &materials)
        };
        let sdf_share = match (sdf_lights.is_empty(), lights.is_empty()) {
            (true, _) => 0.,
            (false, true) => 1.,
            (false, false) => 0.5,
        };
        // The environment and the emitters share light samples half and half.
        let p_env = match (&env, lights.is_empty() && sdf_lights.is_empty()) {
            (None, _) => 0.,
            (Some(_), true) => 1.,
            (Some(_), false) => 0.5,
        };
        let mut scene = Scene {
            s,
            bvh,
            sdfs,
            materials,
            lights,
            cdf,
            tri_light,
            sdf_lights,
            sdf_share,
            eye: Vec3::ZERO,
            inverse_vp: Mat4::IDENTITY,
            vp: Mat4::IDENTITY,
            film_area: 0.,
            light_tracing: false,
            frame: Default::default(),
            lens: None,
            camera_media: vec![],
            size,
            env,
            p_env,
            textures,
            spectral: false,
            glass_blocks: false,
            integrator: 0,
            region: [0; 4],
            share: 1.,
            photons: 0,
        };
        scene.set_camera(s, size);
        scene.set_quadrant(s.integrator, [0, 0, size[0], size[1]]);
        scene
    }
    /// Renders `region` with `integrator` from here on (battle mode: one quadrant's). Spectral
    /// mode holds unless light path expressions force the path tracer to RGB; clear glass blocks
    /// BDPT's and photon mapping's shadow rays; photon mapping traces the region's share of the
    /// photons, at least 1024, as on the GPU (`passes.rs`, without battle mode's time balance,
    /// which only moves noise).
    pub(super) fn set_quadrant(&mut self, integrator: u32, region: [u32; 4]) {
        let s = self.s;
        let area = |r: [u32; 4]| (r[2] - r[0]) as f32 * (r[3] - r[1]) as f32;
        self.integrator = integrator;
        self.region = region;
        self.share = area(region) / area([0, 0, self.size[0], self.size[1]]).max(1.);
        self.spectral = s.spectral && (integrator != 0 || s.lpe.iter().all(|e| e.is_empty()));
        self.glass_blocks = integrator != 0;
        self.photons =
            ((s.photons as f32 * self.share) as u32).clamp(1024.min(s.photons), s.photons);
    }
    /// A sample's wavelengths, from its own stream (`r` stays as it was, so RGB draws don't
    /// move), or RGB.
    pub(super) fn waves(&self, r: &super::sampling::Rng) -> super::spectral::Waves {
        if self.spectral {
            super::spectral::Waves::new(r.fork(0x7761_7665).f())
        } else {
            super::spectral::Waves::rgb()
        }
    }
    /// Takes the camera of `camera`, a copy of the scene that differs from `self.s` in its camera
    /// fields at most. What renders reads the camera from here, never from `self.s`.
    pub(super) fn set_camera(&mut self, camera: &Settings, size: [u32; 2]) {
        let (view, proj, eye) = camera.camera(size[0] as f32 / size[1] as f32);
        self.eye = eye;
        self.vp = proj * view;
        self.inverse_vp = self.vp.inverse();
        self.film_area = 4. / (proj.x_axis.x * proj.y_axis.y);
        self.light_tracing = camera.perspective() && camera.aperture <= 0.;
        self.frame = super::camera::Frame {
            right: view.row(0).truncate(),
            up: view.row(1).truncate(),
            forward: -view.row(2).truncate(),
            aspect: size[0] as f32 / size[1] as f32,
            tan_half: 1. / proj.y_axis.y,
            ortho_half: camera.distance * (camera.fov.to_radians() * 0.5).tan(),
            projection: camera.projection,
            aperture: camera.aperture,
            focus: camera.focus_distance,
            blades: camera.aperture_blades,
            rotation: camera.blade_rotation.to_radians(),
            cat_eye: camera.cat_eye,
            tilt: camera.tilt.to_radians(),
        };
        self.lens = camera
            .real_lens()
            .then(|| super::camera::RealLens::new(camera, size[0] as f32 / size[1] as f32));
        self.camera_media = match crate::geometry::camera_medium(camera, eye) {
            0 => vec![],
            e => vec![(e >> 13, (e & 0x1fff) - 1)],
        };
        if self.size != size {
            self.size = size;
            self.set_quadrant(self.integrator, [0, 0, size[0], size[1]]);
        }
    }
}
