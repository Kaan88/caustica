//! The `--self-test` script (edits per frame, before rendering) and its checks (after
//! rendering). The frame map is in docs/claude/testing.md.
use crate::*;

impl App {
    /// The scripted edits of frame `self.rendered`.
    pub(crate) fn self_test_script(&mut self) -> Result<()> {
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
            8 => self.settings.denoiser = crate::scene::Denoiser::Native,
            9 => self.settings.denoiser = crate::scene::Denoiser::Rr,
            // Frames 10–125 cycle all presets; 23 is the empty stage.
            // Physical sky (track 9): noon, then night, then off.
            126 => {
                self.settings.physical_sky.on = true;
                self.settings.physical_sky.time = 12.;
            }
            127 => self.settings.physical_sky.time = 1.,
            128 => self.settings.physical_sky.on = false,
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
            // Every image format through the shared writer: the quick saves, then a JPEG
            // high-resolution still (one pass, so it ends within the next frame).
            141 => {
                for f in video::ImageFormat::ALL {
                    let _ = std::fs::remove_file(f.file("caustica"));
                    let _ = std::fs::remove_file(f.file("caustica-high-res"));
                }
                self.actions.save_image = Some(video::ImageFormat::Jpeg);
            }
            142 => {
                self.expect_image(video::ImageFormat::Jpeg.file("caustica"), None)?;
                self.actions.save_image = Some(video::ImageFormat::Exr);
            }
            143 => {
                self.expect_image(video::ImageFormat::Exr.file("caustica"), None)?;
                self.ui.export_scale = 1;
                self.ui.export_samples = 4;
                self.ui.export_format = video::ImageFormat::Jpeg;
                self.actions.high_res = true;
            }
            145 => {
                anyhow::ensure!(self.export.is_none(), "The JPEG still did not finish");
                let g = self.gpu.as_ref().unwrap();
                let size = [g.width, g.height];
                self.expect_image(
                    video::ImageFormat::Jpeg.file("caustica-high-res"),
                    Some(size),
                )?;
                self.ui.export_format = video::ImageFormat::Png;
            }
            // Keyframes (track 8): object 0 keyed at 0 s and 1 s, read back at 0.5 s; then
            // a two-frame timeline export with a 180° shutter (motion blur).
            146 => {
                self.ui.selection = ui::Selection::Object(0);
                self.timeline.time = 0.;
                self.key_selection(false);
                self.timeline.time = 1.;
                self.settings.objects[0].position[0] += 0.4;
                self.key_selection(false);
                self.timeline.time = 0.5;
                self.ui.show_timeline = true;
            }
            147 => {
                let x = self.settings.objects[0].position[0];
                let start = self
                    .settings
                    .animation
                    .track("objects/0/position/0")
                    .map_or(f64::NAN, |t| t.keys[0].value) as f32;
                anyhow::ensure!(
                    self.settings.animation.tracks.len() == 10 && (x - start - 0.2).abs() < 1e-4,
                    "Keyframes: {} tracks, x {x} from {start}",
                    self.settings.animation.tracks.len()
                );
                let a = &mut self.settings.animation;
                (a.length, a.fps, a.shutter) = (0.2, 10, 0.5);
                let folder = std::env::temp_dir().join("caustica-self-test-timeline");
                let _ = std::fs::remove_dir_all(&folder);
                let mut o = video::Options::turntable(
                    &self.settings,
                    [160, 100],
                    1,
                    8,
                    folder.with_extension("exr"),
                );
                (o.format, o.timeline) = (video::Format::Exr, true);
                self.ui.video_pending = Some(o);
            }
            149 => {
                anyhow::ensure!(self.export.is_none(), "The timeline export did not finish");
                let folder = std::env::temp_dir().join("caustica-self-test-timeline");
                let frames = std::fs::read_dir(&folder)?.count();
                anyhow::ensure!(frames == 2, "The timeline export wrote {frames} frames");
                self.settings.animation = Default::default();
                self.timeline.time = 0.;
                self.ui.selection = ui::Selection::None;
            }
            150..=157 => self.settings.view = self.rendered - 150,
            // The other pixel filters on the native path (sample albedo and coverage too).
            158 => {
                self.settings.view = 0;
                self.settings.denoiser = crate::scene::Denoiser::Native;
                self.settings.pixel_filter = 1;
                self.settings.filter_width = 3.;
            }
            159 => {
                self.settings.pixel_filter = 0;
                self.settings.filter_width = 1.;
            }
            160 => {
                self.settings.view = 0;
                self.settings.denoiser = crate::scene::Denoiser::Off;
            }
            161 => {
                self.settings.denoiser = crate::scene::Denoiser::Native;
                self.settings.pixel_filter = 2;
                self.settings.filter_width = 1.5;
            }
            162 => self.settings.denoiser = crate::scene::Denoiser::Rr,
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
            // The CPU reference in the viewport: started here, its first pass awaited in 171,
            // uploaded at the end of 171 and checked in 172.
            169 => self.actions.cpu_reference = Some(true),
            171 => {
                let live = self
                    .cpu_reference
                    .as_ref()
                    .context("The CPU reference did not start")?;
                let start = Instant::now();
                while live.running()
                    && live.state().samples == 0
                    && start.elapsed() < Duration::from_secs(60)
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            170 => {
                self.ui.export_scale = 1;
                // One pass (4 spp), so the job ends within frame 171; no stale file counts.
                self.ui.export_samples = 4;
                let _ = std::fs::remove_file(video::ImageFormat::Png.file("caustica-high-res"));
                self.actions.high_res = true;
            }
            // Every upscaler on the native path at half resolution, then back to RR.
            172..=176 => {
                if self.rendered == 172 {
                    let st = self
                        .cpu_reference
                        .as_ref()
                        .map(|l| l.state())
                        .unwrap_or_default();
                    if reference::unsupported(&self.settings).is_empty() {
                        anyhow::ensure!(
                            st.samples > 0
                                && self.settings.compare
                                && self.gpu.as_ref().is_some_and(|g| g.reference_valid),
                            "The CPU reference did not reach the viewport: {:?}",
                            st.error
                        );
                    } else {
                        anyhow::ensure!(
                            st.error.is_some(),
                            "The CPU reference took an unsupported scene"
                        );
                    }
                    self.actions.cpu_reference = Some(false);
                    self.settings.compare = false;
                }
                anyhow::ensure!(
                    self.export.is_none()
                        && video::ImageFormat::Png.file("caustica-high-res").is_file(),
                    "The high-resolution still did not finish"
                );
                self.settings.denoiser = crate::scene::Denoiser::Native;
                self.settings.scale = 0.5;
                self.settings.upscaler = scene::Upscaler::ALL[(self.rendered - 172) as usize];
            }
            177 => self.settings.denoiser = crate::scene::Denoiser::Rr,
            // A loaded value must survive drawing its slider (the Render tab is open).
            178 => {
                self.ui.tab = ui::Tab::Render;
                self.settings.max_samples = 100_000;
            }
            // Every DLSS Super Resolution preset recreates the feature, then back to RR.
            180..=184 => {
                self.settings.denoiser = crate::scene::Denoiser::Native;
                self.settings.scale = 0.5;
                self.settings.upscaler = scene::Upscaler::Dlss;
                self.settings.dlss_preset = scene::DLSS_PRESETS[(self.rendered - 180) as usize].0;
            }
            185 => {
                self.settings.dlss_preset = 0;
                self.settings.denoiser = crate::scene::Denoiser::Rr;
            }
            // scRGB output with fixed levels (works on SDR monitors too), then SDR, then Auto.
            186 => {
                self.settings.hdr_mode = 2;
                self.settings.hdr_peak = 1000.;
                self.settings.hdr_white = 200.;
            }
            // The HDR PNG needs HDR output: skipped where the swapchain offers none (checked at 187).
            187 if !self.gpu.as_ref().is_some_and(|g| g.hdr_refused) => {
                self.actions.save_image = Some(video::ImageFormat::HdrPng)
            }
            188 => {
                if !self.gpu.as_ref().is_some_and(|g| g.hdr_refused) {
                    self.expect_image(video::ImageFormat::HdrPng.file("caustica"), None)?;
                }
                self.settings.hdr_mode = 1;
            }
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
                self.settings.denoiser = crate::scene::Denoiser::Native;
                self.settings.scale = 1.;
                self.settings.upscaler = scene::Upscaler::Dlss;
                self.settings.native_aa = true;
            }
            197 => {
                self.settings.native_aa = false;
                self.settings.denoiser = crate::scene::Denoiser::Rr;
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
                    self.ui.graph.len() >= 2 && self.ui.graph.iter().all(|(_, e)| e.is_finite()),
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
                let path = std::env::temp_dir().join("caustica-self-test.cube");
                std::fs::write(&path, text)?;
                self.settings.lut_path = path.to_string_lossy().into();
            }
            // Track 17: a running simulation (a ball falls on a fixed box, a cloth drapes),
            // then a spawn mid-run joins it without a restart.
            211 => {
                let saved = Box::new(self.settings.clone());
                let s = &mut self.settings;
                s.objects.clear();
                let grey = scene::Material {
                    color: [0.7; 3],
                    ..Default::default()
                };
                s.objects.push(scene::Object {
                    name: "Table".into(),
                    shape: scene::Shape::Box,
                    body: scene::Body::Fixed,
                    position: [0., 0.],
                    grounded: false,
                    elevation: -0.7,
                    material: grey.clone(),
                    ..Default::default()
                });
                s.objects.push(scene::Object {
                    name: "Ball".into(),
                    shape: scene::Shape::Sphere,
                    position: [0.6, 0.],
                    size: 0.4,
                    grounded: false,
                    elevation: 0.5,
                    material: grey.clone(),
                    ..Default::default()
                });
                s.objects.push(scene::Object {
                    name: "Cloth".into(),
                    shape: scene::Shape::Box,
                    position: [0., 0.],
                    size: 1.6,
                    grounded: false,
                    elevation: -0.2,
                    material: grey,
                    cloth: Some(scene::Cloth {
                        resolution: 12,
                        ..Default::default()
                    }),
                    ..Default::default()
                });
                s.physics.enabled = true;
                self.physics_test = Some((saved, 0.));
            }
            212 => {
                // A fixed 8 steps (headless frames can be shorter than one step).
                let sim = self
                    .physics
                    .as_mut()
                    .context("The simulation did not start")?;
                sim.advance(&mut self.settings, 0.1);
                let ball = &self.settings.objects[1];
                anyhow::ensure!(
                    ball.elevation < 0.5,
                    "The ball did not fall ({})",
                    ball.elevation
                );
                let cloth = &self.settings.objects[2].mesh;
                anyhow::ensure!(
                    cloth.starts_with("cloth:"),
                    "The cloth has no mesh ({cloth})"
                );
                let t = sim.elapsed();
                if let Some(test) = &mut self.physics_test {
                    test.1 = t;
                }
                let mut box_ = self.settings.objects[0].clone();
                box_.name = "Spawned".into();
                box_.body = scene::Body::Dynamic;
                box_.size = 0.3;
                box_.position = [-0.6, 0.];
                box_.elevation = 0.3;
                self.settings.objects.push(box_);
            }
            213 => {
                // 4 steps: a restarted simulation would be at 0.033 s, short of `before`.
                let sim = self.physics.as_mut().context("The simulation stopped")?;
                sim.advance(&mut self.settings, 0.034);
                let before = self.physics_test.as_ref().map_or(0., |t| t.1);
                anyhow::ensure!(
                    sim.elapsed() > before,
                    "The spawn restarted the simulation ({} after {before})",
                    sim.elapsed()
                );
                anyhow::ensure!(
                    sim.moving_objects().len() == 3,
                    "Moving objects after the spawn: {:?}",
                    sim.moving_objects()
                );
                let (saved, _) = self.physics_test.take().context("No saved scene")?;
                self.settings = *saved;
                self.settings.physics.enabled = false;
            }
            // Physics grab (owner reports 2026-11-03): a press on a body grabs it; a gizmo
            // handle under the pointer wins over a body that is not the gizmo's own.
            292 => {
                // A ball at the view's centre; the simulation starts next frame (with it on, the
                // new object would reach the GPU only as a pose update).
                let t = self.settings.target;
                self.settings.objects.push(scene::Object {
                    name: "Grab test".into(),
                    shape: scene::Shape::Sphere,
                    size: 0.5,
                    stretch: 1.,
                    rotation: 0.,
                    grounded: false,
                    position: [t[0], t[2]],
                    elevation: t[1],
                    ..Default::default()
                });
                self.ui.select(ui::Selection::Stage);
            }
            293 => {
                self.settings.physics.enabled = true;
                self.step_physics(1. / 60.);
                let i = self.settings.objects.len() - 1;
                let view = gizmo::View::new(&self.settings, self.size);
                let at = view
                    .project(crate::geometry::object_frame(&self.settings.objects[i]).0)
                    .context("The test ball is off screen")?;
                self.cursor = [at.x as f64, at.y as f64];
                self.ui.hover = Some(gizmo::Handle::Center);
                self.ui.hover_inner = true;
                anyhow::ensure!(
                    !self.physics_press(),
                    "A press on a gizmo handle grabbed an unselected body in front of it"
                );
                self.ui.hover = None;
                self.ui.hover_inner = false;
                anyhow::ensure!(self.physics_press(), "A press on a body did not grab it");
                let before = self.settings.objects[i].elevation;
                self.cursor[1] -= 60.;
                anyhow::ensure!(self.physics_drag(), "The grab did not follow the pointer");
                for _ in 0..30 {
                    self.step_physics(1. / 60.);
                }
                let held = self.physics.as_ref().is_some_and(|p| p.grab.is_some());
                let now = self.settings.objects[i].elevation;
                self.physics_release();
                self.settings.physics.enabled = false;
                self.settings.objects.pop();
                anyhow::ensure!(held, "The grab was lost while held");
                anyhow::ensure!(
                    now > before,
                    "The held body did not rise ({before} -> {now})"
                );
            }
            // Spectral BDPT, then RGB BDPT after it (the spectral twin once hung the RGB one
            // that followed, 2026-11-03), then back to the path tracer.
            294 => {
                self.settings.integrator = 1;
                self.settings.spectral = true;
            }
            295 => self.settings.spectral = false,
            296 => self.settings.integrator = 0,
            // Shadow catcher with ground projection on (checked at 298), then off.
            297 => {
                self.settings.shadow_catcher = true;
                self.settings.hdri_ground = 1.7;
            }
            299 => {
                self.settings.shadow_catcher = false;
                self.settings.hdri_ground = 0.;
            }
            // Heterogeneous fog (checked at 301), then back to none.
            300 => {
                self.settings.fog.density = 0.3;
                self.settings.fog.noise = 1.5;
            }
            // The software view: the CPU's picture in the display pass, a restart on a camera move,
            // and back. Waited for here, checked in `self_test_checks`.
            303 => {
                self.settings.max_samples = 4;
                self.settings.bounces = 3;
                self.cpu_view = true;
            }
            304 | 306 => {
                let start = Instant::now();
                while self.cpu_reference.as_ref().is_some_and(|l| l.running())
                    && start.elapsed() < Duration::from_secs(60)
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            305 => self.settings.yaw += 0.5,
            302 => self.settings.fog = Default::default(),
            // Physics on while a gizmo arrow holds an object: it must not fall.
            237 => {
                self.settings.physics.enabled = true;
                self.ui.select(ui::Selection::Object(0));
                self.settings.objects[0].elevation += 0.3;
                anyhow::ensure!(
                    self.start_transform(gizmo::Mode::Move, None),
                    "The move did not start"
                );
            }
            238 => {
                let held = self
                    .ui
                    .transform
                    .as_ref()
                    .map(|t| t.original.objects[0].elevation);
                let now = self.settings.objects[0].elevation;
                anyhow::ensure!(
                    held.is_some_and(|h| (h - now).abs() < 1e-4),
                    "The held object fell from {held:?} to {now}"
                );
                self.cancel_transform();
                self.settings.objects[0].elevation -= 0.3;
                self.settings.physics.enabled = false;
            }
            // Multi-select (3b): a typed group move, duplicate, select all, delete, undo.
            233 => {
                if self.settings.objects.len() < 2 {
                    let o = self.settings.objects[0].clone();
                    self.settings.objects.push(o);
                }
                let both = vec![ui::Selection::Object(0), ui::Selection::Object(1)];
                self.ui.select_many(both, &self.settings);
                let before = self.settings.clone();
                anyhow::ensure!(
                    self.start_transform(gizmo::Mode::Move, None),
                    "The group move did not start"
                );
                if let Some(t) = &mut self.ui.transform {
                    t.typed = "0.25".into();
                }
                self.update_transform();
                self.finish_transform();
                for i in 0..2 {
                    let (a, b) = (
                        self.settings.objects[i].position,
                        before.objects[i].position,
                    );
                    anyhow::ensure!(
                        (a[0] - b[0] - 0.25).abs() < 1e-4 && a[1] == b[1],
                        "Object {i} moved to {a:?} from {b:?}"
                    );
                }
                let n = self.settings.objects.len();
                anyhow::ensure!(
                    self.ui
                        .duplicate_selected(&mut self.settings, false)
                        .is_some()
                        && self.settings.objects.len() == n + 2
                        && self.ui.targets().len() == 2,
                    "Duplicating two objects made {} objects",
                    self.settings.objects.len()
                );
                self.ui.undo(&mut self.settings);
            }
            234 => {
                let (objects, lights) = (self.settings.objects.len(), self.settings.lights.len());
                self.select_all();
                anyhow::ensure!(
                    self.ui.targets().len()
                        == self.settings.objects.iter().filter(|o| o.visible).count()
                            + self.settings.lights.iter().filter(|l| l.enabled).count(),
                    "Select all chose {} items",
                    self.ui.targets().len()
                );
                self.ui.delete_selected(&mut self.settings);
                self.ui.undo(&mut self.settings);
                anyhow::ensure!(
                    self.settings.objects.len() == objects && self.settings.lights.len() == lights,
                    "Deleting all and undoing left {} objects",
                    self.settings.objects.len()
                );
                self.ui.select(ui::Selection::None);
            }
            // The HDR10 swapchain (10-bit PQ / BT.2020) with fixed levels, then Auto again.
            226 => {
                self.settings.hdr_mode = 2;
                self.settings.hdr10 = true;
                self.settings.hdr_peak = 1000.;
                self.settings.hdr_white = 200.;
            }
            // HDR PNG, then JPEG XR: both need HDR output (skipped as at 187).
            227 | 228 if self.gpu.as_ref().is_some_and(|g| g.hdr_refused) => {}
            227 => self.actions.save_image = Some(video::ImageFormat::HdrPng),
            228 => {
                self.expect_image(video::ImageFormat::HdrPng.file("caustica"), None)?;
                let _ = std::fs::remove_file(video::ImageFormat::Jxr.file("caustica"));
                if cfg!(windows) {
                    self.actions.save_image = Some(video::ImageFormat::Jxr);
                }
            }
            229 => {
                // The image crate has no JPEG XR decoder: check WIC wrote a plausible file.
                let jxr = video::ImageFormat::Jxr.file("caustica");
                if cfg!(windows) && !self.gpu.as_ref().is_some_and(|g| g.hdr_refused) {
                    let size = std::fs::metadata(&jxr).map(|m| m.len()).unwrap_or(0);
                    anyhow::ensure!(size > 1000, "{} was not written", jxr.display());
                }
                let _ = std::fs::remove_file(&jxr);
                self.settings.hdr_mode = 0;
                self.settings.hdr10 = false;
                self.settings.hdr_peak = 0.;
                self.settings.hdr_white = 0.;
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
            // Path visualizer: record the centre pixel's paths (checked after the render).
            230 => {
                self.ui.paths.count = 8;
                self.ui.paths.request([self.size[0] / 2, self.size[1] / 2]);
            }
            // Light path expressions: a partition of every path, checked against the raw
            // beauty at 236.
            232 => {
                // The check compares render-size AOVs with the output-size beauty.
                self.settings.scale = 1.;
                self.settings.denoiser = crate::scene::Denoiser::Off;
                self.settings.view = 0;
                self.settings.lpe = ["C [LB]", "C D .* [LB]", "C [GS] .* [LB]", "C [TV] .* [LB]"]
                    .map(String::from)
                    .to_vec();
            }
            // Portals: a paired pair of boxes, then removed.
            243 => {
                self.settings.preset(0);
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
            // Flashlight mode while walking, with a hidden light that casts no shadows.
            248 => {
                self.settings.flashlight = true;
                self.settings.lights.push(scene::AreaLight {
                    visible: false,
                    shadows: false,
                    audio_band: 4,
                    ..scene::AreaLight::default()
                });
            }
            251 => {
                self.settings.flashlight = false;
                self.settings.lights.pop();
            }
            // A three-frame turntable (PNG frames when ffmpeg is missing).
            253 => {
                let path = std::env::temp_dir().join("caustica-self-test-turntable.mp4");
                let folder = path.with_extension("");
                let _ = std::fs::remove_dir_all(&folder);
                let g = self.gpu.as_mut().unwrap();
                let mut t = video::Options::turntable(
                    &self.settings,
                    [g.width, g.height],
                    3,
                    2,
                    path.clone(),
                );
                t.degrees = 30.;
                let text = unsafe { video::render(g, &self.settings.clone(), t)? };
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
                let path = std::env::temp_dir().join("caustica-self-test-pyramid.obj");
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
            // HDRI: a generated map with a small bright sun, then hidden from the camera,
            // each light-sampling mode, and cleared again.
            260 => {
                let (w, h) = (64u32, 32u32);
                let map = image::Rgb32FImage::from_fn(w, h, |x, y| {
                    let sun = if (x, y) == (40, 9) { 400. } else { 0. };
                    image::Rgb([0.3 + sun, 0.35 + y as f32 * 0.01, 0.5])
                });
                let path = std::env::temp_dir().join("caustica-self-test-sky.hdr");
                image::DynamicImage::ImageRgb32F(map).save(&path)?;
                self.settings.hdri = path.to_string_lossy().into();
                self.settings.hdri_rotation = 30.;
            }
            261 => {
                anyhow::ensure!(
                    self.gpu.as_ref().unwrap().environment_loaded(),
                    "HDRI not loaded"
                );
                self.settings.hdri_visible = false;
            }
            262 => self.settings.mis_mode = 1,
            263 => self.settings.mis_mode = 2,
            264 => {
                self.settings.mis_mode = 0;
                self.settings.hdri.clear();
                self.settings.hdri_visible = true;
                self.settings.hdri_rotation = 0.;
            }
            265 => anyhow::ensure!(
                !self.gpu.as_ref().unwrap().environment_loaded(),
                "HDRI still loaded"
            ),
            // Light outlines are off by default; switched on they draw (under the debug
            // layer) for the main light and every area light, then go off again.
            266 => {
                anyhow::ensure!(
                    !self.settings.light_outline && self.settings.lights.iter().all(|l| !l.outline),
                    "Light outlines should be off by default"
                );
                self.settings.lights.push(scene::AreaLight::default());
                self.settings.light_outline = true;
                self.settings
                    .lights
                    .iter_mut()
                    .for_each(|l| l.outline = true);
                self.ui.selection = ui::Selection::None;
                self.show_ui = true;
            }
            268 => {
                self.settings.light_outline = false;
                self.settings.lights.pop();
            }
            // Texture maps (core track 5): a masked, normal-mapped glTF quad uploads two
            // maps; switching its maps off releases them.
            269 => {
                let path = std::env::temp_dir().join("caustica-self-test-tiles.gltf");
                mesh::write_test_gltf(&path)?;
                let mut quad = mesh::import_objects(&path)?.remove(0);
                quad.position = [0., 0.4];
                self.settings.objects.push(quad);
            }
            270 => {
                let maps = self.gpu.as_ref().unwrap().map_count();
                anyhow::ensure!(maps == 2, "{maps} texture maps on the GPU, expected 2");
                self.settings.objects.last_mut().unwrap().material.textures = false;
            }
            271 => {
                let maps = self.gpu.as_ref().unwrap().map_count();
                anyhow::ensure!(maps == 0, "{maps} texture maps left after switching off");
                self.settings.objects.pop();
            }
            // The Assets window, offline: a two-entry catalog with the first selected. It
            // stays open (drawn every frame) while the scene files load.
            272 => {
                let (mut duck, _) = assets::from_spec("khronos/model/Duck")?;
                duck.tags = vec!["test".into()];
                let (mut other, _) = assets::from_spec("khronos/model/NotDownloaded")?;
                other.name = "Not downloaded".into();
                self.ui.assets.offline = true;
                self.ui
                    .assets
                    .set_catalog(assets::Source::Khronos, vec![duck, other]);
                self.ui.assets.open = true;
                self.show_ui = true;
            }
            // Video export through the dialog: three EXR frames at their own (odd) size while
            // the window keeps its own, then a long PNG export cancelled while it runs.
            273 => {
                let folder = std::env::temp_dir().join("caustica-self-test-video");
                let _ = std::fs::remove_dir_all(&folder);
                self.open_video_dialog();
                let o = &mut self.ui.video;
                (o.format, o.width, o.height, o.frames, o.spp) =
                    (video::Format::Exr, 201, 121, 3, 4);
                o.path = folder.with_extension("exr");
                self.actions.export_video = true;
            }
            276 => {
                anyhow::ensure!(self.export.is_none(), "The EXR export did not finish");
                let folder = std::env::temp_dir().join("caustica-self-test-video");
                for i in 1..=3 {
                    let path = folder.join(format!("{i:05}.exr"));
                    let size = image::image_dimensions(&path)
                        .with_context(|| format!("{}", path.display()))?;
                    anyhow::ensure!(size == (201, 121), "EXR frame size {size:?}");
                }
                let g = self.gpu.as_ref().unwrap();
                anyhow::ensure!(
                    g.swap_size == [g.width, g.height],
                    "The output did not return to the window size"
                );
                let o = &mut self.ui.video;
                (o.format, o.width, o.height, o.frames, o.spp) =
                    (video::Format::Png, 640, 360, 100, 4096);
                o.path = folder.with_extension("");
                self.actions.export_video = true;
            }
            277 => {
                let g = self.gpu.as_ref().unwrap();
                anyhow::ensure!(
                    self.export.is_some() && [g.width, g.height] == [640, 360],
                    "The PNG export is not running at its size"
                );
                self.actions.cancel_export = true;
            }
            278 => {
                let g = self.gpu.as_ref().unwrap();
                anyhow::ensure!(
                    self.export.is_none() && g.swap_size == [g.width, g.height],
                    "Cancelling did not end the export"
                );
            }
            // Viewport commands (UI phase 2): duplicate, hide, show all, delete, the camera
            // reset and scene stepping; the menu, outlines and shortcut sheet get drawn.
            279 => {
                let n = self.settings.objects.len();
                anyhow::ensure!(n > 0, "No object to duplicate");
                self.ui.selection = ui::Selection::Object(0);
                self.run_command(ui::Command::Duplicate);
                anyhow::ensure!(
                    self.settings.objects.len() == n + 1
                        && self.ui.selection == ui::Selection::Object(n),
                    "Duplicate did not add and select a copy"
                );
                self.run_command(ui::Command::Hide);
                anyhow::ensure!(!self.settings.objects[n].visible, "Hide did not hide");
                self.run_command(ui::Command::ShowAll);
                anyhow::ensure!(
                    self.settings.objects.iter().all(|o| o.visible),
                    "Show all left objects hidden"
                );
                self.run_command(ui::Command::Delete);
                anyhow::ensure!(self.settings.objects.len() == n, "Delete did not delete");
                self.ui.selection = ui::Selection::Object(0);
                self.ui.hovered = Some(n.min(1).min(n - 1));
                self.ui.context_menu = Some(egui::pos2(200., 200.));
                self.ui.show_shortcuts = true;
                self.settings.yaw += 30.;
                self.reset_camera();
                anyhow::ensure!(
                    self.settings.yaw == self.reset_baseline.yaw,
                    "Home did not reset the camera"
                );
            }
            // The command palette (Ctrl+K): open with a query, drawn once, a setting entry
            // goes to its tab and selects what shows it; 242 closes it.
            241 => {
                use crate::palette::{Act, Focus};
                self.ui.palette.open = true;
                self.ui.palette.query = "script".into();
                self.ui.tab = ui::Tab::Scene;
                self.run_palette(Act::Go(ui::Tab::Look, Focus::Stage, "Sky and light"));
                anyhow::ensure!(
                    self.ui.tab == ui::Tab::Look && self.ui.selection == ui::Selection::Stage,
                    "A palette setting did not open its tab"
                );
                anyhow::ensure!(
                    !crate::palette::matches("script").is_empty(),
                    "The palette found no Script console"
                );
            }
            242 => {
                self.ui.palette.open = false;
                self.ui.tab = ui::Tab::Scene;
            }
            280 => {
                let name = self.settings.scene_name.clone();
                self.step_scene(1);
                self.step_scene(-1);
                anyhow::ensure!(
                    self.settings.scene_name == name,
                    "Scene stepping did not come back: {name}, then {}",
                    self.settings.scene_name
                );
                self.ui.show_shortcuts = false;
                self.ui.context_menu = None;
            }
            // UI phases 3–4: a scene saved to a file outside the list and opened again
            // (title kept), the favourite and recent lists drawn, quality bundles, the A/B
            // split handle and a Show in folder toast.
            281 => {
                let path = std::env::temp_dir().join("caustica-self-test-scene.json");
                let mut s = self.settings.clone();
                s.exposure = 1.7;
                scenes::save_to(&path, &s, "Self test scene", "")?;
                self.open_scene(&path);
                anyhow::ensure!(
                    self.settings.scene_name == "Self test scene" && self.settings.exposure == 1.7,
                    "Opening a saved scene failed: {}",
                    self.settings.scene_name
                );
                anyhow::ensure!(
                    self.ui.prefs.recency(&path) == Some(0),
                    "The opened scene is not first in the recent list"
                );
                self.ui.show_scenes = true;
                self.ui.set_scene_list(2);
                self.ui.toast_file("Self-test file toast", path);
                self.settings.compare = true;
                ui::Bundle::Native.apply(&mut self.settings, scene::Upscaler::Fsr);
                anyhow::ensure!(
                    ui::Bundle::of(&self.settings) == Some(ui::Bundle::Native),
                    "Quality bundle did not round trip"
                );
            }
            282 => {
                self.ui.set_scene_list(1);
                self.settings.compare = false;
                ui::Bundle::Best.apply(&mut self.settings, scene::Upscaler::Fsr);
            }
            // Track 7: one light of every kind (the per-frame check covers their triangles
            // and emitter entries), the spot's inspector and every outline drawn; then the
            // sun alone, with the room open so it reaches the floor.
            // ReSTIR DI (track 15) runs from here with two layers, over every light kind.
            283 => {
                self.settings.restir = true;
                self.settings.spp = 2;
                self.ui.tab = ui::Tab::Scene;
                for (n, kind) in scene::LightKind::ALL.into_iter().enumerate() {
                    let mut l = editor::new_light(kind, n);
                    l.outline = true;
                    self.settings.lights.push(l);
                }
                self.ui.selection = ui::Selection::Light(self.settings.lights.len() - 2);
            }
            284 => {
                let g = self.gpu.as_ref().unwrap();
                anyhow::ensure!(
                    g.sun_count == 1,
                    "The sun is not in the emitter table ({} suns)",
                    g.sun_count
                );
                self.settings
                    .lights
                    .retain(|l| l.kind == scene::LightKind::Sun);
                self.settings.cornell_room = false;
                self.ui.selection = ui::Selection::Light(0);
            }
            // Track 15: BDPT, photon mapping and battle mode on the glass and mirror box.
            285 => {
                self.settings.preset(3);
                self.settings.integrator = 1;
            }
            // Spectral rendering from here to 289: spectral photons through dispersive glass,
            // then battle mode (the spectral path tracer and its SER twin) and the spectral lens.
            286 => {
                self.settings.integrator = 2;
                self.settings.spectral = true;
                for o in &mut self.settings.objects {
                    if o.material.transmission > 0. {
                        o.material.dispersion = 20.;
                    }
                }
            }
            287 => {
                self.settings.integrator = 0;
                self.settings.battle = true;
            }
            288 => {
                self.settings.battle = false;
                self.settings.restir = false;
            }
            // Track 16 lens systems: a stopped-down Double Gauss with blades, dispersion and
            // glare, then the fisheye (both checked after the render), then back to the thin lens.
            289 => {
                self.settings.lens_system = 1;
                self.settings.f_number = 4.;
                self.settings.aperture_blades = 6;
                self.settings.dispersion = 1.;
                self.settings.glare = 1.;
            }
            290 => {
                self.settings.lens_system = 3;
                self.settings.spectral = false;
            }
            291 => {
                self.settings.lens_system = 0;
                self.settings.f_number = 0.;
                self.settings.aperture_blades = 0;
                self.settings.dispersion = 0.;
                self.settings.glare = 0.;
            }
            // A user texture (as a scene thumbnail) drawn in the open Scenes window, then the
            // scene files below load with the window still open.
            203 => {
                let image =
                    egui::ColorImage::filled([64, 40], egui::Color32::from_rgb(40, 120, 200));
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
        Ok(())
    }
    /// Checks after rendering frame `self.rendered` with the frame settings `s`.
    pub(crate) fn self_test_checks(&mut self, s: &Settings) -> Result<()> {
        let g = self.gpu.as_mut().unwrap();
        // Every edit must reach the GPU: geometry and light tables match the settings.
        let vertices = scene::geometry(s);
        let (emitters, _) =
            lighting::emitter_table(&vertices, &lighting::materials(s), &lighting::analytic(s));
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
        let suns = |s: &Settings| {
            s.lights
                .iter()
                .filter(|l| l.enabled && l.kind == scene::LightKind::Sun)
                .count() as u32
        };
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
            126 => anyhow::ensure!(
                g.environment_loaded() && g.sun_count == 1 + suns(s),
                "The physical sky's map or sun is missing"
            ),
            127 => anyhow::ensure!(
                g.environment_loaded() && g.sun_count == suns(s) && g.stars_loaded(),
                "The night sky must load without a sun, with its stars"
            ),
            128 => anyhow::ensure!(!g.environment_loaded(), "The sky stayed loaded"),
            // Walking (frames 247-251) draws the avatar into the frame's copy, the head hidden.
            249 => anyhow::ensure!(
                s.objects
                    .iter()
                    .filter(|o| crate::avatar::is_part(o))
                    .count()
                    == crate::avatar::PARTS
                    && s.objects.iter().filter(|o| o.camera_hidden).count() == 1,
                "The walk avatar is missing from the frame"
            ),
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
            // RGBA16F keeps 11 significant bits.
            187 | 227 if g.hdr_refused => {
                println!(
                    "SELF-TEST NOTE: the swapchain offers no HDR format here; HDR checks skipped"
                )
            }
            187 => {
                anyhow::ensure!(g.hdr_active, "Forced HDR output did not start");
                check_hdr_output(g, s, 2e-3)?;
            }
            227 => {
                anyhow::ensure!(g.hdr_active, "Forced HDR10 output did not start");
                if g.hdr10_refused {
                    println!("SELF-TEST NOTE: the swapchain refused HDR10; checked scRGB instead");
                } else {
                    anyhow::ensure!(g.hdr10, "The HDR10 swapchain did not start");
                }
                // 10-bit PQ: a code step is under 1% of the value above 1 nit.
                check_hdr_output(g, s, 2e-2)?;
            }
            229 => anyhow::ensure!(!g.hdr10, "HDR10 stayed on after Auto"),
            // Every reconstruction backend is either usable or says why not.
            231 => {
                let caps = g.capabilities();
                for (name, b) in [
                    ("RR", &caps.rr),
                    ("DLSS SR", &caps.dlss_sr),
                    ("FSR", &caps.fsr),
                    ("XeSS", &caps.xess),
                ] {
                    anyhow::ensure!(
                        b.available == b.reason.is_empty(),
                        "{name}: available {} with reason {:?}",
                        b.available,
                        b.reason
                    );
                }
            }
            188 => anyhow::ensure!(!g.hdr_active, "SDR output did not return"),
            285..=287 | 294 | 295 | 298 | 301 => {
                let image = unsafe { g.read_floats(18)? };
                let pixels = image.as_chunks::<4>().0;
                let mean = pixels
                    .iter()
                    .map(|p| (p[0] + p[1] + p[2]) as f64 / 3.)
                    .sum::<f64>()
                    / pixels.len().max(1) as f64;
                anyhow::ensure!(
                    mean.is_finite() && mean > 0.01,
                    "Integrator {} rendered a mean of {mean}",
                    s.integrator
                );
                anyhow::ensure!(
                    !s.battle || g.battle.iter().all(|q| q.samples > 0 && q.ms > 0.),
                    "A battle quadrant did not render"
                );
            }

            289 | 290 => {
                let image = unsafe { g.read_floats(18)? };
                let pixels = image.as_chunks::<4>().0;
                let mean = pixels
                    .iter()
                    .map(|p| (p[0] + p[1] + p[2]) as f64 / 3.)
                    .sum::<f64>()
                    / pixels.len().max(1) as f64;
                anyhow::ensure!(
                    mean.is_finite() && mean > 0.002 && !g.rr_active,
                    "Lens {} rendered a mean of {mean} (RR {})",
                    s.lens_system,
                    g.rr_active
                );
            }
            283 | 284 => anyhow::ensure!(
                g.restir_layers() == 2,
                "ReSTIR did not run with two layers ({})",
                g.restir_layers()
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
            // The image uploaded at the end of 304 is the one displayed, and saved, in 305; the
            // GPU has stopped after its pass.
            305 => {
                let live = self.cpu_reference.as_ref().context("No software view")?;
                let image = live.image(true).context("The software view has no image")?;
                anyhow::ensure!(
                    live.view && live.state().samples == 4,
                    "The software view did not finish"
                );
                anyhow::ensure!(
                    g.samples <= s.spp,
                    "The GPU went on in the software view: {} spp",
                    g.samples
                );
                self.cpu_view_image = Some(image.clone());
                let linear = unsafe { g.read_floats(18)? };
                anyhow::ensure!(linear == *image, "The linear output is not the CPU's image");
                // The save buttons' EXR is that image too (the file may hold halves).
                let path = video::ImageFormat::Exr.file("caustica-cpu-view");
                unsafe { video::save_current(g, video::ImageFormat::Exr, 92, &path)? };
                let saved = image::open(&path)?.to_rgba32f();
                let _ = std::fs::remove_file(&path);
                anyhow::ensure!(
                    saved.len() == image.len()
                        && saved
                            .iter()
                            .zip(image.iter())
                            .all(|(a, b)| (a - b).abs() <= 1e-3 * b.abs().max(1.)),
                    "The saved EXR is not the CPU's image"
                );
            }
            307 => {
                let live = self.cpu_reference.as_ref().context("No software view")?;
                let image = live.image(true).context("The software view has no image")?;
                anyhow::ensure!(
                    live.state().samples == 4
                        && self.cpu_view_image.as_ref().is_some_and(|i| **i != *image),
                    "The software view did not restart on the camera move"
                );
                self.cpu_view_image = None;
                self.cpu_view = false;
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
            230 => {
                let paths = &self.ui.paths.paths;
                anyhow::ensure!(
                    paths.len() == 8 && self.ui.paths.pixel.is_some(),
                    "Path visualizer recorded {} paths",
                    paths.len()
                );
                for (i, p) in paths.iter().enumerate() {
                    anyhow::ensure!(
                        p.end.is_some()
                            && p.vertices.first().map(|v| v.kind) == Some(paths::Kind::Camera)
                            && (p.vertices.len() >= 2 || p.end == Some(5))
                            && p.vertices.iter().all(|v| v.position.is_finite())
                            && p.radiance.is_finite(),
                        "Path {i} is malformed: {p:?}"
                    );
                }
                self.ui.paths = Default::default();
            }
            236 => {
                anyhow::ensure!(g.lpe_slots == 4, "LPE slots: {}", g.lpe_slots);
                let beauty = unsafe { g.read_floats(18)? };
                let mut sum = 0.;
                for k in 0..4 {
                    let aov = unsafe { g.read_lpe(k)? };
                    anyhow::ensure!(
                        aov.len() / 3 == beauty.len() / 4,
                        "LPE AOV size {} vs {}",
                        aov.len() / 3,
                        beauty.len() / 4
                    );
                    sum += aov.iter().map(|v| *v as f64).sum::<f64>();
                }
                let b: f64 = beauty
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| (p[0] + p[1] + p[2]) as f64)
                    .sum();
                // Equal in expectation (labels are drawn per path), so a loose bound.
                anyhow::ensure!(
                    sum.is_finite() && (sum - b).abs() <= 0.03 * b.abs() + 1e-3,
                    "LPE partition sums to {sum}, beauty {b}"
                );
                self.settings.lpe.clear();
                self.settings.denoiser = crate::scene::Denoiser::Rr;
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
                !g.upscaler_available(scene::Upscaler::Dlss) || g.upscale_status.contains("active"),
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
        Ok(())
    }
}
