//! Shader hot reload (roadmap 5a, debug builds): watches `shaders/` and recompiles every entry
//! with the same DXC and flags as build.rs. On an error the old pipelines stay and the DXC
//! message is shown.
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Compiled DXIL for every pipeline.
pub struct Shaders {
    pub trace: Vec<u8>,
    pub trace_maps: Vec<u8>,
    pub trace_paths: Vec<u8>,
    pub trace_lpe: Vec<u8>,
    pub restir_presample: Vec<u8>,
    pub restir_initial: Vec<u8>,
    pub restir_spatial: Vec<u8>,
    pub bdpt: Vec<u8>,
    pub bdpt_resolve: Vec<u8>,
    pub photon_clear: Vec<u8>,
    pub photon_trace: Vec<u8>,
    pub photon_gather: Vec<u8>,
    pub filter: Vec<u8>,
    pub display: Vec<u8>,
    pub vs: Vec<u8>,
    pub ps: Vec<u8>,
}
/// The entries build.rs compiles: (file, entry, profile, output name, defines).
pub const ENTRIES: [(&str, &str, &str, &str, &[&str]); 16] = [
    ("pathtrace.hlsl", "trace", "cs_6_5", "trace", &[]),
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_maps",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_paths",
        &["-D", "TEXTURE_MAPS", "-D", "PATH_RECORD"],
    ),
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_lpe",
        &["-D", "TEXTURE_MAPS", "-D", "LPE"],
    ),
    (
        "pathtrace.hlsl",
        "restir_presample",
        "cs_6_5",
        "restir_presample",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "pathtrace.hlsl",
        "restir_initial",
        "cs_6_5",
        "restir_initial",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "pathtrace.hlsl",
        "restir_spatial",
        "cs_6_5",
        "restir_spatial",
        &["-D", "TEXTURE_MAPS"],
    ),
    // Track 15 integrators (BDPT, photon mapping).
    (
        "integrators.hlsl",
        "bdpt",
        "cs_6_5",
        "bdpt",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "integrators.hlsl",
        "bdpt_resolve",
        "cs_6_5",
        "bdpt_resolve",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "integrators.hlsl",
        "photon_clear",
        "cs_6_5",
        "photon_clear",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "integrators.hlsl",
        "photon_trace",
        "cs_6_5",
        "photon_trace",
        &["-D", "TEXTURE_MAPS"],
    ),
    (
        "integrators.hlsl",
        "photon_gather",
        "cs_6_5",
        "photon_gather",
        &["-D", "TEXTURE_MAPS"],
    ),
    ("post.hlsl", "filter", "cs_6_5", "filter", &[]),
    ("post.hlsl", "display", "cs_6_5", "display", &[]),
    ("gui.hlsl", "vs_main", "vs_6_0", "vs_main", &[]),
    ("gui.hlsl", "ps_main", "ps_6_0", "ps_main", &[]),
];
pub struct HotReload {
    dir: PathBuf,
    dxc: PathBuf,
    stamp: SystemTime,
    checked: Instant,
}
impl HotReload {
    /// Watches the source tree's `shaders/` (baked in at build time), if it still exists.
    pub fn new() -> Option<Self> {
        let dir = PathBuf::from(env!("CORNELL_SHADERS"));
        let dxc = PathBuf::from(env!("CORNELL_DXC"));
        (dir.is_dir() && dxc.is_file()).then(|| Self {
            stamp: newest(&dir),
            dir,
            dxc,
            checked: Instant::now(),
        })
    }
    /// After a shader file changes: Some(compiled shaders or the DXC error).
    pub fn poll(&mut self) -> Option<Result<Shaders, String>> {
        if self.checked.elapsed() < Duration::from_millis(500) {
            return None;
        }
        self.checked = Instant::now();
        let stamp = newest(&self.dir);
        if stamp <= self.stamp {
            return None;
        }
        self.stamp = stamp;
        Some(self.compile())
    }
    pub fn compile(&self) -> Result<Shaders, String> {
        let out = std::env::temp_dir().join("cornell-rtx-shaders");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let mut blobs = vec![];
        for (file, entry, profile, name, defines) in ENTRIES {
            let target = out.join(format!("{name}.dxil"));
            let mut command = std::process::Command::new(&self.dxc);
            command
                .args([
                    "-T", profile, "-E", entry, "-HV", "2021", "-O3", "-Ges", "-I",
                ])
                .arg(&self.dir)
                .args(defines)
                .arg("-Fo")
                .arg(&target)
                .arg(self.dir.join(file));
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let result = command.output().map_err(|e| format!("DXC: {e}"))?;
            if !result.status.success() {
                let text = String::from_utf8_lossy(&result.stderr);
                // The first error lines are enough for a toast.
                let short: Vec<&str> = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(4)
                    .collect();
                return Err(format!("{name}: {}", short.join("\n")));
            }
            blobs.push(std::fs::read(&target).map_err(|e| e.to_string())?);
        }
        let mut b = blobs.into_iter();
        Ok(Shaders {
            trace: b.next().unwrap(),
            trace_maps: b.next().unwrap(),
            trace_paths: b.next().unwrap(),
            trace_lpe: b.next().unwrap(),
            restir_presample: b.next().unwrap(),
            restir_initial: b.next().unwrap(),
            restir_spatial: b.next().unwrap(),
            bdpt: b.next().unwrap(),
            bdpt_resolve: b.next().unwrap(),
            photon_clear: b.next().unwrap(),
            photon_trace: b.next().unwrap(),
            photon_gather: b.next().unwrap(),
            filter: b.next().unwrap(),
            display: b.next().unwrap(),
            vs: b.next().unwrap(),
            ps: b.next().unwrap(),
        })
    }
}
fn newest(dir: &PathBuf) -> SystemTime {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
