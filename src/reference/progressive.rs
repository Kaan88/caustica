//! Renders that grow by passes: the CLI's and the viewport's.
use super::camera::filter_offset;
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
fn for_rows<T: Send>(
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

/// A render that grows by passes: per-pixel sums of samples `0..samples`, each seeded by pixel
/// and sample index, so any split into passes gives the same bits as one pass. The sums are f64:
/// past tens of thousands of samples an f32 sum drops the low bits of what it adds.
pub struct Progressive<'a> {
    scene: Scene<'a>,
    pub(super) sums: Vec<DVec3>,
    pub samples: u32,
    /// Its threads run at low priority: a render behind a window.
    background: bool,
}
impl<'a> Progressive<'a> {
    pub fn new(s: &'a Settings, size: [u32; 2]) -> Result<Self> {
        let missing = unsupported(s);
        anyhow::ensure!(
            missing.is_empty(),
            "The CPU reference can't render this scene: {}",
            missing.join(", ")
        );
        Ok(Self::partial(s, size))
    }
    /// A render of what the renderer knows of the scene, whatever `unsupported` lists: for looking
    /// at, with that list on screen, never for measuring.
    pub(super) fn partial(s: &'a Settings, size: [u32; 2]) -> Self {
        Progressive {
            scene: Scene::new(s, size),
            sums: vec![DVec3::ZERO; size[0] as usize * size[1] as usize],
            samples: 0,
            background: false,
        }
    }
    /// Starts over from another viewpoint of the same scene, keeping the BVH and the light tables.
    pub(super) fn move_camera(&mut self, camera: &Settings, size: [u32; 2]) {
        self.scene.set_camera(camera, size);
        self.sums.clear();
        self.sums
            .resize(size[0] as usize * size[1] as usize, DVec3::ZERO);
        self.samples = 0;
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
        let scene = &self.scene;
        let s = scene.s;
        let (w, h) = (scene.size[0] as usize, scene.size[1] as usize);
        let first = self.samples;
        let done = AtomicUsize::new(0);
        for_rows(
            &mut self.sums,
            w,
            threads,
            self.background,
            cancel,
            &|y, row| {
                for (x, sum) in row.iter_mut().enumerate() {
                    for k in first..first + spp {
                        let mut r = Rng::new((y * w + x) as u64, k as u64);
                        let off = filter_offset(s, Vec2::new(r.f(), r.f()));
                        let pixel = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) + off;
                        let d = scene.camera_direction(pixel);
                        *sum += scene.radiance(scene.eye, d, &mut r).as_dvec3();
                    }
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
        self.samples += spp;
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
    /// A quick look while the camera moves: one sample through the middle of each `k` × `k` block,
    /// spread over the block, as an image of the full size. It doesn't touch the sums.
    pub(super) fn preview(
        &self,
        k: usize,
        threads: usize,
        cancel: &AtomicBool,
    ) -> Option<Vec<f32>> {
        let scene = &self.scene;
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
                    let d = scene.camera_direction(pixel);
                    *c = scene.radiance(scene.eye, d, &mut r);
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
}
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
    shared: Arc<Shared>,
}
#[derive(Default)]
struct Shared {
    /// Ends the thread.
    stop: AtomicBool,
    /// Ends the current pass: a request is waiting. Set and cleared under `inner`'s lock.
    cancel: AtomicBool,
    paused: AtomicBool,
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
    /// The latest mean, and whether it has not been shown yet.
    pub image: Option<Arc<Vec<f32>>>,
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
        let first = Request { settings: s, size };
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
            shared,
        }
    }
    /// Renders this instead, as soon as the running pass lets go. The newest request wins, so a
    /// drag can't queue work. The view keeps its last image until the next one lands; the
    /// reference has none until then.
    pub fn follow(&mut self, s: Settings, key: String, size: [u32; 2]) {
        self.key = key;
        self.size = size;
        let mut inner = lock(&self.shared.inner);
        let st = &mut inner.state;
        st.cap = s.max_samples.max(1);
        st.preview = self.view;
        st.samples = 0;
        st.seconds = 0.;
        st.error = None;
        if !self.view {
            st.image = None;
            st.fresh = false;
        }
        inner.request = Some(Request { settings: s, size });
        self.shared.cancel.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
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
    fn publish(&self, image: Vec<f32>, preview: bool, samples: u32, seconds: f32) {
        let mut inner = lock(&self.inner);
        if inner.request.is_some() && !preview {
            return;
        }
        let st = &mut inner.state;
        st.images += 1;
        st.image = Some(Arc::new(image));
        st.fresh = true;
        st.preview = preview;
        st.samples = samples;
        st.seconds = seconds;
    }
    /// The thread: builds the scene of each request whose scene differs from the last.
    fn run(&self, first: Request, view: bool, threads: usize) {
        let mut next = first;
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
                    self.drive(&s, &mut p, view, threads)
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
    /// Renders one scene from whatever viewpoints arrive. Returns a request for another scene, or
    /// None when told to stop. A cancelled pass always has a request behind it, which starts the
    /// sums over.
    fn drive(
        &self,
        s: &Settings,
        p: &mut Progressive<'_>,
        view: bool,
        threads: usize,
    ) -> Option<Request> {
        let cap = s.max_samples.max(1);
        let mut block = 8;
        let mut previewed = !view;
        let mut spp = 1;
        let mut spent = 0.;
        while !self.stopped() {
            if let Some(r) = self.take() {
                if !same_but_camera(s, &r.settings) {
                    return Some(r);
                }
                p.move_camera(&r.settings, r.size);
                previewed = !view;
                spp = 1;
                spent = 0.;
            }
            if !previewed {
                previewed = true;
                // Not cancelled by the next request: while the camera moves each frame brings one,
                // and this image is the whole point of the preview. At block 1 it costs a sample
                // per pixel, little beside a full pass that wouldn't finish between two frames.
                let t = Instant::now();
                if let Some(image) = p.preview(block, threads, &self.stop) {
                    self.publish(image, true, 0, 0.);
                }
                let blocks = (p.scene.size[0] as usize).div_ceil(block)
                    * (p.scene.size[1] as usize).div_ceil(block);
                let pixels = p.scene.size[0] as f32 * p.scene.size[1] as f32;
                block = preview_block(t.elapsed().as_secs_f32() * pixels / blocks as f32);
                continue;
            }
            if p.samples >= cap || (self.paused.load(Ordering::Relaxed) && p.samples > 0) {
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
            self.publish(p.image(), false, p.samples, spent);
        }
        None
    }
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
