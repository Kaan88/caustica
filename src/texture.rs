//! Texture sets for any material (core track 5): a folder of maps named the ambientCG or
//! Poly Haven way (`*_Color`, `*_NormalGL`, `*_Roughness`, `*_Metalness`, or `*_diff`,
//! `*_nor_gl`, `*_rough`, `*_arm`), or a single image used as base colour. Built-in shapes,
//! the floor and the walls have no UVs, so the shader box-maps them (`Material::texture_scale`).
use crate::mesh::{Map, Maps};
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

type Cache = HashMap<PathBuf, Result<Maps, String>>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// The maps of a texture set (cached per resolved path; failures too).
pub fn load(path: &str) -> Result<Maps> {
    let key = crate::mesh::resolve(path);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(&key) {
        return hit.clone().map_err(anyhow::Error::msg);
    }
    let result = read(&key).map_err(|e| format!("{e:#}"));
    cache.insert(key, result.clone());
    result.map_err(anyhow::Error::msg)
}
/// Which map a file is, from its name (lowercase, without the extension).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Base,
    NormalGl,
    NormalDx,
    Roughness,
    Metalness,
    /// Poly Haven's packed ambient occlusion / roughness / metalness (glTF layout in G and B).
    Arm,
}
const MAP_WORDS: [&str; 17] = [
    "color",
    "basecolor",
    "albedo",
    "diffuse",
    "diff",
    "col",
    "normalgl",
    "normaldx",
    "normal",
    "nrm",
    "nor",
    "arm",
    "roughness",
    "rough",
    "metalness",
    "metallic",
    "metal",
];
/// The name up to the first map word: `rock_wall_nor_gl_2k` → `rock_wall`.
fn prefix(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    let mut start = 0;
    for token in lower.split(['_', '-', ' ', '.']) {
        if MAP_WORDS.contains(&token) {
            break;
        }
        start += token.len() + 1;
    }
    lower[..start.min(lower.len())]
        .trim_end_matches(['_', '-', ' ', '.'])
        .to_string()
}
fn kind(stem: &str) -> Option<Kind> {
    let words: Vec<&str> = stem.split(['_', '-', ' ', '.']).collect();
    let has = |w: &str| words.iter().any(|x| x.eq_ignore_ascii_case(w));
    let pair = |a: &str, b: &str| {
        words
            .windows(2)
            .any(|p| p[0].eq_ignore_ascii_case(a) && p[1].eq_ignore_ascii_case(b))
    };
    Some(if has("normalgl") || pair("nor", "gl") {
        Kind::NormalGl
    } else if has("normaldx") || pair("nor", "dx") {
        Kind::NormalDx
    } else if has("normal") || has("nrm") || has("nor") {
        Kind::NormalGl
    } else if has("arm") {
        Kind::Arm
    } else if has("roughness") || has("rough") {
        Kind::Roughness
    } else if has("metalness") || has("metallic") || has("metal") {
        Kind::Metalness
    } else if MAP_WORDS[..6].iter().any(|w| has(w)) {
        Kind::Base
    } else {
        return None;
    })
}
fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["jpg", "jpeg", "png"].contains(&e.to_ascii_lowercase().as_str()))
}
/// The files of a set: every recognised image in a folder, or, for a file, the images next to
/// it that share its prefix (the part before the map name), or the file alone as base colour.
fn files(path: &Path) -> Result<Vec<(Kind, PathBuf)>> {
    let listing = |dir: &Path| -> Result<Vec<PathBuf>> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("{}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_image(p))
            .collect();
        v.sort();
        Ok(v)
    };
    let stem = |p: &Path| {
        p.file_stem()
            .map_or(String::new(), |s| s.to_string_lossy().into())
    };
    let chosen: Vec<PathBuf> = if path.is_dir() {
        listing(path)?
    } else {
        ensure!(path.exists(), "{} not found", path.display());
        if kind(&stem(path)).is_none() {
            return Ok(vec![(Kind::Base, path.to_path_buf())]);
        }
        // `Wood_Color.jpg` → every `Wood_*` image beside it.
        let prefix = prefix(&stem(path));
        listing(path.parent().unwrap_or(Path::new(".")))?
            .into_iter()
            .filter(|p| crate::texture::prefix(&stem(p)) == prefix)
            .collect()
    };
    let mut out: Vec<(Kind, PathBuf)> = vec![];
    for p in chosen {
        if let Some(k) = kind(&stem(&p))
            && !out.iter().any(|(o, _)| *o == k)
        {
            out.push((k, p));
        }
    }
    ensure!(
        out.iter().any(|(k, _)| *k == Kind::Base),
        "No colour map (a name with Color, BaseColor, Albedo or diff) in {}",
        path.display()
    );
    Ok(out)
}
fn read(path: &Path) -> Result<Maps> {
    let files = files(path)?;
    let images = crate::mesh::decode_all(files, |(kind, file)| {
        let image = image::open(file).ok()?.to_rgba8();
        let srgb = *kind == Kind::Base;
        Some(Map::new(
            image.width(),
            image.height(),
            image.into_raw(),
            srgb,
        ))
    });
    let get = |k: Kind| {
        images
            .iter()
            .find(|((kind, _), _)| *kind == k)
            .map(|(_, m)| m)
    };
    let base = get(Kind::Base).context("The colour map could not be read")?;
    // Normals: DirectX maps have green pointing down.
    let normal = match (get(Kind::NormalGl), get(Kind::NormalDx)) {
        (Some(n), _) => Some(n.clone()),
        (None, Some(n)) => {
            let mut rgba = n.rgba.clone();
            rgba.as_chunks_mut::<4>()
                .0
                .iter_mut()
                .for_each(|t| t[1] = 255 - t[1]);
            Some(Arc::new(Map::new(n.width, n.height, rgba, false)))
        }
        _ => None,
    }
    .filter(|n| n.looks_like_normals());
    // Roughness in G and metalness in B, as glTF: an ARM map is that already.
    let metal_rough = match (get(Kind::Arm), get(Kind::Roughness), get(Kind::Metalness)) {
        (Some(arm), _, _) => Some(arm.clone()),
        (None, None, None) => None,
        (None, rough, metal) => {
            let (w, h) = rough.or(metal).map(|m| (m.width, m.height)).unwrap();
            let sample = |m: Option<&Arc<Map>>, x: u32, y: u32, default: u8| {
                m.map_or(default, |m| {
                    // Nearest texel, in case the two maps differ in size.
                    let (mx, my) = (x * m.width / w, y * m.height / h);
                    m.rgba[((my * m.width + mx) * 4) as usize]
                })
            };
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    rgba.extend([255, sample(rough, x, y, 255), sample(metal, x, y, 0), 255]);
                }
            }
            Some(Arc::new(Map::new(w, h, rgba, false)))
        }
    };
    Ok(Maps {
        base: Some(base.clone()),
        normal,
        metal_rough,
        cutoff: 0.,
    })
}
/// Colour, roughness and metallic of a material that now uses a set: the map averages, so the
/// maps show as they are (they are scaled by value / average on the GPU).
pub fn apply_averages(m: &mut crate::scene::Material, maps: &Maps) {
    if let Some(b) = &maps.base {
        m.color = [b.average[0], b.average[1], b.average[2]];
    }
    if let Some(mr) = &maps.metal_rough {
        m.roughness = mr.average[1];
        m.metallic = mr.average[2];
    } else {
        m.metallic = 0.;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_names_are_recognised() {
        assert_eq!(kind("WoodFloor041_2K-JPG_Color"), Some(Kind::Base));
        assert_eq!(kind("WoodFloor041_2K-JPG_NormalGL"), Some(Kind::NormalGl));
        assert_eq!(kind("WoodFloor041_2K-JPG_NormalDX"), Some(Kind::NormalDx));
        assert_eq!(kind("Metal032_2K-JPG_Metalness"), Some(Kind::Metalness));
        assert_eq!(kind("rock_wall_diff_2k"), Some(Kind::Base));
        assert_eq!(kind("rock_wall_nor_gl_2k"), Some(Kind::NormalGl));
        assert_eq!(kind("rock_wall_arm_2k"), Some(Kind::Arm));
        assert_eq!(kind("WoodFloor041_2K-JPG_Displacement"), None);
        assert_eq!(kind("holiday photo"), None);
        assert_eq!(prefix("rock_wall_nor_gl_2k"), "rock_wall");
        assert_eq!(prefix("WoodFloor041_2K-JPG_Color"), "woodfloor041_2k-jpg");
    }
    #[test]
    fn a_folder_becomes_one_set() {
        let dir = std::env::temp_dir().join("caustica-test-set");
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, c: [u8; 4]| {
            image::RgbaImage::from_pixel(4, 4, image::Rgba(c))
                .save(dir.join(name))
                .unwrap()
        };
        save("Test_Color.png", [200, 100, 50, 255]);
        save("Test_NormalDX.png", [128, 127, 255, 255]);
        save("Test_Roughness.png", [64, 64, 64, 255]);
        save("Test_Displacement.png", [0, 0, 0, 255]);
        let maps = read(&dir).unwrap();
        assert!(maps.base.as_ref().unwrap().srgb);
        // DirectX green flipped to OpenGL's.
        assert_eq!(maps.normal.as_ref().unwrap().rgba[1], 128);
        let mr = maps.metal_rough.as_ref().unwrap();
        assert_eq!(&mr.rgba[..4], &[255, 64, 0, 255]);
        // A file inside the folder finds its siblings.
        let again = read(&dir.join("Test_Color.png")).unwrap();
        assert!(again.normal.is_some() && again.metal_rough.is_some());
        let mut m = crate::scene::Material::default();
        apply_averages(&mut m, &maps);
        assert!((m.roughness - 64. / 255.).abs() < 1e-3 && m.metallic == 0.);
    }
}
