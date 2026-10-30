//! The interface (UI plan phase 1): top bar, a right side panel with Scene / Render / Look /
//! Export tabs, a status bar and toasts. UI-only state lives in `UiState`, never in `Settings`,
//! because every `Settings` field outside the exclusion list restarts the render.
use crate::scene::{self, DLSS_PRESETS, PRESETS, Settings, Shape, Upscaler};
use crate::{App, editor, gizmo};
use std::time::{Duration, Instant};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 202, 216);
const WARN: egui::Color32 = egui::Color32::from_rgb(232, 176, 72);
const VIEWS: [&str; 12] = [
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
    /// Scenes window: the scene files found on disk, filters, selection and the save form.
    pub show_scenes: bool,
    /// Look tools (roadmap 5b).
    pub zebras: bool,
    pub show_scopes: bool,
    pub meter: bool,
    pub show_graph: bool,
    pub show_stats: bool,
    scope_images: Option<(egui::TextureHandle, egui::TextureHandle)>,
    /// Convergence graph: reference image (width, height, RGBA), points (samples, RMSE x1000).
    pub reference: Option<(u32, u32, Vec<f32>)>,
    pub reference_path: String,
    pub graph: Vec<(u32, f64)>,
    /// Thumbnails: loaded textures by file, the background render (results, total, done) and
    /// whether this session already asked for the missing ones.
    pub thumbnails: std::collections::HashMap<std::path::PathBuf, egui::TextureHandle>,
    thumbnail_jobs: Option<ThumbnailJobs>,
    thumbnails_requested: bool,
    pub scenes: Vec<crate::scenes::Entry>,
    scene_search: String,
    scene_tag: Option<String>,
    pub scene_selected: Option<usize>,
    save_title: String,
    save_description: String,
    toasts: Vec<Toast>,
    /// Transform gizmo: active tool, Global / Local axes, handle under the pointer, and the
    /// running transform (handle drag or modal G / R / S) with its header text.
    pub tool: gizmo::Tool,
    pub local: bool,
    pub hover: Option<gizmo::Handle>,
    pub transform: Option<gizmo::Transform>,
    pub header: String,
    /// Snapshots for Ctrl+Z / Ctrl+Y, taken before each edit that the UI makes.
    undo: Vec<Settings>,
    redo: Vec<Settings>,
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
            show_scenes: false,
            zebras: false,
            show_scopes: false,
            meter: false,
            show_graph: false,
            show_stats: false,
            scope_images: None,
            reference: None,
            reference_path: "captures/reference.exr".into(),
            graph: Vec::new(),
            thumbnails: Default::default(),
            thumbnail_jobs: None,
            thumbnails_requested: false,
            scenes: crate::scenes::list(),
            scene_search: String::new(),
            scene_tag: None,
            scene_selected: None,
            save_title: String::new(),
            save_description: String::new(),
            toasts: Vec::new(),
            tool: gizmo::Tool::Move,
            local: false,
            hover: None,
            transform: None,
            header: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}
impl UiState {
    pub fn toast(&mut self, text: impl Into<String>) {
        self.push(text.into(), false);
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), true);
    }
    fn push(&mut self, text: String, error: bool) {
        let life = if error { 8. } else { 3. };
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast {
            text,
            error,
            until: Instant::now() + Duration::from_secs_f32(life),
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
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
    /// Deletes the selected object or area light; returns its name.
    pub fn delete_selected(&mut self, s: &mut Settings) -> Option<String> {
        if matches!(self.selection, Selection::Object(i) if i < s.objects.len())
            || matches!(self.selection, Selection::Light(i) if i < s.lights.len())
        {
            self.checkpoint(s);
        }
        match self.selection {
            Selection::Object(i) if i < s.objects.len() => {
                let name = s.objects.remove(i).name;
                self.selection = if s.objects.is_empty() {
                    Selection::Stage
                } else {
                    Selection::Object(i.min(s.objects.len() - 1))
                };
                Some(name)
            }
            Selection::Light(i) if i < s.lights.len() => {
                let name = s.lights.remove(i).name;
                self.selection = Selection::MainLight;
                Some(name)
            }
            _ => None,
        }
    }
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
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.);
    ui.label(egui::RichText::new(title).strong().color(ACCENT));
}
fn grid(id: &str) -> egui::Grid {
    egui::Grid::new(id).num_columns(2).spacing([12., 6.])
}
fn note(ui: &mut egui::Ui, text: &str) {
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
fn placeholder(ui: &mut egui::Ui, title: &str, size: egui::Vec2) -> egui::Response {
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
            "{name}: {windows} · HDR output, {:.0} nits peak, paper white {:.0} nits",
            g.hdr_levels[1], g.hdr_levels[0]
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
        // Loaded and randomized settings carry their preset by name.
        let known = PRESETS.iter().position(|n| *n == self.settings.scene_name);
        if let Some(i) = known {
            self.preset = i;
        }
        self.top_bar(ctx, known.is_some());
        if !self.ui.panel_hidden {
            self.status_bar(ctx);
            self.side_panel(ctx, known.is_some());
        }
        self.scenes_window(ctx);
        self.tool_windows(ctx);
        self.overlay(ctx);
        self.toasts(ctx);
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
        match crate::scenes::load_into(&mut self.settings, &e.path) {
            Ok(()) => {
                self.ui.selection = Selection::Stage;
                self.ui.scene_selected = Some(index);
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
                let visible: Vec<usize> = (0..state.scenes.len())
                    .filter(|i| {
                        let e = &state.scenes[*i];
                        (e.title.to_lowercase().contains(&search)
                            || e.description.to_lowercase().contains(&search)
                            || e.tags.iter().any(|t| t.contains(&search)))
                            && state.scene_tag.as_ref().is_none_or(|t| e.tags.contains(t))
                    })
                    .collect();
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
                if let Some(e) = state.scene_selected.and_then(|i| state.scenes.get(i)) {
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
                            if ui.button("Load (or double-click)").clicked() {
                                load = state.scene_selected;
                            }
                        });
                    });
                } else {
                    note(ui, "Pick a scene; double-click loads it. Your render settings (resolution, RR, upscaler, HDR) stay as they are.");
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
                load = ui.button("Load EXR").clicked();
                use_current = ui.button("Use current image").on_hover_text(format!("{} spp now", g.samples)).clicked();
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(300.), 200.), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2., egui::Color32::from_gray(16));
            let points = &self.ui.graph;
            if points.len() >= 2 {
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
        gizmo::draw_lights(&painter, ppp, &view, &self.settings, self.ui.selection);
        self.ui.hover = None;
        if let Some(t) = &self.ui.transform {
            if let Some(f) = gizmo::frame(&t.original, t.target) {
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
                        note(ui, "X / Y / Z axis (again: local, again: free) · Shift+X/Y/Z plane · type a value · Ctrl snap · Shift fine · click or Enter to confirm · right-click or Esc to cancel");
                    });
                });
            return;
        }
        let mode = self.tool_mode();
        // The gizmo's projection math assumes a perspective camera.
        if self.ui.tool == gizmo::Tool::Select
            || !gizmo::allows(self.ui.selection, mode)
            || !self.settings.perspective()
        {
            return;
        }
        let Some(f) = gizmo::frame(&self.settings, self.ui.selection) else {
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
        let sel = self.ui.selection;
        if !gizmo::allows(sel, mode) || gizmo::frame(&self.settings, sel).is_none() {
            return false;
        }
        let mouse = glam::Vec2::new(self.cursor[0] as f32, self.cursor[1] as f32);
        let t = match handle {
            Some(h) => {
                gizmo::Transform::from_handle(mode, h, sel, mouse, &self.settings, self.ui.local)
            }
            None => gizmo::Transform::new(mode, sel, mouse, &self.settings, self.ui.local),
        };
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
            }
            self.ui.header.clear();
        }
    }
    pub fn cancel_transform(&mut self) {
        if let Some(t) = self.ui.transform.take() {
            t.restore(&mut self.settings);
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
            ui.horizontal(|ui| {
                ui.menu_button("Menu", |ui| {
                    ui.checkbox(&mut self.ui.advanced, "Show advanced settings");
                    if ui.button("Statistics and achievements").clicked() {
                        self.ui.show_stats = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(!self.ui.undo.is_empty(), egui::Button::new("Undo (Ctrl+Z)")).clicked() {
                        self.ui.undo(&mut self.settings);
                        ui.close_menu();
                    }
                    if ui.add_enabled(!self.ui.redo.is_empty(), egui::Button::new("Redo (Ctrl+Y)")).clicked() {
                        self.ui.redo(&mut self.settings);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save scene (settings.json)").clicked() {
                        self.actions.save = true;
                        ui.close_menu();
                    }
                    if ui.button("Load scene (settings.json)").clicked() {
                        self.actions.load = true;
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
                        "Ctrl+Z / Ctrl+Y: undo / redo",
                        "Space: pause · R: restart (nothing selected) · F: frame the scene · A: auto orbit",
                        "D: native denoiser · C: freeze A/B reference · 1–8: views",
                        "F12: save PNG · Tab: hide panels · Ctrl +/−: UI size",
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
                let count = PRESETS.len();
                // ◀ ▶ step through the scene files while one is loaded, else the presets.
                let file = self.scene_entry();
                if ui.button("◀").on_hover_text("Previous scene").clicked() {
                    match file {
                        Some(i) => self.load_scene_file((i + self.ui.scenes.len() - 1) % self.ui.scenes.len()),
                        None => {
                            self.preset = (self.preset + count - 1) % count;
                            self.settings.preset(self.preset);
                        }
                    }
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
                if ui.button("▶").on_hover_text("Next scene").clicked() {
                    match file {
                        Some(i) => self.load_scene_file((i + 1) % self.ui.scenes.len()),
                        None => {
                            self.preset = (self.preset + 1) % count;
                            self.settings.preset(self.preset);
                        }
                    }
                }
                if ui
                    .selectable_label(self.ui.show_scenes, format!("Scenes ({})", self.ui.scenes.len()))
                    .on_hover_text("Browse the scene files in scenes/")
                    .clicked()
                {
                    self.ui.show_scenes = !self.ui.show_scenes;
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
                ui.separator();
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
                ui.separator();
                for (tool, label, tip) in [
                    (gizmo::Tool::Select, "Select", "Click to select, no handles"),
                    (gizmo::Tool::Move, "Move", "Arrows and planes. Key: G"),
                    (gizmo::Tool::Rotate, "Rotate", "Rings. Key: R"),
                    (gizmo::Tool::Scale, "Scale", "Axis handles; the circle scales evenly. Key: S"),
                ] {
                    ui.selectable_value(&mut self.ui.tool, tool, label).on_hover_text(tip);
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
                ui.separator();
                ui.monospace(format!("{:.2} ms GPU", g.gpu_ms));
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
            .interactable(false)
            .show(ctx, |ui| {
                for t in self.ui.toasts.iter().rev() {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(if t.error {
                            egui::RichText::new(&t.text).color(WARN)
                        } else {
                            egui::RichText::new(&t.text)
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
    fn scene_tab(&mut self, ui: &mut egui::Ui, known: bool) {
        let description = if known {
            crate::presets::description(self.preset).to_string()
        } else if let Some(i) = self.scene_entry() {
            self.ui.scenes[i].description.clone()
        } else {
            "Custom scene from a settings file.".to_string()
        };
        let g = self.gpu.as_ref().unwrap();
        let aspect = g.width as f32 / g.height as f32;
        let s = &mut self.settings;
        let state = &mut self.ui;
        note(ui, &description);
        section(ui, "Outliner");
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
                    ui.selectable_value(&mut state.selection, Selection::MainLight, "Main light");
                });
                for (i, l) in s.lights.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.push_id(("light", i), |ui| {
                            ui.checkbox(&mut l.enabled, "").on_hover_text("On / off");
                        });
                        ui.selectable_value(
                            &mut state.selection,
                            Selection::Light(i),
                            format!("Light · {}", l.name),
                        );
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
                        let r = ui.selectable_value(
                            &mut state.selection,
                            Selection::Object(i),
                            &o.name,
                        );
                        if r.double_clicked() {
                            state.selection = Selection::Object(i);
                        }
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
                    s.lights.len() < scene::MAX_LIGHTS,
                    egui::Button::new("+ Light"),
                )
                .on_hover_text("Add an area light")
                .clicked()
            {
                state.checkpoint(s);
                s.lights.push(scene::AreaLight {
                    name: format!("Softbox {}", s.lights.len() + 1),
                    ..Default::default()
                });
                state.selection = Selection::Light(s.lights.len() - 1);
            }
        });
        // A selection can outlive its item (preset switch, load, undo-free deletes).
        state.selection = match state.selection {
            Selection::Object(i) if i >= s.objects.len() => Selection::Stage,
            Selection::Light(i) if i >= s.lights.len() => Selection::MainLight,
            other => other,
        };
        ui.separator();
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
                            egui::Button::new("Duplicate"),
                        )
                        .clicked()
                    {
                        state.checkpoint(s);
                        let mut o = s.objects[i].clone();
                        o.name = format!("{} copy", o.name.chars().take(110).collect::<String>());
                        o.position[0] += 0.25;
                        o.position[1] += 0.15;
                        s.objects.push(o);
                        state.selection = Selection::Object(s.objects.len() - 1);
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
        ui.add_enabled(g.rr_available(), egui::Checkbox::new(&mut s.rr, "DLSS Ray Reconstruction"))
            .on_hover_text("NVIDIA's neural denoiser and upscaler in one. When off, the native denoiser and the upscaler below are used.")
            .on_disabled_hover_text(&g.rr_status);
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
                                ui.add_enabled_ui(g.upscaler_available(u), |ui| {
                                    ui.selectable_value(&mut s.upscaler, u, u.label())
                                        .on_disabled_hover_text("Not available on this system");
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
                    ui.add(crate::slider(&mut s.spp, 1..=16).suffix(" spp"));
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
                row(ui, "Glass casts shadows", "Off lets light through glass for shadow rays, a common shortcut.", |ui| {
                    ui.checkbox(&mut s.glass_shadow, "");
                });
                row(ui, "VSync", "Wait for the display between frames.", |ui| {
                    ui.checkbox(&mut s.vsync, "");
                });
            });
        });
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
                    if ui
                        .add(
                            crate::slider(&mut ev, -6. ..=3.32)
                                .suffix(" EV")
                                .fixed_decimals(1),
                        )
                        .changed()
                    {
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
        });
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
        section(ui, "Save the current image");
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("PNG (F12)")
                .on_hover_text("cornell.png: the tone-mapped image, without the UI")
                .clicked()
            {
                self.actions.png = true;
            }
            if ui
                .button("HDR EXR")
                .on_hover_text("cornell.exr: linear scene light, 32-bit float")
                .clicked()
            {
                self.actions.exr = true;
            }
            if g.hdr_active
                && ui
                    .button("HDR PNG")
                    .on_hover_text(
                        "cornell-hdr.png: what the HDR screen shows, 16-bit PQ / BT.2020",
                    )
                    .clicked()
            {
                self.actions.hdr_png = true;
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
                "Multiple of the window size, up to 4096 pixels on the long side.",
                |ui| {
                    ui.add(crate::slider(&mut self.ui.export_scale, 1..=4).suffix("×"));
                },
            );
            row(ui, "Samples", "Samples per pixel for the export.", |ui| {
                ui.add(
                    crate::slider(&mut self.ui.export_samples, 16..=2048)
                        .logarithmic(true)
                        .suffix(" spp"),
                );
            });
        });
        let factor = (self.ui.export_scale as f32).min(4096. / g.width.max(g.height) as f32);
        if ui.button("Render high-resolution PNG").clicked() {
            self.actions.high_res = true;
        }
        note(
            ui,
            &format!(
                "cornell-high-res.png, {} × {}, traced fresh at 100% resolution. The window waits until it is done. For larger images use the command line (--width, --height, --frames, --capture).",
                (g.width as f32 * factor).round(),
                (g.height as f32 * factor).round()
            ),
        );
        section(ui, "Scene file");
        ui.horizontal(|ui| {
            if ui.button("Save settings.json").clicked() {
                self.actions.save = true;
            }
            if ui.button("Load settings.json").clicked() {
                self.actions.load = true;
            }
        });
    }
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
    if !s.perspective() {
        note(
            ui,
            "This camera model renders with the native denoiser; RR, temporal upscalers and the gizmo are off. A 360° panorama exports best at 2:1 (e.g. --width 4096 --height 2048).",
        );
    }
    note(
        ui,
        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom · Ctrl+click: focus",
    );
    ui.button("Reset camera")
        .on_hover_text("Back to the scene's own view")
        .clicked()
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
