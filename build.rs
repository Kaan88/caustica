use std::{env, path::PathBuf, process::Command};
fn main() {
    assert!(
        env::var("TARGET").unwrap().contains("windows-msvc"),
        "Cornell RTX requires 64-bit Windows and the MSVC Rust toolchain"
    );
    assert!(
        PathBuf::from("vendor/dlss/include/nvsdk_ngx.h").exists(),
        "Run powershell -File tools/setup.ps1 to fetch the pinned NVIDIA SDK"
    );
    println!("cargo:rerun-if-changed=shaders");
    println!("cargo:rerun-if-changed=bridge/ngx.cpp");
    println!("cargo:rerun-if-env-changed=DXC_PATH");
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
    println!("cargo:rustc-env=CORNELL_DXC={}", dxc.display());
    println!(
        "cargo:rustc-env=CORNELL_SHADERS={}",
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("shaders")
            .display()
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
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
        ("post.hlsl", "filter", "cs_6_5", "filter", &[]),
        ("post.hlsl", "display", "cs_6_5", "display", &[]),
        ("gui.hlsl", "vs_main", "vs_6_0", "vs_main", &[]),
        ("gui.hlsl", "ps_main", "ps_6_0", "ps_main", &[]),
    ] {
        let status = Command::new(&dxc)
            .args([
                "-T", profile, "-E", entry, "-HV", "2021", "-O3", "-Ges", "-I", "shaders",
            ])
            .args(defines)
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
        .compile("cornell_ngx");
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
}
