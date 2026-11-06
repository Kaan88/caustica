//! Renders that grow by passes: the CLI's and the viewport's.
use super::camera::filter_offset;
use super::sampling::Rng;
use super::scene::Scene;
use super::unsupported;
use crate::scene::Settings;
use anyhow::Result;
use glam::{DVec3, Vec2};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// A render that grows by passes: per-pixel sums of samples `0..samples`, each seeded by pixel
/// and sample index, so any split into passes gives the same bits as one pass. The sums are f64:
/// past tens of thousands of samples an f32 sum drops the low bits of what it adds.
pub struct Progressive<'a> {
    scene: Scene<'a>,
    pub(super) sums: Vec<DVec3>,
    pub samples: u32,
}
impl<'a> Progressive<'a> {
    pub fn new(s: &'a Settings, size: [u32; 2]) -> Result<Self> {
        let missing = unsupported(s);
        anyhow::ensure!(
            missing.is_empty(),
            "The CPU reference can't render this scene: {}",
            missing.join(", ")
        );
        Ok(Progressive {
            scene: Scene::new(s, size),
            sums: vec![DVec3::ZERO; size[0] as usize * size[1] as usize],
            samples: 0,
        })
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
        let next_row = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let rows: Vec<std::sync::Mutex<&mut [DVec3]>> =
            self.sums.chunks_mut(w).map(std::sync::Mutex::new).collect();
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let y = next_row.fetch_add(1, Ordering::Relaxed);
                        if y >= h || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let mut row = rows[y].lock().unwrap_or_else(|e| e.into_inner());
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
                    }
                });
            }
        });
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
}

/// The CPU reference rendering in the background for the viewport (Look → Compare). It renders
/// in passes of about half a second and publishes the mean after each; dropping it cancels the
/// render (the thread finishes its current row and exits on its own).
pub struct Live {
    /// The transport key and output size it renders; a change means a restart.
    pub key: String,
    pub size: [u32; 2],
    /// A different key and size waiting to settle (camera or slider drags) before a restart.
    pub pending: Option<(String, [u32; 2], std::time::Instant)>,
    shared: std::sync::Arc<Shared>,
}
#[derive(Default)]
struct Shared {
    cancel: AtomicBool,
    state: std::sync::Mutex<LiveState>,
}
#[derive(Default, Clone)]
pub struct LiveState {
    /// The latest mean, and whether it has not been shown yet.
    pub image: Option<std::sync::Arc<Vec<f32>>>,
    pub fresh: bool,
    pub samples: u32,
    pub cap: u32,
    pub seconds: f32,
    /// Why it can't render (unsupported features), shown instead of the image.
    pub error: Option<String>,
}
impl Live {
    pub fn start(s: Settings, size: [u32; 2], key: String) -> Live {
        let shared = std::sync::Arc::new(Shared::default());
        let cap = s.max_samples.max(1);
        lock(&shared.state).cap = cap;
        let job = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("cpu-reference".into())
            .spawn(move || {
                let start = std::time::Instant::now();
                let mut p = match Progressive::new(&s, size) {
                    Ok(p) => p,
                    Err(e) => {
                        lock(&job.state).error = Some(format!("{e:#}"));
                        return;
                    }
                };
                // One core stays free for the UI and the GPU's submissions.
                let threads = cores().saturating_sub(1).max(1);
                let mut spp = 1;
                while p.samples < cap && !job.cancel.load(Ordering::Relaxed) {
                    let t = std::time::Instant::now();
                    let n = spp.min(cap - p.samples);
                    if !p.pass(n, threads, &job.cancel, &|_| {}) {
                        return;
                    }
                    let per_sample = t.elapsed().as_secs_f32() / n as f32;
                    spp = ((0.5 / per_sample.max(1e-6)) as u32).clamp(1, 64);
                    let image = std::sync::Arc::new(p.image());
                    let mut st = lock(&job.state);
                    st.image = Some(image);
                    st.fresh = true;
                    st.samples = p.samples;
                    st.seconds = start.elapsed().as_secs_f32();
                }
            });
        if let Err(e) = spawned {
            lock(&shared.state).error = Some(format!("Starting the CPU render: {e}"));
        }
        Live {
            key,
            size,
            pending: None,
            shared,
        }
    }
    /// Progress without the image.
    pub fn state(&self) -> LiveState {
        let st = lock(&self.shared.state);
        LiveState {
            image: None,
            fresh: st.fresh,
            samples: st.samples,
            cap: st.cap,
            seconds: st.seconds,
            error: st.error.clone(),
        }
    }
    /// Still rendering (not finished, refused or failed).
    pub fn running(&self) -> bool {
        let st = lock(&self.shared.state);
        st.error.is_none() && st.samples < st.cap
    }
    /// The latest mean if it is new, or with `again` whenever there is one (the reference
    /// texture was recreated by a resize, an export or an HDR switch).
    pub fn image(&self, again: bool) -> Option<std::sync::Arc<Vec<f32>>> {
        let mut st = lock(&self.shared.state);
        if !(st.fresh || again) {
            return None;
        }
        st.fresh = false;
        st.image.clone()
    }
    /// Stops the render (idempotent); the thread finishes its row and exits.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
