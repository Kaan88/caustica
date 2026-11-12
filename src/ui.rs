//! The interface (UI plan phase 1): top bar, a right side panel with Scene / Render / Look /
//! Export tabs, a status bar and toasts. UI-only state lives in `UiState`, never in `Settings`,
//! because every `Settings` field outside the exclusion list restarts the render.
mod ai;
mod chrome;
pub(crate) mod icons;
mod look_tab;
mod overlay;
mod render_tab;
mod scene_tab;
pub(crate) mod skin;
mod stage;
mod windows;
use crate::scene::{
    self, DLSS_PRESETS, Denoiser, EV_BIAS_MAX, EXPOSURE_EV_MAX, EXPOSURE_EV_MIN, EXPOSURE_MAX,
    MeterMode, PRESETS, Settings, Upscaler,
};
use crate::{App, editor, gizmo};
use stage::*;
use std::time::{Duration, Instant};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 202, 216);
pub(crate) const WARN: egui::Color32 = egui::Color32::from_rgb(232, 176, 72);
/// The fill of what is selected or switched on: rail tools, outliner rows, pressed pins.
pub(crate) const SELECT: egui::Color32 = egui::Color32::from_rgb(43, 107, 122);
/// The bars' background (top bar, rail, tab strip, status bar) and the hairlines between areas.
pub(crate) const BAR: egui::Color32 = egui::Color32::from_rgb(32, 32, 32);
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
/// A menu that stays open while its widgets are used (egui 0.32+ closes menus on any click inside by
/// default). Its submenus inherit this; plain items close it with `ui.close()`.
pub(crate) fn sticky_menu<R>(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    skin::menu_button_sticky(ui, text, add).inner
}

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
    if s.rr() {
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
        (s.scale - scale).abs() < 0.005 && (*scale < 1. || s.native_aa == *aa || s.rr())
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
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Render,
    Look,
    Lights,
    /// Old prefs files say "Scene": the tab that was split into Lights, Object and Material.
    #[serde(alias = "Scene")]
    Object,
    Material,
    Export,
}
impl Tab {
    /// In the order of the tab strip.
    pub const ALL: [Tab; 6] = [
        Tab::Render,
        Tab::Look,
        Tab::Lights,
        Tab::Object,
        Tab::Material,
        Tab::Export,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Render => "Render",
            Tab::Look => "Look",
            Tab::Lights => "Lights",
            Tab::Object => "Object",
            Tab::Material => "Material",
            Tab::Export => "Export",
        }
    }
    /// Render, Look and Export don't depend on the selection: a pick leaves them open, and the
    /// outliner folds to one line above them.
    pub fn follows_selection(self) -> bool {
        matches!(self, Tab::Lights | Tab::Object | Tab::Material)
    }
    /// The tab after `sel` was picked while `self` was open: a light opens Lights, anything else
    /// Object (Material stays for an object or the stage, which have one). Render, Look and
    /// Export stay put unless `force` (a double-click, a palette entry).
    pub fn after_pick(self, sel: Selection, force: bool) -> Tab {
        if !force && !self.follows_selection() {
            return self;
        }
        match sel {
            Selection::MainLight | Selection::Flashlight | Selection::Light(_) => Tab::Lights,
            Selection::Object(_) | Selection::Stage if self == Tab::Material => Tab::Material,
            _ => Tab::Object,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    None,
    Stage,
    Camera,
    MainLight,
    /// Only its inspector: it rides with the camera, so nothing to move or key.
    Flashlight,
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
/// One undo step: the settings on the other side, and whether stepping there takes the camera
/// along. Whole-scene steps and Camera inspector edits do; every other edit leaves the view
/// where it is, so undoing a material never swings the camera.
struct UndoStep {
    settings: Settings,
    camera: bool,
}
type Camera = (f32, f32, f32, [f32; 3]);
/// The free camera: what orbiting, panning and zooming change (the field of view is an edit).
fn camera_of(s: &Settings) -> Camera {
    (s.yaw, s.pitch, s.distance, s.target)
}
fn set_camera(s: &mut Settings, c: Camera) {
    (s.yaw, s.pitch, s.distance, s.target) = c;
}
/// `a == b`, whatever their free cameras (`a` only lends its camera for the comparison).
/// Gives `base` the poses a simulation wrote to `s` (and the strings they pull). With objects
/// added or removed since, indices don't match: the difference is an edit anyway.
fn follow_poses(base: &mut Settings, s: &Settings, moving: &[usize]) {
    if base.objects.len() != s.objects.len() {
        return;
    }
    for &i in moving {
        let (Some(b), Some(o)) = (base.objects.get_mut(i), s.objects.get(i)) else {
            continue;
        };
        b.grounded = o.grounded;
        b.position = o.position;
        b.elevation = o.elevation;
        b.rotation = o.rotation;
        b.tilt = o.tilt;
        b.mesh.clone_from(&o.mesh);
    }
    crate::physics::update_strings(base);
}
fn same_but_camera(a: &mut Settings, b: &Settings) -> bool {
    let camera = camera_of(a);
    set_camera(a, camera_of(b));
    let same = *a == *b;
    set_camera(a, camera);
    same
}
/// A mesh import running on a worker thread.
pub struct MeshImport {
    pub name: String,
    result: std::sync::mpsc::Receiver<anyhow::Result<(Vec<scene::Object>, Vec<scene::AreaLight>)>>,
}
pub struct UiState {
    pub tab: Tab,
    pub advanced: bool,
    /// Tab cycles: everything → top bar only → nothing (screenshot mode).
    pub panel_hidden: bool,
    /// The meter is held after a light or material edit (E7b): the status bar says so.
    pub exposure_held: bool,
    pub selection: Selection,
    /// Mesh imports stand Z-up files upright (scans, CAD exports).
    pub mesh_z_up: bool,
    /// Mesh files being parsed on worker threads; `land_mesh_imports` adds them when done.
    pub mesh_imports: Vec<MeshImport>,
    /// A viewport pick: the tab brings the selection's inspector into view once.
    pub scroll_to_inspector: bool,
    /// The outliner's body is shown where it folds to one line (Render, Look, Export, or a short
    /// column).
    pub outliner_open: bool,
    /// The outliner's share of the right column, while its edge is dragged (saved on release).
    pub outliner_split: f32,
    pub(crate) outliner_resized: bool,
    /// The outliner's Area lights and Objects groups are closed.
    pub outliner_closed: [bool; 2],
    /// Windows that were sections of the old Scene tab.
    pub show_describe: bool,
    pub show_puzzle: bool,
    /// The scene's puzzle was on last frame: turning on opens its window once.
    pub(crate) puzzle_was_on: bool,
    /// Menu > Settings > Reset everything asks first.
    pub confirm_reset: bool,
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
    /// It was drawn last frame: opening it gives the search box the keyboard.
    pub scenes_were_open: bool,
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
    /// Scenes window list: 0 all, 1 favourites, 2 recent (most recent first), 3 the user's own.
    scene_list: u8,
    /// Scene files' dates for the browser's Date modified order, read when that order is first
    /// shown and again after the list is.
    scene_dates: std::collections::HashMap<std::path::PathBuf, Option<std::time::SystemTime>>,
    /// The scene browser's middle: 0 the cards, 1 Generate, 2 Save current scene.
    scene_page: u8,
    /// Preferences (prefs.rs); written back only when `prefs_live` (interactive runs).
    pub prefs: crate::prefs::Prefs,
    pub prefs_live: bool,
    /// Menu → Renderer → Restart now: the app starts itself again and exits.
    pub restart: bool,
    /// The update window asked to install the verified download.
    pub install_update: bool,
    /// The About box's logos (`chrome::LOGOS`), once it has been opened.
    pub logos: Vec<egui::TextureHandle>,
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
    /// Why the HDRI or the LUT did not load, for their rows (the toast is gone in seconds).
    pub environment_error: Option<String>,
    pub lut_error: Option<String>,
    /// The right-click menu's position in the viewport, while it is open.
    pub context_menu: Option<egui::Pos2>,
    /// The keyboard and mouse sheet (F1).
    pub show_shortcuts: bool,
    /// The skin's control sheet (`--ui-skin-sheet`): every control in every state.
    pub skin_sheet: bool,
    /// The command palette (Ctrl+K, palette.rs).
    pub palette: crate::palette::State,
    /// The timeline strip (track 8); it opens when the scene gains keys, and can then be closed.
    pub show_timeline: bool,
    pub timeline_had_keys: bool,
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
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    /// Interactive runs only: panel edits become undo steps, the scene gets its unsaved mark
    /// and last-session.json is kept. Headless runs, the self-test, scripted and frame-limited
    /// runs stay out of all of it.
    pub edits_live: bool,
    /// The command line chose the tab or tool: the saved prefs don't replace them.
    pub tab_from_args: bool,
    pub tool_from_args: bool,
    /// The settings when `watch_edits` last looked (the free camera follows every frame).
    edit_base: Settings,
    /// A settled frame should compare: input arrived since the last comparison.
    edit_check: bool,
    /// Something other than an edit wrote settings (keys, a recording, a script, walking).
    edit_foreign: bool,
    /// The objects a running simulation moves: their poses are not edits, so the bases follow
    /// them when compared, and panel edits go on making steps while it runs.
    sim_objects: Vec<usize>,
    /// The keys applied next belong to the scene just loaded, not to an edit of them.
    keys_follow_load: bool,
    /// The Camera inspector moved the camera: this one step takes the camera with it.
    pub camera_edit: bool,
    /// The state of the last load or save, and whether the scene has moved away from it.
    clean_base: Settings,
    pub scene_dirty: bool,
    /// `scene_dirty` must be worked out again, whatever `edit_base` says (undo, redo).
    dirty_check: bool,
    /// last-session.json, in interactive runs. Edited since it was written, and when it was.
    session_file: Option<std::path::PathBuf>,
    session_stale: bool,
    session_saved: Instant,
    /// The session found at startup, until it is restored or dismissed, and how long the
    /// prompt stays up (Menu → Restore last session stays after that).
    pub session_offer: Option<crate::prefs::Session>,
    session_prompt_until: Instant,
    /// The first close during an export: a second one soon after cancels it.
    pub close_armed: Option<Instant>,
    /// The running CPU export was cancelled (it ends on its own thread a moment later).
    pub export_cancelled: bool,
    /// The side panel's width, for the preferences.
    pub panel_width: f32,
    /// The user dragged the panel's edge; saved when the drag ends.
    panel_resized: bool,
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
            tab: Tab::Object,
            advanced: false,
            panel_hidden: false,
            exposure_held: false,
            selection: Selection::Stage,
            mesh_z_up: false,
            mesh_imports: vec![],
            scroll_to_inspector: false,
            outliner_open: false,
            outliner_split: 0.3,
            outliner_resized: false,
            outliner_closed: [false; 2],
            show_describe: false,
            show_puzzle: false,
            puzzle_was_on: false,
            confirm_reset: false,
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
            scenes_were_open: false,
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
            scene_dates: Default::default(),
            scene_page: 0,
            prefs: Default::default(),
            prefs_live: false,
            restart: false,
            install_update: false,
            logos: Vec::new(),
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
            environment_error: None,
            lut_error: None,
            context_menu: None,
            show_shortcuts: false,
            skin_sheet: false,
            palette: Default::default(),
            show_timeline: false,
            timeline_had_keys: false,
            physics_record: false,
            open_pending: None,
            physics_record_time: 0.,
            physics_recorded: vec![],
            pending: None,
            transform: None,
            header: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            edits_live: false,
            sim_objects: Vec::new(),
            keys_follow_load: false,
            tab_from_args: false,
            tool_from_args: false,
            edit_base: Settings::default(),
            edit_check: false,
            edit_foreign: false,
            camera_edit: false,
            clean_base: Settings::default(),
            scene_dirty: false,
            dirty_check: false,
            session_file: None,
            session_stale: false,
            session_saved: Instant::now(),
            session_offer: None,
            session_prompt_until: Instant::now(),
            close_armed: None,
            export_cancelled: false,
            panel_width: 370.,
            panel_resized: false,
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
        // No leading "-" from a scene without a name.
        let scene = match s.scene_name.trim() {
            "" => "caustica",
            name => name,
        };
        crate::video::expand_name(&self.prefs.name_template, scene, samples, w, h)
    }
    pub fn toast(&mut self, text: impl Into<String>) {
        self.push(text.into(), false, None);
    }
    /// A toast about a saved file, with Show in folder.
    pub fn toast_file(&mut self, text: impl Into<String>, file: std::path::PathBuf) {
        self.push(text.into(), false, Some(file));
    }
    /// Parses a mesh file on a worker thread (a big glTF takes seconds); the objects land in
    /// the scene through `land_mesh_imports`, with one undo step.
    pub fn import_mesh(&mut self, path: std::path::PathBuf, z_up: bool) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let (send, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(crate::mesh::import_scene(&path, z_up));
        });
        self.toast(format!("Importing {name}…"));
        self.mesh_imports.push(MeshImport { name, result });
    }
    /// Adds the imports that finished since the last frame. Returns whether any are still running.
    pub fn land_mesh_imports(&mut self, s: &mut Settings) -> bool {
        use std::sync::mpsc::TryRecvError;
        let mut landed = vec![];
        self.mesh_imports.retain(|job| match job.result.try_recv() {
            Ok(r) => {
                landed.push((job.name.clone(), r));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                landed.push((job.name.clone(), Err(anyhow::anyhow!("the import stopped"))));
                false
            }
        });
        for (name, result) in landed {
            match result {
                Ok((objects, lights)) => {
                    self.checkpoint(s);
                    let room = scene::MAX_LIGHTS.saturating_sub(s.lights.len());
                    s.lights.extend(lights.into_iter().take(room));
                    let first = s.objects.len();
                    let room = scene::MAX_OBJECTS.saturating_sub(first);
                    let total = objects.len();
                    s.objects.extend(objects.into_iter().take(room));
                    let added = s.objects.len() - first;
                    if total > added {
                        self.error(format!(
                            "{name}: only {added} of {total} parts fit ({} objects)",
                            scene::MAX_OBJECTS
                        ));
                    } else {
                        self.toast(format!("Imported {name}: {added} object(s)"));
                    }
                    if added > 0 {
                        self.selection = Selection::Object(first);
                    }
                }
                Err(e) => self.error(format!("{name}: {e:#}")),
            }
        }
        !self.mesh_imports.is_empty()
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
    /// Scenes window list: 0 all, 1 favourites, 2 recent, 3 My scenes.
    pub fn set_scene_list(&mut self, list: u8) {
        self.scene_list = list.min(3);
    }
    /// An export ended: it joins Recent exports, and its folder opens if the user asked for
    /// that (`open`: not in hidden or scripted runs).
    pub fn export_ended(&mut self, file: &std::path::Path, state: &str, open: bool) {
        let file_name = file.display().to_string();
        let recent = &mut self.prefs.recent_exports;
        recent.retain(|e| e.file != file_name);
        recent.insert(
            0,
            crate::prefs::RecentExport {
                file: file_name,
                state: state.into(),
            },
        );
        recent.truncate(8);
        self.save_prefs();
        if open && state == "saved" && self.prefs.open_folder_done {
            show_in_folder(file);
        }
    }
    /// Writes the preferences file (interactive runs only).
    pub fn save_prefs(&self) {
        if self.prefs_live {
            self.prefs.save();
        }
    }
    /// Remembers `s` for undo, before an edit. The camera stays where it is on undo.
    pub fn checkpoint(&mut self, s: &Settings) {
        self.push_undo(s, false);
    }
    /// `checkpoint` before the whole scene changes: undo brings back the old scene as it was
    /// framed.
    pub fn checkpoint_scene(&mut self, s: &Settings) {
        self.push_undo(s, true);
    }
    fn push_undo(&mut self, s: &Settings, camera: bool) {
        let top = self.undo.last_mut();
        // An edit's step ignores the camera, so an orbit between the action's checkpoint and
        // the watcher's does not make two steps of one edit.
        let same = top.is_some_and(|top| {
            let same = if camera {
                top.settings == *s
            } else {
                same_but_camera(&mut top.settings, s)
            };
            top.camera |= same && camera;
            same
        });
        if !same {
            self.undo.push(UndoStep {
                settings: s.clone(),
                camera,
            });
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    pub fn undo(&mut self, s: &mut Settings) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        let back = self.take_step(s, step);
        self.redo.push(back);
        true
    }
    pub fn redo(&mut self, s: &mut Settings) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        let back = self.take_step(s, step);
        self.undo.push(back);
        true
    }
    /// Puts `step` in place and returns the step that leads back.
    fn take_step(&mut self, s: &mut Settings, step: UndoStep) -> UndoStep {
        let mut settings = step.settings;
        if !step.camera {
            set_camera(&mut settings, camera_of(s));
        }
        let back = UndoStep {
            settings: std::mem::replace(s, settings),
            camera: step.camera,
        };
        if self.edits_live {
            // Not an edit for the watcher (it would clear the redo list), but maybe unsaved.
            self.edit_base = s.clone();
            self.dirty_check = true;
            self.edit_check = true;
            self.session_stale = true;
        }
        back
    }
    /// Undo for panel edits, once per frame after the UI: when input has come in and nothing
    /// is held (a drag, a focused text field), a difference from the last look becomes one
    /// step. Actions with their own checkpoint merge with it (`push_undo`). A full comparison
    /// only runs on those frames, so a large scene does not pay for it while idle. `foreign`:
    /// something other than the user writes settings now; what it wrote makes no step.
    pub fn watch_edits(&mut self, s: &Settings, input: bool, settled: bool, foreign: bool) {
        if !self.edits_live {
            return;
        }
        if foreign {
            // What still waits for its field came before these writes.
            self.flush_pending(s);
            self.edit_foreign = true;
            return;
        }
        if std::mem::take(&mut self.edit_foreign) {
            self.rebase_edits(s);
            // Keys a recording left are unsaved work, though no step.
            self.dirty_check = true;
        }
        self.edit_check |= input || self.dirty_check;
        if !self.camera_edit {
            set_camera(&mut self.edit_base, camera_of(s));
        }
        // finish_transform records the transform's own step.
        if self.edit_check && settled && self.transform.is_none() {
            self.flush_edits(s);
        }
    }
    /// What `s` holds now is not an edit (timeline keys applied, a simulation ended).
    pub fn rebase_edits(&mut self, s: &Settings) {
        if self.edits_live {
            self.edit_base = s.clone();
            self.camera_edit = false;
        }
    }
    /// A comparison `watch_edits` still owes (a field holds the keyboard), run now because
    /// something else is about to write settings.
    pub fn flush_pending(&mut self, s: &Settings) {
        if self.edits_live && self.edit_check && !self.edit_foreign && self.transform.is_none() {
            self.flush_edits(s);
        }
    }
    /// After `Timeline::update`. Keys applied for the playhead or a load are not an edit, and
    /// a clean scene stays clean under them. Keys applied because the keys were edited are
    /// that edit's result: the next comparison takes them with it.
    pub fn keys_applied(&mut self, s: &Settings, applied: bool, keys_edited: bool) {
        let loaded = std::mem::take(&mut self.keys_follow_load);
        if !self.edits_live || !applied || (keys_edited && !loaded) {
            return;
        }
        if !self.scene_dirty {
            self.clean_base = s.clone();
        }
        self.rebase_edits(s);
    }
    /// The simulation stopped with `s` as its last poses.
    pub fn sim_ended(&mut self, s: &Settings) {
        self.follow_sim(s);
        self.sim_objects.clear();
    }
    /// Takes the simulated poses into what `s` is compared with.
    fn follow_sim(&mut self, s: &Settings) {
        if !self.sim_objects.is_empty() {
            follow_poses(&mut self.edit_base, s, &self.sim_objects);
            follow_poses(&mut self.clean_base, s, &self.sim_objects);
        }
    }
    /// The comparison of `watch_edits`; also run on closing, whatever is held.
    fn flush_edits(&mut self, s: &Settings) {
        self.edit_check = false;
        self.follow_sim(s);
        let camera = std::mem::take(&mut self.camera_edit);
        if !camera {
            set_camera(&mut self.edit_base, camera_of(s));
        }
        let changed = self.edit_base != *s;
        if changed {
            let base = std::mem::replace(&mut self.edit_base, s.clone());
            self.push_undo(&base, camera);
        }
        if std::mem::take(&mut self.dirty_check) || changed {
            self.refresh_dirty(s);
            self.session_stale = true;
        }
    }
    /// Whether the scene differs from its last load or save. The camera and what the viewer
    /// chose to look through don't count: an orbit is not unsaved work.
    fn refresh_dirty(&mut self, s: &Settings) {
        let base = &mut self.clean_base;
        set_camera(base, camera_of(s));
        base.keep_output(s);
        base.view = s.view;
        (base.compare, base.split) = (s.compare, s.split);
        (base.rotate, base.rotate_speed) = (s.rotate, s.rotate_speed);
        self.scene_dirty = *base != *s;
    }
    /// `s` was just loaded or saved: nothing to warn about, and nothing for the next start to
    /// offer (a session still on offer keeps its file until it is answered).
    pub fn mark_clean(&mut self, s: &Settings) {
        if !self.edits_live {
            return;
        }
        self.clean_base = s.clone();
        self.edit_base = s.clone();
        self.keys_follow_load = true;
        self.scene_dirty = false;
        self.session_stale = false;
        if self.session_offer.is_none() {
            self.remove_session();
        }
    }
    /// An interactive run starts: `s` is the clean state, and a session left by the last run
    /// goes on offer.
    pub fn start_edit_tracking(&mut self, s: &Settings) {
        self.edits_live = true;
        self.keys_follow_load = true;
        self.edit_base = s.clone();
        self.clean_base = s.clone();
        self.session_file = crate::prefs::session_path();
        self.session_offer = self
            .session_file
            .as_deref()
            .and_then(crate::prefs::read_session);
        self.session_saved = Instant::now();
        self.session_prompt_until = Instant::now() + Duration::from_secs(30);
    }
    /// While the prompt for the last session should be drawn.
    pub fn session_prompt(&self) -> bool {
        self.session_offer.is_some() && Instant::now() < self.session_prompt_until
    }
    /// Writes last-session.json at most once a minute while there is unsaved work.
    pub fn autosave_session(&mut self, s: &Settings) {
        if self.scene_dirty
            && self.session_stale
            && self.session_saved.elapsed() >= Duration::from_secs(60)
        {
            self.write_session(s);
        }
    }
    /// On closing: unsaved work goes to last-session.json; with none, the file goes too.
    pub fn close_session(&mut self, s: &Settings) {
        if !self.edits_live {
            return;
        }
        // Whatever a running simulation or script wrote makes no step, as in `watch_edits`.
        if !self.edit_foreign {
            self.flush_edits(s);
        }
        if !self.scene_dirty {
            if self.session_offer.is_none() {
                self.remove_session();
            }
        } else if self.session_stale {
            self.write_session(s);
        }
    }
    fn write_session(&mut self, s: &Settings) {
        self.session_saved = Instant::now();
        let Some(path) = &self.session_file else {
            return;
        };
        match crate::prefs::write_session(path, s, &self.loaded_file) {
            Ok(()) => self.session_stale = false,
            Err(e) => eprintln!("Saving the session: {e:#}"),
        }
    }
    /// Nothing unsaved to bring back.
    fn remove_session(&self) {
        if let Some(path) = &self.session_file {
            let _ = std::fs::remove_file(path);
        }
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

/// The screen minus the panels, as of the last frame's panels (the content rect before any).
pub(crate) fn central_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|d| d.get_temp(egui::Id::unique("central rect")))
        .unwrap_or_else(|| ctx.content_rect())
}
fn set_central_rect(ctx: &egui::Context, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("central rect"), rect));
}
pub fn theme(ctx: &egui::Context) {
    // egui's proportional family has no arrows (U+2192 is only in Hack, its monospace font): add
    // Hack as the last resort, so "Stage → HDRI" is not a box. The ui_strings_have_glyphs test
    // checks every non-ASCII UI string against this family.
    let mut fonts = egui::FontDefinitions::default();
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.push("Hack".to_owned());
    }
    // The skin's own families (IBM Plex), over the same fallbacks.
    skin::fonts(&mut fonts);
    ctx.set_fonts(fonts);
    let mut style = (*ctx.global_style()).clone();
    style.visuals = egui::Visuals::dark();
    skin::style(&mut style);
    ctx.set_global_style(style);
}
/// What the controls of the `row` being drawn tell it (the context's temp data).
#[derive(Clone, Copy, Default)]
struct RowState {
    active: bool,
    /// A slider leaves its reset hint to the row's tooltip.
    slider: bool,
    /// A control is showing a tooltip of its own, where the row's would land on top of it.
    quiet: bool,
    /// Where the row's label ends and where its controls start, for a slider's unit.
    label_right: f32,
    controls_left: f32,
}
fn row_state(ctx: &egui::Context) -> RowState {
    ctx.data(|d| d.get_temp(egui::Id::unique("row state")))
        .unwrap_or_default()
}
fn set_row_state(ctx: &egui::Context, state: RowState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("row state"), state));
}
/// True while a `row`'s controls are being drawn.
fn in_row(ctx: &egui::Context) -> bool {
    row_state(ctx).active
}
/// For `Slider`: true when a row shows its reset hint.
pub(crate) fn slider_in_row(ctx: &egui::Context) -> bool {
    let state = row_state(ctx);
    if state.active {
        set_row_state(
            ctx,
            RowState {
                slider: true,
                ..state
            },
        );
    }
    state.active
}
/// For the skin's `Slider`: where its unit may go when the slider is the first control of a
/// row, as (the label's right edge, the controls' left edge). The unit is drawn at the end of
/// the label column, so every number field is as wide as the next and the rails line up.
pub(crate) fn row_unit_room(ui: &egui::Ui) -> Option<(f32, f32)> {
    let state = row_state(ui.ctx());
    let first = (ui.cursor().left() - state.controls_left).abs() < 0.5;
    (state.active && first).then_some((state.label_right, state.controls_left))
}
/// The row's tooltip stands aside while the pointer is on `r`, which has its own.
pub(crate) fn row_tip_yields(r: &egui::Response) {
    let state = row_state(&r.ctx);
    if state.active && r.ctx.rect_contains_pointer(r.layer_id, r.rect) {
        set_row_state(
            &r.ctx,
            RowState {
                quiet: true,
                ..state
            },
        );
    }
}
/// A tooltip of its own for a control in a `row`.
pub(crate) fn own_tip(r: egui::Response, text: impl Into<egui::WidgetText>) -> egui::Response {
    row_tip_yields(&r);
    r.on_hover_text(text)
}
/// Why a control is disabled, in a `row` or not.
pub(crate) fn why_disabled(r: egui::Response, text: impl Into<egui::WidgetText>) -> egui::Response {
    if !r.enabled() {
        row_tip_yields(&r);
    }
    r.on_disabled_hover_text(text)
}
/// The controls column of a row, and the tooltip over the whole row: a hover-only widget on top
/// of the label and controls, which still get their clicks and drags.
fn row_controls<R>(
    ui: &mut egui::Ui,
    label: egui::Response,
    tip: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let outer = row_state(ui.ctx());
    // Active before the key mark, whose own tooltip makes the row's stand aside.
    set_row_state(
        ui.ctx(),
        RowState {
            active: true,
            label_right: label.rect.right(),
            ..Default::default()
        },
    );
    let controls = ui.horizontal(|ui| {
        // The key mark's slot, in the tabs whose values the timeline keys.
        crate::timeline_ui::key_slot(ui);
        let state = row_state(ui.ctx());
        set_row_state(
            ui.ctx(),
            RowState {
                controls_left: ui.cursor().left(),
                ..state
            },
        );
        // The slider gives way in a narrow column (a number box of up to 76 points follows
        // it), and a long combo text is cut, so a row never runs past the column's edge.
        ui.spacing_mut().slider_width = (ui.available_width() - 84.).clamp(48., 130.);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        add(ui)
    });
    let state = row_state(ui.ctx());
    set_row_state(ui.ctx(), outer);
    if !state.quiet && !tip.is_empty() {
        let hover = ui.interact(
            label.rect.union(controls.response.rect),
            ui.auto_id_with("row tip"),
            egui::Sense::hover(),
        );
        if state.slider {
            hover.on_hover_text(format!("{tip}\n\nDouble-click the slider to reset."));
        } else {
            hover.on_hover_text(tip);
        }
    }
    ui.end_row();
    controls.inner
}
/// A row's label: the skin's are dimmer than the values beside them.
fn row_label(label: &str) -> egui::RichText {
    egui::RichText::new(label).color(skin::DIM)
}
/// A labelled row in a two-column property grid, with the tooltip over the whole row.
pub fn row(ui: &mut egui::Ui, label: &str, tip: &str, add: impl FnOnce(&mut egui::Ui)) {
    let label = ui.label(row_label(label));
    row_controls(ui, label, tip, add);
}
/// `row` for a control that does nothing right now: `idle` says why. The controls stay visible
/// but greyed with their stored values, and the row's tooltip leads with the reason.
pub fn row_if(
    ui: &mut egui::Ui,
    label: &str,
    tip: &str,
    idle: Option<&str>,
    add: impl FnOnce(&mut egui::Ui),
) {
    let label = ui.add_enabled(idle.is_none(), egui::Label::new(row_label(label)));
    let tip = match idle {
        Some(why) => format!("{why}\n\n{tip}"),
        None => tip.to_string(),
    };
    row_controls(ui, label, &tip, |ui| {
        ui.add_enabled_ui(idle.is_none(), add);
    });
}
/// A row whose control is a checkbox: clicking the label toggles it too. Returns the checkbox's
/// response (`changed()` covers both).
pub fn check_row(ui: &mut egui::Ui, label: &str, tip: &str, on: &mut bool) -> egui::Response {
    check_row_with(ui, label, tip, on, None, |_, _| {})
}
/// `check_row` with more controls after the checkbox (`extra` gets its state), disabled with
/// the reason when `idle` says why it does nothing right now.
pub fn check_row_with(
    ui: &mut egui::Ui,
    label: &str,
    tip: &str,
    on: &mut bool,
    idle: Option<&str>,
    extra: impl FnOnce(&mut egui::Ui, bool),
) -> egui::Response {
    // As plain a colour as any other row's label; a clickable one would be brighter.
    let text = egui::Label::new(egui::RichText::new(label).color(skin::DIM))
        .sense(egui::Sense::click())
        .selectable(false);
    let label = ui.add_enabled(idle.is_none(), text);
    let flipped = label.clicked();
    if flipped {
        *on = !*on;
    }
    row_controls(ui, label, tip, |ui| {
        let mut checkbox = ui
            .add_enabled_ui(idle.is_none(), |ui| skin::checkbox(ui, on))
            .inner;
        if flipped {
            checkbox.mark_changed();
        }
        if let Some(idle) = idle {
            checkbox = why_disabled(checkbox, idle);
        }
        extra(ui, *on);
        checkbox
    })
}
/// A file row's mark for a file that did not load; the error is its tooltip.
pub(crate) fn cant_load(ui: &mut egui::Ui, error: &str) {
    own_tip(
        ui.label(egui::RichText::new("can't load").color(WARN)),
        error,
    );
}
/// What a click or B says where picking needs a plain perspective camera.
pub(crate) const PICKING_OFF: &str = "Picking is off with this camera: select in the outliner";
/// Time left, as the status bar and the Export tab word it.
pub(crate) fn time_left(seconds: f32) -> String {
    if seconds < 90. {
        format!("~{seconds:.0} s")
    } else {
        format!("~{:.0} min", seconds / 60.)
    }
}
/// A value with at most four significant digits and no trailing zeros, for a logarithmic
/// slider whose range spans many decades: 0, 0.012, 0.0004321. Small values keep their digits,
/// so clicking into the field and out does not round them away.
pub(crate) fn short_number(v: f64) -> String {
    if v == 0. {
        return "0".into();
    }
    let decimals = (3 - v.abs().log10().floor() as i32).clamp(0, 12) as usize;
    let text = format!("{v:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}
/// Advanced rows: shown inline with "Show advanced settings", otherwise under "More".
pub fn more(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    advanced: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    if advanced {
        add(ui);
    } else {
        skin::disclosure(ui, id, "More", false, add);
    }
}
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.);
    skin::heading(ui, title);
    ui.add_space(2.);
}
/// The name at the top of a property tab.
pub(crate) fn tab_title(ui: &mut egui::Ui, title: &str) {
    skin::title(ui, title);
}
/// `tab_title` with a control at the right end of the line.
pub(crate) fn tab_title_with(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    skin::title_with(ui, title, add);
}
/// A collapsible section of a property tab. Its header is one clickable line: an arrow, the
/// title, and at the right a preview of the values inside, so a closed section still tells the
/// state. Open or closed is remembered per section.
pub(crate) fn fold<R>(
    ui: &mut egui::Ui,
    title: &str,
    preview: impl Into<String>,
    default_open: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    skin::fold(ui, title, preview.into(), default_open, add)
}
/// One row of the outliner's tree: `label` after `indent`, `dim` text at the right, filled when
/// selected. Takes the rest of the line.
pub(crate) fn tree_row(
    ui: &mut egui::Ui,
    indent: f32,
    selected: bool,
    label: &str,
    dim: &str,
) -> egui::Response {
    tree_row_keyed(ui, indent, selected, label, dim, false)
}
/// `tree_row` with a diamond at the right for something that holds keyframes.
pub(crate) fn tree_row_keyed(
    ui: &mut egui::Ui,
    indent: f32,
    selected: bool,
    label: &str,
    dim: &str,
    keyed: bool,
) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        // The row is a flat button: lit under the pointer, pressed in with accent text while
        // it is the selection.
        let down = skin::button_chrome(ui, rect, &r, true, selected);
        let (text, faint) = if selected {
            (ACCENT, skin::DIM)
        } else {
            (skin::TX, skin::FAINT)
        };
        let mut right = rect.right() - 6.;
        if keyed {
            let c = egui::pos2(right - 4., rect.center().y);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(0., -4.),
                    c + egui::vec2(4., 0.),
                    c + egui::vec2(0., 4.),
                    c + egui::vec2(-4., 0.),
                ],
                ACCENT,
                egui::Stroke::NONE,
            ));
            right -= 14.;
        }
        if !dim.is_empty() {
            let galley = painter.layout_no_wrap(dim.to_owned(), skin::sans(11.5), faint);
            right -= galley.size().x;
            painter.galley(
                egui::pos2(right, rect.center().y - galley.size().y / 2. + down),
                galley,
                faint,
            );
            right -= 8.;
        }
        let font = skin::sans(12.5);
        let left = rect.left() + 6. + indent;
        painter
            .with_clip_rect(
                egui::Rect::from_min_max(rect.min, egui::pos2(right, rect.bottom()))
                    .intersect(ui.clip_rect()),
            )
            .text(
                egui::pos2(left, rect.center().y + down),
                egui::Align2::LEFT_CENTER,
                label,
                font,
                text,
            );
    }
    r
}
/// The label column of the Stage and material grids. Blocks that are separate grids give
/// their columns the same minimum, so the controls line up from one block to the next.
pub(crate) const LABEL_WIDTH: f32 = 128.;
pub(crate) fn grid(id: &str) -> egui::Grid {
    // Rows of 28 points: a control and 4 between.
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8., 4.])
        .min_col_width(LABEL_WIDTH)
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
    // Room for the radio, the button and their spacing, or the row widens the panel every frame.
    let width = (ui.available_width() - 140.).max(80.);
    for (k, text) in edit.iter_mut().enumerate() {
        let current = s.lpe.get(k).cloned().unwrap_or_default();
        let mut error = None;
        ui.horizontal(|ui| {
            if crate::ui::skin::radio(ui, shown && s.lpe_slot == k as u32, format!("{}", k + 1))
                .on_hover_text("Show this expression's image")
                .clicked()
            {
                s.view = 12;
                s.lpe_slot = k as u32;
            }
            let field = crate::ui::skin::text_edit(
                ui,
                width,
                egui::TextEdit::singleline(text).hint_text(field_hint("e.g. C D L")),
            );
            let mut set = None;
            if field.changed() {
                set = Some(text.clone());
            }
            sticky_menu(ui, "Presets", |ui| {
                for (name, expr) in lpe::PRESETS {
                    if crate::ui::skin::button(ui, format!("{name}  ({expr})")).clicked() {
                        set = Some(expr.to_string());
                        ui.close();
                    }
                }
                if crate::ui::skin::button(ui, "Clear").clicked() {
                    set = Some(String::new());
                    ui.close();
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
    ui.label(egui::RichText::new(text).small().color(skin::FAINT));
}
/// How wide `text` is set in `font`, for sizing a control that sits beside it.
pub(crate) fn text_width(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, skin::TX)
        .size()
        .x
        .ceil()
}
/// How wide a plain skin button with `text` is.
pub(crate) fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, skin::sans(12.5)) + 18.
}
/// A group of numbers (x/y/z) that fills the controls column evenly, the revert dot after
/// them where a slider's is. `prefixes` and `suffix` label each field.
pub(crate) fn drag_group(
    ui: &mut egui::Ui,
    values: &mut [f32],
    prefixes: &[&str],
    suffix: &str,
    speed: f32,
    range: std::ops::RangeInclusive<f32>,
) {
    let n = values.len() as f32;
    let gap = ui.spacing().item_spacing.x;
    // The dot is 8 wide; every item has a gap before the next.
    let width = ((ui.available_width() - 8. - gap * n) / n).clamp(40., 120.);
    for (i, v) in values.iter_mut().enumerate() {
        let mut d = egui::DragValue::new(v).speed(speed).range(range.clone());
        if let Some(p) = prefixes.get(i) {
            d = d.prefix(*p);
        }
        if !suffix.is_empty() {
            d = d.suffix(suffix);
        }
        skin::drag_value_sized(ui, width, d);
    }
    crate::revert_dot(ui, values);
}
/// A text field's placeholder. egui's default is the weak text colour, about 2:1 on the field;
/// this is the plain label grey, still dimmer than typed text.
pub(crate) fn field_hint(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(skin::FAINT)
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
    let mut text = if s.native_filter() {
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
    if s.rr() {
        return (format!("Ray Reconstruction unavailable · {text}"), true);
    }
    (text, fallback)
}
pub fn view_name(view: u32) -> &'static str {
    VIEWS[view as usize]
}
/// A grey card with the scene's initial, until its thumbnail exists.
pub(crate) fn placeholder(ui: &mut egui::Ui, title: &str, size: egui::Vec2) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        3.,
        if r.hovered() {
            skin::TAB_HOVER
        } else {
            skin::DEEP
        },
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title.chars().next().unwrap_or('?'),
        egui::FontId::proportional(size.y * 0.4),
        skin::FAINT,
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
    pub fn ui(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        // scRGB blends the UI in linear light; the skin corrects its translucent pixels.
        let linear = self.gpu.as_ref().is_some_and(|g| g.hdr_active && !g.hdr10);
        skin::blend_in_linear_light(linear);
        // Text too: egui's dark-mode curve (2c - c²) thickens glyph edges for a blend on code
        // values. In linear light a light glyph on a dark panel already gains that much, so
        // both together made every font a weight bolder in HDR. Plain coverage there lands
        // within 4 % of the SDR result for the UI's text colours (takes effect next frame).
        use egui::epaint::FontColorTransferFunction as Transfer;
        let transfer = if linear {
            Transfer::Gamma(1.)
        } else {
            Transfer::DARK_MODE_DEFAULT
        };
        if ctx
            .global_style()
            .visuals
            .text_options
            .color_transfer_function
            != transfer
        {
            ctx.global_style_mut(|s| s.visuals.text_options.color_transfer_function = transfer);
        }
        self.poll_describe(ctx);
        // Double-click resets: the scene as it was when it got its name (loaded, preset,
        // generated or randomized), each object and light by index, and default UI values.
        // A search from the last scene would hide this one's objects too.
        if self.reset_baseline.scene_name != self.settings.scene_name {
            self.reset_baseline = self.settings.clone();
            self.ui.search.clear();
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
        self.draw_ui(root);
        crate::clear_reset_sources();
        // Nothing can be edited while an export runs.
        if !self.exporting() {
            self.track_edits(ctx);
        }
    }
    /// Panel edits become undo steps, and unsaved work reaches last-session.json.
    fn track_edits(&mut self, ctx: &egui::Context) {
        if !self.ui.edits_live {
            return;
        }
        let (input, down) = ctx.input(|i| {
            (
                i.pointer.any_released()
                    || i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::Key { .. })),
                i.pointer.any_down(),
            )
        });
        let settled = !down && !ctx.egui_wants_keyboard_input();
        match &self.physics {
            Some(sim) => self.ui.sim_objects = sim.moving_objects(),
            None => self.ui.sim_objects.clear(),
        }
        let foreign = self.edits_foreign();
        self.ui.watch_edits(&self.settings, input, settled, foreign);
        self.ui.autosave_session(&self.settings);
    }
    /// Something other than the user writes more than poses now (a simulation's poses are
    /// `sim_objects`). Keys applied at a still playhead rebase where that happens (main.rs).
    pub(crate) fn edits_foreign(&self) -> bool {
        self.timeline.playing
            || self.ui.physics_record
            || !self.ui.physics_recorded.is_empty()
            || self.script.is_some()
            || self.walking.is_some()
    }
    /// Replaces the scene from a UI control (scene picker, ◀ ▶, Randomize, Today, gamepad,
    /// phone remote): undoable, with the old selection gone. Not for the
    /// self-test or headless runs, which call `Settings::preset` directly.
    pub fn switch_scene(&mut self, label: &str, change: impl FnOnce(&mut Settings)) {
        self.ui.checkpoint_scene(&self.settings);
        self.laser = crate::laser::Laser::default();
        change(&mut self.settings);
        self.ui.select(Selection::Stage);
        self.ui.mark_clean(&self.settings);
        self.ui
            .toast(format!("{label}: {}", self.settings.scene_name));
    }
    /// Menu → Restore last session, or the prompt at startup: loads like opening a scene, so
    /// undo goes back to what was on screen. It stays marked unsaved.
    pub fn restore_session(&mut self) {
        let Some(session) = self.ui.session_offer.take() else {
            return;
        };
        self.ui.checkpoint_scene(&self.settings);
        self.laser = crate::laser::Laser::default();
        self.physics = None;
        self.settings = session.settings;
        // Its values stay as they were saved; the keys apply once the playhead moves.
        self.timeline.settle(&self.settings);
        self.ui.rebase_edits(&self.settings);
        self.ui.select(Selection::Stage);
        self.ui.loaded_file = session.loaded_file;
        self.ui.scene_selected = self.scene_entry();
        self.ui.scene_dirty = true;
        self.ui
            .toast(format!("Restored {}", self.settings.scene_name));
    }
    fn draw_ui(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        // Loaded and randomized settings carry their preset by name.
        let known = PRESETS.iter().position(|n| *n == self.settings.scene_name);
        if let Some(i) = known {
            self.preset = i;
        }
        self.top_bar(root, known.is_some());
        // An export locks everything but its card: the bars and the rail are drawn disabled,
        // the column shows Export, and no window, overlay or shortcut sheet opens.
        if self.exporting() {
            self.status_bar(root);
            self.tool_rail(root);
            self.side_panel(root, known.is_some());
            set_central_rect(ctx, root.available_rect_before_wrap());
            self.export_lock_note(ctx);
            self.toasts(ctx);
            return;
        }
        if !self.ui.panel_hidden {
            // The rail and the right column before the timeline, which then runs under the
            // viewport only and leaves the column its full height.
            self.status_bar(root);
            self.tool_rail(root);
            self.side_panel(root, known.is_some());
            self.timeline_panel(root);
        }
        // What the panels left: popups and toasts stay out of them (egui 0.34's content_rect no
        // longer shrinks, and `available_rect` is deprecated).
        set_central_rect(ctx, root.available_rect_before_wrap());
        self.cpu_view_message(ctx);
        self.transform_strip(ctx);
        self.describe_window(ctx);
        self.puzzle_window(ctx);
        self.reset_dialog(ctx);
        // Both browsers fill the viewport: the one opened last replaces the other.
        if self.ui.show_scenes && self.ui.assets.open {
            if self.ui.scenes_were_open {
                self.ui.show_scenes = false;
            } else {
                self.ui.assets.open = false;
            }
        }
        self.scenes_window(ctx);
        self.assets_window(ctx);
        self.video_window(ctx);
        self.tool_windows(ctx);
        self.overlay(ctx);
        self.context_menu(ctx);
        self.shortcuts_window(ctx);
        if self.ui.skin_sheet {
            skin::sheet(ctx);
        }
        self.palette_window(ctx);
        self.update_window(ctx);
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
                let preset = self.preset;
                self.switch_scene("Scene", |s| s.preset(preset));
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
                self.ui.checkpoint_scene(&self.settings);
                self.laser = crate::laser::Laser::default();
                match crate::scenes::load_into(&mut self.settings, path) {
                    Ok(()) => {
                        self.ui.mesh_imports.clear();
                        self.ui.mark_clean(&self.settings);
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
                self.ui.mark_clean(&self.settings);
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
        // Where the UI was left. The panel's width waits for its drag to end.
        let p = &mut self.ui.prefs;
        let layout = (
            self.ui.tab,
            self.ui.tool,
            self.ui.local,
            self.ui.pivot,
            self.ui.snap_to,
        );
        let released = !ctx.input(|i| i.pointer.any_down());
        let mut panel_moved = self.ui.panel_resized && released;
        if panel_moved {
            self.ui.panel_resized = false;
            p.panel_width = self.ui.panel_width;
        }
        if self.ui.outliner_resized && released {
            self.ui.outliner_resized = false;
            p.outliner_split = self.ui.outliner_split;
            panel_moved = true;
        }
        if panel_moved || layout != (p.tab, p.tool, p.local, p.pivot, p.snap_to) {
            (p.tab, p.tool, p.local, p.pivot, p.snap_to) = layout;
            self.ui.save_prefs();
        }
        if self.ui.prefs_live && !self.ui.prefs.hint_seen {
            let mut seen = false;
            egui::Area::new(egui::Id::unique("first run hint"))
                .anchor(egui::Align2::CENTER_BOTTOM, [0., -8.])
                .constrain_to(central_rect(ctx))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    crate::ui::skin::card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label("Left drag orbits · right drag pans · wheel zooms · click selects · right-click opens a menu · F1 lists every shortcut");
                                ui.label("Scenes: pick a scene · Look tab: adjust the image · F12: save PNG · Tab: hide the panels · Ctrl+K: find anything");
                            });
                            seen = crate::ui::skin::button(ui, "Got it").clicked();
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
                self.start_transform(mode, None);
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
        if ui
            .add_enabled(selected, crate::ui::skin::Button::new(label))
            .clicked()
        {
            chosen = Some(c);
            ui.close();
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
/// Which groups of `SHORTCUTS` stack in each column of the sheet.
const SHORTCUT_COLUMNS: [&[usize]; 3] = [&[0, 2], &[1], &[3]];
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
            ("O", "Auto orbit"),
            ("`", "Walk mode (WASD, Esc to leave)"),
            ("T", "Flashlight on or off (also while walking)"),
            (
                "Walking",
                "Space jump, C crouch, Shift run, F fly, B head bob, wheel body size",
            ),
            ("While walking", "F12, F1, Tab, Ctrl+Z / Y / O also work"),
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
            ("G / R / S", "Over the viewport: move / rotate / scale"),
            ("Then", "X Y Z, a value, Enter"),
            ("Ctrl+D", "Duplicate"),
            ("Shift+D / Alt+drag a handle", "Duplicate and move"),
            (
                "Ctrl while moving",
                "Snap (top bar: increment, surface, vertex, edge)",
            ),
            ("Tab while moving", "Next axis of a typed vector"),
            ("H / Alt+H", "Hide or show / show all"),
            ("Del", "Delete"),
            ("Esc", "Deselect (or close Scenes)"),
            ("Ctrl+Z / Ctrl+Y", "Undo / redo (also Ctrl+Shift+Z)"),
        ],
    ),
    (
        "Files and windows",
        &[
            ("Ctrl+S / Ctrl+Shift+S", "Save the scene / save as"),
            ("Ctrl+O", "Open a scene file"),
            (
                "Ctrl+K",
                "Command palette: find a setting or command by name",
            ),
            ("Ctrl+P", "Scenes: type, arrows, Enter loads, Esc closes"),
            ("Ctrl + / −", "UI size"),
            (
                "E / N",
                "Physics: explode / drop a sphere (Shift+N: a cube)",
            ),
        ],
    ),
    (
        "Render and view",
        &[
            ("Space", "Pause"),
            ("R", "Restart, unless an object or light is selected"),
            (
                "Alt+D",
                "Cycle the denoiser: Ray Reconstruction, native, none",
            ),
            ("1 – 8", "Debug views"),
            ("C", "Freeze an A/B reference"),
            (
                "P / Alt+click",
                "Path visualizer / trace the paths of a pixel",
            ),
            ("F12", "Save the image as PNG"),
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

/// Tab belongs to the app (the panel cycle, a typed vector's next axis): given to egui, it would
/// put a button in focus. Only from one text field to the next does egui get it.
/// The textures a frame's UI pass asked for, on their way to the uploaders. egui 0.36 made a
/// `TexturesDelta` assert (debug builds) when dropped with deltas nobody applied; the uploaders
/// take it by reference and never empty it, so this clears it once the frame is done.
pub struct Textures(pub egui::TexturesDelta);
impl Drop for Textures {
    fn drop(&mut self) {
        self.0.clear();
    }
}
/// Test passes draw no pixels: drop the textures the pass asked for, on purpose.
#[cfg(test)]
pub fn discard(mut output: egui::FullOutput) -> egui::FullOutput {
    output.textures_delta.clear();
    output
}
pub fn keep_tab_for_app(input: &mut egui::RawInput, text_field_tab: bool) {
    if !text_field_tab {
        input.events.retain(|e| {
            !matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    ..
                }
            )
        });
    }
}
/// After a frame: whether a text field holds the keyboard. egui's own `wants_keyboard_input` means
/// any focus, and the mark of an editable focused TextEdit (a DragValue being typed in too) is its
/// IME output. Any other focus goes, or Space and Enter would click that widget as well as run
/// their shortcuts; it gets a frame, because a DragValue's field appears the frame after its click.
pub fn settle_focus(ctx: &egui::Context, editing: bool, seen: &mut Option<egui::Id>) -> bool {
    let focused = ctx.memory(|m| m.focused());
    if !editing && focused.is_some() && focused == *seen {
        ctx.memory_mut(|m| m.stop_text_input());
        *seen = None;
    } else {
        *seen = if editing { None } else { focused };
    }
    editing
}
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
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("explorer");
        if path.is_dir() {
            c.arg(&text);
        } else {
            c.arg(format!("/select,{text}"));
        }
        let _ = c.spawn();
    }
    // xdg-open can't select a file: it shows the folder.
    #[cfg(not(windows))]
    crate::platform::open_url(if path.is_dir() {
        &text
    } else {
        text.rsplit_once('/').map_or(".", |(dir, _)| dir)
    });
}

#[cfg(test)]
pub use crate::scene::ACCURATE_BOUNCES;

/// Quality bundles (UI plan 5.1): each writes `rr`, `scale`, `denoise`, `native_aa` and, for
/// the upscaled native one, `upscaler`. `Realtime` also sets the sampling (spp 1, bounces 3,
/// specular bounces 8, ReSTIR DI on, scale 0.67; engine-notes "Realtime preset"): the only
/// bundle that touches transport, so a scene's own settings are replaced when it is chosen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bundle {
    Realtime,
    Best,
    Balanced,
    Fast,
    Fastest,
    Native,
    NativeFast,
    /// Accuracy bundle (docs/claude/accuracy-audit.md, "Recommended Accurate quality bundle"):
    /// nothing that lowers quality, for stills, exports and the CPU comparison.
    Accurate,
    Reference,
}
impl Bundle {
    pub const ALL: [Bundle; 9] = [
        Bundle::Realtime,
        Bundle::Best,
        Bundle::Balanced,
        Bundle::Fast,
        Bundle::Fastest,
        Bundle::Native,
        Bundle::NativeFast,
        Bundle::Accurate,
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
            Bundle::Realtime => 0.67,
            Bundle::Balanced | Bundle::NativeFast => 2. / 3.,
            Bundle::Fast => 0.5,
            Bundle::Fastest => 1. / 3.,
            _ => 1.,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            // The render scale is part of the name, and no word here is a DLSS mode name
            // ("Balanced" is 58% in the Render tab; "Native" is the denoiser, not 100%).
            Bundle::Realtime => "Realtime 67%",
            Bundle::Best => "Best 100%",
            Bundle::Balanced => "Good 67%",
            Bundle::Fast => "Fast 50%",
            Bundle::Fastest => "Fastest 33%",
            Bundle::Native => "Native denoiser 100%",
            Bundle::NativeFast => "Native denoiser 67%",
            Bundle::Accurate => "Accurate",
            Bundle::Reference => "Reference",
        }
    }
    fn about(self, upscaler: Upscaler) -> String {
        match self {
            Bundle::Realtime => "1 sample, ReSTIR, RR or native filter".into(),
            Bundle::Best => "DLSS Ray Reconstruction, DLAA".into(),
            Bundle::Balanced | Bundle::Fast | Bundle::Fastest => "DLSS Ray Reconstruction".into(),
            Bundle::Native => "no DLSS needed".into(),
            Bundle::NativeFast => format!("upscaled by {}", upscaler.label()),
            Bundle::Accurate => {
                "no denoiser, exact rough glass, mip 0 textures, up to 64 bounces; slower".into()
            }
            Bundle::Reference => "no denoiser (unbiased)".into(),
        }
    }
    /// The bundle the settings match, if any.
    pub fn of(s: &Settings) -> Option<Bundle> {
        let near = |b: Bundle| (s.scale - b.scale()).abs() < 0.005;
        // Realtime shares its scale with Good 67%: it is told apart by its sampling.
        let realtime = s.spp == 1
            && s.bounces == 3
            && s.specular_bounces == 8
            && s.restir
            && s.integrator == 0
            && matches!(s.denoiser, Denoiser::Rr | Denoiser::Native)
            && !s.native_aa
            && near(Bundle::Realtime);
        if realtime {
            return Some(Bundle::Realtime);
        }
        Bundle::ALL.into_iter().find(|b| {
            *b != Bundle::Realtime
                && near(*b)
                && (*b != Bundle::Accurate || Bundle::accurate_settings(s))
                && if b.rr() {
                    s.rr()
                } else {
                    !s.rr()
                        && !s.native_aa
                        && match b {
                            // Any upscaler: without a temporal one it falls back to bicubic.
                            Bundle::Native | Bundle::NativeFast => s.denoiser == Denoiser::Native,
                            _ => s.denoiser == Denoiser::Off,
                        }
                }
        })
    }
    /// The quality settings the Accurate bundle writes besides the output ones (all changeable
    /// one by one afterwards, which leaves the bundle as "Custom").
    fn accurate_settings(s: &Settings) -> bool {
        s.accurate_quality()
    }
    /// `rr_ok`: RR can run here; Realtime takes the native filter when it cannot.
    pub fn apply(self, s: &mut Settings, upscaler: Upscaler, rr_ok: bool) {
        // Realtime's marker is read before Accurate's set or clear changes bounces and ReSTIR.
        let was_realtime = s.spp == 1 && s.bounces == 3 && s.specular_bounces == 8 && s.restir;
        // Accurate first: leaving it puts bounces and ReSTIR back to the defaults, and Realtime
        // (below) then sets its own.
        if self == Bundle::Accurate {
            s.set_accurate_quality();
        } else if s.accurate_quality() {
            // Coming from Accurate: its quality fields do not carry into another bundle.
            s.clear_accurate_quality();
        }
        if self == Bundle::Realtime {
            s.spp = 1;
            s.bounces = 3;
            s.specular_bounces = 8;
            s.restir = true;
            // specular_bounces only acts in the path tracer (0 = path).
            s.integrator = 0;
        } else if was_realtime {
            // Leaving Realtime: drop its marker so the menu names the bundle just chosen.
            s.specular_bounces = 0;
        }
        s.denoiser = match self {
            Bundle::Realtime if !rr_ok => Denoiser::Native,
            _ if self.rr() => Denoiser::Rr,
            Bundle::Accurate | Bundle::Reference => Denoiser::Off,
            _ => Denoiser::Native,
        };
        s.scale = self.scale();
        s.native_aa = false;
        if self == Bundle::NativeFast || (self == Bundle::Realtime && !rr_ok) {
            s.upscaler = upscaler;
        }
    }
}

#[cfg(test)]
mod tests {
    /// One frame of `ui`, with `events`, as `redraw` runs it: Tab filtered, focus settled after.
    fn focus_frame(
        ctx: &egui::Context,
        text_focus: &mut bool,
        seen: &mut Option<egui::Id>,
        transform: bool,
        events: Vec<egui::Event>,
        mut ui: impl FnMut(&mut egui::Ui),
    ) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        };
        keep_tab_for_app(&mut input, *text_focus && !transform);
        let output = discard(ctx.run_ui(input, |root| {
            egui::CentralPanel::default().show(root, |u| ui(u));
        }));
        *text_focus = settle_focus(ctx, output.platform_output.ime.is_some(), seen);
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn tab_never_puts_a_button_in_focus() {
        let ctx = egui::Context::default();
        let (mut text, mut seen) = (false, None);
        for _ in 0..4 {
            focus_frame(
                &ctx,
                &mut text,
                &mut seen,
                false,
                vec![key(egui::Key::Tab)],
                |ui| {
                    let _ = crate::ui::skin::button(ui, "Menu");
                },
            );
            assert_eq!(ctx.memory(|m| m.focused()), None);
            assert!(!text && seen.is_none(), "the shortcuts stay alive");
        }
    }
    #[test]
    fn text_fields_keep_the_keyboard_and_tab_between_them() {
        let ctx = egui::Context::default();
        let (mut text, mut seen) = (false, None);
        let (mut a, mut b) = (String::new(), String::new());
        let (a_id, b_id) = (egui::Id::unique("a"), egui::Id::unique("b"));
        let mut go = |text: &mut bool, seen: &mut _, transform, events, focus| {
            focus_frame(&ctx, text, seen, transform, events, |ui| {
                let ra = ui.add(egui::TextEdit::singleline(&mut a).id(a_id));
                if focus {
                    ra.request_focus();
                }
                ui.add(egui::TextEdit::singleline(&mut b).id(b_id));
            });
        };
        go(&mut text, &mut seen, false, vec![], true);
        go(&mut text, &mut seen, false, vec![], false);
        assert!(text, "a focused text field holds the keyboard");
        go(
            &mut text,
            &mut seen,
            false,
            vec![egui::Event::Text("x".into())],
            false,
        );
        go(
            &mut text,
            &mut seen,
            false,
            vec![key(egui::Key::Tab)],
            false,
        );
        go(&mut text, &mut seen, false, vec![], false);
        assert_eq!(ctx.memory(|m| m.focused()), Some(b_id));
        assert!(text, "Tab went on to the next field");
        // During a transform Tab is the transform's.
        go(&mut text, &mut seen, true, vec![key(egui::Key::Tab)], false);
        assert_eq!(ctx.memory(|m| m.focused()), Some(b_id));
        // Esc lets go, and the shortcuts are back.
        go(
            &mut text,
            &mut seen,
            false,
            vec![key(egui::Key::Escape)],
            false,
        );
        go(&mut text, &mut seen, false, vec![], false);
        assert!(!text && seen.is_none());
        assert_eq!(a, "x");
    }
    #[test]
    fn a_drag_value_can_still_be_typed_in() {
        let ctx = egui::Context::default();
        let (mut text, mut seen) = (false, None);
        let mut value = 1.0f32;
        let mut rect = egui::Rect::NOTHING;
        let mut go = |text: &mut bool, seen: &mut _, events| {
            focus_frame(&ctx, text, seen, false, events, |ui| {
                rect = ui.add(egui::DragValue::new(&mut value)).rect;
            });
            rect.center()
        };
        let at = go(&mut text, &mut seen, vec![]);
        go(&mut text, &mut seen, vec![egui::Event::PointerMoved(at)]);
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        go(&mut text, &mut seen, vec![click(true)]);
        go(&mut text, &mut seen, vec![click(false)]);
        assert!(seen.is_some(), "the shortcuts wait for the field");
        go(&mut text, &mut seen, vec![]);
        assert!(text, "the click opened its field, which holds the keyboard");
        go(&mut text, &mut seen, vec![egui::Event::Text("7".into())]);
        go(&mut text, &mut seen, vec![key(egui::Key::Enter)]);
        go(&mut text, &mut seen, vec![]);
        assert_eq!(value, 7.);
        assert!(!text && seen.is_none(), "Enter gives the keys back");
    }

    use super::*;
    /// A live state with no session file, so tests touch nothing on disk.
    fn live(s: &Settings) -> UiState {
        let mut ui = UiState::default();
        ui.edits_live = true;
        ui.edit_base = s.clone();
        ui.clean_base = s.clone();
        ui
    }
    /// A frame with a release or key: nothing held, nothing else writing.
    fn released(ui: &mut UiState, s: &Settings) {
        ui.watch_edits(s, true, true, false);
    }
    #[test]
    fn one_drag_is_one_undo_step_and_undo_keeps_the_camera() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        let before = s.exposure;
        // During the drag: the pointer is down, so nothing is recorded.
        for i in 1..5 {
            s.exposure = before + i as f32 * 0.1;
            ui.watch_edits(&s, false, false, false);
        }
        assert!(ui.undo.is_empty());
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
        let after = s.exposure;
        // Orbiting makes no step, and undo leaves the view alone.
        s.yaw += 40.;
        s.target[0] += 1.;
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
        let camera = camera_of(&s);
        assert!(ui.undo(&mut s));
        assert_eq!(s.exposure, before);
        assert_eq!(camera_of(&s), camera);
        // The watcher does not see undo as an edit: redo survives it.
        released(&mut ui, &s);
        assert!(ui.redo(&mut s));
        assert_eq!(s.exposure, after);
        assert_eq!(camera_of(&s), camera);
        assert!(ui.undo.len() == 1 && ui.redo.is_empty());
    }
    #[test]
    fn an_action_with_its_own_checkpoint_stays_one_step() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        // Orbit first: the action's snapshot and the watcher's base differ only in camera.
        s.yaw += 10.;
        ui.watch_edits(&s, false, true, false);
        ui.checkpoint(&s);
        s.objects.pop();
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
        // A held text field waits, and the later settled frame picks it up.
        s.scene_name = "Renamed".into();
        ui.watch_edits(&s, true, false, false);
        assert_eq!(ui.undo.len(), 1);
        ui.watch_edits(&s, false, true, false);
        assert_eq!(ui.undo.len(), 2);
    }
    #[test]
    fn a_scene_step_brings_its_own_camera_back() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        s.exposure = 3.;
        s.yaw = 25.;
        released(&mut ui, &s);
        let old = s.clone();
        ui.checkpoint_scene(&s);
        s.preset(12);
        s.yaw = 99.;
        // The watcher's own look at the load merges with the load's step.
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 2);
        assert!(ui.undo(&mut s));
        assert!(s == old, "the old scene, framed as it was");
        assert!(ui.redo(&mut s));
        assert_eq!(s.yaw, 99.);
    }
    #[test]
    fn a_camera_inspector_edit_is_undone_with_its_camera() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        let yaw = s.yaw;
        ui.watch_edits(&s, false, true, false);
        s.yaw = yaw + 30.;
        ui.camera_edit = true;
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
        assert!(ui.undo(&mut s));
        assert_eq!(s.yaw, yaw);
    }
    #[test]
    fn what_keys_physics_and_transforms_write_makes_no_step() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        // A simulation or playback runs, and ends with no input.
        s.objects[0].position[0] += 1.;
        ui.watch_edits(&s, true, true, true);
        s.objects[0].position[0] += 1.;
        ui.watch_edits(&s, false, true, false);
        released(&mut ui, &s);
        assert!(ui.undo.is_empty());
        // Keys applied after the UI.
        s.exposure += 1.;
        ui.rebase_edits(&s);
        released(&mut ui, &s);
        assert!(ui.undo.is_empty());
        // The next real edit counts.
        s.exposure += 1.;
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
    }
    #[test]
    fn panel_edits_make_steps_while_a_simulation_moves_objects() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        ui.sim_objects = vec![0];
        // The simulation's poses alone: no step, and nothing unsaved, a viewer choice included.
        s.objects[0].position[0] += 1.;
        s.view = 2;
        released(&mut ui, &s);
        assert!(ui.undo.len() == 1 && !ui.scene_dirty, "the view is a step");
        ui.undo.clear();
        s.objects[0].position[0] += 1.;
        released(&mut ui, &s);
        assert!(ui.undo.is_empty() && !ui.scene_dirty);
        // A panel edit counts.
        s.objects[0].position[0] += 1.;
        s.exposure += 1.;
        released(&mut ui, &s);
        assert!(ui.undo.len() == 1 && ui.scene_dirty);
        assert_eq!(ui.undo[0].settings.exposure + 1., s.exposure);
        // It stopped: what it left is no edit either.
        s.objects[0].position[0] += 1.;
        ui.sim_ended(&s);
        released(&mut ui, &s);
        assert!(ui.undo.len() == 1 && ui.sim_objects.is_empty());
    }
    #[test]
    fn a_recording_is_unsaved_but_no_step() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        s.animation.key("exposure", 0., 1.);
        ui.watch_edits(&s, false, true, true);
        released(&mut ui, &s);
        assert!(ui.undo.is_empty() && ui.scene_dirty);
    }
    #[test]
    fn keys_applied_for_a_load_or_the_playhead_leave_a_clean_scene_clean() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        // A load: the keys' first values are no work of the user's.
        ui.mark_clean(&s);
        s.exposure += 1.;
        ui.keys_applied(&s, true, true);
        s.view = 2;
        released(&mut ui, &s);
        assert!(!ui.scene_dirty);
        // A scrub, then a viewer choice.
        s.exposure += 1.;
        ui.keys_applied(&s, true, false);
        s.view = 3;
        released(&mut ui, &s);
        assert!(!ui.scene_dirty);
        // A real edit, and its undo after another scrub: not clean by accident.
        ui.undo.clear();
        s.gamma += 0.5;
        released(&mut ui, &s);
        assert!(ui.scene_dirty);
        s.exposure += 1.;
        ui.keys_applied(&s, true, false);
        assert!(ui.scene_dirty);
    }
    #[test]
    fn an_edit_of_the_keys_stays_an_edit() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        // Its action's checkpoint, then the applied keys, a button keeping the focus.
        ui.checkpoint(&s);
        s.animation.key("exposure", 0., 3.);
        s.exposure = 3.;
        ui.watch_edits(&s, true, false, false);
        ui.keys_applied(&s, true, true);
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1, "one step");
        assert!(ui.scene_dirty);
    }
    #[test]
    fn an_edit_in_a_field_survives_the_playhead_moving() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        s.gamma += 0.5;
        ui.watch_edits(&s, true, false, false);
        assert!(ui.undo.is_empty());
        ui.flush_pending(&s);
        s.exposure += 1.;
        ui.keys_applied(&s, true, false);
        released(&mut ui, &s);
        assert_eq!(ui.undo.len(), 1);
        assert!(ui.scene_dirty);
        // Nor does playback starting swallow one.
        s.gamma += 0.5;
        ui.watch_edits(&s, true, false, false);
        ui.watch_edits(&s, true, true, true);
        assert_eq!(ui.undo.len(), 2);
    }
    #[test]
    fn nothing_is_watched_outside_interactive_runs() {
        let mut s = Settings::default();
        let mut ui = UiState::default();
        s.exposure += 1.;
        ui.watch_edits(&s, true, true, false);
        assert!(ui.undo.is_empty() && !ui.scene_dirty);
        ui.close_session(&s);
        ui.mark_clean(&s);
        assert!(ui.edit_base != s, "no clones for headless runs either");
    }
    #[test]
    fn the_unsaved_mark_ignores_the_camera_and_viewer_choices() {
        let mut s = Settings::default();
        let mut ui = live(&s);
        s.yaw += 5.;
        s.view = 2;
        s.denoiser = s.denoiser.with_rr(!s.rr());
        released(&mut ui, &s);
        assert!(!ui.scene_dirty);
        s.exposure += 1.;
        released(&mut ui, &s);
        assert!(ui.scene_dirty);
        // Undo back: clean again. Saving or loading clears it.
        assert!(ui.undo(&mut s));
        assert!(ui.undo(&mut s));
        released(&mut ui, &s);
        assert!(!ui.scene_dirty);
        s.exposure += 1.;
        released(&mut ui, &s);
        assert!(ui.scene_dirty);
        ui.mark_clean(&s);
        assert!(!ui.scene_dirty);
        released(&mut ui, &s);
        assert!(!ui.scene_dirty);
    }
    #[test]
    fn closing_writes_unsaved_work_and_clears_the_file_otherwise() {
        let dir = std::env::temp_dir().join(format!("caustica-close-{}", std::process::id()));
        let path = dir.join("last-session.json");
        let mut s = Settings::default();
        let mut ui = live(&s);
        ui.session_file = Some(path.clone());
        // A value still in a focused field at close counts.
        s.exposure = 2.5;
        ui.watch_edits(&s, true, false, false);
        ui.close_session(&s);
        assert_eq!(
            crate::prefs::read_session(&path).unwrap().settings.exposure,
            2.5
        );
        // Saved since: nothing left to offer.
        ui.mark_clean(&s);
        assert!(!path.exists());
        // But an unanswered offer keeps its file through a clean close.
        crate::prefs::write_session(&path, &s, &None).unwrap();
        ui.session_offer = crate::prefs::read_session(&path);
        ui.close_session(&s);
        assert!(path.exists());
        ui.session_offer = None;
        ui.close_session(&s);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn a_nameless_scene_gets_no_leading_dash() {
        let mut ui = UiState::default();
        ui.prefs.name_template = "{scene}-x".into();
        let mut s = Settings::default();
        s.scene_name = " ".into();
        assert_eq!(ui.export_stem(&s, 1, 1, 1), "caustica-x");
    }

    /// One headless egui frame of `add` in a grid, with `events` as this frame's input.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        add: &mut dyn FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        let time = ctx.input(|i| i.time) + 0.05;
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        };
        discard(ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::Grid::new("test")
                    .num_columns(2)
                    .show(ui, |ui| add(ui));
            });
        }))
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    /// Moves there and clicks, one input per frame.
    fn click(ctx: &egui::Context, pos: egui::Pos2, add: &mut dyn FnMut(&mut egui::Ui)) {
        frame(ctx, vec![egui::Event::PointerMoved(pos)], add);
        frame(ctx, vec![button(pos, true)], add);
        frame(ctx, vec![button(pos, false)], add);
    }
    /// Every piece of text painted in a frame, tooltips included.
    fn painted(output: &egui::FullOutput) -> String {
        fn walk(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    text.push_str(t.galley.text());
                    text.push('\n');
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text)),
                _ => {}
            }
        }
        let mut text = String::new();
        output.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        text
    }
    /// A context whose tooltips show at once, and what a few frames hovering `pos` paint.
    fn hover(pos: egui::Pos2, add: &mut dyn FnMut(&mut egui::Ui)) -> String {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|s| s.interaction.tooltip_delay = 0.);
        frame(&ctx, vec![], add);
        frame(&ctx, vec![egui::Event::PointerMoved(pos)], add);
        for _ in 0..3 {
            frame(&ctx, vec![], add);
        }
        painted(&frame(&ctx, vec![], add))
    }
    #[test]
    fn a_rows_label_toggles_its_checkbox_and_reports_the_change() {
        let ctx = egui::Context::default();
        let mut on = false;
        let box_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut changed = 0;
        let mut add = |ui: &mut egui::Ui| {
            let r = check_row(ui, "Spectral", "tip", &mut on);
            box_rect.set(r.rect);
            changed += r.changed() as u32;
        };
        frame(&ctx, vec![], &mut add);
        let label = egui::pos2(box_rect.get().left() - 25., box_rect.get().center().y);
        click(&ctx, label, &mut add);
        drop(add);
        assert!(on, "a click on the label toggles");
        assert_eq!(changed, 1, "and changed() fires once");
    }
    #[test]
    fn the_row_tooltip_does_not_take_a_controls_clicks() {
        let ctx = egui::Context::default();
        let mut on = false;
        let box_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut add = |ui: &mut egui::Ui| {
            box_rect.set(check_row(ui, "Spectral", "tip", &mut on).rect);
        };
        frame(&ctx, vec![], &mut add);
        let center = box_rect.get().center();
        click(&ctx, center, &mut add);
        drop(add);
        assert!(on, "the box itself still toggles");
    }
    #[test]
    fn a_disabled_check_row_ignores_its_label_and_says_why() {
        let ctx = egui::Context::default();
        let mut on = false;
        let box_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut add = |ui: &mut egui::Ui| {
            box_rect.set(
                check_row_with(
                    ui,
                    "Denoiser",
                    "the row tip",
                    &mut on,
                    Some("the reason"),
                    |_, _| {},
                )
                .rect,
            );
        };
        frame(&ctx, vec![], &mut add);
        let (center, label) = (
            box_rect.get().center(),
            egui::pos2(box_rect.get().left() - 25., box_rect.get().center().y),
        );
        click(&ctx, label, &mut add);
        click(&ctx, center, &mut add);
        drop(add);
        assert!(!on);
        let mut on = false;
        let text = hover(center, &mut |ui: &mut egui::Ui| {
            check_row_with(
                ui,
                "Denoiser",
                "the row tip",
                &mut on,
                Some("the reason"),
                |_, _| {},
            );
        });
        assert!(text.contains("the reason"), "{text}");
        assert!(!text.contains("the row tip"), "only one tooltip: {text}");
    }
    #[test]
    fn the_row_tooltip_shows_over_the_control_and_takes_the_slider_hint() {
        let ctx = egui::Context::default();
        let mut v = 0.5_f32;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut add = |ui: &mut egui::Ui| {
            row(ui, "Level", "What the level does.", |ui| {
                rect.set(ui.add(crate::slider(&mut v, 0. ..=1.)).rect);
            });
        };
        frame(&ctx, vec![], &mut add);
        drop(add);
        let text = hover(rect.get().center(), &mut |ui: &mut egui::Ui| {
            row(ui, "Level", "What the level does.", |ui| {
                ui.add(crate::slider(&mut v, 0. ..=1.));
            });
        });
        assert!(text.contains("What the level does."), "{text}");
        assert!(text.contains("Double-click the slider to reset."), "{text}");
    }
    #[test]
    fn a_controls_own_tooltip_replaces_the_rows() {
        let ctx = egui::Context::default();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut add = |ui: &mut egui::Ui| {
            row(ui, "HDRI", "the row tip", |ui| {
                rect.set(own_tip(ui.label("file.hdr"), "the path").rect);
            });
        };
        frame(&ctx, vec![], &mut add);
        let text = hover(rect.get().center(), &mut |ui: &mut egui::Ui| {
            row(ui, "HDRI", "the row tip", |ui| {
                own_tip(ui.label("file.hdr"), "the path");
            });
        });
        assert!(text.contains("the path"), "{text}");
        assert!(!text.contains("the row tip"), "{text}");
    }
    #[test]
    fn the_revert_dot_restores_a_group_of_values() {
        let reference = [1_f32, 2., 3.];
        let mut live = [1_f32, 5., 3.];
        crate::clear_reset_sources();
        crate::reset_source(&live, &reference);
        let ctx = egui::Context::default();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut add = |ui: &mut egui::Ui| {
            rect.set(ui.max_rect());
            crate::revert_dot(ui, &mut live);
        };
        frame(&ctx, vec![], &mut add);
        let dot = rect.get().min + egui::vec2(4., 8.);
        click(&ctx, dot, &mut add);
        drop(add);
        crate::clear_reset_sources();
        assert_eq!(live, reference);
    }
    #[test]
    fn double_clicking_a_wheels_level_slider_resets_the_wheel() {
        let ctx = egui::Context::default();
        let mut value = [0.1_f32, 0.05, 0.05];
        let reset = std::cell::Cell::new(false);
        // Not in `frame`'s grid: the wheels are laid out across the panel's width.
        let mut run = |events: Vec<egui::Event>| {
            let time = ctx.input(|i| i.time) + 0.05;
            let input = egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(330., 600.),
                )),
                events,
                ..Default::default()
            };
            let _ = discard(ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    crate::look::color_wheels(
                        ui,
                        &mut [crate::look::Wheel {
                            label: "Lift",
                            value: &mut value,
                            center: 0.,
                            range: 0.15,
                        }],
                    );
                    reset.set(value.iter().all(|v| v.abs() < 1e-6));
                });
            }));
        };
        run(vec![]);
        run(vec![]);
        // The level's row sits under the wheel, its slider after the label column: its height
        // is found by trying rows from the top.
        for y in (100..260).step_by(4) {
            let slider = egui::pos2(LABEL_WIDTH + 48., y as f32);
            let button = |pressed| egui::Event::PointerButton {
                pos: slider,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            for _ in 0..2 {
                run(vec![egui::Event::PointerMoved(slider)]);
                run(vec![button(true)]);
                run(vec![button(false)]);
                run(vec![]);
            }
            if reset.get() {
                break;
            }
        }
        assert!(reset.get());
    }
    #[test]
    fn short_numbers_drop_what_says_nothing() {
        assert_eq!(short_number(0.), "0");
        assert_eq!(short_number(0.012), "0.012");
        assert_eq!(short_number(0.00043216), "0.0004322");
        assert_eq!(short_number(1.5), "1.5");
        assert_eq!(short_number(10.), "10");
        assert_eq!(short_number(0.1 + 0.2), "0.3");
    }
}
