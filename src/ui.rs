//! The interface (UI plan phase 1): top bar, a right side panel with Scene / Render / Look /
//! Export tabs, a status bar and toasts. UI-only state lives in `UiState`, never in `Settings`,
//! because every `Settings` field outside the exclusion list restarts the render.
mod ai;
mod chrome;
mod look_tab;
mod overlay;
mod render_tab;
mod scene_tab;
mod stage;
mod windows;
use crate::scene::{self, DLSS_PRESETS, PRESETS, Settings, Shape, Upscaler};
use crate::{App, editor, gizmo};
use stage::*;
use std::time::{Duration, Instant};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 202, 216);
const WARN: egui::Color32 = egui::Color32::from_rgb(232, 176, 72);
const VIEWS: [&str; 13] = [
    "Beauty",
    "Normals",
    "Albedo",
    "Depth",
    "Direct light",
    "Indirect light",
    "Specular",
    "Convergence",
    "Bounces (heat)",
    "BVH cost (heat)",
    "Variance (heat)",
    "False colour",
    "Light path (LPE)",
];
const TONE_MAPS: [&str; 8] = [
    "Linear",
    "Reinhard",
    "Reinhard (white point)",
    "ACES filmic",
    "Uncharted 2",
    "High contrast",
    "AgX",
    "Khronos PBR Neutral",
];
/// Settings::integrator (track 15).
const INTEGRATORS: [&str; 3] = ["Path tracing", "Bidirectional (BDPT)", "Photon mapping"];
/// Battle mode's quadrant colours in the convergence graph.
const BATTLE_COLORS: [egui::Color32; 4] = [
    ACCENT,
    egui::Color32::from_rgb(236, 176, 76),
    egui::Color32::from_rgb(214, 104, 196),
    egui::Color32::from_rgb(130, 214, 110),
];
const PROJECTIONS: [&str; 5] = [
    "Perspective",
    "Orthographic",
    "360° panorama",
    "Little planet",
    "Tilt-shift",
];
const RESOLUTIONS: [(f32, &str); 5] = [
    (1., "Native (100%)"),
    (2. / 3., "Quality (67%)"),
    (0.58, "Balanced (58%)"),
    (0.5, "Performance (50%)"),
    (1. / 3., "Ultra performance (33%)"),
];
/// The anti-aliasing name of a temporal upscaler run at 100%.
fn aa_name(u: Upscaler) -> Option<&'static str> {
    match u {
        Upscaler::Dlss => Some("DLAA"),
        Upscaler::Fsr => Some("FSR Native AA"),
        Upscaler::Xess => Some("XeSS Native AA"),
        _ => None,
    }
}
/// Quality-mode entries as (scale, native AA, label). At 100% RR is always DLAA; without RR
/// there is Native (progressive) and, for a temporal upscaler, its anti-aliasing mode.
fn quality_modes(s: &Settings) -> Vec<(f32, bool, String)> {
    let mut modes = Vec::new();
    if s.rr {
        modes.push((1., false, "DLAA (100%)".to_string()));
    } else {
        modes.push((1., false, "Native (100%)".to_string()));
        if let Some(aa) = aa_name(s.upscaler) {
            modes.push((1., true, format!("{aa} (100%)")));
        }
    }
    for (scale, label) in &RESOLUTIONS[1..] {
        modes.push((*scale, false, label.to_string()));
    }
    modes
}
fn quality_label(s: &Settings) -> String {
    match quality_modes(s).into_iter().find(|(scale, aa, _)| {
        (s.scale - scale).abs() < 0.005 && (*scale < 1. || s.native_aa == *aa || s.rr)
    }) {
        Some((_, _, label)) => label,
        None => format!("Custom ({:.0}%)", s.scale * 100.),
    }
}
/// Preset groups for the scene picker, as index ranges into `PRESETS`.
const GROUPS: [(&str, std::ops::Range<usize>); 3] = [
    ("Cornell box", 0..9),
    ("Studios", 9..23),
    ("Build your own", 23..24),
];
/// Which server a model list belongs to.
fn ai_server(ai: &crate::textscene::AiConfig) -> String {
    format!("{:?} {:?} {}", ai.provider, ai.protocol, ai.base_url.trim())
}
/// What "Describe a scene" asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AiMode {
    New,
    Edit,
    Critique,
}
/// A running text-to-scene request (on its own thread).
pub struct AiJob {
    pub rx: std::sync::mpsc::Receiver<AiMsg>,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub started: std::time::Instant,
    pub status: String,
    pub mode: AiMode,
}
pub enum AiMsg {
    Progress(String),
    Done(Box<anyhow::Result<crate::textscene::Outcome>>),
}

/// Background thumbnail renders: results as they finish, total, done so far.
type ThumbnailJobs = (
    std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>,
    usize,
    usize,
);
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Scene,
    Render,
    Look,
    Export,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    None,
    Stage,
    Camera,
    MainLight,
    Light(usize),
    Object(usize),
}
struct Toast {
    text: String,
    error: bool,
    until: Instant,
    /// A saved file: the toast offers Show in folder.
    file: Option<std::path::PathBuf>,
}
pub struct UiState {
    pub tab: Tab,
    pub advanced: bool,
    /// Tab cycles: everything → top bar only → nothing (screenshot mode).
    pub panel_hidden: bool,
    pub selection: Selection,
    /// + Mesh imports stand Z-up files upright (scans, CAD exports).
    pub mesh_z_up: bool,
    pub add_shape: Shape,
    pub search: String,
    pub export_scale: u32,
    pub export_samples: u32,
    /// Video export dialog (video_ui.rs): its options, whether it is open, the progress preview.
    pub video: crate::video::Options,
    pub show_video: bool,
    pub video_preview: Option<egui::TextureHandle>,
    /// An export to start this frame (the dialog's video, or the high-resolution still).
    pub video_pending: Option<crate::video::Options>,
    /// The high-resolution still's format, and the JPEG quality of every JPEG save.
    pub export_format: crate::video::ImageFormat,
    pub jpeg_quality: u32,
    /// Scenes window: the scene files found on disk, filters, selection and the save form.
    pub show_scenes: bool,
    /// The Assets window (item 6, browser.rs).
    pub assets: crate::browser::Browser,
    /// Look tools (roadmap 5b).
    pub zebras: bool,
    pub show_scopes: bool,
    pub meter: bool,
    /// Path visualizer (track 14).
    pub paths: crate::paths::PathView,
    /// The light path expression fields as typed (`Settings::lpe` takes them once they compile).
    pub lpe_edit: Vec<String>,
    pub show_graph: bool,
    pub show_stats: bool,
    pub show_remote: bool,
    pub console: crate::console::Console,
    /// Quick saves and stills use "caustica" in the self-test and headless runs, whatever the
    /// saved template (their checks look for those files).
    pub fixed_names: bool,
    remote_qr: Option<egui::TextureHandle>,
    scope_images: Option<(egui::TextureHandle, egui::TextureHandle)>,
    /// Convergence graph: reference image (width, height, RGBA), points (samples, RMSE x1000).
    pub reference: Option<(u32, u32, Vec<f32>)>,
    pub reference_path: String,
    pub graph: Vec<(u32, f64)>,
    /// Battle mode's graph (track 15): per quadrant, (GPU ms spent, RMSE x1000).
    pub battle_graph: [Vec<(f64, f64)>; 4],
    /// Thumbnails: loaded textures by file, the background render (results, total, done) and
    /// whether this session already asked for the missing ones.
    pub thumbnails: std::collections::HashMap<std::path::PathBuf, egui::TextureHandle>,
    thumbnail_jobs: Option<ThumbnailJobs>,
    thumbnails_requested: bool,
    pub scenes: Vec<crate::scenes::Entry>,
    scene_search: String,
    scene_tag: Option<String>,
    /// Scenes window list: 0 all, 1 favourites, 2 recent (most recent first).
    scene_list: u8,
    /// Preferences (prefs.rs); written back only when `prefs_live` (interactive runs).
    pub prefs: crate::prefs::Prefs,
    pub prefs_live: bool,
    /// Menu → Renderer → Restart now: the app starts itself again and exits.
    pub restart: bool,
    /// The scene file last loaded or saved and the scene name it had: Ctrl+S overwrites it
    /// while the name is unchanged (never by title alone).
    pub loaded_file: Option<(std::path::PathBuf, String)>,
    pub scene_selected: Option<usize>,
    /// Generator section of the Scenes window: family index and seed.
    gen_family: usize,
    gen_seed: u64,
    save_title: String,
    save_description: String,
    toasts: Vec<Toast>,
    /// Transform gizmo: active tool, Global / Local axes, handle under the pointer, and the
    /// running transform (handle drag or modal G / R / S) with its header text.
    pub tool: gizmo::Tool,
    pub local: bool,
    pub hover: Option<gizmo::Handle>,
    /// The pointer is on the gizmo's inner part (`gizmo::inner`): with physics running, a
    /// press there grabs the body under it instead.
    pub hover_inner: bool,
    /// Multi-select (3b): the selected items besides `selection` (the active one), the pivot
    /// they turn and scale around, the 3D cursor, and what Ctrl snaps a move to.
    pub also: Vec<Selection>,
    pub pivot: gizmo::Pivot,
    pub cursor3d: glam::Vec3,
    pub snap_to: crate::snap::Target,
    /// Box select: B arms it for the next drag (the Select tool always drags a box); the
    /// corner where the drag started, in points.
    pub box_armed: bool,
    pub box_start: Option<egui::Pos2>,
    /// Object and light counts `also` belongs to; any other change drops it.
    also_counts: (usize, usize),
    /// The object under the pointer (from the GPU object-id probe), outlined faintly.
    pub hovered: Option<usize>,
    /// Status bar: when samples were last counted, how many, and samples per second.
    sample_rate: (Instant, u32, f32),
    /// The right-click menu's position in the viewport, while it is open.
    pub context_menu: Option<egui::Pos2>,
    /// The keyboard and mouse sheet (F1).
    pub show_shortcuts: bool,
    /// The timeline strip (track 8); it also shows whenever the scene has keys.
    pub show_timeline: bool,
    /// Track 17: physics poses are being keyed into the timeline, and the simulated time so far.
    pub physics_record: bool,
    /// A scene file to open after this frame's UI (the puzzle level menu).
    pub open_pending: Option<std::path::PathBuf>,
    pub physics_record_time: f32,
    /// Objects the running recording keys (cleared first, thinned out when it stops).
    pub physics_recorded: Vec<usize>,
    /// A command chosen in an outliner menu, run once the panel is drawn.
    pub pending: Option<Command>,
    pub transform: Option<gizmo::Transform>,
    pub header: String,
    /// Snapshots for Ctrl+Z / Ctrl+Y, taken before each edit that the UI makes.
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    /// Track 19 text-to-scene: the description, and the request running in the background.
    pub describe: String,
    /// A key typed in Menu → Text-to-scene model, for this session only.
    pub ai_key: String,
    pub describe_job: Option<AiJob>,
    /// New scene, edit this scene, or critique the render.
    pub describe_mode: AiMode,
    /// A critique waits for main.rs to save the current render (it owns the GPU).
    pub critique_capture: bool,
    /// The saved render, for `poll_describe` to start the critique with.
    pub critique_png: Option<Vec<u8>>,
    /// The model list fetched from the server, and the fetch in progress.
    pub ai_models: Vec<String>,
    /// With the server it asked (provider, protocol, URL): a list that comes back after a
    /// switch is dropped.
    pub ai_models_job: Option<(
        std::sync::mpsc::Receiver<anyhow::Result<Vec<String>>>,
        String,
    )>,
    /// The model's last notes (a critique, or text around a scene) and a critique's scene
    /// waiting for Apply.
    pub ai_notes: String,
    pub critique_scene: Option<Settings>,
}
impl Default for UiState {
    fn default() -> Self {
        Self {
            tab: Tab::Scene,
            advanced: false,
            panel_hidden: false,
            selection: Selection::Stage,
            mesh_z_up: false,
            add_shape: Shape::Sphere,
            search: String::new(),
            export_scale: 2,
            export_samples: 128,
            video: crate::video::Options {
                frames: 120,
                spp: 32,
                ..crate::video::Options::turntable(
                    &Settings::default(),
                    [1920, 1080],
                    120,
                    32,
                    crate::dirs::output("caustica-turntable.mp4"),
                )
            },
            show_video: false,
            video_preview: None,
            video_pending: None,
            export_format: crate::video::ImageFormat::Png,
            jpeg_quality: crate::video::JPEG_QUALITY,
            show_scenes: false,
            assets: Default::default(),
            zebras: false,
            show_scopes: false,
            meter: false,
            paths: Default::default(),
            lpe_edit: vec![],
            show_graph: false,
            show_stats: false,
            show_remote: false,
            console: Default::default(),
            fixed_names: false,
            remote_qr: None,
            scope_images: None,
            reference: None,
            reference_path: crate::dirs::captures()
                .join("reference.exr")
                .to_string_lossy()
                .into(),
            graph: Vec::new(),
            battle_graph: Default::default(),
            thumbnails: Default::default(),
            thumbnail_jobs: None,
            thumbnails_requested: false,
            scenes: crate::scenes::list(),
            scene_search: String::new(),
            scene_tag: None,
            scene_list: 0,
            prefs: Default::default(),
            prefs_live: false,
            restart: false,
            loaded_file: None,
            scene_selected: None,
            gen_family: 0,
            gen_seed: 1,
            save_title: String::new(),
            save_description: String::new(),
            toasts: Vec::new(),
            tool: gizmo::Tool::Move,
            local: false,
            hover: None,
            hover_inner: false,
            also: vec![],
            pivot: gizmo::Pivot::Median,
            cursor3d: glam::Vec3::new(0., -1., 0.),
            snap_to: crate::snap::Target::Increment,
            box_armed: false,
            box_start: None,
            also_counts: (0, 0),
            hovered: None,
            sample_rate: (Instant::now(), 0, 0.),
            context_menu: None,
            show_shortcuts: false,
            show_timeline: false,
            physics_record: false,
            open_pending: None,
            physics_record_time: 0.,
            physics_recorded: vec![],
            pending: None,
            transform: None,
            header: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            describe: String::new(),
            ai_key: String::new(),
            describe_job: None,
            describe_mode: AiMode::New,
            critique_capture: false,
            critique_png: None,
            ai_models: Vec::new(),
            ai_models_job: None,
            ai_notes: String::new(),
            critique_scene: None,
        }
    }
}
impl UiState {
    /// The file stem of quick saves and stills (`video::expand_name` of the saved template).
    pub fn export_stem(&self, s: &Settings, samples: u32, w: u32, h: u32) -> String {
        if self.fixed_names {
            return "caustica".into();
        }
        crate::video::expand_name(&self.prefs.name_template, &s.scene_name, samples, w, h)
    }
    pub fn toast(&mut self, text: impl Into<String>) {
        self.push(text.into(), false, None);
    }
    /// A toast about a saved file, with Show in folder.
    pub fn toast_file(&mut self, text: impl Into<String>, file: std::path::PathBuf) {
        self.push(text.into(), false, Some(file));
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), true, None);
    }
    fn push(&mut self, text: String, error: bool, file: Option<std::path::PathBuf>) {
        let life = if error || file.is_some() { 8. } else { 3. };
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast {
            text,
            error,
            until: Instant::now() + Duration::from_secs_f32(life),
            file,
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }
    /// Scenes window list: 0 all, 1 favourites, 2 recent.
    pub fn set_scene_list(&mut self, list: u8) {
        self.scene_list = list.min(2);
    }
    /// Writes the preferences file (interactive runs only).
    pub fn save_prefs(&self) {
        if self.prefs_live {
            self.prefs.save();
        }
    }
    /// Remembers `s` for undo, before an edit.
    pub fn checkpoint(&mut self, s: &Settings) {
        if self.undo.last() != Some(s) {
            self.undo.push(s.clone());
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    pub fn undo(&mut self, s: &mut Settings) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(s, previous));
        true
    }
    pub fn redo(&mut self, s: &mut Settings) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(s, next));
        true
    }
    /// Every selected item that can be transformed, the active one first.
    pub fn targets(&self) -> Vec<Selection> {
        let mut v = vec![];
        if transformable(self.selection) {
            v.push(self.selection);
        }
        for s in &self.also {
            if !v.contains(s) {
                v.push(*s);
            }
        }
        v
    }
    pub fn is_selected(&self, sel: Selection) -> bool {
        self.selection == sel || self.also.contains(&sel)
    }
    /// A plain click: only `sel`.
    pub fn select(&mut self, sel: Selection) {
        self.selection = sel;
        self.also.clear();
    }
    /// Shift+click: adds `sel` and makes it active, or removes it when already selected.
    pub fn toggle(&mut self, sel: Selection) {
        if self.selection == sel {
            self.selection = self.also.pop().unwrap_or(Selection::None);
        } else if let Some(k) = self.also.iter().position(|x| *x == sel) {
            self.also.remove(k);
        } else {
            if transformable(self.selection) {
                self.also.push(self.selection);
            }
            self.selection = sel;
        }
    }
    /// `toggle` with the counts recorded, so `tidy_selection` keeps what it added.
    pub fn toggle_in(&mut self, sel: Selection, s: &Settings) {
        self.also_counts = (s.objects.len(), s.lights.len());
        self.toggle(sel);
    }
    /// Selects `items`, keeping the active one when it is among them.
    pub fn select_many(&mut self, items: Vec<Selection>, s: &Settings) {
        self.also_counts = (s.objects.len(), s.lights.len());
        let active = if items.contains(&self.selection) {
            self.selection
        } else {
            items.first().copied().unwrap_or(Selection::None)
        };
        self.selection = active;
        self.also = items.into_iter().filter(|x| *x != active).collect();
    }
    /// Drops the extra selections when the active item is not an object or light, and after
    /// objects or lights were added or removed elsewhere (undo, a load): their indices moved.
    pub fn tidy_selection(&mut self, s: &Settings) {
        let counts = (s.objects.len(), s.lights.len());
        if counts != self.also_counts || !transformable(self.selection) {
            self.also.clear();
        }
        self.also_counts = counts;
        let active = self.selection;
        self.also.retain(|x| *x != active);
    }
    /// Duplicates the selected objects and area lights (next to the originals when `offset`,
    /// else in place, for Alt-drag) and selects the copies; returns a name for the toast.
    pub fn duplicate_selected(&mut self, s: &mut Settings, offset: bool) -> Option<String> {
        let copy_name = |n: &str| format!("{} copy", n.chars().take(110).collect::<String>());
        let targets = self.targets();
        let (objects, lights) = (s.objects.len(), s.lights.len());
        let copyable = |t: &&Selection| match **t {
            Selection::Object(i) => i < objects,
            Selection::Light(i) => i < lights,
            _ => false,
        };
        let targets: Vec<Selection> = targets.iter().filter(copyable).copied().collect();
        if targets.is_empty() {
            return None;
        }
        let snapshot = s.clone();
        let mut copies = vec![];
        let mut names = vec![];
        for t in &targets {
            match *t {
                Selection::Object(i) if s.objects.len() < scene::MAX_OBJECTS => {
                    let mut o = s.objects[i].clone();
                    o.name = copy_name(&o.name);
                    // A pair is two-way: a copy starts unpaired.
                    o.portal = -1;
                    if offset {
                        o.position[0] += 0.25;
                        o.position[1] += 0.15;
                    }
                    names.push(o.name.clone());
                    s.objects.push(o);
                    copies.push(Selection::Object(s.objects.len() - 1));
                }
                Selection::Light(i) if s.lights.len() < scene::MAX_LIGHTS => {
                    let mut l = s.lights[i].clone();
                    l.name = copy_name(&l.name);
                    if offset {
                        l.position[0] += 0.25;
                    }
                    names.push(l.name.clone());
                    s.lights.push(l);
                    copies.push(Selection::Light(s.lights.len() - 1));
                }
                _ => {}
            }
        }
        if copies.is_empty() {
            return None;
        }
        self.checkpoint(&snapshot);
        self.also_counts = (s.objects.len(), s.lights.len());
        self.selection = copies[0];
        self.also = copies[1..].to_vec();
        Some(if names.len() == 1 {
            names.remove(0)
        } else {
            format!("{} items", names.len())
        })
    }
    /// Hides or shows the selected objects and area lights (camera visibility; all follow the
    /// first); returns a toast.
    pub fn toggle_hidden_selected(&mut self, s: &mut Settings) -> Option<String> {
        let items: Vec<Selection> = self
            .targets()
            .into_iter()
            .filter(|t| match *t {
                Selection::Object(i) => i < s.objects.len(),
                Selection::Light(i) => i < s.lights.len(),
                _ => false,
            })
            .collect();
        let first = *items.first()?;
        self.checkpoint(s);
        let (name, visible) = match first {
            Selection::Object(i) => (s.objects[i].name.clone(), !s.objects[i].visible),
            Selection::Light(i) => (s.lights[i].name.clone(), !s.lights[i].visible),
            _ => unreachable!(),
        };
        for t in &items {
            match *t {
                Selection::Object(i) => s.objects[i].visible = visible,
                Selection::Light(i) => s.lights[i].visible = visible,
                _ => {}
            }
        }
        let name = if items.len() > 1 {
            format!("{} items", items.len())
        } else {
            name
        };
        Some(format!(
            "{} {name}",
            if visible { "Shown:" } else { "Hidden:" }
        ))
    }
    /// Shows every hidden object; returns how many.
    pub fn show_all(&mut self, s: &mut Settings) -> usize {
        let hidden = s.objects.iter().filter(|o| !o.visible).count();
        if hidden > 0 {
            self.checkpoint(s);
            for o in &mut s.objects {
                o.visible = true;
            }
        }
        hidden
    }
    /// Removes object `i`, fixing the indices that point at objects (animation tracks, the
    /// laser target, tethers).
    fn remove_object(s: &mut Settings, i: usize) -> String {
        let name = s.objects.remove(i).name;
        s.animation.removed("objects", i);
        // The laser puzzle's target and the strings' objects are indices (track 17).
        let t = &mut s.puzzle.target;
        if *t == i as i32 {
            *t = -1;
        } else if *t > i as i32 {
            *t -= 1;
        }
        let i32_i = i as i32;
        s.puzzle.targets.retain(|&t| t != i32_i);
        for t in &mut s.puzzle.targets {
            if *t > i32_i {
                *t -= 1;
            }
        }
        for o in &mut s.objects {
            if let Some(t) = &mut o.tether {
                if t.object as usize == i {
                    o.tether = None;
                } else if t.object as usize > i {
                    t.object -= 1;
                }
            }
            o.links.retain(|l| l.object as usize != i);
            for l in &mut o.links {
                if l.object as usize > i {
                    l.object -= 1;
                }
            }
        }
        name
    }
    /// Deletes the selected objects and area lights; returns a name for the toast.
    pub fn delete_selected(&mut self, s: &mut Settings) -> Option<String> {
        let mut objects: Vec<usize> = vec![];
        let mut lights: Vec<usize> = vec![];
        for t in self.targets() {
            match t {
                Selection::Object(i) if i < s.objects.len() => objects.push(i),
                Selection::Light(i) if i < s.lights.len() => lights.push(i),
                _ => {}
            }
        }
        if objects.is_empty() && lights.is_empty() {
            return None;
        }
        self.checkpoint(s);
        let single = objects.len() + lights.len() == 1;
        // Highest index first, so the lower ones stay valid.
        objects.sort_unstable_by(|a, b| b.cmp(a));
        lights.sort_unstable_by(|a, b| b.cmp(a));
        let mut names = vec![];
        for &i in &objects {
            names.push(Self::remove_object(s, i));
        }
        for &i in &lights {
            names.push(s.lights.remove(i).name);
            s.animation.removed("lights", i);
        }
        self.also.clear();
        self.also_counts = (s.objects.len(), s.lights.len());
        self.selection = if single && objects.len() == 1 && !s.objects.is_empty() {
            // One object: its neighbour becomes selected, so Delete can be held down.
            Selection::Object(objects[0].min(s.objects.len() - 1))
        } else if single && lights.len() == 1 {
            Selection::MainLight
        } else if s.objects.is_empty() {
            Selection::Stage
        } else {
            Selection::None
        };
        Some(if single {
            names.remove(0)
        } else {
            format!("{} items", names.len())
        })
    }
}
/// Items gizmos can move: objects, area lights and the main light.
pub fn transformable(s: Selection) -> bool {
    matches!(
        s,
        Selection::Object(_) | Selection::Light(_) | Selection::MainLight
    )
}

pub fn theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(24, 26, 30, 250);
    style.spacing.item_spacing = egui::vec2(8., 6.);
    style.spacing.interact_size.y = 22.;
    style.spacing.button_padding = egui::vec2(8., 3.);
    use egui::{FontFamily::*, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(16., Proportional)),
        (TextStyle::Body, FontId::new(14., Proportional)),
        (TextStyle::Button, FontId::new(14., Proportional)),
        (TextStyle::Small, FontId::new(12., Proportional)),
        (TextStyle::Monospace, FontId::new(13., Monospace)),
    ]
    .into();
    ctx.set_style(style);
}
/// A labelled row in a two-column property grid. The label carries the tooltip.
pub fn row(ui: &mut egui::Ui, label: &str, tip: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(label).on_hover_text(tip);
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = 130.;
        add(ui);
    });
    ui.end_row();
}
/// Advanced rows: shown inline with "Show advanced settings", otherwise under "More".
pub fn more(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    advanced: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    if advanced {
        add(ui);
    } else {
        egui::CollapsingHeader::new("More")
            .id_salt(id)
            .show(ui, add);
    }
}
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.);
    ui.label(egui::RichText::new(title).strong().color(ACCENT));
}
pub(crate) fn grid(id: &str) -> egui::Grid {
    egui::Grid::new(id).num_columns(2).spacing([12., 6.])
}
/// Light path expressions (track 14): one row per slot (show it, the expression, ready-made
/// ones), and the compile error while one does not parse. A field holds what was typed; the
/// settings take it once it compiles, and it shows the settings again when it loses focus.
fn lpe_section(ui: &mut egui::Ui, s: &mut Settings, edit: &mut Vec<String>) {
    use crate::lpe;
    note(
        ui,
        "Each expression gets its own image (the Light path view, raw samples). Events from the camera: D diffuse, G glossy, S specular, T transmission, V volume, then L light or B background; . any, [DG] [^S] sets, * + ? | ( ). A quoted name after an event limits it to that object or light: C D'red wall' L, C .* L'key' (also left wall, right wall, back wall, floor, main light, sun).",
    );
    edit.resize(lpe::SLOTS, String::new());
    let shown = s.view == 12;
    // The field takes what the radio and the Presets button leave.
    let width = (ui.available_width() - 120.).max(80.);
    for (k, text) in edit.iter_mut().enumerate() {
        let current = s.lpe.get(k).cloned().unwrap_or_default();
        let mut error = None;
        ui.horizontal(|ui| {
            if ui
                .radio(shown && s.lpe_slot == k as u32, format!("{}", k + 1))
                .on_hover_text("Show this expression's image")
                .clicked()
            {
                s.view = 12;
                s.lpe_slot = k as u32;
            }
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("e.g. C D L")
                    .desired_width(width),
            );
            let mut set = None;
            if field.changed() {
                set = Some(text.clone());
            }
            ui.menu_button("Presets", |ui| {
                for (name, expr) in lpe::PRESETS {
                    if ui.button(format!("{name}  ({expr})")).clicked() {
                        set = Some(expr.to_string());
                        ui.close_menu();
                    }
                }
                if ui.button("Clear").clicked() {
                    set = Some(String::new());
                    ui.close_menu();
                }
            });
            if let Some(t) = set {
                *text = t.clone();
                match lpe::check(&s.lpe, k, &t) {
                    Ok(_) => store(s, k, t),
                    Err(_) if t.trim().is_empty() => store(s, k, String::new()),
                    Err(_) => {}
                }
            } else if !field.has_focus() && *text != current {
                *text = current.clone();
            }
            if !text.trim().is_empty()
                && let Err(e) = lpe::check(&s.lpe, k, text)
            {
                error = Some(e);
            }
        });
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(255, 120, 100), e);
        }
    }
    /// Sets slot k, keeping the list no longer than its last expression.
    fn store(s: &mut Settings, k: usize, text: String) {
        if s.lpe.len() <= k {
            s.lpe.resize(k + 1, String::new());
        }
        s.lpe[k] = text;
        while s.lpe.last().is_some_and(|e| e.trim().is_empty()) {
            s.lpe.pop();
        }
    }
}
pub(crate) fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}
fn rr_model(id: u32) -> &'static str {
    match id {
        4 => "Transformer D",
        5 => "Transformer E",
        6 => "RR2 / F (recommended)",
        _ => "NVIDIA default",
    }
}
/// What turns samples into the shown image, in words; true when it is a fallback.
fn pipeline(app: &App) -> (String, bool) {
    let (g, s) = (app.gpu.as_ref().unwrap(), &app.settings);
    if s.battle {
        return ("Battle mode (raw samples)".into(), false);
    }
    if g.rr_active {
        return (
            format!(
                "DLSS Ray Reconstruction ({})",
                rr_model(s.rr_preset).trim_end_matches(" (recommended)")
            ),
            false,
        );
    }
    let mut text = if s.denoise {
        "Native denoiser"
    } else {
        "No denoiser (raw samples)"
    }
    .to_string();
    let mut fallback = false;
    if g.rw < g.width {
        if g.upscaler_usable(s.upscaler) {
            text += &format!(" + {}", s.upscaler.label());
        } else {
            // Failed mid-session or unavailable: bicubic stands in (amber, like RR's fallback).
            text += &format!(" + Bicubic ({} unavailable)", s.upscaler.label());
            fallback = true;
        }
    } else if s.native_aa
        && let Some(aa) = aa_name(s.upscaler)
    {
        text += &format!(" + {aa}");
    }
    if s.rr {
        return (format!("Ray Reconstruction unavailable · {text}"), true);
    }
    (text, fallback)
}
pub fn view_name(view: u32) -> &'static str {
    VIEWS[view as usize]
}
/// A scene card: thumbnail (or a placeholder) and the title under it, all cards the same size.
fn card(
    ui: &mut egui::Ui,
    e: &crate::scenes::Entry,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
) -> egui::Response {
    let size = egui::vec2(160., 100.);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(size.x, size.y + 18.), egui::Sense::click());
    let image = egui::Rect::from_min_size(rect.min, size);
    match texture {
        Some(t) => egui::Image::new((t.id(), size))
            .corner_radius(3.)
            .paint_at(ui, image),
        None => {
            ui.painter()
                .rect_filled(image, 3., egui::Color32::from_gray(38));
            ui.painter().text(
                image.center(),
                egui::Align2::CENTER_CENTER,
                e.title.chars().next().unwrap_or('?'),
                egui::FontId::proportional(40.),
                egui::Color32::from_gray(110),
            );
        }
    }
    if selected || r.hovered() {
        let color = if selected {
            ACCENT
        } else {
            egui::Color32::from_gray(150)
        };
        ui.painter().rect_stroke(
            image.expand(1.),
            4.,
            egui::Stroke::new(2_f32, color),
            egui::StrokeKind::Outside,
        );
    }
    let title: String = if e.title.chars().count() > 26 {
        e.title.chars().take(25).collect::<String>() + "…"
    } else {
        e.title.clone()
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 2., image.max.y + 3.),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(12.),
        if selected {
            ACCENT
        } else {
            ui.visuals().text_color()
        },
    );
    r.on_hover_text(&e.description)
}
/// A grey card with the scene's initial, until its thumbnail exists.
pub(crate) fn placeholder(ui: &mut egui::Ui, title: &str, size: egui::Vec2) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        3.,
        egui::Color32::from_gray(if r.hovered() { 52 } else { 38 }),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title.chars().next().unwrap_or('?'),
        egui::FontId::proportional(size.y * 0.4),
        egui::Color32::from_gray(110),
    );
    r
}
/// The pack's licence as a small label; amber when it does not allow commercial use.
fn license_badge(ui: &mut egui::Ui, l: &crate::scenes::License) {
    if l.name.is_empty() {
        return;
    }
    let (text, color) = if l.commercial_use {
        (l.name.clone(), ACCENT)
    } else {
        (format!("{} · non-commercial", l.name), WARN)
    };
    egui::Frame::new()
        .stroke(egui::Stroke::new(1_f32, color))
        .corner_radius(3.)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).small().color(color));
        })
        .response
        .on_hover_text(if l.commercial_use {
            "Licence of this pack"
        } else {
            "Licence of this pack: not for commercial use"
        });
}
/// Today's date as a seed (YYYYMMDD, UTC), so the same day gives the same scene.
pub fn seed_of_the_day() -> u32 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as u32;
    days.wrapping_mul(2_654_435_761) | 1
}
fn hdr_line(app: &App) -> String {
    let g = app.gpu.as_ref().unwrap();
    let info = app
        .monitor
        .as_ref()
        .map(|m| m.info.clone())
        .unwrap_or_default();
    let name = if info.name.is_empty() {
        "Display".to_string()
    } else {
        info.name.clone()
    };
    let windows = match (cfg!(windows), info.hdr) {
        (true, true) => "Windows HDR on",
        (true, false) => "Windows HDR off",
        (false, true) => "desktop HDR on",
        (false, false) => "desktop HDR off",
    };
    let mut line = if g.hdr_active {
        format!(
            "{name}: {windows} · HDR output ({}), {:.0} nits peak, paper white {:.0} nits",
            if g.hdr10 { "HDR10" } else { "scRGB" },
            g.hdr_levels[1],
            g.hdr_levels[0]
        )
    } else {
        format!("{name}: {windows} · SDR output")
    };
    if info.peak_nits > 0. {
        line += &format!(
            "\nPanel: {:.0} nits peak, {:.0} full frame, SDR white {:.0} nits",
            info.peak_nits, info.full_frame_nits, info.sdr_white_nits
        );
    }
    if !app.hdr_note.is_empty() {
        line += &format!("\n{}", app.hdr_note);
    }
    line
}

impl App {
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.poll_describe(ctx);
        // Double-click resets: the scene as it was when it got its name (loaded, preset,
        // generated or randomized), each object and light by index, and default UI values.
        if self.reset_baseline.scene_name != self.settings.scene_name {
            self.reset_baseline = self.settings.clone();
        }
        static OBJECT: std::sync::LazyLock<scene::Object> =
            std::sync::LazyLock::new(Default::default);
        static LIGHT: std::sync::LazyLock<scene::AreaLight> =
            std::sync::LazyLock::new(Default::default);
        crate::clear_reset_sources();
        crate::reset_source(&self.settings, &self.reset_baseline);
        for (i, o) in self.settings.objects.iter().enumerate() {
            crate::reset_source(o, self.reset_baseline.objects.get(i).unwrap_or(&OBJECT));
        }
        for (i, l) in self.settings.lights.iter().enumerate() {
            crate::reset_source(l, self.reset_baseline.lights.get(i).unwrap_or(&LIGHT));
        }
        crate::reset_source(&self.ui, &*self.ui_defaults);
        self.ui.tidy_selection(&self.settings);
        self.draw_ui(ctx);
        crate::clear_reset_sources();
    }
    fn draw_ui(&mut self, ctx: &egui::Context) {
        // Loaded and randomized settings carry their preset by name.
        let known = PRESETS.iter().position(|n| *n == self.settings.scene_name);
        if let Some(i) = known {
            self.preset = i;
        }
        self.top_bar(ctx, known.is_some());
        if !self.ui.panel_hidden {
            self.status_bar(ctx);
            self.timeline_panel(ctx);
            self.side_panel(ctx, known.is_some());
        }
        self.scenes_window(ctx);
        self.assets_window(ctx);
        self.video_window(ctx);
        self.tool_windows(ctx);
        self.overlay(ctx);
        self.context_menu(ctx);
        self.shortcuts_window(ctx);
        self.sync_prefs(ctx);
        self.toasts(ctx);
    }
    /// ◀ ▶, Page Up / Page Down: the scene files while one is loaded, else the presets.
    pub fn step_scene(&mut self, delta: isize) {
        match self.scene_entry() {
            Some(i) => {
                let n = self.ui.scenes.len() as isize;
                self.load_scene_file((i as isize + delta).rem_euclid(n) as usize);
            }
            None => {
                let n = PRESETS.len() as isize;
                self.preset = (self.preset as isize + delta).rem_euclid(n) as usize;
                self.settings.preset(self.preset);
            }
        }
    }
    /// Ctrl+O: picks a scene file and loads it (from the scenes list when it is in there).
    pub fn open_scene_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Scene", &["json"])
            .set_directory(crate::scenes::folder().unwrap_or_default())
            .pick_file()
        else {
            return;
        };
        self.open_scene(&path);
    }
    pub fn open_scene(&mut self, path: &std::path::Path) {
        self.ui.scenes = crate::scenes::list();
        let same =
            |p: &std::path::Path| std::fs::canonicalize(p).ok() == std::fs::canonicalize(path).ok();
        match self.ui.scenes.iter().position(|e| same(&e.path)) {
            Some(i) => self.load_scene_file(i),
            None => {
                self.ui.checkpoint(&self.settings);
                self.laser = crate::laser::Laser::default();
                match crate::scenes::load_into(&mut self.settings, path) {
                    Ok(()) => {
                        self.ui.selection = Selection::Stage;
                        self.ui.loaded_file =
                            Some((path.to_path_buf(), self.settings.scene_name.clone()));
                        self.ui.prefs.opened(path);
                        self.ui.save_prefs();
                        self.ui.toast(format!("Loaded {}", path.display()));
                    }
                    Err(e) => self.ui.error(format!("{e:#}")),
                }
            }
        }
    }
    /// Ctrl+S overwrites the loaded scene when it is a user scene (scenes/user), else asks
    /// where to save, as Ctrl+Shift+S (`as_new`) always does. The file list and thumbnails
    /// follow.
    pub fn save_scene(&mut self, as_new: bool) {
        // Only the file this scene came from, while it still has that name, and only a user
        // scene (scenes/user, not a generator file).
        let current = self
            .ui
            .loaded_file
            .clone()
            .filter(|(_, name)| *name == self.settings.scene_name)
            .and_then(|(path, _)| crate::scenes::read_entry(&path))
            .filter(|e| {
                e.tags.iter().any(|t| t == "user") && !e.tags.iter().any(|t| t == "generated")
            });
        let title = if self.settings.scene_name.trim().is_empty() {
            "My scene".to_string()
        } else {
            self.settings.scene_name.clone()
        };
        let (path, description) = match current {
            Some(e) if !as_new => (e.path, e.description),
            _ => {
                let dir = crate::dirs::user_scenes()
                    .unwrap_or_else(|| std::path::PathBuf::from("scenes"))
                    .join("user");
                let _ = std::fs::create_dir_all(&dir);
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Scene", &["json"])
                    .set_directory(&dir)
                    .set_file_name(format!("{}.json", crate::scenes::slug(&title)))
                    .save_file()
                else {
                    return;
                };
                (path, String::new())
            }
        };
        // A new file is titled after its name, so Save as… can rename the scene.
        let title = if as_new || self.ui.loaded_file.is_none() {
            path.file_stem().map_or(title.clone(), |s| {
                s.to_string_lossy().replace(['-', '_'], " ")
            })
        } else {
            title
        };
        match crate::scenes::save_to(&path, &self.settings, &title, &description) {
            Ok(()) => {
                self.settings.scene_name = title.clone();
                self.ui.loaded_file = Some((path.clone(), title));
                self.reset_baseline = self.settings.clone();
                self.ui.scenes = crate::scenes::list();
                self.ui.scene_selected = self.scene_entry();
                self.ui.prefs.opened(&path);
                self.ui.save_prefs();
                if !self.hidden {
                    self.request_thumbnails(false);
                }
                self.ui
                    .toast_file(format!("Saved {}", path.display()), path);
            }
            Err(e) => self.ui.error(format!("Save: {e:#}")),
        }
    }
    /// Keeps the preferences in step with the UI size and the advanced toggle, and shows the
    /// first-run hint.
    fn sync_prefs(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        let p = &mut self.ui.prefs;
        if (zoom - p.zoom).abs() > 1e-3 || self.ui.advanced != p.advanced {
            p.zoom = zoom;
            p.advanced = self.ui.advanced;
            self.ui.save_prefs();
        }
        if self.ui.prefs_live && !self.ui.prefs.hint_seen {
            let mut seen = false;
            egui::Area::new(egui::Id::new("first run hint"))
                .anchor(egui::Align2::CENTER_BOTTOM, [0., -48.])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Left drag orbits · right drag pans · wheel zooms · click selects · right-click opens a menu · F1 lists every shortcut");
                            seen = ui.button("Got it").clicked();
                        });
                    });
                });
            if seen {
                self.ui.prefs.hint_seen = true;
                self.ui.save_prefs();
            }
        }
    }
    /// Home: the camera as the scene was loaded.
    pub fn reset_camera(&mut self) {
        let b = &self.reset_baseline;
        let s = &mut self.settings;
        (s.yaw, s.pitch, s.distance, s.target, s.fov) =
            (b.yaw, b.pitch, b.distance, b.target, b.fov);
        self.ui.toast("Camera reset");
    }
    /// F: frames the selected object, or the whole scene when none is selected.
    pub fn frame_selection(&mut self) {
        let aspect = self
            .gpu
            .as_ref()
            .map_or(1.6, |g| g.width as f32 / g.height as f32);
        match self.ui.selection {
            Selection::Object(i) if i < self.settings.objects.len() => {
                self.settings.focus_object(i, aspect);
                self.ui
                    .toast(format!("Framed {}", self.settings.objects[i].name));
            }
            _ => {
                self.settings.frame_objects(aspect);
                self.ui.toast("Framed the scene");
            }
        }
    }
    /// Runs a context-menu or shortcut command on the selection.
    pub fn run_command(&mut self, c: Command) {
        let s = &mut self.settings;
        match c {
            Command::Duplicate => {
                if let Some(name) = self.ui.duplicate_selected(s, true) {
                    self.ui.toast(format!("Duplicated: {name}"));
                }
            }
            Command::Hide => {
                if let Some(text) = self.ui.toggle_hidden_selected(s) {
                    self.ui.toast(text);
                }
            }
            Command::ShowAll => {
                let n = self.ui.show_all(s);
                self.ui.toast(format!("Showed {n} hidden objects"));
            }
            Command::Delete => {
                if let Some(name) = self.ui.delete_selected(s) {
                    self.ui.toast(format!("Deleted {name}"));
                }
            }
            Command::Frame => self.frame_selection(),
            Command::FrameScene => {
                self.ui.selection = Selection::None;
                self.frame_selection();
            }
            Command::ResetCamera => self.reset_camera(),
            Command::Move | Command::Rotate | Command::Scale => {
                let mode = match c {
                    Command::Move => gizmo::Mode::Move,
                    Command::Rotate => gizmo::Mode::Rotate,
                    _ => gizmo::Mode::Scale,
                };
                if !self.start_transform(mode, None) {
                    self.ui.toast("The main light only moves and scales");
                }
            }
        }
    }
}
/// Viewport and outliner commands on the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Duplicate,
    Hide,
    ShowAll,
    Delete,
    Frame,
    FrameScene,
    ResetCamera,
    Move,
    Rotate,
    Scale,
}
/// The selection's entries of a context menu; returns the one clicked.
fn context_items(ui: &mut egui::Ui, selected: bool) -> Option<Command> {
    let mut chosen = None;
    let items = [
        ("Frame (F)", Command::Frame),
        ("Move (G)", Command::Move),
        ("Rotate (R)", Command::Rotate),
        ("Scale (S)", Command::Scale),
        ("Duplicate (Ctrl+D)", Command::Duplicate),
        ("Hide / show (H)", Command::Hide),
        ("Delete (Del)", Command::Delete),
    ];
    for (label, c) in items {
        if ui.add_enabled(selected, egui::Button::new(label)).clicked() {
            chosen = Some(c);
            ui.close_menu();
        }
    }
    chosen
}
/// Film stock looks (roadmap 5c): name, grain, grain size (0 fine, 1 medium, 2 coarse) and
/// halation. Matched by eye to the stocks' reputations (fine slow slide film, pushed 3200
/// speed, CineStill's red halos from the removed anti-halation layer); "None" clears them.
const FILM_STOCKS: &[(&str, f32, u32, f32)] = &[
    ("None", 0., 1, 0.),
    ("Fujichrome Velvia 50", 0.08, 0, 0.),
    ("Kodak Ektar 100", 0.12, 0, 0.05),
    ("Kodak Portra 400", 0.25, 0, 0.1),
    ("Kodak Vision3 500T", 0.3, 1, 0.2),
    ("CineStill 800T", 0.4, 1, 0.65),
    ("Kodak Portra 800 pushed", 0.55, 1, 0.12),
    ("Ilford Delta 3200 (grain only)", 0.85, 2, 0.),
];
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Camera",
        &[
            ("Left drag", "Orbit"),
            ("Right / middle drag", "Pan (or Shift+drag)"),
            ("Wheel", "Zoom"),
            ("F", "Frame the selection, or the scene"),
            ("Double-click", "Frame what is under the pointer"),
            ("Home", "Reset the camera to the scene's"),
            ("Ctrl+click", "Focus the lens there"),
            ("A", "Auto orbit"),
            ("`", "Walk mode (WASD, Esc to leave)"),
            (
                "Walking",
                "Space jump, C crouch, Shift run, F fly, B head bob, wheel body size",
            ),
        ],
    ),
    (
        "Selection",
        &[
            ("Click", "Select (empty space deselects)"),
            ("Shift+click", "Add to or remove from the selection"),
            ("B / Select tool drag", "Box select (Shift adds)"),
            ("Ctrl+A / Alt+A", "Select all / none"),
            ("Right-click", "Menu"),
            ("Shift+right-click", "Place the 3D cursor"),
            (
                "G / R / S",
                "Move / rotate / scale (then X Y Z, a value, Enter)",
            ),
            ("Ctrl+D", "Duplicate"),
            ("Shift+D / Alt+drag a handle", "Duplicate and move"),
            (
                "Ctrl while moving",
                "Snap (top bar: increment, surface, vertex, edge)",
            ),
            ("Tab while moving", "Next axis of a typed vector"),
            ("H / Alt+H", "Hide or show / show all"),
            ("Del", "Delete"),
            ("Esc", "Deselect"),
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
        ],
    ),
    (
        "Render and view",
        &[
            ("Space", "Pause"),
            ("R", "Restart (nothing selected)"),
            ("D", "Native denoiser"),
            ("1 – 8", "Debug views"),
            ("C", "Freeze an A/B reference"),
            ("L", "Light painting"),
            (
                "P / Alt+click",
                "Path visualizer / trace the paths of a pixel",
            ),
            ("F12", "Save caustica.png"),
            (
                "I / Shift+I / Alt+I",
                "Key the selection / key every edit / delete its keys",
            ),
            ("K", "Play or stop the timeline"),
            ("Shift+Left / Right", "One frame back / on"),
            ("Page Up / Page Down", "Previous / next scene"),
            ("Tab", "Cycle panels, top bar only, nothing"),
            ("F1", "This sheet"),
        ],
    ),
];

/// Opens Explorer with `file` selected (or the folder itself).
pub fn show_in_folder(file: &std::path::Path) {
    // A file that is not there (a cancelled still) shows its folder instead.
    let file = if file.exists() {
        file
    } else {
        file.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."))
    };
    let path = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    // canonicalize gives a \\?\ path, which Explorer does not take.
    let text = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    let mut c = std::process::Command::new("explorer");
    if path.is_dir() {
        c.arg(&text);
    } else {
        c.arg(format!("/select,{text}"));
    }
    let _ = c.spawn();
}

/// Quality bundles (UI plan 5.1): each writes `rr`, `scale`, `denoise`, `native_aa` and, for
/// the upscaled native one, `upscaler`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bundle {
    Best,
    Balanced,
    Fast,
    Fastest,
    Native,
    NativeFast,
    Reference,
}
impl Bundle {
    pub const ALL: [Bundle; 7] = [
        Bundle::Best,
        Bundle::Balanced,
        Bundle::Fast,
        Bundle::Fastest,
        Bundle::Native,
        Bundle::NativeFast,
        Bundle::Reference,
    ];
    pub fn rr(self) -> bool {
        matches!(
            self,
            Bundle::Best | Bundle::Balanced | Bundle::Fast | Bundle::Fastest
        )
    }
    fn scale(self) -> f32 {
        match self {
            Bundle::Balanced | Bundle::NativeFast => 2. / 3.,
            Bundle::Fast => 0.5,
            Bundle::Fastest => 1. / 3.,
            _ => 1.,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Bundle::Best => "Best",
            Bundle::Balanced => "Balanced",
            Bundle::Fast => "Fast",
            Bundle::Fastest => "Fastest",
            Bundle::Native => "Native denoiser",
            Bundle::NativeFast => "Native, upscaled",
            Bundle::Reference => "Reference",
        }
    }
    fn about(self, upscaler: Upscaler) -> String {
        match self {
            Bundle::Best => "DLSS RR, DLAA (100%)".into(),
            Bundle::Balanced => "DLSS RR, 67%".into(),
            Bundle::Fast => "DLSS RR, 50%".into(),
            Bundle::Fastest => "DLSS RR, 33%".into(),
            Bundle::Native => "native denoiser, 100%".into(),
            Bundle::NativeFast => format!("native denoiser + {}, 67%", upscaler.label()),
            Bundle::Reference => "no denoiser, 100% (unbiased)".into(),
        }
    }
    /// The bundle the settings match, if any.
    pub fn of(s: &Settings) -> Option<Bundle> {
        let near = |b: Bundle| (s.scale - b.scale()).abs() < 0.005;
        Bundle::ALL.into_iter().find(|b| {
            near(*b)
                && if b.rr() {
                    s.rr
                } else {
                    !s.rr
                        && !s.native_aa
                        && match b {
                            Bundle::Native => s.denoise,
                            // Any upscaler: without a temporal one it falls back to bicubic.
                            Bundle::NativeFast => s.denoise,
                            _ => !s.denoise,
                        }
                }
        })
    }
    pub fn apply(self, s: &mut Settings, upscaler: Upscaler) {
        s.rr = self.rr();
        s.scale = self.scale();
        s.native_aa = false;
        if !self.rr() {
            s.denoise = self != Bundle::Reference;
        }
        if self == Bundle::NativeFast {
            s.upscaler = upscaler;
        }
    }
}
