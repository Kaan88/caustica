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

mod aov;
mod bsdf;
mod bvh;
mod camera;
mod denoise;
pub mod display;
mod environment;
mod integrators;
mod lights;
mod media;
mod oidn;
mod prepare;
mod progressive;
mod sampling;
mod scene;
mod sdf;
mod spectral;
#[cfg(test)]
mod tests;
mod textures;

pub use display::display;
#[cfg(test)]
pub use prepare::SHUTTER_MOMENTS;
pub use prepare::{at_time, exposure, prepare};
#[cfg(test)]
pub use progressive::render;
pub use progressive::{Live, LiveState, Progressive};

/// Samples the path tracer dropped because they were NaN or infinite (accuracy audit B4: how
/// often that shortcut fires). `run` prints the total as `dropped=`.
pub(crate) static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The truth flags that are on in `s` whose exact version is not written yet (`Truth` hooks): the
/// reference renders them as the GPU's model and says so (`REFERENCE_TRUTH_PENDING`).
pub fn truth_pending(s: &Settings) -> Vec<&'static str> {
    s.truth.pending()
}

/// What a scene uses that the reference does not render; empty when it can, which since battle
/// mode is every scene. `Fog::equiangular` only changes the GPU's sampling, and measured spectra
/// and blackbodies render as their RGB colour outside spectral mode, as on the GPU. The list
/// stays the place a new GPU feature is refused until the reference has it.
pub fn unsupported(_s: &Settings) -> Vec<String> {
    vec![]
}

/// `--reference SPP`: renders `moments` (one scene, `prepare`d, or a motion-blurred frame's,
/// `exposure`) and writes a linear EXR, or a PNG or JPEG through the display transform
/// (`display`), as the GPU's capture does. `denoise` (`--cpu-denoise`) filters the mean of a PNG or JPEG
/// first (`denoise`), never an EXR's; the oracle never asks for it. `lenient` renders the first moment alone. A
/// `name.layers.exr` holds every AOV as a layer (`aov`), as the GPU's capture does; with the light
/// path expression view (view 12) the image is the expression `lpe_slot`'s light instead of the
/// beauty (black for a slot not in use), as on the GPU.
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
                    | crate::video::ImageFormat::Layers
                    | crate::video::ImageFormat::Png
                    | crate::video::ImageFormat::Jpeg
            )
        })
        .context("--reference writes .exr, .layers.exr, .png or .jpg")?;
    let layers = format == crate::video::ImageFormat::Layers;
    // An EXR (and its layers) is the plain mean, as the software view captures it.
    let denoise = denoise
        && matches!(
            format,
            crate::video::ImageFormat::Png | crate::video::ImageFormat::Jpeg
        );
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
    if !frame.truth.on().is_empty() {
        println!("REFERENCE_TRUTH {}", frame.truth.on().join(","));
    }
    if !truth_pending(frame).is_empty() {
        println!("REFERENCE_TRUTH_PENDING {}", truth_pending(frame).join(","));
    }
    let mut p = if lenient {
        Progressive::partial(frame, size)
    } else {
        Progressive::over(&moments, size)?
    };
    if denoise {
        p.keep_guides();
    }
    let lpe_view = frame.view == 12;
    if layers || lpe_view {
        p.keep_aovs();
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    p.pass(
        spp,
        threads,
        &std::sync::atomic::AtomicBool::new(false),
        &progress,
    );
    // OIDN filters the finished mean, so the meter reads the unfiltered one, as on the GPU
    // (which meters before `video::denoise_capture` filters the written file).
    let mut exposure = None;
    let rgba = if lpe_view {
        p.lpe_image(frame.lpe_slot)
            .unwrap_or_else(|| [0., 0., 0., 1.].repeat((size[0] * size[1]) as usize))
    } else if frame.still_denoise
        && !lenient
        && matches!(
            format,
            crate::video::ImageFormat::Png | crate::video::ImageFormat::Jpeg
        )
    {
        // Intel OIDN on the finished mean, guided by albedo and normal (a PNG or JPEG only).
        exposure = Some(display::still_exposures(frame, &p.image(), size));
        p.oidn_image(threads).unwrap_or_else(|e| {
            eprintln!("OIDN skipped: {e:#}");
            p.image()
        })
    } else if denoise {
        p.denoised(frame.denoise_strength, threads)
    } else {
        p.image()
    };
    if layers {
        crate::video::write_layers(path, &p.layers(rgba))?;
    } else {
        crate::video::write_cpu_image_exposed(
            frame,
            path,
            format,
            crate::video::JPEG_QUALITY,
            rgba,
            size,
            0,
            exposure,
        )?;
    }
    println!(
        "REFERENCE_OK spp={spp} size={}x{} seconds={:.1} dropped={}",
        size[0],
        size[1],
        start.elapsed().as_secs_f32(),
        DROPPED.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}
