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

static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Vec<f32>>>>> = LazyLock::new(Default::default);

/// The profile of the file at `path` (resolved like mesh paths), cached; None if it can't be
/// read (the light then shines without one).
pub fn profile(path: &str) -> Option<Vec<f32>> {
    let full = crate::mesh::resolve(path);
    let mut cache = CACHE.lock().ok()?;
    cache
        .entry(full.clone())
        .or_insert_with(|| {
            let text = std::fs::read_to_string(&full).ok()?;
            parse(&text)
                .inspect_err(|e| eprintln!("{}: {e:#}", full.display()))
                .ok()
        })
        .clone()
}
/// Forgets cached profiles (a file was replaced on disk).
pub fn clear_cache() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
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
