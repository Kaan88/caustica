//! Scene-tab panels as free functions: settings dialog, fog, physical sky, stage, camera, lens, main light.
use super::*;

/// Picks a settings JSON to save (`save`) or load, starting in the output folder where the
/// quick buttons keep settings.json (`dirs::output_dir`).
pub(super) fn settings_dialog(save: bool) -> Option<std::path::PathBuf> {
    let d = rfd::FileDialog::new()
        .add_filter("Settings", &["json"])
        .set_directory(std::path::absolute(crate::dirs::output_dir()).unwrap_or_default());
    if save {
        d.set_file_name("settings.json").save_file()
    } else {
        d.pick_file()
    }
}
/// Global fog rows of the Stage grid (track 12).
pub(super) fn fog_ui(ui: &mut egui::Ui, s: &mut Settings) {
    let f = &mut s.fog;
    row(
        ui,
        "Fog",
        "A homogeneous medium from the floor up to a height: god rays and visible light beams. Extinction per unit length; 0 is off.",
        |ui| {
            ui.add(crate::slider(&mut f.density, 0. ..=10.).logarithmic(true));
        },
    );
    if f.density <= 0. {
        return;
    }
    row(
        ui,
        "Fog colour",
        "The share of light each scattering keeps, per channel.",
        |ui| {
            ui.color_edit_button_rgb(&mut f.color);
        },
    );
    row(
        ui,
        "Fog extinction",
        "How strongly each colour is scattered, relative to the density. White treats all alike; more blue than red gives a Rayleigh-like haze: blue light scatters toward you and distant lights turn orange.",
        |ui| {
            ui.color_edit_button_rgb(&mut f.extinction);
            // Black would fail validation on load; it means grey.
            if f.extinction.iter().all(|v| *v <= 0.) {
                f.extinction = [1.; 3];
            }
        },
    );
    row(
        ui,
        "Fog phase",
        "Henyey-Greenstein g: positive scatters forward, which makes beams glow toward the camera.",
        |ui| {
            ui.add(crate::slider(&mut f.phase, -0.95..=0.95));
        },
    );
    row(
        ui,
        "Fog height",
        "The fog's top, in world units (the floor is at −1).",
        |ui| {
            ui.add(crate::slider(&mut f.height, -1. ..=50.));
        },
    );
    row(
        ui,
        "Fog extent",
        "How far the fog reaches from the centre on x and z.",
        |ui| {
            ui.add(crate::slider(&mut f.extent, 0.1..=100.).logarithmic(true));
        },
    );
    row(
        ui,
        "Fog noise",
        "Heterogeneous fog: how much the density varies (0 = uniform; above 1 leaves clear gaps). Rendered exactly by delta and ratio tracking; equiangular sampling is off while it is on.",
        |ui| {
            ui.add(crate::slider(&mut f.noise, 0. ..=4.));
        },
    );
    if f.noise > 0. {
        row(
            ui,
            "Noise size",
            "The size of the fog's puffs, in world units.",
            |ui| {
                ui.add(crate::slider(&mut f.noise_size, 0.05..=100.).logarithmic(true));
            },
        );
    }
    row(
        ui,
        "Equiangular",
        "Also samples scattering along each ray where it passes close to a light (path tracer). Cleaner beams per sample, but about 1.5× the trace time in fog: off by default.",
        |ui| {
            ui.checkbox(&mut f.equiangular, "");
        },
    );
}
/// Physical sky rows of the Stage grid (track 9).
pub(super) fn physical_sky_ui(ui: &mut egui::Ui, s: &mut Settings) {
    row(
        ui,
        "Physical sky",
        "A sun and an Earth sky computed from the time, date and latitude. Replaces the sky light and the HDRI while on.",
        |ui| {
            ui.checkbox(&mut s.physical_sky.on, "");
        },
    );
    if !s.physical_sky.on {
        return;
    }
    let p = &mut s.physical_sky;
    row(
        ui,
        "Time of day",
        "Solar time: 12:00 is when the sun is highest.",
        |ui| {
            ui.add(
                crate::slider(&mut p.time, 0. ..=24.).custom_formatter(|v, _| {
                    let minutes = (v * 60.).round() as u32;
                    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
                }),
            );
        },
    );
    row(
        ui,
        "Clock time",
        "On: the time above is the clock at a longitude and time zone (with the equation of time), not solar time.",
        |ui| {
            ui.checkbox(&mut p.clock, "");
        },
    );
    if p.clock {
        row(ui, "Longitude", "Degrees east (negative: west).", |ui| {
            ui.add(crate::slider(&mut p.longitude, -180. ..=180.).suffix("°"));
        });
        row(
            ui,
            "Time zone",
            "Hours from UTC (daylight saving included).",
            |ui| {
                ui.add(crate::slider(&mut p.utc_offset, -14. ..=14.).suffix(" h"));
            },
        );
    }
    row(
        ui,
        "Date",
        "Day of the year: the sun is highest at the June solstice in the north.",
        |ui| {
            ui.add(
                crate::slider(&mut p.day, 1. ..=366.).custom_formatter(|v, _| day_name(v as f32)),
            );
        },
    );
    row(ui, "Latitude", "Degrees north (negative: south).", |ui| {
        ui.add(crate::slider(&mut p.latitude, -90. ..=90.).suffix("°"));
    });
    row(
        ui,
        "North",
        "Turns the sky (and the sun's path) about the vertical axis. 0: north is −z.",
        |ui| {
            ui.add(crate::slider(&mut p.north, -180. ..=180.).suffix("°"));
        },
    );
    row(
        ui,
        "Turbidity",
        "Haze: 1 is pure air, 2 a clear day, 10 very hazy.",
        |ui| {
            ui.add(crate::slider(&mut p.turbidity, 1. ..=10.));
        },
    );
    row(
        ui,
        "Sun intensity",
        "The sun above the atmosphere lights a white surface facing it to this; the sky follows.",
        |ui| {
            ui.add(
                crate::slider(&mut p.intensity, 0. ..=1000.)
                    .logarithmic(true)
                    .suffix("×"),
            );
        },
    );
    row(
        ui,
        "Sun size",
        "Angular diameter. The real sun is 0.53°; larger gives softer shadows.",
        |ui| {
            ui.add(
                crate::slider(&mut p.sun_size, 0.05..=20.)
                    .logarithmic(true)
                    .suffix("°"),
            );
        },
    );
    row(
        ui,
        "Ground",
        "Albedo of the ground below the horizon.",
        |ui| {
            ui.add(crate::slider(&mut p.ground, 0. ..=1.));
        },
    );
    row(
        ui,
        "Altitude",
        "The observer's height above the ground: higher up the sky darkens and the horizon drops.",
        |ui| {
            ui.add(
                crate::slider(&mut p.altitude, 0. ..=20.)
                    .logarithmic(true)
                    .suffix(" km"),
            );
        },
    );
    row(ui, "Stars", "Star brightness at night (0: none).", |ui| {
        ui.add(crate::slider(&mut p.stars, 0. ..=1000.).logarithmic(true));
    });
    row(
        ui,
        "Moon",
        "The moon, its phase from the date; after twilight it lights the sky.",
        |ui| {
            ui.checkbox(&mut p.moon, "");
        },
    );
    row(
        ui,
        "Seen by camera",
        "Off: the camera sees the background colour while the sky still lights.",
        |ui| {
            ui.checkbox(&mut p.visible, "");
        },
    );
    row(
        ui,
        "Day cycle",
        "Keys the time of day from 06:00 at the start of the timeline to 20:00 at its end (linear).",
        |ui| {
            if ui.small_button("Key a day").clicked() {
                let a = &mut s.animation;
                let path = "physical_sky/time";
                let length = a.length;
                a.key(path, 0., 6.);
                a.key(path, length, 20.);
                a.set_interp(&[path.into()], 0., crate::anim::Interp::Linear);
            }
        },
    );
}
/// "21 Jun" for a day of the year (a non-leap year).
pub(super) fn day_name(day: f32) -> String {
    const MONTHS: [(&str, u32); 12] = [
        ("Jan", 31),
        ("Feb", 28),
        ("Mar", 31),
        ("Apr", 30),
        ("May", 31),
        ("Jun", 30),
        ("Jul", 31),
        ("Aug", 31),
        ("Sep", 30),
        ("Oct", 31),
        ("Nov", 30),
        ("Dec", 31),
    ];
    let mut d = (day.round() as u32).clamp(1, 365);
    for (name, len) in MONTHS {
        if d <= len {
            return format!("{d} {name}");
        }
        d -= len;
    }
    "31 Dec".into()
}
pub(super) fn stage_ui(ui: &mut egui::Ui, s: &mut Settings, advanced: bool) {
    section(ui, "Stage");
    grid("stage").show(ui, |ui| {
        row(
            ui,
            "Cornell box",
            "The closed box with colored side walls.",
            |ui| {
                ui.checkbox(&mut s.cornell_room, "");
            },
        );
        if s.cornell_room {
            row(
                ui,
                "Measured walls",
                "The original Cornell box's measured spectra on the walls and the light (red left, green right; exact with Spectral on in the Render tab). The wall colours and the light's colour and temperature are then ignored.",
                |ui| {
                    ui.checkbox(&mut s.measured_room, "");
                },
            );
        }
        row(
            ui,
            "Ground size",
            "Extent of the floor in scene units.",
            |ui| {
                ui.add(
                    crate::slider(&mut s.floor_extent, 0.5..=40.)
                        .logarithmic(true)
                        .suffix(" units"),
                );
            },
        );
        row(
            ui,
            "Sky light",
            "Light from the surroundings. 0 is a black void.",
            |ui| {
                ui.add(crate::slider(&mut s.sky, 0. ..=4.).suffix("×"));
            },
        );
        physical_sky_ui(ui, s);
        fog_ui(ui, s);
        row(
            ui,
            "HDRI",
            "An equirectangular .hdr or .exr that lights the scene instead of the sky light, importance-sampled.",
            |ui| {
                let name = std::path::Path::new(&s.hdri)
                    .file_name()
                    .map_or("None".into(), |n| n.to_string_lossy().to_string());
                ui.label(name).on_hover_text(&s.hdri);
                if ui.small_button("Load…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("HDRI", &["hdr", "exr"])
                        .set_directory(
                            crate::dirs::scene_roots()
                                .into_iter()
                                .map(|f| f.join("hdri"))
                                .find(|f| f.is_dir())
                                .or_else(crate::scenes::folder)
                                .unwrap_or_default(),
                        )
                        .pick_file()
                {
                    s.hdri = crate::mesh::store(&path);
                }
                if !s.hdri.is_empty() && ui.small_button("Clear").clicked() {
                    s.hdri.clear();
                }
            },
        );
        if !s.hdri.is_empty() && !s.physical_sky.on {
            row(ui, "HDRI strength", "Multiplier on the HDRI's light.", |ui| {
                ui.add(
                    crate::slider(&mut s.hdri_strength, 0. ..=1000.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            row(ui, "HDRI rotation", "Turns the HDRI about the vertical axis.", |ui| {
                ui.add(crate::slider(&mut s.hdri_rotation, -180. ..=180.).suffix("°"));
            });
            row(
                ui,
                "Seen by camera",
                "Off: the camera sees the background colour while the HDRI still lights the scene.",
                |ui| {
                    ui.checkbox(&mut s.hdri_visible, "");
                },
            );
            row(
                ui,
                "Split sun",
                "Takes a compact sun out of the HDRI as a separate disc light of the same power and colour, so light sampling always finds it: much less noise with low-resolution sunlit HDRIs (about the same at 8k). Nothing changes for maps without one.",
                |ui| {
                    ui.checkbox(&mut s.hdri_sun, "");
                },
            );
            row(
                ui,
                "Ground height",
                "Ground projection: the height the HDRI was shot from, in scene units (0 = off). The camera then sees the HDRI's ground on the floor's plane, at a finite distance, instead of at infinity.",
                |ui| {
                    ui.add(crate::slider(&mut s.hdri_ground, 0. ..=20.));
                },
            );
        }
        row(
            ui,
            "Shadow catcher",
            "The floor shows the background behind it, darkened only where objects shadow it: for compositing objects onto an HDRI (with Ground height) or a background colour.",
            |ui| {
                ui.checkbox(&mut s.shadow_catcher, "");
            },
        );
        row(ui, "Left wall", "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.left);
        });
        row(ui, "Right wall", "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.right);
        });
        let base = if s.cornell_room {
            "Back wall, ceiling & floor"
        } else {
            "Ground"
        };
        row(ui, base, "Linear RGB.", |ui| {
            ui.color_edit_button_rgb(&mut s.base);
        });
        row(ui, "Background", "Color seen where rays escape.", |ui| {
            ui.color_edit_button_rgb(&mut s.background);
        });
    });
    more(ui, "stage more", advanced, |ui| {
        ui.collapsing("Wall material", |ui| {
            editor::material_ui(ui, "walls", &mut s.walls, advanced)
        });
        ui.collapsing("Floor material", |ui| {
            editor::material_ui(ui, "floor", &mut s.floor, advanced)
        });
    });
}
/// Returns true when "Reset camera" was pressed.
pub(super) fn camera_ui(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    section(ui, "Camera");
    grid("camera").show(ui, |ui| {
        row(ui, "Auto orbit", "Circle the scene (A).", |ui| {
            ui.checkbox(&mut s.rotate, "");
        });
        row(
            ui,
            "Orbit speed",
            "Degrees per second; negative turns the other way.",
            |ui| {
                ui.add(crate::slider(&mut s.rotate_speed, -60. ..=60.).suffix("°/s"));
            },
        );
        row(ui, "Field of view", "Vertical angle of view.", |ui| {
            ui.add(crate::slider(&mut s.fov, 10. ..=100.).suffix("°"));
        });
        row(
            ui,
            "Distance",
            "From the camera to its target, in scene units.",
            |ui| {
                ui.add(
                    crate::slider(&mut s.distance, 0.5..=200.)
                        .logarithmic(true)
                        .suffix(" units"),
                );
            },
        );
    });
    lens_ui(ui, s);
    note(
        ui,
        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom · Ctrl+click: focus",
    );
    ui.button("Reset camera")
        .on_hover_text("Back to the scene's own view")
        .clicked()
}
/// Camera model, lens system and aperture (Scene → Camera).
pub(super) fn lens_ui(ui: &mut egui::Ui, s: &mut Settings) {
    section(ui, "Lens");
    grid("lens").show(ui, |ui| {
        row(ui, "Camera model", "Orthographic, panorama and little planet use the native denoiser: RR, the temporal upscalers and the gizmo need a perspective camera.", |ui| {
            egui::ComboBox::from_id_salt("projection").selected_text(PROJECTIONS[s.projection as usize]).show_ui(ui, |ui| {
                for (i, n) in PROJECTIONS.iter().enumerate() {
                    ui.selectable_value(&mut s.projection, i as u32, *n);
                }
            });
        });
        if s.projection == 4 {
            row(ui, "Focal-plane tilt", "Tilts the plane of sharp focus about the horizontal axis: the miniature look. Needs an aperture.", |ui| {
                ui.add(crate::slider(&mut s.tilt, -60. ..=60.).suffix("°"));
            });
        }
        if s.projection == 0 {
            let name = |i: u32| if i == 0 { "Thin lens".to_string() } else { let l = &crate::lens::LENSES[i as usize - 1]; let n = crate::lens::INFO[i as usize - 1]; format!("{} f/{:.1}", l.name, n.f_number) };
            row(ui, "Lens", "Thin lens: an ideal lens with the aperture below. The others trace each ray through a real lens prescription (track 16): distortion, vignetting, chromatic aberration and bokeh come from the glass. Real lenses use the native denoiser.", |ui| {
                egui::ComboBox::from_id_salt("lens_system").selected_text(name(s.lens_system)).show_ui(ui, |ui| {
                    for i in 0..=crate::lens::LENSES.len() as u32 {
                        ui.selectable_value(&mut s.lens_system, i, name(i));
                    }
                });
            });
        }
        if s.real_lens() {
            let open = crate::lens::INFO[s.lens_system as usize - 1].f_number;
            row(ui, "f-number", "Stops the lens down: higher is a smaller aperture, deeper focus and less aberration. The image keeps its brightness.", |ui| {
                let mut n = if s.f_number == 0. { open } else { s.f_number.max(open) };
                if ui.add(crate::slider(&mut n, open..=32.).logarithmic(true).custom_formatter(|v, _| format!("f/{v:.1}"))).changed() {
                    s.f_number = if n <= open + 1e-3 { 0. } else { n };
                }
            });
            row(ui, "Focus distance", "Distance of the sharp plane from the front of the lens. Ctrl+click in the viewport focuses there.", |ui| {
                ui.add(crate::slider(&mut s.focus_distance, 0.05..=500.).logarithmic(true).suffix(" units"));
            });
            row(ui, "Scene scale", "Metres per scene unit. The lens is real size, so a small scale (a miniature) gets shallow focus.", |ui| {
                ui.add(crate::slider(&mut s.lens_scale, 0.001..=1000.).logarithmic(true).suffix(" m"));
            });
            row(ui, "Dispersion", "Chromatic aberration from the glass: 1 is real glass, 0 none, higher exaggerates it. Each sample traces one colour, so it adds some colour noise.", |ui| {
                ui.add(crate::slider(&mut s.dispersion, 0. ..=20.));
            });
            row(ui, "Aperture blades", "Shape of the aperture stop: round, or a polygon with this many blades.", |ui| {
                egui::ComboBox::from_id_salt("lens_blades")
                    .selected_text(if s.aperture_blades == 0 { "Round".to_string() } else { format!("{} blades", s.aperture_blades) })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.aperture_blades, 0, "Round");
                        for n in [3, 5, 6, 7, 8, 9] {
                            ui.selectable_value(&mut s.aperture_blades, n, format!("{n} blades"));
                        }
                    });
            });
            if s.aperture_blades > 0 {
                row(ui, "Blade rotation", "Turns the polygonal aperture.", |ui| {
                    ui.add(crate::slider(&mut s.blade_rotation, -180. ..=180.).suffix("°"));
                });
            }
        }
        if s.perspective() {
            row(ui, "Aperture", "Lens radius in scene units. 0 is a pinhole: everything sharp.", |ui| {
                ui.add(crate::slider(&mut s.aperture, 0. ..=2.).logarithmic(true).smallest_positive(0.001).suffix(" units"));
            });
            row(ui, "Focus distance", "Distance of the sharp plane along the view axis. Ctrl+click in the viewport focuses there.", |ui| {
                ui.add(crate::slider(&mut s.focus_distance, 0.05..=500.).logarithmic(true).suffix(" units"));
            });
            row(ui, "Aperture blades", "Shape of the bokeh: round, or a polygon with this many blades.", |ui| {
                egui::ComboBox::from_id_salt("blades")
                    .selected_text(if s.aperture_blades == 0 { "Round".to_string() } else { format!("{} blades", s.aperture_blades) })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.aperture_blades, 0, "Round");
                        for n in [3, 5, 6, 7, 8, 9] {
                            ui.selectable_value(&mut s.aperture_blades, n, format!("{n} blades"));
                        }
                    });
            });
            if s.aperture_blades > 0 {
                row(ui, "Blade rotation", "Turns the polygonal aperture.", |ui| {
                    ui.add(crate::slider(&mut s.blade_rotation, -180. ..=180.).suffix("°"));
                });
            }
            row(ui, "Cat-eye", "Optical vignetting: the lens barrel clips the aperture toward the frame edges, so bokeh turns cat-eye shaped and corners darken.", |ui| {
                ui.add(crate::slider(&mut s.cat_eye, 0. ..=1.));
            });
        }
    });
    if s.real_lens() {
        note(
            ui,
            "Real lenses render with the native denoiser; RR, temporal upscalers, click picking and the gizmo are off.",
        );
    } else if !s.perspective() {
        note(
            ui,
            "This camera model renders with the native denoiser; RR, temporal upscalers and the gizmo are off. A 360° panorama exports best at 2:1 (e.g. --width 4096 --height 2048).",
        );
    }
}
pub(super) fn main_light_ui(ui: &mut egui::Ui, s: &mut Settings, advanced: bool) {
    section(ui, "Main light");
    grid("main light").show(ui, |ui| {
        row(
            ui,
            "On",
            "The ceiling light of the Cornell box, or the studio key light.",
            |ui| {
                ui.checkbox(&mut s.light_on, "");
            },
        );
        row(
            ui,
            "Show outline",
            "Draw the light's rectangle over the viewport, to see and grab it. The selected light always shows it.",
            |ui| {
                ui.checkbox(&mut s.light_outline, "");
            },
        );
        row(ui, "Intensity", "Brightness multiplier.", |ui| {
            ui.add(
                crate::slider(&mut s.light_intensity, 0. ..=2000.)
                    .logarithmic(true)
                    .suffix("×"),
            );
        });
        row(
            ui,
            "Color temperature",
            "Warm (low) to cool (high) white.",
            |ui| {
                ui.add(crate::slider(&mut s.light_temperature, 1000. ..=12000.).suffix(" K"));
            },
        );
        row(ui, "Color filter", "Tints the light, like a gel.", |ui| {
            ui.color_edit_button_rgb(&mut s.light_tint);
        });
        row(ui, "Size", "Bigger lights give softer shadows.", |ui| {
            ui.add(
                crate::slider(&mut s.light_size, 0.05..=40.)
                    .logarithmic(true)
                    .suffix(" units"),
            );
        });
        row(ui, "Position", "In scene units.", |ui| {
            for (p, axis) in s.light_position.iter_mut().zip(["x ", "y ", "z "]) {
                ui.add(
                    egui::DragValue::new(p)
                        .speed(0.025)
                        .range(-50. ..=50.)
                        .prefix(axis),
                );
            }
        });
    });
    more(ui, "main light more", advanced, |ui| {
        grid("main light advanced").show(ui, |ui| {
            row(
                ui,
                "Visible to camera",
                "Show the light's panel in the image.",
                |ui| {
                    ui.checkbox(&mut s.light_visible, "");
                },
            );
            row(ui, "Aspect", "Width relative to depth.", |ui| {
                ui.add(
                    crate::slider(&mut s.light_aspect, 0.1..=10.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
        });
    });
}
