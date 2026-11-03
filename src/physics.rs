//! Track 17 physics sandbox: rapier3d rigid bodies driven from `Settings::objects`, plus cloth
//! (an XPBD particle grid of our own) and joints between bodies (chains, ropes, bridges).
//!
//! The simulation owns the poses while it runs: each step writes the bodies back into the
//! objects (`grounded` off, `elevation` = body centre), which changes only the geometry key's
//! transforms, so the renderer rebuilds just the TLAS. Outside edits (gizmo, inspector, undo,
//! spawning, Freeze) are synced body by body (`Sim::sync`): only the edited bodies are replaced,
//! the others keep their velocities. Objects driven by keyframes become kinematic bodies that
//! follow the animation and push the others. See engine-notes "Physics sandbox".
use crate::scene::{Body, Object, Settings, Shape};
use glam::{EulerRot, Quat, Vec3};
use rapier3d::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    /// One body per object (None for hidden objects, portals, strings and cloths).
    handles: Vec<Option<RigidBodyHandle>>,
    /// The fixed body strings hang from.
    world: RigidBodyHandle,
    /// Objects keyframed on a transform field (kinematic bodies), per object.
    animated: Vec<bool>,
    cloths: Vec<ClothSim>,
    /// The objects as the simulation last wrote them: anything else means an outside edit.
    pub written: Vec<Object>,
    /// The settings the static colliders came from (floor and room).
    stage: (bool, u32),
    accumulator: f32,
    pub grab: Option<Grab>,
    /// Dynamic bodies a gizmo transform holds, made kinematic until it ends (`hold`).
    held: Vec<usize>,
    /// The physics settings last applied.
    applied: crate::scene::Physics,
    /// Simulated seconds (wind gusts).
    time: f32,
}
/// Whether two objects give the same body: pose, shape and body settings, not colours or
/// names (a material edit keeps the simulation and its velocities).
fn same_body(x: &Object, y: &Object) -> bool {
    same_shape(x, y)
        && (x.position, x.elevation, x.rotation, x.tilt)
            == (y.position, y.elevation, y.rotation, y.tilt)
}
/// `same_body` without the pose (a kinematic body follows its pose without a restart).
fn same_shape(x: &Object, y: &Object) -> bool {
    (
        x.shape, x.sphere, x.sdf, x.body, x.visible, x.portal, x.grounded,
    ) == (
        y.shape, y.sphere, y.sdf, y.body, y.visible, y.portal, y.grounded,
    ) && (x.size, x.stretch, x.scale_xz, x.mass) == (y.size, y.stretch, y.scale_xz, y.mass)
        && x.mesh == y.mesh
        && x.tether == y.tether
        && x.links == y.links
        && x.cloth == y.cloth
}
fn same_bodies(a: &[Object], b: &[Object]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_body(x, y))
}
fn jointed(o: &Object) -> bool {
    o.tether.is_some() || !o.links.is_empty()
}
/// Which objects have keyframes on a transform field (they become kinematic bodies). None
/// while recording: the recorded keys are the simulation's own output.
fn animated(s: &Settings, recording: bool) -> Vec<bool> {
    let mut out = vec![false; s.objects.len()];
    if recording {
        return out;
    }
    for t in &s.animation.tracks {
        let Some(rest) = t.path.strip_prefix("objects/") else {
            continue;
        };
        let (i, field) = rest.split_once('/').unwrap_or((rest, ""));
        let pose = ["position", "elevation", "rotation", "tilt", ""]
            .iter()
            .any(|f| field == *f || field.starts_with(&format!("{f}/")));
        // A keyed fixed body is replaced at each new pose (rapier moves only kinematic ones).
        if let (Ok(i), true) = (i.parse::<usize>(), pose)
            && i < out.len()
            && s.objects[i].body != Body::Fixed
        {
            out[i] = true;
        }
    }
    out
}

/// Shapes with holes or hollows (tori, rings, knots, teapots, bunnies, concave lenses,
/// imported meshes) as a compound of convex parts (V-HACD), so a ring can hang on a peg and
/// a bowl holds a ball. Meshes over 20k triangles keep the hull (the decomposition would
/// stall the frame). Cached by shape and scale: a restart (any outside edit) reuses them.
fn concave(o: &Object) -> bool {
    match o.sdf {
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
    }
}
/// Triangles scaled into an object's local frame, as rapier points and indices.
fn local_triangles(source: &crate::geometry::Mesh, scale: Vec3) -> (Vec<Vector>, Vec<[u32; 3]>) {
    let points: Vec<Vector> = source.iter().map(|(p, _)| v(*p * scale)).collect();
    let indices: Vec<[u32; 3]> = (0..points.len() as u32 / 3)
        .map(|t| [3 * t, 3 * t + 1, 3 * t + 2])
        .collect();
    (points, indices)
}
/// A fixed concave mesh collides as its exact triangles: a fixed body never needs a volume
/// (rapier handles dynamic bodies and cloth against a triangle mesh), and a convex hull or
/// decomposition fills the openings. Sponza's arcades must keep their arches open for the
/// cloth curtains (scene sponza-cloth).
fn fixed_trimesh(o: &Object, scale: Vec3) -> Option<SharedShape> {
    static CACHE: Mutex<Option<HashMap<String, SharedShape>>> = Mutex::new(None);
    if o.body != Body::Fixed || !concave(o) {
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
    // The object's own triangles, as drawn (a part of a file, not the whole file).
    let part = crate::mesh::of(o);
    let source = part
        .as_deref()
        .unwrap_or_else(|| crate::geometry::mesh(o.kind()));
    let (points, indices) = local_triangles(source, scale);
    if indices.is_empty() {
        return None;
    }
    let shape = SharedShape::trimesh(points, indices).ok()?;
    cache.insert(key, shape.clone());
    Some(shape)
}
fn decomposed(o: &Object, scale: Vec3) -> Option<SharedShape> {
    static CACHE: Mutex<Option<HashMap<String, SharedShape>>> = Mutex::new(None);
    if !concave(o) {
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
    let source = ground
        .as_deref()
        .unwrap_or_else(|| crate::geometry::mesh(o.kind()));
    let (points, indices) = local_triangles(source, scale);
    if indices.len() > 20_000 || indices.len() < 4 {
        return None;
    }
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
    if let Some(shape) = fixed_trimesh(o, s) {
        return ColliderBuilder::new(shape);
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

// --- Cloth -------------------------------------------------------------------------------

/// Meshes the cloths published, by `Object::mesh` key (`cloth:<object>/<generation>`); the
/// renderer reads them through `mesh::of`. The last two generations per cloth are kept.
static CLOTH_MESHES: Mutex<Option<HashMap<String, Arc<crate::geometry::Mesh>>>> = Mutex::new(None);
/// A cloth's mesh in its object's local frame: the published one for `key`, else the flat
/// rest grid over the local x-z square [-1, 1]² at y = 0.
pub fn cloth_mesh(key: &str, c: &crate::scene::Cloth) -> Arc<crate::geometry::Mesh> {
    if let Some(m) = CLOTH_MESHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(key))
    {
        return m.clone();
    }
    let n = c.resolution.clamp(4, 48) as usize;
    let rest: Vec<Vec3> = (0..n * n)
        .map(|k| {
            let (i, j) = (k % n, k / n);
            Vec3::new(
                i as f32 / (n - 1) as f32 * 2. - 1.,
                0.,
                j as f32 / (n - 1) as f32 * 2. - 1.,
            )
        })
        .collect();
    Arc::new(grid_mesh(&rest, n))
}
/// A cloth's texture coordinates, parallel to its mesh (`grid_mesh`'s triangle order): the
/// particle grid spread over the `Cloth::uv` rectangle.
pub fn cloth_uvs(c: &crate::scene::Cloth) -> crate::mesh::Uvs {
    let n = c.resolution.clamp(4, 48) as usize;
    let [u0, v0, u1, v1] = c.uv;
    let at = |k: usize| {
        let (i, j) = ((k % n) as f32, (k / n) as f32);
        let s = (n - 1) as f32;
        [u0 + (u1 - u0) * i / s, v0 + (v1 - v0) * j / s]
    };
    let mut out = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let (a, b, cc, d) = (
                j * n + i,
                j * n + i + 1,
                (j + 1) * n + i,
                (j + 1) * n + i + 1,
            );
            out.extend([a, cc, b, b, cc, d].map(at));
        }
    }
    out
}
/// Triangles of an n × n particle grid with smooth normals (one side; the renderer shades
/// back faces too).
fn grid_mesh(p: &[Vec3], n: usize) -> crate::geometry::Mesh {
    let mut normals = vec![Vec3::ZERO; p.len()];
    let quads = (0..n - 1).flat_map(|j| (0..n - 1).map(move |i| (i, j)));
    for (i, j) in quads.clone() {
        let (a, b, c, d) = (
            j * n + i,
            j * n + i + 1,
            (j + 1) * n + i,
            (j + 1) * n + i + 1,
        );
        for [x, y, z] in [[a, c, b], [b, c, d]] {
            let f = (p[y] - p[x]).cross(p[z] - p[x]);
            normals[x] += f;
            normals[y] += f;
            normals[z] += f;
        }
    }
    let normals: Vec<Vec3> = normals.iter().map(|m| m.normalize_or(Vec3::Y)).collect();
    let mut out = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for (i, j) in quads {
        let (a, b, c, d) = (
            j * n + i,
            j * n + i + 1,
            (j + 1) * n + i,
            (j + 1) * n + i + 1,
        );
        for k in [a, c, b, b, c, d] {
            out.push((p[k], normals[k]));
        }
    }
    out
}
/// One simulated cloth: position-based dynamics (XPBD-style Gauss-Seidel distance
/// constraints, Verlet integration), pinned particles, collisions with the floor and every
/// collider (pushed out to `THICKNESS`). One-way: the cloth does not push bodies.
struct ClothSim {
    index: usize,
    n: usize,
    p: Vec<Vec3>,
    prev: Vec<Vec3>,
    /// Pinned particles stay at their start.
    pinned: Vec<Option<Vec3>>,
    /// (a, b, rest length, stiffness).
    constraints: Vec<(u32, u32, f32, f32)>,
    /// Keys this cloth published, newest last (the last two stay in `CLOTH_MESHES`).
    published: Vec<String>,
    /// The object's frame: centre, rotation, scale (to write the mesh back in local space).
    frame: (Vec3, Quat, Vec3),
    /// Steps with almost no motion; asleep after a second of them.
    still: u32,
}
const THICKNESS: f32 = 0.02;
impl ClothSim {
    fn new(index: usize, o: &Object) -> Option<Self> {
        let c = o.cloth?;
        let n = c.resolution.clamp(4, 48) as usize;
        let (centre, r) = crate::geometry::object_frame(o);
        let rot = Quat::from_mat3(&r).normalize();
        let scale = crate::geometry::object_scale(o);
        // Start from the mesh it shows (a restart keeps the drape), else the rest grid.
        let shown = cloth_mesh(&o.mesh, &c);
        let rest = cloth_mesh("", &c);
        let local = |m: &crate::geometry::Mesh| -> Option<Vec<Vec3>> {
            let mut out = vec![Vec3::ZERO; n * n];
            let mut seen = vec![false; n * n];
            // grid_mesh emits quads in order: recover each particle from the triangles.
            let quads = (0..n - 1).flat_map(|j| (0..n - 1).map(move |i| (i, j)));
            for (t, (i, j)) in quads.enumerate() {
                let (a, b, cc, d) = (
                    j * n + i,
                    j * n + i + 1,
                    (j + 1) * n + i,
                    (j + 1) * n + i + 1,
                );
                for (k, idx) in [a, cc, b, b, cc, d].into_iter().enumerate() {
                    let (pos, _) = *m.get(t * 6 + k)?;
                    out[idx] = pos;
                    seen[idx] = true;
                }
            }
            seen.iter().all(|s| *s).then_some(out)
        };
        let start = local(&shown).or_else(|| local(&rest))?;
        let world = |l: Vec3| centre + rot * (l * scale);
        let p: Vec<Vec3> = start.iter().map(|l| world(*l)).collect();
        let rest_world: Vec<Vec3> = local(&rest)?.iter().map(|l| world(*l)).collect();
        let pinned = (0..n * n)
            .map(|k| {
                let (i, j) = (k % n, k / n);
                let pin = match c.pin {
                    crate::scene::ClothPin::None => false,
                    crate::scene::ClothPin::Corners => j == 0 && (i == 0 || i == n - 1),
                    crate::scene::ClothPin::Edge => j == 0,
                };
                pin.then_some(rest_world[k])
            })
            .collect();
        let mut constraints = vec![];
        let mut add = |a: usize, b: usize, k: f32| {
            let l = (rest_world[a] - rest_world[b]).length();
            constraints.push((a as u32, b as u32, l, k));
        };
        let bend = (0.05 + 0.95 * c.stiffness).min(1.);
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if i + 1 < n {
                    add(k, k + 1, 1.);
                }
                if j + 1 < n {
                    add(k, k + n, 1.);
                }
                if i + 1 < n && j + 1 < n {
                    add(k, k + n + 1, 0.8);
                    add(k + 1, k + n, 0.8);
                }
                if i + 2 < n {
                    add(k, k + 2, bend);
                }
                if j + 2 < n {
                    add(k, k + 2 * n, bend);
                }
            }
        }
        Some(Self {
            index,
            n,
            prev: p.clone(),
            p,
            pinned,
            constraints,
            published: vec![],
            frame: (centre, rot, scale),
            still: 0,
        })
    }
    fn asleep(&self) -> bool {
        self.still > 120
    }
    /// `meshes`: the triangle-mesh colliders (fixed concave meshes), left out of `query`.
    fn step(
        &mut self,
        gravity: Vec3,
        wind: Vec3,
        query: &QueryPipeline,
        meshes: &[(Pose, SharedShape)],
    ) {
        let dt2 = STEP * STEP;
        let mut motion = 0f32;
        for k in 0..self.p.len() {
            if let Some(pin) = self.pinned[k] {
                self.p[k] = pin;
                self.prev[k] = pin;
                continue;
            }
            let vel = (self.p[k] - self.prev[k]) * 0.99;
            self.prev[k] = self.p[k];
            // Wind pushes harder on cloth than on solids (area per mass): 4×.
            self.p[k] += vel + (gravity + wind * 4.) * dt2;
            motion = motion.max(vel.length());
        }
        for _ in 0..6 {
            for &(a, b, rest, k) in &self.constraints {
                let (a, b) = (a as usize, b as usize);
                let d = self.p[b] - self.p[a];
                let l = d.length();
                if l < 1e-9 {
                    continue;
                }
                let corr = d * ((l - rest) / l * k);
                match (self.pinned[a].is_some(), self.pinned[b].is_some()) {
                    (true, true) => {}
                    (true, false) => self.p[b] -= corr,
                    (false, true) => self.p[a] += corr,
                    (false, false) => {
                        self.p[a] += corr * 0.5;
                        self.p[b] -= corr * 0.5;
                    }
                }
            }
        }
        for k in 0..self.p.len() {
            if self.pinned[k].is_some() {
                continue;
            }
            let mut p = self.p[k];
            let mut touched = false;
            if p.y < -1. + THICKNESS {
                p.y = -1. + THICKNESS;
                touched = true;
            }
            // rapier's `project_point` culls colliders by box only, then searches a triangle mesh
            // for its nearest triangle with no distance bound: with Sponza's parts (whole-building
            // boxes) that was most of the frame. The mesh's own bounded query prunes its BVH.
            let mut nearest = query
                .project_point(v(p), THICKNESS * 2., false)
                .map(|(_, proj)| proj);
            for (pose, shape) in meshes {
                if let Some(proj) =
                    shape.project_point_with_max_dist(pose, v(p), false, THICKNESS * 2.)
                    && nearest.is_none_or(|n| g(n.point).distance(p) > g(proj.point).distance(p))
                {
                    nearest = Some(proj);
                }
            }
            if let Some(proj) = nearest {
                let s = g(proj.point);
                let d = p - s;
                let len = d.length();
                if proj.is_inside {
                    // Out through the surface (up when the point sits on it).
                    p = s - d.normalize_or(Vec3::NEG_Y) * THICKNESS;
                    touched = true;
                } else if len < THICKNESS {
                    p = s + d.normalize_or(Vec3::Y) * THICKNESS;
                    touched = true;
                }
            }
            if touched {
                // Friction: a contact keeps most of the particle from sliding.
                self.prev[k] = self.prev[k].lerp(p, 0.6);
            }
            self.p[k] = p;
        }
        // Asleep below 1.2 cm/s at its fastest particle (a hanging corner sways for long).
        self.still = if motion < 1e-4 { self.still + 1 } else { 0 };
    }
    /// The mesh in the object's frame, published under a new key written to the object.
    fn publish(&mut self, o: &mut Object) {
        let (c, r, s) = self.frame;
        let inverse = r.inverse();
        let safe = Vec3::select(s.abs().cmpgt(Vec3::splat(1e-6)), s, Vec3::ONE);
        let local: Vec<Vec3> = self.p.iter().map(|p| (inverse * (*p - c)) / safe).collect();
        // A global counter: a restarted simulation never reuses a key the renderer has seen.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let key = format!("cloth:{}/{generation}", self.index);
        let mut m = CLOTH_MESHES.lock().unwrap_or_else(|e| e.into_inner());
        let m = m.get_or_insert_with(HashMap::new);
        while self.published.len() >= 2 {
            m.remove(&self.published.remove(0));
        }
        m.insert(key.clone(), Arc::new(grid_mesh(&local, self.n)));
        self.published.push(key.clone());
        o.mesh = key;
        // The vertices are relative to the frame the cloth started in: keep the object there
        // (grounding would shift it by the drape's lowest point).
        o.grounded = false;
        o.position = [c.x, c.z];
        o.elevation = c.y;
    }
}

impl Sim {
    pub fn new(s: &Settings) -> Self {
        Self::with(s, false)
    }
    /// A new simulation of `s`; `recording`: keyframed objects stay dynamic (see `animated`).
    pub fn with(s: &Settings, recording: bool) -> Self {
        let mut bodies = RigidBodySet::new();
        let world = bodies.insert(RigidBodyBuilder::fixed().build());
        let mut sim = Self {
            pipeline: PhysicsPipeline::new(),
            params: IntegrationParameters {
                dt: STEP,
                ..IntegrationParameters::default()
            },
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies,
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            soft: SoftBodySet::new(),
            handles: vec![],
            world,
            animated: animated(s, recording),
            cloths: vec![],
            written: s.objects.clone(),
            stage: Self::stage_key(s),
            accumulator: 0.,
            grab: None,
            held: vec![],
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
        for (i, o) in s.objects.iter().enumerate() {
            let h = sim.insert_body(i, o);
            sim.handles.push(h);
            if let Some(c) = ClothSim::new(i, o) {
                sim.cloths.push(c);
            }
        }
        sim.attach_strings(s);
        sim.attach_links(s);
        sim
    }
    /// The rigid body for object `i` (None for objects that are not bodies).
    fn insert_body(&mut self, i: usize, o: &Object) -> Option<RigidBodyHandle> {
        if !o.visible
            || o.portal >= 0
            || o.body == Body::None
            || o.tether.is_some()
            || o.cloth.is_some()
        {
            return None;
        }
        let (center, r) = crate::geometry::object_frame(o);
        let rotation = Quat::from_mat3(&r).normalize();
        let kinematic = self.animated.get(i).copied().unwrap_or(false);
        let builder = if o.body == Body::Fixed {
            RigidBodyBuilder::fixed()
        } else if kinematic {
            RigidBodyBuilder::kinematic_position_based()
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
        let h = self.bodies.insert(body);
        self.bodies[h].wake_up(true);
        let p = self.applied;
        self.colliders.insert_with_parent(
            collider(o)
                .friction(p.friction)
                .restitution(p.bounce)
                .density(o.mass)
                .build(),
            h,
            &mut self.bodies,
        );
        Some(h)
    }
    fn remove_body(&mut self, i: usize) {
        if let Some(Some(h)) = self.handles.get(i).copied() {
            self.bodies.remove(
                h,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft,
                true,
            );
        }
        if self.grab.as_ref().is_some_and(|g| g.index == i) {
            self.grab = None;
        }
        self.cloths.retain(|c| c.index != i);
    }
    /// Joints from the strings (`Object::tether`): each dynamic body hangs from the fixed
    /// points of the strings that end at it.
    fn attach_strings(&mut self, s: &Settings) {
        let world = self.world;
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
    /// Ball joints from `Object::links`, anchored where the world point sits in each body now.
    fn attach_links(&mut self, s: &Settings) {
        for (i, o) in s.objects.iter().enumerate() {
            for l in &o.links {
                let (Some(Some(a)), Some(Some(b))) = (
                    self.handles.get(i).copied(),
                    self.handles.get(l.object as usize).copied(),
                ) else {
                    continue;
                };
                let local = |h: RigidBodyHandle| {
                    let pose = self.bodies[h].position();
                    let r = pose.rotation;
                    Quat::from_xyzw(r.x, r.y, r.z, r.w).inverse()
                        * (Vec3::from(l.at) - g(pose.translation))
                };
                // Jointed neighbours touch at the joint: no contacts between them.
                let joint = GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
                    .local_anchor1(v(local(a)))
                    .local_anchor2(v(local(b)))
                    .contacts_enabled(false)
                    .build();
                self.impulse_joints.insert(a, b, joint, true);
            }
        }
    }
    fn stage_key(s: &Settings) -> (bool, u32) {
        (s.cornell_room, s.floor_extent.to_bits())
    }
    /// Whether the settings still match this simulation (no outside edit of the objects,
    /// the stage or the material constants).
    #[cfg(test)]
    pub fn matches(&self, s: &Settings) -> bool {
        same_bodies(&self.written, &s.objects) && self.stage == Self::stage_key(s)
    }
    /// Takes outside edits into the running simulation body by body: an edited object's body
    /// is replaced (it starts at rest), a new object gets a body, a deleted one loses it, and
    /// every other body keeps its velocity. A keyframed (kinematic) body moves to its new
    /// pose. Returns false when only a restart fits: the stage changed, an edit touches a
    /// joint (strings, links), several objects were deleted, or what is keyframed changed.
    pub fn sync(&mut self, s: &Settings, recording: bool) -> bool {
        if self.stage != Self::stage_key(s) {
            return false;
        }
        let old = &self.written;
        let new = &s.objects;
        if same_bodies(old, new) && animated(s, recording) == self.animated {
            self.written = new.clone();
            return true;
        }
        let mut wanted = animated(s, recording);
        // A deletion: one object gone, the rest shifted down.
        if new.len() + 1 == old.len() {
            let i = (0..new.len())
                .find(|&i| !same_body(&old[i], &new[i]))
                .unwrap_or(new.len());
            let shifted = old[i + 1..]
                .iter()
                .zip(&new[i..])
                .all(|(a, b)| same_body(a, b));
            if !shifted || old.iter().any(jointed) {
                return false;
            }
            wanted.insert(i, false);
            if wanted
                .iter()
                .enumerate()
                .any(|(k, a)| k != i && self.animated.get(k) != Some(a))
            {
                return false;
            }
            self.remove_body(i);
            self.handles.remove(i);
            self.animated.remove(i);
            for c in &mut self.cloths {
                if c.index > i {
                    c.index -= 1;
                }
            }
            if let Some(g) = &mut self.grab
                && g.index > i
            {
                g.index -= 1;
            }
            self.written = new.clone();
            return true;
        }
        if new.len() < old.len() {
            return false;
        }
        // What is keyframed changed for an existing object: its body type changes, restart.
        if (0..old.len()).any(|i| self.animated.get(i) != wanted.get(i)) {
            return false;
        }
        let pointed_at = |i: usize| {
            new.iter().chain(old).any(|o| {
                o.tether.is_some_and(|t| t.object as usize == i)
                    || o.links.iter().any(|l| l.object as usize == i)
            })
        };
        let mut edits = vec![];
        for i in 0..new.len() {
            if i < old.len() && same_body(&old[i], &new[i]) {
                continue;
            }
            if i < old.len()
                && (self.animated[i] || self.held.contains(&i))
                && same_shape(&old[i], &new[i])
            {
                edits.push((i, true));
                continue;
            }
            if jointed(&new[i]) || (i < old.len() && jointed(&old[i])) || pointed_at(i) {
                return false;
            }
            edits.push((i, false));
        }
        self.animated = wanted;
        for (i, kinematic_move) in edits {
            let o = &new[i];
            if kinematic_move {
                if let Some(Some(h)) = self.handles.get(i).copied() {
                    let (c, r) = crate::geometry::object_frame(o);
                    let pose = Pose::from_parts(v(c), q(Quat::from_mat3(&r).normalize()));
                    self.bodies[h].set_next_kinematic_position(pose);
                }
                continue;
            }
            if i < self.handles.len() {
                self.remove_body(i);
                self.handles[i] = self.insert_body(i, o);
                // A held body that changed shape (a scale) stays held: kinematic, not falling.
                if self.held.contains(&i)
                    && let Some(Some(h)) = self.handles.get(i)
                    && let Some(b) = self.bodies.get_mut(*h)
                    && b.is_dynamic()
                {
                    b.set_body_type(RigidBodyType::KinematicPositionBased, true);
                }
            } else {
                let h = self.insert_body(i, o);
                self.handles.push(h);
            }
            if let Some(c) = ClothSim::new(i, o) {
                self.cloths.push(c);
            }
            // Bodies resting against the edited one must feel the change.
            if let Some(Some(h)) = self.handles.get(i).copied() {
                self.bodies[h].wake_up(true);
            }
            for c in &mut self.cloths {
                c.still = 0;
            }
        }
        self.written = new.clone();
        self.wake_touching();
        true
    }
    /// A gizmo transform holds `objects` (owner report 2026-11-03): their dynamic bodies turn
    /// kinematic, so `sync` moves them to the edited poses and they push the others, which
    /// keep simulating. Bodies no longer held turn dynamic again, at rest where they were put.
    pub fn hold(&mut self, objects: &[usize]) {
        for i in std::mem::take(&mut self.held) {
            if objects.contains(&i) {
                self.held.push(i);
            } else if let Some(Some(h)) = self.handles.get(i)
                && let Some(b) = self.bodies.get_mut(*h)
            {
                b.set_body_type(RigidBodyType::Dynamic, true);
                b.set_linvel(v(Vec3::ZERO), true);
                b.set_angvel(v(Vec3::ZERO), true);
            }
        }
        for &i in objects {
            if self.held.contains(&i) || self.grab.as_ref().is_some_and(|g| g.index == i) {
                continue;
            }
            if let Some(Some(h)) = self.handles.get(i)
                && let Some(b) = self.bodies.get_mut(*h)
                && b.is_dynamic()
            {
                b.set_body_type(RigidBodyType::KinematicPositionBased, true);
                self.held.push(i);
            }
        }
    }
    /// After an edit: wakes every body whose island is asleep near an awake one (rapier
    /// wakes contacts of a moving body itself; a removed support needs this).
    fn wake_touching(&mut self) {
        for h in self.handles.iter().flatten() {
            if let Some(b) = self.bodies.get_mut(*h)
                && b.is_dynamic()
                && b.is_sleeping()
            {
                b.wake_up(false);
            }
        }
    }
    /// Advances by `dt` seconds of wall time and writes the poses into `s`. Returns whether any
    /// object moved.
    pub fn advance(&mut self, s: &mut Settings, dt: f32) -> bool {
        let p = s.physics;
        self.accumulator = (self.accumulator + dt.min(0.25) * p.speed).min(STEP * MAX_STEPS as f32);
        let mut steps = 0;
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            steps += 1;
        }
        if steps == 0 {
            return false;
        }
        self.steps(s, steps);
        self.write_back(s, false)
    }
    /// Advances exactly `seconds` of simulated time (video exports: frame by frame, whatever
    /// the wall time) and writes the poses into `s`.
    pub fn run(&mut self, s: &mut Settings, seconds: f32) -> bool {
        // The remainder carries over, so any frame rate keeps simulated time on the clock.
        self.accumulator += seconds.max(0.);
        let n = (self.accumulator / STEP + 1e-4).floor() as u32;
        self.accumulator -= n as f32 * STEP;
        if n == 0 {
            return false;
        }
        self.steps(s, n);
        self.write_back(s, true)
    }
    fn steps(&mut self, s: &Settings, n: u32) {
        let p = s.physics;
        let down = Vec3::from(p.down).normalize_or(Vec3::NEG_Y);
        let gravity = down * p.gravity;
        let wind = Vec3::from(p.wind);
        for _ in 0..n {
            self.time += STEP;
            let t = self.time;
            // Gusts: two slow sines, so the wind swells and drops without a pattern.
            let gust =
                (1. + p.gusts * (0.6 * (1.3 * t).sin() + 0.4 * (3.7 * t + 1.).sin())).max(0.);
            if wind != Vec3::ZERO {
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
                v(gravity),
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
            if !self.cloths.is_empty() {
                // Rigid bodies only: counting the cloths themselves would keep them awake.
                let bodies_awake = wind != Vec3::ZERO
                    || self.handles.iter().flatten().any(|h| {
                        self.bodies
                            .get(*h)
                            .is_some_and(|b| !b.is_fixed() && !b.is_sleeping())
                    });
                let meshes: Vec<(Pose, SharedShape)> = self
                    .colliders
                    .iter()
                    .filter(|(_, c)| c.shape().as_trimesh().is_some())
                    .map(|(_, c)| (*c.position(), c.shared_shape().clone()))
                    .collect();
                for c in &mut self.cloths {
                    if bodies_awake {
                        c.still = c.still.min(60);
                    }
                }
                // The cloths are independent (one-way collisions against the same bodies), so
                // they step in parallel with the same result. Each thread builds its own query
                // view: the filter's predicate isn't Sync.
                let (broad, narrow, bodies, colliders) =
                    (&self.broad, &self.narrow, &self.bodies, &self.colliders);
                let meshes = &meshes;
                let wind = wind * gust;
                let step = move |part: &mut [ClothSim]| {
                    let not_mesh = |_, c: &Collider| c.shape().as_trimesh().is_none();
                    let query = broad.as_query_pipeline(
                        narrow.query_dispatcher(),
                        bodies,
                        colliders,
                        QueryFilter::default().predicate(&not_mesh),
                    );
                    for c in part.iter_mut().filter(|c| !c.asleep()) {
                        c.step(gravity, wind, &query, meshes);
                    }
                };
                let awake = self.cloths.iter().filter(|c| !c.asleep()).count();
                let threads = std::thread::available_parallelism()
                    .map_or(4, |n| n.get())
                    .min(awake);
                if threads <= 1 {
                    step(&mut self.cloths);
                } else {
                    let chunk = self.cloths.len().div_ceil(threads);
                    std::thread::scope(|scope| {
                        for part in self.cloths.chunks_mut(chunk) {
                            scope.spawn(move || step(part));
                        }
                    });
                }
            }
        }
    }
    /// Writes the bodies' poses into the objects; `all` also writes bodies that barely moved
    /// (exports, where every frame starts from the scene's own poses).
    fn write_back(&mut self, s: &mut Settings, all: bool) -> bool {
        let mut moved = false;
        for (i, h) in self.handles.iter().enumerate() {
            let Some(h) = *h else { continue };
            let Some(b) = self.bodies.get_mut(h) else {
                continue;
            };
            if !b.is_dynamic() || (b.is_sleeping() && !all) {
                continue;
            }
            let t = g(b.translation());
            if t.y < FLOOR_LIMIT || t.x.abs() > 39. || t.z.abs() > 39. {
                b.set_body_type(RigidBodyType::Fixed, false);
                continue;
            }
            let r = *b.rotation();
            let (yaw, x, z) = Quat::from_xyzw(r.x, r.y, r.z, r.w).to_euler(EulerRot::YXZ);
            let Some(o) = s.objects.get_mut(i) else {
                continue;
            };
            // Sub-millimetre jitter of a settling body is not written, so the geometry stays
            // put and the image keeps accumulating instead of restarting every frame.
            let (yaw, x, z) = (yaw.to_degrees(), x.to_degrees(), z.to_degrees());
            let still = !all
                && !o.grounded
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
        for c in &mut self.cloths {
            if (c.asleep() && !all && !c.published.is_empty()) || c.index >= s.objects.len() {
                continue;
            }
            c.publish(&mut s.objects[c.index]);
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
        // Cloth particles in the blast fly too.
        for c in &mut self.cloths {
            for k in 0..c.p.len() {
                let d = c.p[k] - at;
                let f = 1. - d.length() / radius;
                if f > 0. && c.pinned[k].is_none() {
                    c.prev[k] -=
                        (d.normalize_or_zero() + Vec3::Y * 0.5).normalize() * speed * f * STEP;
                }
            }
            c.still = 0;
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
        for c in &mut self.cloths {
            c.still = 0;
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
            + self.cloths.iter().filter(|c| !c.asleep()).count()
    }
    /// Puts the simulated poses (dynamic bodies, cloth meshes, strings) into `s`, whose
    /// objects otherwise come from elsewhere (a video export's keyframe evaluation).
    pub fn take_poses(&self, s: &mut Settings) {
        for i in self.moving_objects() {
            let (Some(w), Some(o)) = (self.written.get(i), s.objects.get_mut(i)) else {
                continue;
            };
            o.grounded = w.grounded;
            o.position = w.position;
            o.elevation = w.elevation;
            o.rotation = w.rotation;
            o.tilt = w.tilt;
            o.mesh = w.mesh.clone();
        }
        update_strings(s);
    }
    /// Simulated seconds since the simulation started (a restart begins at 0).
    pub fn elapsed(&self) -> f32 {
        self.time
    }
    /// The objects that are dynamic bodies or cloths (what Record keys).
    pub fn moving_objects(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .handles
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                h.and_then(|h| self.bodies.get(h))
                    .is_some_and(|b| b.is_dynamic())
                    .then_some(i)
                    .or_else(|| self.held.contains(&i).then_some(i))
            })
            .collect();
        v.extend(self.cloths.iter().map(|c| c.index));
        v.sort_unstable();
        v
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

/// Drops keys that the remaining keys reproduce by linear interpolation within `tolerance`
/// (one pass, greedy from the left). Record keys every frame; most of a resting or falling
/// body's keys are redundant.
pub fn reduce_keys(track: &mut crate::anim::Track, tolerance: f64) -> usize {
    let k = &track.keys;
    if k.len() < 3 {
        return 0;
    }
    let mut keep = vec![k[0].clone()];
    let mut anchor = 0;
    for i in 1..k.len() - 1 {
        // Can every key between the anchor and i + 1 be dropped?
        let (a, b) = (&k[anchor], &k[i + 1]);
        let span = (b.time - a.time).max(1e-6) as f64;
        let fits = (anchor + 1..=i).all(|j| {
            let u = (k[j].time - a.time) as f64 / span;
            (a.value + (b.value - a.value) * u - k[j].value).abs() <= tolerance
        });
        if !fits {
            keep.push(k[i].clone());
            anchor = i;
        }
    }
    keep.push(k[k.len() - 1].clone());
    let removed = k.len() - keep.len();
    track.keys = keep;
    removed
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
            self.finish_recording();
            return;
        }
        // The objects a gizmo transform moves are kinematic while it runs: they follow the
        // edited poses (gravity can't drop them between pointer moves) and push the rest,
        // which keeps simulating. Afterwards they join the running world at rest.
        let held: Vec<usize> = self.ui.transform.as_ref().map_or(vec![], |t| {
            t.targets
                .iter()
                .filter_map(|s| match s {
                    crate::ui::Selection::Object(i) => Some(*i),
                    _ => None,
                })
                .collect()
        });
        // Strings and links: a held end changes the joint (its length), which only a restart
        // takes, so the simulation waits for the transform to end, as before.
        let jointed_held = held.iter().any(|&i| {
            self.settings.objects.get(i).is_some_and(jointed)
                || self.settings.objects.iter().any(|o| {
                    o.tether.is_some_and(|t| t.object as usize == i)
                        || o.links.iter().any(|l| l.object as usize == i)
                })
        });
        if jointed_held {
            return;
        }
        let recording = self.ui.physics_record;
        let rebuild = match &mut self.physics {
            None => {
                // Undo goes back to the poses from before the simulation started.
                self.ui.checkpoint(&self.settings);
                true
            }
            Some(sim) => {
                sim.hold(&held);
                !sim.sync(&self.settings, recording)
            }
        };
        if rebuild {
            let mut sim = Sim::with(&self.settings, recording);
            sim.hold(&held);
            self.physics = Some(sim);
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
        sim.advance(&mut self.settings, dt);
        if recording {
            self.record_physics(dt * p.speed);
        } else {
            self.finish_recording();
        }
    }
    /// Track 17: while recording, keys every dynamic object's pose into the timeline at its fps
    /// (simulated time), so a timeline video export renders the simulation. A new recording
    /// first clears the earlier transform keys of the bodies it records.
    fn record_physics(&mut self, dt: f32) {
        let dynamic = self.physics.as_ref().unwrap().moving_objects();
        // Nothing to record (an empty prefix list would match every track below).
        if dynamic.is_empty() {
            return;
        }
        if self.ui.physics_recorded.is_empty() {
            let prefixes: Vec<String> = dynamic.iter().map(|i| format!("objects/{i}")).collect();
            self.settings
                .animation
                .tracks
                .retain(|t| !crate::anim::under_any(&t.path, &prefixes) || !pose_track(&t.path));
            self.ui.physics_recorded = dynamic.clone();
        }
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
            if o.cloth.is_some() {
                // Cloth shapes are meshes, not keyable floats: recordings key bodies only.
                continue;
            }
            let values = [
                ("position/0", o.position[0]),
                ("position/1", o.position[1]),
                ("elevation", o.elevation),
                ("rotation", o.rotation),
                ("tilt/0", o.tilt[0]),
                ("tilt/1", o.tilt[1]),
            ];
            for (field, value) in values {
                let path = format!("objects/{i}/{field}");
                a.key(&path, t, value as f64);
                // Linear: the reduction after the recording assumes straight segments.
                if let Some(track) = a.tracks.iter_mut().find(|tr| tr.path == path)
                    && let Some(k) = track.keys.iter_mut().find(|k| (k.time - t).abs() < 1e-4)
                {
                    k.interp = crate::anim::Interp::Linear;
                }
            }
        }
        // The new keys are the poses as they are: without this the timeline would see changed
        // tracks next frame and pull every body back to the playhead's pose.
        self.timeline.settle(&self.settings);
    }
    /// After Record stops: thins out the recorded tracks (keys a straight line reproduces
    /// within 2 mm or 0.2° go).
    fn finish_recording(&mut self) {
        if self.ui.physics_recorded.is_empty() {
            return;
        }
        let prefixes: Vec<String> = std::mem::take(&mut self.ui.physics_recorded)
            .iter()
            .map(|i| format!("objects/{i}"))
            .collect();
        let mut removed = 0;
        for t in &mut self.settings.animation.tracks {
            if crate::anim::under_any(&t.path, &prefixes) && pose_track(&t.path) {
                let angle = t.path.ends_with("rotation") || t.path.contains("/tilt/");
                removed += reduce_keys(t, if angle { 0.2 } else { 0.002 });
            }
        }
        self.timeline.settle(&self.settings);
        if removed > 0 {
            self.ui
                .toast(format!("Recording kept; {removed} redundant keys dropped"));
        }
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
/// A transform field of an object track (what Record writes).
fn pose_track(path: &str) -> bool {
    ["/position/", "/elevation", "/rotation", "/tilt/"]
        .iter()
        .any(|f| path.contains(f) && !path.contains("/material/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Timing probe for sponza-cloth (needs the Sponza assets): `cargo test --release
    /// sponza_cloth_cost -- --ignored --nocapture`, optionally with CAUSTICA_CLOTH_SCENE.
    #[test]
    #[ignore]
    fn sponza_cloth_cost() {
        let path = std::env::var("CAUSTICA_CLOTH_SCENE")
            .unwrap_or_else(|_| "scenes/meshes/sponza-cloth.json".into());
        let mut s = crate::scenes::read(std::path::Path::new(&path)).expect("scene");
        let t = std::time::Instant::now();
        let mut sim = Sim::new(&s);
        println!("setup {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        for second in 0..4 {
            let t = std::time::Instant::now();
            let mut synced = 0;
            for _ in 0..60 {
                synced += usize::from(!same_bodies(&sim.written, &s.objects));
                assert!(sim.sync(&s, false));
                sim.advance(&mut s, 1. / 60.);
            }
            println!("  frames whose objects differ from the written ones: {synced}");
            let awake = sim.cloths.iter().filter(|c| !c.asleep()).count();
            println!(
                "second {second}: {:.2} ms per frame, {awake} of {} cloths awake",
                t.elapsed().as_secs_f64() * 1e3 / 60.,
                sim.cloths.len()
            );
        }
    }

    #[test]
    fn cloth_uvs_follow_the_mesh() {
        let c = crate::scene::Cloth {
            resolution: 6,
            uv: [0.5, 0.25, 2.5, 1.25],
            ..Default::default()
        };
        let mesh = cloth_mesh("", &c);
        let uvs = cloth_uvs(&c);
        assert_eq!(mesh.len(), uvs.len());
        // The rest grid spans local x-z [-1, 1]: u follows x, v follows z (from the -z edge).
        for ((p, _), uv) in mesh.iter().zip(&uvs) {
            assert!(
                (uv[0] - (0.5 + (p.x + 1.) / 2. * 2.)).abs() < 1e-4,
                "{p} {uv:?}"
            );
            assert!(
                (uv[1] - (0.25 + (p.z + 1.) / 2.)).abs() < 1e-4,
                "{p} {uv:?}"
            );
        }
    }

    #[test]
    fn fixed_concave_meshes_collide_as_triangles() {
        let torus = |body| Object {
            shape: Shape::Torus,
            body,
            ..Object::default()
        };
        let s = crate::geometry::object_scale(&torus(Body::Fixed));
        let shape = fixed_trimesh(&torus(Body::Fixed), s).expect("a triangle mesh");
        assert!(shape.as_trimesh().is_some());
        // The hole stays open: its centre is well away from the surface (a convex hull or a
        // coarse decomposition would cover it).
        let centre = shape.project_local_point(v(Vec3::ZERO), false);
        assert!(g(centre.point).length() > 0.2 * s.x, "{:?}", centre.point);
        assert!(fixed_trimesh(&torus(Body::Dynamic), s).is_none());
    }

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

    /// Owner report 2026-11-03: a gizmo drag moves its body kinematically while the rest keeps
    /// simulating (as `step_physics` does: hold, sync the edited pose, advance).
    #[test]
    fn a_gizmo_held_box_pushes_the_others() {
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
                elevation: -0.4,
                ..Object::default()
            });
        }
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        // Held half a unit up and dragged through the second box over one second: gravity
        // must not drop it, and the other box (which falls meanwhile) is pushed ahead.
        for k in 0..60 {
            s.objects[0].position[0] = -0.6 + 1.6 * k as f32 / 59.;
            sim.hold(&[0]);
            assert!(sim.sync(&s, false), "a held move restarted the simulation");
            sim.advance(&mut s, 1. / 60.);
            assert_eq!(
                s.objects[0].elevation, -0.4,
                "the held box moved on its own"
            );
        }
        let [a, b] = [s.objects[0].position[0], s.objects[1].position[0]];
        assert!(b > a + 0.6, "the other box was pushed ahead ({a} {b})");
        // A scale while held replaces the body (new shape); it stays held, not falling.
        for k in 0..30 {
            s.objects[0].size = 1. + 0.2 * k as f32 / 29.;
            sim.hold(&[0]);
            assert!(sim.sync(&s, false));
            sim.advance(&mut s, 1. / 60.);
            assert_eq!(s.objects[0].elevation, -0.4, "the scaled box fell");
        }
        assert_eq!(
            sim.moving_objects(),
            vec![0, 1],
            "Record keys the held box too"
        );
        // Let go: it is dynamic again and falls.
        sim.hold(&[]);
        assert!(sim.sync(&s, false));
        for _ in 0..60 {
            sim.advance(&mut s, 1. / 60.);
        }
        assert!(s.objects[0].elevation < -0.45, "the released box stayed up");
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

    #[test]
    fn edits_sync_body_by_body_and_keep_the_other_velocities() {
        let mut s = Settings {
            // Below the Cornell room's ceiling (y 1).
            objects: vec![ball(0.5), ball(0.6)],
            ..Settings::default()
        };
        s.objects[1].position = [0.9, 0.];
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        for _ in 0..20 {
            sim.advance(&mut s, 1. / 60.);
        }
        let falling = sim.bodies[sim.handles[0].unwrap()].linvel().y;
        assert!(falling < -1., "falling at {falling}");
        // A spawn and an edit of the second ball: the first keeps falling as it was.
        s.objects[1].size = 0.5;
        s.objects.push(Object {
            position: [-0.9, 0.],
            ..ball(0.3)
        });
        assert!(sim.sync(&s, false), "a spawn and a body edit sync");
        assert_eq!(sim.handles.len(), 3);
        let after = sim.bodies[sim.handles[0].unwrap()].linvel().y;
        assert_eq!(falling, after, "the first ball kept its velocity");
        // A deletion shifts the bodies down.
        s.objects.remove(1);
        assert!(sim.sync(&s, false));
        assert_eq!(sim.handles.len(), 2);
        let y = sim.bodies[sim.handles[1].unwrap()].translation().y;
        assert!(
            (y - 0.3).abs() < 0.05,
            "the spawned ball is body 1 now (y {y})"
        );
        // A changed stage restarts.
        s.cornell_room = !s.cornell_room;
        assert!(!sim.sync(&s, false));
    }

    #[test]
    fn a_keyed_object_is_kinematic_and_pushes_without_being_pushed() {
        let mut s = Settings {
            objects: vec![ball(-0.5), ball(-0.5)],
            ..Settings::default()
        };
        s.objects[0].position = [-2., 0.];
        s.objects[1].position = [0., 0.];
        s.physics.enabled = true;
        s.animation.key("objects/0/position/0", 0., -2.);
        s.animation.key("objects/0/position/0", 1., 2.);
        let mut sim = Sim::new(&s);
        assert!(sim.animated[0] && !sim.animated[1]);
        for f in 1..=60 {
            let mut next = s.animation.apply(&s, f as f32 / 60.);
            sim.take_poses(&mut next);
            s = next;
            assert!(sim.sync(&s, false), "a keyed move syncs (frame {f})");
            sim.run(&mut s, 1. / 60.);
        }
        assert!(
            (s.objects[0].position[0] - 2.).abs() < 0.05,
            "the keys drove ball 0 to {:?}",
            s.objects[0].position
        );
        assert!(
            s.objects[1].position[0] > 0.5,
            "ball 0 pushed ball 1 to {:?}",
            s.objects[1].position
        );
        // While recording the keyed object is a dynamic body again.
        assert!(!sim.sync(&s, true));
    }

    #[test]
    fn a_chain_hangs_from_a_fixed_hook() {
        // A fixed hook and three links, each jointed to the one above at their touching ends.
        let link = |y: f32| Object {
            shape: Shape::Box,
            size: 0.29,
            stretch: 2.,
            elevation: y,
            position: [0., 0.],
            ..ball(0.)
        };
        let mut hook = link(0.9);
        hook.body = Body::Fixed;
        let mut s = Settings {
            objects: vec![hook, link(0.5), link(0.1), link(-0.3)],
            ..Settings::default()
        };
        for i in 1..4 {
            let top = 0.9 - 0.4 * i as f32 + 0.2;
            s.objects[i].links.push(crate::scene::Link {
                object: i as u32 - 1,
                at: [0., top, 0.],
            });
        }
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        // Kick the last link sideways: the chain swings and still hangs together.
        let h = sim.handles[3].unwrap();
        sim.bodies[h].set_linvel(Vector::new(3., 0., 0.), true);
        for _ in 0..240 {
            sim.advance(&mut s, 1. / 60.);
        }
        for i in 1..4 {
            let (a, b) = (&s.objects[i - 1], &s.objects[i]);
            let d =
                (crate::geometry::object_frame(a).0 - crate::geometry::object_frame(b).0).length();
            assert!(d < 0.5, "links {} and {i} are {d} apart", i - 1);
        }
        assert!(
            s.objects[3].elevation > -0.75,
            "the chain did not fall ({})",
            s.objects[3].elevation
        );
    }

    #[test]
    fn a_cloth_drapes_over_a_box_and_a_curtain_hangs() {
        let mut cloth = Object {
            shape: Shape::Box,
            size: 2.,
            stretch: 1.,
            grounded: false,
            elevation: 0.2,
            cloth: Some(crate::scene::Cloth {
                resolution: 12,
                ..Default::default()
            }),
            ..Object::default()
        };
        let mut s = Settings {
            objects: vec![ball(-0.6), cloth.clone()],
            ..Settings::default()
        };
        s.objects[0].shape = Shape::Box;
        s.objects[0].body = Body::Fixed;
        // Clear of the Cornell room's walls.
        s.objects[0].position = [0., 0.];
        s.objects[1].position = [0., 0.];
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        assert!(sim.handles[1].is_none(), "a cloth is no rigid body");
        for _ in 0..180 {
            sim.advance(&mut s, 1. / 60.);
        }
        assert!(
            s.objects[1].mesh.starts_with("cloth:1/"),
            "mesh key {}",
            s.objects[1].mesh
        );
        let m = crate::mesh::of(&s.objects[1]).unwrap();
        let (c, r) = crate::geometry::object_frame(&s.objects[1]);
        let scale = crate::geometry::object_scale(&s.objects[1]);
        let world: Vec<Vec3> = m.iter().map(|(p, _)| c + r * (*p * scale)).collect();
        let top = world.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let low = world.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        // The box top is at -0.6 + 0.35: the middle rests on it, the corners hang lower.
        assert!(
            top > -0.3 && top < -0.15,
            "the cloth rests on the box (top {top})"
        );
        assert!(
            low < top - 0.2,
            "the corners hang down (low {low}, top {top})"
        );
        // Settled, it sleeps (and stops publishing meshes).
        for _ in 0..300 {
            sim.advance(&mut s, 1. / 60.);
        }
        assert!(sim.cloths[0].asleep(), "the draped cloth still moves");
        let key = s.objects[1].mesh.clone();
        sim.advance(&mut s, 1. / 60.);
        assert_eq!(s.objects[1].mesh, key, "a sleeping cloth published a mesh");
        // A curtain pinned along its far edge keeps that edge.
        cloth.cloth = Some(crate::scene::Cloth {
            resolution: 8,
            pin: crate::scene::ClothPin::Edge,
            stiffness: 0.2,
            ..Default::default()
        });
        cloth.tilt = [90., 0.];
        cloth.elevation = 0.5;
        cloth.position = [0., 0.];
        let mut s = Settings {
            objects: vec![cloth],
            ..Settings::default()
        };
        s.physics.enabled = true;
        let mut sim = Sim::new(&s);
        for _ in 0..60 {
            sim.advance(&mut s, 1. / 60.);
        }
        let c = &sim.cloths[0];
        assert!(c.pinned[0].is_some_and(|p| (p - c.p[0]).length() < 1e-5));
        assert!(
            c.p.iter().all(|p| p.y > -1.),
            "nothing fell through the floor"
        );
    }

    #[test]
    fn key_reduction_keeps_the_shape_of_a_track() {
        let mut t = crate::anim::Track {
            path: "objects/0/elevation".into(),
            keys: (0..=60)
                .map(|f| {
                    let time = f as f32 / 60.;
                    // Falls (a parabola), then rests.
                    let y = if time < 0.5 {
                        1. - 4. * (time * time) as f64
                    } else {
                        0.
                    };
                    crate::anim::Key {
                        time,
                        value: y,
                        interp: crate::anim::Interp::Linear,
                        text: None,
                    }
                })
                .collect(),
        };
        let original = t.clone();
        let removed = reduce_keys(&mut t, 0.002);
        assert!(removed > 30, "only {removed} keys dropped");
        for f in 0..=600 {
            let time = f as f32 / 600.;
            let d = (t.value(time) - original.value(time)).abs();
            assert!(d < 0.003, "off by {d} at {time}");
        }
    }

    #[test]
    fn a_strided_hull_keeps_the_tips() {
        let o = Object {
            shape: Shape::Bunny,
            ..ball(0.)
        };
        let full = crate::geometry::hull_points(&o, usize::MAX);
        let few = crate::geometry::hull_points(&o, 50);
        for d in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            let far = |p: &[Vec3]| p.iter().map(|p| p.dot(d)).fold(f32::MIN, f32::max);
            assert_eq!(far(&full), far(&few), "extent along {d}");
        }
    }
}
