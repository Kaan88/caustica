//! Texture maps on the CPU: the maps `render::attach_maps` binds, sampled bilinearly at level 0,
//! and the box mapping of texture sets on shapes, the floor and the walls.
//!
//! Level 0, not the GPU's ray cones: the reference is the ground truth a converged image tends
//! to, and supersampling a level-0 bilinear lookup over the pixel filter converges to the map
//! filtered by the pixel itself. The GPU's cones pick a coarser mip (trilinear, anisotropic,
//! with vendor-specific footprints) to cut noise and aliasing; the mips are linear box averages,
//! so the mean agrees and only the sharpness of distant, high-contrast texture differs.
use super::bsdf::{flags, rgb};
use super::bvh::Hit;
use super::scene::Scene;
use crate::lighting::PackedMaterial;
use crate::mesh::Map;
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use std::sync::Arc;

/// The maps of a scene (slot k + 1 in `PackedMaterial::maps` is `maps[k]`, 0 = none) and each
/// object's frame for box mapping.
#[derive(Default)]
pub(super) struct Textures {
    pub(super) maps: Vec<Arc<Map>>,
    /// Per object: world to object, object to world (3 × 3) and the axes' world lengths.
    frames: Vec<(Mat4, Mat3, Vec3)>,
}
impl Textures {
    /// Fills the map slots, flags and scales of `materials` as the GPU does (`attach_maps`).
    pub(super) fn new(s: &crate::scene::Settings, materials: &mut [PackedMaterial]) -> Self {
        let records: Vec<_> = s
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.visible && o.sdf.is_none())
            .map(|(i, _)| crate::render::plan::InstanceRecord {
                vertex_base: 0,
                material: 5 + i as u32,
                emitter_base: 0,
                object: i as u32,
            })
            .filter(|r| (r.material as usize) < materials.len())
            .collect();
        let mut maps: Vec<Arc<Map>> = vec![];
        let mut slot_of = |map: &Arc<Map>| {
            Ok(match maps.iter().position(|m| Arc::ptr_eq(m, map)) {
                Some(i) => i as u32,
                None => {
                    maps.push(map.clone());
                    maps.len() as u32 - 1
                }
            })
        };
        if let Err(e) = crate::render::attach_maps(s, &records, materials, &mut slot_of) {
            eprintln!("CPU reference: {e:#}");
        }
        let frames = s
            .objects
            .iter()
            .map(|o| {
                let c = crate::geometry::object_transform(o);
                let m = Mat4::from_cols_array_2d(&[
                    [c[0], c[4], c[8], 0.],
                    [c[1], c[5], c[9], 0.],
                    [c[2], c[6], c[10], 0.],
                    [c[3], c[7], c[11], 1.],
                ]);
                let m3 = Mat3::from_mat4(m);
                let size = Vec3::new(m3.x_axis.length(), m3.y_axis.length(), m3.z_axis.length());
                (m.inverse(), m3, size)
            })
            .collect();
        Textures { maps, frames }
    }
    /// Object `i`'s world x and y axes (scaled) and origin.
    pub(super) fn object_axes(&self, i: usize) -> Option<(Vec3, Vec3, Vec3)> {
        let (w2o, m3, _) = self.frames.get(i)?;
        Some((m3.x_axis, m3.y_axis, -(*m3 * w2o.w_axis.truncate())))
    }
}

/// Texel (x, y) of `map` at level 0, wrapped, in linear values (sRGB colour decoded, alpha not).
pub(super) fn texel(map: &Map, x: i64, y: i64) -> Vec4 {
    let (w, h) = (map.width as i64, map.height as i64);
    let i = ((y.rem_euclid(h) * w + x.rem_euclid(w)) * 4) as usize;
    let v = &map.rgba[i..i + 4];
    let c = |k: usize| {
        if map.srgb {
            crate::mesh::srgb_decode(v[k])
        } else {
            v[k] as f32 / 255.
        }
    };
    Vec4::new(c(0), c(1), c(2), v[3] as f32 / 255.)
}
/// Bilinear filtering at level 0 with wrapping, in linear space (the GPU's sRGB view and
/// wrapping sampler at mip 0).
pub(super) fn bilinear(map: &Map, uv: Vec2) -> Vec4 {
    let x = uv.x as f64 * map.width as f64 - 0.5;
    let y = uv.y as f64 * map.height as f64 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = texel(map, x0, y0).lerp(texel(map, x0 + 1, y0), fx);
    let bottom = texel(map, x0, y0 + 1).lerp(texel(map, x0 + 1, y0 + 1), fx);
    top.lerp(bottom, fy)
}
/// Box mapping along the dominant axis of `n` (ties go to x, then y): uv at `p` and the
/// directions dP/du, dP/dv, as `box_map` in pathtrace.hlsl lays the faces out.
pub(super) fn box_map(p: Vec3, n: Vec3, scale: f32) -> (Vec2, Vec3, Vec3) {
    let a = n.abs();
    let sign = |c: f32| if c < 0. { -1. } else { 1. };
    let (uv, t, bt) = if a.x >= a.y && a.x >= a.z {
        let s = sign(n.x);
        (Vec2::new(-s * p.z, -p.y), Vec3::new(0., 0., -s), -Vec3::Y)
    } else if a.y >= a.z {
        let s = sign(n.y);
        (Vec2::new(p.x, s * p.z), Vec3::X, Vec3::new(0., 0., s))
    } else {
        let s = sign(n.z);
        (Vec2::new(s * p.x, -p.y), Vec3::new(s, 0., 0.), -Vec3::Y)
    };
    (uv * scale, t, bt)
}

/// glTF's v runs down the image, so a map's +Y (green) is -dP/dv: the bitangent's sign, from a
/// normal and dP/du, dP/dv in one space (`tangent_sign` in pathtrace.hlsl).
pub(super) fn tangent_sign(n: Vec3, t: Vec3, bt: Vec3) -> f32 {
    if n.cross(t).dot(bt) < 0. { 1. } else { -1. }
}
/// A triangle's UV frame: world dP/du and dP/dv from its world edges `e` and UV edges `d`,
/// unnormalized, or `None` when its UVs are degenerate (no tangent frame, no normal map). The
/// edges of an instance are the object's edges under its affine map, so these are the world
/// images of the object-space derivatives the GPU transforms.
pub(super) fn uv_frame(e: [Vec3; 2], d: [Vec2; 2]) -> Option<(Vec3, Vec3)> {
    let det = d[0].x * d[1].y - d[0].y * d[1].x;
    (det.abs() > 1e-12).then(|| {
        (
            (e[0] * d[1].y - e[1] * d[0].y) / det,
            (e[1] * d[0].x - e[0] * d[1].x) / det,
        )
    })
}
/// A tangent-space normal map's texel `t` (UNORM, unrenormalized: a filtered texel is shorter)
/// applied to the shading normal `n` with tangent `tangent` (bitangent sign in w): the tangent
/// made perpendicular to `n`, the bitangent `n × T` times the sign, and z kept above 1e-3.
pub(super) fn normal_map(n: Vec3, tangent: Vec4, t: Vec3) -> Vec3 {
    let t = t * 2. - Vec3::ONE;
    let tt = tangent.truncate() - n * n.dot(tangent.truncate());
    if tt.length_squared() <= 1e-12 {
        return n;
    }
    let tt = tt.normalize();
    let b = n.cross(tt) * tangent.w;
    (tt * t.x + b * t.y + n * t.z.max(1e-3)).normalize()
}

impl Scene<'_> {
    /// Where hit `h` reads its maps: the uv and the world tangent with the bitangent's sign in w
    /// (w = 0: no frame), as `intersect` derives them: box mapping on the walls and the floor
    /// (world axes) and on shapes with a set (object axes at world size), else the mesh's UVs,
    /// repeated |w| times per unit for a set on them (`texture_uv`).
    pub(super) fn map_frame(&self, h: &Hit) -> Option<(Vec2, Vec4)> {
        let tri = &self.bvh.tris[h.tri];
        let raw = &self.materials[tri.material as usize];
        let (u, v) = (h.b.x, h.b.y);
        let p = tri.p[0] * (1. - u - v) + tri.p[1] * u + tri.p[2] * v;
        // The interpolated normal before it is turned toward the ray, as `intersect` maps.
        let n = tri.n[0] * (1. - u - v) + tri.n[1] * u + tri.n[2] * v;
        let scale = raw.base_scale[3];
        let Some(i) = (tri.material as usize).checked_sub(5) else {
            let (uv, t, bt) = box_map(p, n, scale.abs());
            return Some((uv, t.extend(tangent_sign(n, t, bt))));
        };
        // The object's frame. Its normal is the transpose of the triangle's (the inverse
        // transpose of the object's interpolated normal, `interpolation_normal`), exactly.
        let (w2o, m3, size) = self.textures.frames.get(i)?;
        let n_object = m3.transpose() * n;
        if scale > 0. {
            // In the object's axes at world size.
            let (uv, t, bt) = box_map(w2o.transform_point3(p) * *size, n_object, scale);
            return Some((uv, (*m3 * t).extend(tangent_sign(n_object, t, bt))));
        }
        let mut uv = tri.uv[0] * (1. - u - v) + tri.uv[1] * u + tri.uv[2] * v;
        let e = [tri.p[1] - tri.p[0], tri.p[2] - tri.p[0]];
        let d = [tri.uv[1] - tri.uv[0], tri.uv[2] - tri.uv[0]];
        let tangent = match uv_frame(e, d) {
            Some((t, bt)) => {
                // The sign in object space, as the GPU takes it.
                let to_object = Mat3::from_mat4(*w2o);
                let sign = tangent_sign(n_object, to_object * t, to_object * bt);
                t.extend(sign)
            }
            None => Vec4::ZERO,
        };
        if scale < 0. {
            uv *= -scale;
        }
        Some((uv, tangent))
    }
    /// The colour triangle `tri` emits at barycentrics `b` (`emitted_color`): with an emissive
    /// map (flag 64 on its raw record) the level-0 texel at the mesh UV, unscaled even under box
    /// mapping, times 1 / the map's mean (medium.xyz) and the mean emitted colour
    /// (secondary.yzw), so the triangle's mean is that colour; otherwise `m`'s own colour.
    pub(super) fn emission_colour(&self, m: &PackedMaterial, tri: usize, b: Vec2) -> Vec3 {
        // A procedural object (no triangle) emits its own colour: it has no UVs.
        let Some(t) = self.bvh.tris.get(tri) else {
            return rgb(m.color_rough);
        };
        let raw = &self.materials[t.material as usize];
        let map = (flags(raw) & 64 != 0)
            .then(|| (raw.secondary[0] as usize).checked_sub(1))
            .flatten()
            .and_then(|k| self.textures.maps.get(k));
        let Some(map) = map else {
            return rgb(m.color_rough);
        };
        let uv = t.uv[0] * (1. - b.x - b.y) + t.uv[1] * b.x + t.uv[2] * b.y;
        Vec3::from_slice(&raw.secondary[1..4])
            * bilinear(map, uv).truncate()
            * Vec3::from_slice(&raw.medium[..3])
    }
    /// What a material's maps give `m` at hit `h` and the shading normal `n` (`sample_maps`,
    /// then `apply_maps`): the base colour (the map times its scale, or without one the record's
    /// own colour, unpatterned, as the GPU reads `materials[h.material]`), roughness (G) and
    /// metal (B) scaled where the scale is nonzero, and the normal map bending `n`, which faces
    /// the ray already and may then lean away from it, as on the GPU.
    pub(super) fn apply_maps(&self, m: &mut PackedMaterial, h: &Hit, n: &mut Vec3) {
        // The GPU samples no maps on a procedural hit.
        let Some(tri) = self.bvh.tris.get(h.tri) else {
            return;
        };
        let raw = &self.materials[tri.material as usize];
        if flags(raw) & 16 == 0 {
            return;
        }
        let Some((uv, tangent)) = self.map_frame(h) else {
            return;
        };
        let map = |k: u32| match k {
            0 => None,
            k => self.textures.maps.get(k as usize - 1),
        };
        let albedo = match (raw.maps[0], map(raw.maps[0])) {
            (0, _) => rgb(raw.color_rough),
            (_, Some(map)) => (bilinear(map, uv).truncate() * Vec3::from_slice(&raw.base_scale))
                .clamp(Vec3::ZERO, Vec3::ONE),
            (_, None) => return,
        };
        m.color_rough[..3].copy_from_slice(&albedo.to_array());
        m.color_rough[3] = raw.color_rough[3];
        m.lobes[0] = raw.lobes[0];
        if let Some(map) = map(raw.maps[2]) {
            let r = bilinear(map, uv);
            if raw.surface_scale[0] > 0. {
                m.color_rough[3] = (r.y * raw.surface_scale[0]).clamp(0., 1.);
            }
            if raw.surface_scale[1] > 0. {
                m.lobes[0] = (r.z * raw.surface_scale[1]).clamp(0., 1.);
            }
        }
        if let Some(map) = map(raw.maps[1])
            && tangent.w != 0.
        {
            *n = normal_map(*n, tangent, bilinear(map, uv).truncate());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A 2 × 2 map: black, white / white, black (linear values unless `srgb`).
    fn checker(srgb: bool, bright: u8) -> Map {
        let px = |v: u8| [v, v, v, 255];
        let rgba = [px(0), px(bright), px(bright), px(0)].concat();
        Map::new(2, 2, rgba, srgb)
    }
    #[test]
    fn bilinear_level_0_is_exact_at_texel_centres_wraps_and_mixes_in_linear() {
        let m = checker(false, 255);
        assert_eq!(bilinear(&m, Vec2::new(0.25, 0.25)).x, 0.);
        assert_eq!(bilinear(&m, Vec2::new(0.75, 0.25)).x, 1.);
        assert_eq!(bilinear(&m, Vec2::new(0.25, 0.75)).x, 1.);
        // Wrapping: u = 1.25 is u = 0.25, and v = -0.25 is v = 0.75.
        assert_eq!(bilinear(&m, Vec2::new(1.25, 0.25)).x, 0.);
        assert_eq!(bilinear(&m, Vec2::new(0.25, -0.25)).x, 1.);
        // Halfway between two texels (also across the wrap at u = 0).
        assert!((bilinear(&m, Vec2::new(0.5, 0.25)).x - 0.5).abs() < 1e-6);
        assert!((bilinear(&m, Vec2::new(0., 0.25)).x - 0.5).abs() < 1e-6);
        // sRGB decodes before filtering; alpha is never decoded.
        let s = checker(true, 128);
        let c = bilinear(&s, Vec2::new(0.75, 0.25));
        assert!((c.x - 0.2158).abs() < 1e-3 && c.w == 1.);
        assert!((bilinear(&s, Vec2::new(0.5, 0.25)).x - c.x / 2.).abs() < 1e-6);
    }
    #[test]
    fn box_mapping_lays_each_face_out_as_the_shader() {
        let p = Vec3::new(1., 2., 3.);
        let cases = [
            (Vec3::X, Vec2::new(-3., -2.), Vec3::NEG_Z, Vec3::NEG_Y),
            (Vec3::NEG_X, Vec2::new(3., -2.), Vec3::Z, Vec3::NEG_Y),
            (Vec3::Y, Vec2::new(1., 3.), Vec3::X, Vec3::Z),
            (Vec3::NEG_Z, Vec2::new(-1., -2.), Vec3::NEG_X, Vec3::NEG_Y),
        ];
        for (n, uv, t, bt) in cases {
            let (u, tt, b) = box_map(p, n, 0.5);
            assert_eq!((u, tt, b), (uv * 0.5, t, bt), "{n}");
            // dP/du and dP/dv: moving p along them moves uv by one unit each (at scale 1).
            let (u1, _, _) = box_map(p + t, n, 1.);
            let (u2, _, _) = box_map(p + bt, n, 1.);
            assert_eq!((u1 - uv, u2 - uv), (Vec2::X, Vec2::Y), "{n}");
        }
        // Ties go to x, then y.
        assert_eq!(box_map(p, Vec3::ONE, 1.).1, Vec3::NEG_Z);
        assert_eq!(box_map(p, Vec3::new(0., 1., 1.), 1.).1, Vec3::X);
    }
    #[test]
    fn a_triangles_uv_frame_is_its_edges_per_uv_unit() {
        let e = [Vec3::new(2., 0., 0.), Vec3::new(0., 0., -3.)];
        // UVs (0,0), (1,0), (0,1): dP/du = e1, dP/dv = e2.
        assert_eq!(uv_frame(e, [Vec2::X, Vec2::Y]), Some((e[0], e[1])));
        // Halving the UV span doubles the derivatives; a swapped layout swaps them.
        assert_eq!(
            uv_frame(e, [Vec2::X * 0.5, Vec2::Y * 0.5]),
            Some((e[0] * 2., e[1] * 2.))
        );
        assert_eq!(uv_frame(e, [Vec2::Y, Vec2::X]), Some((e[1], e[0])));
        // Collinear UVs have no frame.
        assert_eq!(uv_frame(e, [Vec2::X, Vec2::X * 2.]), None);
        // Moving along T by one unit moves u by one: p(u, v) = u e1' + v e2' in general.
        let d = [Vec2::new(0.3, 0.1), Vec2::new(-0.2, 0.5)];
        let (t, bt) = uv_frame(e, d).unwrap();
        for (k, ek) in e.iter().enumerate() {
            assert!((t * d[k].x + bt * d[k].y - *ek).length() < 1e-5);
        }
        // +Y of the map is -dP/dv: right-handed (n, t, bt) gets sign -1.
        assert_eq!(tangent_sign(Vec3::Z, Vec3::X, Vec3::Y), -1.);
        assert_eq!(tangent_sign(Vec3::Z, Vec3::X, Vec3::NEG_Y), 1.);
    }
    #[test]
    fn a_flat_texel_keeps_the_normal_and_green_tilts_along_the_signed_bitangent() {
        let n = Vec3::new(0.2, 0.9, 0.1).normalize();
        let tangent = Vec3::new(1., 0.3, 0.).extend(-1.);
        let flat = normal_map(n, tangent, Vec3::new(0.5, 0.5, 1.));
        assert!((flat - n).length() < 1e-2, "{flat}");
        let t = (tangent.truncate() - n * n.dot(tangent.truncate())).normalize();
        let b = n.cross(t) * tangent.w;
        let up = normal_map(n, tangent, Vec3::new(0.5, 1., 0.5));
        assert!(up.dot(b) > 0.5 && up.dot(t).abs() < 1e-2, "{up}");
        let right = normal_map(n, tangent, Vec3::new(1., 0.5, 0.5));
        assert!(right.dot(t) > 0.5 && right.dot(b).abs() < 1e-2, "{right}");
        // A tangent along the normal has no frame: the normal stays.
        assert_eq!(normal_map(n, n.extend(1.), Vec3::new(1., 0., 0.5)), n);
    }
}
