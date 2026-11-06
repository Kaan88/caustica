//! The monitor the window is on, as Windows describes it: whether HDR is on, the panel's
//! luminance range (from Windows HDR Calibration when the user ran it) and the
//! "SDR content brightness" slider.
use anyhow::Result;
#[cfg(windows)]
use windows::Win32::{
    Devices::Display::*,
    Foundation::{HWND, RECT},
    Graphics::Dxgi::{Common::*, *},
    UI::WindowsAndMessaging::GetWindowRect,
};
#[cfg(windows)]
use windows::core::Interface;

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    /// Windows HDR is on for this monitor (ST.2084 / BT.2020 desktop).
    pub hdr: bool,
    pub peak_nits: f32,
    pub full_frame_nits: f32,
    pub min_nits: f32,
    /// SDR white level in nits (Settings → Display → HDR → SDR content brightness).
    pub sdr_white_nits: f32,
    pub name: String,
}
impl Default for DisplayInfo {
    fn default() -> Self {
        Self {
            hdr: false,
            peak_nits: 0.,
            full_frame_nits: 0.,
            min_nits: 0.,
            sdr_white_nits: 80.,
            name: String::new(),
        }
    }
}

/// Holds the factory the description came from: once Windows changes the display setup
/// (HDR toggled, monitor added, calibration rerun) it is no longer current.
#[cfg(windows)]
pub struct Monitor {
    factory: IDXGIFactory1,
    pub info: DisplayInfo,
}
#[cfg(windows)]
impl Monitor {
    pub unsafe fn query(hwnd: HWND) -> Result<Self> {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut window = RECT::default();
        GetWindowRect(hwnd, &mut window)?;
        // The output that holds most of the window, as in Microsoft's D3D12HDR sample.
        let mut best: Option<(i64, IDXGIOutput6)> = None;
        for a in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(a) else {
                break;
            };
            for o in 0.. {
                let Ok(output) = adapter.EnumOutputs(o) else {
                    break;
                };
                let r = output.GetDesc()?.DesktopCoordinates;
                let w = (window.right.min(r.right) - window.left.max(r.left)).max(0) as i64;
                let h = (window.bottom.min(r.bottom) - window.top.max(r.top)).max(0) as i64;
                if let Ok(o6) = output.cast::<IDXGIOutput6>()
                    && best.as_ref().is_none_or(|(area, _)| w * h > *area)
                {
                    best = Some((w * h, o6));
                }
            }
        }
        let mut info = DisplayInfo::default();
        if let Some((_, output)) = best {
            let d = output.GetDesc1()?;
            info.hdr = d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
            info.peak_nits = d.MaxLuminance;
            info.full_frame_nits = d.MaxFullFrameLuminance;
            info.min_nits = d.MinLuminance;
            let end = d.DeviceName.iter().position(|c| *c == 0).unwrap_or(32);
            if let Some((white, name)) = white_level(&d.DeviceName[..end]) {
                info.sdr_white_nits = white;
                info.name = name;
            }
        }
        Ok(Self { factory, info })
    }
    /// False once Windows changed the display configuration since `query`.
    pub unsafe fn current(&self) -> bool {
        self.factory.IsCurrent().as_bool()
    }
}

/// SDR white level and friendly name of the display path whose source is `gdi_name`
/// (e.g. `\\.\DISPLAY1`).
#[cfg(windows)]
unsafe fn white_level(gdi_name: &[u16]) -> Option<(f32, String)> {
    let (mut paths, mut modes) = (0, 0);
    GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes)
        .ok()
        .ok()?;
    let mut path = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
    let mut mode = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
    QueryDisplayConfig(
        QDC_ONLY_ACTIVE_PATHS,
        &mut paths,
        path.as_mut_ptr(),
        &mut modes,
        mode.as_mut_ptr(),
        None,
    )
    .ok()
    .ok()?;
    for p in &path[..paths as usize] {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = size_of_val(&source) as u32;
        source.header.adapterId = p.sourceInfo.adapterId;
        source.header.id = p.sourceInfo.id;
        if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
            continue;
        }
        let end = source
            .viewGdiDeviceName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(32);
        if source.viewGdiDeviceName[..end] != *gdi_name {
            continue;
        }
        let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
        white.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        white.header.size = size_of_val(&white) as u32;
        white.header.adapterId = p.targetInfo.adapterId;
        white.header.id = p.targetInfo.id;
        if DisplayConfigGetDeviceInfo(&mut white.header) != 0 {
            return None;
        }
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = size_of_val(&target) as u32;
        target.header.adapterId = p.targetInfo.adapterId;
        target.header.id = p.targetInfo.id;
        let name = if DisplayConfigGetDeviceInfo(&mut target.header) == 0 {
            let n = &target.monitorFriendlyDeviceName;
            String::from_utf16_lossy(&n[..n.iter().position(|c| *c == 0).unwrap_or(n.len())])
        } else {
            String::new()
        };
        // Reported in thousandths of 80 nits.
        return Some((white.SDRWhiteLevel as f32 * 80. / 1000., name));
    }
    None
}

/// CPU copy of `tone_curve` in post.hlsl.
fn tone_curve(c: f32, mode: u32) -> f32 {
    let aces = |c: f32| (c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14)).clamp(0., 1.);
    let filmic =
        |x: f32| ((x * (0.15 * x + 0.05) + 0.004) / (x * (0.15 * x + 0.5) + 0.06)) - 0.066_666_67;
    let v = match mode {
        1 => c / (1. + c),
        2 => c * (1. + c / 16.) / (1. + c),
        3 => aces(c),
        4 => filmic(c * 2.) / filmic(11.2),
        5 => {
            let a = aces(c);
            a * a * (3. - 2. * a)
        }
        _ => c,
    };
    v.clamp(0., 1.)
}
/// CPU copy of `hdr_map` in post.hlsl, for tests: scene-linear RGB → scRGB (1.0 = 80 nits).
/// Tone map 0 (Linear) uses the BT.2390 roll-off; the others keep their SDR curve and expand
/// only the highlights into the headroom peak / white.
pub fn hdr_map(
    c: [f32; 3],
    exposure: f32,
    gamma: f32,
    tonemap: u32,
    white: f32,
    peak: f32,
) -> [f32; 3] {
    if tonemap != 0 {
        let big_r = ((peak / white).max(1.) - 0.5) / 0.5;
        return c.map(|v| {
            let t = tone_curve((v * exposure).max(0.), tonemap).powf(2.2 / gamma.max(0.1));
            let u = (t - 0.5).max(0.) / 0.5;
            (t.min(0.5) + 0.5 * u / (1. - u + u / big_r)) * white / 80.
        });
    }
    fn pq(nits: f32) -> f32 {
        let y = (nits.clamp(0., 10000.) / 10000.).powf(0.159_301_76);
        ((0.835_937_5 + 18.851_563 * y) / (1. + 18.6875 * y)).powf(78.84375)
    }
    fn pq_inverse(e: f32) -> f32 {
        let p = e.clamp(0., 1.).powf(1. / 78.84375);
        10000. * ((p - 0.835_937_5).max(0.) / (18.851_563 - 18.6875 * p)).powf(1. / 0.159_301_76)
    }
    let c = c.map(|v| (v * exposure).max(0.) * white);
    let m = c[0].max(c[1]).max(c[2]);
    if m <= 0. {
        return [0.; 3];
    }
    let (top, mut e) = (pq(peak), pq(m));
    let knee = 1.5 * top - 0.5;
    if e > knee {
        let t = (e - knee) / (1. - knee);
        let (t2, t3) = (t * t, t * t * t);
        e = (2. * t3 - 3. * t2 + 1.) * knee
            + (t3 - 2. * t2 + t) * (1. - knee)
            + (-2. * t3 + 3. * t2) * top;
    }
    let scale = pq_inverse(e).min(peak) / m / 80.;
    c.map(|v| v * scale)
}

/// Writes scRGB pixels (RGBA, 1.0 = 80 nits, BT.709 primaries) as a 16-bit PNG in BT.2020 / PQ,
/// tagged with a cICP chunk so HDR-aware viewers (Windows Photos, Chrome) show it as HDR.
pub fn write_hdr_png(path: &std::path::Path, scrgb: &[f32], width: u32, height: u32) -> Result<()> {
    let pixels = scrgb_to_pq(scrgb);
    let image = image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(width, height, pixels)
        .ok_or_else(|| anyhow::anyhow!("HDR PNG size mismatch"))?;
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    // cICP right after IHDR (8-byte signature + 25-byte IHDR chunk): BT.2020 primaries (9),
    // PQ transfer (16), RGB (0), full range (1).
    let body = [b'c', b'I', b'C', b'P', 9, 16, 0, 1];
    let mut chunk = 4u32.to_be_bytes().to_vec();
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    png.splice(33..33, chunk);
    std::fs::write(path, png)?;
    Ok(())
}
/// scRGB pixels (RGBA, 1.0 = 80 nits, BT.709) to 16-bit RGB in BT.2020 primaries with the PQ
/// curve: the HDR PNG and the HDR10 video take this.
pub fn scrgb_to_pq(scrgb: &[f32]) -> Vec<u16> {
    const TO_2020: [[f32; 3]; 3] = [
        [0.627_404, 0.329_282, 0.043_314],
        [0.069_097, 0.919_540, 0.011_361],
        [0.016_392, 0.088_013, 0.895_595],
    ];
    let pq = |nits: f32| {
        let y = (nits.clamp(0., 10000.) / 10000.).powf(0.159_301_76);
        ((0.835_937_5 + 18.851_563 * y) / (1. + 18.6875 * y)).powf(78.84375)
    };
    let mut pixels = Vec::with_capacity(scrgb.len() / 4 * 3);
    for p in scrgb.as_chunks::<4>().0 {
        for row in TO_2020 {
            let nits = (row[0] * p[0] + row[1] * p[1] + row[2] * p[2]) * 80.;
            pixels.push((pq(nits) * 65535.).round() as u16);
        }
    }
    pixels
}
/// Writes scRGB pixels (RGBA, 1.0 = 80 nits) as a lossless JPEG XR of half floats through the
/// Windows Imaging Component, the format Game Bar and the Snipping Tool use for HDR screenshots.
#[cfg(windows)]
pub fn write_jxr(path: &std::path::Path, scrgb: &[f32], width: u32, height: u32) -> Result<()> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::{HSTRING, PWSTR};
    anyhow::ensure!(
        scrgb.len() == width as usize * height as usize * 4,
        "JPEG XR size mismatch"
    );
    let bytes: Vec<u8> = scrgb
        .iter()
        .enumerate()
        .flat_map(|(i, &v)| {
            // Opaque alpha; negative scRGB stays (it is valid out-of-gamut colour).
            let v = if i % 4 == 3 { 1. } else { v };
            crate::gpu::f32_to_half(v).to_le_bytes()
        })
        .collect();
    unsafe {
        // Already initialised on this thread (any model) is fine.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(
            &HSTRING::from(path.as_os_str()),
            windows::Win32::Foundation::GENERIC_WRITE.0,
        )?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatWmp, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(|| anyhow::anyhow!("No JPEG XR frame"))?;
        if let Some(options) = &options {
            let mut name: Vec<u16> = "Lossless\0".encode_utf16().collect();
            let bag = PROPBAG2 {
                pstrName: PWSTR(name.as_mut_ptr()),
                ..Default::default()
            };
            options.Write(1, &bag, &VARIANT::from(true))?;
        }
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat64bppRGBAHalf;
        frame.SetPixelFormat(&mut format)?;
        anyhow::ensure!(
            format == GUID_WICPixelFormat64bppRGBAHalf,
            "The JPEG XR encoder does not take half floats"
        );
        frame.WritePixels(height, width * 8, &bytes)?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
/// Off Windows the display counts as SDR until the Vulkan backend reads its HDR state.
#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::Monitor;
#[cfg(not(windows))]
pub fn write_jxr(_path: &std::path::Path, _scrgb: &[f32], _width: u32, _height: u32) -> Result<()> {
    anyhow::bail!("JPEG XR needs Windows (WIC)")
}
/// One R10G10B10A2 pixel of the HDR10 path (PQ code values in BT.2020) back to scRGB RGBA
/// (BT.709, 1.0 = 80 nits): readbacks of the HDR10 output and backbuffer go through this.
pub fn hdr10_to_scrgb(word: u32) -> [f32; 4] {
    const TO_709: [[f32; 3]; 3] = [
        [1.660_491, -0.587_641, -0.072_850],
        [-0.124_550, 1.132_9, -0.008_349],
        [-0.018_151, -0.100_579, 1.118_73],
    ];
    let decode = |e: f32| {
        let p = e.clamp(0., 1.).powf(1. / 78.84375);
        10000. * ((p - 0.835_937_5).max(0.) / (18.851_563 - 18.6875 * p)).powf(1. / 0.159_301_76)
    };
    let c = [0, 10, 20].map(|shift| decode(((word >> shift) & 1023) as f32 / 1023.) / 80.);
    let [r, g, b] = TO_709.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2]);
    [r, g, b, (word >> 30) as f32 / 3.]
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::{hdr_map, hdr10_to_scrgb, scrgb_to_pq, tone_curve};
    #[test]
    fn hdr_tone_maps_keep_the_sdr_midtones_and_fill_the_headroom() {
        for mode in 1..6 {
            // Midtones: same display light as SDR at paper white (gamma 2.2 cancels).
            let mid = hdr_map([0.18; 3], 1., 2.2, mode, 200., 1000.)[0];
            let sdr = tone_curve(0.18, mode) * 200. / 80.;
            assert!(
                (mid - sdr).abs() / sdr < 1e-4,
                "mode {mode}: {mid} vs {sdr}"
            );
            // Highlights go above paper white but never past the peak, monotonically.
            let mut last = 0.;
            for i in 0..2000 {
                let v = hdr_map([i as f32 * 0.05; 3], 1., 2.2, mode, 200., 1000.)[0];
                assert!(v >= last - 1e-5 && v <= 12.5 + 1e-3, "mode {mode}: {v}");
                last = v;
            }
            assert!(
                last > 200. / 80. * 1.5,
                "mode {mode} stays at paper white: {last}"
            );
        }
    }
    #[test]
    fn hdr10_decodes_what_the_pq_writer_encodes() {
        // scRGB in, the HDR PNG's 16-bit PQ / BT.2020, truncated to 10 bits, back to scRGB.
        for c in [
            [1., 1., 1.],
            [0.02, 0.5, 3.],
            [12.5, 0.1, 0.],
            [100., 100., 100.],
        ] {
            let pq = scrgb_to_pq(&[c[0], c[1], c[2], 1.]);
            let ten: Vec<u32> = pq
                .iter()
                .map(|&v| ((v as f32 / 65535. * 1023.).round() as u32).min(1023))
                .collect();
            let word = ten[0] | ten[1] << 10 | ten[2] << 20 | 3 << 30;
            let back = hdr10_to_scrgb(word);
            for i in 0..3 {
                // Relative to the brightest channel: the BT.2020 to BT.709 matrix spreads a
                // bright channel's 10-bit step into the dim ones of saturated colours.
                let err = (back[i] - c[i]).abs() / (c[0].max(c[1]).max(c[2]) + 0.01);
                assert!(err < 0.01, "{c:?} -> {back:?}");
            }
            assert_eq!(back[3], 1.);
        }
    }
    #[test]
    fn png_crc_matches_the_spec() {
        // CRC of the IEND chunk type, as in every PNG.
        assert_eq!(super::crc32(b"IEND"), 0xAE42_6082);
    }
    #[test]
    fn hdr_transform_keeps_midtones_and_respects_the_peak() {
        // Paper white 200 nits: scene 0.4 is 80 nits, which scRGB writes as 1.0.
        let gray = hdr_map([0.4; 3], 1., 2.2, 0, 200., 1000.);
        assert!(gray.iter().all(|v| (v - 1.).abs() < 1e-3), "{gray:?}");
        // Far above the panel: clamped to peak / 80.
        let light = hdr_map([1000.; 3], 1., 2.2, 0, 200., 1000.);
        assert!(
            light.iter().all(|v| *v <= 12.5 + 1e-3 && *v > 12.),
            "{light:?}"
        );
        // Monotonic, and hue kept (channel ratios unchanged).
        let mut last = 0.;
        for i in 1..400 {
            let v = hdr_map([i as f32 * 0.05; 3], 1., 2.2, 0, 200., 1000.)[0];
            assert!(v >= last - 1e-5);
            last = v;
        }
        let tinted = hdr_map([20., 10., 5.], 1., 2.2, 0, 200., 1000.);
        assert!(
            (tinted[0] / tinted[1] - 2.).abs() < 1e-3 && (tinted[1] / tinted[2] - 2.).abs() < 1e-3
        );
    }
}
