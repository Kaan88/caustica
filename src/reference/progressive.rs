//! Renders that grow by passes: the CLI's and the viewport's.
use super::aov::{GuideAov, Lpe, PixelAov, Record};
use super::camera::filter_offset;
use super::denoise::{Guide, Moments, luminance};
use super::integrators::bdpt::Splat;
use super::integrators::photon::PhotonMap;
use super::sampling::Rng;
use super::scene::Scene;
use super::unsupported;
use crate::scene::Settings;
use anyhow::Result;
use glam::{DVec3, Vec2, Vec3};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Renders `spp` samples per pixel on every core: linear RGBA, rows top first.
pub fn render(
    s: &Settings,
    size: [u32; 2],
    spp: u32,
    progress: &(dyn Fn(f32) + Sync),
) -> Result<Vec<f32>> {
    let mut p = Progressive::new(s, size)?;
    p.pass(spp, cores(), &AtomicBool::new(false), progress);
    Ok(p.image())
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// Runs `work(y, row)` for each row of `data` on `threads` threads until `cancel`.
pub(super) fn for_rows<T: Send>(
    data: &mut [T],
    width: usize,
    threads: usize,
    background: bool,
    cancel: &AtomicBool,
    work: &(dyn Fn(usize, &mut [T]) + Sync),
) {
    let next_row = AtomicUsize::new(0);
    let rows: Vec<Mutex<&mut [T]>> = data.chunks_mut(width).map(Mutex::new).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                if background {
                    crate::platform::background_thread();
                }
                loop {
                    let y = next_row.fetch_add(1, Ordering::Relaxed);
                    if y >= rows.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    work(y, &mut lock(&rows[y]));
                }
            });
        }
    });
}

/// The integrators and the pixels (x0, y0, x1, y1) they render: battle mode's four quadrants
/// (path tracing twice, the GPU's second one with ReSTIR, which only moves noise; then BDPT and
/// photon mapping), split at the integer halves of the render size as on the GPU
/// (`passes::battle_frame`), or the whole image with `Settings::integrator`.
pub(super) fn quadrants(s: &Settings, [w, h]: [u32; 2]) -> Vec<(u32, [u32; 4])> {
    if !s.battle {
        return vec![(s.integrator, [0, 0, w, h])];
    }
    let (hw, hh) = (w / 2, h / 2);
    vec![
        (0, [0, 0, hw, hh]),
        (0, [hw, 0, w, hh]),
        (1, [0, hh, hw, h]),
        (2, [hw, hh, w, h]),
    ]
}

/// A render that grows by passes: per-pixel sums of samples `0..samples`, each seeded by pixel
/// and sample index, so any split into passes gives the same bits as one pass. The sums are f64:
/// past tens of thousands of samples an f32 sum drops the low bits of what it adds.
pub struct Progressive<'a> {
    /// The scene, or with motion blur its moments inside the open shutter (`exposure`): each
    /// sample draws one, so the mean is the shutter's average.
    scenes: Vec<Scene<'a>>,
    pub(super) sums: Vec<DVec3>,
    pub samples: u32,
    /// The denoiser's inputs (`denoise`), empty until `keep_guides` asks for them: per pixel the
    /// guide ray's hit and what the samples add besides their light.
    guides: Vec<Guide>,
    moments: Vec<Moments>,
    /// The AOVs (`aov`), empty until `keep_aovs` asks for them: per pixel the samples' sums and
    /// the guide ray's layers; and the light path expressions they run.
    aovs: Vec<PixelAov>,
    aov_guides: Vec<GuideAov>,
    lpe: Option<Lpe>,
    /// Its threads run at low priority: a render behind a window.
    background: bool,
    /// Photon mapping's map of the frame the last samples belong to, one per moment.
    photon_maps: Vec<PhotonMap>,
}
impl<'a> Progressive<'a> {
    pub fn new(s: &'a Settings, size: [u32; 2]) -> Result<Self> {
        Self::over(std::slice::from_ref(s), size)
    }
    /// The average of the scenes `moments` (one or more, of the same camera): a motion-blurred
    /// frame. Every moment is checked, since a key can switch a feature on.
    pub fn over(moments: &'a [Settings], size: [u32; 2]) -> Result<Self> {
        anyhow::ensure!(!moments.is_empty(), "No scene to render");
        let mut missing: Vec<String> = vec![];
        for m in moments.iter().flat_map(unsupported) {
            if !missing.contains(&m) {
                missing.push(m);
            }
        }
        anyhow::ensure!(
            missing.is_empty(),
            "The CPU reference can't render this scene: {}",
            missing.join(", ")
        );
        let mut p = Self::partial(&moments[0], size);
        p.scenes
            .extend(moments[1..].iter().map(|s| Scene::new(s, size)));
        for scene in &p.scenes {
            let s = scene.s;
            anyhow::ensure!(
                scene.env.is_some() || !s.physical_sky.on && s.hdri.is_empty(),
                "The CPU reference couldn't load the environment"
            );
        }
        Ok(p)
    }
    /// The first (or only) moment: what the camera, the size and the preview read.
    fn scene(&self) -> &Scene<'a> {
        &self.scenes[0]
    }
    /// A render of what the renderer knows of the scene, whatever `unsupported` lists: for looking
    /// at, with that list on screen, never for measuring.
    pub(super) fn partial(s: &'a Settings, size: [u32; 2]) -> Self {
        Progressive {
            scenes: vec![Scene::new(s, size)],
            sums: vec![DVec3::ZERO; size[0] as usize * size[1] as usize],
            samples: 0,
            guides: vec![],
            moments: vec![],
            aovs: vec![],
            aov_guides: vec![],
            lpe: None,
            background: false,
            photon_maps: vec![],
        }
    }
    /// Records the AOVs from here on (`layers`), starting the render over; the sums' bits don't
    /// change, since the records draw from their own random streams.
    pub fn keep_aovs(&mut self) {
        let [w, h] = self.scene().size.map(|v| v as usize);
        let scene = self.scene();
        let guides = (0..w * h).map(|i| scene.aov_guide(i % w, i / w)).collect();
        let lpe = Lpe::new(scene.s);
        self.aov_guides = guides;
        self.lpe = lpe;
        self.aovs = vec![PixelAov::default(); w * h];
        self.sums.iter_mut().for_each(|c| *c = DVec3::ZERO);
        self.samples = 0;
    }
    /// Gathers the denoiser's inputs from here on. It starts the render over (the moments must
    /// cover every sample); the sums' bits don't change, since the guides draw no random numbers.
    pub fn keep_guides(&mut self) {
        let [w, h] = self.scene().size.map(|v| v as usize);
        let scene = self.scene();
        self.guides = (0..w * h).map(|i| scene.guide(i % w, i / w)).collect();
        self.moments = vec![Moments::default(); w * h];
        self.sums.iter_mut().for_each(|c| *c = DVec3::ZERO);
        self.samples = 0;
    }
    /// The mean through the denoiser (RGBA, rows top first), or the plain mean without guides.
    /// Gathering the denoiser's inputs.
    pub fn denoising(&self) -> bool {
        !self.guides.is_empty()
    }
    /// Stops gathering the denoiser's inputs; the sums go on.
    pub fn drop_guides(&mut self) {
        self.guides = vec![];
        self.moments = vec![];
    }
    pub fn denoised(&self, strength: f32, threads: usize) -> Vec<f32> {
        if self.guides.is_empty() {
            return self.image();
        }
        let n = self.samples.max(1) as f64;
        let mean: Vec<Vec3> = self.sums.iter().map(|c| (*c / n).as_vec3()).collect();
        let size = self.scene().size.map(|v| v as usize);
        let out = super::denoise::filter(
            &mean,
            &self.moments,
            &self.guides,
            self.samples,
            size,
            strength,
            threads,
        );
        out.iter().flat_map(|c| [c.x, c.y, c.z, 1.]).collect()
    }
    /// Starts over from another viewpoint of the same scene, keeping the BVH and the light tables.
    pub fn move_camera(&mut self, camera: &Settings, size: [u32; 2]) {
        self.aim(camera, size);
        self.sums.clear();
        self.sums
            .resize(size[0] as usize * size[1] as usize, DVec3::ZERO);
        self.samples = 0;
        if !self.guides.is_empty() {
            self.keep_guides();
        }
        if !self.aovs.is_empty() {
            self.keep_aovs();
        }
    }
    /// Moves the camera and keeps the sums: light painting's long exposure, which a camera move
    /// smears as it would a photograph's.
    pub(super) fn aim(&mut self, camera: &Settings, size: [u32; 2]) {
        for scene in &mut self.scenes {
            scene.set_camera(camera, size);
        }
    }
    /// Adds `spp` samples to every pixel on `threads` threads. Returns false when `cancel` stopped
    /// it partway; the sums are then mixed and the render should be dropped.
    pub fn pass(
        &mut self,
        spp: u32,
        threads: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(f32) + Sync),
    ) -> bool {
        // Photon mapping's frames are `Settings::spp` samples each, with a map of their own
        // (`integrators::photon`): a pass runs frame by frame, so its split doesn't matter.
        // Battle mode renders its quadrants one after another, each with its own integrator.
        let s = self.scenes[0].s;
        let quadrants = quadrants(s, self.scenes[0].size);
        let photons = quadrants.iter().any(|q| q.0 == 2);
        let (end, per) = (self.samples + spp, s.spp.max(1));
        while self.samples < end {
            let frame = self.samples / per;
            let n = if photons {
                ((frame + 1) * per).min(end) - self.samples
            } else {
                end - self.samples
            };
            for (i, &(integrator, region)) in quadrants.iter().enumerate() {
                for scene in &mut self.scenes {
                    scene.set_quadrant(integrator, region);
                }
                if integrator == 2 && self.photon_maps.first().is_none_or(|m| m.frame != frame) {
                    self.photon_maps.clear();
                    for scene in &self.scenes {
                        let Some(map) = scene.photon_map(frame, threads, cancel) else {
                            self.photon_maps.clear();
                            return false;
                        };
                        self.photon_maps.push(map);
                    }
                }
                let k = quadrants.len() as f32;
                let progress = |f: f32| progress((i as f32 + f) / k);
                if !self.samples_pass(n, region, threads, cancel, &progress) {
                    return false;
                }
            }
            self.samples += n;
        }
        if s.battle {
            let [w, h] = self.scenes[0].size;
            for scene in &mut self.scenes {
                scene.set_quadrant(s.integrator, [0, 0, w, h]);
            }
        }
        true
    }
    /// `pass` within one photon-mapping frame, over the pixels of `region` with the scenes'
    /// integrator (`Scene::set_quadrant`). The caller counts the samples.
    fn samples_pass(
        &mut self,
        spp: u32,
        region: [u32; 4],
        threads: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(f32) + Sync),
    ) -> bool {
        let scenes = &self.scenes;
        let photon_maps = &self.photon_maps;
        let s = scenes[0].s;
        let (w, h) = (scenes[0].size[0] as usize, scenes[0].size[1] as usize);
        let first = self.samples;
        let done = AtomicUsize::new(0);
        let guides = &self.guides;
        let aov_guides = &self.aov_guides;
        let lpe = self.lpe.as_ref();
        // Each pixel's sum, with its moments when the denoiser wants them and its AOVs.
        let mut moments = self.moments.iter_mut();
        let mut aovs = self.aovs.iter_mut();
        type Cell<'c> = (
            &'c mut DVec3,
            Option<&'c mut Moments>,
            Option<&'c mut PixelAov>,
        );
        let mut cells: Vec<Cell> = self
            .sums
            .iter_mut()
            .map(|c| (c, moments.next(), aovs.next()))
            .collect();
        // BDPT's light-tracing terms land in any pixel: each row keeps its own, added after the
        // pass in row order, so the sums don't depend on the threads.
        let integrator = scenes[0].integrator;
        let bdpt = integrator == 1;
        let [x0, y0, x1, y1] = region.map(|v| v as usize);
        let splats: Vec<Mutex<Vec<Splat>>> = (0..if bdpt { h } else { 0 })
            .map(|_| Mutex::new(vec![]))
            .collect();
        for_rows(
            &mut cells,
            w,
            threads,
            self.background,
            cancel,
            &|y, row| {
                let mut row_splats = vec![];
                let row = if (y0..y1).contains(&y) {
                    &mut row[x0..x1]
                } else {
                    &mut row[..0]
                };
                for (x, (sum, moments, aov)) in row.iter_mut().enumerate() {
                    let x = x + x0;
                    let mut rec = match aov {
                        Some(_) => Record::new(true, lpe, aov_guides[y * w + x].material),
                        None => Record::off(),
                    };
                    for k in first..first + spp {
                        let mut r = Rng::new((y * w + x) as u64, k as u64);
                        // Drawn only when there are moments to choose from, so a still keeps
                        // its sample sequences.
                        let moment = match scenes.len() {
                            1 => 0,
                            m => ((r.f() * m as f32) as usize).min(m - 1),
                        };
                        let scene = &scenes[moment];
                        let off = filter_offset(s, Vec2::new(r.f(), r.f()));
                        let pixel = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) + off;
                        let mut waves = scene.waves(&r);
                        let ray = scene.camera_ray(pixel, &mut waves, &mut r);
                        rec.start();
                        // A sample the lens cuts off counts, as black.
                        let c = ray.map_or(Vec3::ZERO, |(o, d, w)| {
                            if bdpt {
                                let c = scene.bdpt(o, d, w, k, &mut waves, &mut r, &mut row_splats);
                                // Its AOVs: the whole estimate as direct light, no specular.
                                rec.direct = c;
                                c
                            } else if let Some(map) =
                                photon_maps.get(moment).filter(|_| integrator == 2)
                            {
                                let c = scene.photon_gather(o, d, w, map, &mut waves, &mut r);
                                rec.direct = c;
                                c
                            } else {
                                scene.radiance(o, d, w, &mut waves, &mut r, &mut rec)
                            }
                        });
                        // The firefly clamp is the path tracer's, as on the GPU.
                        let scale = if integrator != 0 {
                            1.
                        } else {
                            firefly_scale(s.firefly, c, **sum, k)
                        };
                        let c = c * scale;
                        **sum += c.as_dvec3();
                        if let Some(a) = aov {
                            a.add(&rec, c, scale);
                        }
                        if let Some(m) = moments {
                            if let Some((o, d, _)) = ray {
                                let (albedo, id) = scene.first_albedo(o, d);
                                m.albedo += albedo;
                                m.covered += (id == guides[y * w + x].id) as u32;
                            }
                            m.m2 += (luminance(c) as f64).powi(2);
                        }
                    }
                    if let Some(a) = aov {
                        a.take_lpe(&rec);
                    }
                }
                if bdpt {
                    *lock(&splats[y]) = row_splats;
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of((h / 20).max(1)) {
                    progress(n as f32 / h as f32);
                }
            },
        );
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        for row in splats {
            for (i, c) in row.into_inner().unwrap_or_else(|e| e.into_inner()) {
                self.sums[i as usize] += c.as_dvec3();
            }
        }
        true
    }
    /// The mean so far: linear RGBA, rows top first.
    pub fn image(&self) -> Vec<f32> {
        let n = self.samples.max(1) as f64;
        self.sums
            .iter()
            .flat_map(|c| {
                let c = (*c / n).as_vec3();
                [c.x, c.y, c.z, 1.]
            })
            .collect()
    }
    /// Light path expression `slot`'s mean as RGBA (`keep_aovs`), or None for a slot not in use.
    pub fn lpe_image(&self, slot: u32) -> Option<Vec<f32>> {
        let used = self.lpe.as_ref().map_or(0, |l| l.slots);
        if self.aovs.is_empty() || slot as usize >= used {
            return None;
        }
        let n = self.samples.max(1) as f64;
        Some(
            self.aovs
                .iter()
                .flat_map(|a| {
                    let c = (a.lpe[slot as usize] / n).as_vec3();
                    [c.x, c.y, c.z, 1.]
                })
                .collect(),
        )
    }
    /// A multi-layer EXR's layers (`keep_aovs`), in the GPU's order (`render::aov_layers`, then
    /// each light path expression in use): `beauty` (RGBA), the albedos, the guide ray's normal,
    /// roughness, depth and motion (0: a still), the direct and specular light, the object ids.
    pub fn layers(&self, beauty: Vec<f32>) -> Vec<crate::render::AovLayer> {
        let [w, h] = self.scene().size;
        let n = self.samples.max(1) as f64;
        let mut layers = vec![];
        let mut add = |name: &str, channels: &[&'static str], data: Vec<f32>| {
            layers.push(crate::render::AovLayer {
                name: name.into(),
                width: w,
                height: h,
                channels: channels.to_vec(),
                data,
            })
        };
        let mean = |f: &dyn Fn(&PixelAov) -> DVec3| -> Vec<f32> {
            self.aovs
                .iter()
                .flat_map(|a| (f(a) / n).as_vec3().to_array())
                .collect()
        };
        const RGB: [&str; 3] = ["R", "G", "B"];
        let g = &self.aov_guides;
        add("beauty", &["R", "G", "B", "A"], beauty);
        add(
            "diffuse_albedo",
            &RGB,
            mean(&|a| a.diffuse_albedo.truncate()),
        );
        add("specular_albedo", &RGB, mean(&|a| a.specular_albedo));
        add(
            "normal",
            &["X", "Y", "Z"],
            g.iter().flat_map(|g| g.normal.to_array()).collect(),
        );
        add("roughness", &["Y"], g.iter().map(|g| g.roughness).collect());
        add("depth", &["Z"], g.iter().map(|g| g.depth).collect());
        add("motion", &["X", "Y"], vec![0.; g.len() * 2]);
        add("direct", &RGB, mean(&|a| a.direct));
        add("specular", &RGB, mean(&|a| a.specular));
        add(
            "object_id",
            &["Y"],
            g.iter()
                .map(|g| g.material.wrapping_add(1) as f32)
                .collect(),
        );
        let exprs = &self.scene().s.lpe;
        for slot in 0..self.lpe.as_ref().map_or(0, |l| l.slots) {
            let name = crate::render::lpe_layer_name(slot as u32, exprs.get(slot));
            add(&name, &RGB, mean(&|a| a.lpe[slot]));
        }
        layers
    }
    /// The sums so far as RGBA (alpha the sample count), for adding renders of different moments.
    pub fn total(&self) -> Vec<f32> {
        let n = self.samples as f32;
        self.sums
            .iter()
            .flat_map(|c| {
                let c = c.as_vec3();
                [c.x, c.y, c.z, n]
            })
            .collect()
    }
    /// A quick look while the camera moves: one sample through the middle of each `k` × `k` block,
    /// spread over the block, as an image of the full size. It doesn't touch the sums.
    pub(super) fn preview(
        &self,
        k: usize,
        threads: usize,
        cancel: &AtomicBool,
    ) -> Option<Vec<f32>> {
        let scene = self.scene();
        let [w, h] = scene.size.map(|v| v as usize);
        let cols = w.div_ceil(k);
        let mut blocks = vec![Vec3::ZERO; cols * h.div_ceil(k)];
        for_rows(
            &mut blocks,
            cols,
            threads,
            self.background,
            cancel,
            &|y, row| {
                for (x, c) in row.iter_mut().enumerate() {
                    let mut r = Rng::new((y * cols + x) as u64, 0);
                    // The last block can hang over the edge of the image.
                    let pixel = Vec2::new(
                        ((x as f32 + 0.5) * k as f32).min(w as f32 - 0.5),
                        ((y as f32 + 0.5) * k as f32).min(h as f32 - 0.5),
                    );
                    let mut waves = scene.waves(&r);
                    *c = scene
                        .camera_ray(pixel, &mut waves, &mut r)
                        .map_or(Vec3::ZERO, |(o, d, w)| {
                            scene.radiance(o, d, w, &mut waves, &mut r, &mut Record::off())
                        });
                }
            },
        );
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let mut image = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let c = blocks[y / k * cols + x / k];
                image.extend([c.x, c.y, c.z, 1.]);
            }
        }
        Some(image)
    }
}

/// A preview should take about this long (20 images a second with the camera moving).
const PREVIEW_SECONDS: f32 = 0.05;
/// The block side (`Progressive::preview`) for which a preview takes `PREVIEW_SECONDS`, given how
/// long one sample per pixel takes. 1 means the full passes are quick enough by themselves.
pub(super) fn preview_block(seconds_per_sample: f32) -> usize {
    ((seconds_per_sample / PREVIEW_SECONDS).sqrt().ceil() as usize).clamp(1, 32)
}
/// The same scene from another viewpoint: only what `Progressive::move_camera` takes differs. The
/// fields are those the app's camera-move test ignores (`redraw`).
pub(super) fn same_but_camera(a: &Settings, b: &Settings) -> bool {
    let mut c = b.clone();
    (c.yaw, c.pitch, c.distance, c.target, c.fov) = (a.yaw, a.pitch, a.distance, a.target, a.fov);
    c == *a
}

/// What the viewport wants rendered next.
struct Request {
    settings: Settings,
    size: [u32; 2],
    /// Light painting keeps the exposure through this edit (`crate::painting_keeps`).
    paint: bool,
}
/// A published image: linear RGBA, rows top first.
pub type Image = Arc<Vec<f32>>;
/// The CPU renderer running in the background for the viewport, in one of two roles: the full
/// view (`view`: renders what it can and says what it left out, previews while the camera moves),
/// or Look → Compare's reference (refuses what it can't render, and is measured against). It
/// renders in passes of about half a second and publishes the mean after each; `follow` restarts
/// it; dropping it cancels the render (the thread finishes its current row and exits on its own).
pub struct Live {
    /// The transport key and output size it renders; a change means `follow`.
    pub key: String,
    pub size: [u32; 2],
    pub view: bool,
    /// The scene's sample cap, which light painting lifts.
    cap: u32,
    shared: Arc<Shared>,
}
#[derive(Default)]
struct Shared {
    /// Ends the thread.
    stop: AtomicBool,
    /// Ends the current pass: a request is waiting. Set and cleared under `inner`'s lock.
    cancel: AtomicBool,
    paused: AtomicBool,
    /// Light painting (the app's L): edits of the lights and the camera keep the exposure, which
    /// goes on past the sample cap, as the GPU's does.
    painting: AtomicBool,
    /// The view's denoiser (`denoise`): on, and its strength's bits.
    denoise: AtomicBool,
    strength: std::sync::atomic::AtomicU32,
    inner: Mutex<Inner>,
    wake: Condvar,
}
#[derive(Default)]
struct Inner {
    request: Option<Request>,
    state: LiveState,
}
#[derive(Default, Clone)]
pub struct LiveState {
    /// The latest mean (denoised when the view's denoiser is on), and whether it has not been
    /// shown yet.
    pub image: Option<Arc<Vec<f32>>>,
    /// The plain mean behind a denoised `image`, for saving: what is saved as the render's light
    /// never goes through the denoiser.
    pub raw: Option<Arc<Vec<f32>>>,
    pub fresh: bool,
    /// The image is a preview: no full pass has finished since the last restart.
    pub preview: bool,
    pub samples: u32,
    pub cap: u32,
    pub seconds: f32,
    pub threads: usize,
    /// Images published since the start.
    pub images: u32,
    /// Why it can't render (unsupported features), shown instead of the image.
    pub error: Option<String>,
    /// The view: what the scene uses that the image leaves out (`unsupported`).
    pub missing: Vec<String>,
}
impl Live {
    pub fn start(s: Settings, size: [u32; 2], key: String, view: bool) -> Live {
        // Two cores stay free for the UI and the GPU's submissions, and the workers run at low
        // priority, so what's left of those two doesn't decide how the window feels.
        let threads = cores().saturating_sub(2).max(1);
        let shared = Arc::new(Shared::default());
        {
            let mut inner = lock(&shared.inner);
            inner.state.cap = s.max_samples.max(1);
            inner.state.preview = view;
            inner.state.threads = threads;
        }
        let job = shared.clone();
        let cap = s.max_samples.max(1);
        let first = Request {
            settings: s,
            size,
            paint: false,
        };
        let spawned = std::thread::Builder::new()
            .name("cpu-render".into())
            .spawn(move || {
                let run = std::panic::AssertUnwindSafe(|| job.run(first, view, threads));
                if std::panic::catch_unwind(run).is_err() {
                    job.refuse("The CPU render stopped on an internal error".into());
                }
            });
        if let Err(e) = spawned {
            shared.refuse(format!("Starting the CPU render: {e}"));
        }
        Live {
            key,
            size,
            view,
            cap,
            shared,
        }
    }
    /// Renders this instead, as soon as the running pass lets go. The newest request wins, so a
    /// drag can't queue work. The view keeps its last image until the next one lands; the
    /// reference has none until then.
    pub fn follow(&mut self, s: Settings, key: String, size: [u32; 2]) {
        let painting = self.shared.painting.load(Ordering::Relaxed);
        let paint = painting && size == self.size && crate::painting_keeps(&self.key, &key);
        self.key = key;
        self.size = size;
        self.cap = s.max_samples.max(1);
        let mut inner = lock(&self.shared.inner);
        let st = &mut inner.state;
        st.cap = if painting { u32::MAX } else { self.cap };
        st.error = None;
        if !paint {
            st.preview = self.view;
            st.samples = 0;
            st.seconds = 0.;
            if !self.view {
                st.image = None;
                st.fresh = false;
            }
        }
        inner.request = Some(Request {
            settings: s,
            size,
            paint,
        });
        // A painted exposure lets the running pass finish: a cancelled one leaves some rows with
        // a sample more than the rest.
        if !paint {
            self.shared.cancel.store(true, Ordering::Relaxed);
        }
        self.shared.wake.notify_all();
    }
    /// Light painting on or off; turning it on or off doesn't restart (the app restarts it).
    pub fn set_painting(&self, on: bool) {
        if self.shared.painting.swap(on, Ordering::Relaxed) != on {
            lock(&self.shared.inner).state.cap = if on { u32::MAX } else { self.cap };
            self.shared.wake.notify_all();
        }
    }
    /// Progress without the image.
    pub fn state(&self) -> LiveState {
        LiveState {
            image: None,
            ..lock(&self.shared.inner).state.clone()
        }
    }
    /// Still rendering (not finished, refused or failed).
    pub fn running(&self) -> bool {
        let inner = lock(&self.shared.inner);
        inner.state.error.is_none() && inner.state.samples < inner.state.cap
    }
    /// Rendering, or with an image the viewport hasn't taken yet.
    pub fn busy(&self) -> bool {
        let inner = lock(&self.shared.inner);
        let st = &inner.state;
        st.fresh || (st.error.is_none() && st.samples < st.cap)
    }
    /// The view's denoiser, off by default: turning it on starts the render over (its inputs
    /// must cover every sample); the strength only refilters.
    pub fn set_denoise(&self, on: bool, strength: f32) {
        self.shared
            .denoise
            .store(on && self.view, Ordering::Relaxed);
        self.shared
            .strength
            .store(strength.to_bits(), Ordering::Relaxed);
    }
    /// The plain mean behind the latest image when that is denoised: what a save writes.
    pub fn raw(&self) -> Option<Arc<Vec<f32>>> {
        let pixels = self.size[0] as usize * self.size[1] as usize;
        let inner = lock(&self.shared.inner);
        let st = &inner.state;
        st.raw.clone().filter(|i| i.len() == pixels * 4)
    }
    /// The latest image of a full pass as shown (denoised when the denoiser is on), its plain mean
    /// and its samples, whether or not the viewport has taken it: what a script's capture saves
    /// and what the convergence graph measures. None for a preview, a stale image or none yet.
    pub fn latest(&self) -> Option<(Image, Image, u32)> {
        let pixels = self.size[0] as usize * self.size[1] as usize;
        let inner = lock(&self.shared.inner);
        let st = &inner.state;
        if st.preview || st.samples == 0 || inner.request.is_some() {
            return None;
        }
        let image = st.image.clone().filter(|i| i.len() == pixels * 4)?;
        let plain = st.raw.clone().filter(|i| i.len() == pixels * 4);
        Some((image.clone(), plain.unwrap_or(image), st.samples))
    }
    /// Holds the render after its current pass (the app's pause); a preview still runs.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }
    /// The latest mean if it is new and of the output size, or with `again` whenever there is one
    /// (the reference texture was recreated by a resize, an export or an HDR switch).
    pub fn image(&self, again: bool) -> Option<Arc<Vec<f32>>> {
        let mut inner = lock(&self.shared.inner);
        let st = &mut inner.state;
        if !(st.fresh || again) {
            return None;
        }
        st.fresh = false;
        let pixels = self.size[0] as usize * self.size[1] as usize;
        st.image.clone().filter(|i| i.len() == pixels * 4)
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        // Under the lock, so the thread can't check and then sleep through the notification.
        let _inner = lock(&self.shared.inner);
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cancel.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}
impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// The waiting request, if any.
    fn take(&self) -> Option<Request> {
        let mut inner = lock(&self.inner);
        let request = inner.request.take();
        if request.is_some() && !self.stopped() {
            self.cancel.store(false, Ordering::Relaxed);
        }
        request
    }
    /// Sleeps until there may be something to do. The timeout is for `paused`, which no one
    /// notifies.
    fn wait(&self) {
        let inner = lock(&self.inner);
        if inner.request.is_none() && !self.stopped() {
            let _ = self.wake.wait_timeout(inner, Duration::from_millis(50));
        }
    }
    fn refuse(&self, why: String) {
        lock(&self.inner).state.error = Some(why);
    }
    /// A new image, unless a request has already made it stale. A preview always lands: with the
    /// camera moving there is always a request, and a frame old beats none.
    fn publish(
        &self,
        image: Vec<f32>,
        raw: Option<Vec<f32>>,
        preview: bool,
        samples: u32,
        seconds: f32,
    ) {
        let mut inner = lock(&self.inner);
        if inner.request.is_some() && !preview {
            return;
        }
        let st = &mut inner.state;
        st.images += 1;
        st.image = Some(Arc::new(image));
        st.raw = raw.map(Arc::new);
        st.fresh = true;
        st.preview = preview;
        st.samples = samples;
        st.seconds = seconds;
    }
    /// The thread: builds the scene of each request whose scene differs from the last.
    fn run(&self, first: Request, view: bool, threads: usize) {
        let mut next = first;
        // Light painting's exposure, carried from one scene to the next.
        let mut carry: Option<(Vec<DVec3>, u32, f32)> = None;
        loop {
            let (s, size) = (next.settings, next.size);
            let built = if view {
                let missing = unsupported(&s);
                lock(&self.inner).state.missing = missing;
                Ok(Progressive::partial(&s, size))
            } else {
                Progressive::new(&s, size)
            };
            let request = match built {
                Ok(mut p) => {
                    p.background = true;
                    let mut spent = 0.;
                    if let Some((sums, samples, seconds)) = carry.take()
                        && next.paint
                        && sums.len() == p.sums.len()
                    {
                        (p.sums, p.samples, spent) = (sums, samples, seconds);
                    }
                    let request = self.drive(&s, &mut p, view, threads, spent);
                    if let Some((r, seconds)) = &request
                        && r.paint
                    {
                        carry = Some((std::mem::take(&mut p.sums), p.samples, *seconds));
                    }
                    request.map(|(r, _)| r)
                }
                Err(e) => {
                    self.refuse(format!("{e:#}"));
                    loop {
                        if self.stopped() {
                            break None;
                        }
                        match self.take() {
                            Some(r) => break Some(r),
                            None => self.wait(),
                        }
                    }
                }
            };
            match request {
                Some(r) => next = r,
                None => return,
            }
        }
    }
    /// Renders one scene from whatever viewpoints arrive. Returns a request for another scene
    /// (with the seconds spent, which light painting carries on), or None when told to stop. A
    /// cancelled pass always has a request behind it, which starts the sums over, except under
    /// light painting, whose request keeps them: the cancelled pass never added to them.
    fn drive(
        &self,
        s: &Settings,
        p: &mut Progressive<'_>,
        view: bool,
        threads: usize,
        mut spent: f32,
    ) -> Option<(Request, f32)> {
        let mut block = 8;
        // A painted exposure has its image already; a preview would replace it.
        let mut previewed = !view || p.samples > 0;
        let mut spp = 1;
        // The denoiser setting the last image was made with.
        let mut shown = (false, 0);
        while !self.stopped() {
            let want = (
                self.denoise.load(Ordering::Relaxed),
                self.strength.load(Ordering::Relaxed),
            );
            if want.0 && !p.denoising() {
                p.keep_guides();
                spent = 0.;
            } else if !want.0 && p.denoising() {
                p.drop_guides();
            }
            if let Some(r) = self.take() {
                if !same_but_camera(s, &r.settings) {
                    return Some((r, spent));
                }
                if r.paint {
                    p.aim(&r.settings, r.size);
                } else {
                    p.move_camera(&r.settings, r.size);
                    previewed = !view;
                    spp = 1;
                    spent = 0.;
                }
            }
            let cap = if self.painting.load(Ordering::Relaxed) {
                u32::MAX
            } else {
                s.max_samples.max(1)
            };
            if !previewed {
                previewed = true;
                // Not cancelled by the next request: while the camera moves each frame brings one,
                // and this image is the whole point of the preview. At block 1 it costs a sample
                // per pixel, little beside a full pass that wouldn't finish between two frames.
                let t = Instant::now();
                if let Some(image) = p.preview(block, threads, &self.stop) {
                    self.publish(image, None, true, 0, 0.);
                }
                let blocks = (p.scene().size[0] as usize).div_ceil(block)
                    * (p.scene().size[1] as usize).div_ceil(block);
                let pixels = p.scene().size[0] as f32 * p.scene().size[1] as f32;
                block = preview_block(t.elapsed().as_secs_f32() * pixels / blocks as f32);
                continue;
            }
            if p.samples >= cap || (self.paused.load(Ordering::Relaxed) && p.samples > 0) {
                if shown != want && p.samples > 0 {
                    self.show(p, want, threads, spent);
                    shown = want;
                }
                self.wait();
                continue;
            }
            let t = Instant::now();
            let n = spp.min(cap - p.samples);
            if !p.pass(n, threads, &self.cancel, &|_| {}) {
                continue;
            }
            let per_sample = t.elapsed().as_secs_f32() / n as f32;
            spent += t.elapsed().as_secs_f32();
            spp = ((0.5 / per_sample.max(1e-6)) as u32).clamp(1, 64);
            block = preview_block(per_sample);
            self.show(p, want, threads, spent);
            shown = want;
        }
        None
    }
    /// Publishes the mean, through the denoiser when `want` says so (and its inputs are there).
    fn show(&self, p: &Progressive<'_>, want: (bool, u32), threads: usize, spent: f32) {
        if want.0 && p.denoising() {
            let image = p.denoised(f32::from_bits(want.1), threads);
            self.publish(image, Some(p.image()), false, p.samples, spent);
        } else {
            self.publish(p.image(), None, false, p.samples, spent);
        }
    }
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The firefly clamp (biased, off at 0): after four samples a pixel's sample may be at most
/// `limit` times the luminance of the mean so far (never below 0.05), scaled down to that.
/// The GPU compares against the mean at the start of its frame; here it is the mean of the
/// samples before this one, which is the same thing at one sample a frame.
#[cfg(test)]
fn firefly_clamp(limit: f32, c: Vec3, sum: DVec3, k: u32) -> Vec3 {
    c * firefly_scale(limit, c, sum, k)
}
/// The firefly clamp's factor for sample `k` of value `c`, the pixel's sum so far `sum`: 1, or
/// less for an outlier. The direct light AOV is scaled with the sample, as on the GPU.
fn firefly_scale(limit: f32, c: Vec3, sum: DVec3, k: u32) -> f32 {
    if limit <= 0. || k <= 4 {
        return 1.;
    }
    let lum = |v: Vec3| v.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let mean = (sum / k as f64).as_vec3();
    (limit * lum(mean).max(0.05) / lum(c).max(1e-6)).min(1.)
}

#[cfg(test)]
mod firefly_tests {
    use super::*;

    #[test]
    fn firefly_clamp_scales_only_outliers_after_four_samples() {
        let sum = DVec3::splat(5.); // mean 1 after 5 samples
        let hot = Vec3::splat(100.);
        assert_eq!(firefly_clamp(0., hot, sum, 5), hot);
        assert_eq!(firefly_clamp(10., hot, sum, 4), hot);
        assert!((firefly_clamp(10., hot, sum, 5) - Vec3::splat(10.)).length() < 1e-4);
        assert_eq!(firefly_clamp(10., Vec3::splat(2.), sum, 5), Vec3::splat(2.));
        // A dark mean still allows 0.05 x limit.
        let dark = firefly_clamp(10., hot, DVec3::ZERO, 5);
        assert!((dark - Vec3::splat(0.5)).length() < 1e-4);
    }
}
