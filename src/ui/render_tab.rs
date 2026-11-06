//! The Render tab.
use super::*;

const BATTLE_RAW: &str = "Battle mode shows raw samples";
const RR_RECONSTRUCTS: &str =
    "Ray Reconstruction is on and does its own upscaling and filtering; turn it off to choose";
/// Why the Upscaler row does nothing right now.
fn upscaler_idle(rr: bool, battle: bool) -> Option<&'static str> {
    if battle {
        Some(BATTLE_RAW)
    } else {
        rr.then_some(RR_RECONSTRUCTS)
    }
}
/// Why the native denoiser does nothing right now. RR counts when it runs, not when it is
/// merely asked for: without it (a real lens, a failed start) the native denoiser takes over.
fn denoiser_idle(rr_active: bool, battle: bool) -> Option<&'static str> {
    if battle {
        Some(BATTLE_RAW)
    } else {
        rr_active.then_some("Ray Reconstruction is on")
    }
}
/// True when the Sample pattern choice is not used: RR, BDPT and photon mapping use white
/// noise. Battle mode's path-traced quadrants do use it.
fn pattern_is_random(rr_active: bool, integrator: u32, battle: bool) -> bool {
    rr_active || (integrator != 0 && !battle)
}

impl App {
    pub(super) fn render_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        section(ui, "Renderer");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.cpu_view, false, "GPU")
                .on_hover_text("The viewport shows the GPU's path tracer");
            ui.selectable_value(&mut self.cpu_view, true, "CPU (software)")
                .on_hover_text(chrome::CPU_VIEW_HELP);
        });
        if self.cpu_view {
            if let Some(live) = self.cpu_reference.as_ref().filter(|l| l.view) {
                let st = live.state();
                note(
                    ui,
                    &format!(
                        "{} threads at low priority. {} / {} spp, {:.1} s.",
                        st.threads, st.samples, st.cap, st.seconds
                    ),
                );
                let text = chrome::cpu_view_text(&st);
                if !text.is_empty() {
                    ui.label(egui::RichText::new(text).small().color(WARN));
                }
            }
            note(
                ui,
                "It always shows the plain image, whatever the view keys pick. The GPU settings below wait for the GPU view.",
            );
        }
        section(ui, "Reconstruction");
        let caps = g.capabilities();
        ui.add_enabled((caps.rr.available || s.rr) && !s.battle, egui::Checkbox::new(&mut s.rr, "DLSS Ray Reconstruction"))
            .on_hover_text("NVIDIA's neural denoiser and upscaler in one. When off, the native denoiser and the upscaler below are used.")
            .on_disabled_hover_text(if s.battle {
                BATTLE_RAW.to_string()
            } else {
                format!("Unavailable: {}", caps.rr.reason)
            });
        if s.rr {
            let color = if g.rr_active {
                ui.visuals().text_color()
            } else {
                WARN
            };
            ui.label(egui::RichText::new(&g.rr_status).small().color(color));
        }
        // At 100% only DLAA / Native AA runs an upscaler (`Gpu::prepare_upscaler`'s test).
        let at_native = g.rw >= g.width && !(s.native_aa && s.upscaler.temporal());
        grid("reconstruction").show(ui, |ui| {
            if s.rr {
                row(ui, "RR model", "The DLSS Ray Reconstruction model.", |ui| {
                    ui.add_enabled_ui(!s.battle, |ui| {
                        let r = egui::ComboBox::from_id_salt("rr model")
                            .selected_text(rr_model(s.rr_preset))
                            .show_ui(ui, |ui| {
                                for id in [6, 5, 4, 0] {
                                    ui.selectable_value(&mut s.rr_preset, id, rr_model(id));
                                }
                            })
                            .response;
                        why_disabled(r, BATTLE_RAW);
                    });
                });
            }
            row(
                ui,
                "Quality mode",
                "Share of the pixels that are traced; DLSS, FSR or XeSS (or RR) reconstruct the rest. At 100%, DLAA / Native AA runs the upscaler as anti-aliasing; Native accumulates samples progressively.",
                |ui| {
                    egui::ComboBox::from_id_salt("quality mode")
                        .selected_text(quality_label(s))
                        .width(175.)
                        .show_ui(ui, |ui| {
                            for (scale, aa, label) in quality_modes(s) {
                                let current = (s.scale - scale).abs() < 0.005 && (scale < 1. || s.native_aa == aa);
                                if ui.selectable_label(current, label).clicked() {
                                    s.scale = scale;
                                    s.native_aa = aa;
                                }
                            }
                        });
                },
            );
            row(ui, "Upscaler", "Used below 100%, and as DLAA / Native AA at 100%. Ray Reconstruction does its own upscaling.", |ui| {
                let idle = upscaler_idle(s.rr, s.battle);
                ui.add_enabled_ui(idle.is_none(), |ui| {
                    let r = egui::ComboBox::from_id_salt("upscaler")
                        .selected_text(s.upscaler.label())
                        .width(175.)
                        .show_ui(ui, |ui| {
                            for u in Upscaler::ALL {
                                let backend = caps.upscaler(u);
                                ui.add_enabled_ui(backend.is_none_or(|b| b.available) || s.upscaler == u, |ui| {
                                    ui.selectable_value(&mut s.upscaler, u, u.label())
                                        .on_disabled_hover_text(format!("Unavailable: {}", backend.map_or("", |b| b.reason.as_str())));
                                });
                            }
                        })
                        .response;
                    why_disabled(r, idle.unwrap_or_default());
                });
            });
            if !s.rr && s.upscaler == Upscaler::Dlss {
                row(
                    ui,
                    "DLSS preset",
                    "Default lets NVIDIA pick: K for DLAA, Quality and Balanced; M for Performance; L for Ultra Performance.",
                    |ui| {
                        let label = DLSS_PRESETS.iter().find(|(id, _)| *id == s.dlss_preset).map_or("NVIDIA default", |(_, l)| *l);
                        let idle = if s.battle {
                            Some(BATTLE_RAW)
                        } else {
                            at_native.then_some("Not used at full resolution: pick a lower Quality mode or an AA mode")
                        };
                        ui.add_enabled_ui(idle.is_none(), |ui| {
                            let r = egui::ComboBox::from_id_salt("dlss preset")
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    for (id, label) in DLSS_PRESETS {
                                        ui.selectable_value(&mut s.dlss_preset, id, label);
                                    }
                                })
                                .response;
                            why_disabled(r, idle.unwrap_or_default());
                        });
                    },
                );
            }
            check_row_with(
                ui,
                "Native denoiser (D)",
                "SVGF-style denoiser, used when Ray Reconstruction is off. Off shows raw samples.",
                &mut s.denoise,
                denoiser_idle(g.rr_active, s.battle),
                |_, _| {},
            );
            row(
                ui,
                "Pixel filter",
                "How the progressive path spreads each pixel's samples. Box is the old sharp, slightly jagged look; Blackman-Harris and Gaussian give smoother edges. Ray Reconstruction and the temporal upscalers use their own jitter instead.",
                |ui| {
                    ui.add_enabled_ui(!s.rr, |ui| {
                        let r = egui::ComboBox::from_id_salt("pixel_filter")
                            .selected_text(
                                *scene::PIXEL_FILTERS
                                    .get(s.pixel_filter as usize)
                                    .unwrap_or(&"?"),
                            )
                            .show_ui(ui, |ui| {
                                for (i, name) in scene::PIXEL_FILTERS.iter().enumerate() {
                                    ui.selectable_value(&mut s.pixel_filter, i as u32, *name);
                                }
                            })
                            .response;
                        why_disabled(r, RR_RECONSTRUCTS);
                    });
                },
            );
            if advanced || (s.filter_width - 1.5).abs() > 1e-3 {
                row(
                    ui,
                    "Filter width",
                    "Full width of the pixel filter: wider is softer and hides more aliasing. 1.5 pixels matches Cycles.",
                    |ui| {
                        ui.add_enabled_ui(!s.rr, |ui| {
                            why_disabled(
                                ui.add(crate::slider(&mut s.filter_width, 0.5..=4.).suffix(" px")),
                                RR_RECONSTRUCTS,
                            );
                        });
                    },
                );
            }
            if advanced || !RESOLUTIONS.iter().any(|(v, _)| (v - s.scale).abs() < 0.005) {
                row(ui, "Render scale", "Any render scale from 25% to 100%.", |ui| {
                    ui.add(
                        crate::slider(&mut s.scale, 0.25..=1.)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.))
                            .custom_parser(|t| t.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.)),
                    );
                });
            }
        });
        note(
            ui,
            &format!(
                "Render {} × {} · output {} × {}",
                g.rw, g.rh, g.width, g.height
            ),
        );
        if !s.rr && !g.upscale_status.is_empty() {
            note(ui, &g.upscale_status);
        } else if !s.rr && !s.battle && at_native {
            note(
                ui,
                "The upscaler is not running at Native; it sets the AA entry in Quality mode.",
            );
        }
        section(ui, "Sampling");
        let bdpt_over = s.bounces > 24;
        grid("sampling").show(ui, |ui| {
            row(
                ui,
                "Stop after",
                "Rendering pauses at this many samples per pixel.",
                |ui| {
                    ui.add(
                        crate::slider(&mut s.max_samples, 1..=1_000_000)
                            .logarithmic(true)
                            .suffix(" spp"),
                    );
                },
            );
            row(
                ui,
                "Samples per frame",
                "More is smoother per frame but slower to respond.",
                |ui| {
                    ui.add(crate::slider(&mut s.spp, 1..=64).logarithmic(true).suffix(" spp"));
                },
            );
            row(
                ui,
                "Light bounces",
                "How many times a path may bounce. More brightens deep corners and glass.",
                |ui| {
                    let slider = crate::slider(&mut s.bounces, 1..=32);
                    ui.add(if s.integrator == 1 && !s.battle && bdpt_over {
                        slider.suffix(" (BDPT stops at 24)")
                    } else {
                        slider
                    });
                },
            );
            row(
                ui,
                "Integrator",
                "Path tracing follows light backwards from the camera. Bidirectional path tracing (BDPT) also traces paths from the lights and joins the two: it finds light that squeezes through gaps (a door ajar) and caustics seen directly. Photon mapping scatters photons from the lights and gathers them where camera paths land: caustics under glass converge fast, at the cost of a slight blur that fades as samples grow. BDPT and photon mapping ignore volumes and portals. BDPT uses up to 24 bounces.",
                |ui| {
                    egui::ComboBox::from_id_salt("integrator")
                        .selected_text(INTEGRATORS[s.integrator.min(2) as usize])
                        .show_ui(ui, |ui| {
                            for (i, name) in INTEGRATORS.iter().enumerate() {
                                ui.selectable_value(&mut s.integrator, i as u32, *name);
                            }
                        });
                },
            );
            if s.integrator != 1 || s.battle {
                check_row(ui, "Spectral", "Trace wavelengths of light instead of red, green and blue: glass with dispersion splits white light into a rainbow, and thin films and gratings get their exact colours. Colours shift slightly where light bounces between coloured surfaces, as in reality. A little more colour noise at low samples. Path tracing and photon mapping (there the photons carry wavelengths, so a prism throws a rainbow); BDPT stays RGB.", &mut s.spectral);
            }
            if s.integrator == 2 || s.battle {
                row(ui, "Photons per frame", "Photons traced from the lights each frame.", |ui| {
                    ui.add(crate::slider(&mut s.photons, 1 << 12..=1 << 20).logarithmic(true));
                });
                row(ui, "Gather radius", "The starting radius photons are gathered in, as a share of the scene's size. It shrinks as samples grow; larger is smoother but blurrier at first.", |ui| {
                    ui.add(crate::slider(&mut s.photon_radius, 0.0005..=0.2).logarithmic(true));
                });
            }
            check_row(ui, "Battle mode", "Splits the image into four quadrants rendered at equal GPU time: path tracing, path tracing with ReSTIR DI, BDPT and photon mapping. Raw samples only (no denoiser, Ray Reconstruction or upscaler). Open the convergence graph with a reference to compare their error over time.", &mut s.battle);

            row(
                ui,
                "Texture filtering",
                "Anisotropic filtering keeps textures sharp on floors and walls seen at grazing angles: up to this many taps along the stretched footprint. 1× is plain trilinear mipmapping, which blurs them.",
                |ui| {
                    egui::ComboBox::from_id_salt("anisotropy")
                        .selected_text(format!("{}×", s.anisotropy))
                        .show_ui(ui, |ui| {
                            for n in [1, 2, 4, 8, 16] {
                                ui.selectable_value(&mut s.anisotropy, n, format!("{n}×"));
                            }
                        });
                },
            );
        });
        more(ui, "render more", advanced, |ui| {
            section(ui, "Advanced");
            grid("render advanced").show(ui, |ui| {
                if !g.rr_active && s.denoise {
                    row(ui, "Denoise strength", "How strongly the native denoiser smooths.", |ui| {
                        ui.add_enabled_ui(!s.battle, |ui| {
                            why_disabled(ui.add(crate::slider(&mut s.denoise_strength, 0. ..=1.)), BATTLE_RAW);
                        });
                    });
                    check_row_with(
                        ui,
                        "Reuse temporal history",
                        "The native denoiser blends in earlier frames.",
                        &mut s.temporal,
                        s.battle.then_some(BATTLE_RAW),
                        |_, _| {},
                    );
                }
                row(ui, "Sample pattern", "Low-discrepancy (Owen-scrambled Sobol) converges faster. Ray Reconstruction always uses random samples.", |ui| {
                    let random = pattern_is_random(g.rr_active, s.integrator, s.battle);
                    ui.add_enabled_ui(!random, |ui| {
                        let r = egui::ComboBox::from_id_salt("pattern")
                            .selected_text(if s.sobol { "Low-discrepancy" } else { "Random" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.sobol, true, "Low-discrepancy");
                                ui.selectable_value(&mut s.sobol, false, "Random");
                            })
                            .response;
                        why_disabled(r, "Random samples are always used here");
                    });
                });
                let mut on = s.firefly > 0.;
                let firefly = check_row_with(ui, "Suppress fireflies", "Clamp single very bright samples. Biased, but removes white specks.", &mut on, None, |ui, on| {
                    if on {
                        ui.add(crate::slider(&mut s.firefly, 0.1..=100.).logarithmic(true).suffix("× max"));
                    }
                });
                if firefly.changed() {
                    s.firefly = if on { 10. } else { 0. };
                }
                row(ui, "Light sampling", "MIS combines light sampling (NEE) and BSDF sampling. The single strategies show what each one is good at: NEE for small lights and rough surfaces, BSDF for large lights and mirrors.", |ui| {
                    for (i, label) in ["MIS", "NEE only", "BSDF only"].iter().enumerate() {
                        ui.selectable_value(&mut s.mis_mode, i as u32, *label);
                    }
                });
                check_row(ui, "ReSTIR DI", "First-hit light sampling through per-pixel reservoirs that reuse good light samples across neighbouring pixels and frames. Unbiased; it pays off with many lights at a few samples per pixel.", &mut s.restir);
                if s.restir {
                    row(ui, "Candidates", "Fresh light samples per pixel and frame.", |ui| {
                        ui.add(crate::slider(&mut s.restir_candidates, 1..=32));
                    });
                    row(ui, "Neighbours", "Reservoirs of nearby pixels merged each frame (0 = none).", |ui| {
                        ui.add(crate::slider(&mut s.restir_neighbors, 0..=8));
                    });
                    row(ui, "History", "How much last frame's reservoir may count against a fresh one (0 = no temporal reuse).", |ui| {
                        ui.add(crate::slider(&mut s.restir_history, 0..=64));
                    });
                }
                check_row(ui, "Glass casts shadows", "Off lets light through glass for shadow rays, a common shortcut.", &mut s.glass_shadow);
                check_row(ui, "VSync", "Wait for the display between frames.", &mut s.vsync);
            });
        });
    }
    /// The Quality dropdown (UI plan 5.1): bundles of the reconstruction fields, the current
    /// one derived from the settings (never stored). RR rows are disabled with the reason
    /// when RR is not available here.
    pub(super) fn quality_combo(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let caps = g.capabilities();
        let rr_ok = caps.rr.available;
        let upscaler = [Upscaler::Dlss, Upscaler::Xess, Upscaler::Fsr]
            .into_iter()
            .find(|u| caps.upscaler(*u).is_some_and(|b| b.available))
            .unwrap_or(Upscaler::Bicubic);
        let rr_reason = caps.rr.reason;
        let current = Bundle::of(&self.settings);
        let mut chosen = None;
        egui::ComboBox::from_id_salt("quality")
            .selected_text(current.map_or("Custom quality", |b| b.label()))
            .width(120.)
            .show_ui(ui, |ui| {
                for b in Bundle::ALL {
                    if b == Bundle::Native {
                        ui.separator();
                    }
                    let usable = rr_ok || !b.rr();
                    let text = format!("{}  ·  {}", b.label(), b.about(upscaler));
                    let r = ui
                        .add_enabled(usable, egui::SelectableLabel::new(current == Some(b), text));
                    let r = r.on_disabled_hover_text(format!(
                        "Needs DLSS Ray Reconstruction: {rr_reason}"
                    ));
                    if r.clicked() {
                        chosen = Some(b);
                    }
                }
                if current.is_none() {
                    ui.separator();
                    note(ui, "Custom: the Render tab's settings match no bundle.");
                }
            })
            .response
            .on_hover_text(
                "Quality: denoiser and render resolution in one choice (details in the Render tab)",
            );
        if let Some(b) = chosen {
            b.apply(&mut self.settings, upscaler);
            self.ui.toast(format!("Quality: {}", b.label()));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_rows_name_the_live_cause() {
        assert_eq!(upscaler_idle(false, false), None);
        assert_eq!(upscaler_idle(true, false), Some(RR_RECONSTRUCTS));
        assert_eq!(upscaler_idle(true, true), Some(BATTLE_RAW));
        // RR asked for but not running: the native denoiser is what runs.
        assert_eq!(denoiser_idle(false, false), None);
        assert!(denoiser_idle(true, false).is_some());
        assert_eq!(denoiser_idle(false, true), Some(BATTLE_RAW));
        assert!(!pattern_is_random(false, 0, false));
        assert!(pattern_is_random(true, 0, false));
        assert!(pattern_is_random(false, 1, false));
        assert!(pattern_is_random(false, 2, false));
        // Battle mode's path-traced quadrants read the pattern, whatever the integrator says.
        assert!(!pattern_is_random(false, 2, true));
    }
}
