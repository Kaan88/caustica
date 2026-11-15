//! Lifetime and session statistics (roadmap 5b), kept in %APPDATA%\Caustica\stats.json.
//! Interactive runs only: headless captures and the self-test never touch the file.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Stats {
    /// Camera paths traced: pixels × samples.
    pub paths: f64,
    pub gpu_seconds: f64,
    pub images_saved: u64,
    pub videos_exported: u64,
    pub max_samples: u32,
    pub presets_seen: BTreeSet<String>,
    pub scenes_seen: BTreeSet<String>,
    /// Scene units walked.
    pub walked: f64,
    /// Seconds with physics running.
    pub physics_seconds: f64,
    /// Seconds on screen, by integrator and by denoiser (labels from `integrator_label` and
    /// `Denoise::label`).
    pub integrator_seconds: BTreeMap<String, f64>,
    pub denoiser_seconds: BTreeMap<String, f64>,
    /// Samples and seconds per scene title.
    pub scene_use: BTreeMap<String, SceneUse>,
    /// Images and videos saved, by format label.
    pub exports: BTreeMap<String, u64>,
    /// The best camera paths per GPU second seen over a window of about a second.
    pub best_paths_per_s: f64,
    /// Since the app started; never saved.
    #[serde(skip)]
    pub session: Session,
    /// Where the walker was last frame, to measure the distance.
    #[serde(skip)]
    last_eye: Option<glam::Vec3>,
    /// The paths-per-second window being filled (paths, GPU seconds), and the last result.
    #[serde(skip)]
    rate_window: (f64, f64),
    #[serde(skip)]
    pub paths_per_s: f64,
}
/// What one scene has had: samples added to the image and seconds on screen.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SceneUse {
    pub samples: f64,
    pub seconds: f64,
}
/// The figures counted from app start, not saved.
#[derive(Clone)]
pub struct Session {
    pub started: std::time::Instant,
    pub paths: f64,
    pub gpu_seconds: f64,
    pub images: u64,
    pub videos: u64,
    pub max_samples: u32,
    pub integrator_seconds: BTreeMap<String, f64>,
    pub denoiser_seconds: BTreeMap<String, f64>,
    pub exports: BTreeMap<String, u64>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            started: std::time::Instant::now(),
            paths: 0.,
            gpu_seconds: 0.,
            images: 0,
            videos: 0,
            max_samples: 0,
            integrator_seconds: BTreeMap::new(),
            denoiser_seconds: BTreeMap::new(),
            exports: BTreeMap::new(),
        }
    }
}
/// Which denoiser cleaned the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Denoise {
    Raw,
    Native,
    Rr,
}
impl Denoise {
    pub fn label(self) -> &'static str {
        match self {
            Self::Raw => "None (raw samples)",
            Self::Native => "Native filter",
            Self::Rr => "DLSS Ray Reconstruction",
        }
    }
}
/// The integrator's name as the statistics count it (`Moment::integrator`: 0 path tracing,
/// 1 BDPT, 2 photon mapping; battle mode runs them side by side).
pub fn integrator_label(integrator: u32, battle: bool) -> &'static str {
    match (battle, integrator) {
        (true, _) => "Battle mode",
        (_, 1) => "Bidirectional (BDPT)",
        (_, 2) => "Photon mapping",
        _ => "Path tracing",
    }
}
/// Adds to a map entry without allocating a key once it exists (runs every frame).
fn bump<V: std::ops::AddAssign>(map: &mut BTreeMap<String, V>, key: &str, by: V) {
    match map.get_mut(key) {
        Some(v) => *v += by,
        None => {
            map.insert(key.to_string(), by);
        }
    }
}
/// One frame, as `Stats::observe` needs it.
pub struct Moment<'a> {
    pub samples: u32,
    pub seconds: f32,
    pub scene: &'a str,
    pub integrator: u32,
    pub battle: bool,
    pub denoiser: Denoise,
    /// Samples added to the image since the last frame, the render size in pixels, and the
    /// GPU time of the frame.
    pub new_samples: u32,
    pub pixels: u64,
    pub gpu_seconds: f64,
    /// The eye, while walking.
    pub walking: Option<glam::Vec3>,
    pub physics: bool,
}
fn path() -> Option<PathBuf> {
    crate::prefs::app_dir().map(|d| d.join("stats.json"))
}
impl Stats {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, json);
            }
        }
    }
    /// Counts a saved image (`still`) or exported video under its format label.
    pub fn exported(&mut self, label: &str, still: bool) {
        if still {
            self.images_saved += 1;
            self.session.images += 1;
        } else {
            self.videos_exported += 1;
            self.session.videos += 1;
        }
        bump(&mut self.exports, label, 1);
        bump(&mut self.session.exports, label, 1);
    }
    /// Clears the saved figures.
    /// Clears the saved figures; this session's keep counting.
    pub fn reset(&mut self) {
        let session = std::mem::take(&mut self.session);
        *self = Self {
            session,
            ..Self::default()
        };
    }
    /// The scenes with the most time, longest first.
    pub fn top_scenes(&self, n: usize) -> Vec<(&str, &SceneUse)> {
        let mut v: Vec<_> = self
            .scene_use
            .iter()
            .map(|(k, u)| (k.as_str(), u))
            .collect();
        v.sort_by(|a, b| b.1.seconds.total_cmp(&a.1.seconds));
        v.truncate(n);
        v
    }
    /// Counts one frame towards the goals that depend on what is on screen.
    pub fn observe(&mut self, m: Moment) {
        // A frame longer than a second was a pause or a stall, not time spent.
        let dt = f64::from(m.seconds).clamp(0., 1.);
        let paths = f64::from(m.new_samples) * m.pixels as f64;
        self.paths += paths;
        self.gpu_seconds += m.gpu_seconds;
        self.max_samples = self.max_samples.max(m.samples);
        let ses = &mut self.session;
        ses.paths += paths;
        ses.gpu_seconds += m.gpu_seconds;
        ses.max_samples = ses.max_samples.max(m.samples);
        let integrator = integrator_label(m.integrator, m.battle);
        bump(&mut self.integrator_seconds, integrator, dt);
        bump(&mut ses.integrator_seconds, integrator, dt);
        bump(&mut self.denoiser_seconds, m.denoiser.label(), dt);
        bump(&mut ses.denoiser_seconds, m.denoiser.label(), dt);
        if !m.scene.is_empty() {
            match self.scene_use.get_mut(m.scene) {
                Some(u) => {
                    u.samples += f64::from(m.new_samples);
                    u.seconds += dt;
                }
                None => {
                    self.scene_use.insert(
                        m.scene.to_string(),
                        SceneUse {
                            samples: f64::from(m.new_samples),
                            seconds: dt,
                        },
                    );
                }
            }
        }
        // Paths per GPU second over windows of about one second of GPU time.
        self.rate_window.0 += paths;
        self.rate_window.1 += m.gpu_seconds;
        if self.rate_window.1 >= 1. {
            self.paths_per_s = self.rate_window.0 / self.rate_window.1;
            self.best_paths_per_s = self.best_paths_per_s.max(self.paths_per_s);
            self.rate_window = (0., 0.);
        } else if m.new_samples == 0 && m.gpu_seconds == 0. {
            // Nothing rendering (converged or paused): the rate is not "now" any more.
            self.paths_per_s = 0.;
            self.rate_window = (0., 0.);
        }
        match m.walking {
            Some(eye) => {
                // A jump this big in one frame is a teleport, or a new scene.
                if let Some(last) = self.last_eye
                    && eye.distance(last) < 2.
                {
                    self.walked += f64::from(eye.distance(last));
                }
                self.last_eye = Some(eye);
            }
            None => self.last_eye = None,
        }
        if m.physics {
            self.physics_seconds += dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn moment<'a>() -> Moment<'a> {
        Moment {
            samples: 1,
            seconds: 0.01,
            denoiser: Denoise::Rr,
            new_samples: 1,
            pixels: 1000,
            gpu_seconds: 0.005,
            scene: "",
            integrator: 0,
            battle: false,
            walking: None,
            physics: false,
        }
    }
    #[test]
    fn walking_counts_steps_but_not_teleports() {
        let mut s = Stats::default();
        for x in [0., 1., 2., 100., 101.] {
            s.observe(Moment {
                walking: Some(glam::Vec3::X * x),
                ..moment()
            });
        }
        assert_eq!(s.walked, 3.);
    }
    #[test]
    fn old_files_with_dropped_fields_still_load() {
        let old = r#"{"paths":5e9,"dark_seconds":12.5,"hot_reloads":7,"images_saved":2,
            "puzzles_solved":["Laser 1"],"puzzles_par":[],"audio_seconds":12.5,
            "marks":["first-light"],"achievements":{"billion":"2026-11-01"}}"#;
        let s: Stats = serde_json::from_str(old).unwrap();
        assert_eq!(s.images_saved, 2);
        assert!(s.exports.is_empty() && s.scene_use.is_empty());
        assert_eq!(s.best_paths_per_s, 0.);
    }
    #[test]
    fn frames_add_up_per_scene_integrator_and_denoiser() {
        let mut s = Stats::default();
        for (scene, integrator, battle, denoiser) in [
            ("A", 0, false, Denoise::Rr),
            ("A", 1, false, Denoise::Native),
            ("B", 0, true, Denoise::Raw),
        ] {
            s.observe(Moment {
                scene,
                integrator,
                battle,
                denoiser,
                new_samples: 2,
                ..moment()
            });
        }
        assert_eq!(s.paths, 6000.);
        assert_eq!(s.session.paths, 6000.);
        assert_eq!(s.scene_use["A"].samples, 4.);
        assert!((s.scene_use["A"].seconds - 0.02).abs() < 1e-9);
        assert_eq!(s.integrator_seconds.len(), 3);
        assert!(s.integrator_seconds.contains_key("Battle mode"));
        assert_eq!(s.denoiser_seconds.len(), 3);
        assert_eq!(s.top_scenes(1)[0].0, "A");
    }
    #[test]
    fn paths_per_second_tracks_now_and_best() {
        let mut s = Stats::default();
        // 1000 px x 1 sample per 5 ms GPU frame = 200,000 paths/s; 200 frames = 1 s.
        for _ in 0..201 {
            s.observe(moment());
        }
        assert!((s.paths_per_s - 200_000.).abs() < 1.);
        assert!((s.best_paths_per_s - 200_000.).abs() < 1.);
        // Slower later: now drops, best stays.
        for _ in 0..101 {
            s.observe(Moment {
                gpu_seconds: 0.01,
                ..moment()
            });
        }
        assert!((s.paths_per_s - 100_000.).abs() < 1000.);
        assert!((s.best_paths_per_s - 200_000.).abs() < 1.);
        s.observe(Moment {
            new_samples: 0,
            gpu_seconds: 0.,
            ..moment()
        });
        assert_eq!(s.paths_per_s, 0.);
    }
    #[test]
    fn exports_count_by_format_and_reset_clears_them() {
        let mut s = Stats::default();
        s.exported("PNG", true);
        s.exported("PNG", true);
        s.exported("H.264 MP4", false);
        assert_eq!((s.images_saved, s.videos_exported), (2, 1));
        assert_eq!(s.exports["PNG"], 2);
        assert_eq!((s.session.images, s.session.videos), (2, 1));
        s.observe(moment());
        s.reset();
        assert_eq!((s.images_saved, s.paths), (0, 0.));
        assert!(s.exports.is_empty() && s.scene_use.is_empty());
        // The session's figures are not saved, so a reset leaves them.
        assert_eq!((s.session.images, s.session.videos), (2, 1));
    }
}
