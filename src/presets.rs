//! Built-in rendering studies. Every object and light remains editable.
use crate::scene::{AreaLight, Material, Object, Pattern, Settings, Shape};
use std::f32::consts::TAU;
pub const MATERIAL_NAMES: [&str; 17] = [
    "Porcelain",
    "Chrome",
    "Gold",
    "Copper",
    "Clear glass",
    "Frosted glass",
    "Jade",
    "Red rubber",
    "Blue lacquer",
    "Marble",
    "Warm emitter",
    "Cool emitter",
    "Car paint",
    "Brushed steel",
    "Velvet",
    "Oil slick",
    "CD grooves",
];
pub fn material(id: usize) -> Material {
    let mut m = Material::default();
    match id {
        0 => {
            m.color = [0.88, 0.87, 0.83];
            m.roughness = 0.2;
        }
        1 => {
            m.color = [0.92, 0.94, 0.97];
            m.metallic = 1.;
            m.roughness = 0.035;
        }
        2 => {
            m.color = [1., 0.71, 0.29];
            m.metallic = 1.;
            m.roughness = 0.18;
        }
        3 => {
            m.color = [0.95, 0.47, 0.26];
            m.metallic = 1.;
            m.roughness = 0.25;
        }
        4 => {
            m.color = [0.96, 0.99, 1.];
            m.transmission = 1.;
            m.roughness = 0.;
            m.ior = 1.52;
        }
        5 => {
            m.color = [0.78, 0.92, 0.89];
            m.transmission = 1.;
            m.roughness = 0.19;
        }
        6 => {
            m.color = [0.09, 0.46, 0.25];
            m.roughness = 0.15;
            m.reflectivity = 0.15;
        }
        7 => {
            m.color = [0.68, 0.025, 0.018];
            m.roughness = 0.9;
        }
        8 => {
            m.color = [0.025, 0.17, 0.65];
            m.roughness = 0.12;
            m.reflectivity = 0.2;
        }
        9 => {
            m.color = [0.85, 0.84, 0.79];
            m.roughness = 0.25;
            m.pattern = Pattern::Marble;
            m.secondary_color = [0.05, 0.07, 0.1];
            m.pattern_scale = 5.;
        }
        10 => {
            m.color = [1., 0.32, 0.08];
            m.emission = 8.;
            m.roughness = 0.4;
        }
        11 => {
            m.color = [0.08, 0.5, 1.];
            m.emission = 8.;
            m.roughness = 0.4;
        }
        // Track 11's layered lobes, so the inspector combo can show each one off.
        12 => {
            m.color = [0.02, 0.12, 0.4];
            m.roughness = 0.4;
            m.clearcoat = 1.;
            m.clearcoat_roughness = 0.06;
        }
        13 => {
            m.color = [0.55, 0.56, 0.58];
            m.metallic = 1.;
            m.roughness = 0.3;
            m.anisotropy = 0.85;
            m.anisotropy_rotation = 90.;
        }
        14 => {
            m.color = [0.35, 0.05, 0.12];
            m.roughness = 0.6;
            m.sheen = 0.7;
            m.sheen_roughness = 0.35;
        }
        15 => {
            m.color = [0.04, 0.04, 0.05];
            m.roughness = 0.25;
            m.film_thickness = 420.;
        }
        16 => {
            m.color = [0.7, 0.71, 0.73];
            m.metallic = 1.;
            m.roughness = 0.06;
            m.grating = 1600.;
        }
        _ => {}
    }
    m
}
pub(crate) fn object(
    name: impl Into<String>,
    shape: Shape,
    x: f32,
    z: f32,
    size: f32,
    mat: usize,
) -> Object {
    Object {
        name: name.into(),
        shape,
        position: [x, z],
        size,
        stretch: 1.,
        rotation: 0.,
        material: material(mat),
        ..Object::default()
    }
}
pub(crate) fn light(
    name: &str,
    p: [f32; 3],
    size: [f32; 2],
    color: [f32; 3],
    intensity: f32,
) -> AreaLight {
    AreaLight {
        name: name.into(),
        position: p,
        size,
        color,
        intensity,
        visible: false,
        ..AreaLight::default()
    }
}
pub(crate) fn studio(s: &mut Settings) {
    s.objects.clear();
    s.cornell_room = false;
    s.floor_extent = 15.;
    s.sky = 0.08;
    s.base = [0.22, 0.25, 0.3];
    s.floor = Material {
        roughness: 0.3,
        reflectivity: 0.08,
        ..Material::default()
    };
    s.light_position = [-1., 4., 2.];
    s.light_size = 3.;
    s.light_intensity = 12.;
    s.light_visible = false;
    s.light_temperature = 5600.;
    s.lights = vec![light(
        "Cool rim",
        [2.5, 3., -2.5],
        [2., 2.],
        [0.42, 0.65, 1.],
        10.,
    )];
    s.pitch = 14.;
    s.yaw = 15.;
    s.distance = 8.;
    s.target = [0., -0.1, 0.];
    s.bounces = 10;
}
pub fn description(id: usize) -> &'static str {
    match id {
        9 => {
            "25 spheres: roughness across columns, metallic weight down rows. A quick BRDF sanity check."
        }
        10 => {
            "A seeded field of small spheres with glass, chrome and diffuse heroes on a checker ground."
        }
        11 => "The classic mirror-and-glass sphere study on a reflective checkerboard.",
        12 => {
            "Martin Newell's Utah teapot, tessellated from its original Bézier patches. Porcelain, copper and glass."
        }
        13 => {
            "The real Stanford bunny scan in jade, gold and porcelain. Stanford research-use terms apply."
        }
        14 => {
            "Refraction, absorption and rough transmission across boxes, spheres, cones and a torus."
        }
        15 => {
            "Three colored area lights produce overlapping RGB shadows around matte white geometry."
        }
        16 => "A close-packed pyramid of gold, copper, chrome and lacquer spheres.",
        17 => {
            "Parallel mirrors, colored emitters and repeating geometry. Raise the bounce limit for deeper reflections."
        }
        18 => {
            "Trefoil knots and torus sculptures on dark plinths. Useful for glossy reflection and silhouette tests."
        }
        19 => "A sunlit courtyard of columns, lintels, plinths and a central sculpture.",
        20 => "Two ranks of turned chess pawns over a polished checkerboard.",
        21 => {
            "A chrome knot lit by emissive rings and colored softboxes. Emissive meshes participate in direct-light sampling."
        }
        22 => {
            "Floating rings of colored spheres around a glass core. All three-dimensional transforms remain editable."
        }
        23 => "An empty, softly lit stage. Add objects and lights to build a scene from scratch.",
        _ => {
            "The Cornell-box rendering studies, with editable walls, materials, geometry and lighting."
        }
    }
}
pub fn apply(s: &mut Settings, id: usize) {
    studio(s);
    match id {
        9 => {
            s.distance = 10.5;
            s.pitch = 32.;
            s.yaw = 0.;
            s.target = [0., -0.5, 0.];
            for row in 0..5 {
                for col in 0..5 {
                    let mut o = object(
                        format!("Roughness {} / metal {}", col, row),
                        Shape::Sphere,
                        (col as f32 - 2.) * 1.05,
                        (row as f32 - 2.) * 1.05,
                        1.15,
                        2,
                    );
                    o.material.roughness = 0.025 + col as f32 * 0.23;
                    o.material.metallic = row as f32 * 0.25;
                    s.objects.push(o);
                }
            }
            s.light_size = 5.;
            s.light_position = [-2., 5., 1.];
        }
        10 => {
            s.floor_extent = 25.;
            s.sky = 0.55;
            s.background = [0.; 3];
            s.floor.pattern = Pattern::Checker;
            s.floor.secondary_color = [0.12, 0.19, 0.08];
            s.floor.pattern_scale = 0.7;
            s.base = [0.65, 0.68, 0.45];
            s.floor.roughness = 0.85;
            s.light_position = [-4., 8., 2.];
            s.light_size = 6.;
            s.light_intensity = 8.;
            s.lights.clear();
            s.distance = 13.;
            s.pitch = 10.;
            s.yaw = 20.;
            s.target = [0., 0., 0.];
            let mut seed = 1234567u32;
            let mut rand = || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed >> 8) as f32 / 16777216.
            };
            for a in -5..=5 {
                for b in -4..=4 {
                    let x = a as f32 * 0.9 + (rand() - 0.5) * 0.25;
                    let z = b as f32 * 0.9 + (rand() - 0.5) * 0.25;
                    if x.abs() < 2.8 && z.abs() < 1.15 {
                        continue;
                    }
                    let mat = if rand() < 0.2 { 1 } else { 7 };
                    let mut o = object(
                        format!("Field sphere {a},{b}"),
                        Shape::Sphere,
                        x,
                        z,
                        0.35 + rand() * 0.4,
                        mat,
                    );
                    if mat != 1 {
                        o.material.color = [rand() * rand(), rand() * rand(), rand() * rand()];
                        o.material.roughness = 0.5 + rand() * 0.5;
                    }
                    s.objects.push(o);
                }
            }
            s.objects.extend([
                object("Glass hero", Shape::Sphere, 0., 0., 2.8, 4),
                object("Chrome hero", Shape::Sphere, 2.3, 0.15, 2.8, 1),
                object("Matte hero", Shape::Sphere, -2.3, 0., 2.8, 7),
            ]);
        }
        11 => {
            s.floor_extent = 18.;
            s.base = [0.86; 3];
            s.floor.pattern = Pattern::Checker;
            s.floor.secondary_color = [0.025; 3];
            s.floor.pattern_scale = 1.;
            s.floor.roughness = 0.22;
            s.sky = 0.25;
            s.distance = 8.;
            s.pitch = 12.;
            s.yaw = 0.;
            s.objects = vec![
                object("Mirror sphere", Shape::Sphere, -1.3, 0., 2.5, 1),
                object("Glass sphere", Shape::Sphere, 1., -0.2, 2.5, 4),
                object("Red sphere", Shape::Sphere, 0., 1.4, 1.2, 7),
            ];
            s.bounces = 16;
        }
        12 | 13 => {
            let shape = if id == 12 {
                Shape::Teapot
            } else {
                Shape::Bunny
            };
            s.distance = 9.;
            s.pitch = 13.;
            s.yaw = 12.;
            s.target = [0., 0., 0.];
            for (i, mat) in if id == 12 { [0, 3, 4] } else { [6, 2, 0] }
                .into_iter()
                .enumerate()
            {
                let x = (i as f32 - 1.) * 2.1;
                let mut plinth =
                    object(format!("Plinth {}", i + 1), Shape::Cylinder, x, 0., 2.2, 9);
                plinth.stretch = 0.3;
                s.objects.push(plinth);
                let mut hero = object(
                    format!("{} {}", shape.label(), i + 1),
                    shape,
                    x,
                    0.,
                    if id == 12 { 2.8 } else { 2.1 },
                    mat,
                );
                hero.elevation = 0.462;
                hero.rotation = if id == 12 { -25. } else { 15. };
                s.objects.push(hero);
            }
            s.lights.push(light(
                "Warm fill",
                [-3., 2.5, 2.],
                [2., 3.],
                [1., 0.74, 0.48],
                8.,
            ));
            s.bounces = 14;
        }
        14 => {
            s.base = [0.8; 3];
            s.floor.pattern = Pattern::Checker;
            s.floor.secondary_color = [0.08; 3];
            s.floor.pattern_scale = 1.5;
            s.floor.roughness = 0.45;
            s.distance = 9.;
            s.pitch = 20.;
            s.yaw = 8.;
            let shapes = [
                Shape::Box,
                Shape::Sphere,
                Shape::Cone,
                Shape::Torus,
                Shape::Teapot,
                Shape::Octahedron,
            ];
            for (i, shape) in shapes.into_iter().enumerate() {
                let mut o = object(
                    format!("Transmission {}", i + 1),
                    shape,
                    (i % 3) as f32 * 1.7 - 1.7,
                    (i / 3) as f32 * 1.8 - 0.9,
                    2.,
                    if i < 3 { 4 } else { 5 },
                );
                o.material.color = if i % 3 == 0 {
                    [0.95, 0.62, 0.52]
                } else if i % 3 == 1 {
                    [0.55, 0.86, 0.98]
                } else {
                    [0.75, 0.97, 0.65]
                };
                if shape == Shape::Torus {
                    o.tilt = [65., 0.];
                }
                s.objects.push(o);
            }
            s.bounces = 20;
        }
        15 => {
            s.base = [0.85; 3];
            s.floor.roughness = 0.9;
            s.sky = 0.;
            s.light_on = false;
            s.lights = vec![
                light(
                    "Red softbox",
                    [-2., 3., 1.],
                    [0.65, 0.65],
                    [1., 0.025, 0.015],
                    24.,
                ),
                light(
                    "Green softbox",
                    [0., 3., -2.],
                    [0.65, 0.65],
                    [0.02, 1., 0.03],
                    24.,
                ),
                light(
                    "Blue softbox",
                    [2., 3., 1.],
                    [0.65, 0.65],
                    [0.025, 0.07, 1.],
                    24.,
                ),
            ];
            s.objects = vec![
                object("White sphere", Shape::Sphere, -0.8, 0.3, 1.8, 0),
                object("White teapot", Shape::Teapot, 0.8, -0.25, 2.5, 0),
            ];
            for o in &mut s.objects {
                o.material.roughness = 0.9;
            }
            s.distance = 7.;
            s.pitch = 28.;
            s.yaw = 0.;
        }
        16 => {
            s.distance = 9.;
            s.pitch = 17.;
            s.yaw = 20.;
            let r = 0.42;
            for layer in 0..4 {
                let n = 4 - layer;
                for z in 0..n {
                    for x in 0..n {
                        let mut o = object(
                            format!("Layer {} sphere {},{}", layer + 1, x + 1, z + 1),
                            Shape::Sphere,
                            (x as f32 - (n - 1) as f32 * 0.5) * r * 2.05,
                            (z as f32 - (n - 1) as f32 * 0.5) * r * 2.05,
                            r / 0.35,
                            (x + z + layer) % 4 + 1,
                        );
                        if o.material.transmission > 0.5 {
                            o.material = material(8);
                        }
                        o.elevation = layer as f32 * r * 1.64;
                        s.objects.push(o);
                    }
                }
            }
        }
        17 => {
            s.distance = 7.;
            s.pitch = 6.;
            s.yaw = 5.;
            s.target = [0., 0., -1.];
            s.bounces = 24;
            s.sky = 0.;
            s.base = [0.12; 3];
            for x in [-1.65, 1.65] {
                let mut o = object("Mirror wall", Shape::Box, x, -1., 4., 1);
                o.scale_xz = [0.025, 2.];
                o.stretch = 1.1;
                s.objects.push(o);
            }
            for i in 0..5 {
                let z = 1. - i as f32 * 1.1;
                let mut o = object(
                    format!("Luminous ring {}", i + 1),
                    Shape::Ring,
                    0.,
                    z,
                    2.4,
                    if i % 2 == 0 { 10 } else { 11 },
                );
                o.tilt = [90., 0.];
                o.elevation = 0.2;
                s.objects.push(o);
            }
            s.light_size = 2.;
            s.light_intensity = 3.;
            s.lights.clear();
        }
        18 => {
            s.distance = 9.;
            s.pitch = 14.;
            s.yaw = 10.;
            for (i, shape) in [Shape::Knot, Shape::Torus, Shape::Knot]
                .into_iter()
                .enumerate()
            {
                let mut o = object(
                    format!("Sculpture {}", i + 1),
                    shape,
                    (i as f32 - 1.) * 2.3,
                    0.,
                    2.8,
                    [2, 1, 3][i],
                );
                o.tilt = [55., 15.];
                o.elevation = 0.35;
                s.objects.push(o);
                let mut base = object(
                    "Black plinth",
                    Shape::Cylinder,
                    (i as f32 - 1.) * 2.3,
                    0.,
                    2.3,
                    7,
                );
                base.material.color = [0.018; 3];
                base.material.roughness = 0.2;
                base.stretch = 0.2;
                s.objects.push(base);
            }
        }
        19 => {
            s.sky = 0.5;
            s.floor_extent = 20.;
            s.base = [0.7, 0.66, 0.55];
            s.floor.roughness = 0.8;
            s.light_position = [-4., 8., 2.];
            s.light_size = 3.;
            s.light_intensity = 28.;
            s.light_temperature = 4600.;
            s.lights.clear();
            s.distance = 14.;
            s.pitch = 18.;
            s.yaw = 25.;
            s.target = [0., 0.5, 0.];
            for z in [-2.5, 2.5] {
                for x in [-3.5, -1.2, 1.2, 3.5] {
                    let mut o = object("Marble column", Shape::Cylinder, x, z, 1.1, 9);
                    o.stretch = 4.6;
                    s.objects.push(o);
                    for e in [0., 3.55] {
                        let mut cap = object("Column capital", Shape::Box, x, z, 1.5, 9);
                        cap.stretch = 0.18;
                        cap.elevation = e;
                        s.objects.push(cap);
                    }
                }
                let mut beam = object("Stone lintel", Shape::Box, 0., z, 3., 9);
                beam.scale_xz = [4., 0.6];
                beam.stretch = 0.2;
                beam.elevation = 3.75;
                s.objects.push(beam);
            }
            s.objects
                .push(object("Courtyard bunny", Shape::Bunny, 0., 0., 3.5, 2));
        }
        20 => {
            s.base = [0.84, 0.8, 0.66];
            s.floor.pattern = Pattern::Checker;
            s.floor.secondary_color = [0.045, 0.065, 0.055];
            s.floor.pattern_scale = 2.;
            s.floor.roughness = 0.16;
            s.distance = 6.5;
            s.pitch = 30.;
            s.yaw = 18.;
            s.target = [0., -0.7, 0.];
            for side in 0..2 {
                for i in 0..8 {
                    let mut o = object(
                        format!(
                            "{} pawn {}",
                            if side == 0 { "Ivory" } else { "Ebony" },
                            i + 1
                        ),
                        Shape::Pawn,
                        (i as f32 - 3.5) * 0.5,
                        if side == 0 { 1.25 } else { -1.25 },
                        0.75,
                        0,
                    );
                    if side == 1 {
                        o.material.color = [0.018, 0.024, 0.03];
                    }
                    s.objects.push(o);
                }
            }
            s.light_size = 5.;
            s.light_position = [-2., 5., 1.];
        }
        21 => {
            s.sky = 0.;
            s.base = [0.025; 3];
            s.floor.roughness = 0.1;
            s.light_on = false;
            s.lights.clear();
            s.distance = 10.;
            s.target = [0., 0.9, 0.];
            s.pitch = 14.;
            s.yaw = 20.;
            let mut knot = object("Chrome heart", Shape::Knot, 0., 0., 3.4, 1);
            knot.tilt = [45., 20.];
            knot.elevation = 0.2;
            s.objects.push(knot);
            for i in 0..3 {
                let mut ring = object(
                    format!("Neon ring {}", i + 1),
                    Shape::Ring,
                    0.,
                    0.,
                    4.8 + i as f32 * 0.45,
                    if i % 2 == 0 { 10 } else { 11 },
                );
                ring.scale_xz = [1., 1.];
                ring.stretch = 1.;
                ring.material.color = if i % 2 == 0 {
                    [1., 0.018, 0.004]
                } else {
                    [0.004, 0.12, 1.]
                };
                ring.material.emission = 2.8;
                ring.grounded = false;
                ring.elevation = 0.3 + i as f32 * 0.45;
                ring.tilt = [35. + i as f32 * 35., i as f32 * 40.];
                s.objects.push(ring);
            }
            s.lights.push(light(
                "Magenta fill",
                [-2., 3., 1.],
                [2., 2.],
                [1., 0.08, 0.45],
                4.,
            ));
            s.bounces = 12;
        }
        22 => {
            s.distance = 11.;
            s.pitch = 20.;
            s.yaw = 20.;
            s.target = [0., 1.7, 0.];
            s.sky = 0.04;
            s.base = [0.08, 0.09, 0.13];
            let mut core = object("Glass nucleus", Shape::Sphere, 0., 0., 3., 4);
            core.grounded = false;
            core.elevation = 1.7;
            s.objects.push(core);
            for ring in 0..3 {
                for i in 0..14 {
                    let a = i as f32 / 14. * TAU;
                    let x = a.cos() * 2.4;
                    let y = a.sin() * 2.4;
                    let (px, py, pz) = match ring {
                        0 => (x, 1.7, y),
                        1 => (x, 1.7 + y, 0.),
                        _ => (0., 1.7 + x, y),
                    };
                    let mut o = object(
                        format!("Orbit {} / {}", ring + 1, i + 1),
                        Shape::Sphere,
                        px,
                        pz,
                        0.65,
                        [2, 3, 8][ring],
                    );
                    o.grounded = false;
                    o.elevation = py;
                    s.objects.push(o);
                }
            }
            s.light_size = 5.;
            s.light_position = [-2., 5., 2.];
        }
        23 => {
            s.objects.clear();
            s.distance = 7.;
            s.yaw = 0.;
            s.pitch = 15.;
            s.target = [0., 0., 0.];
        }
        _ => {}
    }
}
