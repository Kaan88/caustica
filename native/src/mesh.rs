//! Imported meshes (roadmap core track 2): OBJ through tobj, glTF / GLB through gltf. A mesh is
//! centred and scaled into the same -1..1 box as the built-in shapes, so an object's size means
//! the same for both. Loads (and failures) are cached per resolved path.
//!
//! A file with several materials is split into parts, one per material, addressed as
//! `file.glb#2`. All parts share the file's box, so objects with the same transform line up.
use crate::scene::{Material, Object, Shape};
use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// Triangle soup: three (position, normal) pairs per triangle, like the built-in shapes.
pub type Mesh = Vec<(Vec3, Vec3)>;
pub const MAX_TRIANGLES: usize = 20_000_000;
const MAX_FILE: u64 = 2 << 30;

/// One material's triangles of a file.
pub struct Part {
    pub name: String,
    pub mesh: Arc<Mesh>,
    pub material: Option<Material>,
}
/// A loaded file: its parts (at least one), and every part merged, built on first use.
pub struct Imported {
    pub parts: Vec<Part>,
    whole: OnceLock<Arc<Mesh>>,
}
impl Imported {
    /// All parts as one mesh (a file used without `#part`).
    pub fn whole(&self) -> Arc<Mesh> {
        self.whole
            .get_or_init(|| match &self.parts[..] {
                [one] => one.mesh.clone(),
                parts => Arc::new(parts.iter().flat_map(|p| p.mesh.iter().copied()).collect()),
            })
            .clone()
    }
    pub fn material(&self) -> Option<Material> {
        self.parts.iter().find_map(|p| p.material.clone())
    }
}
/// `file#n` → (file, Some(n)); anything else → (path, None).
fn split(path: &str) -> (&str, Option<usize>) {
    match path.rsplit_once('#') {
        Some((file, n)) if n.parse::<usize>().is_ok() => (file, n.parse().ok()),
        _ => (path, None),
    }
}
type Cache = HashMap<PathBuf, Result<Arc<Imported>, String>>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// Relative paths are relative to the scenes folder (where scene files and `meshes/` live).
pub fn resolve(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() || path.starts_with(crate::procedural::PREFIX) {
        return p;
    }
    crate::scenes::folder().map(|f| f.join(&p)).unwrap_or(p)
}
/// The path to store for a file: relative to the scenes folder when inside it.
pub fn store(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(root) = crate::scenes::folder().and_then(|f| f.canonicalize().ok())
        && let Ok(rel) = path.strip_prefix(&root)
    {
        return rel.to_string_lossy().replace('\\', "/");
    }
    path.to_string_lossy().into()
}
/// Loads a file (a `#part` suffix is ignored here).
pub fn load(path: &str) -> Result<Arc<Imported>> {
    let key = resolve(split(path).0);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(&key) {
        return hit.clone().map_err(anyhow::Error::msg);
    }
    let result = read(&key).map(Arc::new).map_err(|e| format!("{e:#}"));
    cache.insert(key, result.clone());
    result.map_err(anyhow::Error::msg)
}
/// The mesh of an object, or None for built-in shapes, unreadable files and missing parts.
pub fn of(o: &Object) -> Option<Arc<Mesh>> {
    if o.mesh.is_empty() {
        return None;
    }
    let imported = load(&o.mesh).ok()?;
    match split(&o.mesh).1 {
        Some(n) => imported.parts.get(n).map(|p| p.mesh.clone()),
        None => Some(imported.whole()),
    }
}
/// What grounding measures: the whole file, so the parts of one file rest on the floor
/// together (a chess piece stays on its board).
pub fn ground(o: &Object) -> Option<Arc<Mesh>> {
    of(o)?;
    load(&o.mesh).ok().map(|m| m.whole())
}
/// One object for a mesh file, with the file's first material.
pub fn import_object(path: &Path) -> Result<Object> {
    let stored = store(path);
    let imported = load(&stored)?;
    Ok(object(path, stored, imported.material()))
}
/// One object per part (per material) of a mesh file, all with the same transform.
pub fn import_objects(path: &Path) -> Result<Vec<Object>> {
    let stored = store(path);
    let imported = load(&stored)?;
    if imported.parts.len() == 1 {
        return Ok(vec![object(path, stored, imported.material())]);
    }
    Ok(imported
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| !part.mesh.is_empty())
        .map(|(i, part)| {
            let mut o = object(path, format!("{stored}#{i}"), part.material.clone());
            o.name = format!("{} · {}", o.name, part.name);
            o.name.truncate(o.name.floor_char_boundary(128));
            o
        })
        .collect())
}
fn object(path: &Path, mesh: String, material: Option<Material>) -> Object {
    let name = path
        .file_stem()
        .map_or("Mesh".into(), |s| s.to_string_lossy().to_string());
    Object {
        name,
        shape: Shape::Box,
        mesh,
        position: [0., 0.],
        size: 2.,
        stretch: 1.,
        rotation: 0.,
        material: material.unwrap_or_default(),
        ..Object::default()
    }
}
fn read(path: &Path) -> Result<Imported> {
    let mut parts = match path
        .to_str()
        .and_then(|p| p.strip_prefix(crate::procedural::PREFIX))
    {
        Some(spec) => read_procedural(spec)?,
        None => read_file(path)?,
    };
    // Empty parts stay, so `#n` indices never shift (a terrain band can be empty).
    let triangles: usize = parts.iter().map(|p| p.0.len() / 3).sum();
    ensure!(triangles > 0, "{} has no triangles", path.display());
    ensure!(
        triangles <= MAX_TRIANGLES,
        "{} has {triangles} triangles (limit 20M)",
        path.display(),
    );
    normalize(&mut parts);
    Ok(Imported {
        parts: parts
            .into_iter()
            .map(|(mesh, name, material)| Part {
                name,
                mesh: Arc::new(mesh),
                material,
            })
            .collect(),
        whole: OnceLock::new(),
    })
}
fn read_file(path: &Path) -> Result<Vec<RawPart>> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("{}", path.display()))?
        .len();
    ensure!(size <= MAX_FILE, "{} is larger than 2 GB", path.display());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "obj" => read_obj(path),
        "gltf" | "glb" => read_gltf(path),
        _ => bail!("Unsupported mesh format .{ext} (OBJ, glTF, GLB)"),
    }
}
fn read_procedural(spec: &str) -> Result<Vec<RawPart>> {
    use crate::procedural::Geometry;
    Ok(crate::procedural::build(spec)?
        .into_iter()
        .map(|(name, geometry)| {
            let mesh = match geometry {
                Geometry::Indexed(p, idx) => {
                    let mut out = Mesh::new();
                    triangles(&p, None, &idx, &mut out);
                    out
                }
                Geometry::Flat(tris) => tris,
            };
            (mesh, name, None)
        })
        .collect())
}
/// Centre every part on the file's bounding box and scale its largest half-extent to 1.
fn normalize(parts: &mut [RawPart]) {
    let (lo, hi) = parts.iter().flat_map(|p| &p.0).fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), (p, _)| (lo.min(*p), hi.max(*p)),
    );
    let centre = (lo + hi) * 0.5;
    let scale = 1. / ((hi - lo) * 0.5).max_element().max(1e-12);
    for (p, _) in parts.iter_mut().flat_map(|p| p.0.iter_mut()) {
        *p = (*p - centre) * scale;
    }
}
/// A part as read: triangles, material name, material.
type RawPart = (Mesh, String, Option<Material>);
/// The part for a material key, created in first-seen order.
fn part_for<K: PartialEq>(
    keys: &mut Vec<K>,
    parts: &mut Vec<RawPart>,
    key: K,
    make: impl FnOnce() -> (String, Option<Material>),
) -> usize {
    if let Some(i) = keys.iter().position(|k| *k == key) {
        return i;
    }
    let (name, material) = make();
    keys.push(key);
    parts.push((Mesh::new(), name, material));
    parts.len() - 1
}
/// Triangles from indexed positions, with given normals or smooth ones averaged per vertex.
fn triangles(positions: &[Vec3], normals: Option<&[Vec3]>, indices: &[u32], out: &mut Mesh) {
    let smooth: Vec<Vec3>;
    let normals = match normals {
        Some(n) if n.len() == positions.len() => n,
        _ => {
            let mut acc = vec![Vec3::ZERO; positions.len()];
            for t in indices.as_chunks::<3>().0 {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| positions[i as usize]);
                let n = (b - a).cross(c - a); // area weighted
                for &i in t {
                    acc[i as usize] += n;
                }
            }
            smooth = acc.into_iter().map(|n| n.normalize_or(Vec3::Y)).collect();
            &smooth
        }
    };
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| positions[i as usize]);
        if (b - a).cross(c - a).length_squared() < 1e-20 {
            continue;
        }
        for &i in t {
            out.push((
                positions[i as usize],
                normals[i as usize].normalize_or(Vec3::Y),
            ));
        }
    }
}
fn read_obj(path: &Path) -> Result<Vec<RawPart>> {
    let options = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ..Default::default()
    };
    let (models, materials) =
        tobj::load_obj(path, &options).with_context(|| format!("Reading {}", path.display()))?;
    let materials = materials.unwrap_or_default();
    let (mut keys, mut parts) = (vec![], vec![]);
    for m in &models {
        let id = m.mesh.material_id;
        let i = part_for(&mut keys, &mut parts, id, || {
            match id.and_then(|i| materials.get(i)) {
                Some(mat) => (mat.name.clone(), obj_material(mat)),
                None => ("default".into(), None),
            }
        });
        let p: Vec<Vec3> = m
            .mesh
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| Vec3::new(v[0], v[1], v[2]))
            .collect();
        let n: Vec<Vec3> = m
            .mesh
            .normals
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| Vec3::new(v[0], v[1], v[2]))
            .collect();
        triangles(
            &p,
            (!n.is_empty()).then_some(n.as_slice()),
            &m.mesh.indices,
            &mut parts[i].0,
        );
    }
    Ok(parts)
}
fn obj_material(m: &tobj::Material) -> Option<Material> {
    m.diffuse.map(|d| Material {
        color: d.map(|v| v.clamp(0., 1.)),
        roughness: m
            .shininess
            .map_or(0.5, |s| (1. - (s / 1000.).sqrt()).clamp(0.05, 1.)),
        ..Material::default()
    })
}
/// Mean linear RGB of a texture's image (sRGB decoded when `srgb`), from up to ~64k texels.
/// None for formats other than 8-bit RGB / RGBA or images that fail to load.
fn average_texture(
    texture: &gltf::Texture,
    buffers: &[gltf::buffer::Data],
    path: &Path,
    srgb: bool,
) -> Option<[f32; 3]> {
    use gltf::image::Format;
    let image =
        gltf::image::Data::from_source(texture.source().source(), path.parent(), buffers).ok()?;
    let channels = match image.format {
        Format::R8G8B8 => 3,
        Format::R8G8B8A8 => 4,
        _ => return None,
    };
    let texels = image.pixels.len() / channels;
    let step = (texels / 65536).max(1);
    let decode = |v: u8| {
        let v = v as f32 / 255.;
        if srgb {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        } else {
            v
        }
    };
    let mut sum = [0f64; 3];
    let mut count = 0;
    for texel in image.pixels.chunks_exact(channels).step_by(step) {
        (0..3).for_each(|k| sum[k] += decode(texel[k]) as f64);
        count += 1;
    }
    (count > 0).then(|| sum.map(|s| (s / count as f64) as f32))
}
fn read_gltf(path: &Path) -> Result<Vec<RawPart>> {
    let gltf = gltf::Gltf::open(path).with_context(|| format!("Reading {}", path.display()))?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone())
        .context("Loading glTF buffers")?;
    let (mut keys, mut parts): (Vec<Option<usize>>, Vec<RawPart>) = (vec![], vec![]);
    let mut total = 0;
    let scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next())
        .context("glTF has no scene")?;
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
            for prim in mesh
                .primitives()
                .filter(|p| p.mode() == gltf::mesh::Mode::Triangles)
            {
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let p: Vec<Vec3> = positions
                    .map(|v| world.transform_point3(Vec3::from(v)))
                    .collect();
                let n: Option<Vec<Vec3>> = reader.read_normals().map(|ns| {
                    ns.map(|v| (normal_matrix * Vec3::from(v)).normalize_or(Vec3::Y))
                        .collect()
                });
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..p.len() as u32).collect(),
                };
                let m = prim.material();
                let i = part_for(&mut keys, &mut parts, m.index(), || {
                    let name = m.name().map_or_else(
                        || {
                            m.index()
                                .map_or("default".into(), |i| format!("material {i}"))
                        },
                        String::from,
                    );
                    let pbr = m.pbr_metallic_roughness();
                    // Textures are not sampled yet (core track 5): their average stands in, times
                    // the factors. Metallic is B and roughness G of the metallic-roughness map.
                    let mut c = pbr.base_color_factor();
                    if let Some(avg) = pbr
                        .base_color_texture()
                        .and_then(|t| average_texture(&t.texture(), &buffers, path, true))
                    {
                        (0..3).for_each(|k| c[k] *= avg[k]);
                    }
                    let (mut metallic, mut roughness) =
                        (pbr.metallic_factor(), pbr.roughness_factor());
                    if let Some(avg) = pbr
                        .metallic_roughness_texture()
                        .and_then(|t| average_texture(&t.texture(), &buffers, path, false))
                    {
                        metallic *= avg[2];
                        roughness *= avg[1];
                    }
                    let mut e = m.emissive_factor();
                    if let Some(avg) = m
                        .emissive_texture()
                        .and_then(|t| average_texture(&t.texture(), &buffers, path, true))
                    {
                        (0..3).for_each(|k| e[k] *= avg[k]);
                    }
                    let material = Material {
                        color: [c[0], c[1], c[2]].map(|v| v.clamp(0., 1.)),
                        metallic: metallic.clamp(0., 1.),
                        roughness: roughness.clamp(0., 1.),
                        emission: (e[0].max(e[1]).max(e[2]) * m.emissive_strength().unwrap_or(1.))
                            .clamp(0., 20.),
                        transmission: m
                            .transmission()
                            .map_or(0., |t| t.transmission_factor())
                            .clamp(0., 1.),
                        ior: m.ior().unwrap_or(1.5).clamp(1., 2.5),
                        ..Material::default()
                    };
                    (name, Some(material))
                });
                let before = parts[i].0.len();
                triangles(&p, n.as_deref(), &indices, &mut parts[i].0);
                total += (parts[i].0.len() - before) / 3;
                ensure!(
                    total <= MAX_TRIANGLES,
                    "{} has more than 20M triangles",
                    path.display()
                );
            }
        }
        stack.extend(node.children().map(|c| (c, world)));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    #[test]
    fn obj_files_load_and_normalize() {
        let p = temp(
            "cornell-test-quad.obj",
            "v 0 0 0\nv 4 0 0\nv 4 2 0\nv 0 2 0\nf 1 2 3 4\n",
        );
        let m = read(&p).unwrap().whole();
        assert_eq!(m.len(), 6, "a quad is two triangles");
        let max = m
            .iter()
            .map(|(p, _)| p.abs().max_element())
            .fold(0., f32::max);
        assert!((max - 1.).abs() < 1e-5, "largest half-extent is 1: {max}");
        assert!(
            m.iter().all(|(_, n)| (n.z.abs() - 1.).abs() < 1e-5),
            "smooth normals of a flat quad face z"
        );
    }
    #[test]
    fn gltf_files_load_with_transforms_and_material() {
        // One triangle, translated by a node, with an embedded buffer (3 vec3 positions).
        let floats: [f32; 9] = [0., 0., 0., 1., 0., 0., 0., 1., 0.];
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        let b64 = {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut s = String::new();
            for c in bytes.chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | *c.get(2).unwrap_or(&0) as u32;
                for k in 0..4 {
                    s.push(if k <= c.len() {
                        T[(n >> (18 - 6 * k) & 63) as usize] as char
                    } else {
                        '='
                    });
                }
            }
            s
        };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"mesh":0,"translation":[5,0,0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[0.8,0.1,0.1,1],"metallicFactor":1,"roughnessFactor":0.2}}}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}}],
            "buffers":[{{"byteLength":36,"uri":"data:application/octet-stream;base64,{b64}"}}]}}"#
        );
        let p = temp("cornell-test-triangle.gltf", &json);
        let m = read(&p).unwrap();
        assert_eq!(m.whole().len(), 3);
        let mat = m.material().unwrap();
        assert!(
            mat.metallic == 1.
                && (mat.color[0] - 0.8).abs() < 1e-6
                && (mat.roughness - 0.2).abs() < 1e-6
        );
        assert!(read(&temp("cornell-test.stl", "solid")).is_err());
    }
    #[test]
    fn materials_split_into_parts_that_share_the_file_box() {
        let dir = std::env::temp_dir().join("cornell-test-parts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("two.mtl"),
            "newmtl red\nKd 1 0 0\nnewmtl blue\nKd 0 0 1\n",
        )
        .unwrap();
        // A small red triangle near the origin and a large blue one far away.
        std::fs::write(
            dir.join("two.obj"),
            "mtllib two.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 8 0 0\nv 10 0 0\nv 8 2 0\n\
             usemtl red\nf 1 2 3\nusemtl blue\nf 4 5 6\n",
        )
        .unwrap();
        let m = read(&dir.join("two.obj")).unwrap();
        assert_eq!(m.parts.len(), 2);
        assert_eq!(m.parts[0].name, "red");
        assert_eq!(m.parts[1].material.as_ref().unwrap().color, [0., 0., 1.]);
        // The file spans x 0..10, so the red triangle lands at the left end of -1..1.
        assert!(m.parts[0].mesh.iter().all(|(p, _)| p.x <= -0.79));
        assert_eq!(m.whole().len(), 6);
        assert_eq!(split("a/b.glb#12"), ("a/b.glb", Some(12)));
        assert_eq!(split("a#b.obj"), ("a#b.obj", None));
    }
}
