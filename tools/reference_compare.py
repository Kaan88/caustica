"""Compare a GPU render with the CPU reference (src/reference/) of the same scene.

    python tools/reference_compare.py gpu.exr cpu.exr [--blocks N] [--heat out.png]

Both are linear EXRs of the same size: the GPU's raw accumulation (`--raw --no-rr --capture`)
and `--reference SPP --capture`. Prints the mean of each channel and their ratio, the relative
RMSE, and the worst blocks of an N x N grid by luminance ratio (a localized bias shows there
when the whole-image means agree). Both images carry noise, so a ratio is only meaningful
within a few of its standard errors (printed per block from the CPU image's spread).
"""
import argparse
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exr  # noqa: E402

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('gpu')
    p.add_argument('cpu')
    p.add_argument('--blocks', type=int, default=8)
    p.add_argument('--heat', help='PNG of the per-pixel log2 luminance ratio (red: GPU brighter)')
    a = p.parse_args()
    g, c = exr.read(a.gpu), exr.read(a.cpu)
    assert g.shape == c.shape, f'sizes differ: {g.shape} vs {c.shape}'
    gm, cm = g.reshape(-1, 3).mean(0), c.reshape(-1, 3).mean(0)
    print('mean GPU', np.round(gm, 5), 'CPU', np.round(cm, 5))
    print('ratio GPU/CPU per channel', np.round(gm / np.maximum(cm, 1e-12), 4))
    gl, cl = g @ LUM, c @ LUM
    rmse = np.sqrt(((gl - cl) ** 2).mean()) / max(cl.mean(), 1e-12)
    print(f'relative RMSE (luminance) {rmse:.4f}')
    h, w = gl.shape
    n = a.blocks
    rows = []
    for by in range(n):
        for bx in range(n):
            ys, xs = slice(by * h // n, (by + 1) * h // n), slice(bx * w // n, (bx + 1) * w // n)
            gb, cb = gl[ys, xs], cl[ys, xs]
            if cb.mean() <= 1e-6:
                continue
            # Standard error of the block mean of the CPU image, from its pixel spread.
            se = cb.std() / np.sqrt(cb.size) / cb.mean()
            rows.append((gb.mean() / cb.mean(), se, by, bx, cb.mean()))
    rows.sort(key=lambda r: -abs(np.log(r[0])))
    print(f'worst {min(6, len(rows))} of {len(rows)} lit blocks ({n}x{n} grid): ratio, block row/col, relative spread')
    for r, se, by, bx, m in rows[:6]:
        print(f'  {r:.4f}  ({by},{bx})  spread {se:.4f}  mean {m:.4g}')
    if a.heat:
        from PIL import Image
        ratio = np.log2(np.maximum(gl, 1e-6) / np.maximum(cl, 1e-6))
        t = np.clip(ratio / 0.5, -1, 1)
        img = np.stack([np.maximum(t, 0), 1 - np.abs(t), np.maximum(-t, 0)], -1)
        Image.fromarray((img * 255).astype(np.uint8)).save(a.heat)


if __name__ == '__main__':
    main()
