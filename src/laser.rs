//! Track 17 laser puzzle: a laser beam traced on the CPU through the scene's objects, drawn
//! over the viewport, and a target object to route it to in as few moves as possible.
//!
//! The beam is geometric optics on the objects' triangles: mirrors (metallic, roughness below
//! 0.2) reflect, beam splitters fork it, transmissive objects refract (total internal reflection
//! included), anything else stops it. Hitting the target solves the level. Tracing on the CPU
//! keeps win detection exact and free of GPU readback; see engine-notes "Laser puzzle".
use crate::scene::{Object, Settings};
use glam::Vec3;

/// Segments drawn per beam at most (splitters fork it).
const MAX_SEGMENTS: usize = 64;
/// How far a segment that hits nothing is drawn.
const FAR: f32 = 30.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: Vec3,
    pub b: Vec3,
    /// Share of the laser's power along this segment (splitters halve it).
    pub power: f32,
}

/// One triangle of an object, world space, with its vertex normals.
struct Tri {
    p: [Vec3; 3],
    n: [Vec3; 3],
    object: usize,
}

fn triangles(s: &Settings) -> Vec<Tri> {
    let mut out = vec![];
    for (i, o) in s.objects.iter().enumerate() {
        if !o.visible {
            continue;
        }
        for t in crate::geometry::object_geometry(o, 0).as_chunks::<3>().0 {
            out.push(Tri {
                p: t.map(|v| Vec3::from(v.position)),
                n: t.map(|v| Vec3::from(v.normal)),
                object: i,
            });
        }
    }
    out
}

/// Nearest hit along the ray: distance, object, interpolated normal.
fn hit(tris: &[Tri], o: Vec3, d: Vec3) -> Option<(f32, usize, Vec3)> {
    let mut best: Option<(f32, usize, Vec3)> = None;
    for t in tris {
        let e1 = t.p[1] - t.p[0];
        let e2 = t.p[2] - t.p[0];
        let q = d.cross(e2);
        let det = e1.dot(q);
        if det.abs() < 1e-10 {
            continue;
        }
        let inv = 1. / det;
        let s = o - t.p[0];
        let u = s.dot(q) * inv;
        if !(0. ..=1.).contains(&u) {
            continue;
        }
        let r = s.cross(e1);
        let v = d.dot(r) * inv;
        if v < 0. || u + v > 1. {
            continue;
        }
        let dist = e2.dot(r) * inv;
        if dist > 1e-4 && best.is_none_or(|b| dist < b.0) {
            let n = (t.n[0] * (1. - u - v) + t.n[1] * u + t.n[2] * v).normalize_or_zero();
            let n = if n == Vec3::ZERO {
                e1.cross(e2).normalize()
            } else {
                n
            };
            best = Some((dist, t.object, n));
        }
    }
    best
}

fn mirror(o: &Object) -> bool {
    o.material.metallic > 0.5 && o.material.roughness < 0.2
}

/// Traces the laser: the drawn segments and whether any part of the beam reached the target.
pub fn trace(s: &Settings) -> (Vec<Segment>, bool) {
    let p = &s.puzzle;
    let tris = triangles(s);
    let mut segments = vec![];
    // Every target (the main one and `targets`) the beam must reach.
    let required: Vec<usize> = std::iter::once(p.target)
        .chain(p.targets.iter().copied())
        .filter_map(|t| usize::try_from(t).ok())
        .filter(|&t| t < s.objects.len())
        .collect();
    let mut reached = vec![false; required.len()];
    // (origin, direction, power, inside glass of this ior or 0)
    let mut stack = vec![(
        Vec3::from(p.origin),
        Vec3::from(p.direction).normalize_or(Vec3::X),
        1f32,
    )];
    while let Some((o, d, power)) = stack.pop() {
        if segments.len() >= MAX_SEGMENTS {
            break;
        }
        // A faint branch ends; the others on the stack go on.
        if power < 0.01 {
            continue;
        }
        let Some((t, i, n)) = hit(&tris, o, d) else {
            segments.push(Segment {
                a: o,
                b: o + d * FAR,
                power,
            });
            continue;
        };
        let x = o + d * t;
        segments.push(Segment { a: o, b: x, power });
        if let Some(k) = required.iter().position(|&t| t == i) {
            reached[k] = true;
            continue;
        }
        let m = &s.objects[i].material;
        let front = d.dot(n) < 0.;
        let nf = if front { n } else { -n };
        let reflect = d - 2. * d.dot(nf) * nf;
        let ahead = |dir: Vec3| x + dir * 1e-3;
        if m.splitter {
            if front {
                stack.push((ahead(reflect), reflect, power * m.reflectivity));
                stack.push((ahead(d), d, power * (1. - m.reflectivity)));
            } else {
                stack.push((ahead(d), d, power));
            }
        } else if mirror(&s.objects[i]) {
            stack.push((ahead(reflect), reflect, power));
        } else if m.transmission > 0.5 {
            // Snell at this face; inside → outside when the hit is a back face.
            let eta = if front { 1. / m.ior } else { m.ior };
            let c = -d.dot(nf);
            let k = 1. - eta * eta * (1. - c * c);
            let next = if k < 0. {
                reflect
            } else {
                (eta * d + (eta * c - k.sqrt()) * nf).normalize()
            };
            stack.push((ahead(next), next, power));
        }
    }
    let solved = !required.is_empty() && reached.iter().all(|&r| r);
    (segments, solved)
}

/// Track 17: the beam as thin unlit emissive cylinders (one per segment, at most 20 units
/// long), appended to a frame's copy of the settings so the image shows it.
pub fn beam_objects(s: &Settings, segments: &[Segment]) -> Vec<Object> {
    let mut out = vec![];
    for seg in segments {
        let mut d = seg.b - seg.a;
        let length = d.length().min(20.);
        if length < 1e-3 || out.len() + s.objects.len() >= crate::scene::MAX_OBJECTS {
            continue;
        }
        d /= d.length();
        let (yaw, x, z) = glam::Quat::from_rotation_arc(Vec3::Y, d).to_euler(glam::EulerRot::YXZ);
        let centre = seg.a + d * length * 0.5;
        // Half height 0.35 · size · stretch, radius 0.35 · size · scale (validate's ranges).
        let size = (length * 0.5 / 0.35 / 10.).max(1.);
        let radius = 0.015;
        out.push(Object {
            name: "Laser beam".into(),
            shape: crate::scene::Shape::Cylinder,
            size,
            stretch: (length * 0.5 / (0.35 * size)).clamp(0.05, 10.),
            scale_xz: [(radius / (0.35 * size)).max(0.01); 2],
            position: [centre.x, centre.z],
            elevation: centre.y,
            grounded: false,
            rotation: yaw.to_degrees(),
            tilt: [x.to_degrees(), z.to_degrees()],
            body: crate::scene::Body::None,
            material: crate::scene::Material {
                color: s.puzzle.color,
                emission: 30. * seg.power,
                unlit: true,
                ..Default::default()
            },
            ..Object::default()
        });
    }
    out
}

/// Runtime state of the puzzle (not in `Settings`).
#[derive(Default)]
pub struct Laser {
    /// The settings the beam was traced for (only objects and the puzzle matter).
    key: String,
    pub segments: Vec<Segment>,
    pub solved: bool,
    /// Moves made on this level (finished transforms of any object).
    pub moves: u32,
    /// The level the move count belongs to.
    pub level: String,
    /// The solved toast was shown.
    pub announced: bool,
}

impl Laser {
    /// Retraces when the objects or the puzzle changed. Returns true when the level was just
    /// solved. `busy`: a transform is in progress (its move is not counted yet, and passing
    /// through a solved pose mid-drag is not announced).
    pub fn update(&mut self, s: &Settings, busy: bool) -> bool {
        if !s.puzzle.enabled {
            self.segments.clear();
            self.solved = false;
            self.key.clear();
            return false;
        }
        if self.level != s.scene_name {
            self.level = s.scene_name.clone();
            self.moves = 0;
            self.announced = false;
        }
        let key = serde_json::to_string(&(&s.objects, &s.puzzle)).unwrap_or_default();
        if key != self.key {
            self.key = key;
            (self.segments, self.solved) = trace(s);
        }
        if busy {
            return false;
        }
        let just = self.solved && !self.announced;
        if just {
            self.announced = true;
        }
        if !self.solved {
            self.announced = false;
        }
        just
    }
    pub fn draw(
        &self,
        painter: &egui::Painter,
        ppp: f32,
        view: &crate::gizmo::View,
        color: [f32; 3],
    ) {
        let to_points = |p: glam::Vec2| egui::pos2(p.x / ppp, p.y / ppp);
        let c = color.map(|v| (v.clamp(0., 1.) * 255.) as u8);
        for s in &self.segments {
            if let Some((a, b)) = view.segment(s.a, s.b) {
                let alpha = (80. + 175. * s.power.sqrt()) as u8;
                let stroke = egui::Stroke::new(
                    1. + 2. * s.power.sqrt(),
                    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], alpha),
                );
                painter.line_segment([to_points(a), to_points(b)], stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Material, Shape};

    fn wall(x: f32, z: f32, rotation: f32, material: Material) -> Object {
        Object {
            shape: Shape::Box,
            size: 1.,
            stretch: 2.,
            scale_xz: [0.05, 2.],
            rotation,
            position: [x, z],
            material,
            ..Object::default()
        }
    }

    #[test]
    fn a_mirror_at_45_degrees_routes_the_beam_to_the_target() {
        let silver = Material {
            metallic: 1.,
            roughness: 0.,
            ..Material::default()
        };
        let mut s = Settings {
            cornell_room: false,
            objects: vec![
                wall(1., 0., 45., silver),
                wall(1., 2., 90., Material::default()),
            ],
            ..Settings::default()
        };
        s.puzzle.enabled = true;
        s.puzzle.origin = [-2., -0.5, 0.];
        s.puzzle.direction = [1., 0., 0.];
        s.puzzle.target = 1;
        let (segments, solved) = trace(&s);
        assert_eq!(segments.len(), 2, "{segments:?}");
        assert!(solved);
        // Turned away from the target, the beam misses it.
        s.objects[0].rotation = -45.;
        assert!(!trace(&s).1);
    }

    #[test]
    fn a_splitter_forks_the_beam_and_glass_lets_it_through() {
        let split = Material {
            splitter: true,
            reflectivity: 0.5,
            ..Material::default()
        };
        let glass = Material {
            transmission: 1.,
            ior: 1.5,
            ..Material::default()
        };
        let mut s = Settings {
            cornell_room: false,
            objects: vec![wall(0., 0., 45., split), wall(2., 0., 0., glass)],
            ..Settings::default()
        };
        s.puzzle.enabled = true;
        s.puzzle.origin = [-2., -0.5, 0.];
        s.puzzle.direction = [1., 0., 0.];
        s.puzzle.target = -1;
        let (segments, _) = trace(&s);
        let half = segments
            .iter()
            .filter(|g| (g.power - 0.5).abs() < 1e-4)
            .count();
        // Reflected arm (one segment); transmitted arm: across the splitter's own thickness
        // (its back face passes it on), to the glass, through it, out.
        assert_eq!(half, 5, "{segments:?}");
    }

    /// Each level starts unsolved and its intended solution (within par) solves it.
    #[test]
    fn the_puzzle_levels_have_their_solutions() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/puzzles");
        let level = |n: u32| crate::scenes::read(&dir.join(format!("laser-{n}.json"))).unwrap();
        // (level, [(object, rotation, optional new z)])
        let solutions: [(u32, &[(usize, f32, Option<f32>)]); 3] = [
            (1, &[(0, 45., None)]),
            (2, &[(0, 45., None), (1, 45., None)]),
            (3, &[(0, 45., None), (1, -45., Some(2.))]),
        ];
        for (n, moves) in solutions {
            let mut s = level(n);
            assert!(!trace(&s).1, "level {n} starts solved");
            for &(i, rotation, z) in moves {
                s.objects[i].rotation = rotation;
                if let Some(z) = z {
                    s.objects[i].position[1] = z;
                }
            }
            assert!(trace(&s).1, "level {n}: {:?}", trace(&s).0);
            assert!(moves.len() + usize::from(n == 3) <= s.puzzle.par as usize);
        }
    }
}
