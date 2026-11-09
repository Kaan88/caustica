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
mod denoise;
mod display;
mod environment;
mod integrators;
mod lights;
mod media;
mod prepare;
mod progressive;
mod sampling;
mod scene;
mod sdf;
#[cfg(test)]
mod tests;
mod textures;

pub use display::display;
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
    // `Fog::equiangular` only changes the GPU's sampling; the CPU ignores it.
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
    // Measured spectra and blackbodies render as their RGB colour outside spectral mode, as on
    // the GPU.
    out
}

/// `--reference SPP`: renders `moments` (one scene, `prepare`d, or a motion-blurred frame's,
/// `exposure`) and writes a linear EXR, or a PNG or JPEG through the display transform
/// (`display`), as the GPU's capture does. `denoise` (`--cpu-denoise`) filters the mean first
/// (`denoise`); the oracle never asks for it. `lenient` renders the first moment alone.
pub fn run(
    moments: &[Settings],
    size: [u32; 2],
    spp: u32,
    path: &std::path::Path,
    denoise: bool,
    lenient: bool,
) -> Result<()> {
    let start = std::time::Instant::now();
    let format = crate::video::ImageFormat::from_path(path)
        .filter(|f| {
            matches!(
                f,
                crate::video::ImageFormat::Exr
                    | crate::video::ImageFormat::Png
                    | crate::video::ImageFormat::Jpeg
            )
        })
        .context("--reference writes .exr, .png or .jpg")?;
    anyhow::ensure!(!moments.is_empty(), "No scene to render");
    let mut moments = moments.to_vec();
    let progress = |f: f32| println!("REFERENCE_PROGRESS {:.0}%", f * 100.);
    // Lenient (thumbnails): what it knows of the scene, as the software view shows it, with what
    // it left out printed (glare too, which the CPU display transform lacks); never for measuring.
    if lenient {
        let frame = &mut moments[0];
        let mut missing = unsupported(frame);
        if format != crate::video::ImageFormat::Exr && frame.glare > 0. {
            frame.glare = 0.;
            missing.push("glare".into());
        }
        if !missing.is_empty() {
            println!("REFERENCE_MISSING {}", missing.join(", "));
        }
    }
    let frame = &moments[0];
    let mut p = if lenient {
        Progressive::partial(frame, size)
    } else {
        Progressive::over(&moments, size)?
    };
    if denoise {
        p.keep_guides();
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    p.pass(
        spp,
        threads,
        &std::sync::atomic::AtomicBool::new(false),
        &progress,
    );
    let rgba = if denoise {
        p.denoised(frame.denoise_strength, threads)
    } else {
        p.image()
    };
    crate::video::write_cpu_image(
        frame,
        path,
        format,
        crate::video::JPEG_QUALITY,
        rgba,
        size,
        0,
    )?;
    println!(
        "REFERENCE_OK spp={spp} size={}x{} seconds={:.1}",
        size[0],
        size[1],
        start.elapsed().as_secs_f32()
    );
    Ok(())
}
