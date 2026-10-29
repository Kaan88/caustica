//! Blender-style transform gizmos (roadmap item 3b): move arrows and planes, rotation rings,
//! scale handles, and the modal G / R / S transforms with axis constraints, typed values,
//! snapping and cancel. All math is in physical pixels; egui drawing converts to points.
use crate::scene::Settings;
use crate::ui::Selection;
use glam::{EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Gizmo length on screen, in physical pixels.
const LENGTH: f32 = 100.;
const COLORS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(232, 72, 82),
    egui::Color32::from_rgb(130, 204, 52),
    egui::Color32::from_rgb(64, 132, 245),
];
const HOVER: egui::Color32 = egui::Color32::from_rgb(255, 214, 90);
const AXES: [&str; 3] = ["X", "Y", "Z"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Move,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Constraint {
    Free,
    Axis(usize),
    /// The plane that excludes this axis (Shift+X/Y/Z).
    Plane(usize),
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Axis(usize),
    Plane(usize),
    /// Move in the view plane, or uniform scale.
    Center,
    /// Rotate around the view axis.
    ViewRing,
}

/// The camera as seen by the gizmo, in physical pixels.
pub struct View {
    view: Mat4,
    view_proj: Mat4,
    inverse: Mat4,
    eye: Vec3,
    size: Vec2,
    tan_half_fov: f32,
}
impl View {
    pub fn new(s: &Settings, size: [u32; 2]) -> Self {
        let (view, proj, eye) = s.camera(size[0] as f32 / size[1] as f32);
        let view_proj = proj * view;
        Self {
            view,
            view_proj,
            inverse: view_proj.inverse(),
            eye,
            size: Vec2::new(size[0] as f32, size[1] as f32),
            tan_half_fov: (s.fov.to_radians() * 0.5).tan(),
        }
    }
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj * p.extend(1.);
        (c.w > 1e-4).then(|| {
            Vec2::new(
                (c.x / c.w * 0.5 + 0.5) * self.size.x,
                (0.5 - c.y / c.w * 0.5) * self.size.y,
            )
        })
    }
    pub fn ray(&self, pixel: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec4::new(
            pixel.x / self.size.x * 2. - 1.,
            1. - pixel.y / self.size.y * 2.,
            1.,
            1.,
        );
        let p = self.inverse * ndc;
        (self.eye, (p.truncate() / p.w - self.eye).normalize())
    }
    /// World length of one pixel at depth of `p`.
    fn world_per_pixel(&self, p: Vec3) -> f32 {
        let depth = (-self.view.transform_point3(p).z).max(0.01);
        2. * depth * self.tan_half_fov / self.size.y
    }
    fn forward(&self) -> Vec3 {
        -self.view.inverse().z_axis.truncate().normalize()
    }
}

/// Where the gizmo sits and how the item is oriented.
pub struct Frame {
    pub pivot: Vec3,
    pub rotation: Mat3,
    /// Lights are flat quads: their scale gizmo has no Y handle.
    pub flat: bool,
}
pub fn frame(s: &Settings, selection: Selection) -> Option<Frame> {
    match selection {
        Selection::Object(i) => s.objects.get(i).map(|o| {
            let (pivot, rotation) = crate::geometry::object_frame(o);
            Frame {
                pivot,
                rotation,
                flat: false,
            }
        }),
        Selection::MainLight => Some(Frame {
            pivot: Vec3::from(s.light_position),
            rotation: Mat3::IDENTITY,
            flat: true,
        }),
        Selection::Light(i) => s.lights.get(i).map(|l| Frame {
            pivot: Vec3::from(l.position),
            rotation: Mat3::from_quat(light_quat(l.rotation)),
            flat: true,
        }),
        _ => None,
    }
}
/// The main light is always horizontal, so it has no rotate gizmo.
pub fn allows(selection: Selection, mode: Mode) -> bool {
    !(selection == Selection::MainLight && mode == Mode::Rotate)
}
fn axes(f: &Frame, local: bool) -> [Vec3; 3] {
    if local {
        [f.rotation.x_axis, f.rotation.y_axis, f.rotation.z_axis]
    } else {
        [Vec3::X, Vec3::Y, Vec3::Z]
    }
}
fn object_quat(yaw: f32, tilt: [f32; 2]) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        yaw.to_radians(),
        tilt[0].to_radians(),
        tilt[1].to_radians(),
    )
}
fn light_quat(r: [f32; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        r[1].to_radians(),
        r[0].to_radians(),
        r[2].to_radians(),
    )
}
/// Degrees in -180..180, as `validate()` and the sliders expect.
fn wrap(degrees: f32) -> f32 {
    (degrees + 180.).rem_euclid(360.) - 180.
}

/// A running transform: from a handle drag, or modal (G / R / S).
#[derive(Clone)]
pub struct Transform {
    pub mode: Mode,
    pub constraint: Constraint,
    pub local: bool,
    /// The orientation chosen in the top bar; the axis-key toggle flips away from it.
    base_local: bool,
    pub target: Selection,
    /// Pointer position at the start, and now (Shift moves it at a tenth of the speed).
    pub start: Vec2,
    pub mouse: Vec2,
    pub typed: String,
    pub snap: bool,
    pub from_handle: bool,
    /// The settings before the transform: cancel restores them, undo stores them.
    pub original: Settings,
}
impl Transform {
    pub fn new(
        mode: Mode,
        target: Selection,
        mouse: Vec2,
        original: &Settings,
        local: bool,
    ) -> Self {
        Self {
            mode,
            constraint: Constraint::Free,
            local,
            base_local: local,
            target,
            start: mouse,
            mouse,
            typed: String::new(),
            snap: false,
            from_handle: false,
            original: original.clone(),
        }
    }
    pub fn from_handle(
        mode: Mode,
        handle: Handle,
        target: Selection,
        mouse: Vec2,
        original: &Settings,
        local: bool,
    ) -> Self {
        let mut t = Self::new(mode, target, mouse, original, local);
        t.from_handle = true;
        t.constraint = match handle {
            Handle::Axis(a) => Constraint::Axis(a),
            Handle::Plane(n) => Constraint::Plane(n),
            Handle::Center | Handle::ViewRing => Constraint::Free,
        };
        t
    }
    /// X / Y / Z (Shift: plane). Pressing the same axis again switches global ↔ local, a
    /// third time frees it, as in Blender.
    pub fn constrain(&mut self, axis: usize, plane: bool) {
        let wanted = if plane {
            Constraint::Plane(axis)
        } else {
            Constraint::Axis(axis)
        };
        if self.constraint != wanted {
            self.constraint = wanted;
            self.local = self.base_local;
        } else if self.local == self.base_local {
            self.local = !self.base_local;
        } else {
            self.constraint = Constraint::Free;
            self.local = self.base_local;
        }
    }
    pub fn typed_value(&self) -> Option<f32> {
        self.typed.parse::<f32>().ok()
    }
    /// Restores the target from `original`, then applies the transform. Returns header text.
    pub fn apply(&self, s: &mut Settings, view: &View) -> String {
        if !self.restore(s) {
            return String::new();
        }
        self.transform(s, view)
    }
    /// Puts the target back as it was before the transform (cancel). False if it is gone.
    pub fn restore(&self, s: &mut Settings) -> bool {
        match self.target {
            Selection::Object(i) if i < s.objects.len() && i < self.original.objects.len() => {
                s.objects[i] = self.original.objects[i].clone();
            }
            Selection::Light(i) if i < s.lights.len() && i < self.original.lights.len() => {
                s.lights[i] = self.original.lights[i].clone();
            }
            Selection::MainLight => {
                s.light_position = self.original.light_position;
                s.light_size = self.original.light_size;
                s.light_aspect = self.original.light_aspect;
            }
            _ => return false,
        }
        true
    }
    fn transform(&self, s: &mut Settings, view: &View) -> String {
        let Some(f) = frame(&self.original, self.target) else {
            return String::new();
        };
        let axes = axes(&f, self.local);
        let space = if self.local { "local" } else { "global" };
        let typed = self.typed_value();
        match self.mode {
            Mode::Move => {
                let (delta, text) = match self.constraint {
                    Constraint::Axis(a) => {
                        let t = typed.unwrap_or_else(|| {
                            let t = along_axis(view, f.pivot, axes[a], self.start, self.mouse);
                            if self.snap {
                                (t * 10.).round() / 10.
                            } else {
                                t
                            }
                        });
                        (
                            axes[a] * t,
                            format!("Move {} ({space}): {t:.3} units", AXES[a]),
                        )
                    }
                    c => {
                        let normal = match c {
                            Constraint::Plane(n) => axes[n],
                            _ => view.forward(),
                        };
                        let mut d = on_plane(view, f.pivot, normal, self.start, self.mouse);
                        if self.snap {
                            d = axes
                                .iter()
                                .map(|a| *a * (a.dot(d) * 10.).round() / 10.)
                                .sum();
                        }
                        let label = match c {
                            Constraint::Plane(n) => format!("Move (not {}, {space})", AXES[n]),
                            _ => "Move".into(),
                        };
                        (
                            d,
                            format!("{label}: {:.3}, {:.3}, {:.3} units", d.x, d.y, d.z),
                        )
                    }
                };
                translate(s, self.target, delta);
                text
            }
            Mode::Rotate => {
                let (axis, name) = match self.constraint {
                    Constraint::Axis(a) | Constraint::Plane(a) => {
                        (axes[a], format!("{} ({space})", AXES[a]))
                    }
                    Constraint::Free => (-view.forward(), "view".into()),
                };
                let degrees = typed.unwrap_or_else(|| {
                    let d = screen_angle(view, f.pivot, axis, self.start, self.mouse).to_degrees();
                    if self.snap {
                        (d / 15.).round() * 15.
                    } else {
                        d
                    }
                });
                rotate(
                    s,
                    self.target,
                    Quat::from_axis_angle(axis, degrees.to_radians()),
                );
                format!("Rotate {name}: {degrees:.1}°")
            }
            Mode::Scale => {
                let factor = typed.unwrap_or_else(|| {
                    let c = view.project(f.pivot).unwrap_or(self.start);
                    let (a, b) = ((self.start - c).length(), (self.mouse - c).length());
                    let k = if a < 4. { 1. } else { b / a };
                    if self.snap {
                        ((k * 10.).round() / 10.).max(0.1)
                    } else {
                        k
                    }
                });
                let name = match self.constraint {
                    Constraint::Axis(a) => format!(" {}", AXES[a]),
                    Constraint::Plane(n) => format!(" (not {})", AXES[n]),
                    Constraint::Free => String::new(),
                };
                scale(s, self.target, self.constraint, factor);
                format!("Scale{name}: {factor:.3}×")
            }
        }
    }
}
/// World distance along `axis` for a pointer move, by the axis's on-screen direction.
fn along_axis(view: &View, pivot: Vec3, axis: Vec3, start: Vec2, mouse: Vec2) -> f32 {
    let unit = view.world_per_pixel(pivot) * LENGTH;
    let (Some(a), Some(b)) = (view.project(pivot), view.project(pivot + axis * unit)) else {
        return 0.;
    };
    let per_unit = (b - a) / unit;
    if per_unit.length_squared() < 1e-6 {
        return 0.;
    }
    (mouse - start).dot(per_unit) / per_unit.length_squared()
}
/// Pointer movement on the plane through `pivot` with `normal`, in world units.
fn on_plane(view: &View, pivot: Vec3, normal: Vec3, start: Vec2, mouse: Vec2) -> Vec3 {
    let hit = |p: Vec2| {
        let (o, d) = view.ray(p);
        let denom = d.dot(normal);
        (denom.abs() > 0.02).then(|| o + d * ((pivot - o).dot(normal) / denom))
    };
    match (hit(start), hit(mouse)) {
        (Some(a), Some(b)) => b - a,
        _ => Vec3::ZERO,
    }
}
/// Rotation angle around `axis` for a pointer move around the pivot on screen.
fn screen_angle(view: &View, pivot: Vec3, axis: Vec3, start: Vec2, mouse: Vec2) -> f32 {
    let Some(c) = view.project(pivot) else {
        return 0.;
    };
    let (a, b) = (start - c, mouse - c);
    if a.length() < 4. || b.length() < 4. {
        return 0.;
    }
    // Screen y points down, so a positive cross product is a clockwise turn on screen. A
    // rotation around an axis that points at the viewer looks counter-clockwise.
    let clockwise = a.perp_dot(b).atan2(a.dot(b));
    let toward_viewer = axis.dot(view.eye - pivot) > 0.;
    if toward_viewer { -clockwise } else { clockwise }
}
fn translate(s: &mut Settings, target: Selection, d: Vec3) {
    match target {
        Selection::Object(i) => {
            let o = &mut s.objects[i];
            o.position[0] = (o.position[0] + d.x).clamp(-40., 40.);
            o.position[1] = (o.position[1] + d.z).clamp(-40., 40.);
            o.elevation = (o.elevation + d.y).clamp(-40., 40.);
        }
        Selection::Light(i) => {
            let l = &mut s.lights[i];
            for k in 0..3 {
                l.position[k] = (l.position[k] + d[k]).clamp(-50., 50.);
            }
        }
        Selection::MainLight => {
            for k in 0..3 {
                s.light_position[k] = (s.light_position[k] + d[k]).clamp(-50., 50.);
            }
        }
        _ => {}
    }
}
fn rotate(s: &mut Settings, target: Selection, q: Quat) {
    match target {
        Selection::Object(i) => {
            let o = &mut s.objects[i];
            let (y, x, z) = (q * object_quat(o.rotation, o.tilt)).to_euler(EulerRot::YXZ);
            o.rotation = wrap(y.to_degrees());
            o.tilt = [wrap(x.to_degrees()), wrap(z.to_degrees())];
        }
        Selection::Light(i) => {
            let l = &mut s.lights[i];
            let (y, x, z) = (q * light_quat(l.rotation)).to_euler(EulerRot::YXZ);
            l.rotation = [
                wrap(x.to_degrees()),
                wrap(y.to_degrees()),
                wrap(z.to_degrees()),
            ];
        }
        _ => {}
    }
}
fn scale(s: &mut Settings, target: Selection, c: Constraint, k: f32) {
    // Which local axes (X, Y, Z) the constraint scales.
    let on = match c {
        Constraint::Free => [true; 3],
        Constraint::Axis(a) => [a == 0, a == 1, a == 2],
        Constraint::Plane(n) => [n != 0, n != 1, n != 2],
    };
    match target {
        Selection::Object(i) => {
            let o = &mut s.objects[i];
            if c == Constraint::Free {
                o.size = (o.size * k).clamp(0.05, 30.);
            } else {
                if on[0] {
                    o.scale_xz[0] = (o.scale_xz[0] * k).clamp(0.01, 20.);
                }
                if on[1] {
                    o.stretch = (o.stretch * k).clamp(0.05, 10.);
                }
                if on[2] {
                    o.scale_xz[1] = (o.scale_xz[1] * k).clamp(0.01, 20.);
                }
            }
        }
        Selection::Light(i) => {
            let l = &mut s.lights[i];
            if on[0] {
                l.size[0] = (l.size[0] * k).clamp(0.05, 40.);
            }
            if on[2] {
                l.size[1] = (l.size[1] * k).clamp(0.05, 40.);
            }
        }
        Selection::MainLight => {
            // Width is size, depth is size × aspect.
            let (width, depth) = (s.light_size, s.light_size * s.light_aspect);
            let width = if on[0] { width * k } else { width };
            let depth = if on[2] { depth * k } else { depth };
            s.light_size = width.clamp(0.05, 40.);
            s.light_aspect = (depth / s.light_size).clamp(0.1, 10.);
        }
        _ => {}
    }
}

/// Screen-space shapes of the gizmo, shared by drawing and hit testing.
struct Layout {
    center: Vec2,
    tips: [Option<Vec2>; 3],
    planes: [Option<[Vec2; 4]>; 3],
    rings: [Vec<(Vec2, bool)>; 3],
}
fn layout(view: &View, f: &Frame, local: bool, tool: Tool) -> Option<Layout> {
    let center = view.project(f.pivot)?;
    let axes = axes(f, local);
    let unit = view.world_per_pixel(f.pivot) * LENGTH;
    let mut tips = [None; 3];
    let mut planes = [None; 3];
    let mut rings: [Vec<(Vec2, bool)>; 3] = Default::default();
    for a in 0..3 {
        tips[a] = view
            .project(f.pivot + axes[a] * unit)
            // An axis pointing at the camera has no usable on-screen direction.
            .filter(|t| (*t - center).length() > 12.)
            .filter(|_| !(tool == Tool::Scale && f.flat && a == 1));
        let (i, j) = ((a + 1) % 3, (a + 2) % 3);
        if tool == Tool::Move {
            let corner =
                |u: f32, v: f32| view.project(f.pivot + (axes[i] * u + axes[j] * v) * unit);
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (
                corner(0.22, 0.22),
                corner(0.42, 0.22),
                corner(0.42, 0.42),
                corner(0.22, 0.42),
            ) {
                // Skip planes seen edge-on.
                if (p1 - p0).perp_dot(p3 - p0).abs() > 120. {
                    planes[a] = Some([p0, p1, p2, p3]);
                }
            }
        }
        if tool == Tool::Rotate {
            for k in 0..=64 {
                let t = k as f32 / 64. * std::f32::consts::TAU;
                let p = f.pivot + (axes[i] * t.cos() + axes[j] * t.sin()) * unit;
                if let Some(q) = view.project(p) {
                    rings[a].push((q, (p - f.pivot).dot(view.eye - f.pivot) >= -1e-4));
                }
            }
        }
    }
    Some(Layout {
        center,
        tips,
        planes,
        rings,
    })
}
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0., 1.);
    (a + ab * t - p).length()
}
fn inside(p: Vec2, q: &[Vec2; 4]) -> bool {
    let sign = |a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
    let s: Vec<f32> = (0..4).map(|i| sign(q[i], q[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.) || s.iter().all(|v| *v <= 0.)
}
/// The handle under `mouse`, if any.
pub fn hit(view: &View, f: &Frame, local: bool, tool: Tool, mouse: Vec2) -> Option<Handle> {
    let l = layout(view, f, local, tool)?;
    match tool {
        Tool::Select => None,
        Tool::Move | Tool::Scale => {
            if (mouse - l.center).length() < 11. {
                return Some(Handle::Center);
            }
            if let Some(n) =
                (0..3).find(|n| l.planes[*n].as_ref().is_some_and(|q| inside(mouse, q)))
            {
                return Some(Handle::Plane(n));
            }
            (0..3)
                .filter_map(|a| l.tips[a].map(|t| (a, segment_distance(mouse, l.center, t))))
                .filter(|(_, d)| *d < 9.)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(a, _)| Handle::Axis(a))
        }
        Tool::Rotate => {
            let ring = (0..3)
                .filter_map(|a| {
                    l.rings[a]
                        .windows(2)
                        .filter(|w| w[0].1 && w[1].1)
                        .map(|w| segment_distance(mouse, w[0].0, w[1].0))
                        .min_by(f32::total_cmp)
                        .map(|d| (a, d))
                })
                .filter(|(_, d)| *d < 8.)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((a, _)) = ring {
                return Some(Handle::Axis(a));
            }
            ((mouse - l.center).length() - LENGTH * 1.2)
                .abs()
                .lt(&8.)
                .then_some(Handle::ViewRing)
        }
    }
}

/// Draws the gizmo for `tool` (or for the running transform's constraint) onto `painter`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    ppp: f32,
    view: &View,
    f: &Frame,
    local: bool,
    tool: Tool,
    hover: Option<Handle>,
    running: Option<&Transform>,
) {
    let pt = |p: Vec2| egui::pos2(p.x / ppp, p.y / ppp);
    let stroke = |w: f32, c: egui::Color32| egui::Stroke::new(w, c);
    if let Some(t) = running {
        // While transforming: the constraint axis as a long line through the pivot, like Blender.
        let axes = axes(f, t.local);
        let lines: Vec<usize> = match t.constraint {
            Constraint::Axis(a) => vec![a],
            Constraint::Plane(n) => (0..3).filter(|a| *a != n).collect(),
            Constraint::Free => vec![],
        };
        for a in lines {
            let far = 1000.;
            if let (Some(p), Some(q)) = (
                clip_project(view, f.pivot, f.pivot - axes[a] * far),
                clip_project(view, f.pivot, f.pivot + axes[a] * far),
            ) {
                painter.line_segment([pt(p), pt(q)], stroke(1.5, COLORS[a]));
            }
        }
        if let Some(c) = view.project(f.pivot) {
            painter.circle_filled(pt(c), 3., egui::Color32::WHITE);
            if t.mode != Mode::Move {
                painter.line_segment(
                    [pt(c), pt(t.mouse)],
                    stroke(1., egui::Color32::from_white_alpha(140)),
                );
            }
        }
        return;
    }
    let Some(l) = layout(view, f, local, tool) else {
        return;
    };
    let color = |h: Handle, base: egui::Color32| if hover == Some(h) { HOVER } else { base };
    match tool {
        Tool::Select => {}
        Tool::Move | Tool::Scale => {
            for (n, q) in l.planes.iter().enumerate() {
                if let Some(q) = q {
                    let c = color(Handle::Plane(n), COLORS[n]);
                    painter.add(egui::Shape::convex_polygon(
                        q.iter().map(|p| pt(*p)).collect(),
                        c.gamma_multiply(0.45),
                        stroke(1., c),
                    ));
                }
            }
            for (a, (tip, base)) in l.tips.iter().zip(COLORS).enumerate() {
                let Some(tip) = *tip else { continue };
                let c = color(Handle::Axis(a), base);
                painter.line_segment([pt(l.center), pt(tip)], stroke(2.5, c));
                let dir = (tip - l.center).normalize();
                if tool == Tool::Move {
                    let side = dir.perp() * 6.;
                    painter.add(egui::Shape::convex_polygon(
                        vec![pt(tip + dir * 14.), pt(tip + side), pt(tip - side)],
                        c,
                        egui::Stroke::NONE,
                    ));
                } else {
                    let r = egui::Rect::from_center_size(pt(tip), egui::vec2(9. / ppp, 9. / ppp));
                    painter.rect_filled(r, 1., c);
                }
            }
            let c = color(Handle::Center, egui::Color32::WHITE);
            painter.circle_stroke(pt(l.center), 9. / ppp, stroke(1.5, c));
        }
        Tool::Rotate => {
            for (a, (ring, base)) in l.rings.iter().zip(COLORS).enumerate() {
                let c = color(Handle::Axis(a), base);
                for w in ring.windows(2) {
                    let (width, col) = if w[0].1 && w[1].1 {
                        (2.5, c)
                    } else {
                        (1., c.gamma_multiply(0.25))
                    };
                    painter.line_segment([pt(w[0].0), pt(w[1].0)], stroke(width, col));
                }
            }
            let c = color(Handle::ViewRing, egui::Color32::from_white_alpha(200));
            painter.circle_stroke(pt(l.center), LENGTH * 1.2 / ppp, stroke(1.5, c));
        }
    }
}
/// Projects `p`, pulling it toward `from` until it is in front of the camera.
fn clip_project(view: &View, from: Vec3, p: Vec3) -> Option<Vec2> {
    let mut q = p;
    for _ in 0..24 {
        if let Some(s) = view.project(q) {
            return Some(s);
        }
        q = from + (q - from) * 0.5;
    }
    None
}
/// Outlines of the main light and the area lights, so hidden lights can be found and picked.
pub fn draw_lights(
    painter: &egui::Painter,
    ppp: f32,
    view: &View,
    s: &Settings,
    selected: Selection,
) {
    for (sel, quad) in light_quads(s) {
        let pts: Vec<_> = quad.iter().filter_map(|p| view.project(*p)).collect();
        if pts.len() != 4 {
            continue;
        }
        let (width, color) = if sel == selected {
            (2., HOVER)
        } else {
            (
                1_f32,
                egui::Color32::from_rgba_unmultiplied(255, 236, 170, 90),
            )
        };
        let mut line: Vec<egui::Pos2> = pts
            .iter()
            .map(|p| egui::pos2(p.x / ppp, p.y / ppp))
            .collect();
        line.push(line[0]);
        painter.add(egui::Shape::line(line, egui::Stroke::new(width, color)));
    }
}
/// Quads of the lights that are on (off lights leave no geometry, so nothing to see or click).
fn light_quads(s: &Settings) -> Vec<(Selection, [Vec3; 4])> {
    let mut out = vec![];
    let l = Vec3::from(s.light_position);
    let (x, z) = (s.light_size * 0.5, s.light_size * 0.5 * s.light_aspect);
    if s.light_on {
        out.push((
            Selection::MainLight,
            [
                l + Vec3::new(-x, 0., -z),
                l + Vec3::new(x, 0., -z),
                l + Vec3::new(x, 0., z),
                l + Vec3::new(-x, 0., z),
            ],
        ));
    }
    for (i, a) in s.lights.iter().enumerate().filter(|(_, a)| a.enabled) {
        let r = light_quat(a.rotation);
        let c = Vec3::from(a.position);
        let (x, z) = (a.size[0] * 0.5, a.size[1] * 0.5);
        out.push((
            Selection::Light(i),
            [
                Vec3::new(-x, 0., -z),
                Vec3::new(x, 0., -z),
                Vec3::new(x, 0., z),
                Vec3::new(-x, 0., z),
            ]
            .map(|v| r * v + c),
        ));
    }
    out
}
/// Picks the nearest object or light quad under `pixel`.
pub fn pick(s: &Settings, pixel: [f32; 2], size: [u32; 2]) -> Option<Selection> {
    let view = View::new(s, size);
    let (o, d) = view.ray(Vec2::from(pixel));
    let mut best = s
        .pick_object_hit(pixel, size)
        .map(|(i, t)| (Selection::Object(i), t));
    for (sel, q) in light_quads(s) {
        let n = (q[1] - q[0]).cross(q[3] - q[0]).normalize_or_zero();
        let denom = d.dot(n);
        if denom.abs() < 1e-6 {
            continue;
        }
        let t = (q[0] - o).dot(n) / denom;
        let p = o + d * t;
        let (u, v) = ((p - q[0]).dot(q[1] - q[0]), (p - q[0]).dot(q[3] - q[0]));
        let inside = (0. ..=(q[1] - q[0]).length_squared()).contains(&u)
            && (0. ..=(q[3] - q[0]).length_squared()).contains(&v);
        if t > 0. && inside && best.is_none_or(|(_, b)| t < b) {
            best = Some((sel, t));
        }
    }
    best.map(|(sel, _)| sel)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (Settings, View) {
        let s = Settings::default();
        let v = View::new(&s, [1280, 720]);
        (s, v)
    }
    #[test]
    fn typed_values_move_rotate_and_scale_exactly() {
        let (mut s, view) = setup();
        let before = s.clone();
        let mut t = Transform::new(
            Mode::Move,
            Selection::Object(0),
            Vec2::new(600., 300.),
            &s,
            false,
        );
        t.constrain(0, false);
        t.typed = "0.5".into();
        t.apply(&mut s, &view);
        assert!((s.objects[0].position[0] - before.objects[0].position[0] - 0.5).abs() < 1e-5);
        assert_eq!(s.objects[0].position[1], before.objects[0].position[1]);

        let mut s = before.clone();
        let mut t = Transform::new(Mode::Rotate, Selection::Object(0), Vec2::ZERO, &s, false);
        t.constrain(1, false);
        t.typed = "30".into();
        t.apply(&mut s, &view);
        assert!((wrap(s.objects[0].rotation - before.objects[0].rotation) - 30.).abs() < 1e-3);

        let mut s = before.clone();
        let mut t = Transform::new(Mode::Scale, Selection::Object(0), Vec2::ZERO, &s, false);
        t.typed = "2".into();
        t.apply(&mut s, &view);
        assert!((s.objects[0].size - before.objects[0].size * 2.).abs() < 1e-5);
        t.constrain(1, false);
        t.apply(&mut s, &view);
        assert_eq!(
            s.objects[0].size, before.objects[0].size,
            "a re-apply starts from the original"
        );
        assert!((s.objects[0].stretch - before.objects[0].stretch * 2.).abs() < 1e-5);
    }
    #[test]
    fn dragging_the_x_arrow_moves_along_x_only() {
        let (mut s, view) = setup();
        let before = s.clone();
        let f = frame(&s, Selection::Object(0)).unwrap();
        let l = layout(&view, &f, false, Tool::Move).unwrap();
        let tip = l.tips[0].unwrap();
        let grab = l.center + (tip - l.center) * 0.6;
        assert_eq!(
            hit(&view, &f, false, Tool::Move, grab),
            Some(Handle::Axis(0))
        );
        let mut t = Transform::from_handle(
            Mode::Move,
            Handle::Axis(0),
            Selection::Object(0),
            grab,
            &s,
            false,
        );
        t.mouse = grab + (tip - l.center); // one gizmo length further
        t.apply(&mut s, &view);
        let moved = s.objects[0].position[0] - before.objects[0].position[0];
        assert!(moved > 0.05, "{moved}");
        assert_eq!(s.objects[0].position[1], before.objects[0].position[1]);
        assert_eq!(s.objects[0].elevation, before.objects[0].elevation);
        // The arrow's tip follows the pointer: the new tip sits where the pointer went.
        let f2 = frame(&s, Selection::Object(0)).unwrap();
        let c2 = view.project(f2.pivot).unwrap();
        assert!(
            (c2 - (l.center + (tip - l.center))).length() < 3.,
            "{c2} vs {}",
            l.center + (tip - l.center)
        );
    }
    #[test]
    fn rotating_around_y_by_screen_angle_keeps_tilt() {
        let (mut s, view) = setup();
        let before = s.clone();
        let f = frame(&s, Selection::Object(0)).unwrap();
        let c = view.project(f.pivot).unwrap();
        let mut t = Transform::new(
            Mode::Rotate,
            Selection::Object(0),
            c + Vec2::new(80., 0.),
            &s,
            false,
        );
        t.constrain(1, false);
        t.mouse = c + Vec2::new(0., -80.); // a quarter turn on screen
        t.apply(&mut s, &view);
        let turned = wrap(s.objects[0].rotation - before.objects[0].rotation).abs();
        assert!((turned - 90.).abs() < 1., "{turned}");
        assert!(s.objects[0].tilt.iter().all(|v| v.abs() < 1e-3));
    }
    #[test]
    fn constraint_toggles_like_blender() {
        let (s, _) = setup();
        let mut t = Transform::new(Mode::Move, Selection::Object(0), Vec2::ZERO, &s, false);
        t.constrain(0, false);
        assert_eq!((t.constraint, t.local), (Constraint::Axis(0), false));
        t.constrain(0, false);
        assert_eq!((t.constraint, t.local), (Constraint::Axis(0), true));
        t.constrain(0, false);
        assert_eq!((t.constraint, t.local), (Constraint::Free, false));
        t.constrain(2, true);
        assert_eq!(t.constraint, Constraint::Plane(2));
    }
    #[test]
    fn lights_are_pickable_and_scale_their_quads() {
        let (mut s, view) = setup();
        let c = view.project(Vec3::from(s.light_position)).unwrap();
        assert_eq!(
            pick(&s, c.to_array(), [1280, 720]),
            Some(Selection::MainLight)
        );
        let mut t = Transform::new(Mode::Scale, Selection::MainLight, Vec2::ZERO, &s, false);
        let depth = s.light_size * s.light_aspect;
        t.constrain(0, false);
        t.typed = "2".into();
        t.apply(&mut s, &view);
        assert!(
            (s.light_size * s.light_aspect - depth).abs() < 1e-4,
            "depth kept"
        );
        assert!(!allows(Selection::MainLight, Mode::Rotate));
    }
}
