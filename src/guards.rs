//! Guard rails (roadmap Plan step 4): tests that fail when two things that must stay in sync
//! drift apart. They read the sources (`include_str!`) so they need no GPU.

/// `Constants` (render.rs) against `cbuffer Frame` (common.hlsli): same members in the same
/// order, same types and sizes, so every offset matches. Two members carry different names.
#[test]
fn constants_match_cbuffer_frame() {
    const RENAMED: &[(&str, &str)] = &[
        ("display", "display_settings"),
        ("filter", "filter_settings"),
    ];
    let hlsl = include_str!("../shaders/common.hlsli");
    let body = block(hlsl, "cbuffer Frame:register(b0) {", "};");
    let hlsl_fields: Vec<(String, &str, usize)> = body
        .lines()
        .map(|l| l.split("//").next().unwrap().trim())
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (ty, name) = l.trim_end_matches(';').split_once(' ').expect(l);
            let (scalar, n) = match ty {
                "float4x4" => ("f32", 16),
                "float4" => ("f32", 4),
                "uint4" => ("u32", 4),
                "int4" => ("i32", 4),
                // Anything narrower than 16 bytes follows HLSL's packing rules, which this
                // test doesn't model: extend it before adding one.
                other => panic!("cbuffer Frame member `{l}`: type {other} is not handled here"),
            };
            (name.trim().to_string(), scalar, n)
        })
        .collect();
    let rust = include_str!("render.rs");
    let rust_body = block(rust, "pub struct Constants {", "\n}");
    let rust_fields: Vec<(String, &str, usize)> = rust_body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub "))
        .map(|l| {
            let (name, ty) = l["pub ".len()..]
                .trim_end_matches(',')
                .split_once(": ")
                .expect(l);
            let (scalar, n) = ty
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split_once("; ")
                .expect(l);
            let name = RENAMED
                .iter()
                .find(|(r, _)| *r == name)
                .map_or(name, |(_, h)| *h);
            (name.to_string(), scalar, n.parse().expect(l))
        })
        .collect();
    assert_eq!(
        rust_fields.iter().map(|f| f.2 * 4).sum::<usize>(),
        std::mem::size_of::<crate::render::Constants>(),
        "the parse of Constants missed a field"
    );
    for (i, (r, h)) in rust_fields.iter().zip(&hlsl_fields).enumerate() {
        assert_eq!(
            r, h,
            "member {i}: Constants has {r:?}, cbuffer Frame has {h:?}"
        );
    }
    assert_eq!(
        rust_fields.len(),
        hlsl_fields.len(),
        "Constants and cbuffer Frame have different member counts"
    );
    // Spot-check the parse against the compiler's layout.
    let offset = |name: &str| -> usize {
        let i = rust_fields.iter().position(|f| f.0 == name).unwrap();
        rust_fields[..i].iter().map(|f| f.2 * 4).sum()
    };
    assert_eq!(
        offset("dimensions"),
        std::mem::offset_of!(crate::render::Constants, dimensions)
    );
    assert_eq!(
        offset("meter_ctrl"),
        std::mem::offset_of!(crate::render::Constants, meter_ctrl)
    );
}

/// gui.hlsl declares only the head of `cbuffer Frame` and reads `dimensions` from it.
#[test]
fn gui_cbuffer_reaches_dimensions() {
    let gui = include_str!("../shaders/gui.hlsl");
    let head = block(gui, "cbuffer Frame:register(b0){", "};");
    let mut offset = 0;
    for member in head.split(';').map(str::trim).filter(|m| !m.is_empty()) {
        let (ty, name) = member.split_once(' ').expect(member);
        let (name, count) = match name.split_once('[') {
            Some((n, c)) => (n, c.trim_end_matches(']').parse::<usize>().expect(member)),
            None => (name, 1),
        };
        if name == "dimensions" {
            assert_eq!(ty, "uint4");
            assert_eq!(
                offset,
                std::mem::offset_of!(crate::render::Constants, dimensions)
            );
            return;
        }
        offset += count
            * match ty {
                "float4x4" => 64,
                "float4" | "uint4" => 16,
                other => panic!("gui.hlsl cbuffer member {member}: type {other} not handled"),
            };
    }
    panic!("gui.hlsl's cbuffer Frame has no dimensions");
}

/// `hotreload::Shaders` has one field per shader entry, in `ENTRIES` order (build.rs writes the
/// files under the same names and `HotReload` fills the struct in that order). The UI shaders'
/// fields drop the `_main`.
#[test]
fn shaders_struct_follows_entries() {
    const RENAMED: &[(&str, &str)] = &[("vs", "vs_main"), ("ps", "ps_main")];
    let src = include_str!("hotreload.rs");
    let fields: Vec<&str> = block(src, "pub struct Shaders {", "\n}")
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("pub "))
        .map(|l| l.split(':').next().unwrap())
        .map(|f| RENAMED.iter().find(|(r, _)| *r == f).map_or(f, |(_, e)| *e))
        .collect();
    let names: Vec<&str> = crate::shader_entries::ENTRIES.iter().map(|e| e.3).collect();
    assert_eq!(fields, names);
}

/// Every numeric `Settings` field is checked in `Settings::validate` (as `self.<field>`) (a scene file or `--set`
/// can carry any number). Bools, enums and nested types bound themselves or have their own
/// `validate`.
#[test]
fn numeric_settings_are_validated() {
    let src = include_str!("scene.rs");
    let validate = block(
        src,
        "    pub fn validate(&mut self) -> anyhow::Result<()> {",
        "\n    }\n",
    );
    let mut missing = vec![];
    for (name, ty) in settings_fields() {
        let numeric = [
            "f32",
            "f64",
            "u8",
            "u16",
            "u32",
            "u64",
            "i32",
            "usize",
            "[f32",
            "[u32",
            "Option<f32>",
            "Vec<f32>",
        ]
        .iter()
        .any(|t| ty.starts_with(t));
        let used = validate
            .match_indices(&format!("self.{name}"))
            .any(|(i, m)| {
                !validate[i + m.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
            });
        if numeric && !used {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "Settings fields with no bound in validate(): {missing:?}"
    );
}

/// Settings fields that change the rendered image, so `transport_key` keeps them (a change
/// restarts accumulation). With `TRANSPORT_EXCLUDED` (main.rs) and the fields `transport_key`
/// handles itself, every field is classed once: a new field fails this test until it is added
/// to one list or the other.
const TRANSPORT_KEYED: &[&str] = &[
    "flashlight",
    "flashlight_config",
    "cornell_room",
    "measured_room",
    "floor_extent",
    "sky",
    "left",
    "right",
    "base",
    "background",
    "walls",
    "floor",
    "light_on",
    "light_visible",
    "light_intensity",
    "light_size",
    "light_aspect",
    "light_position",
    "light_temperature",
    "light_tint",
    "glass_shadow",
    "exact_glass",
    "gentle_roulette",
    "lod0_textures",
    "bounces",
    "specular_bounces",
    "spp",
    "max_samples",
    "scale",
    "firefly",
    "truth",
    "sobol",
    "rr_preset",
    "upscaler",
    "dlss_preset",
    "native_aa",
    "pixel_filter",
    "filter_width",
    "anisotropy",
    "yaw",
    "pitch",
    "distance",
    "target",
    "fov",
    "aperture",
    "focus_distance",
    "aperture_blades",
    "blade_rotation",
    "cat_eye",
    "projection",
    "tilt",
    "lens_system",
    "f_number",
    "lens_scale",
    "dispersion",
    "mis_mode",
    "restir",
    "restir_candidates",
    "restir_neighbors",
    "restir_history",
    "integrator",
    "spectral",
    "photons",
    "photon_radius",
    "battle",
    "walk_avatar",
    "lpe",
    "hdri",
    "hdri_strength",
    "hdri_rotation",
    "hdri_visible",
    "hdri_sun",
    "hdri_ground",
    "shadow_catcher",
    "physical_sky",
    "fog",
];
/// Handled inside `transport_key` (hashed, or replaced by `rr`), or never serialized.
const TRANSPORT_SPECIAL: &[&str] = &[
    "objects",
    "lights",
    "denoiser",
    "legacy_rr",
    "legacy_denoise",
];

#[test]
fn settings_fields_are_classed_for_transport() {
    let excluded = crate::TRANSPORT_EXCLUDED;
    let fields: Vec<&str> = settings_fields().into_iter().map(|f| f.0).collect();
    let mut unclassed = vec![];
    for f in &fields {
        let lists = [TRANSPORT_KEYED, excluded, TRANSPORT_SPECIAL];
        match lists.iter().filter(|l| l.contains(f)).count() {
            0 => unclassed.push(*f),
            1 => {}
            _ => panic!("Settings::{f} is in more than one transport list"),
        }
    }
    assert!(
        unclassed.is_empty(),
        "Settings fields not classed for transport_key (add each to guards::TRANSPORT_KEYED if it \
         changes the image, else to main.rs TRANSPORT_EXCLUDED): {unclassed:?}"
    );
    let stale: Vec<_> = [TRANSPORT_KEYED, excluded, TRANSPORT_SPECIAL]
        .concat()
        .into_iter()
        .filter(|n| !fields.contains(n))
        .collect();
    assert!(
        stale.is_empty(),
        "transport lists name fields Settings no longer has: {stale:?}"
    );
}

/// `(name, type)` of every field of `scene::Settings`, from its source.
fn settings_fields() -> Vec<(&'static str, &'static str)> {
    let body = block(include_str!("scene.rs"), "pub struct Settings {", "\n}");
    body.lines()
        .filter_map(|l| {
            let l = l.strip_prefix("    ")?;
            let l = l
                .strip_prefix("pub(crate) ")
                .or_else(|| l.strip_prefix("pub "))?;
            let (name, ty) = l.split_once(": ")?;
            Some((name, ty.trim_end_matches(',')))
        })
        .collect()
}

/// The text between `start` and the first `end` after it.
fn block<'a>(src: &'a str, start: &str, end: &str) -> &'a str {
    let from = src
        .find(start)
        .unwrap_or_else(|| panic!("{start:?} not found"))
        + start.len();
    let len = src[from..]
        .find(end)
        .unwrap_or_else(|| panic!("{end:?} not found after {start:?}"));
    &src[from..from + len]
}
