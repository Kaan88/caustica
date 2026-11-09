use super::bsdf::{eval_bsdf, ggx_albedo, ggx_table};
use super::bvh::{Bvh, Hit, Tri};
use super::lights::shadow_limit;
use super::sampling::{Rng, cosine_sample};
use super::scene::{Ray, Scene, Surface, interpolation_normal};
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
        let (f, _) = eval_bsdf(&m, n, Vec3::X, v, l, 0.);
        sum += f * PI;
    }
    let mean = sum / k as f32;
    // The GPU's model: a white rough dielectric with f0 = 0 (IOR 1) keeps about all light.
    assert!((mean.x - 1.).abs() < 0.06, "{mean}");
}
/// With a vanishing strength the anisotropic lobe is the isotropic one: same D, G1 and pdf
/// up to the pdf's form (visible normals against D·nh), and the same value.
#[test]
fn anisotropy_reduces_to_isotropic() {
    use super::bsdf::{aniso_frame, g1, g1_aniso, ggx_d, ggx_d_aniso};
    let a = glam::Vec2::splat(0.3);
    let (n, t) = (Vec3::Y, Vec3::new(1., 0.2, 0.));
    let (tt, bb) = aniso_frame(n, t, 0.7);
    assert!(tt.dot(n).abs() < 1e-6 && bb.dot(tt).abs() < 1e-6);
    let h = Vec3::new(0.3, 0.9, -0.2).normalize();
    let local = Vec3::new(h.dot(tt), h.dot(bb), h.dot(n));
    assert!((ggx_d_aniso(local, a) / ggx_d(h.y, 0.3) - 1.).abs() < 1e-4);
    assert!((g1_aniso(local, a) / g1(h.y, 0.3) - 1.).abs() < 1e-4);
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.color_rough = [0.9, 0.6, 0.3, 0.55];
    m.optical[0] = 1.5;
    m.lobes[0] = 1.;
    let v = Vec3::new(0.5, 0.7, 0.1).normalize();
    let l = Vec3::new(-0.4, 0.8, 0.3).normalize();
    let (fi, _) = eval_bsdf(&m, n, t, v, l, 0.);
    m.layers[3] = 1e-4;
    let (fa, _) = eval_bsdf(&m, n, t, v, l, 0.);
    assert!(
        (fa / fi - Vec3::ONE).abs().max_element() < 0.03,
        "{fa} vs {fi}"
    );
}
/// A white anisotropic metal in a furnace keeps its light at every view azimuth: visible-normal
/// samples weighted by f·cos / pdf average 1 (the albedo table's compensation), and the
/// sampler's directions are the ones eval_bsdf's pdf describes.
#[test]
fn anisotropic_white_metal_furnace() {
    use super::bsdf::{aniso_alpha, aniso_frame, spec_probability};
    use super::sampling::{cosine_sample, ggx_sample_aniso};
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.color_rough = [1., 1., 1., 0.5];
    m.optical[0] = 1.5;
    m.lobes[0] = 1.;
    m.layers[3] = 0.8;
    m.sheen[3] = 0.4;
    let (n, t) = (Vec3::Y, Vec3::X);
    let (tt, bb) = aniso_frame(n, t, m.sheen[3]);
    let ps = spec_probability(&m);
    for (nv, phi) in [(0.9f32, 0.), (0.5, 0.8), (0.3, 1.6), (0.6, 2.5)] {
        let s = (1. - nv * nv).sqrt();
        let v = Vec3::new(s * f32::cos(phi), nv, s * f32::sin(phi));
        let mut r = Rng::new(7, 8);
        let mut sum = 0.;
        let k = 100000;
        for _ in 0..k {
            let l = if r.f() < ps {
                ggx_sample_aniso(n, tt, bb, -v, aniso_alpha(&m), &mut r)
            } else {
                cosine_sample(n, &mut r)
            };
            if l.dot(n) <= 0. {
                continue;
            }
            let (f, pdf) = eval_bsdf(&m, n, t, v, l, 0.);
            sum += f.x * l.dot(n) / pdf.max(1e-7);
        }
        let mean = sum / k as f32;
        assert!((mean - 1.).abs() < 0.03, "nv {nv}, azimuth {phi}: {mean}");
    }
}
/// A white metal grating in a furnace keeps its light: the order and mirror samples weighted by
/// f·cos / pdf average 1 over wavelengths (no order evanescent at this spacing and view), and
/// the pdf integrates to 1 over the hemisphere. A Blu-ray's 320 nm grooves at a normal view have
/// no order leaving, so all the light stays in the mirror.
#[test]
fn grating_white_metal_furnace_and_evanescent_orders() {
    use super::bsdf::{
        GRATING_ORDER, aniso_frame, grating, grating_move, grating_wavelength, order_shares,
        spec_probability,
    };
    use super::sampling::{cosine_sample, ggx_sample};
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.color_rough = [1., 1., 1., 0.3];
    m.optical[0] = 1.5;
    m.lobes[0] = 1.;
    m.optical[3] = -(800. * 16384. + 4000.);
    assert_eq!(grating(&m), Some((0.8, 4000.)));
    let (n, t) = (Vec3::Y, Vec3::X);
    let (_, b) = aniso_frame(n, t, 0.);
    let a = 0.09;
    let ps = spec_probability(&m);
    let v = Vec3::new(0.3, 0.954, 0.).normalize();
    let mut r = Rng::new(5, 6);
    let (mut sum, mut mass, mut lost) = (Vec3::ZERO, 0., 0);
    let k = 200000;
    for _ in 0..k {
        let lambda = grating_wavelength(&mut r.fork(1));
        let (share, spacing) = grating(&m).unwrap();
        let w = order_shares(share, lambda / spacing, n, b, v);
        assert!((w.iter().sum::<f32>() - 0.8).abs() < 1e-5);
        // The pdf over the hemisphere, by uniform directions.
        let (u, phi) = (r.f(), 2. * PI * r.f());
        let s = (1. - u * u).sqrt();
        let lu = Vec3::new(s * phi.cos(), u, s * phi.sin());
        mass += eval_bsdf(&m, n, t, v, lu, lambda).1 * 2. * PI;
        let l = if r.f() < ps {
            let u = r.f();
            let mut c = 0.;
            let o = (0..4).find(|&o| {
                c += w[o];
                u < c
            });
            let d = ggx_sample(n, -v, a, &mut r);
            match o {
                Some(o) => match grating_move(n, b, d, GRATING_ORDER[o] * lambda / spacing) {
                    Some(d) if n.dot(d) > 0. => d,
                    _ => {
                        lost += 1;
                        continue;
                    }
                },
                None => d,
            }
        } else {
            cosine_sample(n, &mut r)
        };
        if l.dot(n) <= 0. {
            continue;
        }
        let (f, pdf) = eval_bsdf(&m, n, t, v, l, lambda);
        sum += f * l.dot(n) / pdf.max(1e-7);
    }
    let (mean, mass) = (sum / k as f32, mass / k as f32);
    for c in 0..3 {
        assert!((mean[c] - 1.).abs() < 0.03, "{mean}, {lost} lost");
    }
    assert!((mass - 1.).abs() < 0.03, "pdf integrates to {mass}");
    // A Blu-ray at a normal view: k = 550 / 320 > 1, every order evanescent.
    assert_eq!(order_shares(1., 550. / 320., n, b, n), [0.; 4]);
    // At normal view a 1600 nm grating's first order leaves at sin θ = λ / d.
    let l1 = grating_move(n, b, n, 550. / 1600.).unwrap();
    assert!((l1.dot(b) - 550. / 1600.).abs() < 1e-6 && (l1.length() - 1.).abs() < 1e-6);
}
/// A white base under a full sheen and a clear coat keeps at most the light it gets (each layer
/// dims what lies below by what it takes), and the pdf of the sampling mix integrates to
/// at most 1.
#[test]
fn coat_and_sheen_layers_conserve_energy() {
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.color_rough = [1., 1., 1., 0.6];
    m.optical[0] = 1.5;
    m.layers = [1., 0.3, 0.5, 0.];
    m.sheen = [1., 1., 1., 0.];
    let n = Vec3::Y;
    for nv in [0.9f32, 0.5, 0.2] {
        let v = Vec3::new((1. - nv * nv).sqrt(), nv, 0.);
        let mut r = Rng::new(3, 4);
        let (mut albedo, mut mass) = (Vec3::ZERO, 0.);
        let k = 40000;
        for _ in 0..k {
            // Uniform hemisphere directions: pdf 1 / 2π.
            let (u, phi) = (r.f(), 2. * PI * r.f());
            let s = (1. - u * u).sqrt();
            let l = Vec3::new(s * phi.cos(), u, s * phi.sin());
            let (f, pdf) = eval_bsdf(&m, n, Vec3::X, v, l, 0.);
            albedo += f * u * 2. * PI;
            mass += pdf * 2. * PI;
        }
        let (albedo, mass) = (albedo / k as f32, mass / k as f32);
        assert!(
            albedo.x < 1.03 && albedo.x > 0.7,
            "nv {nv}: albedo {albedo}"
        );
        // GGX samples that land below the horizon end the path, so the mass may fall short of 1.
        assert!(
            mass > 0.85 && mass < 1.02,
            "nv {nv}: pdf integrates to {mass}"
        );
    }
}
#[test]
fn the_bvh_finds_the_nearest_triangle() {
    let tri = |z: f32, m| Tri {
        uv: [glam::Vec2::ZERO; 3],
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
        .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|_, _, _| true, false)
        .unwrap();
    assert_eq!(bvh.tris[h.tri].material, 0);
    let h = bvh
        .intersect(Vec3::ZERO, -Vec3::Z, 1e4, &|m, _, _| m >= 5, false)
        .unwrap();
    assert_eq!(bvh.tris[h.tri].material, 5);
    assert!(
        bvh.intersect(Vec3::ZERO, Vec3::Z, 1e4, &|_, _, _| true, false)
            .is_none()
    );
}
/// Each feature the reference does not have is refused, not rendered wrong.
#[test]
fn every_missing_feature_is_refused() {
    use crate::scene::{LightKind, Object};
    let edit = |f: &dyn Fn(&mut Settings)| {
        let mut s = Settings::default();
        f(&mut s);
        s
    };
    let cases: Vec<(&str, Settings)> = vec![
        ("spectral", edit(&|s| s.spectral = true)),
        ("battle", edit(&|s| s.battle = true)),
        ("BDPT", edit(&|s| s.integrator = 1)),
        ("LPE", edit(&|s| s.lpe = vec!["C.*L".into()])),
    ];
    for (what, s) in &cases {
        assert!(!unsupported(s).is_empty(), "{what} was not refused");
    }
    // A profile's numbers in its light's record don't read as materials.
    let ies = light_with_ies(crate::ies::beam_spec(5., 0.1, 0.006));
    assert!(unsupported(&ies).is_empty(), "{:?}", unsupported(&ies));
}
/// Box-mapped colour and roughness maps render, bound as the GPU binds them (`attach_maps`:
/// slot, flag 16, value / average), on an object or the floor.
#[test]
fn colour_and_roughness_sets_bind_as_the_gpus() {
    let dir = std::env::temp_dir().join(format!("caustica_sets_{}", std::process::id()));
    let plain = dir.join("plain");
    let rough = dir.join("rough");
    for d in [&plain, &rough] {
        std::fs::create_dir_all(d).unwrap();
        image::RgbaImage::from_fn(2, 2, |x, _| {
            image::Rgba([if x == 0 { 64 } else { 192 }, 128, 128, 255])
        })
        .save(d.join("set_color.png"))
        .unwrap();
    }
    image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 128, 255]))
        .save(rough.join("set_roughness.png"))
        .unwrap();
    let colour = plain.join("set_color.png").to_string_lossy().into_owned();
    let with_rough = rough.join("set_color.png").to_string_lossy().into_owned();
    let mut s = Settings::default();
    s.floor.texture = colour.clone();
    s.objects[0].material.texture = colour.clone();
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let mut materials = crate::lighting::materials(&s);
    let textures = super::textures::Textures::new(&s, &mut materials);
    assert_eq!(textures.maps.len(), 1);
    for i in [3, 5] {
        let m = &materials[i];
        assert_eq!(m.maps[0], 1);
        assert!(m.lobes[3] as u32 & 16 != 0 && m.lobes[3] as u32 & 8 == 0);
        let average = textures.maps[0].average;
        for k in 0..3 {
            assert!((m.base_scale[k] * average[k] - m.color_rough[k]).abs() < 1e-5);
        }
    }
    // A roughness map renders too, scaled by value / average (G = 128).
    s.floor.texture = with_rough;
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let mut materials = crate::lighting::materials(&s);
    let textures = super::textures::Textures::new(&s, &mut materials);
    let floor = &materials[3];
    assert_ne!(floor.maps[2], 0);
    let average = textures.maps[floor.maps[2] as usize - 1].average[1];
    assert!((floor.surface_scale[0] * average - floor.color_rough[3]).abs() < 1e-5);
    let _ = std::fs::remove_dir_all(&dir);
}
/// A mesh's own maps with an alpha cutout render, and borrowed ones (`maps_from`).
#[test]
fn a_meshs_cutout_renders() {
    let dir = std::env::temp_dir().join(format!("caustica_ref_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("quad.gltf");
    crate::mesh::write_test_gltf(&path).unwrap();
    let mut quad = crate::mesh::import_objects(&path).unwrap().remove(0);
    let mut s = Settings::default();
    s.objects = vec![quad.clone()];
    assert!(crate::mesh::maps(&quad).is_some());
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    // Its UVs reach the triangles and give a tangent frame, as the GPU's instance vertices.
    let scene = super::scene::Scene::new(&s, [8, 8]);
    let i = (scene.bvh.tris.iter())
        .position(|t| t.material == 5)
        .unwrap();
    let uvs = crate::mesh::uvs(&quad).unwrap();
    assert!(
        scene.bvh.tris[i]
            .uv
            .iter()
            .all(|uv| uvs.contains(&uv.to_array()))
    );
    assert!(
        scene.bvh.tris[i]
            .uv
            .iter()
            .any(|uv| *uv != glam::Vec2::ZERO)
    );
    let hit = super::bvh::Hit {
        t: 1.,
        tri: i,
        b: glam::Vec2::splat(0.25),
        material: scene.bvh.tris[i].material,
        sdf: None,
    };
    let (_, tangent) = scene.map_frame(&hit).unwrap();
    assert!(tangent.w.abs() == 1. && tangent.truncate().length() > 0.);
    s.objects[0].material.textures = false;
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    quad.maps_from = quad.mesh.clone();
    quad.mesh = String::new();
    s.objects = vec![quad];
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let _ = std::fs::remove_dir_all(&dir);
}
/// A light's packed profile reads back as the profile: its samples at their angles, linear in
/// angle between them, nothing behind the light, and it replaces a spot's cone.
#[test]
fn ies_profile_reads_the_packed_samples() {
    use super::bsdf::{ies_profile, spot};
    let spec = crate::ies::beam_spec(20., 0.1, 0.006);
    let p = crate::ies::profile(&spec).unwrap();
    let mut s = light_with_ies(spec);
    s.lights[0].kind = crate::scene::LightKind::Spot;
    let m = *crate::lighting::materials(&s)
        .iter()
        .find(|m| m.lobes[3] as u32 & 16384 != 0)
        .unwrap();
    assert_eq!(m.lobes[3] as u32 & 32, 0, "a profile has no cone");
    let step = 90. / (crate::ies::SAMPLES - 1) as f32;
    let byte = 0.5 / 255. + 1e-4;
    for i in [0, 7, 30, 58] {
        let c = (i as f32 * step).to_radians().cos();
        assert!((ies_profile(&m, c) - p[i]).abs() < byte, "sample {i}");
    }
    // Halfway between samples 3 and 4 (45 degrees falls on neither).
    let c = (3.5 * step).to_radians().cos();
    assert!((ies_profile(&m, c) - 0.5 * (p[3] + p[4])).abs() < byte);
    let c45 = 45f32.to_radians().cos();
    let x = 45. / step;
    let want = p[x as usize] + (p[x as usize + 1] - p[x as usize]) * x.fract();
    assert!((ies_profile(&m, c45) - want).abs() < byte);
    assert_eq!(ies_profile(&m, 0.), 0.);
    assert_eq!(ies_profile(&m, -0.5), 0.);
    assert_eq!(spot(&m, 1.), ies_profile(&m, 1.));
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
/// ReSTIR renders everywhere; its share of samples follows the GPU's reservoir layers.
#[test]
fn restir_renders_with_the_gpus_share_of_samples() {
    let mut s = Settings::default();
    s.restir = true;
    s.bounces = 1;
    assert!(unsupported(&s).is_empty());
    use super::lights::restir_share;
    assert_eq!(restir_share(1, 320 * 240), 1.);
    assert_eq!(restir_share(16, 320 * 240), 0.25);
    // 4K: two layers fit in 1 GiB.
    assert_eq!(restir_share(8, 3840 * 2160), 0.25);
    assert_eq!(restir_share(0, 0), 1.);
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
        uv: [glam::Vec2::ZERO; 3],
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
    let hits = |limit| {
        bvh.intersect(origin, ld, limit, &|_, _, _| true, true)
            .is_some()
    };
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
        uv: [glam::Vec2::ZERO; 3],
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
        sdfs: vec![],
        materials: vec![bytemuck::Zeroable::zeroed()],
        lights: vec![],
        tri_light: vec![],
        cdf: vec![],
        sdf_lights: vec![],
        sdf_share: 0.,
        eye: Vec3::ZERO,
        inverse_vp: Mat4::IDENTITY,
        frame: Default::default(),
        lens: None,
        camera_media: vec![],
        size: [1, 1],
        env: None,
        p_env: 0.,
        textures: Default::default(),
    };
    let hit = Hit {
        t: 1.,
        tri: 0,
        b: Vec2::splat(0.25),
        material: 0,
        sdf: None,
    };
    let sf = scene.surface(&hit, Vec3::ZERO, d);
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
/// A moved camera renders the same bits as a fresh render from there, at any size.
#[test]
fn a_moved_camera_matches_a_fresh_render() {
    let mut s = Settings::default();
    s.bounces = 3;
    let mut moved = s.clone();
    moved.yaw += 0.4;
    moved.distance *= 0.8;
    let stop = AtomicBool::new(false);
    let mut p = Progressive::new(&s, [16, 12]).unwrap();
    assert!(p.pass(2, 2, &stop, &|_| {}));
    p.move_camera(&moved, [12, 10]);
    assert_eq!(p.samples, 0);
    assert!(p.pass(3, 2, &stop, &|_| {}));
    assert_eq!(p.image(), render(&moved, [12, 10], 3, &|_| {}).unwrap());
}
#[test]
fn a_preview_spreads_one_sample_over_each_block() {
    let mut s = Settings::default();
    s.bounces = 3;
    let p = Progressive::new(&s, [17, 10]).unwrap();
    let image = p.preview(4, 2, &AtomicBool::new(false)).unwrap();
    assert_eq!(image.len(), 17 * 10 * 4);
    assert!(image.iter().all(|v| v.is_finite()));
    let at = |x: usize, y: usize| &image[(y * 17 + x) * 4..(y * 17 + x) * 4 + 4];
    assert_eq!(at(0, 0), at(3, 3));
    assert_eq!(at(16, 9), at(16, 8));
    assert!(image.iter().any(|v| *v != image[0]));
}
#[test]
fn slow_samples_take_bigger_blocks() {
    assert_eq!(progressive::preview_block(0.01), 1);
    assert_eq!(progressive::preview_block(0.05), 1);
    assert_eq!(progressive::preview_block(0.2), 2);
    assert_eq!(progressive::preview_block(3.2), 8);
    assert_eq!(progressive::preview_block(1e6), 32);
}
#[test]
fn only_the_camera_keeps_the_scene() {
    let s = Settings::default();
    let mut camera = s.clone();
    camera.yaw += 1.;
    camera.pitch += 0.1;
    camera.distance += 1.;
    camera.target[0] += 1.;
    camera.fov += 1.;
    assert!(progressive::same_but_camera(&s, &camera));
    let mut edit = camera.clone();
    edit.bounces += 1;
    assert!(!progressive::same_but_camera(&s, &edit));
}
/// Waits for the render to finish (a stuck thread fails the test instead of hanging it).
fn finish(live: &Live) -> LiveState {
    let start = std::time::Instant::now();
    while live.running() && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!live.running());
    live.state()
}
#[test]
fn the_view_follows_the_camera_and_the_scene() {
    let mut s = Settings::default();
    s.bounces = 3;
    s.max_samples = 3;
    let size = [16, 12];
    let mut live = Live::start(s.clone(), size, "a".into(), true);
    let st = finish(&live);
    assert_eq!((st.samples, st.preview), (3, false));
    let image = live.image(false).unwrap();
    assert_eq!(*image, render(&s, size, 3, &|_| {}).unwrap());
    assert!(live.image(false).is_none());
    // Taken or not, the finished image stays readable for a capture or the graph.
    let (shown, plain, samples) = live.latest().unwrap();
    assert_eq!(
        (*shown == *image, *plain == *image, samples),
        (true, true, 3)
    );
    // A new camera at a new size: the old image is no use at that size, and the new one is exact.
    let mut moved = s.clone();
    moved.yaw += 0.5;
    live.follow(moved.clone(), "b".into(), [12, 10]);
    assert!(live.running());
    assert!(live.latest().is_none());
    finish(&live);
    let image = live.image(true).unwrap();
    assert_eq!(*image, render(&moved, [12, 10], 3, &|_| {}).unwrap());
    // A different scene: the worker builds it.
    let mut edited = moved.clone();
    edited.bounces = 5;
    live.follow(edited.clone(), "c".into(), [12, 10]);
    finish(&live);
    let image = live.image(true).unwrap();
    assert_eq!(*image, render(&edited, [12, 10], 3, &|_| {}).unwrap());
}
/// The reference refuses what the view renders and lists.
#[test]
fn the_view_lists_what_the_reference_refuses() {
    let mut s = Settings::default();
    s.max_samples = 2;
    s.spectral = true;
    let size = [8, 6];
    let reference = Live::start(s.clone(), size, String::new(), false);
    assert!(finish(&reference).error.unwrap().contains("spectral"));
    assert!(reference.image(true).is_none());
    let view = Live::start(s, size, String::new(), true);
    let st = finish(&view);
    assert!(st.error.is_none());
    assert_eq!(st.missing, ["spectral rendering"]);
    assert!(view.image(false).is_some());
}
#[test]
fn a_paused_view_holds_after_its_pass_and_goes_on() {
    let mut s = Settings::default();
    s.max_samples = 1 << 30;
    s.bounces = 2;
    let live = Live::start(s, [4, 3], String::new(), true);
    live.set_paused(true);
    let start = std::time::Instant::now();
    while live.state().samples == 0 && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    let held = live.state().samples;
    assert!(held > 0);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(live.state().samples, held);
    live.set_paused(false);
    while live.state().samples == held && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(live.state().samples > held);
}

#[test]
fn camera_rays_follow_each_projection() {
    let mut s = Settings::default();
    let centre = Vec2::new(160., 120.);
    let corner = Vec2::new(10., 10.);
    let perspective = Scene::new(&s, [320, 240]);
    let f = perspective.frame;
    // Perspective: the dispatcher is the plain pinhole, and the centre looks down the axis.
    let (o, d) = pinhole(&perspective, corner);
    assert_eq!(
        (o, d),
        (perspective.eye, perspective.camera_direction(corner))
    );
    assert!(pinhole(&perspective, centre).1.dot(f.forward) > 0.9999);
    // Orthographic: parallel rays from points spread across the film.
    s.projection = 1;
    let ortho = Scene::new(&s, [320, 240]);
    let ((oa, da), (ob, db)) = (pinhole(&ortho, centre), pinhole(&ortho, corner));
    assert_eq!(da, db);
    assert!((oa - ortho.eye).length() < 1e-3 && (ob - oa).length() > 0.1);
    // Equirectangular: the left and right edges look backward, the top straight up.
    s.projection = 2;
    let pano = Scene::new(&s, [320, 240]);
    assert!(pinhole(&pano, Vec2::new(0., 120.)).1.dot(f.forward) < -0.999);
    assert!(pinhole(&pano, Vec2::new(160., 0.)).1.dot(f.up) > 0.999);
    // Little planet: the centre looks straight down.
    s.projection = 3;
    let planet = Scene::new(&s, [320, 240]);
    assert!(pinhole(&planet, centre).1.y < -0.999);
}
/// A pinhole camera ray: it must not draw random numbers or be cut off.
fn pinhole(scene: &Scene, pixel: Vec2) -> (Vec3, Vec3) {
    let mut r = Rng::new(0, 0);
    let (o, d, w) = scene.camera_ray(pixel, &mut r).unwrap();
    assert_eq!(r.f(), Rng::new(0, 0).f());
    assert_eq!(w, Vec3::ONE);
    (o, d)
}
/// The thin lens: every aperture sample of a pixel passes through one focal point (on the
/// tilted plane too), bladed samples stay inside the polygon, and the cat-eye cuts the corners
/// more than the centre.
#[test]
fn thin_lens_samples_focus_and_stay_in_the_aperture() {
    let mut s = Settings::default();
    (s.aperture, s.focus_distance) = (0.1, 3.);
    for (projection, tilt) in [(0, 0.), (4, 10.)] {
        (s.projection, s.tilt) = (projection, tilt);
        let scene = Scene::new(&s, [320, 240]);
        let pixel = Vec2::new(40., 200.);
        let f = scene.frame;
        let n = if projection == 4 {
            (f.forward * f.tilt.cos() + f.up * f.tilt.sin()).normalize()
        } else {
            f.forward
        };
        // Where a ray meets the focal plane, through the point 3 ahead on the axis.
        let plane = scene.eye + f.forward * 3.;
        let meet = |(o, d): (Vec3, Vec3)| o + d * ((plane - o).dot(n) / d.dot(n));
        let focal = meet((scene.eye, scene.camera_direction(pixel)));
        let mut r = Rng::new(1, 2);
        for _ in 0..64 {
            let (o, d, _) = scene.camera_ray(pixel, &mut r).unwrap();
            assert!((meet((o, d)) - focal).length() < 1e-3);
        }
    }
    let rot = 20f32.to_radians();
    let mut r = Rng::new(3, 4);
    for _ in 0..4096 {
        let l = super::camera::aperture_point(6, rot, &mut r);
        // Inside a hexagon whose corners sit at rot + k·60°: within the apothem of each edge.
        for k in 0..6 {
            let mid = Vec2::from_angle(rot + (k as f32 + 0.5) * PI / 3.);
            assert!(l.dot(mid) <= (PI / 6.).cos() + 1e-5);
        }
    }
    s.cat_eye = 0.5;
    let scene = Scene::new(&s, [320, 240]);
    let cut = |pixel: Vec2| {
        let mut r = Rng::new(5, 6);
        (0..4096)
            .filter(|_| scene.camera_ray(pixel, &mut r).is_none())
            .count()
    };
    assert_eq!(cut(Vec2::new(160., 120.)), 0);
    assert!(cut(Vec2::new(2., 2.)) > 500);
}
/// Henyey-Greenstein's pdf integrates to one and its samples follow it (mean cos θ = g).
#[test]
fn henyey_greenstein_is_normalised_and_sampled() {
    use super::media::{hg_pdf, hg_sample};
    for g in [-0.6f32, 0., 0.4, 0.85] {
        let n = 20000;
        let integral: f32 = (0..n)
            .map(|i| {
                let c = -1. + 2. * (i as f32 + 0.5) / n as f32;
                hg_pdf(c, g) * 2. * std::f32::consts::PI * 2. / n as f32
            })
            .sum();
        assert!((integral - 1.).abs() < 1e-2, "g {g}: {integral}");
        let mut r = super::sampling::Rng::new(7, 3);
        let d = glam::Vec3::Z;
        let mean: f32 = (0..n)
            .map(|_| hg_sample(d, g, &mut r).0.dot(d))
            .sum::<f32>()
            / n as f32;
        assert!((mean - g).abs() < 0.02, "g {g}: mean cos {mean}");
    }
}

/// A spot's cosine is signed against the emitter's normal both when a bounce hits it and when
/// light sampling picks it (`sample_light`), so MIS weighs the same radiance: from behind a
/// two-sided spot, both see the cosine as negative (outside the cone).
#[test]
fn a_spots_cosine_is_the_same_hit_or_sampled() {
    let t = Tri {
        uv: [glam::Vec2::ZERO; 3],
        p: [
            Vec3::new(-1., -1., 0.),
            Vec3::new(1., -1., 0.),
            Vec3::new(0., 1., 0.),
        ],
        n: [Vec3::Z; 3],
        material: 0,
    };
    for d in [Vec3::new(0.3, 0., -1.), Vec3::new(0.3, 0., 1.)] {
        let d = d.normalize();
        let sf = Surface::on(&t, 0.3, 0.3, d);
        // As `sample_light` takes it: ld = d toward the light, the cosine (-ld)·n.
        let (_, n) = t.normals(0.3, 0.3);
        assert!((sf.emission_cos(d) - (-d).dot(n)).abs() < 1e-6);
        assert_eq!(sf.emission_cos(d) > 0., sf.front);
    }
}
/// A sphere light in the Cornell box: light sampling in its cone alone, BSDF sampling alone and
/// MIS agree, as with the triangle lights.
#[test]
fn a_sphere_lights_strategies_agree() {
    let mut s = Settings::default();
    s.bounces = 8;
    s.lights.push(crate::scene::AreaLight {
        kind: crate::scene::LightKind::Sphere,
        position: [0.3, 0.2, 0.],
        size: [0.3, 0.3],
        intensity: 20.,
        ..Default::default()
    });
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
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
    assert!(both > 0.01, "{both}");
}
/// The sphere's mesh maps to its one table entry, the probabilities sum to 1, and from inside
/// the sphere it is neither sampled nor weighted (its cone is the whole sphere of directions).
#[test]
fn a_sphere_light_is_one_entry() {
    use super::lights::{cone_sample, sphere_om};
    use super::scene::Light;
    let mut s = Settings::default();
    s.lights.push(crate::scene::AreaLight {
        kind: crate::scene::LightKind::Sphere,
        position: [0., 0.2, 0.],
        size: [0.4, 0.4],
        intensity: 5.,
        ..Default::default()
    });
    let scene = Scene::new(&s, [8, 8]);
    let k = scene
        .lights
        .iter()
        .position(|l| matches!(l, Light::Sphere { .. }))
        .unwrap() as u32
        + 1;
    let material = 5 + s.objects.len() as u32 + s.lights.len() as u32 - 1;
    let mesh: Vec<_> = (0..scene.bvh.tris.len())
        .filter(|&i| scene.bvh.tris[i].material == material)
        .collect();
    assert!(!mesh.is_empty());
    assert!(mesh.iter().all(|&i| scene.tri_light[i] == k));
    assert!((scene.cdf.last().unwrap() - 1.).abs() < 1e-5);
    let c = Vec3::new(0., 0.2, 0.);
    let at = scene.bvh.tris[mesh[0]].p[0];
    assert_eq!(scene.light_pdf(mesh[0], c, at), 0.);
    assert!(scene.light_pdf(mesh[0], c + Vec3::Z * 0.8, at) > 0.);
    assert_eq!(sphere_om(0.01, 0.2), 0.);
    // Far: 1 − cos θ = r² / 2d² to first order; the cone's samples stay inside it.
    assert!((sphere_om(1e6, 1.) / 5e-7 - 1.).abs() < 1e-3);
    let mut r = Rng::new(5, 6);
    let om = sphere_om(4., 1.);
    for _ in 0..100 {
        let (d, pdf) = cone_sample(Vec3::X, om, &mut r);
        assert!(1. - d.x <= om * 1.001 && (d.length() - 1.).abs() < 1e-4);
        assert!((pdf * 2. * PI * om - 1.).abs() < 1e-5);
    }
}
/// A camera inside a sphere light sees its emission and stays finite.
#[test]
fn a_camera_inside_a_sphere_light_stays_finite() {
    let mut s = Settings::default();
    let (_, _, eye) = s.camera(1.);
    s.lights.push(crate::scene::AreaLight {
        kind: crate::scene::LightKind::Sphere,
        position: eye.to_array(),
        size: [0.5, 0.5],
        intensity: 2.,
        ..Default::default()
    });
    let image = render(&s, [8, 8], 4, &|_| {}).unwrap();
    assert!(image.iter().all(|v| v.is_finite()));
    assert!(image.iter().any(|v| *v > 0.));
}
/// A sun outdoors: light sampling in its disc alone, BSDF sampling alone (meeting the disc on a
/// miss) and MIS agree; its table radiance is `AreaLight::radiance`'s (intensity 1 lights a
/// white surface facing it to 1); a hidden sun is seen from bounce 1 on only.
#[test]
fn a_sun_lights_as_defined() {
    use super::scene::Light;
    let mut s = Settings::default();
    s.cornell_room = false;
    s.bounces = 4;
    s.lights.clear();
    s.lights.push(crate::scene::AreaLight {
        kind: crate::scene::LightKind::Sun,
        rotation: [30., 20., 0.],
        // Wide enough for BSDF sampling to find it in a test-sized render.
        angle: 12.,
        intensity: 1.,
        ..Default::default()
    });
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let scene = Scene::new(&s, [8, 8]);
    let Some(&Light::Sun { d, om, le, hidden }) = scene.lights.first() else {
        panic!("no sun entry: {:?}", scene.lights);
    };
    assert!(!hidden);
    assert!((le.x * om * (2. - om) - 1.).abs() < 1e-3, "{le} {om}");
    assert!((d + s.lights[0].direction()).length() < 1e-5);
    assert_eq!(scene.suns(d, 0, |_| 1.), le);
    assert_eq!(scene.suns(-d, 1, |_| 1.), Vec3::ZERO);
    let mut r = Rng::new(3, 4);
    let (ld, l, pl, limit) = scene.sample_light(Vec3::ZERO, &mut r).unwrap();
    assert!(d.cross(ld).length_squared() <= om * (2. - om) * 1.001 && l == le);
    let pick = (1. - scene.p_env) * scene.cdf[0];
    assert!(
        (pl * 2. * PI * om / pick - 1.).abs() < 1e-4 && limit > 1e3,
        "{pl} {pick}"
    );
    let mean = |mis_mode| {
        let mut s = s.clone();
        s.mis_mode = mis_mode;
        let image = render(&s, [32, 24], 256, &|_| {}).unwrap();
        image.iter().map(|v| *v as f64).sum::<f64>() / image.len() as f64
    };
    let both = mean(0);
    for mis_mode in [1, 2] {
        let one = mean(mis_mode);
        assert!((one / both - 1.).abs() < 0.03, "{mis_mode}: {one} {both}");
    }
    assert!(both > 0.01, "{both}");
    s.lights[0].visible = false;
    let scene = Scene::new(&s, [8, 8]);
    assert_eq!(scene.suns(d, 0, |_| 1.), Vec3::ZERO);
    assert_eq!(scene.suns(d, 1, |_| 1.), le);
}
/// A lens system: at the frame's centre the mean weight, a blocked sample counting 0, is 1 for
/// every prescription (`lens::pack` normalises the centre's pupil); with dispersion each channel
/// keeps that mean, and red and blue leave the lens apart at the frame's edge.
#[test]
fn lens_systems_weigh_one_at_the_centre() {
    let mut s = Settings::default();
    (s.f_number, s.focus_distance) = (4., 4.7);
    let mean = |scene: &Scene, pixel: Vec2| {
        let mut r = Rng::new(7, 8);
        let n = 20000;
        (0..n)
            .filter_map(|_| scene.camera_ray(pixel, &mut r))
            .map(|(_, _, w)| w)
            .sum::<Vec3>()
            / n as f32
    };
    let centre = Vec2::new(160., 120.);
    for lens in 1..=5 {
        s.lens_system = lens;
        let scene = Scene::new(&s, [320, 240]);
        let m = mean(&scene, centre);
        assert!(
            (m - Vec3::ONE).abs().max_element() < 0.03,
            "lens {lens}: {m}"
        );
    }
    (s.lens_system, s.dispersion) = (4, 1.);
    let scene = Scene::new(&s, [320, 240]);
    let m = mean(&scene, centre);
    assert!(
        (m - Vec3::ONE).abs().max_element() < 0.06,
        "dispersion: {m}"
    );
    // One film point and pupil sample traced in red and in blue.
    let lens = scene.lens.as_ref().unwrap();
    let (o, d) = (
        Vec3::new(15., 10., 0.),
        Vec3::new(-0.2, -0.1, -1.).normalize(),
    );
    let red = lens.trace_channel(o, d, 0).unwrap().1;
    let blue = lens.trace_channel(o, d, 2).unwrap().1;
    assert!(red.angle_between(blue) > 1e-4);
}
/// The stop test accepts exactly the polygon `aperture_point` samples.
#[test]
fn the_bladed_stop_holds_the_aperture_samples() {
    let rot = 20f32.to_radians();
    let mut r = Rng::new(9, 10);
    for _ in 0..4096 {
        let l = super::camera::aperture_point(6, rot, &mut r);
        assert!(super::camera::inside_stop(l * 0.999, 1., 6, rot));
    }
    // A corner is in, past it is out; the edge's midpoint sits at the apothem.
    let corner = Vec2::from_angle(rot);
    assert!(super::camera::inside_stop(corner * 0.999, 1., 6, rot));
    assert!(!super::camera::inside_stop(corner * 1.001, 1., 6, rot));
    let mid = Vec2::from_angle(rot + PI / 6.) * (PI / 6.).cos();
    assert!(super::camera::inside_stop(mid * 0.999, 1., 6, rot));
    assert!(!super::camera::inside_stop(mid * 1.001, 1., 6, rot));
}
#[test]
fn a_portal_maps_its_centre_to_its_pairs_and_turns_the_ray_around() {
    use crate::reference::integrators::path::portal_map;
    use crate::scene::Object;
    let mut s = Settings::default();
    s.objects = vec![Object::default(), Object::default()];
    s.objects[0].position = [-0.5, 0.];
    s.objects[1].position = [0.5, -0.8];
    s.objects[0].portal = 1;
    s.objects[1].portal = -1;
    let all = crate::lighting::materials(&s);
    let a = all.iter().find(|m| m.lobes[3] as u32 & 4 != 0).unwrap();
    let map = &all[a.secondary[3] as usize];
    let ca = crate::geometry::object_frame(&s.objects[0]).0;
    let cb = crate::geometry::object_frame(&s.objects[1]).0;
    let (o, d) = portal_map(map, ca, Vec3::new(0.6, 0., -0.8));
    assert!((o - (cb + d * 0.001)).length() < 1e-4, "{o} vs {cb}");
    assert!((d - Vec3::new(-0.6, 0., 0.8)).length() < 1e-4, "{d}");
    // Its matrix rows (a negative z translation here) don't read as a grating or a splitter.
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
}
/// A beam splitter is a mix of a clear sheet and a mirror: white at reflectivity 0 it vanishes,
/// and at 0.5 the image is the mean of its two ends.
#[test]
fn a_beam_splitter_mixes_a_clear_sheet_and_a_mirror() {
    let size = [24, 18];
    let mean = |reflectivity: Option<f32>| {
        let mut s = Settings::default();
        s.bounces = 4;
        // BSDF sampling only: shadow rays see a splitter as opaque (as on the GPU), so with
        // light sampling the last bounce's light beyond a clear sheet is lost, and it isn't
        // the same image as no sheet.
        s.mis_mode = 2;
        let m = &mut s.objects[0].material;
        m.color = [1.; 3];
        match reflectivity {
            Some(r) => {
                m.splitter = true;
                m.reflectivity = r;
            }
            None => s.objects[0].visible = false,
        }
        assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
        let img = render(&s, size, 64, &|_| {}).unwrap();
        assert!(img.iter().all(|v| v.is_finite()));
        img.chunks_exact(4).map(|p| p[0] + p[1] + p[2]).sum::<f32>() / (size[0] * size[1]) as f32
    };
    let (gone, clear, mirror, half) = (mean(None), mean(Some(0.)), mean(Some(1.)), mean(Some(0.5)));
    assert!((clear / gone - 1.).abs() < 0.03, "{clear} {gone}");
    assert!((mirror / clear - 1.).abs() > 0.01, "{mirror} {clear}");
    let mix = 0.5 * (clear + mirror);
    assert!((half / mix - 1.).abs() < 0.03, "{half} {mix}");
}
/// Light painting's rule: lights and the camera keep the exposure, anything else starts over.
#[test]
fn painting_keeps_the_exposure_through_lights_and_the_camera_only() {
    let s = Settings::default();
    let key = crate::transport_key(&s);
    let mut lit = s.clone();
    lit.light_position[0] += 0.1;
    lit.yaw += 10.;
    assert!(crate::painting_keeps(&key, &crate::transport_key(&lit)));
    let mut edited = s.clone();
    edited.bounces += 1;
    assert!(!crate::painting_keeps(&key, &crate::transport_key(&edited)));
    assert!(!crate::painting_keeps("", &key));
}
fn samples_reach(live: &Live, n: u32) -> u32 {
    let start = std::time::Instant::now();
    while live.state().samples < n && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    live.state().samples
}
/// Painting goes on past the cap and through a light edit; another edit starts over.
#[test]
fn a_painting_view_keeps_its_exposure_through_a_light_edit() {
    let mut s = Settings::default();
    s.bounces = 2;
    s.max_samples = 2;
    let size = [6, 4];
    let mut live = Live::start(s.clone(), size, crate::transport_key(&s), true);
    live.set_painting(true);
    assert!(samples_reach(&live, 4) >= 4);
    let mut lit = s.clone();
    lit.light_position[0] += 0.2;
    let before = live.state().samples;
    live.follow(lit.clone(), crate::transport_key(&lit), size);
    let start = std::time::Instant::now();
    while live.state().images < 2 && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(samples_reach(&live, before + 2) >= before + 2);
    // Off again: the cap holds, and an edit that isn't a light's restarts from nothing.
    live.set_painting(false);
    let mut edited = lit.clone();
    edited.bounces = 3;
    live.follow(edited.clone(), crate::transport_key(&edited), size);
    let st = finish(&live);
    assert_eq!(st.samples, 2);
    assert_eq!(
        *live.image(true).unwrap(),
        render(&edited, size, 2, &|_| {}).unwrap()
    );
}
/// A keyed object under an open shutter: the frame averages moments around its time, and any
/// split into passes still gives the bits of one pass.
#[test]
fn an_open_shutter_blurs_the_frame_over_its_moments() {
    use crate::anim::{Interp, Key, Track};
    let mut s = Settings::default();
    s.bounces = 2;
    assert!(!s.objects.is_empty());
    let key = |time: f32, value: f64| Key {
        time,
        value,
        interp: Interp::Linear,
        text: None,
    };
    s.animation.tracks.push(Track {
        path: "objects/0/position/0".into(),
        keys: vec![key(0., -0.6), key(1., 0.6)],
    });
    s.animation.shutter = 0.;
    let still = exposure(&s, 0.5).unwrap();
    assert_eq!(still.len(), 1);
    assert!(still[0] == prepare(&s, 0.5));
    s.animation.shutter = 1.;
    let moments = exposure(&s, 0.5).unwrap();
    assert_eq!(moments.len(), SHUTTER_MOMENTS);
    let x = |m: &Settings| m.objects[0].position[0];
    let step = 1.2 / s.animation.fps as f32 / SHUTTER_MOMENTS as f32;
    assert!(
        (x(&moments[0]) - (x(&still[0]) - 1.2 / s.animation.fps as f32 / 2. + step / 2.)).abs()
            < 1e-4
    );
    assert!(x(&moments[SHUTTER_MOMENTS - 1]) > x(&still[0]));
    let size = [8, 6];
    let mut one = Progressive::over(&moments, size).unwrap();
    one.pass(4, 2, &AtomicBool::new(false), &|_| {});
    let mut split = Progressive::over(&moments, size).unwrap();
    split.pass(1, 1, &AtomicBool::new(false), &|_| {});
    split.pass(3, 3, &AtomicBool::new(false), &|_| {});
    assert_eq!(one.image(), split.image());
    assert_ne!(one.image(), render(&still[0], size, 4, &|_| {}).unwrap());
    // The simulation isn't replayed.
    s.physics.enabled = true;
    assert!(exposure(&s, 0.5).is_err());
}
/// An emissive map (flag 64) emits its texel at the mesh UV times its mean colour over the
/// map's mean, on BSDF hits and light samples alike, and the light table weighs it by that mean.
/// Lantern.glb is the local model with an emissiveTexture; skipped where the scenes are absent.
#[test]
fn an_emissive_map_emits_its_texel_over_its_mean() {
    use super::scene::Light;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/models/Lantern.glb");
    if !path.exists() {
        eprintln!("skipped: no {}", path.display());
        return;
    }
    let mut lantern = crate::mesh::import_objects(&path).unwrap().remove(0);
    lantern.material.emission = 4.;
    let mut s = Settings::default();
    s.objects = vec![lantern];
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let scene = Scene::new(&s, [8, 8]);
    let raw = &scene.materials[5];
    assert!(super::bsdf::flags(raw) & 64 != 0);
    let map = &scene.textures.maps[raw.secondary[0] as usize - 1];
    let i = (scene.bvh.tris.iter())
        .position(|t| t.material == 5 && t.uv.iter().any(|uv| *uv != Vec2::ZERO))
        .unwrap();
    assert!(scene.lights.contains(&Light::Tri(i)));
    let t = &scene.bvh.tris[i];
    let b = Vec2::new(0.2, 0.3);
    let uv = t.uv[0] * 0.5 + t.uv[1] * 0.2 + t.uv[2] * 0.3;
    let want = Vec3::from_slice(&raw.secondary[1..4])
        * super::textures::bilinear(map, uv).truncate()
        * Vec3::from_slice(&raw.medium[..3]);
    let got = scene.emission_colour(raw, i, b);
    assert!((got - want).length() < 1e-6, "{got} {want}");
    // The mean colour times the map's mean over itself is the mean colour.
    let mean = Vec3::from_slice(&map.average) * Vec3::from_slice(&raw.medium[..3]);
    assert!((mean - Vec3::ONE).abs().max_element() < 1e-3, "{mean}");
    // The light table's power is the emitter table's (`lighting::emitted`, the mean colour).
    // The table is normalised; this scene has triangle lights only.
    let k = scene
        .lights
        .iter()
        .position(|&l| l == Light::Tri(i))
        .unwrap();
    let c = crate::lighting::emitted(raw);
    let w = Scene::area(t) * (0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z);
    let before = if k == 0 { 0. } else { scene.cdf[k - 1] };
    let total: f32 = (scene.lights.iter())
        .map(|l| {
            let Light::Tri(l) = *l else {
                panic!("{l:?} is not a triangle")
            };
            let c = crate::lighting::emitted(&scene.materials[scene.bvh.tris[l].material as usize]);
            Scene::area(&scene.bvh.tris[l]) * (0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z).max(1e-6)
        })
        .sum();
    assert!(((scene.cdf[k] - before) * total / w - 1.).abs() < 1e-3);
    // Without the flag, the material's own colour.
    let plain = (scene.bvh.tris.iter())
        .position(|t| t.material != 5)
        .unwrap();
    let m = &scene.materials[scene.bvh.tris[plain].material as usize];
    assert_eq!(
        scene.emission_colour(m, plain, b),
        super::bsdf::rgb(m.color_rough)
    );
}

/// The test quad's map has one transparent texel (uv around (0.75, 0.75)) under MASK at 0.5:
/// every ray type passes through it and stops on the opaque ones. As BLEND with the factor's
/// alpha 0.8, an opaque texel stops 80% of rays, each ray deciding once.
#[test]
fn alpha_cutouts_and_stochastic_blend() {
    use super::scene::Ray;
    let dir = std::env::temp_dir().join(format!("caustica_ref_alpha_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("quad.gltf");
    crate::mesh::write_test_gltf(&path).unwrap();
    let quad = crate::mesh::import_objects(&path).unwrap().remove(0);
    let mut s = Settings::default();
    s.objects = vec![quad];
    let mut scene = super::scene::Scene::new(&s, [8, 8]);
    assert_ne!(super::bsdf::flags(&scene.materials[5]) & 8, 0);
    // The triangle and barycentrics at `uv`.
    let at = |scene: &super::scene::Scene, uv: Vec2| {
        (0..scene.bvh.tris.len())
            .filter(|&i| scene.bvh.tris[i].material == 5)
            .find_map(|i| {
                let t = &scene.bvh.tris[i];
                let (e1, e2, r) = (t.uv[1] - t.uv[0], t.uv[2] - t.uv[0], uv - t.uv[0]);
                let det = e1.perp_dot(e2);
                let b = Vec2::new(r.perp_dot(e2), e1.perp_dot(r)) / det;
                (b.min_element() >= 0. && b.x + b.y <= 1.).then_some((i, b))
            })
            .unwrap()
    };
    let (hole, solid) = (at(&scene, Vec2::splat(0.75)), at(&scene, Vec2::splat(0.25)));
    for kind in [Ray::Camera, Ray::Bounce, Ray::Shadow] {
        let accept = scene.accept(kind, Vec3::ZERO, Vec3::Z);
        assert!(!accept(5, hole.0, hole.1));
        assert!(accept(5, solid.0, solid.1));
    }
    scene.materials[5].maps[3] = (-0.8f32).to_bits();
    let n = 20000;
    let mut r = super::sampling::Rng::new(7, 0);
    let blocked = (0..n)
        .filter(|_| {
            let o = Vec3::new(r.f(), r.f(), r.f());
            let accept = scene.accept(Ray::Shadow, o, Vec3::Z);
            let first = accept(5, solid.0, solid.1);
            // The same ray decides the same way again.
            assert_eq!(first, accept(5, solid.0, solid.1));
            first
        })
        .count() as f64;
    let (p, sd) = (0.8, (0.8f64 * 0.2 / n as f64).sqrt());
    assert!((blocked / n as f64 - p).abs() < 4. * sd, "{blocked}");
    let accept = scene.accept(Ray::Shadow, Vec3::ZERO, Vec3::Z);
    assert!(!accept(5, hole.0, hole.1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thin_film_decodes_and_stays_in_range() {
    use super::bsdf::{film_decode, film_fresnel, fresnel, prepare_film, spec_fresnel};
    // lighting.rs packs thickness + 2048 × IOR in hundredths.
    assert_eq!(film_decode(400. + 2048. * 133.), (400., 1.33));
    assert_eq!(film_decode(0. + 2048. * 100.), (0., 1.));
    // No thickness: the film fades to the outside, leaving the base's Fresnel at normal
    // incidence (away from it, Schlick's r12 of an index-matched film is not 0, as on the GPU).
    let eta3 = Vec3::new(1.5, 2., 3.);
    // The Airy terms keep ~0.007 from r123's 1e-5 floor.
    {
        let c = 1f32;
        let f = film_fresnel(c, 1.33, eta3, 0.);
        let want = eta3.map(|e| {
            let f0 = ((e - 1.) / (e + 1.)).powi(2);
            f0 + (1. - f0) * (1. - c).powi(5)
        });
        assert!((f - want).abs().max_element() < 1e-2, "{f} vs {want}");
    }
    // Any film, any angle: a reflectance, and coloured at a soap-bubble thickness.
    for t in [50f32, 300., 700., 1500.] {
        for c in [0.05f32, 0.4, 0.9, 1.] {
            let f = film_fresnel(c, 1.33, Vec3::ONE, t);
            assert!(
                f.min_element() >= 0. && f.max_element() <= 1.,
                "{t} {c}: {f}"
            );
        }
    }
    let f = film_fresnel(1., 1.33, Vec3::ONE, 300.);
    assert!(f.max_element() - f.min_element() > 0.02, "no colour: {f}");
    // Strength blends the base Fresnel toward the film; 0 strength leaves it alone.
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.optical[0] = 1.5;
    m.surface_scale = [0., 0., 0.5, 300. + 2048. * 133.];
    prepare_film(&mut m, 0.8, 1., 0.);
    let f0 = Vec3::splat(0.04);
    let film = Vec3::new(m.base_scale[0], m.base_scale[1], m.base_scale[2]);
    let want = fresnel(0.8, f0).lerp(film, 0.5);
    assert!((spec_fresnel(&m, 0.8, f0) - want).abs().max_element() < 1e-6);
    m.surface_scale[2] = 0.;
    assert_eq!(spec_fresnel(&m, 0.8, f0), fresnel(0.8, f0));
}
/// Chromatic free flight is unbiased per channel: the weight of the flights that pass the
/// segment averages to its transmittance, and a scatter's weight to the probability of
/// scattering anywhere in it, 1 − T. The tint reads back as packed, and grey keeps w = 1.
#[test]
fn chromatic_flight_is_unbiased_per_channel() {
    use super::media::{flight, tint_of};
    let t = tint_of(crate::lighting::pack_tint([0.4, 0.7, 1.]));
    assert!(
        (t - Vec3::new(0.4, 0.7, 1.)).abs().max_element() < 3e-3,
        "{t}"
    );
    assert_eq!(tint_of(crate::lighting::pack_tint([2.; 3])), Vec3::ONE);
    let mut r = Rng::new(11, 5);
    let (_, w) = flight(Vec3::splat(0.5), 0., 1., &mut r);
    assert_eq!(w, Vec3::ONE);
    let sigma = Vec3::new(0.3, 1.2, 2.5);
    let (t0, t1) = (0.5f32, 2.);
    let n = 200_000;
    let (mut pass, mut pass2, mut scatter) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    for _ in 0..n {
        let (ts, w) = flight(sigma, t0, t1, &mut r);
        if ts >= t1 {
            pass += w;
            pass2 += w * w;
        } else {
            assert!(ts >= t0);
            scatter += w;
        }
    }
    let tr = (-sigma * (t1 - t0)).exp();
    let mean = pass / n as f32;
    let se = ((pass2 / n as f32 - mean * mean) / n as f32)
        .max(Vec3::splat(1e-12))
        .powf(0.5);
    assert!(
        ((mean - tr).abs() / se).max_element() < 4.,
        "{mean} {tr} se {se}"
    );
    let s = scatter / n as f32;
    assert!(
        (s - (Vec3::ONE - tr)).abs().max_element() < 0.01,
        "{s} {tr}"
    );
}

#[test]
fn heterogeneous_fog_tracking_matches_the_integrated_density() {
    use super::media::{delta_tracking, fog_density, ratio_tracking};
    let (noise, size) = (0.75, 0.3);
    // The density is non-negative, below the majorant's 1 + noise, and about 1 on average.
    let mut r = Rng::new(3, 9);
    let mut sum = 0.;
    for _ in 0..20_000 {
        let p = Vec3::new(r.f(), r.f(), r.f()) * 20. - 10.;
        let k = fog_density(p, noise, size);
        assert!((0. ..=1. + noise).contains(&k), "{k}");
        sum += k;
    }
    assert!((sum / 20_000. - 1.).abs() < 0.05, "{}", sum / 20_000.);
    // Along a fixed ray, both estimators against exp(-∫σ·k) in 4096 steps.
    let sigma = Vec3::new(0.4, 0.7, 1.) * 1.3;
    let (o, d) = (
        Vec3::new(-1., 0.2, 0.3),
        Vec3::new(0.8, 0.1, -0.3).normalize(),
    );
    let (t0, t1) = (0.2f32, 2.5);
    let steps = 4096;
    let h = (t1 - t0) / steps as f32;
    let optical: f32 = (0..steps)
        .map(|i| fog_density(o + d * (t0 + (i as f32 + 0.5) * h), noise, size) * h)
        .sum();
    let tr = (-sigma * optical).exp();
    let n = 200_000;
    let (mut ratio, mut ratio2, mut pass, mut pass2, mut scatter) =
        (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    for _ in 0..n {
        let x = ratio_tracking(sigma, noise, size, o, d, t0, t1, &mut r);
        ratio += x;
        ratio2 += x * x;
        let (ts, w) = delta_tracking(sigma, noise, size, o, d, t0, t1, &mut r);
        if ts >= t1 {
            pass += w;
            pass2 += w * w;
        } else {
            assert!(ts >= t0);
            scatter += w;
        }
    }
    let check = |s: Vec3, s2: Vec3| {
        let mean = s / n as f32;
        let se = ((s2 / n as f32 - mean * mean) / n as f32)
            .max(Vec3::splat(1e-12))
            .powf(0.5);
        assert!(
            ((mean - tr).abs() / se).max_element() < 4.,
            "{mean} {tr} se {se}"
        );
    };
    check(ratio, ratio2);
    check(pass, pass2);
    let s = scatter / n as f32;
    assert!(
        (s - (Vec3::ONE - tr)).abs().max_element() < 0.01,
        "{s} {tr}"
    );
}

/// An object's extinction is its density times its tint, grey where phase.w is not a tint.
#[test]
fn an_objects_extinction_reads_its_tint() {
    let mut m: PackedMaterial = bytemuck::Zeroable::zeroed();
    m.medium[3] = 2.;
    assert_eq!(super::media::extinction(&m), Vec3::splat(2.));
    m.phase[3] = crate::lighting::pack_tint([1., 0.5, 0.]);
    let e = super::media::extinction(&m);
    assert!(
        (e - Vec3::new(2., 2. * 128. / 255., 0.))
            .abs()
            .max_element()
            < 1e-6,
        "{e}"
    );
    // A blackbody's or the measured light's phase.w is a scale.
    for spectrum in [-3000., 4.] {
        m.phase[2] = spectrum;
        assert_eq!(super::media::extinction(&m), Vec3::splat(2.));
    }
}
/// A shadow ray inside a scattering object loses its extinction and its colour's absorption up
/// to the first surface; inside clear glass, and outside without fog, nothing.
#[test]
fn shadows_through_an_objects_medium() {
    let mut s = Settings::default();
    let m = &mut s.objects[0].material;
    m.transmission = 1.;
    m.color = [1., 0.8, 0.6];
    m.density = 3.;
    m.extinction = [1., 0.5, 0.25];
    assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
    let scene = Scene::new(&s, [32, 24]);
    let own: Vec<_> = scene.bvh.tris.iter().filter(|t| t.material == 5).collect();
    let centre = own.iter().map(|t| t.p[0]).sum::<Vec3>() / own.len() as f32;
    let entry = (scene.materials[5].lobes[3] as u32 >> 8 & 7, 5);
    let mut r = Rng::new(1, 1);
    let d = Vec3::new(0.3, 1., 0.2).normalize();
    let exit = scene
        .intersect(centre, d, 10., &|_, _, _| true, false)
        .unwrap()
        .t;
    let sigma = 3. * Vec3::new(1., 128. / 255., 64. / 255.);
    let want = (-sigma * exit).exp() * Vec3::new(1., 0.8, 0.6).powf(exit);
    let got = scene.shadow_transmittance(&[entry], centre, d, 10., &mut r);
    assert!((got - want).abs().max_element() < 1e-4, "{got} {want}");
    assert_eq!(
        scene.shadow_transmittance(&[], centre, d, 10., &mut r),
        Vec3::ONE
    );
    s.objects[0].material.density = 0.;
    let clear = Scene::new(&s, [32, 24]);
    assert_eq!(
        clear.shadow_transmittance(&[entry], centre, d, 10., &mut r),
        Vec3::ONE
    );
}
/// A scattering object renders, and a denser one lets less of the room through.
#[test]
fn a_scattering_object_renders() {
    let mean = |density: f32| {
        let mut s = Settings::default();
        s.bounces = 6;
        let m = &mut s.objects[0].material;
        m.transmission = 1.;
        m.ior = 1.;
        m.density = density;
        m.scatter_color = [0.2, 0.2, 0.2];
        let img = render(&s, [32, 24], 8, &|_| {}).unwrap();
        assert!(img.iter().all(|v| v.is_finite()));
        img.chunks_exact(4).map(|p| p[0] + p[1] + p[2]).sum::<f32>()
    };
    let (thin, dense) = (mean(0.01), mean(30.));
    assert!(dense < thin * 0.98, "{dense} {thin}");
}
/// A camera inside a dielectric starts in its medium: a tinted box around the eye darkens the
/// image, and a scattering one renders finite and darker still.
#[test]
fn the_camera_starts_in_the_medium_around_it() {
    let base = Settings::default();
    let own: Vec<_> = Scene::new(&base, [8, 6])
        .bvh
        .tris
        .iter()
        .filter(|t| t.material == 5)
        .flat_map(|t| t.p)
        .collect();
    let centre = own.iter().sum::<Vec3>() / own.len() as f32;
    let mean = |color: [f32; 3], density: f32| {
        let mut s = base.clone();
        s.target = centre.into();
        s.distance = 0.02;
        let m = &mut s.objects[0].material;
        m.transmission = 1.;
        m.ior = 1.;
        m.color = color;
        m.density = density;
        assert!(unsupported(&s).is_empty(), "{:?}", unsupported(&s));
        assert_eq!(Scene::new(&s, [8, 6]).camera_media, vec![(0, 5)]);
        let img = render(&s, [32, 24], 8, &|_| {}).unwrap();
        assert!(img.iter().all(|v| v.is_finite()));
        img.chunks_exact(4).map(|p| p[0] + p[1] + p[2]).sum::<f32>()
    };
    let clear = mean([1.; 3], 0.);
    let tinted = mean([0.2; 3], 0.);
    let foggy = mean([0.2; 3], 20.);
    assert!(tinted < clear * 0.9, "{tinted} {clear}");
    assert!(foggy < tinted, "{foggy} {tinted}");
}

/// A stretched, turned and moved exact sphere is an ellipsoid: the hit and the normal against
/// the ellipsoid's own equation, from outside and from inside (a refraction's exit).
#[test]
fn an_exact_sphere_under_a_transform_is_an_ellipsoid() {
    use super::sdf::{SPHERE, SdfObject};
    let (scale, turn, at) = (
        Vec3::new(2., 0.5, 1.),
        glam::Quat::from_rotation_z(0.6) * glam::Quat::from_rotation_x(0.3),
        Vec3::new(0.3, -0.2, 0.1),
    );
    let o2w = Mat4::from_scale_rotation_translation(scale, turn, at);
    let sdf = SdfObject::new(SPHERE, 5, o2w);
    // Points x with |S⁻¹ R⁻¹ (x − at)| = 1; the gradient of that, for the normal.
    let local = |x: Vec3| (turn.inverse() * (x - at)) / scale;
    let grad = |x: Vec3| (turn * (local(x) / scale)).normalize();
    for (o, d) in [
        (Vec3::new(-5., 0.3, 0.2), Vec3::new(1., 0.05, -0.02)),
        (Vec3::new(0.2, 4., -1.), Vec3::new(0.1, -1., 0.3)),
        (at, Vec3::new(0.3, 0.8, -0.5)),
    ] {
        let d = d.normalize();
        let t = sdf.hit(o, d, 1e-4, 1e4).expect("a hit");
        let x = o + d * t;
        assert!((local(x).length() - 1.).abs() < 1e-4, "on the surface");
        // No nearer crossing: the ray's points before t are on one side.
        let before = local(o + d * (t * 0.5)).length() - 1.;
        assert!(before * (local(o).length() - 1.) > 0.);
        let n = sdf.normal(o, d, t);
        assert!(n.dot(grad(x)) > 0.9999, "{n} vs {}", grad(x));
    }
    assert!(
        sdf.hit(Vec3::new(-5., 3., 0.), Vec3::X, 1e-4, 1e4)
            .is_none()
    );
}

/// The closed-form cylinder and cone agree with sphere tracing their exact distance fields, and
/// their normals with the fields' gradients away from the rims.
#[test]
fn the_exact_cylinder_and_cone_agree_with_their_fields() {
    use super::sdf::{CONE, CYLINDER, field, hit, normal};
    let mut r = super::sampling::Rng::new(3, 0);
    for kind in [CYLINDER, CONE] {
        // The same shape, but marched (the generic path keys on kinds it doesn't solve).
        let marched = |o: Vec3, d: Vec3| {
            let mut s = 1e-4f32;
            for _ in 0..4000 {
                let h = field(kind, o + d * s).abs();
                if h < 1e-5 {
                    return Some(s);
                }
                s += h;
                if s > 20. {
                    break;
                }
            }
            None
        };
        let mut hits = 0;
        for _ in 0..2000 {
            let o = Vec3::new(r.f() - 0.5, r.f() - 0.5, r.f() - 0.5) * 6.;
            let d = (Vec3::new(r.f() - 0.5, r.f() - 0.5, r.f() - 0.5) * 2. - o * 0.2)
                .normalize_or(Vec3::X);
            if field(kind, o).abs() < 0.01 {
                continue;
            }
            let exact = hit(kind, o, d, 1e-4, 1e4);
            let traced = marched(o, d);
            match (exact, traced) {
                (Some(a), Some(b)) => {
                    assert!(
                        (a - b).abs() < 1e-3,
                        "kind {kind} at {o} along {d}: {a} vs {b}"
                    );
                    let p = o + d * a;
                    let rim = (p.y.abs() - 1.).abs() < 0.01
                        || (p.x * p.x + p.z * p.z).sqrt() > 0.98 && p.y < -0.98;
                    if !rim {
                        let e = 1e-3;
                        let g = Vec3::new(
                            field(kind, p + Vec3::X * e) - field(kind, p - Vec3::X * e),
                            field(kind, p + Vec3::Y * e) - field(kind, p - Vec3::Y * e),
                            field(kind, p + Vec3::Z * e) - field(kind, p - Vec3::Z * e),
                        );
                        let n = normal(kind, p).normalize();
                        assert!(
                            n.dot(g.normalize()) > 0.99,
                            "kind {kind} at {p}: {n} vs {g}"
                        );
                    }
                    hits += 1;
                }
                (None, None) => {}
                // Grazing rays can split the two by a hair.
                (a, b) => assert!(
                    a.or(b).is_some_and(|t| field(kind, o + d * t).abs() < 1e-3
                        && (o + d * t).y.abs() > 0.99
                        || glam::Vec3Swizzles::xz(o + d * t).length() > 0.97),
                    "kind {kind} at {o} along {d}: {a:?} vs {b:?}"
                ),
            }
        }
        assert!(hits > 100, "kind {kind}: {hits} hits");
    }
}

/// From inside an exact shape the ray finds the exit, and a scene's rays see procedural objects
/// with the same visibility rules as triangles.
#[test]
fn procedural_objects_are_hit_with_the_triangles_rules() {
    use crate::scene::Sdf;
    for sdf in [Sdf::Sphere, Sdf::Cylinder, Sdf::Cone] {
        let t = super::sdf::hit(sdf.id(0., 0.), Vec3::new(0., -0.5, 0.), Vec3::X, 1e-4, 1e4)
            .expect("the exit");
        let want = match sdf {
            Sdf::Sphere => 0.75f32.sqrt(),
            Sdf::Cylinder => 1.,
            _ => 0.75,
        };
        assert!((t - want).abs() < 1e-5, "{sdf:?}: {t}");
    }
    let mut s = Settings::default();
    s.objects.clear();
    s.objects.push(crate::scene::Object {
        sdf: Sdf::Sphere,
        ..crate::scene::Object::default()
    });
    let scene = Scene::new(&s, [8, 6]);
    assert_eq!(scene.sdfs.len(), 1);
    let o = scene.sdfs[0].w2o.inverse().transform_point3(Vec3::ZERO) + Vec3::new(0., 0., 20.);
    let h = scene
        .intersect(
            o,
            Vec3::NEG_Z,
            1e4,
            &scene.accept(Ray::Camera, o, Vec3::NEG_Z),
            false,
        )
        .expect("the sphere");
    assert_eq!((h.sdf, h.material), (Some(0), 5));
    let sf = scene.surface(&h, o, Vec3::NEG_Z);
    assert!(sf.front && sf.n.dot(Vec3::Z) > 0.999 && sf.ng == sf.n);
    // Clear glass: shadow rays pass it unless glass casts shadows, other rays stop.
    s.objects[0].material.transmission = 1.;
    for glass_shadow in [false, true] {
        s.glass_shadow = glass_shadow;
        let scene = Scene::new(&s, [8, 6]);
        let shadow = scene.intersect(
            o,
            Vec3::NEG_Z,
            1e4,
            &scene.accept(Ray::Shadow, o, Vec3::NEG_Z),
            false,
        );
        assert_eq!(shadow.and_then(|h| h.sdf).is_some(), glass_shadow);
        let bounce = scene.intersect(
            o,
            Vec3::NEG_Z,
            1e4,
            &scene.accept(Ray::Bounce, o, Vec3::NEG_Z),
            false,
        );
        assert_eq!(bounce.and_then(|h| h.sdf), Some(0));
    }
}

/// The torus, Mandelbulb, Menger sponge and metaballs fields, at points whose distance or sign
/// is known, and marched by a scene ray.
#[test]
fn procedural_fields_match_their_shapes() {
    use super::sdf::{MANDELBULB, MENGER, METABALLS, TORUS, field, hit};
    use crate::scene::Sdf;
    use glam::Vec3Swizzles;
    let id = |sdf: Sdf, detail: f32| sdf.id(0., detail);
    // Torus: exact distance to the tube of radius r about the ring of radius 1 − r.
    let mut r = super::sampling::Rng::new(5, 0);
    for detail in [0., 0.28, 0.1, 0.5] {
        let tube = if detail > 0. { detail } else { 0.28 };
        for _ in 0..200 {
            let p = Vec3::new(r.f() - 0.5, r.f() - 0.5, r.f() - 0.5) * 3.;
            let rho = (p.x * p.x + p.z * p.z).sqrt();
            let want = ((rho - (1. - tube)).powi(2) + p.y * p.y).sqrt() - tube;
            let got = field(id(Sdf::Torus, detail), p);
            assert!((got - want).abs() < 1e-5, "torus {detail} at {p}");
        }
    }
    assert_eq!(id(Sdf::Torus, 0.) & 255, TORUS);
    // Detail 0 is each kind's default.
    for (sdf, default) in [(Sdf::Mandelbulb, 8.), (Sdf::Menger, 5.)] {
        let p = Vec3::new(0.3, -0.4, 0.55);
        assert_eq!(field(id(sdf, 0.), p), field(id(sdf, default), p));
    }
    // Menger, one level: the cube with its centre crosses removed. The centre lies a third
    // inside a removed tunnel's wall, a face centre is open, a corner cube is solid.
    let m1 = id(Sdf::Menger, 1.);
    assert_eq!(m1 & 255, MENGER);
    assert!((field(m1, Vec3::ZERO) - 1. / 3.).abs() < 1e-6);
    assert!(field(m1, Vec3::new(0., 0., 0.9)) > 0.);
    assert!((field(m1, Vec3::splat(2. / 3.)) + 1. / 3.).abs() < 1e-6);
    assert!((field(m1, Vec3::new(2., 0.8, 0.8)) - 1.).abs() < 1e-6);
    // Deeper levels only remove material.
    for _ in 0..200 {
        let p = Vec3::new(r.f(), r.f(), r.f()) * 2. - 1.;
        assert!(field(id(Sdf::Menger, 3.), p) >= field(m1, p) - 1e-6);
    }
    // Mandelbulb: power 8's polynomial form and power 8.001's spherical one find the same
    // surface along rays from outside (shape checks, about 1%), once x and z swap: the
    // polynomial form measures its azimuth from z (Quilez's atan(x, z)), the spherical from x.
    let poly = id(Sdf::Mandelbulb, 8.);
    let polar = id(Sdf::Mandelbulb, 8.001);
    assert_eq!(poly & 255, MANDELBULB);
    assert!(field(poly, Vec3::new(0.1, 0.05, 0.12)) < 0. && field(poly, Vec3::splat(0.95)) > 0.);
    let mut both = 0;
    for _ in 0..200 {
        let d = (Vec3::new(r.f(), r.f(), r.f()) * 2. - 1.).normalize_or(Vec3::X);
        let o = -d * 3.;
        if let (Some(a), Some(b)) = (
            hit(poly, o, d, 1e-4, 1e4),
            hit(polar, o.zyx(), d.zyx(), 1e-4, 1e4),
        ) && (a - b).abs() < 0.01 * a
        {
            both += 1;
        }
    }
    assert!(both > 180, "{both} of 200 rays agree");
    // Metaballs: inside the bottom pool, outside at the cube's corner; the phase moves blobs.
    let wax = Sdf::Metaballs.id(0., 0.);
    assert_eq!(wax & 255, METABALLS);
    assert!(field(wax, Vec3::new(0., -1.1, 0.)) < 0.);
    assert!(field(wax, Vec3::ONE) > 0.);
    let moved = Sdf::Metaballs.id(3.7, 0.);
    assert!((0..50).any(|i| {
        let p = Vec3::new(0., i as f32 / 25. - 1., 0.);
        (field(wax, p) - field(moved, p)).abs() > 0.01
    }));
    // A torus seen from above along its tube's centre line.
    let t = hit(
        id(Sdf::Torus, 0.),
        Vec3::new(0.72, 3., 0.),
        Vec3::NEG_Y,
        1e-4,
        1e4,
    );
    assert!(t.is_some_and(|t| (t - 2.72).abs() < 2e-4), "{t:?}");
}

/// Light sampling of an emissive procedural object leaves the mean where hits alone put it and
/// cuts the noise: the floor under an emissive torus, direct light only, beside the room's
/// area light (so the two share the samples).
#[test]
fn sdf_emitter_light_sampling() {
    use crate::scene::Sdf;
    let mut s = Settings::default();
    s.objects.clear();
    s.objects.push(crate::scene::Object {
        sdf: Sdf::Torus,
        ..crate::scene::Object::default()
    });
    s.objects[0].material.emission = 8.;
    s.bounces = 2;
    let mut scene = Scene::new(&s, [8, 6]);
    assert_eq!(scene.sdf_lights.len(), 1);
    assert_eq!(scene.sdf_lights[0].cdf, 1.);
    assert_eq!(
        scene.sdf_share,
        if scene.lights.is_empty() { 1. } else { 0.5 }
    );
    let l = &scene.sdf_lights[0];
    // Beside the torus, looking down at the floor it lights.
    let o = l.centre + Vec3::new(l.radius * 0.8, 0., l.radius * 0.3);
    let d = Vec3::new(0.2, -1., 0.).normalize();
    assert!(
        scene
            .intersect(o, d, 1e4, &scene.accept(Ray::Camera, o, d), false)
            .is_some_and(|h| h.sdf.is_none()),
        "the fixture sees the floor"
    );
    let run = |scene: &Scene| {
        let n = 20000;
        let (mut sum, mut sq) = (0f64, 0f64);
        for i in 0..n {
            let mut r = Rng::new(11, i);
            let v = scene
                .radiance(o, d, Vec3::ONE, &mut r)
                .dot(Vec3::splat(1. / 3.)) as f64;
            sum += v;
            sq += v * v;
        }
        let mean = sum / n as f64;
        (mean, (sq / n as f64 - mean * mean).max(0.) / n as f64)
    };
    let (with, var_with) = run(&scene);
    let sdf_share = scene.sdf_share;
    let off = std::mem::take(&mut scene.sdf_lights);
    scene.sdf_share = 0.;
    let (hits, var_hits) = run(&scene);
    let z = (with - hits) / (var_with + var_hits).sqrt();
    assert!(with > 0. && z.abs() < 4., "{with} vs {hits}: z {z}");
    assert!(var_with < var_hits, "{var_with} vs {var_hits}");
    // The hit side's weight agrees with the sample's pdf.
    scene.sdf_lights = off;
    scene.sdf_share = sdf_share;
    assert!(scene.sdf_light_pdf(0, o) > 0. && scene.sdf_light_pdf(1, o) == 0.);
}
#[test]
fn the_denoisers_inputs_leave_the_sums_alone() {
    let mut s = Settings::default();
    s.bounces = 3;
    let stop = AtomicBool::new(false);
    let mut p = Progressive::new(&s, [24, 16]).unwrap();
    p.keep_guides();
    assert!(p.pass(4, 2, &stop, &|_| {}));
    assert_eq!(p.image(), render(&s, [24, 16], 4, &|_| {}).unwrap());
    let d = p.denoised(0.5, 2);
    assert_eq!(d.len(), p.image().len());
    assert!(d.iter().all(|v| v.is_finite() && *v >= 0.));
    // Filtering moves the pixels but not the overall level much.
    let mean = |i: &[f32]| i.iter().sum::<f32>() / i.len() as f32;
    assert!(d != p.image());
    assert!((mean(&d) / mean(&p.image()) - 1.).abs() < 0.1);
    // Without guides it is the plain mean.
    p.drop_guides();
    assert_eq!(p.denoised(0.5, 2), p.image());
}
