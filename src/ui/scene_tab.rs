//! The Scene tab.
use super::*;

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
        egui::ScrollArea::vertical()
            .id_salt("outliner")
            .max_height(230.)
            .show(ui, |ui| {
                ui.selectable_value(
                    &mut state.selection,
                    Selection::Stage,
                    "Stage: room, floor and sky",
                );
                ui.selectable_value(&mut state.selection, Selection::Camera, "Camera");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.light_on, "").on_hover_text("On / off");
                    let r =
                        ui.selectable_label(state.is_selected(Selection::MainLight), "Main light");
                    pick(state, &r, Selection::MainLight);
                });
                for (i, l) in s.lights.iter_mut().enumerate() {
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
            });
        ui.separator();
        ui.label(
            egui::RichText::new(format!(
                "Objects ({} / {})",
                s.objects.len(),
                scene::MAX_OBJECTS
            ))
            .small(),
        );
        if s.objects.len() > 8 {
            ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Search objects…"));
        }
        let search = state.search.to_lowercase();
        let shown: Vec<usize> = (0..s.objects.len())
            .filter(|&i| s.objects[i].name.to_lowercase().contains(&search))
            .collect();
        // Only the rows in view are laid out (4096 objects stay cheap).
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("outliner objects")
            .max_height(260.)
            .show_rows(ui, row_height, shown.len(), |ui, rows| {
                for &i in &shown[rows] {
                    let o = &mut s.objects[i];
                    ui.horizontal(|ui| {
                        ui.push_id(("object", i), |ui| {
                            ui.checkbox(&mut o.visible, "").on_hover_text("Visible");
                        });
                        let r =
                            ui.selectable_label(state.is_selected(Selection::Object(i)), &o.name);
                        pick(state, &r, Selection::Object(i));
                        // A right click keeps a selection the row is part of (its menu acts
                        // on all of it).
                        if r.secondary_clicked() && !state.is_selected(Selection::Object(i)) {
                            state.select(Selection::Object(i));
                        }
                        r.context_menu(|ui| {
                            if let Some(c) = context_items(ui, true) {
                                state.pending = Some(c);
                            }
                        });
                    });
                }
            });
        ui.horizontal(|ui| {
            editor::shape_combo(ui, "add shape", &mut state.add_shape);
            if ui
                .add_enabled(
                    s.objects.len() < scene::MAX_OBJECTS,
                    egui::Button::new("+ Object"),
                )
                .clicked()
            {
                state.checkpoint(s);
                s.objects
                    .push(editor::new_object(state.add_shape, s.objects.len()));
                state.selection = Selection::Object(s.objects.len() - 1);
            }
            if ui
                .add_enabled(
                    s.objects.len() < scene::MAX_OBJECTS,
                    egui::Button::new("+ Mesh…"),
                )
                .on_hover_text("Import an OBJ, glTF, GLB, PLY or STL file (glTF lights come along)")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
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
            ui.checkbox(&mut s.flashlight, "Flashlight").on_hover_text(
                "A spot light on the camera, aimed where you look (with walk mode: a horror game)",
            );
            ui.checkbox(&mut state.mesh_z_up, "Z-up").on_hover_text(
                "Stand the next imported mesh upright when its file is Z-up (scans, CAD)",
            );
            ui.add_enabled_ui(s.lights.len() < scene::MAX_LIGHTS, |ui| {
                ui.menu_button("+ Light", |ui| {
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
                .on_hover_text("Add a light");
            });
        });
        // A selection can outlive its item (preset switch, load, undo-free deletes).
        state.selection = match state.selection {
            Selection::Object(i) if i >= s.objects.len() => Selection::Stage,
            Selection::Light(i) if i >= s.lights.len() => Selection::MainLight,
            other => other,
        };
        ui.separator();
        // Track 17 physics sandbox (see physics.rs).
        section(ui, "Physics");
        let p = &mut s.physics;
        ui.checkbox(&mut p.enabled, "Simulate (rigid bodies)").on_hover_text(
            "Objects fall, stack and collide. Drag an object to grab and throw it; E explodes under the pointer.",
        );
        if p.enabled {
            egui::Grid::new("physics").num_columns(2).show(ui, |ui| {
                row(
                    ui,
                    "Gravity",
                    "Downward acceleration, scene units/s² (negative pulls up)",
                    |ui| {
                        ui.add(egui::Slider::new(&mut p.gravity, -30. ..=30.));
                    },
                );
                row(
                    ui,
                    "Speed",
                    "1 = real time, lower is slow motion, 0 pauses",
                    |ui| {
                        ui.add(egui::Slider::new(&mut p.speed, 0. ..=4.));
                    },
                );
                row(ui, "Bounce", "Restitution of every body", |ui| {
                    ui.add(egui::Slider::new(&mut p.bounce, 0. ..=1.));
                });
                row(ui, "Friction", "Friction of every body", |ui| {
                    ui.add(egui::Slider::new(&mut p.friction, 0. ..=2.));
                });
                row(
                    ui,
                    "Gravity direction",
                    "The way gravity pulls (x, y, z); straight down is 0, −1, 0",
                    |ui| {
                        for c in &mut p.down {
                            ui.add(egui::DragValue::new(c).range(-1. ..=1.).speed(0.01));
                        }
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
                    },
                );
                row(
                    ui,
                    "Gusts",
                    "0 steady, 1 the wind drops away and doubles",
                    |ui| {
                        ui.add(egui::Slider::new(&mut p.gusts, 0. ..=1.));
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
                    .small()
                    .weak(),
            );
        }
        // Track 19 audio-reactive lights (see audio.rs).
        section(ui, "Audio");
        let a = &mut s.audio;
        ui.checkbox(&mut a.enabled, "Lights follow the music").on_hover_text(
            "Captures what the PC plays (loopback) and scales the lights by bass, mid and treble: the main light follows the bass, area light and emitter i band i mod 3. Every frame changes, so use DLSS RR. The scene's intensities are not changed.",
        );
        if a.enabled {
            egui::Grid::new("audio").num_columns(2).show(ui, |ui| {
                row(ui, "Gain", "Added intensity at full level (×)", |ui| {
                    ui.add(egui::Slider::new(&mut a.gain, 0. ..=8.));
                });
                row(ui, "Floor", "Intensity at silence (×)", |ui| {
                    ui.add(egui::Slider::new(&mut a.floor, 0. ..=1.));
                });
                row(ui, "Smoothing", "Higher is calmer", |ui| {
                    ui.add(egui::Slider::new(&mut a.smoothing, 0. ..=0.99));
                });
                row(
                    ui,
                    "Colour shift",
                    "How far each light's colour moves toward its band's at full level: bass red, mid green, treble blue, beats white. Each light's band is in its own settings.",
                    |ui| {
                        ui.add(egui::Slider::new(&mut a.color_shift, 0. ..=1.));
                    },
                );
            });
        }
        // Track 17 laser puzzle (see laser.rs).
        section(ui, "Laser puzzle");
        let p = &mut s.puzzle;
        ui.checkbox(&mut p.enabled, "Laser").on_hover_text(
            "A laser beam traced through the objects: mirrors (metallic, smooth) reflect it, beam splitters fork it, glass bends it. Route it to the target; each finished move counts.",
        );
        if p.enabled {
            egui::Grid::new("puzzle").num_columns(2).show(ui, |ui| {
                row(ui, "Origin", "Where the beam starts (x, y, z)", |ui| {
                    for v in &mut p.origin {
                        ui.add(egui::DragValue::new(v).speed(0.01).range(-40. ..=40.));
                    }
                });
                row(
                    ui,
                    "Direction",
                    "The beam's direction (x, y, z), any length",
                    |ui| {
                        for v in &mut p.direction {
                            ui.add(egui::DragValue::new(v).speed(0.01).range(-1. ..=1.));
                        }
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
                row(
                    ui,
                    "Beam in image",
                    "Also renders the beam as thin glowing rods, so it lights what it passes and shows in captures and videos",
                    |ui| {
                        ui.checkbox(&mut p.beam, "");
                    },
                );
                row(
                    ui,
                    "Colour",
                    "The laser's colour: its red, green and blue are filtered by coloured glass and split apart by glass with dispersion",
                    |ui| {
                        ui.color_edit_button_rgb(&mut p.color);
                    },
                );
                row(
                    ui,
                    "Match colours",
                    "Targets count only when the beam arriving has exactly their colour's channels",
                    |ui| {
                        ui.checkbox(&mut p.colors, "");
                    },
                );
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
        }
        let advanced = state.advanced;
        match state.selection {
            Selection::None => note(
                ui,
                "Click an object in the viewport, or pick an item above.",
            ),
            Selection::Stage => stage_ui(ui, s, advanced),
            Selection::Camera => {
                if camera_ui(ui, s) {
                    let mut d = Settings::default();
                    d.preset(self.preset);
                    (s.yaw, s.pitch, s.distance, s.target, s.fov) =
                        (d.yaw, d.pitch, d.distance, d.target, d.fov);
                }
                if ui.button("Frame the scene (F)").clicked() {
                    self.actions.frame_scene = true;
                }
            }
            Selection::MainLight => main_light_ui(ui, s, advanced),
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
                        .button("Focus")
                        .on_hover_text("Point the camera at it")
                        .clicked()
                    {
                        s.focus_object(i, aspect);
                    }
                });
                // Portals pair two objects both ways.
                let current = s.objects[i].portal;
                let mut pick = current;
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
                        if let Some(third) = usize::try_from(other.portal).ok().filter(|t| *t != i)
                        {
                            s.objects[third].portal = -1;
                        }
                        s.objects[pick as usize].portal = i as i32;
                    }
                }
                if let Selection::Object(i) = state.selection {
                    editor::object_ui(ui, &mut s.objects[i], advanced);
                }
            }
        }
    }
}
