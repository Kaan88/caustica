//! Physical sky and day cycle (track 9). Hosek–Wilkie "An Analytic Model for Full Spectral
//! Sky-Dome Radiance" (SIGGRAPH 2012) with the solar radiance of the 2013 follow-up: a fitted
//! atmosphere whose coefficient sets live in `assets/sky/` (their own 3-clause BSD data; the
//! dataset layout notes are in docs/research/track-9-physical-sky.md). The CPU bakes the model
//! into an equirectangular map that then goes through the HDRI environment path (u30/u31 and
//! their CDFs), so lighting, importance sampling and the background need no path-trace changes.
//! The sun is baked as a 0.53° disc; the model is a day model, so below the horizon the map
//! blends to a plain night hemisphere (and the day model's lower half is scaled by the ground
//! albedo, which the model itself takes from the floor colour).
use crate::environment::{EnvMap, build};
use crate::scene::Settings;
use anyhow::Result;
use glam::Vec3;
use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::LazyLock;
use std::thread;

const DEGREES: f32 = PI / 180.;

/// Bake size: 0.35° per texel, so the 0.53° sun disc keeps a 3×3 texel spot, enough for the
/// environment CDF to importance-sample it like an HDRI sun.
pub const BAKE_WIDTH: usize = 2048;
pub const BAKE_HEIGHT: usize = 1024;
/// Solar angular diameter (the mean of the orbit), degrees.
pub const SUN_DIAMETER: f32 = 0.53;

/// The datasets' solar fit is a handful of times the sky radiance, which is right for how the
/// sun *looks* but not for a physically consistent 0.53° disc: at that size a disc carrying real
/// sun irradiance would need thousands of times the dome values. Gaining the disc by this factor
/// makes direct sun through a window read about three times the sky pool, like a real room.
pub const SOLAR_GAIN: f32 = 8000.;

/// Night hemisphere radiances (the 2012 model is daytime-only below the horizon).
const NIGHT_ZENITH: [f32; 3] = [0.0035, 0.0055, 0.0125];
const NIGHT_HORIZON: [f32; 3] = [0.009, 0.011, 0.021];
const GROUND_SCALE: f32 = 0.4;

const PARAMS_R: &[u8] = include_bytes!("../assets/sky/params-r");
const PARAMS_G: &[u8] = include_bytes!("../assets/sky/params-g");
const PARAMS_B: &[u8] = include_bytes!("../assets/sky/params-b");
const RADIANCES_R: &[u8] = include_bytes!("../assets/sky/radiances-r");
const RADIANCES_G: &[u8] = include_bytes!("../assets/sky/radiances-g");
const RADIANCES_B: &[u8] = include_bytes!("../assets/sky/radiances-b");

/// Little-endian f32s of a dataset file.
fn f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_bits(u32::from_le_bytes(*c)))
        .collect()
}

// Per channel: 20 blocks (turbidity index 0..9 = turbidity 1..10, times ground-albedo halves
// 0 and 1 at 540) of 54 f32 = 6 quintic control points × 9 model parameters, quintic-major.
// The radiance sets are the same blocks with 6 controls each (the solar disc radiance).
fn load_datasets() -> Vec<Vec<f32>> {
    [
        f32s(PARAMS_R),
        f32s(PARAMS_G),
        f32s(PARAMS_B),
        f32s(RADIANCES_R),
        f32s(RADIANCES_G),
        f32s(RADIANCES_B),
    ]
    .to_vec()
}

// Rows: channels 0..2 are the parameter sets, 3..5 the solar radiances.
static DATA: LazyLock<Vec<Vec<f32>>> = LazyLock::new(load_datasets);

/// Bernstein weights of t on the quintic basis (the datasets store Bezier control points).
fn bernstein(t: f32) -> [f32; 6] {
    let u = 1. - t;
    let u2 = u * u;
    let t2 = t * t;
    [
        u2 * u2 * u,
        5. * t * u2 * u2,
        10. * t2 * u2 * u,
        10. * t2 * u2 * t,
        5. * t2 * t2 * u,
        t2 * t2 * t,
    ]
}

/// Nine model parameters plus the solar radiance, per RGB channel. `elevation_deg` is the
/// solar elevation above the horizon, `turbidity` 1..10, `albedo` the ground albedo 0..1.
pub struct SkyState {
    pub p: [[f32; 9]; 3],
    pub solar: [f32; 3],
}

/// Builds the model state for sun conditions. The datasets interpolate in t = (elevation/π·2)^(1/3)
/// along a quintic, and bilinearly across the turbidity blocks and the two ground-albedo halves.
pub fn model(elevation_deg: f32, turbidity: f32, albedo: [f32; 3]) -> SkyState {
    let t = (elevation_deg.clamp(0., 90.) * DEGREES / FRAC_PI_2).powf(1. / 3.);
    let tau = turbidity.clamp(1., 10.);
    let whole = tau.trunc() as usize;
    let rem = tau - tau.trunc();
    let (lo, hi) = (whole.saturating_sub(1), whole.min(9));
    let b = bernstein(t);
    let mut st = SkyState {
        p: [[0f32; 9]; 3],
        solar: [0f32; 3],
    };
    for c in 0..3 {
        let a = albedo[c].clamp(0., 1.);
        let w = [
            (1. - a) * (1. - rem),
            (1. - a) * rem,
            a * (1. - rem),
            a * rem,
        ];
        let d = &DATA[c];
        let starts = [lo * 54, hi * 54, 540 + lo * 54, 540 + hi * 54];
        for (blk, weight) in starts.iter().zip(w) {
            if weight == 0. {
                continue;
            }
            let block = &d[*blk..];
            for j in 0..6 {
                let s = weight * b[j];
                for i in 0..9 {
                    st.p[c][i] += s * block[j * 9 + i];
                }
            }
        }
        let r = &DATA[3 + c];
        let r_starts = [lo * 6, hi * 6, 60 + lo * 6, 60 + hi * 6];
        for (blk, weight) in r_starts.iter().zip(w) {
            if weight == 0. {
                continue;
            }
            let block = &r[*blk..];
            for j in 0..6 {
                st.solar[c] += weight * b[j] * block[j];
            }
        }
    }
    st
}

impl SkyState {
    /// Sky-dome radiance for a view at zenith angle `theta` and scattering angle `gamma` from
    /// the sun, as in their Figure 5 (their own units, linear nits per steradian).
    pub fn radiance(&self, theta: f32, gamma: f32) -> [f32; 3] {
        let cos_t = theta.cos().abs();
        let g = gamma.cos();
        let g2 = g * g;
        let zen = cos_t.sqrt();
        [0, 1, 2].map(|c| {
            let p = self.p[c];
            let lhs = 1. + p[0] * (p[1] / (cos_t + 0.01)).exp();
            let mie = (1. + g2) / (1. + p[8] * p[8] - 2. * p[8] * g).powf(1.5);
            let rhs = p[2] + p[3] * (p[4] * gamma).exp() + p[5] * g2 + p[6] * mie + p[7] * zen;
            (self.solar[c] * lhs * rhs).max(0.)
        })
    }
}

/// Sun position: elevation above the horizon and azimuth clockwise from north, degrees.
pub struct Sun {
    pub elevation: f32,
    pub azimuth: f32,
}

/// Solar position from day of year (1..365), latitude in degrees and local solar time in
/// hours: mean ecliptic longitude anchored at the June solstice (day 172.4), projected
/// through the 23.44° obliquity for declination and right ascension, and the hour angle
/// carries the equation of the equinoxes (λ−α, ±10 min; the ±8 min of equation of center
/// from eccentricity is left out — plenty for an artistic day cycle).
pub fn sun(day: f32, latitude: f32, time: f32) -> Sun {
    let λ = (90. + (day - 172.4) * (360. / 365.25)) * DEGREES;
    let ε = 23.44 * DEGREES;
    let decl = (ε.sin() * λ.sin()).asin();
    let alpha = (ε.cos() * λ.sin()).atan2(λ.cos());
    let equinox_eq = (λ - alpha + PI) % (2. * PI);
    let equinox_eq = (if equinox_eq < 0. {
        equinox_eq + 2. * PI
    } else {
        equinox_eq
    }) - PI;
    let h = (time - 12.) * 15. * DEGREES + equinox_eq;
    let φ = latitude * DEGREES;
    let sin_e = (φ.sin() * decl.sin() + φ.cos() * decl.cos() * h.cos()).clamp(-1., 1.);
    let east = -decl.cos() * h.sin();
    let north = decl.sin() * φ.cos() - decl.cos() * h.cos() * φ.sin();
    let az = east.atan2(north) / DEGREES;
    Sun {
        elevation: sin_e.asin() / DEGREES,
        azimuth: (az + 360.) % 360.,
    }
}

impl Sun {
    /// Unit world direction of the sun: +Y up, north is −Z and east is +X, which matches the
    /// equirect mapping of environment.rs (u = φ/2π + 0.5 with φ = atan2(x, −z)).
    pub fn direction(&self) -> Vec3 {
        let e = self.elevation * DEGREES;
        let a = self.azimuth * DEGREES;
        Vec3::new(a.sin() * e.cos(), e.sin(), -(a.cos() * e.cos()))
    }
    /// 1 with the sun up, 0 well below the horizon, smooth through civil twilight.
    pub fn day_factor(&self) -> f32 {
        let t = ((self.elevation + 7.) / 8.).clamp(0., 1.);
        t * t * (3. - 2. * t)
    }
}

/// `bake` at a chosen size (tests use small ones). A few million texels of closed form, done in
/// parallel row strips so a live day cycle stays responsive.
pub fn bake_at(width: usize, height: usize, s: &Settings) -> Result<EnvMap> {
    let sun_pos = sun(s.sky_day, s.sky_latitude, s.sky_time);
    let albedo = std::array::from_fn(|c| s.floor.color[c].clamp(0., 1.));
    let st = model(sun_pos.elevation.max(0.), s.sky_turbidity, albedo);
    let day = sun_pos.day_factor();
    let inv_day = 1. - day;
    let sun_dir = sun_pos.direction();
    let radius = 0.5 * SUN_DIAMETER * DEGREES;
    let strength = s.sky_strength.max(0.);
    let solar = std::array::from_fn(|c| st.solar[c] * SOLAR_GAIN);
    let phi: Vec<f32> = (0..width)
        .map(|x| 2. * PI * ((x as f32 + 0.5) / width as f32 - 0.5))
        .collect();
    let sin_phi: Vec<f32> = phi.iter().map(|p| p.sin()).collect();
    let cos_phi: Vec<f32> = phi.iter().map(|p| p.cos()).collect();
    let theta: Vec<f32> = (0..height)
        .map(|y| PI * (y as f32 + 0.5) / height as f32)
        .collect();
    let sin_t: Vec<f32> = theta.iter().map(|p| p.sin()).collect();
    let cos_t: Vec<f32> = theta.iter().map(|p| p.cos()).collect();
    // Shared read-only inputs; the row chunks are the only thing moved per thread.
    let phi_s = &sin_phi;
    let phi_c = &cos_phi;
    let th = &theta;
    let th_s = &sin_t;
    let th_c = &cos_t;
    let st_r = &st;
    let mut rgb = vec![[0f32; 3]; width * height];
    let threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let rows = height.div_ceil(threads);
    thread::scope(|sc| {
        for (t, chunk) in rgb.chunks_mut(rows * width).enumerate() {
            sc.spawn(move || {
                for (i, cell) in chunk.iter_mut().enumerate() {
                    let y = t * rows + i / width;
                    let x = i % width;
                    let c_t = th_c[y];
                    let night_c = if c_t > 0. {
                        NIGHT_HORIZON[0] + (NIGHT_ZENITH[0] - NIGHT_HORIZON[0]) * c_t
                    } else {
                        NIGHT_HORIZON[0] * GROUND_SCALE
                    };
                    let night_g = if c_t > 0. {
                        NIGHT_HORIZON[1] + (NIGHT_ZENITH[1] - NIGHT_HORIZON[1]) * c_t
                    } else {
                        NIGHT_HORIZON[1] * GROUND_SCALE
                    };
                    let night_b = if c_t > 0. {
                        NIGHT_HORIZON[2] + (NIGHT_ZENITH[2] - NIGHT_HORIZON[2]) * c_t
                    } else {
                        NIGHT_HORIZON[2] * GROUND_SCALE
                    };
                    let scale = if c_t > 0. { [1f32; 3] } else { albedo };
                    let d = (th_s[y] * (phi_s[x] * sun_dir.x - phi_c[x] * sun_dir.z)
                        + c_t * sun_dir.y)
                        .clamp(-1., 1.);
                    let gamma = d.acos();
                    let sky = if gamma <= radius {
                        solar
                    } else {
                        st_r.radiance(th[y], gamma)
                    };
                    *cell = [
                        (sky[0] * scale[0] * day + night_c * inv_day) * strength,
                        (sky[1] * scale[1] * day + night_g * inv_day) * strength,
                        (sky[2] * scale[2] * day + night_b * inv_day) * strength,
                    ];
                }
            });
        }
    });
    build(width, height, rgb)
}

/// Bakes the sky at the app's size into the environment map (the HDRI pipeline adds the CDFs).
pub fn bake(s: &Settings) -> Result<EnvMap> {
    bake_at(BAKE_WIDTH, BAKE_HEIGHT, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky_settings(day: f32, latitude: f32, time: f32, turbidity: f32) -> Settings {
        let mut s = Settings::default();
        s.sky_model = 1;
        s.sky_time = time;
        s.sky_latitude = latitude;
        s.sky_day = day;
        s.sky_turbidity = turbidity;
        s.sky_strength = 1.;
        s
    }

    #[test]
    /// High summer noon, night before sunrise, rising in the east and setting in the west.
    fn the_sun_position_follows_the_clock() {
        let noon = sun(210., 48., 12.);
        assert!(noon.elevation > 50. && noon.elevation < 70.);
        assert!(noon.azimuth > 170. && noon.azimuth < 190.);
        assert!(sun(210., 48., 3.5).elevation < -5.);
        let morning = sun(210., 48., 8.);
        assert!(morning.elevation > 10. && morning.azimuth < 150.);
        let evening = sun(210., 48., 18.);
        assert!(evening.elevation > 5. && evening.azimuth > 240.);
    }

    #[test]
    /// The 2013 solar radiance: much brighter, and red against blue, when the sun is low.
    fn the_solar_radiance_gets_warmer_and_brighter_higher() {
        let low = model(2., 5., [0.5, 0.5, 0.5]);
        let high = model(45., 5., [0.5, 0.5, 0.5]);
        assert!(low.solar[0] > low.solar[2] * 1.4);
        assert!(high.solar[0] < high.solar[2] * 1.3);
        let total = |s: SkyState| s.solar[0] + s.solar[1] + s.solar[2];
        assert!(total(high) > total(low) * 3.);
    }

    #[test]
    /// Rayleigh scattering: looking up on a fairly clear day reads blue.
    fn the_zenith_is_blue() {
        let st = model(30., 3., [0.3, 0.3, 0.3]);
        let up = st.radiance(0., 60. * PI / 180.);
        assert!(up[2] > up[0] * 1.3);
    }

    #[test]
    /// The sun disc is baked as the exact solar radiance on a ~3×3 texel patch, which the
    /// environment CDF then importance-samples like an HDRI sun.
    fn the_bake_holds_the_sun_disc() {
        let s = sky_settings(210., 47., 13.5, 3.);
        let env = bake(&s).unwrap();
        assert_eq!((env.width, env.height), (BAKE_WIDTH, BAKE_HEIGHT));
        let sun_pos = sun(s.sky_day, s.sky_latitude, s.sky_time);
        let albedo = std::array::from_fn(|c| s.floor.color[c].clamp(0., 1.));
        let solar = model(sun_pos.elevation.max(0.), s.sky_turbidity, albedo).solar;
        let hits = env
            .texels
            .iter()
            .filter(|t| {
                t[0] == solar[0] * SOLAR_GAIN
                    && t[1] == solar[1] * SOLAR_GAIN
                    && t[2] == solar[2] * SOLAR_GAIN
            })
            .count();
        assert!(
            hits >= 1 && hits <= 20,
            "{hits} sun texels, solar {solar:?}"
        );
    }

    #[test]
    /// Before sunrise the map is the night hemisphere: dim, but never black.
    fn the_night_sky_is_dark_and_never_black() {
        let env = bake(&sky_settings(210., 47., 3., 3.)).unwrap();
        let brightest = env
            .texels
            .iter()
            .map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2])
            .fold(0., f32::max);
        assert!(brightest > 0.001 && brightest < 0.05, "{brightest}");
    }

    #[test]
    /// Same inputs, same texels: the thread strip split is fixed by the size, not the schedule.
    fn the_bake_is_deterministic() {
        let s = sky_settings(60., 15., 9.25, 6.5);
        let a = bake_at(300, 150, &s).unwrap();
        let b = bake_at(300, 150, &s).unwrap();
        assert_eq!(a.texels, b.texels);
        assert_eq!(a.cdf, b.cdf);
    }
}
