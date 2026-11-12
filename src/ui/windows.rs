//! Context menu, shortcut sheet, Scenes window and the tool windows (scopes, convergence graph, remote, stats).
use super::*;
use crate::palette;
use crate::ui::skin::ShowSkin as _;

/// The browser's arrow keys, with no modifier (Shift+Left and Right step the timeline frame): Up
/// and Down, and in the search box they alone, as Left and Right move its cursor.
fn scene_step(ui: &mut egui::Ui, keys_free: bool, in_search: bool) -> i32 {
    if !(keys_free || in_search) || !ui.input(|i| i.modifiers.is_none()) {
        return 0;
    }
    ui.input_mut(|i| {
        let mut key = |key| i.consume_key(egui::Modifiers::NONE, key) as i32;
        let down = key(egui::Key::ArrowDown) - key(egui::Key::ArrowUp);
        if in_search {
            down
        } else {
            down + key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft)
        }
    })
}
/// The card `step` on from the highlighted one among those the filters show, stopping at the ends.
/// A highlight the filters hide starts from the first card.
fn step_card(visible: &[usize], selected: Option<usize>, step: i32) -> Option<usize> {
    let at = selected.and_then(|s| visible.iter().position(|v| *v == s));
    let next = match at {
        Some(at) => (at as i32 + step).clamp(0, visible.len() as i32 - 1) as usize,
        None => 0,
    };
    visible.get(next).copied()
}
/// Enter in the search box: the highlighted card if the filters show it, else the first.
fn enter_card(visible: &[usize], selected: Option<usize>) -> Option<usize> {
    selected
        .filter(|s| visible.contains(s))
        .or(visible.first().copied())
}
/// The scene cards' widths in points: small, medium and large (`Prefs::scene_cards`). Three small
/// cards fit the narrowest browser (about 340 pt between the sidebar and the details).
const CARD_WIDTHS: [f32; 3] = [104., 160., 232.];
/// True for the user's own scenes, as opposed to the shipped ones: those in the `user` pack
/// (Save current scene, a saved generator seed, Save as… into it) and, when installed, any pack
/// in the per-user scenes folder (`user_root`; in the other modes that is the shipped folder).
fn own_scene(e: &crate::scenes::Entry, user_root: Option<&std::path::Path>) -> bool {
    e.path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "user")
        || user_root.is_some_and(|root| e.path.starts_with(root))
}
/// The tags the Scenes window shows as chips, in this order (the ones that exist).
const TOP_TAGS: [&str; 10] = [
    "material",
    "glass",
    "interior",
    "emissive",
    "night",
    "caustics",
    "physics",
    "still life",
    "outdoor",
    "generated",
];
/// The scene tags split into the chips (`TOP_TAGS` that occur) and the rest with their scene
/// counts, alphabetical, for the 'More tags' popup.
fn split_tags<'a>(all: impl Iterator<Item = &'a str>) -> (Vec<&'static str>, Vec<(String, usize)>) {
    let mut count: std::collections::BTreeMap<&str, usize> = Default::default();
    for t in all {
        *count.entry(t).or_default() += 1;
    }
    let top = TOP_TAGS
        .iter()
        .copied()
        .filter(|t| count.contains_key(t))
        .collect();
    let more = count
        .into_iter()
        .filter(|(t, _)| !TOP_TAGS.contains(t))
        .map(|(t, n)| (t.to_string(), n))
        .collect();
    (top, more)
}
/// A scene card: the board's card with the thumbnail (or the scene's initial) in it and the
/// title under it, all the same size (`width` points of picture, 16:10).
fn scene_card(
    ui: &mut egui::Ui,
    e: &crate::scenes::Entry,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
    width: f32,
) -> egui::Response {
    use crate::ui::skin;
    const PAD: f32 = 4.;
    let size = egui::vec2(width, width * 0.625);
    let (rect, r) = ui.allocate_exact_size(
        egui::vec2(size.x + 2. * PAD, size.y + 2. * PAD + 22.),
        egui::Sense::click(),
    );
    skin::card_face(ui, &r, rect, selected);
    let image = egui::Rect::from_min_size(rect.min + egui::vec2(PAD, PAD), size);
    match texture {
        Some(t) => {
            egui::Image::new((t.id(), size))
                .corner_radius(3.)
                .paint_at(ui, image);
        }
        None => {
            ui.painter()
                .rect_filled(image, 3., egui::Color32::from_rgb(0x14, 0x14, 0x16));
            ui.painter().text(
                image.center(),
                egui::Align2::CENTER_CENTER,
                e.title.chars().next().unwrap_or('?'),
                skin::semibold(size.y * 0.4),
                skin::FAINT,
            );
        }
    }
    let mut job = egui::text::LayoutJob::single_section(
        e.title.clone(),
        egui::TextFormat {
            font_id: skin::sans(12.5),
            color: skin::TX,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(size.x - 2.);
    let title = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(image.min.x + 1., image.max.y + 5.),
        title,
        skin::TX,
    );
    r.on_hover_text(&e.description)
}
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
        let r = egui::Area::new(egui::Id::unique("viewport menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .info(egui::UiStackInfo::new(egui::UiKind::Menu))
            .show(ctx, |ui| {
                crate::ui::skin::menu(ui, |ui| {
                    ui.set_min_width(170.);
                    let name = match self.ui.selection {
                        Selection::Object(i) => {
                            self.settings.objects.get(i).map(|o| o.name.clone())
                        }
                        Selection::Light(i) => self.settings.lights.get(i).map(|l| l.name.clone()),
                        _ => None,
                    };
                    if let Some(name) = name {
                        ui.label(egui::RichText::new(name).small());
                    }
                    chosen = context_items(ui, selected);
                    crate::ui::skin::separator(ui);
                    if crate::ui::skin::button(ui, "Frame the scene").clicked() {
                        chosen = Some(Command::FrameScene);
                    }
                    if crate::ui::skin::button(ui, "Reset the camera (Home)").clicked() {
                        chosen = Some(Command::ResetCamera);
                    }
                    if crate::ui::skin::button(ui, "Show all hidden (Alt+H)").clicked() {
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
        crate::ui::skin::window("Keyboard and mouse")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .resizable(false)
            .collapsible(false)
            .show_skin(ctx, |ui| {
                // Three columns side by side, so the sheet fits a laptop screen.
                ui.horizontal_top(|ui| {
                    for column in SHORTCUT_COLUMNS {
                        ui.vertical(|ui| {
                            for &i in column {
                                let (group, keys) = SHORTCUTS[i];
                                section(ui, group);
                                // Long rows wrap: the sheet must fit a 1280 wide window.
                                grid(group).max_col_width(200.).show(ui, |ui| {
                                    for (k, what) in keys {
                                        ui.label(
                                            egui::RichText::new(*k)
                                                .font(crate::ui::skin::semibold(12.5)),
                                        );
                                        ui.label(*what);
                                        ui.end_row();
                                    }
                                });
                            }
                        });
                        ui.add_space(12.);
                    }
                });
            });
        self.ui.show_shortcuts = open;
    }
    /// Ctrl+K: type a setting or command, arrows and Enter go there, Esc closes.
    pub(super) fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.ui.palette.open {
            self.ui.palette.was_open = false;
            return;
        }
        let opened = !self.ui.palette.was_open;
        self.ui.palette.was_open = true;
        // A right-click on a command or window pins it to the top bar, or unpins it.
        let mut pins = self.ui.prefs.pins();
        let mut pin_toggle: Option<crate::prefs::Pin> = None;
        // Before the box: it would take Up and Down for the ends of its text.
        let (down, enter, esc, toggle) = ctx.input_mut(|i| {
            let mut k = |m, key| i.consume_key(m, key);
            let none = egui::Modifiers::NONE;
            (
                k(none, egui::Key::ArrowDown) as i32 - k(none, egui::Key::ArrowUp) as i32,
                k(none, egui::Key::Enter),
                k(none, egui::Key::Escape),
                k(egui::Modifiers::COMMAND, egui::Key::K),
            )
        });
        let mut run = None;
        // The opening press is still in egui's input on the first frame: it must not close the box again.
        let mut close = esc || (toggle && !opened);
        let mut changed = false;
        egui::Area::new(egui::Id::unique("command palette"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, [0., 70.])
            .show(ctx, |ui| {
                crate::ui::skin::card(ui, |ui| {
                    ui.set_width(460.);
                    let st = &mut self.ui.palette;
                    let r = crate::ui::skin::text_edit(
                        ui,
                        f32::INFINITY,
                        egui::TextEdit::singleline(&mut st.query)
                            .hint_text(field_hint("Find a setting or command…")),
                    );
                    if opened {
                        r.request_focus();
                    }
                    changed = r.changed();
                    if changed {
                        st.at = 0;
                    }
                    let hits = palette::matches(&st.query);
                    st.at = (st.at as i32 + down).clamp(0, hits.len().max(1) as i32 - 1) as usize;
                    if hits.is_empty() {
                        note(ui, "Nothing matches");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(340.)
                        .show(ui, |ui| {
                            for (n, &i) in hits.iter().enumerate() {
                                let e = &palette::ENTRIES[i];
                                let r = ui
                                    .horizontal(|ui| {
                                        let r =
                                            crate::ui::skin::selectable(ui, n == st.at, e.label);
                                        if !e.key.is_empty() {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.weak(e.key);
                                                },
                                            );
                                        }
                                        r
                                    })
                                    .inner;
                                if n == st.at && down != 0 {
                                    r.scroll_to_me(None);
                                }
                                if r.clicked() {
                                    run = Some(e.act);
                                }
                                let pin = match e.act {
                                    palette::Act::Do(d) => Some(crate::prefs::Pin::Do(d)),
                                    palette::Act::Window(w) => Some(crate::prefs::Pin::Window(w)),
                                    palette::Act::Go(..) => None,
                                };
                                if let Some(pin) = pin {
                                    crate::ui::skin::context_menu(&r, |ui| {
                                        let label = if pins.contains(&pin) {
                                            "Unpin from top bar"
                                        } else {
                                            "Pin to top bar"
                                        };
                                        if crate::ui::skin::button(ui, label).clicked() {
                                            pin_toggle = Some(pin);
                                            ui.close();
                                        }
                                    });
                                }
                            }
                        });
                    if enter && let Some(&i) = hits.get(st.at) {
                        run = Some(palette::ENTRIES[i].act);
                    }
                });
            });
        if let Some(pin) = pin_toggle {
            if pins.contains(&pin) {
                pins.retain(|p| *p != pin);
            } else {
                pins.push(pin);
            }
            self.ui.prefs.set_pins(&pins);
            self.ui.save_prefs();
        }
        if run.is_some() {
            close = true;
        }
        if close {
            self.ui.palette.open = false;
        }
        if let Some(a) = run {
            self.run_palette(a);
        }
    }
    /// What a palette entry does. A setting opens its tab and selects what shows it.
    pub fn run_palette(&mut self, a: palette::Act) {
        use palette::{Act, Do, Focus, Win};
        match a {
            Act::Go(tab, focus, section) => {
                let s = &self.settings;
                let sel = match focus {
                    Focus::Keep => None,
                    Focus::Stage => Some(Selection::Stage),
                    Focus::Camera => Some(Selection::Camera),
                    Focus::MainLight => Some(Selection::MainLight),
                    Focus::Flashlight => Some(Selection::Flashlight),
                    Focus::Object => match self.ui.selection {
                        sel @ Selection::Object(_) => Some(sel),
                        _ if !s.objects.is_empty() => Some(Selection::Object(0)),
                        _ => {
                            self.ui.toast("No object in the scene yet");
                            return;
                        }
                    },
                    Focus::AreaLight => match self.ui.selection {
                        sel @ Selection::Light(_) => Some(sel),
                        _ if !s.lights.is_empty() => Some(Selection::Light(0)),
                        _ => {
                            self.ui.toast("No area light in the scene yet");
                            return;
                        }
                    },
                };
                self.ui.tab = tab;
                if let Some(sel) = sel {
                    self.ui.select(sel);
                    self.ui.scroll_to_inspector = true;
                }
                // The outliner is folded above Render, Look and Export.
                self.ui.outliner_open |= section == "Outliner";
                self.ui.panel_hidden = false;
                self.show_ui = true;
                self.ui.toast(format!("{section}: {} tab", tab.label()));
            }
            Act::Window(w) => match w {
                Win::Remote => self.ui.show_remote = true,
                Win::Console => self.ui.console.open = true,
                Win::Stats => self.ui.show_stats = true,
                Win::Shortcuts => self.ui.show_shortcuts = true,
                Win::Scenes => self.ui.show_scenes = true,
                Win::Assets => self.ui.assets.open = true,
                Win::Video => self.ui.show_video = true,
                Win::Scopes => self.ui.show_scopes = true,
                Win::Graph => self.ui.show_graph = true,
                Win::Paths => self.ui.paths.open = true,
                Win::Describe => self.ui.show_describe = true,
                Win::Puzzle => self.ui.show_puzzle = true,
            },
            Act::Do(d) => match d {
                Do::OpenScene => self.open_scene_dialog(),
                Do::SaveScene => self.save_scene(false),
                Do::SaveSceneAs => self.save_scene(true),
                Do::SaveSettings => {
                    self.actions.save = Some(crate::dirs::output("settings.json"));
                }
                Do::Advanced => self.ui.advanced = !self.ui.advanced,
                Do::CpuView => {
                    if self.gpu.as_ref().is_some_and(|g| g.ray_tracing) {
                        self.cpu_view = !self.cpu_view;
                    } else {
                        self.ui.toast(chrome::NO_RT_HELP);
                    }
                }
                Do::Orbit => self.toggle_orbit(),
                Do::Walk => self.toggle_walk(),
                Do::Flashlight => self.toggle_flashlight(),
                Do::Denoiser => self.cycle_denoiser(),
                Do::FreezeReference => {
                    self.actions.reference = true;
                    self.ui
                        .toast("Reference frozen: Compare in the top bar or Look > Compare shows the split");
                }
                Do::Restart => {
                    self.actions.reset = true;
                    self.ui.toast("Restarted");
                }
                Do::FrameScene => self.run_command(Command::FrameScene),
                Do::ResetCamera => self.reset_camera(),
                Do::SavePng => self.actions.save_image = Some(crate::video::ImageFormat::Png),
                Do::HidePanel => self.ui.panel_hidden = !self.ui.panel_hidden,
                // Auto off; the frame loop seeds the manual exposure from the meter (E7b).
                Do::LockExposure => self.settings.auto_exposure = false,
                Do::UpdateCheck => {
                    self.ui.prefs.update_check = !self.ui.prefs.update_check;
                    self.ui.save_prefs();
                }
                Do::Compare => self.toggle_compare(),
                Do::Randomize => {
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos()
                        | 1;
                    self.switch_scene("Random scene", |s| s.randomize(seed));
                }
                Do::Today => {
                    self.switch_scene("Seed of the day", |s| s.randomize(seed_of_the_day()));
                }
                Do::HighRes => {
                    self.ui.tab = Tab::Export;
                    self.ui.panel_hidden = false;
                    self.show_ui = true;
                }
            },
        }
    }
    /// The top bar's Compare: the A/B split on or off. With no reference yet it freezes the
    /// current image first, so the split has something to show.
    pub fn toggle_compare(&mut self) {
        if self.cpu_view {
            self.ui
                .toast("The viewport shows the CPU's render: nothing to compare against");
            return;
        }
        if self.settings.compare {
            self.settings.compare = false;
            return;
        }
        if !self.gpu.as_ref().is_some_and(|g| g.reference_valid) {
            self.actions.reference = true;
            self.ui
                .toast("Reference frozen: change something to see the difference");
        }
        self.settings.compare = true;
    }
    /// Whether a pinned command reads as pressed (toggles and window openers).
    pub(crate) fn pin_active(&self, pin: crate::prefs::Pin) -> bool {
        use crate::prefs::Pin;
        use palette::{Do, Win};
        match pin {
            Pin::Window(w) => match w {
                Win::Remote => self.ui.show_remote,
                Win::Console => self.ui.console.open,
                Win::Stats => self.ui.show_stats,
                Win::Shortcuts => self.ui.show_shortcuts,
                Win::Scenes => self.ui.show_scenes,
                Win::Assets => self.ui.assets.open,
                Win::Video => self.ui.show_video,
                Win::Scopes => self.ui.show_scopes,
                Win::Graph => self.ui.show_graph,
                Win::Paths => self.ui.paths.open,
                Win::Describe => self.ui.show_describe,
                Win::Puzzle => self.ui.show_puzzle,
            },
            Pin::Do(d) => match d {
                Do::Advanced => self.ui.advanced,
                Do::CpuView => self.cpu_view,
                Do::Orbit => self.settings.rotate,
                Do::Walk => self.walking.is_some(),
                Do::Flashlight => self.settings.flashlight,
                Do::Compare => self.settings.compare,
                Do::UpdateCheck => self.ui.prefs.update_check,
                _ => false,
            },
            Pin::Widget(_) => false,
        }
    }
    /// A pinned button's click: windows toggle, commands run.
    pub(crate) fn run_pin(&mut self, pin: crate::prefs::Pin) {
        use crate::prefs::Pin;
        use palette::{Act, Win};
        match pin {
            Pin::Window(w) if self.pin_active(pin) => match w {
                Win::Remote => self.ui.show_remote = false,
                Win::Console => self.ui.console.open = false,
                Win::Stats => self.ui.show_stats = false,
                Win::Shortcuts => self.ui.show_shortcuts = false,
                Win::Scenes => self.ui.show_scenes = false,
                Win::Assets => self.ui.assets.open = false,
                Win::Video => self.ui.show_video = false,
                Win::Scopes => self.ui.show_scopes = false,
                Win::Graph => self.ui.show_graph = false,
                Win::Paths => self.ui.paths.open = false,
                Win::Describe => self.ui.show_describe = false,
                Win::Puzzle => self.ui.show_puzzle = false,
            },
            Pin::Window(w) => self.run_palette(Act::Window(w)),
            Pin::Do(d) => self.run_palette(Act::Do(d)),
            Pin::Widget(_) => {}
        }
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
        self.ui.checkpoint_scene(&self.settings);
        self.laser = crate::laser::Laser::default();
        match crate::scenes::load_into(&mut self.settings, &e.path) {
            Ok(()) => {
                // Imports still parsing were meant for the old scene.
                self.ui.mesh_imports.clear();
                self.ui.mark_clean(&self.settings);
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
        // Stale thumbnails go and bundled ones are copied in first; "all" re-renders regardless.
        crate::scenes::sync_thumbnails(&self.ui.scenes);
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
            self.ui.thumbnail_jobs = Some((
                crate::scenes::render_thumbnails(missing, self.cpu_view),
                total,
                0,
            ));
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
        let opened = self.ui.show_scenes && !self.ui.scenes_were_open;
        self.ui.scenes_were_open = self.ui.show_scenes;
        if !self.ui.show_scenes {
            return;
        }
        // The arrows and Enter drive the cards unless another text field has them.
        let keys_free = !self.text_focus && self.ui.transform.is_none();
        // Headless runs (captures, self-test) never spawn renders.
        if !self.ui.thumbnails_requested && !self.hidden {
            self.request_thumbnails(false);
        }
        self.update_thumbnails(ctx);
        let open = self.ui.show_scenes;
        let mut load = None;
        let mut rerender = false;
        let mut close = false;
        // Over the viewport, not a floating window: a sidebar (library, tags, create), the cards
        // and the selected scene's details. The image keeps rendering behind it.
        let rect = central_rect(ctx);
        crate::ui::skin::window("Scenes")
            .title_bar(false)
            .resizable(false)
            .fixed_rect(rect)
            .frame(crate::ui::skin::panel_frame())
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                let state = &mut self.ui;
                let search_id = ui.make_persistent_id("scene search");
                let in_search = ui.memory(|m| m.has_focus(search_id));
                // Before the box: it would take Up and Down for the start and end of its text.
                // Opening shows the cards, whatever page was left open.
                if opened {
                    state.scene_page = 0;
                    state.scene_dates.clear();
                }
                let user_root = (crate::dirs::mode() == crate::dirs::Mode::Installed)
                    .then(crate::dirs::user_scenes)
                    .flatten();
                let own: Vec<bool> = state.scenes.iter().map(|e| own_scene(e, user_root.as_deref())).collect();
                let step = if state.scene_page == 0 {
                    scene_step(ui, keys_free, in_search)
                } else {
                    0
                };
                let searched = state.scene_search.clone();
                let mut enter = false;
                let (top, more) = split_tags(state.scenes.iter().flat_map(|e| e.tags.iter().map(String::as_str)));
                let header = |ui: &mut egui::Ui, text: &str| {
                    ui.add_space(10.);
                    crate::ui::skin::heading(ui, text);
                    ui.add_space(2.);
                };
                egui::Panel::left("scenes side")
                    .exact_size(200.)
                    .resizable(false)
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("scenes side").show(ui, |ui| {
                        header(ui, "LIBRARY");
                        let counts = [
                            state.scenes.len(),
                            state.scenes.iter().filter(|e| state.prefs.is_favorite(&e.path)).count(),
                            state.scenes.iter().filter(|e| state.prefs.recency(&e.path).is_some()).count(),
                            own.iter().filter(|o| **o).count(),
                        ];
                        for (i, name) in ["All scenes", "Favorites", "Recent", "My scenes"].iter().enumerate() {
                            let on = state.scene_page == 0 && state.scene_list == i as u8;
                            if tree_row(ui, 0., on, name, &counts[i].to_string()).clicked() {
                                state.scene_list = i as u8;
                                state.scene_page = 0;
                            }
                        }
                        header(ui, "TAGS");
                        for t in &top {
                            let on = state.scene_tag.as_deref() == Some(*t);
                            let n = state.scenes.iter().filter(|e| e.tags.iter().any(|x| x == t)).count();
                            if tree_row(ui, 0., on, t, &n.to_string())
                                .on_hover_text("Only scenes with this tag; click again to clear")
                                .clicked()
                            {
                                state.scene_tag = if on { None } else { Some(t.to_string()) };
                                state.scene_page = 0;
                            }
                        }
                        // A tag chosen from the popup stays visible as a row while it is active.
                        let hidden = state.scene_tag.clone().filter(|t| more.iter().any(|(m, _)| m == t));
                        if let Some(t) = hidden
                            && tree_row(ui, 0., true, &t, "clear").clicked()
                        {
                            state.scene_tag = None;
                        }
                        crate::ui::skin::menu_button_raised(ui, format!("More tags ({})", more.len()), |ui| {
                            egui::ScrollArea::vertical().max_height(320.).show(ui, |ui| {
                                for (t, n) in &more {
                                    let on = state.scene_tag.as_deref() == Some(t.as_str());
                                    if crate::ui::skin::selectable(ui, on, format!("{t}  ({n})")).clicked() {
                                        state.scene_tag = if on { None } else { Some(t.clone()) };
                                        ui.close();
                                    }
                                }
                            });
                        })
                        .response
                        .on_hover_text("Every other tag, with how many scenes have it. Search matches all tags too.");
                        header(ui, "CREATE");
                        let families = format!("{} families", crate::generators::FAMILIES.len());
                        if tree_row(ui, 0., state.scene_page == 1, "Generate…", &families).clicked() {
                            state.scene_page = 1;
                        }
                        if tree_row(ui, 0., state.scene_page == 2, "Save current scene…", "").clicked() {
                            state.scene_page = 2;
                        }
                        });
                    });
                egui::Panel::right("scenes details")
                    .exact_size(300.)
                    .resizable(false)
                    .frame(egui::Frame::new().inner_margin(14))
                    .show(ui, |ui| {
                        if let Some(e) = state.scene_selected.and_then(|i| state.scenes.get(i).cloned()) {
                            let width = ui.available_width();
                            let size = egui::vec2(width, width * 0.625);
                            match e.thumbnail().and_then(|p| state.thumbnails.get(&p)) {
                                Some(t) => {
                                    ui.add(egui::Image::new((t.id(), size)).corner_radius(6.));
                                }
                                None => {
                                    placeholder(ui, &e.title, size);
                                }
                            }
                            ui.add_space(4.);
                            ui.label(egui::RichText::new(&e.title).heading());
                            ui.label(&e.description);
                            ui.horizontal_wrapped(|ui| {
                                license_badge(ui, &e.license);
                                for t in &e.tags {
                                    ui.label(egui::RichText::new(t).small());
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
                            ui.add_space(4.);
                            ui.horizontal(|ui| {
                                if crate::ui::skin::button(ui, "Load").clicked() {
                                    load = state.scene_selected;
                                }
                                let mut favorite = state.prefs.is_favorite(&e.path);
                                if crate::ui::skin::checkbox_label(ui, &mut favorite, "Favorite").changed() {
                                    state.prefs.toggle_favorite(&e.path);
                                    state.save_prefs();
                                }
                            });
                        } else {
                            note(ui, "Pick a scene. Your render settings (resolution, RR, upscaler, HDR) stay as they are.");
                        }
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                            note(ui, "Double-click a card to load. Enter loads the selection. Esc returns to the image.");
                        });
                    });
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                let mut refresh = false;
                ui.horizontal(|ui| {
                    // The search takes what More and Close leave, down to the narrowest browser.
                    let width = (ui.available_width() - 130.).clamp(80., 280.);
                    let r = crate::ui::skin::text_edit(ui, width, egui::TextEdit::singleline(&mut state.scene_search).id(search_id).hint_text(field_hint("Search scenes…")));
                    if opened {
                        r.request_focus();
                    }
                    // A single line gives the focus up on Enter and Esc.
                    if r.lost_focus() {
                        enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            if state.scene_search.is_empty() {
                                close = true;
                            } else {
                                state.scene_search.clear();
                                r.request_focus();
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::skin::button(ui, "Close").on_hover_text("Back to the image (Esc)").clicked() {
                            close = true;
                        }
                        // Refresh and the thumbnail render sit in a menu: the middle is too
                        // narrow for their buttons beside the search.
                        crate::ui::skin::menu_button_raised(ui, "More", |ui| {
                            if crate::ui::skin::button(ui, "Refresh").on_hover_text("Read the scenes folder again").clicked() {
                                refresh = true;
                                ui.close();
                            }
                            match &state.thumbnail_jobs {
                                Some((_, total, done)) => {
                                    ui.add_enabled(false, crate::ui::skin::Button::new(format!("Rendering thumbnails {done} / {total}")));
                                }
                                None => {
                                    if crate::ui::skin::button(ui, "Re-render thumbnails").on_hover_text("Render every thumbnail again in the background").clicked() {
                                        rerender = true;
                                        ui.close();
                                    }
                                }
                            }
                        });
                    });
                });
                if refresh {
                    state.scenes = crate::scenes::list();
                    state.scene_selected = None;
                    state.scene_dates.clear();
                }
                ui.add_space(6.);
                // Typing in the search box goes back to the cards it filters.
                if state.scene_search != searched {
                    state.scene_page = 0;
                }
                if state.scene_page == 1 {
                crate::ui::skin::title(ui, "Generate");
                let mut generate = false;
                ui.horizontal(|ui| {
                    let families = crate::generators::FAMILIES;
                    crate::ui::skin::ComboBox::from_id_salt("generator family")
                        .selected_text(families[state.gen_family].1)
                        .width(170.)
                        .show_ui(ui, |ui| {
                            for (i, (_, label, about)) in families.iter().enumerate() {
                                crate::ui::skin::selectable_value(ui, &mut state.gen_family, i, *label).on_hover_text(*about);
                            }
                        });
                    ui.label("seed");
                    crate::ui::skin::drag_value(ui, egui::DragValue::new(&mut state.gen_seed).speed(1.));
                    if crate::ui::skin::button(ui, "Generate").clicked() {
                        generate = true;
                    }
                    if crate::ui::skin::button(ui, "Next seed").clicked() {
                        state.gen_seed = state.gen_seed.wrapping_add(1);
                        generate = true;
                    }
                    if crate::ui::skin::button(ui, "Random").clicked() {
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
                            state.checkpoint_scene(&self.settings);
                            g.keep_output(&self.settings);
                            self.settings = g;
                            state.selection = Selection::Stage;
                            state.mark_clean(&self.settings);
                        }
                        Err(e) => state.error(format!("{e:#}")),
                    }
                }
                if crate::ui::skin::button(ui, "Save this generator and seed to scenes/user").clicked() {
                    let family = crate::generators::FAMILIES[state.gen_family].0;
                    let title = format!("{} {}", crate::generators::FAMILIES[state.gen_family].1, state.gen_seed);
                    match crate::scenes::save_generated(family, state.gen_seed, &title) {
                        Ok(path) => {
                            state.scenes = crate::scenes::list();
                            state.scene_dates.clear();
                            state.toast(format!("Saved {}", path.display()));
                        }
                        Err(e) => state.error(format!("{e:#}")),
                    }
                }
                } else if state.scene_page == 2 {
                crate::ui::skin::title(ui, "Save the current scene");
                ui.horizontal(|ui| {
                    crate::ui::skin::text_edit(ui, 180., egui::TextEdit::singleline(&mut state.save_title).hint_text(field_hint("Title")));
                    crate::ui::skin::text_edit(ui, 260., egui::TextEdit::singleline(&mut state.save_description).hint_text(field_hint("Description")));
                    if crate::ui::skin::button(ui, "Save to scenes/user").clicked() {
                        match crate::scenes::save_user(&self.settings, &state.save_title, &state.save_description) {
                            Ok(path) => {
                                state.scenes = crate::scenes::list();
                                state.scene_dates.clear();
                                state.thumbnails_requested = false;
                                self.settings.scene_name = state.save_title.trim().to_string();
                                state.toast(format!("Saved {}", path.display()));
                            }
                            Err(e) => state.error(format!("{e:#}")),
                        }
                    }
                });
                } else {
                use crate::prefs::SceneSort;
                // Sort and card size, then the thumbnail renders' progress: within 340 pt.
                ui.horizontal(|ui| {
                    let mut changed = false;
                    crate::ui::skin::ComboBox::from_id_salt("scene sort")
                        .selected_text(format!("Sort: {}", state.prefs.scene_sort.name()))
                        .width(150.)
                        .show_ui(ui, |ui| {
                            for (sort, name) in SceneSort::ALL {
                                let about = match sort {
                                    SceneSort::Pack => "As listed, under a heading per pack",
                                    SceneSort::Name => "By title",
                                    SceneSort::Recent => "The scenes opened last come first (the Recent list), then the rest",
                                    SceneSort::Favorites => "Favorites, then the rest",
                                    SceneSort::Modified => "By the scene file's date, newest first",
                                };
                                changed |= crate::ui::skin::selectable_value(ui, &mut state.prefs.scene_sort, sort, name).on_hover_text(about).changed();
                            }
                        });
                    let mut cards = state.prefs.scene_cards;
                    if crate::ui::skin::segment(ui, &mut cards, &[(0, "S"), (1, "M"), (2, "L")]) {
                        state.prefs.scene_cards = cards;
                        changed = true;
                    }
                    if changed {
                        state.save_prefs();
                    }
                    if let Some((_, total, done)) = &state.thumbnail_jobs {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            note(ui, &format!("{done} / {total}"));
                            ui.add(egui::Spinner::new()).on_hover_text("Rendering thumbnails");
                        });
                    }
                });
                ui.add_space(4.);
                let sort = state.prefs.scene_sort;
                let card_width = CARD_WIDTHS[(state.prefs.scene_cards as usize).min(2)];
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
                                3 => own_scene(e, user_root.as_deref()),
                                _ => true,
                            }
                    })
                    .collect();
                // Stable sorts: ties keep the pack order. The Recent list is most recent first
                // until another order is chosen.
                match sort {
                    SceneSort::Pack if state.scene_list != 2 => {}
                    SceneSort::Pack | SceneSort::Recent => {
                        visible.sort_by_key(|i| state.prefs.recency(&state.scenes[*i].path).unwrap_or(usize::MAX));
                    }
                    SceneSort::Name => visible.sort_by_cached_key(|i| state.scenes[*i].title.to_lowercase()),
                    SceneSort::Favorites => visible.sort_by_key(|i| !state.prefs.is_favorite(&state.scenes[*i].path)),
                    SceneSort::Modified => {
                        for e in &state.scenes {
                            if !state.scene_dates.contains_key(&e.path) {
                                let date = std::fs::metadata(&e.path).and_then(|m| m.modified()).ok();
                                state.scene_dates.insert(e.path.clone(), date);
                            }
                        }
                        // Files without a date last.
                        visible.sort_by_key(|i| std::cmp::Reverse(state.scene_dates.get(&state.scenes[*i].path).copied().flatten()));
                    }
                }
                // Pack headings only over the pack order: any other makes a run per card.
                let grouped = sort == SceneSort::Pack && state.scene_list != 2;
                let move_card = step != 0;
                if move_card {
                    state.scene_selected = step_card(&visible, state.scene_selected, step).or(state.scene_selected);
                }
                if enter {
                    load = enter_card(&visible, state.scene_selected);
                } else if keys_free && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    load = state.scene_selected.filter(|i| visible.contains(i));
                }
                if visible.is_empty() {
                    // The list itself is empty, or the search and tag leave nothing of it.
                    let listed = match state.scene_list {
                        0 => state.scenes.len(),
                        1 => state.scenes.iter().filter(|e| state.prefs.is_favorite(&e.path)).count(),
                        2 => state.scenes.iter().filter(|e| state.prefs.recency(&e.path).is_some()).count(),
                        _ => state.scenes.iter().filter(|e| own_scene(e, user_root.as_deref())).count(),
                    };
                    note(ui, match state.scene_list {
                        _ if listed > 0 => "No scene here matches the search and the tag.",
                        0 => "No scenes found: the scenes folder is missing or empty.",
                        1 => "No favourites yet: tick Favorite on a scene.",
                        2 => "No scenes opened yet.",
                        _ => "No scenes of your own yet: save the current scene or a generator seed under Create.",
                    });
                }
                egui::ScrollArea::vertical().id_salt("scene grid").auto_shrink([false; 2]).show(ui, |ui| {
                    let mut start = 0;
                    while start < visible.len() {
                        let pack = state.scenes[visible[start]].pack.clone();
                        let end = if grouped {
                            visible[start..].iter().position(|i| state.scenes[*i].pack != pack).map_or(visible.len(), |n| start + n)
                        } else {
                            visible.len()
                        };
                        if grouped {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&pack).font(crate::ui::skin::semibold(12.5)).color(crate::ui::skin::TX));
                                license_badge(ui, &state.scenes[visible[start]].license);
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            for &i in &visible[start..end] {
                                let e = &state.scenes[i];
                                let texture = e.thumbnail().and_then(|p| state.thumbnails.get(&p));
                                let r = scene_card(ui, e, texture, state.scene_selected == Some(i), card_width);
                                if r.clicked() {
                                    state.scene_selected = Some(i);
                                }
                                if move_card && state.scene_selected == Some(i) {
                                    r.scroll_to_me(None);
                                }
                                if r.double_clicked() {
                                    load = Some(i);
                                }
                            }
                        });
                        start = end;
                    }
                });
                }
                });
            });
        self.ui.show_scenes = open && !close;
        if rerender {
            self.request_thumbnails(true);
        }
        if let Some(i) = load {
            self.load_scene_file(i);
        }
    }
    /// T: the flashlight on or off, also while walking.
    pub fn toggle_flashlight(&mut self) {
        self.settings.flashlight = !self.settings.flashlight;
        self.ui.toast(if self.settings.flashlight {
            "Flashlight on (T)"
        } else {
            "Flashlight off (T)"
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
                "Walk mode: WASD, Space jump, C crouch, Shift run, F fly, T flashlight, wheel body size, drag to look, Esc to stop",
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
                egui::ColorImage::new([cw, ch], pixels)
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
        crate::ui::skin::window("Scopes")
            .open(&mut open)
            .default_pos([140., 420.])
            .resizable(false)
            .show_skin(ctx, |ui| {
                let g = self.gpu.as_ref().unwrap();
                if g.scopes.len() != w * h {
                    note(ui, "Waiting for the next frame…");
                    return;
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        crate::ui::skin::heading(ui, "Histogram");
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(256., 128.), egui::Sense::hover());
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 2., egui::Color32::from_rgb(0x12, 0x12, 0x14));
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
                        crate::ui::skin::heading(ui, "Luma waveform");
                        if let Some((wave, _)) = &self.ui.scope_images {
                            ui.add(egui::Image::new((wave.id(), egui::vec2(256., 128.))));
                        }
                    });
                    ui.vertical(|ui| {
                        crate::ui::skin::heading(ui, "Vectorscope");
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
        let mut use_cpu = false;
        let mut load = false;
        // The CPU's finished render: the software view's own image, or Look → Compare's reference.
        let cpu = self
            .cpu_reference
            .as_ref()
            .and_then(|l| l.latest().map(|i| (l.view, i.1, i.2)));
        crate::ui::skin::window("Convergence").open(&mut open).default_pos([140., 120.]).default_width(420.).show_skin(ctx, |ui| {
            let g = self.gpu.as_ref().unwrap();
            match &self.ui.reference {
                Some((w, h, _)) => note(ui, &format!("Reference: {w} × {h}. RMSE ×1000 of l/(1+l), measured as samples grow (log–log).")),
                None => note(ui, "No reference yet: load a linear EXR of the same size, or use the current image once it has converged."),
            }
            ui.horizontal(|ui| {
                crate::ui::skin::text_edit(ui, 220., egui::TextEdit::singleline(&mut self.ui.reference_path));
                if crate::ui::skin::button(ui, "Browse…").clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("Linear EXR", &["exr"]).set_directory(std::path::absolute(crate::dirs::captures()).unwrap_or_default()).pick_file()
                {
                    self.ui.reference_path = p.to_string_lossy().into();
                    load = true;
                }
                load |= crate::ui::skin::button(ui, "Load EXR").clicked();
                match &cpu {
                    Some((true, _, n)) => use_cpu = crate::ui::skin::button(ui, "Use current image").on_hover_text(format!("The CPU's render, {n} spp now")).clicked(),
                    None if self.cpu_view => {
                        ui.add_enabled(false, crate::ui::skin::Button::new("Use current image")).on_disabled_hover_text("Waiting for the CPU's first pass");
                    }
                    _ => use_current = crate::ui::skin::button(ui, "Use current image").on_hover_text(format!("{} spp now", g.samples)).clicked(),
                }
                if let Some((false, _, n)) = &cpu {
                    use_cpu = crate::ui::skin::button(ui, "Use the CPU render").on_hover_text(format!("Look → Compare's CPU reference, {n} spp now: measure the GPU against it")).clicked();
                }
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(300.), 200.), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2., egui::Color32::from_rgb(0x12, 0x12, 0x14));
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
                    painter.text(rect.left_bottom() + egui::vec2(6., -6.), egui::Align2::LEFT_BOTTOM, "x: GPU time per quadrant", egui::FontId::proportional(11.), crate::ui::skin::FAINT);
                } else {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Points appear as the quadrants converge", egui::FontId::proportional(13.), crate::ui::skin::FAINT);
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
                painter.text(rect.left_bottom() + egui::vec2(6., -6.), egui::Align2::LEFT_BOTTOM, "grey: 1/√n", egui::FontId::proportional(11.), crate::ui::skin::FAINT);
            } else {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Points appear as samples accumulate", egui::FontId::proportional(13.), crate::ui::skin::FAINT);
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
        if use_cpu
            && let Some((view, image, _)) = cpu
            && image.len() == g.width as usize * g.height as usize * 4
        {
            self.ui.reference = Some((g.width, g.height, (*image).clone()));
            self.ui.graph.clear();
            self.actions.reset = true;
            // Measuring the GPU against the CPU: the CPU's render must go on, not restart.
            self.actions.keep_cpu = !view;
            self.ui.toast(if view {
                "Reference set; the CPU render restarts to measure against it"
            } else {
                "The CPU render is the reference; the GPU's restarts to measure against it"
            });
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
        crate::ui::skin::window("Phone remote").open(&mut open).default_pos([240., 120.]).resizable(false).show_skin(ctx, |ui| {
            let mut on = self.remote.is_some();
            if crate::ui::skin::checkbox_label(ui, &mut on, "Serve the remote on the local network").changed() {
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
                    let preset = self.preset;
                    self.switch_scene("Scene", |s| s.preset(preset));
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
        let mut switch = false;
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
                        switch = true;
                    }
                    Button::DPadUp if self.settings.auto_exposure => {
                        self.settings.ev_bias = (self.settings.ev_bias + 0.25).min(EV_BIAS_MAX)
                    }
                    Button::DPadDown if self.settings.auto_exposure => {
                        self.settings.ev_bias = (self.settings.ev_bias - 0.25).max(-EV_BIAS_MAX)
                    }
                    Button::DPadUp => {
                        self.settings.exposure =
                            (self.settings.exposure * 1.25).clamp(0., EXPOSURE_MAX)
                    }
                    Button::DPadDown => self.settings.exposure /= 1.25,
                    Button::Start => self.ui.panel_hidden = !self.ui.panel_hidden,
                    _ => {}
                }
            }
        }
        if switch {
            let preset = self.preset;
            self.switch_scene("Scene", |s| s.preset(preset));
        }
        let Some(pad) = &mut self.gamepad else { return };
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
    /// The update offer, then its download (update.rs). `--update-yes` clicks through.
    pub(super) fn update_window(&mut self, ctx: &egui::Context) {
        use crate::update::{Phase, Target};
        if self.updater.poll() {
            println!("UPDATE_CANCELLED");
            self.ui.toast("Update cancelled");
        }
        if let Phase::Available(r) = &self.updater.phase
            && r.version == self.ui.prefs.update_skip
        {
            self.updater.phase = Phase::Idle;
        }
        enum Do {
            Nothing,
            Close,
            Skip(String),
            Download(crate::update::Release),
            Page(String),
        }
        let mut action = Do::Nothing;
        let yes = self.updater.yes;
        let target = self.updater.target.clone();
        let mut install = false;
        let window = crate::ui::skin::window("Update")
            .default_pos([300., 140.])
            .default_width(420.)
            .collapsible(false);
        match &self.updater.phase {
            Phase::Idle => return,
            Phase::Available(r) => {
                let can_update = r.asset.is_some() && !matches!(target, Target::Page(_));
                if yes {
                    if can_update {
                        println!("UPDATE_START {}", r.version);
                        action = Do::Download(r.clone());
                    } else {
                        println!("UPDATE_PAGE {target:?}");
                        std::process::exit(4);
                    }
                }
                window.show_skin(ctx, |ui| {
                    ui.heading(format!(
                        "Caustica {} is available",
                        r.version.trim_start_matches('v')
                    ));
                    ui.label(format!("You have {}.", crate::VERSION));
                    if let Some(a) = &r.asset
                        && a.size > 0
                    {
                        ui.label(format!("Download: {} MB", a.size >> 20));
                    }
                    egui::ScrollArea::vertical()
                        .max_height(200.)
                        .show(ui, |ui| ui.label(&r.notes));
                    if let Target::Page(why) = &target {
                        ui.colored_label(ui.visuals().warn_fg_color, *why);
                    }
                    ui.horizontal(|ui| {
                        if can_update {
                            if crate::ui::skin::button(ui, "Update").clicked() {
                                action = Do::Download(r.clone());
                            }
                        } else if crate::ui::skin::button(ui, "Open release page").clicked() {
                            action = Do::Page(r.page.clone());
                        }
                        if crate::ui::skin::button(ui, "Skip this version").clicked() {
                            action = Do::Skip(r.version.clone());
                        }
                        if crate::ui::skin::button(ui, "Later").clicked() {
                            action = Do::Close;
                        }
                    });
                });
            }
            Phase::Downloading {
                release, progress, ..
            } => {
                use std::sync::atomic::Ordering;
                let done = progress.done.load(Ordering::Relaxed);
                let total = progress.total.load(Ordering::Relaxed);
                if self.updater.cancel_at.is_some_and(|at| done >= at) {
                    progress.cancel.store(true, Ordering::Relaxed);
                }
                window.show_skin(ctx, |ui| {
                    ui.label(format!(
                        "Downloading Caustica {}",
                        release.version.trim_start_matches('v')
                    ));
                    let share = if total > 0 {
                        done as f32 / total as f32
                    } else {
                        0.
                    };
                    crate::ui::skin::bar(
                        ui,
                        share,
                        ui.available_width(),
                        16.,
                        &format!("{} / {} MB", done >> 20, total >> 20),
                    );
                    if crate::ui::skin::button(ui, "Cancel").clicked() {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                });
            }
            Phase::Ready { release, .. } => {
                if yes {
                    println!("UPDATE_READY {}", release.version);
                    install = true;
                }
                window.show_skin(ctx, |ui| {
                    ui.label(format!(
                        "Caustica {} is downloaded and verified.",
                        release.version.trim_start_matches('v')
                    ));
                    ui.horizontal(|ui| {
                        if crate::ui::skin::button(ui, "Restart and install").clicked() {
                            install = true;
                        }
                        if crate::ui::skin::button(ui, "Later").clicked() {
                            action = Do::Close;
                        }
                    });
                });
            }
            Phase::Failed { release, error } => {
                if yes {
                    println!("UPDATE_FAILED {error}");
                    std::process::exit(3);
                }
                window.show_skin(ctx, |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, "The update didn't finish.");
                    ui.label(error);
                    ui.horizontal(|ui| {
                        if !release.page.is_empty()
                            && crate::ui::skin::button(ui, "Open release page").clicked()
                        {
                            action = Do::Page(release.page.clone());
                        }
                        if crate::ui::skin::button(ui, "Close").clicked() {
                            action = Do::Close;
                        }
                    });
                });
            }
        }
        self.ui.install_update |= install;
        match action {
            Do::Nothing => {}
            Do::Close => self.updater.phase = Phase::Idle,
            Do::Skip(version) => {
                self.ui.prefs.update_skip = version;
                self.ui.save_prefs();
                self.updater.phase = Phase::Idle;
            }
            Do::Download(release) => self.updater.download(release),
            Do::Page(url) => {
                crate::platform::open_url(&url);
                self.updater.phase = Phase::Idle;
            }
        }
    }
    pub(super) fn stats_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.show_stats;
        crate::ui::skin::window("Statistics")
            .open(&mut open)
            .default_pos([200., 120.])
            .default_width(380.)
            .show_skin(ctx, |ui| {
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
                        ("Videos exported", s.videos_exported.to_string()),
                        ("Laser levels solved", s.puzzles_solved.len().to_string()),
                        ("Distance walked", format!("{:.0} units", s.walked)),
                    ] {
                        ui.label(label);
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
                let done = crate::stats::ACHIEVEMENTS
                    .iter()
                    .filter(|a| s.achievements.contains_key(a.id))
                    .count();
                section(
                    ui,
                    &format!(
                        "Achievements ({done} / {})",
                        crate::stats::ACHIEVEMENTS.len()
                    ),
                );
                egui::ScrollArea::vertical()
                    .max_height(420.)
                    .show(ui, |ui| {
                        for group in crate::stats::GROUPS {
                            crate::ui::skin::heading(ui, group);
                            for a in crate::stats::ACHIEVEMENTS
                                .iter()
                                .filter(|a| a.group == group)
                            {
                                match s.achievements.get(a.id) {
                                    Some(date) => {
                                        ui.label(
                                            egui::RichText::new(a.title)
                                                .font(crate::ui::skin::semibold(12.5))
                                                .color(ACCENT),
                                        );
                                        note(ui, &format!("{} · {date}", a.how));
                                    }
                                    None => {
                                        ui.label(a.title);
                                        note(ui, a.how);
                                        let (now, goal) = s.progress(a.goal);
                                        if goal > 1. {
                                            crate::ui::skin::bar(
                                                ui,
                                                (now / goal).min(1.) as f32,
                                                ui.available_width(),
                                                6.,
                                                "",
                                            );
                                        }
                                    }
                                }
                            }
                            ui.add_space(6.);
                        }
                    });
            });
        self.ui.show_stats = open;
    }
}

#[cfg(test)]
mod scene_browser_tests {
    use super::{enter_card, step_card};
    #[test]
    fn arrows_walk_the_visible_cards_and_stop_at_the_ends() {
        let visible = [3, 7, 9];
        assert_eq!(step_card(&visible, Some(3), 1), Some(7));
        assert_eq!(step_card(&visible, Some(7), -1), Some(3));
        assert_eq!(step_card(&visible, Some(3), -1), Some(3));
        assert_eq!(step_card(&visible, Some(9), 1), Some(9));
        // Filtered away or nothing yet: the first card.
        assert_eq!(step_card(&visible, Some(5), 1), Some(3));
        assert_eq!(step_card(&visible, None, 1), Some(3));
        assert_eq!(step_card(&[], Some(5), 1), None);
    }
    #[test]
    fn enter_never_loads_a_scene_the_search_hides() {
        assert_eq!(enter_card(&[3, 7], Some(7)), Some(7));
        assert_eq!(enter_card(&[3, 7], Some(5)), Some(3));
        assert_eq!(enter_card(&[3, 7], None), Some(3));
        assert_eq!(enter_card(&[], Some(5)), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tags_split_into_chips_and_a_counted_rest() {
        let all = [
            "glass",
            "glass",
            "bokeh",
            "night",
            "bokeh",
            "water",
            "still life",
        ];
        let (top, more) = split_tags(all.into_iter());
        // Chips keep TOP_TAGS order and only the ones that occur.
        assert_eq!(top, ["glass", "night", "still life"]);
        assert_eq!(more, [("bokeh".to_string(), 2), ("water".to_string(), 1)]);
        assert!(split_tags(std::iter::empty()).0.is_empty());
    }
    /// The shipped scenes: every chip is used, and the hyphenated generator tag is gone.
    #[test]
    fn shipped_tags_have_no_duplicate_spellings() {
        let entries = crate::scenes::list();
        let tags: Vec<&str> = entries
            .iter()
            .flat_map(|e| e.tags.iter().map(String::as_str))
            .collect();
        if tags.is_empty() {
            return; // scenes folder not found from this working directory
        }
        assert!(!tags.contains(&"still-life"));
        let (top, _) = split_tags(tags.into_iter());
        assert_eq!(
            top.len(),
            TOP_TAGS.len(),
            "a TOP_TAGS entry no scene uses: {top:?}"
        );
    }
}
