//! The timeline strip (track 8): transport, playhead, key markers, keying and the animation's
//! length, frame rate and shutter. Drawn above the status bar while it is shown or the scene has
//! keys.
use crate::App;
use crate::anim::{self, Interp, Timeline};
use crate::ui::{ACCENT, Selection};

impl App {
    /// I: key the selection (the camera when nothing is selected); Shift+I: every edited value.
    pub fn key_selection(&mut self, changed: bool) {
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
    pub(crate) fn timeline_panel(&mut self, ctx: &egui::Context) {
        if !self.ui.show_timeline && self.settings.animation.is_empty() {
            return;
        }
        let prefixes = anim::prefixes(self.ui.selection);
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
            ui.add_space(2.);
            ui.horizontal(|ui| {
                let a = &self.settings.animation;
                let keys = a.times(&prefixes);
                let t = self.timeline.time;
                if ui.button("|◀").on_hover_text("To the start").clicked() {
                    self.timeline.time = 0.;
                }
                let previous = keys.iter().rev().find(|k| **k < t - 1e-4).copied();
                if ui
                    .add_enabled(previous.is_some(), egui::Button::new("◀ key"))
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
                if ui.button(play).on_hover_text("K").clicked() {
                    self.toggle_playback();
                }
                let next = keys.iter().find(|k| **k > t + 1e-4).copied();
                if ui
                    .add_enabled(next.is_some(), egui::Button::new("key ▶"))
                    .on_hover_text("The selection's next key")
                    .clicked()
                {
                    self.timeline.time = next.unwrap_or(t);
                    self.timeline.playing = false;
                }
                if ui.button("▶|").on_hover_text("To the end").clicked() {
                    self.timeline.time = self.settings.animation.length;
                }
                let a = &self.settings.animation;
                let mut time = self.timeline.time;
                let r = ui
                    .add(
                        egui::DragValue::new(&mut time)
                            .range(0. ..=a.length)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .on_hover_text("Playhead. Shift+Left / Right step a frame.");
                if r.changed() {
                    self.timeline.time = time;
                    self.timeline.playing = false;
                }
                ui.label(
                    egui::RichText::new(format!(
                        "frame {}",
                        (self.timeline.time * a.fps as f32).round() as u32
                    ))
                    .weak(),
                );
                ui.checkbox(&mut self.timeline.looping, "Loop");
                let mut path = self.settings.animation.camera_path;
                if ui
                    .checkbox(&mut path, "Camera path")
                    .on_hover_text("Between the camera's keys, the eye and its target move along smooth curves through their keyed positions (drawn in the viewport), instead of yaw, pitch, distance and target each moving alone, which swings the eye around the target")
                    .changed()
                {
                    self.ui.checkpoint(&self.settings);
                    self.settings.animation.camera_path = path;
                    self.timeline.applied = None;
                }
                ui.checkbox(&mut self.timeline.curves, "Curves").on_hover_text(
                    "The curve editor: the selection's animated values over time, each scaled to its own range. Drag a key to move it in time (frames, Shift for free) and value.",
                );
                ui.separator();
                ui.checkbox(&mut self.timeline.auto_key, "Auto-key").on_hover_text(
                    "Every edit you finish at a still playhead is keyed there (as Shift+I)",
                );
                if ui
                    .button("Key")
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
                    ui.menu_button("Keys", |ui| {
                        for (label, r) in [
                            ("Later by a frame", (1., frame)),
                            ("Earlier by a frame", (1., -frame)),
                            ("Later by a second", (1., 1.)),
                            ("Earlier by a second", (1., -1.)),
                            ("Twice as slow (from the playhead)", (2., 0.)),
                            ("Twice as fast (from the playhead)", (0.5, 0.)),
                        ] {
                            if ui.button(label).clicked() {
                                retime = Some(r);
                                ui.close_menu();
                            }
                        }
                        let others = self.ui.also.len();
                        if ui
                            .add_enabled(
                                others > 0,
                                egui::Button::new(format!("Copy to the {others} other selected")),
                            )
                            .on_hover_text("Replace the other selected objects' or lights' keys with the active one's")
                            .clicked()
                        {
                            copy = true;
                            ui.close_menu();
                        }
                    })
                    .response
                    .on_hover_text("Move, stretch or copy the selection's keys");
                });
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
                    .add_enabled(here, egui::Button::new("Delete key"))
                    .on_hover_text("Alt+I: the selection's keys at the playhead")
                    .clicked()
                {
                    self.delete_keys();
                }
                if here {
                    let current = self.settings.animation.interp_at(&prefixes, t);
                    let mut chosen = None;
                    egui::ComboBox::from_id_salt("interp")
                        .selected_text(current.map_or("Mixed", Interp::label))
                        .show_ui(ui, |ui| {
                            for i in Interp::ALL {
                                if ui.selectable_label(current == Some(i), i.label()).clicked() {
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
                    if ui
                        .add_enabled(
                            !self.settings.animation.is_empty(),
                            egui::Button::new("Clear"),
                        )
                        .on_hover_text("Remove every key in the scene")
                        .clicked()
                    {
                        self.ui.checkpoint(&self.settings);
                        self.settings.animation.tracks.clear();
                        self.timeline.playing = false;
                    }
                    let a = &mut self.settings.animation;
                    ui.add(
                        egui::DragValue::new(&mut a.shutter)
                            .range(0. ..=1.)
                            .speed(0.01)
                            .custom_formatter(|v, _| format!("{:.0}°", v * 360.))
                            .custom_parser(|t| {
                                t.trim_end_matches('°').trim().parse::<f64>().ok().map(|v| v / 360.)
                            }),
                    )
                    .on_hover_text(
                        "Motion blur in timeline video exports: how long the shutter stays open, as a shutter angle (180° = half a frame; 0 = off)",
                    );
                    ui.label("Shutter");
                    ui.add(egui::DragValue::new(&mut a.fps).range(1..=240).suffix(" fps"));
                    ui.add(
                        egui::DragValue::new(&mut a.length)
                            .range(0.1..=3600.)
                            .speed(0.05)
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .on_hover_text("Length of the animation");
                    ui.label("Length");
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
    /// The scrubbing strip: seconds, every key (faint), the selection's keys and the playhead.
    fn track_strip(&mut self, ui: &mut egui::Ui, prefixes: &[String]) {
        let a = &self.settings.animation;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 30.),
            egui::Sense::click_and_drag(),
        );
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
                painter.text(
                    egui::pos2(x + 2., rect.top() + 1.),
                    egui::Align2::LEFT_TOP,
                    format!("{:.*}", if step < 1. { 1 } else { 0 }, i as f32 * step),
                    egui::FontId::proportional(10.),
                    v.weak_text_color(),
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
            diamond(egui::pos2(x_of(t), mid), 3.5, v.weak_text_color());
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
    }
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
                visuals.weak_text_color(),
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
fn selection_name(s: Selection) -> String {
    match s {
        Selection::Object(i) => format!("object {}", i + 1),
        Selection::Light(i) => format!("light {}", i + 1),
        Selection::MainLight => "the main light".into(),
        _ => "the camera".into(),
    }
}
