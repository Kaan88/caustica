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

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct PhysicalSky {
    /// Replaces the HDRI and the sky light while on.
    pub on: bool,
    /// Solar time, hours (12 = the sun due south or north).
    pub time: f32,
    /// Day of the year, 1–366 (172 is the June solstice).
    pub day: f32,
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
    /// Star brightness (0: none).
    pub stars: f32,
    /// The moon: its phase follows the date, and at night it lights the sky.
    pub moon: bool,
}
impl Default for PhysicalSky {
    fn default() -> Self {
        Self {
            on: false,
            time: 15.,
            day: 172.,
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
            moon: true,
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
                && (-90. ..=90.).contains(&self.latitude)
                && (-360. ..=360.).contains(&self.north)
                && (1. ..=10.).contains(&self.turbidity)
                && (0. ..=1000.).contains(&self.intensity)
                && (0.05..=20.).contains(&self.sun_size)
                && (0. ..=1.).contains(&self.ground)
                && (-180. ..=180.).contains(&self.longitude)
                && (-14. ..=14.).contains(&self.utc_offset)
                && (0. ..=20.).contains(&self.altitude)
                && (0. ..=1000.).contains(&self.stars),
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
        let b = 2. * PI * (self.day - 81.) / 364.;
        let equation = 9.87 * (2. * b).sin() - 7.53 * b.cos() - 1.5 * b.sin();
        (self.time + self.longitude / 15. - self.utc_offset + equation / 60.).rem_euclid(24.)
    }
    /// The moon's age as a fraction of the synodic month (0 new, 0.5 full), from the date.
    pub fn moon_age(&self) -> f32 {
        ((self.day + 9.) / 29.53).fract()
    }
    /// Unit direction towards the sun in the sky's own frame (north −z, east +x, up +y).
    fn sun_local(&self) -> Vec3 {
        self.body_local(self.solar_time())
    }
    /// The moon: it trails the sun by its age (in hours of the hour angle); the sun's
    /// declination stands in for the moon's (it stays within about 5° of the ecliptic).
    fn moon_local(&self) -> Vec3 {
        self.body_local(self.solar_time() - 24. * self.moon_age())
    }
    fn body_local(&self, solar: f32) -> Vec3 {
        // Declination (Cooper) and hour angle.
        let decl = (23.44f32).to_radians() * (2. * PI * (284. + self.day) / 365.).sin();
        let hour = (15. * (solar - 12.)).to_radians();
        let lat = self.latitude.to_radians();
        let east = -decl.cos() * hour.sin();
        let north = lat.cos() * decl.sin() - lat.sin() * decl.cos() * hour.cos();
        let up = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour.cos();
        Vec3::new(east, up, -north).normalize()
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
            "sky:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.solar_time(),
            self.day,
            self.latitude,
            self.turbidity,
            self.intensity,
            self.ground,
            self.altitude,
            self.stars,
            self.moon
        )
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
    // A faint night floor keeps the map from being black (the sampling tables need light).
    let night = Vec3::new(2e-7, 3e-7, 6e-7) * sky.intensity.max(1e-3);
    let solid = |y: usize| {
        (PI / h as f32) * (2. * PI / w as f32) * (PI * (y as f32 + 0.5) / h as f32).sin().max(1e-4)
    };
    // Stars: about one texel in 150 above the horizon, a steep brightness distribution, a
    // colour from blue-white to orange, dimmed by the air along the view (a day sky drowns
    // them on its own). Energy per star, so the map size changes only their count.
    if sky.stars > 0. {
        for y in 0..h / 2 {
            for x in 0..w {
                let hash = |k: u32| {
                    let mut v = (x as u32).wrapping_mul(0x9e37_79b9)
                        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
                        ^ k.wrapping_mul(0xc2b2_ae35);
                    v ^= v >> 16;
                    v = v.wrapping_mul(0x7feb_352d);
                    v ^= v >> 15;
                    v = v.wrapping_mul(0x846c_a68b);
                    (v ^ (v >> 16)) as f32 / u32::MAX as f32
                };
                if hash(1) > 1. / 150. {
                    continue;
                }
                let d = direction(x, y);
                let Some(t) = atmosphere.march_transmittance(atmosphere.observer, d.y) else {
                    continue;
                };
                let tint = Vec3::new(0.75, 0.85, 1.).lerp(Vec3::new(1., 0.8, 0.55), hash(2));
                let e = 7.5e-8 * hash(3).powi(4).max(0.002) * sky.stars * sky.intensity;
                texels[y * w + x] += tint * t * e / solid(y);
            }
        }
    }
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
    crate::environment::build(
        w,
        h,
        texels.into_iter().map(|l| (l + night).to_array()).collect(),
    )
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
    g: &mut crate::gpu::Gpu,
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
}
