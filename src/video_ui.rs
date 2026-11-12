//! The video export dialog and its progress window (item 6). The options live in
//! `UiState::video`; `redraw` starts the `video::Job` and renders it a pass at a time.
use crate::App;
use crate::ui::{ACCENT, BAR, grid, note, row, section};
use crate::video::{Easing, Format, PRORES_PROFILES};

const SIZES: [(&str, u32, u32); 8] = [
    ("720p", 1280, 720),
    ("1080p", 1920, 1080),
    ("1440p", 2560, 1440),
    ("4K UHD", 3840, 2160),
    ("8K UHD", 7680, 4320),
    ("Square 1080", 1080, 1080),
    ("Vertical 1080", 1080, 1920),
    ("Vertical 4K", 2160, 3840),
];
const RATES: [u32; 6] = [24, 25, 30, 48, 50, 60];

impl App {
    /// Opens the dialog with the orbit taken from the current camera.
    pub fn open_video_dialog(&mut self) {
        self.ui.show_video = true;
        self.ui.video.pitch = self.settings.pitch;
        self.ui.video.distance = self.settings.distance;
    }
    pub fn video_window(&mut self, ctx: &egui::Context) {
        if !self.ui.show_video {
            return;
        }
        let Some(g) = self.gpu.as_ref() else {
            return;
        };
        let (window, hdr) = ([g.width, g.height], g.hdr_active);
        let ffmpeg = crate::video::ffmpeg_available();
        let mut open = true;
        let mut start = false;
        let camera = (self.settings.pitch, self.settings.distance);
        let animation = self.settings.animation.clone();
        // A running simulation plays on the timeline too (keys or not).
        let simulated = self.settings.physics.enabled;
        if animation.is_empty() && !simulated {
            self.ui.video.timeline = false;
        }
        if self.ui.video.timeline && !animation.is_empty() {
            // The keys set the rate and the length.
            self.ui.video.fps = animation.fps;
            self.ui.video.frames = animation.frames();
        }
        egui::Window::new("Export video")
            .open(&mut open)
            .default_pos([160., 80.])
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let o = &mut self.ui.video;
                section(ui, "Output");
                grid("video-output").show(ui, |ui| {
                    row(ui, "Format", "Videos go through ffmpeg; sequences are numbered files in a folder named after the file.", |ui| {
                        egui::ComboBox::from_id_salt("video-format")
                            .selected_text(o.format.label())
                            .width(200.)
                            .show_ui(ui, |ui| {
                                for f in Format::ALL {
                                    let usable = (!f.video() || ffmpeg) && (f != Format::Hdr10 || hdr);
                                    let r = ui.add_enabled(usable, egui::Button::selectable(o.format == f, f.label()));
                                    let r = if f == Format::Hdr10 && !hdr {
                                        r.on_disabled_hover_text("Needs HDR output (Look > Display).")
                                    } else if f.video() && !ffmpeg {
                                        r.on_disabled_hover_text("ffmpeg was not found next to the exe or on PATH (tools/setup.ps1 fetches it).")
                                    } else {
                                        r
                                    };
                                    if r.clicked() && o.format != f {
                                        o.format = f;
                                        if let Some((_, _, q)) = f.quality_range() {
                                            o.quality = q;
                                        }
                                        let ext = f.extension();
                                        o.path = if ext.is_empty() { o.path.with_extension("") } else { o.path.with_extension(ext) };
                                    }
                                }
                            });
                    });
                    match o.format.quality_range() {
                        Some((lo, hi, _)) if o.format == Format::ProRes => {
                            row(ui, "Profile", "ProRes profile: larger files keep more detail.", |ui| {
                                for (i, name) in PRORES_PROFILES.iter().enumerate() {
                                    ui.selectable_value(&mut o.quality, i as u32, *name);
                                }
                                o.quality = o.quality.clamp(lo, hi);
                            });
                        }
                        Some((lo, hi, _)) => {
                            row(ui, "Quality (CRF)", "Constant rate factor: lower is better and larger. 16–18 looks lossless for renders.", |ui| {
                                ui.add(crate::slider(&mut o.quality, lo..=hi));
                            });
                        }
                        None => {}
                    }
                    row(ui, "File", "Relative paths are in the working folder.", |ui| {
                        let mut text = o.path.to_string_lossy().into_owned();
                        if ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.)).changed() {
                            o.path = text.into();
                        }
                        if ui.button("Browse…").clicked() {
                            let ext = o.format.extension();
                            let mut d = rfd::FileDialog::new().set_file_name(
                                o.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            );
                            if !ext.is_empty() {
                                d = d.add_filter(o.format.label(), &[ext]);
                            }
                            if let Some(p) = d.save_file() {
                                o.path = p;
                            }
                        }
                    });
                });
                section(ui, "Size and timing");
                grid("video-size").show(ui, |ui| {
                    row(ui, "Size", "Independent of the window, up to 8192 on each side. Videos round down to even sizes.", |ui| {
                        let current = SIZES
                            .iter()
                            .find(|s| [s.1, s.2] == [o.width, o.height])
                            .map(|s| s.0.to_string())
                            .unwrap_or_else(|| if [o.width, o.height] == window { "Window".into() } else { "Custom".into() });
                        egui::ComboBox::from_id_salt("video-size")
                            .selected_text(current)
                            .width(110.)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(false, format!("Window ({}×{})", window[0], window[1])).clicked() {
                                    [o.width, o.height] = window;
                                }
                                for (name, w, h) in SIZES {
                                    if ui.selectable_label([w, h] == [o.width, o.height], format!("{name} ({w}×{h})")).clicked() {
                                        [o.width, o.height] = [w, h];
                                    }
                                }
                            });
                        ui.add(egui::DragValue::new(&mut o.width).range(16..=8192));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut o.height).range(16..=8192));
                    });
                    let timed = !o.timeline || animation.is_empty();
                    row(ui, "Frame rate", "Frames per second. A timeline export takes the timeline's.", |ui| {
                        if !timed {
                            ui.disable();
                        }
                        let seconds = o.frames as f32 / o.fps as f32;
                        for fps in RATES {
                            if ui.selectable_label(o.fps == fps, fps.to_string()).clicked() && o.fps != fps {
                                // Keep the length in seconds.
                                o.fps = fps;
                                o.frames = ((seconds * fps as f32).round() as u32).max(1);
                            }
                        }
                    });
                    row(ui, "Length", "Duration in seconds; the frame count follows. A timeline export takes the timeline's.", |ui| {
                        if !timed {
                            ui.disable();
                        }
                        let mut seconds = o.frames as f32 / o.fps as f32;
                        if ui
                            .add(crate::slider(&mut seconds, 0.1..=600.).logarithmic(true).suffix(" s"))
                            .changed()
                        {
                            o.frames = ((seconds * o.fps as f32).round() as u32).max(1);
                        }
                        ui.add(egui::DragValue::new(&mut o.frames).range(1..=100_000).suffix(" frames"));
                    });
                    row(ui, "Samples per frame", "Each frame converges this far before the next.", |ui| {
                        ui.add(crate::slider(&mut o.spp, 1..=65536).logarithmic(true).suffix(" spp"));
                    });
                    row(ui, "Time limit per frame", "A frame also ends when this time is up (0 = no limit), so noise varies with the view.", |ui| {
                        ui.add(crate::slider(&mut o.seconds_per_frame, 0. ..=600.).logarithmic(true).smallest_positive(0.1).custom_formatter(|v, _| if v <= 0. { "off".into() } else { format!("{v:.1} s") }));
                    });
                });
                section(ui, "Motion");
                grid("video-motion").show(ui, |ui| {
                    row(ui, "Camera", "Orbit turns the camera around its target; Timeline plays the scene's keyframes (camera, objects, lights, materials), with motion blur when the timeline's shutter is open.", |ui| {
                        ui.selectable_value(&mut o.timeline, false, "Orbit");
                        ui.add_enabled_ui(!animation.is_empty() || simulated, |ui| {
                            crate::ui::why_disabled(
                                ui.selectable_value(&mut o.timeline, true, "Timeline"),
                                "The scene has no keyframes (I keys the selection).",
                            );
                        });
                    });
                });
                if o.timeline && simulated {
                    note(ui, "The physics simulation runs from the scene's current poses, one step per frame in simulated time; keyframed objects drive it. Motion blur is off for it.");
                }
                if o.timeline && !animation.is_empty() {
                    note(
                        ui,
                        &format!(
                            "{:.2} s at {} fps from the timeline. Motion blur: {}.",
                            animation.length,
                            animation.fps,
                            if animation.shutter > 0. {
                                format!("{:.0}° shutter, one moment per pass (Ray Reconstruction is off for it)", animation.shutter * 360.)
                            } else {
                                "off (set Shutter on the timeline)".into()
                            }
                        ),
                    );
                }
                if !o.timeline {
                grid("video-orbit").show(ui, |ui| {
                    row(ui, "Angle", "How far the camera turns over the whole video, from the current view.", |ui| {
                        ui.add(crate::slider(&mut o.degrees, -1080. ..=1080.).suffix("°"));
                    });
                    row(ui, "Elevation", "Camera pitch for the whole orbit.", |ui| {
                        ui.add(crate::slider(&mut o.pitch, -89. ..=89.).suffix("°"));
                    });
                    row(ui, "Distance", "Orbit radius around the target.", |ui| {
                        ui.add(crate::slider(&mut o.distance, 0.1..=200.).logarithmic(true));
                    });
                    row(ui, "Easing", "Linear loops seamlessly at 360°; ease in-out starts and stops at rest.", |ui| {
                        ui.selectable_value(&mut o.easing, Easing::Linear, "Linear");
                        ui.selectable_value(&mut o.easing, Easing::EaseInOut, "Ease in-out");
                    });
                    row(ui, "", "", |ui| {
                        if ui.button("Use the current camera").clicked() {
                            (o.pitch, o.distance) = camera;
                        }
                    });
                });
                }
                let [w, h] = o.size();
                let seconds = o.frames as f32 / o.fps as f32;
                note(
                    ui,
                    &format!(
                        "{} frames at {w}×{h}, {seconds:.1} s. The window stays responsive and shows progress; the scene can't be edited until it ends.",
                        o.frames
                    ),
                );
                ui.add_space(4.);
                if ui.button("Render video").clicked() {
                    start = true;
                }
            });
        if start {
            self.actions.export_video = true;
            self.ui.show_video = false;
        }
        if !open {
            self.ui.show_video = false;
        }
    }
    pub(crate) fn exporting(&self) -> bool {
        self.export.is_some() || self.cpu_export.is_some()
    }
    /// The running export in numbers, for the top bar's chip, the status bar and the card.
    pub(crate) fn export_info(&self) -> Option<ExportInfo> {
        if let Some(job) = &self.cpu_export {
            let progress = job.progress();
            let elapsed = job.started.elapsed().as_secs_f32();
            return Some(ExportInfo {
                title: if job.options.still {
                    "Rendering image (CPU)"
                } else {
                    "Exporting video (CPU)"
                },
                progress,
                left: (progress > 0.01).then(|| (elapsed * (1. - progress) / progress) as u64),
                frame: None,
            });
        }
        let job = self.export.as_ref()?;
        let samples = self.gpu.as_ref().map_or(0, |g| g.samples);
        let o = &job.options;
        let progress = job.progress(samples);
        // A video knows after its first frame; a still goes by its samples.
        let left = job.remaining().map(|d| d.as_secs()).or_else(|| {
            (progress > 0.02)
                .then(|| (job.started.elapsed().as_secs_f32() * (1. - progress) / progress) as u64)
        });
        Some(ExportInfo {
            title: if o.still {
                "Rendering image"
            } else {
                "Exporting video"
            },
            progress,
            left,
            frame: Some(((job.frame + 1).min(o.frames), o.frames)),
        })
    }
    /// The top bar's middle while an export runs: what it is, how far, how long still.
    pub(crate) fn export_chip(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.export_info() else {
            return;
        };
        ui.add_space(((ui.available_width() - 330.) / 2.).max(0.));
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(38, 42, 43))
            .stroke(egui::Stroke::new(
                1_f32,
                egui::Color32::from_rgb(53, 71, 75),
            ))
            .corner_radius(4)
            .inner_margin(egui::Margin::symmetric(9, 2))
            .show(ui, |ui| {
                ui.label(format!("{} {:.0} %", info.title, info.progress * 100.));
                ui.add(
                    egui::ProgressBar::new(info.progress)
                        .desired_width(70.)
                        .desired_height(6.),
                );
                ui.monospace(info.left_text());
            })
            .response
            .on_hover_text(
                "The scene is locked until the export ends. Cancel is in the Export tab.",
            );
    }
    /// The job's newest preview, as the texture the viewport and the card draw.
    fn take_export_preview(&mut self, ctx: &egui::Context) {
        if let Some(job) = &mut self.export
            && let Some(image) = job.preview.take()
        {
            match &mut self.ui.video_preview {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.ui.video_preview =
                        Some(ctx.load_texture("video-preview", image, egui::TextureOptions::LINEAR))
                }
            }
        }
    }
    /// Over the viewport while an export runs.
    pub(crate) fn export_lock_note(&mut self, ctx: &egui::Context) {
        let rect = crate::ui::central_rect(ctx);
        // A GPU export renders at its own size, which the window can't show as it is (D3D12
        // clears to a near-black, Vulkan stretches it under the panels): the viewport shows the
        // export's last finished frame instead, fitted, and before the first one the view it
        // started from. A CPU export leaves the output alone, so the view stays as it was.
        if self.export.is_some() && rect.width() > 1. && rect.height() > 1. {
            self.take_export_preview(ctx);
            let painter = ctx
                .layer_painter(egui::LayerId::new(
                    egui::Order::Background,
                    egui::Id::unique("export view"),
                ))
                .with_clip_rect(rect);
            painter.rect_filled(rect, 0, egui::Color32::from_gray(16));
            if let Some(t) = &self.ui.video_preview {
                let size = t.size_vec2();
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                painter.image(
                    t.id(),
                    egui::Rect::from_center_size(rect.center(), size * scale),
                    egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
            }
        }
        egui::Area::new(egui::Id::unique("export lock note"))
            .order(egui::Order::Middle)
            .interactable(false)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(rect.center_top() + egui::vec2(0., 10.))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(egui::RichText::new("Locked while exporting").small());
                });
            });
    }
    /// The Export tab's progress card. False when no export runs.
    pub(crate) fn export_card(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(info) = self.export_info() else {
            return false;
        };
        let ctx = ui.ctx().clone();
        self.take_export_preview(&ctx);
        let (options, started, spp) = match (&self.cpu_export, &self.export) {
            (Some(job), _) => (job.options.clone(), job.started, None),
            (None, Some(job)) => {
                let samples = self.gpu.as_ref().map_or(0, |g| g.samples);
                let cap = job.settings().max_samples;
                (
                    job.options.clone(),
                    job.started,
                    Some(format!("{} / {cap} spp", samples.min(cap))),
                )
            }
            (None, None) => return false,
        };
        let gpu_job = self.export.is_some();
        let [w, h] = options.size();
        let armed = self
            .ui
            .close_armed
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(6));
        let mut cancel = false;
        let mut pref_changed = false;
        egui::Frame::new()
            .fill(BAR)
            .stroke(egui::Stroke::new(1_f32, ACCENT.gamma_multiply(0.5)))
            .corner_radius(6)
            .inner_margin(10)
            .show(ui, |ui| {
                let width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(info.title).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.monospace(format!("{:.0} %", info.progress * 100.));
                    });
                });
                if gpu_job {
                    match &self.ui.video_preview {
                        Some(t) => {
                            let size = t.size_vec2();
                            let scale = (width / size.x).min(170. / size.y);
                            ui.image((t.id(), size * scale));
                        }
                        None => {
                            let height = (width * h as f32 / w as f32).min(170.);
                            ui.add_sized([width, height], egui::Spinner::new());
                        }
                    }
                }
                if armed {
                    ui.colored_label(
                        crate::ui::WARN,
                        "Closing cancels this export and deletes the unfinished file. Close again to do that.",
                    );
                }
                ui.label(match (info.frame, &spp) {
                    (Some((n, m)), Some(spp)) if m > 1 => format!("Frame {n} of {m} · {spp}"),
                    (_, Some(spp)) => spp.clone(),
                    _ => format!("{} spp", options.spp),
                });
                note(
                    ui,
                    &format!(
                        "{} elapsed · {}",
                        clock(started.elapsed().as_secs()),
                        info.left_text()
                    ),
                );
                ui.add(
                    egui::ProgressBar::new(info.progress)
                        .desired_width(width)
                        .desired_height(8.),
                );
                if let Some((n, m)) = info.frame
                    && m > 1
                {
                    frame_strip(ui, n, m);
                }
                let format = if options.still {
                    options.image_format().label()
                } else {
                    options.format.label()
                };
                note(ui, &format!("{format} · {w}×{h}"));
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    pref_changed = ui
                        .checkbox(&mut self.ui.prefs.open_folder_done, "Open folder when done")
                        .changed();
                });
            });
        if pref_changed {
            self.ui.save_prefs();
        }
        if cancel {
            self.actions.cancel_export = true;
        }
        // A CPU export does not drive the frame loop.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        true
    }
}
/// What the chip, the status bar and the card say about the running export.
pub(crate) struct ExportInfo {
    pub title: &'static str,
    pub progress: f32,
    /// Seconds left, once that can be estimated.
    pub left: Option<u64>,
    /// The frame being rendered (from 1) and how many there are. A CPU export reports only
    /// its progress.
    pub frame: Option<(u32, u32)>,
}
impl ExportInfo {
    pub fn left_text(&self) -> String {
        self.left.map_or("estimating".into(), about)
    }
    /// The status bar's state.
    pub fn status(&self) -> String {
        match self.frame {
            Some((n, m)) if m > 1 => format!("Exporting frame {n} / {m} · {}", self.left_text()),
            _ => format!(
                "{} {:.0} % · {}",
                self.title,
                self.progress * 100.,
                self.left_text()
            ),
        }
    }
}
/// "~40 s", "~14 min", "~2 h 5 min".
fn about(s: u64) -> String {
    if s < 90 {
        format!("~{s} s")
    } else if s < 5400 {
        format!("~{} min", (s + 30) / 60)
    } else {
        format!("~{} h {} min", s / 3600, s / 60 % 60)
    }
}
/// One cell per frame: done, the current one, still to do. Too many to tell apart become
/// three spans.
fn frame_strip(ui: &mut egui::Ui, current: u32, frames: u32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.), egui::Sense::hover());
    let cell = rect.width() / frames as f32;
    let (done, now, left) = (
        ACCENT.gamma_multiply(0.5),
        ACCENT,
        egui::Color32::from_gray(58),
    );
    let span = |from: u32, to: u32| {
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + from as f32 * cell, rect.top()),
            egui::pos2(rect.left() + to as f32 * cell, rect.bottom()),
        )
    };
    let painter = ui.painter();
    if cell < 4. {
        painter.rect_filled(span(0, current - 1), 0., done);
        painter.rect_filled(
            span(current - 1, current).expand2(egui::vec2(0.5, 0.)),
            0.,
            now,
        );
        painter.rect_filled(span(current, frames), 0., left);
        return;
    }
    for i in 0..frames {
        let color = match (i + 1).cmp(&current) {
            std::cmp::Ordering::Less => done,
            std::cmp::Ordering::Equal => now,
            std::cmp::Ordering::Greater => left,
        };
        let mut r = span(i, i + 1);
        r.max.x -= 1.;
        painter.rect_filled(r, 1., color);
    }
}
fn clock(s: u64) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
