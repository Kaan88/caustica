//! Where the app reads and writes its files.
//!
//! Three modes, decided once from where the exe is:
//! - **Installed** (the default, e.g. Program Files or the installer's per-user folder): the exe
//!   folder is read-only. Settings, prefs, stats, user scenes, downloaded assets and captures go
//!   to `%APPDATA%\Caustica`; thumbnails and asset catalogs (caches) to `%LOCALAPPDATA%\Caustica`.
//!   Uninstalling removes only the exe folder.
//! - **Portable** (a file named `portable` beside the exe): everything stays beside the exe,
//!   prefs and stats included.
//! - **Development** (the exe is inside a cargo `target` folder): the repo's `scenes/` holds user
//!   scenes, downloads and thumbnails as before, captures go to the working directory, and prefs
//!   and stats stay in `%APPDATA%\Caustica`.
//!
//! The first installed start copies files an older build wrote beside the exe (user scenes,
//! downloaded assets with their manifest, settings.json, captures) into the per-user folders.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Installed,
    Portable,
    Development,
}
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}
fn detect(exe_dir: Option<&Path>) -> Mode {
    let Some(dir) = exe_dir else {
        return Mode::Development;
    };
    if dir.join("portable").exists() {
        return Mode::Portable;
    }
    if dir.ancestors().any(|a| {
        a.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("target"))
    }) {
        return Mode::Development;
    }
    Mode::Installed
}
pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| detect(exe_dir().as_deref()))
}
#[cfg(windows)]
fn known(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(|v| PathBuf::from(v).join("Caustica"))
}
/// Linux: %APPDATA% is the XDG config folder and %LOCALAPPDATA% the XDG cache folder.
#[cfg(not(windows))]
fn known(var: &str) -> Option<PathBuf> {
    let (xdg, fallback) = match var {
        "APPDATA" => ("XDG_CONFIG_HOME", ".config"),
        _ => ("XDG_CACHE_HOME", ".cache"),
    };
    std::env::var_os(xdg)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
        .map(|d| d.join("caustica"))
}
/// %APPDATA%\Caustica, or the exe folder when portable. Builds before the rename kept prefs in
/// %APPDATA%\CornellRTX: that folder is moved over once.
pub fn config() -> Option<PathBuf> {
    if mode() == Mode::Portable {
        return exe_dir();
    }
    let dir = known("APPDATA")?;
    let old = dir.with_file_name("CornellRTX");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    Some(dir)
}

/// The shipped scenes folder (read-only when installed): next to the working directory (the
/// launcher runs in the repo), next to the exe, or three levels up (`target/<dir>/release`).
pub fn scenes() -> Option<PathBuf> {
    let exe = exe_dir();
    let candidates = [
        (mode() == Mode::Development).then(|| PathBuf::from("scenes")),
        exe.as_ref().map(|e| e.join("scenes")),
        exe.as_ref()
            .and_then(|e| e.parent()?.parent().map(|p| p.join("scenes"))),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}
/// Where the user's scenes (`user/`), downloaded assets (`hdri/`, `models/`, `textures/`) and
/// their manifest (`assets.json`) go: %APPDATA%\Caustica\scenes when installed, else the
/// shipped scenes folder.
pub fn user_scenes() -> Option<PathBuf> {
    match mode() {
        Mode::Installed => config().map(|c| c.join("scenes")),
        _ => scenes().or_else(|| Some(PathBuf::from("scenes"))),
    }
}
/// The folders a relative scene path (a mesh, texture or HDRI) is looked up in, user first.
pub fn scene_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = user_scenes().into_iter().collect();
    if let Some(s) = scenes()
        && !v.iter().any(|u| same(u, &s))
    {
        v.push(s);
    }
    v
}
fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
/// A relative scene path resolved against the roots: the first root holding the file, else the
/// user root (where a download would put it).
pub fn resolve_scene_path(rel: &Path) -> PathBuf {
    let roots = scene_roots();
    roots
        .iter()
        .map(|r| r.join(rel))
        .find(|p| p.exists())
        .or_else(|| roots.first().map(|r| r.join(rel)))
        .unwrap_or_else(|| rel.to_path_buf())
}
/// `path` relative to whichever scene root holds it, with forward slashes.
pub fn relative_to_scenes(path: &Path) -> Option<String> {
    let full = path.canonicalize().ok()?;
    scene_roots().into_iter().find_map(|root| {
        let root = root.canonicalize().ok()?;
        full.strip_prefix(&root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    })
}
static CACHE_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
/// Points `cache()` somewhere else for this run (`--make-thumbnails --thumbnails-out DIR` renders
/// the release's bundled set there).
pub fn set_cache_override(dir: PathBuf) {
    let _ = CACHE_OVERRIDE.set(dir);
}
/// True on a `--thumbnails-out` run, whose `cache()` is the release's bundled folder: per-machine
/// caches (the Vulkan pipeline cache) must stay out of it.
pub fn cache_overridden() -> bool {
    CACHE_OVERRIDE.get().is_some()
}
/// The thumbnails shipped with the app (`thumbnails` beside the exe, read-only, made at release
/// time): scene thumbnails named by their scene's hash, and `materials/` with the material
/// balls. `scenes::sync_thumbnails` and `balls` copy or read from here and ignore any file whose
/// hash no longer matches. None in a development build or a snapshot, which have no such folder.
pub fn bundled_thumbnails() -> Option<PathBuf> {
    exe_dir()
        .map(|e| e.join("thumbnails"))
        .filter(|d| d.is_dir())
}
/// Scene thumbnails and the asset browser's catalogs and previews (deletable caches):
/// %LOCALAPPDATA%\Caustica\thumbnails when installed, else `scenes/thumbnails`.
pub fn cache() -> Option<PathBuf> {
    if let Some(dir) = CACHE_OVERRIDE.get() {
        return Some(dir.clone());
    }
    match mode() {
        Mode::Installed => known("LOCALAPPDATA")
            .or_else(config)
            .map(|c| c.join("thumbnails")),
        _ => user_scenes().map(|s| s.join("thumbnails")),
    }
}
/// The folder for saved images, videos and settings files: %APPDATA%\Caustica\captures when
/// installed, the working directory otherwise (empty path).
pub fn output_dir() -> PathBuf {
    match mode() {
        Mode::Installed => config().map(|c| c.join("captures")).unwrap_or_default(),
        _ => PathBuf::new(),
    }
}
/// Reference images and the like: the output folder when installed, else `captures/`.
pub fn captures() -> PathBuf {
    match mode() {
        Mode::Installed => output_dir(),
        _ => PathBuf::from("captures"),
    }
}
/// `name` inside [`output_dir`], creating the folder.
pub fn output(name: impl AsRef<Path>) -> PathBuf {
    let dir = output_dir();
    if !dir.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir.join(name)
}

/// Copies what an older installed build wrote beside the exe into the per-user folders, once
/// (a `migrated` marker in the config folder, written once something was copied). Copies, not
/// moves: the exe folder may be read-only, and the old files go with the uninstall.
pub fn migrate() {
    if mode() != Mode::Installed {
        return;
    }
    let (Some(config), Some(exe), Some(user)) = (config(), exe_dir(), user_scenes()) else {
        return;
    };
    let marker = config.join("migrated");
    if marker.exists() {
        return;
    }
    let old = exe.join("scenes");
    let mut copied = 0;
    copied += copy_tree(&old.join("user"), &user.join("user"));
    // Downloads: every file the manifest lists (plus the manifest), not the shipped folders.
    if let Ok(bytes) = std::fs::read(old.join("assets.json")) {
        if let Ok(m) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(files) = m.get("files").and_then(|f| f.as_object())
        {
            for rel in files.keys() {
                let rel = Path::new(rel);
                if rel.is_absolute() || rel.components().any(|c| c.as_os_str() == "..") {
                    continue;
                }
                // Zips are extracted on download; their maps sit in the zip's folder.
                let src = old.join(rel);
                if src.is_file() {
                    copied += copy_file(&src, &user.join(rel));
                } else if let Some(dir) = rel.parent()
                    && dir.as_os_str() != ""
                {
                    copied += copy_tree(&old.join(dir), &user.join(dir));
                }
            }
        }
        if !user.join("assets.json").exists() {
            let _ = std::fs::create_dir_all(&user);
            if std::fs::write(user.join("assets.json"), &bytes).is_ok() {
                copied += 1;
            }
        }
    }
    let out = output_dir();
    copied += copy_tree(&exe.join("captures"), &out);
    if exe.join("settings.json").is_file() {
        copied += copy_file(&exe.join("settings.json"), &out.join("settings.json"));
    }
    // Only a migration that copied something is final: an installed-mode run from a folder
    // with nothing to bring over (a test copy in a temp folder) must not block the real one.
    if copied == 0 {
        return;
    }
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::write(
        &marker,
        format!("{copied} files copied from {}\n", exe.display()),
    );
}
fn copy_file(src: &Path, dst: &Path) -> usize {
    if dst.exists() {
        return 0;
    }
    if let Some(p) = dst.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::copy(src, dst).is_ok() as usize
}
fn copy_tree(src: &Path, dst: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(src) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            n += copy_tree(&p, &dst.join(e.file_name()));
        } else {
            n += copy_file(&p, &dst.join(e.file_name()));
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_follows_the_exe_folder() {
        let tmp = std::env::temp_dir().join("caustica-test-dirs");
        let installed = tmp.join("Programs").join("Caustica");
        let portable = tmp.join("portable-copy");
        let _ = std::fs::create_dir_all(&installed);
        let _ = std::fs::create_dir_all(&portable);
        std::fs::write(portable.join("portable"), "").unwrap();
        assert_eq!(detect(Some(&installed)), Mode::Installed);
        assert_eq!(detect(Some(&portable)), Mode::Portable);
        assert_eq!(
            detect(Some(
                &Path::new("src")
                    .join("caustica")
                    .join("target")
                    .join("release")
            )),
            Mode::Development
        );
        // Tests run from target/…/deps.
        assert_eq!(mode(), Mode::Development);
        let n = copy_tree(&portable, &tmp.join("copy"));
        assert_eq!(n, 1);
        assert_eq!(
            copy_tree(&portable, &tmp.join("copy")),
            0,
            "never overwrites"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
