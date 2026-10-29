use crate::scene::{
    AreaLight, MAX_LIGHTS, MAX_OBJECTS, Material, Object, Pattern, Settings, Shape,
};
pub fn material_ui(ui: &mut egui::Ui, m: &mut Material) {
    egui::ComboBox::from_id_salt("material_library")
        .selected_text("Apply material preset…")
        .show_ui(ui, |ui| {
            for (i, name) in crate::presets::MATERIAL_NAMES.iter().enumerate() {
                if ui.selectable_label(false, *name).clicked() {
                    *m = crate::presets::material(i);
                }
            }
        });
    crate::color(ui, "Base color (linear)", &mut m.color);
    ui.add(crate::slider(&mut m.roughness, 0. ..=1.).text("Roughness"));
    ui.add(crate::slider(&mut m.metallic, 0. ..=1.).text("Metallic"));
    ui.add(crate::slider(&mut m.reflectivity, 0. ..=1.).text("Reflectivity"));
    ui.add(crate::slider(&mut m.transmission, 0. ..=1.).text("Transmission"));
    ui.add(crate::slider(&mut m.ior, 1. ..=2.5).text("IOR"));
    ui.add(crate::slider(&mut m.emission, 0. ..=20.).text("Emission"));
    ui.checkbox(&mut m.unlit, "Emission only (no scattering)");
    egui::ComboBox::from_label("Pattern")
        .selected_text(format!("{:?}", m.pattern))
        .show_ui(ui, |ui| {
            for p in [
                Pattern::Solid,
                Pattern::Checker,
                Pattern::Stripes,
                Pattern::Marble,
            ] {
                ui.selectable_value(&mut m.pattern, p, format!("{p:?}"));
            }
        });
    if m.pattern != Pattern::Solid {
        crate::color(ui, "Secondary color", &mut m.secondary_color);
        ui.add(crate::slider(&mut m.pattern_scale, 0.1..=30.).text("Pattern scale"));
    }
}
pub fn objects(
    ctx: &egui::Context,
    s: &mut Settings,
    selected: &mut usize,
    add_shape: &mut Shape,
    search: &mut String,
    open: &mut bool,
    aspect: f32,
) {
    egui::Window::new("Scene objects")
        .open(open)
        .default_pos([ctx.screen_rect().width() - 352., 16.])
        .default_width(322.)
        .max_height(ctx.screen_rect().height() - 32.)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("{} / {} objects", s.objects.len(), MAX_OBJECTS))
                    .color(egui::Color32::from_rgb(100, 210, 220)),
            );
            ui.add(egui::TextEdit::singleline(search).hint_text("Search objects…"));
            egui::ScrollArea::vertical()
                .id_salt("object_list")
                .max_height(175.)
                .show(ui, |ui| {
                    for (i, o) in s.objects.iter_mut().enumerate() {
                        if !o.name.to_lowercase().contains(&search.to_lowercase()) {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut o.visible, "");
                            if ui
                                .selectable_label(
                                    *selected == i,
                                    format!("{:03}  {}", i + 1, o.name),
                                )
                                .clicked()
                            {
                                *selected = i;
                            }
                        });
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("add_shape")
                    .selected_text(add_shape.label())
                    .width(105.)
                    .show_ui(ui, |ui| {
                        for shape in Shape::ALL {
                            ui.selectable_value(add_shape, shape, shape.label());
                        }
                    });
                if ui
                    .add_enabled(s.objects.len() < MAX_OBJECTS, egui::Button::new("+ Add"))
                    .clicked()
                {
                    s.objects.push(Object {
                        name: format!("{} {}", add_shape.label(), s.objects.len() + 1),
                        shape: *add_shape,
                        sphere: false,
                        position: [0., 0.],
                        stretch: 1.,
                        rotation: 0.,
                        ..Object::default()
                    });
                    *selected = s.objects.len() - 1;
                }
            });
            *selected = (*selected).min(s.objects.len().saturating_sub(1));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !s.objects.is_empty() && s.objects.len() < MAX_OBJECTS,
                        egui::Button::new("Duplicate"),
                    )
                    .clicked()
                {
                    let mut o = s.objects[*selected].clone();
                    o.name = format!("{} copy", o.name.chars().take(110).collect::<String>());
                    o.position[0] += 0.25;
                    o.position[1] += 0.15;
                    s.objects.push(o);
                    *selected = s.objects.len() - 1;
                }
                if ui
                    .add_enabled(!s.objects.is_empty(), egui::Button::new("Delete"))
                    .clicked()
                {
                    s.objects.remove(*selected);
                    *selected = (*selected).min(s.objects.len().saturating_sub(1));
                }
                if ui
                    .add_enabled(!s.objects.is_empty(), egui::Button::new("Focus"))
                    .clicked()
                {
                    s.focus_object(*selected, aspect);
                }
            });
            ui.label(egui::RichText::new("Click an object in the viewport to select it.").small());
            ui.separator();
            if let Some(o) = s.objects.get_mut(*selected) {
                ui.add(egui::TextEdit::singleline(&mut o.name).char_limit(128));
                let mut kind = o.kind();
                egui::ComboBox::from_label("Shape")
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for shape in Shape::ALL {
                            ui.selectable_value(&mut kind, shape, shape.label());
                        }
                    });
                if kind != o.kind() {
                    o.shape = kind;
                    o.sphere = false;
                }
                ui.collapsing("Transform", |ui| {
                    ui.checkbox(&mut o.grounded, "Rest on floor");
                    ui.horizontal(|ui| {
                        ui.label("Position X / Z");
                        ui.add(
                            egui::DragValue::new(&mut o.position[0])
                                .speed(0.02)
                                .range(-40. ..=40.),
                        );
                        ui.add(
                            egui::DragValue::new(&mut o.position[1])
                                .speed(0.02)
                                .range(-40. ..=40.),
                        );
                    });
                    ui.add(
                        crate::slider(&mut o.elevation, -10. ..=20.).text(if o.grounded {
                            "Lift above floor"
                        } else {
                            "World Y"
                        }),
                    );
                    // Ranges match Settings::validate, so any valid scene stays editable.
                    ui.add(
                        crate::slider(&mut o.size, 0.05..=30.)
                            .logarithmic(true)
                            .text("Size"),
                    );
                    ui.add(
                        crate::slider(&mut o.stretch, 0.05..=10.)
                            .logarithmic(true)
                            .text("Y stretch"),
                    );
                    ui.add(
                        crate::slider(&mut o.scale_xz[0], 0.01..=20.)
                            .logarithmic(true)
                            .text("X stretch"),
                    );
                    ui.add(
                        crate::slider(&mut o.scale_xz[1], 0.01..=20.)
                            .logarithmic(true)
                            .text("Z stretch"),
                    );
                    ui.add(crate::slider(&mut o.rotation, -180. ..=180.).text("Yaw"));
                    ui.add(crate::slider(&mut o.tilt[0], -180. ..=180.).text("Pitch"));
                    ui.add(crate::slider(&mut o.tilt[1], -180. ..=180.).text("Roll"));
                });
                egui::CollapsingHeader::new("Material")
                    .default_open(true)
                    .show(ui, |ui| material_ui(ui, &mut o.material));
            } else {
                ui.label("The stage is empty. Add a primitive or a classic mesh.");
            }
        });
}
pub fn lights(ui: &mut egui::Ui, s: &mut Settings) {
    ui.label("Area lights are sampled directly, along with emissive meshes.");
    let mut remove = None;
    for (i, l) in s.lights.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.collapsing(format!("{} · {}", i + 1, l.name), |ui| {
                ui.checkbox(&mut l.enabled, "Enabled");
                ui.checkbox(&mut l.visible, "Visible to camera");
                ui.add(egui::TextEdit::singleline(&mut l.name).char_limit(128));
                crate::color(ui, "Color", &mut l.color);
                ui.add(crate::slider(&mut l.intensity, 0. ..=100.).text("Intensity"));
                ui.horizontal(|ui| {
                    ui.label("Position");
                    for p in &mut l.position {
                        ui.add(egui::DragValue::new(p).speed(0.025).range(-50. ..=50.));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Width / depth");
                    for p in &mut l.size {
                        ui.add(egui::DragValue::new(p).speed(0.025).range(0.05..=40.));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Pitch / yaw / roll");
                    for p in &mut l.rotation {
                        ui.add(egui::DragValue::new(p).speed(1.).range(-360. ..=360.));
                    }
                });
                if ui.button("Remove light").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        s.lights.remove(i);
    }
    if ui
        .add_enabled(
            s.lights.len() < MAX_LIGHTS,
            egui::Button::new("+ Add area light"),
        )
        .clicked()
    {
        s.lights.push(AreaLight {
            name: format!("Softbox {}", s.lights.len() + 1),
            ..AreaLight::default()
        });
    }
}
