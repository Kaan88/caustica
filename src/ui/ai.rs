//! Text-to-scene and image description (the AI menu).
use super::*;

impl App {
    /// Track 19: "Describe a scene" sends the text to Claude and loads the scene it returns.
    /// Applies a finished text-to-scene request (whatever tab is open).
    /// Starts a text-to-scene request for `mode` on its own thread (`png`: the render, for a
    /// critique).
    pub fn start_describe(&mut self, mode: AiMode, png: Option<Vec<u8>>, ctx: &egui::Context) {
        use crate::textscene::Task;
        let task = match (mode, png) {
            (AiMode::New, _) => Task::New,
            (AiMode::Edit, _) => Task::Edit(self.settings.clone()),
            (AiMode::Critique, Some(png)) => Task::Critique {
                settings: self.settings.clone(),
                png,
            },
            (AiMode::Critique, None) => return,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ai, key, prompt) = (
            self.ui.prefs.ai.clone(),
            self.ui.ai_key.clone(),
            self.ui.describe.clone(),
        );
        let (stop, ctx) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let mut progress = move |s: String| {
                let _ = progress_tx.send(AiMsg::Progress(s));
                progress_ctx.request_repaint();
            };
            let result =
                crate::textscene::transport(&ai, &key, stop.clone()).and_then(|mut send| {
                    crate::textscene::generate(&prompt, &task, &mut *send, &mut progress, &stop)
                });
            let _ = tx.send(AiMsg::Done(Box::new(result)));
            ctx.request_repaint();
        });
        self.ui.ai_notes.clear();
        self.ui.critique_scene = None;
        self.ui.describe_job = Some(AiJob {
            rx,
            cancel,
            started: std::time::Instant::now(),
            status: "Starting".into(),
            mode,
        });
    }
    pub(super) fn poll_describe(&mut self, ctx: &egui::Context) {
        if let Some(png) = self.ui.critique_png.take() {
            self.start_describe(AiMode::Critique, Some(png), ctx);
        }
        let fetched = self
            .ui
            .ai_models_job
            .as_ref()
            .and_then(|(rx, server)| match rx.try_recv() {
                Ok(r) => Some((r, server.clone())),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    Err(anyhow::anyhow!("the request stopped unexpectedly")),
                    server.clone(),
                )),
            });
        if let Some((r, server)) = fetched {
            self.ui.ai_models_job = None;
            let current = ai_server(&self.ui.prefs.ai);
            match r {
                // Asked of another server than the one now chosen.
                _ if server != current => {}
                Ok(list) if list.is_empty() => self.ui.error("The server listed no models"),
                Ok(list) => {
                    self.ui.toast(format!("{} models", list.len()));
                    self.ui.ai_models = list;
                }
                Err(e) => self.ui.error(format!("Fetching models: {e:#}")),
            }
        }
        let Some(job) = &mut self.ui.describe_job else {
            return;
        };
        let result = loop {
            match job.rx.try_recv() {
                Ok(AiMsg::Progress(s)) => job.status = s,
                Ok(AiMsg::Done(r)) => break *r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    break Err(anyhow::anyhow!("the request stopped unexpectedly"));
                }
            }
        };
        let mode = job.mode;
        self.ui.describe_job = None;
        match result {
            Ok(outcome) => {
                self.ui.ai_notes = outcome.notes;
                if mode == AiMode::Critique {
                    // A critique's scene waits for Apply, next to the notes.
                    self.ui.critique_scene = Some(outcome.settings);
                    self.ui.toast("The critique is in the Scene tab");
                } else {
                    if let Some(st) = &mut self.stats {
                        st.mark("wordsmith");
                    }
                    let who = self.ui.prefs.ai.who();
                    let title = self.apply_ai_scene(outcome.settings, mode == AiMode::New);
                    let verb = if mode == AiMode::New {
                        "made"
                    } else {
                        "changed"
                    };
                    self.ui.toast(format!("{who} {verb} {title}"));
                }
            }
            Err(e) => self.ui.error(format!("{e:#}")),
        }
    }
    /// Loads a scene from the model (undoable). `fresh`: a new scene, not the file or list
    /// entry that was open (Ctrl+S must not overwrite that file because the titles happen to
    /// match); an edit keeps them. Returns the title.
    pub(super) fn apply_ai_scene(&mut self, mut loaded: Settings, fresh: bool) -> String {
        self.ui.checkpoint_scene(&self.settings);
        self.laser = crate::laser::Laser::default();
        self.physics = None;
        self.ui.selection = Selection::Stage;
        if fresh {
            self.ui.loaded_file = None;
            self.ui.scene_selected = None;
        }
        loaded.keep_output(&self.settings);
        if loaded.scene_name.trim().is_empty() {
            loaded.scene_name = if fresh {
                "Described scene".into()
            } else {
                self.settings.scene_name.clone()
            };
        }
        let title = loaded.scene_name.clone();
        self.settings = loaded;
        self.reset_baseline = self.settings.clone();
        self.ui.search.clear();
        title
    }
    /// Menu > Text-to-scene model: the backend, its endpoint and key (the model and the
    /// reasoning level sit under the text box).
    pub(super) fn ai_menu(&mut self, ui: &mut egui::Ui) {
        use crate::textscene::{Protocol, Provider};
        let before = self.ui.prefs.ai.clone();
        let ai = &mut self.ui.prefs.ai;
        for p in Provider::ALL {
            ui.radio_value(&mut ai.provider, p, p.label());
        }
        ui.separator();
        egui::Grid::new("ai grid").num_columns(2).show(ui, |ui| {
            if ai.provider == Provider::Custom {
                ui.label("Endpoint");
                ui.add(
                    egui::TextEdit::singleline(&mut ai.base_url)
                        .hint_text(field_hint("http://localhost:11434/v1"))
                        .desired_width(220.),
                )
                .on_hover_text("Ollama: http://localhost:11434/v1 · LM Studio: http://localhost:1234/v1 · llama.cpp: http://localhost:8080/v1");
                ui.end_row();
                ui.label("Protocol");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut ai.protocol, Protocol::OpenAi, "OpenAI-compatible");
                    ui.radio_value(&mut ai.protocol, Protocol::Anthropic, "Anthropic-compatible");
                });
                ui.end_row();
            }
            if matches!(ai.provider, Provider::ClaudeApi | Provider::OpenAi | Provider::Custom) {
                ui.label("API key");
                let hint = ai
                    .provider
                    .key_env()
                    .map_or("optional".to_string(), |e| format!("or set {e}"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.ui.ai_key)
                        .password(true)
                        .hint_text(field_hint(hint))
                        .desired_width(220.),
                )
                .on_hover_text("Kept for this session only, never saved");
                ui.end_row();
            }
        });
        let note = match ai.provider {
            Provider::ClaudeCli => Some(
                "Runs `claude -p` with the CLI's login: a Claude subscription works, no key needed.",
            ),
            Provider::CodexCli => Some(
                "Runs `codex exec` with the Codex CLI's login: sign in once with `codex login` (a ChatGPT account works). Install it with: npm install -g @openai/codex",
            ),
            Provider::OpencodeCli => Some(
                "Runs `opencode run` with the providers set up in opencode (`opencode auth login`). Fetch under the text box lists `opencode models`; a model is provider/model. Install it with: npm install -g opencode-ai",
            ),
            _ => None,
        };
        if let Some(note) = note {
            ui.label(egui::RichText::new(note).small());
        }
        let now = &self.ui.prefs.ai;
        if *now != before {
            if now.provider != before.provider || now.protocol != before.protocol {
                // Another kind of server: its models and reasoning levels differ.
                self.ui.ai_models.clear();
                self.ui.prefs.ai.model.clear();
                self.ui.prefs.ai.effort.clear();
            } else if now.base_url != before.base_url {
                // Typing a URL keeps the model; only the fetched list belongs to the old one.
                self.ui.ai_models.clear();
            }
            self.ui.save_prefs();
        }
    }
    /// The model and reasoning pickers under the text box.
    /// The saved backends (track 19): pick one in a click, save the current one, forget one.
    pub(super) fn ai_backend_row(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Backend");
            let prefs = &mut self.ui.prefs;
            egui::ComboBox::from_id_salt("ai backend")
                .selected_text(prefs.ai.short_label())
                .show_ui(ui, |ui| {
                    for p in prefs.ai_profiles.clone() {
                        if ui.selectable_label(p == prefs.ai, p.short_label()).clicked() {
                            prefs.ai = p;
                            changed = true;
                        }
                    }
                    if prefs.ai_profiles.is_empty() {
                        ui.label("No saved backends: set one up in Menu > Text-to-scene model, then Save");
                    }
                });
            let saved = prefs.ai_profiles.contains(&prefs.ai);
            if !saved
                && ui
                    .small_button("Save")
                    .on_hover_text("Adds this backend to the list (keys are never saved)")
                    .clicked()
            {
                prefs.ai_profiles.push(prefs.ai.clone());
                changed = true;
            }
            if saved
                && ui
                    .small_button("Forget")
                    .on_hover_text("Removes this backend from the list")
                    .clicked()
            {
                let current = prefs.ai.clone();
                prefs.ai_profiles.retain(|p| *p != current);
                changed = true;
            }
        });
        if changed {
            self.ui.save_prefs();
        }
    }
    pub(super) fn ai_model_row(&mut self, ui: &mut egui::Ui) {
        use crate::textscene::Provider;
        let before = self.ui.prefs.ai.clone();
        let provider = before.provider;
        ui.horizontal_wrapped(|ui| {
            ui.label("Model");
            let hint = match provider {
                Provider::ClaudeCli | Provider::CodexCli | Provider::OpencodeCli => "the CLI's default",
                Provider::ClaudeApi => "claude-opus-5-5",
                _ => "required",
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.ui.prefs.ai.model)
                    .hint_text(field_hint(hint))
                    .desired_width(170.),
            );
            let mut names: Vec<String> = provider
                .suggested_models()
                .iter()
                .map(|s| s.to_string())
                .collect();
            for m in &self.ui.ai_models {
                if !names.contains(m) {
                    names.push(m.clone());
                }
            }
            if !names.is_empty() {
                ui.menu_button("Pick", |ui| {
                    egui::ScrollArea::vertical().max_height(300.).show(ui, |ui| {
                        if ui.button("Default").clicked() {
                            self.ui.prefs.ai.model.clear();
                            ui.close();
                        }
                        for n in &names {
                            if ui.button(n).clicked() {
                                self.ui.prefs.ai.model = n.clone();
                                ui.close();
                            }
                        }
                    });
                });
            }
            if provider.can_fetch() {
                let fetching = self.ui.ai_models_job.is_some();
                if ui
                    .add_enabled(!fetching, egui::Button::new("Fetch"))
                    .on_hover_text("Lists the models the server offers")
                    .clicked()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (ai, key, ctx) = (
                        self.ui.prefs.ai.clone(),
                        self.ui.ai_key.clone(),
                        ui.ctx().clone(),
                    );
                    let server = ai_server(&ai);
                    std::thread::spawn(move || {
                        let _ = tx.send(crate::textscene::fetch_models(&ai, &key));
                        ctx.request_repaint();
                    });
                    self.ui.ai_models_job = Some((rx, server));
                }
                if fetching {
                    ui.spinner();
                }
            }
            ui.label("Reasoning");
            let levels = provider.efforts(before.protocol);
            let shown = |e: &str| {
                if e.is_empty() {
                    "Default".to_string()
                } else {
                    e.to_string()
                }
            };
            egui::ComboBox::from_id_salt("ai effort")
                .selected_text(shown(&self.ui.prefs.ai.effort))
                .show_ui(ui, |ui| {
                    for e in levels {
                        ui.selectable_value(&mut self.ui.prefs.ai.effort, e.to_string(), shown(e));
                    }
                })
                .response
                .on_hover_text(
                    "How long the model thinks before answering: higher is slower and usually better",
                );
        });
        if self.ui.prefs.ai != before {
            self.ui.save_prefs();
        }
    }
    pub(super) fn describe_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Describe a scene (AI)")
            .id_salt("describe")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let m = &mut self.ui.describe_mode;
                    ui.selectable_value(m, AiMode::New, "New scene")
                        .on_hover_text("Builds a new scene from the description");
                    ui.selectable_value(m, AiMode::Edit, "Edit this scene")
                        .on_hover_text("Changes the current scene as described (\"make it night\")");
                    ui.selectable_value(m, AiMode::Critique, "Critique the render")
                        .on_hover_text("Sends the current render and scene: the model suggests lighting and composition changes, which you can apply");
                });
                let mode = self.ui.describe_mode;
                let hint = match mode {
                    AiMode::New => "A lighthouse on a rock at dusk, a warm lamp in its top…",
                    AiMode::Edit => "Make it night, with the lamp as the only light…",
                    AiMode::Critique => "Optional: what to focus on (mood, colour, the subject)…",
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.ui.describe)
                        .hint_text(field_hint(hint))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                self.ai_backend_row(ui);
                self.ai_model_row(ui);
                let missing = self.ui.prefs.ai.missing(&self.ui.ai_key);
                ui.horizontal_wrapped(|ui| {
                    if let Some(job) = &self.ui.describe_job {
                        ui.spinner();
                        ui.label(format!("{} ({} s)", job.status, job.started.elapsed().as_secs()));
                        if ui.button("Cancel").clicked() {
                            // Stops a running CLI; an HTTP request finishes in the background
                            // and its result is dropped with the channel.
                            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            self.ui.describe_job = None;
                        }
                        // The elapsed time ticks.
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        return;
                    }
                    if self.ui.critique_capture {
                        ui.spinner();
                        ui.label("Saving the render");
                        return;
                    }
                    let text_ok = mode == AiMode::Critique || !self.ui.describe.trim().is_empty();
                    let label = match mode {
                        AiMode::New => "Generate",
                        AiMode::Edit => "Apply edit",
                        AiMode::Critique => "Critique",
                    };
                    if ui
                        .add_enabled(missing.is_none() && text_ok, egui::Button::new(label))
                        .on_hover_text(format!(
                            "Sends it to {}. The result is undoable (Ctrl+Z). Change the backend in Menu > Text-to-scene model",
                            self.ui.prefs.ai.provider.label()
                        ))
                        .on_disabled_hover_text(missing.as_deref().unwrap_or("Describe what you want first"))
                        .clicked()
                    {
                        if mode == AiMode::Critique {
                            // main.rs saves the render after this frame and starts it.
                            self.ui.critique_capture = true;
                        } else {
                            self.start_describe(mode, None, ui.ctx());
                        }
                    }
                    match &missing {
                        Some(why) => ui.label(egui::RichText::new(why).small()),
                        None => ui.label(
                            egui::RichText::new(self.ui.prefs.ai.provider.label()).small(),
                        ),
                    };
                });
                if !self.ui.ai_notes.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.ui.ai_notes).small());
                }
                if self.ui.critique_scene.is_some() {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Apply the suggestions")
                            .on_hover_text("Loads the improved scene (Ctrl+Z undoes it)")
                            .clicked()
                            && let Some(s) = self.ui.critique_scene.take()
                        {
                            let title = self.apply_ai_scene(s, false);
                            self.ui.toast(format!("Applied the critique to {title}"));
                        }
                        if ui.button("Dismiss").clicked() {
                            self.ui.critique_scene = None;
                            self.ui.ai_notes.clear();
                        }
                    });
                }
            });
    }
}
