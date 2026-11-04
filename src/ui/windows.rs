//! Context menu, shortcut sheet, Scenes window and the tool windows (scopes, convergence graph, remote, stats).
use super::*;

impl App {
    /// The viewport's right-click menu, kept open at its position until a click elsewhere.
    pub(super) fn context_menu(&mut self, ctx: &egui::Context) {
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
    pub(super) fn shortcuts_window(&mut self, ctx: &egui::Context) {
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
    pub(super) fn scene_entry(&self) -> Option<usize> {
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
    pub(super) fn request_thumbnails(&mut self, all: bool) {
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
    pub(super) fn update_thumbnails(&mut self, ctx: &egui::Context) {
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
    pub(super) fn scenes_window(&mut self, ctx: &egui::Context) {
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
    pub(super) fn tool_windows(&mut self, ctx: &egui::Context) {
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
        if self.ui.console.open {
            self.console_window(ctx);
        }
        if self.ui.paths.open
            && crate::paths::window(ctx, &mut self.ui.paths, self.settings.perspective())
        {
            self.ui.paths.again();
        }
    }
    pub(super) fn scopes_window(&mut self, ctx: &egui::Context) {
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
    pub(super) fn graph_window(&mut self, ctx: &egui::Context) {
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
                    && let Some(p) = rfd::FileDialog::new().add_filter("Linear EXR", &["exr"]).set_directory(std::path::absolute(crate::dirs::captures()).unwrap_or_default()).pick_file()
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
    pub(super) fn remote_window(&mut self, ctx: &egui::Context) {
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
    pub(super) fn stats_window(&mut self, ctx: &egui::Context) {
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
}
