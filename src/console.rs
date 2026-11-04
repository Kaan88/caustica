//! The script console (wave 3; Menu → *Script console…*): runs a Rhai script with the
//! `--script` API (`script.rs`: load, set, get, frames, capture, add, clear, …) from a text box,
//! with its printed output and errors in a log, and edits the animation's per-frame script
//! (`Animation::script`, run on every frame by `Animation::apply`).
use crate::App;

#[derive(Default)]
pub struct Console {
    pub open: bool,
    pub source: String,
    pub log: Vec<String>,
    /// The *Per frame* tab is shown (else *Run*).
    pub per_frame: bool,
}

/// Appends a line to the console's log, keeping the last 2000.
pub fn log(log: &mut Vec<String>, line: String) {
    log.push(line);
    if log.len() > 2000 {
        log.drain(..log.len() - 2000);
    }
}

const RUN_HELP: &str = "load(scene), set(key, json), get(key), frames(n), reset(), capture(path), samples(), scenes(), add(object json), clear(), print(x). A key is a setting or a / path such as objects/0/rotation. Ctrl+Enter runs.";
const FRAME_HELP: &str = "Runs on every frame of playback and timeline exports, after the keyframes: set(key, value) and get(key) on any setting by its / path, with t (seconds), frame and fps. For example: set(\"objects/0/rotation\", t * 90.0);";

impl App {
    pub(crate) fn console_window(&mut self, ctx: &egui::Context) {
        let mut open = self.ui.console.open;
        let (mut run, mut stop, mut edit_started) = (false, false, false);
        let running = self.script.is_some();
        egui::Window::new("Script console")
            .open(&mut open)
            .default_size([560., 440.])
            .show(ctx, |ui| {
                let c = &mut self.ui.console;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut c.per_frame, false, "Run");
                    ui.selectable_value(&mut c.per_frame, true, "Per frame");
                });
                ui.separator();
                if !c.per_frame {
                    ui.label(egui::RichText::new(RUN_HELP).weak());
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut c.source)
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY),
                    );
                    let ctrl_enter = r.has_focus()
                        && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!running, egui::Button::new("Run"))
                            .on_hover_text("Ctrl+Enter")
                            .clicked()
                            || (ctrl_enter && !running)
                        {
                            run = true;
                        }
                        if ui.add_enabled(running, egui::Button::new("Stop")).clicked() {
                            stop = true;
                        }
                        if ui.button("Clear log").clicked() {
                            c.log.clear();
                        }
                        if running {
                            ui.spinner();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, true])
                        .max_height(180.)
                        .show(ui, |ui| {
                            for line in &c.log {
                                ui.monospace(line);
                            }
                        });
                } else {
                    ui.label(egui::RichText::new(FRAME_HELP).weak());
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut self.settings.animation.script)
                            .code_editor()
                            .desired_rows(12)
                            .desired_width(f32::INFINITY),
                    );
                    edit_started = r.gained_focus();
                    let error = crate::script::FRAME_ERROR
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    match error {
                        Some(e) if !self.settings.animation.script.trim().is_empty() => {
                            ui.colored_label(egui::Color32::from_rgb(240, 110, 100), e);
                        }
                        _ => {
                            ui.label(egui::RichText::new("Runs when the playhead moves.").weak());
                        }
                    }
                }
            });
        self.ui.console.open = open;
        if edit_started {
            self.ui.checkpoint(&self.settings);
        }
        if run {
            self.ui.checkpoint(&self.settings);
            log(&mut self.ui.console.log, "Run".into());
            self.script = Some(crate::script::Script::run(self.ui.console.source.clone()));
        }
        if stop && let Some(sc) = &mut self.script {
            sc.stop();
        }
    }
}
