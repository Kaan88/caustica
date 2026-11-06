//! The Look and Export tabs.
use super::*;

impl App {
    pub(super) fn look_tab(&mut self, ui: &mut egui::Ui) {
        let hdr_text = hdr_line(self);
        let g = self.gpu.as_ref().unwrap();
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        section(ui, "Image");
        grid("look").show(ui, |ui| {
            row(ui, "View", "What the viewport shows (keys 1–8).", |ui| {
                egui::ComboBox::from_id_salt("view")
                    .selected_text(VIEWS[s.view as usize])
                    .show_ui(ui, |ui| {
                        for (i, n) in VIEWS.iter().enumerate() {
                            ui.selectable_value(&mut s.view, i as u32, format!("{n}  ({})", i + 1));
                        }
                    });
            });
            row(
                ui,
                "Exposure",
                "Brightness in photographic stops; +1 EV doubles it.",
                |ui| {
                    let mut ev = if s.exposure > 0. {
                        s.exposure.log2()
                    } else {
                        -6.
                    };
                    let r = ui.add(
                        crate::slider(&mut ev, -6. ..=3.32)
                            .suffix(" EV")
                            .fixed_decimals(1),
                    );
                    // EV is a local, so the double-click reset is done here.
                    if crate::slider_double_clicked(ui, &r) {
                        s.exposure = self.reset_baseline.exposure;
                    } else if r.changed() {
                        s.exposure = 2f32.powf(ev).clamp(0., 10.);
                    }
                },
            );
            row(
                ui,
                "Tone map",
                "How bright values are compressed. In HDR the curve keeps its SDR look and its highlights expand into the panel's headroom; Linear uses a plain HDR roll-off.",
                |ui| {
                    egui::ComboBox::from_id_salt("tone map")
                        .selected_text(TONE_MAPS[s.tonemap as usize])
                        .show_ui(ui, |ui| {
                            for (i, n) in TONE_MAPS.iter().enumerate() {
                                ui.selectable_value(&mut s.tonemap, i as u32, *n);
                            }
                        });
                },
            );
        });
        more(ui, "look more", advanced, |ui| {
            grid("look advanced").show(ui, |ui| {
                row(ui, "Display gamma", "Display gamma for the SDR image.", |ui| {
                    ui.add(crate::slider(&mut s.gamma, 0.1..=4.));
                });
                row(ui, "Direct light", "Relight: scale light that reached the surface straight from a light. Approximate on denoised output.", |ui| {
                    ui.add(crate::slider(&mut s.direct, 0. ..=4.).suffix("×"));
                });
                row(ui, "Indirect light", "Relight: scale light that bounced first. Approximate on denoised output.", |ui| {
                    ui.add(crate::slider(&mut s.indirect, 0. ..=4.).suffix("×"));
                });
            });
        });
        section(ui, "Film");
        grid("film").show(ui, |ui| {
            row(
                ui,
                "Halation",
                "Red-orange glow around highlights, as light scatters back through film.",
                |ui| {
                    ui.add(crate::slider(&mut s.halation, 0. ..=1.));
                },
            );
            row(
                ui,
                "Glare",
                "Diffraction starbursts around highlights: the aperture's shape (Lens: aperture blades) diffracts light into spikes, coloured because red spreads wider than blue. An FFT convolution of the highlights.",
                |ui| {
                    ui.add(crate::slider(&mut s.glare, 0. ..=4.).logarithmic(true).smallest_positive(0.01));
                },
            );
            if s.glare > 0. {
                row(ui, "Glare size", "How far the spikes and rings reach.", |ui| {
                    ui.add(crate::slider(&mut s.glare_size, 0. ..=1.));
                });
                row(ui, "Glare threshold", "Only light brighter than this (luminance, before exposure) glares.", |ui| {
                    ui.add(crate::slider(&mut s.glare_threshold, 0. ..=100.).logarithmic(true).smallest_positive(0.01));
                });
            }
            row(
                ui,
                "Film stock",
                "Sets grain, grain size and halation to resemble a film stock (rough matches by eye: the grain is monochrome and the colour response is not modelled).",
                |ui| {
                    let current = FILM_STOCKS
                        .iter()
                        .find(|(_, g, z, h)| (s.grain - g).abs() < 1e-4 && s.grain_size == *z && (s.halation - h).abs() < 1e-4)
                        .map_or("Custom", |(n, ..)| *n);
                    egui::ComboBox::from_id_salt("film stock")
                        .selected_text(current)
                        .width(170.)
                        .show_ui(ui, |ui| {
                            for (name, grain, size, halation) in FILM_STOCKS {
                                if ui.selectable_label(current == *name, *name).clicked() {
                                    s.grain = *grain;
                                    s.grain_size = *size;
                                    s.halation = *halation;
                                }
                            }
                        });
                },
            );
            row(
                ui,
                "Grain",
                "Film grain, strongest in the midtones.",
                |ui| {
                    ui.add(crate::slider(&mut s.grain, 0. ..=1.));
                },
            );
            row(ui, "Grain size", "How coarse the grain is.", |ui| {
                egui::ComboBox::from_id_salt("grain size")
                    .selected_text(["Fine", "Medium", "Coarse"][s.grain_size.min(2) as usize])
                    .width(80.)
                    .show_ui(ui, |ui| {
                        for (i, n) in ["Fine", "Medium", "Coarse"].iter().enumerate() {
                            ui.selectable_value(&mut s.grain_size, i as u32, *n);
                        }
                    });
            });
            row(ui, "Vignette", "Darkens toward the corners.", |ui| {
                ui.add(crate::slider(&mut s.vignette, 0. ..=1.));
            });
        });
        section(ui, "Grade");
        ui.horizontal(|ui| {
            crate::look::color_wheel(ui, "Lift", &mut s.lift, 0., 0.15);
            crate::look::color_wheel(ui, "Gamma", &mut s.grade_gamma, 1., 0.5);
            crate::look::color_wheel(ui, "Gain", &mut s.gain, 1., 0.5);
        });
        note(
            ui,
            "Drag a wheel to tint, the slider for the level; double-click either to reset.",
        );
        grid("lut").show(ui, |ui| {
            row(
                ui,
                "LUT",
                "A .cube 3D LUT applied after the grade, on the display-encoded image.",
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut s.lut_path)
                            .hint_text(field_hint("path to .cube"))
                            .desired_width(170.),
                    );
                    if ui.small_button("Browse…").clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter("3D LUT", &["cube"])
                            .pick_file()
                    {
                        s.lut_path = crate::mesh::store(&p);
                    }
                    if !s.lut_path.is_empty() && ui.small_button("Clear").clicked() {
                        s.lut_path.clear();
                    }
                },
            );
            // A row of its own: this one already fills the panel.
            if !s.lut_path.is_empty()
                && let Some(e) = &self.ui.lut_error
            {
                row(ui, "", "", |ui| cant_load(ui, e));
            }
            if !s.lut_path.is_empty() {
                row(
                    ui,
                    "LUT strength",
                    "Blend between the graded image and the LUT result.",
                    |ui| {
                        ui.add(crate::slider(&mut s.lut_strength, 0. ..=1.));
                    },
                );
            }
        });
        section(ui, "Display");
        grid("display").show(ui, |ui| {
            row(ui, "Output", if cfg!(windows) { "Auto shows real HDR when Windows HDR is on for this monitor, using Windows' calibration and SDR content brightness." } else { "Auto shows real HDR when the desktop has HDR on for this monitor (Wayland colour management), using its brightness settings." }, |ui| {
                let outputs = [if cfg!(windows) { "Auto (follow Windows)" } else { "Auto (follow the desktop)" }, "SDR", "HDR (forced)"];
                egui::ComboBox::from_id_salt("output").selected_text(outputs[s.hdr_mode as usize]).show_ui(ui, |ui| {
                    for (i, n) in outputs.iter().enumerate() {
                        ui.selectable_value(&mut s.hdr_mode, i as u32, *n);
                    }
                });
            });
            if g.hdr_active || s.hdr_mode != 1 {
                row(ui, "Swapchain", "scRGB: 16-bit float, the desktop converts it for the panel. HDR10: 10-bit PQ in BT.2020, sent to the panel as it is (for comparison).", |ui| {
                    ui.selectable_value(&mut s.hdr10, false, "scRGB (FP16)");
                    ui.selectable_value(&mut s.hdr10, true, "HDR10 (PQ)");
                });
            }
            if g.hdr_active {
                row(ui, "Peak brightness", if cfg!(windows) { "From Windows HDR Calibration, or set by hand." } else { "From the desktop's display settings, or set by hand." }, |ui| {
                    let mut windows = s.hdr_peak == 0.;
                    if ui.checkbox(&mut windows, if cfg!(windows) { "Windows" } else { "Desktop" }).changed() {
                        s.hdr_peak = if windows { 0. } else { g.hdr_levels[1].clamp(80., 10000.) };
                    }
                    if !windows {
                        ui.add(crate::slider(&mut s.hdr_peak, 80. ..=4000.).logarithmic(true).suffix(" nits"));
                    }
                });
                row(ui, "Paper white", if cfg!(windows) { "Brightness of white; Windows' SDR content brightness by default." } else { "Brightness of white; the desktop's SDR brightness by default." }, |ui| {
                    let mut windows = s.hdr_white == 0.;
                    if ui.checkbox(&mut windows, if cfg!(windows) { "Windows" } else { "Desktop" }).changed() {
                        s.hdr_white = if windows { 0. } else { g.hdr_levels[0].clamp(crate::scene::HDR_WHITE_MIN, 1000.) };
                    }
                    if !windows {
                        ui.add(crate::slider(&mut s.hdr_white, crate::scene::HDR_WHITE_MIN..=1000.).logarithmic(true).suffix(" nits"));
                    }
                });
            }
        });
        note(ui, &hdr_text);
        if g.hdr_active {
            note(
                ui,
                "In HDR, midtones look as in SDR and the tone map's highlights expand up to the panel peak. Linear shows true scene brightness with a roll-off near the peak.",
            );
        }
        section(ui, "Tools");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.ui.zebras, "Zebras")
                .on_hover_text("Stripes where the displayed image clips");
            ui.checkbox(&mut self.ui.show_scopes, "Scopes")
                .on_hover_text("Histogram, luma waveform and vectorscope");
            ui.checkbox(&mut self.ui.meter, "Light meter")
                .on_hover_text("Luminance and illuminance under the cursor");
            ui.checkbox(&mut self.ui.show_graph, "Convergence graph")
                .on_hover_text("Error against a reference image as samples grow");
            ui.checkbox(&mut self.ui.paths.open, "Path visualizer (P)")
                .on_hover_text("Click a pixel to see its light paths in 3D");
        });
        section(ui, "Light path expressions");
        lpe_section(ui, s, &mut self.ui.lpe_edit);
        section(ui, "Compare");
        let cpu = self
            .cpu_reference
            .as_ref()
            .filter(|l| !l.view)
            .map(|l| l.state());
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Freeze reference (C)")
                .on_hover_text("Keep the current image to compare against")
                .clicked()
            {
                self.actions.reference = true;
            }
            let label = if cpu.is_some() {
                "Stop CPU reference"
            } else {
                "Render reference on CPU"
            };
            if ui
                .add_enabled(!self.cpu_view, egui::Button::new(label))
                .on_hover_text(
                    "Path-trace the scene on the CPU (an independent renderer of the same model) and show it left of the split, refining as it goes. Edits and camera moves start it over.",
                )
                .on_disabled_hover_text("The viewport is already showing the CPU's render")
                .clicked()
            {
                self.actions.cpu_reference = Some(cpu.is_none());
            }
            ui.add_enabled(
                g.reference_valid && !self.cpu_view,
                egui::Checkbox::new(&mut s.compare, "Show split"),
            )
            .on_disabled_hover_text("Freeze a reference (C) or render one on CPU first");
        });
        if let Some(st) = &cpu {
            match &st.error {
                Some(e) => note(ui, e),
                None if st.samples == 0 => note(ui, "CPU reference: first pass…"),
                None => note(
                    ui,
                    &format!(
                        "CPU reference: {} / {} spp, {:.0} s{}",
                        st.samples,
                        st.cap,
                        st.seconds,
                        if st.samples >= st.cap { ", done" } else { "" }
                    ),
                ),
            }
        }
        if s.compare && !self.cpu_view {
            let what = if cpu.is_some() {
                "Left of the line shows the CPU reference."
            } else {
                "Left of the line shows the frozen reference."
            };
            grid("compare").show(ui, |ui| {
                row(ui, "Split", what, |ui| {
                    ui.add(
                        crate::slider(&mut s.split, 0. ..=1.)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.)),
                    );
                });
            });
        }
    }
    pub(super) fn export_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        use crate::video::ImageFormat;
        section(ui, "Save the current image");
        grid("export_name").show(ui, |ui| {
            row(
                ui,
                "File name",
                "Of quick saves and high-resolution stills (which add -high-res), in the folder named below. {scene}, {date}, {time}, {spp}, {w} and {h} are filled in.",
                |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.prefs.name_template)
                            .desired_width(220.),
                    );
                    if r.lost_focus() {
                        self.ui.prefs.save();
                    }
                },
            );
        });
        // The software view saves the CPU's image; stills and videos below render on the GPU.
        let samples = self
            .cpu_reference
            .as_ref()
            .filter(|l| l.view)
            .map_or(g.samples, |l| l.state().samples);
        let stem = self
            .ui
            .export_stem(&self.settings, samples, g.width, g.height);
        note(
            ui,
            &format!(
                "Saves the view as it is now: {} samples, {} × {}",
                samples, g.width, g.height
            ),
        );
        if self.cpu_view {
            note(
                ui,
                "These save the CPU's image. High-resolution stills and videos still render on the GPU.",
            );
        }
        let (state, refining) = save_state(
            samples,
            self.settings.max_samples,
            self.paused,
            self.ui.sample_rate.2,
        );
        let state = egui::RichText::new(state).small();
        ui.label(if refining { state.color(WARN) } else { state });
        ui.horizontal_wrapped(|ui| {
            for format in ImageFormat::ALL {
                if format.needs_hdr() && !g.hdr_active {
                    continue;
                }
                let label = if format == ImageFormat::Png {
                    "PNG (F12)".to_string()
                } else {
                    format.label().to_string()
                };
                let file = format.file(&stem);
                if ui
                    .button(label)
                    .on_hover_text(format!("{}: {}", file.display(), format.description()))
                    .clicked()
                {
                    self.actions.save_image = Some(format);
                }
            }
        });
        grid("export_quality").show(ui, |ui| {
            row(
                ui,
                "JPEG quality",
                "Used by every JPEG save: 100 is the largest file and the fewest artefacts.",
                |ui| {
                    ui.add(crate::slider(&mut self.ui.jpeg_quality, 50..=100));
                },
            );
        });
        // The folder as a real path: the output folder is empty in development and portable runs.
        let dir = crate::dirs::output_dir();
        let folder = if dir.as_os_str().is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            std::path::absolute(&dir).unwrap_or(dir)
        };
        ui.label(
            egui::RichText::new(format!("Saves as {stem}.png in {}", folder.display())).small(),
        );
        if ui
            .button("Open folder")
            .on_hover_text("Show the folder images are saved to")
            .clicked()
        {
            let _ = std::fs::create_dir_all(&folder);
            show_in_folder(&folder);
        }
        if crate::video::name_repeats(&self.ui.prefs.name_template) {
            ui.label(
                egui::RichText::new(
                    "Saving again overwrites the last file: add {time} to the name to keep both.",
                )
                .small()
                .color(WARN),
            );
        }
        section(ui, "High-resolution render");
        note(
            ui,
            "Renders the same camera again from scratch, at the size and samples set here; the view keeps its own progress.",
        );
        grid("export").show(ui, |ui| {
            row(
                ui,
                "Size",
                "Multiple of the window size, up to 8192 pixels on the long side.",
                |ui| {
                    ui.add(crate::slider(&mut self.ui.export_scale, 1..=6).suffix("×"));
                },
            );
            row(ui, "Samples", "Samples per pixel for the export.", |ui| {
                ui.add(
                    crate::slider(&mut self.ui.export_samples, 16..=65536)
                        .logarithmic(true)
                        .suffix(" spp"),
                );
            });
            row(
                ui,
                "Format",
                "The same formats as Save the current image.",
                |ui| {
                    egui::ComboBox::from_id_salt("export_format")
                        .selected_text(self.ui.export_format.label())
                        .show_ui(ui, |ui| {
                            for format in ImageFormat::ALL {
                                ui.add_enabled_ui(!format.needs_hdr() || g.hdr_active, |ui| {
                                    ui.selectable_value(
                                        &mut self.ui.export_format,
                                        format,
                                        format.label(),
                                    )
                                    .on_hover_text(format.description())
                                    .on_disabled_hover_text("Needs HDR output");
                                });
                            }
                        });
                },
            );
            if self.ui.export_format == ImageFormat::Jpeg {
                row(
                    ui,
                    "JPEG quality",
                    "Used by every JPEG save: 100 is the largest file and the fewest artefacts.",
                    |ui| {
                        ui.add(crate::slider(&mut self.ui.jpeg_quality, 50..=100));
                    },
                );
            }
        });
        // HDR output may have been turned off since the format was picked.
        if self.ui.export_format.needs_hdr() && !g.hdr_active {
            self.ui.export_format = ImageFormat::Png;
        }
        let factor = (self.ui.export_scale as f32).min(8192. / g.width.max(g.height) as f32);
        let name = {
            let (w, h) = (
                (g.width as f32 * factor).round() as u32,
                (g.height as f32 * factor).round() as u32,
            );
            let stem = self
                .ui
                .export_stem(&self.settings, self.ui.export_samples, w, h);
            self.ui.export_format.file(&format!("{stem}-high-res"))
        };
        if ui.button("Render high-resolution image").clicked() {
            self.actions.high_res = true;
        }
        note(
            ui,
            &format!(
                "{} ({}), {} × {}, traced fresh at 100% resolution. The window stays responsive, with progress and Cancel.",
                name.display(),
                self.ui.export_format.description().trim_end_matches('.'),
                (g.width as f32 * factor).round(),
                (g.height as f32 * factor).round()
            ),
        );
        section(ui, "Video");
        let o = &self.ui.video;
        let [w, h] = o.size();
        let animation = &self.settings.animation;
        // As the export dialog decides: keys set the rate and the length.
        let text = if o.timeline && !animation.is_empty() {
            format!(
                "Plays the timeline: {} at {w}×{h}, {:.2} s at {} fps, {} spp each.",
                o.format.label(),
                animation.length,
                animation.fps,
                o.spp
            )
        } else if o.timeline && self.settings.physics.enabled {
            format!(
                "Runs the simulation: {} at {w}×{h}, {} frames at {} fps, {} spp each.",
                o.format.label(),
                o.frames,
                o.fps,
                o.spp
            )
        } else {
            format!(
                "An orbit around the target: {} at {w}×{h}, {} frames at {} fps, {} spp each.",
                o.format.label(),
                o.frames,
                o.fps,
                o.spp
            )
        };
        note(ui, &text);
        if ui.button("Export video…").clicked() {
            self.open_video_dialog();
        }
        section(ui, "Scene file");
        ui.horizontal(|ui| {
            if ui.button("Save settings.json").clicked() {
                self.actions.save = Some(crate::dirs::output("settings.json"));
            }
            if ui.button("Save as…").clicked() {
                self.actions.save = settings_dialog(true);
            }
            if ui.button("Load…").clicked() {
                self.actions.load = settings_dialog(false);
            }
        });
    }
}
/// The state of the image a quick save would write, as the status bar judges it, and whether
/// it is still getting cleaner. `rate` is in samples per second, 0 until it is known.
fn save_state(samples: u32, cap: u32, paused: bool, rate: f32) -> (String, bool) {
    if samples >= cap {
        ("Converged".into(), false)
    } else if paused {
        (format!("Paused at {samples} of {cap} samples"), false)
    } else if rate > 0. {
        let left = (cap - samples) as f32 / rate;
        (
            format!("Still refining, {} to the cap", time_left(left)),
            true,
        )
    } else {
        ("Still refining".into(), true)
    }
}
#[cfg(test)]
mod tests {
    use super::save_state;
    #[test]
    fn a_quick_save_says_what_state_the_image_is_in() {
        assert_eq!(
            save_state(4096, 4096, false, 50.),
            ("Converged".into(), false)
        );
        assert_eq!(
            save_state(4096, 4096, true, 0.),
            ("Converged".into(), false)
        );
        assert_eq!(
            save_state(20, 4096, true, 50.),
            ("Paused at 20 of 4096 samples".into(), false)
        );
        assert_eq!(
            save_state(20, 4096, false, 0.),
            ("Still refining".into(), true)
        );
        assert_eq!(
            save_state(96, 4096, false, 100.),
            ("Still refining, ~40 s to the cap".into(), true)
        );
    }
}
