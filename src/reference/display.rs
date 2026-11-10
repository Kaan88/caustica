//! The display transform on the CPU, for images written with no device (`--reference out.png`,
//! CPU video and high-resolution stills): the same model as post.hlsl's beauty view (halation and
//! vignette on scene light, exposure, the tone map, the film look with its LUT, the output gamma,
//! then grain), written from its definitions rather than its code. The A/B split line, zebras
//! and HDR outputs are the screen's, not a file's. The software view on the GPU ignores the
//! direct / indirect sliders (they need the GPU's direct-light AOV), and so does this.
use crate::scene::Settings;
use anyhow::{Context, Result};
use glam::Vec3;

/// A film-look LUT: `size`³ RGB, red fastest (as in .cube files).
pub struct Lut {
    size: usize,
    data: Vec<[f32; 3]>,
}
impl Lut {
    /// The scene's LUT (`lut_path`, resolved as the app resolves it), or none.
    pub fn load(s: &Settings) -> Result<Option<Lut>> {
        if s.lut_path.is_empty() {
            return Ok(None);
        }
        let path = crate::mesh::resolve(&s.lut_path);
        let path = if path.exists() {
            path
        } else {
            s.lut_path.clone().into()
        };
        let text = std::fs::read_to_string(&path).with_context(|| format!("LUT {}", s.lut_path))?;
        let (size, data) = crate::look::read_cube(&text)?;
        Ok(Some(Lut {
            size: size as usize,
            data,
        }))
    }
    fn at(&self, x: usize, y: usize, z: usize) -> Vec3 {
        let n = self.size - 1;
        let [r, g, b] = self.data[(z.min(n) * self.size + y.min(n)) * self.size + x.min(n)];
        Vec3::new(r, g, b)
    }
    /// Trilinear lookup of an encoded colour in 0..1.
    fn sample(&self, e: Vec3) -> Vec3 {
        let x = e.clamp(Vec3::ZERO, Vec3::ONE) * (self.size - 1) as f32;
        let b = x.floor();
        let f = x - b;
        let [bx, by, bz] = [b.x as usize, b.y as usize, b.z as usize];
        let along_x = |y, z| self.at(bx, y, z).lerp(self.at(bx + 1, y, z), f.x);
        let near = along_x(by, bz).lerp(along_x(by + 1, bz), f.y);
        let far = along_x(by, bz + 1).lerp(along_x(by + 1, bz + 1), f.y);
        near.lerp(far, f.z)
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}
fn saturate(c: Vec3) -> Vec3 {
    c.clamp(Vec3::ZERO, Vec3::ONE)
}
fn powf(c: Vec3, e: f32) -> Vec3 {
    Vec3::new(c.x.powf(e), c.y.powf(e), c.z.powf(e))
}
fn smoothstep(c: Vec3) -> Vec3 {
    let t = saturate(c);
    t * t * (3. - 2. * t)
}
/// Hable's filmic curve (Uncharted 2).
fn filmic(x: f32) -> f32 {
    (x * (0.15 * x + 0.05) + 0.004) / (x * (0.15 * x + 0.5) + 0.06) - 0.066_666_67
}
fn aces(c: Vec3) -> Vec3 {
    saturate(c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14))
}
/// AgX: Benjamin Wrensch's polynomial fit of Troy Sobotka's base, from log2 exposure in
/// -12.47..4.03 stops; the matrices are its inset and outset (rows).
fn agx(c: Vec3) -> Vec3 {
    let inset = [
        [0.842479, 0.0784336, 0.0792237],
        [0.0423282, 0.878468, 0.0791661],
        [0.0423756, 0.0784336, 0.879142],
    ];
    let outset = [
        [1.19688, -0.0980209, -0.0990297],
        [-0.0528969, 1.15190, -0.0989612],
        [-0.0529716, -0.0980435, 1.15107],
    ];
    let mul = |m: [[f32; 3]; 3], v: Vec3| {
        Vec3::new(
            Vec3::from(m[0]).dot(v),
            Vec3::from(m[1]).dot(v),
            Vec3::from(m[2]).dot(v),
        )
    };
    let l = mul(inset, c).max(Vec3::splat(1e-10));
    let l = Vec3::new(l.x.log2(), l.y.log2(), l.z.log2())
        .clamp(Vec3::splat(-12.47393), Vec3::splat(4.026069));
    let x = (l + 12.47393) / 16.5;
    let (x2, x4) = (x * x, x * x * x * x);
    let p =
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - 0.00232;
    saturate(powf(mul(outset, p).max(Vec3::ZERO), 2.2))
}
/// Khronos PBR Neutral: base colours kept, highlights compressed toward white.
fn pbr_neutral(c: Vec3) -> Vec3 {
    let start = 0.8 - 0.04;
    let low = c.min_element();
    let c = c - if low < 0.08 {
        low - 6.25 * low * low
    } else {
        0.04
    };
    let peak = c.max_element();
    if peak < start {
        return c;
    }
    let d = 1. - start;
    let new_peak = 1. - d * d / (peak + d - start);
    let g = 1. - 1. / (0.15 * (peak - new_peak) + 1.);
    (c * (new_peak / peak)).lerp(Vec3::splat(new_peak), g)
}
/// Exposed scene light to display light (1 = white) by tone map `mode` (`Settings::tonemap`).
pub fn tone_map(c: Vec3, mode: u32) -> Vec3 {
    match mode {
        1 => saturate(c / (1. + c)),
        2 => saturate(c * (1. + c / 16.) / (1. + c)),
        3 => aces(c),
        4 => {
            saturate(Vec3::new(filmic(2. * c.x), filmic(2. * c.y), filmic(2. * c.z)) / filmic(11.2))
        }
        5 => smoothstep(aces(c)),
        6 => agx(c),
        7 => saturate(pbr_neutral(c)),
        _ => saturate(c),
    }
}
/// Lift, gamma and gain on the display-encoded value (1/2.2), then the LUT by its strength.
fn film_look(s: &Settings, lut: Option<&Lut>, t: Vec3) -> Vec3 {
    let e = powf(saturate(t), 1. / 2.2);
    let lift = Vec3::from(s.lift);
    let e = Vec3::from(s.gain) * (e + lift * (1. - e));
    let g = Vec3::from(s.grade_gamma).max(Vec3::splat(0.05));
    let mut e = saturate(e);
    e = Vec3::new(e.x.powf(1. / g.x), e.y.powf(1. / g.y), e.z.powf(1. / g.z));
    if let Some(lut) = lut.filter(|l| l.size > 1) {
        e = e.lerp(lut.sample(e), s.lut_strength);
    }
    powf(saturate(e), 2.2)
}
/// Film grain's per-cell noise in -0.5..0.5: cells of `size` pixels, `seed` per frame.
fn grain_noise(x: u32, y: u32, size: u32, seed: u32) -> f32 {
    let (cx, cy) = (x / size.max(1), y / size.max(1));
    let mut h = cx
        .wrapping_mul(1973)
        .wrapping_add(cy.wrapping_mul(9277))
        .wrapping_add(seed.wrapping_mul(26699));
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535. - 0.5
}

/// A film / lens threshold given in exposed units (1 = white after exposure) in scene units, at
/// the displayed `exposure` (post.hlsl `exposed_threshold`): halation and glare catch the same
/// highlights whatever the meter picks. Exposure 1 leaves it unchanged.
pub fn exposed_threshold(t: f32, exposure: f32) -> f32 {
    t / exposure.max(1e-30)
}

/// What a file of the display would hold: RGBA8 of linear RGBA `linear` (`size`, rows top first).
/// `seed` is the grain's frame number. Glare's diffraction (an FFT of the whole frame) is not
/// here yet, so a look with glare is refused rather than written without it. A still: with
/// `auto_exposure` the meter settles on this image (`still_exposure`).
pub fn display(s: &Settings, linear: &[f32], size: [u32; 2], seed: u32) -> Result<Vec<u8>> {
    display_exposed(s, linear, size, seed, still_exposures(s, linear, size))
}
/// `display` at given linear exposures, displayed and metered (the video export's adapted ones);
/// the metered one (no `ev_bias`) drives the Eyes look.
pub fn display_exposed(
    s: &Settings,
    linear: &[f32],
    size: [u32; 2],
    seed: u32,
    [exposure, metered]: [f32; 2],
) -> Result<Vec<u8>> {
    let eyes = eyes_mix(s, metered);
    anyhow::ensure!(
        s.glare <= 0.,
        "The CPU display transform has no glare yet (set glare=0, or write an EXR)"
    );
    let lut = Lut::load(s)?;
    let [w, h] = size.map(|v| v as usize);
    anyhow::ensure!(linear.len() == w * h * 4, "image size");
    let scene_at = |x: i32, y: i32| {
        let i = (y.clamp(0, h as i32 - 1) as usize * w + x.clamp(0, w as i32 - 1) as usize) * 4;
        Vec3::new(linear[i], linear[i + 1], linear[i + 2])
    };
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let mut c = scene_at(x as i32, y as i32);
        // Halation: light above white after exposure (post.hlsl `exposed_threshold`), from 24
        // taps on two rings, glowing red-orange.
        if s.halation > 0. {
            let (r, th) = (h as f32 * 0.012, exposed_threshold(1., exposure));
            let mut glow = Vec3::ZERO;
            for i in 0..12 {
                let a = i as f32 * std::f32::consts::PI / 6.;
                for k in 1..=2 {
                    let o = Vec3::new(a.cos(), a.sin(), 0.) * r * k as f32;
                    // Truncated toward zero, as a float-to-int conversion is on the GPU.
                    glow += (scene_at(x as i32 + o.x as i32, y as i32 + o.y as i32) - th)
                        .max(Vec3::ZERO);
                }
            }
            c += glow / 24. * s.halation * Vec3::new(1., 0.3, 0.12);
        }
        if s.vignette > 0. {
            let qx = (x as f32 / w as f32 - 0.5) * (w as f32 / h as f32);
            let qy = y as f32 / h as f32 - 0.5;
            let v = 1. / (1. + (qx * qx + qy * qy) * 1.6);
            c *= 1. + (v * v - 1.) * s.vignette;
        }
        if eyes > 0. {
            c = eyes_look(c, eyes);
        }
        let exposed = (c * exposure).max(Vec3::ZERO);
        let t = tone_map(exposed, s.tonemap);
        let mut d = powf(film_look(s, lut.as_ref(), t), 1. / s.gamma.max(0.1));
        // Grain on the displayed value, strongest in the midtones.
        if s.grain > 0. {
            let size = [1, 2, 3][s.grain_size.min(2) as usize];
            let g = grain_noise(x as u32, y as u32, size, seed) * s.grain * 0.18;
            let l = luminance(saturate(d));
            d = saturate(d + g * (4. * l * (1. - l) + 0.15));
        }
        let q = |v: f32| (v.clamp(0., 1.) * 255. + 0.5) as u8;
        [q(d.x), q(d.y), q(d.z), 255]
    };
    let mut out = vec![0u8; w * h * 4];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = h.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk, band) in out.chunks_mut(rows * w * 4).enumerate() {
            let pixel = &pixel;
            scope.spawn(move || {
                for (i, p) in band.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let at = chunk * rows * w + i;
                    *p = pixel(at % w, at / w);
                }
            });
        }
    });
    Ok(out)
}

// ---- The Eyes look (E4) ----------------------------------------------------------------------
// Rod vision in the dark, mirrored by post.hlsl's `eyes_mix` / `eyes_look`. engine-notes "The Eyes
// look (E4)" has the reasoning; the constants are defaults, changeable.

/// Adapted luminances (cd/m²) where rod vision starts (above: nothing changes) and is complete.
pub const EYES_PHOTOPIC_CD: f32 = 3.;
pub const EYES_SCOTOPIC_CD: f32 = 0.01;
/// Rod (scotopic, V'(λ)) response from linear Rec. 709, normalised so white keeps its luminance:
/// rods peak near 507 nm, so green and blue count and red barely does.
pub const EYES_ROD: [f32; 3] = [0.04, 0.53, 0.43];
/// The Purkinje tint of rod vision (a mild blue shift), Rec. 709 luminance 1.
pub const EYES_TINT: [f32; 3] = [0.80, 1.00, 1.30];

/// How much of the rod look applies, 0..1: none above `EYES_PHOTOPIC_CD`, all of
/// `eyes_strength` below `EYES_SCOTOPIC_CD`, smooth in log luminance between. The adapted
/// luminance is middle grey over the metered exposure (the manual one without the meter).
pub fn eyes_mix(s: &Settings, metered: f32) -> f32 {
    if !s.eyes || s.eyes_strength <= 0. {
        return 0.;
    }
    let cd = crate::photometry::scene_to_cd(crate::photometry::MIDDLE_GREY / metered.max(1e-30));
    let t = ((EYES_PHOTOPIC_CD.log10() - cd.max(1e-30).log10())
        / (EYES_PHOTOPIC_CD.log10() - EYES_SCOTOPIC_CD.log10()))
    .clamp(0., 1.);
    t * t * (3. - 2. * t) * s.eyes_strength
}
/// Scene light `c` toward rod vision by `m`: its scotopic luminance in the Purkinje tint.
pub fn eyes_look(c: Vec3, m: f32) -> Vec3 {
    let rod = c.max(Vec3::ZERO).dot(Vec3::from(EYES_ROD));
    let tint = Vec3::from(EYES_TINT) / luminance(Vec3::from(EYES_TINT));
    c + (tint * rod - c) * m
}

// ---- Exposure meter and adaptation (E3) -------------------------------------------------------
// The CPU twin of post.hlsl's `meter_pass` / `meter_update`: the same sample grid, weights, bins,
// trimmed mean and adaptation, so the CPU export and the GPU agree. engine-notes "Exposure meter
// and adaptation (E3)" has the reasoning; every constant here is a default, changeable.

/// Histogram bins over log2 scene luminance, from `METER_LOG_MIN` to `METER_LOG_MAX` (half a stop
/// each). post.hlsl mirrors the four numbers below.
pub const METER_BINS: usize = 128;
pub const METER_LOG_MIN: f32 = -40.;
pub const METER_LOG_MAX: f32 = 24.;
/// Where a sample sits inside its bin, in 1/`METER_FRAC` steps (summed per bin).
pub const METER_FRAC: u32 = 64;
/// One sample per `METER_STEP`² pixels, at the centre of each cell.
pub const METER_STEP: u32 = 4;
/// The weighted share ignored at the dark end (noise, black backgrounds) and kept below the
/// bright end (the top 3%: visible lamps and the sun).
pub const METER_LOW: f32 = 0.01;
pub const METER_HIGH: f32 = 0.97;
/// Accumulated samples before the meter reads (fewer: it keeps adapting to its last reading).
pub const METER_MIN_SAMPLES: u32 = 4;
/// Frames under `METER_MIN_SAMPLES` in a row after which it reads anyway (a moving camera).
pub const METER_LOW_FRAMES: u32 = 4;
/// Time constants in seconds at the default speed (2/s): to a brighter scene (the exposure falls),
/// to a darker one, and to a darker one whose metered luminance is scotopic.
pub const ADAPT_BRIGHTER: f32 = 0.2;
pub const ADAPT_DARKER: f32 = 3.;
pub const ADAPT_SCOTOPIC: f32 = 10.;
/// `exposure_speed` at which the time constants above hold; faster speeds shorten them.
pub const ADAPT_SPEED: f32 = 2.;

/// A sample's weight (0-8) at output pixel `q` of `size`.
pub fn meter_weight(q: [u32; 2], size: [u32; 2], mode: crate::scene::MeterMode) -> u32 {
    use crate::scene::MeterMode;
    let u = (q[0] as f32 + 0.5) / size[0] as f32 * 2. - 1.;
    let v = (q[1] as f32 + 0.5) / size[1] as f32 * 2. - 1.;
    let r2 = u * u + v * v;
    match mode {
        MeterMode::CentreWeighted => 1 + (7. * (1. - r2).clamp(0., 1.) + 0.5) as u32,
        MeterMode::Spot => {
            if r2 < 0.04 {
                8
            } else {
                0
            }
        }
        MeterMode::Histogram => 8,
    }
}
/// The bin and the in-bin step of a luminance; none for black (or NaN), which the meter skips:
/// an unlit background is no light rather than a dim one.
pub fn meter_bin(l: f32) -> Option<(usize, u32)> {
    if l.is_nan() || l <= METER_LOG_MIN.exp2() {
        return None;
    }
    let t = ((l.log2() - METER_LOG_MIN) * (METER_BINS as f32 / (METER_LOG_MAX - METER_LOG_MIN)))
        .clamp(0., METER_BINS as f32 - 0.001);
    let b = t as usize;
    let f = (((t - b as f32) * METER_FRAC as f32) as u32).min(METER_FRAC - 1);
    Some((b, f))
}
/// The meter's histogram of linear RGBA `linear` (`size`): weighted counts in 0..128, weighted
/// in-bin steps in 128..256 (post.hlsl row 260 of u28).
pub fn meter_histogram(linear: &[f32], size: [u32; 2], mode: crate::scene::MeterMode) -> Vec<u32> {
    let mut h = vec![0u32; 2 * METER_BINS];
    let grid = size.map(|v| v.div_ceil(METER_STEP));
    for gy in 0..grid[1] {
        for gx in 0..grid[0] {
            let q = [
                (gx * METER_STEP + METER_STEP / 2).min(size[0] - 1),
                (gy * METER_STEP + METER_STEP / 2).min(size[1] - 1),
            ];
            let w = meter_weight(q, size, mode);
            let i = (q[1] as usize * size[0] as usize + q[0] as usize) * 4;
            let l = crate::photometry::luminance([linear[i], linear[i + 1], linear[i + 2]]);
            if let (true, Some((b, f))) = (w > 0, meter_bin(l)) {
                h[b] += w;
                h[METER_BINS + b] += w * f;
            }
        }
    }
    h
}
/// A reading: the exposure in stops that puts the metered luminance at middle grey, and that
/// luminance (log2), from a histogram; none when nothing lit was sampled. Centre-weighted and spot
/// take the weighted mean of log2 luminance between the low and high cuts; histogram mode the
/// median.
pub fn meter_reading(h: &[u32], mode: crate::scene::MeterMode) -> Option<(f32, f32)> {
    let total = h[..METER_BINS].iter().map(|&c| c as f32).sum::<f32>();
    if total <= 0. {
        return None;
    }
    let pos = |b: usize| {
        let c = h[b] as f32;
        b as f32 + (h[METER_BINS + b] as f32 / c + 0.5) / METER_FRAC as f32
    };
    let mut at = 0.;
    if mode == crate::scene::MeterMode::Histogram {
        let mut cum = 0.;
        for (b, &c) in h[..METER_BINS].iter().enumerate() {
            cum += c as f32;
            if c > 0 && cum >= total * 0.5 {
                at = pos(b);
                break;
            }
        }
    } else {
        let (lo, hi) = (total * METER_LOW, total * METER_HIGH);
        let (mut cum, mut acc) = (0f32, 0f32);
        for (b, &c) in h[..METER_BINS].iter().enumerate() {
            let c = c as f32;
            if c > 0. {
                let kept = (cum + c).min(hi) - cum.max(lo);
                if kept > 0. {
                    acc += kept * pos(b);
                }
            }
            cum += c;
        }
        at = acc / (hi - lo);
    }
    let log2 = METER_LOG_MIN + at * ((METER_LOG_MAX - METER_LOG_MIN) / METER_BINS as f32);
    let ev = (crate::photometry::MIDDLE_GREY.log2() - log2)
        .clamp(crate::scene::EXPOSURE_EV_MIN, crate::scene::EXPOSURE_EV_MAX);
    Some((ev, log2))
}

/// What one display step does with the meter (post.hlsl `meter_ctrl.y`; `Freeze` dispatches
/// nothing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeterOp {
    /// Read and adapt toward the reading over `dt`.
    #[default]
    Adapt,
    /// Read and jump to it (stills, `--capture`, headless, the first frame of a video, the
    /// frame the accumulation reaches its cap).
    Settle,
    /// No reading (too few samples yet): keep adapting toward the last one.
    Hold,
    /// Nothing changes (converged).
    Freeze,
}
impl MeterOp {
    pub fn code(self) -> u32 {
        self as u32
    }
}
/// The adaptation state (post.hlsl keeps the same five words in row 261 of u28). `current` is
/// the metered exposure in stops; the displayed one adds `ev_bias`. `base` is where this frame's
/// adaptation started: a step that does not `commit` adapts from it again, so the passes of one
/// video frame give the frame one step of `dt` whatever their count.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Adaptation {
    pub valid: bool,
    pub base: f32,
    pub current: f32,
    pub target: f32,
    /// log2 of the metered luminance (scene units), for the scotopic time constant.
    pub scene_log2: f32,
}
/// The time constant from `from` toward `to` (stops), at `speed` (1/s).
pub fn adapt_tau(from: f32, to: f32, scene_log2: f32, speed: f32) -> f32 {
    let tau = if to < from {
        ADAPT_BRIGHTER
    } else if scene_log2 < crate::photometry::SCOTOPIC_BELOW.log2() {
        ADAPT_SCOTOPIC
    } else {
        ADAPT_DARKER
    };
    tau * ADAPT_SPEED / speed.max(1e-3)
}
impl Adaptation {
    /// One display step: `reading` from `meter_reading` (ignored by `Hold`), `dt` simulated
    /// seconds (1/fps in a video, the app's frame step live; never wall time in an export).
    pub fn step(
        &mut self,
        reading: Option<(f32, f32)>,
        op: MeterOp,
        commit: bool,
        dt: f32,
        speed: f32,
    ) {
        if op == MeterOp::Freeze {
            return;
        }
        let reading = reading.filter(|_| op != MeterOp::Hold);
        if let Some((ev, l)) = reading {
            (self.target, self.scene_log2) = (ev, l);
        }
        if !self.valid {
            // The first reading settles: there is nothing to adapt from.
            if reading.is_some() {
                (self.base, self.current, self.valid) = (self.target, self.target, true);
            }
            return;
        }
        if commit {
            self.base = self.current;
        }
        if op == MeterOp::Settle {
            if reading.is_some() {
                (self.base, self.current) = (self.target, self.target);
            }
            return;
        }
        let tau = adapt_tau(self.base, self.target, self.scene_log2, speed);
        let k = 1. - (-dt.max(0.) / tau).exp();
        self.current = self.base + (self.target - self.base) * k;
    }
    /// The displayed linear exposure: metered plus `ev_bias` in auto, else the manual one (also
    /// before the first reading).
    pub fn exposure(&self, s: &Settings) -> f32 {
        if s.auto_exposure && self.valid {
            (self.current + s.ev_bias).exp2()
        } else {
            s.exposure
        }
    }
    /// The metered linear exposure without `ev_bias` (the manual one in manual or before a
    /// reading): what the Eyes look takes the adapted luminance from, as post.hlsl's
    /// `metered_exposure`.
    pub fn metered(&self, s: &Settings) -> f32 {
        if s.auto_exposure && self.valid {
            self.current.exp2()
        } else {
            s.exposure
        }
    }
}
/// The manual exposure that shows what auto exposure shows now (E7b): `2^(current + ev_bias)`
/// from the last valid adaptation, for the auto to manual switch (the checkbox, AE-L, the
/// remote, the console) so the image does not jump. None before a reading.
pub fn manual_from_auto(m: Option<Adaptation>, ev_bias: f32) -> Option<f32> {
    m.filter(|m| m.valid).map(|m| {
        (m.current + ev_bias)
            .exp2()
            .clamp(0., crate::scene::EXPOSURE_MAX)
    })
}
/// What the auto to manual seed compares between frames (E7b): it seeds only when auto alone
/// went off, so loading a scene, a preset or an undo that also sets the exposure keeps theirs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExposureSeen {
    pub auto: bool,
    pub exposure: f32,
    pub ev_bias: f32,
    pub scene: String,
}
impl ExposureSeen {
    pub fn of(s: &Settings) -> Self {
        Self {
            auto: s.auto_exposure,
            exposure: s.exposure,
            ev_bias: s.ev_bias,
            scene: s.scene_name.clone(),
        }
    }
    /// Seeds `s.exposure` when auto went off since `self` (and nothing else here changed);
    /// returns whether it did. Updates `self`.
    pub fn seed(&mut self, s: &mut Settings, m: Option<Adaptation>) -> bool {
        let now = Self::of(s);
        let flipped = self.auto
            && !now.auto
            && self.exposure == now.exposure
            && self.ev_bias == now.ev_bias
            && self.scene == now.scene;
        let seeded = match manual_from_auto(m, s.ev_bias).filter(|_| flipped) {
            Some(e) => {
                s.exposure = e;
                true
            }
            None => false,
        };
        *self = Self::of(s);
        seeded
    }
}
/// The meter's fields on the RENDER_OK line (E7b): `meter_ev`, the settled metered exposure in
/// stops without `ev_bias` (`none` in manual or before a reading), and `exposure`, the final
/// linear exposure the display used (bias included; the manual one in manual).
pub fn render_ok_meter(s: &Settings, m: Option<Adaptation>) -> String {
    let m = m.unwrap_or_default();
    let ev = if s.auto_exposure && m.valid {
        format!("{:.4}", m.current)
    } else {
        "none".into()
    };
    format!("meter_ev={ev} exposure={:.6e}", m.exposure(s))
}
/// How long the meter stays held after a light or material edit stops (E7b; default, changeable).
pub const METER_HOLD_SECONDS: f32 = 1.;
/// The light and material values whose edit holds the meter (E7b): emission, area light and main
/// light power, the sky and sun, the HDRI strength and albedo. A hash of their bits, so any change
/// shows; positions and the camera are left out, so moving things still adapts.
pub fn light_key(s: &Settings) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut f = |v: f32| v.to_bits().hash(&mut h);
    let m = |m: &crate::scene::Material, f: &mut dyn FnMut(f32)| {
        m.color.iter().chain(&m.secondary_color).for_each(|&v| f(v));
        f(m.emission);
    };
    for o in &s.objects {
        m(&o.material, &mut f);
    }
    m(&s.walls, &mut f);
    m(&s.floor, &mut f);
    for l in &s.lights {
        l.color.iter().for_each(|&v| f(v));
        f(l.intensity);
        f(l.enabled as u32 as f32);
    }
    for v in s
        .left
        .iter()
        .chain(&s.right)
        .chain(&s.base)
        .chain(&s.background)
    {
        f(*v);
    }
    for v in [
        s.sky,
        s.light_intensity,
        s.light_on as u32 as f32,
        s.light_temperature,
        s.hdri_strength,
        s.physical_sky.intensity,
        s.physical_sky.on as u32 as f32,
    ] {
        f(v);
    }
    s.light_tint.iter().for_each(|&v| f(v));
    h.finish()
}
/// Holds the meter while lights or materials are edited and `METER_HOLD_SECONDS` after (E7b),
/// so the edit shows instead of being metered away. Only edits made with the pointer held (a
/// slider drag, a colour picker) count: animation, physics and scripts still adapt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterHold {
    key: Option<u64>,
    left: f32,
}
impl MeterHold {
    /// One frame: `editing` is whether the UI has the pointer held; `dt` the frame step.
    /// Returns whether the meter is held this frame.
    pub fn update(&mut self, s: &Settings, editing: bool, dt: f32) -> bool {
        let key = light_key(s);
        if self.key.is_some_and(|k| k != key) && editing && s.auto_exposure {
            self.left = METER_HOLD_SECONDS;
        } else {
            self.left = (self.left - dt.max(0.)).max(0.);
        }
        self.key = Some(key);
        self.left > 0. && s.auto_exposure
    }
}
/// The live app's meter step for one frame (the GPU's; the CPU export decides its own): `still`
/// settles every frame (headless, `--capture`, still exports), a converged image settles once
/// and then freezes, too few samples hold. `samples` is what the displayed image will have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeterPlan {
    low_frames: u32,
    settled_at_cap: bool,
}
impl MeterPlan {
    pub fn next(&mut self, still: bool, samples: u32, max_samples: u32) -> MeterOp {
        let at_cap = samples >= max_samples;
        if !at_cap {
            self.settled_at_cap = false;
        }
        if still {
            return MeterOp::Settle;
        }
        if at_cap {
            return if std::mem::replace(&mut self.settled_at_cap, true) {
                MeterOp::Freeze
            } else {
                MeterOp::Settle
            };
        }
        if samples >= METER_MIN_SAMPLES {
            self.low_frames = 0;
            return MeterOp::Adapt;
        }
        self.low_frames += 1;
        if self.low_frames >= METER_LOW_FRAMES {
            MeterOp::Adapt
        } else {
            MeterOp::Hold
        }
    }
    /// Unfreezes (a meter setting changed while converged).
    pub fn wake(&mut self) {
        self.settled_at_cap = false;
    }
}
/// A still's exposure: the meter settled on `linear` in auto, the manual one otherwise.
#[cfg(test)]
pub fn still_exposure(s: &Settings, linear: &[f32], size: [u32; 2]) -> f32 {
    still_exposures(s, linear, size)[0]
}
/// A still's displayed and metered exposures (see `Adaptation::metered`).
pub fn still_exposures(s: &Settings, linear: &[f32], size: [u32; 2]) -> [f32; 2] {
    if !s.auto_exposure || linear.len() < (size[0] * size[1] * 4) as usize {
        return [s.exposure; 2];
    }
    let mut a = Adaptation::default();
    let h = meter_histogram(linear, size, s.meter_mode);
    a.step(
        meter_reading(&h, s.meter_mode),
        MeterOp::Settle,
        true,
        0.,
        s.exposure_speed,
    );
    [a.exposure(s), a.metered(s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Settings {
        let mut s = Settings::default();
        (s.exposure, s.gamma, s.tonemap) = (1., 1., 0);
        (s.auto_exposure, s.eyes) = (false, false);
        (s.halation, s.vignette, s.grain, s.glare) = (0., 0., 0., 0.);
        (s.lift, s.grade_gamma, s.gain) = ([0.; 3], [1.; 3], [1.; 3]);
        s.lut_path.clear();
        s
    }

    /// A `w` x `h` image of grey `l`, with `f(x, y)` overriding pixels.
    fn image(w: u32, h: u32, l: f32, f: impl Fn(u32, u32) -> Option<f32>) -> Vec<f32> {
        (0..w * h)
            .flat_map(|i| {
                let v = f(i % w, i / w).unwrap_or(l);
                [v, v, v, 1.]
            })
            .collect()
    }
    fn auto() -> Settings {
        Settings {
            auto_exposure: true,
            ev_bias: 0.,
            ..plain()
        }
    }

    #[test]
    fn the_meter_puts_a_uniform_image_at_middle_grey() {
        use crate::scene::MeterMode;
        for mode in MeterMode::ALL {
            let s = Settings {
                meter_mode: mode,
                ..auto()
            };
            for l in [3e-9, 1e-4, 0.02, 0.18, 3.7, 30.] {
                let e = still_exposure(&s, &image(64, 48, l, |_, _| None), [64, 48]);
                let off = (e * l / 0.18).log2();
                assert!(off.abs() < 0.01, "{mode:?} {l}: {off} EV off");
            }
        }
        // Deterministic: the same image gives the same bits.
        let img = image(97, 61, 0.3, |x, y| {
            Some(((x * 7 + y * 13) % 17) as f32 * 0.05)
        });
        let h = meter_histogram(&img, [97, 61], MeterMode::CentreWeighted);
        assert_eq!(
            h,
            meter_histogram(&img, [97, 61], MeterMode::CentreWeighted)
        );
        // The bias shifts the settled exposure by its stops; manual ignores the meter.
        let s = Settings {
            ev_bias: 1.5,
            ..auto()
        };
        let e = still_exposure(&s, &image(16, 16, 0.18, |_, _| None), [16, 16]);
        assert!((e.log2() - 1.5).abs() < 0.01, "{e}");
        let m = Settings {
            exposure: 3.,
            ..plain()
        };
        assert_eq!(still_exposure(&m, &img, [97, 61]), 3.);
    }

    #[test]
    fn lamps_black_and_the_edges_do_not_move_the_meter() {
        use crate::scene::MeterMode;
        let s = auto();
        let at = |img: &[f32], size| (still_exposure(&s, img, size) * 0.18 / 0.18).log2();
        // 2% of the samples on a lamp a million times brighter: ignored (the top 3% are).
        let lamps = image(200, 200, 0.18, |x, y| (x < 28 && y < 28).then_some(1e5));
        assert!(
            at(&lamps, [200, 200]).abs() < 0.05,
            "{}",
            at(&lamps, [200, 200])
        );
        // An unlit background is not light: half black still meters the grey half.
        let half = image(200, 200, 0.18, |x, _| (x < 100).then_some(0.));
        assert!(at(&half, [200, 200]).abs() < 0.01);
        // Centre-weighted leans to the centre; spot sees only it.
        let centre = image(200, 200, 0.02, |x, y| {
            let (u, v) = (x as f32 / 100. - 1., y as f32 / 100. - 1.);
            (u * u + v * v < 0.06).then_some(2.)
        });
        let w = |mode| {
            let s = Settings {
                meter_mode: mode,
                ..auto()
            };
            still_exposure(&s, &centre, [200, 200]).log2()
        };
        assert!((w(MeterMode::Spot) - (0.18f32 / 2.).log2()).abs() < 0.05);
        assert!(w(MeterMode::CentreWeighted) < w(MeterMode::Histogram));
        // Nothing lit: no reading, so the exposure stays manual.
        let dark = image(8, 8, 0., |_, _| None);
        assert!(
            meter_reading(&meter_histogram(&dark, [8, 8], s.meter_mode), s.meter_mode).is_none()
        );
        assert_eq!(still_exposure(&s, &dark, [8, 8]), s.exposure);
    }

    #[test]
    fn stills_settle_and_a_converged_view_settles_once_then_freezes() {
        let mut plan = MeterPlan::default();
        // Headless and `--capture` settle on every frame, whatever the samples.
        for n in [1, 2, 100] {
            assert_eq!(plan.next(true, n, 4096), MeterOp::Settle);
        }
        // Live: too few samples hold (until a moving camera has held for a few frames),
        // then adapt, then the cap settles once and freezes.
        let mut plan = MeterPlan::default();
        let ops: Vec<_> = [1, 1, 1, 1, 8, 50, 64, 64, 64]
            .iter()
            .map(|&n| plan.next(false, n, 64))
            .collect();
        use MeterOp::*;
        assert_eq!(
            ops,
            [
                Hold, Hold, Hold, Adapt, Adapt, Adapt, Settle, Freeze, Freeze
            ]
        );
        plan.wake();
        assert_eq!(plan.next(false, 64, 64), Settle);
        // A settle jumps to the reading whatever the time step.
        let mut a = Adaptation::default();
        a.step(Some((5., -8.)), Adapt, true, 1. / 60., 2.);
        assert_eq!(
            (a.valid, a.current),
            (true, 5.),
            "the first reading settles"
        );
        a.step(Some((-3., 0.)), Settle, true, 0., 2.);
        assert_eq!((a.base, a.current), (-3., -3.));
        let before = a;
        a.step(Some((9., -12.)), Freeze, true, 1., 2.);
        assert_eq!(a, before);
    }

    #[test]
    fn adaptation_is_eye_like_and_a_video_is_deterministic() {
        // Toward a brighter scene (lower exposure) it is fast, toward a darker one slow, and
        // slower still when the darker scene is scotopic.
        let half = |target: f32, scene: f32| {
            let mut a = Adaptation::default();
            a.step(Some((0., -2.)), MeterOp::Settle, true, 0., ADAPT_SPEED);
            let mut t = 0.;
            while (a.current / target) < 0.5 {
                a.step(
                    Some((target, scene)),
                    MeterOp::Adapt,
                    true,
                    1. / 30.,
                    ADAPT_SPEED,
                );
                t += 1. / 30.;
            }
            t
        };
        let (bright, dark, night) = (half(-4., 2.), half(4., -6.), half(4., -30.));
        assert!(
            bright < 0.3 && dark > 1.5 && night > dark * 2.,
            "{bright} {dark} {night}"
        );
        // Twice the speed halves the time.
        let mut a = Adaptation::default();
        a.step(Some((0., -2.)), MeterOp::Settle, true, 0., 4.);
        a.step(Some((4., -6.)), MeterOp::Adapt, true, 1., 4.);
        let mut b = Adaptation::default();
        b.step(Some((0., -2.)), MeterOp::Settle, true, 0., 2.);
        b.step(Some((4., -6.)), MeterOp::Adapt, true, 2., 2.);
        assert!((a.current - b.current).abs() < 1e-5);
        // A video: frames of a sunrise metered and adapted at 1/fps, exported twice, give the
        // same exposure per frame; a frame's extra passes (no commit) don't add steps.
        let frames: Vec<Vec<f32>> = (0..24)
            .map(|i| image(40, 30, 1e-6 * 2f32.powi(i), |x, _| (x == 3).then_some(50.)))
            .collect();
        let export = |passes: u32| {
            let s = auto();
            let mut a = Adaptation::default();
            frames
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let op = if i == 0 {
                        MeterOp::Settle
                    } else {
                        MeterOp::Adapt
                    };
                    for p in 0..passes {
                        let h = meter_histogram(f, [40, 30], s.meter_mode);
                        a.step(meter_reading(&h, s.meter_mode), op, p == 0, 1. / 24., 2.);
                    }
                    a.exposure(&s).to_bits()
                })
                .collect::<Vec<_>>()
        };
        let first = export(1);
        assert_eq!(first, export(1));
        assert_eq!(first, export(3));
        // It follows the sunrise down, behind the meter.
        let ev: Vec<f32> = first.iter().map(|b| f32::from_bits(*b).log2()).collect();
        assert!(ev.windows(2).all(|w| w[1] <= w[0]), "{ev:?}");
    }

    #[test]
    fn tone_maps_at_known_points() {
        let v = |x| Vec3::splat(x);
        // Linear clips; Reinhard halves 1; extended Reinhard keeps 16 at white.
        assert_eq!(tone_map(v(2.), 0), v(1.));
        assert!((tone_map(v(1.), 1).x - 0.5).abs() < 1e-6);
        assert!((tone_map(v(16.), 2).x - 1.).abs() < 1e-5);
        // ACES (Narkowicz): 0 stays 0, high values clip.
        assert!(tone_map(v(0.), 3).x.abs() < 1e-6);
        assert_eq!(tone_map(v(100.), 3), v(1.));
        // Hable at its white point 11.2 (input doubled) is 1.
        assert!((tone_map(v(5.6), 4).x - 1.).abs() < 1e-5);
        // AgX maps middle grey near 0.18 display-linear's usual ~0.2 range, and is monotonic.
        let a = tone_map(v(0.18), 6).x;
        assert!(a > 0.1 && a < 0.35, "{a}");
        assert!(tone_map(v(1.), 6).x > a);
        // PBR Neutral leaves base colours under its start untouched (after the toe offset).
        assert!((tone_map(v(0.5), 7).x - 0.46).abs() < 1e-6);
        for mode in 0..8 {
            let mut last = -1.;
            for i in 0..64 {
                let y = tone_map(v(i as f32 * 0.25), mode).x;
                assert!((0. ..=1.).contains(&y) && y >= last - 1e-6, "mode {mode}");
                last = y;
            }
        }
    }

    #[test]
    fn plain_display_is_clamped_linear() {
        let s = plain();
        let image = [0.25f32, 0.5, 2., 1.];
        assert_eq!(
            display(&s, &image, [1, 1], 0).unwrap(),
            vec![64, 128, 255, 255]
        );
        let mut s = plain();
        s.gamma = 2.2;
        let out = display(&s, &[0.5, 0.5, 0.5, 1.], [1, 1], 0).unwrap();
        assert_eq!(out[0], (0.5f32.powf(1. / 2.2) * 255. + 0.5) as u8);
    }

    #[test]
    fn look_vignette_halation_and_grain() {
        // Lift raises black; gain scales the encoded value.
        let mut s = plain();
        s.lift = [0.2; 3];
        let black = display(&s, &[0., 0., 0., 1.], [1, 1], 0).unwrap();
        assert_eq!(black[0], (0.2f32.powf(2.2) * 255. + 0.5) as u8);
        // The vignette darkens corners, not the centre.
        let mut s = plain();
        s.vignette = 1.;
        let grey = [0.5f32, 0.5, 0.5, 1.].repeat(9 * 9);
        let out = display(&s, &grey, [9, 9], 0).unwrap();
        assert!(out[0] < out[(4 * 9 + 4) * 4] && out[(4 * 9 + 4) * 4] >= 120);
        // Halation adds red around a highlight.
        let mut s = plain();
        s.halation = 1.;
        let mut img = [0f32, 0., 0., 1.].repeat(100 * 100);
        for y in 45..55 {
            for x in 45..55 {
                img[(y * 100 + x) * 4..][..3].copy_from_slice(&[3., 3., 3.]);
            }
        }
        let out = display(&s, &img, [100, 100], 0).unwrap();
        let i = (50 * 100 + 55) * 4;
        assert!(out[i] > out[i + 2], "{:?}", &out[i..i + 4]);
        // Grain changes midtones, by frame.
        let mut s = plain();
        s.grain = 1.;
        let grey = [0.5f32, 0.5, 0.5, 1.].repeat(64);
        let a = display(&s, &grey, [8, 8], 0).unwrap();
        assert_ne!(a, display(&s, &grey, [8, 8], 1).unwrap());
        assert!(a.chunks(4).any(|p| p[0] != 128));
        // Glare is refused, not left out.
        let mut s = plain();
        s.glare = 0.5;
        assert!(display(&s, &grey, [8, 8], 0).is_err());
    }

    #[test]
    fn switching_auto_off_keeps_the_exposure() {
        let m = Adaptation {
            valid: true,
            base: 3.,
            current: 3.25,
            target: 4.,
            scene_log2: -5.,
        };
        let mut s = Settings {
            ev_bias: 0.5,
            exposure: 1.,
            ..auto()
        };
        let shown = m.exposure(&s);
        let mut seen = ExposureSeen::of(&s);
        assert!(!seen.seed(&mut s, Some(m)), "auto still on");
        s.auto_exposure = false;
        assert!(seen.seed(&mut s, Some(m)));
        assert_eq!(m.exposure(&s), shown);
        assert_eq!(s.exposure, 3.75f32.exp2());
        // Back on and off with no reading: nothing to seed from.
        s.auto_exposure = true;
        seen.seed(&mut s, None);
        s.auto_exposure = false;
        assert!(!seen.seed(&mut s, None));
        // A scene load (or preset, or undo) that sets auto off and its own exposure keeps it.
        s.auto_exposure = true;
        seen.seed(&mut s, Some(m));
        (s.auto_exposure, s.exposure) = (false, 0.5);
        assert!(!seen.seed(&mut s, Some(m)));
        assert_eq!(s.exposure, 0.5);
        s.auto_exposure = true;
        seen.seed(&mut s, Some(m));
        (s.auto_exposure, s.scene_name) = (false, "other".into());
        assert!(!seen.seed(&mut s, Some(m)));
    }

    #[test]
    fn a_light_edit_holds_the_meter_for_a_second() {
        let mut s = auto();
        let mut h = MeterHold::default();
        assert!(
            !h.update(&s, true, 0.1),
            "the first frame has nothing to compare"
        );
        s.light_intensity *= 2.;
        assert!(
            !h.update(&s, false, 0.1),
            "not a UI edit (animation, a script)"
        );
        s.light_intensity *= 2.;
        assert!(h.update(&s, true, 0.1));
        assert!(h.update(&s, true, 0.5), "held, unchanged");
        assert!(h.update(&s, false, 0.4));
        assert!(!h.update(&s, false, 0.2), "released after a second");
        // Moving the camera or an object is not a light edit.
        s.yaw += 10.;
        assert!(!h.update(&s, true, 0.1));
        // Albedo and HDRI strength count; manual exposure never holds.
        s.walls.color[0] *= 0.5;
        assert!(h.update(&s, true, 0.1));
        s.auto_exposure = false;
        s.hdri_strength *= 2.;
        assert!(!h.update(&s, true, 0.1));
    }

    #[test]
    fn the_render_ok_line_carries_the_meter() {
        let m = Adaptation {
            valid: true,
            current: -2.5,
            ..Default::default()
        };
        let s = Settings {
            ev_bias: 1.,
            ..auto()
        };
        assert_eq!(
            render_ok_meter(&s, Some(m)),
            "meter_ev=-2.5000 exposure=3.535534e-1"
        );
        assert_eq!(
            render_ok_meter(&s, None),
            format!("meter_ev=none exposure={:.6e}", s.exposure)
        );
        let manual = Settings {
            exposure: 4.,
            ..plain()
        };
        assert_eq!(
            render_ok_meter(&manual, Some(m)),
            "meter_ev=none exposure=4.000000e0"
        );
    }

    #[test]
    fn halation_threshold_is_in_exposed_units() {
        // A scene 2^k brighter shown at 2^-k exposure is the same picture, halation included
        // (the threshold follows the exposure, E7b); exposure 1 keeps the old threshold of 1.
        assert_eq!(exposed_threshold(1., 1.), 1.);
        assert_eq!(exposed_threshold(2., 4.), 0.5);
        let mut s = plain();
        s.halation = 1.;
        let img = |k: f32| {
            let mut v = [0.1f32 * k, 0.1 * k, 0.1 * k, 1.].repeat(100 * 100);
            for y in 45..55 {
                for x in 45..55 {
                    v[(y * 100 + x) * 4..][..3].copy_from_slice(&[3. * k; 3]);
                }
            }
            v
        };
        let base = display_exposed(&s, &img(1.), [100, 100], 0, [1., 1.]).unwrap();
        for k in [1. / 64., 8., 1024.] {
            let out = display_exposed(&s, &img(k), [100, 100], 0, [1. / k, 1. / k]).unwrap();
            assert_eq!(out, base, "scene x{k}");
        }
        // And in auto: the meter's exposure moves the threshold the same way.
        let a = Settings {
            halation: 1.,
            ..auto()
        };
        let dim = display(&a, &img(1. / 64.), [100, 100], 0).unwrap();
        let bright = display(&a, &img(64.), [100, 100], 0).unwrap();
        assert_eq!(dim, bright);
    }

    #[test]
    fn lut_identity_and_inverse() {
        let n = 4;
        let mut data = vec![];
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let v = |i: usize| i as f32 / (n - 1) as f32;
                    data.push([1. - v(r), 1. - v(g), 1. - v(b)]);
                }
            }
        }
        let lut = Lut { size: n, data };
        let c = lut.sample(Vec3::new(0.2, 0.5, 0.9));
        assert!(
            (c - Vec3::new(0.8, 0.5, 0.1)).abs().max_element() < 1e-5,
            "{c}"
        );
    }
    #[test]
    fn eyes_leave_daylight_bit_identical_and_desaturate_the_night() {
        let size = [16, 8];
        let img: Vec<f32> = (0..16 * 8)
            .flat_map(|i| [0.6, 0.2 + (i % 5) as f32 * 0.05, 0.1, 1.])
            .collect();
        let run = |scale: f32, eyes: bool| {
            let mut s = plain();
            (s.auto_exposure, s.eyes, s.eyes_strength) = (true, eyes, 1.);
            let lit: Vec<f32> = img
                .iter()
                .enumerate()
                .map(|(i, v)| if i % 4 == 3 { *v } else { v * scale })
                .collect();
            display(&s, &lit, size, 0).unwrap()
        };
        // Daylight (about 1,000 cd/m² adapted): the same bytes with the Eyes on or off.
        assert_eq!(run(0.1, true), run(0.1, false));
        // A moonlit night (adapted well under 0.01 cd/m²): grey with a blue cast.
        let (on, off) = (run(1e-8, true), run(1e-8, false));
        let chroma = |p: &[u8]| p.chunks(4).map(|q| q[0] as i32 - q[1] as i32).sum::<i32>();
        assert!(
            chroma(&off) > 100 * 4 && chroma(&on) < 0,
            "{} {}",
            chroma(&off),
            chroma(&on)
        );
        assert!(on.chunks(4).all(|q| q[2] >= q[0]));
        // Strength 0 is off.
        let mut s = plain();
        (s.eyes, s.eyes_strength) = (true, 0.);
        assert_eq!(eyes_mix(&s, 1e8), 0.);
    }
}
