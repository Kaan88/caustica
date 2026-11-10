//! Night sky N3: catalogue stars (`scenes/sky/stars.bin`, `astro::load_stars`) as point lights
//! (engine-notes "Night sky: stars as point lights").
//!
//! - The table stays in the J2000 celestial frame; each frame rotates it into the world with
//!   `Frame::world_from_celestial` (`PhysicalSky::local_from_celestial`, turned by `north`).
//! - Light sampling: a separate power-weighted table (`cdf`, `prob`). A star is a delta
//!   direction, so it is reached only by next-event estimation at non-delta vertices.
//! - Camera rays and pure-delta chains (mirrors, water, glass) find stars with a cone test
//!   through a latitude-longitude grid built once at load (`cell`, `seen`). The cone is half a
//!   pixel across, so a star stays a point at any resolution.
//! - Irradiance per star, relative to the sun's: 10^(−0.4 (V + 26.74)), coloured by B−V
//!   (a blackbody, normalised so the sun's B−V is white), times π · intensity · `stars`, dimmed
//!   by the air (Kasten-Young air mass on the zenith optical depth) and zero below the horizon.
//!
//! The GPU reads `gpu_words` at t18 (`star_data` in common.hlsli) and `Frame::rows` in the
//! constants; the CPU reference uses `Table` and `Frame` themselves.
use glam::{Mat3, Vec3};
use std::f32::consts::PI;

/// The grid: rows of declination from +90°, columns of right ascension from 0.
pub const ROWS: usize = 180;
pub const COLS: usize = 360;
/// Each star is listed in every cell its cone of this radius touches (degrees), so a lookup
/// needs one cell for any cone up to 0.9 of it (`max_chord`).
const MARGIN_DEG: f64 = 0.3;
/// u32 words before the star records, and per record (`gpu_words`).
pub const HEADER: usize = 8;
pub const RECORD: usize = 8;
/// The sun's V magnitude and B−V (the irradiance and colour references).
pub const SUN_V: f32 = -26.74;
pub const SUN_BV: f32 = 0.65;
/// Below this ratio of star light to sky light (a day sky) stars take no light samples.
const SHARE_FLOOR: f32 = 1e-6;
/// The share of sky samples stars take, when they take any.
const SHARE_RANGE: (f32, f32) = (0.05, 0.95);

/// The largest cone chord a lookup may use: 0.9 of the margin.
pub fn max_chord() -> f32 {
    (2. * (MARGIN_DEG * 0.9 * 0.5).to_radians().sin()) as f32
}
/// The smallest (a 10° field at 16k: f32 holds a chord of 1e-5 well).
pub const MIN_CHORD: f32 = 2e-6;
/// The cone's chord (about its angular radius) for an image `height` pixels tall with a
/// vertical field of view `fov` degrees: a disc `size` pixels across (1: half a pixel's radius).
pub fn chord(fov: f32, height: u32, size: f32) -> f32 {
    (size * (fov.to_radians() * 0.5).tan() / height.max(1) as f32).clamp(MIN_CHORD, max_chord())
}

/// A star's colour: a blackbody at the temperature B−V gives (Ballesteros 2012), relative to
/// the sun's B−V per channel, at luminance 1.
pub fn colour(bv: f32) -> Vec3 {
    let kelvin = |bv: f32| 4600. * (1. / (0.92 * bv + 1.7) + 1. / (0.92 * bv + 0.62));
    let bv = if bv.is_finite() {
        bv.clamp(-0.4, 2.)
    } else {
        SUN_BV
    };
    let c = Vec3::from(crate::spectrum::blackbody(kelvin(bv)))
        / Vec3::from(crate::spectrum::blackbody(kelvin(SUN_BV)));
    c / crate::environment::luminance(c.to_array()).max(1e-12)
}

/// The star table: directions, irradiance relative to the sun's, the power CDF and the grid.
pub struct Table {
    /// J2000 unit vectors (`astro::equatorial_vector`).
    pub dir: Vec<Vec3>,
    /// RGB irradiance over the sun's (luminance 10^(−0.4 (V − SUN_V))).
    pub rgb: Vec<Vec3>,
    /// V magnitudes, ascending (the table is sorted brightest first).
    pub mag: Vec<f32>,
    /// The power CDF (luminance; the last entry exactly 1) and each star's probability.
    pub cdf: Vec<f32>,
    pub prob: Vec<f32>,
    /// The sum of the luminances.
    pub total: f32,
    /// Per cell (`ROWS` × `COLS`), its first entry; one more at the end.
    pub offsets: Vec<u32>,
    /// Star indices, cell by cell.
    pub entries: Vec<u32>,
}

impl Table {
    pub fn new(stars: &[crate::astro::Star]) -> Self {
        // Brightest first: a limiting magnitude then keeps a prefix of the table (`kept`).
        let mut stars: Vec<_> = stars
            .iter()
            .filter(|s| s.v.is_finite() && s.ra.is_finite() && s.dec.is_finite())
            .collect();
        stars.sort_by(|a, b| a.v.total_cmp(&b.v));
        let mag: Vec<f32> = stars.iter().map(|s| s.v).collect();
        let dir: Vec<Vec3> = stars
            .iter()
            .map(|s| {
                let v = crate::astro::equatorial_vector(s.ra as f64, s.dec as f64);
                Vec3::new(v.x as f32, v.y as f32, v.z as f32)
            })
            .collect();
        let rgb: Vec<Vec3> = stars
            .iter()
            .map(|s| colour(s.bv) * 10f32.powf(-0.4 * (s.v - SUN_V)))
            .collect();
        let power: Vec<f64> = rgb
            .iter()
            .map(|c| crate::environment::luminance(c.to_array()) as f64)
            .collect();
        let total: f64 = power.iter().sum();
        let mut sum = 0f64;
        let mut cdf: Vec<f32> = power
            .iter()
            .map(|p| {
                sum += p;
                (sum / total) as f32
            })
            .collect();
        if let Some(last) = cdf.last_mut() {
            *last = 1.;
        }
        let prob = power.iter().map(|p| (p / total) as f32).collect();
        // The grid: each star in every cell its margin cone touches.
        let margin = MARGIN_DEG.to_radians();
        let mut pairs: Vec<(u32, u32)> = vec![];
        for (i, s) in stars.iter().enumerate() {
            let (ra, dec) = (s.ra as f64, s.dec as f64);
            let row = |d: f64| ((0.5 - d / std::f64::consts::PI) * ROWS as f64).floor();
            let (r0, r1) = (
                row(dec + margin).max(0.) as usize,
                (row(dec - margin) as usize).min(ROWS - 1),
            );
            let width = std::f64::consts::TAU / COLS as f64;
            let columns: Vec<usize> = if dec.abs() + margin >= std::f64::consts::FRAC_PI_2 {
                (0..COLS).collect()
            } else {
                let half = (margin.sin() / dec.cos()).min(1.).asin() * 1.01;
                let (c0, c1) = (
                    ((ra - half) / width).floor() as i64,
                    ((ra + half) / width).floor() as i64,
                );
                if c1 - c0 + 1 >= COLS as i64 {
                    (0..COLS).collect()
                } else {
                    (c0..=c1)
                        .map(|c| c.rem_euclid(COLS as i64) as usize)
                        .collect()
                }
            };
            for r in r0..=r1 {
                for &c in &columns {
                    pairs.push(((r * COLS + c) as u32, i as u32));
                }
            }
        }
        pairs.sort_unstable();
        let mut offsets = vec![0u32; ROWS * COLS + 1];
        for &(cell, _) in &pairs {
            offsets[cell as usize + 1] += 1;
        }
        for k in 0..ROWS * COLS {
            offsets[k + 1] += offsets[k];
        }
        Self {
            dir,
            rgb,
            mag,
            cdf,
            prob,
            total: total as f32,
            offsets,
            entries: pairs.into_iter().map(|(_, i)| i).collect(),
        }
    }
    pub fn len(&self) -> usize {
        self.dir.len()
    }
    /// The catalogue, loaded once (`None`, with the error logged, if it is missing).
    pub fn get() -> Option<&'static Self> {
        static TABLE: std::sync::OnceLock<Option<Table>> = std::sync::OnceLock::new();
        TABLE
            .get_or_init(|| {
                let path = crate::mesh::resolve("sky/stars.bin");
                crate::astro::load_stars(&path)
                    .map(|s| Table::new(&s))
                    .map_err(|e| eprintln!("Star catalogue: {e:#}"))
                    .ok()
                    .filter(|t| t.len() > 0)
            })
            .as_ref()
    }
    /// How many stars are at or brighter than the limiting magnitude `limit` (a prefix).
    pub fn kept(&self, limit: f32) -> usize {
        self.mag.partition_point(|m| *m <= limit)
    }
    /// The power the first `count` stars hold, as a share of the table's (their CDF mass).
    pub fn mass(&self, count: usize) -> f32 {
        if count == 0 {
            0.
        } else {
            self.cdf[count.min(self.len()) - 1]
        }
    }
    /// Among the first `count` stars, the one whose CDF interval holds `u` (the shader's
    /// `star_pick`: `u` scaled to their mass), and its probability among them.
    pub fn pick(&self, u: f32, count: usize) -> (usize, f32) {
        let mass = self.mass(count);
        let i = self.cdf[..count]
            .partition_point(|c| *c <= u * mass)
            .min(count - 1);
        (i, self.prob[i] / mass)
    }
    /// The grid cell of a celestial direction.
    pub fn cell(c: Vec3) -> usize {
        let dec = c.z.clamp(-1., 1.).asin();
        let ra = c.y.atan2(c.x);
        let row = (((0.5 - dec / PI) * ROWS as f32).max(0.) as usize).min(ROWS - 1);
        let col = (((ra / (2. * PI)).rem_euclid(1.) * COLS as f32) as usize).min(COLS - 1);
        row * COLS + col
    }
    /// The stars within `chord` of the celestial direction `c`: their summed relative
    /// irradiance.
    pub fn within(&self, c: Vec3, chord: f32, count: usize) -> Vec3 {
        let cell = Self::cell(c);
        let mut sum = Vec3::ZERO;
        for &i in &self.entries[self.offsets[cell] as usize..self.offsets[cell + 1] as usize] {
            let v = c - self.dir[i as usize];
            if (i as usize) < count && v.dot(v) < chord * chord {
                sum += self.rgb[i as usize];
            }
        }
        sum
    }
    /// The GPU's copy (t18): a header (count, rows, columns, the offsets' word, the entries'
    /// word, 0, 0, 0), per star its direction, CDF, RGB and probability (`RECORD` words), then
    /// the grid's offsets and entries.
    pub fn gpu_words(&self) -> Vec<u32> {
        let n = self.len();
        let offsets_at = HEADER + RECORD * n;
        let entries_at = offsets_at + self.offsets.len();
        let mut w = vec![
            n as u32,
            ROWS as u32,
            COLS as u32,
            offsets_at as u32,
            entries_at as u32,
            0,
            0,
            0,
        ];
        for i in 0..n {
            let (d, c) = (self.dir[i], self.rgb[i]);
            w.extend([d.x, d.y, d.z, self.cdf[i], c.x, c.y, c.z, self.prob[i]].map(f32::to_bits));
        }
        w.extend(&self.offsets);
        w.extend(&self.entries);
        w
    }
}

/// What a frame needs beyond the table (the shader's `star_x`, `star_y`, `star_z`, `star_tau`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// J2000 to world: the sky's local frame turned by `north`.
    pub world_from_celestial: Mat3,
    /// π · intensity · `stars`: relative irradiance to scene irradiance.
    pub scale: f32,
    /// The share of sky light samples stars take (0: none).
    pub share: f32,
    /// The camera cone's chord (`chord`).
    pub chord: f32,
    /// The zenith optical depth (RGB).
    pub tau: Vec3,
    /// The stars kept by the limiting magnitude (a prefix of the table) and their CDF mass.
    pub count: usize,
    pub mass: f32,
}

impl Frame {
    pub fn new(sky: &crate::sky::PhysicalSky, share: f32, chord: f32) -> Self {
        let t = sky.zenith_transmittance().max(Vec3::splat(1e-30));
        let count = Table::get().map_or(0, |t| t.kept(sky.limiting_magnitude));
        let mass = Table::get().map_or(0., |t| t.mass(count));
        Self {
            count,
            mass,
            world_from_celestial: Mat3::from_rotation_y(-sky.rotation())
                * sky.local_from_celestial(),
            scale: PI * sky.intensity * sky.stars,
            share,
            chord,
            tau: -Vec3::new(t.x.ln(), t.y.ln(), t.z.ln()),
        }
    }
    /// The air's transmittance toward a direction of world height `y` (0 below the horizon):
    /// exp(−τ X) with Kasten and Young's (1989) air mass X.
    pub fn air(&self, y: f32) -> Vec3 {
        if y <= 0. {
            return Vec3::ZERO;
        }
        let z = y.min(1.).acos().to_degrees();
        let mass = 1. / (y + 0.50572 * (96.07995 - z).powf(-1.6364));
        Vec3::new(
            (-self.tau.x * mass).exp(),
            (-self.tau.y * mass).exp(),
            (-self.tau.z * mass).exp(),
        )
    }
    pub fn world(&self, c: Vec3) -> Vec3 {
        self.world_from_celestial * c
    }
    pub fn celestial(&self, d: Vec3) -> Vec3 {
        self.world_from_celestial.transpose() * d
    }
    /// The radiance a camera ray (or a pure-delta chain) along world direction `d` sees: the
    /// stars inside its cone, spread over the cone's solid angle (π chord²).
    pub fn seen(&self, table: &Table, d: Vec3) -> Vec3 {
        let chord = self.chord;
        table.within(self.celestial(d), chord, self.count) * self.scale * self.air(d.y)
            / (PI * chord * chord)
    }
    /// Star `i` as a light sample: its world direction and irradiance (zero below the horizon).
    pub fn light(&self, table: &Table, i: usize) -> (Vec3, Vec3) {
        let d = self.world(table.dir[i]);
        (d, table.rgb[i] * self.scale * self.air(d.y))
    }
    /// The constants' rows: the rotation's rows with the scale, share and chord in w; the
    /// optical depth with w = 1 (stars on); the kept count and its mass.
    pub fn rows(&self) -> [[f32; 4]; 5] {
        let m = self.world_from_celestial;
        let r = |i: usize, w: f32| {
            let v = m.row(i);
            [v.x, v.y, v.z, w]
        };
        [
            r(0, self.scale),
            r(1, self.share),
            r(2, self.chord),
            [self.tau.x, self.tau.y, self.tau.z, 1.],
            [self.count as f32, self.mass, 0., 0.],
        ]
    }
}

/// The share of sky light samples the stars take: their irradiance above the horizon over it
/// plus the map's (`1 / inv_norm`, both luminance), 0 when they are under `SHARE_FLOOR` of it
/// (daylight), else clamped to `SHARE_RANGE`.
pub fn share(sky: &crate::sky::PhysicalSky, table: &Table, inv_norm: f32) -> f32 {
    let f = Frame::new(sky, 0., 0.);
    let stars: f64 = (0..f.count)
        .map(|i| crate::environment::luminance(f.light(table, i).1.to_array()) as f64)
        .sum();
    let map = if inv_norm > 0. {
        1. / inv_norm as f64
    } else {
        0.
    };
    let ratio = (stars / (stars + map).max(1e-300)) as f32;
    if stars <= 0. || ratio < SHARE_FLOOR {
        0.
    } else {
        ratio.clamp(SHARE_RANGE.0, SHARE_RANGE.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table() -> &'static Table {
        Table::get().expect("scenes/sky/stars.bin")
    }
    #[test]
    fn the_catalogue_loads_with_a_cdf_ending_at_one() {
        let t = table();
        assert!(t.len() > 80_000, "{}", t.len());
        assert_eq!(*t.cdf.last().unwrap(), 1.);
        assert!(t.cdf.windows(2).all(|w| w[0] <= w[1]));
        let sum: f64 = t.prob.iter().map(|p| *p as f64).sum();
        assert!((sum - 1.).abs() < 1e-4, "{sum}");
        // Sirius: the brightest, V −1.46, about 7.6e-11 of the sun.
        let (i, _) = t
            .rgb
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .unwrap();
        let l = crate::environment::luminance(t.rgb[i].to_array());
        assert!(
            (l / 10f32.powf(-0.4 * (-1.46 - SUN_V)) - 1.).abs() < 0.05,
            "{l}"
        );
        // Its probability is its luminance over the total.
        assert!((t.prob[i] / (l / t.total) - 1.).abs() < 1e-3);
    }
    #[test]
    fn picks_follow_the_cdf() {
        let t = table();
        for &i in &[0, 1, 77, t.len() / 2, t.len() - 1] {
            let lo = if i > 0 { t.cdf[i - 1] } else { 0. };
            if t.cdf[i] > lo {
                let (j, p) = t.pick((lo + t.cdf[i]) * 0.5, t.len());
                assert_eq!(j, i);
                assert_eq!(p, t.prob[i]);
            }
        }
        assert_eq!(t.pick(0., t.len()).0, t.cdf.partition_point(|c| *c <= 0.));
        assert_eq!(t.pick(0.999_999_9, t.len()).0, t.len() - 1);
    }
    #[test]
    fn a_limiting_magnitude_keeps_a_bright_prefix() {
        let t = table();
        assert!(t.mag.windows(2).all(|w| w[0] <= w[1]));
        let (naked, deep) = (t.kept(6.5), t.kept(9.));
        // About 9,000 stars to 6.5 over the whole sky; the camera preset's 9 several times more.
        assert!((7_000..13_000).contains(&naked), "{naked}");
        assert!(deep > 3 * naked && deep <= t.len());
        assert_eq!(t.kept(-30.), 0);
        assert_eq!(t.kept(30.), t.len());
        // Picks stay inside the prefix and their probabilities sum to one over it.
        let mass = t.mass(naked);
        assert!(mass > 0.5 && mass < 1.);
        for u in [0., 0.3, 0.77, 0.999_999] {
            let (i, p) = t.pick(u, naked);
            assert!(i < naked && p > 0.);
        }
        let sum: f64 = (0..naked).map(|i| (t.prob[i] / mass) as f64).sum();
        assert!((sum - 1.).abs() < 1e-3, "{sum}");
        // The cone sees only the kept stars.
        let c = t.dir[0];
        assert_eq!(t.within(c, 1e-4, 0), Vec3::ZERO);
        assert!(t.within(c, 1e-4, 1).y > 0.);
    }
    #[test]
    fn star_size_scales_the_cone() {
        assert!((chord(60., 1000, 4.) / chord(60., 1000, 1.) - 4.).abs() < 1e-4);
    }
    #[test]
    fn the_grid_finds_every_star_a_brute_force_cone_finds() {
        let t = table();
        let mut state = 0x2545_f491u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32
        };
        let mut found = 0;
        for k in 0..400 {
            // Half the probes right next to a star (cell edges, poles included), half anywhere.
            let c = if k % 2 == 0 {
                let s = t.dir[(next() * t.len() as f32) as usize % t.len()];
                let jitter = Vec3::new(next() - 0.5, next() - 0.5, next() - 0.5) * max_chord();
                (s + jitter).normalize()
            } else {
                let z = next() * 2. - 1.;
                let a = next() * 2. * PI;
                let r = (1. - z * z).sqrt();
                Vec3::new(r * a.cos(), r * a.sin(), z)
            };
            for chord in [max_chord(), max_chord() * 0.3, 1e-4] {
                let brute = t
                    .dir
                    .iter()
                    .zip(&t.rgb)
                    .filter(|(d, _)| (c - **d).length_squared() < chord * chord)
                    .fold(Vec3::ZERO, |s, (_, l)| s + *l);
                let grid = t.within(c, chord, t.len());
                assert!(
                    (brute - grid).length() <= 1e-6 * brute.length(),
                    "{c} {chord}"
                );
                found += (brute != Vec3::ZERO) as u32;
            }
        }
        assert!(found > 100, "{found}");
        // The poles: every column of the polar rows.
        for z in [1f32, -1.] {
            let c = Vec3::new(0., 0., z);
            assert_eq!(Table::cell(c) / COLS, if z > 0. { 0 } else { ROWS - 1 });
        }
    }
    #[test]
    fn the_cone_holds_a_star_s_irradiance() {
        // A star seen through its cone integrates to its irradiance: radiance × π chord².
        let t = table();
        let sky = crate::sky::PhysicalSky {
            on: true,
            time: 23.,
            ..Default::default()
        };
        let f = Frame::new(&sky, 0.5, 1e-4);
        let i = (0..f.count)
            .max_by(|a, b| f.world(t.dir[*a]).y.total_cmp(&f.world(t.dir[*b]).y))
            .unwrap();
        let (d, e) = f.light(t, i);
        let seen = f.seen(t, d) * PI * 1e-8;
        assert!(e.y > 0.);
        assert!(
            (seen.y / e.y - 1.).abs() < 1e-3 || seen.y > e.y,
            "{seen} {e}"
        );
        // Colours: the sun's B−V is white, a B star blue, an M star red.
        assert!((colour(SUN_BV) - Vec3::ONE).length() < 1e-4);
        assert!(colour(-0.2).z > colour(-0.2).x);
        assert!(colour(1.6).x > colour(1.6).z);
    }
    #[test]
    fn night_gives_stars_a_share_and_day_none() {
        let t = table();
        let night = crate::sky::PhysicalSky {
            on: true,
            time: 23.5,
            ..Default::default()
        };
        let day = crate::sky::PhysicalSky {
            time: 12.,
            ..night.clone()
        };
        // Sky luminance integrals of the order of the real ones (a night map about 1e-8 of
        // the sun's π, a day map about π/5).
        assert!(share(&night, t, 1. / 2e-8) > 0.05);
        assert_eq!(share(&day, t, 1. / 0.6), 0.);
        // Air: clear at the zenith, darker to the horizon, nothing below it.
        let f = Frame::new(&night, 0., 0.);
        assert!(f.air(1.).y > 0.7 && f.air(0.05).y < f.air(1.).y && f.air(-0.1) == Vec3::ZERO);
    }
}
