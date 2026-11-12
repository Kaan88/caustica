//! Film-look helpers (roadmap 5c): the .cube 3D LUT reader and the colour-wheel widget.
use anyhow::{Context, Result, bail, ensure};

/// Reads a .cube 3D LUT: `LUT_3D_SIZE n`, then n³ "r g b" lines, red fastest. 1D LUTs and
/// domains other than 0..1 are rejected.
pub fn read_cube(text: &str) -> Result<(u32, Vec<[f32; 3]>)> {
    let mut size = None;
    let mut data = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("TITLE") {
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            Some("LUT_3D_SIZE") => {
                size = Some(
                    words
                        .next()
                        .context("LUT_3D_SIZE needs a value")?
                        .parse::<u32>()?,
                )
            }
            Some("LUT_1D_SIZE") => bail!("1D LUTs are not supported"),
            Some("DOMAIN_MIN") => ensure!(
                words.all(|w| w.parse::<f32>().ok() == Some(0.)),
                "Only DOMAIN_MIN 0 0 0"
            ),
            Some("DOMAIN_MAX") => ensure!(
                words.all(|w| w.parse::<f32>().ok() == Some(1.)),
                "Only DOMAIN_MAX 1 1 1"
            ),
            Some(first) => {
                let r: f32 = first
                    .parse()
                    .with_context(|| format!("Unexpected line: {line}"))?;
                let g: f32 = words.next().context("Missing green")?.parse()?;
                let b: f32 = words.next().context("Missing blue")?.parse()?;
                data.push([r, g, b]);
            }
            None => {}
        }
    }
    let size = size.context("No LUT_3D_SIZE")?;
    ensure!(
        (2..=128).contains(&size),
        "LUT size {size} out of range 2..128"
    );
    ensure!(
        data.len() == (size * size * size) as usize,
        "Expected {} entries, found {}",
        size * size * size,
        data.len()
    );
    Ok((size, data))
}

/// One of the grade's colour wheels: `value` is the colour, `center` its neutral value (0 for
/// lift, 1 for gamma and gain), `range` how far the rim and the level slider reach.
pub struct Wheel<'a> {
    pub label: &'a str,
    pub value: &'a mut [f32; 3],
    pub center: f32,
    pub range: f32,
}

/// The board's edge colour, the ring around each wheel.
const EDGE: egui::Color32 = egui::Color32::from_rgb(0x0c, 0x0c, 0x0d);

/// The wheels for lift / gamma / gain, side by side in equal cells: drag a dot to tint,
/// double-click a wheel to reset it. Under them, a row for each wheel's overall level (a
/// slider, double-click to reset). Returns true when a value changed.
pub fn color_wheels(ui: &mut egui::Ui, wheels: &mut [Wheel<'_>]) -> bool {
    let e1 = [2. / 6f32.sqrt(), -1. / 6f32.sqrt(), -1. / 6f32.sqrt()];
    let e2 = [0., 1. / 2f32.sqrt(), -1. / 2f32.sqrt()];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // The tint's point on the wheel (screen axes, 1 at the rim) and the level, per wheel.
    let mut state: Vec<(egui::Vec2, f32)> = wheels
        .iter()
        .map(|w| {
            let v = *w.value;
            let mean = (v[0] + v[1] + v[2]) / 3.;
            let chroma = [v[0] - mean, v[1] - mean, v[2] - mean];
            (
                egui::vec2(dot(chroma, e1), -dot(chroma, e2)) / w.range,
                mean - w.center,
            )
        })
        .collect();
    let mut changed = vec![false; wheels.len()];
    ui.columns(wheels.len(), |columns| {
        for (i, ui) in columns.iter_mut().enumerate() {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(wheels[i].label)
                        .small()
                        .color(crate::ui::skin::DIM),
                );
                let size = ui.available_width().clamp(48., 96.);
                let (rect, r) =
                    ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
                let c = rect.center();
                let radius = size * 0.5 - 2.;
                // Hue ring as a triangle fan with per-vertex colours.
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(c, egui::Color32::from_gray(70));
                let n = 48;
                for k in 0..=n {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU;
                    let rgb = [0., 1., 2.]
                        .map(|j: f32| 0.5 + 0.5 * (a - j * std::f32::consts::TAU / 3.).cos());
                    let color = egui::Color32::from_rgb(
                        (rgb[0] * 200.) as u8,
                        (rgb[1] * 200.) as u8,
                        (rgb[2] * 200.) as u8,
                    );
                    mesh.colored_vertex(c + egui::vec2(a.cos(), -a.sin()) * radius, color);
                    if k > 0 {
                        mesh.add_triangle(0, k, k + 1);
                    }
                }
                ui.painter().add(egui::Shape::mesh(mesh));
                ui.painter()
                    .circle_stroke(c, radius + 0.5, egui::Stroke::new(1_f32, EDGE));
                if (r.dragged() || r.clicked())
                    && let Some(p) = r.interact_pointer_pos()
                {
                    let v = (p - c) / radius;
                    state[i].0 = if v.length() > 1. { v.normalized() } else { v };
                    changed[i] = true;
                }
                if r.double_clicked() {
                    state[i] = (egui::Vec2::ZERO, 0.);
                    changed[i] = true;
                }
                let at = c + state[i].0 * radius;
                ui.painter().circle_filled(at, 4., egui::Color32::WHITE);
                ui.painter()
                    .circle_stroke(at, 5., egui::Stroke::new(1_f32, EDGE));
            });
        }
    });
    crate::ui::grid("grade wheels").show(ui, |ui| {
        for (i, w) in wheels.iter().enumerate() {
            crate::ui::row(ui, w.label, "", |ui| {
                let r =
                    ui.add(crate::slider(&mut state[i].1, -w.range..=w.range).fixed_decimals(2));
                // The level is a local, so the double-click reset is done here.
                if crate::slider_double_clicked(ui, &r) {
                    state[i] = (egui::Vec2::ZERO, 0.);
                    changed[i] = true;
                } else {
                    changed[i] |= r.changed();
                }
            });
        }
    });
    for (i, w) in wheels.iter_mut().enumerate() {
        if changed[i] {
            let (point, level) = state[i];
            // Screen y is down, so the second axis flips.
            let (x, y) = (point.x * w.range, -point.y * w.range);
            for k in 0..3 {
                w.value[k] = w.center + level + x * e1[k] + y * e2[k];
            }
        }
    }
    changed.contains(&true)
}

#[cfg(test)]
mod tests {
    use super::read_cube;
    #[test]
    fn cube_files_parse() {
        let mut text = String::from("TITLE \"identity\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text += &format!("{r} {g} {b}\n");
                }
            }
        }
        let (size, data) = read_cube(&text).unwrap();
        assert_eq!(size, 2);
        assert_eq!(data[1], [1., 0., 0.]);
        assert_eq!(data[7], [1., 1., 1.]);
        assert!(read_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
    }
}
