//! Cached primitive meshes, transformed scene geometry, and CPU picking.
use crate::scene::{Object, Settings, Shape, Vertex};
use glam::{EulerRot, Mat4, Vec3};
use std::sync::LazyLock;
type Mesh = Vec<(Vec3, Vec3)>;
static MESHES: LazyLock<Vec<Mesh>> =
    LazyLock::new(|| Shape::ALL.iter().map(|s| make_mesh(*s)).collect());
fn mesh(shape: Shape) -> &'static Mesh {
    &MESHES[shape as usize]
}
fn triangle(out: &mut Mesh, p: [Vec3; 3], n: [Vec3; 3]) {
    let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
    if geometric.length_squared() < 1e-14 {
        return;
    }
    for i in 0..3 {
        out.push((
            p[i],
            if n[i].length_squared() > 1e-12 {
                n[i].normalize()
            } else {
                geometric.normalize()
            },
        ));
    }
}
fn parametric(out: &mut Mesh, nu: u32, nv: u32, f: impl Fn(f32, f32) -> (Vec3, Vec3)) {
    for j in 0..nv {
        for i in 0..nu {
            let (a, na) = f(i as f32 / nu as f32, j as f32 / nv as f32);
            let (b, nb) = f((i + 1) as f32 / nu as f32, j as f32 / nv as f32);
            let (c, nc) = f((i + 1) as f32 / nu as f32, (j + 1) as f32 / nv as f32);
            let (d, nd) = f(i as f32 / nu as f32, (j + 1) as f32 / nv as f32);
            triangle(out, [a, b, c], [na, nb, nc]);
            triangle(out, [a, c, d], [na, nc, nd]);
        }
    }
}
fn make_mesh(shape: Shape) -> Mesh {
    let mut out = vec![];
    let tau = std::f32::consts::TAU;
    match shape {
        Shape::Box => {
            for (n, u, v) in [
                (Vec3::X, Vec3::Z, Vec3::Y),
                (-Vec3::X, -Vec3::Z, Vec3::Y),
                (Vec3::Y, Vec3::X, Vec3::Z),
                (-Vec3::Y, Vec3::X, -Vec3::Z),
                (Vec3::Z, -Vec3::X, Vec3::Y),
                (-Vec3::Z, Vec3::X, Vec3::Y),
            ] {
                triangle(&mut out, [n - u - v, n + u - v, n + u + v], [n; 3]);
                triangle(&mut out, [n - u - v, n + u + v, n - u + v], [n; 3]);
            }
        }
        Shape::Sphere => parametric(&mut out, 64, 32, |u, v| {
            let a = u * tau;
            let b = v * std::f32::consts::PI;
            let n = Vec3::new(b.sin() * a.cos(), b.cos(), b.sin() * a.sin());
            (n, n)
        }),
        Shape::Torus | Shape::Ring => parametric(&mut out, 80, 32, |u, v| {
            let a = u * tau;
            let b = v * tau;
            let n = Vec3::new(b.cos() * a.cos(), b.sin(), b.cos() * a.sin());
            let (major, minor) = if shape == Shape::Ring {
                (0.92, 0.08)
            } else {
                (0.72, 0.28)
            };
            (
                Vec3::new(
                    (major + minor * b.cos()) * a.cos(),
                    minor * b.sin(),
                    (major + minor * b.cos()) * a.sin(),
                ),
                n,
            )
        }),
        Shape::Knot => {
            let center = |t: f32| {
                Vec3::new(
                    (2. + (3. * t).cos()) * (2. * t).cos(),
                    (3. * t).sin(),
                    (2. + (3. * t).cos()) * (2. * t).sin(),
                ) / 3.5
            };
            parametric(&mut out, 192, 20, |u, v| {
                let t = u * tau;
                let c = center(t);
                let tangent = (center(t + 0.001) - center(t - 0.001)).normalize();
                let side = tangent.cross(Vec3::Y).normalize();
                let up = side.cross(tangent).normalize();
                let n = side * (v * tau).cos() + up * (v * tau).sin();
                (c + n * 0.13, n)
            });
        }
        Shape::Cylinder | Shape::Cone => {
            let cone = shape == Shape::Cone;
            parametric(&mut out, 64, 1, |u, v| {
                let a = u * tau;
                let r = if cone { 1. - v } else { 1. };
                let n = Vec3::new(a.cos(), if cone { 0.5 } else { 0. }, a.sin()).normalize();
                (Vec3::new(r * a.cos(), v * 2. - 1., r * a.sin()), n)
            });
            for i in 0..64 {
                let a = i as f32 / 64. * tau;
                let b = (i + 1) as f32 / 64. * tau;
                for y in [-1., 1.] {
                    if cone && y > 0. {
                        continue;
                    }
                    let n = Vec3::Y * y;
                    triangle(
                        &mut out,
                        [
                            Vec3::new(0., y, 0.),
                            Vec3::new(a.cos(), y, a.sin()),
                            Vec3::new(b.cos(), y, b.sin()),
                        ],
                        [n; 3],
                    );
                }
            }
        }
        Shape::Tetrahedron => {
            // A regular tetrahedron standing on a face: base at y = -1, apex at y = 1.
            let r = 2f32.sqrt();
            let base: Vec<Vec3> = (0..3)
                .map(|i| {
                    let a = i as f32 / 3. * tau;
                    Vec3::new(r * a.cos(), -1., r * a.sin())
                })
                .collect();
            let apex = Vec3::new(0., 1., 0.);
            for f in [
                [base[0], base[2], base[1]],
                [base[0], base[1], apex],
                [base[1], base[2], apex],
                [base[2], base[0], apex],
            ] {
                let mut n = (f[1] - f[0]).cross(f[2] - f[0]).normalize();
                if n.dot(f[0] + f[1] + f[2]) < 0. {
                    n = -n;
                }
                triangle(&mut out, f, [n; 3]);
            }
        }
        Shape::Octahedron => {
            for y in [-1., 1.] {
                for i in 0..4 {
                    let a = i as f32 / 4. * tau;
                    let b = (i + 1) as f32 / 4. * tau;
                    let p = [
                        Vec3::Y * y,
                        Vec3::new(a.cos(), 0., a.sin()),
                        Vec3::new(b.cos(), 0., b.sin()),
                    ];
                    let mut n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                    if n.dot(p[0] + p[1] + p[2]) < 0. {
                        n = -n;
                    }
                    triangle(&mut out, p, [n; 3]);
                }
            }
        }
        Shape::Pawn => {
            let profile: [(f32, f32); 13] = [
                (0., -1.),
                (0.72, -1.),
                (0.78, -0.86),
                (0.68, -0.7),
                (0.4, -0.58),
                (0.28, -0.25),
                (0.22, 0.15),
                (0.42, 0.28),
                (0.44, 0.38),
                (0.31, 0.46),
                (0.46, 0.65),
                (0.3, 0.92),
                (0., 1.),
            ];
            parametric(&mut out, 64, 96, |u, v| {
                let x = v * (profile.len() - 1) as f32;
                let i = (x as usize).min(profile.len() - 2);
                let t = (x - i as f32).min(1.);
                let (r0, y0) = profile[i];
                let (r1, y1) = profile[i + 1];
                let r = r0 + (r1 - r0) * t;
                let y = y0 + (y1 - y0) * t;
                let a = u * tau;
                let n = Vec3::new((y1 - y0) * a.cos(), r0 - r1, (y1 - y0) * a.sin()).normalize();
                (Vec3::new(r * a.cos(), y, r * a.sin()), n)
            });
        }
        Shape::Teapot => {
            use crate::teapot_data::{PATCHES, POINTS};
            let basis = |t: f32| {
                let a = 1. - t;
                [a * a * a, 3. * t * a * a, 3. * t * t * a, t * t * t]
            };
            let deriv = |t: f32| {
                [
                    -3. * (1. - t) * (1. - t),
                    3. * (1. - t) * (1. - 3. * t),
                    3. * t * (2. - 3. * t),
                    3. * t * t,
                ]
            };
            for patch in PATCHES.as_chunks::<16>().0 {
                parametric(&mut out, 14, 14, |u, v| {
                    let bu = basis(u);
                    let bv = basis(v);
                    let du = deriv(u);
                    let dv = deriv(v);
                    let (mut p, mut tu, mut tv) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
                    for j in 0..4 {
                        for i in 0..4 {
                            let mut c = Vec3::from(POINTS[patch[j * 4 + i]]);
                            if patch[0] >= 203 && patch[0] < 265 {
                                c.x *= 1.077;
                                c.y *= 1.077;
                            }
                            let c = Vec3::new(c.x, c.z, -c.y);
                            p += c * bu[j] * bv[i];
                            tu += c * du[j] * bv[i];
                            tv += c * bu[j] * dv[i];
                        }
                    }
                    let mut n = tv.cross(tu).normalize_or_zero();
                    if n.length_squared() < 0.5 {
                        n = if p.y > 1.5 { Vec3::Y } else { -Vec3::Y };
                    }
                    ((p - Vec3::new(0.2, 1.575, 0.)) / 3.2, n)
                });
            }
        }
        Shape::Bunny => {
            let mut points = vec![];
            let mut faces = vec![];
            for line in include_str!("../assets/bunny.source.obj").lines() {
                if let Some(rest) = line.strip_prefix("v ") {
                    let p: Vec<f32> = rest
                        .split_whitespace()
                        .take(3)
                        .map(|x| x.parse().expect("Bunny coordinate"))
                        .collect();
                    points.push(Vec3::new(p[0], p[1], p[2]));
                } else if let Some(rest) = line.strip_prefix("f ") {
                    let p: Vec<usize> = rest
                        .split_whitespace()
                        .map(|x| x.split('/').next().unwrap().parse::<usize>().unwrap() - 1)
                        .collect();
                    for i in 1..p.len() - 1 {
                        faces.push([p[0], p[i], p[i + 1]]);
                    }
                }
            }
            let min = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            let center = (min + max) * 0.5;
            let scale = 2. / (max - min).max_element();
            let mut normals = vec![Vec3::ZERO; points.len()];
            for f in &faces {
                let n = (points[f[1]] - points[f[0]]).cross(points[f[2]] - points[f[0]]);
                for i in f {
                    normals[*i] += n;
                }
            }
            for f in faces {
                triangle(
                    &mut out,
                    f.map(|i| (points[i] - center) * scale),
                    f.map(|i| normals[i]),
                );
            }
        }
    }
    out
}
/// The object's pivot (mesh origin in the world, as placed by `object_geometry`) and rotation.
pub fn object_frame(o: &Object) -> (Vec3, glam::Mat3) {
    let scale = Vec3::new(o.scale_xz[0], o.stretch, o.scale_xz[1]) * o.size * 0.35;
    let r = Mat4::from_euler(
        EulerRot::YXZ,
        o.rotation.to_radians(),
        o.tilt[0].to_radians(),
        o.tilt[1].to_radians(),
    );
    let y = if o.grounded {
        let imported = crate::mesh::of(o);
        let source: &Mesh = imported.as_deref().unwrap_or_else(|| mesh(o.kind()));
        let ymin = source
            .iter()
            .map(|(p, _)| r.transform_vector3(*p * scale).y)
            .fold(f32::INFINITY, f32::min);
        -1. - ymin + o.elevation
    } else {
        o.elevation
    };
    (
        Vec3::new(o.position[0], y, o.position[1]),
        glam::Mat3::from_mat4(r),
    )
}
pub fn object_geometry(o: &Object, id: u32) -> Vec<Vertex> {
    if !o.visible {
        return vec![];
    }
    let scale = Vec3::new(o.scale_xz[0], o.stretch, o.scale_xz[1]) * o.size * 0.35;
    let r = Mat4::from_euler(
        EulerRot::YXZ,
        o.rotation.to_radians(),
        o.tilt[0].to_radians(),
        o.tilt[1].to_radians(),
    );
    let imported = crate::mesh::of(o);
    let source: &Mesh = imported.as_deref().unwrap_or_else(|| mesh(o.kind()));
    let ymin = source
        .iter()
        .map(|(p, _)| r.transform_vector3(*p * scale).y)
        .fold(f32::INFINITY, f32::min);
    let center = Vec3::new(
        o.position[0],
        if o.grounded {
            -1. - ymin + o.elevation
        } else {
            o.elevation
        },
        o.position[1],
    );
    source
        .iter()
        .map(|(p, n)| Vertex {
            position: (r.transform_vector3(*p * scale) + center).to_array(),
            normal: r.transform_vector3(*n / scale).normalize().to_array(),
            material: id,
            pad: 0,
        })
        .collect()
}
pub fn geometry(s: &Settings) -> Vec<Vertex> {
    let mut out = vec![];
    let p = Vec3::new;
    let mut quad = |q: [Vec3; 4], n: Vec3, id: u32| {
        for i in [0, 1, 2, 0, 2, 3] {
            out.push(Vertex {
                position: q[i].to_array(),
                normal: n.to_array(),
                material: id,
                pad: 0,
            });
        }
    };
    let e = s.floor_extent;
    quad(
        [p(-e, -1., e), p(e, -1., e), p(e, -1., -e), p(-e, -1., -e)],
        Vec3::Y,
        3,
    );
    if s.cornell_room {
        quad(
            [
                p(-1., 1., -1.),
                p(1., 1., -1.),
                p(1., 1., 1.),
                p(-1., 1., 1.),
            ],
            -Vec3::Y,
            2,
        );
        quad(
            [
                p(-1., -1., -1.),
                p(1., -1., -1.),
                p(1., 1., -1.),
                p(-1., 1., -1.),
            ],
            Vec3::Z,
            2,
        );
        quad(
            [
                p(-1., -1., 1.),
                p(-1., -1., -1.),
                p(-1., 1., -1.),
                p(-1., 1., 1.),
            ],
            Vec3::X,
            0,
        );
        quad(
            [
                p(1., -1., -1.),
                p(1., -1., 1.),
                p(1., 1., 1.),
                p(1., 1., -1.),
            ],
            -Vec3::X,
            1,
        );
    }
    // A light that is off leaves no geometry: its unlit quad would otherwise show up black in
    // reflections and cast a shadow.
    if s.light_on {
        let l = Vec3::from(s.light_position);
        let x = s.light_size * 0.5;
        let z = x * s.light_aspect;
        quad(
            [
                l + p(-x, 0., -z),
                l + p(x, 0., -z),
                l + p(x, 0., z),
                l + p(-x, 0., z),
            ],
            -Vec3::Y,
            4,
        );
    }
    for (i, l) in s.lights.iter().enumerate().filter(|(_, l)| l.enabled) {
        let r = Mat4::from_euler(
            EulerRot::YXZ,
            l.rotation[1].to_radians(),
            l.rotation[0].to_radians(),
            l.rotation[2].to_radians(),
        );
        let center = Vec3::from(l.position);
        let x = l.size[0] * 0.5;
        let z = l.size[1] * 0.5;
        let q = [p(-x, 0., -z), p(x, 0., -z), p(x, 0., z), p(-x, 0., z)]
            .map(|v| r.transform_point3(v) + center);
        quad(
            q,
            r.transform_vector3(-Vec3::Y),
            5 + s.objects.len() as u32 + i as u32,
        );
    }
    // Objects come from a per-slot cache: a gizmo drag changes one object per frame, and
    // rebuilding every mesh (600k vertices in the chess scene) cost about 11 ms.
    let mut cache = OBJECT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.resize_with(s.objects.len(), || None);
    for (i, o) in s.objects.iter().enumerate() {
        let id = 5 + i as u32;
        match &cache[i] {
            Some((cached, cached_id, mesh)) if cached == o && *cached_id == id => {
                out.extend_from_slice(mesh)
            }
            _ => {
                let mesh = object_geometry(o, id);
                out.extend_from_slice(&mesh);
                cache[i] = Some((o.clone(), id, mesh));
            }
        }
    }
    out
}
type CachedMesh = Option<(Object, u32, Vec<Vertex>)>;
static OBJECT_CACHE: std::sync::Mutex<Vec<CachedMesh>> = std::sync::Mutex::new(Vec::new());
impl Settings {
    pub fn frame_objects(&mut self, aspect: f32) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for o in &self.objects {
            for v in object_geometry(o, 0) {
                let p = Vec3::from(v.position);
                min = min.min(p);
                max = max.max(p);
            }
        }
        if !min.is_finite() {
            self.target = [0., 0., 0.];
            self.distance = 5.;
            return;
        }
        let center = (min + max) * 0.5;
        self.target = center.to_array();
        let radius = (max - min).length() * 0.5;
        let t = (self.fov.to_radians() * 0.5).tan() * aspect.min(1.);
        self.distance = (radius / t * 1.25).clamp(1., 200.);
    }
    pub fn focus_object(&mut self, index: usize, aspect: f32) {
        if let Some(o) = self.objects.get(index) {
            let mut tmp = self.clone();
            tmp.objects = vec![o.clone()];
            tmp.frame_objects(aspect);
            self.target = tmp.target;
            self.distance = tmp.distance;
        }
    }
    pub fn pick_object(&self, pixel: [f32; 2], size: [u32; 2]) -> Option<usize> {
        self.pick_object_hit(pixel, size).map(|(i, _)| i)
    }
    /// The nearest object under `pixel` and its distance along the camera ray.
    pub fn pick_object_hit(&self, pixel: [f32; 2], size: [u32; 2]) -> Option<(usize, f32)> {
        let (view, proj, eye) = self.camera(size[0] as f32 / size[1] as f32);
        let ndc = glam::Vec4::new(
            pixel[0] / size[0] as f32 * 2. - 1.,
            1. - pixel[1] / size[1] as f32 * 2.,
            1.,
            1.,
        );
        let p = (proj * view).inverse() * ndc;
        let d = (p.truncate() / p.w - eye).normalize();
        let (mut closest, mut selected) = (f32::INFINITY, None);
        for (i, o) in self.objects.iter().enumerate() {
            let g = object_geometry(o, 0);
            for t in g.as_chunks::<3>().0 {
                let a = Vec3::from(t[0].position);
                let b = Vec3::from(t[1].position);
                let c = Vec3::from(t[2].position);
                let e1 = b - a;
                let e2 = c - a;
                let p = d.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-8 {
                    continue;
                }
                let inv = 1. / det;
                let rel = eye - a;
                let u = rel.dot(p) * inv;
                if !(0. ..=1.).contains(&u) {
                    continue;
                }
                let q = rel.cross(e1);
                let v = d.dot(q) * inv;
                if v < 0. || u + v > 1. {
                    continue;
                }
                let distance = e2.dot(q) * inv;
                if distance > 0.0001 && distance < closest {
                    closest = distance;
                    selected = Some((i, distance));
                }
            }
        }
        selected
    }
}
