//! Inspectors for the Scene tab: objects, area lights and materials. Every range matches
//! `Settings::validate`, and every slider goes through `crate::slider` (clamps edits only).
use crate::scene::{AreaLight, LightKind, Material, Object, Pattern, Shape};
use crate::timeline_ui::key_mark;
use crate::ui::{cant_load, check_row, check_row_with, more, own_tip, row, row_if, why_disabled};

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
                own_tip(ui.label(name), &m.texture);
                if let Err(e) = crate::texture::load(&m.texture) {
                    cant_load(ui, &format!("{e:#}"));
                }
            }
            if crate::ui::skin::small_button(ui, "Load…").clicked() {
                let mut dialog =
                    rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png"]);
                if let Some(folder) = crate::dirs::scene_roots()
                    .into_iter()
                    .map(|f| f.join("textures"))
                    .find(|f| f.is_dir())
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
            if !m.texture.is_empty() && crate::ui::skin::small_button(ui, "Clear").clicked() {
                m.texture.clear();
            }
        },
    );
    if !m.texture.is_empty() {
        key_mark("texture_scale");
        row(
            ui,
            "Texture scale",
            "Repeats per scene unit (the Cornell box is 2 wide).",
            |ui| {
                ui.add(crate::slider(&mut m.texture_scale, 0.01..=100.).logarithmic(true));
            },
        );
        check_row(
            ui,
            "Mesh UVs",
            "Map the set by the mesh's own UVs (texture scale = repeats per UV unit) instead of projecting it along the axes. For imported meshes; built-in shapes have no UVs.",
            &mut m.texture_uv,
        );
    }
}
pub fn material_ui(ui: &mut egui::Ui, id: &str, m: &mut Material, advanced: bool) {
    ui.horizontal(|ui| {
        crate::balls::ball(ui, Some(id), m, 48.)
            .on_hover_text("This material on a sphere (renders in the background, cached).");
        crate::ui::skin::ComboBox::from_id_salt((id, "library"))
            .selected_text("Apply a material…")
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (i, name) in crate::presets::MATERIAL_NAMES.iter().enumerate() {
                    let preset = crate::presets::material(i);
                    let picked = ui
                        .horizontal(|ui| {
                            let r = crate::balls::ball(ui, None, &preset, 28.);
                            r.clicked() | crate::ui::skin::selectable(ui, false, *name).clicked()
                        })
                        .inner;
                    if picked {
                        *m = preset;
                    }
                }
            });
    });
    egui::Grid::new((id, "material"))
        .num_columns(2)
        .spacing([8., 4.])
        .min_col_width(crate::ui::LABEL_WIDTH)
        .show(ui, |ui| {
            key_mark("color");
            row_if(
                ui,
                "Colour",
                "Base colour (the picker edits linear RGB, not screen colour).",
                (m.spectrum > 0).then_some("A measured spectrum replaces it (Surface, below: set it to None)"),
                |ui| {
                    crate::ui::skin::color_rgb(ui, &mut m.color);
                },
            );
            key_mark("roughness");
            row(
                ui,
                "Roughness",
                "0 is a sharp mirror-like highlight, 1 fully matte.",
                |ui| {
                    ui.add(crate::slider(&mut m.roughness, 0. ..=1.));
                },
            );
            key_mark("metallic");
            row(
                ui,
                "Metal",
                "1 tints reflections with the base colour, like gold or copper.",
                |ui| {
                    ui.add(crate::slider(&mut m.metallic, 0. ..=1.));
                },
            );
            key_mark("transmission");
            row(
                ui,
                "Glass",
                "Fraction of light that passes through (transmission).",
                |ui| {
                    ui.add(crate::slider(&mut m.transmission, 0. ..=1.));
                },
            );
            key_mark("emission");
            row(
                ui,
                "Glow",
                "Emitted light; glowing objects light the scene.",
                |ui| {
                    ui.add(crate::slider(&mut m.emission, 0. ..=20.).suffix("×"));
                },
            );
            if m.emission > 0. {
                key_mark("temperature");
                row(
                    ui,
                    "Temperature",
                    "Glow colour from a heat temperature (blackbody): candles 1900 K, tungsten 2700–3200 K, daylight 6500 K, blue sky 10000 K and up. Off: the colour sets it.",
                    |ui| kelvin_ui(ui, &mut m.temperature),
                );
            }
            texture_row(ui, m);
        });
    let d = Material::default();
    // A group opens by itself when something in it is in use, so an effect is never hidden.
    let surface = m.splitter
        || m.reflectivity != d.reflectivity
        || m.ior != d.ior
        || m.spectrum != d.spectrum;
    let coatings = m.clearcoat > 0. || m.sheen > 0. || m.iridescence > 0.;
    let brushed = m.anisotropy > 0. || m.grating > 0.;
    let volume = m.dispersion > 0. || m.density > 0.;
    let pattern = m.pattern != Pattern::Solid;
    let other = m.priority != d.priority || m.unlit;
    let glass = m.transmission > 0.;
    more(ui, (id, "material more"), advanced, |ui| {
        group(ui, id, "Surface", surface, |ui| {
            key_mark("reflectivity");
            row(
                ui,
                "Reflection",
                "Strength of the clear specular reflection on non-metals.",
                |ui| {
                    ui.add(crate::slider(&mut m.reflectivity, 0. ..=1.));
                },
            );
            key_mark("ior");
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
                "Measured spectrum",
                "Replaces the colour with a measured reflectance or emission spectrum: exact with Spectral on (Render tab), its colour under daylight otherwise. The Cornell box's walls and light were measured by Cornell's Program of Computer Graphics.",
                |ui| {
                    crate::ui::skin::ComboBox::from_id_salt("measured spectrum")
                        .selected_text(
                            crate::spectrum::MEASURED_NAMES[(m.spectrum as usize)
                                .min(crate::spectrum::MEASURED_NAMES.len() - 1)],
                        )
                        .show_ui(ui, |ui| {
                            for (i, n) in crate::spectrum::MEASURED_NAMES.iter().enumerate() {
                                if crate::ui::skin::selectable_value(
                                    ui,
                                    &mut m.spectrum,
                                    i as u32,
                                    *n,
                                )
                                .changed()
                                    && i > 0
                                {
                                    m.color = crate::spectrum::measured(i as u32);
                                }
                            }
                        });
                },
            );

            key_mark("splitter");
            check_row(
                ui,
                "Beam splitter",
                "A thin partial mirror (optics bench): Reflection sets the share reflected, the rest passes straight through (front faces only: a thin box splits once).",
                &mut m.splitter,
            );
        });
        group(ui, id, "Coatings", coatings, |ui| {
            key_mark("clearcoat");
            row(
                ui,
                "Clear coat",
                "A clear varnish over the surface (car paint, lacquer): a sharp reflection on top.",
                |ui| {
                    ui.add(crate::slider(&mut m.clearcoat, 0. ..=1.));
                },
            );
            if m.clearcoat > 0. {
                key_mark("clearcoat_roughness");
                row(ui, "Coat roughness", "0 is a mirror-sharp varnish.", |ui| {
                    ui.add(crate::slider(&mut m.clearcoat_roughness, 0. ..=1.));
                });
            }
            key_mark("sheen");
            row(
                ui,
                "Sheen",
                "A soft glow at grazing angles from fibres or dust (velvet, felt, peach skin).",
                |ui| {
                    ui.add(crate::slider(&mut m.sheen, 0. ..=1.));
                },
            );
            if m.sheen > 0. {
                key_mark("sheen_color");
                row(
                    ui,
                    "Sheen colour",
                    "Colour of the sheen (the picker edits linear RGB, not screen colour).",
                    |ui| {
                        crate::ui::skin::color_rgb(ui, &mut m.sheen_color);
                    },
                );
                key_mark("sheen_roughness");
                row(
                    ui,
                    "Sheen roughness",
                    "1 spreads the sheen over the whole silhouette; near 0 it hugs the edge.",
                    |ui| {
                        ui.add(crate::slider(&mut m.sheen_roughness, 0. ..=1.));
                    },
                );
            }

            key_mark("iridescence");
            row(
                ui,
                "Thin film",
                "An iridescent film on the surface (soap bubble, oil slick, anodized titanium).",
                |ui| {
                    ui.add(crate::slider(&mut m.iridescence, 0. ..=1.));
                },
            );
            if m.iridescence > 0. {
                key_mark("iridescence_thickness");
                row(
                    ui,
                    "Film thickness",
                    "Nanometres; 200–800 gives the strongest colours, thinner fades to plain.",
                    |ui| {
                        ui.add(
                            crate::slider(&mut m.iridescence_thickness, 0. ..=2000.).suffix(" nm"),
                        );
                    },
                );
                key_mark("iridescence_ior");
                row(
                    ui,
                    "Film IOR",
                    "Soap and water 1.33, oil 1.47, titanium oxide 2.4.",
                    |ui| {
                        ui.add(crate::slider(&mut m.iridescence_ior, 1. ..=2.5));
                    },
                );
            }
        });
        group(ui, id, "Brushed and grating", brushed, |ui| {
            key_mark("anisotropy");
            row(
                ui,
                "Anisotropy",
                "Brushed metal: highlights streak across the brushing direction.",
                |ui| {
                    ui.add(crate::slider(&mut m.anisotropy, 0. ..=1.));
                },
            );
            key_mark("grating");
            row(
                ui,
                "Diffraction grating",
                "Fine grooves that split reflections into rainbows (a CD or DVD).",
                |ui| {
                    ui.add(crate::slider(&mut m.grating, 0. ..=1.));
                },
            );
            if m.grating > 0. {
                key_mark("grating_spacing");
                row(
                    ui,
                    "Groove spacing",
                    "Micrometres: CD 1.6, DVD 0.74, Blu-ray 0.32. Grooves follow the brush direction.",
                    |ui| {
                        ui.add(crate::slider(&mut m.grating_spacing, 0.3..=10.).suffix(" um"));
                    },
                );
            }
            if m.anisotropy > 0. || m.grating > 0. {
                key_mark("anisotropy_rotation");
                row(
                    ui,
                    "Brush direction",
                    "Degrees about the surface normal, from the object's x axis.",
                    |ui| {
                        ui.add(crate::slider(&mut m.anisotropy_rotation, -180. ..=180.));
                    },
                );
                key_mark("anisotropy_spun");
                check_row(
                    ui,
                    "Spun",
                    "Brushed in circles around the object's y axis (a pan, a watch dial).",
                    &mut m.anisotropy_spun,
                );
            }
        });
        if glass {
            group(ui, id, "Volume", volume, |ui| {
                let mut on = m.dispersion > 0.;
                key_mark("dispersion");
                let dispersion = check_row_with(
                    ui,
                    "Dispersion",
                    "Splits white light into a rainbow, as a prism does (only with Spectral on, in the Render tab). Set by the Abbe number: lower splits more; crown glass 60, flint 30, dense flint 20.",
                    &mut on,
                    None,
                    |ui, on| {
                        if on {
                            ui.add(crate::slider(&mut m.dispersion, 10. ..=100.));
                        }
                    },
                );
                if dispersion.changed() {
                    m.dispersion = if on { 30. } else { 0. };
                }
                key_mark("density");
                row(
                    ui,
                    "Volume density",
                    "Fills the object with a scattering medium (smoke, milk, jade): how much light it scatters per unit length. 0 is clear; the colour above still tints by absorption.",
                    |ui| {
                        ui.add(crate::slider(&mut m.density, 0. ..=100.).logarithmic(true));
                    },
                );
                if m.density > 0. {
                    key_mark("scatter_color");
                    row(
                        ui,
                        "Scatter colour",
                        "The share of light each scattering keeps, per channel.",
                        |ui| {
                            crate::ui::skin::color_rgb(ui, &mut m.scatter_color);
                        },
                    );
                    key_mark("extinction");
                    row(
                        ui,
                        "Extinction colour",
                        "How strongly each colour is scattered or absorbed, relative to the density. White treats all alike; more blue than red scatters blue out of a beam, so the object looks blue from the side and the light through it turns orange (opal glass, skimmed milk, a sunset).",
                        |ui| {
                            crate::ui::skin::color_rgb(ui, &mut m.extinction);
                            // Black would fail validation on load; it means grey.
                            if m.extinction.iter().all(|v| *v <= 0.) {
                                m.extinction = [1.; 3];
                            }
                        },
                    );
                    key_mark("phase");
                    row(
                        ui,
                        "Scatter direction",
                        "Forward glow vs back-scatter (Henyey-Greenstein g): negative scatters back, 0 evenly, positive forward (fog, milk ~0.8).",
                        |ui| {
                            ui.add(crate::slider(&mut m.phase, -0.95..=0.95));
                        },
                    );
                }
            });
        }
        group(ui, id, "Pattern", pattern, |ui| {
            key_mark("pattern");
            row(
                ui,
                "Pattern",
                "Procedural pattern, evaluated in world space.",
                |ui| {
                    crate::ui::skin::ComboBox::from_id_salt((id, "pattern"))
                        .selected_text(pattern_label(m.pattern))
                        .show_ui(ui, |ui| {
                            for p in [
                                Pattern::Solid,
                                Pattern::Checker,
                                Pattern::Stripes,
                                Pattern::Marble,
                            ] {
                                crate::ui::skin::selectable_value(
                                    ui,
                                    &mut m.pattern,
                                    p,
                                    pattern_label(p),
                                );
                            }
                        });
                },
            );
            if m.pattern != Pattern::Solid {
                key_mark("secondary_color");
                row(ui, "Second colour", "The pattern's other colour.", |ui| {
                    crate::ui::skin::color_rgb(ui, &mut m.secondary_color);
                });
                key_mark("pattern_scale");
                row(ui, "Pattern scale", "Repetitions per scene unit.", |ui| {
                    ui.add(crate::slider(&mut m.pattern_scale, 0.1..=100.).logarithmic(true));
                });
            }
        });
        group(ui, id, "Other", other, |ui| {
            key_mark("priority");
            row(
                ui,
                "Priority",
                "Where glass, water or ice overlap, the higher priority owns the overlap: \
                     model water reaching into the glass wall and rank the glass above it.",
                |ui| {
                    ui.add(crate::slider(&mut m.priority, 0..=7));
                },
            );
            key_mark("unlit");
            check_row(
                ui,
                "Glow only",
                "The surface only emits light; rays stop there.",
                &mut m.unlit,
            );
        });
    });
}
/// One collapsible block of the advanced material rows, with its own grid.
fn group(ui: &mut egui::Ui, id: &str, title: &str, active: bool, add: impl FnOnce(&mut egui::Ui)) {
    crate::ui::skin::fold_with(
        ui,
        (id, "material group", title),
        title,
        String::new(),
        active,
        |ui| {
            egui::Grid::new((id, "material group grid", title))
                .num_columns(2)
                .spacing([8., 4.])
                .min_col_width(crate::ui::LABEL_WIDTH)
                .show(ui, add);
        },
    );
}
pub fn shape_combo(ui: &mut egui::Ui, id: &str, shape: &mut Shape) {
    crate::ui::skin::ComboBox::from_id_salt(id)
        .selected_text(shape.label())
        .show_ui(ui, |ui| {
            for s in Shape::ALL {
                crate::ui::skin::selectable_value(ui, shape, s, s.label());
            }
        });
}
/// The Material tab for an object: its texture-map switch (meshes with maps) and material.
pub fn object_material_ui(ui: &mut egui::Ui, o: &mut Object, advanced: bool) {
    if crate::mesh::has_maps(o) {
        crate::ui::skin::checkbox_label(ui, &mut o.material.textures, "Texture maps").on_hover_text(
            "Base colour, normal and metallic-roughness maps from the mesh file. Colour, roughness and metallic below start at the maps' averages and scale them; off shows those averages as flat values.",
        );
    }
    material_ui(ui, "object", &mut o.material, advanced);
}
/// The Object tab for an object: its name, shape, placement and physics rows.
pub fn object_ui(ui: &mut egui::Ui, o: &mut Object, advanced: bool, physics: bool) {
    crate::ui::skin::text_edit(
        ui,
        f32::INFINITY,
        egui::TextEdit::singleline(&mut o.name).char_limit(128),
    );
    crate::ui::section(ui, "Shape and placement");
    egui::Grid::new("object")
        .num_columns(2)
        .spacing([8., 4.])
        .min_col_width(crate::ui::LABEL_WIDTH)
        .show(ui, |ui| {
            row(
                ui,
                "Surface",
                "Mesh, or an exact procedural surface intersected in the shader (no facets): spheres, tori and fractals.",
                |ui| {
                    crate::ui::skin::ComboBox::from_id_salt("object sdf").selected_text(o.sdf.label()).show_ui(ui, |ui| {
                        for k in crate::scene::Sdf::ALL {
                            // A kind's detail means something else in another (power vs radius).
                            if crate::ui::skin::selectable_value(ui, &mut o.sdf, k, k.label()).changed() {
                                o.detail = 0.;
                            }
                        }
                    });
                },
            );
            if let Some((label, range, default)) = o.sdf.detail() {
                key_mark("detail");
                row(ui, label, "The procedural surface's shape parameter (reset: its default).", |ui| {
                    let mut v = if o.detail > 0. { o.detail } else { default };
                    if ui.add(crate::slider(&mut v, range)).changed() {
                        o.detail = v;
                    }
                });
            }
            if o.sdf == crate::scene::Sdf::Metaballs {
                key_mark("phase");
                row(ui, "Phase", "Where the blobs are; key it (I) and play the timeline to animate them.", |ui| {
                    ui.add(crate::slider(&mut o.phase, 0. ..=100.));
                });
            }
            if o.cloth.is_some() {
                // The mesh is the simulation's.
            } else if o.mesh.is_empty() && o.sdf.is_none() {
                row(ui, "Shape", "The mesh used for this object.", |ui| {
                    let mut kind = o.kind();
                    shape_combo(ui, "object shape", &mut kind);
                    if kind != o.kind() {
                        o.shape = kind;
                        o.sphere = false;
                    }
                });
            } else if o.sdf.is_none() {
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
                        own_tip(ui.label(name), &o.mesh);
                        if let Err(e) = crate::mesh::load(&o.mesh) {
                            cant_load(ui, &format!("{e:#}"));
                        }
                        if crate::ui::skin::small_button(ui, "Use a shape").clicked() {
                            o.mesh.clear();
                        }
                    },
                );
            }
            key_mark("position");
            row(
                ui,
                "Position",
                "X and Z on the floor, in scene units (the Cornell box is 2 wide).",
                |ui| {
                    crate::ui::drag_group(ui, &mut o.position, &["x ", "z "], "", 0.02, -40. ..=40.);
                },
            );
            key_mark("grounded");
            check_row(ui, "Rest on floor", "Keep the object standing on the floor.", &mut o.grounded);
            let mut seen = !o.camera_hidden;
            key_mark("camera_hidden");
            if check_row(ui, "Visible to camera", "Off: camera rays pass through the object, while reflections, refractions and shadows still see it (the walk avatar's head).", &mut seen).changed() {
                o.camera_hidden = !seen;
            }
            key_mark("casts_shadows");
            check_row(ui, "Casts shadows", "Off: lights shine through the object, while the camera and reflections still see it.", &mut o.casts_shadows);
            key_mark("elevation");
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
            key_mark("size");
            row(ui, "Scale", "Overall scale.", |ui| {
                ui.add(
                    crate::slider(&mut o.size, 0.05..=30.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            key_mark("rotation");
            row(ui, "Yaw", "Rotation around the vertical axis.", |ui| {
                ui.add(crate::slider(&mut o.rotation, -180. ..=180.).suffix("°"));
            });
            // Only the simulation reads these; a cloth keeps its row, or it could not be turned off.
            if physics {
                key_mark("mass");
                row(ui, "Mass", "Density multiplier of its physics body (1 = default)", |ui| {
                    ui.add(crate::slider(&mut o.mass, 0.01..=100.).logarithmic(true).suffix("×"));
                });
                row(
                    ui,
                    "Physics",
                    "How the physics sandbox treats it: dynamic bodies fall and collide, fixed ones collide but never move, none are ignored. Strings (tethers) are never bodies.",
                    |ui| {
                        use crate::scene::Body;
                        for (b, name) in [(Body::Dynamic, "Dynamic"), (Body::Fixed, "Fixed"), (Body::None, "None")] {
                            crate::ui::skin::selectable_value(ui, &mut o.body, b, name);
                        }
                        if !o.links.is_empty() {
                            own_tip(
                                ui.label(format!("{} joint(s)", o.links.len())),
                                "Ball joints to other bodies (chains, ropes, bridges), set in the scene file.",
                            );
                        }
                    },
                );
            }
            if (physics || o.cloth.is_some())
                && o.sdf.is_none()
                && (o.mesh.is_empty() || o.cloth.is_some())
            {
                let mut on = o.cloth.is_some();
                let cloth = check_row_with(
                    ui,
                    "Cloth",
                    "Simulates the object as a cloth with the physics sandbox: a particle grid over the object's local x-z square that drapes over bodies and pushes dynamic ones back.",
                    &mut on,
                    None,
                    |ui, _| {
                        if let Some(c) = &mut o.cloth {
                            use crate::scene::ClothPin;
                            for (p, name) in [(ClothPin::None, "Free"), (ClothPin::Corners, "Corners"), (ClothPin::Edge, "Edge")] {
                                own_tip(
                                    crate::ui::skin::selectable_value(ui, &mut c.pin, p, name),
                                    "Pins: none, the two corners of the far (−z) edge, or that whole edge (a curtain).",
                                );
                            }
                            own_tip(
                                ui.add(crate::slider(&mut c.resolution, 4..=48).suffix(" ×")),
                                "Particles per side.",
                            );
                            own_tip(
                                ui.add(crate::slider(&mut c.stiffness, 0. ..=1.)),
                                "Bending stiffness: 0 silk, 1 canvas.",
                            );
                            own_tip(
                                crate::ui::skin::drag_value(ui, egui::DragValue::new(&mut c.mass)
                                        .speed(0.01)
                                        .range(0.01..=100.)
                                        .suffix(" kg"),),
                                "The cloth's total mass: what a dynamic body it touches feels.",
                            );
                            own_tip(
                                crate::ui::skin::checkbox_label(ui, &mut c.self_collision, "Self-collision"),
                                "Folds push apart instead of passing through each other.",
                            );
                            if !o.maps_from.is_empty() {
                                own_tip(ui.label("UV"), format!(
                                    "Texture coordinates of the grid's corners (u0, v0 at the pinned edge, u1, v1 opposite) for the maps of {}.",
                                    o.maps_from
                                ));
                                for u in &mut c.uv {
                                    crate::ui::skin::drag_value(ui, egui::DragValue::new(u).speed(0.01).range(-64. ..=64.));
                                }
                            }
                        }
                    },
                );
                if cloth.changed() {
                    // Ungrounded where it is: the simulation keeps the frame it starts in.
                    let centre = crate::geometry::object_frame(o).0;
                    o.grounded = false;
                    o.elevation = centre.y;
                    o.cloth = on.then(crate::scene::Cloth::default);
                    o.mesh.clear();
                }
            }
        });
    more(ui, "object transform more", advanced, |ui| {
        egui::Grid::new("object advanced")
            .num_columns(2)
            .spacing([8., 4.])
            .min_col_width(crate::ui::LABEL_WIDTH)
            .show(ui, |ui| {
                check_row(
                    ui,
                    "Locked",
                    "The gizmo can't move it (laser puzzle walls and targets)",
                    &mut o.locked,
                );
                key_mark("stretch");
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
                key_mark("scale_xz/0");
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
                key_mark("scale_xz/1");
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
                key_mark("tilt/0");
                row(ui, "Pitch", "Tilt forward or back.", |ui| {
                    ui.add(crate::slider(&mut o.tilt[0], -180. ..=180.).suffix("°"));
                });
                key_mark("tilt/1");
                row(ui, "Roll", "Tilt sideways.", |ui| {
                    ui.add(crate::slider(&mut o.tilt[1], -180. ..=180.).suffix("°"));
                });
            });
    });
}
/// A blackbody temperature: a checkbox (off = 0) and a kelvin slider.
fn kelvin_ui(ui: &mut egui::Ui, t: &mut f32) {
    let mut on = *t > 0.;
    if crate::ui::skin::checkbox_label(ui, &mut on, "").changed() {
        *t = if on { 3200. } else { 0. };
    }
    if on {
        ui.add(
            crate::slider(t, crate::spectrum::KELVIN)
                .logarithmic(true)
                .suffix(" K"),
        );
    }
}
pub fn light_ui(ui: &mut egui::Ui, l: &mut AreaLight, advanced: bool, audio: bool) {
    crate::ui::skin::text_edit(
        ui,
        f32::INFINITY,
        egui::TextEdit::singleline(&mut l.name).char_limit(128),
    );
    egui::Grid::new("area light")
        .num_columns(2)
        .spacing([8., 4.])
        .min_col_width(crate::ui::LABEL_WIDTH)
        .show(ui, |ui| {
            key_mark("enabled");
            check_row(ui, "On", "Switch this light on or off.", &mut l.enabled);
            check_row(ui, "Show outline", "Draw the light's rectangle over the viewport, to see and grab it. The selected light always shows it.", &mut l.outline);
            key_mark("kind");
            row(ui, "Kind", "Rectangle, disk and spot lights shine from their front; a sphere all round; a sun from far away in one direction, with sharp shadows when its angle is small.", |ui| {
                crate::ui::skin::ComboBox::from_id_salt("light kind")
                    .selected_text(l.kind.label())
                    .show_ui(ui, |ui| {
                        for k in LightKind::ALL {
                            crate::ui::skin::selectable_value(ui, &mut l.kind, k, k.label());
                        }
                    });
            });
            let intensity = match l.kind {
                LightKind::Rect => "Brightness of the panel (its radiance).",
                LightKind::Sun => "Brightness of a white surface facing the sun; independent of its angle.",
                _ => "Brightness as a 1 × 1 panel; it does not change with the size.",
            };
            key_mark("intensity");
            row(ui, "Intensity", intensity, |ui| {
                ui.add(
                    crate::slider(&mut l.intensity, 0. ..=2000.)
                        .logarithmic(true)
                        .suffix("×"),
                );
            });
            key_mark("color");
            row(ui, "Colour", "Colour of the light (the picker edits linear RGB, not screen colour).", |ui| {
                ui.add_enabled_ui(l.temperature == 0., |ui| {
                    why_disabled(
                        crate::ui::skin::color_rgb(ui, &mut l.color),
                        "Untick Temperature to pick a colour",
                    );
                });
            });
            key_mark("temperature");
            row(
                ui,
                "Temperature",
                "Colour from a heat temperature (blackbody): candles 1900 K, tungsten 2700–3200 K, daylight 6500 K, blue sky 10000 K and up. Exact with Spectral on. Off: the colour above sets it.",
                |ui| kelvin_ui(ui, &mut l.temperature),
            );
            if !matches!(l.kind, LightKind::Sphere | LightKind::Sun) {
                row(
                    ui,
                    "IES profile",
                    "A measured light distribution (IES file from a lamp maker): shapes the light by the angle from its front, replacing a spot's cone. Intensity sets its brightest direction. scenes/ies has a few.",
                    |ui| {
                        let name = std::path::Path::new(&l.ies)
                            .file_stem()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "None".into());
                        ui.label(name);
                        if crate::ui::skin::small_button(ui, "Load…").clicked() {
                            let mut dialog = rfd::FileDialog::new().add_filter("IES", &["ies"]);
                            if let Some(folder) = crate::dirs::scene_roots()
                                .into_iter()
                                .map(|f| f.join("ies"))
                                .find(|f| f.is_dir())
                            {
                                dialog = dialog.set_directory(folder);
                            }
                            if let Some(path) = dialog.pick_file() {
                                crate::ies::clear_cache();
                                l.ies = crate::mesh::store(&path);
                            }
                        }
                        if !l.ies.is_empty() && crate::ui::skin::small_button(ui, "Clear").clicked() {
                            l.ies.clear();
                        }
                        if !l.ies.is_empty()
                            && let Some(e) = crate::ies::load_error(&l.ies)
                        {
                            cant_load(ui, &e);
                        }
                    },
                );
            }
            key_mark("position");
            row(ui, "Position", "In scene units.", |ui| {
                crate::ui::drag_group(ui, &mut l.position, &["x ", "y ", "z "], "", 0.025, -50. ..=50.);
            });
            // A sphere's diameter is the first size; a sun has its angle instead.
            key_mark(match l.kind {
                LightKind::Sphere => "size/0",
                LightKind::Sun => "angle",
                _ => "size",
            });
            match l.kind {
                LightKind::Sphere => row(
                    ui,
                    "Diameter",
                    "In scene units. Bigger lights give softer shadows.",
                    |ui| {
                        crate::ui::drag_group(ui, &mut l.size[..1], &[], "", 0.01, 0.05..=40.);
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
                        crate::ui::drag_group(ui, &mut l.size, &[], "", 0.025, 0.05..=40.);
                    },
                ),
            }
            if l.kind == LightKind::Spot {
                key_mark("spread");
                row(ui, "Spread", "Full angle of the cone of light.", |ui| {
                    ui.add(crate::slider(&mut l.spread, 1. ..=179.).suffix("°"));
                });
                key_mark("blend");
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
            .spacing([8., 4.])
            .min_col_width(crate::ui::LABEL_WIDTH)
            .show(ui, |ui| {
                key_mark("visible");
                check_row(ui, "Visible to camera", "Show the light's panel in the image.", &mut l.visible);
                if audio {
                    row(
                        ui,
                        "Follows",
                        "What this light follows while lights follow the music (Lights > Audio)",
                        |ui| {
                            const BANDS: [&str; 6] =
                                ["Automatic", "Bass", "Mid", "Treble", "Beats", "Nothing"];
                            crate::ui::skin::ComboBox::from_id_salt("audio band")
                                .selected_text(BANDS[l.audio_band.min(5) as usize])
                                .show_ui(ui, |ui| {
                                    for (i, b) in BANDS.iter().enumerate() {
                                        crate::ui::skin::selectable_value(ui, &mut l.audio_band, i as u32, *b);
                                    }
                                });
                        },
                    );
                }
                key_mark("shadows");
                check_row(ui, "Casts shadows", "The light's own panel blocks other lights. Off: they shine through it, while reflections still see it.", &mut l.shadows);
                // A sphere looks the same turned any way, so it has no rotation to set.
                if !matches!(l.kind, LightKind::Spot | LightKind::Sun | LightKind::Sphere) {
                    rotation_row(ui, l);
                }
            });
    });
}
fn rotation_row(ui: &mut egui::Ui, l: &mut AreaLight) {
    key_mark("rotation");
    row(
        ui,
        "Rotation",
        "Pitch, yaw and roll in degrees; at 0 the light points straight down.",
        |ui| {
            crate::ui::drag_group(ui, &mut l.rotation, &[], "°", 1., -360. ..=360.);
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
