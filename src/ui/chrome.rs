//! Top bar, status bar, toasts and the side panel frame.
use super::*;

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
                ui.close_menu();
            }
        }
    }
    pub(super) fn top_bar(&mut self, ctx: &egui::Context, known: bool) {
        egui::TopBottomPanel::top("top bar").show(ctx, |ui| {
            // Hide/Show panel and ? are laid out first, from the right, so they are never pushed
            // off screen; the clusters wrap in what is left, and only between clusters.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui
                    .selectable_label(self.ui.show_shortcuts, "?")
                    .on_hover_text("Keyboard and mouse (F1)")
                    .clicked()
                {
                    self.ui.show_shortcuts = !self.ui.show_shortcuts;
                }
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
                // One row's height: with the window's full height a wrapped bar centres itself
                // mid-screen and its separators run down the whole window. Extra rows grow down.
                // A gap before the buttons: a combo box can run past the wrap point by its arrow.
                ui.add_space(32.);
                let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                    |ui| {
                ui.menu_button("Menu", |ui| {
                    ui.weak(format!("Caustica {}", crate::VERSION));
                    ui.separator();
                    ui.checkbox(&mut self.ui.advanced, "Show advanced settings");
                    let traces = self.gpu.as_ref().is_some_and(|g| g.ray_tracing);
                    ui.add_enabled(
                        traces,
                        egui::Checkbox::new(&mut self.cpu_view, "Render on the CPU (software)"),
                    )
                    .on_hover_text(CPU_VIEW_HELP)
                    .on_disabled_hover_text(NO_RT_HELP);
                    if ui.button("Phone remote…").clicked() {
                        self.ui.show_remote = true;
                        ui.close_menu();
                    }
                    if ui.button("Script console…").clicked() {
                        self.ui.console.open = true;
                        ui.close_menu();
                    }
                    if ui.button("Statistics and achievements").clicked() {
                        self.ui.show_stats = true;
                        ui.close_menu();
                    }
                    if ui
                        .checkbox(&mut self.ui.prefs.update_check, "Check for updates at startup")
                        .changed()
                    {
                        self.ui.save_prefs();
                    }
                    ui.menu_button("Text-to-scene model", |ui| self.ai_menu(ui));
                    #[cfg(windows)]
                    ui.menu_button("Renderer", |ui| self.renderer_menu(ui));
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
                        ui.close_menu();
                    }
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
                    if ui.button("Save settings.json")
                        .on_hover_text("A bare settings file, scene included, overwritten each time in the output folder. Save scene writes a titled scene file into your scenes instead.").clicked() {
                        self.actions.save = Some(crate::dirs::output("settings.json"));
                        ui.close_menu();
                    }
                    if ui.button("Save settings as…")
                        .on_hover_text("A bare settings file, scene included, at a place you pick. Save scene as… writes a titled scene file into your scenes instead.").clicked() {
                        self.actions.save = settings_dialog(true);
                        ui.close_menu();
                    }
                    if ui.button("Load settings…")
                        .on_hover_text("Replaces the scene and every setting from a bare settings file. Open scene… loads a titled scene file.").clicked() {
                        self.actions.load = settings_dialog(false);
                        ui.close_menu();
                    }
                    if ui.button("Reset everything to defaults").clicked() {
                        self.ui.checkpoint_scene(&self.settings);
                        self.settings = Settings::default();
                        self.preset = 0;
                        self.ui.selection = Selection::Stage;
                        self.ui.mark_clean(&self.settings);
                        self.ui.toast("Reset to the default scene and settings");
                        ui.close_menu();
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
                    ui.separator();
                    ui.label(egui::RichText::new("Keys").strong());
                    for line in [
                        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom",
                        "Click: select an object or light · Delete: remove it · Esc: deselect",
                        "G / R / S over the viewport: move, rotate, scale · then X / Y / Z, a value, Enter",
                        "Ctrl+Z / Ctrl+Y: undo / redo · Ctrl+S / Ctrl+O: save / open",
                        "Space: pause · R: restart (no object or light selected) · F: frame · O: auto orbit",
                        "Alt+D: cycle the denoiser · C: freeze A/B reference · 1–8: views",
                        "F12: save PNG · Tab: hide panels · Ctrl+K: find a setting · F1: all keys",
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
                let traces = self.gpu.as_ref().is_some_and(|g| g.ray_tracing);
                cluster(ui, "Render on", 110.);
                ui.add_enabled_ui(traces, |ui| {
                    ui.selectable_value(&mut self.cpu_view, false, "GPU")
                        .on_hover_text("The viewport shows the GPU's path tracer")
                        .on_disabled_hover_text(NO_RT_HELP);
                });
                ui.selectable_value(&mut self.cpu_view, true, "CPU")
                    .on_hover_text(CPU_VIEW_HELP);
                cluster(ui, "Scene", 620.);
                if ui
                    .button("◀")
                    .on_hover_text("Previous scene (Page Up): in the Scenes list while a scene file is loaded, else in the presets")
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
                let current = if self.ui.scene_dirty {
                    format!("{current} *")
                } else {
                    current
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
                                    self.switch_scene("Scene", |s| s.preset(i));
                                }
                            }
                            ui.separator();
                        }
                        if ui
                            .button(format!("All scenes… ({})", self.ui.scenes.len()))
                            .on_hover_text("The full list with search and tags (Ctrl+P)")
                            .clicked()
                        {
                            self.ui.show_scenes = true;
                            ui.close_menu();
                        }
                    })
                    .response
                    .on_hover_text(
                        "The built-in presets; All scenes… opens the full list. Picking the current one again restores it. A * marks unsaved changes.",
                    );
                if ui
                    .button("▶")
                    .on_hover_text("Next scene (Page Down): in the Scenes list while a scene file is loaded, else in the presets")
                    .clicked()
                {
                    self.step_scene(1);
                }
                if ui
                    .selectable_label(
                        self.ui.show_scenes,
                        format!("Scenes ({})", self.ui.scenes.len()),
                    )
                    .on_hover_text("Browse the scene files in scenes/ (Ctrl+P)")
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
                    self.switch_scene("Random scene", |s| s.randomize(seed));
                }
                if ui
                    .button("Today")
                    .on_hover_text(
                        "Seed of the day: everyone running this today gets the same random scene",
                    )
                    .clicked()
                {
                    self.switch_scene("Seed of the day", |s| s.randomize(seed_of_the_day()));
                }
                cluster(ui, "Image", 340.);
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
                cluster(ui, "Playback", 380.);
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
                    .on_hover_text("Fit the camera to the selected object, or the whole scene (F)")
                    .clicked()
                {
                    self.frame_selection();
                }
                if ui
                    .selectable_label(self.walking.is_some(), "Walk")
                    .on_hover_text(
                        "Walk mode (`): WASD, Space jump, C or Ctrl crouch, Shift run, F fly                          (Q / E down / up), B head bob, T flashlight, wheel body size, drag to look",
                    )
                    .clicked()
                {
                    self.toggle_walk();
                }
                if ui
                    .selectable_label(
                        self.ui.show_timeline,
                        // Closed on a scene with keys: there is something in there.
                        if !self.ui.show_timeline && !self.settings.animation.is_empty() {
                            "Timeline •"
                        } else {
                            "Timeline"
                        },
                    )
                    .on_hover_text(
                        "Keyframes: I keys the selection (the camera when nothing is selected), K plays",
                    )
                    .clicked()
                {
                    self.ui.show_timeline = !self.ui.show_timeline;
                }
                cluster(ui, "Tools", 560.);
                for (tool, label, tip) in [
                    (gizmo::Tool::Select, "Select", "Click to select, no handles"),
                    (
                        gizmo::Tool::Move,
                        "Move",
                        "Handles for dragging: arrows and planes. G starts a keyboard move",
                    ),
                    (
                        gizmo::Tool::Rotate,
                        "Rotate",
                        "Handles for dragging: rings. R starts a keyboard rotation",
                    ),
                    (
                        gizmo::Tool::Scale,
                        "Scale",
                        "Handles for dragging; the circle scales evenly. S starts a keyboard scale",
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
                    },
                );
            });
        });
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
        egui::Area::new(egui::Id::new("cpu view message"))
            .order(egui::Order::Middle)
            .interactable(false)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(8., 8.))
            .constrain_to(ctx.available_rect())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.);
                    ui.label(egui::RichText::new(text).small().color(WARN));
                });
            });
    }
    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
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
        egui::TopBottomPanel::bottom("status bar").show(ctx, |ui| {
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
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10., 10.), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4., color);
                ui.label(state);
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
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-8., -8.])
            .constrain_to(ctx.available_rect())
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
    pub(super) fn side_panel(&mut self, ctx: &egui::Context, known: bool) {
        let panel = egui::SidePanel::right("side panel")
            .resizable(true)
            .default_width(self.ui.prefs.panel_width)
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
                // Each tab keeps its own offset: a short tab would clamp a shared one.
                egui::ScrollArea::vertical()
                    .id_salt(self.ui.tab as u8)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| match self.ui.tab {
                        Tab::Scene => self.scene_tab(ui, known),
                        Tab::Render => self.render_tab(ui),
                        Tab::Look => self.look_tab(ui),
                        Tab::Export => self.export_tab(ui),
                    });
            });
        // Only the user's drag counts: the panel also widens to fit wide content.
        if ctx.is_being_dragged(egui::Id::new("side panel").with("__resize")) {
            self.ui.panel_resized = true;
        }
        self.ui.panel_width = panel.response.rect.width();
    }
}

/// Starts a labelled group of top-bar controls. In a wrapping row it moves to the next row when
/// `width` (the group's estimated size, in points) does not fit, so a group is never split;
/// otherwise it follows the previous group after a separator.
fn cluster(ui: &mut egui::Ui, name: &str, width: f32) {
    let at_start = ui.cursor().min.x <= ui.max_rect().min.x + 1.;
    if !at_start {
        if ui.available_size_before_wrap().x < width {
            ui.end_row();
        } else {
            ui.separator();
        }
    }
    ui.label(egui::RichText::new(name).small().weak());
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
