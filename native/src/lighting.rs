//! GPU material records and power-weighted emissive-triangle sampling tables.
use crate::scene::{Material, Settings, Vertex};
use glam::Vec3;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PackedMaterial {
    pub color_rough: [f32; 4],
    pub lobes: [f32; 4],
    pub optical: [f32; 4],
    pub secondary: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Emitter {
    pub position_area: [f32; 4],
    pub edge1_cdf: [f32; 4],
    pub edge2_probability: [f32; 4],
    pub emission_material: [f32; 4],
}
fn pack(m: &Material) -> PackedMaterial {
    PackedMaterial {
        color_rough: [m.color[0], m.color[1], m.color[2], m.roughness],
        lobes: [
            m.metallic,
            m.reflectivity,
            m.transmission,
            m.unlit as u32 as f32,
        ],
        optical: [m.ior, m.emission, m.pattern as u32 as f32, 0.],
        secondary: [
            m.secondary_color[0],
            m.secondary_color[1],
            m.secondary_color[2],
            m.pattern_scale,
        ],
    }
}
pub fn materials(s: &Settings) -> Vec<PackedMaterial> {
    let mut out = vec![pack(&s.walls); 5];
    for (i, c) in [s.left, s.right, s.base].iter().enumerate() {
        out[i].color_rough[..3].copy_from_slice(c);
    }
    out[3] = pack(&s.floor);
    out[3].color_rough[..3].copy_from_slice(&s.base);
    let c = s.light_color();
    out[4] = pack(&Material {
        color: c,
        emission: 1.,
        unlit: true,
        ..Material::default()
    });
    out[4].optical[3] = 1.;
    out[4].lobes[3] = 1. + if s.light_visible { 0. } else { 2. };
    out.extend(s.objects.iter().map(|o| pack(&o.material)));
    let first_object = 5;
    out.extend(s.lights.iter().map(|l| {
        let mut m = pack(&Material {
            color: l.color,
            emission: if l.enabled { l.intensity } else { 0. },
            unlit: true,
            ..Material::default()
        });
        m.optical[3] = 1.;
        m.lobes[3] = 1. + if l.visible { 0. } else { 2. };
        m
    }));
    // Portals (flag bit 2): the object's record points (secondary.w) at an extra record whose
    // first three rows hold the affine map into the paired portal.
    for (i, o) in s.objects.iter().enumerate() {
        let Some(pair) = usize::try_from(o.portal)
            .ok()
            .and_then(|p| s.objects.get(p))
        else {
            continue;
        };
        let m = portal_matrix(o, pair);
        let row = |r: usize| [m.x_axis[r], m.y_axis[r], m.z_axis[r], m.w_axis[r]];
        let slot = out.len();
        out.push(PackedMaterial {
            color_rough: row(0),
            lobes: row(1),
            optical: row(2),
            secondary: [0.; 4],
        });
        let p = &mut out[first_object + i];
        p.color_rough = [1., 1., 1., 0.];
        p.lobes[3] = (p.lobes[3] as u32 | 4) as f32;
        p.optical[1] = 0.;
        p.optical[2] = 0.;
        p.secondary[3] = slot as f32;
    }
    out
}
/// World-to-world map from portal `a` into portal `b`: into a's local space, turned half way
/// around the local up axis (entering a's front leaves b's front), out of b's local space.
pub fn portal_matrix(a: &crate::scene::Object, b: &crate::scene::Object) -> glam::Mat4 {
    let local = |o: &crate::scene::Object| {
        let (center, rotation) = crate::geometry::object_frame(o);
        let scale = Vec3::new(o.scale_xz[0], o.stretch, o.scale_xz[1]) * o.size * 0.35;
        glam::Mat4::from_translation(center)
            * glam::Mat4::from_mat3(rotation)
            * glam::Mat4::from_scale(scale)
    };
    let flip = glam::Mat4::from_scale(Vec3::new(-1., 1., -1.));
    local(b) * flip * local(a).inverse()
}
pub fn emitter_table(
    vertices: &[Vertex],
    materials: &[PackedMaterial],
) -> (Vec<Emitter>, Vec<u32>) {
    let mut emitters = vec![];
    let mut lookup = vec![0; vertices.len() / 3];
    let mut powers = vec![];
    let mut sum = 0f64;
    for (i, t) in vertices.as_chunks::<3>().0.iter().enumerate() {
        let id = t[0].material;
        let m = materials[id as usize];
        let emission =
            Vec3::new(m.color_rough[0], m.color_rough[1], m.color_rough[2]) * m.optical[1];
        let lum = emission.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        if lum <= 0. {
            continue;
        }
        let a = Vec3::from(t[0].position);
        let u = Vec3::from(t[1].position) - a;
        let v = Vec3::from(t[2].position) - a;
        let area = u.cross(v).length() * 0.5;
        if area < 1e-10 {
            continue;
        }
        let power = area as f64 * lum as f64;
        if power <= 0. {
            continue;
        }
        sum += power;
        powers.push(power);
        lookup[i] = emitters.len() as u32 + 1;
        emitters.push(Emitter {
            position_area: [a.x, a.y, a.z, area],
            edge1_cdf: [u.x, u.y, u.z, 0.],
            edge2_probability: [v.x, v.y, v.z, 0.],
            emission_material: [emission.x, emission.y, emission.z, id as f32],
        });
    }
    let mut cdf = 0.;
    for (i, e) in emitters.iter_mut().enumerate() {
        let prob = (powers[i] / sum) as f32;
        cdf += prob;
        e.edge1_cdf[3] = cdf;
        e.edge2_probability[3] = prob;
    }
    if let Some(last) = emitters.last_mut() {
        last.edge1_cdf[3] = 1.;
    }
    (emitters, lookup)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn powers_are_normalized() {
        let s = Settings::default();
        let g = crate::scene::geometry(&s);
        let (e, l) = emitter_table(&g, &materials(&s));
        assert_eq!(e.len(), 2);
        assert_eq!(l.iter().filter(|x| **x != 0).count(), 2);
        assert!((e.iter().map(|e| e.edge2_probability[3]).sum::<f32>() - 1.).abs() < 1e-6);
        assert_eq!(e.last().unwrap().edge1_cdf[3], 1.);
    }
}
