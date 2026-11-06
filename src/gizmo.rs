//! Blender-style transform gizmos (roadmap item 3b): move arrows and planes, rotation rings,
//! scale handles, and the modal G / R / S transforms with axis constraints, typed values and
//! vectors, snapping (increments, or other surfaces, vertices and edges) and cancel. Several
//! items transform together around a pivot (median, active, individual origins, 3D cursor).
//! All math is in physical pixels; egui drawing converts to points.
use crate::scene::{LightKind, Settings};
use crate::snap;
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
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
/// The point rotations and scales turn around when several items transform together.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Pivot {
    /// The mean of the items' origins.
    Median,
    /// The active (last clicked) item's origin.
    Active,
    /// Each item around its own origin.
    Individual,
    /// The 3D cursor (Shift+right-click places it).
    Cursor,
}
impl Pivot {
    pub const ALL: [Pivot; 4] = [
        Pivot::Median,
        Pivot::Active,
        Pivot::Individual,
        Pivot::Cursor,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Pivot::Median => "Median point",
            Pivot::Active => "Active element",
            Pivot::Individual => "Individual origins",
            Pivot::Cursor => "3D cursor",
        }
    }
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
    /// A world segment in pixels, clipped to the part in front of the camera (path visualizer).
    pub fn segment(&self, a: Vec3, b: Vec3) -> Option<(Vec2, Vec2)> {
        const NEAR: f32 = 1e-3;
        let (mut ca, mut cb) = (self.view_proj * a.extend(1.), self.view_proj * b.extend(1.));
        if ca.w < NEAR && cb.w < NEAR {
            return None;
        }
        if ca.w < NEAR {
            ca = ca.lerp(cb, (NEAR - ca.w) / (cb.w - ca.w));
        } else if cb.w < NEAR {
            cb = cb.lerp(ca, (NEAR - cb.w) / (ca.w - cb.w));
        }
        let pixel = |c: Vec4| {
            Vec2::new(
                (c.x / c.w * 0.5 + 0.5) * self.size.x,
                (0.5 - c.y / c.w * 0.5) * self.size.y,
            )
        };
        Some((pixel(ca), pixel(cb)))
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
    /// The world point of a GPU probe at `pixel`: its first-hit id (0 = miss) and view depth.
    pub fn surface_point(&self, pixel: [f32; 2], id: u32, depth: f32) -> Option<Vec3> {
        if id == 0 || !depth.is_finite() || depth <= 0. {
            return None;
        }
        let (o, d) = self.ray(Vec2::from(pixel));
        Some(o + d * (depth / d.dot(self.forward()).max(1e-4)))
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
            let (mut pivot, rotation) = crate::geometry::object_frame(o);
            // A simulated cloth keeps its object at the frame it started in and moves its
            // vertices: the gizmo goes to the middle of the drape's box instead.
            if o.cloth.is_some() {
                let c = crate::geometry::object_corners(o);
                pivot = c.iter().sum::<Vec3>() / 8.;
            }
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
/// The gizmo of several items: the active one's (first) orientation, at the pivot.
pub fn group_frame(
    s: &Settings,
    targets: &[Selection],
    pivot: Pivot,
    cursor: Vec3,
) -> Option<Frame> {
    let mut f = frame(s, *targets.first()?)?;
    let frames: Vec<Frame> = targets.iter().filter_map(|t| frame(s, *t)).collect();
    f.flat = frames.iter().all(|f| f.flat);
    f.pivot = match pivot {
        Pivot::Active => f.pivot,
        Pivot::Cursor => cursor,
        // Individual origins turn each item in place; the gizmo sits at their middle.
        Pivot::Median | Pivot::Individual => {
            frames.iter().map(|f| f.pivot).sum::<Vec3>() / frames.len().max(1) as f32
        }
    };
    Some(f)
}
/// The main light is always horizontal, so it has no rotate gizmo on its own (in a group it
/// only moves around the pivot).
pub fn allows(targets: &[Selection], mode: Mode) -> bool {
    !(targets == [Selection::MainLight] && mode == Mode::Rotate)
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
    /// What moves: the active item first (its axes orient the gizmo), then the rest.
    pub targets: Vec<Selection>,
    /// Where rotations and scales turn around, and the 3D cursor for `Pivot::Cursor`.
    pub pivot: Pivot,
    pub cursor: Vec3,
    /// Pointer position at the start, and now (Shift moves it at a tenth of the speed).
    pub start: Vec2,
    pub mouse: Vec2,
    /// The value being typed. In a free or plane move it belongs to axis `field`, and Tab
    /// moves on to the next axis, keeping the earlier ones in `vector`.
    pub typed: String,
    pub vector: [String; 3],
    pub field: usize,
    /// Ctrl held, and what it snaps to.
    pub snap: bool,
    pub snap_to: snap::Target,
    /// The real pointer (`mouse` moves at a tenth with Shift); snapping casts from here.
    pub pointer: Vec2,
    /// Started by Alt-drag or Shift+D: cancelling also undoes the copy.
    pub undo_on_cancel: bool,
    pub from_handle: bool,
    /// The settings before the transform: cancel restores them, undo stores them.
    pub original: Settings,
}
impl Transform {
    pub fn new(
        mode: Mode,
        targets: Vec<Selection>,
        mouse: Vec2,
        original: &Settings,
        local: bool,
    ) -> Self {
        Self {
            mode,
            constraint: Constraint::Free,
            local,
            base_local: local,
            targets,
            pivot: Pivot::Median,
            cursor: Vec3::new(0., -1., 0.),
            start: mouse,
            mouse,
            typed: String::new(),
            vector: Default::default(),
            field: 0,
            snap: false,
            snap_to: snap::Target::Increment,
            pointer: mouse,
            undo_on_cancel: false,
            from_handle: false,
            original: original.clone(),
        }
    }
    pub fn from_handle(
        mode: Mode,
        handle: Handle,
        targets: Vec<Selection>,
        mouse: Vec2,
        original: &Settings,
        local: bool,
    ) -> Self {
        let mut t = Self::new(mode, targets, mouse, original, local);
        t.from_handle = true;
        t.constraint = match handle {
            Handle::Axis(a) => Constraint::Axis(a),
            Handle::Plane(n) => Constraint::Plane(n),
            Handle::Center | Handle::ViewRing => Constraint::Free,
        };
        t
    }
    /// The gizmo frame at the start of the transform.
    pub fn frame(&self) -> Option<Frame> {
        group_frame(&self.original, &self.targets, self.pivot, self.cursor)
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
        // A plane has no field for the axis it excludes.
        if let Constraint::Plane(n) = self.constraint
            && self.field == n
        {
            self.next_field();
        }
    }
    /// Tab in a free or plane move: keep the typed value for this axis, edit the next one.
    pub fn next_field(&mut self) {
        self.vector[self.field] = std::mem::take(&mut self.typed);
        loop {
            self.field = (self.field + 1) % 3;
            if self.constraint != Constraint::Plane(self.field) {
                break;
            }
        }
        self.typed = std::mem::take(&mut self.vector[self.field]);
    }
    /// A free or plane move typed per axis (Tab between axes).
    fn typed_vector(&self) -> Option<[f32; 3]> {
        let mut text = self.vector.clone();
        text[self.field] = self.typed.clone();
        if text.iter().all(|t| t.is_empty()) {
            return None;
        }
        Some(text.map(|t| t.parse::<f32>().unwrap_or(0.)))
    }
    pub fn typed_value(&self) -> Option<f32> {
        self.typed.parse::<f32>().ok()
    }
    /// Restores the targets from `original`, then applies the transform. Returns header text.
    pub fn apply(&self, s: &mut Settings, view: &View) -> String {
        if !self.restore(s) {
            return String::new();
        }
        self.transform(s, view)
    }
    /// Puts the targets back as they were before the transform (cancel). False if one is gone.
    pub fn restore(&self, s: &mut Settings) -> bool {
        let mut ok = !self.targets.is_empty();
        for t in &self.targets {
            match *t {
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
                _ => ok = false,
            }
        }
        ok
    }
    /// Object indices among the targets (a snap ray passes through them).
    fn moved_objects(&self) -> Vec<usize> {
        self.targets
            .iter()
            .filter_map(|t| match t {
                Selection::Object(i) => Some(*i),
                _ => None,
            })
            .collect()
    }
    /// Snapping a move to another surface, vertex or edge: the move that brings the pivot
    /// there (on a surface, the group's lowest point along the normal rests on it), and what
    /// it snapped to. `None` when nothing is under the pointer.
    fn snap_move(&self, view: &View, pivot: Vec3) -> Option<(Vec3, &'static str)> {
        let (o, d) = view.ray(self.pointer);
        let hit = snap::cast(&self.original, o, d, &self.moved_objects())?;
        let p = snap::point(self.snap_to, &hit);
        if self.snap_to != snap::Target::Surface {
            return Some((p - pivot, self.snap_to.label()));
        }
        // How far the group reaches below the pivot along the surface normal.
        let reach = self
            .targets
            .iter()
            .flat_map(|t| match t {
                Selection::Object(i) => self
                    .original
                    .objects
                    .get(*i)
                    .map(|o| crate::geometry::object_corners(o).to_vec())
                    .unwrap_or_default(),
                t => frame(&self.original, *t)
                    .map(|f| vec![f.pivot])
                    .unwrap_or_default(),
            })
            .map(|c| (pivot - c).dot(hit.normal))
            .fold(0_f32, f32::max);
        Some((p + hit.normal * reach - pivot, "Surface"))
    }
    fn transform(&self, s: &mut Settings, view: &View) -> String {
        let Some(f) = self.frame() else {
            return String::new();
        };
        let axes = axes(&f, self.local);
        let space = if self.local { "local" } else { "global" };
        let typed = self.typed_value();
        let count = if self.targets.len() > 1 {
            format!(" ({} items)", self.targets.len())
        } else {
            String::new()
        };
        // Each target's own origin, for turning and scaling around the pivot: an object's real
        // origin (a cloth's gizmo sits at its drape, away from it).
        let origins: Vec<Option<Vec3>> = self
            .targets
            .iter()
            .map(|t| match *t {
                Selection::Object(i) => self
                    .original
                    .objects
                    .get(i)
                    .map(|o| crate::geometry::object_frame(o).0),
                _ => frame(&self.original, *t).map(|f| f.pivot),
            })
            .collect();
        let around_pivot = self.pivot != Pivot::Individual;
        match self.mode {
            Mode::Move => {
                let snapped = (self.snap && self.snap_to != snap::Target::Increment)
                    .then(|| self.snap_move(view, f.pivot))
                    .flatten();
                let (delta, text) = if let Some((d, what)) = snapped {
                    // Snapping keeps the constraint: only the allowed part of the move.
                    let d = match self.constraint {
                        Constraint::Axis(a) => axes[a] * axes[a].dot(d),
                        Constraint::Plane(n) => d - axes[n] * axes[n].dot(d),
                        Constraint::Free => d,
                    };
                    (
                        d,
                        format!("Move to {what}: {:.3}, {:.3}, {:.3} units", d.x, d.y, d.z),
                    )
                } else {
                    match self.constraint {
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
                            let label = match c {
                                Constraint::Plane(n) => {
                                    format!("Move (not {}, {space})", AXES[n])
                                }
                                _ => "Move".into(),
                            };
                            if let Some(v) = self.typed_vector() {
                                // Typed per axis, as in Blender: 1 Tab 2 Tab 3.
                                let d: Vec3 = (0..3)
                                    .filter(|k| c != Constraint::Plane(*k))
                                    .map(|k| axes[k] * v[k])
                                    .sum();
                                let fields: Vec<String> = (0..3)
                                    .filter(|k| c != Constraint::Plane(*k))
                                    .map(|k| {
                                        let text = if k == self.field {
                                            format!("{}|", self.typed)
                                        } else if self.vector[k].is_empty() {
                                            "0".into()
                                        } else {
                                            self.vector[k].clone()
                                        };
                                        format!("{} {text}", AXES[k])
                                    })
                                    .collect();
                                (
                                    d,
                                    format!("{label}: {} (Tab: next axis)", fields.join(", ")),
                                )
                            } else {
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
                                (
                                    d,
                                    format!("{label}: {:.3}, {:.3}, {:.3} units", d.x, d.y, d.z),
                                )
                            }
                        }
                    }
                };
                for t in &self.targets {
                    translate(s, *t, delta);
                }
                text + count.as_str()
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
                let q = Quat::from_axis_angle(axis, degrees.to_radians());
                for (t, origin) in self.targets.iter().zip(&origins) {
                    rotate(s, *t, q);
                    // Around a shared pivot the origins swing round it too.
                    if around_pivot && let Some(o) = origin {
                        translate(s, *t, f.pivot + q * (*o - f.pivot) - *o);
                    }
                }
                format!("Rotate {name}: {degrees:.1}°{count}")
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
                // The pivot-to-origin offsets stretch along the same axes as the items.
                let on = match self.constraint {
                    Constraint::Free => [true; 3],
                    Constraint::Axis(a) => [a == 0, a == 1, a == 2],
                    Constraint::Plane(n) => [n != 0, n != 1, n != 2],
                };
                for (t, origin) in self.targets.iter().zip(&origins) {
                    scale(s, *t, self.constraint, factor);
                    if around_pivot && let Some(o) = origin {
                        let d = *o - f.pivot;
                        let stretched: Vec3 = (0..3)
                            .map(|k| {
                                let part = axes[k] * axes[k].dot(d);
                                if on[k] { part * factor } else { part }
                            })
                            .sum();
                        translate(s, *t, stretched - d);
                    }
                }
                format!("Scale{name}: {factor:.3}×{count}")
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
        // A sphere's diameter follows any handle.
        Selection::Light(i) if s.lights[i].kind == LightKind::Sphere => {
            let l = &mut s.lights[i];
            l.size[0] = (l.size[0] * k).clamp(0.05, 40.);
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

/// Whether `mouse` is on the gizmo's inner part: the center, the plane squares and the inner
/// 60% of the arrows, which lie over the object itself. With physics running a press there
/// grabs the body (owner report 2026-11-03); the arrow tips and the rings still transform.
pub fn inner(view: &View, f: &Frame, local: bool, tool: Tool, mouse: Vec2) -> bool {
    tool != Tool::Rotate
        && layout(view, f, local, tool).is_some_and(|l| (mouse - l.center).length() < LENGTH * 0.6)
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
/// The outline of a box's projection: the convex hull of its projected corners. Nothing when a
/// corner is behind the camera.
pub fn draw_box_outline(
    painter: &egui::Painter,
    ppp: f32,
    view: &View,
    corners: &[Vec3; 8],
    stroke: egui::Stroke,
) {
    let mut points = Vec::with_capacity(8);
    for c in corners {
        match view.project(*c) {
            Some(p) => points.push(p / ppp),
            None => return,
        }
    }
    let hull = convex_hull(points);
    if hull.len() >= 3 {
        painter.add(egui::Shape::closed_line(
            hull.iter().map(|p| egui::pos2(p.x, p.y)).collect(),
            stroke,
        ));
    }
}
/// A silhouette outline of object `index` from the GPU's object ids (gui.hlsl): a quad over the
/// object's projected box (the whole viewport while a corner is behind the camera) whose
/// pixels light up just outside the object's visible pixels. `width` is in pixels.
pub fn draw_silhouette(
    painter: &egui::Painter,
    ppp: f32,
    view: &View,
    corners: &[Vec3; 8],
    index: usize,
    width: f32,
    color: egui::Color32,
) {
    let clip = painter.clip_rect();
    let pad = (width + 2.) / ppp;
    let projected: Vec<_> = corners.iter().filter_map(|c| view.project(*c)).collect();
    // Wholly behind the camera: nothing to outline.
    if projected.is_empty() {
        return;
    }
    let rect = if projected.len() < corners.len() {
        clip
    } else {
        egui::Rect::from_points(
            &projected
                .iter()
                .map(|p| egui::pos2(p.x / ppp, p.y / ppp))
                .collect::<Vec<_>>(),
        )
    };
    let rect = rect.expand(pad).intersect(clip);
    if !rect.is_positive() {
        return;
    }
    // Object ids are material + 1, and objects are materials 5 onwards.
    let uv = egui::pos2(-1. - width, (index + 6) as f32);
    let mut mesh = egui::Mesh::default();
    for p in [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ] {
        mesh.vertices
            .push(egui::epaint::Vertex { pos: p, uv, color });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}
/// Andrew's monotone chain; counter-clockwise, no repeated first point.
fn convex_hull(mut p: Vec<Vec2>) -> Vec<Vec2> {
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(p.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let points: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in points {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.
            {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    hull
}
pub fn draw_lights(
    painter: &egui::Painter,
    ppp: f32,
    view: &View,
    s: &Settings,
    selected: &[Selection],
) {
    for (sel, outline) in light_outlines(s, view.forward()) {
        let chosen = selected.contains(&sel);
        if !chosen && !outline_on(s, sel) {
            continue;
        }
        let pts: Vec<_> = outline.iter().filter_map(|p| view.project(*p)).collect();
        if pts.len() != outline.len() {
            continue;
        }
        let (width, color) = if chosen {
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
        // Spots and suns: a stroke along the direction the light travels.
        if let Selection::Light(i) = sel
            && let Some(l) = s.lights.get(i)
            && matches!(l.kind, LightKind::Spot | LightKind::Sun)
        {
            let c = Vec3::from(l.position);
            if let (Some(a), Some(b)) = (view.project(c), view.project(c + l.direction() * 0.6)) {
                painter.line_segment(
                    [
                        egui::pos2(a.x / ppp, a.y / ppp),
                        egui::pos2(b.x / ppp, b.y / ppp),
                    ],
                    egui::Stroke::new(width, color),
                );
            }
        }
    }
}
/// The light's outline is switched on (`light_outline` / `AreaLight::outline`).
fn outline_on(s: &Settings, sel: Selection) -> bool {
    match sel {
        Selection::MainLight => s.light_outline,
        Selection::Light(i) => s.lights.get(i).is_some_and(|l| l.outline),
        _ => false,
    }
}
/// Where a ray meets a planar convex polygon (wound either way), as a ray distance.
fn polygon_hit(poly: &[Vec3], o: Vec3, d: Vec3) -> Option<f32> {
    let n = (poly[1] - poly[0])
        .cross(poly[poly.len() - 1] - poly[0])
        .normalize_or_zero();
    let denom = d.dot(n);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (poly[0] - o).dot(n) / denom;
    let p = o + d * t;
    let side = |i: usize| {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        (b - a).cross(p - a).dot(n)
    };
    let first = side(0).signum();
    ((0..poly.len()).all(|i| side(i) * first >= -1e-9) && t > 0.).then_some(t)
}
/// Outlines of the lights that are on (off lights leave no geometry, so nothing to see or
/// click): rectangles, ellipses for disks and spots, a circle facing the view for spheres, and
/// a small disk for a sun at its handle.
fn light_outlines(s: &Settings, forward: Vec3) -> Vec<(Selection, Vec<Vec3>)> {
    let mut out = vec![];
    let ellipse = |c: Vec3, u: Vec3, v: Vec3| {
        (0..32)
            .map(|k| {
                let a = k as f32 / 32. * std::f32::consts::TAU;
                c + u * a.cos() + v * a.sin()
            })
            .collect::<Vec<_>>()
    };
    let l = Vec3::from(s.light_position);
    let (x, z) = (s.light_size * 0.5, s.light_size * 0.5 * s.light_aspect);
    if s.light_on {
        out.push((
            Selection::MainLight,
            vec![
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
        let outline = match a.kind {
            LightKind::Rect => [
                Vec3::new(-x, 0., -z),
                Vec3::new(x, 0., -z),
                Vec3::new(x, 0., z),
                Vec3::new(-x, 0., z),
            ]
            .map(|v| r * v + c)
            .to_vec(),
            LightKind::Disk | LightKind::Spot => ellipse(c, r * Vec3::X * x, r * Vec3::Z * z),
            LightKind::Sphere => {
                let u = forward.any_orthonormal_vector();
                ellipse(c, u * x, forward.cross(u) * x)
            }
            LightKind::Sun => ellipse(c, r * Vec3::X * 0.15, r * Vec3::Z * 0.15),
        };
        out.push((Selection::Light(i), outline));
    }
    out
}
/// Picks from a GPU readback at the click: the first-hit material id (u19, id + 1) and its
/// view depth. Objects and visible lights come straight from the id; lights hidden from the
/// camera are not in the id buffer, so their quads are still tested on the CPU.
pub fn pick_from_probe(
    s: &Settings,
    pixel: [f32; 2],
    size: [u32; 2],
    id: u32,
    depth: f32,
) -> Option<Selection> {
    let objects = s.objects.len() as u32;
    let hit = match id.checked_sub(1) {
        Some(4) => Some(Selection::MainLight),
        Some(m) if (5..5 + objects).contains(&m) => Some(Selection::Object((m - 5) as usize)),
        Some(m) if m >= 5 + objects && ((m - 5 - objects) as usize) < s.lights.len() => {
            Some(Selection::Light((m - 5 - objects) as usize))
        }
        _ => None,
    };
    let view = View::new(s, size);
    let (o, d) = view.ray(Vec2::from(pixel));
    // Ray distance of the GPU hit, from its view depth.
    let along = d.dot(view.forward()).max(1e-4);
    let mut best = hit.map(|h| (h, if id > 0 { depth / along } else { f32::INFINITY }));
    // Only rectangles that are drawn can be clicked; hidden ones are picked in the Scene tab.
    for (sel, q) in light_outlines(s, view.forward())
        .into_iter()
        .filter(|(sel, _)| outline_on(s, *sel))
    {
        let Some(t) = polygon_hit(&q, o, d) else {
            continue;
        };
        let nearer = t < if id > 0 { depth / along } else { f32::INFINITY };
        if nearer && best.is_none_or(|(_, b)| t < b) {
            best = Some((sel, t));
        }
    }
    best.map(|(sel, _)| sel)
}
/// Picks the nearest object or light quad under `pixel` on the CPU (every triangle; tests).
#[allow(dead_code)]
pub fn pick(s: &Settings, pixel: [f32; 2], size: [u32; 2]) -> Option<Selection> {
    let view = View::new(s, size);
    let (o, d) = view.ray(Vec2::from(pixel));
    let mut best = s
        .pick_object_hit(pixel, size)
        .map(|(i, t)| (Selection::Object(i), t));
    for (sel, q) in light_outlines(s, view.forward()) {
        let Some(t) = polygon_hit(&q, o, d) else {
            continue;
        };
        if best.is_none_or(|(_, b)| t < b) {
            best = Some((sel, t));
        }
    }
    best.map(|(sel, _)| sel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_hull_drops_inner_points() {
        let square = [(0., 0.), (2., 0.), (2., 2.), (0., 2.), (1., 1.), (2., 1.)]
            .map(|(x, y)| Vec2::new(x, y))
            .to_vec();
        let hull = convex_hull(square);
        assert_eq!(hull.len(), 4, "{hull:?}");
        assert!(!hull.contains(&Vec2::new(1., 1.)));
    }
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
            vec![Selection::Object(0)],
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
        let mut t = Transform::new(
            Mode::Rotate,
            vec![Selection::Object(0)],
            Vec2::ZERO,
            &s,
            false,
        );
        t.constrain(1, false);
        t.typed = "30".into();
        t.apply(&mut s, &view);
        assert!((wrap(s.objects[0].rotation - before.objects[0].rotation) - 30.).abs() < 1e-3);

        let mut s = before.clone();
        let mut t = Transform::new(
            Mode::Scale,
            vec![Selection::Object(0)],
            Vec2::ZERO,
            &s,
            false,
        );
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
            vec![Selection::Object(0)],
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
            vec![Selection::Object(0)],
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
        let mut t = Transform::new(
            Mode::Move,
            vec![Selection::Object(0)],
            Vec2::ZERO,
            &s,
            false,
        );
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
        let mut t = Transform::new(
            Mode::Scale,
            vec![Selection::MainLight],
            Vec2::ZERO,
            &s,
            false,
        );
        let depth = s.light_size * s.light_aspect;
        t.constrain(0, false);
        t.typed = "2".into();
        t.apply(&mut s, &view);
        assert!(
            (s.light_size * s.light_aspect - depth).abs() < 1e-4,
            "depth kept"
        );
        assert!(!allows(&[Selection::MainLight], Mode::Rotate));
        assert!(allows(
            &[Selection::MainLight, Selection::Object(0)],
            Mode::Rotate
        ));
    }
    /// Two objects, side by side on the floor.
    fn pair() -> Settings {
        let mut s = Settings::default();
        s.objects.truncate(1);
        s.objects[0].position = [-0.5, 0.];
        s.objects[0].rotation = 0.;
        s.objects[0].tilt = [0., 0.];
        let mut o = s.objects[0].clone();
        o.position = [0.5, 0.];
        s.objects.push(o);
        s
    }
    #[test]
    fn a_group_turns_around_its_median_or_in_place() {
        let (_, view) = setup();
        let both = vec![Selection::Object(0), Selection::Object(1)];
        let mut s = pair();
        let before = s.clone();
        let mut t = Transform::new(Mode::Rotate, both.clone(), Vec2::ZERO, &s, false);
        t.constrain(1, false);
        t.typed = "90".into();
        t.apply(&mut s, &view);
        // Around the median (0, 0): x = ±0.5 swings onto the z axis, and both turn 90°.
        for (i, z) in [(0, 0.5), (1, -0.5)] {
            let p = s.objects[i].position;
            assert!(p[0].abs() < 1e-4 && (p[1] - z).abs() < 1e-4, "{i}: {p:?}");
            assert!((wrap(s.objects[i].rotation - before.objects[i].rotation) - 90.).abs() < 1e-3);
        }
        // Individual origins: same turn, no move.
        let mut s = before.clone();
        t.pivot = Pivot::Individual;
        t.original = s.clone();
        t.apply(&mut s, &view);
        for i in 0..2 {
            assert_eq!(s.objects[i].position, before.objects[i].position);
            assert!((wrap(s.objects[i].rotation - before.objects[i].rotation) - 90.).abs() < 1e-3);
        }
        // Scaling 2x around the median doubles the gap.
        let mut s = before.clone();
        let mut t = Transform::new(Mode::Scale, both.clone(), Vec2::ZERO, &s, false);
        t.typed = "2".into();
        t.apply(&mut s, &view);
        assert!((s.objects[1].position[0] - s.objects[0].position[0] - 2.).abs() < 1e-4);
        // Cancel restores every target.
        assert!(t.restore(&mut s));
        assert!(s == before);
    }
    #[test]
    fn a_free_move_takes_a_typed_vector() {
        let (_, view) = setup();
        let mut s = pair();
        let before = s.clone();
        let mut t = Transform::new(
            Mode::Move,
            vec![Selection::Object(0), Selection::Object(1)],
            Vec2::ZERO,
            &s,
            false,
        );
        t.typed = "1".into();
        t.next_field();
        t.next_field();
        t.typed = "-0.5".into();
        let header = t.apply(&mut s, &view);
        for i in 0..2 {
            let (a, b) = (s.objects[i].position, before.objects[i].position);
            assert!((a[0] - b[0] - 1.).abs() < 1e-5 && (a[1] - b[1] + 0.5).abs() < 1e-5);
        }
        assert!(header.contains("2 items"), "{header}");
        // A plane skips the axis it excludes: Shift+Y leaves X and Z.
        t.constrain(1, true);
        assert_ne!(t.field, 1);
        t.next_field();
        assert_ne!(t.field, 1);
        // A value typed for Y before Shift+Y must not move along Y.
        let mut t = Transform::new(
            Mode::Move,
            vec![Selection::Object(0)],
            Vec2::ZERO,
            &before,
            false,
        );
        let mut s = before.clone();
        t.next_field();
        t.typed = "2".into();
        t.constrain(1, true);
        t.apply(&mut s, &view);
        assert_eq!(
            s.objects[0].elevation, before.objects[0].elevation,
            "Y stayed"
        );
    }
}
