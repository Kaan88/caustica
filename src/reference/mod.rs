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
mod environment;
mod integrators;
mod lights;
mod media;
mod prepare;
mod progressive;
mod sampling;
mod scene;
#[cfg(test)]
mod tests;

pub use prepare::{SHUTTER_MOMENTS, at_time, exposure, prepare};
pub use progressive::{Live, LiveState, Progressive, render};

/// What a scene uses that the reference does not render; empty when it can.
pub fn unsupported(s: &Settings) -> Vec<String> {
    let mut out = vec![];
    let mut need = |c: bool, what: &str| {
        if c {
            out.push(what.to_string());
        }
    };
    // The fog box renders grey and uniform; `Fog::equiangular` only changes the GPU's sampling.
    need(
        s.fog.density > 0. && s.fog.extinction != [1.; 3],
        "chromatic fog",
    );
    need(s.fog.density > 0. && s.fog.noise > 0., "heterogeneous fog");
    need(s.spectral, "spectral rendering");
    // At the last bounce MIS keeps only its share of a light sample, the BSDF sample that would
    // make up the rest not being traced; ReSTIR's first hit keeps all of it. With a bounce to
    // spare the two agree.
    need(
        s.restir && s.mis_mode == 0 && s.bounces == 1,
        "ReSTIR at one bounce (set restir=false or more bounces)",
    );
    need(s.battle, "battle mode (four integrators at once)");
    need(
        s.integrator != 0,
        "BDPT and photon mapping (the reference is a path tracer)",
    );
    // The reference writes beauty only; LPE AOV layers would be missing from its EXR.
    need(
        s.lpe.iter().any(|e| !e.is_empty()),
        "light path expressions (LPE AOVs)",
    );
    need(
        s.objects.iter().any(|o| o.visible && !o.sdf.is_none()),
        "procedural (SDF) objects",
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
    let mats = material_records(s);
    // An IES profile fills these fields of its light's record (`ies::pack`): not a coat or a sheen.
    let mats: Vec<_> = mats
        .into_iter()
        .filter(|m| m.lobes[3] as u32 & 16384 == 0)
        .collect();
    need(mats.iter().any(|m| m.layers[3] > 0.), "anisotropy");
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

/// The records of `lighting::materials` that describe surfaces: without the portal maps (flag 4
/// points at them), whose rows are a matrix, not flags or lobes.
fn material_records(s: &Settings) -> Vec<crate::lighting::PackedMaterial> {
    let all = crate::lighting::materials(s);
    let maps: std::collections::HashSet<usize> = all
        .iter()
        .filter(|m| m.lobes[3] as u32 & 4 != 0)
        .map(|m| m.secondary[3] as usize)
        .collect();
    all.into_iter()
        .enumerate()
        .filter(|(i, _)| !maps.contains(i))
        .map(|(_, m)| m)
        .collect()
}
/// `--reference SPP`: renders `moments` (one scene, `prepare`d, or a motion-blurred frame's,
/// `exposure`) and writes a linear EXR (or a PNG of the clamped values).
pub fn run(moments: &[Settings], size: [u32; 2], spp: u32, path: &std::path::Path) -> Result<()> {
    let start = std::time::Instant::now();
    let mut p = Progressive::over(moments, size)?;
    p.pass(
        spp,
        std::thread::available_parallelism().map_or(4, |n| n.get()),
        &std::sync::atomic::AtomicBool::new(false),
        &|f| println!("REFERENCE_PROGRESS {:.0}%", f * 100.),
    );
    let rgba = p.image();
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
