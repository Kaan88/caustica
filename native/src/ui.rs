//! The interface (UI plan phase 1): top bar, a right side panel with Scene / Render / Look /
//! Export tabs, a status bar and toasts. UI-only state lives in `UiState`, never in `Settings`,
//! because every `Settings` field outside the exclusion list restarts the render.
use crate::scene::{self, DLSS_PRESETS, PRESETS, Settings, Shape, Upscaler};
use crate::{App, editor, gizmo};
use std::time::{Duration, Instant};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 202, 216);
const WARN: egui::Color32 = egui::Color32::from_rgb(232, 176, 72);
const VIEWS: [&str; 8] = [
    "Beauty",
    "Normals",
    "Albedo",
    "Depth",
    "Direct light",
    "Indirect light",
    "Specular",
    "Convergence",
];
const TONE_MAPS: [&str; 6] = [
    "Linear",
    "Reinhard",
    "Reinhard (white point)",
    "ACES filmic",
    "Uncharted 2",
    "High contrast",
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
        self.overlay(ctx);
        self.toasts(ctx);
    }
    pub fn tool_mode(&self) -> gizmo::Mode {
        match self.ui.tool {
            gizmo::Tool::Rotate => gizmo::Mode::Rotate,
            gizmo::Tool::Scale => gizmo::Mode::Scale,
            _ => gizmo::Mode::Move,
        }
    }
    /// Gizmo, light outlines and the transform header, drawn under the panels.
    fn overlay(&mut self, ctx: &egui::Context) {
        let g = self.gpu.as_ref().unwrap();
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
        if self.ui.tool == gizmo::Tool::Select || !gizmo::allows(self.ui.selection, mode) {
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
                if ui.button("◀").on_hover_text("Previous scene").clicked() {
                    self.preset = (self.preset + count - 1) % count;
                    self.settings.preset(self.preset);
                }
                let current = if known {
                    PRESETS[self.preset]
                } else {
                    "Custom scene"
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
                    self.preset = (self.preset + 1) % count;
                    self.settings.preset(self.preset);
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
        let g = self.gpu.as_ref().unwrap();
        let aspect = g.width as f32 / g.height as f32;
        let s = &mut self.settings;
        let state = &mut self.ui;
        note(
            ui,
            if known {
                crate::presets::description(self.preset)
            } else {
                "Custom scene from a settings file."
            },
        );
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
    note(ui, "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom");
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
