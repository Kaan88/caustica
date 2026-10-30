//! Imported meshes (roadmap core track 2): OBJ through tobj, glTF / GLB through gltf. A mesh is
//! centred and scaled into the same -1..1 box as the built-in shapes, so an object's size means
//! the same for both. Loads (and failures) are cached per resolved path.
use crate::scene::{Material, Object, Shape};
use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// Triangle soup: three (position, normal) pairs per triangle, like the built-in shapes.
pub type Mesh = Vec<(Vec3, Vec3)>;
pub const MAX_TRIANGLES: usize = 20_000_000;
const MAX_FILE: u64 = 2 << 30;

/// What a file says about its surface, for the object made from it.
pub struct Imported {
    pub mesh: Arc<Mesh>,
    pub material: Option<Material>,
}
type Cache = HashMap<PathBuf, Result<Arc<Imported>, String>>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// Relative paths are relative to the scenes folder (where scene files and `meshes/` live).
pub fn resolve(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
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
pub fn load(path: &str) -> Result<Arc<Imported>> {
    let key = resolve(path);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(&key) {
        return hit.clone().map_err(anyhow::Error::msg);
    }
    let result = read(&key).map(Arc::new).map_err(|e| format!("{e:#}"));
    cache.insert(key, result.clone());
    result.map_err(anyhow::Error::msg)
}
/// The mesh of an object, or None for built-in shapes and unreadable files.
pub fn of(o: &Object) -> Option<Arc<Mesh>> {
    (!o.mesh.is_empty())
        .then(|| load(&o.mesh).ok().map(|m| m.mesh.clone()))
        .flatten()
}
/// A new object for a mesh file, with the file's material when it has one.
pub fn import_object(path: &Path) -> Result<Object> {
    let stored = store(path);
    let imported = load(&stored)?;
    let name = path
        .file_stem()
        .map_or("Mesh".into(), |s| s.to_string_lossy().to_string());
    Ok(Object {
        name,
        shape: Shape::Box,
        mesh: stored,
        position: [0., 0.],
        size: 2.,
        stretch: 1.,
        rotation: 0.,
        material: imported.material.clone().unwrap_or_default(),
        ..Object::default()
    })
}
fn read(path: &Path) -> Result<Imported> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("{}", path.display()))?
        .len();
    ensure!(size <= MAX_FILE, "{} is larger than 2 GB", path.display());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (mut mesh, material) = match ext.as_str() {
        "obj" => read_obj(path)?,
        "gltf" | "glb" => read_gltf(path)?,
        _ => bail!("Unsupported mesh format .{ext} (OBJ, glTF, GLB)"),
    };
    ensure!(!mesh.is_empty(), "{} has no triangles", path.display());
    ensure!(
        mesh.len() / 3 <= MAX_TRIANGLES,
        "{} has {} triangles (limit 20M)",
        path.display(),
        mesh.len() / 3
    );
    normalize(&mut mesh);
    Ok(Imported {
        mesh: Arc::new(mesh),
        material,
    })
}
/// Centre on the bounding box and scale the largest half-extent to 1.
fn normalize(mesh: &mut Mesh) {
    let (lo, hi) = mesh.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), (p, _)| (lo.min(*p), hi.max(*p)),
    );
    let centre = (lo + hi) * 0.5;
    let scale = 1. / ((hi - lo) * 0.5).max_element().max(1e-12);
    for (p, _) in mesh.iter_mut() {
        *p = (*p - centre) * scale;
    }
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
fn read_obj(path: &Path) -> Result<(Mesh, Option<Material>)> {
    let options = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ..Default::default()
    };
    let (models, materials) =
        tobj::load_obj(path, &options).with_context(|| format!("Reading {}", path.display()))?;
    let mut out = Mesh::new();
    for m in &models {
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
            &mut out,
        );
    }
    let material = materials
        .ok()
        .and_then(|ms| ms.into_iter().next())
        .and_then(|m| {
            m.diffuse.map(|d| Material {
                color: d.map(|v| v.clamp(0., 1.)),
                roughness: m
                    .shininess
                    .map_or(0.5, |s| (1. - (s / 1000.).sqrt()).clamp(0.05, 1.)),
                ..Material::default()
            })
        });
    Ok((out, material))
}
fn read_gltf(path: &Path) -> Result<(Mesh, Option<Material>)> {
    let gltf = gltf::Gltf::open(path).with_context(|| format!("Reading {}", path.display()))?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone())
        .context("Loading glTF buffers")?;
    let mut out = Mesh::new();
    let mut material = None;
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
                triangles(&p, n.as_deref(), &indices, &mut out);
                ensure!(
                    out.len() / 3 <= MAX_TRIANGLES,
                    "{} has more than 20M triangles",
                    path.display()
                );
                if material.is_none() {
                    let m = prim.material();
                    let pbr = m.pbr_metallic_roughness();
                    let c = pbr.base_color_factor();
                    let e = m.emissive_factor();
                    material = Some(Material {
                        color: [c[0], c[1], c[2]].map(|v| v.clamp(0., 1.)),
                        metallic: pbr.metallic_factor().clamp(0., 1.),
                        roughness: pbr.roughness_factor().clamp(0., 1.),
                        emission: (e[0].max(e[1]).max(e[2]) * m.emissive_strength().unwrap_or(1.))
                            .clamp(0., 20.),
                        transmission: m
                            .transmission()
                            .map_or(0., |t| t.transmission_factor())
                            .clamp(0., 1.),
                        ior: m.ior().unwrap_or(1.5).clamp(1., 2.5),
                        ..Material::default()
                    });
                }
            }
        }
        stack.extend(node.children().map(|c| (c, world)));
    }
    Ok((out, material))
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
        let m = read(&p).unwrap();
        assert_eq!(m.mesh.len(), 6, "a quad is two triangles");
        let max = m
            .mesh
            .iter()
            .map(|(p, _)| p.abs().max_element())
            .fold(0., f32::max);
        assert!((max - 1.).abs() < 1e-5, "largest half-extent is 1: {max}");
        assert!(
            m.mesh.iter().all(|(_, n)| (n.z.abs() - 1.).abs() < 1e-5),
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
        assert_eq!(m.mesh.len(), 3);
        let mat = m.material.unwrap();
        assert!(
            mat.metallic == 1.
                && (mat.color[0] - 0.8).abs() < 1e-6
                && (mat.roughness - 0.2).abs() < 1e-6
        );
        assert!(read(&temp("cornell-test.stl", "solid")).is_err());
    }
}
