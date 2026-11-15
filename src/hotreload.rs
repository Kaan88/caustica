//! Shader hot reload (roadmap 5a, debug builds): watches `shaders/` and recompiles every entry
//! with the same DXC and flags as build.rs: DXIL for D3D12, SPIR-V for Vulkan (the SER
//! libraries are left empty there, as build.rs leaves them out). On an error the old pipelines
//! stay and the DXC message is shown.
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Compiled DXIL (or SPIR-V) for every pipeline.
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
    pub glare: Vec<u8>,
    pub vs: Vec<u8>,
    pub ps: Vec<u8>,
    pub trace_lean: Vec<u8>,
    pub trace_maps_lean: Vec<u8>,
    pub trace_ser: Vec<u8>,
    pub trace_maps_ser: Vec<u8>,
    pub trace_lean_ser: Vec<u8>,
    pub trace_maps_lean_ser: Vec<u8>,
    pub trace_lpe_ser: Vec<u8>,
    pub trace_spectral: Vec<u8>,
    pub trace_spectral_ser: Vec<u8>,
    pub photon_trace_spectral: Vec<u8>,
    pub trace_spectral_lean: Vec<u8>,
    pub trace_spectral_lean_ser: Vec<u8>,
    pub photon_gather_spectral: Vec<u8>,
    pub bdpt_spectral: Vec<u8>,
    pub trace_exact: Vec<u8>,
    pub trace_exact_ser: Vec<u8>,
    pub trace_spectral_exact: Vec<u8>,
    pub trace_spectral_exact_ser: Vec<u8>,
    pub bdpt_exact: Vec<u8>,
    pub bdpt_spectral_exact: Vec<u8>,
}
/// The entries build.rs compiles: (file, entry, profile, output name, defines).
pub use crate::shader_entries::ENTRIES;
pub struct HotReload {
    dir: PathBuf,
    dxc: PathBuf,
    spirv: bool,
    stamp: SystemTime,
    checked: Instant,
}
impl HotReload {
    /// Watches the source tree's `shaders/` (baked in at build time), if it still exists.
    /// `spirv`: compile for the Vulkan backend (needs the SPIR-V DXC build.rs used).
    pub fn new(spirv: bool) -> Option<Self> {
        let dir = PathBuf::from(env!("CAUSTICA_SHADERS"));
        let dxc = PathBuf::from(if spirv {
            option_env!("CAUSTICA_DXC_SPIRV")?
        } else {
            env!("CAUSTICA_DXC")
        });
        (dir.is_dir() && dxc.is_file()).then(|| Self {
            stamp: newest(&dir),
            dir,
            dxc,
            spirv,
            checked: Instant::now(),
        })
    }
    /// A shader file is newer than the loaded set: an idle event loop (paused, converged) then
    /// redraws once so `poll` sees it.
    pub fn changed(&self) -> bool {
        newest(&self.dir) > self.stamp
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
    fn compile_one(
        &self,
        out: &std::path::Path,
        file: &str,
        entry: &str,
        profile: &str,
        name: &str,
        defines: &[&str],
    ) -> Result<Vec<u8>, String> {
        if self.spirv && profile.starts_with("lib_") {
            return Ok(vec![]);
        }
        let target = out.join(format!(
            "{name}.{}",
            if self.spirv { "spv" } else { "dxil" }
        ));
        let mut command = std::process::Command::new(&self.dxc);
        if self.spirv {
            command
                .args(["-spirv", "-fspv-target-env=vulkan1.3", "-fvk-use-dx-layout"])
                .args(["-fvk-t-shift", "100", "all", "-fvk-b-shift", "200", "all"])
                .args(["-fvk-s-shift", "300", "all"]);
        }
        command
            .args(["-T", profile])
            .args(if profile.starts_with("lib_") {
                vec!["-exports", entry, "-Wno-misplaced-attributes"]
            } else if self.spirv {
                vec!["-E", entry]
            } else {
                vec!["-E", entry, "-Ges"]
            })
            .args(["-HV", "2021", "-O3", "-I"])
            .arg(&self.dir)
            .args(defines)
            .arg("-Fo")
            .arg(&target)
            .arg(self.dir.join(file));
        crate::platform::no_window(&mut command);
        let result = command.output().map_err(|e| format!("DXC: {e}"))?;
        if !result.status.success() {
            let text = String::from_utf8_lossy(&result.stderr);
            // The first error lines are enough for a toast.
            let short: Vec<&str> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(4)
                .collect();
            return Err(format!(
                "{name}: {}",
                short.join(
                    "
"
                )
            ));
        }
        std::fs::read(&target).map_err(|e| e.to_string())
    }
    pub fn compile(&self) -> Result<Shaders, String> {
        let out = std::env::temp_dir().join("caustica-shaders");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        // One DXC process per entry, all at once (a sequential compile froze the window for over
        // a minute); the results keep ENTRIES' order.
        let blobs: Vec<Result<Vec<u8>, String>> = std::thread::scope(|scope| {
            let jobs: Vec<_> = ENTRIES
                .iter()
                .map(|&(file, entry, profile, name, defines)| {
                    let out = &out;
                    scope.spawn(move || self.compile_one(out, file, entry, profile, name, defines))
                })
                .collect();
            jobs.into_iter()
                .map(|j| {
                    j.join()
                        .unwrap_or_else(|_| Err("DXC thread panicked".into()))
                })
                .collect()
        });
        let blobs = blobs.into_iter().collect::<Result<Vec<_>, String>>()?;
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
            glare: b.next().unwrap(),
            vs: b.next().unwrap(),
            ps: b.next().unwrap(),
            trace_lean: b.next().unwrap(),
            trace_maps_lean: b.next().unwrap(),
            trace_ser: b.next().unwrap(),
            trace_maps_ser: b.next().unwrap(),
            trace_lean_ser: b.next().unwrap(),
            trace_maps_lean_ser: b.next().unwrap(),
            trace_lpe_ser: b.next().unwrap(),
            trace_spectral: b.next().unwrap(),
            trace_spectral_ser: b.next().unwrap(),
            photon_trace_spectral: b.next().unwrap(),
            trace_spectral_lean: b.next().unwrap(),
            trace_spectral_lean_ser: b.next().unwrap(),
            photon_gather_spectral: b.next().unwrap(),
            bdpt_spectral: b.next().unwrap(),
            trace_exact: b.next().unwrap(),
            trace_exact_ser: b.next().unwrap(),
            trace_spectral_exact: b.next().unwrap(),
            trace_spectral_exact_ser: b.next().unwrap(),
            bdpt_exact: b.next().unwrap(),
            bdpt_spectral_exact: b.next().unwrap(),
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
