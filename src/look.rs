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

/// A colour wheel for lift / gamma / gain: drag the dot to tint, the slider sets the overall
/// level. `center` is the neutral value (0 for lift, 1 for gamma and gain), `range` how far the
/// rim and the slider reach. Returns true when the value changed.
pub fn color_wheel(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut [f32; 3],
    center: f32,
    range: f32,
) -> bool {
    let e1 = [2. / 6f32.sqrt(), -1. / 6f32.sqrt(), -1. / 6f32.sqrt()];
    let e2 = [0., 1. / 2f32.sqrt(), -1. / 2f32.sqrt()];
    let mean = (value[0] + value[1] + value[2]) / 3.;
    let chroma = [value[0] - mean, value[1] - mean, value[2] - mean];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut point = egui::vec2(dot(chroma, e1), -dot(chroma, e2)) / range;
    let mut level = mean - center;
    let mut changed = false;
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(label).small());
        let size = 84.;
        let (rect, r) =
            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
        let c = rect.center();
        let radius = size * 0.5 - 2.;
        // Hue ring as a triangle fan with per-vertex colours.
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(c, egui::Color32::from_gray(70));
        let n = 48;
        for i in 0..=n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let rgb =
                [0., 1., 2.].map(|k: f32| 0.5 + 0.5 * (a - k * std::f32::consts::TAU / 3.).cos());
            let color = egui::Color32::from_rgb(
                (rgb[0] * 200.) as u8,
                (rgb[1] * 200.) as u8,
                (rgb[2] * 200.) as u8,
            );
            mesh.colored_vertex(c + egui::vec2(a.cos(), -a.sin()) * radius, color);
            if i > 0 {
                mesh.add_triangle(0, i, i + 1);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        if (r.dragged() || r.clicked())
            && let Some(p) = r.interact_pointer_pos()
        {
            let v = (p - c) / radius;
            point = if v.length() > 1. { v.normalized() } else { v };
            changed = true;
        }
        if r.double_clicked() {
            point = egui::Vec2::ZERO;
            level = 0.;
            changed = true;
        }
        ui.painter()
            .circle_filled(c + point * radius, 4., egui::Color32::WHITE);
        ui.painter().circle_stroke(
            c + point * radius,
            5.,
            egui::Stroke::new(1_f32, egui::Color32::BLACK),
        );
        ui.spacing_mut().slider_width = size - 30.;
        let r = ui.add(crate::slider(&mut level, -range..=range).fixed_decimals(2));
        // The level is a local, so the double-click reset is done here.
        if crate::slider_double_clicked(ui, &r) {
            point = egui::Vec2::ZERO;
            level = 0.;
            changed = true;
        } else {
            changed |= r.changed();
        }
    });
    if changed {
        // Screen y is down, so the second axis flips.
        let (x, y) = (point.x * range, -point.y * range);
        for k in 0..3 {
            value[k] = center + level + x * e1[k] + y * e2[k];
        }
    }
    changed
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
