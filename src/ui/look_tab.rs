//! The Look and Export tabs.
use super::*;

/// A starting point for the Look tab: the fields it writes are the tone map, the film fields
/// and lift / gamma / gain, nothing else (not exposure, glare, the LUT or the grade's
/// neighbours). The selection is derived from the current values, never stored.
pub(super) struct Look {
    pub name: &'static str,
    about: &'static str,
    tonemap: u32,
    halation: f32,
    grain: f32,
    grain_size: u32,
    vignette: f32,
    lift: [f32; 3],
    gamma: [f32; 3],
    gain: [f32; 3],
}
/// Neutral is the default state of every field it writes. The tone maps are indices into
/// `TONE_MAPS` (1 Reinhard, 3 ACES filmic, 5 High contrast, 6 AgX). Changeable by editing
/// this table.
pub(super) const LOOKS: &[Look] = &[
    Look {
        name: "Neutral",
        about: "Plain ACES filmic, no film effects, no grade.",
        tonemap: 3,
        halation: 0.,
        grain: 0.,
        grain_size: 1,
        vignette: 0.,
        lift: [0.; 3],
        gamma: [1.; 3],
        gain: [1.; 3],
    },
    Look {
        name: "Warm film",
        about: "AgX with a little halation, medium grain, a soft vignette and an amber cast.",
        tonemap: 6,
        halation: 0.2,
        grain: 0.2,
        grain_size: 1,
        vignette: 0.2,
        lift: [0.01, 0.005, 0.],
        gamma: [1.04, 1., 0.94],
        gain: [1.06, 1., 0.92],
    },
    Look {
        name: "Cool",
        about: "AgX with fine grain and a blue cast in the shadows and highlights.",
        tonemap: 6,
        halation: 0.,
        grain: 0.05,
        grain_size: 0,
        vignette: 0.1,
        lift: [0., 0.005, 0.015],
        gamma: [0.96, 1., 1.05],
        gain: [0.94, 1., 1.08],
    },
    Look {
        name: "Punchy",
        about: "High-contrast curve, darker midtones, brighter highlights, a light vignette.",
        tonemap: 5,
        halation: 0.,
        grain: 0.,
        grain_size: 1,
        vignette: 0.15,
        lift: [0.; 3],
        gamma: [0.92; 3],
        gain: [1.08; 3],
    },
    Look {
        name: "Soft",
        about: "Reinhard with glowing highlights, lifted blacks and flatter contrast.",
        tonemap: 1,
        halation: 0.35,
        grain: 0.,
        grain_size: 1,
        vignette: 0.1,
        lift: [0.03; 3],
        gamma: [1.05; 3],
        gain: [0.97; 3],
    },
];
impl Look {
    /// The bundle the settings match, if any (floats to the precision of a slider drag).
    pub(super) fn of(s: &Settings) -> Option<&'static Look> {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let near3 = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| near(*a, b));
        LOOKS.iter().find(|l| {
            s.tonemap == l.tonemap
                && near(s.halation, l.halation)
                && near(s.grain, l.grain)
                && s.grain_size == l.grain_size
                && near(s.vignette, l.vignette)
                && near3(s.lift, l.lift)
                && near3(s.grade_gamma, l.gamma)
                && near3(s.gain, l.gain)
        })
    }
    pub(super) fn apply(&self, s: &mut Settings) {
        s.tonemap = self.tonemap;
        s.halation = self.halation;
        s.grain = self.grain;
        s.grain_size = self.grain_size;
        s.vignette = self.vignette;
        s.lift = self.lift;
        s.grade_gamma = self.gamma;
        s.gain = self.gain;
    }
}

impl App {
    pub(super) fn look_tab(&mut self, ui: &mut egui::Ui) {
        let hdr_text = hdr_line(self);
        let g = self.gpu.as_ref().unwrap();
        let metered = g.metered;
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        // The look preset sits on the tab's title.
        tab_title_with(ui, "Look", |ui| {
            let current = Look::of(s);
            let r = crate::ui::skin::ComboBox::from_id_salt("look bundle")
                .width(140.)
                .selected_text(current.map_or("Custom", |l| l.name))
                .show_ui(ui, |ui| {
                    for l in LOOKS {
                        if crate::ui::skin::selectable(
                            ui,
                            current.is_some_and(|c| c.name == l.name),
                            l.name,
                        )
                        .on_hover_text(l.about)
                        .clicked()
                        {
                            l.apply(s);
                        }
                    }
                })
                .response;
            r.on_hover_text("A starting point: sets the tone map, the film fields (halation, grain, vignette) and lift / gamma / gain together. Exposure, glare, the LUT and everything else stay as they are. Tweak any field after and this reads Custom.");
        });
        let preview = if s.auto_exposure {
            format!("Auto · {:+.1} EV", s.ev_bias)
        } else {
            // As the Exposure slider shows it.
            let ev = if s.exposure > 0. {
                s.exposure.log2().clamp(EXPOSURE_EV_MIN, EXPOSURE_EV_MAX)
            } else {
                EXPOSURE_EV_MIN
            };
            format!("Manual · {ev:+.1} EV")
        };
        fold(ui, "Exposure", preview, true, |ui| {
            grid("look").show(ui, |ui| {
            row(
                ui,
                "Auto exposure",
                "Like a camera or an eye: the meter picks the exposure from what the view shows and adapts over time, so a noon beach and a moonlit field both read well. Off, the exposure is the fixed value below, starting from what the meter showed.",
                |ui| {
                    crate::ui::skin::checkbox_label(ui, &mut s.auto_exposure, "");
                    // AE-L: auto off; the frame loop copies the meter's exposure into the manual
                    // one, as for the checkbox (E7b).
                    if s.auto_exposure
                        && crate::ui::skin::small_button(ui, "AE-L")
                            .on_hover_text("Lock the exposure: keep what the meter shows now as the manual exposure and turn auto off.")
                            .clicked()
                    {
                        s.auto_exposure = false;
                    }
                },
            );
            if s.auto_exposure {
                row(
                    ui,
                    "Exposure bias",
                    "Brighter or darker than the meter's choice, in photographic stops; +1 EV doubles it.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut s.ev_bias, -EV_BIAS_MAX..=EV_BIAS_MAX)
                                .suffix(" EV")
                                .fixed_decimals(1),
                        );
                    },
                );
                row(
                    ui,
                    "Meter",
                    "What the meter reads: the frame's log-average weighted to the centre, a spot at the centre, or the histogram's median. The brightest 3% (lamps, the sun), the darkest 1% and unlit black are ignored. Shown: the metered exposure now and where it is heading.",
                    |ui| {
                        // The readout keeps its width and the combo takes what is left.
                        let readout = metered.map(|m| meter_readout(&m));
                        let beside = readout
                            .as_ref()
                            .map_or(0., |t| text_width(ui, t, skin::sans(11.5)) + 8.);
                        crate::ui::skin::ComboBox::from_id_salt("meter mode")
                            .width((ui.available_width() - beside).max(72.))
                            .selected_text(s.meter_mode.label())
                            .show_ui(ui, |ui| {
                                for m in MeterMode::ALL {
                                    crate::ui::skin::selectable_value(ui, &mut s.meter_mode, m, m.label());
                                }
                            });
                        if let Some(t) = readout {
                            ui.label(egui::RichText::new(t).small().color(skin::FAINT));
                        }
                    },
                );
                row(
                    ui,
                    "Adaptation",
                    "How fast the exposure follows a change of brightness, per second. Higher is faster.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut s.exposure_speed, 0.05..=50.)
                                .logarithmic(true)
                                .fixed_decimals(2),
                        );
                    },
                );
            } else {
                row(
                    ui,
                    "Exposure",
                    "Brightness in photographic stops; +1 EV doubles it. Down to -8 EV for a bright day and up to +30 EV for a night sky.",
                    |ui| {
                        let mut ev = if s.exposure > 0. {
                            s.exposure.log2().clamp(EXPOSURE_EV_MIN, EXPOSURE_EV_MAX)
                        } else {
                            EXPOSURE_EV_MIN
                        };
                        let r = ui.add(
                            crate::slider(&mut ev, EXPOSURE_EV_MIN..=EXPOSURE_EV_MAX)
                                .suffix(" EV")
                                .fixed_decimals(1),
                        );
                        // EV is a local, so the double-click reset is done here.
                        if crate::slider_double_clicked(ui, &r) {
                            s.exposure = self.reset_baseline.exposure;
                        } else if r.changed() {
                            s.exposure = 2f32.powf(ev).clamp(0., EXPOSURE_MAX);
                        }
                    },
                );
            }
            row(
                ui,
                "Eyes",
                "Dark scenes look the way the eye sees them: colour drains away and blues stand out as the light fades.",
                |ui| {
                    crate::ui::skin::checkbox_label(ui, &mut s.eyes, "");
                },
            );
            if s.eyes {
                row(ui, "Eyes strength", "How much of the effect to apply.", |ui| {
                    ui.add(crate::slider(&mut s.eyes_strength, 0. ..=1.).fixed_decimals(2));
                });
            }
        });
        });
        fold(ui, "Tone", TONE_MAPS[s.tonemap as usize], true, |ui| {
            grid("tone").show(ui, |ui| {
            row(
                ui,
                "Tone map",
                "How bright values are compressed. In HDR the curve keeps its SDR look and its highlights expand into the panel's headroom; Linear uses a plain HDR roll-off.",
                |ui| {
                    crate::ui::skin::ComboBox::from_id_salt("tone map")
                        .selected_text(TONE_MAPS[s.tonemap as usize])
                        .show_ui(ui, |ui| {
                            for (i, n) in TONE_MAPS.iter().enumerate() {
                                crate::ui::skin::selectable_value(ui, &mut s.tonemap, i as u32, *n);
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
        });
        let preview = grade_preview(s, self.ui.lut_error.is_some());
        fold(ui, "Grade", preview, true, |ui| {
            crate::look::color_wheels(
                ui,
                &mut [
                    crate::look::Wheel {
                        label: "Lift",
                        value: &mut s.lift,
                        center: 0.,
                        range: 0.15,
                    },
                    crate::look::Wheel {
                        label: "Gamma",
                        value: &mut s.grade_gamma,
                        center: 1.,
                        range: 0.5,
                    },
                    crate::look::Wheel {
                        label: "Gain",
                        value: &mut s.gain,
                        center: 1.,
                        range: 0.5,
                    },
                ],
            );
            note(
                ui,
                "Drag a wheel to tint, the slider for the level; double-click either to reset.",
            );
            // A 3D LUT is an expert tool: hidden until Show advanced, or while one is loaded.
            grid("lut").show(ui, |ui| {
                if advanced || !s.lut_path.is_empty() {
                    row(
                        ui,
                        "LUT",
                        "A .cube 3D LUT applied after the grade, on the display-encoded image.",
                        |ui| {
                            // The buttons keep their width and the field takes what is left.
                            let buttons = button_width(ui, "Browse…")
                                + 8.
                                + if s.lut_path.is_empty() {
                                    0.
                                } else {
                                    button_width(ui, "Clear") + 8.
                                };
                            crate::ui::skin::text_edit(
                                ui,
                                (ui.available_width() - buttons).max(48.),
                                egui::TextEdit::singleline(&mut s.lut_path)
                                    .hint_text(field_hint("path to .cube")),
                            );
                            if crate::ui::skin::small_button(ui, "Browse…").clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .add_filter("3D LUT", &["cube"])
                                    .pick_file()
                            {
                                s.lut_path = crate::mesh::store(&p);
                            }
                            if !s.lut_path.is_empty()
                                && crate::ui::skin::small_button(ui, "Clear").clicked()
                            {
                                s.lut_path.clear();
                            }
                        },
                    );
                }
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
        });
        fold(ui, "Film", film_preview(s), false, |ui| {
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
            if s.glare > 0. && (advanced || s.glare_size != 0.5 || s.glare_threshold != 1.) {
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
                    crate::ui::skin::ComboBox::from_id_salt("film stock")
                        .selected_text(current)
                        .width(170_f32.min(ui.available_width()))
                        .show_ui(ui, |ui| {
                            for (name, grain, size, halation) in FILM_STOCKS {
                                if crate::ui::skin::selectable(ui, current == *name, *name).clicked() {
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
            if advanced || s.grain > 0. {
                row(ui, "Grain size", "How coarse the grain is.", |ui| {
                    crate::ui::skin::ComboBox::from_id_salt("grain size")
                        .selected_text(["Fine", "Medium", "Coarse"][s.grain_size.min(2) as usize])
                        .width(80.)
                        .show_ui(ui, |ui| {
                            for (i, n) in ["Fine", "Medium", "Coarse"].iter().enumerate() {
                                crate::ui::skin::selectable_value(ui, &mut s.grain_size, i as u32, *n);
                            }
                        });
                });
            }
            row(ui, "Vignette", "Darkens toward the corners.", |ui| {
                ui.add(crate::slider(&mut s.vignette, 0. ..=1.));
            });
        });
        });
        // The split shows only with a reference to show, and not over the CPU view.
        let preview = if s.compare && g.reference_valid && !self.cpu_view {
            format!("split {:.0}%", s.split * 100.)
        } else {
            "off".to_string()
        };
        fold(ui, "Compare", preview, true, |ui| {
            let cpu = self
                .cpu_reference
                .as_ref()
                .filter(|l| !l.view)
                .map(|l| l.state());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.y = 4.;
                if crate::ui::skin::button(ui, "Freeze reference (C)")
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
                    .add_enabled(!self.cpu_view, crate::ui::skin::Button::new(label))
                    .on_hover_text(
                        "Path-trace the scene on the CPU (an independent renderer of the same model) and show it left of the split, refining as it goes. Edits and camera moves start it over.",
                    )
                    .on_disabled_hover_text("The viewport is already showing the CPU's render")
                    .clicked()
                {
                    self.actions.cpu_reference = Some(cpu.is_none());
                }
            });
            ui.add_space(4.);
            let can_split = g.reference_valid && !self.cpu_view;
            grid("compare").show(ui, |ui| {
                check_row_with(
                    ui,
                    "Show split",
                    "",
                    &mut s.compare,
                    (!can_split).then_some(if self.cpu_view {
                        "The viewport is showing the CPU's render"
                    } else {
                        "Freeze a reference (C) or render one on CPU first"
                    }),
                    |_, _| {},
                );
                if s.compare && !self.cpu_view {
                    let what = if cpu.is_some() {
                        "Left of the line shows the CPU reference."
                    } else {
                        "Left of the line shows the frozen reference."
                    };
                    row(ui, "Split", what, |ui| {
                        ui.add(
                            crate::slider(&mut s.split, 0. ..=1.)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.)),
                        );
                    });
                }
            });
            if let Some(st) = &cpu {
                match &st.error {
                    Some(e) => {
                        ui.label(egui::RichText::new(e).small().color(WARN));
                    }
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
        });
        let on = [
            (self.ui.zebras, "zebras"),
            (self.ui.show_scopes, "scopes"),
            (self.ui.meter, "meter"),
            (self.ui.show_graph, "graph"),
            (self.ui.paths.open, "paths"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>();
        // "off" as Compare says it, so the closed header is not blank; all five names are
        // wider than the header has room for, so then it counts them.
        let preview = match on.len() {
            0 => "off".to_string(),
            1..=4 => on.join(" · "),
            n => format!("{n} on"),
        };
        fold(ui, "Tools", preview, false, |ui| {
            grid("tools").show(ui, |ui| {
                check_row(
                    ui,
                    "Zebras",
                    "Stripes where the displayed image clips",
                    &mut self.ui.zebras,
                );
                check_row(
                    ui,
                    "Scopes",
                    "Histogram, luma waveform and vectorscope",
                    &mut self.ui.show_scopes,
                );
                check_row(
                    ui,
                    "Light meter",
                    "Luminance and illuminance under the cursor",
                    &mut self.ui.meter,
                );
                check_row(
                    ui,
                    "Convergence graph",
                    "Error against a reference image as samples grow",
                    &mut self.ui.show_graph,
                );
                check_row(
                    ui,
                    "Path visualizer (P)",
                    "Click a pixel to see its light paths in 3D",
                    &mut self.ui.paths.open,
                );
            });
        });
        // An expression in use shows in the header, so it is never hidden unnoticed.
        let lpe_used = s.view == 12 || s.lpe.iter().any(|e| !e.is_empty());
        fold(
            ui,
            "Light path expressions",
            if lpe_used { "on" } else { "off" },
            lpe_used,
            |ui| lpe_section(ui, s, &mut self.ui.lpe_edit),
        );
        let preview = if g.hdr_active {
            format!("HDR {:.0} nits", g.hdr_levels[1])
        } else {
            "SDR".to_string()
        };
        fold(ui, "Display", preview, false, |ui| {
            grid("display").show(ui, |ui| {
            row(ui, "Output", if cfg!(windows) { "Auto shows real HDR when Windows HDR is on for this monitor, using Windows' calibration and SDR content brightness." } else { "Auto shows real HDR when the desktop has HDR on for this monitor (Wayland colour management), using its brightness settings." }, |ui| {
                let outputs = [if cfg!(windows) { "Auto (follow Windows)" } else { "Auto (follow the desktop)" }, "SDR", "HDR (forced)"];
                crate::ui::skin::ComboBox::from_id_salt("output").selected_text(outputs[s.hdr_mode as usize]).show_ui(ui, |ui| {
                    for (i, n) in outputs.iter().enumerate() {
                        crate::ui::skin::selectable_value(ui, &mut s.hdr_mode, i as u32, *n);
                    }
                });
            });
            if (g.hdr_active || s.hdr_mode != 1) && (advanced || s.hdr10) {
                row(ui, "Swapchain", "scRGB: 16-bit float, the desktop converts it for the panel. HDR10: 10-bit PQ in BT.2020, sent to the panel as it is (for comparison).", |ui| {
                    crate::ui::skin::selectable_value(ui, &mut s.hdr10, false, "scRGB (FP16)");
                    crate::ui::skin::selectable_value(ui, &mut s.hdr10, true, "HDR10 (PQ)");
                });
            }
            if g.hdr_active && (advanced || s.hdr_peak != 0. || s.hdr_white != 0.) {
                row(ui, "Peak brightness", if cfg!(windows) { "From Windows HDR Calibration, or set by hand." } else { "From the desktop's display settings, or set by hand." }, |ui| {
                    let mut windows = s.hdr_peak == 0.;
                    if crate::ui::skin::checkbox_label(ui, &mut windows, if cfg!(windows) { "Windows" } else { "Desktop" }).changed() {
                        s.hdr_peak = if windows { 0. } else { g.hdr_levels[1].clamp(80., 10000.) };
                    }
                    if !windows {
                        ui.add(crate::slider(&mut s.hdr_peak, 80. ..=4000.).logarithmic(true).suffix(" nits"));
                    }
                });
                row(ui, "Paper white", if cfg!(windows) { "Brightness of white; Windows' SDR content brightness by default." } else { "Brightness of white; the desktop's SDR brightness by default." }, |ui| {
                    let mut windows = s.hdr_white == 0.;
                    if crate::ui::skin::checkbox_label(ui, &mut windows, if cfg!(windows) { "Windows" } else { "Desktop" }).changed() {
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
        });
    }
    pub(super) fn export_tab(&mut self, ui: &mut egui::Ui) {
        use crate::video::ImageFormat;
        tab_title(ui, "Export");
        // A running export: its card on top, and the rest waits.
        if self.export_card(ui) {
            ui.add_space(6.);
            ui.disable();
        }
        let g = self.gpu.as_ref().unwrap();
        fold(ui, "Save the current image", "F12", true, |ui| {
            grid("export_name").show(ui, |ui| {
            row(
                ui,
                "File name",
                "Of quick saves and high-resolution stills (which add -high-res), in the folder named below. {scene}, {date}, {time}, {spp}, {w} and {h} are filled in.",
                |ui| {
                    let width = 220_f32.min(ui.available_width());
                    let r = crate::ui::skin::text_edit(
                        ui,
                        width,
                        egui::TextEdit::singleline(&mut self.ui.prefs.name_template),
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
            if refining {
                ui.label(egui::RichText::new(state).small().color(WARN));
            } else {
                note(ui, &state);
            }
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
                    if crate::ui::skin::button(ui, label)
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
            note(ui, &format!("Saves as {stem}.png in {}", folder.display()));
            if crate::ui::skin::button(ui, "Open folder")
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
        });
        let preview = format!("{}× · {} spp", self.ui.export_scale, self.ui.export_samples);
        fold(ui, "High-resolution still", preview, true, |ui| {
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
                    crate::ui::skin::ComboBox::from_id_salt("export_format")
                        .selected_text(self.ui.export_format.label())
                        .show_ui(ui, |ui| {
                            for format in ImageFormat::ALL {
                                ui.add_enabled_ui(!format.needs_hdr() || g.hdr_active, |ui| {
                                    crate::ui::skin::selectable_value(ui, &mut self.ui.export_format,
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
            if crate::ui::skin::primary(ui, "Render high-resolution image").clicked() {
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
        });
        let mut open_dialog = false;
        // What the export would play (as the note inside decides) and its frame size.
        let preview = {
            let o = &self.ui.video;
            let [w, h] = o.size();
            let plays = if o.timeline && !self.settings.animation.is_empty() {
                "timeline"
            } else if o.timeline && self.settings.physics.enabled {
                "simulation"
            } else {
                "orbit"
            };
            format!("{plays} · {w}×{h}")
        };
        fold(ui, "Video", preview, true, |ui| {
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
            open_dialog = crate::ui::skin::primary(ui, "Export video…").clicked();
        });
        if open_dialog {
            self.open_video_dialog();
        }
        let recent = self.ui.prefs.recent_exports.clone();
        fold(
            ui,
            "Recent exports",
            recent.len().to_string(),
            false,
            |ui| {
                if recent.is_empty() {
                    note(ui, "Nothing exported yet.");
                }
                for e in &recent {
                    let path = std::path::Path::new(&e.file);
                    let name = path
                        .file_name()
                        .map_or_else(|| e.file.clone(), |n| n.to_string_lossy().into());
                    ui.horizontal(|ui| {
                        if ui
                            .link(name)
                            .on_hover_text(format!("{}\nClick to show it in its folder.", e.file))
                            .clicked()
                        {
                            show_in_folder(path);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(&e.state).small().weak());
                        });
                    });
                }
            },
        );
    }
}
/// The state of the image a quick save would write, as the status bar judges it, and whether
/// it is still getting cleaner. `rate` is in samples per second, 0 until it is known.
/// The meter's state for the Look tab: the metered exposure in stops, and its target while it
/// is still adapting.
fn meter_readout(m: &crate::reference::display::Adaptation) -> String {
    if (m.target - m.current).abs() < 0.05 {
        format!("{:+.1} EV", m.current)
    } else {
        format!("{:+.1} → {:+.1} EV", m.current, m.target)
    }
}
/// A section header's preview from the controls that are off their defaults: each with its
/// value while that stays short, otherwise the names alone, and `none` when there are none.
/// Beside a short title a 330-point column has room for about 34 of the header's monospace
/// characters; `fold` cuts off what is longer.
fn brief(parts: &[(&str, String)], none: &str) -> String {
    if parts.is_empty() {
        return none.into();
    }
    let join = |values: bool| {
        parts
            .iter()
            .map(|(name, value)| {
                if values && !value.is_empty() {
                    format!("{name} {value}")
                } else {
                    name.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let full = join(true);
    if full.chars().count() <= 32 {
        full
    } else {
        join(false)
    }
}
/// The Grade section's header: "Neutral", or the wheels that are off neutral (their level as
/// the wheel's slider shows it, "tint" for a moved dot) and the LUT with its strength.
fn grade_preview(s: &Settings, lut_failed: bool) -> String {
    let mut parts = Vec::new();
    for (name, v, center) in [
        ("Lift", s.lift, 0.),
        ("Gamma", s.grade_gamma, 1.),
        ("Gain", s.gain, 1.),
    ] {
        let mean = (v[0] + v[1] + v[2]) / 3.;
        let tinted = v.iter().any(|c| (c - mean).abs() >= 1e-3);
        let level = mean - center;
        if !tinted && level.abs() < 1e-3 {
            continue;
        }
        let text = match (level.abs() >= 0.005, tinted) {
            (true, true) => format!("{level:+.2} tint"),
            (true, false) => format!("{level:+.2}"),
            (false, true) => "tint".to_string(),
            (false, false) => String::new(),
        };
        parts.push((name, text));
    }
    if !s.lut_path.is_empty() {
        // A LUT that did not load is said even when the values are dropped.
        if lut_failed {
            parts.push(("LUT error", String::new()));
        } else if s.lut_strength < 0.995 {
            parts.push(("LUT", format!("{:.0}%", s.lut_strength * 100.)));
        } else {
            parts.push(("LUT", String::new()));
        }
    }
    brief(&parts, "Neutral")
}
/// The Film section's header: "Off", the effects that are on, or their count when all are.
fn film_preview(s: &Settings) -> String {
    let parts: Vec<_> = [
        ("Halation", s.halation),
        ("Glare", s.glare),
        ("Grain", s.grain),
        ("Vignette", s.vignette),
    ]
    .into_iter()
    .filter(|(_, v)| *v > 0.)
    .map(|(name, v)| (name, format!("{v:.2}")))
    .collect();
    // All four names are a character too wide for the header.
    if parts.len() == 4 {
        return "4 effects".into();
    }
    brief(&parts, "Off")
}
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
    use super::{LOOKS, Look, film_preview, grade_preview, save_state};
    use crate::scene::Settings;
    #[test]
    fn closed_grade_and_film_headers_tell_their_state() {
        let mut s = Settings::default();
        assert_eq!(grade_preview(&s, false), "Neutral");
        assert_eq!(film_preview(&s), "Off");
        s.gamma = 2.0; // Tone's display gamma is not the grade's.
        s.gain = [1.1; 3];
        assert_eq!(grade_preview(&s, false), "Gain +0.10");
        s.lift = [0.02, 0., -0.02];
        assert_eq!(grade_preview(&s, false), "Lift tint · Gain +0.10");
        s.lut_path = "a.cube".into();
        s.lut_strength = 0.5;
        assert_eq!(grade_preview(&s, false), "Lift tint · Gain +0.10 · LUT 50%");
        // Too long with values: the names alone, and a failed LUT still says so.
        s.grade_gamma = [0.9, 0.95, 0.9];
        assert_eq!(grade_preview(&s, false), "Lift · Gamma · Gain · LUT");
        assert_eq!(grade_preview(&s, true), "Lift · Gamma · Gain · LUT error");
        s.grain = 0.2;
        s.vignette = 0.15;
        assert_eq!(film_preview(&s), "Grain 0.20 · Vignette 0.15");
        s.halation = 0.1;
        assert_eq!(film_preview(&s), "Halation · Grain · Vignette");
        s.glare = 1.;
        assert_eq!(film_preview(&s), "4 effects");
        // Every preview fits beside its title in the 330-point column.
        for l in LOOKS {
            l.apply(&mut s);
            assert!(grade_preview(&s, true).chars().count() <= 34);
            assert!(film_preview(&s).chars().count() <= 34);
        }
    }
    #[test]
    fn looks_write_only_their_fields_and_are_recognised() {
        // Neutral is the default state, so a fresh start reads Neutral.
        assert_eq!(
            Look::of(&Settings::default()).map(|l| l.name),
            Some("Neutral")
        );
        for l in LOOKS {
            let mut s = Settings::default();
            s.exposure = 0.37;
            s.glare = 1.5;
            s.gamma = 2.0;
            s.lut_strength = 0.4;
            s.direct = 0.8;
            let before = s.clone();
            l.apply(&mut s);
            s.validate().unwrap_or_else(|e| panic!("{}: {e}", l.name));
            assert_eq!(Look::of(&s).map(|x| x.name), Some(l.name));
            // Nothing outside the bundle changed.
            let mut back = s.clone();
            back.tonemap = before.tonemap;
            back.halation = before.halation;
            back.grain = before.grain;
            back.grain_size = before.grain_size;
            back.vignette = before.vignette;
            back.lift = before.lift;
            back.grade_gamma = before.grade_gamma;
            back.gain = before.gain;
            assert!(back == before, "{} touched another field", l.name);
            // Moving one field off the bundle reads Custom.
            s.grain += 0.05;
            assert!(Look::of(&s).is_none());
        }
        // No two bundles are the same.
        for (i, a) in LOOKS.iter().enumerate() {
            let mut s = Settings::default();
            a.apply(&mut s);
            assert_eq!(
                LOOKS
                    .iter()
                    .filter(|b| Look::of(&s).is_some_and(|f| f.name == b.name))
                    .count(),
                1,
                "{i}"
            );
        }
    }
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
