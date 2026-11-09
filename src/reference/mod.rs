//! CPU reference renderer (future goal: ground-truth screenshots). `--reference SPP --capture
//! out.exr` renders the scene on the CPU, with no GPU at all, into a linear EXR to compare with
//! the GPU's raw accumulation (`tools/reference_compare.py`).
//!
//! It is an independent implementation of the same model: the same triangles
//! (`geometry::geometry`), material records (`lighting::materials`), camera (`Settings::camera`),
//! pixel filters, BSDF (`eval_bsdf`'s isotropic core with the same energy compensation table,
//! parsed from layers.hlsli), dielectric branch and visibility rules (lights hidden from the
//! camera, shadows that skip clear glass and lights that cast none), but its own BVH, its own
//! light sampling (emissive triangles by power), its own random numbers (PCG per pixel and
//! sample) and its own MIS. So a disagreement points at the GPU's sampling, sample sequences,
//! acceleration structures or shader arithmetic, not at a different model. Scenes it cannot
//! render (`unsupported`) are refused with a list of what is missing, except by the app's software
//! view (`Live`, `--cpu`), which renders what it can and shows that list.
use crate::scene::Settings;
use anyhow::{Context, Result};

mod bsdf;
mod bvh;
mod camera;
mod integrators;
mod lights;
mod media;
mod prepare;
mod progressive;
mod sampling;
mod scene;
#[cfg(test)]
mod tests;

pub use prepare::{at_time, prepare};
pub use progressive::{Live, LiveState, Progressive, render};

/// What a scene uses that the reference does not render; empty when it can.
pub fn unsupported(s: &Settings) -> Vec<String> {
    let mut out = vec![];
    let mut need = |c: bool, what: &str| {
        if c {
            out.push(what.to_string());
        }
    };
    need(!s.hdri.is_empty(), "an HDRI environment");
    need(s.physical_sky.on, "the physical sky");
    need(s.fog.density > 0., "fog");
    need(s.shadow_catcher, "the shadow catcher");
    need(s.spectral, "spectral rendering");
    // At the last bounce MIS keeps only its share of a light sample, the BSDF sample that would
    // make up the rest not being traced; ReSTIR's first hit keeps all of it. With a bounce to
    // spare the two agree.
    need(
        s.restir && s.mis_mode == 0 && s.bounces == 1,
        "ReSTIR at one bounce (set restir=false or more bounces)",
    );
    need(s.battle, "battle mode (four integrators at once)");
    need(s.firefly > 0., "the firefly clamp (biased; set firefly=0)");
    need(
        s.integrator != 0,
        "BDPT and photon mapping (the reference is a path tracer)",
    );
    need(s.aperture > 0., "depth of field");
    need(s.lens_system != 0, "a real lens system");
    need(
        s.objects.iter().any(|o| o.visible && !o.sdf.is_none()),
        "procedural (SDF) objects",
    );
    need(
        s.lights.iter().any(|l| {
            l.enabled
                && matches!(
                    l.kind,
                    crate::scene::LightKind::Sphere | crate::scene::LightKind::Sun
                )
        }),
        "sphere and sun lights",
    );
    // What `Gpu::attach_maps` binds: a texture set wins over the mesh's own maps (`mesh::maps`:
    // the glTF's or `maps_from`, when the material takes them), and the walls and the floor have
    // sets too. Alpha cutouts and emissive maps come with these; their flag bits are set there,
    // never in `lighting::materials`, so they can't be read from the records below.
    need(
        s.objects.iter().any(|o| {
            o.visible && (!o.material.texture.is_empty() || crate::mesh::maps(o).is_some())
        }) || !s.floor.texture.is_empty()
            || s.cornell_room && !s.walls.texture.is_empty(),
        "texture maps (with their alpha cutouts and emissive maps)",
    );
    for (what, bit) in [
        ("portals", 4u32),
        ("beam splitters", 4096),
        ("IES profiles", 16384),
    ] {
        need(
            crate::lighting::materials(s)
                .iter()
                .any(|m| m.lobes[3] as u32 & bit != 0),
            what,
        );
    }
    // An IES profile fills these fields of its light's record (`ies::pack`): not a coat or a sheen.
    let mats: Vec<_> = crate::lighting::materials(s)
        .into_iter()
        .filter(|m| m.lobes[3] as u32 & 16384 == 0)
        .collect();
    need(mats.iter().any(|m| m.layers[0] > 0.), "clear coats");
    need(mats.iter().any(|m| m.layers[3] > 0.), "anisotropy");
    need(
        mats.iter().any(|m| m.sheen[..3].iter().any(|c| *c > 0.)),
        "sheen",
    );
    need(mats.iter().any(|m| m.surface_scale[2] > 0.), "thin films");
    need(
        mats.iter().any(|m| m.optical[3] < 0.),
        "diffraction gratings",
    );
    need(
        mats.iter().any(|m| m.medium[3] > 0.),
        "participating media in objects",
    );
    // Measured spectra and blackbodies render as their RGB colour outside spectral mode, as on
    // the GPU.
    out
}

/// `--reference SPP`: renders the scene as it stands at time `t` (`prepare`) and writes a linear
/// EXR (or a PNG of the clamped values).
pub fn run(s: &Settings, t: f32, size: [u32; 2], spp: u32, path: &std::path::Path) -> Result<()> {
    let start = std::time::Instant::now();
    let rgba = render(&prepare(s, t), size, spp, &|f| {
        println!("REFERENCE_PROGRESS {:.0}%", f * 100.)
    })?;
    let rgb: Vec<f32> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let image = image::Rgb32FImage::from_raw(size[0], size[1], rgb).context("image size")?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exr"))
    {
        image.save(path)?;
    } else {
        image::DynamicImage::ImageRgb32F(image)
            .to_rgb8()
            .save(path)?;
    }
    println!(
        "REFERENCE_OK spp={spp} size={}x{} seconds={:.1}",
        size[0],
        size[1],
        start.elapsed().as_secs_f32()
    );
    Ok(())
}
