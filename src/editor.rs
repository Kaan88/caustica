//! Inspectors for the Scene tab: objects, area lights and materials. Every range matches
//! `Settings::validate`, and every slider goes through `crate::slider` (clamps edits only).
use crate::scene::{AreaLight, LightKind, Material, Object, Pattern, Shape};
use crate::ui::{more, row};

pub fn pattern_label(p: Pattern) -> &'static str {
    match p {
        Pattern::Solid => "Solid",
        Pattern::Checker => "Checker",
        Pattern::Stripes => "Stripes",
        Pattern::Marble => "Marble",
    }
}
/// A texture set (core track 5): pick any map of a set (or a plain image), clear it, and its
/// scale. Picking one sets colour, roughness and metal to the maps' averages.
fn texture_row(ui: &mut egui::Ui, m: &mut Material) {
    row(
        ui,
        "Texture",
        "An image, or one map of a set (ambientCG / Poly Haven names: Color, NormalGL, \
         Roughness, Metalness): the set's other maps beside it load too. Box-mapped onto \
         shapes, meshes, the floor and the walls; colour, roughness and metal then scale it.",
        |ui| {
            if m.texture.is_empty() {
                ui.weak("none");
            } else {
                let name = std::path::Path::new(&m.texture)
                    .file_name()
                    .map_or(m.texture.clone(), |n| n.to_string_lossy().to_string());
                ui.label(name).on_hover_text(&m.texture);
                if let Err(e) = crate::texture::load(&m.texture) {
                    ui.label(
                        egui::RichText::new("missing").color(egui::Color32::from_rgb(232, 176, 72)),
                    )
                    .on_hover_text(format!("{e:#}"));
                }
            }
            if ui.small_button("Load…").clicked() {
                let mut dialog =
                    rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png"]);
                if let Some(folder) = crate::scenes::folder().map(|f| f.join("textures"))
                    && folder.is_dir()
                {
                    dialog = dialog.set_directory(folder);
                }
                if let Some(path) = dialog.pick_file() {
                    m.texture = crate::mesh::store(&path);
                    if let Ok(maps) = crate::texture::load(&m.texture) {
                        crate::texture::apply_averages(m, &maps);
                    }
                }
            }
            if !m.texture.is_empty() && ui.small_button("Clear").clicked() {
                m.texture.clear();
            }
        },
    );
    if !m.texture.is_empty() {
        row(
            ui,
            "Texture scale",
            "Repeats per scene unit (the Cornell box is 2 wide).",
            |ui| {
                ui.add(crate::slider(&mut m.texture_scale, 0.01..=100.).logarithmic(true));
            },
        );
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
            texture_row(ui, m);
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
    more(ui, (id, "material layers"), advanced, |ui| {
        egui::Grid::new((id, "material layers"))
                .num_columns(2)
                .spacing([12., 6.])
                .show(ui, |ui| {
                    row(
                        ui,
                        "Clearcoat",
                        "A clear lacquer over everything (car paint, varnish); it covers the surface beneath.",
                        |ui| {
                            ui.add(crate::slider(&mut m.clearcoat, 0. ..=1.));
                        },
                    );
                    row(
                        ui,
                        "Coat roughness",
                        "Roughness of the coat itself: 0 is a wet look, 1 a dusty one.",
                        |ui| {
                            ui.add(crate::slider(&mut m.clearcoat_roughness, 0. ..=1.));
                        },
                    );
                    row(
                        ui,
                        "Sheen",
                        "Micro-fibres facing out (cloth, velvet, peach fuzz): a grazing-angle highlight.",
                        |ui| {
                            ui.add(crate::slider(&mut m.sheen, 0. ..=1.));
                        },
                    );
                    row(
                        ui,
                        "Sheen roughness",
                        "How wide the sheen highlight is; the fibre direction.",
                        |ui| {
                            ui.add(crate::slider(&mut m.sheen_roughness, 0. ..=1.));
                        },
                    );
                    row(
                        ui,
                        "Anisotropy",
                        "Brushed-metal stretch of the highlight along the grooves.",
                        |ui| {
                            ui.add(crate::slider(&mut m.anisotropy, 0. ..=1.));
                        },
                    );
                    row(
                        ui,
                        "Groove rotation",
                        "The groove direction, degrees in the surface plane.",
                        |ui| {
                            ui.add(crate::slider(&mut m.anisotropy_rotation, 0. ..=180.));
                        },
                    );
                    row(
                        ui,
                        "Film thickness",
                        "Thin-film interference: the oil/film thickness in nanometres (a soap film is ~400; 0 turns it off); a pattern varies it across the surface.",
                        |ui| {
                            ui.add(crate::slider(&mut m.film_thickness, 0. ..=4000.).suffix(" nm"));
                        },
                    );
                    row(
                        ui,
                        "Film IOR",
                        "Refractive index of the interfering film above the surface.",
                        |ui| {
                            ui.add(crate::slider(&mut m.film_ior, 1. ..=2.5));
                        },
                    );
                    row(
                        ui,
                        "Grating period",
                        "Diffraction grating: the groove spacing in nanometres (a CD is 1600); 0 turns it off.",
                        |ui| {
                            ui.add(crate::slider(&mut m.grating, 0. ..=4000.).suffix(" nm"));
                        },
                    );
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
    if crate::mesh::has_maps(o) {
        ui.checkbox(&mut o.material.textures, "Texture maps").on_hover_text(
            "Base colour, normal and metallic-roughness maps from the mesh file. Colour,              roughness and metallic below start at the maps' averages and scale them; off              shows those averages as flat values.",
        );
    }
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
            row(
                ui,
                "Show outline",
                "Draw the light's rectangle over the viewport, to see and grab it. The selected light always shows it.",
                |ui| {
                    ui.checkbox(&mut l.outline, "");
                },
            );
            row(ui, "Kind", "Rectangle, disk and spot lights shine from their front; a sphere all round; a sun from far away in one direction, with sharp shadows when its angle is small.", |ui| {
                egui::ComboBox::from_id_salt("light kind")
                    .selected_text(l.kind.label())
                    .show_ui(ui, |ui| {
                        for k in LightKind::ALL {
                            ui.selectable_value(&mut l.kind, k, k.label());
                        }
                    });
            });
            let intensity = match l.kind {
                LightKind::Rect => "Brightness of the panel (its radiance).",
                LightKind::Sun => "Brightness of a white surface facing the sun; independent of its angle.",
                _ => "Brightness as a 1 × 1 panel; it does not change with the size.",
            };
            row(ui, "Intensity", intensity, |ui| {
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
            match l.kind {
                LightKind::Sphere => row(
                    ui,
                    "Diameter",
                    "In scene units. Bigger lights give softer shadows.",
                    |ui| {
                        ui.add(
                            egui::DragValue::new(&mut l.size[0])
                                .speed(0.01)
                                .range(0.05..=40.),
                        );
                    },
                ),
                LightKind::Sun => row(
                    ui,
                    "Angle",
                    "Angular diameter of the sun's disk; the real sun is 0.53°. Bigger gives softer shadows. Position only places its handle.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut l.angle, 0.05..=30.)
                                .logarithmic(true)
                                .suffix("°"),
                        );
                    },
                ),
                _ => row(
                    ui,
                    "Size",
                    "Width and depth in scene units (a disk's diameters). Bigger lights give softer shadows.",
                    |ui| {
                        for p in &mut l.size {
                            ui.add(egui::DragValue::new(p).speed(0.025).range(0.05..=40.));
                        }
                    },
                ),
            }
            if l.kind == LightKind::Spot {
                row(ui, "Spread", "Full angle of the cone of light.", |ui| {
                    ui.add(crate::slider(&mut l.spread, 1. ..=179.).suffix("°"));
                });
                row(
                    ui,
                    "Blend",
                    "How much of the cone fades out towards its edge: 0 is a hard edge.",
                    |ui| {
                        ui.add(crate::slider(&mut l.blend, 0. ..=1.));
                    },
                );
            }
            // Where a spot or sun points matters more than a panel's tilt.
            if matches!(l.kind, LightKind::Spot | LightKind::Sun) {
                rotation_row(ui, l);
            }
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
                if !matches!(l.kind, LightKind::Spot | LightKind::Sun) {
                    rotation_row(ui, l);
                }
            });
    });
}
fn rotation_row(ui: &mut egui::Ui, l: &mut AreaLight) {
    row(
        ui,
        "Rotation",
        "Pitch, yaw and roll in degrees; at 0 the light points straight down.",
        |ui| {
            for p in &mut l.rotation {
                ui.add(
                    egui::DragValue::new(p)
                        .speed(1.)
                        .range(-360. ..=360.)
                        .suffix("°"),
                );
            }
        },
    );
}
/// A new light of `kind`, numbered `n` in its name: panels and spots above the stage, a bulb
/// inside the Cornell box's volume, a sun from the upper left.
pub fn new_light(kind: LightKind, n: usize) -> AreaLight {
    let base = AreaLight {
        kind,
        ..Default::default()
    };
    match kind {
        LightKind::Rect => AreaLight {
            name: format!("Softbox {n}"),
            ..base
        },
        LightKind::Disk => AreaLight {
            name: format!("Disk {n}"),
            size: [1., 1.],
            ..base
        },
        LightKind::Sphere => AreaLight {
            name: format!("Bulb {n}"),
            position: [0.4, 0.3, 0.3],
            size: [0.2, 0.2],
            intensity: 2.,
            ..base
        },
        LightKind::Spot => AreaLight {
            name: format!("Spot {n}"),
            size: [0.3, 0.3],
            intensity: 20.,
            spread: 45.,
            ..base
        },
        LightKind::Sun => AreaLight {
            name: format!("Sun {n}"),
            rotation: [35., -30., 0.],
            intensity: 1.,
            ..base
        },
    }
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
