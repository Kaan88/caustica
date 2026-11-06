//! The copy of the scene a CLI render takes: what the app does to its settings over time and to
//! each frame's copy.
use crate::scene::Settings;

/// The scene at time `t`, frozen: the timeline's tracks and per-frame script applied, physics and
/// cloth simulated up to `t` in fixed steps (whatever the wall clock, as a video export does),
/// strings following their objects, the auto-rotating camera turned as far as `t` turns it.
/// `--time` hands the result to either renderer, so both see the same frame; the animation, the
/// simulation and the rotation are off in it.
pub fn at_time(s: &Settings, t: f32) -> Settings {
    let mut now = s.animation.apply(s, 0.);
    if now.physics.enabled && t > 0. {
        let steps = (t * 60.).ceil() as u32;
        let dt = t / steps as f32;
        let mut sim = crate::physics::Sim::new(&now);
        for i in 1..=steps {
            now = s.animation.apply(s, i as f32 * dt);
            sim.take_poses(&mut now);
            if !sim.sync(&now, false) {
                sim = crate::physics::Sim::new(&now);
            }
            sim.run(&mut now, dt);
        }
    } else if t > 0. {
        now = s.animation.apply(s, t);
    }
    if now.objects.iter().any(|o| o.tether.is_some()) {
        crate::physics::update_strings(&mut now);
    }
    now.animation.tracks.clear();
    now.animation.script.clear();
    now.physics.enabled = false;
    if now.rotate {
        now.yaw += now.rotate_speed * t;
        now.rotate = false;
    }
    now
}
/// The copy of the scene a frame renders on the CLI: `at_time`, then what the app adds to each
/// frame's copy (the flashlight, the laser puzzle's beam as rods). The viewport's frame copy
/// already holds them.
pub fn prepare(s: &Settings, t: f32) -> Settings {
    let mut frame = at_time(s, t);
    // Traced before the flashlight joins, as the app traces its settings, not the frame's copy.
    let rods = if frame.puzzle.enabled && frame.puzzle.beam {
        let (segments, _) = crate::laser::trace(&frame);
        crate::laser::beam_objects(&frame, &segments)
    } else {
        vec![]
    };
    frame.add_flashlight();
    frame.objects.extend(rods);
    frame
}
