//! Snapping a move to other geometry (roadmap 3b): the surface, the nearest vertex or the
//! nearest edge point under the pointer, found by a CPU ray cast over every visible object
//! except the ones being moved, plus the room (floor and Cornell walls).
use crate::geometry::{object_geometry, static_geometry};
use crate::scene::Settings;
use glam::Vec3;

/// What Ctrl snaps to during a transform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Steps of 0.1 units, 15° and 0.1× (the original behaviour).
    Increment,
    /// The point under the pointer on another surface.
    Surface,
    /// The nearest corner of the triangle under the pointer.
    Vertex,
    /// The nearest point on an edge of the triangle under the pointer.
    Edge,
}
impl Target {
    pub const ALL: [Target; 4] = [
        Target::Increment,
        Target::Surface,
        Target::Vertex,
        Target::Edge,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Target::Increment => "Increment",
            Target::Surface => "Surface",
            Target::Vertex => "Vertex",
            Target::Edge => "Edge",
        }
    }
}

/// A ray hit on a triangle.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub triangle: [Vec3; 3],
}

/// Möller–Trumbore; the distance along `d` (unit) when the ray meets the triangle.
fn intersect(o: Vec3, d: Vec3, t: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1. / det;
    let rel = o - t[0];
    let u = rel.dot(p) * inv;
    if !(0. ..=1.).contains(&u) {
        return None;
    }
    let q = rel.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0. || u + v > 1. {
        return None;
    }
    let distance = e2.dot(q) * inv;
    (distance > 1e-4).then_some(distance)
}

/// The nearest triangle along the ray (`d` unit) among the visible objects not in `skip` and
/// the room's floor and walls (lights are not snap targets).
pub fn cast(s: &Settings, o: Vec3, d: Vec3, skip: &[usize]) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    let mut test = |t: [Vec3; 3]| {
        if let Some(distance) = intersect(o, d, t)
            && best.is_none_or(|b| distance < b.distance)
        {
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            best = Some(Hit {
                distance,
                point: o + d * distance,
                // Facing the ray, so "on top of" means the side the pointer sees.
                normal: if n.dot(d) > 0. { -n } else { n },
                triangle: t,
            });
        }
    };
    for (i, object) in s.objects.iter().enumerate() {
        if skip.contains(&i) {
            continue;
        }
        for t in object_geometry(object, 0).as_chunks::<3>().0 {
            test(t.map(|v| Vec3::from(v.position)));
        }
    }
    // Room triangles: walls 0–2 and floor 3; the main light (4) and area lights are not.
    for t in static_geometry(s).as_chunks::<3>().0 {
        if t[0].material <= 3 {
            test(t.map(|v| Vec3::from(v.position)));
        }
    }
    best
}

/// The point `target` picks on `hit`: the hit point, its triangle's nearest corner, or the
/// nearest point on one of its edges.
pub fn point(target: Target, hit: &Hit) -> Vec3 {
    let [a, b, c] = hit.triangle;
    match target {
        Target::Increment | Target::Surface => hit.point,
        Target::Vertex => [a, b, c]
            .into_iter()
            .min_by(|x, y| x.distance(hit.point).total_cmp(&y.distance(hit.point)))
            .unwrap(),
        Target::Edge => [(a, b), (b, c), (c, a)]
            .into_iter()
            .map(|(p, q)| {
                let t = ((hit.point - p).dot(q - p) / (q - p).length_squared().max(1e-12))
                    .clamp(0., 1.);
                p + (q - p) * t
            })
            .min_by(|x, y| x.distance(hit.point).total_cmp(&y.distance(hit.point)))
            .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapping_finds_the_floor_corners_and_edges() {
        let s = Settings {
            objects: vec![],
            ..Settings::default()
        };
        // Straight down onto the floor (y = -1) near a corner of its triangles.
        let hit = cast(&s, Vec3::new(0.3, 0.5, 0.2), Vec3::NEG_Y, &[]).expect("floor");
        assert!((hit.point.y + 1.).abs() < 1e-5, "{hit:?}");
        assert!(hit.normal.y > 0.99);
        assert_eq!(point(Target::Surface, &hit), hit.point);
        let v = point(Target::Vertex, &hit);
        assert!(hit.triangle.contains(&v));
        let e = point(Target::Edge, &hit);
        assert!((e.y + 1.).abs() < 1e-5);
        assert!(e.distance(hit.point) <= v.distance(hit.point) + 1e-6);
    }
    #[test]
    fn the_moved_object_is_skipped() {
        let s = Settings::default();
        assert!(!s.objects.is_empty());
        let (pivot, _) = crate::geometry::object_frame(&s.objects[0]);
        // From just under the Cornell box's ceiling (y = 1), straight down through the object.
        let from = Vec3::new(pivot.x, 0.95, pivot.z);
        let hit = cast(&s, from, Vec3::NEG_Y, &[]).expect("object");
        let skipped = cast(&s, from, Vec3::NEG_Y, &[0]).expect("floor");
        assert!(skipped.distance > hit.distance);
    }
}
