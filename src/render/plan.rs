//! Scene planning both backends share (engine-notes "Vulkan backend: design", shared CPU work):
//! which static triangles form which group, which meshes are visible, and the instance list
//! with masks, opacity and the `Instance` records the shaders read. The backends only turn the
//! plan into their own BLAS / TLAS and buffers.
use crate::lighting::PackedMaterial;
use crate::scene::{Settings, Vertex};
use glam::{Mat4, Vec3};

/// Instance mask bits (pathtrace.hlsl MASK_HIDDEN, MASK_CLEAR_GLASS, MASK_NO_SHADOW).
pub const MASK_HIDDEN: u32 = 2;
pub const MASK_CLEAR_GLASS: u32 = 4;
pub const MASK_NO_SHADOW: u32 = 8;

/// An object instance's mask and whether it still needs the shader per candidate (track 20a
/// (a)). Rays leave out `MASK_HIDDEN` on camera rays and `MASK_CLEAR_GLASS` on shadow rays
/// when glass doesn't block (pathtrace.hlsl); every other ray includes all of 0xff. An object
/// under one rule gets just that bit and goes opaque; under both no single bit says it, so it
/// stays non-opaque with the shader's tests (as do glass objects below 0.5 transmission,
/// opaque now: the shader committed every candidate of theirs anyway).
pub fn instance_mask(m: &PackedMaterial) -> (u32, bool) {
    let hidden = (m.lobes[3] as u32 & 2) != 0;
    let clear = m.lobes[2] >= 0.5;
    // A light that casts no shadows (flag 8192) is left out of every shadow ray.
    let unshadowed = (m.lobes[3] as u32 & 8192) != 0;
    match (hidden, clear || unshadowed) {
        (true, true) => (0xff, true),
        (true, false) => (MASK_HIDDEN, false),
        (false, true) if unshadowed => (MASK_NO_SHADOW, false),
        (false, true) => (MASK_CLEAR_GLASS, false),
        (false, false) => (0xff, false),
    }
}

pub fn mesh_uvs(o: &crate::scene::Object, n: usize) -> Vec<[f32; 2]> {
    match crate::mesh::uvs(o).filter(|u| u.len() == n) {
        Some(u) => u.to_vec(),
        None => vec![[0.; 2]; n],
    }
}

/// Mirrors `Instance` in common.hlsli. `material` u32::MAX marks a static instance, whose
/// vertices carry their own material ids; its `vertex_base` is its first static triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRecord {
    pub vertex_base: u32,
    pub material: u32,
    pub emitter_base: u32,
    pub object: u32,
}

/// A run of static triangles that share an instance mask and opacity (`instance_mask`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GroupKey {
    pub mask: u32,
    pub any_hit: bool,
    pub first: u32,
    pub count: u32,
}

/// Static geometry (floor, room, lights) in world space, grouped by `instance_mask` like
/// objects (track 20a (a)): lights hidden from the camera and clear glass become opaque
/// instances with their mask bit, so only triangles under both rules still run the shader per
/// candidate. Each group is its own BLAS and instance; opaque 0xff comes first, so plain
/// scenes keep one static instance at InstanceID 0. Returns the groups and their vertices.
pub fn static_layout(s: &Settings, packed: &[PackedMaterial]) -> (Vec<GroupKey>, Vec<Vertex>) {
    let soup = crate::geometry::static_geometry(s);
    type Group<'a> = ((u32, bool), Vec<&'a [Vertex; 3]>);
    let mut groups: Vec<Group> = vec![];
    for t in soup.as_chunks::<3>().0 {
        let key = instance_mask(&packed[t[0].material as usize]);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push(t),
            None => groups.push((key, vec![t])),
        }
    }
    groups.sort_by_key(|g| g.0 != (0xff, false));
    let mut first = 0;
    let layout = groups
        .iter()
        .map(|g| {
            let group = GroupKey {
                mask: g.0.0,
                any_hit: g.0.1,
                first,
                count: g.1.len() as u32,
            };
            first += group.count;
            group
        })
        .collect();
    let statics = groups
        .into_iter()
        .flat_map(|g| g.1)
        .flatten()
        .copied()
        .collect();
    (layout, statics)
}

pub struct VisibleMesh {
    pub index: usize,
    /// The vertex range's key (`geometry::object_source`).
    pub key: String,
    /// The BLAS's key: `key`, or with micromaps also the base map and cutoff.
    pub blas_key: String,
    pub source: crate::geometry::Source,
    pub omm: Option<(std::sync::Arc<crate::mesh::Map>, f32)>,
}

/// The visible triangle-mesh objects. With `omm` (D3D12 opacity micromaps on), an alpha-tested
/// object whose material needs no other per-candidate test gets a micromap BLAS, keyed by
/// mesh, base map and cutoff.
pub fn visible_meshes(s: &Settings, packed: &[PackedMaterial], omm: bool) -> Vec<VisibleMesh> {
    let omm_of = |i: usize, o: &crate::scene::Object, key: &str| {
        if !omm || !o.material.texture.is_empty() {
            return None;
        }
        let maps = crate::mesh::maps(o)?;
        let base = maps.base.filter(|_| maps.cutoff > 0.)?;
        (instance_mask(&packed[5 + i]) == (0xff, false)).then(|| {
            let omm_key = format!(
                "{key}#omm{:x}/{}",
                std::sync::Arc::as_ptr(&base) as usize,
                maps.cutoff.to_bits()
            );
            (omm_key, base, maps.cutoff)
        })
    };
    s.objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.visible && o.sdf.is_none())
        .map(|(i, o)| {
            let (key, source) = crate::geometry::object_source(o);
            let omm = omm_of(i, o, &key);
            VisibleMesh {
                index: i,
                blas_key: omm.as_ref().map_or_else(|| key.clone(), |m| m.0.clone()),
                key,
                source,
                omm: omm.map(|m| (m.1, m.2)),
            }
        })
        .collect()
}

/// How an instance overrides its geometry's opacity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opacity {
    ForceOpaque,
    ForceNonOpaque,
    /// No override: the geometry's own flag (and its micromaps) stand.
    Geometry,
}

/// Which BLAS an instance uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blas<'a> {
    /// A static group, by index into the `static_layout` groups.
    Static(usize),
    /// An object mesh, by `VisibleMesh::blas_key`.
    Mesh(&'a str),
    /// The unit-cube AABB every procedural object instances.
    Sdf,
}

pub struct Instance<'a> {
    pub transform: [f32; 12],
    pub blas: Blas<'a>,
    pub mask: u32,
    pub opacity: Opacity,
}

/// Instances in InstanceID order: the static groups (emitter_base 0, so emitter_lookup takes
/// the static triangle index directly), then one per visible mesh object, then the procedural
/// objects (track 13). `mesh(blas_key)` gives a mesh's first vertex and triangle count in the
/// mesh vertex buffer. Also returns the `Instance` records and the triangle count.
pub fn instances<'a>(
    s: &Settings,
    packed: &[PackedMaterial],
    groups: &[GroupKey],
    visible: &'a [VisibleMesh],
    mesh: &dyn Fn(&str) -> (u32, usize),
) -> (Vec<Instance<'a>>, Vec<InstanceRecord>, usize) {
    let identity = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
    let mut plan = vec![];
    let mut table = vec![];
    let mut triangle_count = 0;
    for (i, g) in groups.iter().enumerate() {
        plan.push(Instance {
            transform: identity,
            blas: Blas::Static(i),
            mask: g.mask,
            opacity: if g.any_hit {
                Opacity::ForceNonOpaque
            } else {
                Opacity::ForceOpaque
            },
        });
        table.push(InstanceRecord {
            vertex_base: g.first,
            material: u32::MAX,
            emitter_base: 0,
            object: u32::MAX,
        });
        triangle_count += g.count as usize;
    }
    for v in visible {
        let (first, count) = mesh(&v.blas_key);
        let material = 5 + v.index as u32;
        // Masked maps (alpha cutoff) are tested in the any-hit loop as well.
        let o = &s.objects[v.index];
        let masked =
            o.material.texture.is_empty() && crate::mesh::maps(o).is_some_and(|m| m.cutoff != 0.);
        // Track 20a (a): an object's two skip rules (hidden from camera rays, glass that shadow
        // rays pass) become instance masks, which the rays leave out (pathtrace.hlsl MASK_*), so
        // the RT cores commit its hits without running the shader per candidate. Only
        // alpha-tested objects, and ones under both rules, stay non-opaque. With micromaps the
        // geometry's own non-opaque flag stands, so the RT cores read them.
        let (mask, any_hit) = instance_mask(&packed[material as usize]);
        let opacity = if v.omm.is_some() {
            Opacity::Geometry
        } else if any_hit || masked {
            Opacity::ForceNonOpaque
        } else {
            Opacity::ForceOpaque
        };
        plan.push(Instance {
            transform: crate::geometry::object_transform(o),
            blas: Blas::Mesh(&v.blas_key),
            mask: if masked { 0xff } else { mask },
            opacity,
        });
        table.push(InstanceRecord {
            vertex_base: first,
            material,
            emitter_base: u32::MAX,
            object: v.index as u32,
        });
        triangle_count += count;
    }
    // Procedural objects: the shader intersects the surface (kind in vertex_base). Glass and
    // hidden ones are non-opaque like meshes, though the shader sees every procedural
    // candidate either way.
    for (i, o) in s.objects.iter().enumerate() {
        if !o.visible || o.sdf.is_none() {
            continue;
        }
        let material = 5 + i as u32;
        let m = &packed[material as usize];
        let any_hit = (m.lobes[3] as u32 & 2) != 0 || m.lobes[2] > 0.;
        plan.push(Instance {
            transform: crate::geometry::object_transform(o),
            blas: Blas::Sdf,
            mask: 0xff,
            opacity: if any_hit {
                Opacity::ForceNonOpaque
            } else {
                Opacity::ForceOpaque
            },
        });
        table.push(InstanceRecord {
            vertex_base: o.sdf.id(o.phase, o.detail),
            material,
            emitter_base: u32::MAX,
            object: i as u32,
        });
    }
    (plan, table, triangle_count)
}

/// The light tables (`update_materials` on every backend): sets each mesh instance's
/// emitter_base (emissive objects' triangles join the static ones in the emitter table), then
/// builds the emitter table with its alias and guide records, the SDF photon records and the
/// emissive-map UV records, and the per-triangle lookup.
pub struct Lights {
    /// What t4 holds: the emitters, then their alias, guide, SDF and UV records.
    pub table: Vec<crate::lighting::Emitter>,
    pub lookup: Vec<u32>,
    pub emitter_count: usize,
    pub sun_count: u32,
    /// Any material with texture maps or a diffraction grating (the map variants of the trace).
    pub textured: bool,
    /// Every material passes the LEAN variants' gates (pathtrace.hlsl lean_strip).
    pub lean_scene: bool,
    /// SDF emitter count and the index of their first record.
    pub sdf_emitters: [u32; 2],
}

pub fn lights(
    s: &Settings,
    materials: &[PackedMaterial],
    statics: &[Vertex],
    instances: &mut [InstanceRecord],
) -> Lights {
    let mut world = statics.to_vec();
    // Emissive-mapped objects' UVs, by their first triangle in `world`.
    let mut mapped_uvs = vec![];
    for record in instances.iter_mut().filter(|r| r.material != u32::MAX) {
        let m = &materials[record.material as usize];
        let e = crate::lighting::emitted(m);
        let emission = e.x.max(e.y).max(e.z);
        // Procedural objects are not light-sampled: their emission is found by BSDF hits.
        let o = &s.objects[record.object as usize];
        record.emitter_base = if emission > 0. && o.sdf.is_none() {
            let base = (world.len() / 3) as u32;
            world.extend(crate::geometry::object_geometry(o, record.material));
            if m.lobes[3] as u32 & 64 != 0
                && let Some(uv) = crate::mesh::uvs(o)
            {
                mapped_uvs.push((base as usize, uv));
            }
            base
        } else {
            u32::MAX
        };
    }
    let analytic = crate::lighting::analytic(s);
    let (emitters, lookup) = crate::lighting::emitter_table(&world, materials, &analytic);
    let sun_count = crate::lighting::sun_count(&emitters);
    // Only real material records: the portal matrices after them hold a translation in optical.w.
    let textured = materials
        .iter()
        .take(5 + s.objects.len() + s.lights.len())
        .any(|m| m.lobes[3] as u32 & 16 != 0 || m.optical[3] < 0.);
    // The LEAN variants' material gates (pathtrace.hlsl lean_strip), over every record: a
    // portal matrix can only turn them off.
    let lean_scene = !s.objects.iter().any(|o| o.visible && !o.sdf.is_none())
        && materials.iter().all(|m| {
            m.layers[0] <= 0.
                && m.layers[3] <= 0.
                && m.sheen[..3].iter().all(|v| *v <= 0.)
                && m.surface_scale[2] <= 0.
                && m.medium[3] <= 0.
                // IES lights keep their profile in fields LEAN strips.
                && (m.lobes[3] as u32 & 16384) == 0
        });
    let emitter_count = emitters.len();
    // The alias table for ReSTIR follows the emitters (`emitter_count` stays the table),
    // then the CDF's cutpoints for `select_index`.
    let mut table = emitters.clone();
    table.extend(crate::lighting::alias_records(&emitters));
    table.extend(crate::lighting::guide_records(&emitters));
    // Emissive procedural objects for photon mapping (`sdf_emitter_records`), last.
    let sdf = sdf_emitter_records(s, instances, materials);
    let sdf_emitters = [(sdf.len() / 2) as u32, table.len() as u32];
    table.extend(sdf);
    // With an emissive map anywhere, one UV record per emitter after them (pathtrace.hlsl
    // `emitted_color`): a, b in position_area, c in edge1_cdf.xy, zero for the rest.
    if !mapped_uvs.is_empty() {
        let mut uvs: Vec<crate::lighting::Emitter> =
            vec![bytemuck::Zeroable::zeroed(); emitters.len()];
        for (base, uv) in &mapped_uvs {
            for (k, t) in uv.as_chunks::<3>().0.iter().enumerate() {
                if let Some(&e) = lookup.get(base + k)
                    && e != 0
                    && let Some(r) = uvs.get_mut(e as usize - 1)
                {
                    r.position_area = [t[0][0], t[0][1], t[1][0], t[1][1]];
                    r.edge1_cdf = [t[2][0], t[2][1], 0., 0.];
                }
            }
        }
        table.extend(uvs);
    }
    Lights {
        table,
        lookup,
        emitter_count,
        sun_count,
        textured,
        lean_scene,
        sdf_emitters,
    }
}

/// Track 15: photon records for emissive procedural (SDF) objects, which the emitter table
/// leaves out (light sampling can't pick points on them). Two records each: the world bounding
/// sphere of the object's unit cube (centre, radius), then its instance's index and the running
/// sum of the selection probabilities (by emission times sphere area), then the world-to-object
/// matrix's three rows. integrators.hlsl `sdf_photon` emits from them by random lines.
pub fn sdf_emitter_records(
    s: &Settings,
    instances: &[InstanceRecord],
    materials: &[crate::lighting::PackedMaterial],
) -> Vec<crate::lighting::Emitter> {
    let mut found = vec![];
    for (index, record) in instances.iter().enumerate() {
        if record.material == u32::MAX {
            continue;
        }
        let o = &s.objects[record.object as usize];
        let m = &materials[record.material as usize];
        let emission = m.color_rough[0].max(m.color_rough[1]).max(m.color_rough[2]) * m.optical[1];
        if o.sdf.is_none() || emission <= 0. {
            continue;
        }
        let t = crate::geometry::object_transform(o);
        let o2w = Mat4::from_cols_array(&[
            t[0], t[4], t[8], 0., t[1], t[5], t[9], 0., t[2], t[6], t[10], 0., t[3], t[7], t[11],
            1.,
        ]);
        let centre = o2w.transform_point3(Vec3::ZERO);
        let radius = (0..8)
            .map(|k| {
                let corner = Vec3::new(
                    if k & 1 == 0 { -1. } else { 1. },
                    if k & 2 == 0 { -1. } else { 1. },
                    if k & 4 == 0 { -1. } else { 1. },
                );
                (o2w.transform_point3(corner) - centre).length()
            })
            .fold(0f32, f32::max);
        if radius <= 0. || !radius.is_finite() {
            continue;
        }
        found.push((
            index as u32,
            centre,
            radius,
            emission * radius * radius,
            o2w.inverse(),
        ));
    }
    let total: f32 = found.iter().map(|f| f.3).sum();
    let mut sum = 0.;
    let mut out = vec![];
    for (i, (index, centre, radius, power, w2o)) in found.iter().enumerate() {
        sum += power / total;
        let cdf = if i + 1 == found.len() { 1. } else { sum };
        let row = |r: usize| w2o.row(r).to_array();
        out.push(crate::lighting::Emitter {
            position_area: [centre.x, centre.y, centre.z, *radius],
            edge1_cdf: [f32::from_bits(*index), cdf, 0., 0.],
            edge2_probability: row(0),
            emission_material: row(1),
        });
        out.push(crate::lighting::Emitter {
            position_area: row(2),
            edge1_cdf: [0.; 4],
            edge2_probability: [0.; 4],
            emission_material: [0.; 4],
        });
    }
    out
}
