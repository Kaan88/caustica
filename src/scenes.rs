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
    /// FNV-1a of the file, so a thumbnail is re-rendered when the scene changes.
    pub hash: u64,
}
impl Entry {
    /// `scenes/thumbnails/<pack folder>--<file>-<hash>.png`.
    pub fn thumbnail(&self) -> Option<PathBuf> {
        let pack = self
            .path
            .parent()?
            .file_name()?
            .to_string_lossy()
            .to_string();
        let stem = self.path.file_stem()?.to_string_lossy().to_string();
        Some(
            folder()?
                .join("thumbnails")
                .join(format!("{pack}--{stem}-{:016x}.png", self.hash)),
        )
    }
}
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// The scenes folder: next to the working directory (the launcher runs in `native/`), next to
/// the exe, or two levels up from it (`native/target/release`).
pub fn folder() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let candidates = [
        Some(PathBuf::from("scenes")),
        exe.as_ref()
            .and_then(|e| e.parent().map(|p| p.join("scenes"))),
        exe.as_ref()
            .and_then(|e| e.parent()?.parent()?.parent().map(|p| p.join("scenes"))),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}
/// Every scene in every pack folder, in pack order (`pack.json`), then by file name.
pub fn list() -> Vec<Entry> {
    let Some(root) = folder() else { return vec![] };
    let mut packs: Vec<PathBuf> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
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
        hash: fnv(&bytes),
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
    let mut s: Settings = if let Some(g) = value.get("generator") {
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
    };
    s.validate()
        .with_context(|| format!("Scene {} is invalid", path.display()))?;
    Ok(s)
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
    let dir = folder()
        .unwrap_or_else(|| PathBuf::from("scenes"))
        .join("user");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", slug(title)));
    let file = serde_json::json!({
        "format": 1, "title": title, "description": format!("Generated: {family}, seed {seed}."),
        "tags": ["user", "generated", family], "generator": {"family": family, "seed": seed}
    });
    std::fs::write(&path, serde_json::to_string_pretty(&file)?)?;
    Ok(path)
}
pub fn save_user(s: &Settings, title: &str, description: &str) -> Result<PathBuf> {
    let title = title.trim();
    if title.is_empty() {
        bail!("Give the scene a title");
    }
    let root = folder().unwrap_or_else(|| PathBuf::from("scenes"));
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

/// Renders the given scenes' thumbnails one after another on a background thread, by running
/// this exe headless (320 x 200, 32 frames). Each finished thumbnail path is sent back.
pub fn render_thumbnails(
    entries: Vec<Entry>,
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
            command.args([
                "--headless",
                "--frames",
                "32",
                "--width",
                "320",
                "--height",
                "200",
                "--scene",
            ]);
            command.arg(&e.path).arg("--capture").arg(&partial);
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console for debug builds
            }
            let result = match command.status() {
                Ok(s) if s.success() && std::fs::rename(&partial, &out).is_ok() => {
                    // Older thumbnails of the same scene (other hashes) are stale.
                    let prefix = out
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .rsplit_once('-')
                        .map(|(p, _)| p.to_string());
                    if let (Some(prefix), Some(dir)) = (prefix, out.parent()) {
                        for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                            let name = old.file_name().to_string_lossy().to_string();
                            if name.starts_with(&format!("{prefix}-")) && old.path() != out {
                                let _ = std::fs::remove_file(old.path());
                            }
                        }
                    }
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
