#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod display;
mod editor;
mod generators;
mod geometry;
mod gizmo;
mod gpu;
mod hotreload;
mod lighting;
mod look;
mod mesh;
mod presets;
mod procedural;
mod remote;
mod saver;
mod scene;
mod scenes;
mod stats;
mod teapot_data;
mod ui;
mod video;
mod walk;
use anyhow::{Context, Result, bail};
use gpu::Gpu;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
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
    save: bool,
    load: bool,
    png: bool,
    exr: bool,
    hdr_png: bool,
    reference: bool,
    high_res: bool,
    turntable: bool,
    frame_scene: bool,
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
    preset: usize,
    frames: Option<u32>,
    rendered: u32,
    /// Timing for the RENDER_OK line, from frame 10 on (after startup and BVH builds).
    timing: Option<(Instant, f64)>,
    capture: Option<PathBuf>,
    capture_sequence: Option<PathBuf>,
    orbit_test: bool,
    /// Moves object 0 a little every frame, as a gizmo drag does (BLAS rebuild cost).
    drag_test: bool,
    require_rr: bool,
    capture_ui: bool,
    self_test: bool,
    debug: bool,
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
    hwnd: Option<windows::Win32::Foundation::HWND>,
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
    /// Ctrl+click focus: waiting for the depth under the click, then easing toward it.
    focus_pending: bool,
    focus_target: Option<f32>,
    /// A click waiting for the object-id readback at this pixel.
    pick_point: Option<[f64; 2]>,
    /// Light painting (L): light edits keep the accumulation; progressive path only.
    painting: bool,
    /// Walk mode (`): WASD and mouse look; the keys held down right now.
    walking: Option<walk::Walk>,
    keys_down: std::collections::HashSet<KeyCode>,
    /// Screensaver show mode (`/s`): cycles scenes and quits on input.
    saver: Option<saver::Saver>,
    /// `--turntable PATH`: render the video in a headless run, then exit.
    turntable_cli: Option<video::Turntable>,
    /// Phone remote (off by default) and when its page state was last refreshed.
    remote: Option<remote::Remote>,
    remote_synced: Instant,
    /// Gamepad input (interactive runs only).
    gamepad: Option<gilrs::Gilrs>,
}
/// Self-test: the first frame of the scene-file sweep (frames before it are scripted).
const SCENES_FROM: u32 = 260;
fn transport_key(s: &Settings) -> String {
    let mut j = serde_json::to_value(s).unwrap();
    for k in [
        "exposure",
        "gamma",
        "tonemap",
        "view",
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
        "rotate",
        "rotate_speed",
        "halation",
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
    ] {
        j.as_object_mut().unwrap().remove(k);
    }
    for key in ["objects", "lights"] {
        if let Some(items) = j.get_mut(key).and_then(|v| v.as_array_mut()) {
            for item in items {
                item.as_object_mut().unwrap().remove("name");
            }
        }
    }
    j.to_string()
}
fn geometry_key(s: &Settings) -> String {
    serde_json::json!([
        s.objects
            .iter()
            .map(|o| (
                o.kind(),
                &o.mesh,
                // Glass goes to the non-opaque BLAS range (shadow rays may pass it).
                o.material.transmission > 0.,
                o.visible,
                o.position,
                o.size,
                o.stretch,
                o.rotation,
                o.tilt,
                o.scale_xz,
                o.elevation,
                o.grounded
            ))
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
            .map(|l| (l.enabled, l.visible, l.position, l.rotation, l.size))
            .collect::<Vec<_>>()
    ])
    .to_string()
}
fn material_key(s: &Settings) -> Vec<u8> {
    bytemuck::cast_slice(&lighting::materials(s)).to_vec()
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
        let mut hidden = false;
        let mut capture_ui = false;
        let mut interface = ui::UiState::default();
        let mut self_test = false;
        let mut selected_preset = 0;
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
        let mut turntable_cli: Option<video::Turntable> = None;
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
                    turntable_cli = Some(video::Turntable {
                        frames: 60,
                        spp: 16,
                        degrees: 360.,
                        fps: 30,
                        path: PathBuf::from(args.next().context("--turntable needs a file name")?),
                    });
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
                    selected_preset = args.next().context("--preset needs 0–23")?.parse()?;
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
                "--no-rr" => s.rr = false,
                "--rr" => s.rr = true,
                "--require-rr" => {
                    s.rr = true;
                    require_rr = true;
                }
                "--raw" => s.denoise = false,
                "--debug" => debug = true,
                "--headless" => hidden = true,
                "--capture-ui" => capture_ui = true,
                // For UI captures: open a tab, show the advanced rows.
                "--ui-tab" => {
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
                "--ui-scopes" => {
                    interface.show_scopes = true;
                    interface.zebras = true;
                }
                "--ui-tool" => {
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
                        "--ui-select needs stage, camera, main-light, light:N or object:N",
                    )?;
                    let index =
                        |prefix: &str| what.strip_prefix(prefix).and_then(|n| n.parse().ok());
                    interface.selection = match what.as_str() {
                        "stage" => ui::Selection::Stage,
                        "camera" => ui::Selection::Camera,
                        "main-light" => ui::Selection::MainLight,
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
                "--help" => {
                    println!(
                        "Cornell RTX: --frames N --capture image.png|image.exr --width N --height N --scale 0.25..1 --spp 1..16 --samples N --preset 0..23 --settings file.json --set key=json --rr --no-rr --require-rr --raw --debug --headless --no-vsync --capture-ui --ui-tab scene|render|look|export --ui-advanced --ui-scenes --ui-scopes --ui-tool select|move|rotate|scale --ui-select stage|camera|main-light|light:N|object:N --capture-sequence dir --make-thumbnails --mesh-info file… --orbit-test --drag-test --turntable out.mp4 --turntable-frames N --turntable-spp N --self-test"
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
        anyhow::ensure!(frames != Some(0), "--frames must be positive");
        if turntable_cli.is_some() && frames.is_none() {
            frames = Some(4);
        }
        anyhow::ensure!(!hidden || frames.is_some(), "--headless requires --frames");
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
            settings: s,
            paused: false,
            show_ui: !hidden || capture_ui || self_test,
            last: Instant::now(),
            cursor: [0.; 2],
            drag: None,
            drag_distance: 0.,
            shift: false,
            ctrl: false,
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
            debug,
            hidden,
            size,
            failed: None,
            actions: Actions::default(),
            ui: interface,
            hwnd: None,
            monitor: None,
            display_dirty: true,
            hdr_note: String::new(),
            hot: None,
            stats: None,
            stats_saved: Instant::now(),
            last_samples: 0,
            lut_loaded: String::new(),
            focus_pending: false,
            focus_target: None,
            pick_point: None,
            painting: false,
            walking: None,
            keys_down: Default::default(),
            saver: screensaver.then(saver::Saver::new),
            turntable_cli,
            remote: None,
            remote_synced: Instant::now(),
            gamepad: None,
        })
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        eprintln!("ERROR: {error:#}");
        self.failed = Some(format!("{error:#}"));
        event_loop.exit();
    }
    fn redraw(&mut self) -> Result<()> {
        if self.orbit_test && self.rendered >= 64 {
            self.settings.yaw = (self.rendered - 64) as f32 * 0.25;
        }
        if self.drag_test && self.rendered >= 10 && !self.settings.objects.is_empty() {
            let x = &mut self.settings.objects[0].position[0];
            *x = (*x + if self.rendered % 40 < 20 { 0.01 } else { -0.01 }).clamp(-40., 40.);
        }
        if self.self_test {
            match self.rendered {
                1 => self.actions.reference = true,
                2 => {
                    self.settings.exposure = 1.2;
                    self.settings.compare = true;
                }
                3 => self.paused = true,
                4 => self.settings.objects[0].material.roughness = 0.2,
                5 => self.paused = false,
                6 => self.settings.yaw = 8.,
                7 => {
                    self.settings.scale = 0.5;
                    unsafe {
                        self.gpu
                            .as_mut()
                            .unwrap()
                            .resize(800, 600, &self.settings)?;
                    }
                }
                8 => self.settings.rr = false,
                9 => self.settings.rr = true,
                // Frames 10–125 cycle all presets; 23 is the empty stage.
                130 => {
                    self.settings.light_on = false;
                    self.settings.lights.clear();
                    self.settings.sky = 0.;
                }
                131 => {
                    for (i, shape) in scene::Shape::ALL.into_iter().enumerate() {
                        self.settings.objects.push(scene::Object {
                            name: shape.label().into(),
                            shape,
                            position: [(i % 4) as f32 * 0.6 - 0.9, (i / 4) as f32 * 0.6 - 0.6],
                            size: 0.6,
                            stretch: 1.,
                            rotation: 0.,
                            ..Default::default()
                        });
                    }
                }
                132 => self.settings.objects[10].material.emission = 5.,
                133 => self.settings.objects[10].visible = false,
                134 => self.settings.lights.push(scene::AreaLight::default()),
                135 => self.settings.lights[0].visible = false,
                136 => self.settings.lights[0].enabled = false,
                137 => {
                    self.settings.objects.remove(0);
                }
                139 => self.actions.frame_scene = true,
                140 => {
                    self.settings.light_on = true;
                    self.settings.randomize(0x2545f491);
                    self.settings.validate()?;
                }
                150..=157 => self.settings.view = self.rendered - 150,
                160 => {
                    self.settings.view = 0;
                    self.settings.rr = false;
                    self.settings.denoise = false;
                }
                161 => self.settings.denoise = true,
                162 => self.settings.rr = true,
                163 => self.settings.temporal = false,
                164 => self.settings.temporal = true,
                165 => {
                    let saved = serde_json::to_vec(&self.settings)?;
                    let mut loaded: Settings = serde_json::from_slice(&saved)?;
                    loaded.validate()?;
                    anyhow::ensure!(loaded == self.settings, "Settings round trip failed");
                }
                166 => self.settings.rr_preset = 5,
                167 => self.settings.rr_preset = 4,
                168 => self.settings.rr_preset = 6,
                170 => {
                    self.ui.export_scale = 1;
                    self.ui.export_samples = 16;
                    self.actions.high_res = true;
                }
                // Every upscaler on the native path at half resolution, then back to RR.
                172..=176 => {
                    self.settings.rr = false;
                    self.settings.scale = 0.5;
                    self.settings.upscaler = scene::Upscaler::ALL[(self.rendered - 172) as usize];
                }
                177 => self.settings.rr = true,
                // A loaded value must survive drawing its slider (the Render tab is open).
                178 => {
                    self.ui.tab = ui::Tab::Render;
                    self.settings.max_samples = 100_000;
                }
                // Every DLSS Super Resolution preset recreates the feature, then back to RR.
                180..=184 => {
                    self.settings.rr = false;
                    self.settings.scale = 0.5;
                    self.settings.upscaler = scene::Upscaler::Dlss;
                    self.settings.dlss_preset =
                        scene::DLSS_PRESETS[(self.rendered - 180) as usize].0;
                }
                185 => {
                    self.settings.dlss_preset = 0;
                    self.settings.rr = true;
                }
                // scRGB output with fixed levels (works on SDR monitors too), then SDR, then Auto.
                186 => {
                    self.settings.hdr_mode = 2;
                    self.settings.hdr_peak = 1000.;
                    self.settings.hdr_white = 200.;
                }
                188 => self.settings.hdr_mode = 1,
                189 => {
                    self.settings.hdr_mode = 0;
                    self.settings.hdr_peak = 0.;
                    self.settings.hdr_white = 0.;
                }
                // Draw every inspector and tab once, with the advanced rows, under the debug layer.
                190 => {
                    self.ui.advanced = true;
                    self.ui.tab = ui::Tab::Scene;
                    self.ui.selection = ui::Selection::Object(0);
                }
                191 => {
                    self.settings.lights.push(scene::AreaLight::default());
                    self.ui.selection = ui::Selection::Light(self.settings.lights.len() - 1);
                }
                192 => self.ui.selection = ui::Selection::MainLight,
                193 => self.ui.selection = ui::Selection::Camera,
                194 => self.ui.tab = ui::Tab::Look,
                // DLAA: DLSS Super Resolution at 100% as anti-aliasing, then back to RR.
                196 => {
                    self.settings.rr = false;
                    self.settings.scale = 1.;
                    self.settings.upscaler = scene::Upscaler::Dlss;
                    self.settings.native_aa = true;
                }
                197 => {
                    self.settings.native_aa = false;
                    self.settings.rr = true;
                }
                // Gizmos: each tool drawn on an object and on the main light, then a modal
                // G X 0.5 Enter through the same code the keys use, and its undo.
                198 => {
                    self.ui.tab = ui::Tab::Scene;
                    self.ui.selection = ui::Selection::Object(0);
                    self.ui.tool = gizmo::Tool::Move;
                }
                199 => self.ui.tool = gizmo::Tool::Rotate,
                200 => {
                    self.ui.tool = gizmo::Tool::Scale;
                    self.ui.selection = ui::Selection::MainLight;
                }
                201 => {
                    self.ui.selection = ui::Selection::Object(0);
                    let before = self.settings.objects[0].position[0];
                    anyhow::ensure!(
                        self.start_transform(gizmo::Mode::Move, None),
                        "G did not start a transform"
                    );
                    for key in [
                        KeyCode::KeyX,
                        KeyCode::Digit0,
                        KeyCode::Period,
                        KeyCode::Digit5,
                    ] {
                        self.transform_key(key);
                    }
                    self.transform_key(KeyCode::Enter);
                    let moved = self.settings.objects[0].position[0] - before;
                    anyhow::ensure!(
                        (moved - 0.5).abs() < 1e-5 && self.ui.transform.is_none(),
                        "G X 0.5 Enter moved the object by {moved}"
                    );
                    self.ui.tool = gizmo::Tool::Move;
                }
                // Debug views (bounces, BVH cost, variance, false colour), then zebras, scopes,
                // the light meter and the convergence graph.
                204..=207 => self.settings.view = 8 + self.rendered - 204,
                208 => {
                    self.settings.view = 0;
                    self.ui.zebras = true;
                    self.ui.show_scopes = true;
                    self.ui.meter = true;
                    self.cursor = [self.size[0] as f64 * 0.5, self.size[1] as f64 * 0.5];
                }
                210 => {
                    let g = self.gpu.as_mut().unwrap();
                    self.ui.reference = Some((g.width, g.height, unsafe { g.read_floats(18)? }));
                    self.ui.show_graph = true;
                    self.actions.reset = true;
                }
                215 => {
                    anyhow::ensure!(
                        self.ui.graph.len() >= 2
                            && self.ui.graph.iter().all(|(_, e)| e.is_finite()),
                        "Convergence graph has no points: {:?}",
                        self.ui.graph
                    );
                    self.ui.zebras = false;
                    self.ui.show_scopes = false;
                    self.ui.meter = false;
                    self.ui.show_graph = false;
                }
                // Lens and camera models, then the film look and a LUT, then a Ctrl+click focus.
                216 => {
                    self.settings.aperture = 0.06;
                    self.settings.focus_distance = 4.;
                    self.settings.aperture_blades = 6;
                    self.settings.cat_eye = 0.5;
                }
                217..=220 => self.settings.projection = self.rendered - 216,
                221 => {
                    self.settings.projection = 0;
                    self.settings.aperture = 0.;
                    self.settings.tonemap = 6;
                }
                222 => self.settings.tonemap = 7,
                223 => {
                    self.settings.tonemap = 3;
                    self.settings.halation = 0.6;
                    self.settings.grain = 0.5;
                    self.settings.vignette = 0.5;
                    self.settings.lift = [0.02, 0., -0.02];
                    self.settings.gain = [1.1, 1., 0.9];
                    let mut text = String::from("LUT_3D_SIZE 2\n");
                    for b in 0..2 {
                        for g in 0..2 {
                            for r in 0..2 {
                                text += &format!("{} {} {}\n", 1 - r, 1 - g, 1 - b);
                            }
                        }
                    }
                    let path = std::env::temp_dir().join("cornell-self-test.cube");
                    std::fs::write(&path, text)?;
                    self.settings.lut_path = path.to_string_lossy().into();
                }
                225 => {
                    self.settings.lut_path.clear();
                    self.settings.halation = 0.;
                    self.settings.grain = 0.;
                    self.settings.vignette = 0.;
                    self.settings.lift = [0.; 3];
                    self.settings.gain = [1.; 3];
                    self.settings.aperture = 0.05;
                    self.cursor = [self.size[0] as f64 * 0.5, self.size[1] as f64 * 0.5];
                    self.focus_pending = true;
                }
                // Light painting: a light edit keeps the accumulation going.
                240 => {
                    self.settings.preset(0);
                    self.toggle_painting();
                }
                241 => self.settings.light_position[0] += 0.1,
                242 => self.toggle_painting(),
                // Portals: a paired pair of boxes, then removed.
                243 => {
                    let mut a = editor::new_object(scene::Shape::Box, 2);
                    a.position = [-0.5, 0.3];
                    a.stretch = 2.;
                    a.scale_xz = [1.2, 0.1];
                    a.portal = 3;
                    let mut b = a.clone();
                    b.position = [0.5, 0.3];
                    b.rotation = 40.;
                    b.portal = 2;
                    self.settings.objects.push(a);
                    self.settings.objects.push(b);
                }
                246 => {
                    self.settings.objects.truncate(2);
                }
                // Walk mode: hold W for a few frames.
                247 => {
                    // A known start: eye at z = 7.7 looking down -z, target 0.5 ahead.
                    (
                        self.settings.yaw,
                        self.settings.pitch,
                        self.settings.distance,
                    ) = (0., 0., 4.7);
                    self.settings.target = [0., 0., 3.];
                    self.toggle_walk();
                    self.keys_down.insert(KeyCode::KeyW);
                }
                // A three-frame turntable (PNG frames when ffmpeg is missing).
                253 => {
                    let path = std::env::temp_dir().join("cornell-self-test-turntable.mp4");
                    let folder = path.with_extension("");
                    let _ = std::fs::remove_dir_all(&folder);
                    let t = video::Turntable {
                        frames: 3,
                        spp: 2,
                        degrees: 30.,
                        fps: 30,
                        path: path.clone(),
                    };
                    let g = self.gpu.as_mut().unwrap();
                    let text = unsafe { video::render(g, &self.settings.clone(), &t)? };
                    let made = if video::ffmpeg_available() {
                        path.is_file()
                    } else {
                        (1..=3).all(|i| folder.join(format!("{i:05}.png")).is_file())
                    };
                    anyhow::ensure!(made, "Turntable output missing: {text}");
                }
                // Phone remote on this machine only: an edit through HTTP reaches the settings.
                254 => {
                    let r = remote::start(false)?;
                    let port: u16 = r
                        .url
                        .split(':')
                        .nth(2)
                        .and_then(|p| p.split('/').next())
                        .context("remote port")?
                        .parse()?;
                    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
                    use std::io::Write;
                    stream.write_all(
                        format!(
                            "POST /set?t={}&k=exposure HTTP/1.1\r\nContent-Length: 3\r\n\r\n2.5",
                            r.token
                        )
                        .as_bytes(),
                    )?;
                    let mut reply = String::new();
                    use std::io::Read;
                    stream.read_to_string(&mut reply)?;
                    anyhow::ensure!(
                        reply.starts_with("HTTP/1.1 204"),
                        "Remote refused the edit: {reply}"
                    );
                    self.remote = Some(r);
                }
                256 => {
                    anyhow::ensure!(
                        self.settings.exposure == 2.5,
                        "Remote edit not applied ({})",
                        self.settings.exposure
                    );
                    self.remote = None;
                    self.settings.exposure = 1.;
                }
                // GPU picking: a click on object 0's projected centre selects it next frame.
                257 => {
                    let s = &self.settings;
                    let (view, proj, _) = s.camera(self.size[0] as f32 / self.size[1] as f32);
                    let (centre, _) = geometry::object_frame(&s.objects[0]);
                    let ndc = (proj * view).project_point3(centre);
                    let g = self.gpu.as_ref().unwrap();
                    self.pick_point = Some([
                        ((ndc.x * 0.5 + 0.5) * g.width as f32) as f64,
                        ((0.5 - ndc.y * 0.5) * g.height as f32) as f64,
                    ]);
                    self.ui.selection = ui::Selection::None;
                }
                // Mesh import: an OBJ pyramid, plus an object whose mesh file is missing (it
                // falls back to its shape). The per-frame GPU counts check both.
                258 => {
                    let path = std::env::temp_dir().join("cornell-self-test-pyramid.obj");
                    std::fs::write(
                        &path,
                        "v -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nv 0 2 0\n\
                         f 1 2 3 4\nf 1 2 5\nf 2 3 5\nf 3 4 5\nf 4 1 5\n",
                    )?;
                    let mut pyramid = mesh::import_object(&path)?;
                    pyramid.position = [0.5, 0.];
                    let mut missing = pyramid.clone();
                    missing.mesh = "missing/nothing-here.obj".into();
                    missing.position = [-0.5, 0.];
                    self.settings.objects.push(pyramid);
                    self.settings.objects.push(missing);
                    anyhow::ensure!(
                        mesh::of(&self.settings.objects[self.settings.objects.len() - 2])
                            .is_some_and(|m| m.len() == 18),
                        "Imported pyramid is not 6 triangles"
                    );
                }
                259 => anyhow::ensure!(
                    self.ui.selection == ui::Selection::Object(0),
                    "GPU pick selected {:?}",
                    self.ui.selection
                ),
                // A user texture (as a scene thumbnail) drawn in the open Scenes window, then the
                // scene files below load with the window still open.
                203 => {
                    let image =
                        egui::ColorImage::new([64, 40], egui::Color32::from_rgb(40, 120, 200));
                    let texture =
                        self.egui
                            .load_texture("self-test", image, egui::TextureOptions::LINEAR);
                    if let Some(path) = self.ui.scenes.first().and_then(|e| e.thumbnail()) {
                        self.ui.thumbnails.insert(path, texture);
                    }
                    self.ui.show_scenes = true;
                    self.ui.scene_selected = Some(0);
                }
                // Every scene file, one per frame; the per-frame checks compare GPU triangle and
                // emitter counts with the loaded settings.
                n if n >= SCENES_FROM => {
                    let entries = scenes::list();
                    if let Some(e) = entries.get((n - SCENES_FROM) as usize) {
                        scenes::load_into(&mut self.settings, &e.path)
                            .with_context(|| format!("Scene file {}", e.path.display()))?;
                        anyhow::ensure!(
                            self.settings.scene_name == e.title,
                            "Scene title not applied"
                        );
                    }
                }
                202 => {
                    let after = self.settings.objects[0].position[0];
                    anyhow::ensure!(self.ui.undo(&mut self.settings), "Nothing to undo");
                    let back = self.settings.objects[0].position[0];
                    anyhow::ensure!(
                        (after - back - 0.5).abs() < 1e-5,
                        "Undo did not restore the move"
                    );
                }
                195 => {
                    self.ui.tab = ui::Tab::Export;
                    self.ui.selection = ui::Selection::Light(self.settings.lights.len() - 1);
                    let removed = self.ui.delete_selected(&mut self.settings);
                    anyhow::ensure!(removed.is_some(), "Deleting the selected light failed");
                }
                _ => {}
            }
            if (10..=125).contains(&self.rendered) && self.rendered.is_multiple_of(5) {
                self.settings.preset(((self.rendered - 10) / 5) as usize);
            }
        }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.settings.rotate && !self.paused {
            self.settings.yaw += self.settings.rotate_speed * dt;
        }
        let window = self.window.as_ref().unwrap().clone();
        let input = self.input.as_mut().unwrap().take_egui_input(&window);
        let ctx = self.egui.clone();
        let output = ctx.run(input, |ctx| {
            if self.show_ui {
                self.ui(ctx);
            }
        });
        self.input
            .as_mut()
            .unwrap()
            .handle_platform_output(&window, output.platform_output);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        if self.actions.save {
            self.actions.save = false;
            match serde_json::to_string_pretty(&self.settings)
                .map_err(anyhow::Error::from)
                .and_then(|j| std::fs::write("settings.json", j).map_err(Into::into))
            {
                Ok(()) => self.ui.toast("Saved settings.json"),
                Err(e) => self.ui.error(format!("Save failed: {e}")),
            }
        }
        if self.actions.load {
            self.actions.load = false;
            match std::fs::read("settings.json")
                .map_err(anyhow::Error::from)
                .and_then(|b| serde_json::from_slice::<Settings>(&b).map_err(Into::into))
                .and_then(|mut s| {
                    s.validate()?;
                    Ok(s)
                }) {
                Ok(s) => {
                    self.settings = s;
                    self.ui.toast("Loaded settings.json");
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
        }
        self.remote_and_gamepad(dt);
        let g = self.gpu.as_mut().unwrap();
        g.painting = self.painting;
        let s = &self.settings.clone();
        // HDR output follows the monitor: re-read it after a move, or once Windows changed the
        // display setup (HDR toggled, calibration rerun, SDR brightness moved).
        if self.display_dirty
            || (self.rendered.is_multiple_of(30)
                && self
                    .monitor
                    .as_ref()
                    .is_some_and(|m| !unsafe { m.current() }))
        {
            let previous = self.monitor.take().map(|m| m.info);
            self.monitor = self
                .hwnd
                .and_then(|h| unsafe { display::Monitor::query(h) }.ok());
            self.display_dirty = false;
            if let Some(m) = &self.monitor
                && previous.as_ref() != Some(&m.info)
            {
                println!("DISPLAY {:?}", m.info);
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
        if want != g.hdr_active && !unsafe { g.set_hdr(want, s)? } {
            self.hdr_note = "The swapchain can't present scRGB; staying SDR.".into();
        }
        g.hdr_levels = [
            if s.hdr_white > 0. {
                s.hdr_white
            } else {
                info.sdr_white_nits.max(40.)
            },
            if s.hdr_peak > 0. {
                s.hdr_peak
            } else if info.peak_nits >= 80. {
                info.peak_nits
            } else {
                1000.
            },
        ];
        let rw = ((g.width as f32 * s.scale).round() as u32).max(1);
        if g.rw != rw {
            unsafe {
                g.resize(g.width, g.height, s)?;
            }
        }
        // The keys serialize the whole scene, so only rebuild them after an edit.
        if self.checked.as_ref() != Some(s) {
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
                let painted = self.painting && {
                    for k in [
                        "light_position",
                        "light_size",
                        "light_aspect",
                        "light_intensity",
                        "light_temperature",
                        "light_tint",
                        "light_on",
                        "lights",
                    ] {
                        old.as_object_mut().unwrap().remove(k);
                        new.as_object_mut().unwrap().remove(k);
                    }
                    old == new
                };
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
        }
        let draw = (!self.paused || g.samples == 0) && (g.samples < s.max_samples || self.painting);
        // Look tools: zebras and scopes in the display pass, a light-meter texel under the cursor.
        g.zebras = self.ui.zebras;
        g.scopes_on = self.ui.show_scopes;
        let probe_at = self.pick_point.unwrap_or(self.cursor);
        g.probe = ((self.ui.meter && !self.egui.is_pointer_over_area())
            || self.focus_pending
            || self.pick_point.is_some())
        .then_some([probe_at[0].max(0.) as u32, probe_at[1].max(0.) as u32]);
        if s.lut_path != self.lut_loaded {
            self.lut_loaded = s.lut_path.clone();
            let result = if s.lut_path.is_empty() {
                unsafe { g.set_lut(None) }
            } else {
                std::fs::read_to_string(&s.lut_path)
                    .map_err(anyhow::Error::from)
                    .and_then(|text| look::read_cube(&text))
                    .and_then(|(n, data)| unsafe { g.set_lut(Some((n, &data))) })
            };
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
        unsafe {
            g.render(
                s,
                draw,
                dt,
                &primitives,
                &output.textures_delta,
                output.pixels_per_point,
            )?;
        }
        if let (Some(point), Some(p)) = (self.pick_point.take(), g.probe_result) {
            let pixel = [point[0] as f32, point[1] as f32];
            match gizmo::pick_from_probe(&self.settings, pixel, [g.width, g.height], p.id, p.depth)
            {
                Some(sel) => {
                    self.ui.selection = sel;
                    self.ui.tab = ui::Tab::Scene;
                    self.ui.panel_hidden = false;
                    self.show_ui = true;
                }
                None => self.ui.selection = ui::Selection::None,
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
            if g.samples < self.ui.graph.last().map_or(0, |p| p.0) {
                self.ui.graph.clear();
            }
            let last = self.ui.graph.last().map_or(0, |p| p.0);
            if g.samples > 0 && (last == 0 || g.samples as f64 >= last as f64 * 1.25) {
                let image = unsafe { g.read_floats(18)? };
                let compress = |p: &[f32]| {
                    let l = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                    (l / (1. + l.max(0.))) as f64
                };
                let (a, b) = (image.as_chunks::<4>().0, reference.as_chunks::<4>().0);
                let sum: f64 = a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| (compress(x) - compress(y)).powi(2))
                    .sum();
                self.ui
                    .graph
                    .push((g.samples, (sum / a.len().max(1) as f64).sqrt() * 1000.));
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
            let checks = [
                ("first-light", g.samples > 0),
                ("long-exposure", g.samples >= 65_536),
                ("tourist", st.presets_seen.len() >= PRESETS.len()),
                (
                    "collector",
                    !builtin.is_empty() && builtin.iter().all(|t| st.scenes_seen.contains(*t)),
                ),
                ("photographer", st.images_saved >= 10),
                (
                    "caustic-hunter",
                    s.scene_name == "Cornell caustic" && g.samples >= 4096,
                ),
                ("hot-shader", st.hot_reloads > 0),
                ("billion", st.paths >= 1e9),
            ];
            for (id, done) in checks {
                if done && let Some(title) = st.unlock(id) {
                    self.ui.toast(format!("Achievement unlocked: {title}"));
                }
            }
            if self.stats_saved.elapsed() > Duration::from_secs(30) {
                st.save();
                self.stats_saved = Instant::now();
            }
        }
        if self.self_test {
            // Every edit must reach the GPU: geometry and light tables match the settings.
            let vertices = scene::geometry(s);
            let (emitters, _) = lighting::emitter_table(&vertices, &lighting::materials(s));
            anyhow::ensure!(
                g.triangle_count == vertices.len() / 3 && g.emitter_count == emitters.len(),
                "Frame {}: GPU scene is stale ({} / {} triangles, {} / {} emitters)",
                self.rendered,
                g.triangle_count,
                vertices.len() / 3,
                g.emitter_count,
                emitters.len()
            );
            let emitters = g.emitter_count;
            match self.rendered {
                2 | 3 => anyhow::ensure!(
                    g.samples == 3 * s.spp,
                    "Display edit/pause reset accumulation"
                ),
                4 | 6 | 135 => anyhow::ensure!(
                    g.samples == s.spp,
                    "Scene/camera edit failed to reset accumulation"
                ),
                125 => anyhow::ensure!(s.objects.is_empty(), "Empty stage has objects"),
                130 | 133 | 136 => anyhow::ensure!(emitters == 0, "Expected no emitters"),
                132 => anyhow::ensure!(
                    emitters > 0 && g.samples == s.spp,
                    "Emissive object is not sampled"
                ),
                134 => anyhow::ensure!(emitters == 2, "Area light is not sampled"),
                179 => anyhow::ensure!(
                    s.max_samples == 100_000,
                    "Drawing the UI changed a loaded value"
                ),
                187 => {
                    anyhow::ensure!(g.hdr_active, "Forced HDR output did not start");
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
                        for i in 0..3 {
                            worst = worst.max((h[i] - want[i]).abs() / (want[i].abs() + 0.01));
                            brightest = brightest.max(h[i]);
                        }
                    }
                    // RGBA16F keeps 11 significant bits.
                    anyhow::ensure!(
                        worst < 2e-3 && brightest <= 12.5 * 1.001 && brightest > 4.,
                        "HDR transform mismatch: relative error {worst}, brightest {brightest}"
                    );
                }
                188 => anyhow::ensure!(!g.hdr_active, "SDR output did not return"),
                241 => anyhow::ensure!(
                    g.samples >= 2 * s.spp,
                    "Light painting restarted the exposure ({} samples)",
                    g.samples
                ),
                244 => {
                    let packed = lighting::materials(s);
                    anyhow::ensure!(
                        packed.len() == 5 + s.objects.len() + s.lights.len() + 2
                            && (packed[7].lobes[3] as u32 & 4) != 0,
                        "Portal records are missing"
                    );
                }
                252 => {
                    let moved = s.target[2] < 7.15;
                    self.keys_down.clear();
                    self.walking = None;
                    anyhow::ensure!(
                        moved,
                        "Walking forward did not move the camera ({:?})",
                        s.target
                    );
                }
                218 => anyhow::ensure!(!g.rr_active, "RR must be off for a panorama camera"),
                224 => anyhow::ensure!(
                    self.lut_loaded == s.lut_path && !s.lut_path.is_empty(),
                    "The test LUT was not loaded"
                ),
                239 => {
                    anyhow::ensure!(
                        !self.focus_pending && self.focus_target.is_none(),
                        "Ctrl+click focus did not settle"
                    );
                    anyhow::ensure!(
                        (0.5..20.).contains(&s.focus_distance),
                        "Focus distance {} is not plausible",
                        s.focus_distance
                    );
                    self.settings.aperture = 0.;
                }
                209 => {
                    let [w, h] = gpu::SCOPES.map(|v| v as usize);
                    anyhow::ensure!(g.scopes.len() == w * h, "Scopes were not read back");
                    let luma: u64 = g.scopes[3 * w..4 * w].iter().map(|v| *v as u64).sum();
                    anyhow::ensure!(
                        luma == (g.width * g.height) as u64,
                        "Histogram counts {luma} pixels, not {}",
                        g.width * g.height
                    );
                    let gpu::Probe {
                        linear,
                        albedo,
                        depth,
                        ..
                    } = g.probe_result.context("No light-meter reading")?;
                    anyhow::ensure!(depth > 0., "Light-meter depth is not positive");
                    anyhow::ensure!(
                        linear.iter().chain(&albedo).all(|v| v.is_finite()),
                        "Light-meter reading is not finite"
                    );
                }
                196 => anyhow::ensure!(
                    !g.upscaler_available(scene::Upscaler::Dlss)
                        || g.upscale_status.contains("active"),
                    "DLAA did not start: {}",
                    g.upscale_status
                ),
                180..=184 => {
                    let label = scene::DLSS_PRESETS[(self.rendered - 180) as usize].1;
                    anyhow::ensure!(
                        !g.upscaler_available(scene::Upscaler::Dlss)
                            || g.upscale_status.ends_with(label),
                        "DLSS preset {label} did not start: {}",
                        g.upscale_status
                    );
                }
                172..=176 => {
                    let u = s.upscaler;
                    anyhow::ensure!(
                        !u.temporal()
                            || !g.upscaler_available(u)
                            || g.upscale_status.contains("active"),
                        "{} did not start: {}",
                        u.label(),
                        g.upscale_status
                    );
                }
                138 => {
                    let (view, proj, _) = s.camera(g.width as f32 / g.height as f32);
                    let mesh = geometry::object_geometry(&s.objects[0], 0);
                    let center = mesh
                        .iter()
                        .map(|v| glam::Vec3::from(v.position))
                        .sum::<glam::Vec3>()
                        / mesh.len() as f32;
                    let ndc = (proj * view).project_point3(center);
                    let pixel = [
                        (ndc.x * 0.5 + 0.5) * g.width as f32,
                        (0.5 - ndc.y * 0.5) * g.height as f32,
                    ];
                    anyhow::ensure!(
                        s.pick_object(pixel, [g.width, g.height]).is_some(),
                        "Viewport picking missed a visible object"
                    );
                }
                _ => {}
            }
        }
        if self.require_rr && !g.rr_active {
            bail!("--require-rr: {}", g.rr_status);
        }
        if self.actions.reference {
            unsafe {
                g.capture_reference()?;
            }
            self.settings.compare = true;
            self.actions.reference = false;
        }
        if self.actions.png {
            self.actions.png = false;
            unsafe {
                g.capture(std::path::Path::new("cornell.png"), false, false)?;
            }
            self.ui.toast("Saved cornell.png");
            if let Some(st) = &mut self.stats {
                st.images_saved += 1;
            }
        }
        if self.actions.exr {
            self.actions.exr = false;
            unsafe {
                g.capture(std::path::Path::new("cornell.exr"), true, false)?;
            }
            self.ui.toast("Saved linear HDR cornell.exr");
            if let Some(st) = &mut self.stats {
                st.images_saved += 1;
            }
        }
        if self.actions.hdr_png {
            self.actions.hdr_png = false;
            if g.hdr_active {
                let pixels = unsafe { g.read_floats(26)? };
                display::write_hdr_png(
                    std::path::Path::new("cornell-hdr.png"),
                    &pixels,
                    g.width,
                    g.height,
                )?;
                self.ui.toast("Saved cornell-hdr.png (PQ / BT.2020)");
                if let Some(st) = &mut self.stats {
                    st.images_saved += 1;
                }
            } else {
                self.ui.error("HDR PNG needs HDR output");
            }
        }
        if self.actions.high_res {
            self.actions.high_res = false;
            let mut export = s.clone();
            export.scale = 1.;
            // Progressive at full resolution: DLAA would restart every frame.
            export.native_aa = false;
            export.max_samples = self.ui.export_samples;
            export.vsync = false;
            export.rotate = false;
            export.compare = false;
            let (w, h) = (g.width, g.height);
            let factor = (self.ui.export_scale as f32).min(4096. / w.max(h) as f32);
            let ew = ((w as f32 * factor).round() as u32).max(1);
            let eh = ((h as f32 * factor).round() as u32).max(1);
            unsafe {
                g.resize(ew, eh, &export)?;
                while g.samples < export.max_samples {
                    g.render(
                        &export,
                        true,
                        1. / 60.,
                        &[],
                        &egui::TexturesDelta::default(),
                        1.,
                    )?;
                }
                g.capture(std::path::Path::new("cornell-high-res.png"), false, false)?;
                g.resize(w, h, s)?;
            }
            let text = format!(
                "Saved cornell-high-res.png · {ew}×{eh} · {} spp",
                self.ui.export_samples
            );
            self.ui.toast(text);
            if let Some(st) = &mut self.stats {
                st.images_saved += 1;
            }
        }
        let cli_turntable = if self.rendered == 2 {
            self.turntable_cli.take()
        } else {
            None
        };
        if self.actions.turntable || cli_turntable.is_some() {
            self.actions.turntable = false;
            let t = cli_turntable.unwrap_or_else(|| video::Turntable {
                frames: self.ui.tt_frames,
                spp: self.ui.tt_spp,
                degrees: self.ui.tt_degrees,
                fps: self.ui.tt_fps,
                path: PathBuf::from(&self.ui.tt_path),
            });
            let text = unsafe { video::render(g, s, &t)? };
            println!("TURNTABLE {text}");
            self.ui.toast(text);
        }
        if self.debug {
            let messages = unsafe { g.debug_messages() };
            for msg in &messages {
                eprintln!("D3D12: {msg}");
            }
            anyhow::ensure!(
                messages.is_empty(),
                "D3D12 validation produced {} warning/error messages",
                messages.len()
            );
        }
        self.rendered += 1;
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
            "Cornell RTX · {} · {} spp · {:.2} ms GPU · {}",
            g.adapter_name,
            g.samples,
            g.gpu_ms,
            if g.rr_active {
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
        if let Err(e) = self.redraw() {
            self.fail(event_loop, e);
            return;
        }
        let gpu_ms = self.gpu.as_ref().unwrap().gpu_ms;
        match &mut self.timing {
            Some((_, sum)) => *sum += gpu_ms,
            None if self.rendered == 10 => self.timing = Some((Instant::now(), 0.)),
            None => {}
        }
        if self.frames.is_some_and(|f| self.rendered >= f) {
            let (frame_ms, mean_gpu_ms) = match self.timing {
                Some((start, sum)) if self.rendered > 10 => {
                    let n = (self.rendered - 10) as f64;
                    (start.elapsed().as_secs_f64() * 1000. / n, sum / n)
                }
                _ => (0., 0.),
            };
            let g = self.gpu.as_mut().unwrap();
            if let Some(path) = &self.capture {
                let hdr = path.extension().is_some_and(|v| v == "exr");
                // name.hdr.png: the HDR screen image as a PQ PNG (needs HDR output).
                let result = if path.to_string_lossy().ends_with(".hdr.png") {
                    if g.hdr_active {
                        unsafe { g.read_floats(26) }
                            .and_then(|p| display::write_hdr_png(path, &p, g.width, g.height))
                    } else {
                        Err(anyhow::anyhow!(
                            "--capture *.hdr.png needs HDR output (--set hdr_mode=2)"
                        ))
                    }
                } else {
                    unsafe { g.capture(path, hdr, self.capture_ui && !hdr) }
                };
                if let Err(e) = result {
                    self.fail(event_loop, e);
                    return;
                }
            }
            println!(
                "RENDER_OK frames={} samples={} gpu_ms={:.3} rr={} size={}x{} gpu_ms_mean={:.3} frame_ms_mean={:.3}",
                self.rendered,
                g.samples,
                g.gpu_ms,
                g.rr_active,
                g.width,
                g.height,
                mean_gpu_ms,
                frame_ms
            );
            event_loop.exit();
        }
    }
}
/// A slider that only clamps what the user drags. egui's default clamps the stored value
/// whenever the slider is drawn, which would silently rewrite valid values loaded from a file
/// or `--set` that lie outside the slider's convenient range (and restart the render).
fn slider<'a, N: egui::emath::Numeric>(
    value: &'a mut N,
    range: std::ops::RangeInclusive<N>,
) -> egui::Slider<'a> {
    egui::Slider::new(value, range).clamping(egui::SliderClamping::Edits)
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("Cornell RTX")
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
            let hwnd = match window.window_handle()?.as_raw() {
                RawWindowHandle::Win32(h) => windows::Win32::Foundation::HWND(h.hwnd.get() as _),
                _ => bail!("Windows only"),
            };
            let size = window.inner_size();
            let mut gpu =
                unsafe { Gpu::new(hwnd, size.width, size.height, &self.settings, self.debug)? };
            gpu.keep_ui_frame = self.capture_ui;
            self.hwnd = Some(hwnd);
            if cfg!(debug_assertions) && !self.self_test {
                self.hot = hotreload::HotReload::new();
            }
            if !self.hidden {
                self.stats = Some(stats::Stats::load());
                self.gamepad = gilrs::Gilrs::new().ok();
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
        match event {
            WindowEvent::CloseRequested => {
                if let Some(st) = &self.stats {
                    st.save();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0
                    && size.height > 0
                    && let Some(g) = &mut self.gpu
                    && let Err(e) = unsafe { g.resize(size.width, size.height, &self.settings) }
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
                if self.ui.transform.is_some() {
                    self.update_transform();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let transforming = self.ui.transform.as_ref().map(|t| t.from_handle);
                if state == ElementState::Released {
                    if transforming == Some(true) && button == MouseButton::Left {
                        self.finish_transform();
                    } else if self.drag == Some(MouseButton::Left)
                        && self.drag_distance < 3.
                        && self.settings.perspective()
                        && !self.egui.is_pointer_over_area()
                        && let Some(g) = &self.gpu
                    {
                        // A click selects what is under the pointer, read from the GPU's
                        // object-id buffer next frame; empty space deselects, as in Blender.
                        let _ = g;
                        self.pick_point = Some(self.cursor);
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
                } else if !consumed {
                    let mode = self.tool_mode();
                    if button == MouseButton::Left
                        && let Some(h) = self.ui.hover
                        && self.start_transform(mode, Some(h))
                    {
                        self.drag = None;
                    } else {
                        self.drag = Some(button);
                        self.drag_distance = 0.;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let dx = (position.x - self.cursor[0]) as f32;
                let dy = (position.y - self.cursor[1]) as f32;
                self.cursor = [position.x, position.y];
                if let Some(t) = &mut self.ui.transform {
                    // Shift moves the pointer's effect at a tenth of the speed.
                    let k = if self.shift { 0.1 } else { 1. };
                    t.mouse += glam::Vec2::new(dx, dy) * k;
                    self.update_transform();
                } else if self.walking.is_some() && self.drag == Some(MouseButton::Left) {
                    self.drag_distance += dx.abs() + dy.abs();
                    walk::Walk::look(&mut self.settings, dx, dy);
                } else if let Some(button) = self.drag {
                    self.drag_distance += dx.abs() + dy.abs();
                    let s = &mut self.settings;
                    if button == MouseButton::Right || self.shift {
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
                    self.settings.distance =
                        (self.settings.distance * (-d * 0.1).exp()).clamp(0.5, 200.);
                }
            }
            // Walk mode owns WASD / Q / E while it runs; held keys are tracked for movement.
            WindowEvent::KeyboardInput { ref event, .. }
                if self.walking.is_some() && !self.egui.wants_keyboard_input() =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys_down.insert(code);
                        if !event.repeat && matches!(code, KeyCode::Escape | KeyCode::Backquote) {
                            self.walking = None;
                            self.keys_down.clear();
                            self.ui.toast("Walk mode off");
                        }
                    } else {
                        self.keys_down.remove(&code);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && !self.egui.wants_keyboard_input() =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if self.transform_key(code) {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    let selected = !matches!(
                        self.ui.selection,
                        ui::Selection::None | ui::Selection::Stage | ui::Selection::Camera
                    );
                    let over_view = !self.egui.is_pointer_over_area();
                    match code {
                        KeyCode::KeyZ | KeyCode::KeyY if self.ctrl => {
                            let redo = code == KeyCode::KeyY || self.shift;
                            let done = if redo {
                                self.ui.redo(&mut self.settings)
                            } else {
                                self.ui.undo(&mut self.settings)
                            };
                            let text = match (redo, done) {
                                (false, true) => "Undo",
                                (true, true) => "Redo",
                                (false, false) => "Nothing to undo",
                                (true, false) => "Nothing to redo",
                            };
                            self.ui.toast(text);
                        }
                        // Blender's modal transforms, when an object or light is selected.
                        KeyCode::KeyG | KeyCode::KeyS | KeyCode::KeyR
                            if selected && over_view && !self.ctrl =>
                        {
                            let mode = match code {
                                KeyCode::KeyG => gizmo::Mode::Move,
                                KeyCode::KeyR => gizmo::Mode::Rotate,
                                _ => gizmo::Mode::Scale,
                            };
                            if !self.start_transform(mode, None) {
                                self.ui.toast("The main light only moves and scales");
                            }
                        }
                        // Escape deselects; closing the window is the only way to quit.
                        KeyCode::Escape => self.ui.selection = ui::Selection::None,
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
                        KeyCode::Space => {
                            self.paused = !self.paused;
                            self.ui
                                .toast(if self.paused { "Paused" } else { "Resumed" });
                        }
                        KeyCode::KeyR => {
                            self.actions.reset = true;
                            self.ui.toast("Restarted");
                        }
                        KeyCode::KeyF => self.actions.frame_scene = true,
                        KeyCode::KeyL => self.toggle_painting(),
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
                        KeyCode::F12 => self.actions.png = true,
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
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.hidden {
            event_loop.set_control_flow(ControlFlow::Poll);
            self.tick(event_loop);
            return;
        }
        let active = self.gpu.as_ref().is_some_and(|g| {
            (!self.paused && g.samples < self.settings.max_samples)
                || self.frames.is_some()
                || self.settings.rotate
        });
        event_loop.set_control_flow(if active {
            ControlFlow::Poll
        } else {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(200))
        });
        if active && let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let saver::Mode::Preview(parent) = saver::mode(&args) {
        saver::preview(parent);
        return Ok(());
    }
    // Renders the missing scene thumbnails (each one a headless run of this exe) and exits.
    if std::env::args().any(|a| a == "--make-thumbnails") {
        let missing: Vec<_> = scenes::list()
            .into_iter()
            .filter(|e| e.thumbnail().is_some_and(|t| !t.is_file()))
            .collect();
        println!("{} thumbnails to render", missing.len());
        for result in scenes::render_thumbnails(missing) {
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
