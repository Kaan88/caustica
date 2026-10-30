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
                ui.separator();
                if ui
                    .button("Key")
                    .on_hover_text(
                        "I: key the selection here (the camera when nothing is selected): what it already animates, what you changed, and on its first key its transform. Shift+I keys every value you changed anywhere.",
                    )
                    .clicked()
                {
                    self.key_selection(false);
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
fn selection_name(s: Selection) -> String {
    match s {
        Selection::Object(i) => format!("object {}", i + 1),
        Selection::Light(i) => format!("light {}", i + 1),
        Selection::MainLight => "the main light".into(),
        _ => "the camera".into(),
    }
}
