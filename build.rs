use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=shaders");
    println!("cargo:rerun-if-changed=bridge/ngx.cpp");
    println!("cargo:rerun-if-env-changed=DXC_PATH");
    // Cross-check mode (tools/check_windows.sh): on a Windows target, skip what only a Windows
    // machine can do (vendor SDK asserts, rc.exe, the C++ bridges, DLL copies, link args). `1`
    // writes empty .dxil stubs; `dxil` compiles real DXIL with the Linux DXC. Unset: normal build.
    println!("cargo:rerun-if-env-changed=CAUSTICA_CHECK_ONLY");
    let check_only = env::var("CAUSTICA_CHECK_ONLY")
        .ok()
        .filter(|v| !v.is_empty() && v != "0");
    // The switch is for type-checking the Windows side from Linux only: on a Windows host it would
    // build an exe with empty shaders and no bridges without failing, so refuse it there.
    if check_only.is_some() && cfg!(windows) {
        panic!("CAUSTICA_CHECK_ONLY is set on a Windows host; unset it for a real build");
    }
    let check_dxil = check_only.as_deref() == Some("dxil");
    // The release version: CI sets CAUSTICA_VERSION (the tag it is about to create, without
    // the v); local builds are the Cargo.toml version marked -dev.
    println!("cargo:rerun-if-env-changed=CAUSTICA_VERSION");
    let version = env::var("CAUSTICA_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}-dev", env::var("CARGO_PKG_VERSION").unwrap()));
    println!("cargo:rustc-env=CAUSTICA_VERSION={version}");
    let dxc = env::var_os("DXC_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if check_dxil {
                return PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
                    .join("vendor/dxc-linux/bin/dxc");
            }
            let root = PathBuf::from(
                env::var_os("ProgramFiles(x86)")
                    .unwrap_or_else(|| "C:\\Program Files (x86)".into()),
            )
            .join("Windows Kits/10/bin");
            let mut candidates: Vec<_> = std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join("x64/dxc.exe"))
                .filter(|p| p.exists())
                .collect();
            candidates.sort();
            candidates.pop().unwrap_or_else(|| PathBuf::from("dxc.exe"))
        });
    // Debug builds recompile shaders at runtime with the same DXC (src/hotreload.rs).
    println!("cargo:rustc-env=CAUSTICA_DXC={}", dxc.display());
    println!(
        "cargo:rustc-env=CAUSTICA_SHADERS={}",
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("shaders")
            .display()
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // CAUSTICA_SHADER_DEBUG=1 embeds debug info (same -O3 code) so Nsight's shader profiler can
    // map stalls to HLSL lines (track 20a). Off by default.
    println!("cargo:rerun-if-env-changed=CAUSTICA_SHADER_DEBUG");
    // Track 20 experiments: CAUSTICA_SER_BITS sets the SER coherence-hint width (0: no
    // MaybeReorderThread, just the ray generation shader); CAUSTICA_SER_AT the one bounce to reorder at.
    println!("cargo:rerun-if-env-changed=CAUSTICA_SER_BITS");
    println!("cargo:rerun-if-env-changed=CAUSTICA_SER_AT");
    let mut ser_bits: Vec<String> = vec![];
    for (var, define) in [
        ("CAUSTICA_SER_BITS", "SER_BITS"),
        ("CAUSTICA_SER_AT", "SER_AT"),
    ] {
        if let Ok(v) = env::var(var) {
            ser_bits.extend(["-D".into(), format!("{define}={v}")]);
        }
    }
    let shader_debug: &[&str] = if env::var("CAUSTICA_SHADER_DEBUG").is_ok_and(|v| v == "1") {
        &["-Zi", "-Qembed_debug"]
    } else {
        &[]
    };
    // Same list as src/hotreload.rs: (file, entry, profile, output name, defines). The path
    // tracer is built twice: scenes without texture maps use the variant without map code
    // (core track 5; the map code alone cost untextured scenes about 7%).
    let entries: &[(&str, &str, &str, &str, &[&str])] = &[
        ("pathtrace.hlsl", "trace", "cs_6_5", "trace", &[][..]),
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_maps",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        // The path visualizer's recording trace (track 14), with the map code.
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_paths",
            &["-D", "TEXTURE_MAPS", "-D", "PATH_RECORD"][..],
        ),
        // Light path expressions (track 14), with the map code; used while any slot is set.
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_lpe",
            &["-D", "TEXTURE_MAPS", "-D", "LPE"][..],
        ),
        // ReSTIR DI (track 15): light tiles, initial / temporal, spatial.
        (
            "pathtrace.hlsl",
            "restir_presample",
            "cs_6_5",
            "restir_presample",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "pathtrace.hlsl",
            "restir_initial",
            "cs_6_5",
            "restir_initial",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "pathtrace.hlsl",
            "restir_spatial",
            "cs_6_5",
            "restir_spatial",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        // Track 15: BDPT and photon mapping (integrators.hlsl includes pathtrace.hlsl).
        (
            "integrators.hlsl",
            "bdpt",
            "cs_6_5",
            "bdpt",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "integrators.hlsl",
            "bdpt_resolve",
            "cs_6_5",
            "bdpt_resolve",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "integrators.hlsl",
            "photon_clear",
            "cs_6_5",
            "photon_clear",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "integrators.hlsl",
            "photon_trace",
            "cs_6_5",
            "photon_trace",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        (
            "integrators.hlsl",
            "photon_gather",
            "cs_6_5",
            "photon_gather",
            &["-D", "TEXTURE_MAPS"][..],
        ),
        ("post.hlsl", "filter", "cs_6_5", "filter", &[]),
        ("post.hlsl", "display", "cs_6_5", "display", &[]),
        ("post.hlsl", "glare_pass", "cs_6_5", "glare", &[]),
        ("gui.hlsl", "vs_main", "vs_6_0", "vs_main", &[]),
        ("gui.hlsl", "ps_main", "ps_6_0", "ps_main", &[]),
        // Track 20a (b): the tracer without ReSTIR, fog and volumes, SDFs or layered-BSDF
        // extras, for scenes that use none (Gpu::lean_scene); about a third faster.
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_lean",
            &["-D", "LEAN"],
        ),
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_maps_lean",
            &["-D", "TEXTURE_MAPS", "-D", "LEAN"],
        ),
        // Track 20: the four tracer variants as SER ray generation shaders (Shader Model 6.9,
        // a DXIL library exporting trace_rg; Gpu uses them where the device supports SER).
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_ser",
            &["-D", "SER"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_maps_ser",
            &["-D", "TEXTURE_MAPS", "-D", "SER"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_lean_ser",
            &["-D", "LEAN", "-D", "SER"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_maps_lean_ser",
            &["-D", "TEXTURE_MAPS", "-D", "LEAN", "-D", "SER"],
        ),
        // The light-path-expression tracer too (Gpu::ser index 4).
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_lpe_ser",
            &["-D", "TEXTURE_MAPS", "-D", "LPE", "-D", "SER"],
        ),
        // Spectral rendering: the full tracer (with the map code) carrying wavelengths, as a
        // compute kernel and as the SER ray generation shader (Gpu::ser index 5).
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_spectral",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_spectral_ser",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL", "-D", "SER"],
        ),
        // Photon mapping with spectral photons (one wavelength each; the gather stays RGB).
        (
            "integrators.hlsl",
            "photon_trace",
            "cs_6_5",
            "photon_trace_spectral",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL"],
        ),
        // LEAN spectral (track 20a's LEAN rules), compute and SER (Gpu::ser index 6).
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_spectral_lean",
            &["-D", "TEXTURE_MAPS", "-D", "LEAN", "-D", "SPECTRAL"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_spectral_lean_ser",
            &[
                "-D",
                "TEXTURE_MAPS",
                "-D",
                "LEAN",
                "-D",
                "SPECTRAL",
                "-D",
                "SER",
            ],
        ),
        // The photon gather's camera side and BDPT, one wavelength per sample.
        (
            "integrators.hlsl",
            "photon_gather",
            "cs_6_5",
            "photon_gather_spectral",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL"],
        ),
        (
            "integrators.hlsl",
            "bdpt",
            "cs_6_5",
            "bdpt_spectral",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL"],
        ),
        // Accuracy A1 (Settings::exact_glass, the Accurate bundle): the exact rough dielectric
        // (EXACT_BTDF) in the full tracer with the map code (no LEAN twin: Accurate is not a
        // speed mode), its spectral twin, their SER twins (Gpu::ser indices 7, 8), and BDPT.
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_exact",
            &["-D", "TEXTURE_MAPS", "-D", "EXACT_BTDF"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_exact_ser",
            &["-D", "TEXTURE_MAPS", "-D", "EXACT_BTDF", "-D", "SER"],
        ),
        (
            "pathtrace.hlsl",
            "trace",
            "cs_6_5",
            "trace_spectral_exact",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL", "-D", "EXACT_BTDF"],
        ),
        (
            "pathtrace.hlsl",
            "trace_rg",
            "lib_6_9",
            "trace_spectral_exact_ser",
            &[
                "-D",
                "TEXTURE_MAPS",
                "-D",
                "SPECTRAL",
                "-D",
                "EXACT_BTDF",
                "-D",
                "SER",
            ],
        ),
        (
            "integrators.hlsl",
            "bdpt",
            "cs_6_5",
            "bdpt_exact",
            &["-D", "TEXTURE_MAPS", "-D", "EXACT_BTDF"],
        ),
        (
            "integrators.hlsl",
            "bdpt",
            "cs_6_5",
            "bdpt_spectral_exact",
            &["-D", "TEXTURE_MAPS", "-D", "SPECTRAL", "-D", "EXACT_BTDF"],
        ),
    ];
    // SPIR-V for the Vulkan backend from the same entries (not the SER libraries): register
    // spaces are descriptor sets, and t / b / s registers shift apart from u (u0 binding 0, t0
    // binding 100, b0 200, s0 300); D3D layout rules keep `Constants` and the structured
    // buffers byte-identical to D3D12. The DXC release build (tools/setup.ps1 / setup.sh) has
    // the SPIR-V code generator; the Windows SDK's dxc doesn't. Optional on Windows (no
    // Vulkan renderer without it), required elsewhere.
    let windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    println!("cargo:rerun-if-env-changed=DXC_SPIRV");
    println!("cargo::rustc-check-cfg=cfg(spirv)");
    let dxc_spirv = env::var_os("DXC_SPIRV")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if windows && check_only.is_none() {
                "vendor/dxc-spirv/bin/x64/dxc.exe"
            } else {
                "vendor/dxc-linux/bin/dxc"
            })
        });
    if dxc_spirv.exists() {
        println!("cargo:rustc-cfg=spirv");
        // Hot reload on Vulkan compiles SPIR-V with the same DXC and flags as below.
        println!(
            "cargo:rustc-env=CAUSTICA_DXC_SPIRV={}",
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join(&dxc_spirv)
                .display()
        );
        for &(file, entry, profile, name, defines) in entries {
            if profile.starts_with("lib_") {
                continue;
            }
            let status = Command::new(&dxc_spirv)
                .args(["-spirv", "-fspv-target-env=vulkan1.3", "-fvk-use-dx-layout"])
                .args(["-fvk-t-shift", "100", "all", "-fvk-b-shift", "200", "all"])
                .args(["-fvk-s-shift", "300", "all"])
                .args([
                    "-T", profile, "-E", entry, "-HV", "2021", "-O3", "-I", "shaders",
                ])
                .args(defines)
                .arg("-Fo")
                .arg(out.join(format!("{name}.spv")))
                .arg(format!("shaders/{file}"))
                .status()
                .expect("run the SPIR-V DXC");
            assert!(status.success(), "DXC (SPIR-V) failed for {name}");
        }
    } else {
        assert!(
            windows,
            "{} missing: run sh tools/setup.sh",
            dxc_spirv.display()
        );
        println!(
            "cargo:warning=no SPIR-V DXC ({}): the Vulkan renderer is left out",
            dxc_spirv.display()
        );
    }
    // Intel Open Image Denoise (src/oidn.rs loads it at run time): its shared libraries go next to
    // the exe like the upscaler runtimes. Missing, the build goes on and the feature stays off.
    // A check-only build (tools/check_windows.sh) links nothing, so it copies nothing.
    if check_only.is_none() {
        oidn_runtime(
            out.ancestors().nth(3).expect("Cargo profile directory"),
            windows,
        );
    }
    // cfg(upscalers): bridge/vk.cpp is linked (NGX and XeSS on Vulkan). Always on Windows; on
    // Linux when tools/setup.sh fetched the DLSS SDK (engine-notes "Linux DLSS").
    println!("cargo::rustc-check-cfg=cfg(upscalers)");
    // Linux builds the Vulkan backend only (engine-notes "Vulkan backend: design", phase 2b):
    // no DXIL, no D3D12 upscaler bridges, no Windows resources.
    if !windows {
        linux_upscalers(&out);
        return;
    }
    println!("cargo:rustc-cfg=upscalers");
    if check_only.is_some() {
        check_only_dxil(&out, &dxc, check_dxil, entries, &ser_bits, shader_debug);
        return;
    }
    assert!(
        env::var("TARGET").unwrap().contains("windows-msvc"),
        "Caustica requires 64-bit Windows and the MSVC Rust toolchain"
    );
    assert!(
        PathBuf::from("vendor/dlss/include/nvsdk_ngx.h").exists(),
        "Run powershell -File tools/setup.ps1 to fetch the pinned NVIDIA SDK"
    );
    resources(&version);
    for e in entries {
        let status = dxil_command(&dxc, e, &ser_bits, shader_debug, &out)
            .status()
            .expect("DXC missing: install Windows SDK 26100 or set DXC_PATH");
        assert!(status.success(), "DXC failed for {}", e.1);
    }
    for (dir, what) in [
        ("vendor/fsr/Kits/FidelityFX/api/include", "AMD FSR SDK"),
        ("vendor/xess/inc", "Intel XeSS SDK"),
    ] {
        assert!(
            PathBuf::from(dir).exists(),
            "{what} missing: run powershell -File tools/setup.ps1"
        );
    }
    // Optional: ffmpeg.exe (tools/setup.ps1) next to the exe for MP4 turntables.
    println!("cargo:rerun-if-changed=vendor/ffmpeg/ffmpeg.exe");
    let ffmpeg = PathBuf::from("vendor/ffmpeg/ffmpeg.exe");
    if ffmpeg.exists() {
        let exe_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap())
            .ancestors()
            .nth(3)
            .map(PathBuf::from)
            .unwrap();
        let target = exe_dir.join("ffmpeg.exe");
        let stale = std::fs::metadata(&target).and_then(|m| m.modified()).ok()
            < std::fs::metadata(&ffmpeg).and_then(|m| m.modified()).ok();
        if stale {
            std::fs::copy(&ffmpeg, &target).expect("copy ffmpeg.exe");
        }
    }
    println!("cargo:rerun-if-changed=bridge");
    cc::Build::new()
        .cpp(true)
        .file("bridge/ngx.cpp")
        .file("bridge/fsr.cpp")
        .file("bridge/xess.cpp")
        .file("bridge/vk.cpp")
        .include("vendor/dlss/include")
        .include("vendor/vulkan-headers/include")
        .include("vendor/fsr/Kits/FidelityFX/api/include")
        .include("vendor/fsr/Kits/FidelityFX/upscalers/include")
        .include("vendor/xess/inc")
        .flag("/std:c++17")
        .flag("/EHsc")
        .compile("caustica_ngx");
    println!(
        "cargo:rustc-link-search=native={}",
        std::fs::canonicalize("vendor/dlss/lib/Windows_x86_64/x64")
            .unwrap()
            .display()
    );
    println!("cargo:rustc-link-lib=static=nvsdk_ngx_d");
    println!("cargo:rustc-link-lib=d3d12");
    println!("cargo:rustc-link-lib=dxgi");
    println!("cargo:rustc-link-lib=shlwapi");
    let destination = out.ancestors().nth(3).expect("Cargo profile directory");
    // Upscaler runtimes are loaded from the executable's folder.
    for source in [
        "vendor/dlss/lib/Windows_x86_64/rel/nvngx_dlss.dll",
        "vendor/dlss/lib/Windows_x86_64/rel/nvngx_dlssd.dll",
        "vendor/fsr/Kits/FidelityFX/signedbin/amd_fidelityfx_loader_dx12.dll",
        "vendor/fsr/Kits/FidelityFX/signedbin/amd_fidelityfx_upscaler_dx12.dll",
        "vendor/xess/bin/libxess.dll",
    ] {
        let source = PathBuf::from(source);
        let dll = source.file_name().unwrap();
        println!("cargo:rerun-if-changed={}", source.display());
        let dest = destination.join(dll);
        let bytes = std::fs::read(&source).expect("Read NVIDIA runtime");
        if std::fs::read(&dest).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(dest, bytes).expect(
                "Copy NVIDIA runtime next to executable; close any running app using an older DLL",
            );
        }
    }
    // Track 20: the D3D12 Agility SDK runtime (Shader Model 6.9, SER) in D3D12\ beside the
    // exe, found through the D3D12SDKVersion / D3D12SDKPath exports (src/main.rs).
    std::fs::create_dir_all(destination.join("D3D12")).expect("Create the D3D12 folder");
    for dll in ["D3D12Core.dll", "d3d12SDKLayers.dll"] {
        let source = PathBuf::from("vendor/agility").join(dll);
        println!("cargo:rerun-if-changed={}", source.display());
        let dest = destination.join("D3D12").join(dll);
        let bytes = std::fs::read(&source).expect("Read the Agility SDK (tools/setup.ps1)");
        if std::fs::read(&dest).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(dest, bytes)
                .expect("Copy the Agility SDK; close any running app using an older DLL");
        }
    }
    println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKVersion,DATA");
    println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKPath,DATA");
}

type Entry<'a> = (&'a str, &'a str, &'a str, &'a str, &'a [&'a str]);

/// The DXC command line for one DXIL entry. tools/check_dxil.py parses the flag lists below, so
/// keep each as one `.args([...])` / array literal of plain strings.
fn dxil_command(
    dxc: &std::path::Path,
    &(file, entry, profile, name, defines): &Entry,
    ser_bits: &[String],
    shader_debug: &[&str],
    out: &std::path::Path,
) -> Command {
    // A library (lib_*) exports its entry; its compute kernels' numthreads are ignored.
    let entry_args = if profile.starts_with("lib_") {
        ["-exports", entry, "-Wno-misplaced-attributes"]
    } else {
        ["-E", entry, "-Ges"]
    };
    let mut c = Command::new(dxc);
    c.args(["-T", profile])
        .args(entry_args)
        .args(["-HV", "2021", "-O3", "-I", "shaders"])
        .args(defines)
        .args(ser_bits.iter().filter(|_| profile.starts_with("lib_")))
        .args(shader_debug)
        .arg("-Fo")
        .arg(out.join(format!("{name}.dxil")))
        .arg(format!("shaders/{file}"));
    c
}

/// CAUSTICA_CHECK_ONLY on a Windows target: the .dxil files the crate embeds, either empty
/// stubs or (`real`) compiled by the host's DXC, several at a time.
fn check_only_dxil(
    out: &std::path::Path,
    dxc: &std::path::Path,
    real: bool,
    entries: &[Entry],
    ser_bits: &[String],
    shader_debug: &[&str],
) {
    if !real {
        for e in entries {
            std::fs::write(out.join(format!("{}.dxil", e.3)), []).expect("write a DXIL stub");
        }
        return;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(e) = entries.get(i) else { break };
                    let status = dxil_command(dxc, e, ser_bits, shader_debug, out)
                        .status()
                        .expect("run the Linux DXC (vendor/dxc-linux, sh tools/setup.sh)");
                    assert!(status.success(), "DXC failed for {}", e.3);
                }
            });
        }
    });
}

/// The exe's icon (assets/caustica.ico, drawn by tools/make_icon.py) and version info, compiled
/// by the Windows SDK's rc.exe (`RC_PATH` overrides) and linked into the binary. Without
/// rc.exe the build goes on with a warning and a plain exe.
fn resources(version: &str) {
    println!("cargo:rerun-if-changed=assets/caustica.ico");
    println!("cargo:rerun-if-env-changed=RC_PATH");
    let rc = env::var_os("RC_PATH").map(PathBuf::from).or_else(|| {
        let root = PathBuf::from(
            env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".into()),
        )
        .join("Windows Kits/10/bin");
        let mut found: Vec<_> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("x64/rc.exe"))
            .filter(|p| p.exists())
            .collect();
        found.sort();
        found.pop()
    });
    let Some(rc) = rc else {
        println!("cargo:warning=rc.exe not found: the exe gets no icon (set RC_PATH)");
        return;
    };
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/caustica.ico");
    // VERSIONINFO wants four numbers: major, minor, patch, 0 (a -dev suffix is dropped).
    let mut n = version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse::<u16>().ok());
    let v = [
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
    ];
    let numbers = format!("{},{},{},0", v[0], v[1], v[2]);
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Kaan"
      VALUE "FileDescription", "Caustica path tracer"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "caustica"
      VALUE "OriginalFilename", "caustica.exe"
      VALUE "ProductName", "Caustica"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "\\\\"),
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let (rc_file, res) = (out.join("caustica.rc"), out.join("caustica.res"));
    std::fs::write(&rc_file, script).unwrap();
    let status = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&res)
        .arg(&rc_file)
        .status()
        .unwrap_or_else(|e| panic!("{}: {e}", rc.display()));
    assert!(status.success(), "rc.exe failed on {}", rc_file.display());
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

/// Linux: bridge/vk.cpp with the static NGX library and the DLSS / DLSS-D runtimes next to the
/// exe (NGX looks there first). XeSS compiles in but finds no libxess.so: Intel ships none.
/// Without the SDK (`sh tools/setup.sh` fetches it) the build has no upscalers.
fn linux_upscalers(out: &std::path::Path) {
    let lib = PathBuf::from("vendor/dlss/lib/Linux_x86_64");
    let needed = [
        lib.join("libnvsdk_ngx.a"),
        PathBuf::from("vendor/dlss/include/nvsdk_ngx_vk.h"),
        PathBuf::from("vendor/xess/inc/xess/xess_vk.h"),
        PathBuf::from("vendor/vulkan-headers/include/vulkan/vulkan.h"),
    ];
    for path in &needed {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if let Some(missing) = needed.iter().find(|p| !p.exists()) {
        println!(
            "cargo:warning=no {} (sh tools/setup.sh): DLSS is left out",
            missing.display()
        );
        return;
    }
    println!("cargo:rustc-cfg=upscalers");
    println!("cargo:rerun-if-changed=bridge/vk.cpp");
    cc::Build::new()
        .cpp(true)
        .file("bridge/vk.cpp")
        .include("vendor/dlss/include")
        .include("vendor/vulkan-headers/include")
        .include("vendor/xess/inc")
        .flag("-std=c++17")
        // The NGX headers trip these; the bridge itself is clean.
        .flag("-Wno-unused-parameter")
        .flag("-Wno-missing-field-initializers")
        .compile("caustica_vk");
    println!(
        "cargo:rustc-link-search=native={}",
        std::fs::canonicalize(&lib).unwrap().display()
    );
    println!("cargo:rustc-link-lib=static=nvsdk_ngx");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=dl");
    let destination = out.ancestors().nth(3).expect("Cargo profile directory");
    let rel = lib.join("rel");
    for entry in std::fs::read_dir(&rel).expect("vendor/dlss/lib/Linux_x86_64/rel") {
        let source = entry.unwrap().path();
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        // RR (dlssd) and SR (dlss); not frame generation (dlssg).
        if !(name.starts_with("libnvidia-ngx-dlss.so")
            || name.starts_with("libnvidia-ngx-dlssd.so"))
        {
            continue;
        }
        println!("cargo:rerun-if-changed={}", source.display());
        let dest = destination.join(&name);
        let fresh = std::fs::metadata(&dest).map(|m| m.len()).ok()
            == std::fs::metadata(&source).map(|m| m.len()).ok();
        if !fresh {
            std::fs::copy(&source, &dest).expect("copy the DLSS runtime next to the exe");
        }
    }
}

/// Copies the OIDN CPU device's libraries (vendor/oidn, tools/setup.ps1 / setup.sh) beside the exe.
/// Windows: the four DLLs. Linux: the versioned `.so` files, not the symlinks (the loader opens
/// them by file name). A copy that is already the same size is left alone: a running app locks
/// its DLLs.
fn oidn_runtime(destination: &std::path::Path, windows: bool) {
    let dir = PathBuf::from("vendor/oidn").join(if windows { "windows/bin" } else { "linux/lib" });
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!(
            "cargo:warning=no OIDN in {} (tools/setup): the still denoiser is left out",
            dir.display()
        );
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let is_link = std::fs::symlink_metadata(&source).is_ok_and(|m| m.file_type().is_symlink());
        if is_link || !source.is_file() {
            continue;
        }
        let name = source.file_name().unwrap();
        let dest = destination.join(name);
        let same = std::fs::metadata(&dest).map(|m| m.len()).ok()
            == std::fs::metadata(&source).map(|m| m.len()).ok();
        if !same {
            std::fs::copy(&source, &dest).expect("copy OIDN next to the exe");
        }
    }
}
