//! The outliner and the Lights, Object and Material tabs (together, the old Scene tab), and the
//! windows that were its sections (the laser puzzle).
use super::*;
use crate::timeline_ui::{key_mark, key_scope};
use crate::ui::skin::ShowSkin as _;
use egui::collapsing_header::CollapsingState;

/// How far the outliner's children sit in from their group.
const INDENT: f32 = 16.;

/// What the selection is called: in the folded outliner's line and the tab titles.
/// Whether `sel` holds keyframes (the outliner's diamond).
fn has_keys(s: &Settings, sel: Selection) -> bool {
    if s.animation.tracks.is_empty() {
        return false;
    }
    let prefixes = crate::anim::prefixes(sel);
    s.animation
        .tracks
        .iter()
        .any(|t| !t.keys.is_empty() && crate::anim::under_any(&t.path, &prefixes))
}
pub(super) fn selection_label(s: &Settings, sel: Selection) -> String {
    match sel {
        Selection::None => "Nothing".into(),
        Selection::Stage => "Stage".into(),
        Selection::Camera => "Camera".into(),
        Selection::MainLight => "Main light".into(),
        Selection::Flashlight => "Flashlight".into(),
        Selection::Light(i) => s.lights.get(i).map_or("Light".into(), |l| l.name.clone()),
        Selection::Object(i) => s.objects.get(i).map_or("Object".into(), |o| o.name.clone()),
    }
}
/// The entries of an Add menu (the outliner's + and the rail's): objects, lights, a mesh file.
pub(super) fn add_menu(ui: &mut egui::Ui, state: &mut UiState, s: &mut Settings) {
    let room = s.objects.len() < scene::MAX_OBJECTS;
    ui.add_enabled_ui(room, |ui| {
        crate::ui::skin::menu_button(ui, "Object", |ui| {
            for shape in scene::Shape::ALL {
                if crate::ui::skin::button(ui, shape.label()).clicked() {
                    state.checkpoint(s);
                    s.objects.push(editor::new_object(shape, s.objects.len()));
                    state.selection = Selection::Object(s.objects.len() - 1);
                    ui.close();
                }
            }
        })
        .response
        .on_disabled_hover_text(format!("Limit of {} objects reached", scene::MAX_OBJECTS));
    });
    ui.add_enabled_ui(s.lights.len() < scene::MAX_LIGHTS, |ui| {
        crate::ui::skin::menu_button(ui, "Light", |ui| {
            for kind in scene::LightKind::ALL {
                if crate::ui::skin::button(ui, kind.label()).clicked() {
                    state.checkpoint(s);
                    s.lights.push(editor::new_light(kind, s.lights.len() + 1));
                    state.selection = Selection::Light(s.lights.len() - 1);
                    ui.close();
                }
            }
        })
        .response
        .on_disabled_hover_text(format!("Limit of {} lights reached", scene::MAX_LIGHTS));
    });
    crate::ui::skin::separator(ui);
    crate::ui::skin::checkbox_label(ui, &mut state.mesh_z_up, "Z-up")
        .on_hover_text("Stand the next imported mesh upright when its file is Z-up (scans, CAD)");
    if ui
        .add_enabled(room, crate::ui::skin::Button::new("Mesh file…"))
        .on_hover_text("Import an OBJ, glTF, GLB, PLY or STL file (glTF lights come along)")
        .on_disabled_hover_text(format!("Limit of {} objects reached", scene::MAX_OBJECTS))
        .clicked()
    {
        // The file dialog blocks: the menu closes first.
        ui.close();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Meshes", &["obj", "gltf", "glb", "ply", "stl"])
            .set_directory(crate::scenes::folder().unwrap_or_default())
            .pick_file()
        {
            state.import_mesh(path, state.mesh_z_up);
        }
    }
}
/// The outliner's Sky row: what lights the scene from outside.
fn sky_state(s: &Settings) -> &'static str {
    if s.physical_sky.on {
        "physical"
    } else if !s.hdri.is_empty() {
        "HDRI"
    } else if s.sky > 0. {
        "sky light"
    } else {
        "off"
    }
}

impl App {
    /// The outliner: a header line (name, filter, + Add), then the tree. `foldable`: above
    /// Render, Look and Export, or in a short column, the tree shows only while it is opened
    /// from the header. `height` is the tree's room in points.
    pub(super) fn outliner(&mut self, ui: &mut egui::Ui, known: bool, foldable: bool, height: f32) {
        let description = if known {
            crate::presets::description(self.preset).to_string()
        } else if let Some(i) = self.scene_entry() {
            self.ui.scenes[i].description.clone()
        } else {
            "Custom scene from a settings file.".to_string()
        };
        let s = &mut self.settings;
        let state = &mut self.ui;
        let open = !foldable || state.outliner_open;
        ui.horizontal(|ui| {
            let header = skin::band_title(ui, open, "Outliner");
            let header = header.on_hover_text(
                "The list of everything in the scene. It folds away above Render, Look and Export",
            );
            if foldable && header.clicked() {
                state.outliner_open = !state.outliner_open;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !open {
                    let name = selection_label(s, state.selection);
                    let more = state.also.len();
                    let text = match (state.selection, more) {
                        (Selection::None, _) => "nothing selected".to_string(),
                        (_, 0) => format!("{name} selected"),
                        _ => format!("{name} and {more} more selected"),
                    };
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).small().color(skin::DIM))
                            .truncate(),
                    );
                    return;
                }
                // Sticky like crate::ui::sticky_menu (the Z-up checkbox inside must not close
                // it; the buttons call ui.close()).
                let plus =
                    crate::ui::skin::menu_button_sticky(ui, "+", |ui| add_menu(ui, state, s));
                plus.response
                    .on_hover_text("Add an object, a light or a mesh file");
                let clear = !state.search.is_empty();
                if clear
                    && crate::ui::skin::small_button(ui, "×")
                        .on_hover_text("Clear the filter")
                        .clicked()
                {
                    state.search.clear();
                }
                crate::ui::skin::text_edit(
                    ui,
                    110.,
                    egui::TextEdit::singleline(&mut state.search).hint_text(field_hint("Filter…")),
                );
            });
        });
        if let Some(job) = state.mesh_imports.first() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Importing {}…", job.name));
            });
        }
        if open {
            // Shift+click adds to the selection or removes from it (multi-select, 3b).
            let shift = ui.input(|i| i.modifiers.shift);
            let rows = outliner_rows(s, &state.search, state.outliner_closed);
            let filtered = !state.search.is_empty();
            let tab = state.tab;
            // What was clicked: the item, and whether it was the Sky row.
            let mut picked: Option<(Selection, bool)> = None;
            // One list, so the wheel has one list to get through. Only the rows in view are
            // laid out (4096 objects stay cheap).
            let row_height = ui.spacing().interact_size.y;
            ui.spacing_mut().item_spacing.y = 2.;
            // Whole rows only: the list ends under a row, not through one.
            let step = row_height + 2.;
            let whole = ((height + 2.) / step).floor().max(1.) * step - 2.;
            egui::ScrollArea::vertical()
                .id_salt("outliner")
                .max_height(whole)
                .auto_shrink([false, foldable])
                .show_rows(ui, row_height, rows.len(), |ui, range| {
                    for &row in &rows[range] {
                        match row {
                            OutlinerRow::Stage => {
                                let on = state.selection == Selection::Stage && tab != Tab::Lights;
                                if tree_row(ui, 0., on, "Stage", "room and floor")
                                    .on_hover_text(&description)
                                    .clicked()
                                {
                                    picked = Some((Selection::Stage, false));
                                }
                            }
                            OutlinerRow::Camera => {
                                let on = state.is_selected(Selection::Camera);
                                let dim = PROJECTIONS
                                    .get(s.projection as usize)
                                    .filter(|_| s.projection != 0)
                                    .copied()
                                    .unwrap_or("");
                                let keyed = has_keys(s, Selection::Camera);
                                if tree_row_keyed(ui, INDENT, on, "Camera", dim, keyed).clicked() {
                                    picked = Some((Selection::Camera, false));
                                }
                            }
                            OutlinerRow::Sky => {
                                let on = state.selection == Selection::Stage && tab == Tab::Lights;
                                if tree_row(ui, INDENT, on, "Sky", sky_state(s))
                                    .on_hover_text("The sky, the sun, an HDRI and fog: the top of the Lights tab")
                                    .clicked()
                                {
                                    picked = Some((Selection::Stage, true));
                                }
                            }
                            OutlinerRow::MainLight => {
                                ui.horizontal(|ui| {
                                    ui.add_space(INDENT);
                                    crate::ui::skin::checkbox_label(ui, &mut s.light_on, "").on_hover_text("On / off");
                                    let on = state.is_selected(Selection::MainLight);
                                    let dim = if s.light_on { "" } else { "off" };
                                    let keyed = has_keys(s, Selection::MainLight);
                                    if tree_row_keyed(ui, 0., on, "Main light", dim, keyed).clicked() {
                                        picked = Some((Selection::MainLight, false));
                                    }
                                });
                            }
                            OutlinerRow::Flashlight => {
                                let was = state.selection;
                                flashlight_row(ui, state, &mut s.flashlight, INDENT);
                                if state.selection != was {
                                    state.selection = was;
                                    picked = Some((Selection::Flashlight, false));
                                }
                            }
                            OutlinerRow::Lights => {
                                let text = format!("Area lights ({})", s.lights.len());
                                let closed = state.outliner_closed[0] && !filtered;
                                if skin::group_row(ui, !closed, &text, skin::TX).clicked() {
                                    state.outliner_closed[0] = !state.outliner_closed[0];
                                }
                            }
                            OutlinerRow::Light(i) => {
                                let keyed = has_keys(s, Selection::Light(i));
                                let l = &mut s.lights[i];
                                ui.horizontal(|ui| {
                                    ui.add_space(INDENT);
                                    ui.push_id(("light", i), |ui| {
                                        crate::ui::skin::checkbox_label(ui, &mut l.enabled, "").on_hover_text("On / off");
                                    });
                                    let on = state.is_selected(Selection::Light(i));
                                    let dim = if l.enabled { l.kind.label() } else { "off" };
                                    if tree_row_keyed(ui, 0., on, &l.name, dim, keyed).clicked() {
                                        picked = Some((Selection::Light(i), false));
                                    }
                                });
                            }
                            OutlinerRow::Objects => {
                                let shown = rows
                                    .iter()
                                    .filter(|r| matches!(r, OutlinerRow::Object(_)))
                                    .count();
                                let text = if filtered {
                                    format!("Objects ({shown} of {})", s.objects.len())
                                } else {
                                    format!("Objects ({})", s.objects.len())
                                };
                                let closed = state.outliner_closed[1] && !filtered;
                                if skin::group_row(ui, !closed, &text, skin::TX)
                                    .on_hover_text(format!(
                                        "{} of at most {} objects",
                                        s.objects.len(),
                                        scene::MAX_OBJECTS
                                    ))
                                    .clicked()
                                {
                                    state.outliner_closed[1] = !state.outliner_closed[1];
                                }
                            }
                            OutlinerRow::Object(i) => {
                                let keyed = has_keys(s, Selection::Object(i));
                                let o = &mut s.objects[i];
                                ui.horizontal(|ui| {
                                    ui.add_space(INDENT);
                                    ui.push_id(("object", i), |ui| {
                                        crate::ui::skin::checkbox_label(ui, &mut o.visible, "").on_hover_text("Include in the scene (off: hidden from the render and from lighting)");
                                    });
                                    let on = state.is_selected(Selection::Object(i));
                                    let dim = if o.visible { "" } else { "hidden" };
                                    let r = tree_row_keyed(ui, 0., on, &o.name, dim, keyed);
                                    if r.clicked() {
                                        picked = Some((Selection::Object(i), false));
                                    }
                                    // A right click keeps a selection the row is part of (its
                                    // menu acts on all of it).
                                    if r.secondary_clicked() && !on {
                                        state.select(Selection::Object(i));
                                    }
                                    crate::ui::skin::context_menu(&r, |ui| {
                                        if let Some(c) = context_items(ui, true) {
                                            state.pending = Some(c);
                                        }
                                    });
                                });
                            }
                        }
                    }
                });
            if let Some((sel, sky)) = picked {
                // The fixed rows are not transformable: a plain click even with Shift.
                if shift && transformable(sel) {
                    state.toggle(sel);
                } else {
                    state.select(sel);
                }
                // The tabs that show the selection follow it; the Sky row is the Lights tab.
                if sky {
                    state.tab = Tab::Lights;
                } else if state.tab.follows_selection() {
                    state.tab = state.tab.after_pick(sel, false);
                    state.scroll_to_inspector = true;
                }
            }
        }
        // A selection can outlive its item (preset switch, load, undo-free deletes).
        state.selection = match state.selection {
            Selection::Object(i) if i >= s.objects.len() => Selection::Stage,
            Selection::Light(i) if i >= s.lights.len() => Selection::MainLight,
            other => other,
        };
    }
    /// Lights: the sky, HDRI, background and fog on top whatever is selected, the selected
    /// light under them, and the audio feature last.
    pub(super) fn lights_tab(&mut self, ui: &mut egui::Ui) {
        let advanced = self.ui.advanced;
        self.key_marks_begin();
        let s = &mut self.settings;
        let state = &mut self.ui;
        let title = match state.selection {
            sel @ (Selection::MainLight | Selection::Flashlight | Selection::Light(_)) => {
                selection_label(s, sel)
            }
            _ => "Lights".to_string(),
        };
        tab_title(ui, &title);
        sky_ui(ui, s, state.environment_error.as_deref());
        crate::ui::skin::separator(ui);
        // A pick brings the light's inspector into view, under the sky blocks.
        if std::mem::take(&mut state.scroll_to_inspector)
            && !matches!(state.selection, Selection::Stage | Selection::None)
        {
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        match state.selection {
            Selection::MainLight => main_light_ui(ui, s, advanced),
            Selection::Flashlight => {
                section(ui, "Flashlight");
                if !s.flashlight {
                    note(ui, "Off: tick it in the outliner, or press T.");
                }
                flashlight_settings(ui, &mut s.flashlight_config, advanced);
            }
            Selection::Light(i) if i < s.lights.len() => {
                ui.horizontal(|ui| {
                    section(ui, "Area light");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::skin::small_button(ui, "Delete")
                            .on_hover_text("Del")
                            .clicked()
                            && let Some(name) = state.delete_selected(s)
                        {
                            state.toast(format!("Deleted {name}"));
                        }
                    });
                });
                // Delete above can have dropped it.
                if let Selection::Light(i) = state.selection
                    && i < s.lights.len()
                {
                    let audio = s.audio.enabled;
                    key_scope(&s.animation, &format!("lights/{i}"));
                    editor::light_ui(ui, &mut s.lights[i], advanced, audio);
                    key_scope(&s.animation, "");
                }
            }
            _ => note(
                ui,
                "Pick a light in the outliner or the viewport for its settings. + in the outliner adds one.",
            ),
        }
        crate::ui::skin::separator(ui);
        // Track 19 audio-reactive lights (see audio.rs).
        feature(
            ui,
            "Audio: lights follow the music",
            "Lights pulse with whatever sound the PC is playing (not the microphone). Each light follows bass, mid or treble (choose per light under Follows); the main light defaults to bass. The lighting changes every frame, so the picture never settles on its own: set Render → Reconstruction → Denoiser to Ray Reconstruction. The scene's own light settings are not changed.",
            &mut s.audio.enabled,
        )
        .body(|ui| {
            let a = &mut s.audio;
        if a.enabled {
            grid("audio").show(ui, |ui| {
                key_mark("audio/gain");
                row(ui, "Gain", "Added intensity at full level (×)", |ui| {
                    ui.add(crate::slider(&mut a.gain, 0. ..=8.));
                });
                key_mark("audio/floor");
                row(ui, "Floor", "Intensity at silence (×)", |ui| {
                    ui.add(crate::slider(&mut a.floor, 0. ..=1.));
                });
                key_mark("audio/smoothing");
                row(ui, "Smoothing", "Higher is calmer", |ui| {
                    ui.add(crate::slider(&mut a.smoothing, 0. ..=0.99));
                });
                key_mark("audio/color_shift");
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
        self.key_marks_end();
    }
    /// Object: the selected object's name, shape, placement and physics; the Room for the
    /// Stage; the camera and lens otherwise. The physics world is always at the bottom.
    pub(super) fn object_tab(&mut self, ui: &mut egui::Ui) {
        let g = self.gpu.as_ref().unwrap();
        let aspect = g.width as f32 / g.height as f32;
        let awake = self.physics.as_ref().map_or(0, |p| p.awake());
        let advanced = self.ui.advanced;
        self.key_marks_begin();
        let s = &mut self.settings;
        let state = &mut self.ui;
        let title = match state.selection {
            sel @ Selection::Object(_) => selection_label(s, sel),
            Selection::Stage => "Room".to_string(),
            _ => "Camera".to_string(),
        };
        tab_title(ui, &title);
        if std::mem::take(&mut state.scroll_to_inspector) {
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        match state.selection {
            Selection::Object(i) if i < s.objects.len() => {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.;
                    if ui
                        .add_enabled(
                            s.objects.len() < scene::MAX_OBJECTS,
                            crate::ui::skin::Button::new("Duplicate"),
                        )
                        .on_hover_text("Ctrl+D")
                        .on_disabled_hover_text(format!(
                            "Limit of {} objects reached",
                            scene::MAX_OBJECTS
                        ))
                        .clicked()
                    {
                        state.duplicate_selected(s, true);
                    }
                    if crate::ui::skin::button(ui, "Delete")
                        .on_hover_text("Del")
                        .clicked()
                        && let Some(name) = state.delete_selected(s)
                    {
                        state.toast(format!("Deleted {name}"));
                    }
                    if crate::ui::skin::button(ui, "Frame")
                        .on_hover_text("Fit the camera to this object (F)")
                        .clicked()
                    {
                        s.focus_object(i, aspect);
                    }
                });
                // Delete above can have moved or dropped it.
                if let Selection::Object(i) = state.selection
                    && i < s.objects.len()
                {
                    let physics = s.physics.enabled;
                    key_scope(&s.animation, &format!("objects/{i}"));
                    editor::object_ui(ui, &mut s.objects[i], advanced, physics);
                    portal_ui(ui, state, s, i);
                    key_scope(&s.animation, "");
                }
            }
            Selection::Stage => room_ui(ui, s, state.environment_error.is_some()),
            other => {
                if matches!(
                    other,
                    Selection::MainLight | Selection::Flashlight | Selection::Light(_)
                ) {
                    ui.horizontal_wrapped(|ui| {
                        note(
                            ui,
                            &format!(
                                "{} is a light: its settings are in",
                                selection_label(s, other)
                            ),
                        );
                        if crate::ui::skin::small_button(ui, "Lights").clicked() {
                            state.tab = Tab::Lights;
                        }
                    });
                }
                let camera = (s.yaw, s.pitch, s.distance, s.target);
                if camera_ui(ui, s) {
                    let mut d = Settings::default();
                    d.preset(self.preset);
                    (s.yaw, s.pitch, s.distance, s.target, s.fov) =
                        (d.yaw, d.pitch, d.distance, d.target, d.fov);
                }
                // Typed values are an edit, unlike an orbit: undo takes them back.
                state.camera_edit |= camera != (s.yaw, s.pitch, s.distance, s.target);
                if crate::ui::skin::button(ui, "Frame the scene (F)").clicked() {
                    self.actions.frame_scene = true;
                }
            }
        }
        crate::ui::skin::separator(ui);
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
            grid("physics").show(ui, |ui| {
                key_mark("physics/gravity");
                row(
                    ui,
                    "Gravity",
                    "Downward acceleration, scene units/s² (negative pulls up)",
                    |ui| {
                        ui.add(crate::slider(&mut p.gravity, -30. ..=30.));
                    },
                );
                key_mark("physics/speed");
                row(
                    ui,
                    "Speed",
                    "1 = real time, lower is slow motion, 0 pauses",
                    |ui| {
                        ui.add(crate::slider(&mut p.speed, 0. ..=4.));
                    },
                );
                key_mark("physics/bounce");
                row(ui, "Bounce", "Restitution of every body", |ui| {
                    ui.add(crate::slider(&mut p.bounce, 0. ..=1.));
                });
                key_mark("physics/friction");
                row(ui, "Friction", "Friction of every body", |ui| {
                    ui.add(crate::slider(&mut p.friction, 0. ..=2.));
                });
                key_mark("physics/down");
                row(
                    ui,
                    "Gravity direction",
                    "The way gravity pulls (x, y, z); straight down is 0, −1, 0",
                    |ui| {
                        crate::ui::drag_group(ui, &mut p.down, &[], "", 0.01, -1. ..=1.);
                    },
                );
                key_mark("physics/wind");
                row(
                    ui,
                    "Wind",
                    "A steady push on every body (x, y, z), scene units/s²",
                    |ui| {
                        crate::ui::drag_group(ui, &mut p.wind, &[], "", 0.05, -50. ..=50.);
                    },
                );
                key_mark("physics/gusts");
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
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.y = 4.;
                if crate::ui::skin::button(ui, "Wake all").clicked() {
                    self.actions.physics_wake = true;
                }
                if crate::ui::skin::button(ui, "Explode (E)")
                    .on_hover_text("A blast at the middle of the floor")
                    .clicked()
                {
                    self.actions.physics_blast = true;
                }
                note(ui, &format!("{awake} awake"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.y = 4.;
                if let Selection::Object(i) = state.selection
                    && i < s.objects.len()
                {
                    let frozen = s.objects[i].body == scene::Body::Fixed;
                    if crate::ui::skin::button(ui, if frozen { "Unfreeze" } else { "Freeze" })
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
                if crate::ui::skin::toggle(ui, recording, if recording { "⏺ Stop recording" } else { "⏺ Record" })
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
                    note(ui, &format!("{:.1} s", state.physics_record_time));
                }
            });
            note(ui, "N drops a ball under the pointer, Shift+N a box; the wheel moves a held body nearer or farther.");
        } else {
            note(ui, "Off: tick the box to turn it on.");
        }
        });
        self.key_marks_end();
    }
    /// Material: the selected object's, or the walls and floor with the Stage selected.
    pub(super) fn material_tab(&mut self, ui: &mut egui::Ui) {
        let advanced = self.ui.advanced;
        self.key_marks_begin();
        let s = &mut self.settings;
        let state = &mut self.ui;
        state.scroll_to_inspector = false;
        match state.selection {
            Selection::Object(i) if i < s.objects.len() => {
                tab_title(ui, &format!("{} · Material", s.objects[i].name));
                key_scope(&s.animation, &format!("objects/{i}/material"));
                editor::object_material_ui(ui, &mut s.objects[i], advanced);
            }
            Selection::Stage => {
                tab_title(ui, "Walls and floor");
                room_materials_ui(ui, s, advanced);
            }
            other => {
                tab_title(ui, "Material");
                let what = selection_label(s, other);
                note(
                    ui,
                    &if other == Selection::None {
                        "Select an object for its material, or the Stage for the walls and floor."
                            .to_string()
                    } else {
                        format!(
                            "{what} has no material. Select an object, or the Stage for the walls and floor."
                        )
                    },
                );
            }
        }
        self.key_marks_end();
    }
    /// The laser puzzle's window (Menu > Window): it opens by itself when a scene turns the
    /// puzzle on, and can then be closed.
    pub(super) fn puzzle_window(&mut self, ctx: &egui::Context) {
        let on = self.settings.puzzle.enabled;
        if on && !self.ui.puzzle_was_on {
            self.ui.show_puzzle = true;
        }
        self.ui.puzzle_was_on = on;
        if !self.ui.show_puzzle {
            return;
        }
        let (moves, solved) = (self.laser.moves, self.laser.solved);
        let s = &mut self.settings;
        let state = &mut self.ui;
        let mut open = true;
        crate::ui::skin::window("Laser puzzle")
            .open(&mut open)
            .default_width(340.)
            .default_pos(central_rect(ctx).left_top() + egui::vec2(12., 48.))
            .show_skin(ctx, |ui| {
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
            grid("puzzle").show(ui, |ui| {
                row(ui, "Origin", "Where the beam starts (x, y, z)", |ui| {
                    crate::ui::drag_group(ui, &mut p.origin, &[], "", 0.01, -40. ..=40.);
                });
                row(
                    ui,
                    "Direction",
                    "The beam's direction (x, y, z), any length",
                    |ui| {
                        crate::ui::drag_group(ui, &mut p.direction, &[], "", 0.01, -1. ..=1.);
                    },
                );
                row(ui, "Target", "The object the beam must reach", |ui| {
                    let name = |i: i32| {
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| s.objects.get(i))
                            .map_or("None".to_string(), |o| o.name.clone())
                    };
                    crate::ui::skin::ComboBox::from_id_salt("puzzle target")
                        .selected_text(name(p.target))
                        .show_ui(ui, |ui| {
                            for i in -1..s.objects.len() as i32 {
                                crate::ui::skin::selectable_value(ui, &mut p.target, i, name(i));
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
                                    if crate::ui::skin::small_button(ui, "×").clicked() {
                                        remove = Some(k);
                                    }
                                });
                            }
                            if let Some(k) = remove {
                                p.targets.remove(k);
                            }
                            if p.targets.len() < 16 {
                                crate::ui::skin::menu_button_raised(ui, "+ Target", |ui| {
                                    for i in 0..s.objects.len() as i32 {
                                        if i != p.target
                                            && !p.targets.contains(&i)
                                            && crate::ui::skin::button(ui, name(i)).clicked()
                                        {
                                            p.targets.push(i);
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                    },
                );
                row(ui, "Par", "Moves a good solution needs", |ui| {
                    crate::ui::skin::drag_value(ui, egui::DragValue::new(&mut p.par).range(0..=99));
                });
                check_row(ui, "Beam in image", "Also renders the beam as thin glowing rods, so it lights what it passes and shows in captures and videos", &mut p.beam);
                row(
                    ui,
                    "Colour",
                    "The laser's colour: its red, green and blue are filtered by coloured glass and split apart by glass with dispersion",
                    |ui| {
                        crate::ui::skin::color_rgb(ui, &mut p.color);
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
            note(ui, &format!("{status} · {moves} moves · par {}{best}", p.par));
            // Level select: the puzzle pack's scenes with their best scores.
            let levels: Vec<(std::path::PathBuf, String)> = state
                .scenes
                .iter()
                .filter(|e| e.path.components().any(|c| c.as_os_str() == "puzzles"))
                .map(|e| (e.path.clone(), e.title.clone()))
                .collect();
            if !levels.is_empty() {
                crate::ui::sticky_menu(ui, "Levels", |ui| {
                    for (path, title) in levels {
                        let best = state
                            .prefs
                            .puzzle_best
                            .iter()
                            .find(|(k, _)| **k == title)
                            .map_or("not solved".to_string(), |(_, b)| format!("best {b}"));
                        if crate::ui::skin::button(ui, format!("{title} · {best}")).clicked() {
                            state.open_pending = Some(path);
                            ui.close();
                        }
                    }
                });
            }
        } else {
            note(ui, "Off: tick the box to turn it on.");
        }
        });
            });
        self.ui.show_puzzle = open;
    }
}
/// One row of the outliner's tree. The Stage's children (Camera, Sky, Main light, Flashlight)
/// follow it; `Lights` and `Objects` are the headers of the two groups that can be closed.
#[derive(Clone, Copy, PartialEq, Debug)]
enum OutlinerRow {
    Stage,
    Camera,
    Sky,
    MainLight,
    Flashlight,
    Lights,
    Light(usize),
    Objects,
    Object(usize),
}
/// The tree in order. A filter narrows every row by name (a light's row also answers to
/// "light"), drops the group headers that have nothing left under them and opens closed groups;
/// `closed` is the Area lights and Objects groups.
fn outliner_rows(s: &scene::Settings, search: &str, closed: [bool; 2]) -> Vec<OutlinerRow> {
    let search = search.to_lowercase();
    let filtered = !search.is_empty();
    let hit = |name: &str| name.to_lowercase().contains(&search);
    let fixed = [
        ("stage", OutlinerRow::Stage),
        ("camera", OutlinerRow::Camera),
        ("sky", OutlinerRow::Sky),
        ("main light", OutlinerRow::MainLight),
        ("flashlight", OutlinerRow::Flashlight),
    ];
    let mut rows: Vec<OutlinerRow> = fixed
        .into_iter()
        .filter(|(name, _)| hit(name))
        .map(|(_, row)| row)
        .collect();
    let lights: Vec<OutlinerRow> = s
        .lights
        .iter()
        .enumerate()
        .filter(|(_, l)| (filtered && "light".contains(&search)) || hit(&l.name))
        .map(|(i, _)| OutlinerRow::Light(i))
        .collect();
    if !s.lights.is_empty() && (!filtered || !lights.is_empty()) {
        rows.push(OutlinerRow::Lights);
    }
    if filtered || !closed[0] {
        rows.extend(lights);
    }
    let objects: Vec<OutlinerRow> = s
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| hit(&o.name))
        .map(|(i, _)| OutlinerRow::Object(i))
        .collect();
    if !filtered || !objects.is_empty() {
        rows.push(OutlinerRow::Objects);
    }
    if filtered || !closed[1] {
        rows.extend(objects);
    }
    rows
}
/// The outliner's row: its on/off box and a label that opens its inspector. Ticking it selects it,
/// so its settings open when it is turned on. Not transformable, so a plain click even with Shift.
fn flashlight_row(ui: &mut egui::Ui, state: &mut UiState, on: &mut bool, indent: f32) {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let tick = crate::ui::skin::checkbox_label(ui, on, "").on_hover_text("On / off (T)");
        if tick.changed() && *on {
            state.select(Selection::Flashlight);
        }
        let dim = if *on { "T" } else { "off · T" };
        let r = tree_row(ui, 0., state.is_selected(Selection::Flashlight), "Flashlight", dim)
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
            crate::ui::skin::color_rgb(ui, &mut f.color);
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
            if crate::ui::skin::small_button(ui, "Load…").clicked() {
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
            if !f.ies.is_empty() && crate::ui::skin::small_button(ui, "Clear").clicked() {
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
/// A feature's one-line header: the arrow to its settings and its on/off box. It opens when the
/// feature turns on (by the box or a scene load) and closes when it turns off; in between it
/// stays as the user leaves it.
fn feature<'u>(ui: &'u mut egui::Ui, title: &str, tip: &str, on: &mut bool) -> Feature<'u> {
    let id = ui.make_persistent_id(("feature", title));
    let key = id.with("was on");
    let was = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.;
        crate::ui::skin::chevron_toggle(ui, &mut state);
        ui.spacing_mut().item_spacing.x = 8.;
        crate::ui::skin::checkbox_label(ui, on, title).on_hover_text(tip);
        ui.data_mut(|d| d.insert_temp(key, *on));
    });
    if was != *on {
        state.set_open(*on);
    }
    Feature { state, ui }
}
/// What `feature` hands back: its open state, to show the body under the header.
struct Feature<'u> {
    state: CollapsingState,
    ui: &'u mut egui::Ui,
}
impl Feature<'_> {
    #[cfg(test)]
    fn is_open(&self) -> bool {
        self.state.is_open()
    }
    fn body<R>(mut self, add: impl FnOnce(&mut egui::Ui) -> R) {
        // In under the checkbox, with no guide line down the side.
        self.state.show_body_unindented(self.ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: 18,
                    ..Default::default()
                })
                .show(ui, add);
        });
    }
}
/// Portals pair two objects both ways. Rare, so folded away, except while it has one.
fn portal_ui(ui: &mut egui::Ui, state: &mut UiState, s: &mut Settings, i: usize) {
    let current = s.objects[i].portal;
    let mut pick = current;
    if current >= 0 {
        // Open while it has a portal, whatever the section was left at.
        let id = crate::ui::skin::fold_id(ui, "object portal");
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
        if !state.is_open() {
            state.set_open(true);
            state.store(ui.ctx());
        }
    }
    crate::ui::skin::fold_with(ui, "object portal", "Portal", String::new(), false, |ui| {
        crate::ui::skin::ComboBox::from_label("Portal to")
                .selected_text(usize::try_from(current).ok().and_then(|p| s.objects.get(p)).map_or("None".to_string(), |o| o.name.clone()))
                .show_ui(ui, |ui| {
                    crate::ui::skin::selectable_value(ui, &mut pick, -1, "None");
                    for (j, o) in s.objects.iter().enumerate().filter(|(j, _)| *j != i) {
                        crate::ui::skin::selectable_value(ui, &mut pick, j as i32, &o.name);
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
        let _ = crate::ui::discard(ctx.run_ui(input, |root| {
            egui::CentralPanel::default().show(root, |ui| add(ui));
        }));
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
    fn the_outliner_is_one_tree_and_the_filter_narrows_all_of_it() {
        let mut s = scene::Settings::default();
        s.objects.clear();
        s.lights.clear();
        s.lights
            .push(editor::new_light(scene::LightKind::ALL[0], 1));
        s.lights[0].name = "Window".into();
        s.objects.push(editor::new_object(scene::Shape::ALL[0], 0));
        s.objects[0].name = "Red chair".into();
        use OutlinerRow as R;
        let open = [false; 2];
        assert_eq!(
            outliner_rows(&s, "", open),
            [
                R::Stage,
                R::Camera,
                R::Sky,
                R::MainLight,
                R::Flashlight,
                R::Lights,
                R::Light(0),
                R::Objects,
                R::Object(0)
            ]
        );
        // Closed groups keep their header; a filter looks inside them all the same.
        assert_eq!(
            outliner_rows(&s, "", [true, true]),
            [
                R::Stage,
                R::Camera,
                R::Sky,
                R::MainLight,
                R::Flashlight,
                R::Lights,
                R::Objects
            ]
        );
        assert_eq!(
            outliner_rows(&s, "CHAIR", [true, true]),
            [R::Objects, R::Object(0)]
        );
        assert_eq!(outliner_rows(&s, "wind", open), [R::Lights, R::Light(0)]);
        // "light" is on the light's row, whatever its name.
        assert_eq!(
            outliner_rows(&s, "light", open),
            [R::MainLight, R::Flashlight, R::Lights, R::Light(0)]
        );
        // No area lights: no empty group. Objects keeps its header (it holds the count).
        s.lights.clear();
        assert_eq!(outliner_rows(&s, "", open)[5..], [R::Objects, R::Object(0)]);
    }
    #[test]
    fn a_feature_header_opens_once_when_turned_on() {
        let ctx = egui::Context::default();
        let open = std::cell::Cell::new(false);
        let id = std::cell::Cell::new(egui::Id::NULL);
        let mut on = false;
        let draw = |on: &mut bool| {
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
        click(&ctx, tick, |ui| flashlight_row(ui, &mut state, &mut on, 0.));
        assert!(on);
        assert_eq!(state.selection, Selection::Flashlight);
        state.select(Selection::Stage);
        click(&ctx, tick, |ui| flashlight_row(ui, &mut state, &mut on, 0.));
        assert!(!on);
        assert_eq!(state.selection, Selection::Stage);
    }
    #[test]
    fn the_flashlight_label_selects_it_without_switching_it() {
        let ctx = egui::Context::default();
        let mut state = UiState::default();
        let mut on = false;
        click(&ctx, egui::pos2(60., 18.), |ui| {
            flashlight_row(ui, &mut state, &mut on, 0.)
        });
        assert!(!on);
        assert_eq!(state.selection, Selection::Flashlight);
        assert!(state.targets().is_empty(), "gizmos have nothing to move");
    }
}
