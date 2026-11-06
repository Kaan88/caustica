//! UI preferences (UI plan 7.6): favourite and recent scene files, the UI size, the advanced
//! toggle and the first-run hint, kept in %APPDATA%\Caustica\prefs.json beside the stats.
//! Headless runs (captures, the self-test) use defaults and never write the file.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many recent scenes are remembered.
pub const RECENT: usize = 12;

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
    /// Windows: render with Vulkan instead of D3D12 (Menu → Renderer; from the next start).
    pub vulkan: bool,
    /// Look for a new release at startup (update.rs).
    pub update_check: bool,
    /// The version the user chose to skip.
    pub update_skip: String,
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
            name_template: "caustica".into(),
            vulkan: false,
            update_check: true,
            update_skip: String::new(),
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
        p.recent.truncate(RECENT);
        p
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
}
