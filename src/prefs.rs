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
    #[serde(deserialize_with = "lenient_tab")]
    pub tab: crate::ui::Tab,
    pub tool: crate::gizmo::Tool,
    pub local: bool,
    pub pivot: crate::gizmo::Pivot,
    pub snap_to: crate::snap::Target,
    pub panel_width: f32,
    /// The outliner's share of the right column's height.
    pub outliner_split: f32,
    /// The top bar's pinned buttons, left to right (palette entries by name). Read leniently:
    /// a name this version doesn't know is skipped (`pins`). A pin drawn as its icon alone is
    /// `{"pin": …, "icon_only": true}` instead of the bare pin.
    pub top_pins: Vec<serde_json::Value>,
    /// Export tab: show the file in its folder when an export ends.
    pub open_folder_done: bool,
    /// The last exports, newest first (`UiState::export_ended`).
    pub recent_exports: Vec<RecentExport>,
    /// Scenes browser: the cards' order and their size (0 small, 1 medium, 2 large).
    #[serde(deserialize_with = "lenient")]
    pub scene_sort: SceneSort,
    #[serde(deserialize_with = "lenient")]
    pub scene_cards: u8,
}
/// The Scenes browser's card orders.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SceneSort {
    /// As listed: by pack, under a heading per pack.
    #[default]
    Pack,
    Name,
    /// The remembered recent scenes first, latest first.
    Recent,
    Favorites,
    /// By the file's date, newest first.
    Modified,
}
impl SceneSort {
    pub const ALL: [(SceneSort, &str); 5] = [
        (SceneSort::Pack, "Pack"),
        (SceneSort::Name, "Name"),
        (SceneSort::Recent, "Recently used"),
        (SceneSort::Favorites, "Favorites first"),
        (SceneSort::Modified, "Date modified"),
    ];
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(s, _)| *s == self)
            .map_or("", |(_, name)| name)
    }
}
/// A value this version can't read (a sort order from a newer one) becomes the default, like
/// `lenient_tab`.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}
/// A tab name this version doesn't know (a file from a newer one) reads as the default tab,
/// instead of failing the whole file and with it the favourites and recents.
fn lenient_tab<'de, D: serde::Deserializer<'de>>(d: D) -> Result<crate::ui::Tab, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or(crate::ui::Tab::Object))
}
/// One line of the Export tab's Recent exports.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct RecentExport {
    pub file: String,
    /// "saved", "cancelled" or "failed".
    pub state: String,
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
            tab: crate::ui::Tab::Object,
            tool: crate::gizmo::Tool::Move,
            local: false,
            pivot: crate::gizmo::Pivot::Median,
            snap_to: crate::snap::Target::Increment,
            panel_width: 370.,
            outliner_split: 0.3,
            top_pins: default_pins(),
            open_folder_done: false,
            recent_exports: Vec::new(),
            scene_sort: SceneSort::Pack,
            scene_cards: 1,
        }
    }
}
/// The right column's width range, in points (the tab strip takes 36 of them).
pub const PANEL_WIDTH: (f32, f32) = (360., 560.);
/// A top-bar button: a palette command, a window or a live widget.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pin {
    Do(crate::palette::Do),
    Window(crate::palette::Win),
    Widget(crate::palette::Widget),
}
/// A pin with its choices, as the file holds one that has any.
#[derive(Serialize, Deserialize)]
struct PinEntry {
    pin: Pin,
    #[serde(default)]
    icon_only: bool,
}
/// One `top_pins` entry: the pin and "Icon only". None for what this version doesn't know.
fn read_pin(v: &serde_json::Value) -> Option<(Pin, bool)> {
    if let Ok(p) = serde_json::from_value::<Pin>(v.clone()) {
        return Some((p, false));
    }
    serde_json::from_value::<PinEntry>(v.clone())
        .ok()
        .map(|e| (e.pin, e.icon_only))
}
/// The bare pin, so a file without choices reads as it always did.
fn write_pin(pin: Pin, icon_only: bool) -> serde_json::Value {
    if icon_only {
        serde_json::to_value(PinEntry { pin, icon_only })
    } else {
        serde_json::to_value(pin)
    }
    .expect("a pin serializes")
}
fn default_pins() -> Vec<serde_json::Value> {
    use crate::palette::{Do, Win};
    [
        Pin::Window(Win::Scenes),
        Pin::Window(Win::Assets),
        Pin::Do(Do::Compare),
        Pin::Do(Do::SavePng),
    ]
    .iter()
    .map(|p| write_pin(*p, false))
    .collect()
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
            p.panel_width.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1)
        } else {
            370.
        };
        p.outliner_split = if p.outliner_split.is_finite() {
            p.outliner_split.clamp(0.1, 0.45)
        } else {
            0.3
        };
        p.recent.truncate(RECENT);
        p.scene_cards = p.scene_cards.min(2);
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
    /// The pinned buttons this version knows, in order, without repeats.
    pub fn pins(&self) -> Vec<Pin> {
        let mut pins: Vec<Pin> = Vec::new();
        for v in &self.top_pins {
            if let Some((p, _)) = read_pin(v)
                && !pins.contains(&p)
            {
                pins.push(p);
            }
        }
        pins
    }
    /// The pin is drawn as its icon alone ("Icon only" in its right-click menu). The first
    /// entry of a pin counts, as in `pins`.
    pub fn pin_icon_only(&self, pin: Pin) -> bool {
        self.top_pins
            .iter()
            .filter_map(read_pin)
            .find(|(p, _)| *p == pin)
            .is_some_and(|(_, icon_only)| icon_only)
    }
    pub fn set_pin_icon_only(&mut self, pin: Pin, on: bool) {
        let mut seen = false;
        for v in &mut self.top_pins {
            if read_pin(v).is_some_and(|(p, _)| p == pin) && !seen {
                *v = write_pin(pin, on);
                seen = true;
            }
        }
    }
    /// Replaces the known pins and keeps the entries of a newer version at the end. A pin that
    /// stays keeps its "Icon only"; one that is pinned again later starts with its label.
    pub fn set_pins(&mut self, pins: &[Pin]) {
        let unknown: Vec<_> = self
            .top_pins
            .iter()
            .filter(|v| read_pin(v).is_none())
            .cloned()
            .collect();
        self.top_pins = pins
            .iter()
            .map(|p| write_pin(*p, self.pin_icon_only(*p)))
            .chain(unknown)
            .collect();
    }
    pub fn reset_pins(&mut self) {
        self.top_pins = default_pins();
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
    /// A tab from another version costs the tab, not the file.
    #[test]
    fn an_unknown_tab_keeps_the_rest_of_the_file() {
        let p: Prefs = serde_json::from_str(r#"{"tab":"Nodes","zoom":1.5}"#).unwrap();
        assert_eq!((p.tab, p.zoom), (crate::ui::Tab::Object, 1.5));
        let p: Prefs = serde_json::from_str(r#"{"tab":"Scene"}"#).unwrap();
        assert_eq!(p.tab, crate::ui::Tab::Object);
        let p: Prefs = serde_json::from_str(r#"{"tab":"Look"}"#).unwrap();
        assert_eq!(p.tab, crate::ui::Tab::Look);
    }
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
    /// The Scenes browser's sort and card size survive the file; a sort this version doesn't
    /// know reads as the default and keeps the rest.
    #[test]
    fn the_scene_sort_round_trips_and_reads_leniently() {
        let mut p = Prefs::default();
        assert_eq!((p.scene_sort, p.scene_cards), (SceneSort::Pack, 1));
        (p.scene_sort, p.scene_cards) = (SceneSort::Modified, 2);
        let back: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        let p: Prefs =
            serde_json::from_str(r#"{"scene_sort":"Rating","scene_cards":-1,"zoom":1.5}"#).unwrap();
        assert_eq!(
            (p.scene_sort, p.scene_cards, p.zoom),
            (SceneSort::Pack, 0, 1.5)
        );
        for (sort, name) in SceneSort::ALL {
            assert_eq!(sort.name(), name);
        }
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
    /// "Icon only" lives in the pin's own entry: bare pins and unknown entries read as before,
    /// the choice survives reordering and a save, and unpinning or the reset drops it.
    #[test]
    fn icon_only_pins_round_trip_beside_bare_and_unknown_ones() {
        use crate::palette::{Do, Win};
        let (scenes, compare) = (Pin::Window(Win::Scenes), Pin::Do(Do::Compare));
        let mut p: Prefs = serde_json::from_str(
            r#"{"top_pins":[{"Window":"Scenes"},{"Do":"FromTheFuture"},
                {"pin":{"Do":"Compare"},"icon_only":true},{"pin":{"Do":"Later"},"icon_only":true},
                {"pin":{"Window":"Scenes"},"icon_only":true}]}"#,
        )
        .unwrap();
        assert_eq!(p.pins(), [scenes, compare]);
        assert!(!p.pin_icon_only(scenes) && p.pin_icon_only(compare));
        p.set_pin_icon_only(scenes, true);
        p.set_pins(&[compare, scenes]);
        let mut p: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p.pins(), [compare, scenes]);
        assert!(p.pin_icon_only(scenes) && p.pin_icon_only(compare));
        // Both entries of a newer version are still in the file.
        assert_eq!(p.top_pins.len(), 4);
        p.set_pin_icon_only(compare, false);
        assert_eq!(p.top_pins[0], serde_json::json!({"Do": "Compare"}));
        p.set_pins(&[compare]);
        p.set_pins(&[compare, scenes]);
        assert!(!p.pin_icon_only(scenes));
        p.set_pin_icon_only(scenes, true);
        p.reset_pins();
        assert!(!p.pin_icon_only(scenes) && p.top_pins == Prefs::default().top_pins);
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
