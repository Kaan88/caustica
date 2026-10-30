//! Inspectors for the Scene tab: objects, area lights and materials. Every range matches
//! `Settings::validate`, and every slider goes through `crate::slider` (clamps edits only).
use crate::scene::{AreaLight, Material, Object, Pattern, Shape};
use crate::ui::{more, row};

pub fn pattern_label(p: Pattern) -> &'static str {
    match p {
        Pattern::Solid => "Solid",
        Pattern::Checker => "Checker",
        Pattern::Stripes => "Stripes",
        Pattern::Marble => "Marble",
    }
}
pub fn material_ui(ui: &mut egui::Ui, id: &str, m: &mut Material, advanced: bool) {
    egui::ComboBox::from_id_salt((id, "library"))
        .selected_text("Apply a material…")
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (i, name) in crate::presets::MATERIAL_NAMES.iter().enumerate() {
                if ui.selectable_label(false, *name).clicked() {
                    *m = crate::presets::material(i);
                }
            }
        });
    egui::Grid::new((id, "material"))
        .num_columns(2)
        .spacing([12., 6.])
        .show(ui, |ui| {
            row(ui, "Color", "Linear RGB base color.", |ui| {
                ui.color_edit_button_rgb(&mut m.color);
            });
            row(
                ui,
                "Roughness",
                "0 is a sharp mirror-like highlight, 1 fully matte.",
                |ui| {
                    ui.add(crate::slider(&mut m.roughness, 0. ..=1.));
                },
            );
            row(
                ui,
                "Metal",
                "1 tints reflections with the base color, like gold or copper.",
                |ui| {
                    ui.add(crate::slider(&mut m.metallic, 0. ..=1.));
                },
            );
            row(
                ui,
                "Glass",
                "Fraction of light that passes through (transmission).",
                |ui| {
                    ui.add(crate::slider(&mut m.transmission, 0. ..=1.));
                },
            );
            row(
                ui,
                "Glow",
                "Emitted light; glowing objects light the scene.",
                |ui| {
                    ui.add(crate::slider(&mut m.emission, 0. ..=20.).suffix("×"));
                },
            );
        });
    more(ui, (id, "material more"), advanced, |ui| {
        egui::Grid::new((id, "material advanced"))
            .num_columns(2)
            .spacing([12., 6.])
            .show(ui, |ui| {
                row(
                    ui,
                    "Reflection",
                    "Strength of the clear specular reflection on non-metals.",
                    |ui| {
                        ui.add(crate::slider(&mut m.reflectivity, 0. ..=1.));
                    },
                );
                row(
                    ui,
                    "Index of refraction",
                    "Glass 1.5, water 1.33, diamond 2.42.",
                    |ui| {
                        ui.add(crate::slider(&mut m.ior, 1. ..=2.5));
                    },
                );
                row(
                    ui,
                    "Glow only",
                    "The surface only emits light; rays stop there.",
                    |ui| {
                        ui.checkbox(&mut m.unlit, "");
                    },
                );
                row(
                    ui,
                    "Pattern",
                    "Procedural pattern, evaluated in world space.",
                    |ui| {
                        egui::ComboBox::from_id_salt((id, "pattern"))
                            .selected_text(pattern_label(m.pattern))
                            .show_ui(ui, |ui| {
                                for p in [
                                    Pattern::Solid,
                                    Pattern::Checker,
                                    Pattern::Stripes,
                                    Pattern::Marble,
                                ] {
                                    ui.selectable_value(&mut m.pattern, p, pattern_label(p));
                                }
                            });
                    },
                );
                if m.pattern != Pattern::Solid {
                    row(ui, "Second color", "The pattern's other color.", |ui| {
                        ui.color_edit_button_rgb(&mut m.secondary_color);
                    });
                    row(ui, "Pattern scale", "Repetitions per scene unit.", |ui| {
                        ui.add(crate::slider(&mut m.pattern_scale, 0.1..=100.).logarithmic(true));
                    });
                }
            });
    });
}
pub fn shape_combo(ui: &mut egui::Ui, id: &str, shape: &mut Shape) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(shape.label())
        .show_ui(ui, |ui| {
            for s in Shape::ALL {
                ui.selectable_value(shape, s, s.label());
            }
        });
}
pub fn object_ui(ui: &mut egui::Ui, o: &mut Object, advanced: bool) {
    ui.add(
        egui::TextEdit::singleline(&mut o.name)
            .char_limit(128)
            .desired_width(f32::INFINITY),
    );
    egui::Grid::new("object")
        .num_columns(2)
        .spacing([12., 6.])
        .show(ui, |ui| {
            if o.mesh.is_empty() {
                row(ui, "Shape", "The mesh used for this object.", |ui| {
                    let mut kind = o.kind();
                    shape_combo(ui, "object shape", &mut kind);
                    if kind != o.kind() {
                        o.shape = kind;
                        o.sphere = false;
                    }
                });
            } else {
                row(
                    ui,
                    "Mesh",
                    "An imported mesh, scaled into the same box as the built-in shapes.",
                    |ui| {
                        let name = Some(&o.mesh)
                            .filter(|m| m.starts_with(crate::procedural::PREFIX))
                            .map(std::ffi::OsStr::new)
                            .or_else(|| std::path::Path::new(&o.mesh).file_name())
                            .map_or(o.mesh.clone(), |n| n.to_string_lossy().to_string());
                        ui.label(name).on_hover_text(&o.mesh);
                        if let Err(e) = crate::mesh::load(&o.mesh) {
                            ui.label(
                                egui::RichText::new("missing")
                                    .color(egui::Color32::from_rgb(232, 176, 72)),
                            )
                            .on_hover_text(format!("{e:#}"));
                        }
                        if ui.small_button("Use a shape").clicked() {
                            o.mesh.clear();
                        }
                    },
                );
            }
            row(
                ui,
                "Position",
                "X and Z on the floor, in scene units (the Cornell box is 2 wide).",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut o.position[0])
                            .speed(0.02)
                            .range(-40. ..=40.)
                            .prefix("x "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut o.position[1])
                            .speed(0.02)
                            .range(-40. ..=40.)
                            .prefix("z "),
                    );
                },
            );
            row(
                ui,
                "Rest on floor",
                "Keep the object standing on the floor.",
                |ui| {
                    ui.checkbox(&mut o.grounded, "");
                },
            );
            row(
                ui,
                if o.grounded {
                    "Height above floor"
                } else {
                    "Height"
                },
                "Vertical position in scene units.",
                |ui| {
                    ui.add(crate::slider(&mut o.elevation, -40. ..=40.).suffix(" units"));
                },
            );
            row(ui, "Size", "Overall scale.", |ui| {
                ui.add(
                    crate::slider(&mut o.size, 0.05..=30.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            row(ui, "Turn", "Rotation around the vertical axis.", |ui| {
                ui.add(crate::slider(&mut o.rotation, -180. ..=180.).suffix("°"));
            });
        });
    more(ui, "object transform more", advanced, |ui| {
        egui::Grid::new("object advanced")
            .num_columns(2)
            .spacing([12., 6.])
            .show(ui, |ui| {
                row(
                    ui,
                    "Height scale",
                    "Stretch along the vertical axis.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut o.stretch, 0.05..=10.)
                                .logarithmic(true)
                                .suffix("×"),
                        );
                    },
                );
                row(
                    ui,
                    "Width scale",
                    "Stretch along the object's X axis.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut o.scale_xz[0], 0.01..=20.)
                                .logarithmic(true)
                                .suffix("×"),
                        );
                    },
                );
                row(
                    ui,
                    "Depth scale",
                    "Stretch along the object's Z axis.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut o.scale_xz[1], 0.01..=20.)
                                .logarithmic(true)
                                .suffix("×"),
                        );
                    },
                );
                row(ui, "Pitch", "Tilt forward or back.", |ui| {
                    ui.add(crate::slider(&mut o.tilt[0], -180. ..=180.).suffix("°"));
                });
                row(ui, "Roll", "Tilt sideways.", |ui| {
                    ui.add(crate::slider(&mut o.tilt[1], -180. ..=180.).suffix("°"));
                });
            });
    });
    ui.separator();
    ui.label(egui::RichText::new("Material").strong());
    material_ui(ui, "object", &mut o.material, advanced);
}
pub fn light_ui(ui: &mut egui::Ui, l: &mut AreaLight, advanced: bool) {
    ui.add(
        egui::TextEdit::singleline(&mut l.name)
            .char_limit(128)
            .desired_width(f32::INFINITY),
    );
    egui::Grid::new("area light")
        .num_columns(2)
        .spacing([12., 6.])
        .show(ui, |ui| {
            row(ui, "On", "Switch this light on or off.", |ui| {
                ui.checkbox(&mut l.enabled, "");
            });
            row(ui, "Intensity", "Brightness multiplier.", |ui| {
                ui.add(
                    crate::slider(&mut l.intensity, 0. ..=2000.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            row(ui, "Color", "Linear RGB color of the light.", |ui| {
                ui.color_edit_button_rgb(&mut l.color);
            });
            row(ui, "Position", "In scene units.", |ui| {
                for (p, axis) in l.position.iter_mut().zip(["x ", "y ", "z "]) {
                    ui.add(
                        egui::DragValue::new(p)
                            .speed(0.025)
                            .range(-50. ..=50.)
                            .prefix(axis),
                    );
                }
            });
            row(
                ui,
                "Size",
                "Width and depth in scene units. Bigger lights give softer shadows.",
                |ui| {
                    for p in &mut l.size {
                        ui.add(egui::DragValue::new(p).speed(0.025).range(0.05..=40.));
                    }
                },
            );
        });
    more(ui, "area light more", advanced, |ui| {
        egui::Grid::new("area light advanced")
            .num_columns(2)
            .spacing([12., 6.])
            .show(ui, |ui| {
                row(
                    ui,
                    "Visible to camera",
                    "Show the light's panel in the image.",
                    |ui| {
                        ui.checkbox(&mut l.visible, "");
                    },
                );
                row(ui, "Rotation", "Pitch, yaw and roll in degrees.", |ui| {
                    for p in &mut l.rotation {
                        ui.add(
                            egui::DragValue::new(p)
                                .speed(1.)
                                .range(-360. ..=360.)
                                .suffix("°"),
                        );
                    }
                });
            });
    });
}
pub fn new_object(shape: Shape, index: usize) -> Object {
    Object {
        name: format!("{} {}", shape.label(), index + 1),
        shape,
        sphere: false,
        position: [0., 0.],
        stretch: 1.,
        rotation: 0.,
        ..Object::default()
    }
}
