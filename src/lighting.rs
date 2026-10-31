//! GPU material records and power-weighted light sampling tables: emissive triangles, plus
//! spheres and suns sampled by solid angle (track 7).
use crate::scene::{LightKind, Material, Settings, Vertex};
use glam::Vec3;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PackedMaterial {
    pub color_rough: [f32; 4],
    pub lobes: [f32; 4],
    pub optical: [f32; 4],
    pub secondary: [f32; 4],
    /// Texture maps (core track 5): heap slot + 1 of base colour, normal and
    /// metallic-roughness (0 = none), then the alpha cutoff's bits (0 = opaque).
    pub maps: [u32; 4],
    /// Base colour value / map average per channel (rgb); w unused.
    pub base_scale: [f32; 4],
    /// Roughness and metallic value / map average (G and B channels); z, w unused.
    pub surface_scale: [f32; 4],
    /// Track 11: clear coat, coat roughness, sheen roughness; w unused.
    pub layers: [f32; 4],
    /// Sheen colour times its strength; w unused.
    pub sheen: [f32; 4],
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
            // Flag bits 8–10: the nested-dielectric priority.
            (m.unlit as u32 | m.priority.min(7) << 8) as f32,
        ],
        optical: [m.ior, m.emission, m.pattern as u32 as f32, 0.],
        secondary: [
            m.secondary_color[0],
            m.secondary_color[1],
            m.secondary_color[2],
            m.pattern_scale,
        ],
        maps: [0; 4],
        // w: box-mapping repeats per unit for a texture set (0 = the mesh's own UVs).
        base_scale: [
            0.,
            0.,
            0.,
            if m.texture.is_empty() {
                0.
            } else {
                m.texture_scale
            },
        ],
        surface_scale: [0.; 4],
        layers: [m.clearcoat, m.clearcoat_roughness, m.sheen_roughness, 0.],
        sheen: [
            m.sheen_color[0] * m.sheen,
            m.sheen_color[1] * m.sheen,
            m.sheen_color[2] * m.sheen,
            0.,
        ],
    }
}
pub fn materials(s: &Settings) -> Vec<PackedMaterial> {
    let mut out = vec![pack(&s.walls); 5];
    // The room's colours come from left / right / base, except on a textured surface: its own
    // colour is the map average (`Material::textures`), and tinting bricks red would hide them.
    if s.walls.texture.is_empty() {
        for (i, c) in [s.left, s.right, s.base].iter().enumerate() {
            out[i].color_rough[..3].copy_from_slice(c);
        }
    }
    out[3] = pack(&s.floor);
    if s.floor.texture.is_empty() {
        out[3].color_rough[..3].copy_from_slice(&s.base);
    }
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
            emission: if l.enabled { l.radiance() } else { 0. },
            unlit: true,
            ..Material::default()
        });
        // Flat lights shine from their front only; a sphere all round.
        m.optical[3] = (l.kind != LightKind::Sphere) as u32 as f32;
        m.lobes[3] = 1. + if l.visible { 0. } else { 2. };
        // Spot (flag bit 5): emission × smoothstep(cos outer, cos inner, cos to the normal).
        if l.kind == LightKind::Spot {
            let (outer, inner) = l.cone();
            m.lobes[3] += 32.;
            m.secondary = [outer, inner, 0., 0.];
        }
        m
    }));
    // The physical sky's sun (track 9), right after the lights (`sun_material`); black while
    // it is below the horizon, so the record count only follows `on`.
    if s.physical_sky.on {
        let sun = s.physical_sky.sun().filter(|(_, l)| l.max_element() > 0.);
        let (color, emission) = match sun {
            Some((_, l)) => ((l / l.max_element()).to_array(), l.max_element()),
            None => ([1.; 3], 0.),
        };
        let mut m = pack(&Material {
            color,
            emission,
            unlit: true,
            ..Material::default()
        });
        m.optical[3] = 1.;
        m.lobes[3] = 1. + if s.physical_sky.visible { 0. } else { 2. };
        // Unused by the shader (no surface has this material): the direction, so that turning
        // the sky alone changes the material key and the emitter table follows.
        let d = s.physical_sky.sun_direction();
        m.secondary = [d.x, d.y, d.z, 0.];
        out.push(m);
    }
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
            ..bytemuck::Zeroable::zeroed()
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
/// A light sampled by direction rather than by triangle (track 7).
pub struct Analytic {
    pub material: u32,
    pub kind: LightKind,
    /// Sphere: its centre; sun: the unit direction towards it.
    pub at: Vec3,
    /// Sphere: radius; sun: 1 − cosine of its angular radius (as 2·sin² of half of it, which
    /// keeps its precision for small suns).
    pub extent: f32,
}
/// Suns first (the trace tests the first `suns` entries on every miss), then spheres.
pub fn analytic(s: &Settings) -> Vec<Analytic> {
    let first = 5 + s.objects.len() as u32;
    let lights = |kind| {
        s.lights
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.enabled && l.kind == kind)
            .map(move |(i, l)| Analytic {
                material: first + i as u32,
                kind,
                at: match kind {
                    LightKind::Sun => -l.direction(),
                    _ => Vec3::from(l.position),
                },
                extent: match kind {
                    LightKind::Sun => 2. * (l.angle * 0.25).to_radians().sin().powi(2),
                    _ => l.size[0] * 0.5,
                },
            })
    };
    let sky = &s.physical_sky;
    let sky_sun = sky
        .on
        .then(|| sky.sun())
        .flatten()
        .map(|(direction, _)| Analytic {
            material: sun_material(s),
            kind: LightKind::Sun,
            at: direction,
            extent: sky.sun_extent(),
        });
    sky_sun
        .into_iter()
        .chain(lights(LightKind::Sun))
        .chain(lights(LightKind::Sphere))
        .collect()
}
/// The material record of the physical sky's sun.
pub fn sun_material(s: &Settings) -> u32 {
    (5 + s.objects.len() + s.lights.len()) as u32
}
fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}
/// The emitter table (power-weighted CDF) and a per-triangle index into it (1-based, 0 = not
/// an emitter). Analytic lights come first, encoded by a negative `position_area.w` (−1 sphere,
/// −2 sun) with the centre or direction in xyz and radius or 1 − cos in `edge1_cdf.x`; a
/// sphere's triangles all point at its entry.
pub fn emitter_table(
    vertices: &[Vertex],
    materials: &[PackedMaterial],
    analytic: &[Analytic],
) -> (Vec<Emitter>, Vec<u32>) {
    let mut emitters = vec![];
    let mut lookup = vec![0; vertices.len() / 3];
    let mut powers = vec![];
    let mut sum = 0f64;
    let emission = |id: u32| {
        let m = materials[id as usize];
        Vec3::new(m.color_rough[0], m.color_rough[1], m.color_rough[2]) * m.optical[1]
    };
    let mut spheres = std::collections::HashMap::new();
    for a in analytic {
        let l = emission(a.material);
        let power = match a.kind {
            // Total output of the sphere's surface, as a triangle's area × radiance.
            LightKind::Sphere => 4. * std::f32::consts::PI * a.extent * a.extent * luminance(l),
            // Irradiance (L·π·sin²) over a nominal 50-unit area: suns have no finite power.
            _ => luminance(l) * std::f32::consts::PI * a.extent * (2. - a.extent) * 50.,
        } as f64;
        // Black or zero-intensity lights are not sampled (their triangles are not emitters).
        if power <= 0. {
            continue;
        }
        if a.kind == LightKind::Sphere {
            spheres.insert(a.material, emitters.len() as u32 + 1);
        }
        sum += power;
        powers.push(power);
        let code = if a.kind == LightKind::Sphere {
            -1.
        } else {
            -2.
        };
        emitters.push(Emitter {
            position_area: [a.at.x, a.at.y, a.at.z, code],
            edge1_cdf: [a.extent, 0., 0., 0.],
            edge2_probability: [0.; 4],
            emission_material: [l.x, l.y, l.z, a.material as f32],
        });
    }
    for (i, t) in vertices.as_chunks::<3>().0.iter().enumerate() {
        let id = t[0].material;
        if let Some(entry) = spheres.get(&id) {
            lookup[i] = *entry;
            continue;
        }
        let m = materials[id as usize];
        let emission = emission(id);
        let lum = luminance(emission);
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
        // A spot sends ∫cos over its cone, π·sin² of the half angle, instead of π (the inner
        // and outer angles averaged for the blend).
        let cone = if m.lobes[3] as u32 & 32 != 0 {
            let sin2 = |c: f32| 1. - c * c;
            0.5 * (sin2(m.secondary[0]) + sin2(m.secondary[1]))
        } else {
            1.
        };
        let power = area as f64 * lum as f64 * cone as f64;
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
/// How many suns lead the emitter table.
pub fn sun_count(emitters: &[Emitter]) -> u32 {
    emitters
        .iter()
        .take_while(|e| e.position_area[3] == -2.)
        .count() as u32
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn powers_are_normalized() {
        let s = Settings::default();
        let g = crate::scene::geometry(&s);
        let (e, l) = emitter_table(&g, &materials(&s), &analytic(&s));
        assert_eq!(e.len(), 2);
        assert_eq!(l.iter().filter(|x| **x != 0).count(), 2);
        assert!((e.iter().map(|e| e.edge2_probability[3]).sum::<f32>() - 1.).abs() < 1e-6);
        assert_eq!(e.last().unwrap().edge1_cdf[3], 1.);
    }
    #[test]
    fn analytic_lights_lead_the_table() {
        let mut s = Settings::default();
        for kind in [
            LightKind::Sphere,
            LightKind::Sun,
            LightKind::Spot,
            LightKind::Disk,
        ] {
            s.lights.push(crate::scene::AreaLight {
                kind,
                ..Default::default()
            });
        }
        s.validate().unwrap();
        let g = crate::scene::geometry(&s);
        let (e, l) = emitter_table(&g, &materials(&s), &analytic(&s));
        assert_eq!(sun_count(&e), 1);
        // Sun, sphere, then the main light's 2 triangles and 2 × 64 fan triangles.
        assert_eq!(e[0].position_area[3], -2.);
        assert_eq!(e[1].position_area[3], -1.);
        assert_eq!(e.len(), 2 + 2 + 2 * crate::geometry::DISK_SEGMENTS as usize);
        // Every sphere triangle points at the sphere's entry.
        let sphere = 5 + s.objects.len() as u32;
        let tris = g.as_chunks::<3>().0;
        assert!(
            tris.iter()
                .zip(&l)
                .filter(|(t, _)| t[0].material == sphere)
                .all(|(_, i)| *i == 2)
        );
        assert!((e.iter().map(|e| e.edge2_probability[3]).sum::<f32>() - 1.).abs() < 1e-5);
        // The sun points against its travel: straight down by default, so up.
        assert!((Vec3::from_slice(&e[0].position_area[..3]) - Vec3::Y).length() < 1e-6);
        // A disk of the same intensity as a 1 × 1 rectangle has the same radiance × area.
        let d = &s.lights[3];
        let area = std::f32::consts::PI * 0.25 * d.size[0] * d.size[1];
        assert!((d.radiance() * area - d.intensity).abs() < 1e-3);
    }
}
