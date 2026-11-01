//! Track 16: real lens systems (Kolb, Mitchell and Hanrahan 1995; pbrt's `RealisticCamera`).
//!
//! A lens is a list of spherical surfaces from the object side to the film, in millimetres. Each
//! row gives the curvature radius (0 = the aperture stop), the axial distance to the next surface
//! (the last row's distance is the film distance, set by focusing), the refractive index of the
//! medium after the surface (toward the film; 1 or 0 = air) and the clear aperture diameter.
//!
//! The CPU focuses the lens (paraxial trace of an axial object point), measures its focal length
//! and entrance pupil, and bounds the exit pupil per radial film band so the shader samples only
//! the part of the rear element that can pass light. `upload` packs it all for the shader
//! (`lens_data`, t9; layout in `pack`).

use std::sync::LazyLock;

/// One surface: curvature radius, thickness to the next surface, IOR after it, diameter (mm).
type Row = [f32; 4];

pub struct Lens {
    pub name: &'static str,
    /// Equidistant (f·θ) mapping when sizing the film (fisheye), else rectilinear (f·tan θ).
    fisheye: bool,
    rows: &'static [Row],
}

/// Pinhole / thin lens is 0 in `Settings::lens_system`; these are 1..=LENSES.len().
pub const LENSES: [Lens; 4] = [
    // US patent 2,673,491 (Tronnier), Modern Lens Design p. 312, scaled to 50 mm.
    Lens {
        name: "Double Gauss 50 mm",
        fisheye: false,
        rows: &[
            [29.475, 3.76, 1.67, 25.2],
            [84.83, 0.12, 1., 25.2],
            [19.275, 4.025, 1.67, 23.],
            [40.77, 3.275, 1.699, 23.],
            [12.75, 5.705, 1., 18.],
            [0., 4.5, 1., 17.1],
            [-14.495, 1.18, 1.603, 17.],
            [40.77, 6.065, 1.658, 20.],
            [-20.385, 0.19, 1., 20.],
            [437.065, 2.95, 1.717, 20.],
            [-39.73, 0., 1., 20.],
        ],
    },
    // Nakamura wide angle, Modern Lens Design p. 360, scaled to 22 mm.
    Lens {
        name: "Wide angle 22 mm",
        fisheye: false,
        rows: &[
            [35.98738, 1.21638, 1.54, 23.716],
            [11.69718, 9.9957, 1., 17.996],
            [13.08714, 5.12622, 1.772, 12.364],
            [-22.63294, 1.76924, 1.617, 9.812],
            [71.05802, 0.8184, 1., 9.152],
            [0., 2.27766, 1., 8.756],
            [-9.58584, 2.43254, 1.617, 8.184],
            [-11.28864, 0.11506, 1., 9.152],
            [-166.7765, 3.09606, 1.713, 10.648],
            [-7.5911, 1.32682, 1.805, 11.44],
            [-16.7662, 3.98068, 1., 12.276],
            [-7.70286, 1.21638, 1.617, 13.42],
            [-11.97328, 0., 1., 17.996],
        ],
    },
    // Muller 16 mm f/4 fisheye (155.9° field), Modern Lens Design p. 164, scaled to 10 mm.
    Lens {
        name: "Fisheye 10 mm",
        fisheye: true,
        rows: &[
            [30.2249, 0.8335, 1.62, 30.34],
            [11.3931, 7.4136, 1., 20.68],
            [75.2019, 1.0654, 1.639, 17.8],
            [8.3349, 11.1549, 1., 13.42],
            [9.5882, 2.0054, 1.654, 9.02],
            [43.8677, 5.3895, 1., 8.14],
            [0., 1.4163, 1., 6.08],
            [29.4541, 2.1934, 1.517, 5.96],
            [-5.2265, 0.9714, 1.805, 5.84],
            [-14.2884, 0.0627, 1., 5.96],
            [-22.3726, 0.94, 1.673, 5.96],
            [-15.0404, 0., 1., 6.52],
        ],
    },
    // A single uncorrected biconvex BK7 element behind a stop: strong spherical and chromatic
    // aberration, the glow of a toy or Petzval-era lens. 1/f = 2(n-1)/R gives f ≈ 50 mm.
    Lens {
        name: "Singlet 50 mm",
        fisheye: false,
        rows: &[
            [0., 3., 1., 18.],
            [51.7, 5., 1.5168, 24.],
            [-51.7, 0., 1., 24.],
        ],
    },
];

/// Radial film bands with their own exit-pupil bounds.
pub const BINS: usize = 32;
/// Surfaces the shader's buffer holds (the longest lens has 13).
pub const MAX_SURFACES: usize = 16;
/// float4s in the shader's lens buffer: header (2), two per surface, one per band.
pub const FLOAT4S: usize = 2 + 2 * MAX_SURFACES + BINS;

/// Effective focal length and wide-open f-number (from the entrance pupil), per lens.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub focal: f32,
    pub f_number: f32,
}

pub static INFO: LazyLock<Vec<Info>> = LazyLock::new(|| {
    LENSES
        .iter()
        .map(|l| {
            let s = Surfaces::new(l, 1., 1.);
            let h = s.entrance_pupil();
            Info {
                focal: s.focal_length(h * 0.1),
                f_number: s.focal_length(h * 0.1) / (2. * h),
            }
        })
        .collect()
});

/// Abbe number guessed from the index (the prescriptions carry only n_d): crowns near 60,
/// dense flints near 28.
fn abbe(n: f32) -> f32 {
    (60. - (n - 1.52) / (1.80 - 1.52) * 32.).clamp(25., 65.)
}

/// The index at red, green and blue (610, 550, 465 nm) from a Cauchy fit through n_d and the
/// Abbe number. Green stays put (focus does not depend on the slider); `dispersion` scales red's
/// and blue's offsets from it (0 = none, 1 = real glass).
fn indices(n: f32, dispersion: f32) -> [f32; 3] {
    if n <= 1. {
        return [1.; 3];
    }
    // n_F - n_C = (n_d - 1) / V = B (1/0.4861² - 1/0.6563²).
    let b = (n - 1.) / abbe(n) / 1.9107;
    let at = |um: f32| n + b * (1. / (um * um) - 1. / (0.5876 * 0.5876));
    let g = at(0.55);
    [
        g + dispersion * (at(0.61) - g),
        g,
        g + dispersion * (at(0.465) - g),
    ]
}

#[derive(Clone, Copy)]
struct Surface {
    radius: f32,
    /// Vertex position on the axis; the rear vertex is at 0, the object side is -z.
    z: f32,
    aperture: f32,
    /// IOR of the medium after the surface (toward the film), at red, green, blue.
    eta: [f32; 3],
}

struct Surfaces {
    s: Vec<Surface>,
}

type V3 = [f32; 3];
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}
/// HLSL `refract`: `n` faces against `i`; None on total internal reflection.
fn refract(i: V3, n: V3, eta: f32) -> Option<V3> {
    let c = dot(n, i);
    let k = 1. - eta * eta * (1. - c * c);
    if k < 0. {
        return None;
    }
    let s = eta * c + k.sqrt();
    Some([
        eta * i[0] - s * n[0],
        eta * i[1] - s * n[1],
        eta * i[2] - s * n[2],
    ])
}

impl Surfaces {
    /// `stop_scale` scales the aperture stop (stopping down); `dispersion` as `indices`.
    fn new(lens: &Lens, stop_scale: f32, dispersion: f32) -> Self {
        let n = lens.rows.len();
        let mut z = 0.;
        let mut s = vec![
            Surface {
                radius: 0.,
                z: 0.,
                aperture: 0.,
                eta: [1.; 3]
            };
            n
        ];
        for i in (0..n).rev() {
            let r = lens.rows[i];
            if i + 1 < n {
                z -= r[1];
            }
            let eta = indices(r[2], dispersion);
            s[i] = Surface {
                radius: r[0],
                z,
                aperture: r[3] * 0.5 * if r[0] == 0. { stop_scale } else { 1. },
                eta,
            };
        }
        Self { s }
    }
    fn eta_after(&self, i: usize, c: usize) -> f32 {
        let e = self.s[i].eta[c];
        if e <= 0. { 1. } else { e }
    }
    /// Intersect surface i: the new point and the surface normal, or None on a miss or outside
    /// the clear aperture.
    fn hit(&self, i: usize, o: V3, d: V3) -> Option<(V3, V3)> {
        let s = self.s[i];
        let (t, n) = if s.radius == 0. {
            ((s.z - o[2]) / d[2], [0., 0., 1.])
        } else {
            let c = [0., 0., s.z + s.radius];
            let oc = [o[0] - c[0], o[1] - c[1], o[2] - c[2]];
            let b = dot(oc, d);
            let disc = b * b - (dot(oc, oc) - s.radius * s.radius);
            if disc < 0. {
                return None;
            }
            let q = disc.sqrt();
            let closer = (d[2] > 0.) ^ (s.radius < 0.);
            let t = if closer { -b - q } else { -b + q };
            let p = [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
            (t, norm([p[0] - c[0], p[1] - c[1], p[2] - c[2]]))
        };
        // NaN (a ray parallel to the stop plane) fails too.
        if t.is_nan() || t <= 0. {
            return None;
        }
        let p = [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
        if p[0] * p[0] + p[1] * p[1] > s.aperture * s.aperture {
            return None;
        }
        Some((p, n))
    }
    fn bend(&self, d: V3, n: V3, eta: f32) -> Option<V3> {
        let n = if dot(n, d) > 0. {
            [-n[0], -n[1], -n[2]]
        } else {
            n
        };
        refract(d, n, eta)
    }
    /// From the film side (d.z < 0) to the object side, in channel c (0 R, 1 G, 2 B).
    fn trace_from_film(&self, mut o: V3, mut d: V3, c: usize) -> Option<(V3, V3)> {
        for i in (0..self.s.len()).rev() {
            let (p, n) = self.hit(i, o, d)?;
            o = p;
            if self.s[i].radius != 0. {
                let before = if i > 0 { self.eta_after(i - 1, c) } else { 1. };
                d = self.bend(d, n, self.eta_after(i, c) / before)?;
            }
        }
        Some((o, d))
    }
    /// From the object side (d.z > 0) to the film side, in green.
    fn trace_from_scene(&self, mut o: V3, mut d: V3) -> Option<(V3, V3)> {
        for i in 0..self.s.len() {
            let (p, n) = self.hit(i, o, d)?;
            o = p;
            if self.s[i].radius != 0. {
                let before = if i > 0 { self.eta_after(i - 1, 1) } else { 1. };
                d = self.bend(d, n, before / self.eta_after(i, 1))?;
            }
        }
        Some((o, d))
    }
    fn front(&self) -> f32 {
        self.s[0].z
    }
    /// Radius of the widest axial bundle from infinity that passes every aperture.
    fn entrance_pupil(&self) -> f32 {
        let (mut lo, mut hi) = (0., self.s[0].aperture);
        for _ in 0..40 {
            let h = 0.5 * (lo + hi);
            if self
                .trace_from_scene([h, 0., self.front() - 1.], [0., 0., 1.])
                .is_some()
            {
                lo = h;
            } else {
                hi = h;
            }
        }
        lo
    }
    /// Effective focal length from a paraxial ray at height h from infinity.
    fn focal_length(&self, h: f32) -> f32 {
        match self.trace_from_scene([h, 0., self.front() - 1.], [0., 0., 1.]) {
            Some((_, d)) => (h * d[2] / d[0]).abs(),
            None => 50.,
        }
    }
    /// Film distance behind the rear vertex that focuses an axial point `distance` mm in front
    /// of the front vertex (paraxial; infinity past 1e7 mm). Clamped to 4 focal lengths.
    fn film_distance(&self, distance: f32, h: f32, focal: f32) -> f32 {
        let (o, d) = if distance > 1e7 {
            ([h, 0., self.front() - 1.], [0., 0., 1.])
        } else {
            let z = self.front() - distance;
            // Aim at height h on the front vertex plane.
            ([0., 0., z], norm([h, 0., distance]))
        };
        match self.trace_from_scene(o, d) {
            // The image-side ray crosses the axis there (behind the rear vertex at 0).
            Some((p, d)) if d[0] * p[0] < 0. => (p[2] - p[0] / d[0] * d[2]).clamp(0., 4. * focal),
            _ => 4. * focal,
        }
    }
}

/// What the shader needs, and the key it was built for.
#[derive(Clone, PartialEq, Debug)]
pub struct Params {
    /// 1-based lens index.
    pub lens: u32,
    pub f_number: f32,
    /// Focus distance in mm from the front vertex.
    pub focus_mm: f32,
    /// Vertical field of view (radians) and width / height.
    pub fov: f32,
    pub aspect: f32,
    pub dispersion: f32,
}

/// The shader's lens buffer (`FLOAT4S` float4s):
/// - 0: film half width, half height (mm), bands, 1 / (the centre's effective pupil area)
/// - 1: rear vertex z, rear aperture radius, front vertex z, film radius covered by the bands
/// - 2 + 2i: surface radius (0 = stop), vertex z (film at 0), aperture radius, -
/// - 3 + 2i: IOR after the surface (toward the film) at R, G, B
/// - 2 + 2·MAX_SURFACES + b: band b's exit-pupil bounds on the rear vertex plane for a film point
///   on +x: x min, x max, y half extent, area (0 = the band passes no light)
pub fn pack(p: &Params) -> (Vec<[f32; 4]>, usize) {
    let lens = &LENSES[(p.lens as usize - 1).min(LENSES.len() - 1)];
    let info = INFO[(p.lens as usize - 1).min(LENSES.len() - 1)];
    let stop = (info.f_number / p.f_number.max(info.f_number)).min(1.);
    let mut s = Surfaces::new(lens, stop, p.dispersion);
    let h = s.entrance_pupil();
    let film = s.film_distance(p.focus_mm, h * 0.1, info.focal);
    let infinity = s.film_distance(1e8, h * 0.1, info.focal);
    for x in &mut s.s {
        x.z -= film;
    }
    // Film size for the field of view; the effective focal length grows with focus extension.
    let image_focal = info.focal + (film - infinity).max(0.);
    let half = p.fov * 0.5;
    // The fisheye spans 1.6× the field of view setting (at most 100°), so it can show its whole
    // 156° image circle.
    let half_h = image_focal * if lens.fisheye { half * 1.6 } else { half.tan() };
    let half_w = half_h * p.aspect;
    let radius = (half_w * half_w + half_h * half_h).sqrt();
    let n = s.s.len();
    let rear = s.s[n - 1];
    let mut out = vec![[0.; 4]; FLOAT4S];
    for (i, x) in s.s.iter().enumerate() {
        out[2 + 2 * i] = [x.radius, x.z, x.aperture, 0.];
        let e = |c: usize| if x.eta[c] <= 0. { 1. } else { x.eta[c] };
        out[3 + 2 * i] = [e(0), e(1), e(2), 0.];
    }
    // Exit-pupil bounds per band: rays from film points across the band to a grid on the rear
    // vertex plane; the passing grid points, grown by one cell.
    const G: usize = 40;
    const K: usize = 4;
    let cell = 2. * rear.aperture / G as f32;
    let grid = |j: usize| -rear.aperture + (j as f32 + 0.5) * cell;
    let passes = |fx: f32, x: f32, y: f32| {
        let o = [fx, 0., 0.];
        let d = norm([x - fx, y, rear.z]);
        s.trace_from_film(o, d, 1).is_some()
    };
    for b in 0..BINS {
        let (mut x0, mut x1, mut y1) = (f32::MAX, f32::MIN, 0f32);
        for k in 0..K {
            let fx = radius * (b as f32 + (k as f32 + 0.5) / K as f32) / BINS as f32;
            for jy in 0..G / 2 {
                let y = grid(G / 2 + jy);
                for jx in 0..G {
                    let x = grid(jx);
                    if passes(fx, x, y) {
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
            }
        }
        let r = rear.aperture;
        out[2 + 2 * MAX_SURFACES + b] = if x0 <= x1 {
            let (x0, x1, y1) = ((x0 - cell).max(-r), (x1 + cell).min(r), (y1 + cell).min(r));
            [x0, x1, y1, (x1 - x0) * 2. * y1]
        } else {
            [0.; 4]
        };
    }
    // The centre's effective pupil: E[pass · cos⁴θ] over band 0's bounds, times their area.
    let b0 = out[2 + 2 * MAX_SURFACES];
    let mut sum = 0.;
    const C: usize = 64;
    for jy in 0..C {
        for jx in 0..C {
            let x = b0[0] + (b0[1] - b0[0]) * (jx as f32 + 0.5) / C as f32;
            let y = b0[2] * (2. * (jy as f32 + 0.5) / C as f32 - 1.);
            if passes(0., x, y) {
                let c = norm([x, y, rear.z])[2];
                sum += c * c * c * c;
            }
        }
    }
    let center = sum / (C * C) as f32 * b0[3];
    out[0] = [
        half_w,
        half_h,
        BINS as f32,
        if center > 0. { 1. / center } else { 0. },
    ];
    out[1] = [rear.z, rear.aperture, s.s[0].z, radius];
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lenses_match_their_names() {
        // Focal lengths close to the nominal ones, plausible wide-open f-numbers.
        for (l, (f, n)) in INFO
            .iter()
            .zip([(50., 2.), (22., 4.), (10., 4.), (50., 2.8)])
        {
            assert!((l.focal - f).abs() / f < 0.15, "{l:?}");
            assert!(l.f_number > n * 0.6 && l.f_number < n * 1.6, "{l:?}");
        }
    }
    #[test]
    fn focus_moves_the_film_out() {
        let s = Surfaces::new(&LENSES[0], 1., 1.);
        let h = s.entrance_pupil();
        let f = INFO[0].focal;
        let inf = s.film_distance(1e8, h * 0.1, f);
        let near = s.film_distance(1000., h * 0.1, f);
        // Thin lens: extension ≈ f² / (d - f).
        assert!(
            near > inf && ((near - inf) - f * f / (1000. - f)).abs() < 1.5,
            "{inf} {near}"
        );
    }
    #[test]
    fn pupil_bounds_pass_light() {
        for i in 1..=LENSES.len() as u32 {
            let (d, n) = pack(&Params {
                lens: i,
                f_number: 0.,
                focus_mm: 3000.,
                fov: 0.7,
                aspect: 1.6,
                dispersion: 1.,
            });
            assert!(n <= MAX_SURFACES);
            assert!(d[0][3] > 0. && d[2 + 2 * MAX_SURFACES][3] > 0., "lens {i}");
        }
    }
}
