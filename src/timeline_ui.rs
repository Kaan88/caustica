//! The timeline strip (track 8): transport, playhead, key markers, keying and the animation's
//! length, frame rate and shutter. Drawn above the status bar while it is shown; a scene gaining
//! keys opens it, and it can then be closed.
use crate::App;
use crate::anim::{self, Interp, Timeline};
use crate::ui::{ACCENT, Selection};

/// True on the frame the scene goes from no keys to some.
fn gained_keys(had: &mut bool, has: bool) -> bool {
    !std::mem::replace(had, has) && has
}
impl App {
    /// I: key the selection (the camera when nothing is selected); Shift+I: every edited value.
    pub fn key_selection(&mut self, changed: bool) {
        if !changed && self.ui.selection == Selection::Flashlight {
            self.ui.toast("The flashlight can't be keyed");
            return;
        }
        self.ui.checkpoint(&self.settings);
        let n = if changed {
            anim::key_changed(&mut self.settings, &self.timeline)
        } else {
            anim::key_selection(&mut self.settings, &self.timeline, self.ui.selection)
        };
        // The new keys hold the current values, so applying them changes nothing.
        self.timeline.settle(&self.settings);
        self.ui.show_timeline = true;
        let what = if changed {
            "edited values".to_string()
        } else {
            selection_name(self.ui.selection)
        };
        self.ui.toast(match n {
            0 => format!("Nothing to key on {what}"),
            n => format!(
                "Keyed {what} at {:.2} s ({n} value{})",
                self.timeline.time,
                if n == 1 { "" } else { "s" }
            ),
        });
    }
    /// Alt+I: removes the selection's keys at the playhead.
    pub fn delete_keys(&mut self) {
        self.ui.checkpoint(&self.settings);
        let prefixes = anim::prefixes(self.ui.selection);
        let n = self
            .settings
            .animation
            .remove_keys(&prefixes, self.timeline.time);
        self.ui.toast(format!(
            "Removed {n} key{} from {}",
            if n == 1 { "" } else { "s" },
            selection_name(self.ui.selection)
        ));
    }
    /// K: play or stop (from the start when stopped at the end).
    pub fn toggle_playback(&mut self) {
        let t = &mut self.timeline;
        t.playing = !t.playing;
        if t.playing && t.time >= self.settings.animation.length {
            t.time = 0.;
        }
    }
    /// Shift+← / →: one frame back or on.
    pub fn step_frame(&mut self, frames: i32) {
        let a = &self.settings.animation;
        self.timeline.playing = false;
        self.timeline.time =
            Timeline::snap(a, self.timeline.time + frames as f32 / a.fps.max(1) as f32);
    }
    pub(crate) fn timeline_panel(&mut self, root: &mut egui::Ui) {
        // In one place, not in each of the paths that replace the settings (file, generators, AI,
        // undo, startup). From one keyed scene to the next it stays as left.
        let keys = !self.settings.animation.is_empty();
        self.ui.show_timeline |= gained_keys(&mut self.ui.timeline_had_keys, keys);
        if !self.ui.show_timeline {
            return;
        }
        let prefixes = anim::prefixes(self.ui.selection);
        let frame = egui::Frame::new()
            .fill(crate::ui::skin::PANEL)
            .inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::bottom("timeline")
            .show_separator_line(false)
            .frame(frame)
            .show(root, |ui| {
            let edge = ui.max_rect().expand2(egui::vec2(8., 2.));
            ui.painter().hline(
                edge.x_range(),
                edge.top() + 0.5,
                egui::Stroke::new(1_f32, crate::ui::skin::EDGE_LINE),
            );
            ui.add_space(2.);
            ui.horizontal(|ui| {
                let room = ui.available_width();
                let a = &self.settings.animation;
                let keys = a.times(&prefixes);
                let t = self.timeline.time;
                if crate::ui::skin::button(ui, "|◀").on_hover_text("To the start").clicked() {
                    self.timeline.time = 0.;
                }
                let previous = keys.iter().rev().find(|k| **k < t - 1e-4).copied();
                if ui
                    .add_enabled(previous.is_some(), crate::ui::skin::Button::new("◀ key"))
                    .on_hover_text("The selection's previous key")
                    .clicked()
                {
                    self.timeline.time = previous.unwrap_or(t);
                    self.timeline.playing = false;
                }
                let play = if self.timeline.playing {
                    "⏸ Stop"
                } else {
                    "▶ Play"
                };
                if crate::ui::skin::button(ui, play).on_hover_text("K").clicked() {
                    self.toggle_playback();
                }
                let next = keys.iter().find(|k| **k > t + 1e-4).copied();
                if ui
                    .add_enabled(next.is_some(), crate::ui::skin::Button::new("key ▶"))
                    .on_hover_text("The selection's next key")
                    .clicked()
                {
                    self.timeline.time = next.unwrap_or(t);
                    self.timeline.playing = false;
                }
                if crate::ui::skin::button(ui, "▶|").on_hover_text("To the end").clicked() {
                    self.timeline.time = self.settings.animation.length;
                }
                let a = &self.settings.animation;
                let mut time = self.timeline.time;
                let r = crate::ui::skin::drag_value(ui, egui::DragValue::new(&mut time)
                            .range(0. ..=a.length)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .suffix(" s"),)
                    .on_hover_text("Playhead. Shift+Left / Right step a frame.");
                if r.changed() {
                    self.timeline.time = time;
                    self.timeline.playing = false;
                }
                if room >= 900. {
                ui.label(
                    egui::RichText::new(format!(
                        "frame {}",
                        (self.timeline.time * a.fps as f32).round() as u32
                    ))
                    .weak(),
                );
                    self.timeline_toggles(ui);
                }
                crate::ui::skin::rule_v(ui, 18.);
                crate::ui::skin::checkbox_label(ui, &mut self.timeline.auto_key, "Auto-key").on_hover_text(
                    "Every edit you finish at a still playhead is keyed there (as Shift+I)",
                );
                if crate::ui::skin::button(ui, "Key")
                    .on_hover_text(
                        "I: key the selection here (the camera when nothing is selected): what it already animates, what you changed, and on its first key its transform. Shift+I keys every value you changed anywhere.",
                    )
                    .clicked()
                {
                    self.key_selection(false);
                }
                let frame = 1. / self.settings.animation.fps.max(1) as f32;
                let mut retime = None;
                let mut copy = false;
                ui.add_enabled_ui(!keys.is_empty(), |ui| {
                    crate::ui::skin::menu_button_raised(ui, "Keys", |ui| {
                        for (label, r) in [
                            ("Later by a frame", (1., frame)),
                            ("Earlier by a frame", (1., -frame)),
                            ("Later by a second", (1., 1.)),
                            ("Earlier by a second", (1., -1.)),
                            ("Twice as slow (from the playhead)", (2., 0.)),
                            ("Twice as fast (from the playhead)", (0.5, 0.)),
                        ] {
                            if crate::ui::skin::button(ui, label).clicked() {
                                retime = Some(r);
                                ui.close();
                            }
                        }
                        let others = self.ui.also.len();
                        if ui
                            .add_enabled(
                                others > 0,
                                crate::ui::skin::Button::new(format!("Copy to the {others} other selected")),
                            )
                            .on_hover_text("Replace the other selected objects' or lights' keys with the active one's")
                            .clicked()
                        {
                            copy = true;
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Move, stretch or copy the selection's keys")
                    .on_disabled_hover_text("Select something with keyframes");
                });
                // A narrow strip keeps the transport and the keying; the rest is under More.
                if room < 1240. {
                crate::ui::skin::menu_button_raised(ui, "More", |ui| {
                    if room < 900. {
                        self.timeline_toggles(ui);
                        crate::ui::skin::separator(ui);
                    }
                    for (label, which) in [("Length", 0), ("Frame rate", 1), ("Shutter", 2)] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            self.timeline_setting(ui, which);
                        });
                    }
                    self.timeline_setting(ui, 3);
                })
                .response
                .on_hover_text("Length, frame rate, shutter and Clear");
                }
                if let Some((scale, offset)) = retime {
                    self.ui.checkpoint(&self.settings);
                    self.settings.animation.retime(&prefixes, t, scale, offset);
                }
                if copy {
                    self.ui.checkpoint(&self.settings);
                    let (from, _) = anim::entity(self.ui.selection);
                    let mut n = 0;
                    for other in self.ui.also.clone() {
                        let (to, _) = anim::entity(other);
                        n += self.settings.animation.copy_keys(&from, &to);
                    }
                    self.ui.toast(format!("Copied {n} track(s)"));
                }
                let here = keys.iter().any(|k| (k - t).abs() < 1e-4);
                if ui
                    .add_enabled(here, crate::ui::skin::Button::new("Delete key"))
                    .on_hover_text("Alt+I: the selection's keys at the playhead")
                    .clicked()
                {
                    self.delete_keys();
                }
                if here {
                    let current = self.settings.animation.interp_at(&prefixes, t);
                    let mut chosen = None;
                    crate::ui::skin::ComboBox::from_id_salt("interp")
                        .selected_text(current.map_or("Mixed", Interp::label))
                        .show_ui(ui, |ui| {
                            for i in Interp::ALL {
                                if crate::ui::skin::selectable(ui, current == Some(i), i.label()).clicked() {
                                    chosen = Some(i);
                                }
                            }
                        })
                        .response
                        .on_hover_text("How the selection's keys here run on to the next ones");
                    if let Some(i) = chosen {
                        self.ui.checkpoint(&self.settings);
                        self.settings.animation.set_interp(&prefixes, t, i);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if room >= 1240. {
                        self.timeline_setting(ui, 3);
                        self.timeline_setting(ui, 2);
                        ui.label("Shutter");
                        self.timeline_setting(ui, 1);
                        self.timeline_setting(ui, 0);
                        ui.label("Length");
                    }
                });
            });
            self.track_strip(ui, &prefixes);
            if self.timeline.curves {
                ui.add_space(2.);
                self.curve_editor(ui, &prefixes);
            }
            ui.add_space(2.);
        });
    }
    /// Loop, Camera path and Curves: in the toolbar, or under More in a narrow strip.
    fn timeline_toggles(&mut self, ui: &mut egui::Ui) {
        crate::ui::skin::checkbox_label(ui, &mut self.timeline.looping, "Loop");
        let mut path = self.settings.animation.camera_path;
        if crate::ui::skin::checkbox_label(ui, &mut path, "Camera path")
            .on_hover_text("Between the camera's keys, the eye and its target move along smooth curves through their keyed positions (drawn in the viewport), instead of yaw, pitch, distance and target each moving alone, which swings the eye around the target")
            .changed()
        {
            self.ui.checkpoint(&self.settings);
            self.settings.animation.camera_path = path;
            self.timeline.applied = None;
        }
        crate::ui::skin::checkbox_label(ui, &mut self.timeline.curves, "Curves").on_hover_text(
            "The curve editor: the selection's animated values over time, each scaled to its own range. Drag a key to move it in time (frames, Shift for free) and value.",
        );
    }
    /// One of the animation's settings: 0 length, 1 frame rate, 2 shutter, 3 Clear.
    fn timeline_setting(&mut self, ui: &mut egui::Ui, which: u8) {
        let a = &mut self.settings.animation;
        match which {
            0 => {
                crate::ui::skin::drag_value(
                    ui,
                    egui::DragValue::new(&mut a.length)
                        .range(0.1..=3600.)
                        .speed(0.05)
                        .fixed_decimals(2)
                        .suffix(" s"),
                )
                .on_hover_text("Length of the animation");
            }
            1 => {
                crate::ui::skin::drag_value(
                    ui,
                    egui::DragValue::new(&mut a.fps)
                        .range(1..=240)
                        .suffix(" fps"),
                );
            }
            2 => {
                crate::ui::skin::drag_value(ui, egui::DragValue::new(&mut a.shutter)
                        .range(0. ..=1.)
                        .speed(0.01)
                        .custom_formatter(|v, _| format!("{:.0}°", v * 360.))
                        .custom_parser(|t| {
                            t.trim_end_matches('°').trim().parse::<f64>().ok().map(|v| v / 360.)
                        }),)
                .on_hover_text(
                    "Motion blur in timeline video exports: how long the shutter stays open, as a shutter angle (180° = half a frame; 0 = off)",
                );
            }
            _ => {
                if ui
                    .add_enabled(!a.is_empty(), crate::ui::skin::Button::new("Clear"))
                    .on_hover_text("Remove every key in the scene")
                    .clicked()
                {
                    self.ui.checkpoint(&self.settings);
                    self.settings.animation.tracks.clear();
                    self.timeline.playing = false;
                }
            }
        }
    }
    /// The scrubbing strip: seconds, every key (faint), the selection's keys and the playhead.
    fn track_strip(&mut self, ui: &mut egui::Ui, prefixes: &[String]) {
        // One row under the strip for everything that holds keys; their names take a column
        // at the left, and the strip starts where the rows' lanes do.
        let owners = track_owners(&self.settings, self.ui.selection);
        let names = if owners.is_empty() { 0. } else { TRACK_NAMES };
        let a = &self.settings.animation;
        let (full, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 30.),
            egui::Sense::click_and_drag(),
        );
        let rect = egui::Rect::from_min_max(egui::pos2(full.left() + names, full.top()), full.max);
        let length = a.length.max(1e-3);
        let x_of = |t: f32| rect.left() + 8. + (rect.width() - 16.) * (t / length);
        let painter = ui.painter_at(rect);
        let v = ui.visuals();
        painter.rect_filled(rect, 3., v.extreme_bg_color);
        // A tick each second (each tenth when short), labelled every few.
        let step = if length <= 3. {
            0.1
        } else if length <= 60. {
            1.
        } else {
            10.
        };
        let every = 5;
        let mut i = 0;
        while i as f32 * step <= length + 1e-4 {
            let x = x_of(i as f32 * step);
            let major = i % every == 0;
            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom() - if major { 9. } else { 5. }),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(1_f32, v.weak_text_color()),
            );
            if major {
                let label = painter.layout_no_wrap(
                    format!("{:.*}", if step < 1. { 1 } else { 0 }, i as f32 * step),
                    crate::ui::skin::mono(10.),
                    crate::ui::skin::DIM,
                );
                // The last label would run out of the strip: it ends at its tick instead.
                let left = if x + 2. + label.size().x > rect.right() - 1. {
                    x - 2. - label.size().x
                } else {
                    x + 2.
                };
                painter.galley(
                    egui::pos2(left, rect.top() + 1.),
                    label,
                    crate::ui::skin::DIM,
                );
            }
            i += 1;
        }
        let diamond = |c: egui::Pos2, r: f32, color: egui::Color32| {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(0., -r),
                    c + egui::vec2(r, 0.),
                    c + egui::vec2(0., r),
                    c + egui::vec2(-r, 0.),
                ],
                color,
                egui::Stroke::NONE,
            ));
        };
        let mid = rect.center().y + 3.;
        for t in a.times(&[]) {
            diamond(egui::pos2(x_of(t), mid), 3.5, v.text_color());
        }
        if !a.is_empty() {
            for t in a.times(prefixes) {
                diamond(egui::pos2(x_of(t), mid), 5.5, ACCENT);
            }
        }
        let x = x_of(self.timeline.time.min(length));
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2_f32, egui::Color32::from_rgb(255, 120, 80)),
        );
        if let Some(p) = response.interact_pointer_pos()
            && (response.dragged() || response.clicked())
        {
            let t = ((p.x - rect.left() - 8.) / (rect.width() - 16.)).clamp(0., 1.) * length;
            // Frames, unless Shift is held.
            let shift = ui.input(|i| i.modifiers.shift);
            self.timeline.time = if shift { t } else { Timeline::snap(a, t) };
            self.timeline.playing = false;
        }
        if owners.is_empty() {
            return;
        }
        let playhead = self.timeline.time.min(length);
        let shift = ui.input(|i| i.modifiers.shift);
        let (mut pick, mut scrub) = (None, None);
        egui::ScrollArea::vertical()
            .id_salt("timeline tracks")
            .max_height(6. * 18.)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.;
                for (i, (sel, name, times)) in owners.iter().enumerate() {
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 17.),
                        egui::Sense::hover(),
                    );
                    let label = egui::Rect::from_min_max(
                        row.min,
                        egui::pos2(row.left() + TRACK_NAMES - 4., row.bottom()),
                    );
                    let lane = egui::Rect::from_min_max(
                        egui::pos2(row.left() + TRACK_NAMES, row.top()),
                        row.max,
                    );
                    let x_of = |t: f32| lane.left() + 8. + (lane.width() - 16.) * (t / length);
                    let name_click = ui.interact(
                        label,
                        ui.scope_id().with(("track", i)),
                        egui::Sense::click(),
                    );
                    let lane_drag = ui.interact(
                        lane,
                        ui.scope_id().with(("lane", i)),
                        egui::Sense::click_and_drag(),
                    );
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    let on = self.ui.is_selected(*sel);
                    let painter = ui.painter_at(row);
                    if on {
                        painter.rect_filled(label, 3., crate::ui::SELECT);
                    } else if name_click.hovered() {
                        painter.rect_filled(label, 3., crate::ui::skin::TAB_HOVER);
                    }
                    painter
                        .with_clip_rect(label.intersect(ui.clip_rect()))
                        .text(
                            egui::pos2(label.left() + 6., label.center().y),
                            egui::Align2::LEFT_CENTER,
                            name,
                            crate::ui::skin::sans(12.),
                            if on {
                                egui::Color32::WHITE
                            } else {
                                crate::ui::skin::TX
                            },
                        );
                    painter.rect_filled(lane, 2., ui.visuals().extreme_bg_color);
                    for t in times {
                        let c = egui::pos2(x_of(*t), lane.center().y);
                        let r = 4.;
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                c + egui::vec2(0., -r),
                                c + egui::vec2(r, 0.),
                                c + egui::vec2(0., r),
                                c + egui::vec2(-r, 0.),
                            ],
                            if on { ACCENT } else { crate::ui::skin::DIM },
                            egui::Stroke::NONE,
                        ));
                    }
                    painter.vline(
                        x_of(playhead),
                        lane.y_range(),
                        egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(255, 120, 80)),
                    );
                    // "Other values" has nothing to select.
                    if *sel != Selection::None
                        && name_click
                            .on_hover_text("Select it: the strip and the curves then show its keys")
                            .clicked()
                    {
                        pick = Some(*sel);
                    }
                    if let Some(p) = lane_drag.interact_pointer_pos()
                        && (lane_drag.dragged() || lane_drag.clicked())
                    {
                        scrub = Some(
                            ((p.x - lane.left() - 8.) / (lane.width() - 16.)).clamp(0., 1.)
                                * length,
                        );
                    }
                }
            });
        if let Some(t) = scrub {
            let a = &self.settings.animation;
            self.timeline.time = if shift { t } else { Timeline::snap(a, t) };
            self.timeline.playing = false;
        }
        if let Some(sel) = pick {
            self.ui.select(sel);
            self.ui.tab = self.ui.tab.after_pick(sel, false);
        }
    }
}
/// The width of the track rows' name column.
const TRACK_NAMES: f32 = 150.;
/// Everything that holds keys, the selection first: what to select, its name and its key times.
fn track_owners(
    s: &crate::scene::Settings,
    selection: Selection,
) -> Vec<(Selection, String, Vec<f32>)> {
    let camera = anim::entity(Selection::Camera).1;
    let main = anim::entity(Selection::MainLight).1;
    let mut out: Vec<(Selection, String, Vec<f32>)> = Vec::new();
    for track in s.animation.tracks.iter().filter(|t| !t.keys.is_empty()) {
        let mut parts = track.path.split('/');
        let list = parts.next();
        let index = parts.next().and_then(|i| i.parse::<usize>().ok());
        let under = |fields: &[&str]| fields.iter().any(|f| anim::under(&track.path, f));
        let (sel, name) = match (list, index) {
            (Some("objects"), Some(i)) => (
                Selection::Object(i),
                s.objects.get(i).map_or("Object".into(), |o| o.name.clone()),
            ),
            (Some("lights"), Some(i)) => (
                Selection::Light(i),
                s.lights.get(i).map_or("Light".into(), |l| l.name.clone()),
            ),
            _ if under(main) => (Selection::MainLight, "Main light".into()),
            _ if under(camera) => (Selection::Camera, "Camera".into()),
            _ => (Selection::None, "Other values".into()),
        };
        let times = track.keys.iter().map(|k| k.time);
        match out.iter_mut().find(|o| o.0 == sel) {
            Some(o) => o.2.extend(times),
            None => out.push((sel, name, times.collect())),
        }
    }
    for o in &mut out {
        o.2.sort_by(f32::total_cmp);
        o.2.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    }
    out.sort_by_key(|o| o.0 != selection);
    out
}
impl App {
    /// The curve editor: the selection's float tracks as curves (each scaled to its own range),
    /// their keys as dots that drag in time (frames unless Shift; never past a neighbour) and
    /// value, and the playhead. A click away from the keys moves the playhead.
    fn curve_editor(&mut self, ui: &mut egui::Ui, prefixes: &[String]) {
        const COLORS: [egui::Color32; 6] = [
            egui::Color32::from_rgb(240, 110, 100),
            egui::Color32::from_rgb(120, 200, 110),
            egui::Color32::from_rgb(100, 160, 250),
            egui::Color32::from_rgb(240, 200, 90),
            egui::Color32::from_rgb(200, 120, 230),
            egui::Color32::from_rgb(90, 210, 210),
        ];
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 150.),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        painter.rect_filled(rect, 3., visuals.extreme_bg_color);
        let length = self.settings.animation.length.max(1e-3);
        let inner = rect.shrink2(egui::vec2(8., 12.));
        let x_of = |t: f32| inner.left() + inner.width() * (t / length);
        let t_of = |x: f32| ((x - inner.left()) / inner.width()).clamp(0., 1.) * length;
        // Float tracks only: held fields (switches, counts, choices) have no curve.
        let json = serde_json::to_value(&self.settings).ok();
        let a = &self.settings.animation;
        let tracks: Vec<usize> = a
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, tr)| {
                anim::under_any(&tr.path, prefixes)
                    && json
                        .as_ref()
                        .and_then(|j| j.pointer(&format!("/{}", tr.path)))
                        .is_some_and(|v| v.is_f64())
            })
            .map(|(i, _)| i)
            .take(24)
            .collect();
        if tracks.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No animated values on the selection",
                egui::FontId::proportional(12.),
                visuals.text_color(),
            );
        }
        let range = |tr: &anim::Track| {
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for k in 0..=64 {
                let v = tr.value(length * k as f32 / 64.);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            for k in &tr.keys {
                lo = lo.min(k.value);
                hi = hi.max(k.value);
            }
            if hi - lo < 1e-9 {
                let pad = (lo.abs() * 0.5).max(0.5);
                (lo - pad, hi + pad)
            } else {
                let pad = (hi - lo) * 0.05;
                (lo - pad, hi + pad)
            }
        };
        let y_of = |v: f64, (lo, hi): (f64, f64)| {
            inner.bottom() - ((v - lo) / (hi - lo)) as f32 * inner.height()
        };
        let drag = self.timeline.curve_drag.clone();
        // A drag picks the key under where the press began (egui starts it a few pixels on).
        let pointer = if response.drag_started() {
            ui.input(|i| i.pointer.press_origin())
        } else {
            response.hover_pos()
        };
        let mut nearest: Option<(f32, usize, usize, (f64, f64))> = None;
        for (n, &i) in tracks.iter().enumerate() {
            let tr = &a.tracks[i];
            let r = match &drag {
                Some((path, _, lo, hi)) if *path == tr.path => (*lo, *hi),
                _ => range(tr),
            };
            let color = COLORS[n % COLORS.len()];
            let points: Vec<egui::Pos2> = (0..=160)
                .map(|k| {
                    let t = length * k as f32 / 160.;
                    egui::pos2(x_of(t), y_of(tr.value(t), r))
                })
                .collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, color)));
            for (k, key) in tr.keys.iter().enumerate() {
                let c = egui::pos2(x_of(key.time), y_of(key.value, r));
                painter.circle_filled(c, 3.5, color);
                if let Some(p) = pointer {
                    let d = p.distance(c);
                    if d < 8. && nearest.is_none_or(|b| d < b.0) {
                        nearest = Some((d, i, k, r));
                    }
                }
            }
        }
        let x = x_of(self.timeline.time.min(length));
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2_f32, egui::Color32::from_rgb(255, 120, 80)),
        );
        if let Some((_, i, k, _)) = nearest {
            let tr = &a.tracks[i];
            let key = &tr.keys[k];
            painter.text(
                rect.left_top() + egui::vec2(6., 2.),
                egui::Align2::LEFT_TOP,
                format!("{}  {:.4} at {:.2} s", tr.path, key.value, key.time),
                egui::FontId::proportional(11.),
                visuals.text_color(),
            );
        }
        if response.drag_started()
            && let Some((_, i, k, (lo, hi))) = nearest
        {
            self.ui.checkpoint(&self.settings);
            let path = self.settings.animation.tracks[i].path.clone();
            self.timeline.curve_drag = Some((path, k, lo, hi));
            self.timeline.playing = false;
        }
        if response.dragged()
            && let Some((path, k, lo, hi)) = self.timeline.curve_drag.clone()
            && let Some(p) = response.interact_pointer_pos()
        {
            let shift = ui.input(|i| i.modifiers.shift);
            let a = &mut self.settings.animation;
            let t = t_of(p.x);
            let t = if shift { t } else { Timeline::snap(a, t) };
            if let Some(tr) = a.tracks.iter_mut().find(|tr| tr.path == path)
                && k < tr.keys.len()
            {
                let before = if k > 0 {
                    tr.keys[k - 1].time + 1e-3
                } else {
                    0.
                };
                let after = tr.keys.get(k + 1).map_or(length, |n| n.time - 1e-3);
                if before <= after {
                    tr.keys[k].time = t.clamp(before, after);
                }
                if tr.keys[k].text.is_none() {
                    // Inside the editor's range (a value past a field's limits would make the
                    // whole animation fail validation and stop applying).
                    let y = p.y.clamp(inner.top(), inner.bottom());
                    let v = lo + (inner.bottom() - y) as f64 / inner.height() as f64 * (hi - lo);
                    tr.keys[k].value = v;
                }
            }
        }
        if response.drag_stopped() {
            self.timeline.curve_drag = None;
        }
        if response.clicked()
            && nearest.is_none()
            && let Some(p) = response.interact_pointer_pos()
        {
            self.timeline.time = Timeline::snap(&self.settings.animation, t_of(p.x));
            self.timeline.playing = false;
        }
    }
}
/// The width of a row's key mark, in points.
const KEY_SLOT: f32 = 10.;
/// Extra points a key-marks tab keeps between the diamond and the controls, so the slider's
/// "modified" dot (5 points wide, 10 left of its rail) does not land on the diamond.
const KEY_GAP: f32 = 6.;
/// How far left of the controls a slider's unit text must end, on top of its own gap, when the
/// row has a key mark; 0 in the tabs without key marks.
pub(crate) fn key_gutter() -> f32 {
    if KEY_MARKS.with_borrow(|m| m.is_some()) {
        KEY_SLOT
    } else {
        0.
    }
}
/// Key marks: the diamonds on the rows of the Lights, Object and Material tabs. While one of
/// those tabs is drawn (`App::key_marks_begin` to `key_marks_end`), every `ui::row` keeps a slot
/// before its controls, so the controls line up whether a row can be keyed or not. A row named
/// by `key_mark` just before it shows a diamond there: an outline under the pointer when the
/// value has no keys, an accent outline when it has keys elsewhere, filled when one is at the
/// playhead. A click sets a key at the playhead, or removes the one there.
struct KeyMarks {
    /// The playhead.
    time: f32,
    /// The JSON path the rows' own paths are under ("objects/3"; empty for the scene's own).
    prefix: String,
    /// The tracks under `prefix`, and whether each has a key at the playhead.
    tracks: Vec<(String, bool)>,
    /// The path of the row about to be drawn.
    next: Option<String>,
    /// The mark clicked this frame: its path, and whether the click removes its key.
    clicked: Option<(String, bool)>,
}
thread_local! {
    static KEY_MARKS: std::cell::RefCell<Option<KeyMarks>> = const { std::cell::RefCell::new(None) };
}
/// The rows from here on belong to `prefix` ("lights/2", "objects/0/material", "walls"; empty:
/// the scene's own fields). Does nothing outside a tab that shows key marks.
pub(crate) fn key_scope(a: &anim::Animation, prefix: &str) {
    KEY_MARKS.with_borrow_mut(|m| {
        let Some(m) = m else { return };
        let time = m.time;
        m.prefix = prefix.into();
        m.next = None;
        // The scene's own rows never show an object's or a light's fields, which is where a
        // recorded simulation's thousands of tracks are.
        m.tracks = a
            .tracks
            .iter()
            .filter(|t| {
                if prefix.is_empty() {
                    !t.path.starts_with("objects/") && !t.path.starts_with("lights/")
                } else {
                    anim::under(&t.path, prefix)
                }
            })
            .map(|t| {
                let here = t.keys.iter().any(|k| (k.time - time).abs() < 1e-4);
                (t.path.clone(), here)
            })
            .collect();
    });
}
/// The next `ui::row` edits the value at `path` (under the scope's prefix; an array's path
/// covers its components), so it gets a key mark. Call it right before the row.
pub(crate) fn key_mark(path: &str) {
    KEY_MARKS.with_borrow_mut(|m| {
        if let Some(m) = m {
            m.next = Some(if m.prefix.is_empty() {
                path.into()
            } else {
                format!("{}/{path}", m.prefix)
            });
        }
    });
}
/// A row's key mark, first in its controls (`ui::row_controls`). Outside the tabs that show
/// key marks it takes no room.
pub(crate) fn key_slot(ui: &mut egui::Ui) {
    let Some((path, time)) =
        KEY_MARKS.with_borrow_mut(|m| m.as_mut().map(|m| (m.next.take(), m.time)))
    else {
        return;
    };
    let size = egui::vec2(KEY_SLOT, ui.spacing().interact_size.y);
    let sense = if path.is_some() {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(size, sense);
    // Half an item gap to the control: the mark belongs to it.
    ui.add_space(-4. + KEY_GAP);
    let Some(path) = path else { return };
    // Whether the value has keys, and whether one is at the playhead.
    let (animated, here) = KEY_MARKS.with_borrow(|m| {
        let mine = m
            .iter()
            .flat_map(|m| &m.tracks)
            .filter(|t| anim::under(&t.0, &path));
        mine.fold((false, false), |state, t| (true, state.1 || t.1))
    });
    if ui.is_rect_visible(rect) {
        // A value without keys shows its mark only with the pointer on its row.
        let row = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), rect.y_range());
        let color = if r.hovered() {
            ui.visuals().strong_text_color()
        } else if animated {
            ACCENT
        } else {
            ui.visuals().weak_text_color()
        };
        let (c, radius) = (rect.center(), 4.);
        let points = vec![
            c + egui::vec2(0., -radius),
            c + egui::vec2(radius, 0.),
            c + egui::vec2(0., radius),
            c + egui::vec2(-radius, 0.),
        ];
        if here {
            ui.painter().add(egui::Shape::convex_polygon(
                points,
                color,
                egui::Stroke::NONE,
            ));
        } else if animated || (ui.is_enabled() && ui.rect_contains_pointer(row)) {
            ui.painter().add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.2_f32, color),
            ));
        }
    }
    crate::ui::row_tip_yields(&r);
    let tip = if here {
        format!("Keyed at the playhead ({time:.2} s). Click to remove this key.")
    } else if animated {
        format!("Animated. Click to key it at the playhead ({time:.2} s).")
    } else {
        format!("Click to key it at the playhead ({time:.2} s).")
    };
    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
    {
        KEY_MARKS.with_borrow_mut(|m| {
            if let Some(m) = m {
                m.clicked = Some((path, here));
            }
        });
    }
}
impl App {
    /// Rows drawn from here to `key_marks_end` show key marks; they start as the scene's own.
    pub(crate) fn key_marks_begin(&self) {
        KEY_MARKS.set(Some(KeyMarks {
            time: self.timeline.time,
            prefix: String::new(),
            tracks: Vec::new(),
            next: None,
            clicked: None,
        }));
        key_scope(&self.settings.animation, "");
    }
    /// Ends the key marks and acts on a clicked one, as the timeline's Key and Delete key do
    /// for the selection: an undo step, then the key set from the value as it stands (and the
    /// timeline settled on it) or the key at the playhead removed. The timeline may be closed.
    pub(crate) fn key_marks_end(&mut self) {
        let Some(KeyMarks {
            time,
            clicked: Some((path, remove)),
            ..
        }) = KEY_MARKS.take()
        else {
            return;
        };
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        if remove {
            self.ui.checkpoint(&self.settings);
            let n = self.settings.animation.remove_keys(&[path], time);
            // Without this the changed tracks are applied next frame, which would throw away
            // edits not keyed yet on other animated values.
            self.timeline.settle(&self.settings);
            self.ui
                .toast(format!("Removed {n} key{} at {time:.2} s", plural(n)));
            return;
        }
        // The undo step is pushed only when a key was set: a click that keys nothing must
        // not cost an undo press or the redo list.
        let before = self.settings.clone();
        match anim::key_path(&mut self.settings, time, &path) {
            // Scene files leave out some values at their defaults, and keys find values there.
            0 => self
                .ui
                .toast("Not keyed: the value is at its default. Change it first, then key it."),
            n => {
                self.ui.checkpoint(&before);
                // The new keys hold the current values, so applying them changes nothing.
                self.timeline.settle(&self.settings);
                self.ui
                    .toast(format!("Keyed {n} value{} at {time:.2} s", plural(n)));
            }
        }
    }
}
fn selection_name(s: Selection) -> String {
    match s {
        Selection::Object(i) => format!("object {}", i + 1),
        Selection::Light(i) => format!("light {}", i + 1),
        Selection::MainLight => "the main light".into(),
        Selection::Flashlight => "the flashlight".into(),
        _ => "the camera".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_gaining_keys_opens_the_timeline() {
        let mut had = false;
        // Empty scene, then a keyed one: opens once, so it can then stay closed.
        assert!(!super::gained_keys(&mut had, false));
        assert!(super::gained_keys(&mut had, true));
        assert!(!super::gained_keys(&mut had, true));
        // Keys cleared and added again: opens again.
        assert!(!super::gained_keys(&mut had, false));
        assert!(super::gained_keys(&mut had, true));
    }
}
