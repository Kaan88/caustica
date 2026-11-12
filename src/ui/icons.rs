//! Line icons for the tool rail, the tab strip and the top bar: SVG-style paths on a 24-unit
//! grid, stroked by egui's painter (no icon font, so no missing glyphs).
use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Menu,
    Select,
    Move,
    Rotate,
    Scale,
    Walk,
    Frame,
    Add,
    Timeline,
    Render,
    Look,
    Lights,
    Object,
    Material,
    Export,
    Scenes,
    Assets,
    Compare,
    Pause,
    Play,
    Restart,
    // The top bar's pins (chrome.rs `pin_short`).
    Remote,
    Console,
    Stats,
    Keys,
    Video,
    Scopes,
    Graph,
    Paths,
    Describe,
    Puzzle,
    Open,
    Save,
    SaveAs,
    SaveSettings,
    Sliders,
    Cpu,
    Orbit,
    Flashlight,
    Denoiser,
    Freeze,
    Home,
    HidePanel,
    Update,
    Lock,
    Dice,
    Today,
    HighRes,
}
impl Icon {
    const ALL: [Icon; 48] = [
        Icon::Menu,
        Icon::Select,
        Icon::Move,
        Icon::Rotate,
        Icon::Scale,
        Icon::Walk,
        Icon::Frame,
        Icon::Add,
        Icon::Timeline,
        Icon::Render,
        Icon::Look,
        Icon::Lights,
        Icon::Object,
        Icon::Material,
        Icon::Export,
        Icon::Scenes,
        Icon::Assets,
        Icon::Compare,
        Icon::Pause,
        Icon::Play,
        Icon::Restart,
        Icon::Remote,
        Icon::Console,
        Icon::Stats,
        Icon::Keys,
        Icon::Video,
        Icon::Scopes,
        Icon::Graph,
        Icon::Paths,
        Icon::Describe,
        Icon::Puzzle,
        Icon::Open,
        Icon::Save,
        Icon::SaveAs,
        Icon::SaveSettings,
        Icon::Sliders,
        Icon::Cpu,
        Icon::Orbit,
        Icon::Flashlight,
        Icon::Denoiser,
        Icon::Freeze,
        Icon::Home,
        Icon::HidePanel,
        Icon::Update,
        Icon::Lock,
        Icon::Dice,
        Icon::Today,
        Icon::HighRes,
    ];
    /// SVG path data (M L H V A C Z and their relative forms, circular arcs only), plus
    /// `O cx cy r` for a circle.
    fn path(self) -> &'static str {
        match self {
            Icon::Menu => "M4 6h16M4 12h16M4 18h16",
            Icon::Select => "M5 3l14 8-6 2-2 6z",
            Icon::Move => "M12 2v20M2 12h20M9 5l3-3 3 3M9 19l3 3 3-3M5 9l-3 3 3 3M19 9l3 3-3 3",
            Icon::Rotate => "M21 12a9 9 0 1 1-3-6.7L21 8M21 3v5h-5",
            Icon::Scale => "M4 14v6h6M20 10V4h-6M4 20L20 4",
            Icon::Walk => "O13 4 2M9 22l3-7 3 3v4M7 12l3-4 4 1 3 3",
            Icon::Frame => "M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4O12 12 3",
            Icon::Add => "M12 5v14M5 12h14",
            Icon::Timeline => "M3 12h18M7 8v8M12 6v12M17 9v6",
            Icon::Render => "M3 7h4l2-3h6l2 3h4v13H3zO12 13 4",
            Icon::Look => "M3 4h18v16H3zM3 16l5-5 5 5 3-3 5 5",
            Icon::Lights => {
                "O12 12 4M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M5 19l2-2M17 7l2-2"
            }
            Icon::Object => "M12 2l9 5v10l-9 5-9-5V7zM12 12l9-5M12 12v10M12 12L3 7",
            Icon::Material => "O12 12 9M5 9c4 2 10 2 14 0",
            Icon::Export => "M12 3v12M7 10l5 5 5-5M4 21h16",
            Icon::Scenes => "M3 6h7l2 2h9v11H3z",
            Icon::Assets => "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5",
            Icon::Compare => "M8 4v16M16 4v16M4 8h4M4 16h4M16 8h4M16 16h4",
            Icon::Pause => "M8 5v14M16 5v14",
            Icon::Play => "M7 4l12 8-12 8z",
            Icon::Restart => "M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5",
            Icon::Remote => "M7 2h10v20H7zM11 18h2",
            Icon::Console => "M3 4h18v16H3zM7 9l3 3-3 3M12 15h5",
            Icon::Stats => "M3 20h18M6 20v-8M12 20V5M18 20v-11",
            Icon::Keys => "M2 6h20v12H2zM5 10h2M9 10h2M13 10h2M17 10h2M7 14h10",
            Icon::Video => "M3 6h13v12H3zM16 10l5-3v10l-5-3",
            Icon::Scopes => "M3 20h18M3 20c3 0 4-13 7-13c3 0 3 9 5 9c2 0 2-4 3-4c2 0 2 8 3 8",
            Icon::Graph => "M3 3v18h18M6 6c2 7 6 10 13 11",
            Icon::Paths => "M3 20l6-12 6 8 3-6O19 7 2.5",
            Icon::Describe => "M12 3l2 7 7 2-7 2-2 7-2-7-7-2 7-2z",
            Icon::Puzzle => "M3 12h9V3M7 17L17 7",
            Icon::Open => "M3 19V5h6l2 3h10v3M3 19l3-8h16l-3 8z",
            Icon::Save => "M4 4h13l3 3v13H4zM8 4v5h7V4M8 20v-7h8v7",
            Icon::SaveAs => "M4 4h13l3 3v13H4zM8 4v5h7V4M12 12v6M9 15h6",
            Icon::SaveSettings => "M6 3h8l4 4v14H6zM12 10v7M9 14l3 3 3-3",
            Icon::Sliders => "M4 7h3M11 7h9O9 7 2M4 17h9M17 17h3O15 17 2",
            Icon::Cpu => {
                "M7 7h10v10H7zM10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4"
            }
            Icon::Orbit => "O12 12 3M3 12c0-6 18-6 18 0c0 6-18 6-18 0",
            Icon::Flashlight => "M3 10h9v4H3zM12 10l5-3v10l-5-3M20 8l2-1M20 12h2M20 16l2 1",
            Icon::Denoiser => "M3 4h18v16H3zM12 4v16M6 8h2M8 12h2M5 16h2",
            Icon::Freeze => "M12 2v20M3.3 7l17.4 10M3.3 17L20.7 7M9 4l3 2 3-2M9 20l3-2 3 2",
            Icon::Home => "M3 11l9-8 9 8M5 9.5V21h14V9.5M10 21v-6h4v6",
            Icon::HidePanel => "M3 4h18v16H3zM15 4v16M7 9l3 3-3 3",
            Icon::Update => "O12 12 9M12 17V8M8 12l4-4 4 4",
            Icon::Lock => "M6 11h12v10H6zM8 11V8a4 4 0 0 1 8 0v3",
            Icon::Dice => "M4 4h16v16H4zO8 8 1O12 12 1O16 16 1",
            Icon::Today => "M4 5h16v16H4zM4 10h16M8 3v4M16 3v4O12 15.5 1.5",
            Icon::HighRes => "M3 3h18v18H3zM3 14h7v7M13 11l5-5M14 6h4v4",
        }
    }
    fn lines(self) -> &'static [Vec<[f32; 2]>] {
        static PARSED: std::sync::LazyLock<Vec<Vec<Vec<[f32; 2]>>>> =
            std::sync::LazyLock::new(|| Icon::ALL.iter().map(|i| parse(i.path())).collect());
        let index = Icon::ALL
            .iter()
            .position(|i| *i == self)
            .expect("every icon is in ALL");
        &PARSED[index]
    }
}

/// Numbers of a path: "3-3 3 3" and "1-3-6.7" split at each sign.
fn numbers(text: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        let starts = c == '-' || (c == '.' && current.contains('.'));
        if (starts || c == ' ' || c == ',') && !current.is_empty() {
            out.extend(current.parse::<f32>().ok());
            current.clear();
        }
        if c != ' ' && c != ',' {
            current.push(c);
        }
    }
    out.extend(current.parse::<f32>().ok());
    out
}

/// The path as polylines in the 24-unit grid.
fn parse(path: &str) -> Vec<Vec<[f32; 2]>> {
    let mut lines: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut at = [0_f32; 2];
    let mut start = at;
    // Split into (command, arguments).
    let mut commands: Vec<(char, String)> = Vec::new();
    for c in path.chars() {
        if c.is_ascii_alphabetic() {
            commands.push((c, String::new()));
        } else if let Some(last) = commands.last_mut() {
            last.1.push(c);
        }
    }
    for (command, arguments) in commands {
        let n = numbers(&arguments);
        let relative = command.is_ascii_lowercase();
        let to = |at: [f32; 2], x: f32, y: f32| {
            if relative {
                [at[0] + x, at[1] + y]
            } else {
                [x, y]
            }
        };
        match command.to_ascii_uppercase() {
            'M' => {
                for (k, p) in n.as_chunks::<2>().0.iter().enumerate() {
                    at = to(at, p[0], p[1]);
                    if k == 0 {
                        start = at;
                        lines.push(vec![at]);
                    } else if let Some(l) = lines.last_mut() {
                        l.push(at);
                    }
                }
            }
            'L' => {
                for p in n.as_chunks::<2>().0 {
                    at = to(at, p[0], p[1]);
                    if let Some(l) = lines.last_mut() {
                        l.push(at);
                    }
                }
            }
            'H' => {
                for x in n {
                    at = if relative {
                        [at[0] + x, at[1]]
                    } else {
                        [x, at[1]]
                    };
                    if let Some(l) = lines.last_mut() {
                        l.push(at);
                    }
                }
            }
            'V' => {
                for y in n {
                    at = if relative {
                        [at[0], at[1] + y]
                    } else {
                        [at[0], y]
                    };
                    if let Some(l) = lines.last_mut() {
                        l.push(at);
                    }
                }
            }
            'Z' => {
                at = start;
                if let Some(l) = lines.last_mut() {
                    l.push(at);
                }
            }
            'C' => {
                for p in n.as_chunks::<6>().0 {
                    let (c1, c2, end) =
                        (to(at, p[0], p[1]), to(at, p[2], p[3]), to(at, p[4], p[5]));
                    for k in 1..=12 {
                        let t = k as f32 / 12.;
                        let u = 1. - t;
                        let point = |i: usize| {
                            u * u * u * at[i]
                                + 3. * u * u * t * c1[i]
                                + 3. * u * t * t * c2[i]
                                + t * t * t * end[i]
                        };
                        if let Some(l) = lines.last_mut() {
                            l.push([point(0), point(1)]);
                        }
                    }
                    at = end;
                }
            }
            // rx ry rotation large sweep x y, with rx = ry.
            'A' => {
                for p in n.as_chunks::<7>().0 {
                    let end = to(at, p[5], p[6]);
                    let (large, sweep) = (p[3] != 0., p[4] != 0.);
                    let half = [(at[0] - end[0]) / 2., (at[1] - end[1]) / 2.];
                    let d = (half[0] * half[0] + half[1] * half[1]).sqrt();
                    let r = p[0].max(d);
                    let h = (r * r - d * d).max(0.).sqrt();
                    let sign = if large != sweep { 1. } else { -1. };
                    let centre = if d > 0. {
                        [
                            (at[0] + end[0]) / 2. + sign * h * half[1] / d,
                            (at[1] + end[1]) / 2. - sign * h * half[0] / d,
                        ]
                    } else {
                        at
                    };
                    let a0 = (at[1] - centre[1]).atan2(at[0] - centre[0]);
                    let mut delta = (end[1] - centre[1]).atan2(end[0] - centre[0]) - a0;
                    if sweep && delta < 0. {
                        delta += std::f32::consts::TAU;
                    } else if !sweep && delta > 0. {
                        delta -= std::f32::consts::TAU;
                    }
                    for k in 1..=24 {
                        let a = a0 + delta * k as f32 / 24.;
                        if let Some(l) = lines.last_mut() {
                            l.push([centre[0] + r * a.cos(), centre[1] + r * a.sin()]);
                        }
                    }
                    at = end;
                }
            }
            // A circle: cx cy r.
            'O' => {
                if let [cx, cy, r] = n[..] {
                    lines.push(
                        (0..=24)
                            .map(|k| {
                                let a = k as f32 / 24. * std::f32::consts::TAU;
                                [cx + r * a.cos(), cy + r * a.sin()]
                            })
                            .collect(),
                    );
                }
            }
            _ => {}
        }
    }
    lines
}

/// Draws `icon` centred in `rect`, `size` points across.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, size: f32, color: Color32) {
    let scale = size / 24.;
    let origin = rect.center() - Vec2::splat(size / 2.);
    let stroke = Stroke::new((1.7 * scale).max(1.1), color);
    for line in icon.lines() {
        let points: Vec<Pos2> = line
            .iter()
            .map(|p| origin + egui::vec2(p[0], p[1]) * scale)
            .collect();
        painter.add(egui::Shape::line(points, stroke));
    }
}

/// A flat icon button of `size` points, pressed in while `selected` (the tool rail's, the top
/// bar's): the skin's button chrome with the icon in the text colour, or the accent while on.
pub fn button(
    ui: &mut Ui,
    icon: Icon,
    side: f32,
    size: f32,
    selected: bool,
    _accent: bool,
) -> Response {
    button_sized(ui, icon, Vec2::splat(side), size, selected, true)
}
/// `button` with a width and a height; `flat` is the chrome that shows only under the pointer.
pub fn button_sized(
    ui: &mut Ui,
    icon: Icon,
    dims: Vec2,
    size: f32,
    selected: bool,
    flat: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(dims, Sense::click());
    if ui.is_rect_visible(rect) {
        let down = super::skin::button_chrome(ui, rect, &response, flat, selected);
        let color = icon_color(ui, selected);
        paint(
            ui.painter(),
            rect.translate(egui::vec2(0., down)),
            icon,
            size,
            color,
        );
    }
    response
}
/// An icon button that is one of a joined set (`seg` says which corners are round).
pub fn segment_button(
    ui: &mut Ui,
    icon: Icon,
    dims: Vec2,
    size: f32,
    selected: bool,
    seg: super::skin::Seg,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(dims, Sense::click());
    if ui.is_rect_visible(rect) {
        let down = super::skin::segment_chrome(ui, rect, &response, seg, selected);
        let color = icon_color(ui, selected);
        paint(
            ui.painter(),
            rect.translate(egui::vec2(0., down)),
            icon,
            size,
            color,
        );
    }
    response
}
fn icon_color(ui: &Ui, selected: bool) -> Color32 {
    if !ui.is_enabled() {
        super::skin::TX.gamma_multiply(0.42)
    } else if selected {
        super::ACCENT
    } else {
        super::skin::TX
    }
}
/// A tab of the property tabs (`across`: the strip lies along the top): the strip's ground,
/// the panel's and an accent bar while open, a lighter ground under the pointer.
pub fn tab(ui: &mut Ui, icon: Icon, dims: Vec2, size: f32, open: bool, across: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(dims, Sense::click());
    if ui.is_rect_visible(rect) {
        let (down, color) = super::skin::tab_chrome(ui, rect, &response, open, across);
        let color = if ui.is_enabled() {
            color
        } else {
            color.gamma_multiply(0.42)
        };
        paint(
            ui.painter(),
            rect.translate(egui::vec2(0., down)),
            icon,
            size,
            color,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_split_at_signs() {
        assert_eq!(numbers("3-3 3 3"), [3., -3., 3., 3.]);
        assert_eq!(numbers("9 9 0 1 1-3-6.7"), [9., 9., 0., 1., 1., -3., -6.7]);
    }
    /// Every icon parses to something that stays inside its 24-unit box.
    #[test]
    fn icons_parse_inside_their_box() {
        for icon in Icon::ALL {
            let lines = icon.lines();
            assert!(!lines.is_empty(), "{icon:?}");
            for p in lines.iter().flatten() {
                assert!(
                    p.iter().all(|v| (0.5..=23.5).contains(v)),
                    "{icon:?}: {p:?}"
                );
            }
        }
    }
    /// The Rotate arc ends where the path says: its last arc point is (18, 5.3).
    #[test]
    fn an_arc_ends_at_its_end_point() {
        let lines = parse("M21 12a9 9 0 1 1-3-6.7");
        let end = lines[0].last().unwrap();
        assert!((end[0] - 18.).abs() < 1e-3 && (end[1] - 5.3).abs() < 1e-3);
        // The large arc goes round the far side of the circle.
        assert!(lines[0].iter().any(|p| p[0] < 6.));
    }
}
