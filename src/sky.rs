//! Physical sky and day cycle (track 9). The sun's position comes from the time of day, the day
//! of the year and the latitude; the sky is a ray-marched Earth atmosphere (Rayleigh, Mie and
//! ozone, after Bruneton and Hillaire) with single scattering plus Hillaire's (2020) isotropic
//! multiple-scattering term. It is baked into an equirectangular map that goes through the HDRI
//! path (importance sampling, MIS), and the sun itself is an analytic sun light (track 7)
//! whose colour is the atmosphere's transmittance towards it.
//!
//! Units: the sun outside the atmosphere lights a white surface facing it to `intensity`
//! (irradiance π · intensity), as a sun light of that intensity does; the sky map is in the same
//! units, so sun and sky stay in balance at every time of day.
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

use crate::photometry::cd_to_scene;
#[cfg(test)]
use crate::photometry::{luminance, scene_to_cd};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct PhysicalSky {
    /// Replaces the HDRI and the sky light while on.
    pub on: bool,
    /// Solar time, hours (12 = the sun due south or north).
    pub time: f32,
    /// Day of the year, 1–366 (172 is the June solstice).
    pub day: f32,
    /// Calendar year (1900–2100): with the day it fixes the date for the stars' positions
    /// (precession, sidereal time).
    pub year: i32,
    /// Degrees, positive north.
    pub latitude: f32,
    /// Turns the sky about the vertical axis, degrees (0: north is −z).
    pub north: f32,
    /// Haze: 1 is a pure Rayleigh sky, 2 clear, 10 very hazy (scales the aerosol density by
    /// turbidity − 1).
    pub turbidity: f32,
    /// The sun outside the atmosphere lights a white surface facing it to this.
    pub intensity: f32,
    /// Angular diameter of the sun, degrees.
    pub sun_size: f32,
    /// Albedo of the ground below the horizon.
    pub ground: f32,
    /// Camera rays see the sky and the sun (off: the background colour).
    pub visible: bool,
    /// `time` is the clock time at `longitude` in `utc_offset` (with the equation of time),
    /// not solar time.
    pub clock: bool,
    /// Degrees, positive east (clock time only).
    pub longitude: f32,
    /// The time zone, hours from UTC (clock time only).
    pub utc_offset: f32,
    /// Observer altitude above the ground, km (the sky darkens and the horizon drops).
    pub altitude: f32,
    /// Star brightness (0: none): the catalogue's stars as point lights (stars.rs).
    pub stars: f32,
    /// Stars fainter than this V magnitude are left out (6.5: the naked eye's limit in a dark
    /// sky; a camera on a long exposure reaches 9 and beyond).
    pub limiting_magnitude: f32,
    /// A star's disc on screen, pixels across (1: sharp; larger spreads its light, as an
    /// out-of-focus or glowing point).
    pub star_size: f32,
    /// The moon: its phase follows the date, and at night it lights the sky.
    pub moon: bool,
    /// Milky Way brightness (1: the real sky's, from `scenes/sky/milkyway.exr`).
    pub milky_way: f32,
    /// Airglow brightness (1: a dark site, 22.0 mag/arcsec² V at the zenith).
    pub airglow: f32,
    /// The airglow's tint, 0..1: 0 the eye's near-neutral one, 1 the warmer, greener one a
    /// long camera exposure records (OI green with some red and sodium yellow).
    pub airglow_warmth: f32,
    /// Light pollution (1: dark rural, the whole sky 21.5 mag/arcsec² V at the zenith).
    pub light_pollution: f32,
}
impl Default for PhysicalSky {
    fn default() -> Self {
        Self {
            on: false,
            time: 15.,
            day: 172.,
            year: 2026,
            latitude: 45.,
            north: 0.,
            turbidity: 2.,
            intensity: 1.,
            sun_size: 0.53,
            ground: 0.3,
            visible: true,
            clock: false,
            longitude: 0.,
            utc_offset: 0.,
            altitude: 0.05,
            stars: 1.,
            limiting_magnitude: 6.5,
            star_size: 1.,
            moon: true,
            milky_way: 1.,
            airglow: 1.,
            airglow_warmth: 0.,
            light_pollution: 1.,
        }
    }
}
impl PhysicalSky {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (0. ..=24.).contains(&self.time)
                && (1. ..=366.).contains(&self.day)
                && (1900..=2100).contains(&self.year)
                && (-90. ..=90.).contains(&self.latitude)
                && (-360. ..=360.).contains(&self.north)
                && (1. ..=10.).contains(&self.turbidity)
                && (0. ..=1000.).contains(&self.intensity)
                && (0.05..=20.).contains(&self.sun_size)
                && (0. ..=1.).contains(&self.ground)
                && (-180. ..=180.).contains(&self.longitude)
                && (-14. ..=14.).contains(&self.utc_offset)
                && (0. ..=20.).contains(&self.altitude)
                && (0. ..=1000.).contains(&self.stars)
                && (-2. ..=16.).contains(&self.limiting_magnitude)
                && (0.5..=32.).contains(&self.star_size)
                && (0. ..=10.).contains(&self.milky_way)
                && (0. ..=10.).contains(&self.airglow)
                && (0. ..=1.).contains(&self.airglow_warmth)
                && (0. ..=10.).contains(&self.light_pollution),
            "Invalid physical sky"
        );
        Ok(())
    }
    /// Solar time in hours: `time`, or with `clock` the clock time corrected by longitude,
    /// time zone and the equation of time.
    pub fn solar_time(&self) -> f32 {
        if !self.clock {
            return self.time;
        }
        (self.time + self.longitude / 15. - self.utc_offset + self.equation_of_time() / 60.)
            .rem_euclid(24.)
    }
    /// The equation of time, minutes (apparent minus mean solar time), from the sun's exact
    /// right ascension at the instant (`mean_julian_date`; it changes under 30 s a day).
    fn equation_of_time(&self) -> f32 {
        crate::astro::equation_of_time(self.mean_julian_date()) as f32
    }
    /// Julian date with `time` taken as mean time at the place (UT = civil time − offset, or
    /// time − longitude hours): the instant before the equation of time is applied. Neither is
    /// wrapped into a day: an hour outside 0..24 moves the date, which `julian_date_of_day` does.
    fn mean_julian_date(&self) -> f64 {
        let ut = if self.clock {
            self.time as f64 - self.utc_offset as f64
        } else {
            self.time as f64 - self.longitude as f64 / 15.
        };
        crate::astro::julian_date_of_day(self.year, self.day, ut)
    }
    /// Julian date of the sky's instant. With `clock`, `time` is civil time, so UT is
    /// `time − utc_offset`; without it, `time` is apparent solar time at `longitude`, so UT is
    /// that minus the longitude's hours and the equation of time.
    pub fn julian_date(&self) -> f64 {
        let jd = self.mean_julian_date();
        if self.clock {
            jd
        } else {
            jd - self.equation_of_time() as f64 / (60. * 24.)
        }
    }
    /// Rotation from the J2000 celestial frame to the sky's own frame (east +x, up +y,
    /// north −z, before `north` turns it), for the scene's date, time and place.
    pub fn local_from_celestial(&self) -> glam::Mat3 {
        let m = crate::astro::celestial_to_local(
            self.julian_date(),
            (self.latitude as f64).to_radians(),
            (self.longitude as f64).to_radians(),
        );
        glam::Mat3::from_cols_array(&m.to_cols_array().map(|v| v as f32))
    }
    /// The moon's age as a fraction of the synodic month (0 new, 0.5 full): the ecliptic
    /// elongation of the moon from the sun, so the phase matches the moon's position.
    pub fn moon_age(&self) -> f32 {
        crate::astro::moon_age(self.julian_date()) as f32
    }
    /// Unit direction towards the sun in the sky's own frame (north −z, east +x, up +y): the
    /// sun's apparent position at the instant through the same rotation as the stars.
    fn sun_local(&self) -> Vec3 {
        self.celestial_local(crate::astro::sun_j2000(self.julian_date()))
    }
    /// The moon (geocentric, no parallax) in the sky's own frame.
    fn moon_local(&self) -> Vec3 {
        self.celestial_local(crate::astro::moon_j2000(self.julian_date()))
    }
    fn celestial_local(&self, v: glam::DVec3) -> Vec3 {
        let l = self.local_from_celestial() * Vec3::new(v.x as f32, v.y as f32, v.z as f32);
        l.normalize()
    }
    /// The sky's rotation as the environment map's (radians; `environment_map.y`).
    pub fn rotation(&self) -> f32 {
        self.north.to_radians()
    }
    /// Unit direction towards the sun in the world.
    pub fn sun_direction(&self) -> Vec3 {
        // The map is baked with north at −z and the shader turns it by `rotation` (φ grows from
        // −z towards +x, so a turn of β is a rotation of −β about +y).
        glam::Mat3::from_rotation_y(-self.rotation()) * self.sun_local()
    }
    /// The sun as a light: the direction to it and its radiance (RGB) with the atmosphere's
    /// transmittance; `None` while it is below the horizon.
    pub fn sun(&self) -> Option<(Vec3, Vec3)> {
        let d = self.sun_direction();
        let t = Atmosphere::bare(self.turbidity).march_transmittance(self.observer(), d.y)?;
        let s = (self.sun_size * 0.5).to_radians().sin();
        Some((d, t * self.intensity / (s * s)))
    }
    /// 1 − cos of the sun's angular radius (as 2·sin² of half of it), the analytic light's extent.
    pub fn sun_extent(&self) -> f32 {
        2. * (self.sun_size * 0.25).to_radians().sin().powi(2)
    }
    /// Everything the baked map depends on (not the sun's size, visibility or the rotation,
    /// which the GPU applies).
    pub fn map_key(&self) -> String {
        format!(
            "sky:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.solar_time(),
            self.day,
            self.year,
            self.julian_date(),
            self.longitude,
            self.latitude,
            self.turbidity,
            self.intensity,
            self.ground,
            self.altitude,
            self.stars,
            self.limiting_magnitude,
            self.moon,
            self.milky_way,
            self.airglow,
            self.airglow_warmth,
            self.light_pollution
        )
    }
    /// The air's transmittance straight up from the observer (the stars' optical depth).
    pub fn zenith_transmittance(&self) -> Vec3 {
        Atmosphere::bare(self.turbidity)
            .march_transmittance(self.observer(), 1.)
            .unwrap_or(Vec3::ZERO)
    }
    fn observer(&self) -> f32 {
        GROUND + self.altitude.max(0.001)
    }
}

// Atmosphere, in km (Hillaire 2020's Earth).
const GROUND: f32 = 6360.;
const TOP: f32 = 6460.;
const RAYLEIGH: Vec3 = Vec3::new(5.802e-3, 13.558e-3, 33.1e-3);
const MIE_SCATTER: f32 = 3.996e-3;
const MIE_EXTINCT: f32 = 4.44e-3;
const OZONE: Vec3 = Vec3::new(0.65e-3, 1.881e-3, 0.085e-3);
const MIE_G: f32 = 0.8;

const T_ROWS: usize = 64;
const T_COLS: usize = 256;
const MS_SIZE: usize = 32;

struct Atmosphere {
    /// Aerosol density scale (turbidity − 1).
    mie: f32,
    /// Transmittance to the top of the atmosphere by altitude (rows) and cosine (columns).
    transmittance: Vec<Vec3>,
    /// Hillaire's Ψ_ms by altitude and sun cosine.
    multiple: Vec<Vec3>,
    /// The observer's distance from the earth's centre, km.
    observer: f32,
}

/// Distance along a ray from radius `r` with cosine `mu` to the sphere of radius `big`
/// (outside intersection), when the ray starts inside it.
fn to_sphere(r: f32, mu: f32, big: f32) -> f32 {
    let d = r * r * (mu * mu - 1.) + big * big;
    (-r * mu + d.max(0.).sqrt()).max(0.)
}
/// Distance to the ground, if the ray hits it.
fn to_ground(r: f32, mu: f32) -> Option<f32> {
    let d = r * r * (mu * mu - 1.) + GROUND * GROUND;
    (mu < 0. && d >= 0.).then(|| (-r * mu - d.sqrt()).max(0.))
}
/// Rayleigh and Mie scattering and total extinction at radius `r`.
fn medium(r: f32, mie: f32) -> (Vec3, f32, Vec3) {
    let h = (r - GROUND).max(0.);
    let rayleigh = RAYLEIGH * (-h / 8.).exp();
    let m = mie * (-h / 1.2).exp();
    let ozone = OZONE * (1. - (h - 25.).abs() / 15.).max(0.);
    (
        rayleigh,
        MIE_SCATTER * m,
        rayleigh + Vec3::splat(MIE_EXTINCT * m) + ozone,
    )
}
fn rayleigh_phase(nu: f32) -> f32 {
    3. / (16. * PI) * (1. + nu * nu)
}
fn mie_phase(nu: f32) -> f32 {
    let g = MIE_G;
    3. / (8. * PI) * (1. - g * g) * (1. + nu * nu)
        / ((2. + g * g) * (1. + g * g - 2. * g * nu).max(1e-6).powf(1.5))
}
fn exp3(v: Vec3) -> Vec3 {
    Vec3::new(v.x.exp(), v.y.exp(), v.z.exp())
}
/// Cosine ↔ column: s · |s| puts more columns near the horizon, where transmittance changes
/// fastest.
fn mu_of(u: f32) -> f32 {
    let s = u * 2. - 1.;
    s * s.abs()
}
fn u_of(mu: f32) -> f32 {
    (mu.signum() * mu.abs().sqrt() + 1.) * 0.5
}
/// Altitude ↔ row (squared, for more rows low down).
fn r_of(v: f32) -> f32 {
    GROUND + v * v * (TOP - GROUND)
}
fn v_of(r: f32) -> f32 {
    ((r - GROUND) / (TOP - GROUND)).clamp(0., 1.).sqrt()
}
fn bilinear(table: &[Vec3], rows: usize, cols: usize, v: f32, u: f32) -> Vec3 {
    let x = (u * (cols - 1) as f32).clamp(0., (cols - 1) as f32);
    let y = (v * (rows - 1) as f32).clamp(0., (rows - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(cols - 1), (y0 + 1).min(rows - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| table[y * cols + x];
    (at(x0, y0) * (1. - fx) + at(x1, y0) * fx) * (1. - fy)
        + (at(x0, y1) * (1. - fx) + at(x1, y1) * fx) * fy
}

impl Atmosphere {
    /// Without the tables: enough for `march_transmittance` (the sun light is computed every
    /// time the materials are packed).
    fn bare(turbidity: f32) -> Self {
        Self {
            mie: (turbidity - 1.).max(0.),
            transmittance: vec![],
            multiple: vec![],
            observer: GROUND + 0.05,
        }
    }
    fn new(turbidity: f32) -> Self {
        let mut a = Self::bare(turbidity);
        a.transmittance = parallel(T_ROWS * T_COLS, |i| {
            let r = r_of((i / T_COLS) as f32 / (T_ROWS - 1) as f32);
            let mu = mu_of((i % T_COLS) as f32 / (T_COLS - 1) as f32);
            a.march_transmittance(r, mu).unwrap_or(Vec3::ZERO)
        });
        a.multiple = parallel(MS_SIZE * MS_SIZE, |i| {
            let r = r_of((i / MS_SIZE) as f32 / (MS_SIZE - 1) as f32);
            let mu = (i % MS_SIZE) as f32 / (MS_SIZE - 1) as f32 * 2. - 1.;
            a.psi(r, mu)
        });
        a
    }
    /// Transmittance from radius `r` along cosine `mu` to space; `None` if the ground is hit.
    fn march_transmittance(&self, r: f32, mu: f32) -> Option<Vec3> {
        if to_ground(r, mu).is_some() {
            return None;
        }
        let length = to_sphere(r, mu, TOP);
        let n = 40;
        let mut depth = Vec3::ZERO;
        for i in 0..n {
            let t = (i as f32 + 0.5) / n as f32 * length;
            let rp = (r * r + t * t + 2. * r * t * mu).sqrt();
            depth += medium(rp, self.mie).2 * (length / n as f32);
        }
        Some(exp3(-depth))
    }
    fn transmittance(&self, r: f32, mu: f32) -> Vec3 {
        // Below the horizon of this altitude the sun is behind the Earth.
        let horizon = -(1. - (GROUND / r).powi(2)).max(0.).sqrt();
        if mu < horizon {
            return Vec3::ZERO;
        }
        bilinear(&self.transmittance, T_ROWS, T_COLS, v_of(r), u_of(mu))
    }
    fn multiple(&self, r: f32, mu_s: f32) -> Vec3 {
        bilinear(&self.multiple, MS_SIZE, MS_SIZE, v_of(r), (mu_s + 1.) * 0.5)
    }
    /// Hillaire's Ψ_ms: second-order light reaching a point from all directions (isotropic
    /// phase), summed as a geometric series of the share f_ms that scatters again.
    fn psi(&self, r: f32, mu_s: f32) -> Vec3 {
        let sun = Vec3::new((1. - mu_s * mu_s).max(0.).sqrt(), mu_s, 0.);
        let dirs = 64;
        let (mut l2, mut f_ms) = (Vec3::ZERO, Vec3::ZERO);
        let albedo = 0.3;
        for k in 0..dirs {
            // Fibonacci sphere.
            let y = 1. - (k as f32 + 0.5) / dirs as f32 * 2.;
            let phi = k as f32 * 2.399_963;
            let s = (1. - y * y).sqrt();
            let d = Vec3::new(s * phi.cos(), y, s * phi.sin());
            let ground = to_ground(r, y);
            let length = ground.unwrap_or_else(|| to_sphere(r, y, TOP));
            let n = 20;
            let dt = length / n as f32;
            let mut throughput = Vec3::ONE;
            for i in 0..n {
                let t = (i as f32 + 0.5) * dt;
                let p = Vec3::new(0., r, 0.) + d * t;
                let rp = p.length();
                let (ray, mie, ext) = medium(rp, self.mie);
                let scatter = ray + Vec3::splat(mie);
                let step = exp3(-ext * dt);
                // ∫ over the step of the transmittance, divided by the extinction.
                let integral = (Vec3::ONE - step) / ext.max(Vec3::splat(1e-9));
                let ts = self.transmittance(rp, p.dot(sun) / rp);
                l2 += throughput * scatter * ts * integral / (4. * PI);
                f_ms += throughput * scatter * integral;
                throughput *= step;
            }
            if let Some(t) = ground {
                let p = Vec3::new(0., r, 0.) + d * t;
                let mu = p.dot(sun) / p.length();
                l2 += throughput * self.transmittance(p.length(), mu) * mu.max(0.) * albedo / PI;
            }
        }
        // Each direction had weight 4π / dirs, and the isotropic phase of the gathering point
        // another 1 / 4π: a mean over the directions. (The in-scattering inside l2 already
        // carries its own 1 / 4π; f_ms is the share scattered again, without one.)
        let w = 1. / dirs as f32;
        l2 *= w;
        f_ms *= w;
        l2 / (Vec3::ONE - f_ms).max(Vec3::splat(1e-3))
    }
    /// Sky radiance (for a sun irradiance of 1 outside the atmosphere) seen from the observer
    /// along `d`, with the sun direction `sun`; `ground` is the light leaving the ground per
    /// unit albedo-weighted irradiance, added where the ray hits it.
    fn radiance(&self, d: Vec3, sun: Vec3, ground: Vec3) -> Vec3 {
        let r = self.observer;
        let hit = to_ground(r, d.y);
        let length = hit.unwrap_or_else(|| to_sphere(r, d.y, TOP));
        let nu = d.dot(sun);
        let (pr, pm) = (rayleigh_phase(nu), mie_phase(nu));
        // Rays into the ground are short (the observer is 50 m up) and nearly uniform.
        let n = if hit.is_some() { 12 } else { 40 };
        let mut throughput = Vec3::ONE;
        let mut sum = Vec3::ZERO;
        let mut previous = 0.;
        for i in 0..n {
            // Quadratic spacing: the air is densest near the observer.
            let t = ((i + 1) as f32 / n as f32).powi(2) * length;
            let dt = t - previous;
            let mid = previous + dt * 0.5;
            previous = t;
            let p = Vec3::new(0., r, 0.) + d * mid;
            let rp = p.length();
            let mu_s = p.dot(sun) / rp;
            let (ray, mie, ext) = medium(rp, self.mie);
            let step = exp3(-ext * dt);
            let integral = (Vec3::ONE - step) / ext.max(Vec3::splat(1e-9));
            let single = self.transmittance(rp, mu_s) * (ray * pr + Vec3::splat(mie * pm));
            let multiple = self.multiple(rp, mu_s) * (ray + Vec3::splat(mie));
            sum += throughput * (single + multiple) * integral;
            throughput *= step;
        }
        if hit.is_some() {
            sum += throughput * ground;
        }
        sum
    }
}

/// `(0..n).map(f)` on all cores, in order.
fn parallel(n: usize, f: impl Fn(usize) -> Vec3 + Sync) -> Vec<Vec3> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = n.div_ceil(threads).max(1);
    let mut out = vec![Vec3::ZERO; n];
    std::thread::scope(|s| {
        for (c, part) in out.chunks_mut(chunk).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (i, v) in part.iter_mut().enumerate() {
                    *v = f(c * chunk + i);
                }
            });
        }
    });
    out
}
/// The sky as an equirectangular map in the environment's layout (environment.rs; north at
/// −z, the GPU applies `rotation`): linear RGB rows from the zenith down.
pub fn bake(sky: &PhysicalSky) -> anyhow::Result<crate::environment::EnvMap> {
    let (w, h) = if cfg!(debug_assertions) {
        (512, 256)
    } else {
        (1024, 512)
    };
    let mut atmosphere = Atmosphere::new(sky.turbidity);
    atmosphere.observer = sky.observer();
    // After astronomical twilight (sun 18° down) the moon takes the sun's place: its light is
    // sunlight 2.5e-6 times as strong at full moon, times the lit fraction.
    let moon = sky.moon_local();
    let lit = (1. - (2. * PI * sky.moon_age()).cos()) * 0.5;
    let moonlit = sky.moon && sky.sun_local().y < -0.31;
    let sun = if moonlit { moon } else { sky.sun_local() };
    let scale = PI * sky.intensity * if moonlit { 2.5e-6 * lit.max(0.02) } else { 1. };
    let direction = |x: usize, y: usize| {
        let theta = PI * (y as f32 + 0.5) / h as f32;
        let phi = 2. * PI * ((x as f32 + 0.5) / w as f32 - 0.5);
        Vec3::new(
            theta.sin() * phi.sin(),
            theta.cos(),
            -theta.sin() * phi.cos(),
        )
    };
    let rows = |range: std::ops::Range<usize>, ground: Vec3| -> Vec<Vec3> {
        let first = range.start * w;
        parallel(range.len() * w, |i| {
            let k = first + i;
            atmosphere.radiance(direction(k % w, k / w), sun, ground) * scale
        })
    };
    // The sky first; its irradiance on the ground then lights the lower half with the sun.
    let mut texels = rows(0..h / 2, Vec3::ZERO);
    // The night sky's own light (Milky Way, airglow, light pollution) before the ground is lit,
    // so it lights the ground too. Day skies are 1e7 times brighter: nothing changes there.
    let night_sky = night_glow(sky, &atmosphere, w, h / 2, &direction);
    for (t, n) in texels.iter_mut().zip(night_sky) {
        *t += n;
    }
    let mut sky_irradiance = Vec3::ZERO;
    for (k, l) in texels.iter().enumerate() {
        let theta = PI * ((k / w) as f32 + 0.5) / h as f32;
        let solid = (PI / h as f32) * (2. * PI / w as f32) * theta.sin();
        sky_irradiance += *l * theta.cos() * solid;
    }
    let sun_irradiance = atmosphere.transmittance(GROUND, sun.y) * sun.y.max(0.) * scale;
    // `radiance` multiplies the ground term by `scale` with the rest, hence the division.
    let ground = (sun_irradiance + sky_irradiance) * sky.ground / PI / scale.max(1e-12);
    texels.extend(rows(h / 2..h, ground));
    // A floor far below any real sky keeps the map from being black with every night term
    // off (the sampling tables need light).
    let night = Vec3::splat(1e-12) * sky.intensity.max(1e-3);
    let solid = |y: usize| {
        (PI / h as f32) * (2. * PI / w as f32) * (PI * (y as f32 + 0.5) / h as f32).sin().max(1e-4)
    };
    // The moon's disc (at least a texel across): sunlight 2.5e-6 times as strong at full moon.
    if sky.moon && moon.y > 0. {
        let t = atmosphere
            .march_transmittance(atmosphere.observer, moon.y)
            .unwrap_or(Vec3::ZERO);
        let radius = (0.26f32).to_radians().max(PI / h as f32);
        let mut inside = vec![];
        for y in 0..h / 2 {
            for x in 0..w {
                if direction(x, y).dot(moon) > radius.cos() {
                    inside.push(y * w + x);
                }
            }
        }
        let area: f32 = inside.iter().map(|k| solid(k / w)).sum();
        let e = PI * sky.intensity * 2.5e-6 * lit.max(0.02);
        for k in inside {
            texels[k] += Vec3::new(1., 0.97, 0.92) * t * e / area.max(1e-9);
        }
    }
    let mut map = crate::environment::build(
        w,
        h,
        texels.into_iter().map(|l| (l + night).to_array()).collect(),
    )?;
    // The catalogue's stars are point lights (stars.rs), not texels: the map only says whether
    // they are on and the share of sky samples they take.
    if let Some(table) =
        crate::stars::Table::get().filter(|t| sky.stars > 0. && t.kept(sky.limiting_magnitude) > 0)
    {
        map.stars = true;
        map.star_share = crate::stars::share(sky, table, map.inv_norm);
    }
    Ok(map)
}

/// Surface brightness (V mag/arcsec²) to scene radiance for a sun of intensity 1:
/// L = 1.08e5 · 10^(−0.4 μ) cd/m², divided by `SCENE_CD_M2`.
pub fn mag_arcsec2_to_radiance(mu: f32) -> f32 {
    cd_to_scene(1.08e5 * 10f32.powf(-0.4 * mu))
}
/// The Milky Way calibration (engine notes, "Night sky: Milky Way, airglow, light
/// pollution"): the map's luminance `v` over the large Sagittarius star cloud (2° box at
/// RA 270.5°, Dec −28.5°) is `MW_REF_VALUE` and that cloud is 20.2 mag/arcsec² V; the map is
/// display-referred with a contrast stretch, and radiance ∝ v^`MW_POWER` puts Cygnus at 21.6
/// (measured; the sky has about 21.3) and the galactic pole at 23.1 (the sky: 23.0-23.3).
const MW_REF_VALUE: f32 = 0.198;
const MW_REF_MAG: f32 = 20.2;
const MW_POWER: f32 = 0.5;
/// Airglow at the zenith of a dark site, and the height of its layer (van Rhijn), km.
const AIRGLOW_MAG: f32 = 22.0;
const AIRGLOW_HEIGHT: f32 = 90.;
/// OI 557.7 nm makes airglow a little green; near neutral for the eye (luminance 1).
const AIRGLOW_TINT: Vec3 = Vec3::new(0.92, 1.03, 0.92);
/// The camera's: warmer and greener (luminance 1 as well), `airglow_warmth` 1.
const AIRGLOW_WARM_TINT: Vec3 = Vec3::new(1.08, 1.02, 0.70);
/// Light pollution at the zenith at strength 1: 22.6 mag/arcsec², which with the airglow makes
/// a 21.5 dark rural zenith. Warm (sodium and warm LED).
const POLLUTION_MAG: f32 = 22.6;
const POLLUTION_TINT: Vec3 = Vec3::new(1.25, 0.98, 0.68);

/// The Milky Way map in scene radiance (celestial equirect, RGB rows from Dec +90°).
struct MilkyWay {
    w: usize,
    h: usize,
    rgb: Vec<Vec3>,
}
impl MilkyWay {
    fn load() -> anyhow::Result<Self> {
        use anyhow::Context;
        let path = crate::mesh::resolve("sky/milkyway.exr");
        let image = image::open(&path)
            .with_context(|| format!("Reading {}", path.display()))?
            .into_rgb32f();
        let (w, h) = (image.width() as usize, image.height() as usize);
        let reference = mag_arcsec2_to_radiance(MW_REF_MAG);
        let rgb = image
            .pixels()
            .map(|p| {
                let c = Vec3::from(p.0.map(|v| if v.is_finite() { v.max(0.) } else { 0. }));
                let y = crate::environment::luminance(c.to_array());
                if y <= 0. {
                    return Vec3::ZERO;
                }
                // The stretch is undone on the luminance; the colour keeps its chromaticity.
                c * (reference * (y / MW_REF_VALUE).powf(MW_POWER) / y)
            })
            .collect();
        Ok(Self { w, h, rgb })
    }
    /// The map, loaded once (`None` with a logged error if the file is missing).
    fn get() -> Option<&'static Self> {
        static MAP: std::sync::OnceLock<Option<MilkyWay>> = std::sync::OnceLock::new();
        MAP.get_or_init(|| {
            Self::load()
                .map_err(|e| eprintln!("Milky Way map: {e:#}"))
                .ok()
        })
        .as_ref()
    }
    /// Bilinear radiance towards the J2000 unit vector `c`.
    fn sample(&self, c: Vec3) -> Vec3 {
        let ra = c.y.atan2(c.x);
        let dec = c.z.clamp(-1., 1.).asin();
        // Column x has RA (0.5 − (x + 0.5)/W) · 2π, row y Dec π/2 − (y + 0.5)/H · π.
        let fx = (0.5 - ra / (2. * PI)) * self.w as f32 - 0.5;
        let fy = (0.5 - dec / PI) * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |x: f32, y: f32| {
            let x = (x as i64).rem_euclid(self.w as i64) as usize;
            let y = (y as i64).clamp(0, self.h as i64 - 1) as usize;
            self.rgb[y * self.w + x]
        };
        (at(x0, y0) * (1. - tx) + at(x0 + 1., y0) * tx) * (1. - ty)
            + (at(x0, y0 + 1.) * (1. - tx) + at(x0 + 1., y0 + 1.) * tx) * ty
    }
}

/// The night sky's light above the horizon for the first `rows` rows of a `w`-wide bake:
/// the Milky Way and the airglow (above the air, dimmed by its transmittance) and light
/// pollution (scattered inside the air: ∝ 1 − transmittance, so it rises to the horizon).
fn night_glow(
    sky: &PhysicalSky,
    atmosphere: &Atmosphere,
    w: usize,
    rows: usize,
    direction: &(impl Fn(usize, usize) -> Vec3 + Sync),
) -> Vec<Vec3> {
    let milky_way = (sky.milky_way > 0.).then(MilkyWay::get).flatten();
    let celestial_from_local = sky.local_from_celestial().transpose();
    let r = atmosphere.observer;
    let airglow = mag_arcsec2_to_radiance(AIRGLOW_MAG) * sky.airglow;
    let airglow_tint = AIRGLOW_TINT.lerp(AIRGLOW_WARM_TINT, sky.airglow_warmth);
    // van Rhijn: the path through a thin layer at height h grows as 1/√(1 − (R/(R+h))² sin²z).
    let k = (GROUND / (GROUND + AIRGLOW_HEIGHT)).powi(2);
    let pollution = mag_arcsec2_to_radiance(POLLUTION_MAG) * sky.light_pollution;
    let zenith_haze =
        crate::environment::luminance((Vec3::ONE - atmosphere.transmittance(r, 1.)).to_array())
            .max(1e-6);
    parallel(rows * w, |i| {
        let d = direction(i % w, i / w);
        let t = atmosphere.transmittance(r, d.y);
        if t == Vec3::ZERO {
            return Vec3::ZERO;
        }
        let mut l = airglow_tint * (airglow / (1. - k * (1. - d.y * d.y)).max(1e-6).sqrt());
        if let Some(m) = milky_way {
            l += m.sample(celestial_from_local * d) * sky.milky_way;
        }
        l *= t;
        let haze = crate::environment::luminance((Vec3::ONE - t).to_array()) / zenith_haze;
        (l + POLLUTION_TINT * (pollution * haze)) * sky.intensity
    })
}

/// Recent bakes by `map_key`, newest last: scrubbing the time back and forth (or replaying
/// a keyed day) reuses them instead of baking again.
static CACHE: std::sync::Mutex<Vec<(String, std::sync::Arc<crate::environment::EnvMap>)>> =
    std::sync::Mutex::new(Vec::new());
const CACHED: usize = 12;
/// A cached bake of `key`, if there is one.
pub fn cached(key: &str) -> Option<std::sync::Arc<crate::environment::EnvMap>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.iter().find(|(k, _)| k == key).map(|(_, m)| m.clone())
}
/// `bake` through the cache (safe to call from a worker thread).
pub fn bake_cached(
    sky: &PhysicalSky,
) -> anyhow::Result<std::sync::Arc<crate::environment::EnvMap>> {
    let key = sky.map_key();
    if let Some(m) = cached(&key) {
        return Ok(m);
    }
    let map = std::sync::Arc::new(bake(sky)?);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(k, _)| *k != key);
    cache.push((key, map.clone()));
    if cache.len() > CACHED {
        cache.remove(0);
    }
    Ok(map)
}
/// What the GPU's environment slot should hold: the sky's map, an HDRI path, or nothing.
pub fn environment_key(s: &crate::scene::Settings) -> String {
    if s.physical_sky.on {
        s.physical_sky.map_key()
    } else {
        format!("{}{}", s.hdri, if s.hdri_sun { "|sun" } else { "" })
    }
}
/// Loads or bakes the environment `environment_key` names into the GPU.
///
/// # Safety
/// As `Gpu::set_environment`.
pub unsafe fn upload_environment(
    g: &mut crate::Gpu,
    s: &crate::scene::Settings,
) -> anyhow::Result<()> {
    unsafe {
        if s.physical_sky.on {
            g.set_environment(Some(&*bake_cached(&s.physical_sky)?))
        } else if s.hdri.is_empty() {
            g.set_environment(None)
        } else {
            g.set_environment(Some(&crate::environment::load(&s.hdri, s.hdri_sun)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sky(time: f32) -> PhysicalSky {
        PhysicalSky {
            on: true,
            time,
            ..Default::default()
        }
    }
    #[test]
    fn the_sun_rises_in_the_east_and_peaks_at_noon() {
        let at = |t: f32| sky(t).sun_local();
        // June, 45° north: up at 6:00 already, highest at noon (90 − 45 + 23.4), due south.
        assert!(at(6.).y > 0. && at(6.).x > 0.);
        assert!(at(9.).x > 0. && at(15.).x < 0.);
        let noon = at(12.);
        assert!((noon.y.asin().to_degrees() - 68.44).abs() < 0.5, "{noon}");
        assert!(noon.z > 0., "the noon sun is to the south (+z): {noon}");
        assert!(at(0.).y < 0.);
        // December: a low noon sun.
        let winter = PhysicalSky {
            day: 355.,
            ..sky(12.)
        }
        .sun_local();
        assert!(
            (winter.y.asin().to_degrees() - 21.56).abs() < 0.5,
            "{winter}"
        );
    }
    #[test]
    fn old_scenes_load_without_a_year() {
        let old: PhysicalSky = serde_json::from_str(r#"{"on":true,"time":9.5,"day":100}"#).unwrap();
        assert_eq!(old.year, 2026);
        assert!(old.validate().is_ok());
        assert!(PhysicalSky::default().is_default());
        assert!(
            !PhysicalSky {
                year: 2027,
                ..Default::default()
            }
            .is_default()
        );
        for year in [1899, 2101] {
            assert!(
                PhysicalSky {
                    year,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        // The year is part of the bake key.
        let (a, b) = (
            PhysicalSky::default(),
            PhysicalSky {
                year: 2030,
                ..Default::default()
            },
        );
        assert_ne!(a.map_key(), b.map_key());
    }
    /// The sun model this module used before the exact chain: Cooper's declination and the
    /// approximate equation of time. Kept to measure how far old scenes' suns move.
    fn cooper_sun(s: &PhysicalSky) -> Vec3 {
        let b = 2. * PI * (s.day - 81.) / 364.;
        let eot = 9.87 * (2. * b).sin() - 7.53 * b.cos() - 1.5 * b.sin();
        let solar = if s.clock {
            (s.time + s.longitude / 15. - s.utc_offset + eot / 60.).rem_euclid(24.)
        } else {
            s.time
        };
        let decl = (23.44f32).to_radians() * (2. * PI * (284. + s.day) / 365.).sin();
        let hour = (15. * (solar - 12.)).to_radians();
        let lat = s.latitude.to_radians();
        let east = -decl.cos() * hour.sin();
        let north = lat.cos() * decl.sin() - lat.sin() * decl.cos() * hour.cos();
        let up = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour.cos();
        Vec3::new(east, up, -north).normalize()
    }
    fn degrees_between(a: Vec3, b: Vec3) -> f32 {
        a.dot(b).clamp(-1., 1.).acos().to_degrees()
    }
    #[test]
    fn the_sun_is_the_exact_chain_and_close_to_the_old_model() {
        // The sun is the Meeus position through the stars' rotation; the old Cooper model
        // stays within about 1.2 degrees of it.
        let mut worst = 0f32;
        for (year, day, time, lat, lon) in [
            (2026, 80., 9., 45., 0.),
            (2026, 172., 12., 45., 0.),
            (2026, 172., 17., 52., 13.4),
            (2026, 266., 7., -34., 151.),
            (2026, 355., 12., 45., -73.),
            (2024, 45., 15., 60., 24.9),
            (2030, 300., 11., 10., 100.),
            (2026, 355., 21., 45., 0.),
        ] {
            let s = PhysicalSky {
                on: true,
                year,
                day,
                time,
                latitude: lat,
                longitude: lon,
                ..Default::default()
            };
            let jd = s.julian_date();
            let (ra, dec) = crate::astro::sun_equatorial(jd);
            let lst = crate::astro::local_sidereal_time(jd, (lon as f64).to_radians());
            let v = crate::astro::equatorial_to_local((lat as f64).to_radians(), lst)
                * crate::astro::equatorial_vector(ra, dec);
            let real = Vec3::new(v.x as f32, v.y as f32, v.z as f32);
            // The of-date path (no precession round trip) and the J2000 path agree.
            let chain = degrees_between(real, s.sun_local());
            assert!(chain < 0.001, "{chain}");
            let angle = degrees_between(real, cooper_sun(&s));
            println!("SUN_MODEL_DIFF y{year} d{day} t{time} lat{lat} lon{lon}: {angle:.3} deg");
            worst = worst.max(angle);
        }
        println!("SUN_MODEL_WORST {worst:.3} deg");
        assert!(worst < 1.5, "{worst}");
    }
    #[test]
    fn solar_time_puts_the_sun_on_the_meridian_at_noon() {
        // Apparent solar time 12:00: hour angle zero, so the sun is due south (x = 0) at any
        // longitude, any day.
        for (day, lon) in [(40., 0.), (172., 100.), (300., -120.), (355., 179.)] {
            let s = PhysicalSky {
                day,
                longitude: lon,
                ..sky(12.)
            };
            let v = s.sun_local();
            let h = v.x.atan2(v.z).to_degrees();
            assert!(
                h.abs() < 0.05,
                "day {day} lon {lon}: {h} deg off the meridian"
            );
            // Clock mode reaches the same instant: civil noon + longitude correction + EoT.
            let c = PhysicalSky {
                clock: true,
                utc_offset: 0.,
                time: 12. - lon / 15. - s.equation_of_time() / 60.,
                ..s.clone()
            };
            assert!(
                degrees_between(c.sun_local(), s.sun_local()) < 0.05,
                "day {day} lon {lon}"
            );
        }
    }
    #[test]
    fn old_physical_sky_scenes_move_their_sun_a_little() {
        // How far the sun of each built-in physical-sky scene moves from the old model.
        let mut worst = 0f32;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/scenes/builtin");
        let mut n = 0;
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            fn find(v: &serde_json::Value) -> Option<&serde_json::Value> {
                match v {
                    serde_json::Value::Object(m) => {
                        m.get("physical_sky").or_else(|| m.values().find_map(find))
                    }
                    serde_json::Value::Array(a) => a.iter().find_map(find),
                    _ => None,
                }
            }
            let Some(v) = find(&json) else { continue };
            let Ok(s) = serde_json::from_value::<PhysicalSky>(v.clone()) else {
                continue;
            };
            if !s.on {
                continue;
            }
            let d = degrees_between(s.sun_local(), cooper_sun(&s));
            let (old, new) = (cooper_sun(&s), s.sun_local());
            println!(
                "SCENE_SUN_SHIFT {:?}: {d:.3} deg (elevation {:.2} -> {:.2}, moon age {:.3})",
                path.file_stem().unwrap(),
                old.y.asin().to_degrees(),
                new.y.asin().to_degrees(),
                s.moon_age()
            );
            worst = worst.max(d);
            n += 1;
        }
        println!("SCENE_SUN_SHIFT_WORST {worst:.3} deg over {n} scenes");
        assert!(n >= 10 && worst < 1.5, "{n} {worst}");
    }
    #[test]
    fn the_moon_is_real_and_its_phase_matches_its_position() {
        // Full moon 2026-01-03 10:03 UT = day 3, 10.05 h, at longitude 0 in clock time.
        let full = PhysicalSky {
            clock: true,
            day: 3.,
            time: 10.05,
            ..sky(0.)
        };
        println!("FULL_MOON age {:.4}", full.moon_age());
        assert!((full.moon_age() - 0.5).abs() < 0.01, "{}", full.moon_age());
        // At full moon the moon is opposite the sun in the sky: the angle is near 180 degrees
        // (short of it by the moon's ecliptic latitude, up to 5.3).
        let el = degrees_between(full.sun_local(), full.moon_local());
        println!("FULL_MOON elongation {el:.2} deg");
        assert!(el > 174.5, "{el}");
        // New moon 2026-02-17 12:01 UT: age near 0 (or 1), the moon beside the sun.
        let new = PhysicalSky {
            day: 48.,
            time: 12.02,
            ..full.clone()
        };
        let age = new.moon_age();
        assert!(age.min(1. - age) < 0.01, "{age}");
        assert!(degrees_between(new.sun_local(), new.moon_local()) < 6.);
        // Waxing: a week later the age is about a quarter and the moon is east of the sun.
        let q = PhysicalSky {
            day: 55.,
            time: 12.,
            ..new
        };
        assert!((q.moon_age() - 0.25).abs() < 0.03, "{}", q.moon_age());
        // The moon stays within about 29 degrees of the ecliptic's celestial equator band:
        // its declination never passes 29 degrees.
        for day in (1..=366).step_by(5) {
            let jd = crate::astro::julian_date_of_day(2026, day as f32, 0.);
            assert!(crate::astro::moon_equatorial(jd).1.abs().to_degrees() < 29.5);
        }
    }
    #[test]
    fn north_turns_the_sun_with_the_map() {
        let mut s = sky(9.);
        let before = s.sun_direction();
        s.north = 90.;
        let after = s.sun_direction();
        // The shader reads φ = atan2(x, −z) − rotation, so the world φ grows by the rotation.
        let phi = |d: Vec3| d.x.atan2(-d.z);
        let turn = (phi(after) - phi(before)).rem_euclid(2. * PI);
        assert!((turn - PI / 2.).abs() < 1e-4, "{turn}");
        assert!((after.y - before.y).abs() < 1e-6);
    }
    #[test]
    fn the_sun_reddens_and_sets() {
        let noon = sky(12.).sun().unwrap().1;
        let s = sky(19.4);
        let evening = s.sun().map(|(_, l)| l);
        assert!(sky(23.).sun().is_none());
        let evening = evening.expect("the sun is still up at 19:24 in June at 45°");
        assert!(
            evening.z / evening.x < noon.z / noon.x * 0.5,
            "{noon} {evening}"
        );
        // Noon: a white surface facing the sun reads most of the intensity (irradiance
        // π·L·sin² of the radius, over π).
        let l = noon * (0.265f32).to_radians().sin().powi(2);
        assert!((0.6..1.).contains(&l.y), "{l}");
    }
    #[test]
    fn the_sun_record_is_finite_and_follows_north() {
        let mut s = crate::scene::Settings {
            physical_sky: sky(12.),
            ..Default::default()
        };
        let pack = |s: &crate::scene::Settings| {
            let m: Vec<f32> = bytemuck::cast_slice(&crate::lighting::materials(s)).to_vec();
            assert!(m.iter().all(|v| v.is_finite()));
            m
        };
        let before = pack(&s);
        s.physical_sky.north = 30.;
        assert_ne!(
            before,
            pack(&s),
            "turning the sky must change the material key"
        );
        s.physical_sky.intensity = 0.;
        pack(&s);
        let vertices = crate::scene::geometry(&s);
        let (emitters, _) = crate::lighting::emitter_table(
            &vertices,
            &crate::lighting::materials(&s),
            &crate::lighting::analytic(&s),
        );
        assert!(emitters.iter().all(|e| e.edge1_cdf[3].is_finite()));
    }
    #[test]
    fn the_map_is_a_blue_sky_brighter_near_the_sun() {
        let started = std::time::Instant::now();
        let env = bake(&sky(12.)).unwrap();
        println!("SKY_BAKE_MS {:.1}", started.elapsed().as_secs_f64() * 1e3);
        let (w, h) = (env.width, env.height);
        let zenith = env.texels[w / 2];
        assert!(zenith[2] > zenith[0] * 1.5, "{zenith:?}");
        // The noon sun is south (+z), at u = 0 / 1 in this layout; the opposite azimuth is
        // u = 0.5. Compare the two at the sun's elevation (θ ≈ 21.6°).
        let y = (21.6 / 180. * h as f32) as usize;
        let toward = env.texels[y * w];
        let away = env.texels[y * w + w / 2];
        assert!(toward[1] > away[1] * 2., "{toward:?} {away:?}");
        // A white surface sees sky light of the right order (about a fifth of the sun).
        let sky_irradiance: f32 = (0..w * h / 2)
            .map(|k| {
                let theta = PI * ((k / w) as f32 + 0.5) / h as f32;
                env.texels[k][1]
                    * theta.cos()
                    * theta.sin()
                    * (PI / h as f32)
                    * (2. * PI / w as f32)
            })
            .sum();
        assert!(
            (0.05..0.6).contains(&(sky_irradiance / PI)),
            "{sky_irradiance}"
        );
        // Night still builds (the floor keeps the tables valid).
        assert!(bake(&sky(1.)).is_ok());
    }

    #[test]
    fn a_clock_time_before_utc_midnight_is_the_previous_day() {
        // 01:00 at UTC+5 is 20:00 UT the day before; wrapping the hour would keep the date.
        let s = PhysicalSky {
            clock: true,
            utc_offset: 5.,
            time: 1.,
            day: 100.,
            ..PhysicalSky::default()
        };
        let want = crate::astro::julian_date_of_day(s.year, 99., 20.);
        assert!(
            (s.julian_date() - want).abs() < 1e-6,
            "{} vs {want}",
            s.julian_date()
        );
    }
    #[test]
    fn milky_way_is_calibrated_and_where_the_galactic_centre_is() {
        let m = MilkyWay::get().expect("scenes/sky/milkyway.exr");
        let mag = |ra: f64, dec: f64| {
            let c = crate::astro::equatorial_vector(ra.to_radians(), dec.to_radians());
            let l = m.sample(Vec3::new(c.x as f32, c.y as f32, c.z as f32));
            let cd = scene_to_cd(luminance(l.to_array()));
            -2.5 * (cd / 1.08e5).log10()
        };
        let (sgr, cyg, pole) = (mag(270.5, -28.5), mag(300., 35.), mag(192.8, 27.1));
        eprintln!("mag/arcsec²: Sgr cloud {sgr:.2}, Cygnus {cyg:.2}, galactic pole {pole:.2}");
        assert!((19.5..21.).contains(&sgr) && (20.8..22.).contains(&cyg));
        assert!((22.5..24.).contains(&pole));
        // In the bake: the brightest band texel at night, latitude −30°, 23 h in late July,
        // sits within a few degrees of the galactic centre's computed direction.
        let s = PhysicalSky {
            on: true,
            time: 23.,
            day: 200.,
            latitude: -30.,
            stars: 0.,
            moon: false,
            airglow: 0.,
            light_pollution: 0.,
            ..Default::default()
        };
        let a = {
            let mut a = Atmosphere::new(s.turbidity);
            a.observer = s.observer();
            a
        };
        let (w, h) = (512, 256);
        let direction = |x: usize, y: usize| {
            let theta = PI * (y as f32 + 0.5) / h as f32;
            let phi = 2. * PI * ((x as f32 + 0.5) / w as f32 - 0.5);
            Vec3::new(
                theta.sin() * phi.sin(),
                theta.cos(),
                -theta.sin() * phi.cos(),
            )
        };
        let start = std::time::Instant::now();
        let glow = night_glow(&s, &a, w, h / 2, &direction);
        eprintln!("night_glow {w}x{}: {:?}", h / 2, start.elapsed());
        // The galactic centre (J2000 RA 266.40°, Dec −28.94°).
        let gc = crate::astro::equatorial_vector(266.40f64.to_radians(), (-28.94f64).to_radians());
        let gc = s.local_from_celestial() * Vec3::new(gc.x as f32, gc.y as f32, gc.z as f32);
        let (k, _) = glow
            .iter()
            .enumerate()
            .map(|(k, l)| {
                (
                    k,
                    l.y / a.transmittance(a.observer, direction(k % w, k / w).y).y,
                )
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let peak = direction(k % w, k / w);
        let alt = |v: Vec3| v.y.asin().to_degrees();
        let az = |v: Vec3| v.x.atan2(-v.z).to_degrees().rem_euclid(360.);
        eprintln!(
            "galactic centre alt {:.1} az {:.1}; brightest texel alt {:.1} az {:.1}",
            alt(gc),
            az(gc),
            alt(peak),
            az(peak)
        );
        assert!(gc.dot(peak) > 6f32.to_radians().cos(), "{gc} vs {peak}");
    }

    #[test]
    fn night_glow_vanishes_in_daylight_and_dark_rural_zenith_is_21_5() {
        let night = PhysicalSky {
            time: 0.,
            day: 200.,
            latitude: -30.,
            milky_way: 0.,
            stars: 0.,
            moon: false,
            ..Default::default()
        };
        let mut a = Atmosphere::new(night.turbidity);
        a.observer = night.observer();
        let up = |_: usize, _: usize| Vec3::Y;
        let z = night_glow(&night, &a, 1, 1, &up)[0];
        let mag = -2.5 * (scene_to_cd(luminance(z.to_array())) / 1.08e5).log10();
        eprintln!("zenith airglow + light pollution: {mag:.2} mag/arcsec²");
        assert!((21.2..21.8).contains(&mag), "{mag}");
        let start = std::time::Instant::now();
        let day = bake(&sky(12.)).unwrap();
        eprintln!("bake (day, debug size): {:?}", start.elapsed());
        let _ = day;
        // A noon sky at the zenith is ~1e7 times brighter than the night terms.
        let noon = sky(12.);
        let sun = noon.sun_local();
        let blue = a.radiance(Vec3::Y, sun, Vec3::ZERO) * PI * noon.intensity;
        assert!(
            crate::environment::luminance(z.to_array()) < 1e-6 * blue.y,
            "{z} vs {blue}"
        );
    }
}
