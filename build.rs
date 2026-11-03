use std::{env, path::PathBuf, process::Command};
fn main() {
    assert!(
        env::var("TARGET").unwrap().contains("windows-msvc"),
        "Caustica requires 64-bit Windows and the MSVC Rust toolchain"
    );
    assert!(
        PathBuf::from("vendor/dlss/include/nvsdk_ngx.h").exists(),
        "Run powershell -File tools/setup.ps1 to fetch the pinned NVIDIA SDK"
    );
    println!("cargo:rerun-if-changed=shaders");
    println!("cargo:rerun-if-changed=bridge/ngx.cpp");
    println!("cargo:rerun-if-env-changed=DXC_PATH");
    // The release version: CI sets CAUSTICA_VERSION (the tag it is about to create, without
    // the v); local builds are the Cargo.toml version marked -dev.
    println!("cargo:rerun-if-env-changed=CAUSTICA_VERSION");
    let version = env::var("CAUSTICA_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}-dev", env::var("CARGO_PKG_VERSION").unwrap()));
    println!("cargo:rustc-env=CAUSTICA_VERSION={version}");
    resources(&version);
    let dxc = env::var_os("DXC_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
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
    for (file, entry, profile, name, defines) in [
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
    ] {
        // A library (lib_*) exports its entry; its compute kernels' numthreads are ignored.
        let entry_args = if profile.starts_with("lib_") {
            ["-exports", entry, "-Wno-misplaced-attributes"]
        } else {
            ["-E", entry, "-Ges"]
        };
        let status = Command::new(&dxc)
            .args(["-T", profile])
            .args(entry_args)
            .args(["-HV", "2021", "-O3", "-I", "shaders"])
            .args(defines)
            .args(ser_bits.iter().filter(|_| profile.starts_with("lib_")))
            .args(shader_debug)
            .arg("-Fo")
            .arg(out.join(format!("{name}.dxil")))
            .arg(format!("shaders/{file}"))
            .status()
            .expect("DXC missing: install Windows SDK 26100 or set DXC_PATH");
        assert!(status.success(), "DXC failed for {entry}");
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
        .include("vendor/dlss/include")
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
