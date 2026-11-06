//! IES light profiles (IESNA LM-63; engine-notes "Closing the medium term, wave 2"): a file's
//! candela table becomes the relative intensity over 0–90° from the light's normal, averaged over
//! the horizontal angles, as `SAMPLES` values peaking at 1 (`lighting` packs them into the
//! light's material, `ies_profile` in pathtrace.hlsl reads them).
use anyhow::{Context, ensure};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// Samples over 0–90°, evenly spaced (1.5°), the first at the normal.
pub const SAMPLES: usize = 60;

/// Parses an LM-63 file (1986, 1991, 1995, 2002) into the profile.
pub fn parse(text: &str) -> anyhow::Result<Vec<f32>> {
    let rest = text
        .split_once("TILT=")
        .context("IES: no TILT line")?
        .1
        .split_once('\n')
        .context("IES: nothing after TILT")?;
    let tilt = rest.0.trim();
    let mut nums = rest
        .1
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().context("IES: not a number"));
    let mut next = || nums.next().context("IES: file ends early")?;
    // TILT=INCLUDE: lamp-to-luminaire geometry, then n angles and n multipliers (ignored: they
    // depend on the fixture's tilt, which the profile does not model).
    if tilt == "INCLUDE" {
        next()?;
        let n = next()? as usize;
        ensure!(n < 1000, "IES: bad tilt table");
        for _ in 0..2 * n {
            next()?;
        }
    }
    let _lamps = next()?;
    let _lumens = next()?;
    let multiplier = next()?;
    let nv = next()? as usize;
    let nh = next()? as usize;
    let kind = next()? as u32;
    ensure!(
        (2..=1000).contains(&nv) && (1..=1000).contains(&nh),
        "IES: bad angle counts"
    );
    ensure!(kind == 1, "IES: only type C photometry is supported");
    for _ in 0..7 {
        next()?; // units, width, length, height, ballast, future use, watts
    }
    let vertical = (0..nv)
        .map(|_| next())
        .collect::<anyhow::Result<Vec<_>>>()?;
    let horizontal = (0..nh)
        .map(|_| next())
        .collect::<anyhow::Result<Vec<_>>>()?;
    // Each horizontal plane weighs the angular span it stands for (trapezoid rule), so uneven
    // steps don't over-weight the dense planes. The scale cancels in the normalization below.
    let weight = |j: usize| -> f64 {
        if nh == 1 {
            return 1.;
        }
        let lo = horizontal[j.saturating_sub(1)];
        let hi = horizontal[(j + 1).min(nh - 1)];
        ((hi - lo) / 2.).max(0.)
    };
    let mut sum = vec![0.; nv];
    for j in 0..nh {
        let w = weight(j);
        for s in &mut sum {
            *s += next()? * multiplier * w;
        }
    }
    ensure!(
        vertical.windows(2).all(|w| w[1] > w[0]),
        "IES: vertical angles must increase"
    );
    // Type C: vertical 0° points along the light's axis (nadir for a downlight).
    let at = |deg: f64| -> f64 {
        if deg < vertical[0] || deg > vertical[nv - 1] {
            return 0.;
        }
        let i = vertical.partition_point(|&v| v <= deg).clamp(1, nv - 1);
        let (a, b) = (vertical[i - 1], vertical[i]);
        let f = ((deg - a) / (b - a)).clamp(0., 1.);
        sum[i - 1] + (sum[i] - sum[i - 1]) * f
    };
    let raw: Vec<f64> = (0..SAMPLES)
        .map(|i| at(90. * i as f64 / (SAMPLES - 1) as f64).max(0.))
        .collect();
    let peak = raw.iter().cloned().fold(0., f64::max);
    ensure!(peak > 0., "IES: no light below the horizon");
    Ok(raw.iter().map(|v| (v / peak) as f32).collect())
}

/// A flashlight's beam instead of a file: `beam:` half angle, corona, spill (`beam_spec`).
const BEAM: &str = "beam:";
pub fn beam_spec(half_angle: f32, corona: f32, spill: f32) -> String {
    format!("{BEAM}{half_angle},{corona},{spill}")
}
/// A reflector's beam: a Gaussian hot spot that falls to half at the half angle, the slight ring
/// a smooth reflector leaves round it, a corona 2.5 times as wide, and flat spill from the bare
/// LED that the bezel cuts off.
fn beam(spec: &str) -> Option<Vec<f32>> {
    let mut v = spec.split(',').map(|t| t.parse::<f32>().ok());
    let (half, corona, spill) = (v.next()??, v.next()??, v.next()??);
    let sigma = half / 1.1774;
    let wide = 2.5 * half;
    let cut = (4. * half).max(25.);
    let gauss = |a: f32, c: f32, s: f32| (-0.5 * ((a - c) / s).powi(2)).exp();
    let at = |a: f32| {
        let edge = ((cut + 5. - a) / 5.).clamp(0., 1.);
        (1. - corona - spill).max(0.) * gauss(a, 0., sigma)
            + 0.03 * gauss(a, 1.7 * half, 0.3 * half)
            + corona * gauss(a, 0., wide / 1.1774)
            + spill * edge * edge * (3. - 2. * edge)
    };
    let raw: Vec<f32> = (0..SAMPLES)
        .map(|i| at(90. * i as f32 / (SAMPLES - 1) as f32))
        .collect();
    let peak = raw.iter().cloned().fold(0., f32::max);
    (peak > 0.).then(|| raw.iter().map(|v| v / peak).collect())
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Vec<f32>>>>> = LazyLock::new(Default::default);
/// Why the files with no profile in `CACHE` could not be read, for the UI.
static ERRORS: LazyLock<Mutex<HashMap<PathBuf, String>>> = LazyLock::new(Default::default);

/// The profile of the file at `path` (resolved like mesh paths), cached; None if it can't be
/// read (the light then shines without one).
pub fn profile(path: &str) -> Option<Vec<f32>> {
    if let Some(spec) = path.strip_prefix(BEAM) {
        return beam(spec);
    }
    let full = crate::mesh::resolve(path);
    let mut cache = CACHE.lock().ok()?;
    cache
        .entry(full.clone())
        .or_insert_with(|| {
            let parsed = std::fs::read_to_string(&full)
                .map_err(anyhow::Error::from)
                .and_then(|text| parse(&text));
            match parsed {
                Ok(profile) => Some(profile),
                Err(e) => {
                    eprintln!("{}: {e:#}", full.display());
                    if let Ok(mut errors) = ERRORS.lock() {
                        errors.insert(full.clone(), format!("{e:#}"));
                    }
                    None
                }
            }
        })
        .clone()
}
/// Why `path` has no profile, or None when it has one.
pub fn load_error(path: &str) -> Option<String> {
    if profile(path).is_some() {
        return None;
    }
    let known = ERRORS
        .lock()
        .ok()
        .and_then(|errors| errors.get(&crate::mesh::resolve(path)).cloned());
    Some(known.unwrap_or_else(|| "Not a usable light profile".into()))
}
/// Forgets cached profiles (a file was replaced on disk).
pub fn clear_cache() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
    }
    if let Ok(mut e) = ERRORS.lock() {
        e.clear();
    }
}

/// The profile as 20 floats, three 8-bit samples each (integer values below 2^24, so exact and
/// never NaN or denormal), in the order the shader reads them.
pub fn pack(profile: &[f32]) -> [f32; 20] {
    std::array::from_fn(|k| {
        (0..3).fold(0u32, |acc, j| {
            let v = profile.get(3 * k + j).copied().unwrap_or(0.);
            acc | ((v.clamp(0., 1.) * 255.).round() as u32) << (8 * j)
        }) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_beam_has_a_hot_spot_a_corona_and_an_end() {
        let p = profile(&beam_spec(5., 0.1, 0.006)).unwrap();
        let at = |deg: f32| p[(deg / 90. * (SAMPLES - 1) as f32).round() as usize];
        assert_eq!(p.len(), SAMPLES);
        assert!((p[0] - 1.).abs() < 1e-4);
        assert!(
            (0.4..0.65).contains(&at(5.)),
            "half at the half angle: {}",
            at(5.)
        );
        assert!((0.01..0.1).contains(&at(15.)), "corona: {}", at(15.));
        assert!(at(24.) < 0.02 && at(24.) > 0., "spill: {}", at(24.));
        assert!(at(40.) < 1. / 510., "past the bezel: {}", at(40.));
        assert!(beam("x,1,2").is_none());
    }
    #[test]
    fn parses_a_type_c_file() {
        // A downlight: full intensity to 30°, half at 45°, none from 60°; two horizontal planes
        // that average to the same.
        let text = "IESNA:LM-63-2002\n[TEST] x\nTILT=NONE\n1 1000 1 4 2 1 2 0.1 0.1 0\n1 1 50\n0 30 45 60\n0 90\n100 100 50 0\n100 100 50 0\n";
        let p = parse(text).unwrap();
        assert_eq!(p.len(), SAMPLES);
        assert!((p[0] - 1.).abs() < 1e-6);
        let at = |deg: f32| p[(deg / 90. * (SAMPLES - 1) as f32).round() as usize];
        assert!((at(45.) - 0.5).abs() < 0.03, "{}", at(45.));
        assert_eq!(at(75.), 0.);
        let packed = pack(&p);
        assert_eq!(packed[0] as u32 & 255, 255);
        assert!(parse("no tilt here").is_err());
    }
    #[test]
    fn uneven_horizontal_planes_weigh_their_span() {
        // Planes at 0°, 10° and 90° span 5°, 45° and 40°: only the last is lit at 45°.
        let text = "IESNA:LM-63-2002
TILT=NONE
1 1000 1 3 3 1 2 0.1 0.1 0
1 1 50
0 45 90
0 10 90
100 0 0
100 0 0
100 100 0
";
        let p = parse(text).unwrap();
        let at = |deg: f32| p[(deg / 90. * (SAMPLES - 1) as f32).round() as usize];
        assert!((at(45.) - 40. / 90.).abs() < 0.02, "{}", at(45.));
    }
}
