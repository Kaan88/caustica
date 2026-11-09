#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Off Windows the D3D12-only paths are compiled out until the Vulkan passes use their helpers
// (engine-notes "Vulkan backend: design", phase 5 removes this).
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod anim;
mod assets;
mod audio;
mod balls;
mod browser;
mod console;
mod dirs;
mod display;
mod editor;
mod environment;
mod generators;
mod geometry;
mod gizmo;
#[cfg(windows)]
mod gpu;
mod hotreload;
mod ies;
mod laser;
mod lens;
mod lighting;
mod look;
mod lpe;
mod mesh;
mod meshfmt;
mod omm;
mod paths;
mod physics;
mod platform;
mod prefs;
mod presets;
mod procedural;
mod reference;
mod remote;
mod render;
#[cfg(windows)]
mod renderer;
mod saver;
mod scene;
mod scenes;
mod script;
mod selftest;
mod sky;
mod snap;
mod spectral_tables;
mod spectrum;
mod stats;
mod teapot_data;
mod textscene;
mod texture;
mod timeline_ui;
mod ui;
mod update;
mod video;
mod video_ui;
mod vulkan;
#[cfg(not(windows))]
use vulkan::gpu;
mod walk;
use anyhow::{Context, Result, bail};
#[cfg(not(windows))]
use gpu::Gpu;
use platform::WindowAttributesExtWindows;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(windows)]
use renderer::Renderer as Gpu;
use scene::{PRESETS, Settings};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

#[derive(Default)]
struct Actions {
    reset: bool,
    /// With `reset`: the GPU's render only, the CPU's goes on (it became the graph's reference).
    keep_cpu: bool,
    /// Settings JSON to write or read (the quick buttons use settings.json in `dirs::output_dir`).
    save: Option<PathBuf>,
    load: Option<PathBuf>,
    /// Save the current image (Export tab, F12) in this format.
    save_image: Option<video::ImageFormat>,
    reference: bool,
    /// Start (true) or stop (false) the CPU reference in the viewport.
    cpu_reference: Option<bool>,
    high_res: bool,
    /// Start a video export with `UiState::video`; cancel the running one.
    export_video: bool,
    cancel_export: bool,
    frame_scene: bool,
    /// Track 17 physics buttons.
    physics_wake: bool,
    physics_blast: bool,
}
struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui: egui::Context,
    input: Option<egui_winit::State>,
    settings: Settings,
    paused: bool,
    show_ui: bool,
    last: Instant,
    cursor: [f64; 2],
    drag: Option<MouseButton>,
    drag_distance: f32,
    shift: bool,
    ctrl: bool,
    alt: bool,
    /// The last left click (time, position), for double-clicks.
    last_click: Option<(Instant, [f64; 2])>,
    /// A double-click: frame what the pending pick finds.
    frame_pick: bool,
    preset: usize,
    frames: Option<u32>,
    rendered: u32,
    /// Timing for the RENDER_OK line, from frame 10 on (after startup and BVH builds).
    /// Start, summed gpu_ms, fence wait at the start, summed per-pass ms (trace, reconstruction,
    /// display, UI).
    timing: Option<(Instant, f64, f64, [f64; 4])>,
    capture: Option<PathBuf>,
    capture_sequence: Option<PathBuf>,
    orbit_test: bool,
    /// Moves object 0 a little every frame, as a gizmo drag does (BLAS rebuild cost).
    drag_test: bool,
    require_rr: bool,
    capture_ui: bool,
    self_test: bool,
    /// Self-test frames spent waiting for an export its next check needs finished (the
    /// encoder thread can lag a frame or two); `held` this frame.
    hold_frames: u32,
    held: bool,
    debug: bool,
    /// The Vulkan renderer on Windows (`--vulkan` or Menu → Renderer; Linux has no other).
    vulkan: bool,
    /// `--d3d12`: D3D12 whatever Menu → Renderer says.
    d3d12: bool,
    /// Why the Vulkan renderer picked in the menu didn't start (D3D12 runs instead).
    vulkan_failed: Option<String>,
    hidden: bool,
    size: [u32; 2],
    failed: Option<String>,
    last_key: String,
    geom_key: String,
    material_key: Vec<u8>,
    /// Settings the dirty keys were last computed from.
    checked: Option<Settings>,
    actions: Actions,
    /// Interface state (tab, selection, toasts); never part of `Settings`.
    ui: ui::UiState,
    /// What double-clicking a slider resets to: the scene as loaded (taken again whenever
    /// `scene_name` changes) and a default `UiState`.
    reset_baseline: Settings,
    ui_defaults: Box<ui::UiState>,
    hwnd: Option<platform::HWND>,
    updater: update::Updater,
    /// `--update-from`: a local file or URL to offer as the newest version.
    update_from: Option<String>,
    /// The monitor under the window, re-read when it moves or Windows changes the display setup.
    monitor: Option<display::Monitor>,
    display_dirty: bool,
    hdr_note: String,
    /// Shader hot reload, debug builds only (not in the self-test).
    hot: Option<hotreload::HotReload>,
    /// Lifetime statistics (interactive runs only) and when they were last written.
    stats: Option<stats::Stats>,
    stats_saved: Instant,
    last_samples: u32,
    /// The .cube file now on the GPU (compared with `Settings::lut_path` each frame).
    lut_loaded: String,
    /// The environment now on the GPU (`sky::environment_key`: an HDRI path or the physical
    /// sky's map key), compared each frame.
    hdri_loaded: String,
    /// A sky bake running on a worker thread (windowed runs): its key and result. The
    /// previous map stays on screen until it lands, so dragging the time doesn't stall.
    sky_bake: Option<(
        String,
        std::sync::mpsc::Receiver<anyhow::Result<std::sync::Arc<environment::EnvMap>>>,
    )>,
    /// Ctrl+click focus: waiting for the depth under the click, then easing toward it.
    focus_pending: bool,
    focus_target: Option<f32>,
    /// A click waiting for the object-id readback at this pixel; Shift was held (the pick
    /// adds to or removes from the selection).
    pick_point: Option<[f64; 2]>,
    pick_toggle: bool,
    /// Shift+right-click: the 3D cursor goes to the surface under this pixel (probe depth).
    cursor_pick: Option<[f64; 2]>,
    /// Light painting (L): light edits keep the accumulation; progressive path only.
    painting: bool,
    /// Walk mode (`): WASD and mouse look; the keys held down right now.
    walking: Option<walk::Walk>,
    /// Auto-key's reference state before the scene has keys (anim::Timeline::auto_key).
    auto_key_base: Option<scene::Settings>,
    keys_down: std::collections::HashSet<KeyCode>,
    /// A text field holds the keyboard (see `ui::settle_focus`); the widget focused last frame.
    text_focus: bool,
    focus_seen: Option<egui::Id>,
    /// Screensaver show mode (`/s`): cycles scenes and quits on input.
    saver: Option<saver::Saver>,
    /// `--turntable PATH`: render the video in a headless run, then exit (size 0 = the window's).
    turntable_cli: Option<video::Options>,
    /// The running video export (item 6): rendered a pass at a time in `redraw`.
    export: Option<video::Job>,
    /// The same on the CPU renderer, in the software view: a thread, with its own progress.
    cpu_export: Option<video::CpuExport>,
    /// Phone remote (off by default) and when its page state was last refreshed.
    remote: Option<remote::Remote>,
    remote_synced: Instant,
    /// Gamepad input (interactive runs only).
    gamepad: Option<gilrs::Gilrs>,
    /// The animation's playhead (track 8).
    timeline: anim::Timeline,
    /// Track 17: the running rigid-body simulation (`Settings::physics`).
    physics: Option<physics::Sim>,
    /// Self-test frames 211–213: the scene to restore and the simulated time when edited.
    physics_test: Option<(Box<Settings>, f32)>,
    /// Track 17 laser puzzle state (`Settings::puzzle`).
    laser: laser::Laser,
    /// A running `--script` (track 19).
    script: Option<script::Script>,
    /// Loopback capture while `settings.audio.enabled` (track 19), never headless.
    audio: Option<audio::Audio>,
    /// The CPU renderer in the viewport: the full view (`cpu_view`) or Look → Compare's reference.
    cpu_reference: Option<reference::Live>,
    /// The software view: the viewport shows the CPU's render (`--cpu`, Menu, top bar, Render tab).
    cpu_view: bool,
    /// The software view's denoiser (`--cpu-denoise`, Render tab), off by default.
    cpu_denoise: bool,
    /// An image went up after this frame's render: the next frame shows it.
    cpu_uploaded: bool,
    /// The self-test's image from before a camera move.
    cpu_view_image: Option<std::sync::Arc<Vec<f32>>>,
}
/// Self-test: the first frame of the scene-file sweep (frames before it are scripted).
const SCENES_FROM: u32 = 308;
/// Light painting's rule (both renderers): an edit keeps the long exposure when the transport
/// keys differ only in the camera and the lights. Keys that don't parse never do.
pub(crate) fn painting_keeps(old: &str, new: &str) -> bool {
    let strip = |key: &str| {
        let mut v: serde_json::Value = serde_json::from_str(key).ok()?;
        let o = v.as_object_mut()?;
        for k in [
            "yaw",
            "pitch",
            "distance",
            "target",
            "fov",
            "light_position",
            "light_size",
            "light_aspect",
            "light_intensity",
            "light_temperature",
            "light_tint",
            "light_on",
            "lights",
        ] {
            o.remove(k);
        }
        Some(v)
    };
    matches!((strip(old), strip(new)), (Some(a), Some(b)) if a == b)
}
fn transport_key(s: &Settings) -> String {
    // The JSON tree without objects and lights (hashed below); the caller compares fields.
    let mut j = {
        let mut top = serde_json::to_value(Settings {
            objects: vec![],
            lights: vec![],
            ..s.clone()
        })
        .unwrap();
        top.as_object_mut().unwrap().remove("objects");
        top
    };
    for k in [
        "physics",
        "puzzle",
        // Its effect reaches the key through the scaled intensities of the frame copy.
        "audio",
        "exposure",
        "gamma",
        "tonemap",
        "view",
        "lpe_slot",
        "direct",
        "indirect",
        "denoise",
        "denoise_strength",
        "compare",
        "split",
        "vsync",
        "hdr_mode",
        "hdr_peak",
        "hdr_white",
        "hdr10",
        "rotate",
        "rotate_speed",
        "halation",
        "glare",
        "glare_size",
        "glare_threshold",
        "grain",
        "grain_size",
        "vignette",
        "lift",
        "grade_gamma",
        "gain",
        "lut_path",
        "lut_strength",
        "temporal",
        "scene_name",
        "light_outline",
        // Keys reach the picture through the fields they drive.
        "animation",
    ] {
        j.as_object_mut().unwrap().remove(k);
    }
    // Objects and lights by a hash of their JSON, written straight to bytes: as a JSON tree
    // they cost a 1800-object forest 14 ms per edit. Names and outlines don't render.
    let digest = |bytes: &[u8]| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        format!("{:016x}", h.finish())
    };
    let mut bytes = vec![];
    for o in &s.objects {
        let mut o = o.clone();
        o.name.clear();
        serde_json::to_writer(&mut bytes, &o).unwrap();
    }
    j["objects"] = digest(&bytes).into();
    bytes.clear();
    for l in &s.lights {
        let mut l = l.clone();
        l.name.clear();
        l.outline = false;
        serde_json::to_writer(&mut bytes, &l).unwrap();
    }
    j["lights"] = digest(&bytes).into();
    j.to_string()
}
/// Squared error of l/(1+l) against the reference and the pixel count, per screen quadrant
/// (battle mode's layout: left/right, then top/bottom); the convergence graph sums them.
fn error_sums(image: &[f32], reference: &[f32], w: usize, h: usize) -> [(f64, usize); 4] {
    let compress = |p: &[f32]| {
        let l = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        (l / (1. + l.max(0.))) as f64
    };
    let mut sums = [(0f64, 0usize); 4];
    for y in 0..h {
        for x in 0..w {
            let q = (x >= w / 2) as usize + 2 * (y >= h / 2) as usize;
            let i = (y * w + x) * 4;
            let d = compress(&image[i..i + 4]) - compress(&reference[i..i + 4]);
            sums[q].0 += d * d;
            sums[q].1 += 1;
        }
    }
    sums
}
fn geometry_key(s: &Settings) -> String {
    // Serialized directly (a JSON tree cost a 1800-object forest 3 ms per edit).
    serde_json::to_string(&(
        s.objects
            .iter()
            .map(|o| {
                (
                    o.kind(),
                    o.sdf,
                    o.phase,
                    o.detail,
                    &o.mesh,
                    // Glass goes to the non-opaque BLAS range (shadow rays may pass it), and from
                    // 0.5 shadow rays leave its instance out by mask (track 20a (a)).
                    (o.material.transmission > 0., o.material.transmission >= 0.5),
                    // Masked texture maps need the any-hit loop too.
                    o.material.textures,
                    o.visible,
                    o.position,
                    o.size,
                    o.stretch,
                    o.rotation,
                    o.tilt,
                    o.scale_xz,
                    o.elevation,
                    o.grounded,
                )
            })
            .collect::<Vec<_>>(),
        s.light_on,
        // Visibility to the camera picks the BLAS range (hidden lights need the any-hit loop).
        s.light_visible,
        s.light_position,
        s.light_size,
        s.light_aspect,
        s.cornell_room,
        s.floor_extent,
        s.lights
            .iter()
            // `shadows` picks the instance mask, like `visible`.
            .map(|l| {
                (
                    l.enabled, l.visible, l.shadows, l.position, l.rotation, l.size, l.kind,
                )
            })
            .collect::<Vec<_>>(),
    ))
    .unwrap()
}
fn material_key(s: &Settings) -> Vec<u8> {
    let mut key = bytemuck::cast_slice(&lighting::materials(s)).to_vec();
    // Texture sets are attached on the GPU side, so their paths join the key.
    for m in s
        .objects
        .iter()
        .map(|o| &o.material)
        .chain([&s.walls, &s.floor])
    {
        key.extend(m.texture.as_bytes());
        key.push(0);
    }
    key
}
impl App {
    fn new() -> Result<Self> {
        let mut s = Settings::default();
        let mut frames = None;
        let mut capture = None;
        let mut capture_sequence = None;
        let mut orbit_test = false;
        let mut drag_test = false;
        let mut require_rr = false;
        let mut debug = false;
        let mut vulkan = false;
        let mut d3d12 = false;
        let mut hidden = false;
        let mut capture_ui = false;
        let mut cpu_view = false;
        let mut cpu_denoise = false;
        let mut reference_lenient = false;
        let mut script_path: Option<PathBuf> = None;
        let mut interface = ui::UiState::default();
        let mut self_test = false;
        let mut reference: Option<u32> = None;
        let mut time: Option<f32> = None;
        let mut reference_time: Option<f32> = None;
        let mut selected_preset = 0;
        let mut update_from = None;
        let mut update_yes = false;
        let mut update_cancel_at = None;
        let mut size = [1440, 900];
        // --preset and --settings replace the whole scene, so apply them before every other
        // switch: the command line then means the same in any order.
        let mut ordered: Vec<String> = vec![];
        let mut rest: Vec<String> = vec![];
        // A screensaver run (`/s`, `/c`, `/p`) has no other switches.
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let screensaver = matches!(saver::mode(&raw), saver::Mode::Show);
        let raw = if raw.first().is_some_and(|a| a.starts_with('/')) {
            vec![]
        } else {
            raw
        };
        let mut turntable_cli: Option<video::Options> = None;
        let mut all = raw.into_iter();
        while let Some(arg) = all.next() {
            if arg == "--preset" || arg == "--settings" || arg == "--scene" {
                anyhow::ensure!(
                    ordered.is_empty(),
                    "Give only one of --preset, --settings and --scene"
                );
                ordered.push(arg);
                ordered.extend(all.next());
            } else {
                rest.push(arg);
            }
        }
        ordered.extend(rest);
        let mut args = ordered.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--orbit-test" => orbit_test = true,
                "--turntable" => {
                    turntable_cli = Some(video::Options::turntable(
                        &Settings::default(),
                        [0, 0],
                        60,
                        16,
                        PathBuf::from(args.next().context("--turntable needs a file name")?),
                    ));
                }
                "--turntable-frames" | "--turntable-spp" => {
                    let value: u32 = args.next().context("a number")?.parse()?;
                    let t = turntable_cli.as_mut().context("give --turntable first")?;
                    if arg == "--turntable-frames" {
                        t.frames = value
                    } else {
                        t.spp = value
                    }
                }
                // Follow the scene's keyframes instead of orbiting (track 8).
                "--turntable-timeline" => {
                    turntable_cli
                        .as_mut()
                        .context("give --turntable first")?
                        .timeline = true;
                }
                "--turntable-format" => {
                    let name = args.next().context("a format")?;
                    let t = turntable_cli.as_mut().context("give --turntable first")?;
                    t.format = match name.as_str() {
                        "h264" => video::Format::H264,
                        "h265" => video::Format::H265,
                        "hdr10" => video::Format::Hdr10,
                        "prores" => video::Format::ProRes,
                        "png" => video::Format::Png,
                        "exr" => video::Format::Exr,
                        _ => bail!("--turntable-format h264|h265|hdr10|prores|png|exr"),
                    };
                    if let Some((_, _, q)) = t.format.quality_range() {
                        t.quality = q;
                    }
                }
                "--drag-test" => drag_test = true,
                "--capture-sequence" => {
                    capture_sequence = Some(PathBuf::from(
                        args.next()
                            .context("--capture-sequence needs a directory")?,
                    ))
                }
                "--frames" => {
                    frames = Some(
                        args.next()
                            .context("--frames needs a number")?
                            .parse::<u32>()?,
                    )
                }
                "--capture" => {
                    capture = Some(PathBuf::from(
                        args.next().context("--capture needs a path")?,
                    ))
                }
                "--width" => size[0] = args.next().context("--width needs a number")?.parse()?,
                "--height" => size[1] = args.next().context("--height needs a number")?.parse()?,
                "--scale" => s.scale = args.next().context("--scale needs a value")?.parse()?,
                "--spp" => s.spp = args.next().context("--spp needs a value")?.parse()?,
                "--samples" => {
                    s.max_samples = args.next().context("--samples needs a value")?.parse()?
                }
                "--preset" => {
                    // A number or the preset's name as a slug (classic-cornell-box).
                    let name = args.next().context("--preset needs 0–23 or a name")?;
                    selected_preset = match name.parse() {
                        Ok(i) => i,
                        Err(_) => PRESETS
                            .iter()
                            .position(|p| scenes::slug(p) == name)
                            .with_context(|| {
                                format!(
                                    "--preset {name}: none of {}",
                                    PRESETS.map(scenes::slug).join(", ")
                                )
                            })?,
                    };
                    anyhow::ensure!(selected_preset < PRESETS.len(), "--preset must be 0–23");
                    s.preset(selected_preset);
                }
                "--settings" => {
                    s = serde_json::from_slice(&std::fs::read(
                        args.next().context("--settings needs a path")?,
                    )?)?
                }
                // A scene file by path, file name or title (see scenes/).
                "--scene" => {
                    let name = args
                        .next()
                        .context("--scene needs a path or a scene name")?;
                    s = scenes::read(&scenes::find(&name)?)?;
                }
                // Track 19: a scene from a description (--describe) or the scene so far changed
                // as described (--describe-edit), by the model chosen in Menu > Text-to-scene
                // model (prefs.json, only read here); keys from the environment.
                "--describe" | "--describe-edit" => {
                    let prompt = args.next().context("--describe needs a description")?;
                    let task = if arg == "--describe" {
                        textscene::Task::New
                    } else {
                        textscene::Task::Edit(s.clone())
                    };
                    let ai = prefs::Prefs::load().ai;
                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let mut send = textscene::transport(&ai, "", cancel.clone())?;
                    let mut made = textscene::generate(
                        &prompt,
                        &task,
                        &mut *send,
                        &mut |status| eprintln!("describe: {status}"),
                        &cancel,
                    )?;
                    if !made.notes.is_empty() {
                        println!("DESCRIBE_NOTES {}", made.notes.replace('\n', " | "));
                    }
                    made.settings.keep_output(&s);
                    s = made.settings;
                }
                "--set" => {
                    // Override one top-level setting, e.g. --set direct=0.
                    let pair = args.next().context("--set needs key=value")?;
                    let (key, value) = pair.split_once('=').context("--set needs key=value")?;
                    let mut j = serde_json::to_value(&s)?;
                    anyhow::ensure!(j.get(key).is_some(), "--set: unknown setting {key}");
                    j[key] = serde_json::from_str(value)
                        .unwrap_or_else(|_| serde_json::Value::String(value.into()));
                    s = serde_json::from_value(j).with_context(|| format!("--set {pair}"))?;
                }
                // The scene as it stands at this time on the timeline (reference::at_time), frozen.
                "--time" => {
                    time = Some(
                        args.next()
                            .context("--time needs seconds")?
                            .parse()
                            .context("--time needs seconds")?,
                    );
                }
                // `--reference` renders a timeline export's frame at this time, motion blur and all
                // (reference::exposure).
                "--reference-time" => {
                    reference_time = Some(
                        args.next()
                            .context("--reference-time needs seconds")?
                            .parse()
                            .context("--reference-time needs seconds")?,
                    );
                }
                "--no-rr" => s.rr = false,
                "--rr" => s.rr = true,
                "--require-rr" => {
                    s.rr = true;
                    require_rr = true;
                }
                "--raw" => s.denoise = false,
                "--debug" => debug = true,
                "--vulkan" => vulkan = true,
                // Overrides Menu → Renderer for one run.
                "--d3d12" => d3d12 = true,
                "--headless" => hidden = true,
                // Debug: update from a local file or URL, without publishing a release.
                "--update-from" => {
                    update_from = Some(args.next().context("--update-from needs a file or URL")?)
                }
                // Debug: click through the update, and cancel its download after N bytes.
                "--update-yes" => update_yes = true,
                "--update-cancel-at" => {
                    update_cancel_at = Some(
                        args.next()
                            .context("--update-cancel-at needs bytes")?
                            .parse()?,
                    )
                }
                "--capture-ui" => capture_ui = true,
                "--cpu" => cpu_view = true,
                // The CPU's denoiser: the software view's, or `--reference`'s for its PNG or JPEG.
                "--cpu-denoise" => cpu_denoise = true,
                // A Rhai script that drives the app (see src/script.rs).
                "--script" => {
                    script_path = Some(PathBuf::from(args.next().context("--script needs a file")?))
                }
                // For UI captures: open a tab, show the advanced rows.
                "--ui-tab" => {
                    interface.tab_from_args = true;
                    interface.tab = match args.next().context("--ui-tab needs a name")?.as_str() {
                        "scene" => ui::Tab::Scene,
                        "render" => ui::Tab::Render,
                        "look" => ui::Tab::Look,
                        "export" => ui::Tab::Export,
                        other => {
                            bail!("--ui-tab: unknown tab {other} (scene, render, look, export)")
                        }
                    }
                }
                "--ui-advanced" => interface.advanced = true,
                "--ui-scenes" => interface.show_scenes = true,
                "--ui-shortcuts" => interface.show_shortcuts = true,
                "--ui-assets" => interface.assets.open = true,
                "--ui-video" => interface.show_video = true,
                // Path visualizer open, recording the paths through output pixel X,Y (and
                // selecting path I).
                "--ui-paths" => {
                    let at = args.next().context("--ui-paths needs X,Y[,I]")?;
                    let v: Vec<u32> = at
                        .split(',')
                        .map(|t| t.trim().parse())
                        .collect::<Result<_, _>>()
                        .context("--ui-paths needs X,Y[,I]")?;
                    anyhow::ensure!(matches!(v.len(), 2 | 3), "--ui-paths needs X,Y[,I]");
                    interface.paths.request([v[0], v[1]]);
                    interface.paths.selected = v.get(2).map(|i| *i as usize);
                }
                "--ui-scopes" => {
                    interface.show_scopes = true;
                    interface.zebras = true;
                }
                "--ui-tool" => {
                    interface.tool_from_args = true;
                    interface.tool = match args.next().context("--ui-tool needs a name")?.as_str() {
                        "select" => gizmo::Tool::Select,
                        "move" => gizmo::Tool::Move,
                        "rotate" => gizmo::Tool::Rotate,
                        "scale" => gizmo::Tool::Scale,
                        other => {
                            bail!("--ui-tool: unknown tool {other} (select, move, rotate, scale)")
                        }
                    }
                }
                "--ui-select" => {
                    let what = args.next().context(
                        "--ui-select needs stage, camera, main-light, flashlight, light:N or object:N",
                    )?;
                    let index =
                        |prefix: &str| what.strip_prefix(prefix).and_then(|n| n.parse().ok());
                    interface.selection = match what.as_str() {
                        "stage" => ui::Selection::Stage,
                        "camera" => ui::Selection::Camera,
                        "main-light" => ui::Selection::MainLight,
                        "flashlight" => ui::Selection::Flashlight,
                        _ => match (index("light:"), index("object:")) {
                            (Some(i), _) => ui::Selection::Light(i),
                            (_, Some(i)) => ui::Selection::Object(i),
                            _ => bail!("--ui-select: unknown item {what}"),
                        },
                    };
                }
                "--self-test" => {
                    self_test = true;
                    hidden = true;
                    debug = true;
                    // Then one frame per scene file.
                    frames = Some(SCENES_FROM + scenes::list().len() as u32);
                }
                "--no-vsync" => s.vsync = false,
                // The CPU reference renderer (src/reference/): no GPU, an EXR of N samples per pixel.
                "--reference" => {
                    reference = Some(
                        args.next()
                            .context("--reference needs a sample count")?
                            .parse()
                            .context("--reference needs a sample count")?,
                    )
                }
                // With --reference: render what it can of a scene it would refuse, and print what
                // it left out (scene thumbnails on the CPU).
                "--reference-lenient" => reference_lenient = true,
                "--version" => {
                    println!("Caustica {VERSION}");
                    std::process::exit(0);
                }
                "--help" => {
                    println!("Caustica {VERSION}");
                    println!(
                        "Caustica: --version --frames N --capture image.png|image.exr --width N --height N --scale 0.25..1 --spp 1..16 --samples N --preset 0..23 --settings file.json --set key=json --rr --no-rr --require-rr --raw --debug --headless --cpu --cpu-denoise --no-vsync --capture-ui --script file.rhai --describe \"text\" --describe-edit \"text\" --ui-tab scene|render|look|export --ui-advanced --ui-scenes --ui-shortcuts --ui-assets --ui-video --ui-scopes --ui-tool select|move|rotate|scale --ui-select stage|camera|main-light|flashlight|light:N|object:N --capture-sequence dir --time T --reference SPP --reference-time T --reference-lenient --make-thumbnails [--cpu] --mesh-info file… --fetch-asset source/kind/id[@res]… --asset-catalog source… --orbit-test --drag-test --turntable out.mp4 --turntable-frames N --turntable-spp N --turntable-format h264|h265|hdr10|prores|png|exr --turntable-timeline --self-test"
                    );
                    for (i, name) in PRESETS.iter().enumerate() {
                        println!("  preset {i:2}: {name}");
                    }
                    for e in scenes::list() {
                        println!(
                            "  --scene {:24} {} ({})",
                            e.path.file_stem().unwrap_or_default().to_string_lossy(),
                            e.title,
                            e.pack
                        );
                    }
                    std::process::exit(0);
                }
                _ => bail!("Unknown argument: {arg}"),
            }
        }
        s.validate()?;
        anyhow::ensure!(
            (64..=8192).contains(&size[0]) && (64..=8192).contains(&size[1]),
            "Dimensions must be 64–8192"
        );
        anyhow::ensure!(
            reference_time.is_none() || reference.is_some() && time.is_none(),
            "--reference-time goes with --reference, instead of --time"
        );
        if let Some(t) = time {
            anyhow::ensure!(t >= 0., "--time can't be negative");
            s = reference::at_time(&s, t);
            s.validate()?;
        }
        // A hidden software view waits for the finished render, which a scene that keeps moving
        // never reaches: it renders the scene frozen, as `--reference` does.
        if cpu_view && hidden && !orbit_test && !self_test && time.is_none() {
            s = reference::at_time(&s, 0.);
            s.validate()?;
        }
        if let Some(spp) = reference {
            anyhow::ensure!(spp > 0, "--reference needs at least one sample");
            // A turntable or timeline export on the CPU, with no device: `--reference` gives
            // its samples per frame and the window size its frame size.
            if let Some(mut t) = turntable_cli.take() {
                anyhow::ensure!(
                    capture.is_none(),
                    "--reference with --turntable writes no --capture"
                );
                anyhow::ensure!(
                    reference_time.is_none(),
                    "--reference-time renders one frame, not a --turntable"
                );
                [t.width, t.height, t.spp] = [size[0], size[1], spp];
                [t.pitch, t.distance] = [s.pitch, s.distance];
                let text =
                    video::render_cpu(&s, t, &std::sync::atomic::AtomicBool::new(false), &|f| {
                        println!("REFERENCE_PROGRESS {:.0}%", f * 100.)
                    })?;
                println!("TURNTABLE {text}");
                std::process::exit(0);
            }
            let path = capture.context("--reference needs --capture out.exr|out.png")?;
            let moments = match reference_time {
                Some(t) => reference::exposure(&s, t)?,
                None => vec![reference::prepare(&s, 0.)],
            };
            reference::run(&moments, size, spp, &path, cpu_denoise, reference_lenient)?;
            std::process::exit(0);
        }
        anyhow::ensure!(frames != Some(0), "--frames must be positive");
        if turntable_cli.is_some() && frames.is_none() {
            frames = Some(4);
        }
        anyhow::ensure!(
            !hidden || frames.is_some() || script_path.is_some(),
            "--headless requires --frames or --script"
        );
        // A script decides its own frames and captures; a frame limit would cut it short.
        anyhow::ensure!(
            script_path.is_none() || (frames.is_none() && capture.is_none() && !self_test),
            "--script replaces --frames, --capture and --self-test (use frames() and capture())"
        );
        let script = script_path.map(script::Script::start).transpose()?;
        let egui = egui::Context::default();
        ui::theme(&egui);
        Ok(Self {
            window: None,
            gpu: None,
            egui,
            input: None,
            last_key: transport_key(&s),
            geom_key: geometry_key(&s),
            material_key: material_key(&s),
            checked: None,
            reset_baseline: s.clone(),
            ui_defaults: Box::default(),
            settings: s,
            paused: false,
            show_ui: !hidden || capture_ui || self_test,
            last: Instant::now(),
            cursor: [0.; 2],
            drag: None,
            drag_distance: 0.,
            shift: false,
            ctrl: false,
            alt: false,
            last_click: None,
            frame_pick: false,
            preset: selected_preset,
            frames,
            rendered: 0,
            timing: None,
            capture,
            capture_sequence,
            orbit_test,
            drag_test,
            require_rr,
            capture_ui,
            self_test,
            hold_frames: 0,
            held: false,
            debug,
            vulkan,
            d3d12,
            vulkan_failed: None,
            hidden,
            size,
            failed: None,
            actions: Actions::default(),
            ui: {
                interface.fixed_names = hidden || self_test;
                // Material balls render in other processes: not from headless runs or the self-test.
                balls::ENABLED.store(!(hidden || self_test), std::sync::atomic::Ordering::Relaxed);
                interface
            },
            hwnd: None,
            updater: update::Updater::new(update_yes, update_cancel_at),
            update_from,
            monitor: None,
            display_dirty: true,
            hdr_note: String::new(),
            hot: None,
            stats: None,
            stats_saved: Instant::now(),
            last_samples: 0,
            lut_loaded: String::new(),
            hdri_loaded: String::new(),
            sky_bake: None,
            focus_pending: false,
            focus_target: None,
            pick_point: None,
            pick_toggle: false,
            cursor_pick: None,
            painting: false,
            walking: None,
            auto_key_base: None,
            keys_down: Default::default(),
            text_focus: false,
            focus_seen: None,
            saver: screensaver.then(saver::Saver::new),
            turntable_cli,
            export: None,
            cpu_export: None,
            remote: None,
            remote_synced: Instant::now(),
            gamepad: None,
            timeline: anim::Timeline::default(),
            physics: None,
            physics_test: None,
            laser: laser::Laser::default(),
            script,
            audio: None,
            cpu_reference: None,
            cpu_view,
            cpu_denoise,
            cpu_uploaded: false,
            cpu_view_image: None,
        })
    }
    /// Handles the script's requests until it asks for frames or has nothing to ask. Returns
    /// whether to render this frame: false when the app is exiting, and in a headless run
    /// unless the script is waiting for frames (so the frame count is the script's alone).
    fn poll_script(&mut self, event_loop: &ActiveEventLoop) -> bool {
        use script::{Reply, Request};
        let mut handled = 0;
        loop {
            // In the window, at most 256 requests a frame, so a generator adding thousands of
            // objects keeps the UI (and Stop) responsive.
            if !self.hidden && handled >= 256 {
                return true;
            }
            let sc = self.script.as_mut().unwrap();
            for line in sc.output() {
                console::log(&mut self.ui.console.log, line);
            }
            if let Some(result) = sc.finished() {
                // Lines printed just before the end.
                for line in sc.output() {
                    console::log(&mut self.ui.console.log, line);
                }
                self.script = None;
                if result.is_ok()
                    && let Some(st) = &mut self.stats
                {
                    st.mark("scripter");
                }
                console::log(
                    &mut self.ui.console.log,
                    match &result {
                        Ok(()) => "Finished".to_string(),
                        Err(e) => format!("Error: {e}"),
                    },
                );
                match result {
                    Ok(()) => {
                        println!("SCRIPT_OK frames={}", self.rendered);
                        if self.hidden {
                            event_loop.exit();
                            return false;
                        }
                        self.ui.toast("Script finished");
                    }
                    Err(e) => {
                        println!("SCRIPT_ERROR {e}");
                        if self.hidden {
                            self.fail(event_loop, anyhow::anyhow!("script: {e}"));
                            return false;
                        }
                        self.ui.error(format!("Script: {e}"));
                    }
                }
                return true;
            }
            let wait = std::time::Duration::from_millis(if self.hidden { 50 } else { 0 });
            let Some((request, tx)) = sc.next(wait) else {
                return !self.hidden || sc.waiting.is_some();
            };
            // Stopped: requests already queued are refused, not run.
            if sc.stopped() {
                let _ = tx.send(Reply::Done(Err("stopped".into())));
                continue;
            }
            handled += 1;
            let done = |r: Result<()>| Reply::Done(r.map_err(|e| format!("{e:#}")));
            let changes = matches!(
                request,
                Request::Load(_)
                    | Request::Set(..)
                    | Request::Reset
                    | Request::Add(_)
                    | Request::Clear
            );
            if sc.stale && matches!(request, Request::Capture(_) | Request::Samples) {
                let _ = tx.send(Reply::Done(Err(
                    "nothing rendered since the last load/set/reset: call frames(n) first".into(),
                )));
                continue;
            }
            let reply = match request {
                Request::Load(name) => {
                    self.ui.checkpoint_scene(&self.settings);
                    self.ui.selection = ui::Selection::Stage;
                    self.laser = laser::Laser::default();
                    self.physics = None;
                    // A scene file, or a built-in preset by its slug (as --preset takes it).
                    match scenes::find(&name) {
                        Ok(p) => done(scenes::load_into(&mut self.settings, &p)),
                        Err(e) => match PRESETS.iter().position(|p| scenes::slug(p) == name) {
                            Some(i) => {
                                self.settings.preset(i);
                                self.preset = i;
                                done(Ok(()))
                            }
                            None => done(Err(e)),
                        },
                    }
                }
                Request::Set(key, value) => done(script::set(&mut self.settings, &key, &value)),
                Request::Frames(0) => done(Ok(())),
                Request::Frames(n) => {
                    sc.waiting = Some((n, tx));
                    return true;
                }
                Request::Reset => {
                    self.actions.reset = true;
                    done(Ok(()))
                }
                Request::Capture(path) => {
                    let path = PathBuf::from(path);
                    let hdr = path.extension().is_some_and(|v| v == "exr");
                    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    done(match (self.cpu_capture(&path), self.gpu.as_mut()) {
                        (Some(cpu), _) => cpu,
                        (None, Some(g)) => unsafe { g.capture(&path, hdr, false) }
                            .and_then(|_| video::tag_capture(&path, g.width, g.height)),
                        (None, None) => Err(anyhow::anyhow!("no GPU yet")),
                    })
                }
                // The software view's samples are the CPU's (its GPU traces one pass).
                Request::Samples => Reply::Number(match &self.cpu_reference {
                    Some(live) if live.view => live.latest().map_or(0, |l| l.2 as i64),
                    _ => self.gpu.as_ref().map_or(0, |g| g.samples as i64),
                }),
                Request::Get(key) => match script::get(&self.settings, &key) {
                    Ok(t) => Reply::Text(t),
                    Err(e) => done(Err(e)),
                },
                Request::Scenes => {
                    Reply::List(scenes::list().into_iter().map(|e| e.title).collect())
                }
                // Generators from scripts: objects from their scene-file JSON.
                Request::Add(json) => done(
                    serde_json::from_str::<scene::Object>(&json)
                        .map_err(anyhow::Error::from)
                        .and_then(|o| {
                            self.settings.objects.push(o);
                            let valid = self.settings.validate();
                            if valid.is_err() {
                                self.settings.objects.pop();
                            }
                            valid
                        }),
                ),
                Request::Clear => {
                    // Nothing may keep pointing at the removed objects (as `remove_object`).
                    let s = &mut self.settings;
                    s.objects.clear();
                    s.animation
                        .tracks
                        .retain(|t| !anim::under(&t.path, "objects"));
                    s.puzzle.target = -1;
                    s.puzzle.targets.clear();
                    done(Ok(()))
                }
            };
            if changes {
                self.script.as_mut().unwrap().stale = true;
            }
            let _ = tx.send(reply);
        }
    }
    /// A capture in the software view: the CPU's finished render, not the GPU's one pass. A PNG
    /// or JPEG goes through the CPU display transform as `--reference` writes it, an EXR keeps
    /// the plain mean (never the denoised one). None outside the software view.
    fn cpu_capture(&self, path: &std::path::Path) -> Option<Result<()>> {
        let live = self.cpu_reference.as_ref().filter(|l| l.view)?;
        let format = video::ImageFormat::from_path(path).filter(|f| {
            matches!(
                f,
                video::ImageFormat::Exr | video::ImageFormat::Png | video::ImageFormat::Jpeg
            )
        });
        let Some(format) = format else {
            return Some(Err(anyhow::anyhow!(
                "The software view captures .png, .jpg or .exr: layers and HDR images come from the GPU's render"
            )));
        };
        let Some((shown, plain, _)) = live.latest() else {
            return Some(Err(anyhow::anyhow!(
                "The CPU render has no finished pass yet: call frames(n) until samples() grows"
            )));
        };
        let [w, h] = live.size;
        Some(if format == video::ImageFormat::Exr {
            video::save_linear(path, &plain, w, h)
        } else {
            video::write_cpu_image(
                &self.settings,
                path,
                format,
                video::JPEG_QUALITY,
                shown.to_vec(),
                [w, h],
                0,
            )
        })
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        eprintln!("ERROR: {error:#}");
        self.failed = Some(format!("{error:#}"));
        event_loop.exit();
    }
    /// Self-test: an image file was written, is not stale and decodes (at `size` when given).
    fn expect_image(&self, path: PathBuf, size: Option<[u32; 2]>) -> Result<()> {
        let decoded = if path.extension().is_some_and(|e| e == "exr") {
            // The image crate reads EXRs back as float RGBA.
            image::open(&path).map(|i| [i.width(), i.height()])
        } else {
            image::image_dimensions(&path).map(|(w, h)| [w, h])
        };
        let got = decoded.with_context(|| format!("{} was not written", path.display()))?;
        anyhow::ensure!(
            got[0] > 0 && size.is_none_or(|s| s == got),
            "{} is {got:?}, expected {size:?}",
            path.display()
        );
        Ok(())
    }
    fn redraw(&mut self) -> Result<()> {
        if self.orbit_test && self.rendered >= 64 {
            self.settings.yaw = (self.rendered - 64) as f32 * 0.25;
        }
        if self.drag_test && self.rendered >= 10 && !self.settings.objects.is_empty() {
            let x = &mut self.settings.objects[0].position[0];
            *x = (*x + if self.rendered % 40 < 20 { 0.01 } else { -0.01 }).clamp(-40., 40.);
        }
        // An export check waits (up to 600 frames) instead of failing on timing.
        self.held = self.self_test
            && matches!(self.rendered, 145 | 149 | 276)
            && self.export.is_some()
            && self.hold_frames < 600;
        self.hold_frames += self.held as u32;
        // A hidden run of the software view captures the finished render, not the first pass.
        if self.hidden
            && self.cpu_view
            && !self.self_test
            && !self.orbit_test
            && self.frames.is_some()
            && self.cpu_reference.as_ref().is_none_or(|l| l.busy())
        {
            self.held = true;
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.self_test && !self.held {
            self.self_test_script()?;
        }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.settings.rotate && !self.paused && self.export.is_none() {
            self.settings.yaw += self.settings.rotate_speed * dt;
        }
        let window = self.window.as_ref().unwrap().clone();
        let mut input = self.input.as_mut().unwrap().take_egui_input(&window);
        ui::keep_tab_for_app(&mut input, self.text_focus && self.ui.transform.is_none());
        let ctx = self.egui.clone();
        let output = ctx.run(input, |ctx| {
            // During an export only its progress window is drawn (and takes input): the scene
            // must not change under it.
            if self.export.is_some() || self.cpu_export.is_some() {
                self.export_window(ctx);
            } else if self.show_ui {
                self.ui(ctx);
            }
        });
        self.text_focus = ui::settle_focus(
            &ctx,
            output.platform_output.ime.is_some(),
            &mut self.focus_seen,
        );
        // That focus settles next frame, also in an idle viewport.
        if self.focus_seen.is_some() {
            window.request_redraw();
        }
        self.input
            .as_mut()
            .unwrap()
            .handle_platform_output(&window, output.platform_output);
        // Keyframes: the playhead moves while playing, and the animation drives its fields
        // whenever the playhead or the keys change. An export renders its own copy.
        if self.export.is_none() {
            self.timeline.advance(&self.settings.animation, dt);
            let keys_edited = self.timeline.keys_changed(&self.settings);
            // An edit still in its field came before what the playhead is about to write.
            if !keys_edited && self.timeline.playhead_moved() && !self.edits_foreign() {
                self.ui.flush_pending(&self.settings);
            }
            let applied =
                self.timeline.update(&mut self.settings) && !self.settings.animation.is_empty();
            self.ui.keys_applied(&self.settings, applied, keys_edited);
            // Auto-key waits for a drag or transform to end (one key per edit, not per frame).
            let busy = self.ui.transform.is_some() || self.drag.is_some();
            if self
                .timeline
                .auto_key(&mut self.settings, busy, &mut self.auto_key_base)
                > 0
            {
                self.ui.show_timeline = true;
            }
            // The self-test steps a fixed 1/60 s, so its physics checks don't depend on how
            // long a frame took.
            self.step_physics(if self.self_test { 1. / 60. } else { dt });
            if self
                .laser
                .update(&self.settings, self.ui.transform.is_some())
            {
                let (n, par) = (self.laser.moves, self.settings.puzzle.par);
                let best = self
                    .ui
                    .prefs
                    .puzzle_best
                    .entry(self.settings.scene_name.clone())
                    .or_insert(n);
                if n <= *best {
                    *best = n;
                    self.ui.save_prefs();
                }
                if let Some(st) = &mut self.stats {
                    st.solved(&self.settings.scene_name, n <= par);
                }
                let moves = if n == 1 {
                    "1 move".into()
                } else {
                    format!("{n} moves")
                };
                self.ui.toast(if n <= par {
                    format!("Solved in {moves} (par {par})")
                } else {
                    format!("Solved in {moves}; par is {par}")
                });
            }
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        if let Some(path) = self.actions.save.take() {
            match serde_json::to_string_pretty(&self.settings)
                .map_err(anyhow::Error::from)
                .and_then(|j| std::fs::write(&path, j).map_err(Into::into))
            {
                Ok(()) => {
                    self.ui.mark_clean(&self.settings);
                    self.ui.toast(format!("Saved {}", path.display()));
                }
                Err(e) => self.ui.error(format!("Save failed: {e}")),
            }
        }
        if let Some(path) = self.actions.load.take() {
            match std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| serde_json::from_slice::<Settings>(&b).map_err(Into::into))
                .and_then(|mut s| {
                    s.validate()?;
                    Ok(s)
                }) {
                Ok(s) => {
                    self.ui.checkpoint_scene(&self.settings);
                    self.settings = s;
                    self.ui.mark_clean(&self.settings);
                    self.ui.toast(format!("Loaded {}", path.display()));
                }
                Err(e) => self.ui.error(format!("Load rejected: {e}")),
            }
        }
        if let Some(result) = self.hot.as_mut().and_then(|h| h.poll()) {
            let started = Instant::now();
            let g = self.gpu.as_mut().unwrap();
            match result
                .map_err(anyhow::Error::msg)
                .and_then(|sh| unsafe { g.reload_shaders(&sh) })
            {
                Ok(()) => {
                    println!("SHADERS RELOADED");
                    if let Some(st) = &mut self.stats {
                        st.hot_reloads += 1;
                    }
                    self.ui.toast(format!(
                        "Shaders reloaded ({} ms)",
                        started.elapsed().as_millis()
                    ));
                }
                Err(e) => {
                    println!("SHADER ERROR {e:#}");
                    self.ui
                        .error(format!("Shader error, keeping the old shaders:\n{e:#}"));
                }
            }
        }
        if let Some(sv) = &mut self.saver
            && Instant::now() >= sv.next_switch
        {
            // Presets except the empty stage, then every scene file.
            let presets = PRESETS.len() - 1;
            let count = presets + self.ui.scenes.len();
            let i = sv.index % count;
            sv.index += 1;
            sv.next_switch = Instant::now() + Duration::from_secs(saver::Saver::SCENE_SECONDS);
            if i < presets {
                self.settings.preset(i);
            } else {
                self.load_scene_file(i - presets);
            }
            self.settings.rotate = true;
            self.settings.rotate_speed = 4.;
        }
        let g = self.gpu.as_mut().unwrap();
        if self.actions.frame_scene {
            self.settings
                .frame_objects(g.width as f32 / g.height as f32);
            self.actions.frame_scene = false;
        }
        if let Some(w) = &mut self.walking {
            w.update(
                &mut self.settings,
                &self.keys_down,
                dt,
                self.shift,
                &self.geom_key,
            );
            if let Some(note) = w.message() {
                self.ui.toast(note);
            }
        }
        // The phone remote and gamepad edit the settings; they wait while a video renders.
        if self.export.is_none() {
            self.remote_and_gamepad(dt);
        }
        if let Some(path) = self.ui.open_pending.take() {
            self.open_scene(&path);
        }
        // Audio-reactive lights scale this frame's copy only (track 19).
        // Auto-key: what walking, physics, the remote and the gamepad changed this frame is
        // not an edit; only the UI's changes before the next auto-key count.
        self.timeline
            .absorb(&self.settings, &mut self.auto_key_base);
        let mut frame = self.settings.clone();
        frame.add_flashlight();
        // Laser puzzle (track 17): the beam as glowing rods in this frame's copy.
        if frame.puzzle.enabled && frame.puzzle.beam {
            let rods = laser::beam_objects(&frame, &self.laser.segments);
            frame.objects.extend(rods);
        }
        if frame.audio.enabled && !self.hidden && self.export.is_none() {
            if self.audio.is_none() {
                match audio::Audio::start() {
                    Ok(a) => self.audio = Some(a),
                    Err(e) => {
                        self.ui.error(format!("Audio capture: {e:#}"));
                        self.settings.audio.enabled = false;
                    }
                }
            }
            if let Some(a) = &mut self.audio {
                a.update(&frame.audio, dt);
                // Light painting keeps exposing through light changes, not emitter changes.
                audio::apply(&mut frame, a.levels, a.beat, !self.painting);
            }
            if let Some(Err(e)) = self.audio.as_mut().map(|a| a.recover()) {
                self.audio = None;
                self.settings.audio.enabled = false;
                self.ui.error(format!("Audio capture stopped: {e:#}"));
            }
        } else {
            self.audio = None;
        }
        let g = self.gpu.as_mut().unwrap();
        g.painting = self.painting && self.export.is_none();
        let s = &frame;
        // HDR output follows the monitor: re-read it after a move, or once Windows changed the
        // display setup (HDR toggled, calibration rerun, SDR brightness moved).
        // Wayland has no window position: the output's name tells when the window moved. It also
        // can't hide a window, so a hidden run asks for no output, to stay reproducible.
        #[cfg(not(windows))]
        let window_output = self
            .window
            .as_ref()
            .filter(|_| !self.hidden)
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.name());
        #[cfg(not(windows))]
        if self
            .monitor
            .as_ref()
            .is_none_or(|m| m.info.name != window_output.as_deref().unwrap_or_default())
        {
            self.display_dirty = true;
        }
        if self.display_dirty
            || (self.rendered.is_multiple_of(30)
                && self
                    .monitor
                    .as_mut()
                    .is_some_and(|m| !unsafe { m.current() }))
        {
            let previous = self.monitor.take().map(|m| m.info);
            #[cfg(windows)]
            {
                self.monitor = self
                    .hwnd
                    .and_then(|h| unsafe { display::Monitor::query(h) }.ok());
            }
            #[cfg(not(windows))]
            {
                self.monitor = unsafe { display::Monitor::query(window_output.as_deref()) }.ok();
            }
            self.display_dirty = false;
            if let Some(m) = &self.monitor
                && previous.as_ref() != Some(&m.info)
            {
                println!("DISPLAY {:?}", m.info);
                // Another monitor or display setup may take HDR10: ask again.
                g.hdr10_refused = false;
                g.hdr_refused = false;
            }
        }
        let info = self
            .monitor
            .as_ref()
            .map(|m| m.info.clone())
            .unwrap_or_default();
        let want = match s.hdr_mode {
            0 => info.hdr,
            1 => false,
            _ => true,
        };
        self.hdr_note.clear();
        let hdr10 = s.hdr10 && !g.hdr10_refused;
        let switch =
            (want != g.hdr_active || (want && hdr10 != g.hdr10)) && !(want && g.hdr_refused);
        if switch && self.export.is_none() {
            unsafe { g.set_hdr(want, hdr10, s)? };
        }
        if want && g.hdr_refused {
            self.hdr_note = "The swapchain can't present scRGB; staying SDR.".into();
        }
        if want && s.hdr10 && g.hdr10_refused && !g.hdr_refused {
            self.hdr_note = "The swapchain can't present HDR10; using scRGB.".into();
        }
        g.hdr_levels = [
            if s.hdr_white > 0. {
                s.hdr_white
            } else {
                // Wayland's reference follows the brightness slider, well below Windows' range.
                info.sdr_white_nits
                    .max(if cfg!(windows) { 40. } else { 1. })
            },
            if s.hdr_peak > 0. {
                s.hdr_peak
            } else if info.peak_nits >= 80. {
                info.peak_nits
            } else {
                1000.
            },
        ];
        // A known Wayland output: see GpuState::hdr_reference. Windows reads levels as absolute.
        #[cfg(not(windows))]
        {
            g.hdr_reference = if info.peak_nits > 0. {
                info.sdr_white_nits
            } else {
                0.
            };
        }
        let rw = ((g.width as f32 * s.scale).round() as u32).max(1);
        if g.rw != rw && self.export.is_none() {
            unsafe {
                g.resize_output(g.width, g.height, s)?;
            }
        }
        // The keys serialize the whole scene, so only rebuild them after an edit. An export
        // renders its own copy; edits made meanwhile (none through the UI) wait for its end.
        if self.checked.as_ref() != Some(s) && self.export.is_none() {
            let geometry = geometry_key(s);
            let mk = material_key(s);
            if self.geom_key != geometry {
                // Also uploads the materials and light tables.
                unsafe {
                    g.rebuild_scene(s)?;
                }
                self.geom_key = geometry;
            } else if self.material_key != mk {
                unsafe {
                    g.update_materials(s)?;
                }
            }
            self.material_key = mk;
            let key = transport_key(s);
            if key != self.last_key {
                let mut old: serde_json::Value = serde_json::from_str(&self.last_key)?;
                let mut new: serde_json::Value = serde_json::from_str(&key)?;
                if ["rr_preset", "rr", "upscaler", "dlss_preset", "native_aa"]
                    .iter()
                    .any(|k| old[k] != new[k])
                {
                    unsafe {
                        g.reconfigure_upscalers()?;
                    }
                }
                for k in ["yaw", "pitch", "distance", "target", "fov"] {
                    old.as_object_mut().unwrap().remove(k);
                    new.as_object_mut().unwrap().remove(k);
                }
                // Light painting: moving or changing lights keeps the long exposure going.
                let camera_moved = old != new;
                let painted = self.painting && painting_keeps(&self.last_key, &key);
                if !painted {
                    g.reset(camera_moved);
                }
                self.last_key = key;
            }
            self.checked = Some(s.clone());
        }
        if self.actions.reset {
            g.reset(true);
            self.actions.reset = false;
            // Restart means the CPU's render too, unless it is what the GPU is measured against.
            if let Some(live) = self
                .cpu_reference
                .as_mut()
                .filter(|_| !std::mem::take(&mut self.actions.keep_cpu))
            {
                live.key.clear();
            }
        }
        // The software view traces one GPU pass per restart, for what clicks read (object ids,
        // depth) and the albedo under the light meter; the picture is the CPU's.
        // A display-only GPU has only the software view.
        self.cpu_view |= !g.ray_tracing;
        let draw = if self.cpu_view {
            g.samples == 0
        } else {
            (!self.paused || g.samples == 0) && (g.samples < s.max_samples || self.painting)
        };
        // Look tools: zebras and scopes in the display pass, a light-meter texel under the cursor.
        // Zebras would be drawn into the exported frames.
        g.zebras = self.ui.zebras && self.export.is_none();
        g.scopes_on = self.ui.show_scopes && self.export.is_none();
        video::set_equirect(s.projection == 2);
        let probe_at = self.pick_point.or(self.cursor_pick).unwrap_or(self.cursor);
        // No hover outline (owner, 2026-11-05: outlines only for the clicked selection; in
        // Sponza something is under the pointer everywhere), so no probe while hovering.
        g.probe = ((self.ui.meter && !self.egui.is_pointer_over_area())
            || self.focus_pending
            || self.pick_point.is_some()
            || self.cursor_pick.is_some())
        .then_some([probe_at[0].max(0.) as u32, probe_at[1].max(0.) as u32]);
        if s.lut_path != self.lut_loaded && self.export.is_none() {
            self.lut_loaded = s.lut_path.clone();
            let result = if s.lut_path.is_empty() {
                unsafe { g.set_lut(None) }
            } else {
                // Relative paths resolve under the scene folders like mesh and IES paths (a
                // shipped scene's `luts/…`), then against the working directory.
                let path = crate::mesh::resolve(&s.lut_path);
                let path = if path.exists() {
                    path
                } else {
                    s.lut_path.clone().into()
                };
                std::fs::read_to_string(path)
                    .map_err(anyhow::Error::from)
                    .and_then(|text| look::read_cube(&text))
                    .and_then(|(n, data)| unsafe { g.set_lut(Some((n, &data))) })
            };
            self.ui.lut_error = result.as_ref().err().map(|e| format!("{e:#}"));
            match result {
                Ok(()) if !s.lut_path.is_empty() => {
                    self.ui.toast(format!("LUT loaded: {}", s.lut_path))
                }
                Ok(()) => {}
                Err(e) => {
                    unsafe { g.set_lut(None)? };
                    self.ui.error(format!("LUT {}: {e:#}", s.lut_path));
                }
            }
        }
        let environment = sky::environment_key(s);
        // Windowed: an uncached sky bakes on a worker thread (one at a time; the newest key
        // wins when it ends) and lands with an accumulation restart.
        // A bake for an older key still lands while the newest is uncached (live preview
        // while dragging); a panicked worker frees the slot.
        let landed = match &self.sky_bake {
            Some((key, rx)) => match rx.try_recv() {
                Ok(result) => Some((key.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some((key.clone(), Err(anyhow::anyhow!("the sky bake stopped"))))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            },
            None => None,
        };
        if let Some((key, result)) = landed {
            self.sky_bake = None;
            if s.physical_sky.on
                && self.export.is_none()
                && (key == environment || sky::cached(&environment).is_none())
            {
                self.hdri_loaded = key;
                self.actions.reset = true;
                if let Err(e) = result.and_then(|m| unsafe { g.set_environment(Some(&m)) }) {
                    self.ui.error(format!("Sky: {e:#}"));
                }
            }
        }
        if environment != self.hdri_loaded
            && self.export.is_none()
            && s.physical_sky.on
            && !self.hidden
            && !self.self_test
            && sky::cached(&environment).is_none()
        {
            if self.sky_bake.is_none() {
                let (tx, rx) = std::sync::mpsc::channel();
                let sky = s.physical_sky.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(sky::bake_cached(&sky));
                });
                self.sky_bake = Some((environment, rx));
            }
        } else if environment != self.hdri_loaded && self.export.is_none() {
            self.hdri_loaded = environment;
            self.ui.environment_error = None;
            if let Err(e) = unsafe { sky::upload_environment(g, s) } {
                unsafe { g.set_environment(None)? };
                if !s.physical_sky.on {
                    self.ui.environment_error = Some(format!("{e:#}"));
                }
                self.ui.error(if s.physical_sky.on {
                    format!("Sky: {e:#}")
                } else {
                    format!("HDRI {}: {e:#}", s.hdri)
                });
            }
        }
        if self.actions.cancel_export {
            self.actions.cancel_export = false;
            if let Some(job) = &self.cpu_export {
                job.cancel();
            }
            if let Some(job) = self.export.take() {
                let out = job.options.output();
                let text = unsafe { job.finish(g, s, true)? };
                println!("EXPORT {text}");
                self.ui.toast_file(text, out);
            }
        }
        if let Some(job) = &mut self.export {
            g.probe = None;
            // Passes until about 30 ms have gone, so the UI keeps about 30 fps; each pass
            // presents the same UI.
            let started = Instant::now();
            let mut delta = Some(&output.textures_delta);
            let empty = egui::TexturesDelta::default();
            let step = loop {
                let pass = unsafe {
                    job.before_render(g)
                        .and_then(|()| {
                            g.render(
                                job.settings(),
                                !job.waiting(),
                                job.dt(),
                                &primitives,
                                delta.take().unwrap_or(&empty),
                                output.pixels_per_point,
                            )
                        })
                        .and_then(|()| job.after_render(g))
                };
                if job.waiting() {
                    // The encoder is behind: draw the UI only, and give it the CPU.
                    std::thread::sleep(Duration::from_millis(5));
                    break pass;
                }
                if pass.is_err() || job.done() || started.elapsed() > Duration::from_millis(30) {
                    break pass;
                }
            };
            let job_done = job.done();
            if step.is_err() || job_done {
                let job = self.export.take().unwrap();
                let (out, still, width) =
                    (job.options.output(), job.options.still, job.options.width);
                let result = unsafe { job.finish(g, s, step.is_err()) };
                match step.and(result) {
                    Ok(text) => {
                        println!("EXPORT {text}");
                        self.ui.toast_file(text, out);
                        if let Some(st) = &mut self.stats {
                            if still {
                                st.images_saved += 1;
                                if width >= 7680 {
                                    st.mark("billboard");
                                }
                            } else {
                                st.videos_exported += 1;
                            }
                        }
                    }
                    Err(e) => {
                        println!("EXPORT failed: {e:#}");
                        self.ui.error(format!("Video export failed: {e:#}"));
                    }
                }
            }
        } else {
            g.path_request = self.ui.paths.take_request();
            // Through the normal display pass; `cpu_view` in post.hlsl reads this split.
            let shown = self.cpu_view.then(|| Settings {
                compare: true,
                split: 2.,
                view: 0,
                ..s.clone()
            });
            unsafe {
                g.render(
                    shown.as_ref().unwrap_or(s),
                    draw,
                    dt,
                    &primitives,
                    &output.textures_delta,
                    output.pixels_per_point,
                )?;
            }
            if let Some(records) = g.path_result.take() {
                self.ui.paths.receive(&records);
            }
        }
        self.ui.hovered = None;
        // A display-only device runs no probe pass: clicks, the 3D cursor, focus and the
        // light meter read a CPU pick and the software view instead.
        if !g.ray_tracing {
            let raw = self
                .cpu_reference
                .as_ref()
                .filter(|l| l.view)
                .and_then(|l| l.raw());
            g.probe_result = g.probe.map(|at| {
                gizmo::cpu_probe(
                    &self.settings,
                    at,
                    [g.width, g.height],
                    raw.as_deref().map(|v| v.as_slice()),
                )
            });
        }
        let point = self.pick_point.take();
        let frame_pick = std::mem::take(&mut self.frame_pick);
        if let (Some(point), Some(p)) = (point, g.probe_result) {
            let pixel = [point[0] as f32, point[1] as f32];
            let menu = self.ui.context_menu.is_some();
            // The puzzle's beam rods follow the objects in the GPU's ids: pick against the
            // same list, and a rod picks nothing.
            let mut picked = self.settings.clone();
            if picked.puzzle.enabled && picked.puzzle.beam {
                let rods = laser::beam_objects(&picked, &self.laser.segments);
                picked.objects.extend(rods);
            }
            let real = self.settings.objects.len();
            match gizmo::pick_from_probe(&picked, pixel, [g.width, g.height], p.id, p.depth)
                .filter(|sel| !matches!(sel, ui::Selection::Object(i) if *i >= real))
            {
                Some(sel) => {
                    if self.pick_toggle {
                        self.ui.toggle_in(sel, &self.settings);
                    } else if !(menu && self.ui.is_selected(sel)) {
                        // A right click on part of the selection keeps all of it for the menu.
                        self.ui.select(sel);
                    }
                    self.ui.tab = ui::Tab::Scene;
                    self.ui.panel_hidden = false;
                    self.ui.scroll_to_inspector = !menu;
                    self.show_ui = true;
                    if frame_pick && let ui::Selection::Object(i) = sel {
                        self.settings
                            .focus_object(i, g.width as f32 / g.height as f32);
                        self.ui
                            .toast(format!("Framed {}", self.settings.objects[i].name));
                    }
                }
                // A right-click on empty space keeps the selection for its menu, and so
                // does a Shift+click.
                None if menu || self.pick_toggle => {}
                None => self.ui.select(ui::Selection::None),
            }
        }
        // Shift+right-click: the 3D cursor onto the surface under the pointer.
        if let Some(point) = self.cursor_pick
            && let Some(p) = g.probe_result
        {
            self.cursor_pick = None;
            let view = gizmo::View::new(&self.settings, [g.width, g.height]);
            match view.surface_point([point[0] as f32, point[1] as f32], p.id, p.depth) {
                Some(c) => {
                    self.ui.cursor3d = c;
                    self.ui.toast(if self.ui.pivot == gizmo::Pivot::Cursor {
                        "3D cursor placed".to_string()
                    } else {
                        "3D cursor placed (pick 3D cursor as the pivot to turn around it)"
                            .to_string()
                    });
                }
                None => self
                    .ui
                    .toast("No surface under the pointer for the 3D cursor"),
            }
        }
        // Ctrl+click focus: the depth under the click becomes the focus target.
        if self.focus_pending
            && let Some(p) = g.probe_result
        {
            self.focus_pending = false;
            if p.depth > 0. && p.depth < 1e3 {
                self.focus_target = Some(p.depth);
                self.ui.toast(format!("Focus: {:.2} units", p.depth));
            }
        }
        if let Some(target) = self.focus_target {
            let f = &mut self.settings.focus_distance;
            // At least 30% of the way per frame, so slow frames still settle in about ten.
            *f += (target - *f) * (1. - (-dt * 10.).exp()).max(0.3);
            if (*f - target).abs() < target * 0.01 {
                *f = target;
                self.focus_target = None;
            }
        }
        // Convergence graph: RMSE against the reference at log-spaced sample counts.
        if self.ui.show_graph
            && let Some((rw, rh, reference)) = &self.ui.reference
            && (*rw, *rh) == (g.width, g.height)
        {
            // The software view measures the CPU's render, which is what it shows.
            let cpu = self.cpu_reference.as_ref().filter(|l| l.view).map(|l| {
                let pixels = g.width as usize * g.height as usize;
                l.latest().filter(|l| l.1.len() == pixels * 4)
            });
            let samples = match &cpu {
                Some(latest) => latest.as_ref().map_or(0, |l| l.2),
                None => g.samples,
            };
            if samples < self.ui.graph.last().map_or(0, |p| p.0) {
                self.ui.graph.clear();
            }
            // Battle mode: per quadrant, error against the GPU time it has spent.
            let spent = g.battle.iter().map(|q| q.ms).fold(0., f64::max);
            let battle_last = self
                .ui
                .battle_graph
                .iter()
                .flatten()
                .map(|p| p.0)
                .fold(0., f64::max);
            if !s.battle || spent < battle_last {
                self.ui.battle_graph = Default::default();
            }
            if s.battle && spent > 0. && (battle_last == 0. || spent >= battle_last * 1.25) {
                let image = unsafe { g.read_floats(18)? };
                let sums = error_sums(&image, reference, g.width as usize, g.height as usize);
                for (q, (sum, n)) in sums.iter().enumerate() {
                    if g.battle[q].ms > 0. {
                        let e = (sum / (*n).max(1) as f64).sqrt() * 1000.;
                        self.ui.battle_graph[q].push((g.battle[q].ms, e));
                    }
                }
            }
            let last = self.ui.graph.last().map_or(0, |p| p.0);
            if !s.battle && samples > 0 && (last == 0 || samples as f64 >= last as f64 * 1.25) {
                let image = match cpu {
                    Some(Some((_, plain, _))) => plain,
                    _ => std::sync::Arc::new(unsafe { g.read_floats(18)? }),
                };
                let sums = error_sums(&image, reference, g.width as usize, g.height as usize);
                let (sum, n) = sums.iter().fold((0., 0), |a, q| (a.0 + q.0, a.1 + q.1));
                self.ui
                    .graph
                    .push((samples, (sum / n.max(1) as f64).sqrt() * 1000.));
            }
        }
        // Lifetime statistics and achievements.
        if let Some(st) = &mut self.stats {
            if g.samples > self.last_samples {
                st.paths += (g.samples - self.last_samples) as f64 * (g.rw * g.rh) as f64;
            }
            self.last_samples = g.samples;
            st.gpu_seconds += g.gpu_ms / 1000.;
            st.max_samples = st.max_samples.max(g.samples);
            if PRESETS.contains(&s.scene_name.as_str()) {
                st.presets_seen.insert(s.scene_name.clone());
            } else if self.ui.scenes.iter().any(|e| e.title == s.scene_name) {
                st.scenes_seen.insert(s.scene_name.clone());
            }
            let builtin: Vec<&String> = self
                .ui
                .scenes
                .iter()
                .filter(|e| {
                    e.path
                        .parent()
                        .and_then(|d| d.file_name())
                        .is_some_and(|n| n == "builtin")
                })
                .map(|e| &e.title)
                .collect();
            let eye = s.camera(1.).2;
            st.observe(stats::Moment {
                samples: g.samples,
                seconds: dt,
                scene: &s.scene_name,
                integrator: s.integrator,
                battle: s.battle,
                spectral: s.spectral,
                real_lens: s.real_lens(),
                hdr: g.hdr_active,
                walking: self.walking.is_some().then_some(eye),
                flashlight: s.flashlight,
                physics: self.physics.is_some(),
                audio: s.audio.enabled,
                cpu_samples: self.cpu_reference.as_ref().map_or(0, |c| c.state().samples),
            });
            let scenes_done =
                !builtin.is_empty() && builtin.iter().all(|t| st.scenes_seen.contains(*t));
            for title in st.unlock_reached(scenes_done) {
                self.ui.toast(format!("Achievement unlocked: {title}"));
            }
            if self.stats_saved.elapsed() > Duration::from_secs(30) {
                st.save();
                self.stats_saved = Instant::now();
            }
        }
        if self.self_test {
            self.self_test_checks(s)?;
        }
        let g = self.gpu.as_mut().unwrap();
        if self.require_rr && !g.rr_active {
            bail!("--require-rr: {}", g.rr_status);
        }
        if self.actions.reference {
            // A frozen frame replaces the CPU reference.
            self.cpu_reference = None;
            unsafe {
                g.capture_reference()?;
            }
            self.settings.compare = true;
            self.actions.reference = false;
        }
        let size = [g.width, g.height];
        match self.actions.cpu_reference.take() {
            Some(true) => {
                self.cpu_reference = Some(reference::Live::start(
                    s.clone(),
                    size,
                    self.last_key.clone(),
                    false,
                ))
            }
            Some(false) => self.cpu_reference = None,
            None => {}
        }
        // The CPU renderer follows the scene: an edit or a resize restarts it. An export renders
        // at its own size with a stale key, so it waits for its end; the view's switch does too.
        if self.export.is_none() {
            if self.cpu_view != self.cpu_reference.as_ref().is_some_and(|l| l.view) {
                self.cpu_reference = self
                    .cpu_view
                    .then(|| reference::Live::start(s.clone(), size, self.last_key.clone(), true));
                g.reference_valid = false;
            }
            if let Some(live) = &mut self.cpu_reference {
                live.set_paused(self.paused);
                live.set_painting(self.painting);
                live.set_denoise(self.cpu_denoise, s.denoise_strength);
                if live.key != self.last_key || live.size != size {
                    live.follow(s.clone(), self.last_key.clone(), size);
                    // The old image no longer matches; the split shows the GPU until the first
                    // pass. The view keeps it until the preview.
                    if !live.view {
                        g.reference_valid = false;
                    }
                }
                if let Some(image) = live.image(!g.reference_valid) {
                    unsafe {
                        g.upload_reference(&image)?;
                    }
                    self.cpu_uploaded = true;
                    self.settings.compare |= !live.view;
                } else if live.view && !g.reference_valid {
                    // Nothing yet: black, not the GPU's picture under the CPU's name.
                    let black = vec![0f32; size[0] as usize * size[1] as usize * 4];
                    unsafe {
                        g.upload_reference(&black)?;
                    }
                }
            }
        }
        // A text-to-scene critique: the render as a PNG, scaled to fit what vision models take.
        if std::mem::take(&mut self.ui.critique_capture) {
            let path =
                std::env::temp_dir().join(format!("caustica-critique-{}.png", std::process::id()));
            let png = unsafe { video::save_current(g, video::ImageFormat::Png, 92, &path) }
                .and_then(|()| {
                    let img = image::open(&path)?;
                    let _ = std::fs::remove_file(&path);
                    let size = textscene::CRITIQUE_SIZE;
                    let img = if img.width().max(img.height()) > size {
                        img.resize(size, size, image::imageops::FilterType::Triangle)
                    } else {
                        img
                    };
                    let mut png = std::io::Cursor::new(Vec::new());
                    img.to_rgb8().write_to(&mut png, image::ImageFormat::Png)?;
                    Ok(png.into_inner())
                });
            match png {
                // The UI starts the request (next frame).
                Ok(png) => self.ui.critique_png = Some(png),
                Err(e) => self
                    .ui
                    .error(format!("Saving the render for the critique: {e:#}")),
            }
        }
        if let Some(format) = self.actions.save_image.take() {
            let samples = match &self.cpu_reference {
                Some(live) if live.view => live.state().samples,
                _ => g.samples,
            };
            let path = format.file(&self.ui.export_stem(
                &self.settings,
                samples,
                g.width,
                g.height,
            ));
            let saved = if self.cpu_view && format == video::ImageFormat::Layers {
                Err(anyhow::anyhow!(
                    "The software view has no layers to save: they come from the GPU's render"
                ))
            } else if let Some(raw) = self
                .cpu_reference
                .as_ref()
                .filter(|l| l.view && format == video::ImageFormat::Exr)
                .and_then(|l| l.raw())
            {
                // The view shows the denoised mean; the EXR keeps the plain one.
                video::save_linear(&path, &raw, g.width, g.height)
            } else {
                unsafe { video::save_current(g, format, self.ui.jpeg_quality, &path) }
            };
            match saved {
                Ok(()) => {
                    self.ui.toast_file(
                        format!(
                            "Saved {} ({}, {} spp)",
                            path.display(),
                            format.label(),
                            samples
                        ),
                        path,
                    );
                    if let Some(st) = &mut self.stats {
                        st.images_saved += 1;
                    }
                }
                Err(e) => self.ui.error(format!("{e:#}")),
            }
        }
        // The high-resolution still is a one-frame export job (UI phase 4): the window stays
        // responsive, with progress and Cancel.
        if self.actions.high_res {
            self.actions.high_res = false;
            let (w, h) = (g.width, g.height);
            let factor = (self.ui.export_scale as f32).min(8192. / w.max(h) as f32);
            let size = [w, h].map(|v| ((v as f32 * factor).round() as u32).max(1));
            let format = self.ui.export_format;
            let mut o = video::Options::turntable(
                s,
                size,
                1,
                self.ui.export_samples,
                format.file(&format!(
                    "{}-high-res",
                    self.ui
                        .export_stem(&self.settings, self.ui.export_samples, size[0], size[1])
                )),
            );
            o.still = true;
            o.degrees = 0.;
            o.format = video::Format::Png;
            o.image = format;
            o.jpeg_quality = self.ui.jpeg_quality;
            self.ui.video_pending = Some(o);
        }
        let cli_turntable = if self.rendered == 2 {
            self.turntable_cli.take()
        } else {
            None
        };
        if let Some(mut t) = cli_turntable {
            if t.width == 0 {
                [t.width, t.height] = [g.width, g.height];
            }
            let text = unsafe { video::render(g, s, t)? };
            println!("TURNTABLE {text}");
            self.ui.toast(text);
        }
        if self.actions.export_video {
            self.actions.export_video = false;
            self.ui.video_pending = Some(self.ui.video.clone());
        }
        if (self.export.is_some() || self.cpu_export.is_some())
            && self.ui.video_pending.take().is_some()
        {
            self.ui.toast("An export is already running");
        }
        if self.cpu_export.as_ref().is_some_and(|j| j.finished()) {
            let job = self.cpu_export.take().unwrap();
            let out = job.options.output();
            if let Some(live) = &self.cpu_reference {
                live.set_paused(false);
            }
            match job.join() {
                Ok(text) => {
                    println!("EXPORT {text}");
                    self.ui.toast_file(text, out);
                }
                Err(e) => self.ui.error(format!("Export: {e:#}")),
            }
        }
        // The software view exports on the CPU renderer (its own thread), the viewport's
        // render paused meanwhile so the two don't share the cores.
        if self.cpu_view
            && let Some(options) = self.ui.video_pending.take()
        {
            self.drag = None;
            self.keys_down.clear();
            self.ui.video_preview = None;
            if let Some(live) = &self.cpu_reference {
                live.set_paused(true);
            }
            self.cpu_export = Some(video::CpuExport::start(s, options));
        }
        if let Some(options) = self.ui.video_pending.take() {
            match unsafe { video::Job::start(g, s, options) } {
                Ok(job) => {
                    // Input is ignored while it runs, so nothing may stay held.
                    self.drag = None;
                    self.keys_down.clear();
                    self.ui.video_preview = None;
                    self.export = Some(job);
                }
                Err(e) => self.ui.error(format!("Export: {e:#}")),
            }
        }
        if self.debug {
            let messages = unsafe { g.debug_messages() };
            for msg in &messages {
                eprintln!("validation: {msg}");
            }
            anyhow::ensure!(
                messages.is_empty(),
                "GPU validation produced {} warning/error messages",
                messages.len()
            );
        }
        if !self.held {
            self.rendered += 1;
            if self.rendered == 1 {
                startup("first-frame");
            }
        }
        if let Some(dir) = &self.capture_sequence
            && self
                .frames
                .is_some_and(|end| self.rendered > end.saturating_sub(32))
        {
            std::fs::create_dir_all(dir)?;
            unsafe {
                g.capture(&dir.join(format!("{:05}.png", self.rendered)), false, false)?;
            }
        }
        window.set_title(&format!(
            "Caustica · {}{} · {} · {} spp · {:.2} ms GPU · {}",
            self.settings.scene_name,
            if self.ui.scene_dirty { " *" } else { "" },
            g.adapter_name,
            match &self.cpu_reference {
                Some(live) if live.view => live.state().samples,
                _ => g.samples,
            },
            g.gpu_ms,
            if self.cpu_view {
                "CPU"
            } else if g.rr_active {
                "DLSS RR"
            } else if s.denoise {
                "À-trous"
            } else {
                "Raw"
            }
        ));
        Ok(())
    }
    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() || self.frames.is_some_and(|f| self.rendered >= f) {
            return;
        }
        if self.script.is_some() && !self.poll_script(event_loop) {
            return;
        }
        let result = self.redraw();
        // A device removal surfaces from any GPU call: the error names why (and, in debug runs,
        // DRED's last GPU operations); a self-test failure also names its frame and scene.
        let result = result.map_err(|e| {
            let e = match self.gpu.as_ref().and_then(|g| g.removed_reason()) {
                Some(r) => e.context(format!("Device removed: {r}")),
                None => e,
            };
            if self.self_test {
                e.context(format!(
                    "Self-test frame {} ({})",
                    self.rendered, self.settings.scene_name
                ))
            } else {
                e
            }
        });
        if let Err(e) = result {
            self.fail(event_loop, e);
            return;
        }
        if let Some(sc) = &mut self.script {
            sc.frame_done();
        }
        let (gpu_ms, pass_ms) = {
            let g = self.gpu.as_ref().unwrap();
            (g.gpu_ms, g.pass_ms)
        };
        match &mut self.timing {
            Some((_, sum, _, passes)) => {
                *sum += gpu_ms;
                for (p, v) in passes.iter_mut().zip(pass_ms) {
                    *p += v;
                }
            }
            None if self.rendered == 10 => {
                let wait = self.gpu.as_ref().unwrap().fence_wait_ms;
                self.timing = Some((Instant::now(), 0., wait, [0.; 4]))
            }
            None => {}
        }
        if self.frames.is_some_and(|f| self.rendered >= f) {
            let wait_now = self.gpu.as_ref().unwrap().fence_wait_ms;
            let (frame_ms, mean_gpu_ms, wait_ms, pass_mean) = match self.timing {
                Some((start, sum, wait, passes)) if self.rendered > 10 => {
                    let n = (self.rendered - 10) as f64;
                    (
                        start.elapsed().as_secs_f64() * 1000. / n,
                        sum / n,
                        (wait_now - wait) / n,
                        passes.map(|p| p / n),
                    )
                }
                _ => (0., 0., 0., [0.; 4]),
            };
            // The software view's EXR is the CPU's mean: the GPU's accumulation holds its one pass.
            let cpu_exr = self
                .capture
                .as_deref()
                .filter(|p| video::ImageFormat::from_path(p) == Some(video::ImageFormat::Exr))
                .and_then(|p| self.cpu_capture(p));
            let g = self.gpu.as_mut().unwrap();
            if let Some(path) = &self.capture {
                let hdr = path.extension().is_some_and(|v| v == "exr");
                // name.hdr.png: the HDR screen image as a PQ PNG (needs HDR output).
                let name = path.to_string_lossy().to_lowercase();
                let hdr_screen = if name.ends_with(".hdr.png") {
                    Some((".hdr.png", video::ImageFormat::HdrPng))
                } else if name.ends_with(".jxr") {
                    Some((".jxr", video::ImageFormat::Jxr))
                } else {
                    None
                };
                let result = if let Some(saved) = cpu_exr {
                    saved
                } else if let Some((suffix, format)) = hdr_screen {
                    if g.hdr_active {
                        unsafe { video::save_current(g, format, video::JPEG_QUALITY, path) }
                    } else {
                        Err(anyhow::anyhow!(
                            "--capture *{suffix} needs HDR output (--set hdr_mode=2)"
                        ))
                    }
                } else {
                    unsafe { g.capture(path, hdr, self.capture_ui && !hdr) }.and_then(|_| {
                        if self.capture_ui {
                            Ok(())
                        } else {
                            video::tag_capture(path, g.width, g.height)
                        }
                    })
                };
                if let Err(e) = result {
                    self.fail(event_loop, e);
                    return;
                }
            }
            if let Some(live) = self.cpu_reference.as_ref().filter(|l| l.view) {
                let st = live.state();
                if let Some(e) = st.error {
                    self.fail(event_loop, anyhow::anyhow!(e));
                    return;
                }
                println!(
                    "CPU_VIEW samples={} seconds={:.1} threads={} images={} missing={:?}",
                    st.samples, st.seconds, st.threads, st.images, st.missing
                );
            }
            println!(
                "RENDER_OK frames={} samples={} gpu_ms={:.3} rr={} size={}x{} gpu_ms_mean={:.3} frame_ms_mean={:.3} fence_wait_ms_mean={:.3} trace_ms_mean={:.3} reconstruct_ms_mean={:.3} display_ms_mean={:.3} ui_ms_mean={:.3}",
                self.rendered,
                g.samples,
                g.gpu_ms,
                g.rr_active,
                g.width,
                g.height,
                mean_gpu_ms,
                frame_ms,
                wait_ms,
                pass_mean[0],
                pass_mean[1],
                pass_mean[2],
                pass_mean[3]
            );
            event_loop.exit();
        }
    }
}
/// A slider that only clamps what the user drags. egui's default clamps the stored value
/// whenever the slider is drawn, which would silently rewrite valid values loaded from a file
/// or `--set` that lie outside the slider's convenient range (and restart the render).
///
/// Double-clicking it resets the value from a registered reference struct (`reset_source`):
/// the field at the same offset in the scene as loaded, or in a default `UiState`.
fn slider<'a, N: egui::emath::Numeric>(
    value: &'a mut N,
    range: std::ops::RangeInclusive<N>,
) -> Slider<'a> {
    let target = value as *mut N;
    let reset: Box<dyn FnOnce() -> bool + 'a> = Box::new(move || {
        // SAFETY: `target` comes from `value`, borrowed for 'a, and the slider (which also
        // writes through it) has finished by the time this runs.
        match unsafe { reset_value::<N>(target as *const N) } {
            Some(v) => {
                unsafe { *target = v };
                true
            }
            None => false,
        }
    });
    // Modified: the value differs from the reset reference (the scene as loaded).
    let modified: Box<dyn Fn() -> bool + 'a> = Box::new(move || {
        // SAFETY: as above.
        unsafe { reset_value::<N>(target as *const N) }
            .is_some_and(|r| differs(unsafe { *target }, r))
    });
    let range = range.start().to_f64()..=range.end().to_f64();
    let inner = egui::Slider::from_get_set(range, move |v: Option<f64>| {
        // SAFETY: as above; egui calls this only while the slider is being shown.
        unsafe {
            if let Some(v) = v {
                *target = N::from_f64(v);
            }
            (*target).to_f64()
        }
    })
    .clamping(egui::SliderClamping::Edits);
    Slider {
        inner: if N::INTEGRAL { inner.integer() } else { inner },
        reset,
        modified,
    }
}
/// `egui::Slider` plus a double-click reset and a "modified" dot (see `slider`).
pub struct Slider<'a> {
    inner: egui::Slider<'a>,
    reset: Box<dyn FnOnce() -> bool + 'a>,
    modified: Box<dyn Fn() -> bool + 'a>,
}
impl<'a> Slider<'a> {
    pub fn suffix(mut self, s: impl ToString) -> Self {
        self.inner = self.inner.suffix(s);
        self
    }
    pub fn logarithmic(mut self, on: bool) -> Self {
        self.inner = self.inner.logarithmic(on);
        self
    }
    pub fn smallest_positive(mut self, v: f64) -> Self {
        self.inner = self.inner.smallest_positive(v);
        self
    }
    pub fn fixed_decimals(mut self, n: usize) -> Self {
        self.inner = self.inner.fixed_decimals(n);
        self
    }
    pub fn custom_formatter(
        mut self,
        f: impl 'a + Fn(f64, std::ops::RangeInclusive<usize>) -> String,
    ) -> Self {
        self.inner = self.inner.custom_formatter(f);
        self
    }
    pub fn custom_parser(mut self, f: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.inner = self.inner.custom_parser(f);
        self
    }
}
impl egui::Widget for Slider<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut response = ui.add(self.inner);
        // A dot after a value that differs from the scene as loaded; clicking it reverts.
        // The space is always taken, so the row does not shift when the dot appears.
        let mut reset = slider_double_clicked(ui, &response);
        reset |= modified_dot(ui, response.rect.height(), (self.modified)());
        if reset && (self.reset)() {
            response.mark_changed();
        }
        if ui::slider_in_row(ui.ctx()) {
            response
        } else {
            response.on_hover_text_at_pointer("Double-click to reset")
        }
    }
}
/// The dot after a modified value (its space is always taken); true when it was clicked.
fn modified_dot(ui: &mut egui::Ui, height: f32, modified: bool) -> bool {
    let (rect, dot) = ui.allocate_exact_size(egui::vec2(8., height), egui::Sense::click());
    if !modified {
        return false;
    }
    let color = if dot.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui::ACCENT
    };
    ui.painter().circle_filled(rect.center(), 2.5, color);
    ui::row_tip_yields(&dot);
    dot.on_hover_text("Changed from the scene as loaded. Click to revert.")
        .clicked()
}
/// True when `value` differs from its reset reference by more than rounding.
fn differs<N: egui::emath::Numeric>(value: N, reference: N) -> bool {
    let (v, r) = (value.to_f64(), reference.to_f64());
    (v - r).abs() > 1e-6 * r.abs().max(1.)
}
/// The slider's "modified" dot for a group of `DragValue`s (a position, say): it shows when
/// any of `values` differs from the scene as loaded, and a click reverts them all. They get no
/// double-click, since their first click starts a text edit that would write the old text back.
pub fn revert_dot<N: egui::emath::Numeric>(ui: &mut egui::Ui, values: &mut [N]) {
    // SAFETY: the values are live fields of the settings being drawn, which `reset_source`
    // registered for this frame.
    let reference = |v: &N| unsafe { reset_value::<N>(v as *const N) };
    let modified = values
        .iter()
        .any(|v| reference(v).is_some_and(|r| differs(*v, r)));
    if modified_dot(ui, ui.spacing().interact_size.y, modified) {
        for v in values {
            if let Some(r) = reference(v) {
                *v = r;
            }
        }
    }
}
/// A slider senses drags, so egui never reports it as double-clicked: read the pointer.
pub fn slider_double_clicked(ui: &egui::Ui, response: &egui::Response) -> bool {
    (response.hovered() || response.is_pointer_button_down_on())
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
}
thread_local! {
    /// (live address, size, reference address) for the current frame's UI.
    static RESET_SOURCES: std::cell::RefCell<Vec<(usize, usize, usize)>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// Makes sliders on fields of `live` reset to the same field of `reference` (same type, so
/// the same layout). `reference` must stay alive and unmoved until `clear_reset_sources`.
pub fn reset_source<T>(live: &T, reference: &T) {
    RESET_SOURCES.with_borrow_mut(|v| {
        v.push((
            live as *const T as usize,
            size_of::<T>(),
            reference as *const T as usize,
        ))
    });
}
pub fn clear_reset_sources() {
    RESET_SOURCES.with_borrow_mut(Vec::clear);
}
/// The reference value for a numeric field, from the innermost registered source holding it.
///
/// # Safety
/// Every registered reference must still be alive, as `reset_source` requires.
unsafe fn reset_value<N: Copy>(field: *const N) -> Option<N> {
    let at = field as usize;
    RESET_SOURCES.with_borrow(|sources| {
        sources
            .iter()
            .filter(|(start, size, _)| at >= *start && at + size_of::<N>() <= start + size)
            .min_by_key(|(_, size, _)| *size)
            .map(|(start, _, reference)| unsafe {
                std::ptr::read_unaligned((reference + (at - start)) as *const N)
            })
    })
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            let window = Arc::new(
                event_loop.create_window(
                    platform::with_app_id(Window::default_attributes())
                        .with_title(format!("Caustica {VERSION}"))
                        // The icon build.rs linked in (resource 1); none if rc.exe was missing.
                        .with_window_icon(app_icon(None))
                        .with_taskbar_icon(app_icon(Some(256)))
                        .with_inner_size(winit::dpi::PhysicalSize::new(self.size[0], self.size[1]))
                        .with_visible(!self.hidden)
                        .with_fullscreen(
                            self.saver
                                .as_ref()
                                .map(|_| winit::window::Fullscreen::Borderless(None)),
                        ),
                )?,
            );
            if self.saver.is_some() {
                window.set_cursor_visible(false);
                self.show_ui = false;
            }
            let size = window.inner_size();
            startup("window");
            #[cfg(windows)]
            let (hwnd, mut gpu) = {
                let hwnd = match window.window_handle()?.as_raw() {
                    RawWindowHandle::Win32(h) => {
                        windows::Win32::Foundation::HWND(h.hwnd.get() as _)
                    }
                    _ => bail!("Windows only"),
                };
                // Menu → Renderer, for windowed runs (headless runs and the self-test take only
                // the flags, so they stay reproducible).
                let from_prefs = !self.vulkan
                    && !self.d3d12
                    && !self.hidden
                    && !self.self_test
                    && prefs::Prefs::load().vulkan;
                self.vulkan |= from_prefs;
                let made = unsafe {
                    Gpu::new(
                        &window,
                        size.width,
                        size.height,
                        &self.settings,
                        self.debug,
                        self.vulkan,
                    )
                };
                let gpu = match made {
                    // A Vulkan picked in the menu that can't start falls back to D3D12.
                    Err(e) if from_prefs => {
                        eprintln!("Vulkan did not start ({e:#}); using D3D12");
                        self.vulkan = false;
                        self.vulkan_failed = Some(format!("{e:#}"));
                        unsafe {
                            Gpu::new(
                                &window,
                                size.width,
                                size.height,
                                &self.settings,
                                self.debug,
                                false,
                            )?
                        }
                    }
                    made => made?,
                };
                (Some(hwnd), gpu)
            };
            #[cfg(not(windows))]
            let (hwnd, mut gpu) = (None, unsafe {
                Gpu::new(&window, size.width, size.height, &self.settings, self.debug)?
            });
            gpu.keep_ui_frame = self.capture_ui;
            // A GPU that can't ray trace still displays: the CPU renders the view.
            if !gpu.ray_tracing {
                let why = if vulkan::rt_disabled() {
                    "CAUSTICA_NO_RT=1 forces the display-only path: the view renders on the CPU \
                     (software)"
                        .to_string()
                } else {
                    format!(
                        "{} can't ray trace: the view renders on the CPU (software)",
                        gpu.adapter_name
                    )
                };
                eprintln!("{why}");
                self.ui.toast(why);
                self.cpu_view = true;
            }
            // Windowed runs start drawing while DLSS starts; headless runs, the self-test and
            // --require-rr need it from the first frame.
            if self.hidden || self.self_test || self.require_rr {
                gpu.wait_ngx();
            }
            startup("gpu");
            self.hwnd = hwnd;
            if cfg!(debug_assertions) && !self.self_test {
                #[cfg(windows)]
                let spirv = gpu.is_vulkan();
                #[cfg(not(windows))]
                let spirv = true;
                self.hot = hotreload::HotReload::new(spirv);
            }
            if !self.hidden {
                self.stats = Some(stats::Stats::load());
                self.ui.prefs = prefs::Prefs::load();
                self.ui.prefs_live = true;
                self.ui.advanced |= self.ui.prefs.advanced;
                // Where the UI was left, unless the command line chose a tab or tool.
                if !self.ui.tab_from_args {
                    self.ui.tab = self.ui.prefs.tab;
                }
                if !self.ui.tool_from_args {
                    self.ui.tool = self.ui.prefs.tool;
                }
                self.ui.local = self.ui.prefs.local;
                self.ui.pivot = self.ui.prefs.pivot;
                self.ui.snap_to = self.ui.prefs.snap_to;
                self.ui.panel_width = self.ui.prefs.panel_width;
                // Undo for panel edits and the last session: not for frame-limited or scripted
                // runs or the screensaver (nor headless runs and the self-test, above).
                if self.frames.is_none() && self.script.is_none() && self.saver.is_none() {
                    self.ui.start_edit_tracking(&self.settings);
                }
                self.egui.set_zoom_factor(self.ui.prefs.zoom);
                self.gamepad = gilrs::Gilrs::new().ok();
                if let Some(from) = std::env::var_os(update::UPDATED_FROM) {
                    println!("UPDATED from {} to {VERSION}", from.to_string_lossy());
                    self.ui.toast(format!("Updated to {VERSION}"));
                    // A later Menu → Restart must not announce it again.
                    unsafe { std::env::remove_var(update::UPDATED_FROM) };
                }
                // Only interactive, installed runs ever look: not a build from source, a
                // headless, scripted or frame-limited run, the screensaver or the self-test.
                if let Some(source) = &self.update_from {
                    self.updater.offer_from(source);
                } else if self.ui.prefs.update_check
                    && self.frames.is_none()
                    && self.script.is_none()
                    && self.saver.is_none()
                    && dirs::mode() == dirs::Mode::Installed
                    && !VERSION.contains('-')
                {
                    self.updater.check();
                }
            }
            let input = egui_winit::State::new(
                self.egui.clone(),
                egui::ViewportId::ROOT,
                &*window,
                Some(window.scale_factor() as f32),
                None,
                None,
            );
            self.input = Some(input);
            self.gpu = Some(gpu);
            self.window = Some(window);
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(event_loop, e);
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if std::mem::take(&mut self.ui.install_update) {
            // Saved before the new process can read them; it stays running if that can't start.
            let stats = self.stats.take();
            if let Some(st) = &stats {
                st.save();
            }
            match self.updater.install() {
                Ok(()) => self.shut_down(event_loop),
                Err(e) => {
                    self.stats = stats;
                    self.updater.install_failed(format!("{e:#}"));
                }
            }
            return;
        }
        if std::mem::take(&mut self.ui.restart) {
            // Menu → Renderer → Restart now: the same command line, started fresh.
            // The same cleanup as closing the window (stats, a running export), then the child.
            self.shut_down(event_loop);
            let started = std::env::current_exe().and_then(|exe| {
                std::process::Command::new(exe)
                    .args(
                        std::env::args_os()
                            .skip(1)
                            .filter(|a| a != "--vulkan" && a != "--d3d12"),
                    )
                    .spawn()
            });
            if let Err(e) = started {
                eprintln!("Restart: {e}");
            }
            return;
        }
        // Wayland can't hide a window: a hidden run must not react to the user's pointer or keys
        // (scripted input goes straight to egui, not through here).
        if self.hidden
            && matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::Touch(_)
                    | WindowEvent::KeyboardInput { .. }
                    | WindowEvent::ModifiersChanged(_)
            )
        {
            return;
        }
        let is_redraw = matches!(event, WindowEvent::RedrawRequested);
        let consumed = if let (Some(input), Some(window)) = (&mut self.input, &self.window) {
            input.on_window_event(window, &event).consumed
        } else {
            false
        };
        if let Some(sv) = &mut self.saver {
            // Windows sends stray events while a fullscreen window takes focus (synthetic keys,
            // an AltGr press on some layouts), so input counts after a short grace period and
            // modifier keys never do.
            let settled = sv.started.elapsed() > Duration::from_millis(1500);
            let quit = settled
                && match &event {
                    WindowEvent::KeyboardInput {
                        event,
                        is_synthetic: false,
                        ..
                    } => !matches!(
                        event.physical_key,
                        PhysicalKey::Code(
                            KeyCode::AltLeft
                                | KeyCode::AltRight
                                | KeyCode::ControlLeft
                                | KeyCode::ControlRight
                                | KeyCode::ShiftLeft
                                | KeyCode::ShiftRight
                                | KeyCode::SuperLeft
                                | KeyCode::SuperRight
                        )
                    ),
                    WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } => true,
                    WindowEvent::CursorMoved { position, .. } => sv.moved([position.x, position.y]),
                    _ => false,
                };
            if quit {
                event_loop.exit();
                return;
            }
        }
        // A video export takes no viewport or shortcut input: the scene must not change under it.
        if self.export.is_some()
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::Moved(_)
                    | WindowEvent::ScaleFactorChanged { .. }
                    | WindowEvent::RedrawRequested
                    | WindowEvent::ModifiersChanged(_)
                    | WindowEvent::CursorMoved { .. }
            )
        {
            return;
        }
        match event {
            // A close would throw the running export away, and no undo brings that back: the
            // export window asks for a second one.
            WindowEvent::CloseRequested
                if self.export.is_some()
                    && self
                        .ui
                        .close_armed
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(6)) =>
            {
                self.ui.close_armed = Some(Instant::now());
            }
            WindowEvent::CloseRequested => self.shut_down(event_loop),
            WindowEvent::Resized(size) => {
                if size.width > 0
                    && size.height > 0
                    && let Some(g) = &mut self.gpu
                    && let Err(e) = unsafe {
                        if self.export.is_some() {
                            g.resize_swap(size.width, size.height)
                        } else {
                            g.resize(size.width, size.height, &self.settings)
                        }
                    }
                {
                    self.fail(event_loop, e);
                }
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.display_dirty = true;
            }
            WindowEvent::RedrawRequested => {
                if !self.hidden {
                    self.tick(event_loop);
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.shift = m.state().shift_key();
                self.ctrl = m.state().control_key();
                self.alt = m.state().alt_key();
                if self.ui.transform.is_some() {
                    self.update_transform();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let transforming = self.ui.transform.as_ref().map(|t| t.from_handle);
                let armed = state == ElementState::Pressed
                    && button == MouseButton::Left
                    && std::mem::take(&mut self.ui.box_armed);
                if state == ElementState::Released {
                    // A grabbed body is thrown with the velocity the spring gave it; a click
                    // without a drag still selects below.
                    if button == MouseButton::Left {
                        self.physics_release();
                    }
                    // A box drag selects what it encloses; a short one is an ordinary click.
                    let boxed = button == MouseButton::Left
                        && self.ui.box_start.take().is_some_and(|a| {
                            self.drag_distance >= 3. && {
                                self.box_select(a);
                                true
                            }
                        });
                    if boxed {
                    } else if transforming == Some(true) && button == MouseButton::Left {
                        self.finish_transform();
                    } else if self.drag == Some(MouseButton::Left)
                        && self.drag_distance < 3.
                        && (self.ui.paths.open || self.alt)
                        && !self.egui.is_pointer_over_area()
                    {
                        // Path visualizer (track 14): trace the pixel's paths instead of picking.
                        self.ui.paths.request([
                            self.cursor[0].max(0.) as u32,
                            self.cursor[1].max(0.) as u32,
                        ]);
                    } else if self.drag == Some(MouseButton::Left)
                        && self.drag_distance < 3.
                        && self.settings.perspective()
                        && !self.egui.is_pointer_over_area()
                    {
                        // A click selects what is under the pointer, read from the GPU's
                        // object-id buffer next frame; empty space deselects, as in Blender.
                        // A second click close by soon after also frames it.
                        let now = Instant::now();
                        self.frame_pick = self.last_click.is_some_and(|(at, p)| {
                            now - at < Duration::from_millis(400)
                                && (p[0] - self.cursor[0]).abs() + (p[1] - self.cursor[1]).abs()
                                    < 6.
                        });
                        self.last_click = (!self.frame_pick).then_some((now, self.cursor));
                        self.pick_point = Some(self.cursor);
                        self.pick_toggle = self.shift;
                    } else if self.drag == Some(MouseButton::Left)
                        && self.drag_distance < 3.
                        && !self.settings.perspective()
                        && self.walking.is_none()
                        && !self.egui.is_pointer_over_area()
                    {
                        self.ui.toast(ui::PICKING_OFF);
                    } else if self.drag == Some(MouseButton::Right)
                        && self.drag_distance < 3.
                        && self.shift
                        && self.settings.perspective()
                        && !self.egui.is_pointer_over_area()
                    {
                        // Shift+right-click places the 3D cursor, as in Blender.
                        self.cursor_pick = Some(self.cursor);
                    } else if self.drag == Some(MouseButton::Right)
                        && self.drag_distance < 3.
                        && !self.egui.is_pointer_over_area()
                    {
                        // A right click selects what is under the pointer (when the camera
                        // allows picking) and opens the menu there.
                        if self.settings.perspective() {
                            self.pick_point = Some(self.cursor);
                            self.pick_toggle = false;
                        }
                        let ppp = self.egui.pixels_per_point() as f64;
                        self.ui.context_menu = Some(egui::pos2(
                            (self.cursor[0] / ppp) as f32,
                            (self.cursor[1] / ppp) as f32,
                        ));
                    }
                    self.drag = None;
                } else if transforming == Some(false) {
                    // Modal G / R / S: left click confirms, right click cancels.
                    if button == MouseButton::Right {
                        self.cancel_transform();
                    } else {
                        self.finish_transform();
                    }
                } else if !consumed && button == MouseButton::Left && self.ctrl {
                    // Ctrl+click: focus the lens on what is under the pointer.
                    self.focus_pending = true;
                } else if !consumed
                    && button == MouseButton::Left
                    && self.ui.hover.is_none()
                    && self.settings.perspective()
                    && (armed
                        || (self.ui.tool == gizmo::Tool::Select
                            && !self.shift
                            && !self.alt
                            && !self.settings.physics.enabled))
                {
                    // Box select (B, or a drag with the Select tool); a click still picks.
                    let ppp = self.egui.pixels_per_point() as f64;
                    self.ui.box_start = Some(egui::pos2(
                        (self.cursor[0] / ppp) as f32,
                        (self.cursor[1] / ppp) as f32,
                    ));
                    self.drag = Some(button);
                    self.drag_distance = 0.;
                } else if !consumed
                    && button == MouseButton::Left
                    && self.walking.is_none()
                    && self.physics_press()
                {
                    // Track 17: grabbed a body; the drag moves it instead of the camera, and a
                    // release without moving still counts as a click.
                    self.drag = Some(button);
                    self.drag_distance = 0.;
                } else if !consumed {
                    let mode = self.tool_mode();
                    // Alt-drag on a handle: copies in place, then the drag moves the copies.
                    let mut copied = false;
                    if button == MouseButton::Left
                        && self.ui.hover.is_some()
                        && self.alt
                        && let Some(name) = self.ui.duplicate_selected(&mut self.settings, false)
                    {
                        self.ui.toast(format!("Duplicated: {name}"));
                        copied = true;
                    }
                    if button == MouseButton::Left
                        && let Some(h) = self.ui.hover
                        && self.start_transform(mode, Some(h))
                    {
                        if copied && let Some(t) = &mut self.ui.transform {
                            t.undo_on_cancel = true;
                        }
                        self.drag = None;
                    } else {
                        self.drag = Some(button);
                        self.drag_distance = 0.;
                    }
                }
            }
            // A box drag whose release went to another window ends here.
            WindowEvent::Focused(false) => {
                self.ui.box_start = None;
                self.ui.box_armed = false;
                self.drag = None;
                // A dialog or an alt-tab takes the key-ups with it: nothing may stay held.
                // (The self-test holds its keys by hand.)
                if !self.self_test {
                    self.keys_down.clear();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let dx = (position.x - self.cursor[0]) as f32;
                let dy = (position.y - self.cursor[1]) as f32;
                self.cursor = [position.x, position.y];
                if self.physics_drag() {
                    self.drag_distance += dx.abs() + dy.abs();
                } else if let Some(t) = &mut self.ui.transform {
                    // Shift moves the pointer's effect at a tenth of the speed.
                    let k = if self.shift { 0.1 } else { 1. };
                    t.mouse += glam::Vec2::new(dx, dy) * k;
                    t.pointer = glam::Vec2::new(position.x as f32, position.y as f32);
                    self.update_transform();
                } else if self.ui.box_start.is_some() {
                    // Drawing a selection box: the camera stays put.
                    self.drag_distance += dx.abs() + dy.abs();
                } else if self.walking.is_some() && self.drag == Some(MouseButton::Left) {
                    self.drag_distance += dx.abs() + dy.abs();
                    walk::Walk::look(&mut self.settings, dx, dy);
                } else if let Some(button) = self.drag {
                    self.drag_distance += dx.abs() + dy.abs();
                    let s = &mut self.settings;
                    if matches!(button, MouseButton::Right | MouseButton::Middle) || self.shift {
                        // Pan along the camera's screen axes, so the scene follows the
                        // cursor at any pitch.
                        let scale = s.distance * 0.001;
                        let (sy, cy) = s.yaw.to_radians().sin_cos();
                        let (sp, cp) = s.pitch.to_radians().sin_cos();
                        let right = [cy, 0., -sy];
                        let up = [-sp * sy, cp, -sp * cy];
                        for i in 0..3 {
                            s.target[i] = (s.target[i] + (dy * up[i] - dx * right[i]) * scale)
                                .clamp(-20., 20.);
                        }
                    } else if button == MouseButton::Left {
                        s.yaw += dx * 0.25;
                        s.pitch = (s.pitch + dy * 0.25).clamp(-85., 85.);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !consumed {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.,
                    };
                    if let Some(w) = &mut self.walking {
                        // Walking, the wheel sizes the body instead of zooming.
                        w.zoom(d);
                    } else if self.physics_wheel(d) {
                        // Holding a body: the wheel sets its distance.
                    } else {
                        self.settings.distance =
                            (self.settings.distance * (-d * 0.1).exp()).clamp(0.5, 200.);
                    }
                }
            }
            // Walk mode owns WASD / Q / E while it runs; held keys are tracked for movement, and a
            // release always counts, or a key let go in a text field would keep walking.
            WindowEvent::KeyboardInput { ref event, .. }
                if self.walking.is_some()
                    && (self.keys_free() || event.state == ElementState::Released) =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys_down.insert(code);
                        if !event.repeat && matches!(code, KeyCode::Escape | KeyCode::Backquote) {
                            self.walking = None;
                            self.keys_down.clear();
                            self.ui.toast("Walk mode off");
                        } else if !event.repeat
                            && self.ui.transform.is_none()
                            && self.global_key(code)
                        {
                            // Ctrl+Z, Y or O are no movement, F12, F1 and Tab no held keys.
                            self.keys_down.remove(&code);
                        } else if !event.repeat
                            && let Some(w) = &mut self.walking
                        {
                            match code {
                                KeyCode::KeyF => {
                                    w.toggle_fly();
                                    self.ui.toast(if w.fly {
                                        "Fly: Q / E down / up (F to walk)"
                                    } else {
                                        "Walking (F to fly)"
                                    });
                                }
                                KeyCode::KeyB => {
                                    w.bob = !w.bob;
                                    self.ui.toast(if w.bob {
                                        "Head bob on"
                                    } else {
                                        "Head bob off"
                                    });
                                }
                                KeyCode::KeyT => self.toggle_flashlight(),
                                _ => {}
                            }
                        }
                    } else {
                        self.keys_down.remove(&code);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat && self.keys_free() =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if self.transform_key(code) {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    let selected = !self.ui.targets().is_empty();
                    let over_view = !self.egui.is_pointer_over_area();
                    match code {
                        code if self.global_key(code) => {}
                        KeyCode::KeyD if self.ctrl && selected => {
                            self.run_command(ui::Command::Duplicate);
                        }
                        // Blender's Shift+D: duplicate, then move the copy (only when a copy was
                        // made: the main light has none, and the object list can be full).
                        KeyCode::KeyD if self.shift && selected && over_view => {
                            let before = self.ui.selection;
                            self.run_command(ui::Command::Duplicate);
                            if self.ui.selection != before {
                                self.run_command(ui::Command::Move);
                                if let Some(t) = &mut self.ui.transform {
                                    t.undo_on_cancel = true;
                                }
                            }
                        }
                        // Ctrl+D or Shift+D with nothing to copy: not the denoiser (D).
                        KeyCode::KeyD if self.ctrl || self.shift => {}
                        KeyCode::KeyH if self.alt => self.run_command(ui::Command::ShowAll),
                        KeyCode::KeyH if selected => self.run_command(ui::Command::Hide),
                        KeyCode::Home => self.reset_camera(),
                        KeyCode::KeyI if self.alt => self.delete_keys(),
                        KeyCode::KeyI if !self.ctrl => self.key_selection(self.shift),
                        KeyCode::KeyK if !self.ctrl => self.toggle_playback(),
                        KeyCode::KeyT if !self.ctrl => self.toggle_flashlight(),
                        KeyCode::ArrowLeft if self.shift => self.step_frame(-1),
                        KeyCode::ArrowRight if self.shift => self.step_frame(1),
                        KeyCode::KeyS if self.ctrl => self.save_scene(self.shift),
                        KeyCode::PageUp => self.step_scene(-1),
                        KeyCode::PageDown => self.step_scene(1),
                        // Blender's modal transforms, when an object or light is selected.
                        KeyCode::KeyG | KeyCode::KeyS | KeyCode::KeyR
                            if selected && over_view && !self.ctrl =>
                        {
                            let mode = match code {
                                KeyCode::KeyG => gizmo::Mode::Move,
                                KeyCode::KeyR => gizmo::Mode::Rotate,
                                _ => gizmo::Mode::Scale,
                            };
                            self.start_transform(mode, None);
                        }
                        // The same keys away from the viewport say so: R must not throw the
                        // samples away while something is selected.
                        KeyCode::KeyG | KeyCode::KeyS | KeyCode::KeyR if selected && !self.ctrl => {
                            self.ui
                                .toast("Point at the viewport to move, rotate or scale");
                        }
                        KeyCode::KeyG | KeyCode::KeyS if !self.ctrl => {
                            self.ui.toast("Select an object or light first");
                        }
                        // Escape closes the scene browser, else deselects; closing the window is
                        // the only way to quit.
                        KeyCode::Escape => {
                            if self.ui.context_menu.take().is_none()
                                && !std::mem::take(&mut self.ui.box_armed)
                            {
                                if self.ui.show_scenes {
                                    self.ui.show_scenes = false;
                                } else {
                                    self.ui.select(ui::Selection::None);
                                }
                            }
                        }
                        // Multi-select (3b): B boxes, Ctrl+A selects all, Alt+A none.
                        KeyCode::KeyB if over_view && self.settings.perspective() => {
                            self.ui.box_armed = true;
                            self.ui.toast("Box select: drag a rectangle (Shift adds)");
                        }
                        KeyCode::KeyB if over_view => self.ui.toast(ui::PICKING_OFF),
                        KeyCode::KeyA if self.ctrl => self.select_all(),
                        KeyCode::KeyA if self.alt => self.ui.select(ui::Selection::None),
                        KeyCode::Space => {
                            self.paused = !self.paused;
                            self.ui
                                .toast(if self.paused { "Paused" } else { "Resumed" });
                        }
                        KeyCode::KeyR => {
                            self.actions.reset = true;
                            self.ui.toast("Restarted");
                        }
                        KeyCode::KeyF => self.frame_selection(),
                        KeyCode::KeyL => self.toggle_painting(),
                        KeyCode::KeyE
                            if self.settings.physics.enabled && self.walking.is_none() =>
                        {
                            self.physics_explode()
                        }
                        KeyCode::KeyN
                            if self.settings.physics.enabled && self.walking.is_none() =>
                        {
                            self.ui.checkpoint(&self.settings);
                            self.physics_spawn(self.shift)
                        }
                        KeyCode::KeyE | KeyCode::KeyN
                            if !self.settings.physics.enabled && over_view && !self.ctrl =>
                        {
                            self.ui.toast("Physics is off (Scene tab, Simulate)");
                        }
                        KeyCode::KeyP if self.ctrl => {
                            self.ui.show_scenes = !self.ui.show_scenes;
                        }
                        KeyCode::KeyP => {
                            self.ui.paths.open = !self.ui.paths.open;
                            self.ui.toast(if self.ui.paths.open {
                                "Path visualizer: click a pixel"
                            } else {
                                "Path visualizer closed"
                            });
                        }
                        KeyCode::Backquote => self.toggle_walk(),
                        KeyCode::Delete => {
                            if let Some(name) = self.ui.delete_selected(&mut self.settings) {
                                self.ui.toast(format!("Deleted {name}"));
                            }
                        }
                        KeyCode::KeyA => {
                            self.settings.rotate = !self.settings.rotate;
                            let on = self.settings.rotate;
                            self.ui.toast(if on {
                                "Auto orbit on"
                            } else {
                                "Auto orbit off"
                            });
                        }
                        KeyCode::KeyD => {
                            self.settings.denoise = !self.settings.denoise;
                            let text = match (self.settings.denoise, self.settings.rr) {
                                (true, false) => "Native denoiser on",
                                (false, false) => "Native denoiser off (raw samples)",
                                (true, true) => {
                                    "Native denoiser on (used when Ray Reconstruction is off)"
                                }
                                (false, true) => {
                                    "Native denoiser off (used when Ray Reconstruction is off)"
                                }
                            };
                            self.ui.toast(text);
                        }
                        KeyCode::KeyC => {
                            self.actions.reference = true;
                            self.ui
                                .toast("Reference frozen: turn on Show split in the Look tab");
                        }
                        KeyCode::Digit1
                        | KeyCode::Digit2
                        | KeyCode::Digit3
                        | KeyCode::Digit4
                        | KeyCode::Digit5
                        | KeyCode::Digit6
                        | KeyCode::Digit7
                        | KeyCode::Digit8 => {
                            let view = [
                                KeyCode::Digit1,
                                KeyCode::Digit2,
                                KeyCode::Digit3,
                                KeyCode::Digit4,
                                KeyCode::Digit5,
                                KeyCode::Digit6,
                                KeyCode::Digit7,
                                KeyCode::Digit8,
                            ]
                            .iter()
                            .position(|k| *k == code)
                            .unwrap_or(0) as u32;
                            self.settings.view = view;
                            self.ui.toast(format!("View: {}", ui::view_name(view)));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if !is_redraw && let Some(w) = &self.window {
            w.request_redraw();
        }
    }
    /// What talks to the compositor goes before the event loop closes its connection, or it
    /// crashes on Wayland: the swap chain and surface (NVIDIA's vkDestroySwapchainKHR), then the
    /// window, then the clipboard worker inside egui_winit, which shares the connection.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.window = None;
        self.input = None;
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.hidden {
            event_loop.set_control_flow(ControlFlow::Poll);
            self.tick(event_loop);
            return;
        }
        // The software view's GPU has one pass to do per restart, so it doesn't spin the loop.
        let active = self.gpu.as_ref().is_some_and(|g| {
            (!self.cpu_view && !self.paused && g.samples < self.settings.max_samples)
                || (self.cpu_view && g.samples == 0)
                || self.frames.is_some()
                || self.export.is_some()
                || self.cpu_export.is_some()
                || self.timeline.playing
                || self.settings.rotate
                || self.script.is_some()
        });
        // An idle viewport still shows each new pass of the CPU renderer, every 200 ms, or within
        // 33 ms in the software view, whose previews have to keep up with the camera. An image
        // goes up after the frame's render, so the last pass takes one frame more to show.
        let cpu = self.cpu_reference.as_ref().is_some_and(|l| l.busy())
            || std::mem::take(&mut self.cpu_uploaded);
        event_loop.set_control_flow(if active {
            ControlFlow::Poll
        } else if cpu && self.cpu_view {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33))
        } else {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(200))
        });
        // Shader hot reload is polled in redraw: an edit while paused or converged reloads too.
        // (Only when idle: a rendering loop redraws, and polls, anyway.)
        let shaders = !active && self.hot.as_ref().is_some_and(|h| h.changed());
        // The update window follows its background work, also when the render is idle.
        if (active || cpu || shaders || self.updater.busy() || self.ui.install_update)
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }
}
impl App {
    /// Shortcuts run unless a text field holds the keyboard, or a widget has just been given it:
    /// a DragValue's field only appears the frame after its click.
    fn keys_free(&self) -> bool {
        !self.text_focus && self.focus_seen.is_none()
    }
    /// The keys that also work in walk mode, none of which it uses. Ctrl+S is not one: Ctrl and
    /// S crouch and walk back there.
    fn global_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::KeyZ | KeyCode::KeyY if self.ctrl => {
                let redo = code == KeyCode::KeyY || self.shift;
                let done = if redo {
                    self.ui.redo(&mut self.settings)
                } else {
                    self.ui.undo(&mut self.settings)
                };
                // The restored values stay; the keys apply once the playhead moves.
                self.timeline.settle(&self.settings);
                let text = match (redo, done) {
                    (false, true) => "Undo",
                    (true, true) => "Redo",
                    (false, false) => "Nothing to undo",
                    (true, false) => "Nothing to redo",
                };
                self.ui.toast(text);
            }
            KeyCode::KeyO if self.ctrl => self.open_scene_dialog(),
            KeyCode::F1 => self.ui.show_shortcuts = !self.ui.show_shortcuts,
            // Tab: all → top bar only → nothing (screenshot mode) → all.
            KeyCode::Tab => {
                if !self.show_ui {
                    self.show_ui = true;
                    self.ui.panel_hidden = false;
                } else if !self.ui.panel_hidden {
                    self.ui.panel_hidden = true;
                } else {
                    self.show_ui = false;
                }
            }
            KeyCode::F12 => self.actions.save_image = Some(video::ImageFormat::Png),
            _ => return false,
        }
        true
    }
    /// Closing the window, or Menu → Renderer → Restart now: saves the stats and unsaved work
    /// (last-session.json), stops a running export (ffmpeg, and deletes the unfinished video)
    /// and ends the event loop.
    fn shut_down(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(st) = &self.stats {
            st.save();
        }
        if let (Some(job), Some(g)) = (self.export.take(), &mut self.gpu) {
            let _ = unsafe { job.finish(g, &self.settings, true) };
        }
        self.ui.close_session(&self.settings);
        event_loop.exit();
    }

    /// Box select (3b): the objects and lights whose origin projects inside the box from `a`
    /// (points) to the pointer. Shift adds to the selection.
    fn box_select(&mut self, a: egui::Pos2) {
        let Some(g) = self.gpu.as_ref() else { return };
        let ppp = self.egui.pixels_per_point();
        let b = egui::pos2(self.cursor[0] as f32 / ppp, self.cursor[1] as f32 / ppp);
        let rect = egui::Rect::from_two_pos(a, b);
        let view = gizmo::View::new(&self.settings, [g.width, g.height]);
        let s = &self.settings;
        let inside = |p: glam::Vec3| {
            view.project(p)
                .is_some_and(|q| rect.contains(egui::pos2(q.x / ppp, q.y / ppp)))
        };
        let mut hits: Vec<ui::Selection> = vec![];
        for (i, o) in s.objects.iter().enumerate().filter(|(_, o)| o.visible) {
            if inside(geometry::object_frame(o).0) {
                hits.push(ui::Selection::Object(i));
            }
        }
        for (i, l) in s.lights.iter().enumerate().filter(|(_, l)| l.enabled) {
            if inside(glam::Vec3::from(l.position)) {
                hits.push(ui::Selection::Light(i));
            }
        }
        if s.light_on && inside(glam::Vec3::from(s.light_position)) {
            hits.push(ui::Selection::MainLight);
        }
        let n = hits.len();
        let items = if self.shift {
            let mut all = self.ui.targets();
            for h in hits {
                if !all.contains(&h) {
                    all.push(h);
                }
            }
            all
        } else {
            hits
        };
        self.ui.select_many(items, &self.settings);
        self.ui.toast(format!("Box selected {n}"));
    }
    /// Ctrl+A: every visible object and every area light that is on.
    fn select_all(&mut self) {
        let s = &self.settings;
        let items: Vec<ui::Selection> = (0..s.objects.len())
            .filter(|i| s.objects[*i].visible)
            .map(ui::Selection::Object)
            .chain(
                (0..s.lights.len())
                    .filter(|i| s.lights[*i].enabled)
                    .map(ui::Selection::Light),
            )
            .collect();
        let n = items.len();
        self.ui.select_many(items, &self.settings);
        self.ui.toast(format!("Selected {n}"));
    }
}
/// Self-test: the HDR output (texture 26, scRGB or HDR10 decoded back to scRGB) against
/// `display::hdr_map` of the linear output, with paper white 200 and peak 1000 nits.
fn check_hdr_output(g: &mut Gpu, s: &Settings, tolerance: f32) -> Result<()> {
    let (linear, out) = unsafe { (g.read_floats(18)?, g.read_floats(26)?) };
    let (mut worst, mut brightest) = (0f32, 0f32);
    for (l, h) in linear.as_chunks::<4>().0.iter().zip(out.as_chunks::<4>().0) {
        let want = display::hdr_map(
            [l[0], l[1], l[2]],
            s.exposure,
            s.gamma,
            s.tonemap,
            200.,
            1000.,
        );
        let top = want[0].abs().max(want[1].abs()).max(want[2].abs());
        for i in 0..3 {
            worst = worst.max((h[i] - want[i]).abs() / (top + 0.01));
            brightest = brightest.max(h[i]);
        }
    }
    anyhow::ensure!(
        worst < tolerance && brightest <= 12.5 * (1. + tolerance) && brightest > 4.,
        "HDR transform mismatch: relative error {worst}, brightest {brightest}"
    );
    Ok(())
}

/// The release version (`v` tag without the v) or the Cargo.toml version with -dev; build.rs.
pub const VERSION: &str = env!("CAUSTICA_VERSION");
/// The exe's icon resource at `size` (None: the system's small-icon size).
#[cfg(not(windows))]
fn app_icon(_size: Option<u32>) -> Option<winit::window::Icon> {
    None
}
#[cfg(windows)]
fn app_icon(size: Option<u32>) -> Option<winit::window::Icon> {
    use winit::platform::windows::IconExtWindows;
    winit::window::Icon::from_resource(1, size.map(|s| winit::dpi::PhysicalSize::new(s, s))).ok()
}
/// D3D12 Agility SDK (track 20): the runtime loads D3D12Core.dll of this SDK version from
/// D3D12\ beside the exe (build.rs copies it; tools/setup.ps1 fetches it). Must match the
/// version setup.ps1 downloads. Exported by name from the exe (build.rs /EXPORT).
#[unsafe(no_mangle)]
#[used]
#[allow(non_upper_case_globals)]
pub static D3D12SDKVersion: u32 = 620;
#[repr(transparent)]
pub struct SdkPath(*const u8);
unsafe impl Sync for SdkPath {}
#[unsafe(no_mangle)]
#[used]
#[allow(non_upper_case_globals)]
pub static D3D12SDKPath: SdkPath = SdkPath(c".\\D3D12\\".as_ptr().cast());
/// Process start, for the `STARTUP` timings on stderr.
pub static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
/// Prints how long after process start a startup phase ended (stderr: `STARTUP phase ms`).
pub fn startup(phase: &str) {
    eprintln!(
        "STARTUP {phase} {:.0}",
        START.elapsed().as_secs_f64() * 1000.
    );
}
fn main() {
    std::sync::LazyLock::force(&START);
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // An installed copy keeps its files per user; carry over what older builds left beside it.
    dirs::migrate();
    if args.iter().any(|a| a == "--vulkan-smoke") {
        return vulkan::run(&args);
    }
    if let saver::Mode::Preview(parent) = saver::mode(&args) {
        saver::preview(parent);
        return Ok(());
    }
    // Renders the missing scene thumbnails (each one a headless run of this exe, on the CPU
    // renderer with --cpu) and exits.
    if std::env::args().any(|a| a == "--make-thumbnails") {
        let cpu = std::env::args().any(|a| a == "--cpu");
        let missing: Vec<_> = scenes::list()
            .into_iter()
            .filter(|e| e.thumbnail().is_some_and(|t| !t.is_file()))
            .collect();
        println!("{} thumbnails to render", missing.len());
        for result in scenes::render_thumbnails(missing, cpu) {
            match result {
                Ok(path) => println!("THUMBNAIL {}", path.display()),
                Err(title) => println!("THUMBNAIL FAILED {title}"),
            }
        }
        return Ok(());
    }
    // Prints each mesh file's parts (one per material) as JSON objects and exits: what
    // tools/build_scenes.py's mesh pack is written from.
    if let Some(i) = args.iter().position(|a| a == "--mesh-info") {
        for path in &args[i + 1..] {
            let objects = mesh::import_objects(std::path::Path::new(path))?;
            let triangles: Vec<usize> = objects
                .iter()
                .map(|o| mesh::of(o).map_or(0, |m| m.len() / 3))
                .collect();
            println!(
                "{}",
                serde_json::json!({"file": path, "triangles": triangles, "objects": objects})
            );
        }
        return Ok(());
    }
    // Asset browser downloads from the command line (item 6): `--fetch-asset
    // polyhaven/hdri/meadow_2@2k` prints the path to use; `--asset-catalog polyhaven` refreshes
    // a cached catalog and prints its size.
    if let Some(i) = args.iter().position(|a| a == "--fetch-asset") {
        for spec in &args[i + 1..] {
            let (asset, res) = assets::from_spec(spec)?;
            let path = assets::install(&asset, &res, &Default::default())?;
            println!("ASSET {path}");
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--asset-catalog") {
        for name in &args[i + 1..] {
            let source = assets::Source::ALL
                .into_iter()
                .find(|s| s.label().to_lowercase().starts_with(&name.to_lowercase()))
                .with_context(|| format!("Unknown source {name}"))?;
            let catalog = assets::fetch_catalog(source)?;
            println!("CATALOG {} {}", source.label(), catalog.len());
        }
        return Ok(());
    }
    let mut app = App::new()?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.failed {
        bail!(error);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    /// A double-click on a slider (driven through egui's real input) resets the field to the
    /// registered reference; a single click on the rail still sets it.
    #[test]
    fn double_clicking_a_slider_resets_it() {
        struct Fields {
            a: f32,
            _b: u32,
        }
        let reference = Fields { a: 0.25, _b: 7 };
        let mut live = Fields { a: 0.9, _b: 3 };
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut values = vec![];
        for frame in 0..8 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400., 200.),
                )),
                time: Some(frame as f64 * 0.05),
                ..Default::default()
            };
            let at = rect.left_center() + egui::vec2(30., 0.);
            let click = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            match frame {
                1 => input.events.push(egui::Event::PointerMoved(at)),
                2 | 4 => input.events.push(click(true)),
                3 | 5 => input.events.push(click(false)),
                _ => {}
            }
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    crate::reset_source(&live, &reference);
                    rect = ui.add(crate::slider(&mut live.a, 0. ..=1.)).rect;
                    crate::clear_reset_sources();
                });
            });
            values.push(live.a);
        }
        assert!(
            values.iter().any(|v| *v != 0.9 && *v != 0.25),
            "a click on the rail moves the value: {values:?}"
        );
        assert_eq!(live.a, 0.25, "the double-click resets it: {values:?}");
    }
    #[test]
    fn quality_bundles_round_trip() {
        use crate::scene::{self, Settings};
        use crate::ui;
        for b in ui::Bundle::ALL {
            let mut s = Settings::default();
            b.apply(&mut s, scene::Upscaler::Xess);
            assert_eq!(ui::Bundle::of(&s), Some(b), "{b:?}");
        }
        let mut s = Settings::default();
        s.scale = 0.8;
        assert_eq!(ui::Bundle::of(&s), None);
    }
    /// Every non-ASCII character in a UI string must exist in egui's default fonts,
    /// otherwise it renders as an empty box.
    #[test]
    fn ui_strings_have_glyphs() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});
        let font = egui::FontId::proportional(14.);
        for (file, source) in [
            ("main.rs", include_str!("main.rs")),
            ("editor.rs", include_str!("editor.rs")),
            ("ui.rs", include_str!("ui.rs")),
            ("gizmo.rs", include_str!("gizmo.rs")),
            ("look.rs", include_str!("look.rs")),
            ("walk.rs", include_str!("walk.rs")),
            ("video.rs", include_str!("video.rs")),
            ("video_ui.rs", include_str!("video_ui.rs")),
            ("timeline_ui.rs", include_str!("timeline_ui.rs")),
            ("prefs.rs", include_str!("prefs.rs")),
            ("remote.rs", include_str!("remote.rs")),
            ("presets.rs", include_str!("presets.rs")),
            ("scene.rs", include_str!("scene.rs")),
        ] {
            for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for literal in line.split('"').skip(1).step_by(2) {
                    for c in literal.chars().filter(|c| !c.is_ascii()) {
                        assert!(
                            ctx.fonts(|f| f.has_glyph(&font, c)),
                            "{file}: {c:?} (U+{:04X}) in {literal:?} has no glyph",
                            c as u32
                        );
                    }
                }
            }
        }
    }
}
