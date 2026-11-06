//! Self-update: asks GitHub for the latest release at startup, offers it, downloads it in the
//! background, verifies it against the release's SHA256SUMS and installs it, through the setup
//! exe on Windows and by replacing the running AppImage on Linux (engine-notes "Self-update").
//! Other installs (a zip, a build from source) get the release page instead.
use crate::assets::Progress;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The repository whose releases are offered. It must be public: the check sends no token.
pub const REPO: &str = "Kaan88/caustica";
const SUMS: &str = "SHA256SUMS";
/// The app's own arguments that only make sense for the run that updates.
const FLAGS_WITH_VALUE: [&str; 2] = ["--update-from", "--update-cancel-at"];
const FLAGS: [&str; 1] = ["--update-yes"];

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// This platform's download, if the release has one.
    pub asset: Option<Asset>,
    /// Where the checksums are, if the release has them.
    pub sums: Option<String>,
}

/// `v1.2.3` as numbers, and whether a suffix (`-dev`, `-rc1`) follows.
fn parse_version(v: &str) -> Option<(Vec<u64>, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, suffix) = v.split_once('-').map_or((v, false), |(c, _)| (c, true));
    let numbers: Option<Vec<u64>> = core.split('.').map(|n| n.parse().ok()).collect();
    Some((numbers?, suffix))
}
/// Whether `latest` is a newer version than `current`. A suffix ranks below its release, and
/// what can't be read is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some((a, a_suffix)), Some((b, b_suffix))) =
        (parse_version(latest), parse_version(current))
    else {
        return false;
    };
    let len = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    for i in 0..len {
        match at(&a, i).cmp(&at(&b, i)) {
            std::cmp::Ordering::Equal => {}
            order => return order.is_gt(),
        }
    }
    b_suffix && !a_suffix
}

/// Whether a release asset is the download for this kind of install.
fn wanted(name: &str, windows: bool) -> bool {
    if windows {
        name.starts_with("caustica-v") && name.ends_with("-setup.exe")
    } else {
        name.starts_with("Caustica-") && name.ends_with("-x86_64.AppImage")
    }
}
/// A release from GitHub's JSON. Notes written by release.ps1 start with a BOM.
fn parse_release(json: &Value, windows: bool) -> Result<Release> {
    let assets = json["assets"].as_array().context("no assets")?;
    let url = |a: &Value| a["browser_download_url"].as_str().map(str::to_string);
    let name = |a: &Value| a["name"].as_str().unwrap_or_default().to_string();
    Ok(Release {
        version: json["tag_name"].as_str().context("no tag")?.into(),
        notes: json["body"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('\u{feff}')
            .trim()
            .into(),
        page: json["html_url"].as_str().unwrap_or_default().into(),
        asset: assets
            .iter()
            .find(|a| wanted(&name(a), windows))
            .and_then(|a| {
                Some(Asset {
                    name: name(a),
                    url: url(a)?,
                    size: a["size"].as_u64().unwrap_or(0),
                })
            }),
        sums: assets.iter().find(|a| name(a) == SUMS).and_then(url),
    })
}
/// The SHA-256 that a `sha256sum`-format text gives for `file`, in lower case.
fn expected_sum(text: &str, file: &str) -> Result<String> {
    for line in text.trim_start_matches('\u{feff}').lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.trim_start_matches('*') == file
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    bail!("{SUMS} has no checksum for {file}")
}

/// How this copy of the app can be updated.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The setup exe, over an installed copy.
    Setup,
    /// Replace this AppImage.
    AppImage(PathBuf),
    /// Can't update itself: why, for the notice beside the release page.
    Page(&'static str),
}
/// `path` is the running AppImage ($APPIMAGE), if there is one.
#[cfg_attr(windows, allow(dead_code))]
fn appimage_target(path: Option<PathBuf>) -> Target {
    let Some(path) = path.filter(|p| p.is_file()) else {
        return Target::Page(
            "This copy isn't running from an AppImage, so it can't replace itself.",
        );
    };
    // Downloading 700 MB first only to find this out would be a poor surprise.
    let probe = path.with_file_name(format!(".caustica-write-test-{}", std::process::id()));
    if std::fs::write(&probe, b"").is_err() {
        return Target::Page("The AppImage's folder isn't writable, so it can't be replaced.");
    }
    let _ = std::fs::remove_file(&probe);
    Target::AppImage(path)
}
#[cfg(not(windows))]
fn target() -> Target {
    appimage_target(std::env::var_os("APPIMAGE").map(PathBuf::from))
}
#[cfg(windows)]
fn target() -> Target {
    let installed = crate::dirs::mode() == crate::dirs::Mode::Installed
        && std::env::current_exe()
            .ok()
            .and_then(|e| Some(e.parent()?.join("unins000.exe").is_file()))
            .unwrap_or(false);
    if installed {
        Target::Setup
    } else {
        Target::Page("This copy wasn't installed by the setup, so it can't update itself.")
    }
}
impl Target {
    /// The verified download, and the file it passes through while downloading.
    fn download_path(&self) -> Option<PathBuf> {
        match self {
            Target::Setup => Some(
                crate::dirs::cache()?
                    .join(format!("caustica-update-{}-setup.exe", std::process::id())),
            ),
            Target::AppImage(path) => {
                Some(path.with_file_name(format!(".caustica-update-{}", std::process::id())))
            }
            Target::Page(_) => None,
        }
    }
}

/// Removes update files an interrupted run left in `dir`. Each process has its own, and a
/// running download keeps touching its own, so only an old one is safe to remove.
fn remove_stale(dir: &Path, age: std::time::Duration) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|e| e >= age);
        if old && (name.starts_with(".caustica-update-") || name.starts_with("caustica-update-")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
/// A local file or a URL.
fn open(source: &str) -> Result<(Box<dyn std::io::Read + Send>, u64)> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let response = crate::assets::get(source)?;
        let size = response
            .header("Content-Length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok((Box::new(response.into_reader()), size))
    } else {
        let file = std::fs::File::open(source).with_context(|| format!("Opening {source}"))?;
        let size = file.metadata()?.len();
        Ok((Box::new(file), size))
    }
}
/// Fetches the checksums, then the file, and keeps it only if its checksum matches. No
/// checksum, or a wrong one, refuses the update.
fn fetch_verified(asset: &Asset, sums: Option<&str>, dest: &Path, p: &Progress) -> Result<()> {
    use std::io::Read;
    let sums = sums.with_context(|| format!("The release has no {SUMS}"))?;
    let mut text = String::new();
    open(sums)
        .with_context(|| format!("Fetching {SUMS}"))?
        .0
        .take(1 << 20)
        .read_to_string(&mut text)
        .with_context(|| format!("Reading {SUMS}"))?;
    let expected = expected_sum(&text, &asset.name)?;
    let (reader, size) = open(&asset.url)?;
    p.total.store(size, Ordering::Relaxed);
    let actual = crate::assets::save_stream(reader, dest, p)?;
    if actual != expected {
        let _ = std::fs::remove_file(dest);
        bail!("{} doesn't match its checksum", asset.name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
/// This run's arguments without the update switches, for the relaunch.
fn relaunch_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = vec![];
    let mut args = args;
    while let Some(a) = args.next() {
        if FLAGS_WITH_VALUE.contains(&a.as_str()) {
            args.next();
        } else if !FLAGS.contains(&a.as_str()) {
            out.push(a);
        }
    }
    out
}

pub enum Phase {
    Idle,
    Available(Release),
    Downloading {
        release: Release,
        progress: Arc<Progress>,
        done: Receiver<Result<(), String>>,
    },
    Ready {
        release: Release,
        file: PathBuf,
    },
    Failed {
        release: Release,
        error: String,
    },
}
pub struct Updater {
    pub phase: Phase,
    pub target: Target,
    check: Option<Receiver<Option<Release>>>,
    /// Debug switches: click through without asking, and cancel after this many bytes.
    pub yes: bool,
    pub cancel_at: Option<u64>,
}
impl Updater {
    pub fn new(yes: bool, cancel_at: Option<u64>) -> Self {
        Self {
            phase: Phase::Idle,
            target: Target::Page(""),
            check: None,
            yes,
            cancel_at,
        }
    }
    /// Removes what an interrupted run left, and works out how this copy updates.
    fn prepare(&mut self) {
        self.target = target();
        if let Some(dir) = self
            .target
            .download_path()
            .as_deref()
            .and_then(Path::parent)
        {
            remove_stale(dir, std::time::Duration::from_secs(3600));
        }
    }
    /// The startup check: a thread nothing waits for, and any failure just offers nothing.
    pub fn check(&mut self) {
        self.prepare();
        let (send, receive) = channel();
        self.check = Some(receive);
        std::thread::spawn(move || {
            let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
            let release =
                crate::assets::get_json(&url).and_then(|json| parse_release(&json, cfg!(windows)));
            // Only the console hears about a failure.
            match &release {
                Ok(r) => println!("UPDATE_CHECK latest {}", r.version),
                Err(e) => println!("UPDATE_CHECK failed: {e:#}"),
            }
            let release = release
                .ok()
                .filter(|r| is_newer(&r.version, crate::VERSION));
            let _ = send.send(release);
        });
    }
    /// `--update-from`: a local file or URL offered as the newest version, its checksums in a
    /// SHA256SUMS beside it.
    pub fn offer_from(&mut self, source: &str) {
        self.prepare();
        let cut = source.rfind(['/', '\\']).map_or(0, |i| i + 1);
        let size = if source.starts_with("http") {
            0
        } else {
            std::fs::metadata(source).map_or(0, |m| m.len())
        };
        self.phase = Phase::Available(Release {
            version: "test".into(),
            notes: format!("Test update from {source}"),
            page: String::new(),
            asset: Some(Asset {
                name: source[cut..].into(),
                url: source.into(),
                size,
            }),
            sums: Some(format!("{}{SUMS}", &source[..cut])),
        });
    }
    /// Work is under way that nothing else would redraw for.
    pub fn busy(&self) -> bool {
        self.check.is_some() || matches!(self.phase, Phase::Downloading { .. })
    }
    /// Picks up finished work; returns true if a download was cancelled. A thread that died
    /// counts as finished, so the window and the redraws can't wait on it for ever.
    pub fn poll(&mut self) -> bool {
        if let Some(check) = &self.check {
            match check.try_recv() {
                Ok(release) => {
                    self.check = None;
                    if let Some(release) = release {
                        self.phase = Phase::Available(release);
                    }
                }
                Err(TryRecvError::Disconnected) => self.check = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let Phase::Downloading {
            release,
            progress,
            done,
        } = &self.phase
        else {
            return false;
        };
        let result = match done.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => Err("The download stopped unexpectedly".into()),
            Err(TryRecvError::Empty) => return false,
        };
        let cancelled = result.is_err() && progress.cancel.load(Ordering::Relaxed);
        let release = release.clone();
        self.phase = match result {
            Ok(()) => match self.target.download_path() {
                Some(file) => Phase::Ready { release, file },
                None => Phase::Idle,
            },
            Err(_) if cancelled => Phase::Idle,
            Err(error) => Phase::Failed { release, error },
        };
        cancelled
    }
    pub fn install_failed(&mut self, error: String) {
        if let Phase::Ready { release, .. } = &self.phase {
            self.phase = Phase::Failed {
                release: release.clone(),
                error,
            };
        }
    }
    pub fn download(&mut self, release: Release) {
        let (Some(dest), Some(asset)) = (self.target.download_path(), release.asset.clone()) else {
            return;
        };
        let sums = release.sums.clone();
        let progress = Arc::new(Progress::default());
        let (send, done) = channel();
        let worker = progress.clone();
        std::thread::spawn(move || {
            let result = fetch_verified(&asset, sums.as_deref(), &dest, &worker);
            let _ = send.send(result.map_err(|e| format!("{e:#}")));
        });
        self.phase = Phase::Downloading {
            release,
            progress,
            done,
        };
    }
    /// Puts the verified download in place and starts it. On an error the app keeps running.
    pub fn install(&self) -> Result<()> {
        let Phase::Ready { file, .. } = &self.phase else {
            bail!("Nothing is ready to install");
        };
        let mut command = match &self.target {
            Target::Setup => {
                let mut c = std::process::Command::new(file);
                c.args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/relaunch=1",
                ]);
                c
            }
            Target::AppImage(path) => {
                std::fs::rename(file, path)
                    .with_context(|| format!("Replacing {}", path.display()))?;
                let mut c = std::process::Command::new(path);
                c.args(relaunch_args(std::env::args().skip(1)));
                c.env_remove("APPIMAGE")
                    .env_remove("APPDIR")
                    .env_remove("ARGV0");
                if let Some(dir) = std::env::var_os("OWD") {
                    c.current_dir(dir).env_remove("OWD");
                }
                c
            }
            Target::Page(why) => bail!("{why}"),
        };
        command.env(UPDATED_FROM, crate::VERSION);
        command.spawn().context(match self.target {
            Target::AppImage(_) => {
                "The update is installed, but it couldn't start: start it by hand"
            }
            _ => "Starting the update",
        })?;
        Ok(())
    }
}
/// Set for the process an update starts: the version it replaced.
pub const UPDATED_FROM: &str = "CAUSTICA_UPDATED_FROM";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        for (latest, current, newer) in [
            ("v0.10.2", "0.10.1", true),
            ("0.11.0", "0.10.9", true),
            ("v1.0.0", "0.99.99", true),
            // Numbers, not text.
            ("v0.10.0", "0.9.0", true),
            ("v0.9.0", "0.10.0", false),
            ("v0.10.1", "0.10.1", false),
            ("v0.10.1", "v0.10.2", false),
            ("0.11", "0.10.9", true),
            ("0.10", "0.10.0", false),
            // A suffix ranks below its release.
            ("0.10.1", "0.10.1-dev", true),
            ("0.10.1-rc1", "0.10.1", false),
            ("0.10.1-rc2", "0.10.1-rc1", false),
            ("0.10.2-rc1", "0.10.1", true),
            // What can't be read is never newer.
            ("latest", "0.10.1", false),
            ("0.10.1", "unknown", false),
            ("", "0.10.1", false),
            ("v1.x.0", "0.1.0", false),
        ] {
            assert_eq!(is_newer(latest, current), newer, "{latest} over {current}");
        }
    }
    #[test]
    fn release_from_github() {
        let json = serde_json::from_str(include_str!("testdata/release-v0.10.1.json")).unwrap();
        let linux = parse_release(&json, false).unwrap();
        assert_eq!(linux.version, "v0.10.1");
        assert!(
            linux.notes.starts_with("Changes since"),
            "BOM: {:?}",
            &linux.notes[..4]
        );
        let asset = linux.asset.unwrap();
        assert_eq!(asset.name, "Caustica-0.10.1-x86_64.AppImage");
        assert!(asset.size > 500 << 20);
        // v0.10.1 predates the checksums: it stays offered, and the download then refuses.
        assert_eq!(linux.sums, None);
        let windows = parse_release(&json, true).unwrap().asset.unwrap();
        assert_eq!(windows.name, "caustica-v0.10.1-setup.exe");
        assert!(parse_release(&serde_json::json!({"tag_name": "v1"}), false).is_err());
    }
    const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    #[test]
    fn checksum_lines() {
        let text = format!("{HASH}  a.exe\n{}  b.AppImage\n", "0".repeat(64));
        assert_eq!(expected_sum(&text, "a.exe").unwrap(), HASH);
        assert_eq!(expected_sum(&text, "b.AppImage").unwrap(), "0".repeat(64));
        // A BOM, CRLF, binary-mode names and upper case from other tools.
        let text = format!("\u{feff}{}  *a.exe\r\n", HASH.to_uppercase());
        assert_eq!(expected_sum(&text, "a.exe").unwrap(), HASH);
        // Only the exact file; nothing that isn't a SHA-256.
        assert!(expected_sum(&format!("{HASH}  xa.exe\n"), "a.exe").is_err());
        assert!(expected_sum("abc123  a.exe\n", "a.exe").is_err());
        assert!(expected_sum(&format!("{}  a.exe", "z".repeat(64)), "a.exe").is_err());
        assert!(expected_sum("", "a.exe").is_err());
    }
    /// A scratch folder holding `abc` as a.exe (`HASH` is its SHA-256).
    fn fixture(name: &str) -> (PathBuf, Asset) {
        let dir =
            std::env::temp_dir().join(format!("caustica-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.exe"), b"abc").unwrap();
        let asset = Asset {
            name: "a.exe".into(),
            url: dir.join("a.exe").to_string_lossy().into(),
            size: 3,
        };
        (dir, asset)
    }
    fn sums(dir: &Path, text: &str) -> String {
        std::fs::write(dir.join(SUMS), text).unwrap();
        dir.join(SUMS).to_string_lossy().into()
    }
    #[test]
    fn verified_download() {
        let (dir, asset) = fixture("verified");
        let dest = dir.join("out");
        let s = sums(&dir, &format!("{HASH}  a.exe\n"));
        fetch_verified(&asset, Some(&s), &dest, &Progress::default()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn refused_downloads_leave_nothing() {
        let (dir, asset) = fixture("refused");
        let dest = dir.join("out");
        let p = Progress::default();
        let missing = dir.join("none").to_string_lossy().to_string();
        let wrong = sums(&dir, &format!("{}  a.exe\n", "0".repeat(64)));
        assert!(
            fetch_verified(&asset, Some(&wrong), &dest, &p)
                .unwrap_err()
                .to_string()
                .contains("match")
        );
        let other = sums(&dir, &format!("{HASH}  other.exe\n"));
        assert!(fetch_verified(&asset, Some(&other), &dest, &p).is_err());
        assert!(fetch_verified(&asset, Some(&missing), &dest, &p).is_err());
        assert!(fetch_verified(&asset, None, &dest, &p).is_err());
        assert!(!dest.exists() && !dest.with_extension("partial").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn stale_files_go_but_recent_ones_stay() {
        let (dir, _) = fixture("stale");
        let update = dir.join(".caustica-update-1");
        std::fs::write(&update, b"x").unwrap();
        remove_stale(&dir, std::time::Duration::from_secs(3600));
        assert!(update.exists(), "a running download's file");
        remove_stale(&dir, std::time::Duration::ZERO);
        assert!(!update.exists());
        assert!(dir.join("a.exe").exists(), "other files stay");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cancelled_download_leaves_nothing() {
        let (dir, asset) = fixture("cancel");
        let dest = dir.join("out");
        let s = sums(&dir, &format!("{HASH}  a.exe\n"));
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(fetch_verified(&asset, Some(&s), &dest, &p).is_err());
        assert!(!dest.exists() && !dest.with_extension("partial").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn appimage_targets() {
        let (dir, _) = fixture("target");
        let file = dir.join("a.exe");
        assert_eq!(appimage_target(Some(file.clone())), Target::AppImage(file));
        assert!(matches!(appimage_target(None), Target::Page(_)));
        assert!(matches!(
            appimage_target(Some(dir.join("gone"))),
            Target::Page(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn relaunch_drops_update_switches() {
        let args = [
            "--frames",
            "9",
            "--update-from",
            "x",
            "--update-yes",
            "--update-cancel-at",
            "1",
            "--no-rr",
        ];
        assert_eq!(
            relaunch_args(args.iter().map(|a| a.to_string())),
            ["--frames", "9", "--no-rr"]
        );
    }
    /// Scene files from an old release still load (scenes::read validates them too).
    #[test]
    fn old_scene_file_loads() {
        let value = serde_json::from_str(include_str!("testdata/scene-v0.3.2.json")).unwrap();
        crate::scenes::parse(value).unwrap().validate().unwrap();
    }
    /// One-off sweep: every scene under $CAUSTICA_OLD_SCENES, e.g. `git archive v0.3.2 scenes`.
    #[test]
    #[ignore]
    fn old_scene_folder_loads() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "json")
                    && p.file_name().is_some_and(|n| n != "pack.json")
                {
                    out.push(p);
                }
            }
        }
        let mut files = vec![];
        walk(
            Path::new(&std::env::var("CAUSTICA_OLD_SCENES").unwrap()),
            &mut files,
        );
        let failed: Vec<String> = files
            .iter()
            .filter_map(|f| crate::scenes::read(f).err().map(|e| format!("{e:#}")))
            .collect();
        println!("{} files, {} failed", files.len(), failed.len());
        assert!(failed.is_empty(), "{}", failed.join("\n"));
    }
}
