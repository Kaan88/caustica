//! Material ball thumbnails (wave 3): a small render of a material on a sphere, shown beside
//! the material editor and in its library list. Balls render in the background in a headless
//! copy of the app, a batch at a time through the `--script` API (one process sets each
//! material, renders and captures), and are cached as PNGs by a hash of the material's JSON
//! (`dirs::cache()/materials`). Until a ball is ready, a flat disc of the base colour stands in.
use crate::scene::Material;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Off in headless runs and the self-test, which must not start other processes.
pub static ENABLED: AtomicBool = AtomicBool::new(false);

/// The preview stage: a grounded unit sphere on a checker floor under a sky, a key and a rim.
const SCENE: &str = r#"{"format":1,"title":"Material ball","settings":{
 "cornell_room":false,"floor_extent":6.0,"sky":0.35,"background":[0.0,0.0,0.0],
 "floor":{"color":[0.30,0.30,0.30],"secondary_color":[0.08,0.08,0.08],"pattern":"checker","pattern_scale":2.0,"roughness":0.7},
 "light_on":true,"light_position":[-1.5,3.0,2.0],"light_size":0.8,"light_intensity":40.0,"light_visible":false,
 "lights":[{"name":"Rim","position":[2.0,1.5,-2.0],"rotation":[0,0,0],"size":[1.0,1.0],"color":[1,1,1],"intensity":15.0,"enabled":true,"visible":false}],
 "yaw":0,"pitch":12,"distance":3.4,"target":[0,-0.4,0],"fov":30,
 "objects":[{"name":"Ball","shape":"sphere","position":[0,0],"size":1.3,"stretch":1.0,"scale_xz":[1.0,1.0],
  "elevation":0.0,"rotation":0.0,"tilt":[0,0],"grounded":true,"visible":true}]
}}"#;
/// Render size of a ball, frames per ball, balls per process.
const SIZE: u32 = 128;
const FRAMES: u32 = 40;
const BATCH: usize = 8;
/// A material being edited is rendered once it has stayed the same this long.
const SETTLE: Duration = Duration::from_millis(400);

#[derive(Default)]
struct Balls {
    textures: HashMap<u64, egui::TextureHandle>,
    /// Loaded, queued, rendering or failed: never asked for again (until the cache is cleared).
    asked: HashSet<u64>,
    queue: Vec<(u64, Material)>,
    worker: Option<Receiver<(u64, bool)>>,
    /// Per call site: the material's key and when it was first seen, and the last ball shown
    /// (kept on screen while an edited material's new ball renders).
    settle: HashMap<String, (u64, Instant)>,
    shown: HashMap<String, u64>,
}

thread_local! {
    static BALLS: std::cell::RefCell<Balls> = Default::default();
}

pub fn key(m: &Material) -> u64 {
    serde_json::to_vec(m)
        .unwrap_or_default()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)
        })
}
fn folder() -> Option<PathBuf> {
    crate::dirs::cache().map(|c| c.join("materials"))
}
/// Part of every cached ball's name: bump it when the stage or the render settings change, and
/// the old balls are deleted (`prune`).
const VERSION: &str = "v1";
fn file(key: u64) -> Option<PathBuf> {
    folder().map(|f| f.join(format!("{VERSION}-{key:016x}.png")))
}
/// Deletes balls of other versions and files left by an interrupted batch.
fn prune(dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with(&format!("{VERSION}-")) || name.ends_with(".partial.png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
/// A batch that hasn't finished in this long is killed (a hung child would block every ball).
const TIMEOUT: Duration = Duration::from_secs(120);
/// The ball shipped with the app (`dirs::bundled_thumbnails()/materials`), if any. Same name as
/// the cached one, which carries the hash of the material's JSON: a material (or the `Material`
/// struct) that has changed since the release hashes differently and finds no file here.
fn bundled(bundled_dir: &std::path::Path, key: u64) -> PathBuf {
    bundled_dir
        .join("materials")
        .join(format!("{VERSION}-{key:016x}.png"))
}
/// Renders a ball for each material preset not yet in the cache, waiting for each batch (the
/// release's `--make-thumbnails --thumbnails-out`). Returns (rendered, failed).
pub fn render_presets() -> (usize, usize) {
    let todo: Vec<(u64, Material)> = (0..crate::presets::MATERIAL_NAMES.len())
        .map(crate::presets::material)
        .map(|m| (key(&m), m))
        .filter(|(k, m)| !m.texture.contains(['`', '$']) && !file(*k).is_some_and(|f| f.is_file()))
        .collect();
    let (mut done, mut failed) = (0, 0);
    for chunk in todo.chunks(BATCH) {
        for (_, ok) in render(chunk.to_vec()) {
            if ok {
                done += 1;
            } else {
                failed += 1;
            }
        }
    }
    (done, failed)
}
fn load(ctx: &egui::Context, key: u64) -> Option<egui::TextureHandle> {
    let bytes = file(key).and_then(|f| std::fs::read(f).ok()).or_else(|| {
        let dir = crate::dirs::bundled_thumbnails()?;
        std::fs::read(bundled(&dir, key)).ok()
    })?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(
        format!("ball {key:016x}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}

impl Balls {
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.worker {
            loop {
                match rx.try_recv() {
                    Ok((k, true)) => {
                        if let Some(t) = load(ctx, k) {
                            self.textures.insert(k, t);
                        }
                        ctx.request_repaint();
                    }
                    Ok((k, false)) => {
                        // An edited material whose ball failed shows the disc, not the ball of
                        // the material it was before.
                        let slots: Vec<String> = self
                            .settle
                            .iter()
                            .filter(|(_, (key, _))| *key == k)
                            .map(|(s, _)| s.clone())
                            .collect();
                        for s in slots {
                            self.shown.remove(&s);
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(250));
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.worker = None;
                        break;
                    }
                }
            }
        }
        if self.worker.is_none() && !self.queue.is_empty() {
            let n = self.queue.len().min(BATCH);
            let batch: Vec<_> = self.queue.drain(..n).collect();
            self.worker = Some(render(batch));
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
    fn request(&mut self, ctx: &egui::Context, k: u64, m: &Material) {
        if !self.asked.insert(k) {
            return;
        }
        if let Some(t) = load(ctx, k) {
            self.textures.insert(k, t);
        } else if ENABLED.load(Ordering::Relaxed) && !m.texture.contains(['`', '$']) {
            // Backtick strings in Rhai end at ` and interpolate ${ }.
            self.queue.push((k, m.clone()));
            ctx.request_repaint();
        }
    }
}

/// Draws the ball of `m`, `size` points square. `slot` names a call site whose material is
/// being edited: its ball renders only once the material settles, and the previous ball stays
/// up meanwhile. Without a slot (a library entry) the ball is asked for at once.
pub fn ball(ui: &mut egui::Ui, slot: Option<&str>, m: &Material, size: f32) -> egui::Response {
    let ctx = ui.ctx().clone();
    let k = key(m);
    let texture = BALLS.with(|b| {
        let b = &mut *b.borrow_mut();
        if b.textures.len() > 256 {
            b.textures.clear();
            b.asked.clear();
        }
        b.poll(&ctx);
        if let Some(t) = b.textures.get(&k) {
            if let Some(s) = slot {
                b.shown.insert(s.into(), k);
            }
            return Some(t.id());
        }
        let ready = match slot {
            None => true,
            Some(s) => {
                let e = b.settle.entry(s.into()).or_insert((k, Instant::now()));
                if e.0 != k {
                    *e = (k, Instant::now());
                }
                e.1.elapsed() >= SETTLE
            }
        };
        if ready {
            b.request(&ctx, k, m);
        } else {
            ctx.request_repaint_after(SETTLE);
        }
        slot.and_then(|s| b.shown.get(s))
            .and_then(|k| b.textures.get(k))
            .map(|t| t.id())
    });
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    match texture {
        Some(t) => {
            egui::Image::new((t, rect.size()))
                .corner_radius(4.)
                .paint_at(ui, rect);
        }
        None => {
            // The base colour (linear) as a flat disc until the render arrives.
            let [r, g, b] = m
                .color
                .map(|c| (c.clamp(0., 1.).powf(1. / 2.2) * 255.) as u8);
            ui.painter()
                .rect_filled(rect, 4., egui::Color32::from_gray(40));
            ui.painter().circle_filled(
                rect.center(),
                size * 0.36,
                egui::Color32::from_rgb(r, g, b),
            );
        }
    }
    response
}

/// Renders a batch in a headless copy of the app; each ball is reported as it lands in the
/// cache (true) or when the batch failed (false).
fn render(batch: Vec<(u64, Material)>) -> Receiver<(u64, bool)> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let fail = |send: &std::sync::mpsc::Sender<(u64, bool)>| {
            for (k, _) in &batch {
                let _ = send.send((*k, false));
            }
        };
        let (Ok(exe), Some(dir)) = (std::env::current_exe(), folder()) else {
            return fail(&send);
        };
        if std::fs::create_dir_all(&dir).is_err() {
            return fail(&send);
        }
        prune(&dir);
        let tag = format!("{:016x}", batch[0].0);
        let scene = dir.join(format!("stage-{tag}.json"));
        let script = dir.join(format!("batch-{tag}.rhai"));
        let mut source = String::new();
        let mut parts = vec![];
        for (k, m) in &batch {
            let json = serde_json::to_string(m).unwrap_or_default();
            let partial = dir.join(format!("{k:016x}.partial.png"));
            source += &format!(
                "set(\"objects/0/material\", `{json}`);\nframes({FRAMES});\ncapture(\"{}\");\n",
                partial.to_string_lossy().replace('\\', "/")
            );
            parts.push((*k, partial));
        }
        if std::fs::write(&scene, SCENE).is_err() || std::fs::write(&script, source).is_err() {
            return fail(&send);
        }
        let mut command = std::process::Command::new(&exe);
        command.args([
            "--headless",
            "--width",
            &SIZE.to_string(),
            "--height",
            &SIZE.to_string(),
        ]);
        command
            .arg("--scene")
            .arg(&scene)
            .arg("--script")
            .arg(&script);
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::platform::no_window(&mut command); // no console for debug builds
        if let Ok(mut child) = command.spawn() {
            let start = std::time::Instant::now();
            while matches!(child.try_wait(), Ok(None)) {
                if start.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let _ = std::fs::remove_file(&scene);
        let _ = std::fs::remove_file(&script);
        for (k, partial) in parts {
            let ok = file(k).is_some_and(|f| std::fs::rename(&partial, f).is_ok());
            let _ = std::fs::remove_file(&partial);
            if send.send((k, ok)).is_err() {
                return;
            }
        }
    });
    receive
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The stage parses and validates as a scene, and equal materials share a key.
    #[test]
    fn the_ball_stage_is_a_valid_scene() {
        let mut file: crate::scenes::SceneFile = serde_json::from_str(SCENE).unwrap();
        file.settings.validate().unwrap();
        assert_eq!(file.settings.objects.len(), 1);
        let (a, mut b) = (Material::default(), Material::default());
        assert_eq!(key(&a), key(&b));
        b.roughness += 0.01;
        assert_ne!(key(&a), key(&b));
    }
    /// A bundled ball is found by the material's hash only: an edited material finds none.
    #[test]
    fn bundled_balls_are_found_by_material_hash() {
        let dir = std::path::Path::new("/bundle");
        let a = Material::default();
        let mut b = Material::default();
        b.roughness += 0.01;
        let name = |p: PathBuf| p.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            name(bundled(dir, key(&a))),
            format!("v1-{:016x}.png", key(&a))
        );
        assert_ne!(bundled(dir, key(&a)), bundled(dir, key(&b)));
        assert!(bundled(dir, key(&a)).starts_with("/bundle/materials"));
    }
}
