//! The walk avatar: a blocky body under the first-person camera (the owner's "I want to see my
//! legs and my reflection"), ten boxes added to each frame's copy of the settings the way the
//! flashlight and the laser's beam are, so it stays out of the scene file, undo, auto-key,
//! picking and physics. Our own design: a cube head, a box torso, box arms with hands, and legs
//! with knees.
//!
//! The pose is a pure function of the walker's state (`Stance`): the stride phase comes from the
//! distance walked, never the clock, so a standing walker draws the same boxes every frame and
//! the accumulation is not restarted. The legs follow a foot path through two-bone IK (so a
//! crouch bends the knees and leans the torso), the arms swing against the legs, the body yaws
//! with the view and the head pitches with it. The head alone is hidden from camera rays
//! (`Object::camera_hidden`); reflections, refractions and shadows see it.
//!
//! Local frame of the body: x right, y up from the feet, −z forward, origin under the eye. The
//! body is shifted `BACK` behind the eye so that nothing but the head is in front of it.
use crate::scene::{Body, MAX_OBJECTS, Material, Object, Settings, Shape};
use crate::walk::{CROUCH_EYE, EYE};
use glam::{EulerRot, Quat, Vec3};

/// A body unit is a 28th of the eye height: the head is 8 wide, the torso 8 × 12 × 4, a leg 12
/// long (two segments of 6), an arm 12 (a sleeve of 8 and a hand of 4).
const UNITS_TO_EYE: f32 = 28.;
/// The body hangs this many units behind the eye.
const BACK: f32 = 2.5;
/// The legs hang this far in front of the torso's middle, so that looking down shows them past
/// the chest.
const PELVIS: f32 = 2.;
/// Arm swing, in radians, and the foot's reach and lift in units at full stride.
const ARM_SWING: f32 = 0.6;
const FOOT_REACH: f32 = 4.;
const FOOT_LIFT: f32 = 2.5;
/// Boxes in the avatar (what `parts` returns).
pub const PARTS: usize = 10;
/// The name prefix of the injected boxes (`is_part`).
const PREFIX: &str = "avatar: ";

/// Whether `o` is one of the boxes `add` injected into a frame's copy of the settings.
pub fn is_part(o: &Object) -> bool {
    o.name.starts_with(PREFIX)
}

/// Everything the pose depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stance {
    /// Where the feet stand (the point under the eye).
    pub feet: Vec3,
    /// The eye's height above the feet, body scale and crouch included.
    pub eye_height: f32,
    /// The view's yaw and pitch in degrees (`Settings::yaw`, `pitch`; pitch > 0 looks down).
    pub yaw: f32,
    pub pitch: f32,
    /// Stride phase in radians: π per step, advanced by the distance walked.
    pub phase: f32,
    /// How much of the stride shows, 0 (standing) to 1 (walking).
    pub gait: f32,
    /// Body scale (walk mode's mouse wheel).
    pub scale: f32,
    /// 0 standing, 1 crouched.
    pub crouch: f32,
}

/// The pose of a walker standing still at the settings' camera, feet an eye height below it:
/// what a walker at rest on flat ground has, and what the CPU reference and the headless debug
/// switch (`--walk`) draw.
pub fn standing(s: &Settings) -> Stance {
    let (_, _, eye) = s.camera(1.);
    Stance {
        feet: eye - Vec3::Y * EYE,
        eye_height: EYE,
        yaw: s.yaw,
        pitch: s.pitch,
        phase: 0.,
        gait: 0.,
        scale: 1.,
        crouch: 0.,
    }
}

/// One box of the body in the world.
#[derive(Clone, Debug)]
pub struct Part {
    pub name: &'static str,
    pub centre: Vec3,
    /// Half extents along the box's own axes.
    pub half: Vec3,
    pub rotation: Quat,
    pub color: [f32; 3],
    /// Invisible to camera rays (the head).
    pub hidden: bool,
}
impl Part {
    /// The eight corners in the world.
    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let sign = |bit: usize| if i >> bit & 1 == 1 { 1. } else { -1. };
            self.centre + self.rotation * (self.half * Vec3::new(sign(0), sign(1), sign(2)))
        })
    }
    /// Whether `p` is inside the box grown by `margin`.
    pub fn contains(&self, p: Vec3, margin: f32) -> bool {
        let local = self.rotation.inverse() * (p - self.centre);
        local.abs().cmple(self.half + Vec3::splat(margin)).all()
    }
}

const SHIRT: [f32; 3] = [0.08, 0.38, 0.42];
const TROUSERS: [f32; 3] = [0.1, 0.11, 0.2];
const SKIN: [f32; 3] = [0.72, 0.5, 0.38];

/// A limb angle `g` (radians from straight down, forward positive) as the rotation that turns a
/// box's long y axis along the limb in the body's frame.
fn about_x(g: f32) -> Quat {
    Quat::from_rotation_x(g)
}

/// The ten boxes of the body for `st`, in world space. Pure: same stance, same boxes.
pub fn parts(st: &Stance) -> Vec<Part> {
    let u = EYE * st.scale / UNITS_TO_EYE;
    let heading = Quat::from_rotation_y(st.yaw.to_radians());
    let at = |p: Vec3| st.feet + heading * (p * u);
    let part = |name, c: Vec3, half: Vec3, local: Quat, color, hidden| Part {
        name,
        centre: at(c),
        half: half * u,
        rotation: heading * local,
        color,
        hidden,
    };
    let eye_h = st.eye_height / u;
    // The torso hangs from just under the head; its lean follows the hip height of the crouch.
    let neck = Vec3::new(0., eye_h - 4., BACK);
    let hip_height = 12. - 6. * st.crouch;
    let lean = (((neck.y - hip_height) / 12.).clamp(0., 1.)).acos();
    let down = Vec3::new(0., -lean.cos(), lean.sin());
    let hip = neck + down * 12.;
    let mut out = Vec::with_capacity(PARTS);
    // The head turns about the neck with the view's pitch.
    let nod = about_x(-st.pitch.clamp(-70., 70.).to_radians());
    out.push(part(
        "Avatar head",
        // On the eye, forward of the torso.
        neck + nod * Vec3::new(0., 4., 0.) - Vec3::new(0., 0., BACK),
        Vec3::splat(4.),
        nod,
        SKIN,
        true,
    ));
    out.push(part(
        "Avatar torso",
        (neck + hip) * 0.5,
        Vec3::new(4., 6., 2.),
        about_x(-lean),
        SHIRT,
        false,
    ));
    let phase = st.phase.rem_euclid(std::f32::consts::TAU);
    for side in [-1f32, 1.] {
        // The leg that is forward is on the side opposite the arm that is.
        let ph = phase + if side > 0. { 0. } else { std::f32::consts::PI };
        let foot = Vec3::new(
            0.,
            FOOT_LIFT * st.gait * ph.cos().max(0.),
            BACK - PELVIS - FOOT_REACH * st.gait * ph.sin(),
        );
        let hip_joint = hip + Vec3::new(side * 2., 0., -PELVIS);
        let (thigh, shin, knee, ankle) = leg(hip_joint, foot + Vec3::new(side * 2., 0., 0.));
        out.push(part(
            if side > 0. {
                "Avatar right thigh"
            } else {
                "Avatar left thigh"
            },
            (hip_joint + knee) * 0.5,
            Vec3::new(2., 3., 2.),
            about_x(thigh),
            TROUSERS,
            false,
        ));
        out.push(part(
            if side > 0. {
                "Avatar right shin"
            } else {
                "Avatar left shin"
            },
            (knee + ankle) * 0.5,
            Vec3::new(2., 3., 2.),
            about_x(shin),
            TROUSERS,
            false,
        ));
        // The arm swings against the leg of its side.
        let swing = -ARM_SWING * st.gait * ph.sin() + 0.3 * lean;
        let shoulder = neck + down * 2. + Vec3::new(side * 6., 0., 0.);
        let along = Vec3::new(0., -swing.cos(), -swing.sin());
        out.push(part(
            if side > 0. {
                "Avatar right arm"
            } else {
                "Avatar left arm"
            },
            shoulder + along * 2.,
            Vec3::new(2., 4., 2.),
            about_x(swing),
            SHIRT,
            false,
        ));
        out.push(part(
            if side > 0. {
                "Avatar right hand"
            } else {
                "Avatar left hand"
            },
            shoulder + along * 8.,
            Vec3::new(2., 2., 2.),
            about_x(swing),
            SKIN,
            false,
        ));
    }
    out
}

/// Two bones of 6 from `hip` to `foot` in the plane of the body's forward axis: the angles of
/// the thigh and the shin (radians from straight down, forward positive) and the knee. The knee
/// bends forward; a foot out of reach gets a straight leg (and the ankle stops short of it).
fn leg(hip: Vec3, foot: Vec3) -> (f32, f32, Vec3, Vec3) {
    const L: f32 = 6.;
    let forward = hip.z - foot.z;
    let below = hip.y - foot.y;
    let d = forward.hypot(below).clamp(1e-3, 2. * L * 0.9999);
    let aim = forward.atan2(below);
    let bend = (d / (2. * L)).acos();
    let (thigh, shin) = (aim + bend, aim - bend);
    let knee = hip + Vec3::new(0., -thigh.cos(), -thigh.sin()) * L;
    let ankle = knee + Vec3::new(0., -shin.cos(), -shin.sin()) * L;
    (thigh, shin, knee, ankle)
}

/// A box part as a scene object: the transform is the object's size, stretch, per-axis scale,
/// yaw and tilts (a unit box is 0.35 · size · scale on a side; `geometry::scale_rotation`).
fn object(p: &Part) -> Object {
    let size = (p.half.y / 0.35).max(0.05);
    let (yaw, x, z) = p.rotation.to_euler(EulerRot::YXZ);
    Object {
        name: format!("{PREFIX}{}", p.name),
        shape: Shape::Box,
        size,
        stretch: (p.half.y / (0.35 * size)).clamp(0.05, 10.),
        scale_xz: [
            (p.half.x / (0.35 * size)).clamp(0.01, 20.),
            (p.half.z / (0.35 * size)).clamp(0.01, 20.),
        ],
        position: [p.centre.x, p.centre.z],
        elevation: p.centre.y,
        grounded: false,
        rotation: yaw.to_degrees(),
        tilt: [x.to_degrees(), z.to_degrees()],
        body: Body::None,
        camera_hidden: p.hidden,
        // The head sits on the torso the camera looks down on: its shadow would black the chest out.
        casts_shadows: !p.hidden,
        locked: true,
        material: Material {
            color: p.color,
            roughness: 0.8,
            reflectivity: 0.04,
            ..Material::default()
        },
        ..Object::default()
    }
}

/// The body as objects for a frame. A part that holds `light` (the flashlight's position, when
/// it is on) is left out, or every shadow ray to that light would end inside it (the default
/// flashlight hangs 0.3 beside the eye, outside the body; a config that moves it in would not).
pub fn objects(st: &Stance, light: Option<Vec3>) -> Vec<Object> {
    parts(st)
        .iter()
        .filter(|p| light.is_none_or(|l| !p.contains(l, 0.02)))
        .map(object)
        .collect()
}

/// Adds the avatar to a frame's copy of the settings (room permitting).
pub fn add(frame: &mut Settings, st: &Stance) {
    if !frame.walk_avatar {
        return;
    }
    if frame.objects.len() + PARTS > MAX_OBJECTS {
        // All or nothing (a half body is worse than none); said once per run.
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "Walk avatar dropped: {} objects leave no room for its {PARTS} boxes (limit {MAX_OBJECTS})",
                frame.objects.len()
            );
        }
        return;
    }
    let light = frame
        .flashlight
        .then(|| Vec3::from(frame.flashlight_light().position));
    frame.objects.extend(objects(st, light));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn walking(phase: f32, gait: f32) -> Stance {
        Stance {
            feet: Vec3::new(1., 0., 2.),
            eye_height: EYE,
            yaw: 30.,
            pitch: 0.,
            phase,
            gait,
            scale: 1.,
            crouch: 0.,
        }
    }
    fn eye(st: &Stance) -> Vec3 {
        st.feet + Vec3::Y * st.eye_height
    }
    #[test]
    fn the_pose_is_a_pure_function_of_the_stance() {
        let a = parts(&walking(1.3, 0.8));
        let b = parts(&walking(1.3, 0.8));
        assert_eq!(a.len(), PARTS);
        for (p, q) in a.iter().zip(&b) {
            assert_eq!((p.centre, p.rotation), (q.centre, q.rotation));
        }
    }
    #[test]
    fn a_standing_body_ignores_the_stride_phase() {
        // Standing still must not move a box, whatever distance was walked before.
        let a = parts(&walking(0., 0.));
        let b = parts(&walking(2.7, 0.));
        for (p, q) in a.iter().zip(&b) {
            assert!((p.centre - q.centre).length() < 1e-6, "{}", p.name);
            assert!(p.rotation.dot(q.rotation).abs() > 1. - 1e-6, "{}", p.name);
        }
        // And a whole stride later the walking pose is the same again.
        let a = parts(&walking(0.9, 1.));
        let b = parts(&walking(0.9 + std::f32::consts::TAU, 1.));
        for (p, q) in a.iter().zip(&b) {
            assert!((p.centre - q.centre).length() < 1e-4, "{}", p.name);
        }
    }
    #[test]
    fn only_the_head_is_hidden_from_the_camera() {
        let hidden: Vec<_> = parts(&walking(0., 0.))
            .into_iter()
            .filter(|p| p.hidden)
            .map(|p| p.name)
            .collect();
        assert_eq!(hidden, ["Avatar head"]);
    }
    #[test]
    fn the_legs_stay_within_the_body_width() {
        let u = EYE / UNITS_TO_EYE;
        for i in 0..64 {
            let mut st = walking(i as f32 * 0.1, 1.);
            st.yaw = 0.;
            for p in parts(&st)
                .iter()
                .filter(|p| p.name.contains("thigh") || p.name.contains("shin"))
            {
                for c in p.corners() {
                    let x = (c - st.feet).x;
                    assert!(x.abs() <= 4. * u + 1e-5, "{} at {x}", p.name);
                }
            }
        }
    }
    #[test]
    fn feet_stand_on_the_floor_and_lift_in_the_swing() {
        let u = EYE / UNITS_TO_EYE;
        for i in 0..64 {
            let st = walking(i as f32 * 0.1, 1.);
            let low = parts(&st)
                .iter()
                .filter(|p| p.name.contains("shin"))
                .flat_map(|p| p.corners())
                .map(|c| c.y - st.feet.y)
                .fold(f32::INFINITY, f32::min);
            // The lowest corner is the planted foot: on the floor, never through it by more than
            // the box's tilt allows (a shin tilted by the stride dips its far corner a little).
            assert!(low > -1.2 * u && low < 1. * u, "phase {i}: {low}");
        }
        // Standing: both shins end exactly at the floor.
        let st = walking(0., 0.);
        let low = parts(&st)
            .iter()
            .filter(|p| p.name.contains("shin"))
            .flat_map(|p| p.corners())
            .map(|c| c.y - st.feet.y)
            .fold(f32::INFINITY, f32::min);
        assert!(low.abs() < 1e-3, "{low}");
    }
    #[test]
    fn nothing_visible_is_in_front_of_the_eye() {
        // Looking level: no corner of a camera-visible box inside the 45° cone round the view
        // (a 60° vertical, 16:9 view is 46° wide at its edge), whatever the stride or crouch.
        let cone = 45f32.to_radians().cos();
        for crouch in [0., 0.5, 1.] {
            for i in 0..64 {
                let mut st = walking(i as f32 * 0.1, 1.);
                st.crouch = crouch;
                st.eye_height = EYE + (CROUCH_EYE - EYE) * crouch;
                let forward = Vec3::new(-st.yaw.to_radians().sin(), 0., -st.yaw.to_radians().cos());
                for p in parts(&st).iter().filter(|p| !p.hidden) {
                    for c in p.corners() {
                        let w = c - eye(&st);
                        if w.dot(forward) > 0. {
                            assert!(
                                w.normalize().dot(forward) < cone,
                                "{} at phase {i}, crouch {crouch}",
                                p.name
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn looking_down_sees_legs_and_hands() {
        // Pitched 75° down a 72° view (±36° up and down, wider at the corners) reaches the body.
        let mut st = walking(0., 0.);
        st.pitch = 75.;
        let (sy, cy) = st.yaw.to_radians().sin_cos();
        let (p, q) = (75f32.to_radians().sin(), 75f32.to_radians().cos());
        let down = Vec3::new(-sy * q, -p, -cy * q);
        let seen = |name: &str| {
            parts(&st)
                .iter()
                .filter(|p| p.name.contains(name))
                .any(|p| {
                    p.corners()
                        .iter()
                        .any(|c| (*c - eye(&st)).normalize().dot(down) > 40f32.to_radians().cos())
                })
        };
        assert!(seen("shin") && seen("hand"));
    }
    #[test]
    fn a_crouch_bends_the_knees_and_leans_the_torso() {
        let stand = parts(&walking(0., 0.));
        let mut c = walking(0., 0.);
        c.crouch = 1.;
        c.eye_height = CROUCH_EYE;
        let crouched = parts(&c);
        let torso = |v: &[Part]| v.iter().find(|p| p.name.contains("torso")).unwrap().clone();
        let tilt = |p: &Part| (p.rotation * Vec3::Y).y.acos();
        assert!(tilt(&torso(&stand)) < 0.01 && tilt(&torso(&crouched)) > 0.5);
        // The head still sits at the eye.
        let head = crouched.iter().find(|p| p.hidden).unwrap();
        assert!((head.centre - eye(&c)).length() < 0.06);
    }
    #[test]
    fn the_boxes_become_objects_with_the_same_corners() {
        let st = walking(0.7, 1.);
        for (p, o) in parts(&st).iter().zip(objects(&st, None)) {
            assert_eq!(o.camera_hidden, p.hidden);
            assert_eq!(o.body, Body::None);
            let mut want = p.corners().map(|c| c.to_array());
            let mut got = crate::geometry::object_corners(&o).map(|c| c.to_array());
            want.sort_by(|a, b| a.partial_cmp(b).unwrap());
            got.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for (a, b) in want.iter().zip(&got) {
                for k in 0..3 {
                    assert!((a[k] - b[k]).abs() < 2e-4, "{}: {a:?} vs {b:?}", p.name);
                }
            }
        }
    }
    #[test]
    fn the_head_is_hidden_from_camera_rays_and_casts_no_shadow() {
        // Through the packed materials and the instance masks the GPU reads (the CPU reference
        // reads the same flag bits): head = hidden + no shadow (any-hit rules, mask 0xff),
        // everything else plain.
        let st = walking(0., 0.);
        let s = Settings {
            objects: objects(&st, None),
            ..Settings::default()
        };
        let packed = crate::lighting::materials(&s);
        for (i, o) in s.objects.iter().enumerate() {
            let flags = packed[5 + i].lobes[3] as u32;
            assert_eq!(flags & 2 != 0, o.camera_hidden, "{}", o.name);
            assert_eq!(flags & 8192 != 0, !o.casts_shadows, "{}", o.name);
            let want = if o.camera_hidden {
                (0xff, true)
            } else {
                (0xff, false)
            };
            assert_eq!(crate::render::plan::instance_mask(&packed[5 + i]), want);
        }
        assert_eq!(s.objects.iter().filter(|o| o.camera_hidden).count(), 1);
    }
    #[test]
    fn the_new_object_fields_default_for_old_scene_files() {
        let o: Object = serde_json::from_str("{}").unwrap();
        assert!(!o.camera_hidden && o.casts_shadows);
        let text = serde_json::to_string(&Object::default()).unwrap();
        assert!(!text.contains("camera_hidden") && !text.contains("casts_shadows"));
        let hidden = Object {
            camera_hidden: true,
            casts_shadows: false,
            ..Object::default()
        };
        let back: Object = serde_json::from_str(&serde_json::to_string(&hidden).unwrap()).unwrap();
        assert!(back.camera_hidden && !back.casts_shadows);
    }
    #[test]
    fn probe_ids_map_the_same_with_the_avatar_in_the_list() {
        // The GPU numbers materials as 5 + objects (the avatar's boxes included) + lights;
        // a pick list without the boxes read a leg as a light and a light as the next one.
        use crate::{gizmo, ui::Selection};
        let mut s = Settings::default();
        s.objects.truncate(2);
        s.lights = vec![Default::default(), Default::default()];
        let real = s.objects.len();
        let lights = s.lights.len();
        let mut picked = s.clone();
        add(&mut picked, &walking(0.4, 1.));
        assert_eq!(picked.objects.iter().filter(|o| is_part(o)).count(), PARTS);
        let pick = |id: u32| gizmo::pick_from_probe(&picked, [0., 0.], [64, 64], id, 1.);
        for li in 0..lights {
            let id = 5 + picked.objects.len() as u32 + li as u32 + 1;
            assert!(
                matches!(pick(id), Some(Selection::Light(l)) if l == li),
                "light {li}"
            );
        }
        let leg = 5 + real as u32 + 3 + 1;
        assert!(matches!(pick(leg), Some(Selection::Object(i)) if i >= real));
    }
    #[test]
    fn the_objects_pass_validation_at_every_body_scale() {
        for scale in [0.2, 1., 5.] {
            let mut st = walking(0.4, 1.);
            st.scale = scale;
            st.eye_height = EYE * scale;
            let mut s = Settings {
                objects: objects(&st, None),
                ..Settings::default()
            };
            s.validate().unwrap();
        }
    }
    #[test]
    fn the_flashlight_is_never_inside_the_body_it_lights_from() {
        // The default hangs 0.3 beside the eye and 0.4 below, for any pitch and yaw.
        for pitch in [-80., -30., 0., 45., 85.] {
            let s = Settings {
                flashlight: true,
                yaw: 20.,
                pitch,
                distance: 0.5,
                ..Settings::default()
            };
            let st = standing(&s);
            let light = Vec3::from(s.flashlight_light().position);
            assert!(
                parts(&st).iter().all(|p| !p.contains(light, 0.02)),
                "{pitch}"
            );
            assert_eq!(objects(&st, Some(light)).len(), PARTS);
        }
        // A light moved into the torso drops that part instead of shadowing everything.
        let st = walking(0., 0.);
        let torso = parts(&st)
            .into_iter()
            .find(|p| p.name.contains("torso"))
            .unwrap();
        let got = objects(&st, Some(torso.centre));
        assert!(got.iter().all(|o| !o.name.contains("torso")) && got.len() < PARTS);
    }
}
