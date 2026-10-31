//! Imported meshes (roadmap core track 2): OBJ through tobj, glTF / GLB through gltf. A mesh is
//! centred and scaled into the same -1..1 box as the built-in shapes, so an object's size means
//! the same for both. Loads (and failures) are cached per resolved path.
//!
//! A file with several materials is split into parts, one per material, addressed as
//! `file.glb#2`. All parts share the file's box, so objects with the same transform line up.
//!
//! Parts keep their texture coordinates and decoded texture maps (core track 5): base colour,
//! tangent-space normal and metallic-roughness, plus an alpha cutoff for masked materials.
use crate::scene::{Material, Object, Shape};
use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// Triangle soup: three (position, normal) pairs per triangle, like the built-in shapes.
pub type Mesh = Vec<(Vec3, Vec3)>;
/// Texture coordinates, one per vertex of a `Mesh` (zero where the file has none).
pub type Uvs = Vec<[f32; 2]>;
pub const MAX_TRIANGLES: usize = 20_000_000;
const MAX_FILE: u64 = 2 << 30;
/// Larger maps are halved until they fit (D3D12 allows 16384; this keeps memory sane).
const MAX_MAP: u32 = 8192;

/// A decoded texture: RGBA8 rows, its mip chain and the mean of each channel (linear, sRGB
/// decoded when `srgb`; alpha is never decoded).
pub struct Map {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Levels 1.. down to 1×1, each a 2×2 box filter of the last (in the stored encoding, so
    /// sRGB maps are filtered in sRGB: slightly dark, as in most engines).
    pub mips: Vec<Vec<u8>>,
    pub srgb: bool,
    pub average: [f32; 4],
}
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
impl Map {
    pub fn new(mut width: u32, mut height: u32, mut rgba: Vec<u8>, srgb: bool) -> Self {
        while width.max(height) > MAX_MAP && width > 1 && height > 1 {
            (width, height, rgba) = halve(width, height, &rgba);
        }
        // The mean of up to ~64k texels.
        let texels = rgba.len() / 4;
        let step = (texels / 65536).max(1);
        let mut sum = [0f64; 4];
        let mut count = 0;
        for t in rgba.as_chunks::<4>().0.iter().step_by(step) {
            for k in 0..4 {
                let v = t[k] as f32 / 255.;
                sum[k] += if srgb && k < 3 { srgb_to_linear(v) } else { v } as f64;
            }
            count += 1;
        }
        let average = sum.map(|s| (s / count.max(1) as f64) as f32);
        let mut mips: Vec<Vec<u8>> = vec![];
        let (mut w, mut h) = (width, height);
        while w > 1 || h > 1 {
            let next;
            (w, h, next) = halve(w, h, mips.last().unwrap_or(&rgba));
            mips.push(next);
        }
        Self {
            width,
            height,
            rgba,
            mips,
            srgb,
            average,
        }
    }
    /// A tangent-space normal map averages near (0.5, 0.5, 1); grey height maps (OBJ `bump`
    /// often names one) do not, and are ignored.
    pub fn looks_like_normals(&self) -> bool {
        let [r, g, b, _] = self.average;
        (r - 0.5).abs() < 0.15 && (g - 0.5).abs() < 0.15 && b > 0.6 && b > r + 0.15
    }
}
/// 2×2 box filter to D3D's next mip size (odd edges drop their last row or column; a side of
/// 1 stays 1).
fn halve(w: u32, h: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        for x in 0..nw {
            for k in 0..4 {
                let at = |dx: u32, dy: u32| {
                    let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                    rgba[((sy * w + sx) * 4 + k) as usize]
                };
                let sum = at(0, 0) as u32 + at(1, 0) as u32 + at(0, 1) as u32 + at(1, 1) as u32;
                out.push(((sum + 2) / 4) as u8);
            }
        }
    }
    (nw, nh, out)
}
/// A part's texture maps (glTF conventions: roughness in G and metallic in B of
/// `metal_rough`; normals with +Y up).
#[derive(Clone, Default)]
pub struct Maps {
    pub base: Option<Arc<Map>>,
    pub normal: Option<Arc<Map>>,
    pub metal_rough: Option<Arc<Map>>,
    /// Base-colour alpha below this is a hole (glTF MASK, and BLEND treated as MASK at 0.5),
    /// already divided by the base colour factor's alpha. 0 = opaque.
    pub cutoff: f32,
}
impl Maps {
    pub fn is_empty(&self) -> bool {
        self.base.is_none() && self.normal.is_none() && self.metal_rough.is_none()
    }
}

/// One material's triangles of a file.
pub struct Part {
    pub name: String,
    pub mesh: Arc<Mesh>,
    pub uv: Arc<Uvs>,
    pub material: Option<Material>,
    pub maps: Maps,
}
/// A loaded file: its parts (at least one), and every part merged, built on first use.
pub struct Imported {
    pub parts: Vec<Part>,
    whole: OnceLock<(Arc<Mesh>, Arc<Uvs>)>,
}
impl Imported {
    fn whole_parts(&self) -> &(Arc<Mesh>, Arc<Uvs>) {
        self.whole.get_or_init(|| match &self.parts[..] {
            [one] => (one.mesh.clone(), one.uv.clone()),
            parts => (
                Arc::new(parts.iter().flat_map(|p| p.mesh.iter().copied()).collect()),
                Arc::new(parts.iter().flat_map(|p| p.uv.iter().copied()).collect()),
            ),
        })
    }
    /// All parts as one mesh (a file used without `#part`).
    pub fn whole(&self) -> Arc<Mesh> {
        self.whole_parts().0.clone()
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
/// The part an object uses, or None for the whole file, built-in shapes and unreadable files.
fn part_of(o: &Object) -> Option<(Arc<Imported>, Option<usize>)> {
    if o.mesh.is_empty() {
        return None;
    }
    Some((load(&o.mesh).ok()?, split(&o.mesh).1))
}
/// The mesh of an object, or None for built-in shapes, unreadable files and missing parts.
pub fn of(o: &Object) -> Option<Arc<Mesh>> {
    match part_of(o)? {
        (imported, Some(n)) => imported.parts.get(n).map(|p| p.mesh.clone()),
        (imported, None) => Some(imported.whole()),
    }
}
/// Texture coordinates of an object's mesh, parallel to `of`.
pub fn uvs(o: &Object) -> Option<Arc<Uvs>> {
    match part_of(o)? {
        (imported, Some(n)) => imported.parts.get(n).map(|p| p.uv.clone()),
        (imported, None) => Some(imported.whole_parts().1.clone()),
    }
}
/// The texture maps an object shows: its part's, when its material has maps switched on.
/// A whole multi-material file has none (one material per object).
pub fn maps(o: &Object) -> Option<Maps> {
    o.material.textures.then(|| file_maps(o)).flatten()
}
/// Whether an object's mesh has texture maps at all (whatever its material says).
pub fn has_maps(o: &Object) -> bool {
    file_maps(o).is_some()
}
fn file_maps(o: &Object) -> Option<Maps> {
    let (imported, n) = part_of(o)?;
    let part = match n {
        Some(n) => imported.parts.get(n)?,
        None if imported.parts.len() == 1 => &imported.parts[0],
        None => return None,
    };
    (!part.maps.is_empty()).then(|| part.maps.clone())
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
    let triangles: usize = parts.iter().map(|p| p.mesh.len() / 3).sum();
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
            .map(|p| Part {
                name: p.name,
                mesh: Arc::new(p.mesh),
                uv: Arc::new(p.uv),
                material: p.material,
                maps: p.maps,
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
            let mut part = RawPart {
                name,
                ..RawPart::default()
            };
            match geometry {
                Geometry::Indexed(p, idx) => triangles(&p, None, None, &idx, &mut part),
                Geometry::Flat(tris) => {
                    part.uv = vec![[0.; 2]; tris.len()];
                    part.mesh = tris;
                }
            }
            part
        })
        .collect())
}
/// Centre every part on the file's bounding box and scale its largest half-extent to 1.
fn normalize(parts: &mut [RawPart]) {
    let (lo, hi) = parts.iter().flat_map(|p| &p.mesh).fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), (p, _)| (lo.min(*p), hi.max(*p)),
    );
    let centre = (lo + hi) * 0.5;
    let scale = 1. / ((hi - lo) * 0.5).max_element().max(1e-12);
    for (p, _) in parts.iter_mut().flat_map(|p| p.mesh.iter_mut()) {
        *p = (*p - centre) * scale;
    }
}
/// A part as read.
#[derive(Default)]
struct RawPart {
    mesh: Mesh,
    uv: Uvs,
    name: String,
    material: Option<Material>,
    maps: Maps,
}
/// The part for a material key, created in first-seen order.
fn part_for<K: PartialEq>(
    keys: &mut Vec<K>,
    parts: &mut Vec<RawPart>,
    key: K,
    make: impl FnOnce() -> (String, Option<Material>, Maps),
) -> usize {
    if let Some(i) = keys.iter().position(|k| *k == key) {
        return i;
    }
    let (name, material, maps) = make();
    keys.push(key);
    parts.push(RawPart {
        name,
        material,
        maps,
        ..RawPart::default()
    });
    parts.len() - 1
}
/// Triangles from indexed positions, with given normals or smooth ones averaged per vertex,
/// and texture coordinates when the file has them.
fn triangles(
    positions: &[Vec3],
    normals: Option<&[Vec3]>,
    uvs: Option<&[[f32; 2]]>,
    indices: &[u32],
    out: &mut RawPart,
) {
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
    let uvs = uvs.filter(|u| u.len() == positions.len());
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| positions[i as usize]);
        if (b - a).cross(c - a).length_squared() < 1e-20 {
            continue;
        }
        for &i in t {
            out.mesh.push((
                positions[i as usize],
                normals[i as usize].normalize_or(Vec3::Y),
            ));
            out.uv.push(uvs.map_or([0.; 2], |u| u[i as usize]));
        }
    }
}
/// Decodes images on all cores (a chess set has 30 maps of 2–4k).
pub fn decode_all<K: Clone + Send + Sync + Eq + std::hash::Hash>(
    keys: Vec<K>,
    decode: impl Fn(&K) -> Option<Map> + Sync,
) -> HashMap<K, Arc<Map>> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = Mutex::new(HashMap::new());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(keys.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(key) = keys.get(i) else { break };
                    if let Some(map) = decode(key) {
                        out.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(key.clone(), Arc::new(map));
                    }
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(|e| e.into_inner())
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
    let dir = path.parent().unwrap_or(Path::new("."));
    // MTL texture paths are relative to the OBJ; base colour is sRGB, normals linear.
    let mut wanted: Vec<(PathBuf, bool)> = vec![];
    for m in &materials {
        wanted.extend(m.diffuse_texture.iter().map(|t| (dir.join(t), true)));
        wanted.extend(m.normal_texture.iter().map(|t| (dir.join(t), false)));
    }
    wanted.sort();
    wanted.dedup();
    let images = decode_all(wanted, |(file, srgb)| {
        let image = image::open(file).ok()?.to_rgba8();
        Some(Map::new(
            image.width(),
            image.height(),
            image.into_raw(),
            *srgb,
        ))
    });
    let (mut keys, mut parts) = (vec![], vec![]);
    for m in &models {
        let id = m.mesh.material_id;
        let i = part_for(&mut keys, &mut parts, id, || {
            match id.and_then(|i| materials.get(i)) {
                Some(mat) => {
                    let map = |t: &Option<String>, srgb: bool| {
                        t.as_ref()
                            .and_then(|t| images.get(&(dir.join(t), srgb)).cloned())
                    };
                    let maps = Maps {
                        base: map(&mat.diffuse_texture, true),
                        normal: map(&mat.normal_texture, false).filter(|n| n.looks_like_normals()),
                        ..Maps::default()
                    };
                    (mat.name.clone(), obj_material(mat, &maps), maps)
                }
                None => ("default".into(), None, Maps::default()),
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
        // OBJ puts v = 0 at the bottom of the image, D3D at the top.
        let uv: Vec<[f32; 2]> = m
            .mesh
            .texcoords
            .as_chunks::<2>()
            .0
            .iter()
            .map(|t| [t[0], 1. - t[1]])
            .collect();
        triangles(
            &p,
            (!n.is_empty()).then_some(n.as_slice()),
            (!uv.is_empty()).then_some(uv.as_slice()),
            &m.mesh.indices,
            &mut parts[i],
        );
    }
    Ok(parts)
}
/// The MTL diffuse colour times the diffuse map's average (the map is then scaled by
/// colour / average on the GPU, see `Material::textures`).
fn obj_material(m: &tobj::Material, maps: &Maps) -> Option<Material> {
    let average = maps
        .base
        .as_ref()
        .map(|b| [b.average[0], b.average[1], b.average[2]]);
    if m.diffuse.is_none() && average.is_none() {
        return None;
    }
    let d = m.diffuse.unwrap_or([1.; 3]);
    let a = average.unwrap_or([1.; 3]);
    Some(Material {
        color: [0, 1, 2].map(|k| (d[k] * a[k]).clamp(0., 1.)),
        roughness: m
            .shininess
            .map_or(0.5, |s| (1. - (s / 1000.).sqrt()).clamp(0.05, 1.)),
        ..Material::default()
    })
}
/// RGBA8 from the layouts gltf decodes to (16-bit channels keep their high byte; float
/// images are not supported as maps).
fn rgba8(image: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let (channels, bytes) = match image.format {
        Format::R8 => (1, 1),
        Format::R8G8 => (2, 1),
        Format::R8G8B8 => (3, 1),
        Format::R8G8B8A8 => (4, 1),
        Format::R16 => (1, 2),
        Format::R16G16 => (2, 2),
        Format::R16G16B16 => (3, 2),
        Format::R16G16B16A16 => (4, 2),
        _ => return None,
    };
    let texel = channels * bytes;
    let mut out = Vec::with_capacity(image.pixels.len() / texel * 4);
    for t in image.pixels.chunks_exact(texel) {
        let c = |k: usize| t[k * bytes + bytes - 1];
        out.extend_from_slice(&match channels {
            1 => [c(0), c(0), c(0), 255],
            2 => [c(0), c(0), c(0), c(1)],
            3 => [c(0), c(1), c(2), 255],
            _ => [c(0), c(1), c(2), c(3)],
        });
    }
    Some(out)
}
/// KHR_materials_clearcoat and _sheen factors (track 11) over the default material; their
/// texture maps are not read. gltf 1.4 has no typed access, so they come from the raw JSON.
fn layers(m: &gltf::Material) -> Material {
    let number = |ext: &str, key: &str| {
        m.extension_value(ext)
            .and_then(|e| e.get(key))
            .and_then(|v| v.as_f64())
            .map(|v| (v as f32).clamp(0., 1.))
    };
    // A key missing from a present extension takes glTF's default (0); without the extension
    // the fields keep the app's defaults, for when the coat or sheen is turned on later.
    let factor = |ext: &str, key: &str, default: f32| {
        number(ext, key).unwrap_or(if m.extension_value(ext).is_some() {
            0.
        } else {
            default
        })
    };
    let d = Material::default();
    let mut out = Material {
        clearcoat: number("KHR_materials_clearcoat", "clearcoatFactor").unwrap_or(0.),
        clearcoat_roughness: factor(
            "KHR_materials_clearcoat",
            "clearcoatRoughnessFactor",
            d.clearcoat_roughness,
        ),
        sheen_roughness: factor(
            "KHR_materials_sheen",
            "sheenRoughnessFactor",
            d.sheen_roughness,
        ),
        ..d
    };
    // The sheen colour splits into a strength (its largest channel) and a normalised colour.
    if let Some(c) = m
        .extension_value("KHR_materials_sheen")
        .and_then(|e| e.get("sheenColorFactor"))
        .and_then(|v| v.as_array())
        .filter(|c| c.len() == 3)
    {
        let c = [0, 1, 2].map(|i| (c[i].as_f64().unwrap_or(0.) as f32).clamp(0., 1.));
        let s = c[0].max(c[1]).max(c[2]);
        if s > 0. {
            out.sheen = s;
            out.sheen_color = c.map(|v| v / s);
        }
    }
    out
}
fn read_gltf(path: &Path) -> Result<Vec<RawPart>> {
    let gltf = gltf::Gltf::open(path).with_context(|| format!("Reading {}", path.display()))?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone())
        .context("Loading glTF buffers")?;
    // Every image a material uses, decoded once per colour space (base colour is sRGB,
    // normals and metallic-roughness linear).
    let mut wanted: Vec<(usize, bool)> = vec![];
    for m in gltf.document.materials() {
        let pbr = m.pbr_metallic_roughness();
        let mut want = |t: Option<gltf::Texture>, srgb: bool| {
            wanted.extend(t.map(|t| (t.source().index(), srgb)));
        };
        want(pbr.base_color_texture().map(|t| t.texture()), true);
        want(pbr.metallic_roughness_texture().map(|t| t.texture()), false);
        want(m.normal_texture().map(|t| t.texture()), false);
        want(m.emissive_texture().map(|t| t.texture()), true);
    }
    wanted.sort_unstable();
    wanted.dedup();
    let images: Vec<gltf::Image> = gltf.document.images().collect();
    let maps = decode_all(wanted, |&(index, srgb)| {
        let data =
            gltf::image::Data::from_source(images[index].source(), path.parent(), &buffers).ok()?;
        Some(Map::new(data.width, data.height, rgba8(&data)?, srgb))
    });
    let map = |t: Option<gltf::Texture>, srgb: bool| -> Option<Arc<Map>> {
        maps.get(&(t?.source().index(), srgb)).cloned()
    };
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
                let uv: Option<Vec<[f32; 2]>> =
                    reader.read_tex_coords(0).map(|t| t.into_f32().collect());
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
                    let maps = Maps {
                        base: map(pbr.base_color_texture().map(|t| t.texture()), true),
                        normal: map(m.normal_texture().map(|t| t.texture()), false)
                            .filter(|n| n.looks_like_normals()),
                        metal_rough: map(
                            pbr.metallic_roughness_texture().map(|t| t.texture()),
                            false,
                        ),
                        cutoff: 0.,
                    };
                    // The values are the factors times the map averages; the GPU scales each
                    // map by value / average (`Material::textures`). Metallic is B and
                    // roughness G of the metallic-roughness map.
                    let mut c = pbr.base_color_factor();
                    if let Some(b) = &maps.base {
                        (0..3).for_each(|k| c[k] *= b.average[k]);
                    }
                    let (mut metallic, mut roughness) =
                        (pbr.metallic_factor(), pbr.roughness_factor());
                    if let Some(mr) = &maps.metal_rough {
                        metallic *= mr.average[2];
                        roughness *= mr.average[1];
                    }
                    // Emission stays a flat colour (light sampling has no texture lookup).
                    let mut e = m.emissive_factor();
                    if let Some(avg) = map(m.emissive_texture().map(|t| t.texture()), true) {
                        (0..3).for_each(|k| e[k] *= avg.average[k]);
                    }
                    let cutoff = match m.alpha_mode() {
                        gltf::material::AlphaMode::Opaque => 0.,
                        gltf::material::AlphaMode::Mask => m.alpha_cutoff().unwrap_or(0.5),
                        gltf::material::AlphaMode::Blend => 0.5,
                    };
                    let maps = Maps {
                        cutoff: if maps.base.is_some() {
                            cutoff / pbr.base_color_factor()[3].max(1e-3)
                        } else {
                            0.
                        },
                        ..maps
                    };
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
                        ..layers(&m)
                    };
                    (name, Some(material), maps)
                });
                let before = parts[i].mesh.len();
                triangles(&p, n.as_deref(), uv.as_deref(), &indices, &mut parts[i]);
                total += (parts[i].mesh.len() - before) / 3;
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
/// Standard base64 (for the embedded buffers and images of `write_test_gltf`).
fn base64(bytes: &[u8]) -> String {
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
}
/// A textured glTF quad for the tests and the self-test: a 2×2 base colour map (red, green,
/// blue and one transparent texel, alpha MASK at 0.5) and a flat 2×2 normal map.
pub fn write_test_gltf(path: &Path) -> Result<()> {
    let png = |texels: [[u8; 4]; 4]| -> Result<String> {
        let image = image::RgbaImage::from_raw(2, 2, texels.concat()).context("2×2 image")?;
        let mut out = std::io::Cursor::new(vec![]);
        image.write_to(&mut out, image::ImageFormat::Png)?;
        Ok(format!(
            "data:image/png;base64,{}",
            base64(&out.into_inner())
        ))
    };
    let base = png([
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 0],
    ])?;
    let normal = png([[128, 128, 255, 255]; 4])?;
    let mut bytes: Vec<u8> = vec![];
    let floats = |b: &mut Vec<u8>, v: &[f32]| b.extend(v.iter().flat_map(|f| f.to_le_bytes()));
    floats(
        &mut bytes,
        &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
    );
    floats(
        &mut bytes,
        &[0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.],
    );
    floats(&mut bytes, &[0., 1., 1., 1., 1., 0., 0., 0.]);
    bytes.extend([0u32, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()));
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3,"material":0}}]}}],
        "materials":[{{"name":"tiles","alphaMode":"MASK","alphaCutoff":0.5,"normalTexture":{{"index":1}},
            "pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}},"metallicFactor":0}}}}],
        "textures":[{{"source":0}},{{"source":1}}],
        "images":[{{"uri":"{base}"}},{{"uri":"{normal}"}}],
        "accessors":[
            {{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
            {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},
            {{"bufferView":2,"componentType":5126,"count":4,"type":"VEC2"}},
            {{"bufferView":3,"componentType":5125,"count":6,"type":"SCALAR"}}],
        "bufferViews":[{{"buffer":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},
            {{"buffer":0,"byteOffset":96,"byteLength":32}},{{"buffer":0,"byteOffset":128,"byteLength":24}}],
        "buffers":[{{"byteLength":152,"uri":"data:application/octet-stream;base64,{}"}}]}}"#,
        base64(&bytes)
    );
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    /// Khronos' ToyCar has a clear-coated body and a sheen fabric (track 11); skipped when the
    /// asset browser has not downloaded it.
    #[test]
    fn gltf_layers_import() {
        let Some(path) = crate::scenes::folder().map(|f| f.join("models/ToyCar.glb")) else {
            return;
        };
        if !path.exists() {
            return;
        }
        let parts = &read(&path).unwrap().parts;
        let materials: Vec<&Material> = parts.iter().filter_map(|p| p.material.as_ref()).collect();
        assert!(
            materials.iter().any(|m| m.clearcoat > 0.),
            "a clear-coated part"
        );
        assert!(materials.iter().any(|m| m.sheen > 0.), "a sheen part");
        assert!(materials.iter().all(|m| (0. ..=1.).contains(&m.sheen)));
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
        let b64 = base64(&bytes);
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
    fn gltf_texture_maps_load_with_uvs_and_averages() {
        let p = std::env::temp_dir().join("cornell-test-textured.gltf");
        write_test_gltf(&p).unwrap();
        let m = read(&p).unwrap();
        let part = &m.parts[0];
        assert_eq!(part.uv.len(), 6, "one UV per vertex");
        assert_eq!(part.uv[2], [1., 0.], "UVs follow the indices");
        let base = part.maps.base.as_ref().unwrap();
        assert_eq!((base.width, base.height, base.srgb), (2, 2, true));
        // sRGB-decoded mean of red, green, blue and white is 0.5 per channel, and the colour
        // is the factor (1) times that mean.
        for k in 0..3 {
            assert!((base.average[k] - 0.5).abs() < 1e-3, "{:?}", base.average);
            assert!((part.material.as_ref().unwrap().color[k] - 0.5).abs() < 1e-3);
        }
        assert_eq!(part.maps.cutoff, 0.5);
        assert!(part.maps.normal.is_some(), "a flat normal map is accepted");
        let o = &import_objects(&p).unwrap()[0];
        assert!(maps(o).is_some() && has_maps(o));
        let mut off = o.clone();
        off.material.textures = false;
        assert!(maps(&off).is_none() && has_maps(&off));
    }
    #[test]
    fn maps_halve_past_the_limit_and_reject_height_maps() {
        let (w, h, rgba) = halve(4, 2, &[10u8; 32]);
        assert_eq!((w, h, rgba.len()), (2, 1, 8));
        assert!(rgba.iter().all(|&v| v == 10));
        let grey = Map::new(2, 2, vec![140; 16], false);
        assert!(
            !grey.looks_like_normals(),
            "a grey height map is not a normal map"
        );
        let flat = Map::new(2, 2, [128, 128, 255, 255].repeat(4), false);
        assert!(flat.looks_like_normals());
        // The mip chain ends at 1×1, also from a 1-texel-wide image.
        let tall = Map::new(1, 8, vec![7; 32], false);
        assert_eq!(
            tall.mips.iter().map(|m| m.len()).collect::<Vec<_>>(),
            [16, 8, 4]
        );
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
