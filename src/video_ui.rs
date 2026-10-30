//! The video export dialog and its progress window (item 6). The options live in
//! `UiState::video`; `redraw` starts the `video::Job` and renders it a pass at a time.
use crate::App;
use crate::ui::{grid, note, row, section};
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
        if animation.is_empty() {
            self.ui.video.timeline = false;
        }
        if self.ui.video.timeline {
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
                                    let r = ui.add_enabled(usable, egui::SelectableLabel::new(o.format == f, f.label()));
                                    let r = if f == Format::Hdr10 && !hdr {
                                        r.on_disabled_hover_text("Needs HDR output (Render tab, Display).")
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
                    let timed = !o.timeline;
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
                        ui.add_enabled_ui(!animation.is_empty(), |ui| {
                            ui.selectable_value(&mut o.timeline, true, "Timeline")
                                .on_disabled_hover_text("The scene has no keyframes (I keys the selection).");
                        });
                    });
                });
                if o.timeline {
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
    /// Drawn instead of the whole UI while an export runs.
    pub fn export_window(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.export else {
            return;
        };
        if let Some(image) = job.preview.take() {
            match &mut self.ui.video_preview {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.ui.video_preview =
                        Some(ctx.load_texture("video-preview", image, egui::TextureOptions::LINEAR))
                }
            }
        }
        let samples = self.gpu.as_ref().map_or(0, |g| g.samples);
        let o = &job.options;
        let [w, h] = o.size();
        let progress = job.progress(samples);
        let mut cancel = false;
        egui::Window::new(if o.still {
            "Rendering image"
        } else {
            "Exporting video"
        })
        .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.set_width(380.);
            if let Some(t) = &self.ui.video_preview {
                let size = t.size_vec2();
                ui.image((t.id(), size * (380. / size.x)));
            } else {
                ui.add_sized([380., 380. * h as f32 / w as f32], egui::Spinner::new());
            }
            ui.add(egui::ProgressBar::new(progress).show_percentage());
            ui.label(format!(
                "Frame {} of {} · {} / {} spp",
                (job.frame + 1).min(o.frames),
                o.frames,
                samples.min(job.settings().max_samples),
                job.settings().max_samples
            ));
            let elapsed = job.started.elapsed().as_secs();
            let left = job.remaining().map_or("estimating".into(), |d| {
                format!("about {} left", clock(d.as_secs()))
            });
            note(
                ui,
                &format!(
                    "{} · {w}×{h} · {} elapsed, {left}",
                    o.format.label(),
                    clock(elapsed)
                ),
            );
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if cancel {
            self.actions.cancel_export = true;
        }
    }
}
fn clock(s: u64) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
