//! Opacity micromaps for alpha-tested meshes (track 20a (a), DXR 1.2).
//!
//! The shader's alpha test (`alpha_hole` in pathtrace.hlsl) reads the base map's alpha at mip 0
//! through a wrapping linear sampler and calls a hit a hole when it is below the cutoff. Each
//! triangle is cut into 4^level micro-triangles, and each micro-triangle gets one of four
//! states: transparent or opaque when every texel the sampler can reach anywhere inside it
//! (its uv box, plus a texel of margin) agrees, and unknown otherwise. The RT cores skip
//! transparent ones and commit opaque ones without the shader; unknown ones come back to the
//! same alpha test as before, so the image stays bit-identical. Margins (a texel around the
//! box, 1/512 around the cutoff) make the classification conservative.

/// The four 2-bit states (D3D12_RAYTRACING_OPACITY_MICROMAP_STATE).
const TRANSPARENT: u8 = 0;
const OPAQUE: u8 = 1;
const UNKNOWN_TRANSPARENT: u8 = 2;
/// Special per-triangle indices (D3D12_RAYTRACING_OPACITY_MICROMAP_SPECIAL_INDEX).
pub const FULLY_TRANSPARENT: u32 = -1i32 as u32;
pub const FULLY_OPAQUE: u32 = -2i32 as u32;
pub const FULLY_UNKNOWN_TRANSPARENT: u32 = -3i32 as u32;
/// Deepest subdivision used (4^7 = 16384 micro-triangles, 4 KiB per triangle).
const MAX_LEVEL: u32 = 7;
/// Aim for micro-triangles of about this many texels.
const TEXELS_PER_MICRO: f64 = 16.;
/// Upper bound on one mesh's OMM data; deeper levels are dropped until it fits.
const MAX_BYTES: usize = 64 << 20;

/// An OMM array's inputs: `data` holds every micromap (2 bits per micro-triangle, the first in
/// the low bits), `descs` (byte offset, level) one per micromap, and `index` one per triangle:
/// a micromap or a special index.
#[derive(Debug, Default)]
pub struct Micromaps {
    pub data: Vec<u8>,
    pub descs: Vec<(u32, u32)>,
    pub index: Vec<u32>,
}
impl Micromaps {
    /// Count of micromaps per subdivision level (the build's histogram).
    pub fn histogram(&self) -> Vec<(u32, u32)> {
        let mut h: Vec<(u32, u32)> = vec![];
        for &(_, level) in &self.descs {
            match h.iter_mut().find(|e| e.0 == level) {
                Some(e) => e.1 += 1,
                None => h.push((level, 1)),
            }
        }
        h
    }
    /// True when every triangle is settled by a special index (nothing to build).
    pub fn is_trivial(&self) -> bool {
        self.descs.is_empty()
    }
}

/// Min and max alpha over power-of-two blocks of the map, level k holding 2^k × 2^k blocks.
struct Pyramid {
    levels: Vec<(usize, usize, Vec<u8>, Vec<u8>)>,
}
impl Pyramid {
    fn new(width: usize, height: usize, rgba: &[u8]) -> Self {
        let alpha: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let mut levels = vec![(width, height, alpha.clone(), alpha)];
        while levels.last().is_some_and(|l| l.0 > 1 || l.1 > 1) {
            let (w, h, lo, hi) = levels.last().unwrap();
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            let (mut nlo, mut nhi) = (vec![255u8; nw * nh], vec![0u8; nw * nh]);
            for y in 0..*h {
                for x in 0..*w {
                    let (i, j) = (y * w + x, (y / 2) * nw + x / 2);
                    nlo[j] = nlo[j].min(lo[i]);
                    nhi[j] = nhi[j].max(hi[i]);
                }
            }
            levels.push((nw, nh, nlo, nhi));
        }
        Pyramid { levels }
    }
    /// Min and max alpha over texels x0..=x1, y0..=y1 (any integers; the map wraps).
    fn range(&self, x0: i64, x1: i64, y0: i64, y1: i64) -> (u8, u8) {
        let (w, h) = (self.levels[0].0 as i64, self.levels[0].1 as i64);
        let span = (x1 - x0 + 1).max(y1 - y0 + 1).max(1) as u64;
        // Blocks about half the span wide: at most three or four per axis and segment.
        let k = (63 - span.leading_zeros()).saturating_sub(1) as usize;
        let k = k.min(self.levels.len() - 1);
        let (bw, _, lo, hi) = &self.levels[k];
        let segments = |a: i64, b: i64, n: i64| -> Vec<(i64, i64)> {
            if b - a + 1 >= n {
                return vec![(0, n - 1)];
            }
            let s = a.rem_euclid(n);
            let e = s + (b - a);
            if e < n {
                vec![(s, e)]
            } else {
                vec![(s, n - 1), (0, e - n)]
            }
        };
        let (mut mn, mut mx) = (255u8, 0u8);
        for (ya, yb) in segments(y0, y1, h) {
            for (xa, xb) in segments(x0, x1, w) {
                for by in (ya >> k)..=(yb >> k) {
                    for bx in (xa >> k)..=(xb >> k) {
                        let i = by as usize * bw + bx as usize;
                        mn = mn.min(lo[i]);
                        mx = mx.max(hi[i]);
                    }
                }
            }
        }
        (mn, mx)
    }
}

/// Micro-triangle `index` of a triangle cut at `level`, as barycentrics (u, v) of its three
/// corners (u weights the triangle's second vertex, v its third, as in DXR). The order is the
/// hierarchical "bird curve" of the D3D12 / Vulkan micromap specifications.
pub fn micro_triangle(index: u32, level: u32) -> [[f64; 2]; 3] {
    if level == 0 {
        return [[0., 0.], [1., 0.], [0., 1.]];
    }
    fn even_bits(mut x: u32) -> u32 {
        x &= 0x5555_5555;
        x = (x | (x >> 1)) & 0x3333_3333;
        x = (x | (x >> 2)) & 0x0f0f_0f0f;
        x = (x | (x >> 4)) & 0x00ff_00ff;
        (x | (x >> 8)) & 0x0000_ffff
    }
    fn prefix_eor(mut x: u32) -> u32 {
        x ^= (x >> 1) & 0x7fff_7fff;
        x ^= (x >> 2) & 0x3fff_3fff;
        x ^= (x >> 4) & 0x0fff_0fff;
        x ^= (x >> 8) & 0x00ff_00ff;
        x
    }
    let b0 = even_bits(index);
    let b1 = even_bits(index >> 1);
    let fx = prefix_eor(b0);
    let fy = prefix_eor(b0 & !b1);
    let t = fy ^ b1;
    let mask = (1u32 << level) - 1;
    let mut iu = ((fx & !t) | (b0 & !t) | (!b0 & !fx & t)) & mask;
    let mut iv = (fy ^ b0) & mask;
    let iw = ((!fx & !t) | (b0 & !t) | (!b0 & fx & t)) & mask;
    let upright = ((iu & 1) ^ (iv & 1) ^ (iw & 1)) != 0;
    if !upright {
        iu += 1;
        iv += 1;
    }
    let scale = 1. / (1u32 << level) as f64;
    let d = if upright { scale } else { -scale };
    let (u, v) = (iu as f64 * scale, iv as f64 * scale);
    [[u, v], [u + d, v], [u, v + d]]
}

/// Builds the micromaps for `uvs` (three per triangle) over a `width` × `height` RGBA8 map
/// with alpha `cutoff` (holes below it). `None` when the map is empty.
pub fn build(
    uvs: &[[f32; 2]],
    width: u32,
    height: u32,
    rgba: &[u8],
    cutoff: f32,
) -> Option<Micromaps> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || !uvs.len().is_multiple_of(3) {
        return None;
    }
    let pyramid = Pyramid::new(w, h, rgba);
    // Alpha bytes at or below `clear` are surely holes, at or above `solid` surely not.
    let margin = 1. / 512.;
    let clear = ((cutoff - margin) * 255.).ceil() as i64 - 1;
    let solid = ((cutoff + margin) * 255.).ceil() as i64;
    let levels: Vec<u32> = uvs
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let (a, b, c) = (t[0], t[1], t[2]);
            let area = ((b[0] - a[0]) as f64 * (c[1] - a[1]) as f64
                - (c[0] - a[0]) as f64 * (b[1] - a[1]) as f64)
                .abs()
                * 0.5
                * (w * h) as f64;
            let micro = (area / TEXELS_PER_MICRO).max(1.);
            ((micro.log2() / 2.).ceil() as u32).clamp(1, MAX_LEVEL)
        })
        .collect();
    let mut cap = MAX_LEVEL;
    while cap > 1
        && levels
            .iter()
            .map(|&l| (1usize << (2 * l.min(cap))) / 4)
            .sum::<usize>()
            > MAX_BYTES
    {
        cap -= 1;
    }
    let mut out = Micromaps::default();
    let mut states = vec![];
    for (t, &level) in uvs.as_chunks::<3>().0.iter().zip(&levels) {
        let level = level.min(cap);
        let n = 1u32 << (2 * level);
        states.clear();
        for i in 0..n {
            let bary = micro_triangle(i, level);
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for [u, v] in bary {
                let p = |k: usize| {
                    t[0][k] as f64 * (1. - u - v) + t[1][k] as f64 * u + t[2][k] as f64 * v
                };
                // Texel space of a linear sampler: texel centres at integers.
                let (x, y) = (p(0) * w as f64 - 0.5, p(1) * h as f64 - 0.5);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
            if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
                states.push(UNKNOWN_TRANSPARENT);
                continue;
            }
            // A sample at x reads texels floor(x) and floor(x) + 1; one more on each side.
            let (mn, mx) = pyramid.range(
                x0.floor() as i64 - 1,
                x1.floor() as i64 + 2,
                y0.floor() as i64 - 1,
                y1.floor() as i64 + 2,
            );
            states.push(if (mx as i64) <= clear {
                TRANSPARENT
            } else if (mn as i64) >= solid {
                OPAQUE
            } else {
                UNKNOWN_TRANSPARENT
            });
        }
        let first = states[0];
        if states.iter().all(|&s| s == first) {
            out.index.push(match first {
                TRANSPARENT => FULLY_TRANSPARENT,
                OPAQUE => FULLY_OPAQUE,
                _ => FULLY_UNKNOWN_TRANSPARENT,
            });
            continue;
        }
        let offset = out.data.len();
        out.data
            .resize(offset + (n as usize / 4).max(1).next_multiple_of(4), 0);
        for (i, &s) in states.iter().enumerate() {
            out.data[offset + i / 4] |= s << (2 * (i % 4));
        }
        out.index.push(out.descs.len() as u32);
        out.descs.push((offset as u32, level));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The micro-triangles of a level tile the triangle: each has area 1/4^level, and the
    /// centroids are distinct points inside it.
    #[test]
    fn micro_triangles_tile_the_triangle() {
        for level in 0..=5u32 {
            let n = 1u32 << (2 * level);
            let mut centroids = vec![];
            for i in 0..n {
                let [a, b, c] = micro_triangle(i, level);
                let area = ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
                assert!(
                    (area - 1. / n as f64).abs() < 1e-12,
                    "level {level} index {i}"
                );
                let g = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
                assert!(g[0] > 0. && g[1] > 0. && g[0] + g[1] < 1.);
                centroids.push(((g[0] * 1e9) as i64, (g[1] * 1e9) as i64));
            }
            centroids.sort_unstable();
            centroids.dedup();
            assert_eq!(centroids.len(), n as usize);
        }
    }

    /// The curve is connected (neighbours share at least a corner), and level 1 runs: the
    /// corner at the first vertex, the middle, the corner at the second (u), the corner at the
    /// third (v). Whether this matches the hardware is checked by the A/B render (engine-notes
    /// "GPU efficiency": OMM on and off must give the same image).
    #[test]
    fn curve_order() {
        let key = |p: [f64; 2]| ((p[0] * 64.).round() as i64, (p[1] * 64.).round() as i64);
        for level in 1..=4 {
            for i in 0..(1u32 << (2 * level)) - 1 {
                let (a, b) = (micro_triangle(i, level), micro_triangle(i + 1, level));
                let shared = a
                    .iter()
                    .filter(|p| b.iter().any(|q| key(**p) == key(*q)))
                    .count();
                assert!(shared >= 1, "level {level} index {i}");
            }
        }
        let centroid = |i| {
            let t = micro_triangle(i, 1);
            [
                (t[0][0] + t[1][0] + t[2][0]) / 3.,
                (t[0][1] + t[1][1] + t[2][1]) / 3.,
            ]
        };
        let near =
            |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9;
        assert!(near(centroid(0), [1. / 6., 1. / 6.]));
        assert!(near(centroid(1), [1. / 3., 1. / 3.]));
        assert!(near(centroid(2), [2. / 3., 1. / 6.]));
        assert!(near(centroid(3), [1. / 6., 2. / 3.]));
    }

    fn map(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                v[(y * w + x) * 4 + 3] = f(x, y);
            }
        }
        v
    }

    #[test]
    fn uniform_maps_give_special_indices() {
        let uv = [[0., 0.], [1., 0.], [0., 1.]];
        let solid = build(&uv, 64, 64, &map(64, 64, |_, _| 255), 0.5).unwrap();
        assert_eq!(solid.index, vec![FULLY_OPAQUE]);
        let clear = build(&uv, 64, 64, &map(64, 64, |_, _| 0), 0.5).unwrap();
        assert_eq!(clear.index, vec![FULLY_TRANSPARENT]);
        assert!(solid.is_trivial() && clear.is_trivial());
    }

    /// A half-clear map: micro-triangles far from the edge are settled, ones near it unknown,
    /// and no settled state contradicts a dense sample of the bilinear alpha inside it.
    #[test]
    fn states_are_conservative() {
        let (w, h) = (64usize, 64usize);
        let rgba = map(w, h, |x, _| if x < 29 { 0 } else { 255 });
        let uv = [[0.1f32, 0.1], [0.9, 0.1], [0.1, 0.9]];
        let m = build(&uv, w as u32, h as u32, &rgba, 0.5).unwrap();
        assert_eq!(m.descs.len(), 1);
        let (offset, level) = m.descs[0];
        let alpha = |u: f64, v: f64| {
            let (x, y) = (u * w as f64 - 0.5, v * h as f64 - 0.5);
            let (fx, fy) = (x.floor(), y.floor());
            let at = |i: f64, j: f64| {
                let (i, j) = (
                    (i as i64).rem_euclid(w as i64) as usize,
                    (j as i64).rem_euclid(h as i64) as usize,
                );
                rgba[(j * w + i) * 4 + 3] as f64 / 255.
            };
            let (tx, ty) = (x - fx, y - fy);
            (at(fx, fy) * (1. - tx) + at(fx + 1., fy) * tx) * (1. - ty)
                + (at(fx, fy + 1.) * (1. - tx) + at(fx + 1., fy + 1.) * tx) * ty
        };
        let (mut settled, mut unknown) = (0, 0);
        for i in 0..1u32 << (2 * level) {
            let s = (m.data[offset as usize + i as usize / 4] >> (2 * (i % 4))) & 3;
            let [a, b, c] = micro_triangle(i, level);
            if s >= 2 {
                unknown += 1;
                continue;
            }
            settled += 1;
            for p in 0..=8 {
                for q in 0..=8 - p {
                    let (wb, wc) = (p as f64 / 8., q as f64 / 8.);
                    let (bu, bv) = (
                        a[0] + (b[0] - a[0]) * wb + (c[0] - a[0]) * wc,
                        a[1] + (b[1] - a[1]) * wb + (c[1] - a[1]) * wc,
                    );
                    let pt = |k: usize| {
                        uv[0][k] as f64 * (1. - bu - bv)
                            + uv[1][k] as f64 * bu
                            + uv[2][k] as f64 * bv
                    };
                    let hole = alpha(pt(0), pt(1)) < 0.5;
                    assert_eq!(hole, s == TRANSPARENT, "micro {i}");
                }
            }
        }
        assert!(settled > unknown, "{settled} settled, {unknown} unknown");
    }

    #[test]
    fn wrapping_ranges() {
        let rgba = map(8, 8, |x, y| (x * 8 + y) as u8);
        let p = Pyramid::new(8, 8, &rgba);
        // Columns 7 and 8 (= 0) of row 0: alpha 56 and 0.
        assert_eq!(p.range(7, 8, 0, 0), (0, 56));
        // Wider ranges read coarser blocks: never narrower than the texels asked for.
        let (lo, hi) = p.range(-20, 40, 3, 3);
        assert!(lo <= 3 && hi >= 59);
    }
}
