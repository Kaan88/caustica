//! CPU side of spectral colours (engine-notes "Closing the medium term, wave 2"): blackbody and
//! measured spectra as the linear sRGB the RGB tracers use, integrated exactly as the spectral
//! tracer's output does (`shaders/spectral_tables.hlsli`, same tables), so both agree.
use crate::spectral_tables::{COUNT, D65, LO, MEASURED, OUT, STEP};

/// Measured spectra (`Material::spectrum`): 1–3 the Cornell box's white, green and red walls,
/// 4 its light.
pub const MEASURED_NAMES: [&str; 5] = [
    "None",
    "Cornell white",
    "Cornell green",
    "Cornell red",
    "Cornell light",
];
/// Blackbody temperatures the UI and validation allow, in kelvin.
pub const KELVIN: std::ops::RangeInclusive<f32> = 1000. ..=40000.;

/// Linear sRGB of a spectrum given on the table grid (the shader's trapezoid rule).
fn rgb(f: impl Fn(usize) -> f32) -> [f32; 3] {
    let mut c = [0.; 3];
    for (i, out) in OUT.iter().enumerate() {
        let w = if i == 0 || i == COUNT - 1 {
            STEP / 2.
        } else {
            STEP
        } * f(i);
        for (v, o) in c.iter_mut().zip(out) {
            *v += o * w;
        }
    }
    c
}
/// Planck's law up to a constant, at `nm` for `kelvin`.
fn planck(nm: f32, kelvin: f32) -> f32 {
    const C2: f32 = 1.438_777e7; // nm·K
    let l = nm as f64;
    (1. / (l.powi(5) * ((C2 as f64 / (l * kelvin as f64)).exp() - 1.))) as f32
}
/// The spectral tracer's blackbody value at grid entry i: Planck relative to 560 nm, over D65
/// (emission spectra are relative to D65; the shader's `blackbody`).
fn blackbody_at(i: usize, kelvin: f32) -> f32 {
    let nm = LO + STEP * i as f32;
    planck(nm, kelvin) / planck(560., kelvin) / D65[i]
}
/// The colour of a blackbody at `kelvin`: linear sRGB of the spectral tracer's spectrum for it
/// (unnormalized: the shader's spectrum times `scale` gives `rgb × scale`).
pub fn blackbody(kelvin: f32) -> [f32; 3] {
    rgb(|i| blackbody_at(i, kelvin))
}
/// A blackbody colour scaled to peak 1 and the factor that did it (the material's `phase.w`).
pub fn blackbody_peak(kelvin: f32) -> ([f32; 3], f32) {
    let c = blackbody(kelvin);
    let s = 1. / c.iter().fold(1e-12f32, |a, &v| a.max(v));
    (c.map(|v| v * s), s)
}
/// The colour of measured spectrum `code` (1–4) under D65.
pub fn measured(code: u32) -> [f32; 3] {
    let k = (code as usize).clamp(1, 4) - 1;
    rgb(|i| MEASURED[i][k])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blackbody_colours() {
        // D65's correlated colour temperature is about 6504 K: near white.
        let (c, _) = blackbody_peak(6504.);
        assert!(c.iter().all(|&v| v > 0.9), "{c:?}");
        // Candles are orange, blue sky is blue.
        let (warm, _) = blackbody_peak(1900.);
        assert!(warm[0] > 0.999 && warm[2] < 0.1, "{warm:?}");
        let (cold, _) = blackbody_peak(20000.);
        assert!(cold[2] > 0.999 && cold[0] < 0.8, "{cold:?}");
        // The measured walls keep their look.
        let [r, g, b] = measured(3);
        assert!(r > 0.4 && g < 0.1 && b < 0.1);
        assert!((measured(1)[1] - 0.736).abs() < 0.01);
    }
}
