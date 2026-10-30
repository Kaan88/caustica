//! Lifetime statistics and achievements (roadmap 5b), kept in %APPDATA%\CornellRTX\stats.json.
//! Interactive runs only: headless captures and the self-test never touch the file.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Stats {
    /// Camera paths traced: pixels × samples.
    pub paths: f64,
    pub gpu_seconds: f64,
    pub images_saved: u64,
    pub max_samples: u32,
    pub hot_reloads: u64,
    pub presets_seen: BTreeSet<String>,
    pub scenes_seen: BTreeSet<String>,
    /// Achievement id → date unlocked.
    pub achievements: std::collections::BTreeMap<String, String>,
}
/// (id, title, how to get it)
pub const ACHIEVEMENTS: [(&str, &str, &str); 8] = [
    ("first-light", "First light", "Render your first frame"),
    (
        "long-exposure",
        "Long exposure",
        "Reach 65,536 samples per pixel",
    ),
    ("tourist", "Tourist", "Visit every built-in preset"),
    ("collector", "Collector", "Open every built-in scene file"),
    ("photographer", "Photographer", "Save 10 images"),
    (
        "caustic-hunter",
        "Caustic hunter",
        "Converge the Cornell caustic to 4,096 spp",
    ),
    (
        "hot-shader",
        "Hot shader",
        "Reload shaders while the app runs",
    ),
    (
        "billion",
        "A billion paths",
        "Trace 1,000,000,000 camera paths",
    ),
];
fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("CornellRTX").join("stats.json"))
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
    /// Unlocks `id` once; returns its title when it is new.
    pub fn unlock(&mut self, id: &str) -> Option<&'static str> {
        if self.achievements.contains_key(id) {
            return None;
        }
        let (_, title, _) = ACHIEVEMENTS.iter().find(|(i, _, _)| *i == id)?;
        let today = chrono_free_date();
        self.achievements.insert(id.into(), today);
        Some(title)
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
    #[test]
    fn dates_are_valid() {
        let d = super::chrono_free_date();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
