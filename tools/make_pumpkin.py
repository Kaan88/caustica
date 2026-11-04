"""Writes scenes/builtin/jack-o-lantern.obj: a carved pumpkin with real holes.

A ribbed shell with thickness on an (azimuth, polar) grid. The carving (two triangle eyes, a
nose, a grin with three teeth, the lid's zigzag cut) is a signed distance `carve` over the
grid's parameters, negative inside a hole. Each grid triangle is clipped to `carve > 0` with
linear interpolation, on the outer and the inner surface alike, and each clipped edge gets a
wall joining the two, so the cuts are smooth, the shell stays closed and light leaves only
through the holes. Walls have their own vertices, so the importer's smooth normals keep the cut
edges crisp.

Run: python tools/make_pumpkin.py
"""

import math
import pathlib

NA, NP = 240, 120  # azimuth and polar cells (1.5 degrees each)
THICK = 0.075  # shell thickness, in units of the radius


def radius(theta, phi):
    """Outer radius: ten ribs, with dimples at the stem and the base."""
    rib = 1 - 0.07 * (1 - math.cos(10 * phi)) / 2
    dimple = 1 - 0.16 * math.exp(-((theta / 0.3) ** 2)) - 0.12 * math.exp(-(((math.pi - theta) / 0.3) ** 2))
    return rib * dimple


def point(theta, phi, inner):
    r = radius(theta, phi) - (THICK if inner else 0)
    s = math.sin(theta)
    return (r * s * math.cos(phi), 0.8 * r * math.cos(theta), r * s * math.sin(phi))


def triangle(p, a, b, c):
    """Max of the three edges' signed distances: negative inside, exact near the edges."""
    pts = (a, b, c)
    area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    d = -1e9
    for k in range(3):
        u, w = pts[k], pts[(k + 1) % 3]
        ex, ey = w[0] - u[0], w[1] - u[1]
        n = math.hypot(ex, ey)
        # Outward normal for a counter-clockwise triangle.
        s = ((p[0] - u[0]) * ey - (p[1] - u[1]) * ex) / n
        d = max(d, s if area > 0 else -s)
    return d


def carve(theta, phi):
    """The face looks along +z (azimuth 90°): x = azimuth offset, y = polar offset (down > 0)."""
    x = math.atan2(math.sin(phi - math.pi / 2), math.cos(phi - math.pi / 2))
    y = theta - math.pi / 2
    p = (x, y)
    d = 1e9
    # Eyes: triangles with the point up, the outer corner a little lower.
    for s in (-1, 1):
        d = min(d, triangle(p, (s * 0.12, -0.08), (s * 0.50, -0.05), (s * 0.30, -0.42)))
    d = min(d, triangle(p, (-0.10, 0.12), (0.10, 0.12), (0.0, -0.06)))  # nose
    # Grin: between two curves, tapering to points at the corners; three teeth stay.
    top = 0.18 + 0.30 * x * x
    bottom = 0.46 + 0.10 * x * x - 0.22 * (x / 0.66) ** 4
    grin = max(top - y, y - bottom, abs(x) - 0.66)
    for tx, up in ((-0.24, True), (0.24, True), (0.0, False)):
        side = abs(x - tx) - 0.055
        tooth = max(side, (bottom - 0.11) - y) if up else max(side, y - (top + 0.10))
        grin = max(grin, -tooth)
    d = min(d, grin)
    # The lid: a thin zigzag cut around the stem, the lid left in place.
    zig = 0.40 + 0.04 * abs(((phi * 12 / math.pi) % 2) - 1)
    d = min(d, abs(theta - zig) - 0.016)
    return d


def main():
    verts, faces = [], []
    shared = {}

    def add(p):
        verts.append(p)
        return len(verts) - 1

    def at(t, inner):
        """A vertex at grid parameters t = (j, i) (fractional on clipped edges)."""
        key = (round(t[0], 6), round(t[1] % NA, 6), inner)
        if key not in shared:
            shared[key] = add(point(math.pi * t[0] / NP, 2 * math.pi * t[1] / NA, inner))
        return shared[key]

    value = [[carve(math.pi * j / NP, 2 * math.pi * i / NA) for i in range(NA + 1)] for j in range(NP + 1)]

    def cut(a, b):
        """Where carve = 0 between grid points a and b, interpolated in a fixed order so both
        triangles sharing the edge get the same point."""
        if a > b:
            a, b = b, a
        va, vb = value[a[0]][a[1]], value[b[0]][b[1]]
        f = va / (va - vb)
        return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f)

    def emit(poly):
        # Outer surface faces out; the inner one is the same polygon reversed.
        o = [at(t, False) for t in poly]
        n = [at(t, True) for t in poly]
        for k in range(1, len(poly) - 1):
            faces.append((o[0], o[k], o[k + 1]))
            faces.append((n[0], n[k + 1], n[k]))

    def wall(t0, t1, kept):
        # A quad from the outer to the inner surface along the cut, facing away from the kept
        # side (into the hole).
        a, b = point(*ang(t0), False), point(*ang(t1), False)
        c, d = point(*ang(t1), True), point(*ang(t0), True)
        k = point(*ang(kept), False)
        e1 = [b[m] - a[m] for m in range(3)]
        e2 = [d[m] - a[m] for m in range(3)]
        n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
        q = [add(p) for p in (a, b, c, d)]
        if sum(n[m] * (k[m] - a[m]) for m in range(3)) > 0:
            q.reverse()
        faces.append((q[0], q[1], q[2]))
        faces.append((q[0], q[2], q[3]))

    def ang(t):
        return (math.pi * t[0] / NP, 2 * math.pi * t[1] / NA)

    def clip(tri):
        """Clips a grid triangle (counter-clockwise in (j, i)) to carve > 0."""
        inside = [value[t[0]][t[1]] > 0 for t in tri]
        if all(inside):
            emit(tri)
            return
        if not any(inside):
            return
        poly, edge = [], []
        for k in range(3):
            a, b = tri[k], tri[(k + 1) % 3]
            if inside[k]:
                poly.append(a)
            if inside[k] != inside[(k + 1) % 3]:
                p = cut(a, b)
                poly.append(p)
                edge.append(p)
        emit(poly)
        kept = [t for t, ins in zip(tri, inside) if ins][0]
        wall(edge[0], edge[1], kept)

    for j in range(NP):
        for i in range(NA):
            a, b, c, d = (j, i), (j, i + 1), (j + 1, i + 1), (j + 1, i)
            clip((a, d, c))
            clip((a, c, b))
    path = pathlib.Path(__file__).resolve().parent.parent / "scenes" / "builtin" / "jack-o-lantern.obj"
    with open(path, "w", encoding="ascii", newline="\n") as f:
        f.write("# Carved pumpkin, generated by tools/make_pumpkin.py\n")
        for v in verts:
            f.write("v %.4f %.4f %.4f\n" % v)
        for a, b, c in faces:
            f.write("f %d %d %d\n" % (a + 1, b + 1, c + 1))
    print(f"wrote {path}: {len(verts)} vertices, {len(faces)} triangles")


main()
