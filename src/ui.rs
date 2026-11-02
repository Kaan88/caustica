//! The interface (UI plan phase 1): top bar, a right side panel with Scene / Render / Look /
//! Export tabs, a status bar and toasts. UI-only state lives in `UiState`, never in `Settings`,
//! because every `Settings` field outside the exclusion list restarts the render.
use crate::scene::{self, DLSS_PRESETS, PRESETS, Settings, Shape, Upscaler};
use crate::{App, editor, gizmo};
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
                    "caustica-turntable.mp4".into(),
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
            remote_qr: None,
            scope_images: None,
            reference: None,
            reference_path: "captures/reference.exr".into(),
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
        for o in &mut s.objects {
            if let Some(t) = &mut o.tether {
                if t.object as usize == i {
                    o.tether = None;
                } else if t.object as usize > i {
                    t.object -= 1;
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
        "Each expression gets its own image (the Light path view, raw samples). Events from the camera: D diffuse, G glossy, S specular, T transmission, V volume, then L light or B background; . any, [DG] [^S] sets, * + ? | ( ).",
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
                match lpe::compile(&t) {
                    Ok(_) => store(s, k, t),
                    Err(_) if t.trim().is_empty() => store(s, k, String::new()),
                    Err(_) => {}
                }
            } else if !field.has_focus() && *text != current {
                *text = current.clone();
            }
            if !text.trim().is_empty()
                && let Err(e) = lpe::compile(text)
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
    if g.rw < g.width {
        text += &format!(" + {}", s.upscaler.label());
    } else if s.native_aa
        && let Some(aa) = aa_name(s.upscaler)
    {
        text += &format!(" + {aa}");
    }
    if s.rr {
        return (format!("Ray Reconstruction unavailable · {text}"), true);
    }
    (text, false)
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
    let windows = if info.hdr {
        "Windows HDR on"
    } else {
        "Windows HDR off"
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
                let dir = crate::scenes::folder()
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
    /// The viewport's right-click menu, kept open at its position until a click elsewhere.
    fn context_menu(&mut self, ctx: &egui::Context) {
        if let Some(c) = self.ui.pending.take() {
            self.run_command(c);
        }
        let Some(at) = self.ui.context_menu else {
            return;
        };
        let selected = matches!(
            self.ui.selection,
            Selection::Object(_) | Selection::Light(_)
        );
        let mut chosen = None;
        let r = egui::Area::new(egui::Id::new("viewport menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(170.);
                    let name = match self.ui.selection {
                        Selection::Object(i) => {
                            self.settings.objects.get(i).map(|o| o.name.clone())
                        }
                        Selection::Light(i) => self.settings.lights.get(i).map(|l| l.name.clone()),
                        _ => None,
                    };
                    if let Some(name) = name {
                        ui.label(egui::RichText::new(name).small().weak());
                    }
                    chosen = context_items(ui, selected);
                    ui.separator();
                    if ui.button("Frame the scene").clicked() {
                        chosen = Some(Command::FrameScene);
                    }
                    if ui.button("Reset the camera (Home)").clicked() {
                        chosen = Some(Command::ResetCamera);
                    }
                    if ui.button("Show all hidden (Alt+H)").clicked() {
                        chosen = Some(Command::ShowAll);
                    }
                });
            })
            .response;
        if let Some(c) = chosen {
            self.ui.context_menu = None;
            self.run_command(c);
        } else if ctx.input(|i| i.pointer.any_pressed()) && !r.contains_pointer()
            || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.ui.context_menu = None;
        }
    }
    /// F1: every shortcut in one place.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.show_shortcuts;
        egui::Window::new("Keyboard and mouse")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                // Three groups side by side, so the sheet fits a laptop screen.
                ui.horizontal_top(|ui| {
                    for (group, keys) in SHORTCUTS {
                        ui.vertical(|ui| {
                            section(ui, group);
                            grid(group).show(ui, |ui| {
                                for (k, what) in *keys {
                                    ui.label(egui::RichText::new(*k).strong());
                                    ui.label(*what);
                                    ui.end_row();
                                }
                            });
                        });
                        ui.add_space(12.);
                    }
                });
            });
        self.ui.show_shortcuts = open;
    }
    /// The loaded scene file's entry, if the current scene came from one.
    fn scene_entry(&self) -> Option<usize> {
        self.ui
            .scenes
            .iter()
            .position(|e| e.title == self.settings.scene_name)
    }
    pub fn load_scene_file(&mut self, index: usize) {
        let Some(e) = self.ui.scenes.get(index).cloned() else {
            return;
        };
        self.ui.checkpoint(&self.settings);
        self.laser = crate::laser::Laser::default();
        match crate::scenes::load_into(&mut self.settings, &e.path) {
            Ok(()) => {
                self.ui.selection = Selection::Stage;
                self.ui.scene_selected = Some(index);
                self.ui.loaded_file = Some((e.path.clone(), self.settings.scene_name.clone()));
                self.ui.prefs.opened(&e.path);
                self.ui.save_prefs();
                self.ui.toast(format!("Loaded {}", e.title));
            }
            Err(err) => self.ui.error(format!("{err:#}")),
        }
    }
    /// Starts rendering the thumbnails that are missing (or all of them).
    fn request_thumbnails(&mut self, all: bool) {
        let missing: Vec<_> = self
            .ui
            .scenes
            .iter()
            .filter(|e| all || e.thumbnail().is_some_and(|t| !t.is_file()))
            .cloned()
            .collect();
        self.ui.thumbnails_requested = true;
        if !missing.is_empty() {
            if all {
                self.ui.thumbnails.clear();
            }
            let total = missing.len();
            self.ui.thumbnail_jobs = Some((crate::scenes::render_thumbnails(missing), total, 0));
        }
    }
    /// Collects finished renders and loads thumbnail files into textures, a few per frame.
    fn update_thumbnails(&mut self, ctx: &egui::Context) {
        if let Some((jobs, total, done)) = &mut self.ui.thumbnail_jobs {
            while let Ok(result) = jobs.try_recv() {
                *done += 1;
                match result {
                    Ok(path) => {
                        self.ui.thumbnails.remove(&path);
                    }
                    Err(title) => self.ui.toasts.push(Toast {
                        text: format!("Thumbnail of {title} failed"),
                        error: true,
                        until: Instant::now() + Duration::from_secs(6),
                        file: None,
                    }),
                }
            }
            if *done >= *total {
                self.ui.thumbnail_jobs = None;
            }
        }
        let mut loaded = 0;
        for e in &self.ui.scenes {
            let Some(path) = e.thumbnail() else { continue };
            if loaded >= 4 || self.ui.thumbnails.contains_key(&path) || !path.is_file() {
                continue;
            }
            if let Ok(img) = image::open(&path) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let texture =
                    ctx.load_texture(path.to_string_lossy(), image, egui::TextureOptions::LINEAR);
                self.ui.thumbnails.insert(path, texture);
                loaded += 1;
            }
        }
        if loaded > 0 || self.ui.thumbnail_jobs.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
    fn scenes_window(&mut self, ctx: &egui::Context) {
        if !self.ui.show_scenes {
            return;
        }
        // Headless runs (captures, self-test) never spawn renders.
        if !self.ui.thumbnails_requested && !self.hidden {
            self.request_thumbnails(false);
        }
        self.update_thumbnails(ctx);
        let mut open = self.ui.show_scenes;
        let mut load = None;
        let mut rerender = false;
        egui::Window::new("Scenes")
            .open(&mut open)
            .default_size([760., 640.])
            .default_pos([120., 50.])
            .collapsible(false)
            .show(ctx, |ui| {
                let state = &mut self.ui;
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.scene_search).hint_text("Search scenes…").desired_width(220.));
                    if ui.button("Refresh").on_hover_text("Read the scenes folder again").clicked() {
                        state.scenes = crate::scenes::list();
                        state.scene_selected = None;
                    }
                    match &state.thumbnail_jobs {
                        Some((_, total, done)) => {
                            ui.add(egui::Spinner::new());
                            note(ui, &format!("Rendering thumbnails {done} / {total}"));
                        }
                        None => {
                            if ui.button("Re-render thumbnails").on_hover_text("Render every thumbnail again in the background").clicked() {
                                rerender = true;
                            }
                        }
                    }
                });
                let mut tags: Vec<String> = state.scenes.iter().flat_map(|e| e.tags.clone()).collect();
                tags.sort();
                tags.dedup();
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in ["All", "Favorites", "Recent"].iter().enumerate() {
                        if ui.selectable_label(state.scene_list == i as u8, *name).clicked() {
                            state.scene_list = i as u8;
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(state.scene_tag.is_none(), "all").clicked() {
                        state.scene_tag = None;
                    }
                    for t in tags {
                        let on = state.scene_tag.as_deref() == Some(t.as_str());
                        if ui.selectable_label(on, &t).clicked() {
                            state.scene_tag = if on { None } else { Some(t) };
                        }
                    }
                });
                ui.separator();
                let search = state.scene_search.to_lowercase();
                let mut visible: Vec<usize> = (0..state.scenes.len())
                    .filter(|i| {
                        let e = &state.scenes[*i];
                        (e.title.to_lowercase().contains(&search)
                            || e.description.to_lowercase().contains(&search)
                            || e.tags.iter().any(|t| t.contains(&search)))
                            && state.scene_tag.as_ref().is_none_or(|t| e.tags.contains(t))
                            && match state.scene_list {
                                1 => state.prefs.is_favorite(&e.path),
                                2 => state.prefs.recency(&e.path).is_some(),
                                _ => true,
                            }
                    })
                    .collect();
                if state.scene_list == 2 {
                    visible.sort_by_key(|i| state.prefs.recency(&state.scenes[*i].path));
                }
                if visible.is_empty() && state.scene_list > 0 {
                    note(ui, if state.scene_list == 1 { "No favourites yet: tick Favorite on a scene." } else { "No scenes opened yet." });
                }
                egui::ScrollArea::vertical().id_salt("scene grid").max_height(360.).auto_shrink([false, true]).show(ui, |ui| {
                    let mut start = 0;
                    while start < visible.len() {
                        let pack = state.scenes[visible[start]].pack.clone();
                        let end = visible[start..].iter().position(|i| state.scenes[*i].pack != pack).map_or(visible.len(), |n| start + n);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&pack).color(ACCENT));
                            license_badge(ui, &state.scenes[visible[start]].license);
                        });
                        ui.horizontal_wrapped(|ui| {
                            for &i in &visible[start..end] {
                                let e = &state.scenes[i];
                                let texture = e.thumbnail().and_then(|p| state.thumbnails.get(&p));
                                let r = card(ui, e, texture, state.scene_selected == Some(i));
                                if r.clicked() {
                                    state.scene_selected = Some(i);
                                }
                                if r.double_clicked() {
                                    load = Some(i);
                                }
                            }
                        });
                        start = end;
                    }
                });
                ui.separator();
                if let Some(e) = state.scene_selected.and_then(|i| state.scenes.get(i).cloned()) {
                    ui.horizontal(|ui| {
                        let texture = e.thumbnail().and_then(|p| state.thumbnails.get(&p));
                        match texture {
                            Some(t) => {
                                ui.add(egui::Image::new((t.id(), egui::vec2(256., 160.))).corner_radius(4.));
                            }
                            None => {
                                placeholder(ui, &e.title, egui::vec2(256., 160.));
                            }
                        }
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&e.title).heading());
                            ui.label(&e.description);
                            ui.horizontal_wrapped(|ui| {
                                license_badge(ui, &e.license);
                                for t in &e.tags {
                                    ui.label(egui::RichText::new(t).small().weak());
                                }
                            });
                            if !e.license.attribution.is_empty() {
                                note(ui, &format!("Attribution: {}", e.license.attribution));
                            }
                            if !e.credits.is_empty() && e.credits != e.license.attribution {
                                note(ui, &format!("Credits: {}", e.credits));
                            }
                            if !e.license.source.is_empty() {
                                ui.hyperlink_to(egui::RichText::new(&e.license.source).small(), &e.license.source);
                            }
                            note(ui, &e.path.display().to_string());
                            ui.horizontal(|ui| {
                                if ui.button("Load (or double-click)").clicked() {
                                    load = state.scene_selected;
                                }
                                let mut favorite = state.prefs.is_favorite(&e.path);
                                if ui.checkbox(&mut favorite, "Favorite").changed() {
                                    state.prefs.toggle_favorite(&e.path);
                                    state.save_prefs();
                                }
                            });
                        });
                    });
                } else {
                    note(ui, "Pick a scene; double-click loads it. Your render settings (resolution, RR, upscaler, HDR) stay as they are.");
                }
                ui.separator();
                ui.label(egui::RichText::new("Generate").strong());
                let mut generate = false;
                ui.horizontal(|ui| {
                    let families = crate::generators::FAMILIES;
                    egui::ComboBox::from_id_salt("generator family")
                        .selected_text(families[state.gen_family].1)
                        .width(170.)
                        .show_ui(ui, |ui| {
                            for (i, (_, label, about)) in families.iter().enumerate() {
                                ui.selectable_value(&mut state.gen_family, i, *label).on_hover_text(*about);
                            }
                        });
                    ui.label("seed");
                    ui.add(egui::DragValue::new(&mut state.gen_seed).speed(1.));
                    if ui.button("Generate").clicked() {
                        generate = true;
                    }
                    if ui.button("Next seed").clicked() {
                        state.gen_seed = state.gen_seed.wrapping_add(1);
                        generate = true;
                    }
                    if ui.button("Random").clicked() {
                        state.gen_seed = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_nanos() as u64 % 100_000)
                            .unwrap_or(7);
                        generate = true;
                    }
                });
                note(ui, crate::generators::FAMILIES[state.gen_family].2);
                if generate {
                    let family = crate::generators::FAMILIES[state.gen_family].0;
                    match crate::generators::generate(family, state.gen_seed) {
                        Ok(mut g) => {
                            state.checkpoint(&self.settings);
                            g.keep_output(&self.settings);
                            self.settings = g;
                            state.selection = Selection::Stage;
                        }
                        Err(e) => state.error(format!("{e:#}")),
                    }
                }
                if ui.button("Save this generator and seed to scenes/user").clicked() {
                    let family = crate::generators::FAMILIES[state.gen_family].0;
                    let title = format!("{} {}", crate::generators::FAMILIES[state.gen_family].1, state.gen_seed);
                    match crate::scenes::save_generated(family, state.gen_seed, &title) {
                        Ok(path) => {
                            state.scenes = crate::scenes::list();
                            state.toast(format!("Saved {}", path.display()));
                        }
                        Err(e) => state.error(format!("{e:#}")),
                    }
                }
                ui.separator();
                ui.label(egui::RichText::new("Save the current scene").strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.save_title).hint_text("Title").desired_width(180.));
                    ui.add(egui::TextEdit::singleline(&mut state.save_description).hint_text("Description").desired_width(260.));
                    if ui.button("Save to scenes/user").clicked() {
                        match crate::scenes::save_user(&self.settings, &state.save_title, &state.save_description) {
                            Ok(path) => {
                                state.scenes = crate::scenes::list();
                                state.thumbnails_requested = false;
                                self.settings.scene_name = state.save_title.trim().to_string();
                                state.toast(format!("Saved {}", path.display()));
                            }
                            Err(e) => state.error(format!("{e:#}")),
                        }
                    }
                });
            });
        self.ui.show_scenes = open;
        if rerender {
            self.request_thumbnails(true);
        }
        if let Some(i) = load {
            self.load_scene_file(i);
        }
    }
    /// L: light painting on or off. Turning it on restarts once in the progressive mode.
    pub fn toggle_painting(&mut self) {
        self.painting = !self.painting;
        self.actions.reset = true;
        self.ui.toast(if self.painting {
            "Light painting: move lights and they leave trails (L to stop)"
        } else {
            "Light painting off"
        });
    }
    /// `: walk mode on or off.
    pub fn toggle_walk(&mut self) {
        if self.walking.take().is_some() {
            self.keys_down.clear();
            self.ui.toast("Walk mode off");
        } else {
            self.walking = Some(crate::walk::Walk::enter(&mut self.settings));
            self.ui.toast(
                "Walk mode: WASD, Space jump, C crouch, Shift run, F fly, wheel body size, drag to look, Esc to stop",
            );
        }
    }
    pub fn tool_mode(&self) -> gizmo::Mode {
        match self.ui.tool {
            gizmo::Tool::Rotate => gizmo::Mode::Rotate,
            gizmo::Tool::Scale => gizmo::Mode::Scale,
            _ => gizmo::Mode::Move,
        }
    }
    fn tool_windows(&mut self, ctx: &egui::Context) {
        if self.ui.show_scopes {
            self.scopes_window(ctx);
        }
        if self.ui.show_graph {
            self.graph_window(ctx);
        }
        if self.ui.show_stats {
            self.stats_window(ctx);
        }
        if self.ui.show_remote {
            self.remote_window(ctx);
        }
        if self.ui.paths.open
            && crate::paths::window(ctx, &mut self.ui.paths, self.settings.perspective())
        {
            self.ui.paths.again();
        }
    }
    fn scopes_window(&mut self, ctx: &egui::Context) {
        let g = self.gpu.as_ref().unwrap();
        let [w, h] = crate::gpu::SCOPES.map(|v| v as usize);
        if g.scopes.len() == w * h {
            // Waveform (256 x 128) and vectorscope (128 x 128) as images, log-scaled counts.
            let image = |x0: usize, y0: usize, cw: usize, ch: usize, tint: [f32; 3]| {
                let rows = &g.scopes;
                let peak = (y0..y0 + ch)
                    .flat_map(|y| (x0..x0 + cw).map(move |x| rows[y * w + x]))
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let pixels: Vec<egui::Color32> = (y0..y0 + ch)
                    .flat_map(|y| (x0..x0 + cw).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let v = ((1. + rows[y * w + x] as f32).ln() / (1. + peak as f32).ln())
                            .clamp(0., 1.);
                        egui::Color32::from_rgb(
                            (v * 255. * tint[0]) as u8,
                            (v * 255. * tint[1]) as u8,
                            (v * 255. * tint[2]) as u8,
                        )
                    })
                    .collect();
                egui::ColorImage {
                    size: [cw, ch],
                    pixels,
                }
            };
            let wave = image(0, 4, 256, 128, [0.75, 1., 0.8]);
            let vector = image(0, 132, 128, 128, [0.85, 0.95, 1.]);
            match &mut self.ui.scope_images {
                Some((a, b)) => {
                    a.set(wave, egui::TextureOptions::LINEAR);
                    b.set(vector, egui::TextureOptions::LINEAR);
                }
                None => {
                    self.ui.scope_images = Some((
                        ctx.load_texture("waveform", wave, egui::TextureOptions::LINEAR),
                        ctx.load_texture("vectorscope", vector, egui::TextureOptions::LINEAR),
                    ))
                }
            }
        }
        let mut open = self.ui.show_scopes;
        egui::Window::new("Scopes")
            .open(&mut open)
            .default_pos([140., 420.])
            .resizable(false)
            .show(ctx, |ui| {
                let g = self.gpu.as_ref().unwrap();
                if g.scopes.len() != w * h {
                    note(ui, "Waiting for the next frame…");
                    return;
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Histogram").small());
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(256., 128.), egui::Sense::hover());
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 2., egui::Color32::from_gray(16));
                        let colors = [
                            egui::Color32::from_rgb(230, 70, 70),
                            egui::Color32::from_rgb(80, 210, 90),
                            egui::Color32::from_rgb(80, 130, 245),
                            egui::Color32::from_gray(220),
                        ];
                        for (row, color) in colors.iter().enumerate() {
                            let bins = &g.scopes[row * w..row * w + 256];
                            let peak = bins.iter().max().copied().unwrap_or(1).max(1) as f32;
                            let line: Vec<egui::Pos2> = bins
                                .iter()
                                .enumerate()
                                .map(|(i, v)| {
                                    egui::pos2(
                                        rect.min.x + i as f32,
                                        rect.max.y
                                            - (1. + *v as f32).ln() / (1. + peak).ln()
                                                * rect.height(),
                                    )
                                })
                                .collect();
                            painter.add(egui::Shape::line(
                                line,
                                egui::Stroke::new(1_f32, color.gamma_multiply(0.85)),
                            ));
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Luma waveform").small());
                        if let Some((wave, _)) = &self.ui.scope_images {
                            ui.add(egui::Image::new((wave.id(), egui::vec2(256., 128.))));
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Vectorscope").small());
                        if let Some((_, vector)) = &self.ui.scope_images {
                            let r = ui.add(egui::Image::new((vector.id(), egui::vec2(128., 128.))));
                            let c = r.rect.center();
                            ui.painter().circle_stroke(
                                c,
                                64.,
                                egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(40)),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(c.x - 64., c.y), egui::pos2(c.x + 64., c.y)],
                                egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(30)),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(c.x, c.y - 64.), egui::pos2(c.x, c.y + 64.)],
                                egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(30)),
                            );
                        }
                    });
                });
                note(
                    ui,
                    "Of the displayed SDR image (after exposure and tone map).",
                );
            });
        self.ui.show_scopes = open;
    }
    fn graph_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.show_graph;
        let mut use_current = false;
        let mut load = false;
        egui::Window::new("Convergence").open(&mut open).default_pos([140., 120.]).default_width(420.).show(ctx, |ui| {
            let g = self.gpu.as_ref().unwrap();
            match &self.ui.reference {
                Some((w, h, _)) => note(ui, &format!("Reference: {w} × {h}. RMSE ×1000 of l/(1+l), measured as samples grow (log–log).")),
                None => note(ui, "No reference yet: load a linear EXR of the same size, or use the current image once it has converged."),
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ui.reference_path).desired_width(220.));
                if ui.button("Browse…").clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("Linear EXR", &["exr"]).set_directory(std::env::current_dir().unwrap_or_default().join("captures")).pick_file()
                {
                    self.ui.reference_path = p.to_string_lossy().into();
                    load = true;
                }
                load |= ui.button("Load EXR").clicked();
                use_current = ui.button("Use current image").on_hover_text(format!("{} spp now", g.samples)).clicked();
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(300.), 200.), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2., egui::Color32::from_gray(16));
            let points = &self.ui.graph;
            if self.settings.battle {
                // Battle mode: one line per quadrant, error against GPU time (log–log).
                let lines: Vec<Vec<(f64, f64)>> = self.ui.battle_graph.iter().map(|l| l.iter().map(|(t, e)| (t.max(1e-3).log2(), e.max(1e-3).log2())).collect()).collect();
                let all: Vec<&(f64, f64)> = lines.iter().flatten().collect();
                if all.len() >= 2 {
                    let x0 = all.iter().map(|p| p.0).fold(f64::MAX, f64::min);
                    let x1 = all.iter().map(|p| p.0).fold(f64::MIN, f64::max).max(x0 + 1.);
                    let y0 = all.iter().map(|p| p.1).fold(f64::MAX, f64::min) - 0.2;
                    let y1 = all.iter().map(|p| p.1).fold(f64::MIN, f64::max) + 0.2;
                    let to = |x: f64, y: f64| egui::pos2(rect.min.x + ((x - x0) / (x1 - x0)) as f32 * rect.width(), rect.max.y - ((y - y0) / (y1 - y0)) as f32 * rect.height());
                    for (i, l) in lines.iter().enumerate() {
                        if l.len() >= 2 {
                            painter.add(egui::Shape::line(l.iter().map(|(x, y)| to(*x, *y)).collect(), egui::Stroke::new(2_f32, BATTLE_COLORS[i])));
                        }
                        let last = self.ui.battle_graph[i].last().map_or(String::new(), |(_, e)| format!(" · {e:.2}"));
                        painter.text(rect.left_top() + egui::vec2(6., 6. + 15. * i as f32), egui::Align2::LEFT_TOP, format!("{}{last}", crate::gpu::BATTLE_NAMES[i]), egui::FontId::proportional(12.), BATTLE_COLORS[i]);
                    }
                    painter.text(rect.left_bottom() + egui::vec2(6., -6.), egui::Align2::LEFT_BOTTOM, "x: GPU time per quadrant", egui::FontId::proportional(11.), egui::Color32::from_gray(140));
                } else {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Points appear as the quadrants converge", egui::FontId::proportional(13.), egui::Color32::from_gray(140));
                }
            } else if points.len() >= 2 {

                let (xs, ys): (Vec<f64>, Vec<f64>) = points.iter().map(|(s, e)| ((*s as f64).max(1.).log2(), e.max(1e-3).log2())).unzip();
                let (x0, x1) = (xs[0], xs.last().copied().unwrap().max(xs[0] + 1.));
                let (y0, y1) = (ys.iter().copied().fold(f64::MAX, f64::min) - 0.2, ys.iter().copied().fold(f64::MIN, f64::max) + 0.2);
                let to = |x: f64, y: f64| egui::pos2(rect.min.x + ((x - x0) / (x1 - x0)) as f32 * rect.width(), rect.max.y - ((y - y0) / (y1 - y0)) as f32 * rect.height());
                // The ideal Monte Carlo slope: error halves for every 4x samples.
                painter.line_segment([to(x0, ys[0]), to(x1, ys[0] - 0.5 * (x1 - x0))], egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(50)));
                let line: Vec<_> = xs.iter().zip(&ys).map(|(x, y)| to(*x, *y)).collect();
                painter.add(egui::Shape::line(line, egui::Stroke::new(2_f32, ACCENT)));
                let (s, e) = points.last().unwrap();
                painter.text(rect.right_top() + egui::vec2(-6., 6.), egui::Align2::RIGHT_TOP, format!("{s} spp · {e:.2}"), egui::FontId::proportional(13.), egui::Color32::WHITE);
                painter.text(rect.left_bottom() + egui::vec2(6., -6.), egui::Align2::LEFT_BOTTOM, "grey: 1/√n", egui::FontId::proportional(11.), egui::Color32::from_gray(140));
            } else {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Points appear as samples accumulate", egui::FontId::proportional(13.), egui::Color32::from_gray(140));
            }
        });
        self.ui.show_graph = open;
        let g = self.gpu.as_mut().unwrap();
        if use_current {
            match unsafe { g.read_floats(18) } {
                Ok(pixels) => {
                    self.ui.reference = Some((g.width, g.height, pixels));
                    self.ui.graph.clear();
                    self.actions.reset = true;
                    self.ui
                        .toast("Reference set; the render restarts to measure against it");
                }
                Err(e) => self.ui.error(format!("{e:#}")),
            }
        }
        if load {
            match image::open(&self.ui.reference_path) {
                Ok(img) => {
                    let img = img.to_rgba32f();
                    self.ui.reference = Some((img.width(), img.height(), img.into_raw()));
                    self.ui.graph.clear();
                    self.actions.reset = true;
                    self.ui.toast("Reference loaded");
                }
                Err(e) => self
                    .ui
                    .error(format!("Loading {}: {e}", self.ui.reference_path)),
            }
        }
    }
    fn remote_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.show_remote;
        let mut toggle = None;
        egui::Window::new("Phone remote").open(&mut open).default_pos([240., 120.]).resizable(false).show(ctx, |ui| {
            let mut on = self.remote.is_some();
            if ui.checkbox(&mut on, "Serve the remote on the local network").changed() {
                toggle = Some(on);
            }
            match &self.remote {
                Some(r) => {
                    note(ui, "Open this on a phone on the same network (Windows may ask to allow the app through the firewall):");
                    ui.hyperlink_to(&r.url, &r.url);
                    if self.ui.remote_qr.is_none()
                        && let Some(image) = crate::remote::qr_image(&r.url)
                    {
                        self.ui.remote_qr = Some(ctx.load_texture("remote qr", image, egui::TextureOptions::NEAREST));
                    }
                    if let Some(qr) = &self.ui.remote_qr {
                        let size = qr.size_vec2().min(egui::vec2(220., 220.));
                        ui.add(egui::Image::new((qr.id(), size)));
                    }
                    note(ui, &format!("Token {}: a new one every time the remote starts.", r.token));
                }
                None => note(ui, "Off. When on, a phone browser gets sliders for exposure, light, orbit, zoom and aperture, and buttons to switch scenes."),
            }
        });
        self.ui.show_remote = open;
        match toggle {
            Some(true) => match crate::remote::start(true) {
                Ok(r) => {
                    self.ui.remote_qr = None;
                    self.remote = Some(r);
                }
                Err(e) => self.ui.error(format!("Remote: {e}")),
            },
            Some(false) => {
                self.remote = None;
                self.ui.remote_qr = None;
            }
            None => {}
        }
    }
    /// Applies phone-remote edits and gamepad input for this frame.
    pub fn remote_and_gamepad(&mut self, dt: f32) {
        let edits: Vec<_> = self
            .remote
            .as_ref()
            .map(|r| r.edits.try_iter().collect())
            .unwrap_or_default();
        for (key, value) in edits {
            match key.as_str() {
                "preset" => {
                    let n = PRESETS.len() as i64;
                    self.preset =
                        ((self.preset as i64 + value.as_i64().unwrap_or(1)).rem_euclid(n)) as usize;
                    self.settings.preset(self.preset);
                }
                "restart" => self.actions.reset = true,
                _ => {
                    let mut j = serde_json::to_value(&self.settings).unwrap_or_default();
                    j[key.as_str()] = value;
                    match serde_json::from_value::<Settings>(j)
                        .map_err(anyhow::Error::from)
                        .and_then(|mut s| s.validate().map(|_| s))
                    {
                        Ok(s) => self.settings = s,
                        Err(e) => self.ui.error(format!("Remote edit {key} rejected: {e:#}")),
                    }
                }
            }
        }
        if let Some(r) = &self.remote
            && self.remote_synced.elapsed() > Duration::from_millis(500)
        {
            self.remote_synced = Instant::now();
            let j = serde_json::to_value(&self.settings).unwrap_or_default();
            let state: serde_json::Map<_, _> = crate::remote::KEYS
                .iter()
                .map(|k| (k.to_string(), j[*k].clone()))
                .collect();
            if let Ok(mut s) = r.state.lock() {
                *s = serde_json::Value::Object(state);
            }
        }
        let Some(pad) = &mut self.gamepad else { return };
        use gilrs::{Axis, Button, EventType};
        while let Some(ev) = pad.next_event() {
            if let EventType::ButtonPressed(button, _) = ev.event {
                match button {
                    Button::South => self.paused = !self.paused,
                    Button::East => self.actions.reset = true,
                    Button::LeftTrigger | Button::RightTrigger => {
                        let n = PRESETS.len();
                        self.preset = (self.preset
                            + if button == Button::RightTrigger {
                                1
                            } else {
                                n - 1
                            })
                            % n;
                        self.settings.preset(self.preset);
                    }
                    Button::DPadUp => {
                        self.settings.exposure = (self.settings.exposure * 1.25).min(10.)
                    }
                    Button::DPadDown => self.settings.exposure /= 1.25,
                    Button::Start => self.ui.panel_hidden = !self.ui.panel_hidden,
                    _ => {}
                }
            }
        }
        let Some((_, g)) = pad.gamepads().find(|(_, g)| g.is_connected()) else {
            return;
        };
        let dead = |v: f32| if v.abs() < 0.15 { 0. } else { v };
        let (lx, ly, ry) = (
            dead(g.value(Axis::LeftStickX)),
            dead(g.value(Axis::LeftStickY)),
            dead(g.value(Axis::RightStickY)),
        );
        let trigger = |b: Button| g.button_data(b).map_or(0., |d| d.value());
        let light = trigger(Button::RightTrigger2) - trigger(Button::LeftTrigger2);
        let s = &mut self.settings;
        if lx != 0. || ly != 0. {
            s.yaw += lx * 90. * dt;
            s.pitch = (s.pitch - ly * 60. * dt).clamp(-85., 85.);
        }
        if ry != 0. {
            s.distance = (s.distance * (-ry * 1.5 * dt).exp()).clamp(0.5, 200.);
        }
        if light.abs() > 0.05 {
            s.light_intensity = (s.light_intensity * (light * 1.5 * dt).exp()).clamp(0.01, 2000.);
        }
    }
    fn stats_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.show_stats;
        egui::Window::new("Statistics")
            .open(&mut open)
            .default_pos([200., 120.])
            .default_width(380.)
            .show(ctx, |ui| {
                let Some(s) = &self.stats else {
                    note(ui, "Statistics are kept for interactive runs only.");
                    return;
                };
                grid("stats").show(ui, |ui| {
                    for (label, value) in [
                        ("Camera paths traced", format!("{:.3e}", s.paths)),
                        ("GPU time", format!("{:.2} hours", s.gpu_seconds / 3600.)),
                        ("Images saved", s.images_saved.to_string()),
                        ("Highest sample count", s.max_samples.to_string()),
                        (
                            "Presets visited",
                            format!("{} / {}", s.presets_seen.len(), PRESETS.len()),
                        ),
                        ("Scene files opened", s.scenes_seen.len().to_string()),
                        ("Shader hot reloads", s.hot_reloads.to_string()),
                    ] {
                        ui.label(label);
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
                section(ui, "Achievements");
                for (id, title, how) in crate::stats::ACHIEVEMENTS {
                    ui.horizontal(|ui| match s.achievements.get(id) {
                        Some(date) => {
                            ui.label(egui::RichText::new(title).color(ACCENT).strong());
                            note(ui, &format!("{how} · {date}"));
                        }
                        None => {
                            ui.label(egui::RichText::new(title).weak());
                            note(ui, how);
                        }
                    });
                }
            });
        self.ui.show_stats = open;
    }
    /// Gizmo, light outlines and the transform header, drawn under the panels.
    fn overlay(&mut self, ctx: &egui::Context) {
        let g = self.gpu.as_ref().unwrap();
        if self.ui.meter
            && let (Some(crate::gpu::Probe { linear, albedo, .. }), Some(p)) =
                (g.probe_result, ctx.pointer_hover_pos())
        {
            // 1.0 in scene light is 80 nits, as in the HDR output. Illuminance assumes a
            // Lambertian surface: E = π L / albedo.
            let l = 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
            let a = 0.2126 * albedo[0] + 0.7152 * albedo[1] + 0.0722 * albedo[2];
            let nits = l * 80.;
            let text = if a > 0.02 {
                format!(
                    "{nits:.1} nits · {:.0} lux",
                    std::f32::consts::PI * nits / a
                )
            } else {
                format!("{nits:.1} nits")
            };
            ctx.debug_painter().text(
                p + egui::vec2(16., 16.),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(13.),
                egui::Color32::from_rgb(255, 230, 140),
            );
        }
        let view = gizmo::View::new(&self.settings, [g.width, g.height]);
        let ppp = ctx.pixels_per_point();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("gizmo"),
        ));
        let targets = self.ui.targets();
        gizmo::draw_lights(&painter, ppp, &view, &self.settings, &targets);
        if self.settings.puzzle.enabled && self.settings.perspective() {
            self.laser
                .draw(&painter, ppp, &view, self.settings.puzzle.color);
        }
        if self.ui.paths.open && self.settings.perspective() {
            crate::paths::draw(
                &painter,
                ppp,
                &view,
                self.settings.distance.max(1.),
                &self.ui.paths,
            );
        }
        // A/B compare: the split line can be dragged (a 12-point strip over the line).
        if self.settings.compare {
            let screen = ctx.screen_rect();
            let x = screen.left() + self.settings.split * screen.width();
            egui::Area::new(egui::Id::new("ab split"))
                .fixed_pos(egui::pos2(x - 6., screen.top()))
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                    let (rect, r) = ui
                        .allocate_exact_size(egui::vec2(12., screen.height()), egui::Sense::drag());
                    let r = r
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                        .on_hover_text("Drag to move the A/B split");
                    if r.dragged() {
                        self.settings.split =
                            (self.settings.split + r.drag_delta().x / screen.width()).clamp(0., 1.);
                    }
                    let grip = egui::pos2(rect.center().x, rect.center().y);
                    let fill = if r.hovered() || r.dragged() {
                        ACCENT
                    } else {
                        egui::Color32::from_white_alpha(160)
                    };
                    ui.painter().circle_filled(grip, 6., fill);
                });
        }
        // Battle mode (track 15): the quadrant split and each integrator's samples and GPU time.
        if self.settings.battle {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("battle"),
            ));
            let c = screen.center();
            let line = egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(150));
            painter.line_segment(
                [
                    egui::pos2(c.x, screen.top()),
                    egui::pos2(c.x, screen.bottom()),
                ],
                line,
            );
            painter.line_segment(
                [
                    egui::pos2(screen.left(), c.y),
                    egui::pos2(screen.right(), c.y),
                ],
                line,
            );
            let g = self.gpu.as_ref().unwrap();
            for (i, q) in g.battle.iter().enumerate() {
                let at = egui::pos2(
                    if i % 2 == 0 {
                        screen.left() + 12.
                    } else {
                        c.x + 12.
                    },
                    // Below the two toolbar rows.
                    if i < 2 { screen.top() + 62. } else { c.y + 10. },
                );
                let text = format!(
                    "{}\n{} spp · {:.1} s GPU",
                    crate::gpu::BATTLE_NAMES[i],
                    q.samples,
                    q.ms / 1000.
                );
                let font = egui::FontId::proportional(14.);
                painter.text(
                    at + egui::vec2(1., 1.),
                    egui::Align2::LEFT_TOP,
                    &text,
                    font.clone(),
                    egui::Color32::BLACK,
                );
                painter.text(at, egui::Align2::LEFT_TOP, text, font, BATTLE_COLORS[i]);
            }
        }
        // Selection and hover outlines: the projected box of the object.
        if self.settings.perspective() {
            if let Some(i) = self.ui.hovered
                && !self.ui.is_selected(Selection::Object(i))
                && self.ui.transform.is_none()
                && let Some(o) = self.settings.objects.get(i)
            {
                let stroke = egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(90));
                gizmo::draw_box_outline(
                    &painter,
                    ppp,
                    &view,
                    &crate::geometry::object_corners(o),
                    stroke,
                );
            }
            for t in &targets {
                if let Selection::Object(i) = *t
                    && let Some(o) = self.settings.objects.get(i)
                {
                    // The active item brighter than the rest of the selection.
                    let stroke = if *t == self.ui.selection {
                        egui::Stroke::new(1.5_f32, ACCENT)
                    } else {
                        egui::Stroke::new(1_f32, ACCENT.gamma_multiply(0.6))
                    };
                    gizmo::draw_box_outline(
                        &painter,
                        ppp,
                        &view,
                        &crate::geometry::object_corners(o),
                        stroke,
                    );
                }
            }
            // The 3D cursor, while it is the pivot: a red and white ring with a cross.
            if self.ui.pivot == gizmo::Pivot::Cursor
                && let Some(c) = view.project(self.ui.cursor3d)
            {
                let c = egui::pos2(c.x / ppp, c.y / ppp);
                for k in 0..8 {
                    let (a0, a1) = (
                        k as f32 / 8. * std::f32::consts::TAU,
                        (k + 1) as f32 / 8. * std::f32::consts::TAU,
                    );
                    let color = if k % 2 == 0 {
                        egui::Color32::from_rgb(232, 72, 82)
                    } else {
                        egui::Color32::WHITE
                    };
                    painter.line_segment(
                        [
                            c + 9. * egui::vec2(a0.cos(), a0.sin()),
                            c + 9. * egui::vec2(a1.cos(), a1.sin()),
                        ],
                        egui::Stroke::new(1.5_f32, color),
                    );
                }
                let cross = egui::Stroke::new(1_f32, egui::Color32::from_black_alpha(200));
                painter.line_segment([c - egui::vec2(14., 0.), c + egui::vec2(14., 0.)], cross);
                painter.line_segment([c - egui::vec2(0., 14.), c + egui::vec2(0., 14.)], cross);
            }
        }
        // Box select in progress: the rectangle from where the drag started to the pointer.
        if let Some(a) = self.ui.box_start {
            let b = egui::pos2(self.cursor[0] as f32 / ppp, self.cursor[1] as f32 / ppp);
            let r = egui::Rect::from_two_pos(a, b);
            painter.rect_filled(
                r,
                0.,
                egui::Color32::from_rgba_unmultiplied(160, 200, 255, 22),
            );
            painter.rect_stroke(
                r,
                0.,
                egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(170)),
                egui::StrokeKind::Inside,
            );
        }
        self.ui.hover = None;
        if let Some(t) = &self.ui.transform {
            if let Some(f) = t.frame() {
                gizmo::draw(
                    &painter,
                    ppp,
                    &view,
                    &f,
                    t.local,
                    self.ui.tool,
                    None,
                    Some(t),
                );
            }
            egui::Area::new(egui::Id::new("transform header"))
                .anchor(egui::Align2::CENTER_TOP, [0., 44.])
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(egui::RichText::new(&self.ui.header).strong());
                        note(ui, "X / Y / Z axis (again: local, again: free) · Shift+X/Y/Z plane · type a value (a free move: Tab for the next axis) · Ctrl snap · Shift fine · click or Enter to confirm · right-click or Esc to cancel");
                    });
                });
            return;
        }
        let mode = self.tool_mode();
        // The gizmo's projection math assumes a perspective camera.
        if self.ui.tool == gizmo::Tool::Select
            || !gizmo::allows(&targets, mode)
            || !self.settings.perspective()
        {
            return;
        }
        let Some(f) = gizmo::group_frame(&self.settings, &targets, self.ui.pivot, self.ui.cursor3d)
        else {
            return;
        };
        if !ctx.is_pointer_over_area()
            && let Some(p) = ctx.pointer_hover_pos()
        {
            let mouse = glam::Vec2::new(p.x * ppp, p.y * ppp);
            self.ui.hover = gizmo::hit(&view, &f, self.ui.local, self.ui.tool, mouse);
        }
        gizmo::draw(
            &painter,
            ppp,
            &view,
            &f,
            self.ui.local,
            self.ui.tool,
            self.ui.hover,
            None,
        );
    }
    /// Starts a transform of the selection: from the handle under the pointer, or modal.
    pub fn start_transform(&mut self, mode: gizmo::Mode, handle: Option<gizmo::Handle>) -> bool {
        let targets = self.ui.targets();
        if targets.is_empty()
            || !gizmo::allows(&targets, mode)
            || gizmo::frame(&self.settings, targets[0]).is_none()
        {
            return false;
        }
        let mouse = glam::Vec2::new(self.cursor[0] as f32, self.cursor[1] as f32);
        let mut t = match handle {
            Some(h) => gizmo::Transform::from_handle(
                mode,
                h,
                targets,
                mouse,
                &self.settings,
                self.ui.local,
            ),
            None => gizmo::Transform::new(mode, targets, mouse, &self.settings, self.ui.local),
        };
        t.pivot = self.ui.pivot;
        t.cursor = self.ui.cursor3d;
        t.snap_to = self.ui.snap_to;
        self.ui.transform = Some(t);
        self.update_transform();
        true
    }
    pub fn update_transform(&mut self) {
        let Some(g) = self.gpu.as_ref() else { return };
        let view = gizmo::View::new(&self.settings, [g.width, g.height]);
        if let Some(t) = &mut self.ui.transform {
            t.snap = self.ctrl;
            self.ui.header = t.apply(&mut self.settings, &view);
        }
    }
    pub fn finish_transform(&mut self) {
        if let Some(t) = self.ui.transform.take() {
            if self.settings != t.original {
                self.ui.checkpoint(&t.original);
                // Laser puzzle (track 17): each finished transform is one move.
                if self.settings.puzzle.enabled {
                    self.laser.moves += 1;
                }
            }
            self.ui.header.clear();
        }
    }
    pub fn cancel_transform(&mut self) {
        if let Some(t) = self.ui.transform.take() {
            t.restore(&mut self.settings);
            if t.undo_on_cancel {
                // The copy made for this move goes too (Blender's Shift+D / Alt-drag).
                self.ui.undo(&mut self.settings);
            }
            self.ui.header.clear();
            self.ui.toast("Transform cancelled");
        }
    }
    /// Keys while a transform runs. Returns true when the key was used.
    pub fn transform_key(&mut self, code: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode as K;
        let Some(t) = &mut self.ui.transform else {
            return false;
        };
        let digit = [
            (K::Digit0, '0'),
            (K::Digit1, '1'),
            (K::Digit2, '2'),
            (K::Digit3, '3'),
            (K::Digit4, '4'),
            (K::Digit5, '5'),
            (K::Digit6, '6'),
            (K::Digit7, '7'),
            (K::Digit8, '8'),
            (K::Digit9, '9'),
            (K::Numpad0, '0'),
            (K::Numpad1, '1'),
            (K::Numpad2, '2'),
            (K::Numpad3, '3'),
            (K::Numpad4, '4'),
            (K::Numpad5, '5'),
            (K::Numpad6, '6'),
            (K::Numpad7, '7'),
            (K::Numpad8, '8'),
            (K::Numpad9, '9'),
            (K::Period, '.'),
            (K::NumpadDecimal, '.'),
            (K::Minus, '-'),
            (K::NumpadSubtract, '-'),
        ]
        .iter()
        .find(|(k, _)| *k == code)
        .map(|(_, c)| *c);
        match code {
            K::KeyX | K::KeyY | K::KeyZ => {
                let axis = match code {
                    K::KeyX => 0,
                    K::KeyY => 1,
                    _ => 2,
                };
                t.constrain(axis, self.shift);
            }
            K::Backspace => {
                t.typed.pop();
            }
            // A free or plane move takes a value per axis: Tab moves to the next one.
            K::Tab
                if t.mode == gizmo::Mode::Move
                    && !matches!(t.constraint, gizmo::Constraint::Axis(_)) =>
            {
                t.next_field();
            }
            K::Enter | K::NumpadEnter => {
                self.finish_transform();
                return true;
            }
            K::Escape => {
                self.cancel_transform();
                return true;
            }
            _ => match digit {
                Some('-') => {
                    // Minus toggles the sign, as in Blender.
                    if t.typed.starts_with('-') {
                        t.typed.remove(0);
                    } else {
                        t.typed.insert(0, '-');
                    }
                }
                Some(c) => t.typed.push(c),
                None => return true,
            },
        }
        self.update_transform();
        true
    }
    fn top_bar(&mut self, ctx: &egui::Context, known: bool) {
        egui::TopBottomPanel::top("top bar").show(ctx, |ui| {
            // Wraps onto a second row when the window is too narrow, so no control is cut off.
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("Menu", |ui| {
                    ui.weak(format!("Caustica {}", crate::VERSION));
                    ui.separator();
                    ui.checkbox(&mut self.ui.advanced, "Show advanced settings");
                    if ui.button("Phone remote…").clicked() {
                        self.ui.show_remote = true;
                        ui.close_menu();
                    }
                    if ui.button("Statistics and achievements").clicked() {
                        self.ui.show_stats = true;
                        ui.close_menu();
                    }
                    ui.menu_button("Text-to-scene model", |ui| self.ai_menu(ui));
                    ui.separator();
                    if ui
                        .add_enabled(!self.ui.undo.is_empty(), egui::Button::new("Undo (Ctrl+Z)"))
                        .clicked()
                    {
                        self.ui.undo(&mut self.settings);
                        self.timeline.settle(&self.settings);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(!self.ui.redo.is_empty(), egui::Button::new("Redo (Ctrl+Y)"))
                        .clicked()
                    {
                        self.ui.redo(&mut self.settings);
                        self.timeline.settle(&self.settings);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open scene… (Ctrl+O)").clicked() {
                        self.open_scene_dialog();
                        ui.close_menu();
                    }
                    if ui.button("Save scene (Ctrl+S)").clicked() {
                        self.save_scene(false);
                        ui.close_menu();
                    }
                    if ui.button("Save scene as… (Ctrl+Shift+S)").clicked() {
                        self.save_scene(true);
                        ui.close_menu();
                    }
                    if ui.button("Save settings.json").clicked() {
                        self.actions.save = Some("settings.json".into());
                        ui.close_menu();
                    }
                    if ui.button("Save settings as…").clicked() {
                        self.actions.save = settings_dialog(true);
                        ui.close_menu();
                    }
                    if ui.button("Load settings…").clicked() {
                        self.actions.load = settings_dialog(false);
                        ui.close_menu();
                    }
                    if ui.button("Reset everything to defaults").clicked() {
                        self.ui.checkpoint(&self.settings);
                        self.settings = Settings::default();
                        self.preset = 0;
                        self.ui.selection = Selection::Stage;
                        self.ui.toast("Reset to the default scene and settings");
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Keys").strong());
                    for line in [
                        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom",
                        "Click: select an object or light · Delete: remove it · Esc: deselect",
                        "G / R / S: move, rotate, scale · then X / Y / Z, a value, Enter",
                        "Ctrl+Z / Ctrl+Y: undo / redo · Ctrl+S / Ctrl+O: save / open",
                        "Space: pause · R: restart (nothing selected) · F: frame · A: auto orbit",
                        "D: native denoiser · C: freeze A/B reference · 1–8: views",
                        "F12: save PNG · Tab: hide panels · Ctrl +/−: UI size · F1: all keys",
                    ] {
                        ui.label(egui::RichText::new(line).small());
                    }
                    ui.separator();
                    let g = self.gpu.as_ref().unwrap();
                    note(
                        ui,
                        &format!("{} · Rust, Direct3D 12, DXR 1.1", g.adapter_name),
                    );
                });
                ui.separator();
                if ui
                    .button("◀")
                    .on_hover_text("Previous scene (Page Up)")
                    .clicked()
                {
                    self.step_scene(-1);
                }
                let current = if known {
                    PRESETS[self.preset].to_string()
                } else if !self.settings.scene_name.is_empty() {
                    self.settings.scene_name.clone()
                } else {
                    "Custom scene".to_string()
                };
                egui::ComboBox::from_id_salt("scene picker")
                    .selected_text(current)
                    .width(175.)
                    .height(600.)
                    .show_ui(ui, |ui| {
                        for (group, range) in GROUPS {
                            ui.label(egui::RichText::new(group).small().color(ACCENT));
                            for i in range {
                                // Picking the current scene again reverts it to the preset.
                                if ui
                                    .selectable_label(known && self.preset == i, PRESETS[i])
                                    .clicked()
                                {
                                    self.preset = i;
                                    self.settings.preset(i);
                                }
                            }
                            ui.separator();
                        }
                    })
                    .response
                    .on_hover_text("Pick a scene. Picking the current one again restores it.");
                if ui
                    .button("▶")
                    .on_hover_text("Next scene (Page Down)")
                    .clicked()
                {
                    self.step_scene(1);
                }
                if ui
                    .selectable_label(
                        self.ui.show_scenes,
                        format!("Scenes ({})", self.ui.scenes.len()),
                    )
                    .on_hover_text("Browse the scene files in scenes/")
                    .clicked()
                {
                    self.ui.show_scenes = !self.ui.show_scenes;
                }
                if ui
                    .selectable_label(self.ui.assets.open, "Assets")
                    .on_hover_text(
                        "Download HDRIs, textures and models (Poly Haven, ambientCG, Khronos)",
                    )
                    .clicked()
                {
                    self.ui.assets.open = !self.ui.assets.open;
                }
                if ui
                    .button("Randomize")
                    .on_hover_text("A random variation of a random scene")
                    .clicked()
                {
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos()
                        | 1;
                    self.settings.randomize(seed);
                }
                if ui
                    .button("Today")
                    .on_hover_text(
                        "Seed of the day: everyone running this today gets the same random scene",
                    )
                    .clicked()
                {
                    self.settings.randomize(seed_of_the_day());
                    let name = self.settings.scene_name.clone();
                    self.ui.toast(format!("Seed of the day: {name}"));
                }
                ui.separator();
                self.quality_combo(ui);
                let s = &mut self.settings;
                egui::ComboBox::from_id_salt("top view")
                    .selected_text(VIEWS[s.view as usize])
                    .width(110.)
                    .show_ui(ui, |ui| {
                        for (i, n) in VIEWS.iter().enumerate() {
                            ui.selectable_value(&mut s.view, i as u32, format!("{n}  ({})", i + 1));
                        }
                    })
                    .response
                    .on_hover_text("What the viewport shows (keys 1–8)");
                if ui
                    .button(if self.paused {
                        "▶ Resume"
                    } else {
                        "⏸ Pause"
                    })
                    .on_hover_text("Space")
                    .clicked()
                {
                    self.paused = !self.paused;
                }
                if ui
                    .button("Restart")
                    .on_hover_text("Throw away the samples and start again (R)")
                    .clicked()
                {
                    self.actions.reset = true;
                }
                if ui
                    .button("Frame")
                    .on_hover_text("Fit the camera to the scene (F)")
                    .clicked()
                {
                    self.actions.frame_scene = true;
                }
                if ui
                    .selectable_label(self.painting, "Paint light")
                    .on_hover_text(
                        "Light painting (L): lights you move leave trails in one long exposure",
                    )
                    .clicked()
                {
                    self.toggle_painting();
                }
                if ui
                    .selectable_label(self.walking.is_some(), "Walk")
                    .on_hover_text(
                        "Walk mode (`): WASD, Space jump, C or Ctrl crouch, Shift run, F fly                          (Q / E down / up), B head bob, wheel body size, drag to look",
                    )
                    .clicked()
                {
                    self.toggle_walk();
                }
                if ui
                    .selectable_label(
                        self.ui.show_timeline || !self.settings.animation.is_empty(),
                        "Timeline",
                    )
                    .on_hover_text(
                        "Keyframes: I keys the selection (the camera when nothing is selected), K plays",
                    )
                    .clicked()
                {
                    self.ui.show_timeline = !self.ui.show_timeline;
                }
                ui.separator();
                for (tool, label, tip) in [
                    (gizmo::Tool::Select, "Select", "Click to select, no handles"),
                    (gizmo::Tool::Move, "Move", "Arrows and planes. Key: G"),
                    (gizmo::Tool::Rotate, "Rotate", "Rings. Key: R"),
                    (
                        gizmo::Tool::Scale,
                        "Scale",
                        "Axis handles; the circle scales evenly. Key: S",
                    ),
                ] {
                    ui.selectable_value(&mut self.ui.tool, tool, label)
                        .on_hover_text(tip);
                }
                egui::ComboBox::from_id_salt("orientation")
                    .selected_text(if self.ui.local { "Local" } else { "Global" })
                    .width(70.)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ui.local, false, "Global");
                        ui.selectable_value(&mut self.ui.local, true, "Local");
                    })
                    .response
                    .on_hover_text("Gizmo axes: the world's, or the object's own");
                egui::ComboBox::from_id_salt("pivot")
                    .selected_text(self.ui.pivot.label())
                    .width(120.)
                    .show_ui(ui, |ui| {
                        for p in gizmo::Pivot::ALL {
                            ui.selectable_value(&mut self.ui.pivot, p, p.label());
                        }
                    })
                    .response
                    .on_hover_text("What several selected items rotate and scale around. Shift+right-click places the 3D cursor");
                egui::ComboBox::from_id_salt("snap")
                    .selected_text(format!("Snap: {}", self.ui.snap_to.label()))
                    .width(120.)
                    .show_ui(ui, |ui| {
                        for t in crate::snap::Target::ALL {
                            ui.selectable_value(&mut self.ui.snap_to, t, t.label());
                        }
                    })
                    .response
                    .on_hover_text("Hold Ctrl while transforming. Increment: 0.1 units, 15°, 0.1×. Surface / Vertex / Edge: a move goes to what is under the pointer");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.ui.panel_hidden {
                        "Show panel"
                    } else {
                        "Hide panel"
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Tab. A second Tab hides everything.")
                        .clicked()
                    {
                        self.ui.panel_hidden = !self.ui.panel_hidden;
                    }
                });
            });
        });
    }
    fn status_bar(&mut self, ctx: &egui::Context) {
        let (text, fallback) = pipeline(self);
        let g = self.gpu.as_ref().unwrap();
        let s = &self.settings;
        egui::TopBottomPanel::bottom("status bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let converged = g.samples >= s.max_samples;
                let (state, color) = if self.paused {
                    ("Paused", egui::Color32::GRAY)
                } else if converged {
                    ("Converged", ACCENT)
                } else {
                    ("Rendering", egui::Color32::from_rgb(110, 200, 120))
                };
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10., 10.), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4., color);
                ui.label(state);
                ui.separator();
                ui.monospace(format!("{} / {} spp", g.samples, s.max_samples));
                ui.add(
                    egui::ProgressBar::new((g.samples as f32 / s.max_samples as f32).min(1.))
                        .desired_width(80.)
                        .desired_height(8.),
                );
                // Time to converge, from samples per second smoothed over about a second.
                let now = Instant::now();
                let (at, samples, rate) = &mut self.ui.sample_rate;
                let elapsed = now.duration_since(*at).as_secs_f32();
                if g.samples < *samples {
                    (*at, *samples, *rate) = (now, g.samples, 0.);
                } else if elapsed > 0.25 {
                    let r = (g.samples - *samples) as f32 / elapsed;
                    *rate = if *rate > 0. { *rate * 0.6 + r * 0.4 } else { r };
                    (*at, *samples) = (now, g.samples);
                }
                if !converged && !self.paused && *rate > 0. {
                    let left = (s.max_samples - g.samples.min(s.max_samples)) as f32 / *rate;
                    ui.monospace(if left < 90. {
                        format!("~{left:.0} s")
                    } else {
                        format!("~{:.0} min", left / 60.)
                    })
                    .on_hover_text("Time until the sample cap at the current speed");
                }
                ui.separator();
                let [trace, rebuild, display, gui] = g.pass_ms;
                let passes = format!(
                    "Trace {trace:.2} ms · denoise / RR / upscale {rebuild:.2} ms · display {display:.2} ms · UI {gui:.2} ms"
                );
                if self.ui.advanced {
                    ui.monospace(format!(
                        "{:.2} ms GPU (trace {trace:.2} · reconstruct {rebuild:.2} · display {display:.2} · UI {gui:.2})",
                        g.gpu_ms
                    ));
                } else {
                    ui.monospace(format!("{:.2} ms GPU", g.gpu_ms))
                        .on_hover_text(passes);
                }
                ui.separator();
                ui.label(if fallback {
                    egui::RichText::new(&text).color(WARN)
                } else {
                    egui::RichText::new(&text)
                });
                ui.separator();
                if g.rw == g.width {
                    ui.label(format!("{} × {}", g.width, g.height));
                } else {
                    ui.label(format!(
                        "render {} × {} · output {} × {}",
                        g.rw, g.rh, g.width, g.height
                    ));
                }
                if g.hdr_active {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("HDR {:.0} nits", g.hdr_levels[1]))
                            .color(ACCENT),
                    )
                    .on_hover_text("Real HDR output. Details in the Look tab.");
                }
                if self.ui.advanced {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} objects · {} triangles · {} emissive",
                            s.objects.len(),
                            g.triangle_count,
                            g.emitter_count
                        ))
                        .small(),
                    );
                }
            });
        });
    }
    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.ui.toasts.retain(|t| t.until > now);
        if self.ui.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16., -40.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in self.ui.toasts.iter().rev() {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(if t.error {
                                egui::RichText::new(&t.text).color(WARN)
                            } else {
                                egui::RichText::new(&t.text)
                            });
                            if let Some(file) = &t.file
                                && ui.small_button("Show in folder").clicked()
                            {
                                show_in_folder(file);
                            }
                        });
                    });
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn side_panel(&mut self, ctx: &egui::Context, known: bool) {
        egui::SidePanel::right("side panel")
            .resizable(true)
            .default_width(370.)
            .width_range(320. ..=560.)
            .show(ctx, |ui| {
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Scene, "Scene"),
                        (Tab::Render, "Render"),
                        (Tab::Look, "Look"),
                        (Tab::Export, "Export"),
                    ] {
                        ui.selectable_value(
                            &mut self.ui.tab,
                            tab,
                            egui::RichText::new(label).heading(),
                        );
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| match self.ui.tab {
                        Tab::Scene => self.scene_tab(ui, known),
                        Tab::Render => self.render_tab(ui),
                        Tab::Look => self.look_tab(ui),
                        Tab::Export => self.export_tab(ui),
                    });
            });
    }
    /// Track 19: "Describe a scene" sends the text to Claude and loads the scene it returns.
    /// Applies a finished text-to-scene request (whatever tab is open).
    /// Starts a text-to-scene request for `mode` on its own thread (`png`: the render, for a
    /// critique).
    pub fn start_describe(&mut self, mode: AiMode, png: Option<Vec<u8>>, ctx: &egui::Context) {
        use crate::textscene::Task;
        let task = match (mode, png) {
            (AiMode::New, _) => Task::New,
            (AiMode::Edit, _) => Task::Edit(self.settings.clone()),
            (AiMode::Critique, Some(png)) => Task::Critique {
                settings: self.settings.clone(),
                png,
            },
            (AiMode::Critique, None) => return,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ai, key, prompt) = (
            self.ui.prefs.ai.clone(),
            self.ui.ai_key.clone(),
            self.ui.describe.clone(),
        );
        let (stop, ctx) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let mut progress = move |s: String| {
                let _ = progress_tx.send(AiMsg::Progress(s));
                progress_ctx.request_repaint();
            };
            let result =
                crate::textscene::transport(&ai, &key, stop.clone()).and_then(|mut send| {
                    crate::textscene::generate(&prompt, &task, &mut *send, &mut progress, &stop)
                });
            let _ = tx.send(AiMsg::Done(Box::new(result)));
            ctx.request_repaint();
        });
        self.ui.ai_notes.clear();
        self.ui.critique_scene = None;
        self.ui.describe_job = Some(AiJob {
            rx,
            cancel,
            started: std::time::Instant::now(),
            status: "Starting".into(),
            mode,
        });
    }
    fn poll_describe(&mut self, ctx: &egui::Context) {
        if let Some(png) = self.ui.critique_png.take() {
            self.start_describe(AiMode::Critique, Some(png), ctx);
        }
        let fetched = self
            .ui
            .ai_models_job
            .as_ref()
            .and_then(|(rx, server)| match rx.try_recv() {
                Ok(r) => Some((r, server.clone())),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    Err(anyhow::anyhow!("the request stopped unexpectedly")),
                    server.clone(),
                )),
            });
        if let Some((r, server)) = fetched {
            self.ui.ai_models_job = None;
            let current = ai_server(&self.ui.prefs.ai);
            match r {
                // Asked of another server than the one now chosen.
                _ if server != current => {}
                Ok(list) if list.is_empty() => self.ui.error("The server listed no models"),
                Ok(list) => {
                    self.ui.toast(format!("{} models", list.len()));
                    self.ui.ai_models = list;
                }
                Err(e) => self.ui.error(format!("Fetching models: {e:#}")),
            }
        }
        let Some(job) = &mut self.ui.describe_job else {
            return;
        };
        let result = loop {
            match job.rx.try_recv() {
                Ok(AiMsg::Progress(s)) => job.status = s,
                Ok(AiMsg::Done(r)) => break *r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    break Err(anyhow::anyhow!("the request stopped unexpectedly"));
                }
            }
        };
        let mode = job.mode;
        self.ui.describe_job = None;
        match result {
            Ok(outcome) => {
                self.ui.ai_notes = outcome.notes;
                if mode == AiMode::Critique {
                    // A critique's scene waits for Apply, next to the notes.
                    self.ui.critique_scene = Some(outcome.settings);
                    self.ui.toast("The critique is in the Scene tab");
                } else {
                    let who = self.ui.prefs.ai.who();
                    let title = self.apply_ai_scene(outcome.settings, mode == AiMode::New);
                    let verb = if mode == AiMode::New {
                        "made"
                    } else {
                        "changed"
                    };
                    self.ui.toast(format!("{who} {verb} {title}"));
                }
            }
            Err(e) => self.ui.error(format!("{e:#}")),
        }
    }
    /// Loads a scene from the model (undoable). `fresh`: a new scene, not the file or list
    /// entry that was open (Ctrl+S must not overwrite that file because the titles happen to
    /// match); an edit keeps them. Returns the title.
    fn apply_ai_scene(&mut self, mut loaded: Settings, fresh: bool) -> String {
        self.ui.checkpoint(&self.settings);
        self.laser = crate::laser::Laser::default();
        self.physics = None;
        self.ui.selection = Selection::Stage;
        if fresh {
            self.ui.loaded_file = None;
            self.ui.scene_selected = None;
        }
        loaded.keep_output(&self.settings);
        if loaded.scene_name.trim().is_empty() {
            loaded.scene_name = if fresh {
                "Described scene".into()
            } else {
                self.settings.scene_name.clone()
            };
        }
        let title = loaded.scene_name.clone();
        self.settings = loaded;
        self.reset_baseline = self.settings.clone();
        title
    }
    /// Menu > Text-to-scene model: the backend, its endpoint and key (the model and the
    /// reasoning level sit under the text box).
    fn ai_menu(&mut self, ui: &mut egui::Ui) {
        use crate::textscene::{Protocol, Provider};
        let before = self.ui.prefs.ai.clone();
        let ai = &mut self.ui.prefs.ai;
        for p in Provider::ALL {
            ui.radio_value(&mut ai.provider, p, p.label());
        }
        ui.separator();
        egui::Grid::new("ai grid").num_columns(2).show(ui, |ui| {
            if ai.provider == Provider::Custom {
                ui.label("Endpoint");
                ui.add(
                    egui::TextEdit::singleline(&mut ai.base_url)
                        .hint_text("http://localhost:11434/v1")
                        .desired_width(220.),
                )
                .on_hover_text("Ollama: http://localhost:11434/v1 · LM Studio: http://localhost:1234/v1 · llama.cpp: http://localhost:8080/v1");
                ui.end_row();
                ui.label("Protocol");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut ai.protocol, Protocol::OpenAi, "OpenAI-compatible");
                    ui.radio_value(&mut ai.protocol, Protocol::Anthropic, "Anthropic-compatible");
                });
                ui.end_row();
            }
            if matches!(ai.provider, Provider::ClaudeApi | Provider::OpenAi | Provider::Custom) {
                ui.label("API key");
                let hint = ai
                    .provider
                    .key_env()
                    .map_or("optional".to_string(), |e| format!("or set {e}"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.ui.ai_key)
                        .password(true)
                        .hint_text(hint)
                        .desired_width(220.),
                )
                .on_hover_text("Kept for this session only, never saved");
                ui.end_row();
            }
        });
        let note = match ai.provider {
            Provider::ClaudeCli => Some(
                "Runs `claude -p` with the CLI's login: a Claude subscription works, no key needed.",
            ),
            Provider::CodexCli => Some(
                "Runs `codex exec` with the Codex CLI's login: sign in once with `codex login` (a ChatGPT account works). Install it with: npm install -g @openai/codex",
            ),
            _ => None,
        };
        if let Some(note) = note {
            ui.label(egui::RichText::new(note).small());
        }
        let now = &self.ui.prefs.ai;
        if *now != before {
            if now.provider != before.provider || now.protocol != before.protocol {
                // Another kind of server: its models and reasoning levels differ.
                self.ui.ai_models.clear();
                self.ui.prefs.ai.model.clear();
                self.ui.prefs.ai.effort.clear();
            } else if now.base_url != before.base_url {
                // Typing a URL keeps the model; only the fetched list belongs to the old one.
                self.ui.ai_models.clear();
            }
            self.ui.save_prefs();
        }
    }
    /// The model and reasoning pickers under the text box.
    fn ai_model_row(&mut self, ui: &mut egui::Ui) {
        use crate::textscene::Provider;
        let before = self.ui.prefs.ai.clone();
        let provider = before.provider;
        ui.horizontal_wrapped(|ui| {
            ui.label("Model");
            let hint = match provider {
                Provider::ClaudeCli | Provider::CodexCli => "the CLI's default",
                Provider::ClaudeApi => "claude-opus-5-5",
                _ => "required",
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.ui.prefs.ai.model)
                    .hint_text(hint)
                    .desired_width(170.),
            );
            let mut names: Vec<String> = provider
                .suggested_models()
                .iter()
                .map(|s| s.to_string())
                .collect();
            for m in &self.ui.ai_models {
                if !names.contains(m) {
                    names.push(m.clone());
                }
            }
            if !names.is_empty() {
                ui.menu_button("Pick", |ui| {
                    egui::ScrollArea::vertical().max_height(300.).show(ui, |ui| {
                        if ui.button("Default").clicked() {
                            self.ui.prefs.ai.model.clear();
                            ui.close_menu();
                        }
                        for n in &names {
                            if ui.button(n).clicked() {
                                self.ui.prefs.ai.model = n.clone();
                                ui.close_menu();
                            }
                        }
                    });
                });
            }
            if provider.can_fetch() {
                let fetching = self.ui.ai_models_job.is_some();
                if ui
                    .add_enabled(!fetching, egui::Button::new("Fetch"))
                    .on_hover_text("Lists the models the server offers")
                    .clicked()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (ai, key, ctx) = (
                        self.ui.prefs.ai.clone(),
                        self.ui.ai_key.clone(),
                        ui.ctx().clone(),
                    );
                    let server = ai_server(&ai);
                    std::thread::spawn(move || {
                        let _ = tx.send(crate::textscene::fetch_models(&ai, &key));
                        ctx.request_repaint();
                    });
                    self.ui.ai_models_job = Some((rx, server));
                }
                if fetching {
                    ui.spinner();
                }
            }
            ui.label("Reasoning");
            let levels = provider.efforts(before.protocol);
            let shown = |e: &str| {
                if e.is_empty() {
                    "Default".to_string()
                } else {
                    e.to_string()
                }
            };
            egui::ComboBox::from_id_salt("ai effort")
                .selected_text(shown(&self.ui.prefs.ai.effort))
                .show_ui(ui, |ui| {
                    for e in levels {
                        ui.selectable_value(&mut self.ui.prefs.ai.effort, e.to_string(), shown(e));
                    }
                })
                .response
                .on_hover_text(
                    "How long the model thinks before answering: higher is slower and usually better",
                );
        });
        if self.ui.prefs.ai != before {
            self.ui.save_prefs();
        }
    }
    fn describe_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Describe a scene (AI)")
            .id_salt("describe")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let m = &mut self.ui.describe_mode;
                    ui.selectable_value(m, AiMode::New, "New scene")
                        .on_hover_text("Builds a new scene from the description");
                    ui.selectable_value(m, AiMode::Edit, "Edit this scene")
                        .on_hover_text("Changes the current scene as described (\"make it night\")");
                    ui.selectable_value(m, AiMode::Critique, "Critique the render")
                        .on_hover_text("Sends the current render and scene: the model suggests lighting and composition changes, which you can apply");
                });
                let mode = self.ui.describe_mode;
                let hint = match mode {
                    AiMode::New => "A lighthouse on a rock at dusk, a warm lamp in its top…",
                    AiMode::Edit => "Make it night, with the lamp as the only light…",
                    AiMode::Critique => "Optional: what to focus on (mood, colour, the subject)…",
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.ui.describe)
                        .hint_text(hint)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                self.ai_model_row(ui);
                let missing = self.ui.prefs.ai.missing(&self.ui.ai_key);
                ui.horizontal_wrapped(|ui| {
                    if let Some(job) = &self.ui.describe_job {
                        ui.spinner();
                        ui.label(format!("{} ({} s)", job.status, job.started.elapsed().as_secs()));
                        if ui.button("Cancel").clicked() {
                            // Stops a running CLI; an HTTP request finishes in the background
                            // and its result is dropped with the channel.
                            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            self.ui.describe_job = None;
                        }
                        // The elapsed time ticks.
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        return;
                    }
                    if self.ui.critique_capture {
                        ui.spinner();
                        ui.label("Saving the render");
                        return;
                    }
                    let text_ok = mode == AiMode::Critique || !self.ui.describe.trim().is_empty();
                    let label = match mode {
                        AiMode::New => "Generate",
                        AiMode::Edit => "Apply edit",
                        AiMode::Critique => "Critique",
                    };
                    if ui
                        .add_enabled(missing.is_none() && text_ok, egui::Button::new(label))
                        .on_hover_text(format!(
                            "Sends it to {}. The result is undoable (Ctrl+Z). Change the backend in Menu > Text-to-scene model",
                            self.ui.prefs.ai.provider.label()
                        ))
                        .clicked()
                    {
                        if mode == AiMode::Critique {
                            // main.rs saves the render after this frame and starts it.
                            self.ui.critique_capture = true;
                        } else {
                            self.start_describe(mode, None, ui.ctx());
                        }
                    }
                    match &missing {
                        Some(why) => ui.label(egui::RichText::new(why).small()),
                        None => ui.label(
                            egui::RichText::new(self.ui.prefs.ai.provider.label())
                                .small()
                                .weak(),
                        ),
                    };
                });
                if !self.ui.ai_notes.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.ui.ai_notes).small());
                }
                if self.ui.critique_scene.is_some() {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Apply the suggestions")
                            .on_hover_text("Loads the improved scene (Ctrl+Z undoes it)")
                            .clicked()
                            && let Some(s) = self.ui.critique_scene.take()
                        {
                            let title = self.apply_ai_scene(s, false);
                            self.ui.toast(format!("Applied the critique to {title}"));
                        }
                        if ui.button("Dismiss").clicked() {
                            self.ui.critique_scene = None;
                            self.ui.ai_notes.clear();
                        }
                    });
                }
            });
    }
    fn scene_tab(&mut self, ui: &mut egui::Ui, known: bool) {
        self.describe_ui(ui);
        let description = if known {
            crate::presets::description(self.preset).to_string()
        } else if let Some(i) = self.scene_entry() {
            self.ui.scenes[i].description.clone()
        } else {
            "Custom scene from a settings file.".to_string()
        };
        let g = self.gpu.as_ref().unwrap();
        let aspect = g.width as f32 / g.height as f32;
        let awake = self.physics.as_ref().map_or(0, |p| p.awake());
        let (moves, solved) = (self.laser.moves, self.laser.solved);
        let s = &mut self.settings;
        let state = &mut self.ui;
        note(ui, &description);
        section(ui, "Outliner");
        // Shift+click adds to the selection or removes from it (multi-select, 3b).
        let shift = ui.input(|i| i.modifiers.shift);
        let pick = |state: &mut UiState, r: &egui::Response, sel: Selection| {
            if r.clicked() {
                if shift {
                    state.toggle(sel);
                } else {
                    state.select(sel);
                }
            }
        };
        egui::ScrollArea::vertical()
            .id_salt("outliner")
            .max_height(230.)
            .show(ui, |ui| {
                ui.selectable_value(
                    &mut state.selection,
                    Selection::Stage,
                    "Stage: room, floor and sky",
                );
                ui.selectable_value(&mut state.selection, Selection::Camera, "Camera");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.light_on, "").on_hover_text("On / off");
                    let r =
                        ui.selectable_label(state.is_selected(Selection::MainLight), "Main light");
                    pick(state, &r, Selection::MainLight);
                });
                for (i, l) in s.lights.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.push_id(("light", i), |ui| {
                            ui.checkbox(&mut l.enabled, "").on_hover_text("On / off");
                        });
                        let r = ui.selectable_label(
                            state.is_selected(Selection::Light(i)),
                            format!("Light · {}", l.name),
                        );
                        pick(state, &r, Selection::Light(i));
                    });
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "Objects ({} / {})",
                        s.objects.len(),
                        scene::MAX_OBJECTS
                    ))
                    .small(),
                );
                if s.objects.len() > 8 {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.search).hint_text("Search objects…"),
                    );
                }
                let search = state.search.to_lowercase();
                for (i, o) in s.objects.iter_mut().enumerate() {
                    if !o.name.to_lowercase().contains(&search) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.push_id(("object", i), |ui| {
                            ui.checkbox(&mut o.visible, "").on_hover_text("Visible");
                        });
                        let r =
                            ui.selectable_label(state.is_selected(Selection::Object(i)), &o.name);
                        pick(state, &r, Selection::Object(i));
                        // A right click keeps a selection the row is part of (its menu acts
                        // on all of it).
                        if r.secondary_clicked() && !state.is_selected(Selection::Object(i)) {
                            state.select(Selection::Object(i));
                        }
                        r.context_menu(|ui| {
                            if let Some(c) = context_items(ui, true) {
                                state.pending = Some(c);
                            }
                        });
                    });
                }
            });
        ui.horizontal(|ui| {
            editor::shape_combo(ui, "add shape", &mut state.add_shape);
            if ui
                .add_enabled(
                    s.objects.len() < scene::MAX_OBJECTS,
                    egui::Button::new("+ Object"),
                )
                .clicked()
            {
                state.checkpoint(s);
                s.objects
                    .push(editor::new_object(state.add_shape, s.objects.len()));
                state.selection = Selection::Object(s.objects.len() - 1);
            }
            if ui
                .add_enabled(
                    s.objects.len() < scene::MAX_OBJECTS,
                    egui::Button::new("+ Mesh…"),
                )
                .on_hover_text("Import an OBJ, glTF or GLB file")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Meshes", &["obj", "gltf", "glb"])
                    .set_directory(crate::scenes::folder().unwrap_or_default())
                    .pick_file()
            {
                match crate::mesh::import_objects(&path) {
                    Ok(parts) => {
                        state.checkpoint(s);
                        let first = s.objects.len();
                        let room = scene::MAX_OBJECTS - first;
                        if parts.len() > room {
                            state.error(format!(
                                "Only {room} of {} parts fit ({} objects)",
                                parts.len(),
                                scene::MAX_OBJECTS
                            ));
                        }
                        s.objects.extend(parts.into_iter().take(room));
                        state.selection = Selection::Object(first);
                    }
                    Err(e) => state.error(format!("{e:#}")),
                }
            }
            ui.add_enabled_ui(s.lights.len() < scene::MAX_LIGHTS, |ui| {
                ui.menu_button("+ Light", |ui| {
                    for kind in scene::LightKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            state.checkpoint(s);
                            s.lights.push(editor::new_light(kind, s.lights.len() + 1));
                            state.selection = Selection::Light(s.lights.len() - 1);
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Add a light");
            });
        });
        // A selection can outlive its item (preset switch, load, undo-free deletes).
        state.selection = match state.selection {
            Selection::Object(i) if i >= s.objects.len() => Selection::Stage,
            Selection::Light(i) if i >= s.lights.len() => Selection::MainLight,
            other => other,
        };
        ui.separator();
        // Track 17 physics sandbox (see physics.rs).
        section(ui, "Physics");
        let p = &mut s.physics;
        ui.checkbox(&mut p.enabled, "Simulate (rigid bodies)").on_hover_text(
            "Objects fall, stack and collide. Drag an object to grab and throw it; E explodes under the pointer.",
        );
        if p.enabled {
            egui::Grid::new("physics").num_columns(2).show(ui, |ui| {
                row(
                    ui,
                    "Gravity",
                    "Downward acceleration, scene units/s² (negative pulls up)",
                    |ui| {
                        ui.add(egui::Slider::new(&mut p.gravity, -30. ..=30.));
                    },
                );
                row(
                    ui,
                    "Speed",
                    "1 = real time, lower is slow motion, 0 pauses",
                    |ui| {
                        ui.add(egui::Slider::new(&mut p.speed, 0. ..=4.));
                    },
                );
                row(ui, "Bounce", "Restitution of every body", |ui| {
                    ui.add(egui::Slider::new(&mut p.bounce, 0. ..=1.));
                });
                row(ui, "Friction", "Friction of every body", |ui| {
                    ui.add(egui::Slider::new(&mut p.friction, 0. ..=2.));
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Wake all").clicked() {
                    self.actions.physics_wake = true;
                }
                if ui
                    .button("Explode (E)")
                    .on_hover_text("A blast at the middle of the floor")
                    .clicked()
                {
                    self.actions.physics_blast = true;
                }
                ui.label(egui::RichText::new(format!("{awake} awake")).small());
            });
        }
        // Track 19 audio-reactive lights (see audio.rs).
        section(ui, "Audio");
        let a = &mut s.audio;
        ui.checkbox(&mut a.enabled, "Lights follow the music").on_hover_text(
            "Captures what the PC plays (loopback) and scales the lights by bass, mid and treble: the main light follows the bass, area light and emitter i band i mod 3. Every frame changes, so use DLSS RR. The scene's intensities are not changed.",
        );
        if a.enabled {
            egui::Grid::new("audio").num_columns(2).show(ui, |ui| {
                row(ui, "Gain", "Added intensity at full level (×)", |ui| {
                    ui.add(egui::Slider::new(&mut a.gain, 0. ..=8.));
                });
                row(ui, "Floor", "Intensity at silence (×)", |ui| {
                    ui.add(egui::Slider::new(&mut a.floor, 0. ..=1.));
                });
                row(ui, "Smoothing", "Higher is calmer", |ui| {
                    ui.add(egui::Slider::new(&mut a.smoothing, 0. ..=0.99));
                });
            });
        }
        // Track 17 laser puzzle (see laser.rs).
        section(ui, "Laser puzzle");
        let p = &mut s.puzzle;
        ui.checkbox(&mut p.enabled, "Laser").on_hover_text(
            "A laser beam traced through the objects: mirrors (metallic, smooth) reflect it, beam splitters fork it, glass bends it. Route it to the target; each finished move counts.",
        );
        if p.enabled {
            egui::Grid::new("puzzle").num_columns(2).show(ui, |ui| {
                row(ui, "Origin", "Where the beam starts (x, y, z)", |ui| {
                    for v in &mut p.origin {
                        ui.add(egui::DragValue::new(v).speed(0.01).range(-40. ..=40.));
                    }
                });
                row(
                    ui,
                    "Direction",
                    "The beam's direction (x, y, z), any length",
                    |ui| {
                        for v in &mut p.direction {
                            ui.add(egui::DragValue::new(v).speed(0.01).range(-1. ..=1.));
                        }
                    },
                );
                row(ui, "Target", "The object the beam must reach", |ui| {
                    let name = |i: i32| {
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| s.objects.get(i))
                            .map_or("None".to_string(), |o| o.name.clone())
                    };
                    egui::ComboBox::from_id_salt("puzzle target")
                        .selected_text(name(p.target))
                        .show_ui(ui, |ui| {
                            for i in -1..s.objects.len() as i32 {
                                ui.selectable_value(&mut p.target, i, name(i));
                            }
                        });
                });
                row(ui, "Par", "Moves a good solution needs", |ui| {
                    ui.add(egui::DragValue::new(&mut p.par).range(0..=99));
                });
                row(ui, "Colour", "Beam colour in the viewport", |ui| {
                    ui.color_edit_button_rgb(&mut p.color);
                });
            });
            let status = if solved {
                "Solved"
            } else if p.target < 0 {
                "No target"
            } else {
                "Not solved"
            };
            ui.label(
                egui::RichText::new(format!("{status} · {moves} moves · par {}", p.par)).small(),
            );
        }
        let advanced = state.advanced;
        match state.selection {
            Selection::None => note(
                ui,
                "Click an object in the viewport, or pick an item above.",
            ),
            Selection::Stage => stage_ui(ui, s, advanced),
            Selection::Camera => {
                if camera_ui(ui, s) {
                    let mut d = Settings::default();
                    d.preset(self.preset);
                    (s.yaw, s.pitch, s.distance, s.target, s.fov) =
                        (d.yaw, d.pitch, d.distance, d.target, d.fov);
                }
                if ui.button("Frame the scene (F)").clicked() {
                    self.actions.frame_scene = true;
                }
            }
            Selection::MainLight => main_light_ui(ui, s, advanced),
            Selection::Light(i) => {
                section(ui, "Area light");
                editor::light_ui(ui, &mut s.lights[i], advanced);
                if ui.button("Delete light (Del)").clicked()
                    && let Some(name) = state.delete_selected(s)
                {
                    state.toast(format!("Deleted {name}"));
                }
            }
            Selection::Object(i) => {
                section(ui, "Object");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            s.objects.len() < scene::MAX_OBJECTS,
                            egui::Button::new("Duplicate (Ctrl+D)"),
                        )
                        .clicked()
                    {
                        state.duplicate_selected(s, true);
                    }
                    if ui.button("Delete (Del)").clicked()
                        && let Some(name) = state.delete_selected(s)
                    {
                        state.toast(format!("Deleted {name}"));
                    }
                    if ui
                        .button("Focus")
                        .on_hover_text("Point the camera at it")
                        .clicked()
                    {
                        s.focus_object(i, aspect);
                    }
                });
                // Portals pair two objects both ways.
                let current = s.objects[i].portal;
                let mut pick = current;
                egui::ComboBox::from_label("Portal to")
                    .selected_text(usize::try_from(current).ok().and_then(|p| s.objects.get(p)).map_or("None".to_string(), |o| o.name.clone()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, -1, "None");
                        for (j, o) in s.objects.iter().enumerate().filter(|(j, _)| *j != i) {
                            ui.selectable_value(&mut pick, j as i32, &o.name);
                        }
                    })
                    .response
                    .on_hover_text("Rays that hit this object continue from the same spot on the other one: corridors, impossible rooms.");
                if pick != current {
                    state.checkpoint(s);
                    if let Some(old) = usize::try_from(current)
                        .ok()
                        .and_then(|p| s.objects.get_mut(p))
                    {
                        old.portal = -1;
                    }
                    s.objects[i].portal = pick;
                    if let Some(other) = usize::try_from(pick)
                        .ok()
                        .and_then(|p| s.objects.get_mut(p))
                    {
                        if let Some(third) = usize::try_from(other.portal).ok().filter(|t| *t != i)
                        {
                            s.objects[third].portal = -1;
                        }
                        s.objects[pick as usize].portal = i as i32;
                    }
                }
                if let Selection::Object(i) = state.selection {
                    editor::object_ui(ui, &mut s.objects[i], advanced);
                }
            }
        }
    }
    fn render_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        section(ui, "Reconstruction");
        let caps = g.capabilities();
        ui.add_enabled(caps.rr.available || s.rr, egui::Checkbox::new(&mut s.rr, "DLSS Ray Reconstruction"))
            .on_hover_text("NVIDIA's neural denoiser and upscaler in one. When off, the native denoiser and the upscaler below are used.")
            .on_disabled_hover_text(format!("Unavailable: {}", caps.rr.reason));
        if s.rr {
            let color = if g.rr_active {
                ui.visuals().weak_text_color()
            } else {
                WARN
            };
            ui.label(egui::RichText::new(&g.rr_status).small().color(color));
        }
        grid("reconstruction").show(ui, |ui| {
            if s.rr {
                row(ui, "RR model", "The DLSS Ray Reconstruction model.", |ui| {
                    egui::ComboBox::from_id_salt("rr model")
                        .selected_text(rr_model(s.rr_preset))
                        .show_ui(ui, |ui| {
                            for id in [6, 5, 4, 0] {
                                ui.selectable_value(&mut s.rr_preset, id, rr_model(id));
                            }
                        });
                });
            }
            row(
                ui,
                "Quality mode",
                "Share of the pixels that are traced; DLSS, FSR or XeSS (or RR) reconstruct the rest. At 100%, DLAA / Native AA runs the upscaler as anti-aliasing; Native accumulates samples progressively.",
                |ui| {
                    egui::ComboBox::from_id_salt("quality mode")
                        .selected_text(quality_label(s))
                        .width(175.)
                        .show_ui(ui, |ui| {
                            for (scale, aa, label) in quality_modes(s) {
                                let current = (s.scale - scale).abs() < 0.005 && (scale < 1. || s.native_aa == aa);
                                if ui.selectable_label(current, label).clicked() {
                                    s.scale = scale;
                                    s.native_aa = aa;
                                }
                            }
                        });
                },
            );
            row(ui, "Upscaler", "Used below 100%, and as DLAA / Native AA at 100%. Ray Reconstruction does its own upscaling.", |ui| {
                ui.add_enabled_ui(!s.rr, |ui| {
                    egui::ComboBox::from_id_salt("upscaler")
                        .selected_text(s.upscaler.label())
                        .width(175.)
                        .show_ui(ui, |ui| {
                            for u in Upscaler::ALL {
                                let backend = caps.upscaler(u);
                                ui.add_enabled_ui(backend.is_none_or(|b| b.available) || s.upscaler == u, |ui| {
                                    ui.selectable_value(&mut s.upscaler, u, u.label())
                                        .on_disabled_hover_text(format!("Unavailable: {}", backend.map_or("", |b| b.reason.as_str())));
                                });
                            }
                        });
                });
            });
            if !s.rr && s.upscaler == Upscaler::Dlss {
                row(
                    ui,
                    "DLSS preset",
                    "Default lets NVIDIA pick: K for DLAA, Quality and Balanced; M for Performance; L for Ultra Performance.",
                    |ui| {
                        let label = DLSS_PRESETS.iter().find(|(id, _)| *id == s.dlss_preset).map_or("NVIDIA default", |(_, l)| *l);
                        egui::ComboBox::from_id_salt("dlss preset").selected_text(label).show_ui(ui, |ui| {
                            for (id, label) in DLSS_PRESETS {
                                ui.selectable_value(&mut s.dlss_preset, id, label);
                            }
                        });
                    },
                );
            }
            row(ui, "Native denoiser (D)", "SVGF-style denoiser, used when Ray Reconstruction is off. Off shows raw samples.", |ui| {
                ui.checkbox(&mut s.denoise, "");
            });
            row(
                ui,
                "Pixel filter",
                "How the progressive path spreads each pixel's samples. Box is the old sharp, slightly jagged look; Blackman-Harris and Gaussian give smoother edges. Ray Reconstruction and the temporal upscalers use their own jitter instead.",
                |ui| {
                    ui.add_enabled_ui(!s.rr, |ui| {
                        egui::ComboBox::from_id_salt("pixel_filter")
                            .selected_text(
                                *scene::PIXEL_FILTERS
                                    .get(s.pixel_filter as usize)
                                    .unwrap_or(&"?"),
                            )
                            .show_ui(ui, |ui| {
                                for (i, name) in scene::PIXEL_FILTERS.iter().enumerate() {
                                    ui.selectable_value(&mut s.pixel_filter, i as u32, *name);
                                }
                            });
                    });
                },
            );
            if advanced || (s.filter_width - 1.5).abs() > 1e-3 {
                row(
                    ui,
                    "Filter width",
                    "Full width of the pixel filter: wider is softer and hides more aliasing. 1.5 pixels matches Cycles.",
                    |ui| {
                        ui.add_enabled_ui(!s.rr, |ui| {
                            ui.add(crate::slider(&mut s.filter_width, 0.5..=4.).suffix(" px"));
                        });
                    },
                );
            }
            if advanced || !RESOLUTIONS.iter().any(|(v, _)| (v - s.scale).abs() < 0.005) {
                row(ui, "Render scale", "Any render scale from 25% to 100%.", |ui| {
                    ui.add(
                        crate::slider(&mut s.scale, 0.25..=1.)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.))
                            .custom_parser(|t| t.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.)),
                    );
                });
            }
        });
        note(
            ui,
            &format!(
                "Render {} × {} · output {} × {}",
                g.rw, g.rh, g.width, g.height
            ),
        );
        if !s.rr && !g.upscale_status.is_empty() {
            note(ui, &g.upscale_status);
        }
        section(ui, "Sampling");
        grid("sampling").show(ui, |ui| {
            row(
                ui,
                "Stop after",
                "Rendering pauses at this many samples per pixel.",
                |ui| {
                    ui.add(
                        crate::slider(&mut s.max_samples, 1..=1_000_000)
                            .logarithmic(true)
                            .suffix(" spp"),
                    );
                },
            );
            row(
                ui,
                "Samples per frame",
                "More is smoother per frame but slower to respond.",
                |ui| {
                    ui.add(crate::slider(&mut s.spp, 1..=64).logarithmic(true).suffix(" spp"));
                },
            );
            row(
                ui,
                "Light bounces",
                "How many times a path may bounce. More brightens deep corners and glass.",
                |ui| {
                    ui.add(crate::slider(&mut s.bounces, 1..=32));
                },
            );
            row(
                ui,
                "Integrator",
                "Path tracing follows light backwards from the camera. Bidirectional path tracing (BDPT) also traces paths from the lights and joins the two: it finds light that squeezes through gaps (a door ajar) and caustics seen directly. Photon mapping scatters photons from the lights and gathers them where camera paths land: caustics under glass converge fast, at the cost of a slight blur that fades as samples grow. BDPT and photon mapping ignore volumes and portals and use up to 10 bounces.",
                |ui| {
                    egui::ComboBox::from_id_salt("integrator")
                        .selected_text(INTEGRATORS[s.integrator.min(2) as usize])
                        .show_ui(ui, |ui| {
                            for (i, name) in INTEGRATORS.iter().enumerate() {
                                ui.selectable_value(&mut s.integrator, i as u32, *name);
                            }
                        });
                },
            );
            if s.integrator == 2 || s.battle {
                row(ui, "Photons per frame", "Photons traced from the lights each frame.", |ui| {
                    ui.add(crate::slider(&mut s.photons, 1 << 12..=1 << 20).logarithmic(true));
                });
                row(ui, "Gather radius", "The starting radius photons are gathered in, as a share of the scene's size. It shrinks as samples grow; larger is smoother but blurrier at first.", |ui| {
                    ui.add(crate::slider(&mut s.photon_radius, 0.0005..=0.2).logarithmic(true));
                });
            }
            row(
                ui,
                "Battle mode",
                "Splits the image into four quadrants rendered at equal GPU time: path tracing, path tracing with ReSTIR DI, BDPT and photon mapping. Raw samples only (no denoiser, Ray Reconstruction or upscaler). Open the convergence graph with a reference to compare their error over time.",
                |ui| {
                    ui.checkbox(&mut s.battle, "");
                },
            );

            row(
                ui,
                "Texture filtering",
                "Anisotropic filtering keeps textures sharp on floors and walls seen at grazing angles: up to this many taps along the stretched footprint. 1× is plain trilinear mipmapping, which blurs them.",
                |ui| {
                    egui::ComboBox::from_id_salt("anisotropy")
                        .selected_text(format!("{}×", s.anisotropy))
                        .show_ui(ui, |ui| {
                            for n in [1, 2, 4, 8, 16] {
                                ui.selectable_value(&mut s.anisotropy, n, format!("{n}×"));
                            }
                        });
                },
            );
        });
        more(ui, "render more", advanced, |ui| {
            section(ui, "Advanced");
            grid("render advanced").show(ui, |ui| {
                if !s.rr && s.denoise {
                    row(ui, "Denoise strength", "How strongly the native denoiser smooths.", |ui| {
                        ui.add(crate::slider(&mut s.denoise_strength, 0. ..=1.));
                    });
                    row(ui, "Reuse temporal history", "The native denoiser blends in earlier frames.", |ui| {
                        ui.checkbox(&mut s.temporal, "");
                    });
                }
                row(ui, "Sample pattern", "Low-discrepancy (Owen-scrambled Sobol) converges faster. Ray Reconstruction always uses random samples.", |ui| {
                    egui::ComboBox::from_id_salt("pattern")
                        .selected_text(if s.sobol { "Low-discrepancy" } else { "Random" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.sobol, true, "Low-discrepancy");
                            ui.selectable_value(&mut s.sobol, false, "Random");
                        });
                });
                row(ui, "Suppress fireflies", "Clamp single very bright samples. Biased, but removes white specks.", |ui| {
                    let mut on = s.firefly > 0.;
                    if ui.checkbox(&mut on, "").changed() {
                        s.firefly = if on { 10. } else { 0. };
                    }
                    if on {
                        ui.add(crate::slider(&mut s.firefly, 0.1..=100.).logarithmic(true).suffix("× max"));
                    }
                });
                row(ui, "Light sampling", "MIS combines light sampling (NEE) and BSDF sampling. The single strategies show what each one is good at: NEE for small lights and rough surfaces, BSDF for large lights and mirrors.", |ui| {
                    for (i, label) in ["MIS", "NEE only", "BSDF only"].iter().enumerate() {
                        ui.selectable_value(&mut s.mis_mode, i as u32, *label);
                    }
                });
                row(ui, "ReSTIR DI", "First-hit light sampling through per-pixel reservoirs that reuse good light samples across neighbouring pixels and frames. Unbiased; it pays off with many lights at a few samples per pixel.", |ui| {
                    ui.checkbox(&mut s.restir, "");
                });
                if s.restir {
                    row(ui, "Candidates", "Fresh light samples per pixel and frame.", |ui| {
                        ui.add(crate::slider(&mut s.restir_candidates, 1..=32));
                    });
                    row(ui, "Neighbours", "Reservoirs of nearby pixels merged each frame (0 = none).", |ui| {
                        ui.add(crate::slider(&mut s.restir_neighbors, 0..=8));
                    });
                    row(ui, "History", "How much last frame's reservoir may count against a fresh one (0 = no temporal reuse).", |ui| {
                        ui.add(crate::slider(&mut s.restir_history, 0..=64));
                    });
                }
                row(ui, "Glass casts shadows", "Off lets light through glass for shadow rays, a common shortcut.", |ui| {
                    ui.checkbox(&mut s.glass_shadow, "");
                });
                row(ui, "VSync", "Wait for the display between frames.", |ui| {
                    ui.checkbox(&mut s.vsync, "");
                });
            });
        });
    }
    /// The Quality dropdown (UI plan 5.1): bundles of the reconstruction fields, the current
    /// one derived from the settings (never stored). RR rows are disabled with the reason
    /// when RR is not available here.
    fn quality_combo(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let caps = g.capabilities();
        let rr_ok = caps.rr.available;
        let upscaler = [Upscaler::Dlss, Upscaler::Xess, Upscaler::Fsr]
            .into_iter()
            .find(|u| caps.upscaler(*u).is_some_and(|b| b.available))
            .unwrap_or(Upscaler::Bicubic);
        let rr_reason = caps.rr.reason;
        let current = Bundle::of(&self.settings);
        let mut chosen = None;
        egui::ComboBox::from_id_salt("quality")
            .selected_text(current.map_or("Custom quality", |b| b.label()))
            .width(120.)
            .show_ui(ui, |ui| {
                for b in Bundle::ALL {
                    if b == Bundle::Native {
                        ui.separator();
                    }
                    let usable = rr_ok || !b.rr();
                    let text = format!("{}  ·  {}", b.label(), b.about(upscaler));
                    let r = ui
                        .add_enabled(usable, egui::SelectableLabel::new(current == Some(b), text));
                    let r = r.on_disabled_hover_text(format!(
                        "Needs DLSS Ray Reconstruction: {rr_reason}"
                    ));
                    if r.clicked() {
                        chosen = Some(b);
                    }
                }
                if current.is_none() {
                    ui.separator();
                    note(ui, "Custom: the Render tab's settings match no bundle.");
                }
            })
            .response
            .on_hover_text(
                "Quality: denoiser and render resolution in one choice (details in the Render tab)",
            );
        if let Some(b) = chosen {
            b.apply(&mut self.settings, upscaler);
            self.ui.toast(format!("Quality: {}", b.label()));
        }
    }
    fn look_tab(&mut self, ui: &mut egui::Ui) {
        let hdr_text = hdr_line(self);
        let g = self.gpu.as_ref().unwrap();
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        section(ui, "Image");
        grid("look").show(ui, |ui| {
            row(ui, "View", "What the viewport shows (keys 1–8).", |ui| {
                egui::ComboBox::from_id_salt("view")
                    .selected_text(VIEWS[s.view as usize])
                    .show_ui(ui, |ui| {
                        for (i, n) in VIEWS.iter().enumerate() {
                            ui.selectable_value(&mut s.view, i as u32, format!("{n}  ({})", i + 1));
                        }
                    });
            });
            row(
                ui,
                "Exposure",
                "Brightness in photographic stops; +1 EV doubles it.",
                |ui| {
                    let mut ev = if s.exposure > 0. {
                        s.exposure.log2()
                    } else {
                        -6.
                    };
                    let r = ui.add(
                        crate::slider(&mut ev, -6. ..=3.32)
                            .suffix(" EV")
                            .fixed_decimals(1),
                    );
                    // EV is a local, so the double-click reset is done here.
                    if crate::slider_double_clicked(ui, &r) {
                        s.exposure = self.reset_baseline.exposure;
                    } else if r.changed() {
                        s.exposure = 2f32.powf(ev).clamp(0., 10.);
                    }
                },
            );
            row(
                ui,
                "Tone map",
                "How bright values are compressed. In HDR the curve keeps its SDR look and its highlights expand into the panel's headroom; Linear uses a plain HDR roll-off.",
                |ui| {
                    egui::ComboBox::from_id_salt("tone map")
                        .selected_text(TONE_MAPS[s.tonemap as usize])
                        .show_ui(ui, |ui| {
                            for (i, n) in TONE_MAPS.iter().enumerate() {
                                ui.selectable_value(&mut s.tonemap, i as u32, *n);
                            }
                        });
                },
            );
        });
        more(ui, "look more", advanced, |ui| {
            grid("look advanced").show(ui, |ui| {
                row(ui, "Gamma", "Display gamma for the SDR image.", |ui| {
                    ui.add(crate::slider(&mut s.gamma, 0.1..=4.));
                });
                row(ui, "Direct light", "Relight: scale light that reached the surface straight from a light. Approximate on denoised output.", |ui| {
                    ui.add(crate::slider(&mut s.direct, 0. ..=4.).suffix("×"));
                });
                row(ui, "Indirect light", "Relight: scale light that bounced first. Approximate on denoised output.", |ui| {
                    ui.add(crate::slider(&mut s.indirect, 0. ..=4.).suffix("×"));
                });
            });
        });
        section(ui, "Film");
        grid("film").show(ui, |ui| {
            row(
                ui,
                "Halation",
                "Red-orange glow around highlights, as light scatters back through film.",
                |ui| {
                    ui.add(crate::slider(&mut s.halation, 0. ..=1.));
                },
            );
            row(
                ui,
                "Glare",
                "Diffraction starbursts around highlights: the aperture's shape (Lens: aperture blades) diffracts light into spikes, coloured because red spreads wider than blue. An FFT convolution of the highlights.",
                |ui| {
                    ui.add(crate::slider(&mut s.glare, 0. ..=4.).logarithmic(true).smallest_positive(0.01));
                },
            );
            if s.glare > 0. {
                row(ui, "Glare size", "How far the spikes and rings reach.", |ui| {
                    ui.add(crate::slider(&mut s.glare_size, 0. ..=1.));
                });
                row(ui, "Glare threshold", "Only light brighter than this (luminance, before exposure) glares.", |ui| {
                    ui.add(crate::slider(&mut s.glare_threshold, 0. ..=100.).logarithmic(true).smallest_positive(0.01));
                });
            }
            row(
                ui,
                "Film stock",
                "Sets grain, grain size and halation to resemble a film stock (rough matches by eye: the grain is monochrome and the colour response is not modelled).",
                |ui| {
                    let current = FILM_STOCKS
                        .iter()
                        .find(|(_, g, z, h)| (s.grain - g).abs() < 1e-4 && s.grain_size == *z && (s.halation - h).abs() < 1e-4)
                        .map_or("Custom", |(n, ..)| *n);
                    egui::ComboBox::from_id_salt("film stock")
                        .selected_text(current)
                        .width(170.)
                        .show_ui(ui, |ui| {
                            for (name, grain, size, halation) in FILM_STOCKS {
                                if ui.selectable_label(current == *name, *name).clicked() {
                                    s.grain = *grain;
                                    s.grain_size = *size;
                                    s.halation = *halation;
                                }
                            }
                        });
                },
            );
            row(
                ui,
                "Grain",
                "Film grain, strongest in the midtones.",
                |ui| {
                    ui.add(crate::slider(&mut s.grain, 0. ..=1.));
                    egui::ComboBox::from_id_salt("grain size")
                        .selected_text(["Fine", "Medium", "Coarse"][s.grain_size.min(2) as usize])
                        .width(80.)
                        .show_ui(ui, |ui| {
                            for (i, n) in ["Fine", "Medium", "Coarse"].iter().enumerate() {
                                ui.selectable_value(&mut s.grain_size, i as u32, *n);
                            }
                        });
                },
            );
            row(ui, "Vignette", "Darkens toward the corners.", |ui| {
                ui.add(crate::slider(&mut s.vignette, 0. ..=1.));
            });
        });
        section(ui, "Grade");
        ui.horizontal(|ui| {
            crate::look::color_wheel(ui, "Lift", &mut s.lift, 0., 0.15);
            crate::look::color_wheel(ui, "Gamma", &mut s.grade_gamma, 1., 0.5);
            crate::look::color_wheel(ui, "Gain", &mut s.gain, 1., 0.5);
        });
        note(
            ui,
            "Drag a wheel to tint, the slider for the level; double-click resets.",
        );
        grid("lut").show(ui, |ui| {
            row(
                ui,
                "LUT",
                "A .cube 3D LUT applied after the grade, on the display-encoded image.",
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut s.lut_path)
                            .hint_text("path to .cube")
                            .desired_width(170.),
                    );
                    if ui.small_button("Browse…").clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter("3D LUT", &["cube"])
                            .pick_file()
                    {
                        s.lut_path = p.to_string_lossy().into();
                    }
                    if !s.lut_path.is_empty() && ui.small_button("Clear").clicked() {
                        s.lut_path.clear();
                    }
                },
            );
            if !s.lut_path.is_empty() {
                row(
                    ui,
                    "LUT strength",
                    "Blend between the graded image and the LUT result.",
                    |ui| {
                        ui.add(crate::slider(&mut s.lut_strength, 0. ..=1.));
                    },
                );
            }
        });
        section(ui, "Display");
        grid("display").show(ui, |ui| {
            row(ui, "Output", "Auto shows real HDR when Windows HDR is on for this monitor, using Windows' calibration and SDR content brightness.", |ui| {
                let outputs = ["Auto (follow Windows)", "SDR", "HDR (forced)"];
                egui::ComboBox::from_id_salt("output").selected_text(outputs[s.hdr_mode as usize]).show_ui(ui, |ui| {
                    for (i, n) in outputs.iter().enumerate() {
                        ui.selectable_value(&mut s.hdr_mode, i as u32, *n);
                    }
                });
            });
            if g.hdr_active || s.hdr_mode != 1 {
                row(ui, "Swapchain", "scRGB: 16-bit float, Windows converts it for the panel. HDR10: 10-bit PQ in BT.2020, sent to the panel as it is (for comparison).", |ui| {
                    ui.selectable_value(&mut s.hdr10, false, "scRGB (FP16)");
                    ui.selectable_value(&mut s.hdr10, true, "HDR10 (PQ)");
                });
            }
            if g.hdr_active {
                row(ui, "Peak brightness", "From Windows HDR Calibration, or set by hand.", |ui| {
                    let mut windows = s.hdr_peak == 0.;
                    if ui.checkbox(&mut windows, "Windows").changed() {
                        s.hdr_peak = if windows { 0. } else { g.hdr_levels[1].clamp(80., 10000.) };
                    }
                    if !windows {
                        ui.add(crate::slider(&mut s.hdr_peak, 80. ..=4000.).logarithmic(true).suffix(" nits"));
                    }
                });
                row(ui, "Paper white", "Brightness of white; Windows' SDR content brightness by default.", |ui| {
                    let mut windows = s.hdr_white == 0.;
                    if ui.checkbox(&mut windows, "Windows").changed() {
                        s.hdr_white = if windows { 0. } else { g.hdr_levels[0].clamp(40., 1000.) };
                    }
                    if !windows {
                        ui.add(crate::slider(&mut s.hdr_white, 40. ..=1000.).logarithmic(true).suffix(" nits"));
                    }
                });
            }
        });
        note(ui, &hdr_text);
        if g.hdr_active {
            note(
                ui,
                "In HDR, midtones look as in SDR and the tone map's highlights expand up to the panel peak. Linear shows true scene brightness with a roll-off near the peak.",
            );
        }
        section(ui, "Tools");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.ui.zebras, "Zebras")
                .on_hover_text("Stripes where the displayed image clips");
            ui.checkbox(&mut self.ui.show_scopes, "Scopes")
                .on_hover_text("Histogram, luma waveform and vectorscope");
            ui.checkbox(&mut self.ui.meter, "Light meter")
                .on_hover_text("Luminance and illuminance under the cursor");
            ui.checkbox(&mut self.ui.show_graph, "Convergence graph")
                .on_hover_text("Error against a reference image as samples grow");
            ui.checkbox(&mut self.ui.paths.open, "Path visualizer (P)")
                .on_hover_text("Click a pixel to see its light paths in 3D");
        });
        section(ui, "Light path expressions");
        lpe_section(ui, s, &mut self.ui.lpe_edit);
        section(ui, "Compare");
        ui.horizontal(|ui| {
            if ui
                .button("Freeze reference (C)")
                .on_hover_text("Keep the current image to compare against")
                .clicked()
            {
                self.actions.reference = true;
            }
            ui.add_enabled(
                g.reference_valid,
                egui::Checkbox::new(&mut s.compare, "Show split"),
            );
        });
        if s.compare {
            grid("compare").show(ui, |ui| {
                row(
                    ui,
                    "Split",
                    "Left of the line shows the frozen reference.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut s.split, 0. ..=1.)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.)),
                        );
                    },
                );
            });
        }
    }
    fn export_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        use crate::video::ImageFormat;
        section(ui, "Save the current image");
        ui.horizontal_wrapped(|ui| {
            for format in ImageFormat::ALL {
                if format.needs_hdr() && !g.hdr_active {
                    continue;
                }
                let label = if format == ImageFormat::Png {
                    "PNG (F12)".to_string()
                } else {
                    format.label().to_string()
                };
                let file = format.file("caustica");
                if ui
                    .button(label)
                    .on_hover_text(format!("{}: {}", file.display(), format.description()))
                    .clicked()
                {
                    self.actions.save_image = Some(format);
                }
            }
        });
        note(
            ui,
            "Files are written to the working folder and overwrite earlier ones.",
        );
        section(ui, "High-resolution render");
        grid("export").show(ui, |ui| {
            row(
                ui,
                "Size",
                "Multiple of the window size, up to 8192 pixels on the long side.",
                |ui| {
                    ui.add(crate::slider(&mut self.ui.export_scale, 1..=6).suffix("×"));
                },
            );
            row(ui, "Samples", "Samples per pixel for the export.", |ui| {
                ui.add(
                    crate::slider(&mut self.ui.export_samples, 16..=65536)
                        .logarithmic(true)
                        .suffix(" spp"),
                );
            });
            row(
                ui,
                "Format",
                "The same formats as Save the current image.",
                |ui| {
                    egui::ComboBox::from_id_salt("export_format")
                        .selected_text(self.ui.export_format.label())
                        .show_ui(ui, |ui| {
                            for format in ImageFormat::ALL {
                                ui.add_enabled_ui(!format.needs_hdr() || g.hdr_active, |ui| {
                                    ui.selectable_value(
                                        &mut self.ui.export_format,
                                        format,
                                        format.label(),
                                    )
                                    .on_hover_text(format.description())
                                    .on_disabled_hover_text("Needs HDR output");
                                });
                            }
                        });
                },
            );
        });
        // HDR output may have been turned off since the format was picked.
        if self.ui.export_format.needs_hdr() && !g.hdr_active {
            self.ui.export_format = ImageFormat::Png;
        }
        grid("export_quality").show(ui, |ui| {
            row(
                ui,
                "JPEG quality",
                "Used by every JPEG save: 100 is the largest file and the fewest artefacts.",
                |ui| {
                    ui.add(crate::slider(&mut self.ui.jpeg_quality, 50..=100));
                },
            );
        });
        let factor = (self.ui.export_scale as f32).min(8192. / g.width.max(g.height) as f32);
        let name = self.ui.export_format.file("caustica-high-res");
        if ui.button("Render high-resolution image").clicked() {
            self.actions.high_res = true;
        }
        note(
            ui,
            &format!(
                "{} ({}), {} × {}, traced fresh at 100% resolution. The window stays responsive, with progress and Cancel.",
                name.display(),
                self.ui.export_format.description().trim_end_matches('.'),
                (g.width as f32 * factor).round(),
                (g.height as f32 * factor).round()
            ),
        );
        section(ui, "Video");
        let o = &self.ui.video;
        let [w, h] = o.size();
        note(
            ui,
            &format!(
                "An orbit around the target: {} at {w}×{h}, {} frames at {} fps, {} spp each.",
                o.format.label(),
                o.frames,
                o.fps,
                o.spp
            ),
        );
        if ui.button("Export video…").clicked() {
            self.open_video_dialog();
        }
        section(ui, "Scene file");
        ui.horizontal(|ui| {
            if ui.button("Save settings.json").clicked() {
                self.actions.save = Some("settings.json".into());
            }
            if ui.button("Save as…").clicked() {
                self.actions.save = settings_dialog(true);
            }
            if ui.button("Load…").clicked() {
                self.actions.load = settings_dialog(false);
            }
        });
    }
}
/// Picks a settings JSON to save (`save`) or load, starting in the working directory where the
/// quick buttons keep settings.json.
fn settings_dialog(save: bool) -> Option<std::path::PathBuf> {
    let d = rfd::FileDialog::new()
        .add_filter("Settings", &["json"])
        .set_directory(std::env::current_dir().unwrap_or_default());
    if save {
        d.set_file_name("settings.json").save_file()
    } else {
        d.pick_file()
    }
}
/// Global fog rows of the Stage grid (track 12).
fn fog_ui(ui: &mut egui::Ui, s: &mut Settings) {
    let f = &mut s.fog;
    row(
        ui,
        "Fog",
        "A homogeneous medium from the floor up to a height: god rays and visible light beams. Extinction per unit length; 0 is off.",
        |ui| {
            ui.add(crate::slider(&mut f.density, 0. ..=10.).logarithmic(true));
        },
    );
    if f.density <= 0. {
        return;
    }
    row(
        ui,
        "Fog colour",
        "The share of light each scattering keeps, per channel.",
        |ui| {
            ui.color_edit_button_rgb(&mut f.color);
        },
    );
    row(
        ui,
        "Fog phase",
        "Henyey-Greenstein g: positive scatters forward, which makes beams glow toward the camera.",
        |ui| {
            ui.add(crate::slider(&mut f.phase, -0.95..=0.95));
        },
    );
    row(
        ui,
        "Fog height",
        "The fog's top, in world units (the floor is at −1).",
        |ui| {
            ui.add(crate::slider(&mut f.height, -1. ..=50.));
        },
    );
    row(
        ui,
        "Fog extent",
        "How far the fog reaches from the centre on x and z.",
        |ui| {
            ui.add(crate::slider(&mut f.extent, 0.1..=100.).logarithmic(true));
        },
    );
}
/// Physical sky rows of the Stage grid (track 9).
fn physical_sky_ui(ui: &mut egui::Ui, s: &mut Settings) {
    row(
        ui,
        "Physical sky",
        "A sun and an Earth sky computed from the time, date and latitude. Replaces the sky light and the HDRI while on.",
        |ui| {
            ui.checkbox(&mut s.physical_sky.on, "");
        },
    );
    if !s.physical_sky.on {
        return;
    }
    let p = &mut s.physical_sky;
    row(
        ui,
        "Time of day",
        "Solar time: 12:00 is when the sun is highest.",
        |ui| {
            ui.add(
                crate::slider(&mut p.time, 0. ..=24.).custom_formatter(|v, _| {
                    let minutes = (v * 60.).round() as u32;
                    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
                }),
            );
        },
    );
    row(
        ui,
        "Date",
        "Day of the year: the sun is highest at the June solstice in the north.",
        |ui| {
            ui.add(
                crate::slider(&mut p.day, 1. ..=366.).custom_formatter(|v, _| day_name(v as f32)),
            );
        },
    );
    row(ui, "Latitude", "Degrees north (negative: south).", |ui| {
        ui.add(crate::slider(&mut p.latitude, -90. ..=90.).suffix("°"));
    });
    row(
        ui,
        "North",
        "Turns the sky (and the sun's path) about the vertical axis. 0: north is −z.",
        |ui| {
            ui.add(crate::slider(&mut p.north, -180. ..=180.).suffix("°"));
        },
    );
    row(
        ui,
        "Turbidity",
        "Haze: 1 is pure air, 2 a clear day, 10 very hazy.",
        |ui| {
            ui.add(crate::slider(&mut p.turbidity, 1. ..=10.));
        },
    );
    row(
        ui,
        "Sun intensity",
        "The sun above the atmosphere lights a white surface facing it to this; the sky follows.",
        |ui| {
            ui.add(
                crate::slider(&mut p.intensity, 0. ..=1000.)
                    .logarithmic(true)
                    .suffix("×"),
            );
        },
    );
    row(
        ui,
        "Sun size",
        "Angular diameter. The real sun is 0.53°; larger gives softer shadows.",
        |ui| {
            ui.add(
                crate::slider(&mut p.sun_size, 0.05..=20.)
                    .logarithmic(true)
                    .suffix("°"),
            );
        },
    );
    row(
        ui,
        "Ground",
        "Albedo of the ground below the horizon.",
        |ui| {
            ui.add(crate::slider(&mut p.ground, 0. ..=1.));
        },
    );
    row(
        ui,
        "Seen by camera",
        "Off: the camera sees the background colour while the sky still lights.",
        |ui| {
            ui.checkbox(&mut p.visible, "");
        },
    );
    row(
        ui,
        "Day cycle",
        "Keys the time of day from 06:00 at the start of the timeline to 20:00 at its end (linear).",
        |ui| {
            if ui.small_button("Key a day").clicked() {
                let a = &mut s.animation;
                let path = "physical_sky/time";
                let length = a.length;
                a.key(path, 0., 6.);
                a.key(path, length, 20.);
                a.set_interp(&[path.into()], 0., crate::anim::Interp::Linear);
            }
        },
    );
}
/// "21 Jun" for a day of the year (a non-leap year).
fn day_name(day: f32) -> String {
    const MONTHS: [(&str, u32); 12] = [
        ("Jan", 31),
        ("Feb", 28),
        ("Mar", 31),
        ("Apr", 30),
        ("May", 31),
        ("Jun", 30),
        ("Jul", 31),
        ("Aug", 31),
        ("Sep", 30),
        ("Oct", 31),
        ("Nov", 30),
        ("Dec", 31),
    ];
    let mut d = (day.round() as u32).clamp(1, 365);
    for (name, len) in MONTHS {
        if d <= len {
            return format!("{d} {name}");
        }
        d -= len;
    }
    "31 Dec".into()
}
fn stage_ui(ui: &mut egui::Ui, s: &mut Settings, advanced: bool) {
    section(ui, "Stage");
    grid("stage").show(ui, |ui| {
        row(
            ui,
            "Cornell box",
            "The closed box with colored side walls.",
            |ui| {
                ui.checkbox(&mut s.cornell_room, "");
            },
        );
        row(
            ui,
            "Ground size",
            "Extent of the floor in scene units.",
            |ui| {
                ui.add(
                    crate::slider(&mut s.floor_extent, 0.5..=40.)
                        .logarithmic(true)
                        .suffix(" units"),
                );
            },
        );
        row(
            ui,
            "Sky light",
            "Light from the surroundings. 0 is a black void.",
            |ui| {
                ui.add(crate::slider(&mut s.sky, 0. ..=4.).suffix("×"));
            },
        );
        physical_sky_ui(ui, s);
        fog_ui(ui, s);
        row(
            ui,
            "HDRI",
            "An equirectangular .hdr or .exr that lights the scene instead of the sky light, importance-sampled.",
            |ui| {
                let name = std::path::Path::new(&s.hdri)
                    .file_name()
                    .map_or("None".into(), |n| n.to_string_lossy().to_string());
                ui.label(name).on_hover_text(&s.hdri);
                if ui.small_button("Load…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("HDRI", &["hdr", "exr"])
                        .set_directory(
                            crate::scenes::folder()
                                .map(|f| f.join("hdri"))
                                .filter(|f| f.is_dir())
                                .or_else(crate::scenes::folder)
                                .unwrap_or_default(),
                        )
                        .pick_file()
                {
                    s.hdri = crate::mesh::store(&path);
                }
                if !s.hdri.is_empty() && ui.small_button("Clear").clicked() {
                    s.hdri.clear();
                }
            },
        );
        if !s.hdri.is_empty() && !s.physical_sky.on {
            row(ui, "HDRI strength", "Multiplier on the HDRI's light.", |ui| {
                ui.add(
                    crate::slider(&mut s.hdri_strength, 0. ..=1000.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            row(ui, "HDRI rotation", "Turns the HDRI about the vertical axis.", |ui| {
                ui.add(crate::slider(&mut s.hdri_rotation, -180. ..=180.).suffix("°"));
            });
            row(
                ui,
                "Seen by camera",
                "Off: the camera sees the background colour while the HDRI still lights the scene.",
                |ui| {
                    ui.checkbox(&mut s.hdri_visible, "");
                },
            );
        }
        row(ui, "Left wall", "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.left);
        });
        row(ui, "Right wall", "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.right);
        });
        let base = if s.cornell_room {
            "Back wall, ceiling & floor"
        } else {
            "Ground"
        };
        row(ui, base, "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.base);
        });
        row(ui, "Background", "Color seen where rays escape.", |ui| {
            ui.color_edit_button_rgb(&mut s.background);
        });
    });
    more(ui, "stage more", advanced, |ui| {
        ui.collapsing("Wall material", |ui| {
            editor::material_ui(ui, "walls", &mut s.walls, advanced)
        });
        ui.collapsing("Floor material", |ui| {
            editor::material_ui(ui, "floor", &mut s.floor, advanced)
        });
    });
}
/// Returns true when "Reset camera" was pressed.
fn camera_ui(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    section(ui, "Camera");
    grid("camera").show(ui, |ui| {
        row(ui, "Auto orbit", "Circle the scene (A).", |ui| {
            ui.checkbox(&mut s.rotate, "");
        });
        row(
            ui,
            "Orbit speed",
            "Degrees per second; negative turns the other way.",
            |ui| {
                ui.add(crate::slider(&mut s.rotate_speed, -60. ..=60.).suffix("°/s"));
            },
        );
        row(ui, "Field of view", "Vertical angle of view.", |ui| {
            ui.add(crate::slider(&mut s.fov, 10. ..=100.).suffix("°"));
        });
        row(
            ui,
            "Distance",
            "From the camera to its target, in scene units.",
            |ui| {
                ui.add(
                    crate::slider(&mut s.distance, 0.5..=200.)
                        .logarithmic(true)
                        .suffix(" units"),
                );
            },
        );
    });
    lens_ui(ui, s);
    note(
        ui,
        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom · Ctrl+click: focus",
    );
    ui.button("Reset camera")
        .on_hover_text("Back to the scene's own view")
        .clicked()
}
/// Camera model, lens system and aperture (Scene → Camera).
fn lens_ui(ui: &mut egui::Ui, s: &mut Settings) {
    section(ui, "Lens");
    grid("lens").show(ui, |ui| {
        row(ui, "Camera model", "Orthographic, panorama and little planet use the native denoiser: RR, the temporal upscalers and the gizmo need a perspective camera.", |ui| {
            egui::ComboBox::from_id_salt("projection").selected_text(PROJECTIONS[s.projection as usize]).show_ui(ui, |ui| {
                for (i, n) in PROJECTIONS.iter().enumerate() {
                    ui.selectable_value(&mut s.projection, i as u32, *n);
                }
            });
        });
        if s.projection == 4 {
            row(ui, "Focal-plane tilt", "Tilts the plane of sharp focus about the horizontal axis: the miniature look. Needs an aperture.", |ui| {
                ui.add(crate::slider(&mut s.tilt, -60. ..=60.).suffix("°"));
            });
        }
        if s.projection == 0 {
            let name = |i: u32| if i == 0 { "Thin lens".to_string() } else { let l = &crate::lens::LENSES[i as usize - 1]; let n = crate::lens::INFO[i as usize - 1]; format!("{} f/{:.1}", l.name, n.f_number) };
            row(ui, "Lens", "Thin lens: an ideal lens with the aperture below. The others trace each ray through a real lens prescription (track 16): distortion, vignetting, chromatic aberration and bokeh come from the glass. Real lenses use the native denoiser.", |ui| {
                egui::ComboBox::from_id_salt("lens_system").selected_text(name(s.lens_system)).show_ui(ui, |ui| {
                    for i in 0..=crate::lens::LENSES.len() as u32 {
                        ui.selectable_value(&mut s.lens_system, i, name(i));
                    }
                });
            });
        }
        if s.real_lens() {
            let open = crate::lens::INFO[s.lens_system as usize - 1].f_number;
            row(ui, "f-number", "Stops the lens down: higher is a smaller aperture, deeper focus and less aberration. The image keeps its brightness.", |ui| {
                let mut n = if s.f_number == 0. { open } else { s.f_number.max(open) };
                if ui.add(crate::slider(&mut n, open..=32.).logarithmic(true).custom_formatter(|v, _| format!("f/{v:.1}"))).changed() {
                    s.f_number = if n <= open + 1e-3 { 0. } else { n };
                }
            });
            row(ui, "Focus distance", "Distance of the sharp plane from the front of the lens. Ctrl+click in the viewport focuses there.", |ui| {
                ui.add(crate::slider(&mut s.focus_distance, 0.05..=500.).logarithmic(true).suffix(" units"));
            });
            row(ui, "Scene scale", "Metres per scene unit. The lens is real size, so a small scale (a miniature) gets shallow focus.", |ui| {
                ui.add(crate::slider(&mut s.lens_scale, 0.001..=1000.).logarithmic(true).suffix(" m"));
            });
            row(ui, "Dispersion", "Chromatic aberration from the glass: 1 is real glass, 0 none, higher exaggerates it. Each sample traces one colour, so it adds some colour noise.", |ui| {
                ui.add(crate::slider(&mut s.dispersion, 0. ..=20.));
            });
            row(ui, "Aperture blades", "Shape of the aperture stop: round, or a polygon with this many blades.", |ui| {
                egui::ComboBox::from_id_salt("lens_blades")
                    .selected_text(if s.aperture_blades == 0 { "Round".to_string() } else { format!("{} blades", s.aperture_blades) })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.aperture_blades, 0, "Round");
                        for n in [3, 5, 6, 7, 8, 9] {
                            ui.selectable_value(&mut s.aperture_blades, n, format!("{n} blades"));
                        }
                    });
            });
            if s.aperture_blades > 0 {
                row(ui, "Blade rotation", "Turns the polygonal aperture.", |ui| {
                    ui.add(crate::slider(&mut s.blade_rotation, -180. ..=180.).suffix("°"));
                });
            }
        }
        if s.perspective() {
            row(ui, "Aperture", "Lens radius in scene units. 0 is a pinhole: everything sharp.", |ui| {
                ui.add(crate::slider(&mut s.aperture, 0. ..=2.).logarithmic(true).smallest_positive(0.001).suffix(" units"));
            });
            row(ui, "Focus distance", "Distance of the sharp plane along the view axis. Ctrl+click in the viewport focuses there.", |ui| {
                ui.add(crate::slider(&mut s.focus_distance, 0.05..=500.).logarithmic(true).suffix(" units"));
            });
            row(ui, "Aperture blades", "Shape of the bokeh: round, or a polygon with this many blades.", |ui| {
                egui::ComboBox::from_id_salt("blades")
                    .selected_text(if s.aperture_blades == 0 { "Round".to_string() } else { format!("{} blades", s.aperture_blades) })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.aperture_blades, 0, "Round");
                        for n in [3, 5, 6, 7, 8, 9] {
                            ui.selectable_value(&mut s.aperture_blades, n, format!("{n} blades"));
                        }
                    });
            });
            if s.aperture_blades > 0 {
                row(ui, "Blade rotation", "Turns the polygonal aperture.", |ui| {
                    ui.add(crate::slider(&mut s.blade_rotation, -180. ..=180.).suffix("°"));
                });
            }
            row(ui, "Cat-eye", "Optical vignetting: the lens barrel clips the aperture toward the frame edges, so bokeh turns cat-eye shaped and corners darken.", |ui| {
                ui.add(crate::slider(&mut s.cat_eye, 0. ..=1.));
            });
        }
    });
    if s.real_lens() {
        note(
            ui,
            "Real lenses render with the native denoiser; RR, temporal upscalers, click picking and the gizmo are off.",
        );
    } else if !s.perspective() {
        note(
            ui,
            "This camera model renders with the native denoiser; RR, temporal upscalers and the gizmo are off. A 360° panorama exports best at 2:1 (e.g. --width 4096 --height 2048).",
        );
    }
}
fn main_light_ui(ui: &mut egui::Ui, s: &mut Settings, advanced: bool) {
    section(ui, "Main light");
    grid("main light").show(ui, |ui| {
        row(
            ui,
            "On",
            "The ceiling light of the Cornell box, or the studio key light.",
            |ui| {
                ui.checkbox(&mut s.light_on, "");
            },
        );
        row(
            ui,
            "Show outline",
            "Draw the light's rectangle over the viewport, to see and grab it. The selected light always shows it.",
            |ui| {
                ui.checkbox(&mut s.light_outline, "");
            },
        );
        row(ui, "Intensity", "Brightness multiplier.", |ui| {
            ui.add(
                crate::slider(&mut s.light_intensity, 0. ..=2000.)
                    .logarithmic(true)
                    .suffix("×"),
            );
        });
        row(
            ui,
            "Color temperature",
            "Warm (low) to cool (high) white.",
            |ui| {
                ui.add(crate::slider(&mut s.light_temperature, 1000. ..=12000.).suffix(" K"));
            },
        );
        row(ui, "Color filter", "Tints the light, like a gel.", |ui| {
            ui.color_edit_button_rgb(&mut s.light_tint);
        });
        row(ui, "Size", "Bigger lights give softer shadows.", |ui| {
            ui.add(
                crate::slider(&mut s.light_size, 0.05..=40.)
                    .logarithmic(true)
                    .suffix(" units"),
            );
        });
        row(ui, "Position", "In scene units.", |ui| {
            for (p, axis) in s.light_position.iter_mut().zip(["x ", "y ", "z "]) {
                ui.add(
                    egui::DragValue::new(p)
                        .speed(0.025)
                        .range(-50. ..=50.)
                        .prefix(axis),
                );
            }
        });
    });
    more(ui, "main light more", advanced, |ui| {
        grid("main light advanced").show(ui, |ui| {
            row(
                ui,
                "Visible to camera",
                "Show the light's panel in the image.",
                |ui| {
                    ui.checkbox(&mut s.light_visible, "");
                },
            );
            row(ui, "Aspect", "Width relative to depth.", |ui| {
                ui.add(
                    crate::slider(&mut s.light_aspect, 0.1..=10.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
        });
    });
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
