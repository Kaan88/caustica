//! Lifetime statistics and achievements (roadmap 5b), kept in %APPDATA%\Caustica\stats.json.
//! Interactive runs only: headless captures and the self-test never touch the file.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// How many built-in laser levels there are (`scenes/puzzles`).
const PUZZLE_LEVELS: usize = 5;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Stats {
    /// Camera paths traced: pixels × samples.
    pub paths: f64,
    pub gpu_seconds: f64,
    pub images_saved: u64,
    pub videos_exported: u64,
    pub max_samples: u32,
    pub hot_reloads: u64,
    pub presets_seen: BTreeSet<String>,
    pub scenes_seen: BTreeSet<String>,
    /// Laser levels solved, and those solved within par.
    pub puzzles_solved: BTreeSet<String>,
    pub puzzles_par: BTreeSet<String>,
    /// Scene units walked.
    pub walked: f64,
    /// Seconds walking with the flashlight on, with physics running, with audio-reactive lights.
    pub dark_seconds: f64,
    pub physics_seconds: f64,
    pub audio_seconds: f64,
    /// One-off feats, by achievement id.
    pub marks: BTreeSet<String>,
    /// Achievement id → date unlocked. Retired ids stay in the file and aren't shown.
    pub achievements: BTreeMap<String, String>,
    /// Where the walker was last frame, to measure the distance.
    #[serde(skip)]
    last_eye: Option<glam::Vec3>,
}
/// What one achievement counts towards.
#[derive(Clone, Copy)]
pub enum Goal {
    Paths(f64),
    GpuHours(f64),
    Samples(u32),
    Images(u64),
    Videos(u64),
    Reloads(u64),
    Presets(usize),
    Scenes(usize),
    /// Every built-in scene file: only the caller knows the list.
    AllScenes,
    Puzzles(usize),
    PuzzlesPar(usize),
    Walked(f64),
    DarkMinutes(f64),
    PhysicsMinutes(f64),
    AudioMinutes(f64),
    /// Set by `Stats::observe` or `Stats::mark` under the achievement's own id.
    Mark,
}
pub struct Achievement {
    pub id: &'static str,
    pub title: &'static str,
    pub how: &'static str,
    pub group: &'static str,
    pub goal: Goal,
}
const fn a(
    group: &'static str,
    id: &'static str,
    title: &'static str,
    how: &'static str,
    goal: Goal,
) -> Achievement {
    Achievement {
        id,
        title,
        how,
        group,
        goal,
    }
}
pub const GROUPS: [&str; 6] = [
    "Mileage", "Patience", "Explorer", "Craft", "Toys", "Tinkerer",
];
/// Sized so a tier means something on a fast card: an RTX 4070 Ti SUPER at 1080p traces about
/// 200 million paths a second, 65,536 samples in about 11 minutes.
pub const ACHIEVEMENTS: [Achievement; 37] = [
    a(
        "Mileage",
        "first-light",
        "First light",
        "Render your first frame",
        Goal::Mark,
    ),
    a(
        "Mileage",
        "paths-100g",
        "Warming up",
        "Trace 100 billion camera paths",
        Goal::Paths(1e11),
    ),
    a(
        "Mileage",
        "paths-1t",
        "A trillion paths",
        "Trace 1,000,000,000,000 camera paths",
        Goal::Paths(1e12),
    ),
    a(
        "Mileage",
        "paths-10t",
        "Ten trillion",
        "Trace 10 trillion camera paths",
        Goal::Paths(1e13),
    ),
    a(
        "Mileage",
        "paths-1q",
        "A quadrillion",
        "Trace 1,000 trillion camera paths",
        Goal::Paths(1e15),
    ),
    a(
        "Mileage",
        "gpu-1h",
        "Warm card",
        "Spend an hour of GPU time",
        Goal::GpuHours(1.),
    ),
    a(
        "Mileage",
        "gpu-24h",
        "Space heater",
        "Spend a day of GPU time",
        Goal::GpuHours(24.),
    ),
    a(
        "Mileage",
        "gpu-168h",
        "Burn-in",
        "Spend a week of GPU time",
        Goal::GpuHours(168.),
    ),
    a(
        "Patience",
        "long-exposure",
        "Long exposure",
        "Reach 65,536 samples per pixel",
        Goal::Samples(65_536),
    ),
    a(
        "Patience",
        "deep-field",
        "Deep field",
        "Reach 1,000,000 samples per pixel",
        Goal::Samples(1_000_000),
    ),
    a(
        "Patience",
        "caustic-hunter",
        "Caustic hunter",
        "Converge the Cornell caustic to 16,384 spp",
        Goal::Mark,
    ),
    a(
        "Patience",
        "second-opinion",
        "Second opinion",
        "Let the CPU reference reach 1,024 spp in the viewport",
        Goal::Mark,
    ),
    a(
        "Explorer",
        "tourist",
        "Tourist",
        "Visit every built-in preset",
        Goal::Presets(crate::scene::PRESETS.len()),
    ),
    a(
        "Explorer",
        "day-tripper",
        "Day tripper",
        "Open 25 scene files",
        Goal::Scenes(25),
    ),
    a(
        "Explorer",
        "collector",
        "Collector",
        "Open every built-in scene file",
        Goal::AllScenes,
    ),
    a(
        "Craft",
        "photographer",
        "Photographer",
        "Save 10 images",
        Goal::Images(10),
    ),
    a(
        "Craft",
        "portfolio",
        "Portfolio",
        "Save 100 images",
        Goal::Images(100),
    ),
    a(
        "Craft",
        "archive",
        "Archive",
        "Save 1,000 images",
        Goal::Images(1000),
    ),
    a(
        "Craft",
        "billboard",
        "Billboard",
        "Save a still at least 7,680 pixels wide",
        Goal::Mark,
    ),
    a(
        "Craft",
        "director",
        "Director",
        "Export a video",
        Goal::Videos(1),
    ),
    a(
        "Craft",
        "studio",
        "Studio",
        "Export 25 videos",
        Goal::Videos(25),
    ),
    a(
        "Craft",
        "both-ways",
        "Both ways",
        "Take BDPT to 4,096 spp",
        Goal::Mark,
    ),
    a(
        "Craft",
        "photon-count",
        "Photon count",
        "Take photon mapping to 4,096 spp",
        Goal::Mark,
    ),
    a(
        "Craft",
        "battle",
        "Battle royale",
        "Take battle mode to 1,024 spp",
        Goal::Mark,
    ),
    a(
        "Craft",
        "prism",
        "Prism",
        "Take spectral rendering to 4,096 spp",
        Goal::Mark,
    ),
    a(
        "Craft",
        "glass-eye",
        "Glass eye",
        "Take a real lens to 4,096 spp",
        Goal::Mark,
    ),
    a(
        "Craft",
        "beyond-white",
        "Beyond white",
        "Converge to 4,096 spp on an HDR display",
        Goal::Mark,
    ),
    a(
        "Toys",
        "beam",
        "Beam",
        "Solve a laser level",
        Goal::Puzzles(1),
    ),
    a(
        "Toys",
        "optician",
        "Optician",
        "Solve every laser level within par",
        Goal::PuzzlesPar(PUZZLE_LEVELS),
    ),
    a(
        "Toys",
        "long-walk",
        "A long walk",
        "Walk 1,000 units",
        Goal::Walked(1000.),
    ),
    a(
        "Toys",
        "horror-game",
        "Horror game",
        "Walk 10 minutes by flashlight",
        Goal::DarkMinutes(10.),
    ),
    a(
        "Toys",
        "sandbox",
        "Sandbox",
        "Run physics for an hour",
        Goal::PhysicsMinutes(60.),
    ),
    a(
        "Toys",
        "dj",
        "DJ",
        "Run audio-reactive lights for an hour",
        Goal::AudioMinutes(60.),
    ),
    a(
        "Tinkerer",
        "hot-shader",
        "Hot shader",
        "Reload shaders while the app runs",
        Goal::Reloads(1),
    ),
    a(
        "Tinkerer",
        "shader-artist",
        "Shader artist",
        "Reload shaders 100 times",
        Goal::Reloads(100),
    ),
    a(
        "Tinkerer",
        "wordsmith",
        "Wordsmith",
        "Make or change a scene from a description",
        Goal::Mark,
    ),
    a(
        "Tinkerer",
        "scripter",
        "Scripter",
        "Run a script to its end",
        Goal::Mark,
    ),
];
/// One frame, as `Stats::observe` needs it.
pub struct Moment<'a> {
    pub samples: u32,
    pub seconds: f32,
    pub scene: &'a str,
    pub integrator: u32,
    pub battle: bool,
    pub spectral: bool,
    pub real_lens: bool,
    pub hdr: bool,
    /// The eye, while walking.
    pub walking: Option<glam::Vec3>,
    pub flashlight: bool,
    pub physics: bool,
    pub audio: bool,
    pub cpu_samples: u32,
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
    pub fn mark(&mut self, id: &str) {
        self.marks.insert(id.into());
    }
    pub fn solved(&mut self, level: &str, within_par: bool) {
        self.puzzles_solved.insert(level.into());
        if within_par {
            self.puzzles_par.insert(level.into());
        }
    }
    /// Counts one frame towards the goals that depend on what is on screen.
    pub fn observe(&mut self, m: Moment) {
        let dt = f64::from(m.seconds);
        match m.walking {
            Some(eye) => {
                // A jump this big in one frame is a teleport, or a new scene.
                if let Some(last) = self.last_eye
                    && eye.distance(last) < 2.
                {
                    self.walked += f64::from(eye.distance(last));
                }
                self.last_eye = Some(eye);
                if m.flashlight {
                    self.dark_seconds += dt;
                }
            }
            None => self.last_eye = None,
        }
        if m.physics {
            self.physics_seconds += dt;
        }
        if m.audio {
            self.audio_seconds += dt;
        }
        for (id, reached) in [
            ("first-light", m.samples > 0),
            (
                "caustic-hunter",
                m.scene == "Cornell caustic" && m.samples >= 16_384,
            ),
            ("second-opinion", m.cpu_samples >= 1024),
            (
                "both-ways",
                m.integrator == 1 && !m.battle && m.samples >= 4096,
            ),
            (
                "photon-count",
                m.integrator == 2 && !m.battle && m.samples >= 4096,
            ),
            ("battle", m.battle && m.samples >= 1024),
            ("prism", m.spectral && m.samples >= 4096),
            ("glass-eye", m.real_lens && m.samples >= 4096),
            ("beyond-white", m.hdr && m.samples >= 4096),
        ] {
            if reached {
                self.mark(id);
            }
        }
    }
    /// Where a goal stands: (now, target).
    pub fn progress(&self, goal: Goal) -> (f64, f64) {
        match goal {
            Goal::Paths(n) => (self.paths, n),
            Goal::GpuHours(h) => (self.gpu_seconds / 3600., h),
            Goal::Samples(n) => (f64::from(self.max_samples), f64::from(n)),
            Goal::Images(n) => (self.images_saved as f64, n as f64),
            Goal::Videos(n) => (self.videos_exported as f64, n as f64),
            Goal::Reloads(n) => (self.hot_reloads as f64, n as f64),
            Goal::Presets(n) => (self.presets_seen.len() as f64, n as f64),
            Goal::Scenes(n) => (self.scenes_seen.len() as f64, n as f64),
            Goal::Puzzles(n) => (self.puzzles_solved.len() as f64, n as f64),
            Goal::PuzzlesPar(n) => (self.puzzles_par.len() as f64, n as f64),
            Goal::Walked(n) => (self.walked, n),
            Goal::DarkMinutes(n) => (self.dark_seconds / 60., n),
            Goal::PhysicsMinutes(n) => (self.physics_seconds / 60., n),
            Goal::AudioMinutes(n) => (self.audio_seconds / 60., n),
            Goal::AllScenes | Goal::Mark => (0., 1.),
        }
    }
    /// Unlocks every achievement newly reached; returns their titles. `all_scenes`: every
    /// built-in scene file has been opened.
    pub fn unlock_reached(&mut self, all_scenes: bool) -> Vec<&'static str> {
        let mut new = vec![];
        for a in &ACHIEVEMENTS {
            if self.achievements.contains_key(a.id) {
                continue;
            }
            let reached = match a.goal {
                Goal::Mark => self.marks.contains(a.id),
                Goal::AllScenes => all_scenes,
                goal => {
                    let (now, target) = self.progress(goal);
                    now >= target
                }
            };
            if reached {
                self.achievements.insert(a.id.into(), chrono_free_date());
                new.push(a.title);
            }
        }
        new
    }
}
/// Today's date as YYYY-MM-DD from the system clock (UTC), without a date crate.
fn chrono_free_date() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn moment<'a>() -> Moment<'a> {
        Moment {
            samples: 1,
            seconds: 0.01,
            scene: "",
            integrator: 0,
            battle: false,
            spectral: false,
            real_lens: false,
            hdr: false,
            walking: None,
            flashlight: false,
            physics: false,
            audio: false,
            cpu_samples: 0,
        }
    }
    #[test]
    fn dates_are_valid() {
        let d = super::chrono_free_date();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
    #[test]
    fn the_table_holds_together() {
        let mut ids = BTreeSet::new();
        for a in &ACHIEVEMENTS {
            assert!(ids.insert(a.id), "duplicate {}", a.id);
            assert!(GROUPS.contains(&a.group), "{} has no group", a.id);
        }
        // Every mark is set by something: `observe` here, or a caller for these three.
        let mut s = Stats::default();
        s.observe(Moment {
            scene: "Cornell caustic",
            samples: u32::MAX,
            cpu_samples: u32::MAX,
            spectral: true,
            real_lens: true,
            hdr: true,
            battle: true,
            ..moment()
        });
        for integrator in [1, 2] {
            s.observe(Moment {
                integrator,
                samples: 4096,
                ..moment()
            });
        }
        for id in ["billboard", "wordsmith", "scripter"] {
            s.mark(id);
        }
        for a in ACHIEVEMENTS.iter().filter(|a| matches!(a.goal, Goal::Mark)) {
            assert!(s.marks.contains(a.id), "nothing sets {}", a.id);
        }
    }
    #[test]
    fn a_fresh_run_earns_little_and_nothing_fast() {
        // A minute on a fast card: 200 million paths a second.
        let mut s = Stats {
            paths: 200e6 * 60.,
            gpu_seconds: 60.,
            max_samples: 6000,
            ..Stats::default()
        };
        s.observe(moment());
        assert_eq!(s.unlock_reached(false), ["First light"]);
    }
    #[test]
    fn a_tier_unlocks_once_with_its_title() {
        let mut s = Stats {
            paths: 1e12,
            ..Stats::default()
        };
        assert_eq!(s.unlock_reached(false), ["Warming up", "A trillion paths"]);
        assert!(s.unlock_reached(false).is_empty());
    }
    #[test]
    fn walking_counts_steps_but_not_teleports() {
        let mut s = Stats::default();
        for x in [0., 1., 2., 100., 101.] {
            s.observe(Moment {
                walking: Some(glam::Vec3::X * x),
                flashlight: true,
                ..moment()
            });
        }
        assert_eq!(s.walked, 3.);
        assert!(s.dark_seconds > 0.);
    }
    #[test]
    fn an_old_stats_file_loads_and_its_retired_achievement_stays() {
        let old = r#"{"paths":2e9,"images_saved":3,"achievements":{"billion":"2026-11-01"}}"#;
        let mut s: Stats = serde_json::from_str(old).unwrap();
        assert!(s.unlock_reached(false).is_empty());
        assert_eq!(s.achievements.len(), 1);
    }
}
