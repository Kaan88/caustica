//! The chrome of layout B (docs/ui-redesign-b.md): the one-row top bar with its Menu and pinned
//! buttons, the tool rail, the right column (outliner over the property tabs), the status bar
//! and the toasts.
use super::icons::{self, Icon};
use super::scene_tab::add_menu;
use super::*;
use crate::palette::{self, Do, Widget, Win};
use crate::prefs::Pin;

/// Menu → About: each vendor's own logo (nvidia-settings, the FSR SDK's and XeSS SDK's docs), its
/// credit, and whether only Windows ships that component.
const LOGOS: [(&[u8], &str, bool); 3] = [
    (
        include_bytes!("../../assets/nvidia-logo.png"),
        "This application uses the NVIDIA DLSS SDK.\nNVIDIA RTX\u{2122}",
        false,
    ),
    (
        include_bytes!("../../assets/amd-fsr-logo.png"),
        "Uses the AMD FSR SDK.",
        true,
    ),
    (
        include_bytes!("../../assets/intel-logo.jpg"),
        "Uses Intel XeSS.",
        true,
    ),
];
pub(super) const CPU_VIEW_HELP: &str = "Show the software path tracer: every ray on the CPU, through the same exposure, tone map and look. A rough preview while the camera moves, then it refines. The GPU only displays the image.";
pub(super) const NO_RT_HELP: &str = "This GPU can't ray trace (no ray query support), so it only displays: the CPU renders the view.";
/// What the software view's image leaves out, or why there is none; empty when it is all there.
pub(super) fn cpu_view_text(st: &crate::reference::LiveState) -> String {
    match &st.error {
        Some(e) => e.clone(),
        None if st.missing.is_empty() => String::new(),
        None => format!(
            "The CPU can't render these yet, so this image is without them: {}.",
            st.missing.join(", ")
        ),
    }
}
/// Whether this platform shows the logo: the upscalers it doesn't ship stay out.
fn logo_shown(windows_only: bool) -> bool {
    !windows_only || cfg!(windows)
}

impl App {
    /// D3D12 or Vulkan, saved in the prefs and used from the next start (the renderer can't be
    /// swapped under a running scene). `--vulkan` / `--d3d12` override it for one run.
    #[cfg(windows)]
    fn renderer_menu(&mut self, ui: &mut egui::Ui) {
        let running = if self.vulkan { "Vulkan" } else { "D3D12" };
        ui.weak(format!("Running: {running}"));
        if let Some(why) = &self.vulkan_failed {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Vulkan did not start; D3D12 runs instead.",
            )
            .on_hover_text(why.as_str());
        }
        let mut want = self.ui.prefs.vulkan;
        ui.radio_value(&mut want, false, "D3D12 (DirectX 12)");
        ui.radio_value(&mut want, true, "Vulkan");
        if want != self.ui.prefs.vulkan {
            self.ui.prefs.vulkan = want;
            self.ui.save_prefs();
        }
        if want != self.vulkan {
            // After a fallback the choice stays Vulkan: a restart tries it again.
            let retry = want && self.vulkan_failed.is_some();
            ui.label(if retry {
                "Vulkan is tried again at the next start."
            } else {
                "Takes effect after a restart."
            });
            if ui
                .button(if retry {
                    "Retry Vulkan now"
                } else {
                    "Restart now"
                })
                .clicked()
            {
                self.ui.restart = true;
                ui.close();
            }
        }
    }
    /// The top bar: one row that never wraps. Menu, the scene picker, the pinned windows, the
    /// playback block, the pinned commands and live controls, then Quality, View and GPU / CPU
    /// from the right edge. Pins that don't fit lose their label, then move into a » menu.
    pub(super) fn top_bar(&mut self, root: &mut egui::Ui, known: bool) {
        let frame = egui::Frame::new()
            .fill(BAR)
            .inner_margin(egui::Margin::symmetric(8, 4));
        egui::Panel::top("top bar").frame(frame).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                let exporting = self.exporting();
                ui.add_enabled_ui(!exporting, |ui| {
                    self.menu(ui);
                    self.scene_picker(ui, known);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Right to left: the last control first.
                    ui.add_enabled_ui(!exporting, |ui| {
                        let traces = self.gpu.as_ref().is_some_and(|g| g.ray_tracing);
                        ui.spacing_mut().item_spacing.x = 2.;
                        ui.selectable_value(&mut self.cpu_view, true, "CPU")
                            .on_hover_text(CPU_VIEW_HELP);
                        ui.add_enabled_ui(traces, |ui| {
                            ui.selectable_value(&mut self.cpu_view, false, "GPU")
                                .on_hover_text("The viewport shows the GPU's path tracer")
                                .on_disabled_hover_text(NO_RT_HELP);
                        });
                        ui.spacing_mut().item_spacing.x = 6.;
                        let s = &mut self.settings;
                        egui::ComboBox::from_id_salt("top view")
                            .selected_text(VIEWS[s.view as usize])
                            .width(104.)
                            .show_ui(ui, |ui| {
                                for (i, n) in VIEWS.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut s.view,
                                        i as u32,
                                        format!("{n}  ({})", i + 1),
                                    );
                                }
                            })
                            .response
                            .on_hover_text("What the viewport shows (keys 1–8)");
                        self.quality_combo(ui);
                    });
                    bar_separator(ui);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if exporting {
                            self.export_chip(ui);
                        } else {
                            self.top_middle(ui);
                        }
                    });
                });
            });
        });
    }
    /// ☰: File, Edit, Window, Settings, Renderer and Help. Sticky, so its checkboxes can be used;
    /// plain items close it.
    fn menu(&mut self, ui: &mut egui::Ui) {
        use egui::containers::menu::{MenuButton, MenuConfig};
        let button = egui::Button::new("")
            .min_size(egui::vec2(28., 24.))
            .frame_when_inactive(false);
        let (r, _) = MenuButton::from_button(button)
            .config(MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_min_width(190.);
                ui.menu_button("File", |ui| self.file_menu(ui));
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(
                            !self.ui.undo.is_empty(),
                            egui::Button::new("Undo").shortcut_text("Ctrl+Z"),
                        )
                        .clicked()
                    {
                        self.ui.undo(&mut self.settings);
                        self.timeline.settle(&self.settings);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.ui.redo.is_empty(),
                            egui::Button::new("Redo").shortcut_text("Ctrl+Y"),
                        )
                        .clicked()
                    {
                        self.ui.redo(&mut self.settings);
                        self.timeline.settle(&self.settings);
                        ui.close();
                    }
                });
                ui.menu_button("Window", |ui| self.window_menu(ui));
                ui.menu_button("Settings", |ui| self.settings_menu(ui));
                #[cfg(windows)]
                ui.menu_button("Renderer", |ui| self.renderer_menu(ui));
                ui.menu_button("Help", |ui| self.help_menu(ui));
                ui.separator();
                let g = self.gpu.as_ref().unwrap();
                #[cfg(windows)]
                let api = if self.vulkan {
                    "Vulkan"
                } else {
                    "Direct3D 12, DXR 1.1"
                };
                #[cfg(not(windows))]
                let api = "Vulkan";
                note(
                    ui,
                    &format!(
                        "Caustica {} · {} · Rust, {api}",
                        crate::VERSION,
                        g.adapter_name
                    ),
                );
            });
        icons::paint(
            ui.painter(),
            r.rect,
            Icon::Menu,
            16.,
            egui::Color32::from_gray(218),
        );
        r.on_hover_text("Menu · F1 for keys");
    }
    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if self.menu_item(ui, "Open scene…", "Ctrl+O", Some(Pin::Do(Do::OpenScene))) {
            self.open_scene_dialog();
        }
        if self.menu_item(ui, "Save scene", "Ctrl+S", Some(Pin::Do(Do::SaveScene))) {
            self.save_scene(false);
        }
        if self.menu_item(
            ui,
            "Save scene as…",
            "Ctrl+Shift+S",
            Some(Pin::Do(Do::SaveSceneAs)),
        ) {
            self.save_scene(true);
        }
        if self.ui.edits_live
            && ui
                .add_enabled(
                    self.ui.session_offer.is_some(),
                    egui::Button::new("Restore last session"),
                )
                .on_disabled_hover_text("No unsaved work was left by the last session")
                .clicked()
        {
            self.restore_session();
            ui.close();
        }
        ui.separator();
        if ui
            .button("Generate scene…")
            .on_hover_text("The Scenes window's generators: a family and a seed make a scene")
            .clicked()
        {
            self.ui.show_scenes = true;
            ui.close();
        }
    }
    fn window_menu(&mut self, ui: &mut egui::Ui) {
        for (label, key, win) in [
            ("Scenes", "Ctrl+P", Win::Scenes),
            ("Assets", "", Win::Assets),
            ("Scopes", "", Win::Scopes),
            ("Convergence graph", "", Win::Graph),
            ("Path visualizer", "P", Win::Paths),
            ("Statistics and achievements", "", Win::Stats),
            ("Phone remote", "", Win::Remote),
            ("Script console", "", Win::Console),
            ("Describe a scene (AI)", "", Win::Describe),
            ("Laser puzzle", "", Win::Puzzle),
        ] {
            let pin = Pin::Window(win);
            let mut on = self.pin_active(pin);
            let text = if key.is_empty() {
                label.to_string()
            } else {
                format!("{label} ({key})")
            };
            let r = ui.checkbox(&mut on, text);
            if r.changed() {
                self.run_pin(pin);
            }
            self.pin_context(&r, pin);
        }
        ui.separator();
        let mut hidden = self.ui.panel_hidden;
        if ui
            .checkbox(&mut hidden, "Hide panels (Tab)")
            .on_hover_text("Hides the tool rail, the outliner and tabs, the timeline and the status bar. A second Tab hides the top bar too (screenshot mode); a third brings everything back.")
            .changed()
        {
            self.ui.panel_hidden = hidden;
            ui.close();
        }
    }
    fn settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.ui.advanced, "Show advanced settings");
        if ui
            .checkbox(
                &mut self.ui.prefs.update_check,
                "Check for updates at startup",
            )
            .changed()
        {
            self.ui.save_prefs();
        }
        ui.menu_button("Text-to-scene model", |ui| self.ai_menu(ui));
        ui.menu_button("Top bar", |ui| {
            note(ui, "Right-click a Menu item or a Ctrl+K entry to pin it; right-click a pinned button to move or unpin it.");
            let mut pins = self.ui.prefs.pins();
            for w in Widget::ALL {
                let pin = Pin::Widget(w);
                let mut on = pins.contains(&pin);
                if ui.checkbox(&mut on, w.label()).on_hover_text("A live control in the top bar").changed() {
                    if on {
                        pins.push(pin);
                    } else {
                        pins.retain(|p| *p != pin);
                    }
                    self.ui.prefs.set_pins(&pins);
                    self.ui.save_prefs();
                }
            }
            ui.separator();
            if ui.button("Reset top bar").on_hover_text("Back to Scenes, Assets, Compare and Save image").clicked() {
                self.ui.prefs.reset_pins();
                self.ui.save_prefs();
                ui.close();
            }
        });
        ui.separator();
        if ui.button("Save settings.json")
            .on_hover_text("A bare settings file, scene included, overwritten each time in the output folder. Save scene writes a titled scene file into your scenes instead.").clicked() {
            self.actions.save = Some(crate::dirs::output("settings.json"));
            ui.close();
        }
        if ui.button("Save settings as…")
            .on_hover_text("A bare settings file, scene included, at a place you pick. Save scene as… writes a titled scene file into your scenes instead.").clicked() {
            self.actions.save = settings_dialog(true);
            ui.close();
        }
        if ui.button("Load settings…")
            .on_hover_text("Replaces the scene and every setting from a bare settings file. Open scene… loads a titled scene file.").clicked() {
            self.actions.load = settings_dialog(false);
            ui.close();
        }
        ui.separator();
        if ui
            .button("Reset everything to defaults…")
            .on_hover_text("The default scene and settings (asks first; the top bar's pins stay)")
            .clicked()
        {
            self.ui.confirm_reset = true;
            ui.close();
        }
    }
    fn help_menu(&mut self, ui: &mut egui::Ui) {
        if self.menu_item(
            ui,
            "Keyboard and mouse",
            "F1",
            Some(Pin::Window(Win::Shortcuts)),
        ) {
            self.ui.show_shortcuts = true;
        }
        if self.menu_item(ui, "Find a setting or command", "Ctrl+K", None) {
            self.ui.palette.open = true;
        }
        if ui.button("Check for updates").clicked() {
            // A skipped version is offered again when asked for.
            self.ui.prefs.update_skip.clear();
            self.updater.check();
            self.ui
                .toast("Checking for updates: a window opens if there is a newer version");
            ui.close();
        }
        ui.menu_button("About", |ui| {
            ui.label(format!("Caustica {}", crate::VERSION));
            ui.label("MIT license. Third-party components keep their own terms.");
            if self.ui.logos.is_empty() {
                for (i, (bytes, ..)) in LOGOS.iter().enumerate() {
                    let rgba = image::load_from_memory(bytes)
                        .expect("an embedded logo")
                        .to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    self.ui.logos.push(ui.ctx().load_texture(
                        format!("logo-{i}"),
                        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            for (logo, (_, credit, windows_only)) in self.ui.logos.iter().zip(&LOGOS) {
                if logo_shown(*windows_only) {
                    ui.horizontal(|ui| {
                        // The same height, each at its own proportions.
                        let [w, h] = logo.size().map(|v| v as f32);
                        ui.image((logo.id(), egui::vec2(40. * w / h, 40.)));
                        ui.label(*credit);
                    });
                }
            }
            ui.label("NVIDIA, NVIDIA RTX and DLSS are trademarks of NVIDIA Corporation.");
            ui.hyperlink_to(
                "Third-party notices",
                "https://github.com/Kaan88/caustica/blob/main/THIRD-PARTY-NOTICES.md",
            );
        });
    }
    /// A plain Menu item with its key at the right; a right-click offers to pin it. True when
    /// clicked (the menu closes).
    fn menu_item(&mut self, ui: &mut egui::Ui, label: &str, key: &str, pin: Option<Pin>) -> bool {
        let r = ui.add(egui::Button::new(label).shortcut_text(key));
        if let Some(pin) = pin {
            self.pin_context(&r, pin);
        }
        let clicked = r.clicked();
        if clicked {
            ui.close();
        }
        clicked
    }
    /// Right-click on a Menu item or palette entry: pin it to the top bar, or unpin it.
    pub(super) fn pin_context(&mut self, r: &egui::Response, pin: Pin) {
        r.context_menu(|ui| {
            let mut pins = self.ui.prefs.pins();
            let pinned = pins.contains(&pin);
            if ui
                .button(if pinned {
                    "Unpin from top bar"
                } else {
                    "Pin to top bar"
                })
                .clicked()
            {
                if pinned {
                    pins.retain(|p| *p != pin);
                } else {
                    pins.push(pin);
                }
                self.ui.prefs.set_pins(&pins);
                self.ui.save_prefs();
                ui.close();
            }
        });
    }
    /// ‹ the scene combo ›: favourite and recent scene files, the built-in presets, the way to
    /// the full browser, and Randomize / Today in its footer.
    fn scene_picker(&mut self, ui: &mut egui::Ui, known: bool) {
        let current = if known {
            PRESETS[self.preset].to_string()
        } else if !self.settings.scene_name.is_empty() {
            self.settings.scene_name.clone()
        } else {
            "Custom scene".to_string()
        };
        let current = if self.ui.scene_dirty {
            format!("{current} *")
        } else {
            current
        };
        let description = if known {
            crate::presets::description(self.preset).to_string()
        } else if let Some(i) = self.scene_entry() {
            self.ui.scenes[i].description.clone()
        } else {
            String::new()
        };
        let mut load: Option<usize> = None;
        egui::ComboBox::from_id_salt("scene picker")
            .selected_text(current)
            .width(180.)
            .height(640.)
            .show_ui(ui, |ui| {
                // Scene files: favourites, then the most recent ones.
                let prefs = &self.ui.prefs;
                let mut listed: Vec<(usize, bool, usize)> = self
                    .ui
                    .scenes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| {
                        let favorite = prefs.is_favorite(&e.path);
                        let recency = prefs.recency(&e.path);
                        (favorite || recency.is_some())
                            .then_some((i, favorite, recency.unwrap_or(usize::MAX)))
                    })
                    .collect();
                listed.sort_by_key(|(_, favorite, recency)| (!favorite, *recency));
                listed.truncate(10);
                if !listed.is_empty() {
                    ui.label(egui::RichText::new("Favourites and recent").small().color(ACCENT));
                    let selected = self.scene_entry();
                    for (i, favorite, _) in listed {
                        let title = &self.ui.scenes[i].title;
                        let text = if favorite { format!("★ {title}") } else { title.clone() };
                        if ui.selectable_label(selected == Some(i), text).clicked() {
                            load = Some(i);
                        }
                    }
                    ui.separator();
                }
                for (group, range) in GROUPS {
                    ui.label(egui::RichText::new(group).small().color(ACCENT));
                    for i in range {
                        // Picking the current scene again reverts it to the preset.
                        if ui
                            .selectable_label(known && self.preset == i, PRESETS[i])
                            .clicked()
                        {
                            self.preset = i;
                            self.switch_scene("Scene", |s| s.preset(i));
                        }
                    }
                    ui.separator();
                }
                if ui
                    .add(egui::Button::new(format!("All scenes… ({})", self.ui.scenes.len())).shortcut_text("Ctrl+P"))
                    .on_hover_text("The full list with search and tags")
                    .clicked()
                {
                    self.ui.show_scenes = true;
                    ui.close();
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Randomize")
                        .on_hover_text("A random variation of a random scene")
                        .clicked()
                    {
                        self.run_palette(palette::Act::Do(Do::Randomize));
                        ui.close();
                    }
                    if ui
                        .button("Today")
                        .on_hover_text("Seed of the day: everyone running this today gets the same random scene")
                        .clicked()
                    {
                        self.run_palette(palette::Act::Do(Do::Today));
                        ui.close();
                    }
                });
            })
            .response
            .on_hover_text(if description.is_empty() {
                "Scenes: favourites, recent files and the built-in presets. Picking the current one again restores it. A * marks unsaved changes.".to_string()
            } else {
                format!("{description}\n\nPicking the current scene again restores it. A * marks unsaved changes.")
            });
        if let Some(i) = load {
            self.load_scene_file(i);
        }
        ui.spacing_mut().item_spacing.x = 0.;
        if ui
            .add(egui::Button::new("‹").frame_when_inactive(false))
            .on_hover_text("Previous scene (Page Up): in the Scenes list while a scene file is loaded, else in the presets")
            .clicked()
        {
            self.step_scene(-1);
        }
        if ui
            .add(egui::Button::new("›").frame_when_inactive(false))
            .on_hover_text("Next scene (Page Down): in the Scenes list while a scene file is loaded, else in the presets")
            .clicked()
        {
            self.step_scene(1);
        }
        ui.spacing_mut().item_spacing.x = 6.;
    }
    /// Between the scene picker and the Quality combo: pinned windows, the playback block in
    /// the middle, pinned commands and live controls, and + to pin more.
    fn top_middle(&mut self, ui: &mut egui::Ui) {
        let pins = self.ui.prefs.pins();
        let (left, right): (Vec<Pin>, Vec<Pin>) =
            pins.iter().partition(|p| matches!(p, Pin::Window(_)));
        let order: Vec<Pin> = left.iter().chain(&right).copied().collect();
        // The playback block's width, from its own text.
        let g = self.gpu.as_ref().unwrap();
        let cpu = self
            .cpu_reference
            .as_ref()
            .filter(|l| l.view)
            .map(|l| l.state());
        let shown = cpu.as_ref().map_or(g.samples, |st| st.samples);
        let spp = format!("{shown} / {} spp", self.settings.max_samples);
        let spp_width = ui
            .painter()
            .layout_no_wrap(
                spp.clone(),
                egui::FontId::monospace(13.),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let gap = ui.spacing().item_spacing.x;
        let playback = 26. * 2. + spp_width + gap * 3.;
        // + and the separators around the groups.
        let fixed = playback + 26. + gap * 4. + 16.;
        let room = ui.available_width() - fixed;
        let full: Vec<f32> = order
            .iter()
            .map(|p| pin_width(ui, *p, false) + gap)
            .collect();
        let small: Vec<f32> = order
            .iter()
            .map(|p| pin_width(ui, *p, true) + gap)
            .collect();
        // "Icon only" pins never show their label; of the others the rightmost lose theirs
        // first, then pins leave for the » menu.
        let icon_only: Vec<bool> = order
            .iter()
            .map(|p| pin_short(*p).1.is_some() && self.ui.prefs.pin_icon_only(*p))
            .collect();
        let mut compact = icon_only.clone();
        let mut visible = order.len();
        let need = |compact: &[bool], visible: usize| -> f32 {
            let pins: f32 = (0..visible)
                .map(|i| if compact[i] { small[i] } else { full[i] })
                .sum();
            pins + if visible < order.len() { 26. + gap } else { 0. }
        };
        for i in (0..order.len()).rev() {
            if need(&compact, visible) <= room {
                break;
            }
            compact[i] = true;
        }
        while visible > 0 && need(&compact, visible) > room {
            visible -= 1;
        }
        let spare = (room - need(&compact, visible)).max(0.);
        let mut clicked: Option<Pin> = None;
        let mut edit: Option<Vec<Pin>> = None;
        let mut icon_edit: Option<(Pin, bool)> = None;
        let mut draw = |app: &mut App, ui: &mut egui::Ui, i: usize| {
            let pin = order[i];
            let r = match pin {
                Pin::Widget(w) => app.pin_widget(ui, w),
                _ => {
                    let (label, icon) = pin_short(pin);
                    let text = (!compact[i] || icon.is_none()).then_some(label);
                    let r = promo(ui, icon, text, app.pin_active(pin));
                    if r.clicked() {
                        clicked = Some(pin);
                    }
                    r.on_hover_text(pin_tip(pin))
                }
            };
            r.context_menu(|ui| {
                let group: Vec<Pin> = order
                    .iter()
                    .filter(|p| matches!(p, Pin::Window(_)) == matches!(pin, Pin::Window(_)))
                    .copied()
                    .collect();
                let at = group.iter().position(|p| *p == pin).unwrap_or(0);
                let mut moved = None;
                if ui.button("Unpin").clicked() {
                    edit = Some(order.iter().filter(|p| **p != pin).copied().collect());
                    ui.close();
                }
                if ui
                    .add_enabled(at > 0, egui::Button::new("Move left"))
                    .clicked()
                {
                    moved = Some(group[at - 1]);
                }
                if ui
                    .add_enabled(at + 1 < group.len(), egui::Button::new("Move right"))
                    .clicked()
                {
                    moved = Some(group[at + 1]);
                }
                // Live controls and pins without an icon have nothing to fall back to.
                if pin_short(pin).1.is_some() {
                    let mut on = icon_only[i];
                    if ui
                        .checkbox(&mut on, "Icon only")
                        .on_hover_text("Always without its label, which stays in the tooltip")
                        .changed()
                    {
                        icon_edit = Some((pin, on));
                        ui.close();
                    }
                }
                if let Some(other) = moved {
                    let mut pins = order.clone();
                    let (a, b) = (
                        pins.iter().position(|p| *p == pin).unwrap_or(0),
                        pins.iter().position(|p| *p == other).unwrap_or(0),
                    );
                    pins.swap(a, b);
                    edit = Some(pins);
                    ui.close();
                }
            });
        };
        let split = left.len();
        if !left.is_empty() {
            bar_separator(ui);
        }
        for i in 0..visible.min(split) {
            draw(self, ui, i);
        }
        ui.add_space(spare / 2.);
        // Playback: pause, restart, samples.
        let icon = if self.paused { Icon::Play } else { Icon::Pause };
        if icons::button(ui, icon, 26., 14., self.paused, false)
            .on_hover_text(if self.paused {
                "Resume (Space)"
            } else {
                "Pause (Space)"
            })
            .clicked()
        {
            self.paused = !self.paused;
        }
        if icons::button(ui, Icon::Restart, 26., 14., false, false)
            .on_hover_text("Throw away the samples and start again (R)")
            .clicked()
        {
            self.actions.reset = true;
        }
        ui.label(
            egui::RichText::new(spp)
                .monospace()
                .color(egui::Color32::from_gray(189)),
        )
        .on_hover_text(
            "Samples per pixel so far, and the cap where rendering stops. More is cleaner.",
        );
        ui.add_space(spare / 2.);
        for i in split..visible.max(split) {
            draw(self, ui, i);
        }
        if visible < order.len() {
            sticky_menu(ui, "»", |ui| {
                for &pin in order.iter().skip(visible) {
                    match pin {
                        Pin::Widget(w) => {
                            self.pin_widget(ui, w);
                        }
                        pin => {
                            let mut on = self.pin_active(pin);
                            if ui
                                .toggle_value(&mut on, pin_short(pin).0)
                                .on_hover_text(pin_tip(pin))
                                .clicked()
                            {
                                clicked = Some(pin);
                                ui.close();
                            }
                        }
                    }
                }
            });
        }
        // + Pin: every command and window of the palette, and the live controls.
        let _ = sticky_menu(ui, "+", |ui| {
            ui.set_min_width(230.);
            note(ui, "Pin to the top bar");
            egui::ScrollArea::vertical()
                .max_height(420.)
                .show(ui, |ui| {
                    for w in Widget::ALL {
                        let pin = Pin::Widget(w);
                        let mut on = order.contains(&pin);
                        if ui
                            .checkbox(&mut on, format!("{} (live control)", w.label()))
                            .changed()
                        {
                            edit = Some(toggled(&order, pin));
                        }
                    }
                    ui.separator();
                    for e in palette::ENTRIES {
                        let pin = match e.act {
                            palette::Act::Do(d) => Pin::Do(d),
                            palette::Act::Window(w) => Pin::Window(w),
                            palette::Act::Go(..) => continue,
                        };
                        let mut on = order.contains(&pin);
                        if ui.checkbox(&mut on, e.label).changed() {
                            edit = Some(toggled(&order, pin));
                        }
                    }
                });
        });
        if let Some(pin) = clicked {
            self.run_pin(pin);
        }
        if let Some(pins) = edit {
            self.ui.prefs.set_pins(&pins);
            self.ui.save_prefs();
        }
        if let Some((pin, on)) = icon_edit {
            self.ui.prefs.set_pin_icon_only(pin, on);
            self.ui.save_prefs();
        }
    }
    /// A pinned live control.
    fn pin_widget(&mut self, ui: &mut egui::Ui, w: Widget) -> egui::Response {
        let s = &mut self.settings;
        match w {
            Widget::Exposure => {
                let auto = s.auto_exposure;
                let mut ev = if auto {
                    s.ev_bias
                } else {
                    s.exposure.max(1e-6).log2()
                };
                // The Look tab's ranges, so a scene's value is never clamped by showing it.
                let range = if auto {
                    -crate::scene::EV_BIAS_MAX..=crate::scene::EV_BIAS_MAX
                } else {
                    crate::scene::EXPOSURE_EV_MIN..=crate::scene::EXPOSURE_EV_MAX
                };
                let before = ev;
                let r = ui.add(
                    egui::DragValue::new(&mut ev)
                        .speed(0.02)
                        .range(range)
                        .custom_formatter(|v, _| format!("{v:+.1} EV")),
                );
                if r.changed() && ev != before {
                    if auto {
                        s.ev_bias = ev;
                    } else {
                        s.exposure = ev.exp2();
                    }
                }
                r.on_hover_text(if auto {
                    "Exposure bias in stops, on top of the meter (Look > Exposure)"
                } else {
                    "Exposure in stops (Look > Exposure)"
                })
            }
            Widget::Denoiser => egui::ComboBox::from_id_salt("top denoiser")
                .selected_text(match s.denoiser {
                    crate::scene::Denoiser::Rr => "DLSS RR",
                    crate::scene::Denoiser::Native => "Native",
                    crate::scene::Denoiser::Off => "No denoiser",
                })
                .width(96.)
                .show_ui(ui, |ui| {
                    for d in crate::scene::Denoiser::ALL {
                        ui.selectable_value(&mut s.denoiser, d, d.label());
                    }
                })
                .response
                .on_hover_text("Denoiser (Alt+D cycles; Render > Reconstruction)"),
            Widget::MaxSamples => ui
                .add(
                    egui::DragValue::new(&mut s.max_samples)
                        .speed(8.)
                        .range(1..=1_000_000)
                        .suffix(" spp"),
                )
                .on_hover_text("The sample cap: rendering stops there (Render > Sampling)"),
        }
    }
    /// The tool rail at the left: Select, Move, Rotate, Scale; Walk, Frame, Add; the timeline
    /// toggle at the bottom.
    pub(super) fn tool_rail(&mut self, root: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(BAR)
            .inner_margin(egui::Margin::symmetric(5, 8));
        egui::Panel::left("tool rail")
            .exact_size(44.)
            .resizable(false)
            .frame(frame)
            .show(root, |ui| {
                ui.spacing_mut().item_spacing.y = 4.;
                if self.exporting() {
                    ui.disable();
                }
                for (tool, icon, tip) in [
                    (
                        gizmo::Tool::Select,
                        Icon::Select,
                        "Select: click to select, drag a box (B boxes with any tool). No handles",
                    ),
                    (
                        gizmo::Tool::Move,
                        Icon::Move,
                        "Move: handles for dragging, arrows and planes. G starts a keyboard move (pointer over the viewport, something selected)",
                    ),
                    (
                        gizmo::Tool::Rotate,
                        Icon::Rotate,
                        "Rotate: rings for dragging. R starts a keyboard rotation (pointer over the viewport, something selected)",
                    ),
                    (
                        gizmo::Tool::Scale,
                        Icon::Scale,
                        "Scale: handles for dragging; the circle scales evenly. S starts a keyboard scale (pointer over the viewport, something selected)",
                    ),
                ] {
                    if icons::button(ui, icon, 34., 18., self.ui.tool == tool, false)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.ui.tool = tool;
                    }
                }
                let (line, _) = ui.allocate_exact_size(egui::vec2(34., 9.), egui::Sense::hover());
                ui.painter().hline(
                    line.shrink2(egui::vec2(5., 0.)).x_range(),
                    line.center().y,
                    egui::Stroke::new(1_f32, egui::Color32::from_gray(58)),
                );
                let walk = icons::button(ui, Icon::Walk, 34., 18., self.walking.is_some(), false)
                    .on_hover_text(
                        "Walk mode (`): WASD, Space jump, C or Ctrl crouch, Shift run, F fly (Q / E down / up), B head bob, T flashlight, wheel body size, drag to look. Right-click: walk avatar",
                    );
                if walk.clicked() {
                    self.toggle_walk();
                }
                walk.context_menu(|ui| {
                    ui.checkbox(&mut self.settings.walk_avatar, "Walk avatar").on_hover_text(
                        "Walk mode draws a blocky body under the camera: legs and hands in view when looking down, the whole figure in mirrors and reflections.",
                    );
                });
                if icons::button(ui, Icon::Frame, 34., 18., false, false)
                    .on_hover_text("Frame the selection, or the whole scene (F). Home resets the camera; a double-click on an object frames it")
                    .clicked()
                {
                    self.frame_selection();
                }
                // Add: the outliner's + menu, and the way to the asset browser.
                use egui::containers::menu::{MenuButton, MenuConfig};
                let button = egui::Button::new("")
                    .min_size(egui::vec2(34., 34.))
                    .frame_when_inactive(false);
                let (r, _) = MenuButton::from_button(button)
                    .config(
                        MenuConfig::new()
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .ui(ui, |ui| {
                        add_menu(ui, &mut self.ui, &mut self.settings);
                        ui.separator();
                        if ui
                            .button("Assets…")
                            .on_hover_text("Download HDRIs, textures and models (Poly Haven, ambientCG, Khronos)")
                            .clicked()
                        {
                            self.ui.assets.open = true;
                            ui.close();
                        }
                    });
                icons::paint(ui.painter(), r.rect, Icon::Add, 18., egui::Color32::from_gray(208));
                r.on_hover_text("Add an object, a light, a mesh file or an asset");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    let r = icons::button(ui, Icon::Timeline, 34., 18., self.ui.show_timeline, false);
                    // Closed on a scene with keys: there is something in there.
                    if !self.ui.show_timeline && !self.settings.animation.is_empty() {
                        ui.painter().circle_filled(
                            r.rect.right_top() + egui::vec2(-6., 6.),
                            3.,
                            egui::Color32::from_rgb(255, 214, 90),
                        );
                    }
                    if r
                        .on_hover_text("Timeline. I keys the selection (the camera when nothing is selected), Shift+I one of its values, Alt+I deletes the key; K plays; Shift+← / → step a frame")
                        .clicked()
                    {
                        self.ui.show_timeline = !self.ui.show_timeline;
                    }
                });
            });
    }
    /// Space, Pivot and Snap, floating at the viewport's top left whenever something is selected
    /// or a transform runs (G / R / S work under every tool).
    pub(super) fn transform_strip(&mut self, ctx: &egui::Context) {
        if self.ui.targets().is_empty() && self.ui.transform.is_none() {
            return;
        }
        let at = central_rect(ctx).left_top() + egui::vec2(10., 10.);
        egui::Area::new(egui::Id::unique("transform strip"))
            .order(egui::Order::Middle)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(32, 32, 32, 217))
                    .corner_radius(4.)
                    .inner_margin(4.)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.;
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
                        });
                    });
            });
    }
    /// Menu > Settings > Reset everything to defaults asks before it replaces the scene.
    pub(super) fn reset_dialog(&mut self, ctx: &egui::Context) {
        if !self.ui.confirm_reset {
            return;
        }
        let modal = egui::Modal::new(egui::Id::unique("confirm reset")).show(ctx, |ui| {
            ui.set_max_width(360.);
            ui.heading("Reset everything to defaults?");
            ui.label("The scene and every setting go back to the defaults. Ctrl+Z brings the scene back.");
            ui.add_space(6.);
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    self.ui.checkpoint_scene(&self.settings);
                    self.settings = Settings::default();
                    self.preset = 0;
                    self.ui.selection = Selection::Stage;
                    self.ui.mark_clean(&self.settings);
                    self.ui.toast("Reset to the default scene and settings");
                    self.ui.confirm_reset = false;
                }
                if ui.button("Cancel").clicked() {
                    self.ui.confirm_reset = false;
                }
            });
        });
        if modal.should_close() {
            self.ui.confirm_reset = false;
        }
    }
    /// The software view's one plain message: what this scene uses that its image leaves out. On
    /// the viewport itself, since it changes what you are looking at.
    pub(super) fn cpu_view_message(&mut self, ctx: &egui::Context) {
        let Some(st) = self
            .cpu_reference
            .as_ref()
            .filter(|l| l.view)
            .map(|l| l.state())
        else {
            return;
        };
        let text = cpu_view_text(&st);
        if text.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("cpu view message"))
            .order(egui::Order::Middle)
            .interactable(false)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(8., -8.))
            .constrain_to(super::central_rect(ctx))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.);
                    ui.label(egui::RichText::new(text).small().color(WARN));
                });
            });
    }
    pub(super) fn status_bar(&mut self, root: &mut egui::Ui) {
        let (text, fallback) = pipeline(self);
        let g = self.gpu.as_ref().unwrap();
        let s = &self.settings;
        // The software view counts the CPU's samples; the GPU's stop after one pass.
        let cpu = self
            .cpu_reference
            .as_ref()
            .filter(|l| l.view)
            .map(|l| l.state());
        let shown = cpu.as_ref().map_or(g.samples, |st| st.samples);
        let export = self.export_info().map(|e| e.status());
        let frame = egui::Frame::new()
            .fill(BAR)
            .inner_margin(egui::Margin::symmetric(10, 3));
        egui::Panel::bottom("status bar").frame(frame).show(root, |ui| {
            ui.horizontal(|ui| {
                let converged = shown >= s.max_samples;
                let (state, color) = if cpu.as_ref().is_some_and(|st| st.images == 0) {
                    ("Preparing the scene", egui::Color32::from_rgb(110, 200, 120))
                } else if cpu.as_ref().is_some_and(|st| st.preview) {
                    ("Preview", egui::Color32::from_rgb(110, 200, 120))
                } else if self.paused {
                    ("Paused", egui::Color32::GRAY)
                } else if converged {
                    ("Converged", ACCENT)
                } else {
                    ("Rendering", egui::Color32::from_rgb(110, 200, 120))
                };
                let (state, color) = match &export {
                    Some(text) => (text.as_str(), ACCENT),
                    None => (state, color),
                };
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10., 10.), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4., color);
                ui.label(state);
                // An export counts its own samples at its own size.
                if export.is_some() {
                    ui.separator();
                    ui.label(format!("{} × {}", g.width, g.height));
                    return;
                }
                ui.separator();
                ui.monospace(format!("{shown} / {} spp", s.max_samples))
                    .on_hover_text("Samples per pixel: more is cleaner. Rendering stops at the cap.");
                ui.add(
                    egui::ProgressBar::new((shown as f32 / s.max_samples as f32).min(1.))
                        .desired_width(80.)
                        .desired_height(8.),
                );
                // Time to converge, from samples per second smoothed over about a second.
                let now = Instant::now();
                let (at, samples, rate) = &mut self.ui.sample_rate;
                let elapsed = now.duration_since(*at).as_secs_f32();
                if shown < *samples {
                    (*at, *samples, *rate) = (now, shown, 0.);
                } else if elapsed > 0.25 {
                    let r = (shown - *samples) as f32 / elapsed;
                    *rate = if *rate > 0. { *rate * 0.6 + r * 0.4 } else { r };
                    (*at, *samples) = (now, shown);
                }
                if !converged && !self.paused && *rate > 0. {
                    let left = (s.max_samples - shown.min(s.max_samples)) as f32 / *rate;
                    ui.monospace(time_left(left))
                        .on_hover_text("Time until the sample cap at the current speed");
                }
                ui.separator();
                if let Some(st) = &cpu {
                    ui.monospace(format!("CPU · {} threads · {:.1} s", st.threads, st.seconds))
                        .on_hover_text("Time the CPU has spent on this image");
                    ui.separator();
                    ui.label(format!("{} × {}", g.width, g.height));
                    return;
                }
                // Left to right is most to least useful: what clips first in a narrow window is
                // at the end. State, spp and ETA are above; then size and HDR, the pipeline
                // (so an amber fallback is never clipped), GPU time and, last, object counts.
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
                ui.separator();
                ui.label(if fallback {
                    egui::RichText::new(&text).color(WARN)
                } else {
                    egui::RichText::new(&text)
                });
                // What the denoiser is doing right now (Advanced): reprojecting, integrating,
                // or starting over after an edit.
                if self.ui.advanced && !g.denoiser_activity.is_empty() {
                    ui.label(g.denoiser_activity);
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
                if self.ui.exposure_held {
                    ui.separator();
                    ui.weak("Exposure held").on_hover_text(
                        "A light or material is being edited: the meter waits (about a second after the edit) so the change shows instead of being metered away.",
                    );
                }
                // The exposure now and the luminance the meter adapted to (Advanced).
                if self.ui.advanced {
                    ui.separator();
                    ui.label(exposure_readout(s, g.metered))
                        .on_hover_text("The exposure the display uses, in stops (metered plus bias in auto), and the luminance the meter has adapted to, in cd/m2.");
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
                // Last and truncated: it gives way to everything above in a narrow window.
                if !self.ui.targets().is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("G R S move · F frame · H hide · Del delete")
                                    .small()
                                    .weak(),
                            )
                            .truncate(),
                        );
                    });
                }
            });
        });
    }
    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.ui.toasts.retain(|t| t.until > now);
        let offer = self.ui.session_prompt().then(|| {
            let session = self.ui.session_offer.as_ref();
            session.map_or(String::new(), |s| s.settings.scene_name.clone())
        });
        if self.ui.toasts.is_empty() && offer.is_none() {
            return;
        }
        let (mut restore, mut dismiss) = (false, false);
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-8., -8.])
            .constrain_to(super::central_rect(ctx))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if let Some(name) = &offer {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("Unsaved work from last time: {name}"));
                            restore = ui.button("Restore last session").clicked();
                            dismiss = ui.small_button("×").on_hover_text("Discard it").clicked();
                        });
                    });
                }
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
        if restore {
            self.restore_session();
        } else if dismiss {
            self.ui.session_offer = None;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    /// The right column: the outliner on top (a draggable split), then the vertical tab strip
    /// beside the open tab's properties.
    pub(super) fn side_panel(&mut self, root: &mut egui::Ui, known: bool) {
        // A panel's id is its salt under the parent Ui's scope id (egui 0.35+); its resize handle
        // is "__resize" under that.
        let resize_id = root.scope_id().with("side panel").with("__resize");
        let (min, max) = crate::prefs::PANEL_WIDTH;
        let fill = root.visuals().panel_fill;
        let panel = egui::Panel::right("side panel")
            .resizable(true)
            .default_size(self.ui.prefs.panel_width)
            .size_range(min..=max)
            .frame(egui::Frame::new().fill(fill.to_opaque()))
            .show(root, |ui| {
                let column = ui.available_height();
                let width = ui.available_width();
                let row = ui.spacing().interact_size.y + 2.;
                // An export shows its card whatever tab was open, and gives that tab back after.
                let exporting = self.exporting();
                let open = if exporting { Tab::Export } else { self.ui.tab };
                // Folded to its header above Render, Look and Export, and in a short column.
                let foldable = !open.follows_selection() || column < 520.;
                let tree = if foldable {
                    (column * 0.4).min(260.)
                } else {
                    (column * self.ui.outliner_split).clamp(row * 4., column * 0.45)
                };
                ui.add_enabled_ui(!exporting, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| self.outliner(ui, known, foldable, tree));
                });
                // The split's edge: drag it to give the outliner more or less of the column.
                let (edge, r) = ui.allocate_exact_size(
                    egui::vec2(width, if foldable { 1. } else { 5. }),
                    if foldable {
                        egui::Sense::hover()
                    } else {
                        egui::Sense::drag()
                    },
                );
                let hot = r.hovered() || r.dragged();
                ui.painter().hline(
                    edge.x_range(),
                    edge.center().y,
                    egui::Stroke::new(1_f32, if hot && !foldable { ACCENT } else { HAIRLINE }),
                );
                if !foldable {
                    let r = r.on_hover_cursor(egui::CursorIcon::ResizeVertical);
                    if r.dragged() {
                        let px = (tree + r.drag_delta().y).clamp(row * 4., column * 0.45);
                        self.ui.outliner_split = (px / column).clamp(0.1, 0.45);
                        self.ui.outliner_resized = true;
                    }
                }
                let rest = ui.available_rect_before_wrap();
                // In a column narrower than its minimum (a small window) the strip lies across the top.
                let across = width < 350.;
                let (strip, props) = if across {
                    let strip = egui::Rect::from_min_size(rest.min, egui::vec2(rest.width(), 36.));
                    (
                        strip,
                        egui::Rect::from_min_max(strip.left_bottom(), rest.max),
                    )
                } else {
                    let strip = egui::Rect::from_min_size(rest.min, egui::vec2(36., rest.height()));
                    (strip, egui::Rect::from_min_max(strip.right_top(), rest.max))
                };
                ui.painter().rect_filled(strip, 0., BAR);
                let line = egui::Stroke::new(1_f32, HAIRLINE);
                let layout = if across {
                    ui.painter().hline(strip.x_range(), strip.bottom(), line);
                    egui::Layout::left_to_right(egui::Align::Center)
                } else {
                    ui.painter().vline(strip.right(), strip.y_range(), line);
                    egui::Layout::top_down(egui::Align::Center)
                };
                let mut strip_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(strip.shrink2(egui::vec2(3., 3.)))
                        .layout(layout),
                );
                strip_ui.spacing_mut().item_spacing = egui::vec2(2., 2.);
                if !across {
                    strip_ui.add_space(3.);
                }
                if exporting {
                    strip_ui.disable();
                }
                for tab in Tab::ALL {
                    let icon = match tab {
                        Tab::Render => Icon::Render,
                        Tab::Look => Icon::Look,
                        Tab::Lights => Icon::Lights,
                        Tab::Object => Icon::Object,
                        Tab::Material => Icon::Material,
                        Tab::Export => Icon::Export,
                    };
                    if icons::button(&mut strip_ui, icon, 30., 16., open == tab, true)
                        .on_hover_text(tab.label())
                        .clicked()
                    {
                        self.ui.tab = tab;
                    }
                }
                let mut props_ui = ui
                    .new_child(egui::UiBuilder::new().max_rect(props.shrink2(egui::vec2(10., 4.))));
                // Each tab keeps its own offset: a short tab would clamp a shared one.
                egui::ScrollArea::vertical()
                    .id_salt(open as u8)
                    .auto_shrink([false; 2])
                    .show(&mut props_ui, |ui| match open {
                        Tab::Render => self.render_tab(ui),
                        Tab::Look => self.look_tab(ui),
                        Tab::Lights => self.lights_tab(ui),
                        Tab::Object => self.object_tab(ui),
                        Tab::Material => self.material_tab(ui),
                        Tab::Export => self.export_tab(ui),
                    });
                ui.allocate_rect(rest, egui::Sense::hover());
            });
        // Only the user's drag counts: the panel also widens to fit wide content.
        if root.ctx().is_being_dragged(resize_id) {
            self.ui.panel_resized = true;
        }
        self.ui.panel_width = panel.response.rect.width();
    }
}

/// A hairline between groups of the top bar.
fn bar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7., 20.), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1_f32, egui::Color32::from_gray(58)),
    );
}
/// `pins` with `pin` added at the end, or removed.
fn toggled(pins: &[Pin], pin: Pin) -> Vec<Pin> {
    if pins.contains(&pin) {
        pins.iter().filter(|p| **p != pin).copied().collect()
    } else {
        pins.iter().copied().chain([pin]).collect()
    }
}
/// The palette entry a pin runs.
fn pin_entry(pin: Pin) -> Option<&'static palette::Entry> {
    palette::ENTRIES.iter().find(|e| match (pin, e.act) {
        (Pin::Do(d), palette::Act::Do(x)) => d == x,
        (Pin::Window(w), palette::Act::Window(x)) => w == x,
        _ => false,
    })
}
/// A pinned button's short label (the palette's own where there is no shorter one) and its
/// icon: every command and window has one, so it can lose the label when the bar runs out of
/// room or for good ("Icon only"). Live controls have none.
fn pin_short(pin: Pin) -> (&'static str, Option<Icon>) {
    let (short, icon) = match pin {
        Pin::Window(w) => match w {
            Win::Remote => ("", Icon::Remote),
            Win::Console => ("Console", Icon::Console),
            Win::Stats => ("Statistics", Icon::Stats),
            Win::Shortcuts => ("Keys", Icon::Keys),
            Win::Scenes => ("Scenes", Icon::Scenes),
            Win::Assets => ("Assets", Icon::Assets),
            Win::Video => ("Video…", Icon::Video),
            Win::Scopes => ("", Icon::Scopes),
            Win::Graph => ("Convergence", Icon::Graph),
            Win::Paths => ("Paths", Icon::Paths),
            Win::Describe => ("Describe", Icon::Describe),
            Win::Puzzle => ("", Icon::Puzzle),
        },
        Pin::Do(d) => match d {
            Do::OpenScene => ("", Icon::Open),
            Do::SaveScene => ("", Icon::Save),
            Do::SaveSceneAs => ("", Icon::SaveAs),
            Do::SaveSettings => ("", Icon::SaveSettings),
            Do::Advanced => ("Advanced", Icon::Sliders),
            Do::CpuView => ("CPU", Icon::Cpu),
            Do::Orbit => ("", Icon::Orbit),
            Do::Walk => ("Walk", Icon::Walk),
            Do::Flashlight => ("Flashlight", Icon::Flashlight),
            Do::Denoiser => ("", Icon::Denoiser),
            Do::FreezeReference => ("Freeze A/B", Icon::Freeze),
            Do::Restart => ("Restart", Icon::Restart),
            Do::FrameScene => ("Frame scene", Icon::Frame),
            Do::ResetCamera => ("Reset camera", Icon::Home),
            Do::SavePng => ("Save image", Icon::Export),
            Do::HidePanel => ("Hide panels", Icon::HidePanel),
            Do::UpdateCheck => ("", Icon::Update),
            Do::LockExposure => ("AE-L", Icon::Lock),
            Do::Compare => ("Compare", Icon::Compare),
            Do::Randomize => ("Randomize", Icon::Dice),
            Do::Today => ("Today", Icon::Today),
            Do::HighRes => ("High-res…", Icon::HighRes),
        },
        Pin::Widget(w) => return (w.label(), None),
    };
    let label = if short.is_empty() {
        pin_entry(pin).map_or("?", |e| e.label)
    } else {
        short
    };
    (label, Some(icon))
}
/// A pinned button's tooltip: the palette's name for it and its key.
fn pin_tip(pin: Pin) -> String {
    match pin_entry(pin) {
        Some(e) if e.key.is_empty() => e.label.to_string(),
        Some(e) => format!("{} ({})", e.label, e.key),
        None => pin_short(pin).0.to_string(),
    }
}
/// How wide a pin is in the bar, with or without its label.
fn pin_width(ui: &egui::Ui, pin: Pin, compact: bool) -> f32 {
    match pin {
        Pin::Widget(Widget::Denoiser) => 112.,
        Pin::Widget(_) => 84.,
        _ => {
            let (label, icon) = pin_short(pin);
            promo_size(ui, icon, (!compact || icon.is_none()).then_some(label)).0
        }
    }
}
/// A pinned button's width and its text.
fn promo_size(
    ui: &egui::Ui,
    icon: Option<Icon>,
    label: Option<&str>,
) -> (f32, Option<std::sync::Arc<egui::Galley>>) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = label.map(|l| {
        ui.painter()
            .layout_no_wrap(l.to_owned(), font, egui::Color32::PLACEHOLDER)
    });
    let mut width = 18.;
    if icon.is_some() {
        width += 14.;
    }
    if let Some(g) = &galley {
        width += g.size().x + if icon.is_some() { 6. } else { 0. };
    }
    (width, galley)
}
/// A pinned top-bar button: an icon, a label or both, filled while its window is open or its
/// toggle is on.
fn promo(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: Option<&str>,
    pressed: bool,
) -> egui::Response {
    let (width, galley) = promo_size(ui, icon, label);
    let height = ui.spacing().interact_size.y + 2.;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, edge) = if pressed {
            (SELECT, SELECT)
        } else if r.hovered() {
            (
                egui::Color32::from_rgb(48, 58, 60),
                egui::Color32::from_rgb(75, 95, 99),
            )
        } else {
            (
                egui::Color32::from_rgb(38, 42, 43),
                egui::Color32::from_rgb(53, 71, 75),
            )
        };
        let text = if pressed {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(226)
        };
        ui.painter().rect(
            rect,
            4.,
            fill,
            egui::Stroke::new(1_f32, edge),
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + 9.;
        if let Some(icon) = icon {
            let at = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - 7.),
                egui::Vec2::splat(14.),
            );
            icons::paint(ui.painter(), at, icon, 14., text);
            x += 20.;
        }
        if let Some(g) = galley {
            ui.painter()
                .galley(egui::pos2(x, rect.center().y - g.size().y / 2.), g, text);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    /// The About box unwraps these, so a bad file must fail here and not in the menu.
    #[test]
    fn logos_decode() {
        for (bytes, ..) in super::LOGOS {
            let logo = image::load_from_memory(bytes).unwrap();
            assert!(logo.width() > 0 && logo.height() > 0);
        }
        assert!(super::logo_shown(false));
        assert_eq!(super::logo_shown(true), cfg!(windows));
    }
}

/// The status bar's exposure: the displayed EV and, in auto, the adapted luminance in cd/m2
/// (the metered scene luminance, `Adaptation::scene_log2`, in physical units).
fn exposure_readout(s: &Settings, m: Option<crate::reference::display::Adaptation>) -> String {
    match m.filter(|m| s.auto_exposure && m.valid) {
        Some(m) => format!(
            "{:+.1} EV · {} cd/m2",
            m.current + s.ev_bias,
            cd_text(crate::photometry::scene_to_cd(m.scene_log2.exp2()))
        ),
        None => format!("{:+.1} EV", s.exposure.max(1e-6).log2()),
    }
}
fn cd_text(cd: f32) -> String {
    if cd >= 100. {
        format!("{cd:.0}")
    } else if cd >= 1. {
        format!("{cd:.1}")
    } else {
        format!("{cd:.2e}")
    }
}
