//! Procedural meshes for the mesh-based generators (core track 1, after mesh import). An object
//! uses one as `mesh: "gen:<kind>/<seed>"` (`#n` picks a part, as for files); `mesh.rs` builds
//! it on first use, normalises it and caches it like a file. Deterministic per seed.
use crate::generators::Rng;
use glam::{Mat3, Vec3};

pub const PREFIX: &str = "gen:";

/// Parts of a procedural mesh: indexed positions (smooth normals) or explicit (position,
/// normal) triangles, with a part name.
pub enum Geometry {
    Indexed(Vec<Vec3>, Vec<u32>),
    Flat(Vec<(Vec3, Vec3)>),
}
pub fn build(spec: &str) -> anyhow::Result<Vec<(String, Geometry)>> {
    let (kind, seed) = spec
        .split_once('/')
        .and_then(|(k, s)| Some((k, s.parse::<u64>().ok()?)))
        .ok_or_else(|| anyhow::anyhow!("Bad procedural mesh {spec:?} (gen:kind/seed)"))?;
    let mut r = Rng::new(seed ^ 0x6d65_7368);
    Ok(match kind {
        "mandelbulb" => vec![("Mandelbulb".into(), mandelbulb(&mut r))],
        "terrain" => terrain(&mut r),
        "menger" => vec![("Sponge".into(), menger(seed))],
        "tree" => tree(&mut r),
        _ => anyhow::bail!("Unknown procedural mesh {kind}"),
    })
}

/// Grid resolution for implicit surfaces: lower in debug builds, which the self-test runs.
fn resolution(release: usize) -> usize {
    if cfg!(debug_assertions) {
        release / 3
    } else {
        release
    }
}

/// Surface nets over `f` (negative inside) on an n³ grid spanning ±`extent`: one vertex per
/// cell the surface crosses (the mean of its edge crossings), one quad per crossed grid edge.
fn surface_nets(n: usize, extent: f32, f: impl Fn(Vec3) -> f32 + Sync) -> Geometry {
    let p = n + 1;
    let at = |i: usize| -extent + 2. * extent * i as f32 / n as f32;
    let point = |i: usize, j: usize, k: usize| Vec3::new(at(i), at(j), at(k));
    // Sample the grid, z slices in parallel.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut values = vec![0f32; p * p * p];
    std::thread::scope(|s| {
        let rows = p.div_ceil(threads);
        for (t, chunk) in values.chunks_mut(rows * p * p).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (idx, v) in chunk.iter_mut().enumerate() {
                    let k = t * rows + idx / (p * p);
                    let (j, i) = ((idx / p) % p, idx % p);
                    *v = f(point(i, j, k));
                }
            });
        }
    });
    let value = |i: usize, j: usize, k: usize| values[(k * p + j) * p + i];
    const NONE: u32 = u32::MAX;
    let mut cell_vertex = vec![NONE; n * n * n];
    let mut positions = vec![];
    const CORNERS: [[usize; 3]; 8] = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    const EDGES: [[usize; 2]; 12] = [
        [0, 1],
        [2, 3],
        [4, 5],
        [6, 7],
        [0, 2],
        [1, 3],
        [4, 6],
        [5, 7],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ];
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                let v = CORNERS.map(|c| value(i + c[0], j + c[1], k + c[2]));
                if v.iter().all(|x| *x < 0.) || v.iter().all(|x| *x >= 0.) {
                    continue;
                }
                let (mut sum, mut count) = (Vec3::ZERO, 0.);
                for [a, b] in EDGES {
                    if (v[a] < 0.) != (v[b] < 0.) {
                        let t = v[a] / (v[a] - v[b]);
                        let ca = Vec3::from(CORNERS[a].map(|c| c as f32));
                        let cb = Vec3::from(CORNERS[b].map(|c| c as f32));
                        sum += ca + (cb - ca) * t;
                        count += 1.;
                    }
                }
                let local = sum / count;
                let base = point(i, j, k);
                cell_vertex[(k * n + j) * n + i] = positions.len() as u32;
                positions.push(base + local * (2. * extent / n as f32));
            }
        }
    }
    let cell = |i: usize, j: usize, k: usize| cell_vertex[(k * n + j) * n + i];
    let mut indices = vec![];
    let mut quad = |q: [u32; 4], flip: bool| {
        if q.contains(&NONE) {
            return;
        }
        let [a, b, c, d] = if flip { [q[0], q[3], q[2], q[1]] } else { q };
        indices.extend([a, b, c, a, c, d]);
    };
    for k in 1..n {
        for j in 1..n {
            for i in 1..n {
                let inside = value(i, j, k) < 0.;
                // Edges from grid point (i, j, k) along +x, +y, +z, each shared by 4 cells.
                if inside != (value(i + 1, j, k) < 0.) {
                    let q = [
                        cell(i, j - 1, k - 1),
                        cell(i, j, k - 1),
                        cell(i, j, k),
                        cell(i, j - 1, k),
                    ];
                    quad(q, !inside);
                }
                if inside != (value(i, j + 1, k) < 0.) {
                    let q = [
                        cell(i - 1, j, k - 1),
                        cell(i - 1, j, k),
                        cell(i, j, k),
                        cell(i, j, k - 1),
                    ];
                    quad(q, !inside);
                }
                if inside != (value(i, j, k + 1) < 0.) {
                    let q = [
                        cell(i - 1, j - 1, k),
                        cell(i, j - 1, k),
                        cell(i, j, k),
                        cell(i - 1, j, k),
                    ];
                    quad(q, !inside);
                }
            }
        }
    }
    Geometry::Indexed(positions, indices)
}

/// The power-n Mandelbulb (White / Nylander), from its distance estimator, z axis up.
fn mandelbulb(r: &mut Rng) -> Geometry {
    let power = r.pick(&[5., 6., 7., 8., 8., 9.]);
    let twist = r.range(0., std::f32::consts::TAU);
    surface_nets(resolution(240), 1.25, move |q| {
        let c = Vec3::new(q.x, q.z, q.y);
        let mut z = c;
        let (mut dr, mut radius) = (1f32, 0f32);
        for _ in 0..10 {
            radius = z.length();
            if radius > 2. {
                break;
            }
            let theta = (z.z / radius.max(1e-9)).clamp(-1., 1.).acos() * power;
            let phi = z.y.atan2(z.x) * power + twist;
            dr = radius.powf(power - 1.) * power * dr + 1.;
            let zr = radius.powf(power);
            z =
                zr * Vec3::new(
                    theta.sin() * phi.cos(),
                    phi.sin() * theta.sin(),
                    theta.cos(),
                ) + c;
        }
        0.5 * radius.max(1e-9).ln() * radius / dr - 0.002
    })
}

/// Value noise in 2D, smooth, from a seeded hash.
fn noise(seed: u32, x: f32, z: f32) -> f32 {
    let h = |i: i32, j: i32| {
        let mut v = (i as u32).wrapping_mul(0x8da6_b343)
            ^ (j as u32).wrapping_mul(0xd816_3841)
            ^ seed.wrapping_mul(0xcb1a_b31f);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2c1b_3c6d);
        v ^= v >> 12;
        (v & 0xffff) as f32 / 65535.
    };
    let (i, j) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (sx, sz) = (fx * fx * (3. - 2. * fx), fz * fz * (3. - 2. * fz));
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * sx;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * sx;
    a + (b - a) * sz
}

/// An island heightfield (fBm, optionally ridged), split by height and slope into sand,
/// grass, rock and snow parts.
fn terrain(r: &mut Rng) -> Vec<(String, Geometry)> {
    let n = resolution(384);
    let seed = (r.next() * 1e6) as u32;
    let ridged = r.chance(0.5);
    let (octaves, frequency) = (6, r.range(1.5, 3.));
    let height = |x: f32, z: f32| {
        let (mut sum, mut amp, mut f) = (0., 0.5, frequency);
        for o in 0..octaves {
            let v = noise(seed + o, x * f, z * f);
            sum += amp * if ridged { 1. - (2. * v - 1.).abs() } else { v };
            amp *= 0.5;
            f *= 2.03;
        }
        let falloff = (1. - (x * x + z * z).sqrt()).clamp(0., 1.);
        (sum * falloff.powf(0.7) * 1.6 - 0.25).max(-0.2)
    };
    let at = |i: usize| -1. + 2. * i as f32 / n as f32;
    let mut grid = vec![Vec3::ZERO; (n + 1) * (n + 1)];
    for j in 0..=n {
        for i in 0..=n {
            grid[j * (n + 1) + i] = Vec3::new(at(i), height(at(i), at(j)) * 0.6, at(j));
        }
    }
    // Every part keeps the whole grid; its indices are the triangles in its band.
    let names = ["Sand", "Grass", "Rock", "Snow"];
    let mut bands: [Vec<u32>; 4] = Default::default();
    let id = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    for j in 0..n {
        for i in 0..n {
            for t in [
                [id(i, j), id(i, j + 1), id(i + 1, j + 1)],
                [id(i, j), id(i + 1, j + 1), id(i + 1, j)],
            ] {
                let [a, b, c] = t.map(|v| grid[v as usize]);
                let normal = (b - a).cross(c - a).normalize_or(Vec3::Y);
                let y = (a.y + b.y + c.y) / 3.;
                let band = if y > 0.42 {
                    3
                } else if normal.y < 0.62 {
                    2
                } else if y > 0.0 {
                    1
                } else {
                    0
                };
                bands[band].extend(t);
            }
        }
    }
    names
        .iter()
        .zip(bands)
        .map(|(name, idx)| (name.to_string(), Geometry::Indexed(grid.clone(), idx)))
        .collect()
}

/// A Menger sponge of level 3 or 4 as voxels; only faces between solid and empty cells.
fn menger(seed: u64) -> Geometry {
    let level = if seed.is_multiple_of(3) && !cfg!(debug_assertions) {
        4
    } else {
        3
    };
    let n = 3usize.pow(level);
    let solid = |i: i64, j: i64, k: i64| {
        if [i, j, k].iter().any(|v| *v < 0 || *v >= n as i64) {
            return false;
        }
        let (mut a, mut b, mut c) = (i, j, k);
        for _ in 0..level {
            if [a % 3 == 1, b % 3 == 1, c % 3 == 1]
                .iter()
                .filter(|v| **v)
                .count()
                >= 2
            {
                return false;
            }
            (a, b, c) = (a / 3, b / 3, c / 3);
        }
        true
    };
    let mut out = vec![];
    let cell = 2. / n as f32;
    for k in 0..n as i64 {
        for j in 0..n as i64 {
            for i in 0..n as i64 {
                if !solid(i, j, k) {
                    continue;
                }
                let lo = Vec3::new(i as f32, j as f32, k as f32) * cell - 1.;
                for axis in 0..3 {
                    for dir in [-1i64, 1] {
                        let mut nb = [i, j, k];
                        nb[axis] += dir;
                        if solid(nb[0], nb[1], nb[2]) {
                            continue;
                        }
                        let normal = Vec3::from_array(std::array::from_fn(|a| {
                            if a == axis { dir as f32 } else { 0. }
                        }));
                        let (u, v) = (
                            Vec3::from_array(std::array::from_fn(|a| {
                                if a == (axis + 1) % 3 { cell } else { 0. }
                            })),
                            Vec3::from_array(std::array::from_fn(|a| {
                                if a == (axis + 2) % 3 { cell } else { 0. }
                            })),
                        );
                        let o = lo + if dir > 0 { normal * cell } else { Vec3::ZERO };
                        let quad = [o, o + u, o + u + v, o + v];
                        let tris = if dir > 0 {
                            [0, 1, 2, 0, 2, 3]
                        } else {
                            [0, 2, 1, 0, 3, 2]
                        };
                        out.extend(tris.map(|t| (quad[t], normal)));
                    }
                }
            }
        }
    }
    Geometry::Flat(out)
}

/// A branching tree (a stochastic bracketed L-system, interpreted directly): tapered
/// six-sided branches, and leaf cards at the twigs.
fn tree(r: &mut Rng) -> Vec<(String, Geometry)> {
    let mut bark = vec![];
    let mut leaves = vec![];
    let mut tinted = vec![];
    let depth = r.pick(&[6, 7, 7, 8]);
    let spread = r.range(22., 42.).to_radians();
    let children = r.pick(&[2, 2, 3]);
    let leaf_size = r.range(0.05, 0.09);
    struct Branch {
        base: Vec3,
        dir: Vec3,
        length: f32,
        radius: f32,
        depth: u32,
    }
    let mut stack = vec![Branch {
        base: Vec3::ZERO,
        dir: Vec3::Y,
        length: 1.,
        radius: 0.07,
        depth: 0,
    }];
    while let Some(b) = stack.pop() {
        let tip = b.base + b.dir * b.length;
        let tip_radius = b.radius * 0.72;
        frustum(&mut bark, b.base, tip, b.radius, tip_radius);
        if b.depth + 1 >= depth {
            // Leaves around the twig tip, about a third of them in the second leaf part
            // (its own colour).
            for _ in 0..6 {
                let centre =
                    tip + Vec3::new(r.range(-1., 1.), r.range(-0.5, 1.), r.range(-1., 1.)) * 0.12;
                let n = Vec3::new(r.range(-1., 1.), r.range(0.2, 1.), r.range(-1., 1.)).normalize();
                let out = if r.chance(0.35) {
                    &mut tinted
                } else {
                    &mut leaves
                };
                leaf(
                    out,
                    centre,
                    n,
                    n.any_orthonormal_vector(),
                    leaf_size * r.range(1.1, 1.5),
                    r.range(0.1, 0.35),
                );
            }
            continue;
        }
        let side = b.dir.any_orthonormal_vector();
        let roll = r.range(0., std::f32::consts::TAU);
        for c in 0..children {
            let around = roll + c as f32 * std::f32::consts::TAU / children as f32;
            let axis = Mat3::from_axis_angle(b.dir, around) * side;
            let bend = spread * r.range(0.7, 1.3);
            // Branches lean back toward up a little (phototropism).
            let dir = (Mat3::from_axis_angle(axis, bend) * b.dir + Vec3::Y * 0.15).normalize();
            stack.push(Branch {
                base: tip,
                dir,
                length: b.length * r.range(0.68, 0.82),
                radius: tip_radius,
                depth: b.depth + 1,
            });
        }
    }
    vec![
        ("Bark".into(), Geometry::Flat(bark)),
        ("Leaves".into(), Geometry::Flat(leaves)),
        ("Leaves (tint)".into(), Geometry::Flat(tinted)),
    ]
}
/// One leaf: a pointed oval along `along` (petiole at `centre − along·size`, tip at
/// `centre + along·size`) in the plane facing `n`, folded along its midrib by `fold` (the
/// edges drop by fold × half width). Single-sided: the shader turns normals toward the ray.
fn leaf(out: &mut Vec<(Vec3, Vec3)>, centre: Vec3, n: Vec3, along: Vec3, size: f32, fold: f32) {
    let side = n.cross(along);
    let mid = |t: f32| centre + along * size * (2. * t - 1.);
    // Midrib at t = 0, ⅓, ⅔, 1; edges at ⅓ and ⅔, wider toward the base.
    let m = [0., 1. / 3., 2. / 3., 1.].map(mid);
    let edge = |t: f32, s: f32| {
        let w = size * if t < 0.5 { 0.55 } else { 0.4 };
        mid(t) + side * (s * w) - n * (fold * w)
    };
    for s in [-1., 1.] {
        let (e1, e2) = (edge(1. / 3., s), edge(2. / 3., s));
        for tri in [
            [m[0], m[1], e1],
            [m[1], m[2], e2],
            [m[1], e2, e1],
            [m[2], m[3], e2],
        ] {
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or(n);
            out.extend(tri.map(|p| (p, normal)));
        }
    }
}
/// A six-sided tapered cylinder from `a` to `b` with smooth side normals (open ends: each
/// rests inside its parent).
fn frustum(out: &mut Vec<(Vec3, Vec3)>, a: Vec3, b: Vec3, ra: f32, rb: f32) {
    let axis = (b - a).normalize_or(Vec3::Y);
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let ring = |k: usize| {
        let t = k as f32 * std::f32::consts::TAU / 6.;
        u * t.cos() + v * t.sin()
    };
    for k in 0..6 {
        let (d0, d1) = (ring(k), ring(k + 1));
        let quad = [
            (a + d0 * ra, d0),
            (a + d1 * ra, d1),
            (b + d1 * rb, d1),
            (b + d0 * rb, d0),
        ];
        out.extend([0, 1, 2, 0, 2, 3].map(|i| quad[i]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_nets_make_a_closed_outward_sphere() {
        let Geometry::Indexed(p, idx) = surface_nets(24, 1.2, |q| q.length() - 1.) else {
            unreachable!()
        };
        assert!(idx.len() > 300 && idx.len() % 3 == 0);
        assert!(
            p.iter().all(|v| (v.length() - 1.).abs() < 0.05),
            "vertices on the sphere"
        );
        // Every face normal points away from the centre.
        for t in idx.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| p[i as usize]);
            assert!((b - a).cross(c - a).dot(a + b + c) > 0.);
        }
    }
    #[test]
    fn every_kind_builds_deterministically() {
        for kind in ["terrain", "menger", "tree"] {
            let count = |s: &str| {
                build(s)
                    .unwrap()
                    .iter()
                    .map(|(_, g)| match g {
                        Geometry::Indexed(_, i) => i.len(),
                        Geometry::Flat(v) => v.len(),
                    })
                    .sum::<usize>()
            };
            let spec = format!("{kind}/7");
            assert!(count(&spec) > 0);
            assert_eq!(count(&spec), count(&spec));
        }
        assert!(build("nothing/1").is_err() && build("tree/x").is_err());
    }
}
