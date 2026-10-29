#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod editor;
mod geometry;
mod gpu;
mod lighting;
mod presets;
mod scene;
mod teapot_data;
use anyhow::{Context, Result, bail};
use gpu::Gpu;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use scene::{Material, PRESETS, Settings};
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
    status: String,
    preset: usize,
    frames: Option<u32>,
    rendered: u32,
    /// Timing for the RENDER_OK line, from frame 10 on (after startup and BVH builds).
    timing: Option<(Instant, f64)>,
    capture: Option<PathBuf>,
    capture_sequence: Option<PathBuf>,
    orbit_test: bool,
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
    selected_object: usize,
    add_shape: scene::Shape,
    object_search: String,
    show_objects: bool,
    actions: Actions,
    export_scale: u32,
    export_samples: u32,
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
        s.light_position,
        s.light_size,
        s.light_aspect,
        s.cornell_room,
        s.floor_extent,
        s.lights
            .iter()
            .map(|l| (l.position, l.rotation, l.size))
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
        let mut require_rr = false;
        let mut debug = false;
        let mut hidden = false;
        let mut capture_ui = false;
        let mut self_test = false;
        let mut selected_preset = 0;
        let mut size = [1440, 900];
        // --preset and --settings replace the whole scene, so apply them before every other
        // switch: the command line then means the same in any order.
        let mut ordered: Vec<String> = vec![];
        let mut rest: Vec<String> = vec![];
        let mut all = std::env::args().skip(1);
        while let Some(arg) = all.next() {
            if arg == "--preset" || arg == "--settings" {
                anyhow::ensure!(
                    ordered.is_empty(),
                    "Give only one of --preset and --settings"
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
                "--self-test" => {
                    self_test = true;
                    hidden = true;
                    debug = true;
                    frames = Some(180);
                }
                "--no-vsync" => s.vsync = false,
                "--help" => {
                    println!(
                        "Cornell RTX: --frames N --capture image.png|image.exr --width N --height N --scale 0.25..1 --spp 1..16 --samples N --preset 0..23 --settings file.json --set key=json --rr --no-rr --require-rr --raw --debug --headless --no-vsync --capture-ui --capture-sequence dir --orbit-test --self-test"
                    );
                    for (i, name) in PRESETS.iter().enumerate() {
                        println!("  preset {i:2}: {name}");
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
        egui.set_visuals(egui::Visuals::dark());
        Ok(Self {
            window: None,
            gpu: None,
            egui,
            input: None,
            last_key: transport_key(&s),
            geom_key: geometry_key(&s),
            material_key: material_key(&s),
            checked: None,
            selected_object: 0,
            add_shape: scene::Shape::Sphere,
            object_search: String::new(),
            show_objects: true,
            settings: s,
            paused: false,
            show_ui: !hidden || capture_ui || self_test,
            last: Instant::now(),
            cursor: [0.; 2],
            drag: None,
            drag_distance: 0.,
            shift: false,
            status: String::new(),
            preset: selected_preset,
            frames,
            rendered: 0,
            timing: None,
            capture,
            capture_sequence,
            orbit_test,
            require_rr,
            capture_ui,
            self_test,
            debug,
            hidden,
            size,
            failed: None,
            actions: Actions::default(),
            export_scale: 2,
            export_samples: 128,
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
                    self.export_scale = 1;
                    self.export_samples = 16;
                    self.actions.high_res = true;
                }
                // Every upscaler on the native path at half resolution, then back to RR.
                172..=176 => {
                    self.settings.rr = false;
                    self.settings.scale = 0.5;
                    self.settings.upscaler = scene::Upscaler::ALL[(self.rendered - 172) as usize];
                }
                177 => self.settings.rr = true,
                // Valid, but above the sample-limit slider's range (the panel is open).
                178 => self.settings.max_samples = 100_000,
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
                Ok(()) => self.status = "Saved settings.json".into(),
                Err(e) => self.status = e.to_string(),
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
                    self.status = "Loaded settings.json".into();
                }
                Err(e) => self.status = format!("Load rejected: {e}"),
            }
        }
        let g = self.gpu.as_mut().unwrap();
        if self.actions.frame_scene {
            self.settings
                .frame_objects(g.width as f32 / g.height as f32);
            self.actions.frame_scene = false;
        }
        let s = &self.settings.clone();
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
                if ["rr_preset", "rr", "upscaler"]
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
            self.status = "Saved cornell.png".into();
        }
        if self.actions.exr {
            self.actions.exr = false;
            unsafe {
                g.capture(std::path::Path::new("cornell.exr"), true, false)?;
            }
            self.status = "Saved linear HDR cornell.exr".into();
        }
        if self.actions.high_res {
            self.actions.high_res = false;
            let mut export = s.clone();
            export.scale = 1.;
            export.max_samples = self.export_samples;
            export.vsync = false;
            export.rotate = false;
            export.compare = false;
            let (w, h) = (g.width, g.height);
            let factor = (self.export_scale as f32).min(4096. / w.max(h) as f32);
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
            self.status = format!(
                "Saved cornell-high-res.png · {ew}×{eh} · {} spp",
                self.export_samples
            );
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
                if let Err(e) = unsafe { g.capture(path, hdr, self.capture_ui && !hdr) } {
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
    fn ui(&mut self, ctx: &egui::Context) {
        // Loaded and randomized settings carry their preset by name.
        let known = PRESETS.iter().position(|n| *n == self.settings.scene_name);
        if let Some(i) = known {
            self.preset = i;
        }
        let g = self.gpu.as_ref().unwrap();
        egui::Window::new("CORNELL / RTX LAB").default_pos([16.,16.]).default_width(330.).vscroll(true).max_height(ctx.screen_rect().height()-32.).show(ctx,|ui|{
            ui.label(egui::RichText::new("RUST  /  DIRECT3D 12  /  DXR 1.1").small().color(egui::Color32::from_rgb(93,194,211)));
            ui.label(&g.adapter_name);
            ui.label(format!("{} objects · {} triangles · {} emissive triangles",self.settings.objects.len(),g.triangle_count,g.emitter_count));
            ui.horizontal(|ui|{ui.monospace(format!("{} spp  ·  {:.2} ms GPU",g.samples,g.gpu_ms));if g.samples>=self.settings.max_samples{ui.label("Converged");}});
            ui.label(egui::RichText::new(&g.rr_status).small());ui.separator();
            ui.horizontal(|ui|{if ui.button(if self.paused{"▶ Resume"}else{"⏸ Pause"}).clicked(){self.paused= !self.paused;}
if ui.button("Reset [R]").clicked(){self.actions.reset=true;}
if ui.button("A/B [C]").clicked(){self.actions.reference=true;}});
            egui::ComboBox::from_label("Scene preset").selected_text(self.settings.scene_name.clone()).show_ui(ui,|ui|{for (i,p) in PRESETS.iter().enumerate(){if ui.selectable_value(&mut self.preset,i,*p).changed(){self.settings.preset(i);}}});
            ui.label(egui::RichText::new(if known.is_some(){presets::description(self.preset)}else{"Custom scene from a settings file."}).small());
            ui.horizontal(|ui|{if ui.button("◀").clicked(){self.preset=(self.preset+PRESETS.len()-1)%PRESETS.len();self.settings.preset(self.preset);}
if ui.button("▶").clicked(){self.preset=(self.preset+1)%PRESETS.len();self.settings.preset(self.preset);}
if ui.button("Frame scene [F]").clicked(){self.actions.frame_scene=true;}ui.checkbox(&mut self.show_objects,"Objects");});
            ui.horizontal(|ui|{if ui.button("Randomize").clicked(){let seed=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos()|1;self.settings.randomize(seed);}
if ui.button("Reset all").clicked(){self.settings=Settings::default();self.preset=0;}});
            let s=&mut self.settings;
            egui::CollapsingHeader::new("Reconstruction & rendering").default_open(true).show(ui,|ui|{
                ui.checkbox(&mut s.rr,"DLSS Ray Reconstruction").on_hover_text("Uses NVIDIA's neural ray denoiser and upscaler. Native à-trous is the fallback.");
                let model=match s.rr_preset {4=>"Transformer D",5=>"Transformer E",6=>"RR2 / F",_=>"NVIDIA default"};
                egui::ComboBox::from_label("RR model").selected_text(model).show_ui(ui,|ui|{for (id,label) in [(0,"NVIDIA default"),(4,"Transformer D"),(5,"Transformer E"),(6,"RR2 / F")]{ui.selectable_value(&mut s.rr_preset,id,label);}});
                egui::ComboBox::from_label("Quality mode").selected_text(if s.scale>=0.99{"Native (100%)"}else if s.scale>=0.62{"Quality (67%)"}else if s.scale>=0.54{"Balanced (58%)"}else if s.scale>=0.42{"Performance (50%)"}else{"Ultra performance (33%)"}).show_ui(ui,|ui|{for (scale,label) in [(1.,"Native (100%)"),(2./3.,"Quality (67%)"),(0.58,"Balanced (58%)"),(0.5,"Performance (50%)"),(1./3.,"Ultra performance (33%)")]{ui.selectable_value(&mut s.scale,scale,label);}});
                // Without RR, a render scale below 1 needs an upscaler; temporal ones resolve detail from jittered frames.
                ui.add_enabled_ui(!g.rr_active,|ui|{egui::ComboBox::from_label("Upscaler").selected_text(s.upscaler.label()).show_ui(ui,|ui|{for u in scene::Upscaler::ALL{let label=if g.upscaler_available(u){u.label().to_string()}else{format!("{} (unavailable)",u.label())};ui.selectable_value(&mut s.upscaler,u,label);}});}).response.on_hover_text("Used when Ray Reconstruction is off and the quality mode renders below 100%. RR does its own upscaling.");
                if !g.upscale_status.is_empty(){ui.label(egui::RichText::new(&g.upscale_status).small());}
                ui.checkbox(&mut s.temporal,"Reuse temporal history");ui.checkbox(&mut s.denoise,"Native à-trous denoiser [D]");
                ui.add(crate::slider(&mut s.denoise_strength,0. ..=1.).text("Denoise strength"));
                ui.add(crate::slider(&mut s.scale,0.25..=1.).text("Render scale"));
                ui.label(format!("Render {} × {} · output {} × {}",g.rw,g.rh,g.width,g.height));
                ui.add(crate::slider(&mut s.spp,1..=16).text("Samples / frame"));ui.add(crate::slider(&mut s.bounces,1..=32).text("Bounces"));ui.add(crate::slider(&mut s.max_samples,16..=65536).logarithmic(true).text("Sample limit"));
                ui.checkbox(&mut s.sobol,"Owen-scrambled Sobol (native)").on_hover_text("DLSS RR uses independent white noise as recommended by NVIDIA. This controls the native progressive path.");ui.add(crate::slider(&mut s.firefly,0. ..=50.).text("Firefly clamp (0 = off)"));ui.checkbox(&mut s.vsync,"VSync");
            });
            egui::CollapsingHeader::new("Stage & surfaces").show(ui,|ui|{
                ui.checkbox(&mut s.cornell_room,"Cornell enclosure");ui.add(crate::slider(&mut s.floor_extent,0.5..=40.).text("Floor extent"));ui.add(crate::slider(&mut s.sky,0. ..=4.).text("Sky illumination"));
                color(ui,"Left wall",&mut s.left);color(ui,"Right wall",&mut s.right);color(ui,"Room / floor",&mut s.base);color(ui,"Background",&mut s.background);ui.collapsing("Wall material",|ui|material_ui(ui,&mut s.walls));ui.collapsing("Floor material",|ui|material_ui(ui,&mut s.floor));
            });
            egui::CollapsingHeader::new("Lighting").show(ui,|ui|{
                ui.checkbox(&mut s.light_on,"Main area light");ui.checkbox(&mut s.light_visible,"Main light visible to camera");ui.add(crate::slider(&mut s.light_intensity,0. ..=100.).text("Intensity"));ui.add(crate::slider(&mut s.light_size,0.05..=20.).text("Size"));ui.add(crate::slider(&mut s.light_aspect,0.25..=4.).text("Aspect"));ui.add(crate::slider(&mut s.light_temperature,1000. ..=12000.).text("Kelvin"));color(ui,"Gel",&mut s.light_tint);
                for (i,label) in ["X","Y","Z"].iter().enumerate(){ui.add(crate::slider(&mut s.light_position[i],if i==1{-5. ..=20.}else{-20. ..=20.}).text(*label));}
                editor::lights(ui,s);
                ui.checkbox(&mut s.glass_shadow,"Glass casts shadows");ui.add(crate::slider(&mut s.direct,0. ..=4.).text("Direct intensity"));ui.add(crate::slider(&mut s.indirect,0. ..=4.).text("Indirect intensity"));
            });
            egui::CollapsingHeader::new("Camera").show(ui,|ui|{
                ui.checkbox(&mut s.rotate,"Auto orbit [A]");ui.add(crate::slider(&mut s.rotate_speed,-30. ..=30.).text("Degrees / second"));ui.add(crate::slider(&mut s.fov,10. ..=100.).text("Field of view"));ui.add(crate::slider(&mut s.distance,0.5..=200.).logarithmic(true).text("Distance"));
                if ui.button("Reset camera").clicked(){let mut d=Settings::default();d.preset(self.preset);s.yaw=d.yaw;s.pitch=d.pitch;s.distance=d.distance;s.target=d.target;s.fov=d.fov;}
                ui.label("Drag: orbit · Right / Shift-drag: pan\nWheel: dolly · Tab: hide controls");
            });
            egui::CollapsingHeader::new("Display & export").default_open(true).show(ui,|ui|{
                let views=["Beauty","Normals","Albedo","Depth","Direct","Indirect","Specular","Convergence"];
                egui::ComboBox::from_label("View [1–8]").selected_text(views[s.view as usize]).show_ui(ui,|ui|{for(i,n)in views.iter().enumerate(){ui.selectable_value(&mut s.view,i as u32,*n);}});
                let maps=["Linear","Reinhard","Reinhard white","ACES","Uncharted 2","High contrast"];
                egui::ComboBox::from_label("Tone map").selected_text(maps[s.tonemap as usize]).show_ui(ui,|ui|{for(i,n)in maps.iter().enumerate(){ui.selectable_value(&mut s.tonemap,i as u32,*n);}});
                ui.add(crate::slider(&mut s.exposure,0. ..=10.).text("Exposure"));ui.add(crate::slider(&mut s.gamma,0.1..=4.).text("Gamma"));
                ui.checkbox(&mut s.compare,"A/B reference");if s.compare{ui.add(crate::slider(&mut s.split,0. ..=1.).text("Split"));}
                ui.horizontal(|ui|{if ui.button("PNG").clicked(){self.actions.png=true;}
if ui.button("HDR EXR").clicked(){self.actions.exr=true;}
if ui.button("Save settings").clicked(){self.actions.save=true;}
if ui.button("Load").clicked(){self.actions.load=true;}});
                ui.label(egui::RichText::new("For true high-resolution renders use --width / --height / --scale 1 --frames N --capture file.png").small());
                ui.horizontal(|ui|{ui.add(crate::slider(&mut self.export_scale,1..=4).text("Export scale"));});
                ui.add(crate::slider(&mut self.export_samples,16..=2048).logarithmic(true).text("Export samples"));
                if ui.button("Render high-resolution PNG").clicked(){self.actions.high_res=true;}
            });
            if !self.status.is_empty(){ui.label(&self.status);}
        });
        editor::objects(
            ctx,
            &mut self.settings,
            &mut self.selected_object,
            &mut self.add_shape,
            &mut self.object_search,
            &mut self.show_objects,
            g.width as f32 / g.height as f32,
        );
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
fn color(ui: &mut egui::Ui, label: &str, c: &mut [f32; 3]) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(c);
        ui.label(label);
    });
}
fn material_ui(ui: &mut egui::Ui, m: &mut Material) {
    editor::material_ui(ui, m)
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
            WindowEvent::RedrawRequested => {
                if !self.hidden {
                    self.tick(event_loop);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.shift = m.state().shift_key(),
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released {
                    if self.drag == Some(MouseButton::Left)
                        && self.drag_distance < 3.
                        && !self.egui.is_pointer_over_area()
                        && let Some(g) = &self.gpu
                        && let Some(index) = self.settings.pick_object(
                            [self.cursor[0] as f32, self.cursor[1] as f32],
                            [g.width, g.height],
                        )
                    {
                        self.selected_object = index;
                        self.show_objects = true;
                    }
                    self.drag = None;
                } else if !consumed {
                    self.drag = Some(button);
                    self.drag_distance = 0.;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let dx = (position.x - self.cursor[0]) as f32;
                let dy = (position.y - self.cursor[1]) as f32;
                self.cursor = [position.x, position.y];
                if let Some(button) = self.drag {
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
                    match code {
                        KeyCode::Escape => event_loop.exit(),
                        KeyCode::Tab => self.show_ui = !self.show_ui,
                        KeyCode::Space => self.paused = !self.paused,
                        KeyCode::KeyR => self.actions.reset = true,
                        KeyCode::KeyF => self.actions.frame_scene = true,
                        KeyCode::Delete => {
                            // Only with the object list open, so the deletion is visible.
                            if self.show_objects
                                && self.selected_object < self.settings.objects.len()
                            {
                                self.settings.objects.remove(self.selected_object);
                                self.selected_object = self
                                    .selected_object
                                    .min(self.settings.objects.len().saturating_sub(1));
                            }
                        }
                        KeyCode::KeyA => self.settings.rotate = !self.settings.rotate,
                        KeyCode::KeyD => self.settings.denoise = !self.settings.denoise,
                        KeyCode::KeyC => self.actions.reference = true,
                        KeyCode::F12 => self.actions.png = true,
                        KeyCode::Digit1 => self.settings.view = 0,
                        KeyCode::Digit2 => self.settings.view = 1,
                        KeyCode::Digit3 => self.settings.view = 2,
                        KeyCode::Digit4 => self.settings.view = 3,
                        KeyCode::Digit5 => self.settings.view = 4,
                        KeyCode::Digit6 => self.settings.view = 5,
                        KeyCode::Digit7 => self.settings.view = 6,
                        KeyCode::Digit8 => self.settings.view = 7,
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
