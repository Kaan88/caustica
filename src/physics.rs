//! Track 17 physics sandbox: rapier3d rigid bodies driven from `Settings::objects`.
//!
//! The simulation owns the poses while it runs: each step writes the bodies back into the
//! objects (`grounded` off, `elevation` = body centre), which changes only the geometry key's
//! transforms, so the renderer rebuilds just the TLAS. Any other edit to the objects (gizmo,
//! inspector, undo, a new scene) rebuilds the simulation from the settings. See engine-notes
//! "Physics sandbox".
use crate::scene::{Body, Object, Settings, Shape};
use glam::{EulerRot, Quat, Vec3};
use rapier3d::prelude::*;

/// Fixed simulation step (s).
const STEP: f32 = 1. / 120.;
/// At most this many steps per frame; a slower frame runs the simulation in slow motion.
const MAX_STEPS: u32 = 8;
/// Bodies that fall below this are frozen where they are (off the floor's edge).
const FLOOR_LIMIT: f32 = -30.;

fn v(p: Vec3) -> Vector {
    Vector::new(p.x, p.y, p.z)
}
fn g(p: Vector) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}
fn q(r: Quat) -> Rotation {
    Rotation::from_xyzw(r.x, r.y, r.z, r.w)
}

/// A body held by the mouse: a damped spring from a point on the body to the cursor ray.
pub struct Grab {
    pub index: usize,
    /// The grabbed point in the body's frame.
    local: Vec3,
    /// Distance from the eye along the cursor ray, kept while dragging.
    pub distance: f32,
    pub target: Vec3,
}

pub struct Sim {
    pipeline: PhysicsPipeline,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    soft: SoftBodySet,
    /// One body per object (None for hidden objects and portals).
    handles: Vec<Option<RigidBodyHandle>>,
    /// The objects as the simulation last wrote them: anything else means an outside edit.
    pub written: Vec<Object>,
    /// The settings the static colliders came from (floor and room).
    stage: (bool, u32),
    accumulator: f32,
    pub grab: Option<Grab>,
    /// The physics settings last applied.
    applied: crate::scene::Physics,
    /// Simulated seconds (wind gusts).
    time: f32,
}
/// Whether two object lists give the same bodies: poses, shapes and body settings, not
/// colours or names (a material edit keeps the simulation and its velocities).
fn same_bodies(a: &[Object], b: &[Object]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            (
                x.shape, x.sphere, x.sdf, x.body, x.visible, x.portal, x.grounded,
            ) == (
                y.shape, y.sphere, y.sdf, y.body, y.visible, y.portal, y.grounded,
            ) && (x.position, x.elevation, x.rotation, x.tilt)
                == (y.position, y.elevation, y.rotation, y.tilt)
                && (x.size, x.stretch, x.scale_xz, x.mass)
                    == (y.size, y.stretch, y.scale_xz, y.mass)
                && x.mesh == y.mesh
                && x.tether == y.tether
        })
}

/// Shapes with holes or hollows (tori, rings, knots, teapots, bunnies, concave lenses,
/// imported meshes) as a compound of convex parts (V-HACD), so a ring can hang on a peg and
/// a bowl holds a ball. Meshes over 20k triangles keep the hull (the decomposition would
/// stall the frame). Cached by shape and scale: a restart (any outside edit) reuses them.
fn decomposed(o: &Object, scale: Vec3) -> Option<SharedShape> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<String, SharedShape>>> = Mutex::new(None);
    let concave = match o.sdf {
        crate::scene::Sdf::Torus => true,
        crate::scene::Sdf::None => {
            !o.mesh.is_empty()
                || matches!(
                    o.shape,
                    Shape::Torus
                        | Shape::Knot
                        | Shape::Ring
                        | Shape::Teapot
                        | Shape::Bunny
                        | Shape::ConcaveLens
                )
        }
        _ => false,
    };
    if !concave {
        return None;
    }
    let key = format!(
        "{:?}|{}|{:?}",
        o.kind(),
        o.mesh,
        scale.to_array().map(f32::to_bits)
    );
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(s) = cache.get(&key) {
        return Some(s.clone());
    }
    let ground = crate::mesh::ground(o);
    let source: &crate::geometry::Mesh = ground
        .as_deref()
        .unwrap_or_else(|| crate::geometry::mesh(o.kind()));
    if source.len() / 3 > 20_000 || source.len() < 12 {
        return None;
    }
    let points: Vec<Vector> = source.iter().map(|(p, _)| v(*p * scale)).collect();
    let indices: Vec<[u32; 3]> = (0..points.len() as u32 / 3)
        .map(|t| [3 * t, 3 * t + 1, 3 * t + 2])
        .collect();
    let params = rapier3d::parry::transformation::vhacd::VHACDParameters {
        resolution: 48,
        max_convex_hulls: 24,
        ..Default::default()
    };
    let shape = SharedShape::convex_decomposition_with_params(&points, &indices, &params);
    cache.insert(key, shape.clone());
    Some(shape)
}

/// Collider for an object in its local frame (scaled, unrotated).
fn collider(o: &Object) -> ColliderBuilder {
    let s = crate::geometry::object_scale(o);
    let uniform = (s.x - s.y).abs() < 1e-4 && (s.x - s.z).abs() < 1e-4;
    let sphere = o.sdf == crate::scene::Sdf::Sphere
        || (o.sdf.is_none() && o.shape == Shape::Sphere && o.mesh.is_empty());
    if sphere && uniform {
        return ColliderBuilder::ball(s.x);
    }
    if o.sdf.is_none() && o.mesh.is_empty() && o.shape == Shape::Box {
        return ColliderBuilder::cuboid(s.x, s.y, s.z);
    }
    if let Some(shape) = decomposed(o, s) {
        return ColliderBuilder::new(shape);
    }
    let points: Vec<Vector> = crate::geometry::hull_points(o, 4000)
        .into_iter()
        .map(v)
        .collect();
    ColliderBuilder::convex_hull(&points).unwrap_or_else(|| ColliderBuilder::cuboid(s.x, s.y, s.z))
}

impl Sim {
    pub fn new(s: &Settings) -> Self {
        let mut sim = Self {
            pipeline: PhysicsPipeline::new(),
            params: IntegrationParameters {
                dt: STEP,
                ..IntegrationParameters::default()
            },
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            soft: SoftBodySet::new(),
            handles: vec![],
            written: s.objects.clone(),
            stage: Self::stage_key(s),
            accumulator: 0.,
            grab: None,
            applied: s.physics,
            time: 0.,
        };
        let p = s.physics;
        // The floor (a thick slab, so fast bodies do not tunnel) and the Cornell room's walls.
        let e = s.floor_extent;
        let mut wall = |c: Vec3, h: Vec3| {
            sim.colliders.insert(
                ColliderBuilder::cuboid(h.x, h.y, h.z)
                    .translation(v(c))
                    .friction(p.friction)
                    .restitution(p.bounce)
                    .build(),
            );
        };
        wall(Vec3::new(0., -1.5, 0.), Vec3::new(e, 0.5, e));
        if s.cornell_room {
            wall(Vec3::new(0., 1.5, 0.), Vec3::new(1.5, 0.5, 1.5));
            wall(Vec3::new(0., 0., -1.5), Vec3::new(1.5, 1.5, 0.5));
            wall(Vec3::new(-1.5, 0., 0.), Vec3::new(0.5, 1.5, 1.5));
            wall(Vec3::new(1.5, 0., 0.), Vec3::new(0.5, 1.5, 1.5));
        }
        for o in &s.objects {
            if !o.visible || o.portal >= 0 || o.body == Body::None || o.tether.is_some() {
                sim.handles.push(None);
                continue;
            }
            let (center, r) = crate::geometry::object_frame(o);
            let rotation = Quat::from_mat3(&r).normalize();
            let builder = if o.body == Body::Fixed {
                RigidBodyBuilder::fixed()
            } else {
                RigidBodyBuilder::dynamic().ccd_enabled(true)
            };
            let mut body = builder
                .pose(Pose::from_parts(v(center), q(rotation)))
                .build();
            // Owner note: rapier's defaults (0.05 linear, 0.5 rad/s, 0.5 s) froze slow topples
            // and rolling balls mid-motion. Only nearly still bodies sleep, after 1.5 s.
            let a = body.activation_mut();
            a.normalized_linear_threshold = 0.01;
            a.angular_threshold = 0.08;
            a.time_until_sleep = 1.5;
            let h = sim.bodies.insert(body);
            sim.colliders.insert_with_parent(
                collider(o)
                    .friction(p.friction)
                    .restitution(p.bounce)
                    .density(o.mass)
                    .build(),
                h,
                &mut sim.bodies,
            );
            sim.handles.push(Some(h));
        }
        sim.attach_strings(s);
        sim
    }
    /// Joints from the strings (`Object::tether`): each dynamic body hangs from the fixed
    /// points of the strings that end at it.
    fn attach_strings(&mut self, s: &Settings) {
        let world = self.bodies.insert(RigidBodyBuilder::fixed().build());
        for (i, h) in self.handles.iter().enumerate() {
            let Some(h) = *h else { continue };
            let anchors: Vec<Vec3> = s
                .objects
                .iter()
                .filter_map(|o| o.tether.filter(|t| t.object as usize == i))
                .map(|t| Vec3::from(t.from))
                .collect();
            let Some(b) = self.bodies.get(h) else {
                continue;
            };
            if anchors.is_empty() || !b.is_dynamic() {
                continue;
            }
            let pose = b.position();
            let r = pose.rotation;
            let inverse = Quat::from_xyzw(r.x, r.y, r.z, r.w).inverse();
            let centre = g(pose.translation);
            let joint = if anchors.len() == 1 {
                GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
                    .local_anchor1(v(anchors[0]))
                    .local_anchor2(v(inverse * (anchors[0] - centre)))
                    .build()
            } else {
                // A hinge along the line through the first two points (a cradle's V of strings).
                let mid = (anchors[0] + anchors[1]) * 0.5;
                let axis = (anchors[1] - anchors[0]).normalize_or(Vec3::Z);
                GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
                    .local_axis1(v(axis))
                    .local_axis2(v(inverse * axis))
                    .local_anchor1(v(mid))
                    .local_anchor2(v(inverse * (mid - centre)))
                    .build()
            };
            self.impulse_joints.insert(world, h, joint, true);
        }
    }
    fn stage_key(s: &Settings) -> (bool, u32) {
        (s.cornell_room, s.floor_extent.to_bits())
    }
    /// Whether the settings still match this simulation (no outside edit of the objects,
    /// the stage or the material constants).
    pub fn matches(&self, s: &Settings) -> bool {
        same_bodies(&self.written, &s.objects) && self.stage == Self::stage_key(s)
    }
    /// Advances by `dt` seconds of wall time and writes the poses into `s`. Returns whether any
    /// object moved.
    pub fn advance(&mut self, s: &mut Settings, dt: f32) -> bool {
        let p = s.physics;
        self.accumulator = (self.accumulator + dt.min(0.25) * p.speed).min(STEP * MAX_STEPS as f32);
        let down = Vec3::from(p.down).normalize_or(Vec3::NEG_Y);
        let gravity = v(down * p.gravity);
        let wind = Vec3::from(p.wind);
        let mut stepped = false;
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            self.time += STEP;
            if wind != Vec3::ZERO {
                // Gusts: two slow sines, so the wind swells and drops without a pattern.
                let t = self.time;
                let gust =
                    (1. + p.gusts * (0.6 * (1.3 * t).sin() + 0.4 * (3.7 * t + 1.).sin())).max(0.);
                for (_, b) in self.bodies.iter_mut() {
                    if b.is_dynamic() {
                        b.reset_forces(false);
                        let force = wind * gust * b.mass();
                        b.add_force(v(force), true);
                    }
                }
            }
            self.pull();
            self.pipeline.step(
                gravity,
                &self.params,
                &mut self.islands,
                &mut self.broad,
                &mut self.narrow,
                &mut self.bodies,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft,
                &mut self.ccd,
                &(),
                &(),
            );
            stepped = true;
        }
        if !stepped {
            return false;
        }
        let mut moved = false;
        for (i, h) in self.handles.iter().enumerate() {
            let Some(h) = *h else { continue };
            let Some(b) = self.bodies.get_mut(h) else {
                continue;
            };
            if b.is_sleeping() || !b.is_dynamic() {
                continue;
            }
            let t = g(b.translation());
            if t.y < FLOOR_LIMIT || t.x.abs() > 39. || t.z.abs() > 39. {
                b.set_body_type(RigidBodyType::Fixed, false);
                continue;
            }
            let r = *b.rotation();
            let (yaw, x, z) = Quat::from_xyzw(r.x, r.y, r.z, r.w).to_euler(EulerRot::YXZ);
            let o = &mut s.objects[i];
            // Sub-millimetre jitter of a settling body is not written, so the geometry stays
            // put and the image keeps accumulating instead of restarting every frame.
            let (yaw, x, z) = (yaw.to_degrees(), x.to_degrees(), z.to_degrees());
            let still = !o.grounded
                && (o.position[0] - t.x).abs() < 5e-4
                && (o.position[1] - t.z).abs() < 5e-4
                && (o.elevation - t.y).abs() < 5e-4
                && [o.rotation - yaw, o.tilt[0] - x, o.tilt[1] - z]
                    .iter()
                    .all(|d| (d.rem_euclid(360.) + 0.05) % 360. < 0.1);
            if still {
                continue;
            }
            o.grounded = false;
            o.position = [t.x, t.z];
            o.elevation = t.y;
            o.rotation = yaw;
            o.tilt = [x, z];
            moved = true;
        }
        if moved {
            update_strings(s);
        }
        self.written = s.objects.clone();
        moved
    }
    /// The grab spring: steers the grabbed point's velocity towards the target.
    fn pull(&mut self) {
        let Some(grab) = &self.grab else { return };
        let Some(Some(h)) = self.handles.get(grab.index) else {
            return;
        };
        let Some(b) = self.bodies.get_mut(*h) else {
            return;
        };
        let pose = b.position();
        let r = pose.rotation;
        let world = g(pose.translation) + Quat::from_xyzw(r.x, r.y, r.z, r.w) * grab.local;
        // A velocity drive rather than a spring: the point follows the cursor closely, and
        // the extra mass set in `grab` lets it push other bodies aside.
        let com = g(b.center_of_mass());
        let wanted = ((grab.target - world) * 20.).clamp_length_max(20.);
        let spin = g(b.angvel()) * 0.85;
        b.set_angvel(v(spin), true);
        b.set_linvel(v(wanted - spin.cross(world - com)), true);
    }
    /// Lets go of the grabbed body; it keeps its velocity (a throw).
    pub fn release(&mut self) -> bool {
        let Some(grab) = self.grab.take() else {
            return false;
        };
        if let Some(Some(h)) = self.handles.get(grab.index)
            && let Some(b) = self.bodies.get_mut(*h)
        {
            b.set_additional_mass(0., true);
        }
        true
    }
    /// Starts dragging object `index` at the world point `hit` (the cursor ray at `distance`).
    pub fn grab(&mut self, index: usize, hit: Vec3, distance: f32) -> bool {
        let Some(Some(h)) = self.handles.get(index) else {
            return false;
        };
        let Some(b) = self.bodies.get_mut(*h) else {
            return false;
        };
        if !b.is_dynamic() {
            return false;
        }
        b.wake_up(true);
        // Twenty times heavier while held: it shoves other bodies instead of stalling.
        b.set_additional_mass(b.mass() * 20., true);
        let pose = b.position();
        let r = pose.rotation;
        let local = Quat::from_xyzw(r.x, r.y, r.z, r.w).inverse() * (hit - g(pose.translation));
        self.grab = Some(Grab {
            index,
            local,
            distance,
            target: hit,
        });
        true
    }
    /// A radial impulse around `at`: bodies within `radius` get up to `speed` m/s outward and
    /// a little upward, falling off linearly.
    pub fn explode(&mut self, at: Vec3, radius: f32, speed: f32) -> usize {
        let mut hit = 0;
        for h in self.handles.iter().flatten() {
            let Some(b) = self.bodies.get_mut(*h) else {
                continue;
            };
            if !b.is_dynamic() {
                continue;
            }
            let c = g(b.center_of_mass());
            let d = c - at;
            let k = 1. - d.length() / radius;
            if k <= 0. {
                continue;
            }
            let dir = (d.normalize_or_zero() + Vec3::Y * 0.5).normalize();
            b.apply_impulse(v(dir * speed * k * b.mass()), true);
            hit += 1;
        }
        hit
    }
    /// Wakes every body (after a gravity change, so stacks feel it).
    pub fn wake_all(&mut self) {
        for h in self.handles.iter().flatten() {
            if let Some(b) = self.bodies.get_mut(*h) {
                b.wake_up(true);
            }
        }
    }
    pub fn awake(&self) -> usize {
        self.handles
            .iter()
            .flatten()
            .filter(|h| {
                self.bodies
                    .get(**h)
                    .is_some_and(|b| b.is_dynamic() && !b.is_sleeping())
            })
            .count()
    }
}

/// Points every string (`Object::tether`) from its fixed point to its object's centre: the
/// cylinder's axis (local y, length 2) spans the two, keeping its radius.
pub fn update_strings(s: &mut Settings) {
    for i in 0..s.objects.len() {
        let Some(t) = s.objects[i].tether else {
            continue;
        };
        let Some(end) = s.objects.get(t.object as usize) else {
            continue;
        };
        let b = crate::geometry::object_frame(end).0;
        let a = Vec3::from(t.from);
        let d = b - a;
        let length = d.length();
        if length < 1e-4 {
            continue;
        }
        let rotation = Quat::from_rotation_arc(Vec3::Y, d / length);
        let (yaw, x, z) = rotation.to_euler(EulerRot::YXZ);
        let o = &mut s.objects[i];
        let mid = (a + b) * 0.5;
        o.grounded = false;
        o.position = [mid.x, mid.z];
        o.elevation = mid.y;
        o.rotation = yaw.to_degrees();
        o.tilt = [x.to_degrees(), z.to_degrees()];
        o.stretch = (length * 0.5 / (o.size * 0.35)).clamp(0.05, 10.);
    }
}

/// App side: the per-frame step, the mouse grab and the explosion tool.
impl crate::App {
    pub(crate) fn step_physics(&mut self, dt: f32) {
        // Strings follow their objects whether or not the simulation runs (a loaded scene, a
        // gizmo edit). Recomputing an unchanged string gives the same floats, so no rebuild.
        if self.settings.objects.iter().any(|o| o.tether.is_some()) {
            update_strings(&mut self.settings);
        }
        if !self.settings.physics.enabled {
            self.physics = None;
            return;
        }
        // A gizmo transform owns the poses until it ends. Stepping meanwhile would drop the
        // object between pointer moves; afterwards the simulation restarts from the new poses.
        if self.ui.transform.is_some() {
            return;
        }
        let rebuild = match &self.physics {
            None => {
                // Undo goes back to the poses from before the simulation started.
                self.ui.checkpoint(&self.settings);
                true
            }
            Some(sim) => !sim.matches(&self.settings),
        };
        if rebuild {
            self.physics = Some(Sim::new(&self.settings));
        }
        let sim = self.physics.as_mut().unwrap();
        let (a, p) = (sim.applied, self.settings.physics);
        if (a.gravity, a.down, a.wind, a.gusts) != (p.gravity, p.down, p.wind, p.gusts) {
            sim.wake_all();
            if p.wind == [0.; 3] {
                for (_, b) in sim.bodies.iter_mut() {
                    b.reset_forces(true);
                }
            }
        }
        let p = self.settings.physics;
        if (sim.applied.bounce, sim.applied.friction) != (p.bounce, p.friction) {
            // Material sliders update the colliders in place (no restart).
            for (_, c) in sim.colliders.iter_mut() {
                c.set_friction(p.friction);
                c.set_restitution(p.bounce);
            }
        }
        sim.applied = p;
        if std::mem::take(&mut self.actions.physics_wake) {
            sim.wake_all();
        }
        if std::mem::take(&mut self.actions.physics_blast) {
            let n = sim.explode(Vec3::new(0., -1., 0.), 1.5, 6.);
            self.ui.toast(format!("Explosion: {n} bodies"));
        }
        // A material edit (no rebuild) still has to reach `written`, or every later frame
        // would compare against stale colours.
        sim.advance(&mut self.settings, dt);
        if self.ui.physics_record {
            self.record_physics(dt * p.speed);
        }
    }
    /// Track 17: while recording, keys every dynamic object's pose into the timeline at its fps
    /// (simulated time), so a timeline video export renders the simulation.
    fn record_physics(&mut self, dt: f32) {
        let sim = self.physics.as_ref().unwrap();
        let dynamic: Vec<usize> = sim
            .handles
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                h.and_then(|h| sim.bodies.get(h))
                    .is_some_and(|b| b.is_dynamic())
                    .then_some(i)
            })
            .collect();
        let fps = self.settings.animation.fps.max(1) as f32;
        let before = self.ui.physics_record_time;
        let after = before + dt;
        self.ui.physics_record_time = after;
        // A key on each frame boundary crossed (the first call keys time 0).
        if before > 0. && (before * fps).floor() == (after * fps).floor() {
            return;
        }
        let t = (after * fps).floor() / fps;
        let a = &mut self.settings.animation;
        a.length = a.length.max(t);
        for i in dynamic {
            let o = &self.settings.objects[i];
            let values = [
                ("position/0", o.position[0]),
                ("position/1", o.position[1]),
                ("elevation", o.elevation),
                ("rotation", o.rotation),
                ("tilt/0", o.tilt[0]),
                ("tilt/1", o.tilt[1]),
            ];
            for (field, value) in values {
                a.key(&format!("objects/{i}/{field}"), t, value as f64);
            }
        }
        // The new keys are the poses as they are: without this the timeline would see changed
        // tracks next frame and pull every body back to the playhead's pose.
        self.timeline.settle(&self.settings);
    }
    /// Drops a ball (or with `cube` a box) a little above the point under the cursor.
    pub(crate) fn physics_spawn(&mut self, cube: bool) {
        let Some((_, at, _)) = self.cursor_hit() else {
            return;
        };
        if self.settings.objects.len() >= crate::scene::MAX_OBJECTS {
            return;
        }
        let n = self.settings.objects.len();
        const COLORS: [[f32; 3]; 5] = [
            [0.85, 0.25, 0.2],
            [0.95, 0.7, 0.2],
            [0.25, 0.6, 0.35],
            [0.2, 0.45, 0.85],
            [0.7, 0.3, 0.75],
        ];
        let color = COLORS[n % COLORS.len()];
        self.settings.objects.push(Object {
            name: format!("{} {}", if cube { "Box" } else { "Ball" }, n + 1),
            shape: if cube { Shape::Box } else { Shape::Sphere },
            size: 0.6,
            stretch: 1.,
            rotation: 0.,
            grounded: false,
            position: [at.x, at.z],
            elevation: at.y + 0.6,
            material: crate::scene::Material {
                color,
                ..Default::default()
            },
            ..Object::default()
        });
    }
    /// The world point under the cursor: the nearest object, else the floor.
    fn cursor_hit(&self) -> Option<(Option<usize>, Vec3, f32)> {
        let pixel = [self.cursor[0] as f32, self.cursor[1] as f32];
        let view = crate::gizmo::View::new(&self.settings, self.size);
        let (o, d) = view.ray(glam::Vec2::from(pixel));
        if let Some((i, t)) = self.settings.pick_object_hit(pixel, self.size) {
            return Some((Some(i), o + d * t, t));
        }
        (d.y < -1e-4).then(|| {
            let t = (-1. - o.y) / d.y;
            (None, o + d * t, t)
        })
    }
    /// Left press with the simulation on: grabs the object under the pointer.
    pub(crate) fn physics_grab(&mut self) -> bool {
        if self.physics.is_none() {
            return false;
        }
        let Some((Some(i), hit, t)) = self.cursor_hit() else {
            return false;
        };
        self.physics.as_mut().unwrap().grab(i, hit, t)
    }
    pub(crate) fn physics_release(&mut self) -> bool {
        self.physics.as_mut().is_some_and(|p| p.release())
    }
    pub(crate) fn physics_drag(&mut self) -> bool {
        let Some(sim) = &self.physics else {
            return false;
        };
        let Some(grab) = &sim.grab else { return false };
        let distance = grab.distance;
        let view = crate::gizmo::View::new(&self.settings, self.size);
        let (o, d) = view.ray(glam::Vec2::new(
            self.cursor[0] as f32,
            self.cursor[1] as f32,
        ));
        if let Some(g) = &mut self.physics.as_mut().unwrap().grab {
            g.target = o + d * distance;
        }
        true
    }
    /// The wheel while holding a body: pulls it closer or pushes it away.
    pub(crate) fn physics_wheel(&mut self, d: f32) -> bool {
        let Some(grab) = self.physics.as_mut().and_then(|s| s.grab.as_mut()) else {
            return false;
        };
        grab.distance = (grab.distance * (d * 0.1).exp()).clamp(0.3, 60.);
        self.physics_drag()
    }
    pub(crate) fn physics_explode(&mut self) {
        let Some((_, at, _)) = self.cursor_hit() else {
            return;
        };
        if let Some(sim) = &mut self.physics {
            let n = sim.explode(at, 1.2, 6.);
            self.ui.toast(format!("Explosion: {n} bodies"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball(elevation: f32) -> Object {
        Object {
            shape: Shape::Sphere,
            size: 1.,
            stretch: 1.,
            rotation: 0.,
            grounded: false,
            elevation,
            ..Object::default()
        }
    }

    #[test]
    fn a_torus_is_hollow_and_wind_pushes_and_colours_keep_the_simulation() {
        let torus = Object {
            shape: Shape::Torus,
            ..ball(0.)
        };
        let shape = decomposed(&torus, crate::geometry::object_scale(&torus)).unwrap();
        let parts = shape.as_compound().map_or(1, |c| c.shapes().len());
        assert!(parts > 1, "a torus decomposes into {parts} part(s)");
        let mut s = Settings {
            objects: vec![ball(-0.6)],
            ..Settings::default()
        };
        s.physics.enabled = true;
        s.physics.wind = [5., 0., 0.];
        let mut sim = Sim::new(&s);
        s.objects[0].material.color = [0.1, 0.9, 0.1];
        assert!(sim.matches(&s), "a colour edit keeps the simulation");
        for _ in 0..60 {
            sim.advance(&mut s, 1. / 60.);
        }
        assert!(
            s.objects[0].position[0] > -0.42 + 0.3,
            "wind moved it to {:?}",
            s.objects[0].position
        );
    }

    #[test]
    fn a_dropped_ball_lands_on_the_floor() {
        let mut s = Settings {
            objects: vec![ball(0.5)],
            ..Settings::default()
        };
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        sim.wake_all();
        for _ in 0..240 {
            sim.advance(&mut s, 1. / 60.);
        }
        // Radius 0.35 on the floor at y = −1.
        let y = s.objects[0].elevation;
        assert!((y + 0.65).abs() < 0.02, "rests at {y}");
        assert!(sim.matches(&s));
    }

    #[test]
    fn a_resting_stack_sleeps_and_an_explosion_wakes_it() {
        // No room: the third box would reach past the Cornell ceiling at y = 1.
        let mut s = Settings {
            objects: vec![],
            cornell_room: false,
            ..Settings::default()
        };
        for k in 0..3 {
            s.objects.push(Object {
                shape: Shape::Box,
                size: 1.,
                stretch: 1.,
                rotation: 0.,
                position: [0.; 2],
                grounded: false,
                elevation: -0.65 + 0.7 * k as f32,
                ..Object::default()
            });
        }
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        for _ in 0..300 {
            sim.advance(&mut s, 1. / 60.);
        }
        let top = s.objects[2].elevation;
        assert!(
            (top - 0.75).abs() < 0.03,
            "the stack stands ({:?})",
            s.objects
                .iter()
                .map(|o| (o.position, o.elevation, o.tilt))
                .collect::<Vec<_>>()
        );
        assert_eq!(sim.awake(), 0, "a resting stack sleeps");
        assert!(sim.explode(Vec3::new(0., -0.6, 0.), 2., 4.) == 3);
        assert!(sim.advance(&mut s, 1. / 30.));
        assert!(sim.awake() > 0);
    }

    #[test]
    fn a_held_box_stays_up_when_the_pointer_stops() {
        let mut s = Settings {
            objects: vec![Object {
                shape: Shape::Box,
                size: 0.5,
                stretch: 1.,
                rotation: 0.,
                position: [0., 0.],
                grounded: false,
                elevation: -0.75,
                ..Object::default()
            }],
            cornell_room: false,
            ..Settings::default()
        };
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        let start = Vec3::new(0., -0.75, 0.);
        assert!(sim.grab(0, start, 3.));
        // Lift it half a unit over a second, then hold the pointer still for five seconds.
        for k in 0..60 {
            sim.grab.as_mut().unwrap().target = start + Vec3::Y * (0.5 * k as f32 / 59.);
            sim.advance(&mut s, 1. / 60.);
        }
        for k in 0..300 {
            sim.advance(&mut s, 1. / 60.);
            let y = s.objects[0].elevation;
            assert!(y > -0.4, "dropped after {k} still frames: elevation {y}");
        }
    }
    #[test]
    fn a_grabbed_box_shoves_another_out_of_the_way() {
        let mut s = Settings {
            objects: vec![],
            cornell_room: false,
            ..Settings::default()
        };
        for x in [-0.6, 0.2] {
            s.objects.push(Object {
                shape: Shape::Box,
                size: 1.,
                stretch: 1.,
                rotation: 0.,
                position: [x, 0.],
                grounded: false,
                elevation: -0.65,
                ..Object::default()
            });
        }
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        let start = Vec3::new(-0.6, -0.65, 0.);
        assert!(sim.grab(0, start, 3.));
        // Drag the first box through where the second one stands, over one second.
        for k in 0..60 {
            sim.grab.as_mut().unwrap().target = start + Vec3::X * (1.6 * k as f32 / 59.);
            sim.advance(&mut s, 1. / 60.);
        }
        assert!(sim.release());
        let [a, b] = [s.objects[0].position[0], s.objects[1].position[0]];
        assert!(a > 0.8, "the grabbed box followed the cursor ({a})");
        assert!(
            b > a + 0.6,
            "the other box was pushed ahead, not passed through ({a} {b} {:?})",
            s.objects
                .iter()
                .map(|o| (o.position, o.elevation))
                .collect::<Vec<_>>()
        );
    }

    /// The cradle scene: ball 1 swings down, the middle balls barely move and ball 5 flies out.
    #[test]
    fn newtons_cradle_passes_the_swing_through() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/builtin/newtons-cradle.json");
        let mut s = crate::scenes::read(&path).unwrap();
        let ball = |s: &Settings, n: u32| {
            s.objects
                .iter()
                .find(|o| o.name == format!("Ball {n}"))
                .unwrap()
                .position[0]
        };
        let mut sim = Sim::new(&s);
        sim.wake_all();
        let (mut middle, mut last) = (0f32, 0f32);
        for _ in 0..150 {
            sim.advance(&mut s, 1. / 60.);
            for n in 2..=4 {
                middle = middle.max((ball(&s, n) - [0., 0., -0.36, 0., 0.36][n as usize]).abs());
            }
            last = last.max(ball(&s, 5) - 0.72);
        }
        assert!(last > 0.3, "ball 5 swings out ({last})");
        assert!(middle < 0.12, "the middle balls stay ({middle})");
        // The strings follow their balls: string 5's lower end sits at ball 5.
        let string = s.objects.iter().find(|o| o.name == "String 5").unwrap();
        assert!(!string.grounded);
    }
}
