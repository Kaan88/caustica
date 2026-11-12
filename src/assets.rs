//! Online asset catalogs and downloads for the asset browser (roadmap item 6): Poly Haven
//! HDRIs, textures and models, ambientCG materials and the Khronos glTF samples. Downloads land
//! in the user scenes folder (`dirs::user_scenes`: `hdri`, `textures/<id>`, `models`), each
//! file's SHA-256 goes into its `assets.json` (a changed upstream file then fails instead of
//! slipping in), and catalogs and thumbnails are cached under `<dirs::cache>/assets`.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Kind {
    Hdri,
    Texture,
    Model,
}
impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Hdri, Kind::Texture, Kind::Model];
    pub fn label(self) -> &'static str {
        match self {
            Kind::Hdri => "HDRIs",
            Kind::Texture => "Textures",
            Kind::Model => "Models",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Source {
    PolyHaven,
    AmbientCg,
    Khronos,
}
impl Source {
    pub const ALL: [Source; 3] = [Source::PolyHaven, Source::AmbientCg, Source::Khronos];
    pub fn label(self) -> &'static str {
        match self {
            Source::PolyHaven => "Poly Haven",
            Source::AmbientCg => "ambientCG",
            Source::Khronos => "Khronos glTF samples",
        }
    }
    pub fn license(self) -> &'static str {
        match self {
            Source::PolyHaven | Source::AmbientCg => "CC0",
            Source::Khronos => "Per model (see its README)",
        }
    }
    fn slug(self) -> &'static str {
        match self {
            Source::PolyHaven => "polyhaven",
            Source::AmbientCg => "ambientcg",
            Source::Khronos => "khronos",
        }
    }
    /// What each source offers.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Source::PolyHaven => &Kind::ALL,
            Source::AmbientCg => &[Kind::Texture],
            Source::Khronos => &[Kind::Model],
        }
    }
}
/// One catalog entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    pub source: Source,
    pub kind: Kind,
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub thumbnail: String,
    /// The asset's web page.
    pub page: String,
    pub credit: String,
    /// Offered resolutions, smallest first ("1k", "2k", …); empty for single-file models.
    pub resolutions: Vec<String>,
    /// Khronos: the GLB's file name.
    pub file: String,
    /// Downloads so far (Poly Haven, ambientCG): catalogs list the most used first.
    #[serde(default)]
    pub popularity: u64,
}
const KHRONOS: &str =
    "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models";

// ---------------------------------------------------------------- HTTP
static AGENT: LazyLock<Result<ureq::Agent, String>> = LazyLock::new(|| {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .user_agent("caustica (personal path tracer)")
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .build())
});
pub(crate) fn get(url: &str) -> Result<ureq::Response> {
    let agent = AGENT.as_ref().map_err(|e| anyhow::anyhow!("TLS: {e}"))?;
    agent.get(url).call().with_context(|| format!("GET {url}"))
}
pub(crate) fn get_json(url: &str) -> Result<Value> {
    let mut text = String::new();
    get(url)?
        .into_reader()
        .take(256 << 20)
        .read_to_string(&mut text)?;
    serde_json::from_str(&text).with_context(|| format!("JSON from {url}"))
}
fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut out = vec![];
    get(url)?.into_reader().take(limit).read_to_end(&mut out)?;
    Ok(out)
}
/// Bytes received and expected (0 = unknown) of a running download, and a cancel flag.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// Streams a URL into `dest` (through `dest.partial`) and returns its SHA-256.
fn download(url: &str, dest: &Path, progress: &Progress) -> Result<String> {
    let response = get(url)?;
    if let Some(n) = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
    {
        progress.total.fetch_add(n, Ordering::Relaxed);
    }
    save_stream(response.into_reader(), dest, progress)
}
/// Streams `reader` into `dest` (through `dest.partial`, removed on an error or a cancel) and
/// returns its SHA-256.
pub(crate) fn save_stream(reader: impl Read, dest: &Path, progress: &Progress) -> Result<String> {
    use sha2::Digest;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = dest.with_extension("partial");
    let mut file = std::fs::File::create(&partial)
        .with_context(|| format!("Creating {}", partial.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut reader = reader.take(4 << 30);
    let mut buffer = vec![0u8; 1 << 16];
    let copied = (|| -> Result<()> {
        loop {
            if progress.cancel.load(Ordering::Relaxed) {
                bail!("Cancelled");
            }
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                return Ok(());
            }
            file.write_all(&buffer[..n])?;
            hasher.update(&buffer[..n]);
            progress.done.fetch_add(n as u64, Ordering::Relaxed);
        }
    })();
    drop(file);
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, dest)?;
    Ok(hex(&hasher.finalize()))
}

// ---------------------------------------------------------------- catalogs
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default()
}
/// `https://api.polyhaven.com/assets?t=…`: an object of id → asset.
pub fn parse_polyhaven(kind: Kind, json: &Value) -> Vec<Asset> {
    let Some(map) = json.as_object() else {
        return vec![];
    };
    let mut out: Vec<Asset> = map
        .iter()
        .map(|(id, a)| {
            let mut tags = strings(&a["categories"]);
            tags.extend(strings(&a["tags"]));
            tags.dedup();
            let authors: Vec<&str> = a["authors"]
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            Asset {
                source: Source::PolyHaven,
                kind,
                id: id.clone(),
                name: a["name"].as_str().unwrap_or(id).to_string(),
                tags,
                thumbnail: format!(
                    "https://cdn.polyhaven.com/asset_img/thumbs/{id}.png?width=256&height=256"
                ),
                page: format!("https://polyhaven.com/a/{id}"),
                credit: format!("{} (Poly Haven, CC0)", authors.join(", ")),
                resolutions: ["1k", "2k", "4k", "8k"].map(String::from).to_vec(),
                file: String::new(),
                popularity: a["download_count"].as_u64().unwrap_or(0),
            }
        })
        .collect();
    out.sort_by(|a, b| b.popularity.cmp(&a.popularity).then(a.name.cmp(&b.name)));
    out
}
/// One page of `https://ambientcg.com/api/v2/full_json?type=Material…`.
pub fn parse_ambientcg(json: &Value) -> Vec<Asset> {
    let Some(found) = json["foundAssets"].as_array() else {
        return vec![];
    };
    found
        .iter()
        .filter_map(|a| {
            let id = a["assetId"].as_str()?.to_string();
            // Only the JPG zips are offered: 1K-JPG → "1K".
            let mut resolutions: Vec<String> =
                a["downloadFolders"]["default"]["downloadFiletypeCategories"]
                    .as_object()
                    .into_iter()
                    .flat_map(|c| c.values())
                    .flat_map(|c| c["downloads"].as_array().cloned().unwrap_or_default())
                    .filter_map(|d| {
                        d["attribute"]
                            .as_str()
                            .and_then(|s| s.strip_suffix("-JPG"))
                            .map(String::from)
                    })
                    .collect();
            resolutions.sort_by_key(|r| r.trim_end_matches('K').parse::<u32>().unwrap_or(99));
            resolutions.dedup();
            Some(Asset {
                source: Source::AmbientCg,
                kind: Kind::Texture,
                name: a["displayName"].as_str().unwrap_or(&id).to_string(),
                tags: strings(&a["tags"]),
                thumbnail: a["previewImage"]["256-PNG"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                page: format!("https://ambientcg.com/view?id={id}"),
                credit: "ambientCG (CC0)".into(),
                resolutions,
                file: String::new(),
                popularity: a["downloadCount"].as_u64().unwrap_or(0),
                id,
            })
        })
        .collect()
}
/// `model-index.json` of glTF-Sample-Assets: models with a single-file GLB variant.
pub fn parse_khronos(json: &Value) -> Vec<Asset> {
    let Some(models) = json.as_array() else {
        return vec![];
    };
    models
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?;
            let file = m["variants"]["glTF-Binary"].as_str()?;
            Some(Asset {
                source: Source::Khronos,
                kind: Kind::Model,
                id: name.to_string(),
                name: m["label"].as_str().unwrap_or(name).to_string(),
                tags: strings(&m["tags"]),
                thumbnail: m["screenshot"]
                    .as_str()
                    .map(|s| format!("{KHRONOS}/{name}/{s}"))
                    .unwrap_or_default(),
                page: format!(
                    "https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/{name}"
                ),
                credit: "Khronos glTF-Sample-Assets (licence in the model's README)".into(),
                resolutions: vec![],
                file: file.to_string(),
                popularity: 0,
            })
        })
        .collect()
}
/// Downloads a source's whole catalog (blocking; run it off the UI thread) and caches it.
pub fn fetch_catalog(source: Source) -> Result<Vec<Asset>> {
    let assets = match source {
        Source::PolyHaven => {
            let mut all = vec![];
            for (t, kind) in [
                ("hdris", Kind::Hdri),
                ("textures", Kind::Texture),
                ("models", Kind::Model),
            ] {
                let json = get_json(&format!("https://api.polyhaven.com/assets?t={t}"))?;
                all.extend(parse_polyhaven(kind, &json));
            }
            all
        }
        Source::AmbientCg => {
            let mut all = vec![];
            for page in 0..40 {
                let json = get_json(&format!(
                    "https://ambientcg.com/api/v2/full_json?type=Material&sort=Popular&limit=250&offset={}&include=tagData,previewData,downloadData",
                    page * 250
                ))?;
                let found = parse_ambientcg(&json);
                if found.is_empty() {
                    break;
                }
                all.extend(found);
            }
            all
        }
        Source::Khronos => parse_khronos(&get_json(&format!("{KHRONOS}/model-index.json"))?),
    };
    ensure!(!assets.is_empty(), "{} returned no assets", source.label());
    if let Some(dir) = cache_dir() {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("catalog-{}.json", source.slug())),
            serde_json::to_vec(&assets)?,
        )?;
    }
    Ok(assets)
}
/// The catalog from the last fetch, if any (works offline).
pub fn cached_catalog(source: Source) -> Option<Vec<Asset>> {
    let path = cache_dir()?.join(format!("catalog-{}.json", source.slug()));
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
fn cache_dir() -> Option<PathBuf> {
    crate::dirs::cache().map(|f| f.join("assets"))
}
pub fn thumbnail_path(a: &Asset) -> Option<PathBuf> {
    let safe: String =
        a.id.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
    cache_dir().map(|d| d.join(format!("{}-{:?}-{safe}.img", a.source.slug(), a.kind)))
}
/// Downloads a thumbnail into the cache (if missing) and returns its path.
pub fn fetch_thumbnail(a: &Asset) -> Result<PathBuf> {
    let path = thumbnail_path(a).context("No scenes folder")?;
    if !path.is_file() {
        ensure!(!a.thumbnail.is_empty(), "{} has no preview", a.name);
        let bytes = get_bytes(&a.thumbnail, 16 << 20)?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)?;
    }
    Ok(path)
}

// ---------------------------------------------------------------- downloads
/// Where a downloaded asset is used from, relative to the scenes folder: the HDRI, the colour
/// map of a texture set (its other maps sit beside it), or the model file.
pub fn use_path(a: &Asset, res: &str) -> String {
    match (a.source, a.kind) {
        (Source::PolyHaven, Kind::Hdri) => format!("hdri/{}_{res}.hdr", a.id),
        (Source::PolyHaven, Kind::Texture) => format!("textures/{0}/{0}_diff_{res}.jpg", a.id),
        (Source::PolyHaven, Kind::Model) => format!("models/{0}/{0}_{res}.gltf", a.id),
        (Source::AmbientCg, _) => format!("textures/{0}/{0}_{res}-JPG_Color.jpg", a.id),
        (Source::Khronos, _) => format!("models/{}", a.file),
    }
}
/// Already in the scenes folder (from the browser or tools/fetch_assets.ps1).
pub fn installed(a: &Asset, res: &str) -> bool {
    crate::dirs::resolve_scene_path(Path::new(&use_path(a, res))).is_file()
}
/// A relative path from a catalog is used only if it stays inside its folder.
fn safe_relative(path: &str) -> Result<&str> {
    let ok = !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && path
            .split(['/', '\\'])
            .all(|c| !c.is_empty() && c != "." && c != "..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/ ".contains(c));
    ensure!(ok, "Refusing the file path {path:?}");
    Ok(path)
}
/// The files to fetch: (URL, path relative to the scenes folder). ambientCG zips are
/// extracted (`zip` true).
fn plan(a: &Asset, res: &str) -> Result<Vec<(String, String, bool)>> {
    let id = safe_relative(&a.id)?;
    Ok(match (a.source, a.kind) {
        (Source::PolyHaven, kind) => {
            let files = get_json(&format!("https://api.polyhaven.com/files/{id}"))?;
            match kind {
                Kind::Hdri => {
                    let url = files["hdri"][res]["hdr"]["url"]
                        .as_str()
                        .with_context(|| format!("{id} has no {res} HDR"))?;
                    vec![(url.into(), use_path(a, res), false)]
                }
                Kind::Texture => {
                    let mut out = vec![];
                    // Colour and OpenGL normals, then roughness / metal packed (ARM) or alone.
                    for (key, required) in [("Diffuse", true), ("nor_gl", false), ("arm", false)] {
                        match files[key][res]["jpg"]["url"].as_str() {
                            Some(url) => out.push(url.to_string()),
                            None if required => bail!("{id} has no {res} colour map"),
                            None => {}
                        }
                    }
                    if files["arm"].is_null()
                        && let Some(url) = files["Rough"][res]["jpg"]["url"].as_str()
                    {
                        out.push(url.into());
                    }
                    out.into_iter()
                        .map(|url| {
                            let name = url.rsplit('/').next().unwrap_or_default().to_string();
                            let rel = format!("textures/{id}/{}", safe_relative(&name)?);
                            Ok((url, rel, false))
                        })
                        .collect::<Result<_>>()?
                }
                Kind::Model => {
                    let gltf = &files["gltf"][res]["gltf"];
                    let url = gltf["url"]
                        .as_str()
                        .with_context(|| format!("{id} has no {res} glTF"))?;
                    let mut out = vec![(url.to_string(), use_path(a, res), false)];
                    for (rel, f) in gltf["include"].as_object().into_iter().flatten() {
                        let url = f["url"].as_str().context("glTF include without a URL")?;
                        out.push((
                            url.into(),
                            format!("models/{id}/{}", safe_relative(rel)?),
                            false,
                        ));
                    }
                    out
                }
            }
        }
        (Source::AmbientCg, _) => {
            ensure!(
                a.resolutions.iter().any(|r| r == res),
                "{id} has no {res} JPG"
            );
            vec![(
                format!("https://ambientcg.com/get?file={id}_{res}-JPG.zip"),
                format!("textures/{id}/{id}_{res}-JPG.zip"),
                true,
            )]
        }
        (Source::Khronos, _) => {
            let file = safe_relative(&a.file)?;
            vec![(
                format!("{KHRONOS}/{id}/glTF-Binary/{file}"),
                use_path(a, res),
                false,
            )]
        }
    })
}
/// The maps an ambientCG zip contributes (the rest of the zip is skipped).
fn wanted_from_zip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg")
        && ["_color", "_normalgl", "_roughness", "_metalness"]
            .iter()
            .any(|m| lower.contains(m))
}
/// Extracts the wanted maps of a zip into `dir`, by file name only.
pub fn extract_maps(zip_path: &Path, dir: &Path) -> Result<usize> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(name) = Path::new(entry.name())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
        else {
            continue;
        };
        if !entry.is_file() || !wanted_from_zip(&name) {
            continue;
        }
        let mut out = std::fs::File::create(dir.join(safe_relative(&name)?))?;
        std::io::copy(&mut (&mut entry).take(1 << 30), &mut out)?;
        count += 1;
    }
    ensure!(count > 0, "The zip has no colour, normal or roughness maps");
    Ok(count)
}

/// `scenes/assets.json`: every file the browser downloaded, with its SHA-256.
#[derive(Default, Serialize, Deserialize)]
pub struct Manifest {
    pub files: std::collections::BTreeMap<String, Pinned>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Pinned {
    pub sha256: String,
    pub url: String,
    pub credit: String,
    pub page: String,
}
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());
fn manifest_path() -> Option<PathBuf> {
    crate::dirs::user_scenes().map(|f| f.join("assets.json"))
}
pub fn manifest() -> Manifest {
    manifest_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
/// Records a file's hash, or fails if it differs from the one recorded before.
fn pin(rel: &str, pinned: Pinned) -> Result<()> {
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut m = manifest();
    if let Some(old) = m.files.get(rel) {
        ensure!(
            old.sha256 == pinned.sha256,
            "{rel} changed upstream (SHA-256 {} instead of the pinned {}); delete its entry in scenes/assets.json to accept it",
            pinned.sha256,
            old.sha256
        );
        return Ok(());
    }
    m.files.insert(rel.to_string(), pinned);
    let path = manifest_path().context("No scenes folder")?;
    std::fs::write(path, serde_json::to_vec_pretty(&m)?)?;
    Ok(())
}
/// Downloads an asset at a resolution (blocking) and returns its `use_path`.
pub fn install(a: &Asset, res: &str, progress: &Progress) -> Result<String> {
    let root = crate::dirs::user_scenes().context("No scenes folder")?;
    // The file `installed` looks for comes last, so a set broken off by Cancel or an error
    // is not taken for an installed one, and Download stays offered to fetch the rest.
    let used = use_path(a, res);
    let mut files = plan(a, res)?;
    files.sort_by_key(|(_, rel, _)| *rel == used);
    for (url, rel, zip) in files {
        let dest = root.join(&rel);
        if !zip && dest.is_file() {
            continue;
        }
        let sha256 = download(&url, &dest, progress)?;
        let pinned = Pinned {
            sha256,
            url: url.clone(),
            credit: a.credit.clone(),
            page: a.page.clone(),
        };
        if let Err(e) = pin(&rel, pinned) {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
        if zip {
            if let Err(e) = extract_maps(&dest, dest.parent().unwrap()) {
                let _ = std::fs::remove_file(root.join(&used));
                return Err(e);
            }
            std::fs::remove_file(&dest)?;
        }
    }
    ensure!(
        root.join(&used).is_file(),
        "{used} is missing after the download"
    );
    Ok(used)
}
/// A download running on its own thread.
pub struct Job {
    pub asset: Asset,
    pub res: String,
    pub progress: Arc<Progress>,
    pub result: Arc<Mutex<Option<Result<String, String>>>>,
}
pub fn start(asset: Asset, res: String) -> Job {
    let progress = Arc::new(Progress::default());
    let result = Arc::new(Mutex::new(None));
    let (a, r, p, out) = (asset.clone(), res.clone(), progress.clone(), result.clone());
    std::thread::spawn(move || {
        let done = install(&a, &r, &p).map_err(|e| format!("{e:#}"));
        *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
    });
    Job {
        asset,
        res,
        progress,
        result,
    }
}
/// `--fetch-asset source/kind/id[@res]` (e.g. `polyhaven/hdri/meadow_2@2k`, `khronos/model/Duck`):
/// the asset without its catalog entry.
pub fn from_spec(spec: &str) -> Result<(Asset, String)> {
    let (path, res) = spec.split_once('@').unwrap_or((spec, ""));
    let parts: Vec<&str> = path.split('/').collect();
    let [source, kind, id] = parts[..] else {
        bail!("Expected source/kind/id[@res], got {spec}");
    };
    let source = Source::ALL
        .into_iter()
        .find(|s| s.slug() == source)
        .with_context(|| format!("Unknown source {source} (polyhaven, ambientcg, khronos)"))?;
    let kind = match kind {
        "hdri" => Kind::Hdri,
        "texture" => Kind::Texture,
        "model" => Kind::Model,
        _ => bail!("Unknown kind {kind} (hdri, texture, model)"),
    };
    let res = match (source, res) {
        (Source::AmbientCg, "") => "2K".to_string(),
        (_, "") => "2k".to_string(),
        (_, r) => r.to_string(),
    };
    let asset = Asset {
        source,
        kind,
        id: id.to_string(),
        name: id.to_string(),
        tags: vec![],
        thumbnail: String::new(),
        page: String::new(),
        credit: format!("{} ({})", source.label(), source.license()),
        resolutions: vec![res.clone()],
        file: format!("{id}.glb"),
        popularity: 0,
    };
    Ok((asset, res))
}

/// Places a source cannot be browsed from inside the app (no public API, an account, or one
/// download per scene): name, what it has, licence, URL.
pub const OTHER_SOURCES: [(&str, &str, &str, &str); 8] = [
    (
        "Stanford 3D Scanning Repository",
        "The bunny, dragon, Buddha, Lucy and armadillo scans (PLY; common-3d-test-models has OBJ copies).",
        "Research use; credit Stanford",
        "https://graphics.stanford.edu/data/3Dscanrep/",
    ),
    (
        "common-3d-test-models",
        "Alec Jacobson's OBJ collection of the classic test meshes (Spot, Suzanne, the scans).",
        "Per model (see its README)",
        "https://github.com/alecjacobson/common-3d-test-models",
    ),
    (
        "McGuire Computer Graphics Archive",
        "Sponza, San Miguel, Bistro, Conference room, Rungholt and more, as OBJ.",
        "Per scene (mostly CC BY)",
        "https://casual-effects.com/data/",
    ),
    (
        "Benedikt Bitterli's rendering resources",
        "32 scenes from classic papers (Mitsuba / Tungsten format: the OBJ meshes import).",
        "Per scene (CC BY, CC0)",
        "https://benedikt-bitterli.me/resources/",
    ),
    (
        "NVIDIA ORCA",
        "Amazon Lumberyard Bistro, Emerald Square, Zero Day and other large scenes (FBX / glTF).",
        "Per scene (CC BY and others)",
        "https://developer.nvidia.com/orca",
    ),
    (
        "Smithsonian 3D",
        "Museum scans: fossils, aircraft, sculpture (glTF and OBJ, open access ones are CC0).",
        "CC0 for open-access items",
        "https://3d.si.edu/",
    ),
    (
        "Sketchfab",
        "Hundreds of thousands of models; downloads need an account (glTF).",
        "Per model (CC BY, CC0, …)",
        "https://sketchfab.com/search?features=downloadable&type=models",
    ),
    (
        "Poly Haven / ambientCG websites",
        "The full sites, for previews in a browser; everything there is also in the tabs above.",
        "CC0",
        "https://polyhaven.com/",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_parse() {
        let ph: Value = serde_json::from_str(
            r#"{"meadow_2":{"name":"Meadow 2","categories":["outdoor"],"tags":["grass"],"authors":{"Greg Zaal":"All"},"download_count":9},
                "attic":{"name":"Attic","download_count":3}}"#,
        )
        .unwrap();
        let a = &parse_polyhaven(Kind::Hdri, &ph)[0];
        assert_eq!(
            parse_polyhaven(Kind::Hdri, &ph)[1].id,
            "attic",
            "most downloaded first"
        );
        assert_eq!((a.id.as_str(), a.name.as_str()), ("meadow_2", "Meadow 2"));
        assert_eq!(a.tags, ["outdoor", "grass"]);
        assert!(a.credit.contains("Greg Zaal"));
        assert_eq!(use_path(a, "4k"), "hdri/meadow_2_4k.hdr");
        let acg: Value = serde_json::from_str(
            r#"{"foundAssets":[{"assetId":"Bricks097","displayName":"Bricks 097","tags":["brick"],
            "previewImage":{"256-PNG":"https://x/Bricks097.png"},
            "downloadFolders":{"default":{"downloadFiletypeCategories":{"zip":{"downloads":[
            {"attribute":"4K-JPG"},{"attribute":"1K-JPG"},{"attribute":"2K-PNG"},{"attribute":"2K-JPG"}]}}}}}]}"#,
        )
        .unwrap();
        let b = &parse_ambientcg(&acg)[0];
        assert_eq!(b.resolutions, ["1K", "2K", "4K"]);
        assert_eq!(
            use_path(b, "2K"),
            "textures/Bricks097/Bricks097_2K-JPG_Color.jpg"
        );
        let kh: Value = serde_json::from_str(
            r#"[{"label":"Duck","name":"Duck","screenshot":"screenshot/screenshot.png","variants":{"glTF-Binary":"Duck.glb"}},
               {"label":"Draco only","name":"X","variants":{"glTF-Draco":"X.gltf"}}]"#,
        )
        .unwrap();
        let k = parse_khronos(&kh);
        assert_eq!(k.len(), 1, "models without a GLB are skipped");
        assert_eq!(use_path(&k[0], ""), "models/Duck.glb");
        assert!(k[0].thumbnail.ends_with("/Duck/screenshot/screenshot.png"));
    }
    #[test]
    fn unsafe_paths_are_refused() {
        for bad in [
            "../x", "a/../b", "/etc", "C:/x", "a\\..\\b", "", "a//b", "x?y",
        ] {
            assert!(safe_relative(bad).is_err(), "{bad}");
        }
        assert!(safe_relative("textures/Barrel_01_diff_1k.jpg").is_ok());
        let (a, res) = from_spec("polyhaven/texture/rock_wall_08").unwrap();
        assert_eq!((a.kind, res.as_str()), (Kind::Texture, "2k"));
        assert!(from_spec("polyhaven/hdri").is_err());
    }
    #[test]
    fn zips_give_only_the_maps() {
        let dir = std::env::temp_dir().join("caustica-test-zip");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("set.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in [
            "Set_2K-JPG_Color.jpg",
            "Set_2K-JPG_NormalDX.jpg",
            "nested/Set_2K-JPG_Roughness.jpg",
            "Set_2K-JPG.blend",
        ] {
            z.start_file(name, options).unwrap();
            z.write_all(b"x").unwrap();
        }
        z.finish().unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert_eq!(extract_maps(&zip_path, &out).unwrap(), 2);
        assert!(
            out.join("Set_2K-JPG_Roughness.jpg").is_file(),
            "folders are flattened"
        );
        assert!(!out.join("Set_2K-JPG.blend").exists());
    }
}
