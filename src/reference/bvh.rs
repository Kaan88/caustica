//! Geometry: a binned-SAH BVH over the scene's triangles.
use glam::{Vec2, Vec3};

pub(super) struct Tri {
    pub(super) p: [Vec3; 3],
    pub(super) n: [Vec3; 3],
    /// Texture coordinates: a mesh's own where its material maps them (flag 16), else zero.
    pub(super) uv: [Vec2; 3],
    pub(super) material: u32,
}
impl Tri {
    /// The interpolated normal at barycentrics (u, v) and the face normal turned to its side
    /// (winding doesn't decide). Either stands in for the other when it degenerates.
    pub(super) fn normals(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let t = self;
        let n = (t.n[0] * (1. - u - v) + t.n[1] * u + t.n[2] * v).normalize_or_zero();
        let face = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize_or_zero();
        let n = if n == Vec3::ZERO { face } else { n };
        let ng = if face == Vec3::ZERO { n } else { face };
        (n, if ng.dot(n) < 0. { -ng } else { ng })
    }
}
#[derive(Clone, Copy)]
pub(super) struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle and count (count > 0); inner: the right child's index (left = +1).
    start: u32,
    count: u32,
}
pub(super) struct Bvh {
    nodes: Vec<Node>,
    pub(super) tris: Vec<Tri>,
}
#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub(super) t: f32,
    /// The triangle (unused for a procedural hit).
    pub(super) tri: usize,
    pub(super) b: Vec2,
    pub(super) material: u32,
    /// The procedural object (an index into `Scene::sdfs`) when the hit is one.
    pub(super) sdf: Option<usize>,
}
impl Bvh {
    pub(super) fn build(tris: Vec<Tri>) -> Self {
        let mut bvh = Self {
            nodes: vec![],
            tris: vec![],
        };
        let centers: Vec<Vec3> = tris
            .iter()
            .map(|t| (t.p[0] + t.p[1] + t.p[2]) / 3.)
            .collect();
        let mut order: Vec<usize> = (0..tris.len()).collect();
        if !order.is_empty() {
            bvh.split(&tris, &centers, &mut order[..], 0, 0);
        }
        let mut slots: Vec<Option<Tri>> = tris.into_iter().map(Some).collect();
        bvh.tris = order.iter().map(|i| slots[*i].take().unwrap()).collect();
        bvh
    }
    fn bounds(tris: &[Tri], idx: &[usize]) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in idx {
            for p in tris[*i].p {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        (lo, hi)
    }
    fn split(
        &mut self,
        tris: &[Tri],
        centers: &[Vec3],
        idx: &mut [usize],
        start: usize,
        depth: usize,
    ) -> usize {
        let (lo, hi) = Self::bounds(tris, idx);
        let me = self.nodes.len();
        self.nodes.push(Node {
            min: lo,
            max: hi,
            start: start as u32,
            count: idx.len() as u32,
        });
        if idx.len() <= 4 {
            return me;
        }
        let area = |lo: Vec3, hi: Vec3| {
            let e = (hi - lo).max(Vec3::ZERO);
            e.x * e.y + e.y * e.z + e.z * e.x
        };
        // Binned SAH over the centroids' extent, 16 bins per axis.
        const BINS: usize = 16;
        let (clo, chi) = idx.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(a, b), i| (a.min(centers[*i]), b.max(centers[*i])),
        );
        let mut best = (f32::MAX, 0usize, 0f32);
        for axis in 0..3 {
            let extent = chi[axis] - clo[axis];
            if extent <= 1e-9 {
                continue;
            }
            let mut bins = [(0usize, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); BINS];
            for i in idx.iter() {
                let b = (((centers[*i][axis] - clo[axis]) / extent * BINS as f32) as usize)
                    .min(BINS - 1);
                let e = &mut bins[b];
                e.0 += 1;
                for p in tris[*i].p {
                    e.1 = e.1.min(p);
                    e.2 = e.2.max(p);
                }
            }
            for cut in 1..BINS {
                let merge = |r: &[(usize, Vec3, Vec3)]| {
                    r.iter()
                        .fold((0, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |a, b| {
                            (a.0 + b.0, a.1.min(b.1), a.2.max(b.2))
                        })
                };
                let (l, r) = (merge(&bins[..cut]), merge(&bins[cut..]));
                if l.0 == 0 || r.0 == 0 {
                    continue;
                }
                let cost = l.0 as f32 * area(l.1, l.2) + r.0 as f32 * area(r.1, r.2);
                if cost < best.0 {
                    best = (cost, axis, clo[axis] + extent * cut as f32 / BINS as f32);
                }
            }
        }
        // Past depth 40 the list is halved (no SAH), so the tree stays inside the traversal stack.
        let mid = if best.0 < f32::MAX && depth < 40 {
            let (axis, at) = (best.1, best.2);
            let mut i = 0;
            for j in 0..idx.len() {
                if centers[idx[j]][axis] < at {
                    idx.swap(i, j);
                    i += 1;
                }
            }
            i
        } else {
            0
        };
        // No useful cut (all centroids together): split in the middle of the list.
        let mid = if mid == 0 || mid == idx.len() {
            idx.len() / 2
        } else {
            mid
        };
        let (left, right) = idx.split_at_mut(mid);
        self.nodes[me].count = 0;
        self.split(tris, centers, left, start, depth + 1);
        let r = self.split(tris, centers, right, start + mid, depth + 1);
        self.nodes[me].start = r as u32;
        me
    }
    fn slab(n: &Node, o: Vec3, inv: Vec3, tmax: f32) -> Option<f32> {
        let t0 = (n.min - o) * inv;
        let t1 = (n.max - o) * inv;
        let near = t0.min(t1).max_element().max(0.);
        let far = t0.max(t1).min_element().min(tmax);
        (near <= far).then_some(near)
    }
    /// Closest accepted hit in (1e-4, tmax): Möller–Trumbore, both sides. `accept` sees each
    /// candidate in range as (material, triangle index, barycentrics), as an any-hit shader would.
    pub(super) fn intersect(
        &self,
        o: Vec3,
        d: Vec3,
        tmax: f32,
        accept: &dyn Fn(u32, usize, Vec2) -> bool,
        any: bool,
    ) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = d.recip();
        let mut best: Option<Hit> = None;
        let mut limit = tmax;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if Self::slab(node, o, inv, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for i in node.start as usize..(node.start + node.count) as usize {
                    let t = &self.tris[i];
                    let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
                    let pv = d.cross(e2);
                    let det = e1.dot(pv);
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    let id = 1. / det;
                    let tv = o - t.p[0];
                    let u = tv.dot(pv) * id;
                    if !(0. ..=1.).contains(&u) {
                        continue;
                    }
                    let qv = tv.cross(e1);
                    let v = d.dot(qv) * id;
                    if v < 0. || u + v > 1. {
                        continue;
                    }
                    let tt = e2.dot(qv) * id;
                    if tt > 1e-4 && tt < limit && accept(t.material, i, Vec2::new(u, v)) {
                        limit = tt;
                        best = Some(Hit {
                            t: tt,
                            tri: i,
                            b: Vec2::new(u, v),
                            material: t.material,
                            sdf: None,
                        });
                        if any {
                            return best;
                        }
                    }
                }
            } else {
                let here = stack[top] as usize;
                let (l, r) = (here + 1, node.start as usize);
                // Nearer child last, so it pops first.
                let (dl, dr) = (
                    Self::slab(&self.nodes[l], o, inv, limit),
                    Self::slab(&self.nodes[r], o, inv, limit),
                );
                let mut push = |c: usize| {
                    if top < stack.len() {
                        stack[top] = c as u32;
                        top += 1;
                    }
                };
                match (dl, dr) {
                    (Some(a), Some(b)) if a < b => {
                        push(r);
                        push(l);
                    }
                    (Some(_), Some(_)) => {
                        push(l);
                        push(r);
                    }
                    (Some(_), None) => push(l),
                    (None, Some(_)) => push(r),
                    _ => {}
                }
            }
        }
        best
    }
}
