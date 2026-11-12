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
/// The Reconstruction header: the quality mode and what denoises. Beside that title the
/// header's monospace preview has room for about 26 characters and `fold` cuts off the rest at
/// the left, so a longer pair drops what the mode's name repeats: "(100%)" from an AA mode,
/// the name from a scaled one.
fn reconstruction_preview(quality: &str, scale: f32, runs: &str) -> String {
    let full = format!("{quality} · {runs}");
    if full.chars().count() <= 26 {
        full
    } else if scale >= 0.995 {
        format!("{} · {runs}", quality.trim_end_matches(" (100%)"))
    } else {
        format!("{:.0}% · {runs}", scale * 100.)
    }
}
/// An upscaler's name in the Upscaling header, where the combo's full label does not fit.
fn upscaler_short(u: Upscaler) -> &'static str {
    match u {
        Upscaler::Bilinear => "Bilinear",
        Upscaler::Bicubic => "Bicubic",
        Upscaler::Dlss => "DLSS",
        Upscaler::Fsr => "FSR",
        Upscaler::Xess => "XeSS",
    }
}

impl App {
    pub(super) fn render_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let s = &mut self.settings;
        let advanced = self.ui.advanced;
        tab_title(ui, "Render");
        let caps = g.capabilities();
        // At 100% only DLAA / Native AA runs an upscaler (`Gpu::prepare_upscaler`'s test).
        let at_native = g.rw >= g.width && !(s.native_aa && s.upscaler.temporal());
        let preview = reconstruction_preview(
            &quality_label(s),
            s.scale,
            // What runs, not only what is chosen: battle mode shows raw samples, and Ray
            // Reconstruction that cannot run falls back to the native denoiser.
            match s.denoiser {
                _ if s.battle => "raw",
                Denoiser::Rr if g.rr_active => "RR",
                Denoiser::Rr => "RR (native)",
                Denoiser::Native => "native",
                Denoiser::Off => "raw",
            },
        );
        fold(ui, "Reconstruction", preview, true, |ui| {
            grid("reconstruction").show(ui, |ui| {
            row(
                ui,
                "Denoiser",
                "What cleans the noise. Ray Reconstruction is NVIDIA's neural denoiser and upscaler in one; Native is the built-in SVGF-style filter and works with the upscaler below; None shows raw samples. Ray Reconstruction falls back to Native when it cannot run. Alt+D cycles them.",
                |ui| {
                    ui.add_enabled_ui(!s.battle, |ui| {
                        let r = egui::ComboBox::from_id_salt("denoiser")
                            .selected_text(s.denoiser.label())
                            .width(220_f32.min(ui.available_width()))
                            .show_ui(ui, |ui| {
                                for d in Denoiser::ALL {
                                    let backend = caps.denoiser(d);
                                    ui.add_enabled_ui(backend.is_none_or(|b| b.available) || s.denoiser == d, |ui| {
                                        ui.selectable_value(&mut s.denoiser, d, d.label())
                                            .on_disabled_hover_text(format!("Unavailable: {}", backend.map_or("", |b| b.reason.as_str())));
                                    });
                                }
                            })
                            .response;
                        why_disabled(r, BATTLE_RAW);
                    });
                },
            );
            if s.rr() {
                row(ui, "RR status", "Whether Ray Reconstruction is running, and why not.", |ui| {
                    let color = if g.rr_active {
                        ui.visuals().text_color()
                    } else {
                        WARN
                    };
                    ui.label(egui::RichText::new(&g.rr_status).small().color(color));
                });
            }
            if s.rr() {
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
            // The native denoiser's own rows sit with the Denoiser choice, shown when it is the
            // choice (or under Show advanced, where Ray Reconstruction's fallback uses them).
            if s.native_filter() && (advanced || s.denoiser == Denoiser::Native) {
            let idle = denoiser_idle(g.rr_active, s.battle);
                row(ui, "Denoise strength", "How strongly the native denoiser smooths.", |ui| {
                    ui.add_enabled_ui(idle.is_none(), |ui| {
                        why_disabled(ui.add(crate::slider(&mut s.denoise_strength, 0. ..=1.)), idle.unwrap_or_default());
                    });
                });
                check_row_with(
                    ui,
                    "Reuse temporal history",
                    "The native denoiser blends in earlier frames.",
                    &mut s.temporal,
                    idle,
                    |_, _| {},
                );
            }
            check_row_with(
                ui,
                "Still denoiser (OIDN)",
                "Intel Open Image Denoise cleans finished stills: --capture and Save image as PNG or JPEG, and the software view once its render has stopped. EXR keeps the raw samples. It runs on the CPU, guided by the albedo and normal, and takes about a second at 1080p. The live view is not changed.",
                &mut s.still_denoise,
                crate::oidn::status().err(),
                |_, _| {},
            );
            row(
                ui,
                "Quality mode",
                "Share of the pixels that are traced; DLSS, FSR or XeSS (or RR) reconstruct the rest. At 100%, DLAA / Native AA runs the upscaler as anti-aliasing; Native accumulates samples progressively.",
                |ui| {
                    egui::ComboBox::from_id_salt("quality mode")
                        .selected_text(quality_label(s))
                        .width(175_f32.min(ui.available_width()))
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
        });
        // BDPT stops at 24 bounces, as its slider says.
        let bounces = if s.integrator == 1 && !s.battle {
            s.bounces.min(24)
        } else {
            s.bounces
        };
        let preview = format!("{} spp · {} bounces", s.spp, bounces);
        fold(ui, "Sampling", preview, true, |ui| {
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
                    let slider = crate::slider(&mut s.bounces, 1..=64);
                    ui.add(if s.integrator == 1 && !s.battle && bdpt_over {
                        slider.suffix(" (BDPT stops at 24)")
                    } else {
                        slider
                    });
                },
            );
            row(
                ui,
                "Specular bounces",
                "Extra bounces for mirror-like and glass vertices (smooth glass and water, mirrors, roughness up to 0.1), on top of Light bounces, as games do: glass behind glass and windows into rooms stay lit at a low bounce count. 0 counts every bounce against Light bounces. Path tracing only; BDPT, photon mapping and ReSTIR GI use Light bounces.",
                |ui| {
                    ui.add(crate::slider(&mut s.specular_bounces, 0..=64));
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
        });
        let filter = scene::PIXEL_FILTERS
            .get(s.pixel_filter as usize)
            .unwrap_or(&"?");
        // As the rows grey out: battle mode runs no upscaler, and Ray Reconstruction replaces
        // both the upscaler and the pixel filter.
        let preview = if s.battle {
            format!("off · {filter}")
        } else if s.rr() {
            "RR".to_string()
        } else if at_native {
            format!("native · {filter}")
        } else {
            format!("{} · {filter}", upscaler_short(s.upscaler))
        };
        fold(ui, "Upscaling and filter", preview, false, |ui| {
            grid("upscaling").show(ui, |ui| {
            row(ui, "Upscaler", "Used below 100%, and as DLAA / Native AA at 100%. Ray Reconstruction does its own upscaling.", |ui| {
                let idle = upscaler_idle(s.rr(), s.battle);
                ui.add_enabled_ui(idle.is_none(), |ui| {
                    let r = egui::ComboBox::from_id_salt("upscaler")
                        .selected_text(s.upscaler.label())
                        .width(175_f32.min(ui.available_width()))
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
            if !s.rr() && s.upscaler == Upscaler::Dlss {
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
            row(
                ui,
                "Pixel filter",
                "How the progressive path spreads each pixel's samples. Box is the old sharp, slightly jagged look; Blackman-Harris and Gaussian give smoother edges. Ray Reconstruction and the temporal upscalers use their own jitter instead.",
                |ui| {
                    ui.add_enabled_ui(!s.rr(), |ui| {
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
                        ui.add_enabled_ui(!s.rr(), |ui| {
                            why_disabled(
                                ui.add(crate::slider(&mut s.filter_width, 0.5..=4.).suffix(" px")),
                                RR_RECONSTRUCTS,
                            );
                        });
                    },
                );
            }
        });
            if !s.rr() && !g.upscale_status.is_empty() {
                note(ui, &g.upscale_status);
            } else if !s.rr() && !s.battle && at_native {
                note(
                    ui,
                    "The upscaler is not running at Native; it sets the AA entry in Quality mode.",
                );
            }
        });
        let preview = match self.cpu_reference.as_ref().filter(|l| l.view) {
            Some(live) => format!("{} threads", live.state().threads),
            None => "off".to_string(),
        };
        fold(ui, "CPU renderer", preview, self.cpu_view, |ui| {
            if !self.cpu_view {
                note(
                    ui,
                    "Off. CPU in the top bar shows the software path tracer: every ray on the CPU, through the same exposure, tone map and look.",
                );
            }
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
                ui.checkbox(&mut self.cpu_denoise, "Denoise the CPU view")
                .on_hover_text("The native denoiser's spatial passes on the CPU's mean, for looking at only: saved EXRs keep the plain mean. Turning it on starts the render over.");
                if self.cpu_denoise {
                    ui.add(crate::slider(&mut s.denoise_strength, 0. ..=1.))
                        .on_hover_text("How strongly the denoiser smooths.");
                }
                note(
                    ui,
                    "It always shows the plain image, whatever the view keys pick. The GPU settings below wait for the GPU view.",
                );
                if advanced {
                    ui.collapsing("CPU truth", |ui| {
                    note(
                        ui,
                        "Exact versions of the shortcuts the CPU copies from the GPU, for measuring them. All off is the GPU's model. Changing one starts the render over.",
                    );
                    let mut t = s.truth;
                    let mut flags = [
                        ("Russian roulette off", &mut t.roulette),
                        ("Firefly clamp off", &mut t.firefly),
                        ("No pdf floors", &mut t.floors),
                        ("Uncapped fog collisions", &mut t.collisions),
                        ("No fog early out", &mut t.fog_cutoff),
                        ("No absorption colour floor", &mut t.absorption_floor),
                        ("Exact SDF marching", &mut t.sdf_march),
                        ("Rough glass BTDF", &mut t.glass_btdf),
                        ("Internal re-reflection (pending)", &mut t.internal_reflection),
                        ("Multiple scattering (pending)", &mut t.multiscatter),
                        ("Conductor Fresnel (pending)", &mut t.fresnel),
                        ("Float-error ray offsets (pending)", &mut t.offsets),
                        ("Shadow terminator (pending)", &mut t.shadow_terminator),
                    ];
                    for (label, on) in &mut flags {
                        ui.checkbox(on, *label);
                    }
                    if t != s.truth {
                        s.truth = t;
                    }
                });
                }
            }
        });
        let preview = format!(
            "{}{}",
            if s.restir { "ReSTIR · " } else { "" },
            // Random also where the Sample pattern row is greyed out.
            if s.sobol && !pattern_is_random(g.rr_active, s.integrator, s.battle) {
                "Sobol"
            } else {
                "Random"
            }
        );
        fold(ui, "Advanced", preview, advanced, |ui| {
            grid("render advanced").show(ui, |ui| {
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
                let firefly = check_row_with(ui, "Suppress fireflies", "Clamp single very bright samples: after four samples, one may be at most this many times the pixel's mean so far, and never clamped below 0.05 (scene units, not exposed ones: very dark scenes are clamped harder). Biased, but removes white specks.", &mut on, None, |ui, on| {
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
                check_row(ui, "Exact rough glass", "Rough glass as a real microfacet BTDF (light sampling included) instead of a blur of the sharp direction. Slower; part of the Accurate bundle.", &mut s.exact_glass);
                check_row(ui, "Gentle roulette", "Russian roulette starts at bounce 8 and never drops a path below 20% survival (default: bounce 3, 5%). Same image on average, less noise, slower. Part of the Accurate bundle.", &mut s.gentle_roulette);
                check_row(ui, "Textures at full detail", "Reads every texture at its sharpest level instead of a blurrier one for distant or indirect hits, as the CPU reference does. Noisier on fine patterns, exact when converged. Part of the Accurate bundle.", &mut s.lod0_textures);
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
                    if b == Bundle::Best || b == Bundle::Native {
                        ui.separator();
                    }
                    let usable = rr_ok || !b.rr();
                    let text = format!("{}  ·  {}", b.label(), b.about(upscaler));
                    let r =
                        ui.add_enabled(usable, egui::Button::selectable(current == Some(b), text));
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
            b.apply(&mut self.settings, upscaler, rr_ok);
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
    #[test]
    fn the_reconstruction_header_fits_beside_its_title() {
        let p = reconstruction_preview;
        assert_eq!(p("Quality (67%)", 2. / 3., "RR"), "Quality (67%) · RR");
        assert_eq!(
            p("DLAA (100%)", 1., "RR (native)"),
            "DLAA (100%) · RR (native)"
        );
        assert_eq!(
            p("FSR Native AA (100%)", 1., "native"),
            "FSR Native AA · native"
        );
        assert_eq!(
            p("Ultra performance (33%)", 1. / 3., "RR (native)"),
            "33% · RR (native)"
        );
        for (scale, label) in RESOLUTIONS {
            for runs in ["RR", "RR (native)", "native", "raw"] {
                assert!(p(label, scale, runs).chars().count() <= 26);
            }
        }
    }
}
