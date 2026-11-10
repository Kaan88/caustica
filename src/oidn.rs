//! Intel Open Image Denoise 2 for still images: the "RT" filter on the CPU device, loaded at run
//! time (`libloading`), so a missing library only turns the feature off, with the reason in
//! `status()`. `build.rs` copies the libraries (vendor/oidn, `tools/setup`) next to the exe; the
//! AppImage and the installer ship them there. Notes: engine-notes "OIDN for stills".
//!
//! It is for finished images: a capture, Save image and the CPU renderer's converged view. It
//! runs on the CPU (one device per call: the cost of a call is the filter, not the device), on
//! either backend's readback. Inputs are packed RGB `f32` rows, top row first: the colour (HDR
//! linear, or 8-bit sRGB values scaled to 0..1 for the display-referred PNG and JPEG), and the
//! engine's albedo and normal guides, which are noise-free (a pixel-centre ray), so `cleanAux`
//! is set.
use crate::render::AovLayer;
use anyhow::{Context, Result, bail, ensure};
use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type Handle = *mut c_void;
/// `OIDN_DEVICE_TYPE_CPU`, `OIDN_FORMAT_FLOAT3`, `OIDN_QUALITY_HIGH`.
const DEVICE_CPU: i32 = 1;
const FLOAT3: i32 = 3;
const QUALITY_HIGH: i32 = 6;

/// The library's entry points. The function pointers stay valid while `_libs` lives, which is
/// for the rest of the run (`api` keeps it in a static).
struct Api {
    _libs: Vec<Library>,
    new_device: unsafe extern "C" fn(i32) -> Handle,
    commit_device: unsafe extern "C" fn(Handle),
    release_device: unsafe extern "C" fn(Handle),
    device_error: unsafe extern "C" fn(Handle, *mut *const c_char) -> i32,
    new_filter: unsafe extern "C" fn(Handle, *const c_char) -> Handle,
    release_filter: unsafe extern "C" fn(Handle),
    set_image: unsafe extern "C" fn(
        Handle,
        *const c_char,
        *mut c_void,
        i32,
        usize,
        usize,
        usize,
        usize,
        usize,
    ),
    set_bool: unsafe extern "C" fn(Handle, *const c_char, bool),
    set_int: unsafe extern "C" fn(Handle, *const c_char, i32),
    commit_filter: unsafe extern "C" fn(Handle),
    execute_filter: unsafe extern "C" fn(Handle),
}
// The handles are plain function pointers into a library that is never unloaded.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// The libraries in the order they must load: oneTBB, the core, the CPU device, the front. The
/// Linux build's front library looks for the core through `$ORIGIN/../lib`, and the core for the
/// device by name, so with everything in the exe's folder each is loaded by path first and the
/// loader then finds it by its soname. Matched by the start of the file name (a version
/// follows on Linux).
#[cfg(windows)]
const LIBS: [&str; 4] = [
    "tbb12.dll",
    "OpenImageDenoise_core.dll",
    "OpenImageDenoise_device_cpu.dll",
    "OpenImageDenoise.dll",
];
#[cfg(not(windows))]
const LIBS: [&str; 4] = [
    "libtbb.so.",
    "libOpenImageDenoise_core.so.",
    "libOpenImageDenoise_device_cpu.so.",
    "libOpenImageDenoise.so.",
];

/// Where the libraries may be: beside the exe (installed, and `build.rs`'s copy), one folder up
/// (a test binary in `deps`), then the vendored copy the build was made from.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        if let Some(up) = dir.parent() {
            dirs.push(up.to_path_buf());
        }
    }
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/oidn");
    dirs.push(vendor.join(if cfg!(windows) {
        "windows/bin"
    } else {
        "linux/lib"
    }));
    dirs
}
/// The file in `dir` whose name starts with `prefix` (the longest name last: a real file over its
/// symlinks makes no difference to the loader).
fn find(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| e.path())
        .collect();
    names.sort();
    names.pop()
}

fn load() -> Result<Api> {
    let dirs = search_dirs();
    let dir = dirs
        .iter()
        .find(|d| find(d, LIBS[3]).is_some() && find(d, LIBS[1]).is_some())
        .context("The OIDN libraries are not next to the exe (tools/setup fetches them)")?;
    let mut libs = vec![];
    for (i, prefix) in LIBS.iter().enumerate() {
        match find(dir, prefix) {
            // SAFETY: loading a vendor library runs its initialisers.
            Some(path) => libs.push(
                unsafe { Library::new(&path) }
                    .with_context(|| format!("Loading {}", path.display()))?,
            ),
            // oneTBB may come from the system on Linux; the others have to be there.
            None if i == 0 && !cfg!(windows) => {}
            None => bail!("{prefix} is missing from {}", dir.display()),
        }
    }
    let front = libs.last().context("no library")?;
    macro_rules! sym {
        ($name:literal) => {
            *unsafe { front.get(concat!($name, "\0").as_bytes()) }
                .with_context(|| format!("{} is missing from the OIDN library", $name))?
        };
    }
    let api = Api {
        new_device: sym!("oidnNewDevice"),
        commit_device: sym!("oidnCommitDevice"),
        release_device: sym!("oidnReleaseDevice"),
        device_error: sym!("oidnGetDeviceError"),
        new_filter: sym!("oidnNewFilter"),
        release_filter: sym!("oidnReleaseFilter"),
        set_image: sym!("oidnSetSharedFilterImage"),
        set_bool: sym!("oidnSetFilterBool"),
        set_int: sym!("oidnSetFilterInt"),
        commit_filter: sym!("oidnCommitFilter"),
        execute_filter: sym!("oidnExecuteFilter"),
        _libs: vec![],
    };
    let supported: unsafe extern "C" fn() -> bool = sym!("oidnIsCPUDeviceSupported");
    ensure!(unsafe { supported() }, "This CPU is not supported by OIDN");
    Ok(Api { _libs: libs, ..api })
}

fn api() -> Result<&'static Api, &'static str> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        load().map_err(|e| {
            let why = format!("{e:#}");
            eprintln!("OIDN unavailable: {why}");
            why
        })
    })
    .as_ref()
    .map_err(|e| e.as_str())
}

/// Whether the still denoiser can run: Ok, or why not (shown in the UI and the log).
pub fn status() -> Result<(), &'static str> {
    api().map(|_| ())
}

/// What the colour image holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    /// Linear radiance, any range (the CPU renderer's mean).
    Hdr,
    /// Display-referred sRGB values in 0..1 (a PNG or JPEG).
    Srgb,
}

struct Device<'a>(&'a Api, Handle);
impl Drop for Device<'_> {
    fn drop(&mut self) {
        unsafe { (self.0.release_device)(self.1) }
    }
}
struct Filter<'a>(&'a Api, Handle);
impl Drop for Filter<'_> {
    fn drop(&mut self) {
        unsafe { (self.0.release_filter)(self.1) }
    }
}
fn check(api: &Api, device: Handle, what: &str) -> Result<()> {
    let mut message: *const c_char = std::ptr::null();
    let code = unsafe { (api.device_error)(device, &mut message) };
    if code == 0 {
        return Ok(());
    }
    let text = if message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    bail!("OIDN {what}: error {code} {text}")
}

/// Filters packed RGB `color` (`w` × `h`) with the "RT" filter at high quality; `albedo` and
/// `normal` (also packed RGB, noise-free) guide it, the normal only together with the albedo.
/// Returns the filtered image in the same layout.
pub fn denoise(
    w: usize,
    h: usize,
    color: &[f32],
    kind: Colour,
    albedo: Option<&[f32]>,
    normal: Option<&[f32]>,
) -> Result<Vec<f32>> {
    ensure!(
        color.len() == w * h * 3 && w > 0 && h > 0,
        "OIDN: bad image size"
    );
    for aux in albedo.iter().chain(normal.iter()) {
        ensure!(aux.len() == color.len(), "OIDN: a guide has another size");
    }
    let api = api().map_err(anyhow::Error::msg)?;
    // The filter reads its inputs by pointer: NaNs and negatives would poison a neighbourhood.
    let mut input = color.to_vec();
    for v in &mut input {
        *v = if v.is_finite() {
            v.clamp(0., if kind == Colour::Srgb { 1. } else { f32::MAX })
        } else {
            0.
        };
    }
    let clean = |v: &[f32], lo: f32| -> Vec<f32> {
        v.iter()
            .map(|x| if x.is_finite() { x.clamp(lo, 1.) } else { 0. })
            .collect()
    };
    let mut albedo = albedo.map(|a| clean(a, 0.));
    let mut normal = normal.filter(|_| albedo.is_some()).map(|n| clean(n, -1.));
    let mut output = vec![0f32; input.len()];
    unsafe {
        let device = Device(api, (api.new_device)(DEVICE_CPU));
        ensure!(!device.1.is_null(), "OIDN: no CPU device");
        (api.commit_device)(device.1);
        check(api, device.1, "device")?;
        let rt = CString::new("RT")?;
        let filter = Filter(api, (api.new_filter)(device.1, rt.as_ptr()));
        check(api, device.1, "filter")?;
        let image = |name: &str, data: *mut f32| {
            let name = CString::new(name).unwrap();
            (api.set_image)(filter.1, name.as_ptr(), data.cast(), FLOAT3, w, h, 0, 0, 0);
        };
        image("color", input.as_mut_ptr());
        if let Some(a) = &mut albedo {
            image("albedo", a.as_mut_ptr());
        }
        if let Some(n) = &mut normal {
            image("normal", n.as_mut_ptr());
        }
        image("output", output.as_mut_ptr());
        let flag = |name: &str, v: bool| {
            let name = CString::new(name).unwrap();
            (api.set_bool)(filter.1, name.as_ptr(), v);
        };
        flag("hdr", kind == Colour::Hdr);
        flag("srgb", kind == Colour::Srgb);
        flag("cleanAux", true);
        let quality = CString::new("quality")?;
        (api.set_int)(filter.1, quality.as_ptr(), QUALITY_HIGH);
        (api.commit_filter)(filter.1);
        check(api, device.1, "commit")?;
        (api.execute_filter)(filter.1);
        check(api, device.1, "execute")?;
    }
    Ok(output)
}

/// The albedo and normal guides for a `w` × `h` image from the renderer's AOV layers
/// (`Gpu::read_layers`): the albedo is the diffuse plus the specular albedo, so metals keep their
/// colour. A layer of another size (a render scale below 100%) is left out: no guides then.
pub fn guides_from_layers(layers: &[AovLayer], w: usize, h: usize) -> Option<(Vec<f32>, Vec<f32>)> {
    let get = |name: &str| {
        layers
            .iter()
            .find(|l| l.name == name && l.width as usize == w && l.height as usize == h)
            .filter(|l| l.channels.len() == 3 && l.data.len() == w * h * 3)
    };
    let (d, s, n) = (
        get("diffuse_albedo")?,
        get("specular_albedo")?,
        get("normal")?,
    );
    let albedo = d
        .data
        .iter()
        .zip(&s.data)
        .map(|(d, s)| (d + s).min(1.))
        .collect();
    Some((albedo, n.data.clone()))
}

fn pack_rgb(rgba: impl Iterator<Item = [f32; 4]>, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n * 3);
    for c in rgba {
        out.extend_from_slice(&c[..3]);
    }
    out
}

/// Denoises the 8-bit sRGB `rgba` pixels of a PNG or JPEG in place (the alpha stays).
pub fn denoise_rgba8(
    w: usize,
    h: usize,
    rgba: &mut [u8],
    guides: Option<&(Vec<f32>, Vec<f32>)>,
) -> Result<()> {
    ensure!(rgba.len() == w * h * 4, "OIDN: bad image size");
    let color = pack_rgb(
        rgba.as_chunks::<4>().0.iter().map(|p| {
            [
                p[0] as f32 / 255.,
                p[1] as f32 / 255.,
                p[2] as f32 / 255.,
                1.,
            ]
        }),
        w * h,
    );
    let t = std::time::Instant::now();
    let out = denoise(
        w,
        h,
        &color,
        Colour::Srgb,
        guides.map(|g| g.0.as_slice()),
        guides.map(|g| g.1.as_slice()),
    )?;
    eprintln!(
        "OIDN {w}x{h} {:.0} ms ({})",
        t.elapsed().as_secs_f64() * 1000.,
        if guides.is_some() {
            "albedo, normal"
        } else {
            "colour only"
        }
    );
    for (p, c) in rgba
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(out.as_chunks::<3>().0)
    {
        for (d, v) in p.iter_mut().zip(c) {
            *d = (v * 255.).round().clamp(0., 255.) as u8;
        }
    }
    Ok(())
}

/// Denoises linear RGBA `f32` pixels in place (the alpha stays).
pub fn denoise_linear_rgba(
    w: usize,
    h: usize,
    rgba: &mut [f32],
    guides: Option<&(Vec<f32>, Vec<f32>)>,
) -> Result<()> {
    ensure!(rgba.len() == w * h * 4, "OIDN: bad image size");
    let color = pack_rgb(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2], 1.]),
        w * h,
    );
    let t = std::time::Instant::now();
    let out = denoise(
        w,
        h,
        &color,
        Colour::Hdr,
        guides.map(|g| g.0.as_slice()),
        guides.map(|g| g.1.as_slice()),
    )?;
    eprintln!(
        "OIDN {w}x{h} {:.0} ms ({})",
        t.elapsed().as_secs_f64() * 1000.,
        if guides.is_some() {
            "albedo, normal"
        } else {
            "colour only"
        }
    );
    for (p, c) in rgba
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(out.as_chunks::<3>().0)
    {
        p[..3].copy_from_slice(c);
    }
    Ok(())
}

/// A finished PNG or JPEG on disk (`Gpu::capture`, which both backends write themselves),
/// denoised and written back. The guides come from the renderer's layers when it has them at
/// this size. A JPEG is encoded again at `JPEG_QUALITY`.
pub fn denoise_file(path: &Path, layers: Option<&[AovLayer]>) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    ensure!(
        (ext == "png" && !path.to_string_lossy().ends_with(".hdr.png"))
            || ext == "jpg"
            || ext == "jpeg",
        "OIDN denoises PNG and JPEG saves (EXR stays the raw samples)"
    );
    let mut img = image::open(path)?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let guides = layers.and_then(|l| guides_from_layers(l, w, h));
    denoise_rgba8(w, h, &mut img, guides.as_ref())?;
    if ext == "png" {
        img.save_with_format(path, image::ImageFormat::Png)?;
    } else {
        let rgb: Vec<u8> = img
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(file, crate::video::JPEG_QUALITY as u8)
            .encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth gradient with a sharp edge, plus noise from a fixed generator.
    fn noisy(w: usize, h: usize, sigma: f32) -> (Vec<f32>, Vec<f32>) {
        let mut state = 0x1234_5678u32;
        let mut rand = move || {
            state = state.wrapping_mul(747796405).wrapping_add(2891336453);
            let x = ((state >> 16) ^ state).wrapping_mul(0x45d9f3b);
            (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let (mut clean, mut dirty) = (vec![], vec![]);
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 {
                    0.2 + 0.4 * y as f32 / h as f32
                } else {
                    0.8
                };
                for _ in 0..3 {
                    clean.push(base);
                    dirty.push((base + rand() * sigma).max(0.));
                }
            }
        }
        (clean, dirty)
    }
    fn rmse(a: &[f32], b: &[f32]) -> f64 {
        let s: f64 = a.iter().zip(b).map(|(a, b)| ((a - b) as f64).powi(2)).sum();
        (s / a.len() as f64).sqrt()
    }

    #[test]
    fn it_removes_noise_from_a_synthetic_image() {
        if let Err(why) = status() {
            eprintln!("skipped: OIDN is not available here ({why})");
            return;
        }
        let (w, h) = (96, 64);
        let (clean, dirty) = noisy(w, h, 0.4);
        for kind in [Colour::Hdr, Colour::Srgb] {
            let mut dirty = dirty.clone();
            if kind == Colour::Srgb {
                // 8-bit values stop at 1.
                dirty.iter_mut().for_each(|v| *v = v.min(1.));
            }
            let out = denoise(w, h, &dirty, kind, None, None).unwrap();
            let (before, after) = (rmse(&clean, &dirty), rmse(&clean, &out));
            eprintln!("{kind:?}: rmse {before:.4} -> {after:.4}");
            assert!(after < before * 0.5, "{kind:?}: {before} -> {after}");
        }
        // Guides: the albedo marks the edge, the normal is flat.
        let albedo: Vec<f32> = (0..w * h * 3)
            .map(|i| if (i / 3) % w < w / 2 { 0.3 } else { 0.9 })
            .collect();
        let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1.]).collect();
        let with = denoise(w, h, &dirty, Colour::Hdr, Some(&albedo), Some(&normal)).unwrap();
        assert!(rmse(&clean, &with) < rmse(&clean, &dirty) * 0.5);
        assert!(with.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn it_rewrites_png_and_jpeg_files_and_leaves_exr_alone() {
        if let Err(why) = status() {
            eprintln!("skipped: OIDN is not available here ({why})");
            return;
        }
        let dir = std::env::temp_dir().join(format!("caustica-oidn-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (64usize, 48usize);
        let (clean, dirty) = noisy(w, h, 0.4);
        let bytes = |v: &[f32]| -> Vec<u8> {
            v.chunks_exact(3)
                .flat_map(|c| c.iter().map(|x| (x.clamp(0., 1.) * 255.) as u8))
                .collect()
        };
        let err = |path: &Path| {
            let got = image::open(path).unwrap().to_rgb8().into_raw();
            let want: Vec<f32> = bytes(&clean).iter().map(|b| *b as f32 / 255.).collect();
            let got: Vec<f32> = got.iter().map(|b| *b as f32 / 255.).collect();
            rmse(&want, &got)
        };
        let noisy_rgb = bytes(&dirty);
        for name in ["a.png", "a.jpg"] {
            let path = dir.join(name);
            image::save_buffer(
                &path,
                &noisy_rgb,
                w as u32,
                h as u32,
                image::ColorType::Rgb8,
            )
            .unwrap();
            let before = err(&path);
            denoise_file(&path, None).unwrap();
            let after = err(&path);
            assert!(after < before * 0.6, "{name}: {before} -> {after}");
        }
        assert!(denoise_file(&dir.join("a.exr"), None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_refuses_a_bad_size() {
        assert!(denoise(4, 4, &[0.; 10], Colour::Hdr, None, None).is_err());
    }

    #[test]
    fn guides_need_matching_layers() {
        let layer = |name: &str, n: usize| AovLayer {
            name: name.into(),
            width: 2,
            height: 1,
            channels: vec!["R"; n],
            data: vec![0.5; 2 * n],
        };
        let layers = [
            layer("diffuse_albedo", 3),
            layer("specular_albedo", 3),
            layer("normal", 3),
        ];
        let (a, n) = guides_from_layers(&layers, 2, 1).unwrap();
        assert_eq!((a.len(), n.len(), a[0]), (6, 6, 1.0));
        assert!(guides_from_layers(&layers, 4, 1).is_none());
    }
}
