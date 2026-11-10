//! Scene files (roadmap item 4): `scenes/<pack>/*.json`, each a small wrapper around the
//! `Settings` JSON. A bare `Settings` file (what "Save settings" writes) is accepted too.
use crate::scene::Settings;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct SceneFile {
    pub format: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub credits: String,
    pub settings: Settings,
}
#[derive(Deserialize)]
struct Pack {
    name: String,
    #[serde(default)]
    scenes: Vec<String>,
    #[serde(default)]
    license: String,
    #[serde(default = "yes")]
    commercial_use: bool,
    #[serde(default)]
    attribution: String,
    #[serde(default)]
    source: String,
}
fn yes() -> bool {
    true
}
/// What a pack says about its licence, shown in the Scenes window.
#[derive(Clone, Default)]
pub struct License {
    pub name: String,
    pub commercial_use: bool,
    pub attribution: String,
    pub source: String,
}
#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub pack: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub credits: String,
    pub license: License,
    /// FNV-1a of `RENDER_VERSION` and the file, so a thumbnail is re-rendered when the scene or the
    /// engine's look changes.
    pub hash: u64,
}
impl Entry {
    /// `<cache>/<pack folder>--<file>-<hash>.png` (`dirs::cache`).
    pub fn thumbnail(&self) -> Option<PathBuf> {
        Some(crate::dirs::cache()?.join(self.thumbnail_name()?))
    }
    /// The thumbnail's file name: the hash is part of it, so a changed scene is a different name.
    fn thumbnail_name(&self) -> Option<String> {
        let pack = self
            .path
            .parent()?
            .file_name()?
            .to_string_lossy()
            .to_string();
        let stem = self.path.file_stem()?.to_string_lossy().to_string();
        Some(format!("{pack}--{stem}-{:016x}.png", self.hash))
    }
}

/// A file name `<pack>--<stem>-<16 hex digits>.png` (what `Entry::thumbnail_name` makes).
fn is_thumbnail_name(name: &str) -> bool {
    let Some(base) = name.strip_suffix(".png") else {
        return false;
    };
    base.contains("--")
        && base
            .rsplit_once('-')
            .is_some_and(|(_, h)| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}
/// The rule: a scene that changes loses its thumbnail. A thumbnail is named by the hash of its
/// scene file, so an edited scene has no thumbnail until one is rendered (or seeded from the
/// bundle) under its new name, and this removes every scene thumbnail in `cache` that no scene in
/// `entries` names any more: older hashes of an edited scene, and scenes that were deleted.
/// Returns how many files were removed.
fn remove_stale(cache: &Path, entries: &[Entry]) -> usize {
    let current: std::collections::HashSet<String> =
        entries.iter().filter_map(Entry::thumbnail_name).collect();
    let mut removed = 0;
    for f in std::fs::read_dir(cache).into_iter().flatten().flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        if is_thumbnail_name(&name)
            && !current.contains(&name)
            && f.path().is_file()
            && std::fs::remove_file(f.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}
/// Removes the other-hash thumbnails of the scene that `current` (a just-rendered thumbnail) is
/// of: `<pack>--<stem>-<hash>.png` with any other hash.
fn remove_older(current: &Path) {
    let prefix = current
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .rsplit_once('-')
        .map(|(p, _)| p.to_string());
    if let (Some(prefix), Some(dir)) = (prefix, current.parent()) {
        for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = old.file_name().to_string_lossy().to_string();
            // The rest after the prefix must be just the hash: "room-2" is another scene.
            let same_scene = name
                .strip_prefix(&format!("{prefix}-"))
                .is_some_and(|r| r.len() == 20 && is_thumbnail_name(&name));
            if same_scene && old.path() != current {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }
}
/// Copies the bundled thumbnail (shipped read-only next to the exe, made at release time by
/// `--make-thumbnails --thumbnails-out`) of each scene that has none in the cache. Only a file
/// of the same name is used, and the name carries the scene file's hash: a thumbnail of a scene
/// that has since changed (or a scene the bundle never had) is not found, so it renders its own.
/// Returns how many were copied.
fn seed_from(bundled: &Path, cache: &Path, entries: &[Entry]) -> usize {
    let mut copied = 0;
    for name in entries.iter().filter_map(Entry::thumbnail_name) {
        let (from, to) = (bundled.join(&name), cache.join(&name));
        if to.is_file() || !from.is_file() {
            continue;
        }
        let _ = std::fs::create_dir_all(cache);
        let partial = to.with_extension("partial.png");
        if std::fs::copy(&from, &partial).is_ok() && std::fs::rename(&partial, &to).is_ok() {
            copied += 1;
        } else {
            let _ = std::fs::remove_file(&partial);
        }
    }
    copied
}
/// Brings the cache up to date before thumbnails are rendered: stale ones removed, bundled ones
/// copied in. What is still missing afterwards is rendered by `render_thumbnails`.
pub fn sync_thumbnails(entries: &[Entry]) {
    let Some(cache) = crate::dirs::cache() else {
        return;
    };
    remove_stale(&cache, entries);
    if let Some(bundled) = crate::dirs::bundled_thumbnails() {
        seed_from(&bundled, &cache, entries);
    }
}
/// Bump this when an engine change alters what a scene looks like without changing its file
/// (the sun / moon astronomy, the glass lift, a shader fix). It is mixed into every thumbnail's
/// hash, so every thumbnail made before the bump gets a different name: `remove_stale` deletes
/// the old ones and the missing ones are re-rendered. Bundled sets made earlier are never found.
/// 3: the built-ins are auto exposure scenes (E8: the display changed, the files too).
/// Material balls have their own `balls::VERSION`.
pub const RENDER_VERSION: u32 = 3;
/// FNV-1a of the render version, then the scene file: the thumbnail key.
fn thumbnail_hash(bytes: &[u8]) -> u64 {
    RENDER_VERSION
        .to_le_bytes()
        .iter()
        .chain(bytes)
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)
        })
}

/// The shipped scenes folder (see `dirs::scenes`).
pub fn folder() -> Option<PathBuf> {
    crate::dirs::scenes()
}
/// Every scene in every pack folder, in pack order (`pack.json`), then by file name.
pub fn list() -> Vec<Entry> {
    // Pack folders of every root (the user's first); a name in both counts once (the user's).
    let mut packs: Vec<PathBuf> = vec![];
    for root in crate::dirs::scene_roots() {
        for p in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let p = p.path();
            if p.is_dir() && !packs.iter().any(|q| q.file_name() == p.file_name()) {
                packs.push(p);
            }
        }
    }
    // Built-in first, the user's own scenes last, others alphabetically.
    packs.sort_by_key(|p| {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        (name != "builtin", name == "user", name)
    });
    let mut out = vec![];
    for dir in packs {
        let pack: Option<Pack> = std::fs::read(dir.join("pack.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let mut files: Vec<PathBuf> = match &pack {
            // Only plain file names: a pack must not reach outside its folder.
            Some(p) if !p.scenes.is_empty() => p
                .scenes
                .iter()
                .filter(|f| Path::new(f).file_name().is_some_and(|n| n == f.as_str()))
                .map(|f| dir.join(f))
                .collect(),
            _ => {
                let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension().is_some_and(|x| x == "json")
                            && p.file_name().is_some_and(|n| n != "pack.json")
                    })
                    .collect();
                v.sort();
                v
            }
        };
        files.retain(|f| f.is_file());
        let license = pack
            .as_ref()
            .map(|p| License {
                name: p.license.clone(),
                commercial_use: p.commercial_use,
                attribution: p.attribution.clone(),
                source: p.source.clone(),
            })
            .unwrap_or_default();
        let pack_name = pack.map(|p| p.name).unwrap_or_else(|| {
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
        for path in files {
            if let Some(e) = entry(path, pack_name.clone(), license.clone()) {
                out.push(e);
            }
        }
    }
    out
}
/// One scene file's entry (title, description, tags) without its pack's licence; `None` when
/// it is not JSON.
pub fn read_entry(path: &Path) -> Option<Entry> {
    let pack = path.parent()?.file_name()?.to_string_lossy().to_string();
    entry(path.to_path_buf(), pack, License::default())
}
fn entry(path: PathBuf, pack: String, license: License) -> Option<Entry> {
    let bytes = std::fs::read(&path).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
    let text = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let title = match text("title") {
        t if !t.is_empty() => t,
        _ => value
            .get("scene_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or(stem),
    };
    Some(Entry {
        path,
        pack,
        title,
        credits: text("credits"),
        license,
        hash: thumbnail_hash(&bytes),
        description: text("description"),
        tags: value
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}
/// Reads a scene file (wrapper or bare `Settings`) and validates it. The scene name becomes
/// the file's title.
pub fn read(path: &Path) -> Result<Settings> {
    let bytes = std::fs::read(path).with_context(|| format!("Reading {}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("Parsing {}", path.display()))?;
    let mut s = parse(value)?;
    s.validate()
        .with_context(|| format!("Scene {} is invalid", path.display()))?;
    Ok(s)
}
/// A scene file's JSON (a wrapper with a title, a generator and seed, or bare settings) as
/// settings, not yet validated.
pub fn parse(value: serde_json::Value) -> Result<Settings> {
    Ok(if let Some(g) = value.get("generator") {
        // A generated scene: family and seed, regenerated exactly on every load.
        let family = g
            .get("family")
            .and_then(|v| v.as_str())
            .context("generator.family")?;
        let seed = g
            .get("seed")
            .and_then(|v| v.as_u64())
            .context("generator.seed")?;
        let mut s = crate::generators::generate(family, seed)?;
        if let Some(title) = value.get("title").and_then(|v| v.as_str()) {
            s.scene_name = title.to_string();
        }
        s
    } else if value.get("settings").is_some() {
        let file: SceneFile = serde_json::from_value(value)?;
        ensure!(file.format == 1, "Unsupported scene format {}", file.format);
        let mut s = file.settings;
        s.scene_name = file.title;
        s
    } else {
        serde_json::from_value(value)?
    })
}
/// Finds a scene by path, or by file stem or title in the scenes folder.
pub fn find(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(name);
    if path.is_file() {
        return Ok(path);
    }
    let lower = name.to_lowercase();
    list()
        .into_iter()
        .find(|e| {
            e.title.to_lowercase() == lower
                || e.path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase() == lower)
        })
        .map(|e| e.path)
        .with_context(|| format!("No scene file or scene named {name}"))
}
/// Loads a scene into `s`, keeping the viewer's own render and display choices (resolution,
/// RR, upscaler, presets, VSync, HDR), as switching presets does.
pub fn load_into(s: &mut Settings, path: &Path) -> Result<()> {
    let mut loaded = read(path)?;
    loaded.keep_output(s);
    *s = loaded;
    Ok(())
}
pub(crate) fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "scene".into() } else { out }
}
/// Saves the current scene to `scenes/user/<title>.json`, as a wrapper with a title.
/// Saves a generator family and seed to `scenes/user/<title>.json` (a few bytes per scene).
pub fn save_generated(family: &str, seed: u64, title: &str) -> Result<PathBuf> {
    let title = title.trim();
    ensure!(!title.is_empty(), "Give the scene a title");
    let dir = crate::dirs::user_scenes()
        .unwrap_or_else(|| PathBuf::from("scenes"))
        .join("user");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", slug(title)));
    let file = serde_json::json!({
        "format": 1, "title": title, "description": format!("Generated: {family}, seed {seed}."),
        "tags": ["user", "generated", family.replace("still-life", "still life")], "generator": {"family": family, "seed": seed}
    });
    std::fs::write(&path, serde_json::to_string_pretty(&file)?)?;
    Ok(path)
}
pub fn save_user(s: &Settings, title: &str, description: &str) -> Result<PathBuf> {
    let title = title.trim();
    if title.is_empty() {
        bail!("Give the scene a title");
    }
    let root = crate::dirs::user_scenes().unwrap_or_else(|| PathBuf::from("scenes"));
    let path = root.join("user").join(format!("{}.json", slug(title)));
    save_to(&path, s, title, description)?;
    Ok(path)
}
/// Writes a scene file at `path` (Save as…; Ctrl+S on a user scene). The title becomes the
/// scene name; the description and tags of a file being overwritten are kept.
pub fn save_to(path: &Path, s: &Settings, title: &str, description: &str) -> Result<()> {
    let title = title.trim();
    ensure!(!title.is_empty(), "Give the scene a title");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let old: Option<serde_json::Value> = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let tags = old
        .as_ref()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v["tags"].clone()).ok())
        .unwrap_or_else(|| vec!["user".into()]);
    // An overwritten file keeps its description unless a new one is given.
    let description = match (description, &old) {
        ("", Some(v)) => v["description"].as_str().unwrap_or("").to_string(),
        _ => description.to_string(),
    };
    let mut settings = s.clone();
    settings.scene_name = title.into();
    let file = SceneFile {
        format: 1,
        title: title.into(),
        description,
        tags,
        credits: String::new(),
        settings,
    };
    std::fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}

/// A thumbnail run's arguments before the scene's path. On the CPU each run already takes every
/// core, so they stay one at a time; 64 samples keep the noise near the GPU's 32 frames, which
/// RR denoises. Lenient: a scene the CPU can't render in full still gets its picture.
fn thumbnail_args(cpu: bool) -> &'static [&'static str] {
    if cpu {
        &[
            "--reference",
            "64",
            "--reference-lenient",
            "--width",
            "320",
            "--height",
            "200",
            "--scene",
        ]
    } else {
        &[
            "--headless",
            "--frames",
            "32",
            "--width",
            "320",
            "--height",
            "200",
            "--scene",
        ]
    }
}
/// Renders the given scenes' thumbnails one after another on a background thread, by running
/// this exe at 320 x 200: headless on the GPU, or with `cpu` on the CPU renderer (`--reference`,
/// which needs no ray tracing GPU, nor any device). Each finished thumbnail path is sent back.
pub fn render_thumbnails(
    entries: Vec<Entry>,
    cpu: bool,
) -> std::sync::mpsc::Receiver<Result<PathBuf, String>> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        for e in entries {
            let Some(out) = e.thumbnail() else { continue };
            let _ = std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")));
            let partial = out.with_extension("partial.png");
            let mut command = std::process::Command::new(&exe);
            command.args(thumbnail_args(cpu));
            command.arg(&e.path).arg("--capture").arg(&partial);
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            crate::platform::no_window(&mut command); // no console for debug builds
            let result = match command.status() {
                Ok(s) if s.success() && std::fs::rename(&partial, &out).is_ok() => {
                    // Older thumbnails of the same scene (other hashes) are stale.
                    remove_older(&out);
                    Ok(out)
                }
                _ => Err(e.title.clone()),
            };
            if send.send(result).is_err() {
                return; // the window went away
            }
        }
    });
    receive
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The CPU's thumbnail runs need no device and don't fail on what they can't render.
    #[test]
    fn cpu_thumbnails_render_without_a_device() {
        let cpu = thumbnail_args(true);
        assert!(cpu.contains(&"--reference") && cpu.contains(&"--reference-lenient"));
        assert!(!cpu.contains(&"--headless"));
        assert_eq!(cpu.last(), Some(&"--scene"));
        assert_eq!(thumbnail_args(false).last(), Some(&"--scene"));
    }
    fn entry(pack: &str, stem: &str, bytes: &[u8]) -> Entry {
        Entry {
            path: PathBuf::from(format!("scenes/{pack}/{stem}.json")),
            pack: pack.into(),
            title: stem.into(),
            description: String::new(),
            tags: vec![],
            credits: String::new(),
            license: License::default(),
            hash: thumbnail_hash(bytes),
        }
    }
    /// A scratch folder under the target dir (not /tmp), removed first.
    fn scratch(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("thumb-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"png").unwrap();
    }
    /// The owner's rule: the thumbnail is named by the scene file's hash.
    #[test]
    fn a_changed_scene_has_a_different_thumbnail_name() {
        let a = entry("builtin", "room", b"{\"v\":1}");
        let b = entry("builtin", "room", b"{\"v\":2}");
        assert_ne!(a.thumbnail_name(), b.thumbnail_name());
        assert_eq!(
            a.thumbnail_name(),
            entry("builtin", "room", b"{\"v\":1}").thumbnail_name()
        );
        assert!(is_thumbnail_name(&a.thumbnail_name().unwrap()));
        assert!(!is_thumbnail_name("catalog-polyhaven.json"));
        assert!(!is_thumbnail_name(
            "builtin--room-0123456789abcdef.partial.png"
        ));
    }
    /// The render version is part of the key: a bump orphans every older thumbnail.
    #[test]
    fn the_render_version_is_part_of_the_thumbnail_hash() {
        let fnv = |bytes: &[u8]| {
            bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)
            })
        };
        let scene = b"{\"v\":1}";
        let with = |version: u32| fnv(&[&version.to_le_bytes()[..], scene].concat());
        assert_eq!(thumbnail_hash(scene), with(RENDER_VERSION));
        assert_ne!(thumbnail_hash(scene), with(RENDER_VERSION - 1));
        assert_ne!(thumbnail_hash(scene), fnv(scene));
    }
    #[test]
    fn a_bundled_thumbnail_is_used_when_the_hash_matches() {
        let (bundle, cache) = (scratch("seed-match-b"), scratch("seed-match-c"));
        let e = entry("builtin", "room", b"scene v1");
        touch(&bundle, &e.thumbnail_name().unwrap());
        assert_eq!(seed_from(&bundle, &cache, std::slice::from_ref(&e)), 1);
        assert!(cache.join(e.thumbnail_name().unwrap()).is_file());
        // The bundle is only read; a second call has nothing to do.
        assert!(bundle.join(e.thumbnail_name().unwrap()).is_file());
        assert_eq!(seed_from(&bundle, &cache, &[e]), 0);
    }
    #[test]
    fn a_bundled_thumbnail_is_ignored_when_the_scene_changed() {
        let (bundle, cache) = (scratch("seed-changed-b"), scratch("seed-changed-c"));
        touch(
            &bundle,
            &entry("builtin", "room", b"scene v1")
                .thumbnail_name()
                .unwrap(),
        );
        let edited = entry("builtin", "room", b"scene v2");
        let new = entry("user", "mine", b"brand new");
        assert_eq!(
            seed_from(&bundle, &cache, &[edited.clone(), new.clone()]),
            0
        );
        // Both are still missing, so they get rendered.
        assert!(!cache.join(edited.thumbnail_name().unwrap()).exists());
        assert!(!cache.join(new.thumbnail_name().unwrap()).exists());
    }
    #[test]
    fn stale_cached_thumbnails_are_removed() {
        let cache = scratch("stale");
        let old = entry("builtin", "room", b"scene v1");
        let now = entry("builtin", "room", b"scene v2");
        let other = entry("builtin", "hall", b"hall");
        let gone = entry("user", "deleted", b"deleted");
        for e in [&old, &now, &other, &gone] {
            touch(&cache, &e.thumbnail_name().unwrap());
        }
        touch(&cache, "catalog-polyhaven.json");
        std::fs::create_dir_all(cache.join("materials")).unwrap();
        // Older hash of an edited scene and a deleted scene go; current ones, other files and
        // folders stay.
        assert_eq!(remove_stale(&cache, &[now.clone(), other.clone()]), 2);
        assert!(!cache.join(old.thumbnail_name().unwrap()).exists());
        assert!(!cache.join(gone.thumbnail_name().unwrap()).exists());
        assert!(cache.join(now.thumbnail_name().unwrap()).is_file());
        assert!(cache.join(other.thumbnail_name().unwrap()).is_file());
        assert!(cache.join("catalog-polyhaven.json").is_file());
        assert!(cache.join("materials").is_dir());
    }
    #[test]
    fn a_rendered_thumbnail_replaces_the_older_ones_of_its_scene() {
        let cache = scratch("older");
        let old = entry("builtin", "room", b"scene v1");
        let now = entry("builtin", "room", b"scene v2");
        let other = entry("builtin", "room-2", b"other");
        for e in [&old, &now, &other] {
            touch(&cache, &e.thumbnail_name().unwrap());
        }
        remove_older(&cache.join(now.thumbnail_name().unwrap()));
        assert!(!cache.join(old.thumbnail_name().unwrap()).exists());
        assert!(cache.join(now.thumbnail_name().unwrap()).is_file());
        assert!(cache.join(other.thumbnail_name().unwrap()).is_file());
    }
    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(slug("Stonehenge at dusk"), "stonehenge-at-dusk");
        assert_eq!(slug("  ../../evil?  "), "evil");
        assert_eq!(slug("***"), "scene");
    }
    /// Every shipped scene parses, validates and keeps its title.
    #[test]
    fn builtin_scenes_are_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/builtin");
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|x| x == "json")
                && path.file_name().is_some_and(|n| n != "pack.json")
            {
                let s = read(&path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
                assert!(!s.scene_name.is_empty(), "{}", path.display());
                assert!(
                    !crate::scene::PRESETS.contains(&s.scene_name.as_str()),
                    "{} reuses a built-in preset name",
                    path.display()
                );
                count += 1;
            }
        }
        assert!(count >= 42, "only {count} scene files");
    }
}
