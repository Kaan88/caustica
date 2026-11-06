//! UI preferences (UI plan 7.6): favourite and recent scene files, the UI size, the advanced
//! toggle and the first-run hint, kept in %APPDATA%\Caustica\prefs.json beside the stats.
//! Headless runs (captures, the self-test) use defaults and never write the file.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many recent scenes are remembered.
pub const RECENT: usize = 12;
/// The default file name of quick saves: two saves never land on the same file.
const DEFAULT_NAME: &str = "{scene}-{date}-{time}";

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    /// Scene files by `key`: the path relative to the scenes folder, with forward slashes.
    pub favorites: Vec<String>,
    /// Most recent first.
    pub recent: Vec<String>,
    /// egui zoom (Ctrl +/−).
    pub zoom: f32,
    pub advanced: bool,
    /// The first-run hint was dismissed.
    pub hint_seen: bool,
    /// Text-to-scene backend (no keys: those stay in memory).
    pub ai: crate::textscene::AiConfig,
    /// Saved text-to-scene backends, picked in the Describe a scene section (no keys).
    pub ai_profiles: Vec<crate::textscene::AiConfig>,
    /// Laser puzzle (track 17): fewest moves per solved level, by scene name.
    pub puzzle_best: std::collections::BTreeMap<String, u32>,
    /// File name of quick saves and high-resolution stills (`video::expand_name`).
    pub name_template: String,
    /// The old default template ("caustica", which overwrote every save) was replaced once.
    pub name_template_migrated: bool,
    /// Windows: render with Vulkan instead of D3D12 (Menu → Renderer; from the next start).
    pub vulkan: bool,
    /// Look for a new release at startup (update.rs).
    pub update_check: bool,
    /// The version the user chose to skip.
    pub update_skip: String,
    /// Where the UI was left: tab, gizmo tool, Local axes, pivot, snap target, panel width.
    pub tab: crate::ui::Tab,
    pub tool: crate::gizmo::Tool,
    pub local: bool,
    pub pivot: crate::gizmo::Pivot,
    pub snap_to: crate::snap::Target,
    pub panel_width: f32,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            favorites: Vec::new(),
            recent: Vec::new(),
            zoom: 1.,
            advanced: false,
            hint_seen: false,
            ai: Default::default(),
            ai_profiles: Vec::new(),
            puzzle_best: Default::default(),
            name_template: DEFAULT_NAME.into(),
            name_template_migrated: false,
            vulkan: false,
            update_check: true,
            update_skip: String::new(),
            tab: crate::ui::Tab::Scene,
            tool: crate::gizmo::Tool::Move,
            local: false,
            pivot: crate::gizmo::Pivot::Median,
            snap_to: crate::snap::Target::Increment,
            panel_width: 370.,
        }
    }
}
/// %APPDATA%\Caustica, or beside the exe when portable (see `dirs::config`).
pub fn app_dir() -> Option<PathBuf> {
    crate::dirs::config()
}
fn path() -> Option<PathBuf> {
    app_dir().map(|d| d.join("prefs.json"))
}
impl Prefs {
    pub fn load() -> Self {
        let mut p: Prefs = path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        p.zoom = if p.zoom.is_finite() {
            p.zoom.clamp(0.5, 3.)
        } else {
            1.
        };
        p.panel_width = if p.panel_width.is_finite() {
            p.panel_width.clamp(320., 560.)
        } else {
            370.
        };
        p.recent.truncate(RECENT);
        p.migrate();
        p
    }
    /// A file from before the dated default still says "caustica". Once only: someone who types
    /// that name later keeps it.
    fn migrate(&mut self) {
        if !self.name_template_migrated && self.name_template == "caustica" {
            self.name_template = DEFAULT_NAME.into();
        }
        self.name_template_migrated = true;
    }
    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, json);
            }
        }
    }
    pub fn is_favorite(&self, scene: &Path) -> bool {
        self.favorites.contains(&key(scene))
    }
    pub fn toggle_favorite(&mut self, scene: &Path) {
        let k = key(scene);
        match self.favorites.iter().position(|f| *f == k) {
            Some(i) => {
                self.favorites.remove(i);
            }
            None => self.favorites.push(k),
        }
    }
    /// Puts `scene` first in the recent list.
    pub fn opened(&mut self, scene: &Path) {
        let k = key(scene);
        self.recent.retain(|r| *r != k);
        self.recent.insert(0, k);
        self.recent.truncate(RECENT);
    }
    /// Position in the recent list (0 = latest).
    pub fn recency(&self, scene: &Path) -> Option<usize> {
        let k = key(scene);
        self.recent.iter().position(|r| *r == k)
    }
}
/// What Restore last session brings back: the scene, and the file Ctrl+S targets.
#[derive(Serialize, Deserialize)]
pub struct Session<S = crate::scene::Settings> {
    pub settings: S,
    #[serde(default)]
    pub loaded_file: Option<(PathBuf, String)>,
}
/// last-session.json beside the preferences (interactive runs only).
pub fn session_path() -> Option<PathBuf> {
    app_dir().map(|d| d.join("last-session.json"))
}
/// Written beside the file and renamed over it, so a crash mid-write leaves the last good one.
pub fn write_session(
    path: &Path,
    settings: &crate::scene::Settings,
    loaded_file: &Option<(PathBuf, String)>,
) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let session = Session {
        settings,
        loaded_file: loaded_file.clone(),
    };
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(&session)?)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}
/// The saved session, when it parses and passes the same checks as a settings file.
pub fn read_session(path: &Path) -> Option<Session> {
    let mut session: Session = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    session.settings.validate().ok()?;
    Some(session)
}
/// A scene file's key: relative to the scenes folder when inside it, so a moved install keeps
/// its favourites.
pub fn key(scene: &Path) -> String {
    // Both canonical: the list holds relative paths, file dialogs give absolute ones.
    crate::dirs::relative_to_scenes(scene).unwrap_or_else(|| {
        let full = std::fs::canonicalize(scene).unwrap_or_else(|_| scene.to_path_buf());
        full.to_string_lossy().replace('\\', "/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_scenes_move_to_the_front_and_favourites_toggle() {
        let mut p = Prefs::default();
        for name in ["a.json", "b.json", "a.json"] {
            p.opened(Path::new(name));
        }
        assert_eq!(p.recent, ["a.json", "b.json"]);
        for i in 0..20 {
            p.opened(Path::new(&format!("{i}.json")));
        }
        assert_eq!(p.recent.len(), RECENT);
        assert_eq!(p.recency(Path::new("19.json")), Some(0));
        p.toggle_favorite(Path::new("x.json"));
        assert!(p.is_favorite(Path::new("x.json")));
        p.toggle_favorite(Path::new("x.json"));
        assert!(!p.is_favorite(Path::new("x.json")));
        // Unknown fields and missing ones are fine: older and newer files load.
        let p: Prefs = serde_json::from_str(r#"{"zoom": 1.5, "future": 1}"#).unwrap();
        assert_eq!((p.zoom, p.favorites.len()), (1.5, 0));
    }
    #[test]
    fn the_old_default_name_migrates_once() {
        // A file from before the marker, with the old default saved in it.
        let mut p: Prefs = serde_json::from_str(r#"{"name_template": "caustica"}"#).unwrap();
        p.migrate();
        assert_eq!(p.name_template, DEFAULT_NAME);
        // Typed afterwards, it stays, also through a save and load.
        p.name_template = "caustica".into();
        let mut p: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        p.migrate();
        assert_eq!(p.name_template, "caustica");
        // Another name is never touched.
        let mut p: Prefs = serde_json::from_str(r#"{"name_template": "shot-{spp}"}"#).unwrap();
        p.migrate();
        assert_eq!(p.name_template, "shot-{spp}");
        // Where the UI was left survives the file, and older files load without it.
        let mut p = Prefs::default();
        (p.tool, p.tab, p.pivot) = (
            crate::gizmo::Tool::Rotate,
            crate::ui::Tab::Look,
            crate::gizmo::Pivot::Cursor,
        );
        let back: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(Prefs::default().tool, crate::gizmo::Tool::Move);
    }
    #[test]
    fn a_session_round_trips_and_a_bad_one_is_ignored() {
        let dir = std::env::temp_dir().join(format!("caustica-session-{}", std::process::id()));
        let path = dir.join("last-session.json");
        let mut s = crate::scene::Settings::default();
        s.exposure = 2.5;
        let file = Some((PathBuf::from("scenes/user/a.json"), "a".to_string()));
        write_session(&path, &s, &file).unwrap();
        let back = read_session(&path).unwrap();
        assert!(back.settings == s);
        assert_eq!(back.loaded_file, file);
        assert!(!path.with_extension("json.tmp").exists());
        // A second write replaces the first (Windows' rename too).
        s.exposure = 3.;
        write_session(&path, &s, &None).unwrap();
        assert_eq!(read_session(&path).unwrap().settings.exposure, 3.);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_session(&path).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(read_session(&path).is_none());
    }
}
