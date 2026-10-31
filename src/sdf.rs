//! Procedural primitives and SDFs (roadmap track 13, 2026-10-31).
//! An object with a non-empty `Object::procedural` string has no triangles at all: the shape
//! is one procedural primitive (its AABB) in its own BLAS, and the trace shader computes the
//! hit inside the existing Proceed() loop (RayQuery: CandidateType() ==
//! CANDIDATE_PROCEDURAL_PRIMITIVE, march the chain in object space, then
//! CommitProceduralPrimitiveHit(t)). Normals are the gradient of the whole chain by central
//! differences at the hit point (exact for primitives, robust through smooth CSG and fractals).
//! Shapes live in object space (the unit box), so the instance transform's scale and rotation
//! move them like any other object.
//!
//! Grammar (parsed on the CPU here; pathtrace.hlsl mirrors the evaluation exactly):
//!   chain := atom (op atom ('@' k)?)*      at most three atoms
//!   op    := '+' | '-' | '*'              union / subtract / intersect, k = smooth radius
//!   atom  := kind ('/' number)*
//!   kind  := sphere | torus | cylinder | box | bulb | menger | blob
//! Args per kind (offsets x/y/z follow on every atom, default 0):
//!   sphere/r, torus/r1/r2, cylinder/r/h, box/hx/hy/hz, bulb/power, menger/level, blob/seed.
//! A '-' starts an op only when a kind name follows it, so negative coordinates are fine.
//! Out-of-range args clamp instead of erroring.
//!
//! GPU record: 3 float4 per atom, 9 float4 per object slot (PROC_STRIDE 9 in the shader):
//!   float4(kind, p0, p1, p2), float4(p3, p4, p5, op), float4(k, 0, 0, 0).
//! Unused atoms: kind -1. Kind ids: 0 sphere, 1 torus, 2 cylinder, 3 box, 4 bulb, 5 menger,
//! 6 blob. Op codes: 0 union, 1 subtract, 2 intersect. The atom 0 third float4 instead
//! carries (safety, budget, 0, 0), shared with the shader march.
use glam::Vec3;

pub const MAX_ATOMS: usize = 3;
pub const STRIDE: usize = 36; // 9 float4 per object slot
pub const FLOATS_PER_ATOM: usize = 12; // three float4s

#[derive(Clone)]
pub struct Sdf {
    pub n: usize,
    pub kind: [f32; MAX_ATOMS],
    pub p: [[f32; 6]; MAX_ATOMS],
    pub op: [f32; MAX_ATOMS],
    pub k: [f32; MAX_ATOMS],
    /// Local-space AABB: grounding, outlines and the primitive's BLAS AABB.
    pub lo: Vec3,
    pub hi: Vec3,
    /// Whole-chain march safety factor (fractal DE / smooth max overestimates).
    pub safety: f32,
    /// March step budget (fractals need a wider loop than primitives).
    pub budget: f32,
}

pub const KIND_NAMES: [&str; 7] = [
    "sphere", "torus", "cylinder", "box", "bulb", "menger", "blob",
];
pub const KIND_SPHERE: i32 = 0;
pub const KIND_TORUS: i32 = 1;
pub const KIND_CYLINDER: i32 = 2;
pub const KIND_BOX: i32 = 3;
pub const KIND_BULB: i32 = 4;
pub const KIND_MENGER: i32 = 5;
pub const KIND_BLOB: i32 = 6;
pub const OP_UNION: f32 = 0.;
pub const OP_SUBTRACT: f32 = 1.;
pub const OP_INTERSECT: f32 = 2.;

// Shared with pathtrace.hlsl: ball placement for `blob` and the Mandelbulb DE.
pub const HASH_MUL_1: u32 = 0x7feb_352d;
pub const HASH_MUL_2: u32 = 0x846c_a68b;
pub const BLOB_BALLS: usize = 6;
pub const BLOB_K: f32 = 0.16;
pub const BLOB_MUL_A: u32 = 0x9E37_79B1;
pub const BLOB_MUL_B: u32 = 0x85EB_CA6B;
pub const BLOB_ADD: u32 = 0x27D4_EB2F;
pub const BLOB_SPREAD: f32 = 0.66;
pub const BULB_SCALE: f32 = 0.74;
pub const DE_ITERS: usize = 11;
pub const HIT_EPS: f32 = 0.001;
pub const BUDGET_PRIM: f32 = 72.0;
pub const BUDGET_FRACTAL: f32 = 130.0;
pub const SAFETY_SMOOTH: f32 = 0.75;
pub const SAFETY_BULB: f32 = 0.55;

pub fn hash(x: u32) -> u32 {
    let x = x ^ (x >> 16);
    let x = x.wrapping_mul(HASH_MUL_1);
    let x = x ^ (x >> 15);
    let x = x.wrapping_mul(HASH_MUL_2);
    x ^ (x >> 16)
}
pub fn unit(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16777216.0)
}
/// `blob` ball placement, identical on both sides so picking agrees with the render.
pub fn blob_ball(seed: u32, i: usize) -> (Vec3, f32) {
    let h = hash(
        seed.wrapping_mul(BLOB_MUL_A)
            .wrapping_add((i as u32).wrapping_mul(BLOB_MUL_B))
            .wrapping_add(BLOB_ADD),
    );
    let c = Vec3::new(
        unit(h) - 0.5,
        unit(h.wrapping_add(1)) - 0.5,
        unit(h.wrapping_add(2)) - 0.5,
    );
    let r = 0.16 + 0.22 * unit(h.wrapping_add(3));
    (c * BLOB_SPREAD, r)
}

/// Per-kind defaults so a bare `sphere` still means a sensible shape.
fn default_args(k: usize) -> [f32; 6] {
    match k {
        0 => [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        1 => [0.72, 0.28, 0.0, 0.0, 0.0, 0.0],
        2 => [0.7, 1.0, 0.0, 0.0, 0.0, 0.0],
        3 => [1.0, 1.0, 1.0, 0.0, 0.0, 0.0],
        4 => [8.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        5 => [3.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        _ => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    }
}

/// Clamp the args of one atom into their legal ranges.
fn clamp_args(kind: f32, pa: [f32; 6]) -> [f32; 6] {
    let mut out = pa;
    let ki = (kind + 0.5) as i32;
    if ki == 0 {
        out[0] = out[0].clamp(0.02, 1.0);
    } else if ki == 1 {
        out[0] = out[0].clamp(0.02, 1.0);
        out[1] = out[1].clamp(0.02, 1.0);
        if out[0] + out[1] > 1.0 {
            let s = 1.0 / (out[0] + out[1]);
            out[0] *= s;
            out[1] *= s;
        }
    } else if ki == 2 {
        out[0] = out[0].clamp(0.02, 1.0);
        out[1] = out[1].clamp(0.02, 1.0);
    } else if ki == 3 {
        out[0] = out[0].clamp(0.02, 1.0);
        out[1] = out[1].clamp(0.02, 1.0);
        out[2] = out[2].clamp(0.02, 1.0);
    } else if ki == 4 {
        out[0] = out[0].clamp(3.0, 12.0);
    } else if ki == 5 {
        out[0] = (out[0] + 0.5).floor().clamp(1.0, 5.0);
    } else if ki == 6 {
        out[0] = out[0].clamp(0.0, 1.0e9);
    }
    if ki == 0 {
        for v in out[1..4].iter_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
    } else if ki == 1 || ki == 2 {
        for v in out[2..5].iter_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
    } else if ki == 3 {
        for v in out[3..6].iter_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
    }
    out
}

/// Bounds of one atom (its offsets applied). Menger and the blob fit the unit box by
/// construction; the bulb's bailout ball (radius 2 in c-space) reaches 1.48 object units,
/// so its box is ±1.5.
pub fn atom_bounds(kind: f32, pa: [f32; 6]) -> (Vec3, Vec3) {
    let ki = (kind + 0.5) as i32;
    if ki == 0 {
        let c = Vec3::new(pa[1], pa[2], pa[3]);
        let r = Vec3::splat(pa[0]);
        (c - r, c + r)
    } else if ki == 1 {
        let c = Vec3::new(pa[2], pa[3], pa[4]);
        let e = Vec3::new(pa[0] + pa[1], pa[1], pa[0] + pa[1]);
        (c - e, c + e)
    } else if ki == 2 {
        let c = Vec3::new(pa[2], pa[3], pa[4]);
        let e = Vec3::new(pa[0], pa[1], pa[0]);
        (c - e, c + e)
    } else if ki == 3 {
        let c = Vec3::new(pa[3], pa[4], pa[5]);
        let e = Vec3::new(pa[0], pa[1], pa[2]);
        (c - e, c + e)
    } else if ki == 4 {
        (Vec3::splat(-1.5), Vec3::splat(1.5))
    } else {
        (Vec3::splat(-1.0), Vec3::splat(1.0))
    }
}

/// Read one number token starting at `i`; the next index comes back with it.
fn read_number(spec: &str, b: &[u8], i: usize) -> Result<(f32, usize), String> {
    let mut j = i;
    if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
        j += 1;
    }
    let digits = j;
    while j < b.len() && ((b[j] >= b'0' && b[j] <= b'9') || b[j] == b'.') {
        j += 1;
    }
    if j == digits {
        return Err(format!("sdf: expected a number at {i}"));
    }
    let bad = &spec[i..j];
    match spec[i..j].parse::<f32>() {
        Ok(v) => Ok((v, j)),
        Err(_) => Err(format!("sdf: bad number {bad}")),
    }
}

/// Parse the grammar above. Errors carry a message the UI can show.
pub fn parse(spec: &str) -> Result<Sdf, String> {
    let b = spec.as_bytes();
    let m = b.len();
    if m == 0 {
        return Err("sdf: empty shape".to_string());
    }
    if m > 256 {
        return Err("sdf: shape too long".to_string());
    }
    let mut i = 0usize;
    let mut kinds = [0.0; MAX_ATOMS];
    let mut p = [[0.0; 6]; MAX_ATOMS];
    let mut op = [0.0; MAX_ATOMS];
    let mut k = [0.0; MAX_ATOMS];
    let mut n = 0usize;
    loop {
        let s = i;
        while i < m && b[i] >= b'a' && b[i] <= b'z' {
            i += 1;
        }
        if i == s {
            return Err("sdf: expected a shape name".to_string());
        }
        let name = &spec[s..i];
        let pos = KIND_NAMES.iter().position(|nm| *nm == name);
        if pos.is_none() {
            return Err(format!("sdf: unknown shape {name}"));
        }
        if n >= MAX_ATOMS {
            return Err("sdf: at most three shapes per object".to_string());
        }
        let ki = pos.unwrap();
        p[n] = default_args(ki);
        let mut argc = 0;
        while i < m && b[i] == b'/' {
            i += 1;
            match read_number(spec, b, i) {
                Ok((v, j)) => {
                    if argc < 6 {
                        p[n][argc] = v;
                        argc += 1;
                    }
                    i = j;
                }
                Err(e) => return Err(e),
            }
        }
        kinds[n] = ki as f32;
        p[n] = clamp_args(ki as f32, p[n]);
        if i < m && b[i] == b'@' {
            i += 1;
            match read_number(spec, b, i) {
                Ok((v, j)) => {
                    k[n] = v;
                    i = j;
                }
                Err(e) => return Err(e),
            }
        }
        n += 1;
        if i >= m {
            break;
        }
        let o = if b[i] == b'+' {
            OP_UNION
        } else if b[i] == b'-' {
            OP_SUBTRACT
        } else if b[i] == b'*' {
            OP_INTERSECT
        } else {
            return Err(format!("sdf: expected an op at {i}"));
        };
        i += 1;
        if i >= m || !(b[i] >= b'a' && b[i] <= b'z') {
            return Err("sdf: an op must be followed by a shape name".to_string());
        }
        if n >= MAX_ATOMS {
            return Err("sdf: at most three shapes per object".to_string());
        }
        op[n] = o;
    }

    let (mut lo, mut hi) = atom_bounds(kinds[0], p[0]);
    let mut safety: f32 = 1.0;
    let mut fractal = false;
    let mut bulb = false;
    let mut smooth = false;
    let mut j = 1;
    while j < n {
        let (l2, h2) = atom_bounds(kinds[j], p[j]);
        if op[j] == OP_UNION {
            lo = lo.min(l2);
            hi = hi.max(h2);
        }
        let kj = (kinds[j] + 0.5) as i32;
        if kj == 4 || kj == 5 {
            fractal = true;
        }
        if kj == 4 {
            bulb = true;
        }
        if k[j] > 1e-4 && op[j] != OP_UNION {
            smooth = true;
        }
        j += 1;
    }
    if bulb {
        safety = safety.min(SAFETY_BULB);
    }
    if smooth {
        safety = safety.min(SAFETY_SMOOTH);
    }
    let budget = if fractal || smooth {
        BUDGET_FRACTAL
    } else {
        BUDGET_PRIM
    };
    Ok(Sdf {
        n,
        kind: kinds,
        p,
        op,
        k,
        lo,
        hi,
        safety,
        budget,
    })
}

/// Chain combination used identically on both sides. Union smooth-min never overestimates
/// distance, so it is safe for the march; smooth max (subtract / intersect) overestimates,
/// and the chain safety factor keeps those cases conservative.
pub fn sd_chain(c: &Sdf, q: Vec3) -> f32 {
    let d = sd_atom(c.kind[0], c.p[0], q);
    if c.n < 2 {
        return d;
    }
    let d1 = join(d, sd_atom(c.kind[1], c.p[1], q), c.op[1], c.k[1]);
    if c.n < 3 {
        return d1;
    }
    join(d1, sd_atom(c.kind[2], c.p[2], q), c.op[2], c.k[2])
}

fn join(a: f32, b: f32, op: f32, k: f32) -> f32 {
    if op == OP_UNION {
        return smin(a, b, k);
    }
    if op == OP_SUBTRACT {
        return smax(a, -b, k);
    }
    smax(a, b, k)
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 1e-4 {
        if a < b {
            return a;
        }
        return b;
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    if k <= 1e-4 {
        if a > b {
            return a;
        }
        return b;
    }
    let h = (0.5 + 0.5 * (a - b) / k).clamp(0.0, 1.0);
    a * h + b * (1.0 - h) + k * h * (1.0 - h)
}

/// Signed distance to one atom in object space. Slot mapping (mirrored exactly in
/// pathtrace.hlsl): A=(kind,p0,p1,p2), B=(p3,p4,p5,op); centers sit per kind:
/// sphere p1..p3, torus/cylinder p2..p4, box p3..p5.
pub fn sd_atom(kind: f32, pa: [f32; 6], q: Vec3) -> f32 {
    let ki = (kind + 0.5) as i32;
    if ki == KIND_SPHERE {
        let w = q - Vec3::new(pa[1], pa[2], pa[3]);
        return w.length() - pa[0];
    }
    if ki == KIND_TORUS {
        let w = q - Vec3::new(pa[2], pa[3], pa[4]);
        let t = (w.x * w.x + w.z * w.z).sqrt() - pa[0];
        return (t * t + w.y * w.y).sqrt() - pa[1];
    }
    if ki == KIND_CYLINDER {
        let w = q - Vec3::new(pa[2], pa[3], pa[4]);
        return capped2((w.x * w.x + w.z * w.z).sqrt() - pa[0], w.y.abs() - pa[1]);
    }
    if ki == KIND_BOX {
        let w = q - Vec3::new(pa[3], pa[4], pa[5]);
        return box3(w.abs() - Vec3::new(pa[0], pa[1], pa[2]));
    }
    if ki == KIND_BULB {
        return bulb(q, pa[0]);
    }
    if ki == KIND_MENGER {
        return menger(q, pa[0]);
    }
    if ki == KIND_BLOB {
        return blob(q, pa[0]);
    }
    1.0e9
}

fn capped2(ex: f32, ey: f32) -> f32 {
    let ax = if ex > 0.0 { ex } else { 0.0 };
    let ay = if ey > 0.0 { ey } else { 0.0 };
    let mx = if ex > ey { ex } else { ey };
    let inner = if mx < 0.0 { mx } else { 0.0 };
    (ax * ax + ay * ay).sqrt() + inner
}

fn box3(e: Vec3) -> f32 {
    let ax = if e.x > 0.0 { e.x } else { 0.0 };
    let ay = if e.y > 0.0 { e.y } else { 0.0 };
    let az = if e.z > 0.0 { e.z } else { 0.0 };
    let xy = if e.x > e.y { e.x } else { e.y };
    let mx = if xy > e.z { xy } else { e.z };
    let inner = if mx < 0.0 { mx } else { 0.0 };
    (ax * ax + ay * ay + az * az).sqrt() + inner
}

fn rect2(a: f32, b: f32) -> f32 {
    let ax = if a > 0.0 { a } else { 0.0 };
    let bx = if b > 0.0 { b } else { 0.0 };
    let mx = if a > b { a } else { b };
    let inner = if mx < 0.0 { mx } else { 0.0 };
    (ax * ax + bx * bx).sqrt() + inner
}

fn wrap2(v: f32) -> f32 {
    v - (v * 0.5).floor() * 2.0
}

/// Menger sponge on the unit cube, level 1..5: the cube minus middle-third square
/// tubes per level. A max of exact SDFs underestimates the distance, so the march stays
/// conservative; the surface detail at any level comes straight from the loop (roadmap's
/// "Menger at any level" without voxel soup).
pub fn menger(p: Vec3, level: f32) -> f32 {
    let bd = box3(p.abs() - Vec3::splat(1.0));
    let mut rem = 1.0e9;
    let mut sc = 1.0;
    let mut li = 0.0;
    while li + 0.5 < level {
        let fx = wrap2(p.x * sc + 1.0) - 1.0;
        let fy = wrap2(p.y * sc + 1.0) - 1.0;
        let fz = wrap2(p.z * sc + 1.0) - 1.0;
        let sx = (fx.abs() - 1.0 / 3.0) / sc;
        let sy = (fy.abs() - 1.0 / 3.0) / sc;
        let sz = (fz.abs() - 1.0 / 3.0) / sc;
        let t1 = rect2(sx, sy);
        let t2 = rect2(sx, sz);
        let t3 = rect2(sy, sz);
        let ta = if t1 < t2 { t1 } else { t2 };
        let best = if ta < t3 { ta } else { t3 };
        if best < rem {
            rem = best;
        }
        sc *= 3.0;
        li += 1.0;
    }
    let neg = -rem;
    if bd > neg { bd } else { neg }
}

/// Mandelbulb distance estimate with spherical power iteration; the set fits the
/// unit box (input scaled up, the estimate scaled back to object units). The DE
/// underestimates distance (conservative), but needs the wider march budget.
pub fn bulb(q: Vec3, power: f32) -> f32 {
    let c = q * (1.0 / BULB_SCALE);
    let r0 = c.length();
    if r0 > 2.0 {
        // The set fits the bailout ball (radius 2 in c-space); outside it that bound is
        // the only DE that lets the march cross the empty gap (the orbit DE degenerates
        // there). Floor it above the march epsilon: the ball guides the march but is
        // never a commit surface (matches the PROCEDURALS shader branch).
        return ((r0 - 2.0) * BULB_SCALE).max(0.0011);
    }
    let mut z = c;
    let mut r = 0.0;
    let mut dchain = 1.0;
    let mut i = 0usize;
    while i < DE_ITERS {
        r = z.length();
        if r > 2.0 {
            break;
        }
        dchain *= power * if r > 1.0 { r.powf(power - 1.0) } else { 1.0 };
        if z.y < 0.0 {
            z.y = -z.y;
        }
        let inv = if r > 1e-9 { 1.0 / r } else { 0.0 };
        let th = (z.y * inv).clamp(-1.0, 1.0).acos();
        let ph = z.x.atan2(z.z);
        let rn = r.powf(power);
        let th2 = th * power;
        let ph2 = ph * power;
        let s = rn * th2.sin();
        z = Vec3::new(s * ph2.sin(), rn * th2.cos(), s * ph2.cos()) + c;
        i += 1;
    }
    if r > 2.0 {
        let de = if dchain > 1e-12 {
            (r - 2.0) / dchain
        } else {
            0.0
        };
        return de.min(0.5) * BULB_SCALE;
    }
    -r * BULB_SCALE
}

/// Blob of `BLOB_BALLS` hash-placed balls smooth-unioned; the placement uses the same
/// integer hash on both sides (seed bits feed `hash` identically to `asuint` in the shader).
fn blob(q: Vec3, seed: f32) -> f32 {
    let s = seed.to_bits();
    let mut acc = 1.0e9;
    let mut i = 0usize;
    while i < BLOB_BALLS {
        let bb = blob_ball(s, i);
        let d = (q - bb.0).length() - bb.1;
        acc = smin(acc, d, BLOB_K);
        i += 1;
    }
    acc
}

/// CPU march of an object-space ray (picking only needs the t). The step `sdf/|od|`
/// never jumps past the surface in object units; `safety` covers overestimating chains.
pub fn march(c: &Sdf, oo: Vec3, od: Vec3, limit: f32) -> f32 {
    let speed = od.length();
    if speed < 1e-9 {
        return -1.0;
    }
    let mut t = 0.0;
    let mut it = 0.0;
    while it < c.budget {
        let p = oo + od * t;
        let d = sd_chain(c, p);
        if d < HIT_EPS {
            return t;
        }
        let step = if d > 1e-4 { d } else { 1e-4 };
        t += step * c.safety / speed;
        if t > limit {
            return -1.0;
        }
        it += 1.0;
    }
    -1.0
}

/// Flatten a chain into the GPU record (36 floats; layout in the file header).
pub fn table(c: &Sdf) -> [f32; STRIDE] {
    let mut t = [0.0; STRIDE];
    let mut j = 0usize;
    while j < MAX_ATOMS {
        let b = j * FLOATS_PER_ATOM;
        if j < c.n {
            t[b] = c.kind[j];
            t[b + 1] = c.p[j][0];
            t[b + 2] = c.p[j][1];
            t[b + 3] = c.p[j][2];
            t[b + 4] = c.p[j][3];
            t[b + 5] = c.p[j][4];
            t[b + 6] = c.p[j][5];
            t[b + 7] = c.op[j];
            if j == 0 {
                t[b + 8] = c.safety;
                t[b + 9] = c.budget;
            } else {
                t[b + 8] = c.k[j];
            }
        } else {
            t[b] = -1.0;
        }
        j += 1;
    }
    t
}

#[test]
fn sdf_grammar_and_clamps() {
    assert!(parse("sphere").is_ok());
    assert!(parse("sphere/0.4/-0.2/0/0.3").is_ok());
    assert!(parse("box/1/1/1-torus/0.6/0.25@0.08").is_ok());
    assert!(parse("cylinder/0.18/1.0/0/0/0-sphere/0.3/0/0.2/0").is_ok());
    assert!(parse("bulb/8.5").is_ok());
    assert!(parse("menger/4").is_ok());
    assert!(parse("blob/7").is_ok());
    assert!(parse("menger/4-torus/0.5/0.12@0.05-sphere/0.4").is_ok());
    assert!(parse("nope").is_err());
    assert!(parse("sphere/").is_err());
    assert!(parse("box-torus-sphere-sphere").is_err());
    assert!(parse("box-").is_err());
    let c = parse("sphere/9").unwrap();
    assert!(c.p[0][0] == 1.0);
    let m = parse("menger/2.6").unwrap();
    assert!(m.p[0][0] == 3.0);
}

#[test]
fn sdf_distance_semantics() {
    let ball = parse("sphere/0.5").unwrap();
    assert!(sd_chain(&ball, Vec3::new(0.0, 0.0, 0.0)) < 0.0);
    assert!(sd_chain(&ball, Vec3::splat(2.0)) > 2.9);
    let bagel = parse("torus/0.5/0.22-cylinder/0.15/1.0").unwrap();
    assert!(sd_chain(&bagel, Vec3::new(0.0, 0.0, 0.0)) > 0.0);
    assert!(sd_chain(&bagel, Vec3::new(0.5, 0.0, 0.0)) < 0.0);
    let sponge = parse("menger/2").unwrap();
    assert!(sd_chain(&sponge, Vec3::new(0.0, 0.0, 0.5)) > 0.0);
    assert!(sd_chain(&sponge, Vec3::new(0.95, 0.95, 0.95)) < 0.0);
    let wax = parse("blob/7").unwrap();
    let sd: f32 = 7.0;
    let bb = blob_ball(sd.to_bits(), 0);
    assert!(sd_chain(&wax, bb.0) < 0.0);
    let cut = parse("box/1/1/1-sphere/0.8@0.1").unwrap();
    assert!(sd_chain(&cut, Vec3::new(0.0, 0.0, 0.0)) > 0.0);
    assert!(sd_chain(&cut, Vec3::new(0.95, 0.95, 0.0)) < 0.0);
    let shape = parse("bulb/8").unwrap();
    assert!(sd_chain(&shape, Vec3::splat(0.9)) > 0.0);
}

#[test]
fn sdf_march_finds_the_surface() {
    let ball = parse("sphere/0.5").unwrap();
    let t = march(
        &ball,
        Vec3::new(-3.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        1000.0,
    );
    assert!(t > 2.4 && t < 2.51);
    assert!(
        march(
            &ball,
            Vec3::new(0.0, 3.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            1000.0
        ) < 0.0
    );
    let sponge = parse("menger/3").unwrap();
    let ts = march(
        &sponge,
        Vec3::new(0.98, 1.6, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        10.0,
    );
    assert!(ts > 0.5 && ts < 0.75);
}

#[test]
fn sdf_table_matches_the_record() {
    let c = parse("torus").unwrap();
    let t = table(&c);
    assert!(t[0] == KIND_TORUS as f32);
    assert!(t[1] == 0.72);
    assert!(t[2] == 0.28);
    assert!(t[8] == c.safety);
    assert!(t[9] == BUDGET_PRIM);
    assert!(t[12] == -1.0);
}
