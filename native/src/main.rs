#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod display;
mod editor;
mod geometry;
mod gizmo;
mod gpu;
mod lighting;
mod presets;
mod scene;
mod scenes;
mod teapot_data;
mod ui;
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
}
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
        s.light_position,
        s.light_size,
        s.light_aspect,
        s.cornell_room,
        s.floor_extent,
        s.lights
            .iter()
            .map(|l| (l.enabled, l.position, l.rotation, l.size))
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
        let mut all = std::env::args().skip(1);
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
                    frames = Some(204 + scenes::list().len() as u32);
                }
                "--no-vsync" => s.vsync = false,
                "--help" => {
                    println!(
                        "Cornell RTX: --frames N --capture image.png|image.exr --width N --height N --scale 0.25..1 --spp 1..16 --samples N --preset 0..23 --settings file.json --set key=json --rr --no-rr --require-rr --raw --debug --headless --no-vsync --capture-ui --ui-tab scene|render|look|export --ui-advanced --ui-scenes --ui-tool select|move|rotate|scale --ui-select stage|camera|main-light|light:N|object:N --capture-sequence dir --make-thumbnails --orbit-test --drag-test --self-test"
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
                n if n >= 204 => {
                    let entries = scenes::list();
                    if let Some(e) = entries.get((n - 204) as usize) {
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
        let g = self.gpu.as_mut().unwrap();
        if self.actions.frame_scene {
            self.settings
                .frame_objects(g.width as f32 / g.height as f32);
            self.actions.frame_scene = false;
        }
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
                g.reset(old != new);
                self.last_key = key;
            }
            self.checked = Some(s.clone());
        }
        if self.actions.reset {
            g.reset(true);
            self.actions.reset = false;
        }
        let draw = (!self.paused || g.samples == 0) && g.samples < s.max_samples;
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
        }
        if self.actions.exr {
            self.actions.exr = false;
            unsafe {
                g.capture(std::path::Path::new("cornell.exr"), true, false)?;
            }
            self.ui.toast("Saved linear HDR cornell.exr");
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
                        .with_visible(!self.hidden),
                )?,
            );
            let hwnd = match window.window_handle()?.as_raw() {
                RawWindowHandle::Win32(h) => windows::Win32::Foundation::HWND(h.hwnd.get() as _),
                _ => bail!("Windows only"),
            };
            let size = window.inner_size();
            let mut gpu =
                unsafe { Gpu::new(hwnd, size.width, size.height, &self.settings, self.debug)? };
            gpu.keep_ui_frame = self.capture_ui;
            self.hwnd = Some(hwnd);
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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
                        && !self.egui.is_pointer_over_area()
                        && let Some(g) = &self.gpu
                    {
                        // A click selects the object or light under the pointer; empty space
                        // deselects, as in Blender.
                        match gizmo::pick(
                            &self.settings,
                            [self.cursor[0] as f32, self.cursor[1] as f32],
                            [g.width, g.height],
                        ) {
                            Some(sel) => {
                                self.ui.selection = sel;
                                self.ui.tab = ui::Tab::Scene;
                                self.ui.panel_hidden = false;
                                self.show_ui = true;
                            }
                            None => self.ui.selection = ui::Selection::None,
                        }
                    }
                    self.drag = None;
                } else if transforming == Some(false) {
                    // Modal G / R / S: left click confirms, right click cancels.
                    if button == MouseButton::Right {
                        self.cancel_transform();
                    } else {
                        self.finish_transform();
                    }
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
