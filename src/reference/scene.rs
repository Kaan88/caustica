//! What a render reads: the triangles, materials, emitters and camera of the settings, and the
//! visibility and hit-frame rules.
use super::bsdf::{flags, rgb};
use super::bvh::{Bvh, Hit, Tri};
use crate::lighting::PackedMaterial;
use crate::scene::{Settings, Vertex};
use glam::{Mat4, Vec3};

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
    pub(super) materials: Vec<PackedMaterial>,
    /// Emissive triangles (indices into the BVH's order), their power CDF and total.
    pub(super) lights: Vec<usize>,
    pub(super) cdf: Vec<f32>,
    pub(super) eye: Vec3,
    pub(super) inverse_vp: Mat4,
    pub(super) frame: super::camera::Frame,
    /// The dielectric the camera is inside (`geometry::camera_medium`), as the GPU seeds it.
    pub(super) camera_media: Vec<(u32, u32)>,
    pub(super) size: [u32; 2],
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
impl Scene<'_> {
    pub(super) fn accept(&self, kind: Ray) -> impl Fn(u32) -> bool + '_ {
        move |id| {
            let m = &self.materials[id as usize];
            match kind {
                Ray::Camera => flags(m) & 2 == 0,
                Ray::Bounce => true,
                Ray::Shadow => flags(m) & 8192 == 0 && (self.s.glass_shadow || m.lobes[2] < 0.5),
            }
        }
    }
    pub(super) fn surface(&self, h: &Hit, d: Vec3) -> Surface {
        let t = &self.bvh.tris[h.tri];
        let (u, v) = (h.b.x, h.b.y);
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
}

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

impl<'a> Scene<'a> {
    pub(super) fn new(s: &'a Settings, size: [u32; 2]) -> Self {
        let materials = crate::lighting::materials(s);
        let vertices: Vec<Vertex> = crate::geometry::geometry(s);
        let tris: Vec<Tri> = vertices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| Tri {
                p: [0, 1, 2].map(|i| Vec3::from(v[i].position)),
                n: [0, 1, 2]
                    .map(|i| interpolation_normal(s, v[i].material, Vec3::from(v[i].normal))),
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
        let mut scene = Scene {
            s,
            bvh,
            materials,
            lights,
            cdf,
            eye: Vec3::ZERO,
            inverse_vp: Mat4::IDENTITY,
            frame: Default::default(),
            camera_media: vec![],
            size,
        };
        scene.set_camera(s, size);
        scene
    }
    /// Takes the camera of `camera`, a copy of the scene that differs from `self.s` in its camera
    /// fields at most. What renders reads the camera from here, never from `self.s`.
    pub(super) fn set_camera(&mut self, camera: &Settings, size: [u32; 2]) {
        let (view, proj, eye) = camera.camera(size[0] as f32 / size[1] as f32);
        self.eye = eye;
        self.inverse_vp = (proj * view).inverse();
        self.frame = super::camera::Frame {
            right: view.row(0).truncate(),
            up: view.row(1).truncate(),
            forward: -view.row(2).truncate(),
            aspect: size[0] as f32 / size[1] as f32,
            tan_half: 1. / proj.y_axis.y,
            ortho_half: camera.distance * (camera.fov.to_radians() * 0.5).tan(),
            projection: camera.projection,
        };
        self.camera_media = match crate::geometry::camera_medium(camera, eye) {
            0 => vec![],
            e => vec![(e >> 13, (e & 0x1fff) - 1)],
        };
        self.size = size;
    }
}
