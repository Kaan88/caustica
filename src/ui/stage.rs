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
        "Haze from the floor up to a height, which makes light beams and god rays visible. The density is how much light it scatters per unit length; 0 is off.",
        |ui| {
            ui.add(
                crate::slider(&mut f.density, 0. ..=10.)
                    .logarithmic(true)
                    .custom_formatter(|v, _| short_number(v)),
            );
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
        "How strongly each colour is scattered, relative to the density. White treats all alike; more blue than red gives a bluish haze, and distant lights turn orange (as in Rayleigh scattering).",
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
        "Fog scatter direction",
        "Forward glow vs back-scatter (Henyey-Greenstein g). Positive scatters forward, so beams glow toward the camera.",
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
        "Patchy fog: how much the density varies (0 = even; above 1 leaves clear gaps). Exact, so slower than even fog; equiangular sampling is off while it is on.",
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
    check_row(
        ui,
        "Equiangular",
        "Also samples scattering along each ray where it passes close to a light (path tracer). Cleaner beams per sample, but about 1.5× the trace time in fog: off by default.",
        &mut f.equiangular,
    );
}
/// Physical sky rows of the Stage grid (track 9).
pub(super) fn physical_sky_ui(ui: &mut egui::Ui, s: &mut Settings) {
    check_row(
        ui,
        "Physical sky",
        "A sun and an Earth sky computed from the time, date and latitude. Replaces the sky light and the HDRI while on.",
        &mut s.physical_sky.on,
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
    check_row(
        ui,
        "Clock time",
        "On: the time above is the clock at a longitude and time zone (with the equation of time), not solar time.",
        &mut p.clock,
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
    row(
        ui,
        "Year",
        "The calendar year: it fixes where the stars stand (precession, sidereal time).",
        |ui| {
            ui.add(crate::slider(&mut p.year, 1900..=2100));
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
    check_row(
        ui,
        "Visible to camera",
        "Off: the camera sees the background colour while the sky still lights.",
        &mut p.visible,
    );
    ui.label(
        egui::RichText::new("Night sky")
            .strong()
            .color(crate::ui::ACCENT),
    );
    ui.end_row();
    row(
        ui,
        "Star brightness",
        "The catalogue stars' brightness at night (1: the real sky's; 0: none). Stars are point lights: they light surfaces and show in mirrors and glass.",
        |ui| {
            ui.add(crate::slider(&mut p.stars, 0. ..=1000.).logarithmic(true));
        },
    );
    row(
        ui,
        "Limiting magnitude",
        "Stars fainter than this (V magnitude) are left out. 6.5 is the naked eye in a dark sky, about 9,000 stars; a long camera exposure reaches 9 and beyond, many times more.",
        |ui| {
            ui.add(crate::slider(&mut p.limiting_magnitude, -2. ..=16.));
        },
    );
    row(
        ui,
        "Star size",
        "How wide a star's disc is on screen, in pixels (1: a point). Larger spreads the same light over more pixels, like a soft or glowing star. Does not change how stars light the scene.",
        |ui| {
            ui.add(
                crate::slider(&mut p.star_size, 0.5..=32.)
                    .logarithmic(true)
                    .suffix(" px"),
            );
        },
    );
    row(
        ui,
        "Milky Way",
        "Milky Way brightness (1: the real sky's, from the NASA SVS map; 0: none).",
        |ui| {
            ui.add(crate::slider(&mut p.milky_way, 0. ..=10.));
        },
    );
    row(
        ui,
        "Airglow",
        "The upper air's own faint glow, brightest a little above the horizon (1: a dark site).",
        |ui| {
            ui.add(crate::slider(&mut p.airglow, 0. ..=10.));
        },
    );
    row(
        ui,
        "Light pollution",
        "Skyglow from towns, rising to the horizon (1: dark rural, 0: none, 10: a suburb).",
        |ui| {
            ui.add(crate::slider(&mut p.light_pollution, 0. ..=10.));
        },
    );
    check_row(
        ui,
        "Moon",
        "The moon, its phase from the date; after twilight it lights the sky.",
        &mut p.moon,
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
/// Why a Stage control does nothing right now (`None`: it is live). The stored values stay;
/// the rows grey out with the reason as their tooltip.
#[derive(Default, Debug, PartialEq)]
pub(super) struct Overrides {
    pub walls: Option<&'static str>,
    pub base: Option<&'static str>,
    pub sky_light: Option<&'static str>,
    pub background: Option<&'static str>,
    pub hdri: Option<&'static str>,
}
pub(super) fn overrides(s: &Settings, hdri_error: bool) -> Overrides {
    let hdri_lights = !s.hdri.is_empty() && !hdri_error && !s.physical_sky.on;
    let sky_seen = if s.physical_sky.on {
        s.physical_sky.visible
    } else {
        hdri_lights && s.hdri_visible
    };
    let measured = "Measured walls use the original box's spectra";
    Overrides {
        walls: if s.measured_room {
            Some(measured)
        } else if !s.walls.texture.is_empty() {
            Some("The wall texture replaces the colour (Wall material, below)")
        } else {
            None
        },
        base: if s.measured_room {
            Some(measured)
        } else if s.floor.texture.is_empty() || (s.cornell_room && s.walls.texture.is_empty()) {
            None
        } else {
            Some("The textures replace the colour (Wall and Floor material, below)")
        },
        sky_light: if s.physical_sky.on {
            Some("The physical sky replaces it")
        } else if hdri_lights {
            Some("The HDRI replaces it (Clear the HDRI to use it)")
        } else {
            None
        },
        background: if sky_seen {
            Some(if s.physical_sky.on {
                "The camera sees the physical sky (untick Visible to camera to see this colour)"
            } else {
                "The camera sees the HDRI (untick Visible to camera to see this colour)"
            })
        } else {
            None
        },
        hdri: s.physical_sky.on.then_some("The physical sky replaces it"),
    }
}
/// The Stage inspector in three blocks (Room, Sky and light, Fog), each its own grid with the
/// same label column so the controls line up. Rows that something else overrides stay visible
/// but greyed, with the reason (`overrides`).
pub(super) fn stage_ui(
    ui: &mut egui::Ui,
    s: &mut Settings,
    advanced: bool,
    hdri_error: Option<&str>,
) {
    let o = overrides(s, hdri_error.is_some());
    section(ui, "Room");
    grid("stage room").min_col_width(LABEL_WIDTH).show(ui, |ui| {
        check_row(ui, "Cornell box", "The closed box with coloured side walls.", &mut s.cornell_room);
        // Shown while on even without the box, so the switch that greys the colours can be reached.
        if s.cornell_room || s.measured_room {
            check_row(ui, "Measured walls", "The original Cornell box's measured spectra on the walls and the light (red left, green right; exact with Spectral on in the Render tab). The wall colours and the light's colour and temperature are then ignored.", &mut s.measured_room);
        }
        if s.cornell_room {
            row_if(ui, "Left wall", "Colour of the left wall (the picker edits linear RGB, not screen colour).", o.walls, |ui| {
                ui.color_edit_button_rgb(&mut s.left);
            });
            row_if(ui, "Right wall", "Colour of the right wall (the picker edits linear RGB, not screen colour).", o.walls, |ui| {
                ui.color_edit_button_rgb(&mut s.right);
            });
        }
        let base = if s.cornell_room {
            "Other walls"
        } else {
            "Floor colour"
        };
        row_if(ui, base, "Colour of the other walls and the floor (the picker edits linear RGB, not screen colour).", o.base, |ui| {
            ui.color_edit_button_rgb(&mut s.base);
        });
        row(
            ui,
            "Floor size",
            "How far the floor reaches, in scene units.",
            |ui| {
                ui.add(
                    crate::slider(&mut s.floor_extent, 0.5..=40.)
                        .logarithmic(true)
                        .suffix(" units"),
                );
            },
        );
        check_row(ui, "Shadow catcher", "The floor shows the background behind it, darkened only where objects shadow it: for compositing objects onto an HDRI (with Ground height) or a background colour.", &mut s.shadow_catcher);
    });
    more(ui, "stage more", advanced, |ui| {
        if s.cornell_room {
            ui.collapsing("Wall material", |ui| {
                editor::material_ui(ui, "walls", &mut s.walls, advanced)
            });
        }
        ui.collapsing("Floor material", |ui| {
            editor::material_ui(ui, "floor", &mut s.floor, advanced)
        });
    });
    section(ui, "Sky and light");
    grid("stage sky").min_col_width(LABEL_WIDTH).show(ui, |ui| {
        row_if(
            ui,
            "Sky light",
            "Light from the surroundings. 0 is a black void.",
            o.sky_light,
            |ui| {
                ui.add(crate::slider(&mut s.sky, 0. ..=4.).suffix("×"));
            },
        );
        physical_sky_ui(ui, s);
        row_if(
            ui,
            "HDRI",
            "A 360° panorama (.hdr or .exr) that lights the scene and is seen behind it, instead of the sky light.",
            o.hdri,
            |ui| {
                let name = std::path::Path::new(&s.hdri)
                    .file_name()
                    .map_or("None".into(), |n| n.to_string_lossy().to_string());
                own_tip(ui.label(name), &s.hdri);
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
                if !s.hdri.is_empty()
                    && !s.physical_sky.on
                    && let Some(e) = hdri_error
                {
                    cant_load(ui, e);
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
            check_row(ui, "Visible to camera", "Off: the camera sees the background colour while the HDRI still lights the scene.", &mut s.hdri_visible);
            check_row(ui, "Split sun", "Takes a compact sun out of the HDRI as a separate disc light of the same power and colour, so light sampling always finds it: much less noise with low-resolution sunlit HDRIs (about the same at 8k). Nothing changes for maps without one.", &mut s.hdri_sun);
            row(
                ui,
                "Ground height",
                "Ground projection: the height the HDRI was shot from, in scene units (0 = off). The camera then sees the HDRI's ground on the floor's plane, at a finite distance, instead of at infinity.",
                |ui| {
                    ui.add(crate::slider(&mut s.hdri_ground, 0. ..=20.));
                },
            );
        }
        row_if(
            ui,
            "Background",
            "The colour seen where rays escape the scene.",
            o.background,
            |ui| {
                ui.color_edit_button_rgb(&mut s.background);
            },
        );
    });
    section(ui, "Fog");
    grid("stage fog").min_col_width(LABEL_WIDTH).show(ui, |ui| {
        fog_ui(ui, s);
    });
}
/// Returns true when "Reset camera" was pressed.
pub(super) fn camera_ui(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    section(ui, "Camera");
    // The mouse controls and the way back come first: they are what a new user looks for.
    note(
        ui,
        "Drag: orbit · Right or Shift-drag: pan · Wheel: zoom · Ctrl+click: focus",
    );
    let reset = ui
        .button("Reset camera")
        .on_hover_text("Back to the scene's own view")
        .clicked();
    grid("camera").show(ui, |ui| {
        check_row(ui, "Auto orbit", "Circle the scene (O).", &mut s.rotate);
        check_row(
            ui,
            "Walk avatar",
            "Walk mode (`) draws a blocky body under the camera: legs and hands in view when looking down, the whole figure in mirrors and reflections.",
            &mut s.walk_avatar,
        );
        row_if(
            ui,
            "Orbit speed",
            "Degrees per second; negative turns the other way.",
            (!s.rotate).then_some("Auto orbit is off"),
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
    reset
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
            "This camera model renders with the native denoiser; RR, temporal upscalers, click picking and the gizmo are off. A 360° panorama exports best at 2:1 (e.g. --width 4096 --height 2048).",
        );
    }
}
pub(super) fn main_light_ui(ui: &mut egui::Ui, s: &mut Settings, advanced: bool) {
    section(ui, "Main light");
    grid("main light").show(ui, |ui| {
        check_row(ui, "On", "The ceiling light of the Cornell box, or the studio key light.", &mut s.light_on);
        check_row(ui, "Show outline", "Draw the light's rectangle over the viewport, to see and grab it. The selected light always shows it.", &mut s.light_outline);
        row(ui, "Intensity", "Brightness multiplier.", |ui| {
            ui.add(
                crate::slider(&mut s.light_intensity, 0. ..=2000.)
                    .logarithmic(true)
                    .suffix("×"),
            );
        });
        row(
            ui,
            "Colour temperature",
            "Warm (low) to cool (high) white.",
            |ui| {
                ui.add(crate::slider(&mut s.light_temperature, 1000. ..=12000.).suffix(" K"));
            },
        );
        row(ui, "Colour filter", "Tints the light, like a gel.", |ui| {
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
            crate::revert_dot(ui, &mut s.light_position);
        });
    });
    more(ui, "main light more", advanced, |ui| {
        grid("main light advanced").show(ui, |ui| {
            check_row(
                ui,
                "Visible to camera",
                "Show the light's panel in the image.",
                &mut s.light_visible,
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_stage_has_nothing_overridden() {
        let s = Settings::default();
        assert_eq!(overrides(&s, false), Overrides::default());
    }

    #[test]
    fn measured_walls_grey_the_colours_and_the_physical_sky_the_sky_light() {
        let mut s = Settings::default();
        s.measured_room = true;
        let o = overrides(&s, false);
        assert!(o.walls.is_some() && o.base.is_some());
        assert!(o.sky_light.is_none());
        s.measured_room = false;
        s.physical_sky.on = true;
        let o = overrides(&s, false);
        assert!(o.sky_light.is_some() && o.hdri.is_some());
        // Visible to camera is on by default, so the background colour is not seen either.
        assert_eq!(s.physical_sky.visible, o.background.is_some());
        s.physical_sky.visible = false;
        assert!(overrides(&s, false).background.is_none());
    }

    #[test]
    fn an_hdri_that_failed_to_load_overrides_nothing() {
        let mut s = Settings::default();
        s.hdri = "sky.hdr".into();
        assert!(overrides(&s, false).sky_light.is_some());
        assert_eq!(overrides(&s, true), Overrides::default());
        s.hdri_visible = false;
        assert!(overrides(&s, false).background.is_none());
    }

    #[test]
    fn textures_replace_the_wall_and_floor_colours() {
        let mut s = Settings::default();
        s.cornell_room = true;
        s.walls.texture = "bricks.png".into();
        let o = overrides(&s, false);
        assert!(o.walls.is_some());
        // The floor is still untextured, so its colour (the same swatch) still matters.
        assert!(o.base.is_none());
        s.floor.texture = "wood.png".into();
        assert!(overrides(&s, false).base.is_some());
    }
}
