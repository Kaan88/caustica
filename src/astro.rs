//! Astronomy for the night sky: Julian date, sidereal time, precession from J2000 and the
//! rotation from the celestial frame to the sky's local frame (the one `sky.rs` uses: east +x,
//! up +y, north −z). Accuracy is a few arc minutes (no nutation, aberration or refraction),
//! far finer than a star's pixel. Angles are radians, times are hours unless a name says
//! otherwise, and the maths runs in f64.
//!
//! `stars.bin` (made by `tools/prep_sky_data.py` from HYG) is read by `parse_stars`.
// The night-sky passes (N3 on) are the callers.
#![allow(dead_code)]
use glam::{DMat3, DVec3};
use std::f64::consts::TAU;

/// Julian date of a Gregorian calendar date at `ut_hours` UT (Meeus, Astronomical Algorithms,
/// ch. 7; `day` may exceed the month's length, the date rolls on).
pub fn julian_date(year: i32, month: u32, day: u32, ut_hours: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (year as i64 - 1, month as i64 + 12)
    } else {
        (year as i64, month as i64)
    };
    let a = y.div_euclid(100);
    let b = 2 - a + a.div_euclid(4);
    let days = (365.25 * (y + 4716) as f64).floor() + (30.6001 * (m + 1) as f64).floor();
    days + day as f64 + b as f64 - 1524.5 + ut_hours / 24.
}

/// Julian date of day-of-year `day` (1 = 1 January; fractions of a day are dropped) of `year`
/// at `ut_hours` UT. Hours outside 0..24 and days past the year's end roll over.
pub fn julian_date_of_day(year: i32, day: f32, ut_hours: f64) -> f64 {
    julian_date(year, 1, 1, 0.) + (day.floor() as f64 - 1.) + ut_hours / 24.
}

/// Julian centuries since J2000.0.
fn centuries(jd: f64) -> f64 {
    (jd - 2451545.0) / 36525.
}

/// Greenwich mean sidereal time, radians in 0..2π (IAU 1982).
pub fn gmst(jd: f64) -> f64 {
    let t = centuries(jd);
    let deg = 280.46061837 + 360.98564736629 * (jd - 2451545.0) + 0.000387933 * t * t
        - t * t * t / 38710000.;
    deg.to_radians().rem_euclid(TAU)
}

/// Local mean sidereal time at `longitude` (radians, east positive).
pub fn local_sidereal_time(jd: f64, longitude: f64) -> f64 {
    (gmst(jd) + longitude).rem_euclid(TAU)
}

/// Unit vector of a position (right ascension, declination).
pub fn equatorial_vector(ra: f64, dec: f64) -> DVec3 {
    DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
}

/// (right ascension in 0..2π, declination) of an equatorial vector.
pub fn equatorial_angles(v: DVec3) -> (f64, f64) {
    (v.y.atan2(v.x).rem_euclid(TAU), v.z.clamp(-1., 1.).asin())
}

/// Precession from J2000.0 to the equinox of `jd` (IAU 1976, Meeus ch. 21), as a matrix on
/// equatorial vectors.
pub fn precession(jd: f64) -> DMat3 {
    let t = centuries(jd);
    let arcsec = (1. / 3600f64).to_radians();
    let zeta = (2306.2181 * t + 0.30188 * t * t + 0.017998 * t * t * t) * arcsec;
    let z = (2306.2181 * t + 1.09468 * t * t + 0.018203 * t * t * t) * arcsec;
    let theta = (2004.3109 * t - 0.42665 * t * t - 0.041833 * t * t * t) * arcsec;
    let (sz, cz) = zeta.sin_cos();
    let (sx, cx) = z.sin_cos();
    let (st, ct) = theta.sin_cos();
    // Rows of the matrix (glam is column-major, hence the transpose).
    DMat3::from_cols_array(&[
        cz * ct * cx - sz * sx,
        -sz * ct * cx - cz * sx,
        -st * cx,
        cz * ct * sx + sz * cx,
        -sz * ct * sx + cz * cx,
        -st * sx,
        cz * st,
        -sz * st,
        ct,
    ])
    .transpose()
}

/// Rotation from equatorial vectors of the date to the local frame (east +x, up +y,
/// north −z) at `latitude` and local sidereal time `lst`.
pub fn equatorial_to_local(latitude: f64, lst: f64) -> DMat3 {
    let (sp, cp) = latitude.sin_cos();
    let (sl, cl) = lst.sin_cos();
    // With H = lst − α: east = −cosδ sinH, north = cosφ sinδ − sinφ cosδ cosH,
    // up = sinφ sinδ + cosφ cosδ cosH, and cosδ sinH = sin(lst)·x − cos(lst)·y,
    // cosδ cosH = cos(lst)·x + sin(lst)·y.
    let east = DVec3::new(-sl, cl, 0.);
    let north = DVec3::new(-sp * cl, -sp * sl, cp);
    let up = DVec3::new(cp * cl, cp * sl, sp);
    // Local = (east, up, −north): rows.
    DMat3::from_cols(
        DVec3::new(east.x, up.x, -north.x),
        DVec3::new(east.y, up.y, -north.y),
        DVec3::new(east.z, up.z, -north.z),
    )
}

/// Rotation from the J2000 celestial frame to the local frame at the instant `jd` and the
/// place (`latitude`, `longitude`; radians, east positive).
pub fn celestial_to_local(jd: f64, latitude: f64, longitude: f64) -> DMat3 {
    equatorial_to_local(latitude, local_sidereal_time(jd, longitude)) * precession(jd)
}

/// (altitude, azimuth) of an (RA, Dec) of J2000 at `jd`; azimuth from north through east.
pub fn horizontal(jd: f64, latitude: f64, longitude: f64, ra: f64, dec: f64) -> (f64, f64) {
    let l = celestial_to_local(jd, latitude, longitude) * equatorial_vector(ra, dec);
    let north = -l.z;
    (l.y.clamp(-1., 1.).asin(), l.x.atan2(north).rem_euclid(TAU))
}

/// Mean obliquity of the ecliptic, radians (Meeus ch. 22, low order).
fn obliquity(jd: f64) -> f64 {
    (23.439291 - 0.0130042 * centuries(jd)).to_radians()
}

/// The sun's ecliptic longitude of the date (radians) and its mean longitude (degrees), from
/// Meeus' low-accuracy series (ch. 25, about 0.01°).
fn sun_longitude(jd: f64) -> (f64, f64) {
    let n = jd - 2451545.0;
    let l = (280.460 + 0.9856474 * n).rem_euclid(360.);
    let g = (357.528 + 0.9856003 * n).rem_euclid(360.).to_radians();
    let lambda = (l + 1.915 * g.sin() + 0.020 * (2. * g).sin()).to_radians();
    (lambda.rem_euclid(TAU), l)
}

/// The sun's apparent (RA, Dec) of the date (equinox of the date, not J2000), Meeus ch. 25
/// low-accuracy series, about 0.01°.
pub fn sun_equatorial(jd: f64) -> (f64, f64) {
    let (lambda, _) = sun_longitude(jd);
    let eps = obliquity(jd);
    let ra = (eps.cos() * lambda.sin())
        .atan2(lambda.cos())
        .rem_euclid(TAU);
    (ra, (eps.sin() * lambda.sin()).asin())
}

/// The sun's direction as a J2000 equatorial vector (what `celestial_to_local` takes).
pub fn sun_j2000(jd: f64) -> DVec3 {
    let (ra, dec) = sun_equatorial(jd);
    precession(jd).transpose() * equatorial_vector(ra, dec)
}

/// The equation of time, minutes (apparent minus mean solar time): the mean longitude less
/// the right ascension, both of the date.
pub fn equation_of_time(jd: f64) -> f64 {
    let (_, l) = sun_longitude(jd);
    let (ra, _) = sun_equatorial(jd);
    let mut d = l - ra.to_degrees();
    d = (d + 180.).rem_euclid(360.) - 180.;
    d * 4.
}

/// The moon's geocentric ecliptic (longitude, latitude) of the date, radians: Meeus ch. 47
/// truncated to its largest terms (about 0.03° in longitude, a few hundredths in latitude;
/// no nutation). Parallax (up to 1°) is not applied.
pub fn moon_ecliptic(jd: f64) -> (f64, f64) {
    let t = centuries(jd);
    let lp = 218.3164477 + 481267.88123421 * t;
    let d = (297.8501921 + 445267.1114034 * t).to_radians();
    let m = (357.5291092 + 35999.0502909 * t).to_radians();
    let mp = (134.9633964 + 477198.8675055 * t).to_radians();
    let f = (93.2720950 + 483202.0175233 * t).to_radians();
    let e = 1. - 0.002516 * t - 0.0000074 * t * t;
    // (D, M, M', F, coefficient in 1e-6 degree).
    const LON: [(i8, i8, i8, i8, f64); 35] = [
        (0, 0, 1, 0, 6288774.),
        (2, 0, -1, 0, 1274027.),
        (2, 0, 0, 0, 658314.),
        (0, 0, 2, 0, 213618.),
        (0, 1, 0, 0, -185116.),
        (0, 0, 0, 2, -114332.),
        (2, 0, -2, 0, 58793.),
        (2, -1, -1, 0, 57066.),
        (2, 0, 1, 0, 53322.),
        (2, -1, 0, 0, 45758.),
        (0, 1, -1, 0, -40923.),
        (1, 0, 0, 0, -34720.),
        (0, 1, 1, 0, -30383.),
        (2, 0, 0, -2, 15327.),
        (0, 0, 1, 2, -12528.),
        (0, 0, 1, -2, 10980.),
        (4, 0, -1, 0, 10675.),
        (0, 0, 3, 0, 10034.),
        (4, 0, -2, 0, 8548.),
        (2, 1, -1, 0, -7888.),
        (2, 1, 0, 0, -6766.),
        (1, 0, -1, 0, -5163.),
        (1, 1, 0, 0, 4987.),
        (2, -1, 1, 0, 4036.),
        (2, 0, 2, 0, 3994.),
        (4, 0, 0, 0, 3861.),
        (2, 0, -3, 0, 3665.),
        (0, 1, -2, 0, -2689.),
        (2, 0, -1, 2, -2602.),
        (2, -1, -2, 0, 2390.),
        (1, 0, 1, 0, -2348.),
        (2, -2, 0, 0, 2236.),
        (0, 1, 2, 0, -2120.),
        (0, 2, 0, 0, -2069.),
        (2, -2, -1, 0, 2048.),
    ];
    const LAT: [(i8, i8, i8, i8, f64); 14] = [
        (0, 0, 0, 1, 5128122.),
        (0, 0, 1, 1, 280602.),
        (0, 0, 1, -1, 277693.),
        (2, 0, 0, -1, 173237.),
        (2, 0, -1, 1, 55413.),
        (2, 0, -1, -1, 46271.),
        (2, 0, 0, 1, 32573.),
        (0, 0, 2, 1, 17198.),
        (2, 0, 1, -1, 9266.),
        (0, 0, 2, -1, 8822.),
        (2, -1, 0, -1, 8216.),
        (2, 0, -2, -1, 4324.),
        (2, 0, 1, 1, 4200.),
        (2, 1, 0, -1, -3359.),
    ];
    let arg = |k: &(i8, i8, i8, i8, f64)| {
        k.0 as f64 * d + k.1 as f64 * m + k.2 as f64 * mp + k.3 as f64 * f
    };
    let weight = |k: &(i8, i8, i8, i8, f64)| e.powi(k.1.abs() as i32);
    let a1 = (119.75 + 131.849 * t).to_radians();
    let a2 = (53.09 + 479264.290 * t).to_radians();
    let a3 = (313.45 + 481266.484 * t).to_radians();
    let lp_r = lp.to_radians();
    let sl: f64 = LON
        .iter()
        .map(|k| k.4 * weight(k) * arg(k).sin())
        .sum::<f64>()
        + 3958. * a1.sin()
        + 1962. * (lp_r - f).sin()
        + 318. * a2.sin();
    let sb: f64 = LAT
        .iter()
        .map(|k| k.4 * weight(k) * arg(k).sin())
        .sum::<f64>()
        - 2235. * lp_r.sin()
        + 382. * a3.sin()
        + 175. * (a1 - f).sin()
        + 175. * (a1 + f).sin()
        + 127. * (lp_r - mp).sin()
        - 115. * (lp_r + mp).sin();
    (
        (lp + sl / 1e6).to_radians().rem_euclid(TAU),
        (sb / 1e6).to_radians(),
    )
}

/// The moon's (RA, Dec) of the date.
pub fn moon_equatorial(jd: f64) -> (f64, f64) {
    let (lon, lat) = moon_ecliptic(jd);
    let eps = obliquity(jd);
    let ra = (lon.sin() * eps.cos() - lat.tan() * eps.sin())
        .atan2(lon.cos())
        .rem_euclid(TAU);
    let dec = (lat.sin() * eps.cos() + lat.cos() * eps.sin() * lon.sin()).asin();
    (ra, dec)
}

/// The moon's direction as a J2000 equatorial vector.
pub fn moon_j2000(jd: f64) -> DVec3 {
    let (ra, dec) = moon_equatorial(jd);
    precession(jd).transpose() * equatorial_vector(ra, dec)
}

/// The moon's age as a fraction of the synodic month (0 new, 0.5 full): the ecliptic
/// elongation of the moon from the sun over 360°.
pub fn moon_age(jd: f64) -> f64 {
    let (sun, _) = sun_longitude(jd);
    let (moon, _) = moon_ecliptic(jd);
    ((moon - sun).rem_euclid(TAU)) / TAU
}

/// A catalogue star: J2000 position (radians), visual magnitude and B−V colour index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub ra: f32,
    pub dec: f32,
    pub v: f32,
    pub bv: f32,
}

const STAR_MAGIC: &[u8; 4] = b"CSTR";
const STAR_BYTES: usize = 12;

/// An IEEE half as f32.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1. } else { 1. };
    let exp = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as f32;
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0. => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1. + frac / 1024.) * 2f32.powi(exp - 15),
    }
}

/// Reads `stars.bin`: "CSTR", version u32 = 1, count u32, then per star RA f32, Dec f32
/// (radians, J2000), V f16, B−V f16, little-endian.
pub fn parse_stars(data: &[u8]) -> anyhow::Result<Vec<Star>> {
    anyhow::ensure!(
        data.len() >= 12 && &data[..4] == STAR_MAGIC,
        "not a star catalogue (missing CSTR header)"
    );
    let word = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    anyhow::ensure!(
        word(4) == 1,
        "star catalogue version {} (expected 1)",
        word(4)
    );
    let count = word(8) as usize;
    let body = &data[12..];
    anyhow::ensure!(
        body.len() == count * STAR_BYTES,
        "star catalogue holds {} bytes for {count} stars (expected {})",
        body.len(),
        count * STAR_BYTES
    );
    Ok(body
        .as_chunks::<STAR_BYTES>()
        .0
        .iter()
        .map(|c| {
            let f = |i: usize| f32::from_le_bytes(c[i..i + 4].try_into().unwrap());
            let h = |i: usize| half_to_f32(u16::from_le_bytes([c[i], c[i + 1]]));
            Star {
                ra: f(0),
                dec: f(4),
                v: h(8),
                bv: h(10),
            }
        })
        .collect())
}

/// Loads a star catalogue file (`scenes/sky/stars.bin`).
pub fn load_stars(path: &std::path::Path) -> anyhow::Result<Vec<Star>> {
    use anyhow::Context;
    let data = std::fs::read(path)
        .with_context(|| format!("reading the star catalogue {}", path.display()))?;
    parse_stars(&data).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const DEG: f64 = PI / 180.;
    // J2000 positions (HYG / SIMBAD).
    const POLARIS: (f64, f64) = (37.954561 * DEG, 89.264109 * DEG);
    const SIRIUS: (f64, f64) = (101.287155 * DEG, -16.716116 * DEG);

    /// Highest altitude of a J2000 star over the day of `jd0` at the place, scanning the
    /// time; returns (altitude, UT hour).
    fn transit(jd0: f64, lat: f64, lon: f64, star: (f64, f64)) -> (f64, f64) {
        (0..24 * 60)
            .map(|m| {
                let ut = m as f64 / 60.;
                (horizontal(jd0 + ut / 24., lat, lon, star.0, star.1).0, ut)
            })
            .fold((f64::MIN, 0.), |a, b| if b.0 > a.0 { b } else { a })
    }

    #[test]
    fn julian_dates_match_the_references() {
        // Meeus: 1957 Oct 4.81 = JD 2436116.31; J2000.0 = 2000 Jan 1 12h.
        assert!((julian_date(1957, 10, 4, 19.44) - 2436116.31).abs() < 1e-3);
        assert_eq!(julian_date(2000, 1, 1, 12.), 2451545.0);
        assert_eq!(julian_date(1999, 1, 1, 0.), 2451179.5);
        assert_eq!(julian_date_of_day(2000, 1., 12.), 2451545.0);
        // Day 60 is 29 February in a leap year, 1 March otherwise.
        assert_eq!(
            julian_date_of_day(2024, 60., 0.),
            julian_date(2024, 2, 29, 0.)
        );
        assert_eq!(
            julian_date_of_day(2026, 60., 0.),
            julian_date(2026, 3, 1, 0.)
        );
    }

    #[test]
    fn sidereal_time_matches_the_references() {
        // Meeus example 12.a: 1987 April 10 0h UT, GMST = 13h10m46.3668s.
        let jd = julian_date(1987, 4, 10, 0.);
        let hours = gmst(jd) / TAU * 24.;
        assert!(
            (hours - (13. + 10. / 60. + 46.3668 / 3600.)).abs() < 1e-4,
            "{hours}"
        );
        // At J2000.0 itself: 18h41m50.55s.
        let hours = gmst(2451545.0) / TAU * 24.;
        assert!(
            (hours - (18. + 41. / 60. + 50.55 / 3600.)).abs() < 1e-4,
            "{hours}"
        );
    }

    #[test]
    fn precession_matches_the_yearly_rates() {
        // The J2000 equinox point drifts by m = 46.1" in RA and n = 20.04" in Dec per year.
        let jd = julian_date(2025, 1, 1, 12.);
        let v = precession(jd) * equatorial_vector(0., 0.);
        let (ra, dec) = equatorial_angles(v);
        assert!((ra / DEG - 25. * 46.1 / 3600.).abs() < 0.01, "{}", ra / DEG);
        assert!(
            (dec / DEG - 25. * 20.04 / 3600.).abs() < 0.005,
            "{}",
            dec / DEG
        );
        // Polaris' declination of date is about 89.37° in 2026.
        let (_, dec) = equatorial_angles(
            precession(julian_date(2026, 6, 1, 0.)) * equatorial_vector(POLARIS.0, POLARIS.1),
        );
        assert!((dec / DEG - 89.37).abs() < 0.02, "{}", dec / DEG);
        // Identity at J2000.
        let id = precession(2451545.0);
        assert!((id * DVec3::new(0.3, 0.5, 0.8) - DVec3::new(0.3, 0.5, 0.8)).length() < 1e-12);
    }

    #[test]
    fn polaris_stands_at_the_latitude() {
        let jd0 = julian_date(2026, 6, 21, 0.);
        for lat in [45., 20., 60., -5.] {
            let lat = lat * DEG;
            // Over a day Polaris swings about the pole: its altitude is within 1° of the
            // latitude at every hour.
            for m in 0..24 {
                let (alt, az) =
                    horizontal(jd0 + m as f64 / 24., lat, 10. * DEG, POLARIS.0, POLARIS.1);
                assert!(
                    (alt - lat).abs() < 1. * DEG,
                    "lat {} hour {m}: {}",
                    lat / DEG,
                    alt / DEG
                );
                // ... and it is due north within a few degrees (45° latitude: ~1.4° at most).
                if lat > 20. * DEG {
                    let off = (az + PI).rem_euclid(TAU) - PI;
                    assert!(off.abs() < 3. * DEG, "az {}", az / DEG);
                }
            }
        }
    }

    #[test]
    fn sirius_transits_at_the_complement_of_its_distance() {
        let jd0 = julian_date(2026, 1, 15, 0.);
        for lat in [45., 0., -33., 60.] {
            let lat = lat * DEG;
            let (alt, _) = transit(jd0, lat, 0., SIRIUS);
            let expect = PI / 2. - (lat - SIRIUS.1).abs();
            // Precession since J2000 moves the declination by about 0.03°.
            assert!(
                (alt - expect).abs() < 0.1 * DEG,
                "lat {}: {} vs {}",
                lat / DEG,
                alt / DEG,
                expect / DEG
            );
        }
        // At the transit the star is due south (azimuth 180°) for a northern observer.
        let jd0 = julian_date(2026, 1, 15, 0.);
        let lat = 45. * DEG;
        let (_, ut) = transit(jd0, lat, 0., SIRIUS);
        let (_, az) = horizontal(jd0 + ut / 24., lat, 0., SIRIUS.0, SIRIUS.1);
        assert!((az - PI).abs() < 1. * DEG, "{}", az / DEG);
        // Sirius transits at local sidereal time = its RA (nutation aside): at Greenwich on
        // 15 January it is near 02h UT... check the sidereal clock directly.
        let lst = local_sidereal_time(jd0 + ut / 24., 0.);
        assert!(((lst - SIRIUS.0 + PI).rem_euclid(TAU) - PI).abs() < 0.5 * DEG);
    }

    #[test]
    fn east_is_x_and_north_is_minus_z() {
        // A star on the meridian south of the zenith (hour angle 0, dec < lat) is at +z,
        // one in the east (hour angle −6h) at +x.
        let lat = 45. * DEG;
        let m = equatorial_to_local(lat, 0.);
        let south = m * equatorial_vector(0., 10. * DEG);
        assert!(south.z > 0. && south.x.abs() < 1e-12 && south.y > 0.);
        let east = m * equatorial_vector(PI / 2., 0.);
        assert!(east.x > 0.99, "{east}");
        // Pole at altitude = latitude, due north.
        let pole = m * DVec3::Z;
        assert!((pole.y - lat.sin()).abs() < 1e-12 && pole.z < 0.);
    }

    #[test]
    fn celestial_to_local_round_trips() {
        let jd = julian_date(2031, 9, 3, 21.7);
        let (lat, lon) = (-37.8 * DEG, 144.9 * DEG);
        let m = celestial_to_local(jd, lat, lon);
        // Orthonormal, a proper rotation.
        assert!((m.determinant() - 1.).abs() < 1e-12);
        for (ra, dec) in [(0.3, 0.7), (4.1, -0.9), (5.9, 0.0), (2.2, 1.5)] {
            let v = equatorial_vector(ra, dec);
            let back = m.transpose() * (m * v);
            assert!((back - v).length() < 1e-12);
            let (r2, d2) = equatorial_angles(back);
            assert!((r2 - ra).abs() < 1e-9 && (d2 - dec).abs() < 1e-9);
        }
    }

    #[test]
    fn stars_bin_parses_and_fails_clearly() {
        let mut data = Vec::new();
        data.extend_from_slice(b"CSTR");
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        for (ra, dec, v, bv) in [
            (1.5f32, -0.25f32, 0x3c00u16, 0xb800u16),
            (6.0, 1.0, 0xc000, 0x3555),
        ] {
            data.extend_from_slice(&ra.to_le_bytes());
            data.extend_from_slice(&dec.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&bv.to_le_bytes());
        }
        let stars = parse_stars(&data).unwrap();
        assert_eq!(stars.len(), 2);
        assert_eq!(
            stars[0],
            Star {
                ra: 1.5,
                dec: -0.25,
                v: 1.,
                bv: -0.5
            }
        );
        assert_eq!(stars[1].v, -2.);
        assert!((stars[1].bv - 0.3333).abs() < 1e-3);
        // Bad magic, version, truncation, and a missing file.
        let mut bad = data.clone();
        bad[0] = b'X';
        assert!(parse_stars(&bad).unwrap_err().to_string().contains("CSTR"));
        let mut bad = data.clone();
        bad[4] = 2;
        assert!(
            parse_stars(&bad)
                .unwrap_err()
                .to_string()
                .contains("version 2")
        );
        let err = parse_stars(&data[..data.len() - 1])
            .unwrap_err()
            .to_string();
        assert!(err.contains("expected 24"), "{err}");
        assert!(parse_stars(&[]).is_err());
        let err = load_stars(std::path::Path::new("scenes/sky/no-such-stars.bin"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no-such-stars.bin"), "{err}");
    }

    fn angle_between(a: (f64, f64), b: (f64, f64)) -> f64 {
        equatorial_vector(a.0, a.1)
            .dot(equatorial_vector(b.0, b.1))
            .clamp(-1., 1.)
            .acos()
            / DEG
    }

    #[test]
    fn the_sun_matches_meeus_example_25a() {
        // 1992 October 13, 0h TD: apparent α = 198.38083°, δ = −7.78507°.
        let jd = julian_date(1992, 10, 13, 0.);
        assert!((jd - 2448908.5).abs() < 1e-9);
        let got = sun_equatorial(jd);
        let off = angle_between(got, (198.38083 * DEG, -7.78507 * DEG));
        println!("SUN_MEEUS_25A off {off:.4} deg");
        assert!(off < 0.05, "{off}");
    }

    #[test]
    fn the_equinox_sun_is_on_the_celestial_equator() {
        // 2026 March 20 at 14:46 UT, the equinox: δ = 0, α = 0.
        let jd = julian_date(2026, 3, 20, 14. + 46. / 60.);
        let (ra, dec) = sun_equatorial(jd);
        println!("EQUINOX dec {:.4} ra {:.4}", dec / DEG, ra / DEG);
        assert!(dec.abs() < 0.05 * DEG, "{}", dec / DEG);
        assert!(ra.min(TAU - ra) < 0.06 * DEG, "{}", ra / DEG);
        // December solstice 2026-12-21 at 20:50 UT: δ = −ε.
        let (_, dec) = sun_equatorial(julian_date(2026, 12, 21, 20.83));
        assert!((dec / DEG + 23.437).abs() < 0.03, "{}", dec / DEG);
    }

    #[test]
    fn the_equation_of_time_matches_the_almanac() {
        // About +16.4 min on 3 November, −14.2 min on 11 February (almanac values).
        let nov = equation_of_time(julian_date(2026, 11, 3, 12.));
        let feb = equation_of_time(julian_date(2026, 2, 11, 12.));
        println!("EOT nov {nov:.2} feb {feb:.2} min");
        assert!((nov - 16.4).abs() < 0.3, "{nov}");
        assert!((feb + 14.2).abs() < 0.3, "{feb}");
    }

    #[test]
    fn the_moon_matches_meeus_example_47a() {
        // 1992 April 12, 0h TD: λ = 133.162655°, β = −3.229126°, α = 134.688470°,
        // δ = 13.768368° (geometric, before nutation).
        let jd = julian_date(1992, 4, 12, 0.);
        let (lon, lat) = moon_ecliptic(jd);
        println!(
            "MOON_MEEUS_47A lon {:.4} lat {:.4}",
            lon / DEG - 133.162655,
            lat / DEG + 3.229126
        );
        assert!((lon / DEG - 133.162655).abs() < 0.05, "{}", lon / DEG);
        assert!((lat / DEG + 3.229126).abs() < 0.05, "{}", lat / DEG);
        let off = angle_between(moon_equatorial(jd), (134.688470 * DEG, 13.768368 * DEG));
        println!("MOON_MEEUS_47A equatorial off {off:.4} deg");
        assert!(off < 0.1, "{off}");
    }

    #[test]
    fn moon_phases_follow_the_almanac() {
        // Full moon 2026-01-03 10:03 UT; new moon 2026-02-17 12:01 UT; first quarter
        // 2026-02-24 12:28 UT.
        let full = julian_date(2026, 1, 3, 10. + 3. / 60.);
        let new = julian_date(2026, 2, 17, 12. + 1. / 60.);
        let quarter = julian_date(2026, 2, 24, 12. + 28. / 60.);
        let elong = |jd: f64| moon_age(jd) * 360.;
        println!(
            "MOON_PHASES full elong {:.2} new {:.2} quarter {:.2}",
            elong(full),
            elong(new),
            elong(quarter)
        );
        assert!((elong(full) - 180.).abs() < 1.5, "{}", elong(full));
        assert!(
            (((elong(new) + 180.) % 360.) - 180.).abs() < 1.5,
            "{}",
            elong(new)
        );
        assert!((elong(quarter) - 90.).abs() < 1.5, "{}", elong(quarter));
        // The age runs 0..1 over a synodic month (29.53 days).
        let a = moon_age(full);
        let b = moon_age(full + 29.530589 / 2.);
        // (The moon's elliptical orbit makes the elongation uneven by up to about 8 degrees.)
        assert!(((b - a).rem_euclid(1.) - 0.5).abs() < 0.04);
    }
}
