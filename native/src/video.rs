//! Turntable video export (roadmap 5d): the camera circles the target and every frame is
//! rendered to a fixed sample count. Frames go to ffmpeg (H.264) when it is on PATH, otherwise
//! to a numbered PNG sequence in a folder named after the video.
use crate::gpu::Gpu;
use crate::scene::Settings;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;

pub struct Turntable {
    pub frames: u32,
    pub spp: u32,
    pub degrees: f32,
    pub fps: u32,
    pub path: PathBuf,
}
/// Whether `ffmpeg` runs from PATH.
pub fn ffmpeg_available() -> bool {
    let mut c = std::process::Command::new("ffmpeg");
    c.arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.status().is_ok_and(|s| s.success())
}
/// Renders the turntable with `s` (progressive, full resolution) and returns what was written.
pub unsafe fn render(g: &mut Gpu, s: &Settings, t: &Turntable) -> Result<String> {
    let mut v = s.clone();
    v.scale = 1.;
    v.native_aa = false;
    v.vsync = false;
    v.rotate = false;
    v.compare = false;
    v.max_samples = t.spp.max(1);
    let (w, h) = (g.width, g.height);
    g.resize(w, h, &v)?;
    let mut ffmpeg = if ffmpeg_available() {
        let mut c = std::process::Command::new("ffmpeg");
        c.args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ])
        .arg(format!("{w}x{h}"))
        .args([
            "-r",
            &t.fps.to_string(),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "16",
        ])
        .arg(&t.path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null());
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        Some(c.spawn().context("Starting ffmpeg")?)
    } else {
        None
    };
    let folder = t.path.with_extension("");
    if ffmpeg.is_none() {
        std::fs::create_dir_all(&folder)?;
    }
    let start = v.yaw;
    for i in 0..t.frames {
        v.yaw = start + t.degrees * i as f32 / t.frames as f32;
        g.reset(true);
        while g.samples < v.max_samples {
            g.render(
                &v,
                true,
                1. / t.fps as f32,
                &[],
                &egui::TexturesDelta::default(),
                1.,
            )?;
        }
        let rgba = g.read_rgba8(10)?;
        match &mut ffmpeg {
            Some(child) => child
                .stdin
                .as_mut()
                .context("ffmpeg stdin")?
                .write_all(&rgba)?,
            None => image::save_buffer(
                folder.join(format!("{:05}.png", i + 1)),
                &rgba,
                w,
                h,
                image::ColorType::Rgba8,
            )?,
        }
    }
    g.resize(w, h, s)?;
    Ok(match ffmpeg {
        Some(mut child) => {
            drop(child.stdin.take());
            let status = child.wait()?;
            anyhow::ensure!(status.success(), "ffmpeg failed ({status})");
            format!("Saved {} ({} frames, {w}×{h})", t.path.display(), t.frames)
        }
        None => format!(
            "ffmpeg not found: wrote {} PNG frames to {}",
            t.frames,
            folder.display()
        ),
    })
}
