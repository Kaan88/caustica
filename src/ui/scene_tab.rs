//! The Scene tab.
use super::*;
use egui::collapsing_header::{CollapsingState, HeaderResponse};

impl App {
    pub(super) fn scene_tab(&mut self, ui: &mut egui::Ui, known: bool) {
        self.describe_ui(ui);
        let description = if known {
            crate::presets::description(self.preset).to_string()
        } else if let Some(i) = self.scene_entry() {
            self.ui.scenes[i].description.clone()
        } else {
            "Custom scene from a settings file.".to_string()
        };
        let g = self.gpu.as_ref().unwrap();
        let aspect = g.width as f32 / g.height as f32;
        let awake = self.physics.as_ref().map_or(0, |p| p.awake());
        let (moves, solved) = (self.laser.moves, self.laser.solved);
        let s = &mut self.settings;
        let state = &mut self.ui;
        note(ui, &description);
        section(ui, "Outliner");
        // Shift+click adds to the selection or removes from it (multi-select, 3b).
        let shift = ui.input(|i| i.modifiers.shift);
        let pick = |state: &mut UiState, r: &egui::Response, sel: Selection| {
            if r.clicked() {
                if shift {
                    state.toggle(sel);
                } else {
                    state.select(sel);
                }
            }
        };
        let mut shown = outliner_rows(s, &state.search);
        let objects_shown = |rows: &[OutlinerRow]| {
            rows.iter()
                .filter(|r| matches!(r, OutlinerRow::Object(_)))
                .count()
        };
        ui.label(
            egui::RichText::new(if state.search.is_empty() {
                format!("Objects ({} / {})", s.objects.len(), scene::MAX_OBJECTS)
            } else {
                format!(
                    "Objects ({} of {} / {})",
                    objects_shown(&shown),
                    s.objects.len(),
                    scene::MAX_OBJECTS
                )
            })
            .small(),
        );
        // A search stays clearable when the list has shrunk below the box's threshold.
        if shown.len() > 8 || !state.search.is_empty() {
            ui.horizontal(|ui| {
                let clear = !state.search.is_empty();
                // Short of the × and the box's frame: a row wider than the panel widens it.
                let width = (ui.available_width() - 44.).min(280.);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut state.search)
                        .desired_width(width)
                        .hint_text(field_hint("Search the outliner…")),
                );
                if r.changed() {
                    shown = outliner_rows(s, &state.search);
                }
                if clear
                    && ui
                        .small_button("×")
                        .on_hover_text("Clear the search")
                        .clicked()
                {
                    state.search.clear();
                    shown = outliner_rows(s, "");
                }
            });
        }
        // One list, so the wheel has one list to get through on its way to the panel. Only the
        // rows in view are laid out (4096 objects stay cheap).
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("outliner")
            .max_height(260.)
            .show_rows(ui, row_height, shown.len(), |ui, rows| {
                for &row in &shown[rows] {
                    match row {
                        OutlinerRow::Stage => {
                            ui.selectable_value(
                                &mut state.selection,
                                Selection::Stage,
                                "Stage: room, floor and sky",
                            );
                        }
                        OutlinerRow::Camera => {
                            ui.selectable_value(&mut state.selection, Selection::Camera, "Camera");
                        }
                        OutlinerRow::MainLight => {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut s.light_on, "").on_hover_text("On / off");
                                let r = ui.selectable_label(
                                    state.is_selected(Selection::MainLight),
                                    "Main light",
                                );
                                pick(state, &r, Selection::MainLight);
                            });
                        }
                        OutlinerRow::Flashlight => flashlight_row(ui, state, &mut s.flashlight),
                        OutlinerRow::Light(i) => {
                            let l = &mut s.lights[i];
                            ui.horizontal(|ui| {
                                ui.push_id(("light", i), |ui| {
                                    ui.checkbox(&mut l.enabled, "").on_hover_text("On / off");
                                });
                                let r = ui.selectable_label(
                                    state.is_selected(Selection::Light(i)),
                                    format!("Light · {}", l.name),
                                );
                                pick(state, &r, Selection::Light(i));
                            });
                        }
                        OutlinerRow::Object(i) => {
                            let o = &mut s.objects[i];
                            ui.horizontal(|ui| {
                                ui.push_id(("object", i), |ui| {
                                    ui.checkbox(&mut o.visible, "").on_hover_text("Include in the scene (off: hidden from the render and from lighting)");
                                });
                                let r = ui.selectable_label(
                                    state.is_selected(Selection::Object(i)),
                                    &o.name,
                                );
                                pick(state, &r, Selection::Object(i));
                                // A right click keeps a selection the row is part of (its menu
                                // acts on all of it).
                                if r.secondary_clicked() && !state.is_selected(Selection::Object(i))
                                {
                                    state.select(Selection::Object(i));
                                }
                                r.context_menu(|ui| {
                                    if let Some(c) = context_items(ui, true) {
                                        state.pending = Some(c);
                                    }
                                });
                            });
                        }
                    }
                }
            });
        ui.menu_button("+ Add", |ui| {
            let room = s.objects.len() < scene::MAX_OBJECTS;
            ui.add_enabled_ui(room, |ui| {
                ui.menu_button("Object", |ui| {
                    for shape in scene::Shape::ALL {
                        if ui.button(shape.label()).clicked() {
                            state.checkpoint(s);
                            s.objects.push(editor::new_object(shape, s.objects.len()));
                            state.selection = Selection::Object(s.objects.len() - 1);
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_disabled_hover_text(format!("Limit of {} objects reached", scene::MAX_OBJECTS));
            });
            ui.add_enabled_ui(s.lights.len() < scene::MAX_LIGHTS, |ui| {
                ui.menu_button("Light", |ui| {
                    for kind in scene::LightKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            state.checkpoint(s);
                            s.lights.push(editor::new_light(kind, s.lights.len() + 1));
                            state.selection = Selection::Light(s.lights.len() - 1);
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_disabled_hover_text(format!("Limit of {} lights reached", scene::MAX_LIGHTS));
            });
            ui.separator();
            ui.checkbox(&mut state.mesh_z_up, "Z-up").on_hover_text(
                "Stand the next imported mesh upright when its file is Z-up (scans, CAD)",
            );
            if ui
                .add_enabled(room, egui::Button::new("Mesh file…"))
                .on_hover_text("Import an OBJ, glTF, GLB, PLY or STL file (glTF lights come along)")
                .on_disabled_hover_text(format!("Limit of {} objects reached", scene::MAX_OBJECTS))
                .clicked()
            {
                // The file dialog blocks: the menu closes first.
                ui.close_menu();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Meshes", &["obj", "gltf", "glb", "ply", "stl"])
                    .set_directory(crate::scenes::folder().unwrap_or_default())
                    .pick_file()
                {
                    match crate::mesh::import_scene(&path, state.mesh_z_up) {
                        Ok((parts, lights)) => {
                            state.checkpoint(s);
                            let room = scene::MAX_LIGHTS - s.lights.len();
                            s.lights.extend(lights.into_iter().take(room));
                            let first = s.objects.len();
                            let room = scene::MAX_OBJECTS - first;
                            if parts.len() > room {
                                state.error(format!(
                                    "Only {room} of {} parts fit ({} objects)",
                                    parts.len(),
                                    scene::MAX_OBJECTS
                                ));
                            }
                            s.objects.extend(parts.into_iter().take(room));
                            state.selection = Selection::Object(first);
                        }
                        Err(e) => state.error(format!("{e:#}")),
                    }
                }
            }
        })
        .response
        .on_hover_text("Add an object, a light or a mesh file");
        // A selection can outlive its item (preset switch, load, undo-free deletes).
        state.selection = match state.selection {
            Selection::Object(i) if i >= s.objects.len() => Selection::Stage,
            Selection::Light(i) if i >= s.lights.len() => Selection::MainLight,
            other => other,
        };
        ui.separator();
        // A pick in the viewport brings its inspector into view; outliner clicks don't scroll,
        // or the rows would leave from under the pointer.
        if std::mem::take(&mut state.scroll_to_inspector) {
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        let advanced = state.advanced;
        match state.selection {
            Selection::None => note(
                ui,
                "Click an object in the viewport, or pick an item above.",
            ),
            Selection::Stage => stage_ui(ui, s, advanced, state.environment_error.as_deref()),
            Selection::Camera => {
                let camera = (s.yaw, s.pitch, s.distance, s.target);
                if camera_ui(ui, s) {
                    let mut d = Settings::default();
                    d.preset(self.preset);
                    (s.yaw, s.pitch, s.distance, s.target, s.fov) =
                        (d.yaw, d.pitch, d.distance, d.target, d.fov);
                }
                // Typed values are an edit, unlike an orbit: undo takes them back.
                state.camera_edit |= camera != (s.yaw, s.pitch, s.distance, s.target);
                if ui.button("Frame the scene (F)").clicked() {
                    self.actions.frame_scene = true;
                }
            }
            Selection::MainLight => main_light_ui(ui, s, advanced),
            Selection::Flashlight => {
                section(ui, "Flashlight");
                if !s.flashlight {
                    note(ui, "Off: tick it in the outliner, or press T.");
                }
                flashlight_settings(ui, &mut s.flashlight_config, advanced);
            }
            Selection::Light(i) => {
                section(ui, "Area light");
                let audio = s.audio.enabled;
                editor::light_ui(ui, &mut s.lights[i], advanced, audio);
                if ui.button("Delete light (Del)").clicked()
                    && let Some(name) = state.delete_selected(s)
                {
                    state.toast(format!("Deleted {name}"));
                }
            }
            Selection::Object(i) => {
                section(ui, "Object");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            s.objects.len() < scene::MAX_OBJECTS,
                            egui::Button::new("Duplicate (Ctrl+D)"),
                        )
                        .on_disabled_hover_text(format!(
                            "Limit of {} objects reached",
                            scene::MAX_OBJECTS
                        ))
                        .clicked()
                    {
                        state.duplicate_selected(s, true);
                    }
                    if ui.button("Delete (Del)").clicked()
                        && let Some(name) = state.delete_selected(s)
                    {
                        state.toast(format!("Deleted {name}"));
                    }
                    if ui
                        .button("Frame (F)")
                        .on_hover_text("Fit the camera to this object")
                        .clicked()
                    {
                        s.focus_object(i, aspect);
                    }
                });
                // Delete above can have moved or dropped it.
                if let Selection::Object(i) = state.selection {
                    let physics = s.physics.enabled;
                    editor::object_ui(ui, &mut s.objects[i], advanced, physics);
                    portal_ui(ui, state, s, i);
                }
            }
        }
        ui.separator();
        // Track 17 physics sandbox (see physics.rs).
        feature(
            ui,
            "Physics: rigid bodies",
            "Objects fall, stack and collide. Drag an object to grab and throw it; E explodes under the pointer.",
            &mut s.physics.enabled,
        )
        .body(|ui| {
            let p = &mut s.physics;
        if p.enabled {
            egui::Grid::new("physics").num_columns(2).show(ui, |ui| {
                row(
                    ui,
                    "Gravity",
                    "Downward acceleration, scene units/s² (negative pulls up)",
                    |ui| {
                        ui.add(crate::slider(&mut p.gravity, -30. ..=30.));
                    },
                );
                row(
                    ui,
                    "Speed",
                    "1 = real time, lower is slow motion, 0 pauses",
                    |ui| {
                        ui.add(crate::slider(&mut p.speed, 0. ..=4.));
                    },
                );
                row(ui, "Bounce", "Restitution of every body", |ui| {
                    ui.add(crate::slider(&mut p.bounce, 0. ..=1.));
                });
                row(ui, "Friction", "Friction of every body", |ui| {
                    ui.add(crate::slider(&mut p.friction, 0. ..=2.));
                });
                row(
                    ui,
                    "Gravity direction",
                    "The way gravity pulls (x, y, z); straight down is 0, −1, 0",
                    |ui| {
                        for c in &mut p.down {
                            ui.add(egui::DragValue::new(c).range(-1. ..=1.).speed(0.01));
                        }
                        crate::revert_dot(ui, &mut p.down);
                    },
                );
                row(
                    ui,
                    "Wind",
                    "A steady push on every body (x, y, z), scene units/s²",
                    |ui| {
                        for c in &mut p.wind {
                            ui.add(egui::DragValue::new(c).range(-50. ..=50.).speed(0.05));
                        }
                        crate::revert_dot(ui, &mut p.wind);
                    },
                );
                row(
                    ui,
                    "Gusts",
                    "0 steady, 1 the wind drops away and doubles",
                    |ui| {
                        ui.add(crate::slider(&mut p.gusts, 0. ..=1.));
                    },
                );
            });
            if p.down.iter().all(|v| v.abs() <= 0.01) {
                p.down = [0., -1., 0.];
            }
            ui.horizontal(|ui| {
                if ui.button("Wake all").clicked() {
                    self.actions.physics_wake = true;
                }
                if ui
                    .button("Explode (E)")
                    .on_hover_text("A blast at the middle of the floor")
                    .clicked()
                {
                    self.actions.physics_blast = true;
                }
                ui.label(egui::RichText::new(format!("{awake} awake")).small());
            });
            ui.horizontal_wrapped(|ui| {
                if let Selection::Object(i) = state.selection
                    && i < s.objects.len()
                {
                    let frozen = s.objects[i].body == scene::Body::Fixed;
                    if ui
                        .button(if frozen { "Unfreeze" } else { "Freeze" })
                        .on_hover_text("Pins the selected object in place (a fixed body), or lets it fall again")
                        .clicked()
                    {
                        state.checkpoint(s);
                        s.objects[i].body = if frozen {
                            scene::Body::Dynamic
                        } else {
                            scene::Body::Fixed
                        };
                    }
                }
                let recording = state.physics_record;
                if ui
                    .selectable_label(recording, if recording { "⏺ Stop recording" } else { "⏺ Record" })
                    .on_hover_text("Keys every moving body's pose into the timeline at its fps (simulated time), so a timeline video export renders the simulation")
                    .clicked()
                {
                    state.physics_record = !recording;
                    state.physics_record_time = 0.;
                    if recording {
                        state.show_timeline = true;
                    }
                }
                if recording {
                    ui.label(
                        egui::RichText::new(format!("{:.1} s", state.physics_record_time)).small(),
                    );
                }
            });
            ui.label(
                egui::RichText::new("N drops a ball under the pointer, Shift+N a box; the wheel moves a held body nearer or farther.")
                    .small(),
            );
        } else {
            note(ui, "Off: tick the box to turn it on.");
        }
        });
        // Track 19 audio-reactive lights (see audio.rs).
        feature(
            ui,
            "Audio: lights follow the music",
            "Captures what the PC plays (loopback) and scales the lights by bass, mid and treble: the main light follows the bass, area light and emitter i band i mod 3. Every frame changes, so use DLSS RR. The scene's intensities are not changed.",
            &mut s.audio.enabled,
        )
        .body(|ui| {
            let a = &mut s.audio;
        if a.enabled {
            egui::Grid::new("audio").num_columns(2).show(ui, |ui| {
                row(ui, "Gain", "Added intensity at full level (×)", |ui| {
                    ui.add(crate::slider(&mut a.gain, 0. ..=8.));
                });
                row(ui, "Floor", "Intensity at silence (×)", |ui| {
                    ui.add(crate::slider(&mut a.floor, 0. ..=1.));
                });
                row(ui, "Smoothing", "Higher is calmer", |ui| {
                    ui.add(crate::slider(&mut a.smoothing, 0. ..=0.99));
                });
                row(
                    ui,
                    "Colour shift",
                    "How far each light's colour moves toward its band's at full level: bass red, mid green, treble blue, beats white. Each light's band is in its own settings.",
                    |ui| {
                        ui.add(crate::slider(&mut a.color_shift, 0. ..=1.));
                    },
                );
            });
        } else {
            note(ui, "Off: tick the box to turn it on.");
        }
        });
        // Track 17 laser puzzle (see laser.rs).
        feature(
            ui,
            "Laser puzzle",
            "A laser beam traced through the objects: mirrors (metallic, smooth) reflect it, beam splitters fork it, glass bends it. Route it to the target; each finished move counts.",
            &mut s.puzzle.enabled,
        )
        .body(|ui| {
            let p = &mut s.puzzle;
        if p.enabled {
            egui::Grid::new("puzzle").num_columns(2).show(ui, |ui| {
                row(ui, "Origin", "Where the beam starts (x, y, z)", |ui| {
                    for v in &mut p.origin {
                        ui.add(egui::DragValue::new(v).speed(0.01).range(-40. ..=40.));
                    }
                    crate::revert_dot(ui, &mut p.origin);
                });
                row(
                    ui,
                    "Direction",
                    "The beam's direction (x, y, z), any length",
                    |ui| {
                        for v in &mut p.direction {
                            ui.add(egui::DragValue::new(v).speed(0.01).range(-1. ..=1.));
                        }
                        crate::revert_dot(ui, &mut p.direction);
                    },
                );
                row(ui, "Target", "The object the beam must reach", |ui| {
                    let name = |i: i32| {
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| s.objects.get(i))
                            .map_or("None".to_string(), |o| o.name.clone())
                    };
                    egui::ComboBox::from_id_salt("puzzle target")
                        .selected_text(name(p.target))
                        .show_ui(ui, |ui| {
                            for i in -1..s.objects.len() as i32 {
                                ui.selectable_value(&mut p.target, i, name(i));
                            }
                        });
                });
                row(
                    ui,
                    "More targets",
                    "Objects the beam must also reach (every one, to solve)",
                    |ui| {
                        let name = |i: i32| {
                            usize::try_from(i)
                                .ok()
                                .and_then(|i| s.objects.get(i))
                                .map_or("None".to_string(), |o| o.name.clone())
                        };
                        ui.vertical(|ui| {
                            let mut remove = None;
                            for (k, t) in p.targets.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(name(*t));
                                    if ui.small_button("×").clicked() {
                                        remove = Some(k);
                                    }
                                });
                            }
                            if let Some(k) = remove {
                                p.targets.remove(k);
                            }
                            if p.targets.len() < 16 {
                                ui.menu_button("+ Target", |ui| {
                                    for i in 0..s.objects.len() as i32 {
                                        if i != p.target
                                            && !p.targets.contains(&i)
                                            && ui.button(name(i)).clicked()
                                        {
                                            p.targets.push(i);
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }
                        });
                    },
                );
                row(ui, "Par", "Moves a good solution needs", |ui| {
                    ui.add(egui::DragValue::new(&mut p.par).range(0..=99));
                });
                check_row(ui, "Beam in image", "Also renders the beam as thin glowing rods, so it lights what it passes and shows in captures and videos", &mut p.beam);
                row(
                    ui,
                    "Colour",
                    "The laser's colour: its red, green and blue are filtered by coloured glass and split apart by glass with dispersion",
                    |ui| {
                        ui.color_edit_button_rgb(&mut p.color);
                    },
                );
                check_row(ui, "Match colours", "Targets count only when the beam arriving has exactly their colour's channels", &mut p.colors);
            });
            let status = if solved {
                "Solved"
            } else if p.target < 0 {
                "No target"
            } else {
                "Not solved"
            };
            let best = state
                .prefs
                .puzzle_best
                .get(&s.scene_name)
                .map_or(String::new(), |b| format!(" · best {b}"));
            ui.label(
                egui::RichText::new(format!("{status} · {moves} moves · par {}{best}", p.par))
                    .small(),
            );
            // Level select: the puzzle pack's scenes with their best scores.
            let levels: Vec<(std::path::PathBuf, String)> = state
                .scenes
                .iter()
                .filter(|e| e.path.components().any(|c| c.as_os_str() == "puzzles"))
                .map(|e| (e.path.clone(), e.title.clone()))
                .collect();
            if !levels.is_empty() {
                ui.menu_button("Levels", |ui| {
                    for (path, title) in levels {
                        let best = state
                            .prefs
                            .puzzle_best
                            .iter()
                            .find(|(k, _)| **k == title)
                            .map_or("not solved".to_string(), |(_, b)| format!("best {b}"));
                        if ui.button(format!("{title} · {best}")).clicked() {
                            state.open_pending = Some(path);
                            ui.close_menu();
                        }
                    }
                });
            }
        } else {
            note(ui, "Off: tick the box to turn it on.");
        }
        });
    }
}
/// One row of the outliner's list.
#[derive(Clone, Copy, PartialEq, Debug)]
enum OutlinerRow {
    Stage,
    Camera,
    MainLight,
    Flashlight,
    Light(usize),
    Object(usize),
}
/// The list in order, narrowed by the search (a light's row reads "Light · name").
fn outliner_rows(s: &scene::Settings, search: &str) -> Vec<OutlinerRow> {
    let search = search.to_lowercase();
    let hit = |name: &str| name.to_lowercase().contains(&search);
    let fixed = [
        ("stage", OutlinerRow::Stage),
        ("camera", OutlinerRow::Camera),
        ("main light", OutlinerRow::MainLight),
        ("flashlight", OutlinerRow::Flashlight),
    ];
    let lights = s.lights.iter().enumerate();
    let objects = s.objects.iter().enumerate();
    fixed
        .into_iter()
        .filter(|(name, _)| hit(name))
        .map(|(_, row)| row)
        .chain(
            lights
                .filter(|(_, l)| "light".contains(&search) || hit(&l.name))
                .map(|(i, _)| OutlinerRow::Light(i)),
        )
        .chain(
            objects
                .filter(|(_, o)| hit(&o.name))
                .map(|(i, _)| OutlinerRow::Object(i)),
        )
        .collect()
}
/// The outliner's row: its on/off box and a label that opens its inspector. Ticking it selects it,
/// so its settings open when it is turned on. Not transformable, so a plain click even with Shift.
fn flashlight_row(ui: &mut egui::Ui, state: &mut UiState, on: &mut bool) {
    ui.horizontal(|ui| {
        let tick = ui.checkbox(on, "").on_hover_text("On / off (T)");
        if tick.changed() && *on {
            state.select(Selection::Flashlight);
        }
        let r = ui
            .selectable_label(state.is_selected(Selection::Flashlight), "Flashlight")
            .on_hover_text(
                "A flashlight carried beside the camera, aimed where you look (with walk mode: a horror game)",
            );
        if r.clicked() {
            state.select(Selection::Flashlight);
        }
    });
}
/// The flashlight's inspector: the basics, and the rest of the beam and how it is held under More.
fn flashlight_settings(ui: &mut egui::Ui, f: &mut scene::Flashlight, advanced: bool) {
    grid("flashlight").show(ui, |ui| {
        row(ui, "Intensity", "The hot spot's brightness, as a spot's", |ui| {
            ui.add(
                crate::slider(&mut f.intensity, 0.0..=2000.0)
                    .logarithmic(true)
                    .suffix("×"),
            );
        });
        row(ui, "Colour", "Linear RGB colour of the light.", |ui| {
            ui.color_edit_button_rgb(&mut f.color);
        });
        // An IES file replaces the beam: its row takes the beam's place, so a loaded file is
        // never hidden behind More with the beam gone.
        if f.ies.is_empty() {
            row(
                ui,
                "Beam half angle",
                "From the centre of the hot spot to where it falls to half (a spot's Spread is the full angle)",
                |ui| {
                    ui.add(crate::slider(&mut f.beam, 2.0..=30.0).suffix("°"));
                },
            );
        } else {
            flashlight_ies_row(ui, f);
        }
    });
    more(ui, "flashlight more", advanced, |ui| {
        grid("flashlight advanced").show(ui, |ui| {
            if f.ies.is_empty() {
                flashlight_ies_row(ui, f);
                row(
                    ui,
                    "Corona",
                    "The soft glow round the hot spot, against its peak",
                    |ui| {
                        ui.add(crate::slider(&mut f.corona, 0.0..=1.0));
                    },
                );
                row(
                    ui,
                    "Spill",
                    "The dim wide light from the bare LED, against the peak",
                    |ui| {
                        ui.add(crate::slider(&mut f.spill, 0.0..=0.2));
                    },
                );
            }
            row(
                ui,
                "Held right",
                "How far right of the eye. Further out, longer shadows",
                |ui| {
                    ui.add(crate::slider(&mut f.right, -2.0..=2.0).suffix(" units"));
                },
            );
            row(
                ui,
                "Held down",
                "How far below the eye. Further out, longer shadows",
                |ui| {
                    ui.add(crate::slider(&mut f.down, -2.0..=2.0).suffix(" units"));
                },
            );
            row(
                ui,
                "Aim distance",
                "How far ahead the beam crosses the centre of the view",
                |ui| {
                    ui.add(crate::slider(&mut f.aim, 0.5..=50.0).suffix(" units"));
                },
            );
        });
    });
}
fn flashlight_ies_row(ui: &mut egui::Ui, f: &mut scene::Flashlight) {
    row(
        ui,
        "IES profile",
        "An IES file from a lamp maker replaces the beam. Without one, the beam is a reflector's: hot spot, corona and spill. scenes/ies has a few.",
        |ui| {
            let name = std::path::Path::new(&f.ies)
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "None".into());
            ui.label(name);
            if ui.small_button("Load…").clicked() {
                let mut dialog = rfd::FileDialog::new().add_filter("IES", &["ies"]);
                if let Some(folder) = crate::dirs::scene_roots()
                    .into_iter()
                    .map(|d| d.join("ies"))
                    .find(|d| d.is_dir())
                {
                    dialog = dialog.set_directory(folder);
                }
                if let Some(path) = dialog.pick_file() {
                    crate::ies::clear_cache();
                    f.ies = crate::mesh::store(&path);
                }
            }
            if !f.ies.is_empty() && ui.small_button("Clear").clicked() {
                f.ies.clear();
            }
            if !f.ies.is_empty()
                && let Some(e) = crate::ies::load_error(&f.ies)
            {
                cant_load(ui, &e);
            }
        },
    );
}
/// A feature's one-line header: its on/off box and the arrow to its settings. It opens when the
/// feature turns on (by the box or a scene load) and closes when it turns off; in between it
/// stays as the user leaves it.
fn feature<'u>(
    ui: &'u mut egui::Ui,
    title: &str,
    tip: &str,
    on: &mut bool,
) -> HeaderResponse<'u, ()> {
    let id = ui.make_persistent_id(("feature", title));
    let key = id.with("was on");
    let was = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    let mut header =
        CollapsingState::load_with_default_open(ui.ctx(), id, false).show_header(ui, |ui| {
            ui.checkbox(on, title).on_hover_text(tip);
            ui.data_mut(|d| d.insert_temp(key, *on));
        });
    if was != *on {
        header.set_open(*on);
    }
    header
}
/// Portals pair two objects both ways. Rare, so folded away, except while it has one.
fn portal_ui(ui: &mut egui::Ui, state: &mut UiState, s: &mut Settings, i: usize) {
    let current = s.objects[i].portal;
    let mut pick = current;
    egui::CollapsingHeader::new("Portal")
        .id_salt("object portal")
        .open((current >= 0).then_some(true))
        .show(ui, |ui| {
            egui::ComboBox::from_label("Portal to")
                .selected_text(usize::try_from(current).ok().and_then(|p| s.objects.get(p)).map_or("None".to_string(), |o| o.name.clone()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, -1, "None");
                    for (j, o) in s.objects.iter().enumerate().filter(|(j, _)| *j != i) {
                        ui.selectable_value(&mut pick, j as i32, &o.name);
                    }
                })
                .response
                .on_hover_text("Rays that hit this object continue from the same spot on the other one: corridors, impossible rooms.");
        });
    if pick != current {
        state.checkpoint(s);
        if let Some(old) = usize::try_from(current)
            .ok()
            .and_then(|p| s.objects.get_mut(p))
        {
            old.portal = -1;
        }
        s.objects[i].portal = pick;
        if let Some(other) = usize::try_from(pick)
            .ok()
            .and_then(|p| s.objects.get_mut(p))
        {
            if let Some(third) = usize::try_from(other.portal).ok().filter(|t| *t != i) {
                s.objects[third].portal = -1;
            }
            s.objects[pick as usize].portal = i as i32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, add: impl FnMut(&mut egui::Ui)) {
        let mut add = add;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| add(ui));
        });
    }
    /// Four frames: hover, press, release, settle.
    fn click(ctx: &egui::Context, at: egui::Pos2, mut add: impl FnMut(&mut egui::Ui)) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, vec![], &mut add);
        frame(ctx, vec![egui::Event::PointerMoved(at)], &mut add);
        frame(ctx, vec![button(true)], &mut add);
        frame(ctx, vec![button(false)], &mut add);
        frame(ctx, vec![], &mut add);
    }
    #[test]
    fn the_outliner_is_one_list_and_the_search_narrows_all_of_it() {
        let mut s = scene::Settings::default();
        s.objects.clear();
        s.lights.clear();
        s.lights
            .push(editor::new_light(scene::LightKind::ALL[0], 1));
        s.lights[0].name = "Window".into();
        s.objects.push(editor::new_object(scene::Shape::ALL[0], 0));
        s.objects[0].name = "Red chair".into();
        use OutlinerRow as R;
        assert_eq!(
            outliner_rows(&s, ""),
            [
                R::Stage,
                R::Camera,
                R::MainLight,
                R::Flashlight,
                R::Light(0),
                R::Object(0)
            ]
        );
        assert_eq!(outliner_rows(&s, "CHAIR"), [R::Object(0)]);
        assert_eq!(outliner_rows(&s, "wind"), [R::Light(0)]);
        // "light" is on the light's row, whatever its name.
        assert_eq!(
            outliner_rows(&s, "light"),
            [R::MainLight, R::Flashlight, R::Light(0)]
        );
    }
    #[test]
    fn a_feature_header_opens_once_when_turned_on() {
        let ctx = egui::Context::default();
        let open = std::cell::Cell::new(false);
        let id = std::cell::Cell::new(egui::Id::NULL);
        let mut on = false;
        let mut draw = |on: &mut bool| {
            frame(&ctx, vec![], |ui| {
                id.set(ui.make_persistent_id(("feature", "F")));
                let header = feature(ui, "F", "", on);
                open.set(header.is_open());
                header.body(|_| {});
            });
        };
        draw(&mut on);
        assert!(!open.get(), "starts as one line");
        on = true;
        draw(&mut on);
        assert!(open.get(), "turning it on opens it");
        let mut state = CollapsingState::load_with_default_open(&ctx, id.get(), true);
        state.set_open(false);
        state.store(&ctx);
        draw(&mut on);
        draw(&mut on);
        assert!(!open.get(), "the user's close sticks while it stays on");
        on = false;
        draw(&mut on);
        on = true;
        draw(&mut on);
        assert!(open.get(), "turning it on again opens it again");
    }
    #[test]
    fn ticking_the_flashlight_selects_it_and_unticking_keeps_the_selection_alone() {
        let ctx = egui::Context::default();
        let mut state = UiState::default();
        state.select(Selection::Object(0));
        let mut on = false;
        let tick = egui::pos2(18., 18.);
        click(&ctx, tick, |ui| flashlight_row(ui, &mut state, &mut on));
        assert!(on);
        assert_eq!(state.selection, Selection::Flashlight);
        state.select(Selection::Stage);
        click(&ctx, tick, |ui| flashlight_row(ui, &mut state, &mut on));
        assert!(!on);
        assert_eq!(state.selection, Selection::Stage);
    }
    #[test]
    fn the_flashlight_label_selects_it_without_switching_it() {
        let ctx = egui::Context::default();
        let mut state = UiState::default();
        let mut on = false;
        click(&ctx, egui::pos2(60., 18.), |ui| {
            flashlight_row(ui, &mut state, &mut on)
        });
        assert!(!on);
        assert_eq!(state.selection, Selection::Flashlight);
        assert!(state.targets().is_empty(), "gizmos have nothing to move");
    }
}
