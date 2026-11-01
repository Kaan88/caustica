//! Path visualizer (track 14): the recording variant of `trace` re-traces the paths through one
//! pixel with this frame's sample indices and writes every event as a [`Record`] (the layout and
//! kinds are documented above `trace` in pathtrace.hlsl). This module parses them into paths,
//! draws them as 3D lines over the viewport, and shows the inspector window.
use crate::gizmo::View;
use glam::{Vec2, Vec3};

/// One recorded event, as the shader's `PathVertex` (48 B).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Record {
    pub position: [f32; 3],
    /// Kind in bits 0–7, bounce in 8–15, path index from 16.
    pub info: u32,
    pub color: [f32; 3],
    pub value: f32,
    pub direction: [f32; 3],
    pub pad: u32,
}
const _: () = assert!(size_of::<Record>() == 48);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Camera,
    Diffuse,
    Glossy,
    Reflect,
    Refract,
    Scatter,
    PortalIn,
    PortalOut,
    Pass,
    Stop,
    Escape,
}
impl Kind {
    fn from(code: u32) -> Option<Self> {
        use Kind::*;
        Some(match code {
            0 => Camera,
            1 => Diffuse,
            2 => Glossy,
            3 => Reflect,
            4 => Refract,
            5 => Scatter,
            6 => PortalIn,
            7 => PortalOut,
            8 => Pass,
            9 => Stop,
            10 => Escape,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::Camera => "Camera",
            Kind::Diffuse => "Diffuse",
            Kind::Glossy => "Glossy",
            Kind::Reflect => "Reflect (dielectric)",
            Kind::Refract => "Refract",
            Kind::Scatter => "Volume scatter",
            Kind::PortalIn => "Portal entry",
            Kind::PortalOut => "Portal exit",
            Kind::Pass => "Pass through",
            Kind::Stop => "Stops here",
            Kind::Escape => "Escapes",
        }
    }
    fn color(self) -> egui::Color32 {
        use egui::Color32 as C;
        match self {
            Kind::Camera => C::WHITE,
            Kind::Diffuse => C::from_rgb(120, 200, 255),
            Kind::Glossy => C::from_rgb(255, 180, 90),
            Kind::Reflect => C::from_rgb(210, 210, 255),
            Kind::Refract => C::from_rgb(110, 255, 210),
            Kind::Scatter => C::from_rgb(200, 140, 255),
            Kind::PortalIn | Kind::PortalOut => C::from_rgb(255, 120, 255),
            Kind::Pass => C::from_gray(150),
            Kind::Stop => C::from_rgb(255, 90, 90),
            Kind::Escape => C::from_rgb(150, 220, 255),
        }
    }
}

/// Why a path ended (the end record's value): a short name for the list, and the long one.
pub const END_REASONS: [(&str, &str); 6] = [
    ("escaped", "left the scene"),
    ("light", "hit an unlit surface (a light)"),
    ("roulette", "Russian roulette"),
    ("max bounces", "the bounce limit"),
    ("absorbed", "its sample went below the surface"),
    ("lens", "blocked by the lens (cat-eye)"),
];

#[derive(Clone, Debug)]
pub struct Vertex {
    pub kind: Kind,
    pub bounce: u32,
    pub position: Vec3,
    /// Throughput arriving at the vertex.
    pub throughput: Vec3,
    /// Luminance this vertex adds (emission or the environment).
    pub emitted: f32,
    /// The way out (zero where the path stops).
    pub direction: Vec3,
}
/// A light sample (next-event estimation) from a vertex.
#[derive(Clone, Debug)]
pub struct LightSample {
    /// Index of the vertex it was taken from (it is recorded before that vertex).
    pub vertex: usize,
    pub from: Vec3,
    pub direction: Vec3,
    pub distance: f32,
    pub contribution: Vec3,
    pub blocked: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Path {
    pub vertices: Vec<Vertex>,
    pub lights: Vec<LightSample>,
    pub radiance: Vec3,
    /// Index into [`END_REASONS`]; None if the recording ran out of space.
    pub end: Option<usize>,
}

pub fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Groups records by path index (the shader writes them in order, path by path).
pub fn parse(records: &[Record]) -> Vec<Path> {
    let mut paths: Vec<Path> = vec![];
    for r in records {
        let index = (r.info >> 16) as usize;
        if index >= paths.len() {
            paths.resize_with(index + 1, Path::default);
        }
        let path = &mut paths[index];
        let (kind, bounce) = (r.info & 0xff, (r.info >> 8) & 0xff);
        let (position, color, direction) = (
            Vec3::from(r.position),
            Vec3::from(r.color),
            Vec3::from(r.direction),
        );
        match kind {
            11 | 12 => path.lights.push(LightSample {
                vertex: path.vertices.len(),
                from: position,
                direction,
                distance: r.value,
                contribution: color,
                blocked: kind == 12,
            }),
            13 => {
                path.radiance = color;
                path.end = Some((r.value as usize).min(END_REASONS.len() - 1));
            }
            _ => {
                if let Some(kind) = Kind::from(kind) {
                    path.vertices.push(Vertex {
                        kind,
                        bounce,
                        position,
                        throughput: color,
                        emitted: r.value,
                        direction,
                    });
                }
            }
        }
    }
    paths
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Coloring {
    Bounce,
    Throughput,
    Event,
}

pub struct PathView {
    pub open: bool,
    /// Paths to record per click.
    pub count: u32,
    /// The output pixel of the recording shown.
    pub pixel: Option<[u32; 2]>,
    request: Option<[u32; 2]>,
    /// The pixel of the request the GPU is recording.
    in_flight: Option<[u32; 2]>,
    pub paths: Vec<Path>,
    /// One path to inspect (the others are drawn faintly), or all.
    pub selected: Option<usize>,
    /// Vertices of the selected path drawn (the scrubber).
    pub upto: usize,
    pub coloring: Coloring,
    pub light_samples: bool,
}
impl Default for PathView {
    fn default() -> Self {
        Self {
            open: false,
            count: 16,
            pixel: None,
            request: None,
            in_flight: None,
            paths: vec![],
            selected: None,
            upto: usize::MAX,
            coloring: Coloring::Bounce,
            light_samples: true,
        }
    }
}
impl PathView {
    /// Records the paths through `pixel` (output pixels) in the next frame.
    pub fn request(&mut self, pixel: [u32; 2]) {
        self.open = true;
        self.request = Some(pixel);
    }
    pub fn take_request(&mut self) -> Option<([u32; 2], u32)> {
        // Only a new request replaces the one in flight, so a frame that never reached the GPU
        // does not lose it.
        let p = self.request.take()?;
        self.in_flight = Some(p);
        Some((p, self.count))
    }
    pub fn receive(&mut self, records: &[Record]) {
        self.paths = parse(records);
        self.pixel = self.in_flight.take();
        if self.selected.is_some_and(|i| i >= self.paths.len()) {
            self.selected = None;
        }
        self.upto = usize::MAX;
    }
    /// Records the shown pixel again (new sample indices if the frame moved on).
    pub fn again(&mut self) {
        if let Some(p) = self.pixel {
            self.request(p);
        }
    }
}

const BOUNCE_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(255, 255, 255),
    egui::Color32::from_rgb(90, 200, 255),
    egui::Color32::from_rgb(120, 255, 140),
    egui::Color32::from_rgb(255, 230, 90),
    egui::Color32::from_rgb(255, 150, 70),
    egui::Color32::from_rgb(255, 90, 120),
    egui::Color32::from_rgb(220, 110, 255),
    egui::Color32::from_rgb(140, 140, 255),
];
pub fn bounce_color(bounce: u32) -> egui::Color32 {
    BOUNCE_COLORS[bounce as usize % BOUNCE_COLORS.len()]
}
/// Dark blue at 1/1000 of the camera's throughput to bright yellow at 1 and above.
fn throughput_color(t: Vec3) -> egui::Color32 {
    let x = (1. + luminance(t).max(1e-6).log10() / 3.).clamp(0., 1.);
    let lerp = |a: f32, b: f32| (a + (b - a) * x) as u8;
    egui::Color32::from_rgb(lerp(30., 255.), lerp(40., 240.), lerp(140., 90.))
}

/// Draws the recorded paths. `far`: the length of rays that leave the scene.
pub fn draw(painter: &egui::Painter, ppp: f32, view: &View, far: f32, pv: &PathView) {
    let to_points = |p: Vec2| egui::pos2(p.x / ppp, p.y / ppp);
    let line = |a: Vec3, b: Vec3, stroke: egui::Stroke| {
        if let Some((a, b)) = view.segment(a, b) {
            painter.line_segment([to_points(a), to_points(b)], stroke);
        }
    };
    for (i, path) in pv.paths.iter().enumerate() {
        let selected = pv.selected == Some(i);
        let faint = pv.selected.is_some() && !selected;
        let shown = if selected {
            pv.upto.min(path.vertices.len())
        } else {
            path.vertices.len()
        };
        let width = if selected { 2.5_f32 } else { 1.5 };
        let fade = |c: egui::Color32| if faint { c.gamma_multiply(0.2) } else { c };
        for w in path.vertices[..shown].windows(2) {
            let (a, b) = (&w[0], &w[1]);
            // A portal moves the path without a segment between its two sides.
            if a.kind == Kind::PortalIn && b.kind == Kind::PortalOut {
                continue;
            }
            let color = match pv.coloring {
                Coloring::Bounce => bounce_color(b.bounce),
                Coloring::Throughput => throughput_color(b.throughput),
                Coloring::Event => b.kind.color(),
            };
            line(
                a.position,
                b.position,
                egui::Stroke::new(width, fade(color)),
            );
        }
        if let Some(last) = path.vertices[..shown].last()
            && last.kind == Kind::Escape
        {
            let color = match pv.coloring {
                Coloring::Throughput => throughput_color(last.throughput),
                _ => Kind::Escape.color(),
            };
            line(
                last.position,
                last.position + last.direction * far,
                egui::Stroke::new(width, fade(color.gamma_multiply(0.6))),
            );
        }
        if pv.light_samples {
            for l in path.lights.iter().filter(|l| l.vertex < shown) {
                let color = if l.blocked {
                    egui::Color32::from_rgb(170, 60, 60).gamma_multiply(0.6)
                } else {
                    egui::Color32::from_rgb(255, 230, 120).gamma_multiply(if selected {
                        0.8
                    } else {
                        0.4
                    })
                };
                line(
                    l.from,
                    l.from + l.direction * l.distance.min(far),
                    egui::Stroke::new(if selected { 1.5_f32 } else { 1. }, fade(color)),
                );
            }
        }
        for v in path.vertices.iter().take(shown).skip(1) {
            if v.kind == Kind::Escape {
                continue;
            }
            if let Some(p) = view.project(v.position) {
                let r = if selected { 3.5 } else { 2.5 };
                painter.circle_filled(to_points(p), r, fade(v.kind.color()));
            }
        }
    }
}

/// The inspector. Returns true when *Trace again* was pressed.
pub fn window(ctx: &egui::Context, pv: &mut PathView, perspective: bool) -> bool {
    let mut open = pv.open;
    let mut again = false;
    egui::Window::new("Path visualizer")
        .open(&mut open)
        .default_pos([160., 120.])
        .default_width(400.)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Click a pixel while this window is open (or Alt+click any time) to re-trace its paths with this frame's samples.",
                )
                .small()
                .weak(),
            );
            if !perspective {
                ui.label(
                    egui::RichText::new("Paths are drawn for the perspective camera only.")
                        .small()
                        .color(egui::Color32::from_rgb(255, 200, 120)),
                );
            }
            ui.horizontal(|ui| {
                ui.label("Paths");
                ui.add(crate::slider(&mut pv.count, 1..=64));
                if ui
                    .add_enabled(pv.pixel.is_some(), egui::Button::new("Trace again"))
                    .on_hover_text("The same pixel with the current frame's samples")
                    .clicked()
                {
                    again = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Colour");
                ui.selectable_value(&mut pv.coloring, Coloring::Bounce, "Bounce");
                ui.selectable_value(&mut pv.coloring, Coloring::Throughput, "Throughput");
                ui.selectable_value(&mut pv.coloring, Coloring::Event, "Event");
            });
            ui.checkbox(&mut pv.light_samples, "Light samples")
                .on_hover_text("Shadow rays of next-event estimation: yellow reached the light, red was blocked");
            let Some([x, y]) = pv.pixel else {
                return;
            };
            let mean = pv.paths.iter().map(|p| p.radiance).sum::<Vec3>()
                / pv.paths.len().max(1) as f32;
            ui.label(format!(
                "Pixel {x}, {y} · {} paths · mean luminance {:.4}",
                pv.paths.len(),
                luminance(mean)
            ));
            if pv.paths.last().is_some_and(|p| p.end.is_none()) {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 120),
                    "The recording ran out of space: fewer paths or bounces show them all.",
                );
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("paths")
                .max_height(180.)
                .show(ui, |ui| {
                    if ui
                        .selectable_label(pv.selected.is_none(), "All paths")
                        .clicked()
                    {
                        pv.selected = None;
                    }
                    for (i, p) in pv.paths.iter().enumerate() {
                        let bounces = p
                            .vertices
                            .iter()
                            .filter(|v| {
                                matches!(
                                    v.kind,
                                    Kind::Diffuse
                                        | Kind::Glossy
                                        | Kind::Reflect
                                        | Kind::Refract
                                        | Kind::Scatter
                                )
                            })
                            .count();
                        let end = p.end.map_or("out of space", |e| END_REASONS[e].0);
                        let text = format!(
                            "#{i:<2}  {bounces:>2} bounces  L {:.4}  {end}",
                            luminance(p.radiance)
                        );
                        let label = egui::RichText::new(text).monospace();
                        if ui
                            .selectable_label(pv.selected == Some(i), label)
                            .clicked()
                        {
                            pv.selected = Some(i);
                            pv.upto = usize::MAX;
                        }
                    }
                });
            let Some(path) = pv.selected.and_then(|i| pv.paths.get(i)) else {
                return;
            };
            ui.separator();
            let n = path.vertices.len();
            let mut upto = pv.upto.min(n);
            ui.horizontal(|ui| {
                ui.label("Vertices");
                ui.add(egui::Slider::new(&mut upto, 1.min(n)..=n));
            });
            pv.upto = upto;
            egui::ScrollArea::vertical()
                .id_salt("vertices")
                .max_height(220.)
                .show(ui, |ui| {
                    egui::Grid::new("path vertices")
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            for h in ["#", "Bounce", "Event", "Throughput", "Adds"] {
                                ui.label(egui::RichText::new(h).strong());
                            }
                            ui.end_row();
                            for (i, v) in path.vertices.iter().enumerate().take(upto) {
                                let lights: f32 = path
                                    .lights
                                    .iter()
                                    .filter(|l| l.vertex == i)
                                    .map(|l| luminance(l.contribution))
                                    .sum();
                                ui.label(i.to_string());
                                ui.label(v.bounce.to_string());
                                ui.colored_label(v.kind.color(), v.kind.name());
                                ui.label(format!("{:.4}", luminance(v.throughput)));
                                let adds = v.emitted + lights;
                                ui.label(if adds > 0. {
                                    format!("{adds:.4}")
                                } else {
                                    "–".into()
                                })
                                .on_hover_text(format!(
                                    "Emission {:.5}, light samples {lights:.5}",
                                    v.emitted
                                ));
                                ui.end_row();
                            }
                        });
                });
            let end = path.end.map_or("the recording ran out of space", |e| END_REASONS[e].1);
            ui.label(format!("Ends: {end}"));
            ui.label(format!(
                "Radiance {:.4} {:.4} {:.4} (the Adds column sums to its luminance)",
                path.radiance.x, path.radiance.y, path.radiance.z
            ));
        });
    pv.open = open;
    again
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rec(kind: u32, bounce: u32, path: u32, p: [f32; 3], value: f32) -> Record {
        Record {
            position: p,
            info: kind | bounce << 8 | path << 16,
            color: [0.5; 3],
            value,
            direction: [0., 1., 0.],
            pad: 0,
        }
    }
    #[test]
    fn parses_paths_lights_and_ends() {
        let records = [
            rec(0, 0, 0, [0., 0., 5.], 0.),
            rec(11, 0, 0, [0., 0., 0.], 2.),
            rec(1, 0, 0, [0., 0., 0.], 0.),
            rec(12, 1, 0, [1., 0., 0.], 3.),
            rec(9, 1, 0, [1., 0., 0.], 0.25),
            rec(13, 1, 0, [0.; 3], 1.),
            rec(0, 0, 1, [0., 0., 5.], 0.),
            rec(10, 0, 1, [0., 0., 5.], 0.1),
            rec(13, 0, 1, [0.; 3], 0.),
        ];
        let paths = parse(&records);
        assert_eq!(paths.len(), 2);
        let p = &paths[0];
        assert_eq!(
            p.vertices.iter().map(|v| v.kind).collect::<Vec<_>>(),
            [Kind::Camera, Kind::Diffuse, Kind::Stop]
        );
        // Light samples point at the vertex recorded after them.
        assert_eq!(p.lights.len(), 2);
        assert_eq!((p.lights[0].vertex, p.lights[0].blocked), (1, false));
        assert_eq!((p.lights[1].vertex, p.lights[1].blocked), (2, true));
        assert_eq!(p.end, Some(1));
        assert_eq!(p.vertices[2].emitted, 0.25);
        assert_eq!(paths[1].end, Some(0));
        assert_eq!(paths[1].vertices[1].kind, Kind::Escape);
    }
    #[test]
    fn missing_end_and_unknown_kinds() {
        let paths = parse(&[rec(0, 0, 0, [0.; 3], 0.), rec(99, 0, 0, [0.; 3], 0.)]);
        assert_eq!(paths[0].vertices.len(), 1);
        assert_eq!(paths[0].end, None);
    }
}
