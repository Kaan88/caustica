"""Reduce the raw night-sky data to the small files the engine ships (scenes/sky/).

    python tools/prep_sky_data.py [--raw scenes/sky/raw] [--out scenes/sky]

Inputs (gitignored, in scenes/sky/raw/, see docs/engine-notes.md "Night sky data"):
  hygdata_v42.csv.gz     HYG database v4.2, David Nash, astronexus.com/hyg (CC BY-SA 4.0)
  milkyway_2020_8k.exr   NASA SVS Deep Star Maps 2020, svs.gsfc.nasa.gov/4851, 8192x4096 half RGB

Outputs:
  stars.bin     "CSTR", u32 version (1), u32 count, then per star (12 bytes, little-endian):
                ra_rad f32, dec_rad f32 (J2000), V f16, B-V f16. Stars with V <= 9.0 (Sol
                excluded), sorted by V ascending. A missing B-V is stored as 0.65.
  milkyway.exr  2048x1024 half RGB, ZIP, linear, area-downsampled from the SVS map, with the
                low 5 half mantissa bits rounded off so the sensor noise compresses (about 6 MB).

Milky Way map orientation (equirectangular, celestial coordinates, verified against the
globular clusters Omega Cen, 47 Tuc, NGC 2808 and NGC 362 in the map to 0.03 deg):
  row y (0 at the top):  Dec = 90 deg - (y + 0.5) / H * 180 deg
  column x (0 at left):  RA  = (0.5 - (x + 0.5) / W) * 360 deg, wrapped to [0, 360)
  so RA 0 is the image centre (the seam between columns W/2-1 and W/2) and RA increases
  to the LEFT. The galactic centre (RA 17h45m, Dec -29) lands at about 75% of the width.
Pixel values are the SVS display-referred 0..1 range, not calibrated radiance: the sky bake
scales them (N4).

The SVS Milky Way layer ships without the catalogue stars (stars are a separate SVS layer),
which this script checks by stacking the map at HYG star positions against random positions.
If a future map did hold them, stars with V <= 9 are replaced by the local median.
"""
import argparse
import csv
import gzip
import os
import struct
import sys
import zlib

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exr  # noqa: E402

V_LIMIT = 9.0
DEFAULT_BV = 0.65
OUT_W, OUT_H = 2048, 1024
DROP_BITS = 5  # half mantissa bits rounded off: 0.6% mean error, ZIP shrinks 9.2 MB to 5.7 MB
COMMENT = ('Caustica night sky: equirectangular celestial coordinates; image centre is RA 0, '
           'RA increases to the left, row 0 is Dec +90. NASA SVS Deep Star Maps 2020, '
           'star-free, area-downsampled.')


def load_hyg(path):
    stars = []
    with gzip.open(path, 'rt', newline='') as f:
        for r in csv.DictReader(f):
            if r['id'] == '0' or not r['mag']:
                continue
            v = float(r['mag'])
            if v > V_LIMIT:
                continue
            ci = float(r['ci']) if r['ci'] else DEFAULT_BV
            stars.append((float(r['rarad']), float(r['decrad']), v, ci, r['proper']))
    stars.sort(key=lambda s: s[2])
    return stars


def write_stars(path, stars):
    with open(path, 'wb') as f:
        f.write(b'CSTR' + struct.pack('<II', 1, len(stars)))
        for ra, dec, v, ci, _ in stars:
            f.write(struct.pack('<ff', ra, dec) + np.float16(v).tobytes() + np.float16(ci).tobytes())


def read_stars(path):
    d = open(path, 'rb').read()
    assert d[:4] == b'CSTR'
    ver, n = struct.unpack_from('<II', d, 4)
    assert ver == 1 and len(d) == 12 + 12 * n
    a = np.frombuffer(d, np.dtype([('ra', '<f4'), ('dec', '<f4'), ('v', '<f2'), ('bv', '<f2')]), n, 12)
    return a


def pixel_of(ra, dec, w, h):
    """Pixel centre coordinates (x, y) of a direction, per the orientation in the docstring."""
    return ((0.5 - ra / (2 * np.pi)) * w - 0.5) % w, (0.5 - dec / np.pi) * h - 0.5


def solid_angle_weights(h):
    dec = (0.5 - (np.arange(h) + 0.5) / h) * np.pi
    return np.cos(dec)[:, None]


def star_excess(lum, ra, dec):
    h, w = lum.shape
    x, y = pixel_of(ra, dec, w, h)
    xi, yi = np.rint(x).astype(int) % w, np.clip(np.rint(y).astype(int), 0, h - 1)
    out = np.empty(len(xi), np.float32)
    for i, (a, b) in enumerate(zip(xi, yi)):
        pk = lum[max(b - 1, 0):b + 2][:, np.arange(a - 1, a + 2) % w].max()
        bg = np.median(lum[max(b - 6, 0):b + 7][:, np.arange(a - 6, a + 7) % w])
        out[i] = pk - bg
    return out


def remove_stars(img, stars):
    """Local-median inpaint of a 5x5 patch at each star (only used if the map holds stars)."""
    h, w, _ = img.shape
    x, y = pixel_of(np.array([s[0] for s in stars]), np.array([s[1] for s in stars]), w, h)
    for a, b in zip(np.rint(x).astype(int) % w, np.clip(np.rint(y).astype(int), 0, h - 1)):
        ys = slice(max(b - 2, 0), b + 3)
        xs = np.arange(a - 2, a + 3) % w
        yb = slice(max(b - 8, 0), b + 9)
        xb = np.arange(a - 8, a + 9) % w
        med = np.median(img[yb][:, xb].reshape(-1, 3), axis=0)
        for xx in xs:
            img[ys, xx] = med
    return img


def write_exr(path, rgb):
    """Half-float scanline EXR, ZIP (16 lines per block), channels B G R."""
    h, w, _ = rgb.shape
    half = rgb.astype(np.float16)

    def attr(name, typ, data):
        return name.encode() + b'\0' + typ.encode() + b'\0' + struct.pack('<i', len(data)) + data

    chl = b''.join(c.encode() + b'\0' + struct.pack('<iBBBBii', 1, 0, 0, 0, 0, 1, 1) for c in 'BGR') + b'\0'
    hdr = struct.pack('<II', 20000630, 2)
    hdr += attr('channels', 'chlist', chl)
    hdr += attr('comments', 'string', COMMENT.encode())
    hdr += attr('compression', 'compression', b'\x03')
    box = struct.pack('<4i', 0, 0, w - 1, h - 1)
    hdr += attr('dataWindow', 'box2i', box) + attr('displayWindow', 'box2i', box)
    hdr += attr('lineOrder', 'lineOrder', b'\0') + attr('pixelAspectRatio', 'float', struct.pack('<f', 1))
    hdr += attr('screenWindowCenter', 'v2f', struct.pack('<2f', 0, 0))
    hdr += attr('screenWindowWidth', 'float', struct.pack('<f', 1)) + b'\0'
    chunks = []
    for y0 in range(0, h, 16):
        rows = half[y0:y0 + 16]  # (r, w, 3 as R G B)
        raw = np.stack([rows[..., 2], rows[..., 1], rows[..., 0]], axis=1)  # (r, ch B G R, w)
        s = np.ascontiguousarray(raw).astype('<f2').tobytes()
        b = np.frombuffer(s, np.uint8)
        re = np.concatenate([b[0::2], b[1::2]]).astype(np.int16)
        t = np.concatenate([re[:1], (np.diff(re) + 128) & 0xff]).astype(np.uint8)
        z = zlib.compress(t.tobytes(), 6)
        if len(z) >= len(s):
            z = s
        chunks.append(struct.pack('<ii', y0, len(z)) + z)
    pos = len(hdr) + 8 * len(chunks)
    table = b''
    for c in chunks:
        table += struct.pack('<Q', pos)
        pos += len(c)
    with open(path, 'wb') as f:
        f.write(hdr + table + b''.join(chunks))


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'scenes', 'sky')
    ap = argparse.ArgumentParser()
    ap.add_argument('--raw', default=os.path.join(root, 'raw'))
    ap.add_argument('--out', default=root)
    args = ap.parse_args()

    # ---- stars
    stars = load_hyg(os.path.join(args.raw, 'hygdata_v42.csv.gz'))
    out_stars = os.path.join(args.out, 'stars.bin')
    write_stars(out_stars, stars)
    a = read_stars(out_stars)
    print(f'stars.bin: {len(a)} stars, {os.path.getsize(out_stars)} bytes')
    for lo in range(-2, 9):
        n = int(((a['v'] >= lo) & (a['v'] < lo + 1)).sum()) if lo < 8 else int((a['v'] >= 8).sum())
        print(f'  V {lo:>2} .. {lo + 1:<2}: {n}')
    names = {}
    for ra, dec, v, ci, name in stars:
        if name:
            names[name] = (ra, dec, v, ci)
    for nm in ('Sirius', 'Vega', 'Polaris', 'Betelgeuse'):
        ra, dec, v, ci = names[nm]
        i = [k for k, s in enumerate(stars) if s[4] == nm][0]
        print(f'  {nm}: csv V {v} B-V {ci} ra {np.degrees(ra):.4f} dec {np.degrees(dec):.4f}'
              f' | bin V {float(a["v"][i]):.3f} B-V {float(a["bv"][i]):.3f}'
              f' ra {np.degrees(a["ra"][i]):.4f} dec {np.degrees(a["dec"][i]):.4f}')

    # ---- Milky Way
    ch = exr.read_channels(os.path.join(args.raw, 'milkyway_2020_8k.exr'))
    img = np.stack([ch['R'], ch['G'], ch['B']], -1).astype(np.float32)
    h, w, _ = img.shape
    assert w == 2 * h and w % OUT_W == 0
    lum = img.mean(-1)
    rng = np.random.default_rng(1)
    st = np.array([(s[0], s[1]) for s in stars if s[2] <= 6.0])
    pick = st[rng.choice(len(st), min(len(st), 600), replace=False)]
    e_star = star_excess(lum, pick[:, 0], pick[:, 1]).mean()
    e_rand = star_excess(lum, rng.uniform(0, 2 * np.pi, len(pick)),
                         np.arcsin(rng.uniform(-0.9, 0.9, len(pick)))).mean()
    has_stars = e_star - e_rand > 0.02
    print(f'point-source excess at V<=6 star positions {e_star:.4f}, at random positions {e_rand:.4f}:'
          f' catalogue stars {"present, removing V<=9" if has_stars else "absent, nothing to remove"}')
    if has_stars:
        img = remove_stars(img, stars)

    wt = solid_angle_weights(h)
    f_before = float((img * wt[..., None]).sum((0, 1)).sum() / (3 * wt.sum() * w))
    k = w // OUT_W
    small = img.reshape(OUT_H, k, OUT_W, k, 3).mean((1, 3))
    wt2 = solid_angle_weights(OUT_H)
    f_after = float((small * wt2[..., None]).sum((0, 1)).sum() / (3 * wt2.sum() * OUT_W))
    out_mw = os.path.join(args.out, 'milkyway.exr')
    # Non-negative first: the rounding below works on the bit pattern, where a sign bit would
    # clip to the largest half.
    hb = np.clip(small, 0.0, 65504.0).astype(np.float16).view(np.uint16).astype(np.uint32)
    hb = (((hb + (1 << (DROP_BITS - 1))) >> DROP_BITS) << DROP_BITS).clip(0, 0x7bff).astype(np.uint16)
    small = hb.view(np.float16).astype(np.float32)
    write_exr(out_mw, small)
    back = np.stack([exr.read_channels(out_mw)[c] for c in 'RGB'], -1)
    err = np.abs(back - small.astype(np.float16)).max()
    print(f'milkyway.exr: {OUT_W}x{OUT_H}, {os.path.getsize(out_mw)} bytes, roundtrip max error {err}, {DROP_BITS} mantissa bits rounded off')
    print(f'mean pixel value per solid angle: 8k {f_before:.6f} -> 2k {f_after:.6f} '
          f'(half-float storage error {abs(float((back * wt2[..., None]).sum()) / (3 * wt2.sum() * OUT_W) - f_after):.2e})')


if __name__ == '__main__':
    main()
