// The shader entries, one list for build.rs (`include!`) and `hotreload::ENTRIES`: (file, entry,
// profile, output name, defines). The path tracer is built twice: scenes without texture maps use
// the variant without map code (core track 5; the map code alone cost untextured scenes about 7%).
// tools/check_dxil.py, tools/spirv_check.py and tools/lane_overlap.py parse this file.
pub const ENTRIES: [(&str, &str, &str, &str, &[&str]); 37] = [
    ("pathtrace.hlsl", "trace", "cs_6_5", "trace", &[]),
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_maps",
        &["-D", "TEXTURE_MAPS"],
    ),
    // The path visualizer's recording trace (track 14), with the map code.
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_paths",
        &["-D", "TEXTURE_MAPS", "-D", "PATH_RECORD"],
    ),
    // Light path expressions (track 14), with the map code; used while any slot is set.
    (
        "pathtrace.hlsl",
        "trace",
        "cs_6_5",
        "trace_lpe",
        &["-D", "TEXTURE_MAPS", "-D", "LPE"],
    ),
    // ReSTIR DI (track 15): light tiles, initial / temporal, spatial.
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
    // Track 15: BDPT and photon mapping (integrators.hlsl includes pathtrace.hlsl).
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
