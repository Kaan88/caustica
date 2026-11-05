"""GPU-vs-CPU oracle: renders scenes on the GPU (raw accumulation) and with the CPU reference
(reference.rs, an independent path tracer of the same model) and tests that they agree.

    python tools/oracle.py [--exe PATH] [--scenes a,b,...] [--integrator 0|1|2]
                           [--size 320x240] [--gpu-spp 1024] [--cpu-spp 512] [--out DIR]

The test is statistical. d = GPU - CPU luminance per pixel; the scene structure cancels and
what is left is the noise of both renders, so the standard error of the image mean of d is
std(d) / sqrt(pixels) (pixels are independent samples). A scene fails when its mean differs by
more than --z standard errors AND by more than --floor relative (so a long render doesn't fail
on a 0.05% float difference), or when one block of an 8 x 8 grid does (a localized bias that
the whole-image mean hides). Prints one line per scene; exits 1 if any failed.

Scenes the CPU reference refuses (reference::unsupported) are reported as SKIP; the CPU side
always path-traces (a scene file's integrator is overridden), and --rgb turns spectral off on both. Default list:
the scenes measured when the reference was written.
"""
import argparse
import os
import subprocess
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exr  # noqa: E402

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)
DEFAULT = ('preset:0,veach-mis,mirror-room,white-furnace,nighthawks,gem-display,aquarium,'
           'shape-material-matrix')


def scene_args(name):
    return ['--preset', name[7:]] if name.startswith('preset:') else ['--scene', name]


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def test(g, c, blocks, z_max, floor):
    gl, cl = g @ LUM, c @ LUM
    d = gl - cl
    mean = cl.mean()
    se = d.std() / np.sqrt(d.size)
    z = d.mean() / max(se, 1e-12)
    rel = d.mean() / max(mean, 1e-12)
    worst = (0.0, 1.0, None)
    h, w = d.shape
    for by in range(blocks):
        for bx in range(blocks):
            ys = slice(by * h // blocks, (by + 1) * h // blocks)
            xs = slice(bx * w // blocks, (bx + 1) * w // blocks)
            db, cb = d[ys, xs], cl[ys, xs]
            if cb.mean() <= 1e-4 * max(mean, 1e-12):
                continue
            bz = db.mean() / max(db.std() / np.sqrt(db.size), 1e-12)
            br = 1 + db.mean() / cb.mean()
            if abs(bz) > abs(worst[0]):
                worst = (bz, br, (by, bx))
    bad = abs(z) > z_max and abs(rel) > floor
    bad_block = abs(worst[0]) > z_max + 1 and abs(worst[1] - 1) > 10 * floor
    return 1 + rel, z, worst, bad or bad_block


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--exe', default=os.path.join('target', 'validation', 'release', 'caustica.exe'))
    p.add_argument('--scenes', default=DEFAULT)
    p.add_argument('--integrator', type=int, default=0)
    p.add_argument('--rgb', action='store_true', help='render spectral scenes in RGB on both sides')
    p.add_argument('--set', action='append', default=[], help='key=json for both sides (repeatable)')
    p.add_argument('--size', default='320x240')
    p.add_argument('--gpu-spp', type=int, default=1024)
    p.add_argument('--cpu-spp', type=int, default=512)
    p.add_argument('--z', type=float, default=4.0)
    p.add_argument('--floor', type=float, default=0.002)
    p.add_argument('--out', default=os.path.join('captures', 'oracle'))
    a = p.parse_args()
    w, h = a.size.split('x')
    os.makedirs(a.out, exist_ok=True)
    size = ['--width', w, '--height', h]
    if a.rgb:
        size += ['--set', 'spectral=false']
    # BDPT and photon mapping used a box pixel filter until 2026-11-05 (they now take the path
    # tracer's); a wider filter moves the image mean at sharp bright edges (its shifts don't sum
    # to 1 over the grid), so they are still compared with a 1-pixel box on both sides.
    if a.integrator != 0:
        size += ['--set', 'pixel_filter=0', '--set', 'filter_width=1']
    for kv in a.set:
        size += ['--set', kv]
    failed = 0
    print(f'{"scene":28} {"GPU/CPU":>8} {"z":>7}  worst block (ratio, z, row/col)')
    for name in [s for s in a.scenes.split(',') if s]:
        stem = name.replace(':', '-')
        cpu = os.path.join(a.out, stem + '.cpu.exr')
        gpu = os.path.join(a.out, stem + f'.gpu{a.integrator}.exr')
        # The CPU reference is a path tracer; it is the judge for every GPU integrator.
        code, out = run([a.exe, *scene_args(name), *size,
                         '--set', 'integrator=0', '--reference', str(a.cpu_spp), '--capture', cpu])
        if code != 0:
            why = out.strip().splitlines()[-1] if out.strip() else f'exit {code}'
            print(f'{name:28} SKIP  {why[:110]}')
            continue
        frames = max(1, a.gpu_spp // 4)
        code, out = run([a.exe, '--headless', *scene_args(name), *size, '--no-rr', '--raw',
                         '--scale', '1', '--spp', '4', '--samples', str(frames * 4),
                         '--frames', str(frames), '--set', 'firefly=0',
                         '--set', f'integrator={a.integrator}', '--capture', gpu])
        if code != 0:
            print(f'{name:28} GPU FAILED  {out.strip()[-200:]}')
            failed += 1
            continue
        ratio, z, (bz, br, at), bad = test(exr.read(gpu), exr.read(cpu), 8, a.z, a.floor)
        failed += bad
        print(f'{name:28} {ratio:8.4f} {z:7.2f}  {br:.4f} {bz:6.2f} {at}  {"FAIL" if bad else "ok"}')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
