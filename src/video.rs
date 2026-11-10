//! Video export (roadmap 5d turntable; item 6 export dialog): the camera orbits the target and
//! each frame converges to a sample count or a time limit. A `Job` renders at its own size
//! (`Gpu::resize_output`, independent of the window) one pass per `redraw`, so the window stays
//! responsive. Finished frames go to a writer thread: ffmpeg (H.264, H.265, HDR10 HEVC, ProRes)
//! or numbered PNG / EXR files in a folder named after the video.
use crate::Gpu;
use crate::scene::Settings;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    H264,
    H265,
    /// 10-bit HEVC, BT.2020 / PQ, from the HDR output (texture 26).
    Hdr10,
    ProRes,
    Png,
    /// Linear RGB float (texture 18).
    Exr,
}
impl Format {
    pub const ALL: [Format; 6] = [
        Format::H264,
        Format::H265,
        Format::Hdr10,
        Format::ProRes,
        Format::Png,
        Format::Exr,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Format::H264 => "H.264 MP4",
            Format::H265 => "H.265 MP4",
            Format::Hdr10 => "HDR10 HEVC MP4 (10-bit PQ)",
            Format::ProRes => "ProRes MOV",
            Format::Png => "PNG sequence",
            Format::Exr => "EXR sequence (linear)",
        }
    }
    /// The file extension, or "" for a folder of numbered frames.
    pub fn extension(self) -> &'static str {
        match self {
            Format::H264 | Format::H265 | Format::Hdr10 => "mp4",
            Format::ProRes => "mov",
            Format::Png | Format::Exr => "",
        }
    }
    pub fn video(self) -> bool {
        !self.extension().is_empty()
    }
    /// The quality setting's range and default: CRF for H.264 / H.265, the ProRes profile.
    pub fn quality_range(self) -> Option<(u32, u32, u32)> {
        match self {
            Format::H264 => Some((0, 51, 16)),
            Format::H265 | Format::Hdr10 => Some((0, 51, 18)),
            Format::ProRes => Some((0, 3, 3)),
            Format::Png | Format::Exr => None,
        }
    }
}
/// Still image formats, shared by *Save the current image*, the high-resolution still and the
/// image sequences, so they write the same files.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageFormat {
    /// 8-bit sRGB, tone-mapped (texture 10).
    Png,
    /// 8-bit sRGB, tone-mapped, at the JPEG quality (1–100).
    Jpeg,
    /// Linear scene light, 32-bit float RGB (texture 18).
    Exr,
    /// What the HDR screen shows: 16-bit PQ / BT.2020 with a cICP chunk (texture 26, HDR only).
    HdrPng,
    /// What the HDR screen shows as scRGB half floats in JPEG XR, as Game Bar and the
    /// Snipping Tool save HDR screenshots (texture 26, HDR only).
    Jxr,
    /// A multi-layer EXR: the linear beauty and every AOV as its own layer (`Gpu::read_layers`).
    Layers,
}
impl ImageFormat {
    /// JPEG XR is written through WIC, so only Windows offers it.
    pub const ALL: [ImageFormat; if cfg!(windows) { 6 } else { 5 }] = [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Exr,
        ImageFormat::HdrPng,
        #[cfg(windows)]
        ImageFormat::Jxr,
        ImageFormat::Layers,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG",
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::Exr => "HDR EXR",
            ImageFormat::HdrPng => "HDR PNG",
            ImageFormat::Jxr => "HDR JPEG XR",
            ImageFormat::Layers => "EXR with AOVs",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            ImageFormat::Png => "The tone-mapped image, 8-bit sRGB, lossless.",
            ImageFormat::Jpeg => "The tone-mapped image, 8-bit sRGB, at the JPEG quality.",
            ImageFormat::Exr => "Linear scene light, 32-bit float.",
            ImageFormat::HdrPng => {
                "What the HDR screen shows, 16-bit PQ / BT.2020 (HDR output only)."
            }
            ImageFormat::Jxr => {
                "What the HDR screen shows, scRGB half floats, lossless, as Windows' HDR screenshots (HDR output only)."
            }
            ImageFormat::Layers => {
                "Multi-layer EXR: linear light, plus albedo, normal, roughness, depth, motion, direct and specular light, object ids and each light path expression as layers (at render size)."
            }
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Exr => "exr",
            ImageFormat::HdrPng => "hdr.png",
            ImageFormat::Jxr => "jxr",
            ImageFormat::Layers => "layers.exr",
        }
    }
    /// `stem` plus this format's extension (`hdr.png` keeps the PQ PNG apart from the plain one),
    /// in the output folder (`dirs::output`: the working directory unless installed).
    pub fn file(self, stem: &str) -> PathBuf {
        crate::dirs::output(format!("{stem}.{}", self.extension()))
    }
    pub fn needs_hdr(self) -> bool {
        matches!(self, ImageFormat::HdrPng | ImageFormat::Jxr)
    }
    /// The format a file name asks for, by its (case-blind) extension.
    pub fn from_path(path: &Path) -> Option<ImageFormat> {
        let name = path.to_string_lossy().to_lowercase();
        Some(match () {
            _ if name.ends_with(".layers.exr") => ImageFormat::Layers,
            _ if name.ends_with(".hdr.png") => ImageFormat::HdrPng,
            _ if name.ends_with(".exr") => ImageFormat::Exr,
            _ if name.ends_with(".png") => ImageFormat::Png,
            _ if name.ends_with(".jpg") || name.ends_with(".jpeg") => ImageFormat::Jpeg,
            _ if name.ends_with(".jxr") => ImageFormat::Jxr,
            _ => return None,
        })
    }
}
/// Whether saving twice under this name template lands on the same file: only `{time}` and
/// `{spp}` change between two saves of one view.
pub fn name_repeats(template: &str) -> bool {
    !template.contains("{time}") && !template.contains("{spp}")
}
/// Export file names from a template (wave 3; Export → *File name*): `{scene}`, `{date}`
/// (YYYY-MM-DD) and `{time}` (HH-MM-SS, local), `{spp}` (samples so far), `{w}` and `{h}` (the
/// image size). Characters Windows forbids in names become `_`; an empty result is "caustica".
pub fn expand_name(template: &str, scene: &str, samples: u32, w: u32, h: u32) -> String {
    let [year, month, day, hour, minute, second] = crate::platform::local_time();
    let out = template
        .replace("{scene}", scene)
        .replace("{date}", &format!("{:04}-{:02}-{:02}", year, month, day))
        .replace(
            "{time}",
            &format!("{:02}-{:02}-{:02}", hour, minute, second),
        )
        .replace("{spp}", &samples.to_string())
        .replace("{w}", &w.to_string())
        .replace("{h}", &h.to_string());
    let out: String = out
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let out = out.trim().trim_end_matches('.').trim();
    if out.is_empty() {
        return "caustica".into();
    }
    // Windows' device names can't be file names, with any extension.
    let base = out
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    // Long scene names stay well inside MAX_PATH.
    let out: String = out.chars().take(120).collect();
    if device { format!("_{out}") } else { out }
}
/// Reads the texture `format` is written from.
unsafe fn read_image(g: &mut Gpu, format: ImageFormat) -> Result<Frame> {
    Ok(match format {
        ImageFormat::Png | ImageFormat::Jpeg => Frame::Rgba8(g.read_rgba8(10)?),
        ImageFormat::Exr => Frame::Linear(g.read_floats(18)?),
        ImageFormat::Layers => Frame::Layers(g.read_layers()?),
        ImageFormat::HdrPng | ImageFormat::Jxr => {
            anyhow::ensure!(g.hdr_active, "{} needs HDR output", format.label());
            Frame::Scrgb(g.read_floats(26)?)
        }
    })
}
/// The camera is the 360° equirectangular model: PNG and JPEG saves carry Google's GPano XMP,
/// so panorama viewers (Google Photos, VR players, Facebook) show them as a sphere. Set from
/// the settings every frame (an export renders the same camera).
static EQUIRECT: AtomicBool = AtomicBool::new(false);
pub fn set_equirect(on: bool) {
    EQUIRECT.store(on, Ordering::Relaxed);
}
/// GPano XMP for a full-sphere equirectangular image of `w` × `h`.
fn gpano_xmp(w: u32, h: u32) -> String {
    format!(
        concat!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">",
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">",
            "<rdf:Description rdf:about=\"\" xmlns:GPano=\"http://ns.google.com/photos/1.0/panorama/\" ",
            "GPano:ProjectionType=\"equirectangular\" GPano:UsePanoramaViewer=\"True\" ",
            "GPano:FullPanoWidthPixels=\"{w}\" GPano:FullPanoHeightPixels=\"{h}\" ",
            "GPano:CroppedAreaImageWidthPixels=\"{w}\" GPano:CroppedAreaImageHeightPixels=\"{h}\" ",
            "GPano:CroppedAreaLeftPixels=\"0\" GPano:CroppedAreaTopPixels=\"0\"/>",
            "</rdf:RDF></x:xmpmeta>"
        ),
        w = w,
        h = h
    )
}
/// Adds the panorama XMP to a written PNG (an iTXt chunk after IHDR) or JPEG (an APP1
/// segment after SOI).
fn tag_equirect(path: &Path, format: ImageFormat, w: u32, h: u32) -> Result<()> {
    let xmp = gpano_xmp(w, h);
    let mut bytes = std::fs::read(path)?;
    match format {
        ImageFormat::Png => {
            let mut body = b"iTXtXML:com.adobe.xmp\0\0\0\0\0".to_vec();
            body.extend_from_slice(xmp.as_bytes());
            let mut chunk = ((body.len() - 4) as u32).to_be_bytes().to_vec();
            chunk.extend_from_slice(&body);
            chunk.extend_from_slice(&crate::display::crc32(&body).to_be_bytes());
            // 8-byte signature + 25-byte IHDR chunk.
            bytes.splice(33..33, chunk);
        }
        ImageFormat::Jpeg => {
            let mut seg = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
            seg.extend_from_slice(xmp.as_bytes());
            let mut app1 = vec![0xFF, 0xE1];
            app1.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
            app1.extend_from_slice(&seg);
            // After SOI and, when present, the JFIF APP0 segment (which strict readers want
            // first).
            let at = if bytes.get(2..4) == Some(&[0xFF, 0xE0]) {
                4 + u16::from_be_bytes([bytes[4], bytes[5]]) as usize
            } else {
                2
            };
            bytes.splice(at..at, app1);
        }
        _ => return Ok(()),
    }
    std::fs::write(path, bytes)?;
    Ok(())
}
/// Tags a PNG or JPEG written elsewhere (`Gpu::capture`: `--capture`, scripts) when the
/// camera is equirectangular. A UI capture shows more than the panorama, so it is left alone.
pub fn tag_capture(path: &Path, w: u32, h: u32) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "png" if !path.to_string_lossy().ends_with(".hdr.png") => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        _ => return Ok(()),
    };
    if EQUIRECT.load(Ordering::Relaxed) {
        tag_equirect(path, format, w, h)?;
    }
    Ok(())
}
/// Writes one image; `quality` is the JPEG quality.
fn write_image(
    path: &Path,
    format: ImageFormat,
    quality: u32,
    frame: &Frame,
    w: u32,
    h: u32,
) -> Result<()> {
    let rgb = |pixels: &[f32]| -> Vec<f32> {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect()
    };
    match (format, frame) {
        (ImageFormat::Png, Frame::Rgba8(rgba)) => image::save_buffer_with_format(
            path,
            rgba,
            w,
            h,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )?,
        (ImageFormat::Jpeg, Frame::Rgba8(rgba)) => {
            let pixels: Vec<u8> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2]])
                .collect();
            let file = std::io::BufWriter::new(std::fs::File::create(path)?);
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality.clamp(1, 100) as u8)
                .encode(&pixels, w, h, image::ExtendedColorType::Rgb8)?;
        }
        (ImageFormat::Exr, Frame::Linear(f)) => image::Rgb32FImage::from_raw(w, h, rgb(f))
            .context("EXR size")?
            .save_with_format(path, image::ImageFormat::OpenExr)?,
        (ImageFormat::HdrPng, Frame::Scrgb(f)) => crate::display::write_hdr_png(path, f, w, h)?,
        (ImageFormat::Jxr, Frame::Scrgb(f)) => crate::display::write_jxr(path, f, w, h)?,
        (ImageFormat::Layers, Frame::Layers(layers)) => write_layers(path, layers)?,
        _ => anyhow::bail!("{format:?} got the wrong pixels"),
    }
    if EQUIRECT.load(Ordering::Relaxed) {
        tag_equirect(path, format, w, h)?;
    }
    Ok(())
}
/// A multi-layer EXR, one part per layer at its own size (32-bit float, ZIP16), the beauty
/// ("beauty") first, which viewers open by default.
pub fn write_layers(path: &Path, layers: &[crate::gpu::AovLayer]) -> Result<()> {
    use exr::prelude::*;
    let parts: Vec<Layer<AnyChannels<FlatSamples>>> = layers
        .iter()
        .map(|l| {
            let n = l.channels.len();
            let channels: SmallVec<[AnyChannel<FlatSamples>; 4]> = l
                .channels
                .iter()
                .enumerate()
                .map(|(c, name)| {
                    let samples = l.data.iter().skip(c).step_by(n).copied().collect();
                    AnyChannel::new(*name, FlatSamples::F32(samples))
                })
                .collect();
            // A multi-part file needs every part named.
            let attributes = LayerAttributes::named(l.name.as_str());
            Layer::new(
                (l.width as usize, l.height as usize),
                attributes,
                Encoding::SMALL_LOSSLESS,
                AnyChannels::sort(channels),
            )
        })
        .collect();
    let first = layers.first().context("no layers")?;
    let bounds = IntegerBounds::from_dimensions((first.width as usize, first.height as usize));
    Image::from_layers(ImageAttributes::new(bounds), parts)
        .write()
        .to_file(path)
        .map_err(|e| anyhow::anyhow!("EXR: {e}"))?;
    Ok(())
}
/// Saves the image on screen (without the UI) at the output size.
pub unsafe fn save_current(
    g: &mut Gpu,
    format: ImageFormat,
    quality: u32,
    path: &Path,
) -> Result<()> {
    let frame = read_image(g, format)?;
    write_image(path, format, quality, &frame, g.width, g.height)
}
/// Writes a linear RGBA image of `w` × `h` as an EXR (the software view's plain mean).
pub fn save_linear(path: &Path, rgba: &[f32], w: u32, h: u32) -> Result<()> {
    write_image(
        path,
        ImageFormat::Exr,
        0,
        &Frame::Linear(rgba.to_vec()),
        w,
        h,
    )
}
pub const JPEG_QUALITY: u32 = 92;

pub const PRORES_PROFILES: [&str; 4] = ["Proxy", "LT", "422", "422 HQ"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Easing {
    /// Constant speed; the last frame stops one step short of the angle, so 360° loops.
    Linear,
    /// Starts and ends at rest, reaching the full angle on the last frame.
    EaseInOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u32,
    pub spp: u32,
    /// Per-frame time limit in seconds (0 = none): a frame ends at `spp` or at this.
    pub seconds_per_frame: f32,
    pub format: Format,
    /// CRF for H.264 / H.265 (lower is better), the ProRes profile (0–3).
    pub quality: u32,
    /// Orbit: degrees turned over the whole video, then the camera's pitch and distance.
    pub degrees: f32,
    pub pitch: f32,
    pub distance: f32,
    pub easing: Easing,
    /// Follow the scene's keyframes (track 8) instead of orbiting: frame i is the animation at
    /// i / fps, with motion blur when its shutter is open.
    pub timeline: bool,
    pub path: PathBuf,
    /// One image written to `path` itself (the high-resolution still) instead of a folder of
    /// numbered frames.
    pub still: bool,
    /// The still's format and the JPEG quality (sequences take theirs from `format`).
    pub image: ImageFormat,
    pub jpeg_quality: u32,
}
impl Options {
    /// The turntable of older builds: the current camera turning, H.264 at CRF 16.
    pub fn turntable(s: &Settings, size: [u32; 2], frames: u32, spp: u32, path: PathBuf) -> Self {
        Self {
            width: size[0],
            height: size[1],
            fps: 30,
            frames,
            spp,
            seconds_per_frame: 0.,
            format: Format::H264,
            quality: 16,
            degrees: 360.,
            pitch: s.pitch,
            distance: s.distance,
            easing: Easing::Linear,
            timeline: false,
            path,
            still: false,
            image: ImageFormat::Png,
            jpeg_quality: JPEG_QUALITY,
        }
    }
    /// Output size: video encoders take 4:2:x chroma, so their sizes round down to even.
    pub fn size(&self) -> [u32; 2] {
        let even = |v: u32| {
            if self.format.video() {
                (v & !1).max(2)
            } else {
                v.max(1)
            }
        };
        [even(self.width.min(8192)), even(self.height.min(8192))]
    }
    /// Yaw offset of frame `i` from the starting yaw.
    pub fn yaw(&self, i: u32) -> f32 {
        let n = self.frames.max(1) as f32;
        let t = match self.easing {
            Easing::Linear => i as f32 / n,
            Easing::EaseInOut => {
                let t = (i as f32 / (n - 1.).max(1.)).clamp(0., 1.);
                t * t * (3. - 2. * t)
            }
        };
        self.degrees * t
    }
    /// The image sequence folder (the path without its extension).
    pub fn folder(&self) -> PathBuf {
        self.path.with_extension("")
    }
    /// What the export writes: the video or still file, or the folder of frames.
    pub fn output(&self) -> PathBuf {
        if self.format.video() || self.still {
            self.path.clone()
        } else {
            self.folder()
        }
    }
    /// The image format of the still or of a sequence's frames.
    pub fn image_format(&self) -> ImageFormat {
        match self.format {
            _ if self.still => self.image,
            Format::Exr => ImageFormat::Exr,
            _ => ImageFormat::Png,
        }
    }
    /// The file of frame `number` (from 1) of an image export.
    fn frame_path(&self, number: u32) -> PathBuf {
        if self.still {
            self.path.clone()
        } else {
            let ext = self.image_format().extension();
            self.folder().join(format!("{number:05}.{ext}"))
        }
    }
}

/// ffmpeg next to the exe (copied by build.rs from tools/setup.ps1's download), else from PATH.
fn ffmpeg() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| {
            e.parent()
                .map(|d| d.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}
/// Whether ffmpeg runs (next to the exe or from PATH). Asked once per run.
pub fn ffmpeg_available() -> bool {
    static AVAILABLE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        let mut c = std::process::Command::new(ffmpeg());
        c.arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::platform::no_window(&mut c);
        c.status().is_ok_and(|s| s.success())
    });
    *AVAILABLE
}
/// ffmpeg's arguments for a video format: raw frames on stdin (RGBA8, or 16-bit PQ RGB for
/// HDR10), BT.709 (BT.2020 / PQ for HDR10) tagged, `peak` nits as the HDR10 light level.
pub fn ffmpeg_args(o: &Options, peak: f32) -> Vec<String> {
    let [w, h] = o.size();
    let hdr = o.format == Format::Hdr10;
    let mut a: Vec<String> = ["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt"]
        .map(String::from)
        .to_vec();
    a.push(if hdr { "rgb48le" } else { "rgba" }.into());
    a.extend([
        "-s".into(),
        format!("{w}x{h}"),
        "-r".into(),
        o.fps.to_string(),
        "-i".into(),
        "-".into(),
    ]);
    let (matrix, tags) = if hdr {
        ("bt2020nc", ["bt2020nc", "bt2020", "smpte2084"])
    } else {
        ("bt709", ["bt709", "bt709", "bt709"])
    };
    // The filtered frames carry their own colour properties, which win over the output tags, so
    // `setparams` sets them too.
    a.extend([
        "-vf".into(),
        format!(
            "scale=out_color_matrix={matrix}:out_range=tv,setparams=range=tv:colorspace={}:color_primaries={}:color_trc={}",
            tags[0], tags[1], tags[2]
        ),
        "-colorspace".into(),
        tags[0].into(),
        "-color_primaries".into(),
        tags[1].into(),
        "-color_trc".into(),
        tags[2].into(),
    ]);
    let q = o.quality.to_string();
    match o.format {
        Format::H264 => a.extend(
            [
                "-c:v", "libx264", "-preset", "slow", "-pix_fmt", "yuv420p", "-crf", &q,
            ]
            .map(String::from),
        ),
        Format::H265 => a.extend(
            [
                "-c:v", "libx265", "-preset", "medium", "-pix_fmt", "yuv420p", "-crf", &q,
                "-tag:v", "hvc1",
            ]
            .map(String::from),
        ),
        Format::Hdr10 => {
            let cll = peak.round().max(1.) as u32;
            a.extend(
                [
                    "-c:v",
                    "libx265",
                    "-preset",
                    "medium",
                    "-pix_fmt",
                    "yuv420p10le",
                    "-crf",
                    &q,
                    "-tag:v",
                    "hvc1",
                    "-x265-params",
                ]
                .map(String::from),
            );
            a.push(format!(
                "hdr10=1:repeat-headers=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:max-cll={cll},{}",
                cll / 2
            ));
        }
        Format::ProRes => a.extend(
            [
                "-c:v",
                "prores_ks",
                "-profile:v",
                &q,
                "-pix_fmt",
                "yuv422p10le",
                "-vendor",
                "apl0",
            ]
            .map(String::from),
        ),
        Format::Png | Format::Exr => {}
    }
    a.push(o.path.to_string_lossy().into_owned());
    a
}

enum Frame {
    Rgba8(Vec<u8>),
    /// scRGB from texture 26, converted to PQ on the writer thread.
    Scrgb(Vec<f32>),
    /// Linear RGBA from texture 18.
    Linear(Vec<f32>),
    /// Every AOV (`ImageFormat::Layers`).
    Layers(Vec<crate::gpu::AovLayer>),
}
/// The writer thread: ffmpeg's stdin or numbered files. Two frames may queue; it returns the
/// number of frames written.
struct Writer {
    tx: Option<std::sync::mpsc::SyncSender<Frame>>,
    thread: Option<std::thread::JoinHandle<Result<u32>>>,
    cancel: Arc<AtomicBool>,
}
impl Writer {
    fn start(o: &Options, peak: f32) -> Result<Writer> {
        let [w, h] = o.size();
        let mut child = if o.format.video() {
            let mut c = std::process::Command::new(ffmpeg());
            c.args(ffmpeg_args(o, peak))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            crate::platform::no_window(&mut c);
            Some(c.spawn().context("Starting ffmpeg")?)
        } else {
            match o.output().parent() {
                Some(dir) if o.still && !dir.as_os_str().is_empty() => {
                    std::fs::create_dir_all(dir)?
                }
                _ if o.still => {}
                _ => std::fs::create_dir_all(o.folder())?,
            }
            None
        };
        let (tx, rx) = std::sync::mpsc::sync_channel::<Frame>(2);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (options, path) = (o.clone(), o.path.clone());
        // ffmpeg's messages are read as they come, so a full pipe can never stall it.
        let stderr = child.as_mut().and_then(|c| c.stderr.take()).map(|mut e| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = e.read_to_string(&mut text);
                text
            })
        });
        let thread = std::thread::spawn(move || -> Result<u32> {
            let mut written = 0;
            let mut failed = None;
            for frame in rx {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let number = written + 1;
                let result = match (&mut child, frame) {
                    (Some(c), frame) => {
                        let stdin = c.stdin.as_mut().context("ffmpeg stdin")?;
                        match frame {
                            Frame::Rgba8(rgba) => stdin.write_all(&rgba),
                            Frame::Scrgb(f) => {
                                let pq = crate::display::scrgb_to_pq(&f);
                                stdin.write_all(bytemuck::cast_slice(&pq))
                            }
                            Frame::Linear(_) | Frame::Layers(_) => Ok(()),
                        }
                        .map_err(anyhow::Error::from)
                    }
                    (None, frame) => write_image(
                        &options.frame_path(number),
                        options.image_format(),
                        options.jpeg_quality,
                        &frame,
                        w,
                        h,
                    ),
                };
                if let Err(e) = result {
                    failed = Some(e);
                    break;
                }
                written = number;
            }
            if let Some(mut c) = child {
                if stop.load(Ordering::Relaxed) {
                    let _ = c.kill();
                    let _ = c.wait();
                    let _ = std::fs::remove_file(&path);
                    return Ok(written);
                }
                drop(c.stdin.take());
                let status = c.wait()?;
                let err = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
                anyhow::ensure!(status.success(), "ffmpeg failed ({status}): {}", err.trim());
            }
            failed.map_or(Ok(written), Err)
        });
        Ok(Writer {
            tx: Some(tx),
            thread: Some(thread),
            cancel,
        })
    }
    /// Queues a frame without waiting: a full queue hands it back. An error means the thread
    /// stopped (with its own error when it has one).
    fn try_send(&mut self, frame: Frame) -> Result<Option<Frame>> {
        use std::sync::mpsc::TrySendError;
        let tx = self.tx.as_ref().context("writer closed")?;
        match tx.try_send(frame) {
            Ok(()) => Ok(None),
            Err(TrySendError::Full(frame)) => Ok(Some(frame)),
            Err(TrySendError::Disconnected(_)) => Err(self
                .finish(false)
                .err()
                .unwrap_or_else(|| anyhow::anyhow!("writer stopped"))),
        }
    }
    fn finish(&mut self, cancel: bool) -> Result<u32> {
        self.cancel.store(cancel, Ordering::Relaxed);
        drop(self.tx.take());
        match self.thread.take() {
            Some(t) => t.join().map_err(|_| anyhow::anyhow!("writer panicked"))?,
            None => Ok(0),
        }
    }
}

/// A running export. `redraw` renders `settings()` and calls `after_render` after each pass.
pub struct Job {
    pub options: Options,
    settings: Settings,
    start_yaw: f32,
    /// Frames finished (queued, or waiting in `pending`).
    pub frame: u32,
    /// A finished frame the writer had no room for yet; rendering waits for it.
    pending: Option<Frame>,
    frame_started: Instant,
    pub started: Instant,
    /// The last finished frame, at most 360 pixels wide, until the UI takes it.
    pub preview: Option<egui::ColorImage>,
    /// Shown with the result, e.g. that ffmpeg was missing.
    pub note: String,
    writer: Writer,
    /// The settings before animation (timeline exports evaluate the keys on them), the GPU
    /// scene's keys, and whether the job changed that scene.
    base: Settings,
    geometry: String,
    materials: Vec<u8>,
    scene_changed: bool,
    /// The environment on the GPU (`sky::environment_key`; a keyed physical sky re-bakes it),
    /// and whether the job changed it.
    environment: String,
    environment_changed: bool,
    /// Passes rendered (motion blur's shutter samples).
    pass: u32,
    /// Timeline exports with the simulation on run their own one, frame by frame in
    /// simulated time (keyframed objects drive it as kinematic bodies), and the time it is at.
    physics: Option<Box<crate::physics::Sim>>,
    physics_time: f32,
}
impl Job {
    /// Resizes the output to the export size and starts the writer. Video formats fall back to
    /// PNG frames without ffmpeg; HDR10 needs HDR output.
    pub unsafe fn start(g: &mut Gpu, s: &Settings, mut options: Options) -> Result<Job> {
        let mut note = String::new();
        if options.format.video() && !ffmpeg_available() {
            options.format = Format::Png;
            note = "ffmpeg not found: PNG frames".into();
        }
        anyhow::ensure!(
            options.format != Format::Hdr10 || g.hdr_active,
            "HDR10 video needs HDR output"
        );
        anyhow::ensure!(
            options.format.video() || !options.image_format().needs_hdr() || g.hdr_active,
            "HDR PNG needs HDR output"
        );
        if options.timeline {
            // The keys set the frame count and rate; a simulation without keys runs for the
            // frames asked for at the timeline's rate.
            anyhow::ensure!(
                !s.animation.is_empty() || s.physics.enabled,
                "The scene has no keyframes to follow"
            );
            if !s.animation.is_empty() {
                options.fps = s.animation.fps;
                options.frames = s.animation.frames();
            }
        }
        anyhow::ensure!(options.frames > 0, "No frames to render");
        let mut v = s.clone();
        v.scale = 1.;
        // Progressive at full resolution: DLAA would restart every frame.
        v.native_aa = false;
        v.vsync = false;
        v.rotate = false;
        v.compare = false;
        v.battle = false;
        v.max_samples = options.spp.max(1);
        if !options.timeline {
            v.pitch = options.pitch.clamp(-89., 89.);
            v.distance = options.distance;
        }
        // The beauty, whatever debug view is on; light path expressions would only cost time
        // (and a buffer of the export's size).
        v.view = 0;
        v.lpe.clear();
        let [w, h] = options.size();
        // Back to the window's size when the export can't start (out of memory at 8K, ffmpeg).
        let writer = g
            .resize_output(w, h, &v)
            .and_then(|()| Writer::start(&options, g.hdr_signal()[1]));
        let writer = match writer {
            Ok(writer) => writer,
            Err(e) => {
                let [sw, sh] = g.swap_size;
                g.resize_output(sw, sh, s)?;
                return Err(e);
            }
        };
        g.reset(true);
        let mut job = Job {
            start_yaw: v.yaw,
            base: v.clone(),
            settings: v,
            options,
            frame: 0,
            pending: None,
            frame_started: Instant::now(),
            started: Instant::now(),
            preview: None,
            note,
            writer,
            geometry: crate::geometry_key(s),
            materials: crate::material_key(s),
            scene_changed: false,
            environment: crate::sky::environment_key(s),
            environment_changed: false,
            pass: 0,
            physics: None,
            physics_time: 0.,
        };
        if job.options.timeline && s.physics.enabled {
            job.physics = Some(Box::new(crate::physics::Sim::new(&job.base)));
        }
        if job.options.timeline
            && let Err(e) = job.animate(g, 0.)
        {
            let _ = job.finish(g, s, true);
            return Err(e);
        }
        Ok(job)
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// The frame's moment on the timeline.
    fn frame_time(&self, frame: u32) -> f32 {
        frame as f32 / self.options.fps.max(1) as f32
    }
    /// Motion blur: the shutter is open and something is keyed.
    fn blur(&self) -> bool {
        self.options.timeline
            && self.base.animation.shutter > 0.
            && !self.base.animation.is_empty()
            && self.physics.is_none()
    }
    /// Puts the animation at `t` into the settings and the GPU scene (new BLAS / TLAS or
    /// materials when they changed).
    unsafe fn animate(&mut self, g: &mut Gpu, t: f32) -> Result<()> {
        let yaw = self.settings.yaw;
        self.settings = self.base.animation.apply(&self.base, t);
        if !self.options.timeline {
            self.settings.yaw = yaw;
        }
        if let Some(sim) = &mut self.physics {
            // The bodies' poses are the simulation's; the keys move the kinematic ones. Time
            // only runs forward (motion-blur passes before the simulated time hold the poses).
            sim.take_poses(&mut self.settings);
            if !sim.sync(&self.settings, false) {
                **sim = crate::physics::Sim::new(&self.settings);
            }
            if t > self.physics_time {
                sim.run(&mut self.settings, t - self.physics_time);
                self.physics_time = t;
            }
        }
        self.settings.add_flashlight();
        let geometry = crate::geometry_key(&self.settings);
        let materials = crate::material_key(&self.settings);
        if geometry != self.geometry {
            g.rebuild_scene(&self.settings)?;
            self.geometry = geometry;
            self.scene_changed = true;
        } else if materials != self.materials {
            g.update_materials(&self.settings)?;
            self.scene_changed = true;
        }
        self.materials = materials;
        let environment = crate::sky::environment_key(&self.settings);
        if environment != self.environment {
            crate::sky::upload_environment(g, &self.settings)?;
            self.environment = environment;
            self.environment_changed = true;
        }
        Ok(())
    }
    /// Before each pass. With motion blur every pass is one moment inside the open shutter,
    /// centred on the frame (the keep-exposure mode: scene changes keep the accumulation, and
    /// RR, which would see only the last pass, is off).
    pub unsafe fn before_render(&mut self, g: &mut Gpu) -> Result<()> {
        if !self.blur() || self.pending.is_some() || self.frame >= self.options.frames {
            return Ok(());
        }
        g.keep_exposure = true;
        // Stratified over the passes of the frame, in a scrambled order (golden-ratio steps).
        self.pass += 1;
        let u = (self.pass as f32 * 0.618_034).fract();
        let shutter = self.base.animation.shutter / self.options.fps.max(1) as f32;
        let t = self.frame_time(self.frame) + (u - 0.5) * shutter;
        self.animate(g, t)
    }
    /// Every frame rendered and handed to the writer.
    pub fn done(&self) -> bool {
        self.frame >= self.options.frames && self.pending.is_none()
    }
    /// A finished frame waits for room in the writer's queue (the encoder is slower).
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }
    /// Frame duration for the renderer's clock.
    pub fn dt(&self) -> f32 {
        1. / self.options.fps.max(1) as f32
    }
    /// Once the current frame has its samples (or its time is up), reads it back, queues it and
    /// moves the camera to the next frame.
    pub unsafe fn after_render(&mut self, g: &mut Gpu) -> Result<()> {
        if let Some(frame) = self.pending.take() {
            self.pending = self.writer.try_send(frame)?;
            if self.pending.is_some() {
                return Ok(());
            }
        }
        let limit = self.options.seconds_per_frame;
        let timed_out = limit > 0. && self.frame_started.elapsed().as_secs_f32() >= limit;
        if self.frame >= self.options.frames
            || (g.samples < self.settings.max_samples && !(timed_out && g.samples > 0))
        {
            return Ok(());
        }
        if self.blur() {
            g.keep_exposure = false;
        }
        let rgba = g.read_rgba8(10)?;
        self.preview = Some(preview(&rgba, g.width, g.height));
        let frame = match self.options.format {
            Format::Hdr10 => Frame::Scrgb(g.read_floats(26)?),
            f if f.video() => Frame::Rgba8(rgba),
            _ => match self.options.image_format() {
                ImageFormat::Png | ImageFormat::Jpeg => Frame::Rgba8(rgba),
                format => read_image(g, format)?,
            },
        };
        self.pending = self.writer.try_send(frame)?;
        self.frame += 1;
        if self.frame < self.options.frames {
            if self.options.timeline {
                let t = self.frame_time(self.frame);
                self.animate(g, t)?;
            } else {
                self.settings.yaw = self.start_yaw + self.options.yaw(self.frame);
            }
            g.reset(true);
            self.frame_started = Instant::now();
        }
        Ok(())
    }
    /// Progress 0–1, counting the current frame's samples.
    pub fn progress(&self, samples: u32) -> f32 {
        let within = (samples as f32 / self.settings.max_samples as f32).min(1.);
        let within = if self.options.seconds_per_frame > 0. {
            within.max(self.frame_started.elapsed().as_secs_f32() / self.options.seconds_per_frame)
        } else {
            within
        };
        ((self.frame as f32 + within.min(1.)) / self.options.frames as f32).min(1.)
    }
    /// Estimated time left, once a frame is done.
    pub fn remaining(&self) -> Option<Duration> {
        (self.frame > 0).then(|| {
            self.started.elapsed() / self.frame
                * (self.options.frames - self.frame.min(self.options.frames))
        })
    }
    /// Waits for the writer and gives the output back the window's size. Returns what was
    /// written; a cancelled video is deleted, cancelled frames stay.
    pub unsafe fn finish(mut self, g: &mut Gpu, s: &Settings, cancel: bool) -> Result<String> {
        let written = self.writer.finish(cancel);
        g.keep_exposure = false;
        let [w, h] = g.swap_size;
        g.resize_output(w, h, s)?;
        // The GPU holds the animated scene: give it the user's back.
        if self.scene_changed {
            g.rebuild_scene(s)?;
        }
        let environment = if self.environment_changed {
            crate::sky::upload_environment(g, s)
        } else {
            Ok(())
        };
        g.reset(true);
        let written = written?;
        environment?;
        let [ew, eh] = self.options.size();
        let secs = self.started.elapsed().as_secs_f32();
        if self.options.still && !cancel {
            return Ok(format!(
                "Saved {} ({ew}×{eh}, {} spp, {secs:.0} s){note}",
                self.options.path.display(),
                self.settings.max_samples,
                note = if self.note.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", self.note)
                }
            ));
        }
        let what = if self.options.format.video() {
            self.options.path.display().to_string()
        } else {
            format!("{written} frames to {}", self.options.folder().display())
        };
        let note = if self.note.is_empty() {
            String::new()
        } else {
            format!(" · {}", self.note)
        };
        Ok(if cancel {
            if self.options.format.video() {
                format!("Export cancelled after {written} frames")
            } else {
                format!("Export cancelled: wrote {what}")
            }
        } else {
            format!("Saved {what} ({written} frames, {ew}×{eh}, {secs:.0} s){note}")
        })
    }
}
/// A CPU render's frame for `format`: the linear mean for EXR, else the display transform
/// (`reference::display`, grain seeded by `seed`).
fn cpu_frame(
    s: &Settings,
    format: ImageFormat,
    linear: Vec<f32>,
    size: [u32; 2],
    seed: u32,
) -> Result<Frame> {
    Ok(match format {
        ImageFormat::Exr => Frame::Linear(linear),
        ImageFormat::Png | ImageFormat::Jpeg => {
            Frame::Rgba8(crate::reference::display(s, &linear, size, seed)?)
        }
        f => anyhow::bail!("The CPU renderer can't write {} yet", f.label()),
    })
}
/// Writes a CPU render (linear RGBA) as one image, as `write_image` writes the GPU's.
pub fn write_cpu_image(
    s: &Settings,
    path: &Path,
    format: ImageFormat,
    quality: u32,
    linear: Vec<f32>,
    size: [u32; 2],
    seed: u32,
) -> Result<()> {
    let frame = cpu_frame(s, format, linear, size, seed)?;
    write_image(path, format, quality, &frame, size[0], size[1])
}

/// An export on the CPU renderer, with no device: the turntable, timeline and high-resolution
/// still of `Job`, through the same writer. Each frame renders `spp` samples (or until
/// `seconds_per_frame`), then goes through the CPU display transform (EXR stays linear). A
/// turntable keeps one scene and only moves its camera; a timeline frame is the scene at its
/// time (`reference::prepare`, which also runs the simulation), and with the shutter open it adds
/// moments inside it, as the GPU's passes do. Returns what was written.
pub fn render_cpu(
    s: &Settings,
    mut options: Options,
    cancel: &AtomicBool,
    progress: &(dyn Fn(f32) + Sync),
) -> Result<String> {
    let started = Instant::now();
    let mut note = String::new();
    if options.format.video() && !ffmpeg_available() {
        options.format = Format::Png;
        note = " · ffmpeg not found: PNG frames".into();
    }
    anyhow::ensure!(
        options.format != Format::Hdr10,
        "HDR10 video needs the HDR display transform, which the CPU renderer has not yet"
    );
    if options.timeline {
        anyhow::ensure!(
            !s.animation.is_empty() || s.physics.enabled,
            "The scene has no keyframes to follow"
        );
        if !s.animation.is_empty() {
            options.fps = s.animation.fps;
            options.frames = s.animation.frames();
        }
    }
    anyhow::ensure!(options.frames > 0, "No frames to render");
    let format = options.image_format();
    // What `Job::start` sets, less what only the GPU has (DLAA, vsync, battle, the split).
    let mut v = s.clone();
    v.scale = 1.;
    v.rotate = false;
    v.compare = false;
    v.view = 0;
    v.lpe.clear();
    v.max_samples = options.spp.max(1);
    if !options.timeline {
        v.pitch = options.pitch.clamp(-89., 89.);
        v.distance = options.distance;
    }
    let size = options.size();
    let spp = options.spp.max(1);
    let blur = options.timeline
        && v.animation.shutter > 0.
        && !v.animation.is_empty()
        && !v.physics.enabled;
    let fps = options.fps.max(1) as f32;
    // The frame's renders: one per moment, the samples shared out; the sample indices go on
    // across moments, so no two moments repeat a pixel's random numbers.
    let moments = if blur { spp.min(32) } else { 1 };
    let turntable = (!options.timeline).then(|| crate::reference::prepare(&v, 0.));
    let mut orbit = match &turntable {
        Some(t) => Some(crate::reference::Progressive::new(t, size)?),
        None => None,
    };
    // Cheap checks first: a refused scene or format fails before the writer starts.
    if let Some(t) = &turntable {
        cpu_frame(t, format, vec![0.; 4], [1, 1], 0)?;
    } else {
        cpu_frame(&v, format, vec![0.; 4], [1, 1], 0)?;
    }
    let mut writer = Writer::start(&options, 1000.)?;
    let frames = options.frames;
    let result = (|| -> Result<u32> {
        for i in 0..frames {
            let frame_start = Instant::now();
            let limit = options.seconds_per_frame;
            let mut sum = vec![0f32; size[0] as usize * size[1] as usize * 4];
            let mut samples = 0u32;
            let shown;
            let report = |f: f32| progress(((i as f32 + f.min(1.)) / frames as f32).min(1.));
            // Adds passes to `p` until it has `n` samples or the frame's time is up.
            let run = |p: &mut crate::reference::Progressive,
                       n: u32,
                       from: f32,
                       share: f32|
             -> Result<()> {
                let base = p.samples;
                while p.samples < base + n {
                    let step = if limit > 0. { 1 } else { base + n - p.samples };
                    let at = (p.samples - base) as f32 / n as f32;
                    let f = |x: f32| report(from + share * (at + x * step as f32 / n as f32));
                    anyhow::ensure!(
                        p.pass(
                            step,
                            std::thread::available_parallelism().map_or(4, |n| n.get()),
                            cancel,
                            &f
                        ),
                        "cancelled"
                    );
                    if limit > 0. && frame_start.elapsed().as_secs_f32() >= limit {
                        break;
                    }
                }
                Ok(())
            };
            if let (Some(t), Some(p)) = (&turntable, &mut orbit) {
                let mut camera = t.clone();
                camera.yaw = t.yaw + options.yaw(i);
                p.move_camera(&camera, size);
                run(p, spp, 0., 1.)?;
                sum = p.total();
                samples = p.samples;
                shown = camera;
            } else {
                let t = i as f32 / fps;
                let mut first = 0;
                let mut last = None;
                for m in 0..moments {
                    let n = spp / moments + u32::from(m < spp % moments);
                    let at = if blur {
                        // Stratified in a scrambled order, as `Job::before_render`'s passes.
                        let u = ((m + 1) as f32 * 0.618_034).fract();
                        t + (u - 0.5) * v.animation.shutter / fps
                    } else {
                        t
                    };
                    let scene = crate::reference::prepare(&v, at.max(0.));
                    let mut p = crate::reference::Progressive::new(&scene, size)?;
                    p.samples = first;
                    run(&mut p, n, m as f32 / moments as f32, 1. / moments as f32)?;
                    let done = p.samples - first;
                    for (a, b) in sum.iter_mut().zip(p.total()) {
                        *a += b;
                    }
                    first = p.samples;
                    samples += done;
                    drop(p);
                    last = Some(scene);
                    if limit > 0. && frame_start.elapsed().as_secs_f32() >= limit {
                        break;
                    }
                }
                shown = last.context("no moment rendered")?;
            }
            let n = samples.max(1) as f32;
            let mean: Vec<f32> = sum
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| [c[0] / n, c[1] / n, c[2] / n, 1.])
                .collect();
            let mut frame = Some(cpu_frame(&shown, format, mean, size, i % 4096)?);
            while let Some(f) = frame.take() {
                frame = writer.try_send(f)?;
                if frame.is_some() {
                    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "cancelled");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        Ok(frames)
    })();
    let cancelled = cancel.load(Ordering::Relaxed);
    let written = writer.finish(cancelled || result.is_err());
    if !cancelled {
        result?;
    }
    let written = written?;
    let [w, h] = size;
    let secs = started.elapsed().as_secs_f32();
    Ok(if cancelled {
        if options.format.video() || options.still {
            format!("Export cancelled after {written} frames")
        } else {
            format!(
                "Export cancelled: wrote {written} frames to {}",
                options.folder().display()
            )
        }
    } else if options.still {
        format!(
            "Saved {} ({w}×{h}, {spp} spp, {secs:.0} s, CPU){note}",
            options.path.display()
        )
    } else if options.format.video() {
        format!(
            "Saved {} ({written} frames, {w}×{h}, {secs:.0} s, CPU){note}",
            options.path.display()
        )
    } else {
        format!(
            "Saved {written} frames to {} ({w}×{h}, {secs:.0} s, CPU){note}",
            options.folder().display()
        )
    })
}
/// `render_cpu` on a thread, for the app's software view: the export the GPU's `Job` would run.
/// Dropping it cancels the render and waits for the writer (a cancelled video is deleted).
pub struct CpuExport {
    pub options: Options,
    pub started: Instant,
    cancel: Arc<AtomicBool>,
    /// Progress 0–1 as f32 bits.
    progress: Arc<std::sync::atomic::AtomicU32>,
    thread: Option<std::thread::JoinHandle<Result<String>>>,
}
impl CpuExport {
    pub fn start(s: &Settings, options: Options) -> CpuExport {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let (s, o, stop, at) = (s.clone(), options.clone(), cancel.clone(), progress.clone());
        let thread = std::thread::spawn(move || {
            crate::platform::background_thread();
            render_cpu(&s, o, &stop, &|f| at.store(f.to_bits(), Ordering::Relaxed))
        });
        CpuExport {
            options,
            started: Instant::now(),
            cancel,
            progress,
            thread: Some(thread),
        }
    }
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    /// Waits for the thread; what it wrote, or why it stopped.
    pub fn join(mut self) -> Result<String> {
        self.thread
            .take()
            .context("already joined")?
            .join()
            .map_err(|_| anyhow::anyhow!("the CPU export panicked"))?
    }
}
impl Drop for CpuExport {
    fn drop(&mut self) {
        self.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
/// A small RGBA copy for the progress window (nearest texel).
fn preview(rgba: &[u8], w: u32, h: u32) -> egui::ColorImage {
    let step = w.div_ceil(360).max(1);
    let (pw, ph) = (w.div_ceil(step), h.div_ceil(step));
    let mut pixels = Vec::with_capacity((pw * ph) as usize);
    for y in 0..ph {
        for x in 0..pw {
            let i = (((y * step).min(h - 1) * w + (x * step).min(w - 1)) * 4) as usize;
            pixels.push(egui::Color32::from_rgb(rgba[i], rgba[i + 1], rgba[i + 2]));
        }
    }
    egui::ColorImage {
        size: [pw as usize, ph as usize],
        pixels,
    }
}
/// Renders a whole export before returning (headless `--turntable`, the self-test).
pub unsafe fn render(g: &mut Gpu, s: &Settings, options: Options) -> Result<String> {
    let mut job = Job::start(g, s, options)?;
    while !job.done() {
        let step = job
            .before_render(g)
            .and_then(|()| {
                g.render(
                    job.settings(),
                    true,
                    job.dt(),
                    &[],
                    &egui::TexturesDelta::default(),
                    1.,
                )
            })
            .and_then(|()| job.after_render(g));
        if let Err(e) = step {
            let _ = job.finish(g, s, true);
            return Err(e);
        }
    }
    job.finish(g, s, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_turntable_frames_match_the_still() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-cpu-export");
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.preset(0);
        let size = [32, 24];
        let mut o = Options::turntable(&s, size, 3, 2, dir.join("spin.png"));
        o.format = Format::Png;
        let text = render_cpu(&s, o.clone(), &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(text.contains("3 frames"), "{text}");
        let first = image::open(dir.join("spin/00001.png")).unwrap().to_rgba8();
        let still = crate::reference::prepare(&s, 0.);
        let linear = crate::reference::render(&still, size, 2, &|_| {}).unwrap();
        let expect = crate::reference::display(&still, &linear, size, 0).unwrap();
        assert_eq!(first.into_raw(), expect);
        // The camera turns: the second frame differs.
        let second = image::open(dir.join("spin/00002.png")).unwrap().to_rgba8();
        assert_ne!(second.into_raw(), expect);
        // A cancelled export stops with what it had; a refused format writes nothing.
        let cancel = AtomicBool::new(true);
        o.path = dir.join("cancelled.png");
        assert!(
            render_cpu(&s, o.clone(), &cancel, &|_| {})
                .unwrap()
                .contains("cancelled")
        );
        o.format = Format::Hdr10;
        assert!(render_cpu(&s, o, &AtomicBool::new(false), &|_| {}).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn image_formats_by_file_name() {
        for (name, f) in [
            ("a.EXR", ImageFormat::Exr),
            ("a.layers.exr", ImageFormat::Layers),
            ("a.hdr.png", ImageFormat::HdrPng),
            ("a.png", ImageFormat::Png),
            ("a.jpeg", ImageFormat::Jpeg),
        ] {
            assert_eq!(ImageFormat::from_path(Path::new(name)), Some(f));
        }
        assert_eq!(ImageFormat::from_path(Path::new("a.tga")), None);
    }

    #[test]
    fn name_templates_that_repeat() {
        assert!(name_repeats("caustica"));
        assert!(name_repeats("{scene}-{date}-{w}x{h}"));
        assert!(!name_repeats("{scene}-{time}"));
        assert!(!name_repeats("caustica-{spp}"));
    }
    #[test]
    fn name_templates_expand_and_stay_valid() {
        let n = expand_name("{scene}-{spp}spp-{w}x{h}", "Cornell: box?", 64, 640, 360);
        assert_eq!(n, "Cornell_ box_-64spp-640x360");
        let d = expand_name("{date}_{time}", "", 0, 1, 1);
        assert_eq!(d.len(), 19, "{d}");
        assert_eq!(expand_name("  ..", "", 0, 1, 1), "caustica");
        assert_eq!(expand_name("{scene}", "nul", 0, 1, 1), "_nul");
        assert_eq!(expand_name("com1.final", "", 0, 1, 1), "_com1.final");
        assert_eq!(expand_name("{scene}", &"x".repeat(300), 0, 1, 1).len(), 120);
    }
    #[test]
    fn layered_exr_round_trips() {
        let layers = vec![
            crate::gpu::AovLayer {
                name: "beauty".into(),
                width: 4,
                height: 2,
                channels: vec!["R", "G", "B", "A"],
                data: (0..32).map(|v| v as f32).collect(),
            },
            crate::gpu::AovLayer {
                name: "depth".into(),
                width: 2,
                height: 1,
                channels: vec!["Z"],
                data: vec![1.5, 2.5],
            },
        ];
        let path = std::env::temp_dir().join("caustica-layers-test.layers.exr");
        write_layers(&path, &layers).unwrap();
        let image = exr::prelude::read_all_flat_layers_from_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let names: Vec<String> = image
            .layer_data
            .iter()
            .map(|l| l.attributes.layer_name.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(names, ["beauty", "depth"]);
        let depth = &image.layer_data[1];
        assert_eq!((depth.size.0, depth.size.1), (2, 1));
        assert_eq!(
            depth.channel_data.list[0]
                .sample_data
                .values_as_f32()
                .collect::<Vec<_>>(),
            [1.5, 2.5]
        );
        // Channels come back sorted by name (A, B, G, R): R of the second texel is 4.
        let beauty = &image.layer_data[0];
        let r = beauty
            .channel_data
            .list
            .iter()
            .find(|c| c.name.to_string() == "R")
            .unwrap();
        assert_eq!(r.sample_data.values_as_f32().nth(1), Some(4.));
    }
    fn options(format: Format) -> Options {
        let mut o = Options::turntable(&Settings::default(), [1281, 721], 4, 8, "out.mp4".into());
        o.format = format;
        o
    }
    #[test]
    fn orbits_loop_or_ease_to_the_angle() {
        let mut o = options(Format::H264);
        o.degrees = 360.;
        assert_eq!(
            (0..4).map(|i| o.yaw(i)).collect::<Vec<_>>(),
            [0., 90., 180., 270.]
        );
        o.easing = Easing::EaseInOut;
        assert_eq!(o.yaw(0), 0.);
        assert_eq!(o.yaw(3), 360.);
        assert!(o.yaw(1) < 120. && o.yaw(2) > 240.);
    }
    #[test]
    fn video_sizes_are_even_and_sequences_keep_theirs() {
        assert_eq!(options(Format::H264).size(), [1280, 720]);
        assert_eq!(options(Format::Png).size(), [1281, 721]);
        assert_eq!(options(Format::Exr).folder(), PathBuf::from("out"));
    }
    #[test]
    fn ffmpeg_gets_the_right_encoder_and_tags() {
        let args = |f| ffmpeg_args(&options(f), 1000.).join(" ");
        let h264 = args(Format::H264);
        assert!(h264.contains("-pix_fmt rgba -s 1280x720 -r 30"), "{h264}");
        assert!(h264.contains("libx264") && h264.contains("-crf 16") && h264.ends_with("out.mp4"));
        assert!(h264.contains("out_color_matrix=bt709") && h264.contains("-color_trc bt709"));
        let hdr = args(Format::Hdr10);
        assert!(
            hdr.contains("rgb48le") && hdr.contains("yuv420p10le"),
            "{hdr}"
        );
        assert!(hdr.contains("smpte2084") && hdr.contains("max-cll=1000,500"));
        let mut o = options(Format::ProRes);
        o.quality = 3;
        let prores = ffmpeg_args(&o, 1000.).join(" ");
        assert!(prores.contains("prores_ks -profile:v 3") && prores.contains("yuv422p10le"));
    }
    #[test]
    fn equirect_images_carry_the_panorama_xmp() {
        let dir = std::env::temp_dir();
        let rgba = vec![128u8; 8 * 4 * 4];
        for format in [ImageFormat::Png, ImageFormat::Jpeg] {
            let path = dir.join(format!("caustica-gpano-test.{}", format.extension()));
            write_image(&path, format, 90, &Frame::Rgba8(rgba.clone()), 8, 4).unwrap();
            tag_equirect(&path, format, 8, 4).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("GPano:ProjectionType=\"equirectangular\""));
            assert!(text.contains("GPano:FullPanoWidthPixels=\"8\""));
            // Still a valid image of the same size (chunk CRC / segment length right).
            let img = image::load_from_memory(&bytes).unwrap();
            assert_eq!((img.width(), img.height()), (8, 4));
            let _ = std::fs::remove_file(&path);
        }
    }
}
