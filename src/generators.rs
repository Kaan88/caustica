//! Procedural scene generators (roadmap core track 1): seeded, deterministic families that
//! fit the object limit (4096 instances). A scene file can name a generator and a seed instead of listing
//! objects: `{"format": 1, "title": "...", "generator": {"family": "city", "seed": 7}}`.
use crate::presets::{light, material, object, studio};
use crate::scene::{MAX_OBJECTS, Material, Object, Pattern, Settings, Shape};
use std::f32::consts::{PI, TAU};

/// (id, label, what it makes)
pub const FAMILIES: [(&str, &str, &str); 18] = [
    (
        "still-life",
        "Studio still life",
        "3–12 objects from the shapes and material library, with a key, fill and rim light",
    ),
    (
        "material-grid",
        "Material grid",
        "One shape in a grid that sweeps two material parameters",
    ),
    (
        "city",
        "City blocks",
        "Towers on a street grid, by day or by night",
    ),
    (
        "temple",
        "Temple",
        "A colonnade on steps with an entablature",
    ),
    ("stairs", "Stairs", "Straight, L, U or spiral stairs"),
    (
        "stacking",
        "Stacking",
        "A cairn, a Jenga tower, a house of cards or a sphere pyramid",
    ),
    (
        "menger",
        "Menger sponge",
        "Level 2, built from merged runs of cubes",
    ),
    (
        "sierpinski",
        "Sierpinski tetrahedron",
        "A recursive pyramid of tetrahedra",
    ),
    (
        "sphereflake",
        "Sphereflake",
        "Haines' recursive spheres, level 2",
    ),
    (
        "kaleidoscope",
        "Kaleidoscope",
        "A random motif repeated around a centre",
    ),
    (
        "truchet",
        "Truchet maze",
        "Diagonal tiles in random orientations (10 PRINT)",
    ),
    (
        "mandelbulb",
        "Mandelbulb",
        "The 3D Mandelbrot set (power 5–9) as a mesh, in metal, glass or lacquer",
    ),
    (
        "terrain",
        "Terrain island",
        "A noise heightfield island in sand, grass, rock and snow, with a sea",
    ),
    (
        "deep-menger",
        "Deep Menger sponge",
        "A level 3 or 4 Menger sponge as one mesh",
    ),
    (
        "tree",
        "Trees",
        "A grove of branching trees (an L-system), in summer or autumn",
    ),
    (
        "forest",
        "Forest",
        "Hundreds of trees from a few shared meshes: instancing past the old 256-object cap",
    ),
    (
        "crowd",
        "Crowd",
        "Up to 3000 copies of one shape in a field, each with its own colour",
    ),
    (
        "physics",
        "Physics playground",
        "Dominoes, a brick wall, a block pyramid, a Jenga tower or a ball pit, with the simulation on",
    ),
];

/// xorshift64*: small, fast and reproducible across platforms.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next() * items.len() as f32) as usize).min(items.len() - 1)]
    }
    pub fn chance(&mut self, p: f32) -> bool {
        self.next() < p
    }
}
/// Linear RGB of a hue (0..1) at a saturation and value.
fn hue(h: f32, s: f32, v: f32) -> [f32; 3] {
    let k = |n: f32| {
        let k = (n + h * 6.) % 6.;
        v - v * s * k.min(4. - k).clamp(0., 1.)
    };
    [k(5.), k(3.), k(1.)].map(|c: f32| c.powf(2.2))
}
fn kelvin(k: f32) -> [f32; 3] {
    let mut s = Settings {
        light_temperature: k,
        light_intensity: 1.,
        light_tint: [1.; 3],
        ..Settings::default()
    };
    s.light_on = true;
    let c = s.light_color();
    let m = c[0].max(c[1]).max(c[2]).max(1e-6);
    c.map(|v| (v / m).clamp(0., 1.))
}
/// An object whose mesh centre sits at world height `y`.
fn at(o: Object, y: f32) -> Object {
    Object {
        elevation: y,
        grounded: false,
        ..o
    }
}
fn base(r: &mut Rng) -> Settings {
    let mut s = Settings::default();
    studio(&mut s);
    s.objects.clear();
    s.light_temperature = r.range(3200., 6500.);
    s.lights[0].color = kelvin(r.range(6500., 10000.));
    s.yaw = r.range(-40., 40.);
    s.pitch = r.range(12., 28.);
    s
}
/// Sun and sky for open scenes: a far quad aimed at the origin.
fn sun(s: &mut Settings, r: &mut Rng, warm: bool) {
    s.light_on = false;
    let az = r.range(0., TAU);
    let el = if warm {
        r.range(4., 14.)
    } else {
        r.range(30., 60.)
    }
    .to_radians();
    let d = 40.;
    let p = [
        d * el.cos() * az.cos(),
        d * el.sin() - 1.,
        d * el.cos() * az.sin(),
    ];
    let mut l = light(
        "Sun",
        p,
        [4., 4.],
        kelvin(if warm { 3000. } else { 5800. }),
        if warm { 350. } else { 450. },
    );
    l.rotation = aim(p, [0., -1., 0.]);
    s.lights = vec![l];
    s.sky = if warm { 0.2 } else { 0.55 };
    s.background = if warm {
        [0.18, 0.1, 0.09]
    } else {
        [0.35, 0.5, 0.75]
    };
}
/// Pitch / yaw / roll that turn a downward-facing quad toward `target`.
fn aim(p: [f32; 3], target: [f32; 3]) -> [f32; 3] {
    let n = glam::Vec3::from(target) - glam::Vec3::from(p);
    let n = n.normalize();
    let pitch = (-n.y).clamp(-1., 1.).acos().to_degrees();
    let yaw = if n.y.abs() < 0.9999 {
        (-n.x).atan2(-n.z).to_degrees()
    } else {
        0.
    };
    [pitch, yaw, 0.]
}
fn frame(s: &mut Settings) {
    s.frame_objects(16. / 9.);
    // frame_objects leaves room for any view direction; generated scenes can sit closer.
    s.distance = (s.distance * 0.78).max(1.);
}
fn finish(mut s: Settings, family: &str, seed: u64) -> Settings {
    let label = FAMILIES
        .iter()
        .find(|f| f.0 == family)
        .map_or(family, |f| f.1);
    s.scene_name = format!("{label} · seed {seed}");
    s.objects.truncate(MAX_OBJECTS);
    for o in &mut s.objects {
        fit(o);
    }
    s
}
/// `validate()` bounds stretch (0.05..10) and the X/Z scale (0.01..20) separately from size
/// (0.05..30): move any excess into size, which keeps the world dimensions, and wrap angles.
fn fit(o: &mut Object) {
    let [sx, sz] = o.scale_xz;
    let grow = 1f32.max(o.stretch / 10.).max(sx / 20.).max(sz / 20.);
    let shrink = 1f32.min(o.stretch / 0.05).min(sx / 0.01).min(sz / 0.01);
    let k = grow * shrink;
    o.size *= k;
    o.stretch /= k;
    o.scale_xz = [sx / k, sz / k];
    o.rotation = wrap(o.rotation);
    o.tilt = o.tilt.map(wrap);
}
fn wrap(degrees: f32) -> f32 {
    (degrees + 180.).rem_euclid(360.) - 180.
}

/// Builds a scene of `family` from `seed`. Unknown families give an error.
pub fn generate(family: &str, seed: u64) -> anyhow::Result<Settings> {
    let mut r = Rng::new(
        seed ^ family
            .bytes()
            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)),
    );
    let s = match family {
        "still-life" => still_life(&mut r),
        "material-grid" => material_grid(&mut r),
        "city" => city(&mut r),
        "temple" => temple(&mut r),
        "stairs" => stairs(&mut r),
        "stacking" => stacking(&mut r),
        "menger" => menger(&mut r),
        "sierpinski" => sierpinski(&mut r),
        "sphereflake" => sphereflake(&mut r),
        "kaleidoscope" => kaleidoscope(&mut r),
        "truchet" => truchet(&mut r),
        "mandelbulb" => mandelbulb(&mut r, seed),
        "terrain" => terrain(&mut r, seed),
        "deep-menger" => deep_menger(&mut r, seed),
        "tree" => trees(&mut r, seed),
        "forest" => forest(&mut r, seed),
        "crowd" => crowd(&mut r),
        "physics" => playground(&mut r),
        _ => anyhow::bail!("Unknown generator family {family}"),
    };
    Ok(finish(s, family, seed))
}

fn still_life(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let n = (3. + r.next() * 10.) as usize;
    let shapes = [
        Shape::Sphere,
        Shape::Box,
        Shape::Cylinder,
        Shape::Cone,
        Shape::Torus,
        Shape::Knot,
        Shape::Teapot,
        Shape::Bunny,
        Shape::Pawn,
        Shape::Octahedron,
        Shape::Tetrahedron,
    ];
    let mut placed: Vec<([f32; 2], f32)> = vec![];
    for i in 0..n * 6 {
        if placed.len() >= n {
            break;
        }
        // Rule of thirds: cluster around one of the four third-points of a 4 × 3 area.
        let (tx, tz) = (r.pick(&[-0.67f32, 0.67]), r.pick(&[-0.5f32, 0.5]));
        let p = [tx + r.range(-0.8, 0.8), tz + r.range(-0.6, 0.6)];
        let size = r.range(0.5, 1.5);
        let radius = 0.35 * size;
        if placed.iter().any(|(q, rq)| {
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() < radius + rq + 0.05
        }) {
            continue;
        }
        let shape = r.pick(&shapes);
        let mut o = object(
            format!("{} {}", shape.label(), i + 1),
            shape,
            p[0],
            p[1],
            size,
            r.pick(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
        );
        o.rotation = r.range(-180., 180.);
        if matches!(shape, Shape::Torus) {
            o.tilt = [r.range(0., 90.), 0.];
        }
        if r.chance(0.3) {
            o.material.color = hue(r.next(), r.range(0.4, 0.9), r.range(0.5, 0.9));
        }
        placed.push((p, radius));
        s.objects.push(o);
    }
    // Key (main light), fill and rim with their own temperatures.
    let key = r.range(0., TAU);
    s.light_position = [3. * key.cos(), r.range(2.5, 4.5), 3. * key.sin()];
    s.light_size = r.range(1., 3.);
    s.light_intensity = r.range(10., 18.);
    let fill = key + PI * r.range(0.6, 0.9);
    let mut f = light(
        "Fill",
        [4. * fill.cos(), 1.5, 4. * fill.sin()],
        [2.5, 2.5],
        kelvin(r.range(4000., 9000.)),
        r.range(3., 6.),
    );
    f.rotation = aim(f.position, [0., -0.5, 0.]);
    let mut rim = light(
        "Rim",
        [-2. * key.cos(), 2.5, -2. * key.sin()],
        [1.5, 1.5],
        kelvin(r.range(6000., 11000.)),
        r.range(10., 18.),
    );
    rim.rotation = aim(rim.position, [0., -0.5, 0.]);
    s.lights = vec![f, rim];
    s.base = hue(r.next(), r.range(0., 0.3), r.range(0.3, 0.7));
    frame(&mut s);
    s
}
fn material_grid(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let shape = r.pick(&[
        Shape::Sphere,
        Shape::Box,
        Shape::Torus,
        Shape::Knot,
        Shape::Pawn,
        Shape::Teapot,
    ]);
    let n = r.pick(&[5usize, 6, 7, 8, 10]);
    let color = hue(r.next(), r.range(0.4, 0.9), 0.85);
    let sweep = r.pick(&[
        "roughness × metallic",
        "roughness × glass",
        "IOR × roughness",
        "glow × hue",
    ]);
    let step = 0.95;
    for i in 0..n {
        for j in 0..n {
            let (u, v) = (i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32);
            let mut m = Material {
                color,
                ..Material::default()
            };
            match sweep {
                "roughness × metallic" => (m.roughness, m.metallic) = (u, v),
                "roughness × glass" => {
                    (m.roughness, m.transmission, m.color) = (u, v, [0.9, 0.95, 1.])
                }
                "IOR × roughness" => {
                    (m.ior, m.roughness, m.transmission, m.color) =
                        (1. + 1.4 * u, v * 0.4, 1., [0.92, 0.97, 1.])
                }
                _ => (m.emission, m.color) = (0.5 + 4. * u, hue(v, 0.9, 1.)),
            }
            let mut o = object(
                format!("{sweep} {i},{j}"),
                shape,
                (i as f32 - (n - 1) as f32 / 2.) * step,
                (j as f32 - (n - 1) as f32 / 2.) * step,
                0.9,
                0,
            );
            o.material = m;
            if shape == Shape::Torus {
                o.tilt = [60., 0.];
                o.elevation = 0.1;
            }
            s.objects.push(o);
        }
    }
    s.yaw = r.range(-20., 20.);
    s.pitch = r.range(35., 55.);
    s.light_position = [0., 6., 3.];
    s.light_size = 6.;
    frame(&mut s);
    s
}
/// Smooth 2D value noise for heights.
fn noise(r_seed: f32, x: f32, z: f32) -> f32 {
    let h = |i: i32, j: i32| {
        let v = ((i as f32 * 127.1 + j as f32 * 311.7 + r_seed * 74.7).sin() * 43_758.547).fract();
        v.abs()
    };
    let (i, j) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (sx, sz) = (fx * fx * (3. - 2. * fx), fz * fz * (3. - 2. * fz));
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * sx;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * sx;
    a + (b - a) * sz
}
fn city(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let night = r.chance(0.5);
    let blocks = r.pick(&[4i32, 5, 6]);
    let lots = 2;
    let street = 0.9;
    let lot = 1.0;
    let seed = r.range(0., 100.);
    let span = blocks as f32 * (lots as f32 * lot + street);
    for bx in 0..blocks {
        for bz in 0..blocks {
            for lx in 0..lots {
                for lz in 0..lots {
                    if s.objects.len() >= 240 || r.chance(0.08) {
                        continue;
                    }
                    let x = -span / 2.
                        + bx as f32 * (lots as f32 * lot + street)
                        + (lx as f32 + 0.5) * lot;
                    let z = -span / 2.
                        + bz as f32 * (lots as f32 * lot + street)
                        + (lz as f32 + 0.5) * lot;
                    let centre = 1. - ((x * x + z * z).sqrt() / (span * 0.7)).min(1.);
                    let h = (0.6 + 9. * noise(seed, x * 0.25, z * 0.25).powi(2) * (0.3 + centre))
                        .min(13.);
                    let w = r.range(0.6, 0.9);
                    let lit = night && r.chance(0.55);
                    let window = if r.chance(0.7) {
                        [1., 0.72, 0.42]
                    } else {
                        [0.6, 0.8, 1.]
                    };
                    let m = Material {
                        color: if night {
                            if lit { window } else { [0.06, 0.065, 0.08] }
                        } else {
                            hue(r.range(0.05, 0.15), r.range(0., 0.25), r.range(0.45, 0.8))
                        },
                        roughness: 0.45,
                        reflectivity: 0.15,
                        emission: if lit { r.range(0.05, 0.25) } else { 0. },
                        pattern: Pattern::Stripes,
                        secondary_color: if night {
                            [0.01, 0.011, 0.015]
                        } else {
                            [0.1, 0.12, 0.15]
                        },
                        pattern_scale: r.pick(&[12., 16., 20.]),
                        ..Material::default()
                    };
                    let mut o = object(
                        format!("Tower {}", s.objects.len() + 1),
                        Shape::Box,
                        x,
                        z,
                        1.,
                        0,
                    );
                    o.scale_xz = [w / 0.7, w / 0.7];
                    o.stretch = (h / 0.7).min(10.);
                    o.material = m;
                    s.objects.push(o);
                }
            }
        }
    }
    s.floor_extent = 40.;
    s.floor = Material {
        roughness: 0.4,
        reflectivity: 0.1,
        ..Material::default()
    };
    if night {
        s.light_on = false;
        s.lights.clear();
        s.sky = 0.03;
        s.background = [0.01, 0.015, 0.04];
        s.base = [0.04, 0.04, 0.05];
    } else {
        let warm = r.chance(0.4);
        sun(&mut s, r, warm);
        s.base = [0.25, 0.25, 0.27];
    }
    s.pitch = r.range(18., 35.);
    frame(&mut s);
    s
}
fn temple(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let stone = Material {
        color: hue(
            r.range(0.07, 0.12),
            r.range(0.05, 0.25),
            r.range(0.65, 0.85),
        ),
        roughness: 0.75,
        reflectivity: 0.02,
        ..Material::default()
    };
    let (nx, nz) = (r.pick(&[4usize, 6, 8]), r.pick(&[6usize, 8, 10, 12]));
    let spacing = r.range(0.9, 1.3);
    let (w, d) = ((nx - 1) as f32 * spacing, (nz - 1) as f32 * spacing);
    let height = r.range(2.2, 3.4);
    // Three steps (stylobate), each a box a bit smaller than the one below.
    let mut y = -1.;
    for k in 0..3 {
        let grow = (3 - k) as f32 * 0.35;
        let o = at(
            Object {
                name: format!("Step {}", k + 1),
                shape: Shape::Box,
                size: 1.,
                stretch: 0.2 / 0.7,
                scale_xz: [(w + 1.2 + grow) / 0.7, (d + 1.2 + grow) / 0.7],
                rotation: 0.,
                material: stone.clone(),
                ..Object::default()
            },
            y + 0.1,
        );
        s.objects.push(o);
        y += 0.2;
    }
    let top = y;
    for i in 0..nx {
        for j in 0..nz {
            if i != 0 && i != nx - 1 && j != 0 && j != nz - 1 {
                continue; // a peristyle: columns only around the edge
            }
            let (x, z) = (-w / 2. + i as f32 * spacing, -d / 2. + j as f32 * spacing);
            s.objects.push(at(
                Object {
                    name: "Column".into(),
                    shape: Shape::Cylinder,
                    size: 1.,
                    stretch: height / 0.7,
                    scale_xz: [0.22 / 0.7 * 1.0, 0.22 / 0.7],
                    rotation: 0.,
                    material: stone.clone(),
                    ..Object::default()
                },
                top + height / 2.,
            ));
            let last = s.objects.last_mut().unwrap();
            last.position = [x, z];
            s.objects.push(at(
                Object {
                    name: "Capital".into(),
                    shape: Shape::Box,
                    position: [x, z],
                    size: 1.,
                    stretch: 0.15 / 0.7,
                    scale_xz: [0.5 / 0.7, 0.5 / 0.7],
                    rotation: 0.,
                    material: stone.clone(),
                    ..Object::default()
                },
                top + height + 0.075,
            ));
        }
    }
    let roof = top + height + 0.15;
    s.objects.push(at(
        Object {
            name: "Entablature".into(),
            shape: Shape::Box,
            size: 1.,
            stretch: 0.45 / 0.7,
            scale_xz: [(w + 0.6) / 0.7, (d + 0.6) / 0.7],
            rotation: 0.,
            material: stone.clone(),
            ..Object::default()
        },
        roof + 0.225,
    ));
    // Pediment: a low gabled roof from two tilted slabs.
    for side in [-1., 1.] {
        s.objects.push(at(
            Object {
                name: "Roof".into(),
                shape: Shape::Box,
                position: [side * (w + 0.6) / 4., 0.],
                size: 1.,
                stretch: 0.1 / 0.7,
                scale_xz: [
                    ((w + 0.6) / 2. / 15f32.to_radians().cos()) / 0.7,
                    (d + 0.8) / 0.7,
                ],
                rotation: 0.,
                tilt: [0., -side * 15.],
                material: stone.clone(),
                ..Object::default()
            },
            roof + 0.45 + (w + 0.6) / 4. * 15f32.to_radians().tan(),
        ));
    }
    let warm = r.chance(0.5);
    sun(&mut s, r, warm);
    s.base = [0.32, 0.3, 0.24];
    s.floor_extent = 30.;
    frame(&mut s);
    s
}
fn stairs(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let kind = r.pick(&["straight", "L", "U", "spiral"]);
    let wood = Material {
        color: hue(r.range(0.05, 0.1), r.range(0.4, 0.7), r.range(0.35, 0.6)),
        roughness: 0.35,
        reflectivity: 0.1,
        ..Material::default()
    };
    let (rise, run, width) = (0.17, 0.28, r.range(0.9, 1.4));
    let step = |s: &mut Settings, x: f32, z: f32, y: f32, rot: f32, len: f32| {
        s.objects.push(at(
            Object {
                name: format!("Step {}", s.objects.len() + 1),
                shape: Shape::Box,
                position: [x, z],
                size: 1.,
                stretch: 0.06 / 0.7,
                scale_xz: [len / 0.7, run / 0.7],
                rotation: rot,
                material: wood.clone(),
                ..Object::default()
            },
            y,
        ));
    };
    match kind {
        "spiral" => {
            let n = r.pick(&[36usize, 48, 60]);
            let turn = r.range(1.0, 1.75) * TAU;
            for i in 0..n {
                let a = i as f32 / n as f32 * turn;
                let rr = width * 0.55;
                step(
                    &mut s,
                    rr * a.cos(),
                    rr * a.sin(),
                    -1. + rise * (i + 1) as f32,
                    -a.to_degrees(),
                    width,
                );
            }
            let h = rise * n as f32 + 1.;
            s.objects.push(at(
                Object {
                    name: "Newel".into(),
                    shape: Shape::Cylinder,
                    size: 1.,
                    stretch: h / 0.7,
                    scale_xz: [0.12 / 0.7, 0.12 / 0.7],
                    rotation: 0.,
                    material: material(9),
                    ..Object::default()
                },
                -1. + h / 2.,
            ));
        }
        _ => {
            let flights: &[(f32, f32)] = match kind {
                "straight" => &[(0., 0.)],
                "L" => &[(0., 0.), (90., 0.)],
                _ => &[(0., 0.), (180., 0.)],
            };
            let per = r.pick(&[8usize, 10, 12]);
            let (mut x, mut z, mut y) = (0f32, 0f32, -1f32);
            for (k, (heading, _)) in flights.iter().enumerate() {
                let h = heading.to_radians();
                let (dx, dz) = (-h.sin(), -h.cos());
                for _ in 0..per {
                    y += rise;
                    x += dx * run;
                    z += dz * run;
                    step(&mut s, x, z, y, *heading, width);
                }
                if k + 1 < flights.len() {
                    // Landing.
                    x += dx * (width / 2. + run / 2.);
                    z += dz * (width / 2. + run / 2.);
                    s.objects.push(at(
                        Object {
                            name: "Landing".into(),
                            shape: Shape::Box,
                            position: [x, z],
                            size: 1.,
                            stretch: 0.06 / 0.7,
                            scale_xz: [width / 0.7, width / 0.7],
                            rotation: 0.,
                            material: wood.clone(),
                            ..Object::default()
                        },
                        y,
                    ));
                    if kind == "U" {
                        let side = h + PI / 2.;
                        x += -side.sin() * width;
                        z += -side.cos() * width;
                        x -= dx * (width / 2. + run / 2.);
                        z -= dz * (width / 2. + run / 2.);
                    } else {
                        x += dx * 0.;
                    }
                }
            }
        }
    }
    s.light_position = [r.range(-2., 2.), 6., r.range(-2., 2.)];
    s.light_size = r.range(1.5, 4.);
    s.light_intensity = 14.;
    frame(&mut s);
    s
}
fn stacking(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let kind = r.pick(&["cairn", "jenga", "cards", "pyramid"]);
    match kind {
        "cairn" => {
            let mut y = -1.;
            let n = (5. + r.next() * 8.) as usize;
            for i in 0..n {
                let rad = 0.55 * (1. - i as f32 / (n as f32 + 2.)) * r.range(0.8, 1.1);
                let h = rad * r.range(0.45, 0.7);
                let stone = Material {
                    color: hue(0.08, r.range(0., 0.15), r.range(0.3, 0.6)),
                    roughness: 0.85,
                    ..Material::default()
                };
                let mut o = at(
                    Object {
                        name: format!("Stone {}", i + 1),
                        shape: Shape::Sphere,
                        position: [r.range(-0.04, 0.04), r.range(-0.04, 0.04)],
                        size: rad / 0.35,
                        stretch: h / rad,
                        rotation: r.range(0., 360.),
                        tilt: [r.range(-5., 5.), r.range(-5., 5.)],
                        material: stone,
                        ..Object::default()
                    },
                    y + h * 0.95,
                );
                o.scale_xz = [r.range(0.9, 1.15), r.range(0.9, 1.15)];
                s.objects.push(o);
                y += h * 1.85;
            }
        }
        "jenga" => {
            let layers = (10. + r.next() * 8.) as usize;
            let (lw, lh, ll) = (0.25, 0.15, 0.75);
            let wood = hue(0.09, 0.45, 0.8);
            for k in 0..layers {
                for b in 0..3 {
                    if k > 1 && k + 1 < layers && r.chance(0.18) && b != 1 {
                        continue; // pulled blocks
                    }
                    let off = (b as f32 - 1.) * lw;
                    let (x, z, rot) = if k % 2 == 0 {
                        (off, 0., 0.)
                    } else {
                        (0., off, 90.)
                    };
                    let m = Material {
                        color: [wood[0] * r.range(0.9, 1.1), wood[1], wood[2]].map(|v| v.min(1.)),
                        roughness: 0.6,
                        ..Material::default()
                    };
                    s.objects.push(at(
                        Object {
                            name: format!("Block {}", s.objects.len() + 1),
                            shape: Shape::Box,
                            position: [x, z],
                            size: 1.,
                            stretch: lh / 0.7 * 0.98,
                            scale_xz: [lw / 0.7 * 0.98, ll / 0.7],
                            rotation: rot + r.range(-2., 2.),
                            material: m,
                            ..Object::default()
                        },
                        -1. + lh * (k as f32 + 0.5),
                    ));
                }
            }
        }
        "cards" => {
            let levels = (3. + r.next() * 3.) as usize;
            let (cw, ch, lean) = (0.42, 0.6, 20f32);
            let lean_x = ch * lean.to_radians().sin();
            let level_h = ch * lean.to_radians().cos() + 0.01;
            for l in 0..levels {
                let pairs = levels - l;
                let y0 = -1. + l as f32 * level_h;
                for p in 0..pairs {
                    let cx = (p as f32 - (pairs - 1) as f32 / 2.) * (2. * lean_x + 0.04);
                    for side in [-1., 1.] {
                        s.objects.push(at(
                            Object {
                                name: "Card".into(),
                                shape: Shape::Box,
                                position: [cx + side * lean_x / 2., 0.],
                                size: 1.,
                                stretch: ch / 0.7,
                                scale_xz: [0.005 / 0.7 * 2., cw / 0.7],
                                rotation: 0.,
                                tilt: [0., side * lean],
                                material: Material {
                                    color: [0.95, 0.94, 0.9],
                                    roughness: 0.3,
                                    reflectivity: 0.1,
                                    ..Material::default()
                                },
                                ..Object::default()
                            },
                            y0 + level_h / 2.,
                        ));
                    }
                    if l + 1 < levels && p + 1 < pairs {
                        let fx = cx + lean_x + 0.02;
                        s.objects.push(at(
                            Object {
                                name: "Flat card".into(),
                                shape: Shape::Box,
                                position: [fx, 0.],
                                size: 1.,
                                stretch: 0.01 / 0.7,
                                scale_xz: [ch / 0.7, cw / 0.7],
                                rotation: 0.,
                                material: Material {
                                    color: [0.8, 0.1, 0.1],
                                    roughness: 0.3,
                                    ..Material::default()
                                },
                                ..Object::default()
                            },
                            y0 + level_h + 0.005,
                        ));
                    }
                }
            }
        }
        _ => {
            let n = (4. + r.next() * 3.) as usize;
            let rad = 0.2;
            let m = r.pick(&[1usize, 2, 3, 4, 8]);
            for layer in 0..n {
                let k = n - layer;
                for i in 0..k {
                    for j in 0..(k - i) {
                        if s.objects.len() >= 250 {
                            break;
                        }
                        let x = (i as f32 + j as f32 * 0.5 - (k - 1) as f32 / 2.) * 2. * rad
                            + layer as f32 * rad * 0.5 * 0.;
                        let z = (j as f32 * 3f32.sqrt() / 2. - (k - 1) as f32 * 3f32.sqrt() / 6.)
                            * 2.
                            * rad;
                        let y = -1. + rad + layer as f32 * 2. * rad * (2f32 / 3.).sqrt();
                        let mut o = at(
                            object(
                                format!("Ball {}", s.objects.len() + 1),
                                Shape::Sphere,
                                x,
                                z,
                                rad / 0.35,
                                m,
                            ),
                            y,
                        );
                        o.position[1] += layer as f32 * rad / 3f32.sqrt();
                        s.objects.push(o);
                    }
                }
            }
        }
    }
    s.light_size = r.range(1.5, 3.);
    frame(&mut s);
    s
}
fn menger(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let n = 9; // level 2
    let filled = |i: usize, j: usize, k: usize| {
        let (mut a, mut b, mut c) = (i, j, k);
        for _ in 0..2 {
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
    let cell = r.range(0.28, 0.4);
    let m = Material {
        color: hue(r.next(), r.range(0.3, 0.8), r.range(0.6, 0.9)),
        roughness: r.range(0.1, 0.6),
        metallic: if r.chance(0.4) { 1. } else { 0. },
        ..Material::default()
    };
    // Merge filled cells into runs along x so level 2 fits the object budget.
    for j in 0..n {
        for k in 0..n {
            let mut i = 0;
            while i < n {
                if !filled(i, j, k) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < n && filled(i, j, k) {
                    i += 1;
                }
                let len = (i - start) as f32;
                let x = (start as f32 + len / 2. - n as f32 / 2.) * cell;
                let z = (k as f32 + 0.5 - n as f32 / 2.) * cell;
                s.objects.push(at(
                    Object {
                        name: "Sponge".into(),
                        shape: Shape::Box,
                        position: [x, z],
                        size: 1.,
                        stretch: cell / 0.7,
                        scale_xz: [len * cell / 0.7, cell / 0.7],
                        rotation: 0.,
                        material: m.clone(),
                        ..Object::default()
                    },
                    -1. + (j as f32 + 0.5) * cell,
                ));
            }
        }
    }
    s.light_size = 2.;
    s.light_intensity = 16.;
    frame(&mut s);
    s
}
fn sierpinski(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let level = 3;
    let size = r.range(2.4, 3.2);
    let m = Material {
        color: hue(r.next(), r.range(0.4, 0.9), r.range(0.6, 0.95)),
        roughness: r.range(0.05, 0.5),
        metallic: if r.chance(0.5) { 1. } else { 0. },
        transmission: if r.chance(0.2) { 1. } else { 0. },
        ..Material::default()
    };
    // Corners of the unit tetrahedron used by the Tetrahedron shape (base at -1, apex at +1).
    let rr = 2f32.sqrt();
    let corners: Vec<glam::Vec3> = (0..3)
        .map(|i| {
            let a = i as f32 / 3. * TAU;
            glam::Vec3::new(rr * a.cos(), -1., rr * a.sin())
        })
        .chain([glam::Vec3::new(0., 1., 0.)])
        .collect();
    let mut cells = vec![(glam::Vec3::ZERO, 1f32)];
    for _ in 0..level {
        // A half-size child shares one corner k with its parent: centre c + k * scale / 2.
        cells = cells
            .iter()
            .flat_map(|(c, sc)| corners.iter().map(move |k| (*c + *k * (sc / 2.), sc / 2.)))
            .collect();
    }
    let unit = size / 2.;
    for (c, sc) in cells {
        let world = c * unit;
        s.objects.push(at(
            Object {
                name: "Tetrahedron".into(),
                shape: Shape::Tetrahedron,
                position: [world.x, world.z],
                size: sc * unit / 0.35,
                stretch: 1.,
                rotation: 0.,
                material: m.clone(),
                ..Object::default()
            },
            world.y + unit - 1.,
        ));
    }
    s.light_size = 2.;
    frame(&mut s);
    s
}
fn sphereflake(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let m = Material {
        color: hue(r.next(), r.range(0., 0.4), r.range(0.75, 0.95)),
        roughness: r.range(0.0, 0.15),
        metallic: 1.,
        ..Material::default()
    };
    let radius = 0.7;
    let centre = glam::Vec3::new(0., -1. + radius, 0.);
    // Haines' child directions: six around a tilted ring, three on top.
    let dirs: Vec<glam::Vec3> = (0..6)
        .map(|i| {
            let a = i as f32 / 6. * TAU;
            glam::Vec3::new(a.cos(), -0.15, a.sin()).normalize()
        })
        .chain((0..3).map(|i| {
            let a = (i as f32 + 0.5) / 3. * TAU;
            glam::Vec3::new(a.cos() * 0.6, 1., a.sin() * 0.6).normalize()
        }))
        .collect();
    let mut spheres = vec![(centre, radius, glam::Vec3::Y)];
    let mut frontier = spheres.clone();
    for _ in 0..2 {
        let mut next = vec![];
        for (c, rad, up) in &frontier {
            // Rotate the directions so "up" points away from the parent.
            let q = glam::Quat::from_rotation_arc(glam::Vec3::Y, *up);
            for d in &dirs {
                let d = q * *d;
                let child = rad / 3.;
                next.push((*c + d * (rad + child), child, d));
            }
        }
        spheres.extend(next.iter().copied());
        frontier = next;
    }
    for (c, rad, _) in spheres {
        s.objects.push(at(
            Object {
                name: "Flake".into(),
                shape: Shape::Sphere,
                position: [c.x, c.z],
                size: rad / 0.35,
                stretch: 1.,
                rotation: 0.,
                material: m.clone(),
                ..Object::default()
            },
            c.y,
        ));
    }
    s.light_size = r.range(1., 3.);
    s.floor = Material {
        roughness: 0.2,
        reflectivity: 0.2,
        pattern: Pattern::Checker,
        secondary_color: [0.05; 3],
        pattern_scale: 1.5,
        ..Material::default()
    };
    s.base = [0.8, 0.8, 0.78];
    frame(&mut s);
    s
}
/// A kaleidoscope motif piece: shape, distance, side offset, size, height, tilt, material.
type Part = (Shape, f32, f32, f32, f32, [f32; 2], Material);
fn kaleidoscope(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let fold = r.pick(&[5usize, 6, 7, 8, 9, 10, 12]);
    let parts = (2. + r.next() * 4.) as usize;
    let shapes = [
        Shape::Sphere,
        Shape::Torus,
        Shape::Cone,
        Shape::Box,
        Shape::Octahedron,
        Shape::Tetrahedron,
        Shape::Ring,
        Shape::Knot,
    ];
    let motif: Vec<Part> = (0..parts)
        .map(|_| {
            let mut m = material(r.pick(&[0, 1, 2, 3, 4, 6, 8]));
            if r.chance(0.5) {
                m.color = hue(r.next(), 0.8, 0.9);
            }
            if r.chance(0.15) {
                m.emission = 3.;
            }
            (
                r.pick(&shapes),
                r.range(0.6, 2.6),
                r.range(-0.6, 0.6),
                r.range(0.25, 0.8),
                r.range(-0.6, 1.2),
                [r.range(-60., 60.), r.range(-60., 60.)],
                m,
            )
        })
        .collect();
    for k in 0..fold {
        let a = k as f32 / fold as f32 * TAU;
        for (shape, dist, offset, size, y, tilt, m) in &motif {
            let (x, z) = (
                dist * a.cos() - offset * a.sin(),
                dist * a.sin() + offset * a.cos(),
            );
            s.objects.push(at(
                Object {
                    name: format!("{} {k}", shape.label()),
                    shape: *shape,
                    position: [x, z],
                    size: *size,
                    stretch: 1.,
                    rotation: -a.to_degrees(),
                    tilt: *tilt,
                    material: m.clone(),
                    ..Object::default()
                },
                *y,
            ));
        }
    }
    s.objects.push(at(
        object("Centre", Shape::Sphere, 0., 0., r.range(0.5, 1.2), 10),
        0.,
    ));
    s.pitch = r.range(45., 75.);
    s.base = hue(r.next(), 0.3, 0.2);
    frame(&mut s);
    s
}
fn truchet(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let n = 15;
    let tile = 0.5;
    let (a, b) = (
        hue(r.next(), r.range(0.4, 0.9), 0.85),
        hue(r.next(), r.range(0.2, 0.6), 0.3),
    );
    for i in 0..n {
        for j in 0..n {
            let diagonal = if r.chance(0.5) { 45. } else { -45. };
            let x = (i as f32 - (n - 1) as f32 / 2.) * tile;
            let z = (j as f32 - (n - 1) as f32 / 2.) * tile;
            s.objects.push(Object {
                name: format!("Tile {i},{j}"),
                shape: Shape::Box,
                position: [x, z],
                size: 1.,
                stretch: r.range(0.08, 0.2) / 0.7,
                scale_xz: [tile * 1.414 / 0.7 * 0.95, 0.06 / 0.7],
                rotation: diagonal,
                material: Material {
                    color: if (i + j) % 2 == 0 {
                        a
                    } else {
                        [a[0] * 0.9, a[1] * 0.9, a[2] * 0.9]
                    },
                    roughness: 0.35,
                    reflectivity: 0.2,
                    ..Material::default()
                },
                ..Object::default()
            });
        }
    }
    s.base = b;
    sun(&mut s, r, true);
    s.pitch = r.range(40., 65.);
    frame(&mut s);
    s
}

/// An object showing a procedural mesh (see procedural.rs).
fn procedural(name: &str, spec: String, size: f32, material: Material) -> Object {
    Object {
        name: name.into(),
        shape: Shape::Box,
        mesh: format!("{}{spec}", crate::procedural::PREFIX),
        position: [0., 0.],
        size,
        stretch: 1.,
        rotation: 0.,
        material,
        ..Object::default()
    }
}
fn mandelbulb(r: &mut Rng, seed: u64) -> Settings {
    let mut s = base(r);
    let h = r.next();
    let m = match (r.next() * 3.) as u32 {
        0 => Material {
            color: hue(h, r.range(0.2, 0.6), 0.95),
            metallic: 1.,
            roughness: r.range(0.08, 0.3),
            ..Material::default()
        },
        1 => Material {
            color: hue(h, r.range(0.1, 0.4), 0.95),
            transmission: 1.,
            roughness: 0.,
            ..Material::default()
        },
        _ => Material {
            color: hue(h, r.range(0.5, 0.9), r.range(0.5, 0.85)),
            roughness: r.range(0.1, 0.35),
            reflectivity: 0.5,
            ..Material::default()
        },
    };
    let mut o = procedural("Mandelbulb", format!("mandelbulb/{seed}"), 6., m);
    o.rotation = r.range(-180., 180.);
    s.objects.push(o);
    s.bounces = 16;
    frame(&mut s);
    s
}
fn terrain(r: &mut Rng, seed: u64) -> Settings {
    let mut s = base(r);
    let warm = r.chance(0.35);
    sun(&mut s, r, warm);
    let size = 16.;
    let bands = [
        ([0.62, 0.52, 0.36], 0.9),
        (
            hue(r.range(0.2, 0.32), r.range(0.4, 0.7), r.range(0.3, 0.5)),
            0.85,
        ),
        ([0.28, 0.26, 0.24], 0.8),
        ([0.9, 0.92, 0.95], 0.5),
    ];
    for (i, (color, roughness)) in bands.into_iter().enumerate() {
        let mut o = procedural(
            "Terrain",
            format!("terrain/{seed}#{i}"),
            size,
            Material {
                color,
                roughness,
                ..Material::default()
            },
        );
        o.name = ["Sand", "Grass", "Rock", "Snow"][i].into();
        s.objects.push(o);
    }
    // Sea level (height 0) sits 0.12 mesh units above the clamped sea bed.
    s.objects.push(Object {
        name: "Sea".into(),
        shape: Shape::Box,
        position: [0., 0.],
        size,
        stretch: 0.065,
        scale_xz: [1.2, 1.2],
        rotation: 0.,
        material: Material {
            color: [0.01, 0.05, 0.08],
            roughness: 0.12,
            reflectivity: 0.25,
            ..Material::default()
        },
        ..Object::default()
    });
    s.floor_extent = 30.;
    s.floor.color = [0.05, 0.09, 0.12];
    s.bounces = 8;
    frame(&mut s);
    s.distance *= 0.75;
    s.pitch = r.range(18., 35.);
    s
}
fn deep_menger(r: &mut Rng, seed: u64) -> Settings {
    let mut s = base(r);
    let m = Material {
        color: hue(r.next(), r.range(0.2, 0.7), r.range(0.6, 0.9)),
        roughness: r.range(0.2, 0.6),
        metallic: if r.chance(0.3) { 1. } else { 0. },
        ..Material::default()
    };
    let mut o = procedural("Sponge", format!("menger/{seed}"), 5., m);
    o.rotation = r.range(-45., 45.);
    s.objects.push(o);
    s.light_size = 2.;
    s.light_intensity = 16.;
    s.bounces = 12;
    frame(&mut s);
    s
}
fn trees(r: &mut Rng, seed: u64) -> Settings {
    let mut s = base(r);
    let autumn = r.chance(0.4);
    let count = r.pick(&[1, 3, 5, 7]);
    let warm = autumn || r.chance(0.3);
    sun(&mut s, r, warm);
    s.floor.color = hue(r.range(0.22, 0.3), 0.5, 0.35);
    s.floor.roughness = 0.95;
    for i in 0..count {
        let (angle, radius) = (r.range(0., TAU), if i == 0 { 0. } else { r.range(1.5, 4.) });
        let position = [angle.cos() * radius, angle.sin() * radius];
        let size = r.range(4., 6.5);
        let rotation = r.range(-180., 180.);
        let leaves = if autumn {
            hue(r.range(0.0, 0.12), r.range(0.7, 0.95), r.range(0.6, 0.9))
        } else {
            hue(r.range(0.22, 0.36), r.range(0.5, 0.8), r.range(0.35, 0.6))
        };
        let tint = leaf_tint(r, autumn);
        for (part, name, material) in [
            (
                0,
                "Bark",
                Material {
                    color: [0.16, 0.11, 0.07],
                    roughness: 0.9,
                    ..Material::default()
                },
            ),
            (
                1,
                "Leaves",
                Material {
                    color: leaves,
                    roughness: 0.6,
                    ..Material::default()
                },
            ),
            (
                2,
                "Leaves (tint)",
                Material {
                    color: tint,
                    roughness: 0.6,
                    ..Material::default()
                },
            ),
        ] {
            let mut o = procedural(
                name,
                format!("tree/{}#{part}", seed * 16 + i),
                size,
                material,
            );
            o.position = position;
            o.rotation = rotation;
            s.objects.push(o);
        }
    }
    s.bounces = 8;
    frame(&mut s);
    s
}
/// The second leaf colour of a tree: yellower and lighter in summer, any autumn hue.
fn leaf_tint(r: &mut Rng, autumn: bool) -> [f32; 3] {
    if autumn {
        hue(r.range(0.0, 0.14), r.range(0.6, 0.95), r.range(0.5, 0.9))
    } else {
        hue(r.range(0.16, 0.26), r.range(0.5, 0.8), r.range(0.4, 0.65))
    }
}
/// Many trees from a handful of tree meshes: each mesh is one BLAS, every tree an instance.
fn forest(r: &mut Rng, seed: u64) -> Settings {
    let mut s = base(r);
    let autumn = r.chance(0.35);
    let warm = autumn || r.chance(0.3);
    sun(&mut s, r, warm);
    s.floor.color = hue(r.range(0.2, 0.3), 0.5, 0.3);
    s.floor.roughness = 0.95;
    s.floor_extent = 40.;
    let species = 6;
    // Three objects per tree (bark and two leaf colours) within MAX_OBJECTS.
    let count = r.pick(&[400, 800, 1100, 1350]);
    let radius = (count as f32).sqrt() * 0.55;
    for _ in 0..count {
        let (angle, d) = (r.range(0., TAU), radius * r.next().sqrt());
        let kind = (r.next() * species as f32) as u64 % species;
        let size = r.range(2.5, 5.);
        let rotation = r.range(-180., 180.);
        let leaves = if autumn {
            hue(r.range(0.0, 0.12), r.range(0.7, 0.95), r.range(0.55, 0.9))
        } else {
            hue(r.range(0.2, 0.36), r.range(0.5, 0.8), r.range(0.3, 0.55))
        };
        let tint = leaf_tint(r, autumn);
        for (part, name, color, roughness) in [
            (0, "Bark", [0.16, 0.11, 0.07], 0.9),
            (1, "Leaves", leaves, 0.6),
            (2, "Leaves (tint)", tint, 0.6),
        ] {
            let mut o = procedural(
                name,
                format!("tree/{}#{part}", seed * 16 + kind),
                size,
                Material {
                    color,
                    roughness,
                    ..Material::default()
                },
            );
            o.position = [angle.cos() * d, angle.sin() * d];
            o.rotation = rotation;
            s.objects.push(o);
        }
    }
    s.bounces = 6;
    frame(&mut s);
    s.distance *= 0.8;
    s.pitch = r.range(14., 30.);
    s
}
/// A field of one shape, each copy its own colour and turn.
fn crowd(r: &mut Rng) -> Settings {
    let mut s = base(r);
    let shape = r.pick(&[
        Shape::Bunny,
        Shape::Teapot,
        Shape::Sphere,
        Shape::Torus,
        Shape::Knot,
    ]);
    let side = r.pick(&[20usize, 32, 44, 54]);
    let spacing = r.range(0.55, 0.8);
    let (h0, spread) = (r.next(), r.range(0.1, 0.6));
    let metal = r.chance(0.3);
    for j in 0..side {
        for i in 0..side {
            let jitter = [r.range(-0.15, 0.15), r.range(-0.15, 0.15)];
            s.objects.push(Object {
                name: format!("{shape:?}"),
                shape,
                position: [
                    (i as f32 - side as f32 / 2.) * spacing + jitter[0],
                    (j as f32 - side as f32 / 2.) * spacing + jitter[1],
                ],
                size: spacing * r.range(0.9, 1.3),
                stretch: 1.,
                rotation: r.range(-180., 180.),
                material: Material {
                    color: hue(
                        h0 + spread * r.next(),
                        r.range(0.5, 0.9),
                        r.range(0.55, 0.9),
                    ),
                    roughness: r.range(0.15, 0.5),
                    metallic: if metal { 1. } else { 0. },
                    ..Material::default()
                },
                ..Object::default()
            });
        }
    }
    s.light_size = 6.;
    s.light_position = [-3., 9., 4.];
    s.light_intensity = 9.;
    s.floor_extent = 40.;
    s.bounces = 6;
    frame(&mut s);
    s.distance *= 0.7;
    s.pitch = r.range(20., 40.);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_family_is_valid_and_deterministic() {
        for (family, _, _) in FAMILIES {
            // Thousands of objects each: a few seeds keep the debug test fast.
            let seeds = if ["forest", "crowd"].contains(&family) {
                3
            } else {
                24
            };
            for seed in 0..seeds {
                let mut s = generate(family, seed).unwrap();
                s.validate()
                    .unwrap_or_else(|e| panic!("{family} seed {seed}: {e:#}"));
                assert!(!s.objects.is_empty(), "{family} seed {seed} is empty");
                assert!(s.objects.len() <= MAX_OBJECTS);
            }
            let a = serde_json::to_string(&generate(family, 7).unwrap()).unwrap();
            let b = serde_json::to_string(&generate(family, 7).unwrap()).unwrap();
            assert_eq!(a, b, "{family} is not deterministic");
        }
        assert!(generate("nope", 1).is_err());
    }
    #[test]
    fn menger_level_two_fits_the_object_budget() {
        let s = generate("menger", 1).unwrap();
        assert!(
            s.objects.len() <= MAX_OBJECTS && s.objects.len() > 100,
            "{} runs",
            s.objects.len()
        );
    }
}

/// A box of world size `d` (width, height, length) centred at `c`, yawed by `yaw` degrees.
fn block(name: String, d: [f32; 3], c: [f32; 3], yaw: f32, m: Material) -> Object {
    at(
        Object {
            name,
            shape: Shape::Box,
            position: [c[0], c[2]],
            size: 1.,
            stretch: d[1] / 0.7,
            scale_xz: [d[0] / 0.7, d[2] / 0.7],
            rotation: yaw,
            tilt: [0.; 2],
            material: m,
            ..Object::default()
        },
        c[1],
    )
}
/// Track 17: stable structures to knock down, opened with the physics simulation on (the
/// structures settle in the first second, then sleep). A small gap between blocks keeps the first contacts gentle.
fn playground(r: &mut Rng) -> Settings {
    let mut s = base(r);
    s.physics.enabled = true;
    s.floor_extent = s.floor_extent.max(6.);
    let kind = r.pick(&["dominoes", "wall", "pyramid", "jenga", "balls"]);
    let paint = |r: &mut Rng, h: f32| Material {
        color: hue(
            h + r.range(-0.03, 0.03),
            r.range(0.4, 0.8),
            r.range(0.55, 0.9),
        ),
        roughness: r.range(0.3, 0.7),
        ..Material::default()
    };
    match kind {
        "dominoes" => {
            // An Archimedean spiral of dominoes standing on end.
            let (w, h, t) = (0.1, 0.2, 0.03);
            let h0 = r.next();
            let mut a: f32 = 3.;
            for i in 0..140 {
                let rad = 0.12 * a;
                let (x, z) = (rad * a.cos(), rad * a.sin());
                let yaw = -a.to_degrees();
                let m = paint(r, h0 + i as f32 * 0.007);
                s.objects.push(block(
                    format!("Domino {}", i + 1),
                    [w, h, t],
                    [x, -1. + h / 2., z],
                    yaw,
                    m,
                ));
                a += 0.12 / rad.max(0.15);
                if rad > 1.4 {
                    break;
                }
            }
        }
        "wall" => {
            let (bw, bh, bd) = (0.24, 0.11, 0.12);
            let rows = (6. + r.next() * 6.) as usize;
            let cols = 8;
            let clay = r.range(0.0, 0.06);
            for k in 0..rows {
                let shift = if k % 2 == 1 { bw / 2. } else { 0. };
                for c in 0..cols {
                    let x = (c as f32 - cols as f32 / 2. + 0.5) * bw + shift - bw / 4.;
                    let m = paint(r, clay);
                    s.objects.push(block(
                        format!("Brick {}", s.objects.len() + 1),
                        [bw * 0.98, bh * 0.98, bd],
                        [x, -1. + bh * (k as f32 + 0.5), 0.],
                        0.,
                        m,
                    ));
                }
            }
        }
        "pyramid" => {
            let e = 0.16;
            let n = (5. + r.next() * 3.) as usize;
            let h0 = r.next();
            for k in 0..n {
                let side = n - k;
                for i in 0..side {
                    for j in 0..side {
                        let off = |v: usize| (v as f32 - (side as f32 - 1.) / 2.) * e;
                        let m = paint(r, h0 + k as f32 * 0.08);
                        s.objects.push(block(
                            format!("Block {}", s.objects.len() + 1),
                            [e * 0.98; 3],
                            [off(i), -1. + e * (k as f32 + 0.5), off(j)],
                            0.,
                            m,
                        ));
                    }
                }
            }
        }
        "jenga" => {
            let layers = (12. + r.next() * 8.) as usize;
            let (lw, lh, ll) = (0.25, 0.15, 0.75);
            for k in 0..layers {
                for b in 0..3 {
                    let off = (b as f32 - 1.) * lw;
                    let (x, z, yaw) = if k % 2 == 0 {
                        (off, 0., 0.)
                    } else {
                        (0., off, 90.)
                    };
                    let m = paint(r, 0.09);
                    s.objects.push(block(
                        format!("Block {}", s.objects.len() + 1),
                        [lw * 0.98, lh * 0.995, ll],
                        [x, -1. + lh * (k as f32 + 0.5), z],
                        yaw,
                        m,
                    ));
                }
            }
        }
        _ => {
            // A ball pit: a low box of four walls filled with a loose heap that settles.
            let wall = Material {
                color: [0.9; 3],
                roughness: 0.5,
                ..Material::default()
            };
            for (c, d) in [
                ([0., -0.85, 0.9], [1.9, 0.3, 0.1]),
                ([0., -0.85, -0.9], [1.9, 0.3, 0.1]),
                ([0.9, -0.85, 0.], [0.1, 0.3, 1.7]),
                ([-0.9, -0.85, 0.], [0.1, 0.3, 1.7]),
            ] {
                s.objects.push(block(
                    format!("Wall {}", s.objects.len() + 1),
                    d,
                    c,
                    0.,
                    wall.clone(),
                ));
            }
            let rad = 0.09;
            for i in 0..147 {
                let h = r.pick(&[0., 0.15, 0.33, 0.6, 0.8]);
                let m = paint(r, h);
                s.objects.push(at(
                    Object {
                        name: format!("Ball {}", i + 1),
                        shape: Shape::Sphere,
                        // Layers of a 7×7 grid, jittered but never overlapping.
                        position: [
                            ((i % 7) as f32 - 3.) * 0.2 + r.range(-0.005, 0.005),
                            ((i / 7 % 7) as f32 - 3.) * 0.2 + r.range(-0.005, 0.005),
                        ],
                        size: rad / 0.35,
                        stretch: 1.,
                        rotation: 0.,
                        material: m,
                        ..Object::default()
                    },
                    -0.9 + (i / 49) as f32 * 0.2,
                ));
            }
        }
    }
    s
}
