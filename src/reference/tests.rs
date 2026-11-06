use super::bsdf::{eval_bsdf, ggx_albedo, ggx_table};
use super::bvh::{Bvh, Hit, Tri};
use super::lights::shadow_limit;
use super::sampling::{Rng, cosine_sample};
use super::scene::{Scene, interpolation_normal};
use super::*;
use crate::lighting::PackedMaterial;
use glam::{Mat4, Vec2, Vec3};
use std::f32::consts::PI;
use std::sync::atomic::AtomicBool;
#[test]
fn the_albedo_table_parses_and_a_white_furnace_conserves() {
    assert_eq!(ggx_table().len(), 1024);
    // Smooth surfaces keep almost all light at normal incidence.
    assert!(ggx_albedo(1., 0.) > 0.95);
    // A white diffuse BSDF integrates to about 1 (cosine-weighted, Monte Carlo).
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.color_rough = [1., 1., 1., 1.];
    m.optical[0] = 1.;
    let (n, v) = (Vec3::Y, Vec3::Y);
    let mut r = Rng::new(1, 2);
    let mut sum = Vec3::ZERO;
    let k = 20000;
    for _ in 0..k {
        let l = cosine_sample(n, &mut r);
        // Cosine samples: f cos / (cos / pi).
        let (f, _) = eval_bsdf(&m, n, v, l);
        sum += f * PI;
    }
    let mean = sum / k as f32;
    // The GPU's model: a white rough dielectric with f0 = 0 (IOR 1) keeps about all light.
    assert!((mean.x - 1.).abs() < 0.06, "{mean}");
}
#[test]
fn the_bvh_finds_the_nearest_triangle() {
    let tri = |z: f32, m| Tri {
        p: [
            Vec3::new(-1., -1., z),
            Vec3::new(1., -1., z),
            Vec3::new(0., 1., z),
        ],
        n: [Vec3::Z; 3],
        material: m,
    };
    let bvh = Bvh::build((0..20).map(|i| tri(-(i as f32) - 1., i)).collect());
    let h = bvh
        .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|_| true, false)
        .unwrap();
    assert_eq!(bvh.tris[h.tri].material, 0);
    let h = bvh
        .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|m| m >= 5, false)
        .unwrap();
    assert_eq!(bvh.tris[h.tri].material, 5);
    assert!(
        bvh.intersect(Vec3::ZERO, Vec3::Z, 1e4, &|_| true, false)
            .is_none()
    );
}
/// Each feature the reference does not have is refused, not rendered wrong.
#[test]
fn every_missing_feature_is_refused() {
    use crate::scene::{LightKind, Object};
    let with_material = |f: &dyn Fn(&mut crate::scene::Material)| {
        let mut s = Settings::default();
        f(&mut s.objects[0].material);
        s
    };
    let light = |kind| {
        let mut s = Settings::default();
        s.lights.push(crate::scene::AreaLight {
            kind,
            ..Default::default()
        });
        s
    };
    let edit = |f: &dyn Fn(&mut Settings)| {
        let mut s = Settings::default();
        f(&mut s);
        s
    };
    let cases: Vec<(&str, Settings)> = vec![
        ("HDRI", edit(&|s| s.hdri = "x.hdr".into())),
        ("physical sky", edit(&|s| s.physical_sky.on = true)),
        ("fog", edit(&|s| s.fog.density = 0.1)),
        ("shadow catcher", edit(&|s| s.shadow_catcher = true)),
        ("spectral", edit(&|s| s.spectral = true)),
        ("battle", edit(&|s| s.battle = true)),
        ("firefly", edit(&|s| s.firefly = 10.)),
        ("BDPT", edit(&|s| s.integrator = 1)),
        ("projection", edit(&|s| s.projection = 1)),
        ("depth of field", edit(&|s| s.aperture = 0.1)),
        ("real lens", edit(&|s| s.lens_system = 1)),
        ("sphere light", light(LightKind::Sphere)),
        ("sun light", light(LightKind::Sun)),
        ("IES", light_with_ies(crate::ies::beam_spec(5., 0.1, 0.006))),
        (
            "SDF",
            edit(&|s| {
                s.objects.push(Object {
                    sdf: crate::scene::Sdf::Mandelbulb,
                    ..Object::default()
                })
            }),
        ),
        (
            "object texture set",
            with_material(&|m| m.texture = "set".into()),
        ),
        (
            "floor texture set",
            edit(&|s| s.floor.texture = "set".into()),
        ),
        (
            "wall texture set",
            edit(&|s| s.walls.texture = "set".into()),
        ),
        (
            "portal",
            edit(&|s| {
                s.objects.push(Object::default());
                s.objects[0].portal = 1;
                s.objects[1].portal = 0;
            }),
        ),
        ("beam splitter", with_material(&|m| m.splitter = true)),
        ("coat", with_material(&|m| m.clearcoat = 1.)),
        ("anisotropy", with_material(&|m| m.anisotropy = 0.5)),
        ("sheen", with_material(&|m| m.sheen = 1.)),
        ("thin film", with_material(&|m| m.iridescence = 1.)),
        ("grating", with_material(&|m| m.grating = 1.)),
        (
            "object medium",
            with_material(&|m| {
                m.transmission = 1.;
                m.density = 1.;
            }),
        ),
    ];
    for (what, s) in &cases {
        assert!(!unsupported(s).is_empty(), "{what} was not refused");
    }
    // A profile's numbers in its light's record don't read as materials.
    let ies = light_with_ies(crate::ies::beam_spec(5., 0.1, 0.006));
    assert_eq!(unsupported(&ies), ["IES profiles"]);
    // Walls that aren't there need no texture.
    let mut s = Settings::default();
    s.cornell_room = false;
    s.walls.texture = "set".into();
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
}
/// A mesh's own maps are refused, and maps borrowed from another file (`maps_from`), unless
/// the material leaves them off, when the GPU doesn't bind them either.
#[test]
fn a_meshs_maps_are_refused_unless_turned_off() {
    let dir = std::env::temp_dir().join(format!("caustica_ref_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("quad.gltf");
    crate::mesh::write_test_gltf(&path).unwrap();
    let mut quad = crate::mesh::import_objects(&path).unwrap().remove(0);
    let mut s = Settings::default();
    s.objects = vec![quad.clone()];
    assert!(crate::mesh::maps(&quad).is_some());
    assert!(!unsupported(&s).is_empty());
    s.objects[0].material.textures = false;
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    quad.maps_from = quad.mesh.clone();
    quad.mesh = String::new();
    s.objects = vec![quad];
    assert!(!unsupported(&s).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
fn light_with_ies(ies: String) -> Settings {
    let mut s = Settings::default();
    s.lights.push(crate::scene::AreaLight {
        ies,
        ..Default::default()
    });
    s
}
/// The frame at `t` is the animation at `t`, frozen, plus what the app adds to each frame.
#[test]
fn prepare_applies_the_time_the_flashlight_and_the_beam() {
    let mut s = Settings::default();
    let key = |time, value| crate::anim::Key {
        time,
        value,
        interp: Default::default(),
        text: None,
    };
    s.animation.tracks.push(crate::anim::Track {
        path: "sky".into(),
        keys: vec![key(0., 0.), key(2., 1.)],
    });
    let frame = prepare(&s, 1.);
    assert_eq!(frame.sky, s.animation.apply(&s, 1.).sky);
    assert!(frame.sky > 0. && frame.sky < 1.);
    assert!(frame.animation.is_empty());

    s.flashlight = true;
    let frame = prepare(&s, 0.);
    assert_eq!(frame.lights.len(), s.lights.len() + 1);
    assert_eq!(
        serde_json::to_value(frame.lights.last().unwrap()).unwrap(),
        serde_json::to_value(s.flashlight_light()).unwrap(),
    );
    s.flashlight = false;

    s.puzzle.enabled = true;
    s.puzzle.origin = [-2., -0.5, 0.];
    s.puzzle.direction = [1., 0., 0.];
    s.puzzle.beam = true;
    let frame = prepare(&s, 0.);
    let added = frame.objects.len() - s.objects.len();
    assert!(added > 0, "no rods");
    let materials = crate::lighting::materials(&frame);
    let rod = frame.objects.iter().position(|o| o.name == "Laser beam");
    assert!(materials[5 + rod.unwrap()].optical[1] > 0., "a rod glows");
    // The beam is off: no rods.
    s.puzzle.beam = false;
    assert_eq!(prepare(&s, 0.).objects.len(), s.objects.len());
}
/// A body falls for its time, in fixed steps, whichever renderer takes the result.
#[test]
fn at_time_runs_the_simulation_and_freezes_it() {
    let mut s = Settings::default();
    s.physics.enabled = true;
    s.objects.truncate(1);
    s.objects[0].body = crate::scene::Body::Dynamic;
    s.objects[0].grounded = false;
    s.objects[0].elevation = 3.;
    let start = at_time(&s, 0.);
    assert_eq!(start.objects[0].elevation, 3.);
    let a = at_time(&s, 0.25);
    let b = at_time(&s, 0.25);
    assert!(
        a.objects[0].elevation < 3. - 0.1,
        "{}",
        a.objects[0].elevation
    );
    assert_eq!(a.objects[0].elevation, b.objects[0].elevation);
    assert!(!a.physics.enabled);
}
/// ReSTIR changes the image only where MIS has no BSDF sample to balance: at a one-bounce limit.
#[test]
fn restir_is_refused_only_where_it_changes_the_image() {
    let mut s = Settings::default();
    s.restir = true;
    s.bounces = 1;
    assert!(unsupported(&s).iter().any(|w| w.contains("ReSTIR")));
    s.mis_mode = 1;
    assert!(unsupported(&s).is_empty());
    s.mis_mode = 0;
    s.bounces = 2;
    assert!(unsupported(&s).is_empty());
}
/// The app turns the camera each frame while `rotate` is on: frozen, it turns no further.
#[test]
fn at_time_turns_the_camera_and_freezes_it() {
    let mut s = Settings::default();
    s.rotate = true;
    s.rotate_speed = 6.;
    s.yaw = 10.;
    let a = at_time(&s, 2.);
    assert_eq!(a.yaw, 22.);
    assert!(!a.rotate);
    // Twice (`--time`, then `run`'s `prepare`) is once.
    assert_eq!(prepare(&a, 0.).yaw, 22.);
    assert_eq!(at_time(&s, 0.).yaw, 10.);
}
/// A grazing shadow ray from an origin offset toward the emitter crosses the emitter's plane well
/// before the sampled point, so a fixed margin along the ray makes the light shadow itself.
#[test]
fn a_grazing_shadow_ray_stops_short_of_its_own_emitter() {
    let bvh = Bvh::build(vec![Tri {
        p: [
            Vec3::new(-10., 0., -10.),
            Vec3::new(10., 0., -10.),
            Vec3::new(0., 0., 20.),
        ],
        n: [Vec3::Y; 3],
        material: 0,
    }]);
    let point = Vec3::new(1., 0., 0.);
    let (x, ng) = (Vec3::new(0., -0.05, 0.), Vec3::Y);
    let d = point - x;
    let (dist, ld) = (d.length(), d.normalize());
    let origin = x + ng * 0.0003;
    let hits = |limit| bvh.intersect(origin, ld, limit, &|_| true, true).is_some();
    assert!(hits(dist - 2e-3), "the case the fixed margin got wrong");
    assert!(!hits(shadow_limit(dist, Vec3::Y.dot(ld))));
    // Square on, the stop is 0.6 mm; a nearby point keeps half its distance.
    assert!((shadow_limit(1., 1.) - 0.9994).abs() < 1e-5);
    assert_eq!(shadow_limit(0.002, 0.1), 0.001);
    assert_eq!(shadow_limit(0., 1.), 0.0002);
}
/// Light sampling alone and BSDF sampling alone estimate what MIS does, given bounces to spare
/// (at the bounce limit MIS's last light sample has no BSDF partner, so it isn't a fair test).
#[test]
fn the_sampling_strategies_agree() {
    let mut s = Settings::default();
    s.bounces = 8;
    let mean = |mis_mode| {
        let mut s = s.clone();
        s.mis_mode = mis_mode;
        let image = render(&s, [32, 24], 512, &|_| {}).unwrap();
        image.iter().map(|v| *v as f64).sum::<f64>() / image.len() as f64
    };
    let both = mean(0);
    for mis_mode in [1, 2] {
        let one = mean(mis_mode);
        assert!((one / both - 1.).abs() < 0.03, "{mis_mode}: {one} {both}");
    }
    // A black image would pass the above.
    assert!(both > 0.01, "{both}");
}
/// One large sample doesn't swallow a million small ones (in f32 they would all be lost).
#[test]
fn the_sums_keep_small_samples_beside_a_large_one() {
    let s = Settings::default();
    let mut p = Progressive::new(&s, [1, 1]).unwrap();
    let (small, n) = (1e-3f32, 1_000_000u32);
    p.sums[0] = Vec3::splat(1e3).as_dvec3();
    for _ in 0..n {
        p.sums[0] += Vec3::splat(small).as_dvec3();
    }
    p.samples = n + 1;
    let exact = (1e3 + small as f64 * n as f64) / (n + 1) as f64;
    assert!((p.sums[0].x / p.samples as f64 - exact).abs() < 1e-9);
    assert!((p.image()[0] as f64 - exact).abs() < 1e-6 * exact);
}
/// The side a ray hits comes from the triangle, the normal it shades with stays toward the
/// ray: a smooth mesh's interpolated normal can face away at a grazing hit.
#[test]
fn the_side_is_the_faces_not_the_interpolated_normals() {
    // Facing +z; the interpolated normal leans over so a ray along -z + x sees its back.
    let n = Vec3::new(0.9, 0., 0.436).normalize();
    let t = Tri {
        p: [
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(0., 1., 0.),
        ],
        n: [n; 3],
        material: 0,
    };
    let d = Vec3::new(0.95, 0., -0.3).normalize();
    let (shading, face) = t.normals(0.25, 0.25);
    assert!(d.dot(shading) > 0. && d.dot(face) < 0., "the fixture");
    let bvh = Bvh::build(vec![t]);
    let s = Settings::default();
    let scene = Scene {
        s: &s,
        bvh,
        materials: vec![bytemuck::Zeroable::zeroed()],
        lights: vec![],
        cdf: vec![],
        eye: Vec3::ZERO,
        inverse_vp: Mat4::IDENTITY,
        camera_media: vec![],
        size: [1, 1],
    };
    let hit = Hit {
        t: 1.,
        tri: 0,
        b: Vec2::splat(0.25),
    };
    let sf = scene.surface(&hit, d);
    assert!(sf.front, "from the side the face looks at");
    assert_eq!(sf.ng, Vec3::Z);
    assert!(d.dot(sf.n) < 0., "the shading normal still faces the ray");
}
/// Normals interpolate as in object space, however the object is scaled.
#[test]
fn normals_interpolate_in_object_space() {
    let mut s = Settings::default();
    s.objects[0].stretch = 0.15;
    s.objects[0].scale_xz = [2., 1.];
    s.objects[0].rotation = 30.;
    s.objects[0].tilt = [20., -10.];
    let (scale, r) = crate::geometry::scale_rotation(&s.objects[0]);
    let r = glam::Mat3::from_mat4(r);
    // As `object_geometry` writes them, then as the GPU's instance transform takes them.
    let world = |n: Vec3| r.mul_vec3(n / scale).normalize();
    let (a, b) = (
        Vec3::new(0.8, 0.6, 0.),
        Vec3::new(0., 0.28, 0.96).normalize(),
    );
    let wanted = r.mul_vec3((a + b) * 0.5 / scale).normalize();
    let got =
        (interpolation_normal(&s, 5, world(a)) + interpolation_normal(&s, 5, world(b))).normalize();
    assert!((got - wanted).length() < 1e-5, "{got} {wanted}");
    let unit = (world(a) + world(b)).normalize();
    assert!(
        (unit - wanted).length() > 1e-2,
        "the fixture must tell them apart"
    );
    // Static geometry keeps its normals (nothing to undo).
    assert_eq!(interpolation_normal(&s, 3, Vec3::Y), Vec3::Y);
}
/// The default Cornell box renders, finite and lit.
#[test]
fn the_cornell_box_renders() {
    let mut s = Settings::default();
    s.bounces = 4;
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let img = render(&s, [32, 24], 4, &|_| {}).unwrap();
    assert!(img.iter().all(|v| v.is_finite()));
    let mean: f32 = img.chunks_exact(4).map(|p| p[0] + p[1] + p[2]).sum::<f32>() / (32. * 24.);
    assert!(mean > 0.01, "{mean}");
}
/// Passes of any size give the same bits as one pass.
#[test]
fn passes_match_one_render() {
    let mut s = Settings::default();
    s.bounces = 3;
    let one = render(&s, [16, 12], 5, &|_| {}).unwrap();
    let mut p = Progressive::new(&s, [16, 12]).unwrap();
    let stop = AtomicBool::new(false);
    for n in [2, 1, 2] {
        assert!(p.pass(n, 3, &stop, &|_| {}));
    }
    assert_eq!(p.samples, 5);
    assert_eq!(p.image(), one);
}
