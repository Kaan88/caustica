"""GPU-vs-CPU oracle: renders scenes on the GPU (raw accumulation) and with the CPU reference
(src/reference/, an independent path tracer of the same model) and tests that they agree.

    python tools/oracle.py [--exe PATH] [--scenes a,b,...] [--integrator 0|1|2]
                           [--cpu-integrator 0|1|2|same] [--keep-filter] [--time T]
                           [--size 320x240] [--gpu-spp 1024] [--cpu-spp 512] [--out DIR]
                           [--blur-frame N [--fps 24]]

The test is statistical. d = GPU - CPU luminance per pixel; the scene structure cancels and
what is left is the noise of both renders, so the standard error of the image mean of d is
std(d) / sqrt(pixels) (pixels are independent samples). A scene fails when its mean differs by
more than --z standard errors AND by more than --floor relative (so a long render doesn't fail
on a 0.05% float difference), or when one block of an 8 x 8 grid does (a localized bias that
the whole-image mean hides). Prints one line per scene; exits 1 if any failed.

With --time both sides render the scene as it stands at T seconds, frozen (reference::at_time:
tracks, the per-frame script, physics and cloth in fixed steps, the rotating camera). Without it
the GPU's physics and rotation run on the wall clock while the CPU's stay at t = 0, so those
scenes need --time.

With --blur-frame N both sides render frame N of a timeline export, motion blur included: the
GPU exports every frame of the timeline as EXR (`--turntable-timeline`, each pass one moment in
the shutter) and frame N is taken; the CPU renders `--reference-time N/fps` (reference::exposure,
the shutter's moments). --fps must be the scene's timeline rate.

Scenes the CPU reference refuses (reference::unsupported) are reported as SKIP; the CPU side
path-traces unless --cpu-integrator says otherwise (1: BDPT; 2: photon mapping, which takes the
GPU's --spp as its frame, so its bias matches only with --cpu-spp equal to --gpu-spp; same: the
GPU's --integrator; a scene file's integrator is overridden either way), and --rgb turns spectral off on both. With
--integrator other than 0 both sides take a 1-pixel box filter unless --keep-filter. Default list:
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
    p.add_argument('--cpu-integrator', default='0', help="the CPU's integrator: 0, 1, 2 or 'same'")
    p.add_argument('--keep-filter', action='store_true',
                   help="keep the scene's pixel filter for integrators other than 0")
    p.add_argument('--rgb', action='store_true', help='render spectral scenes in RGB on both sides')
    p.add_argument('--set', action='append', default=[], help='key=json for both sides (repeatable)')
    p.add_argument('--time', help='seconds on the timeline, frozen, for both sides')
    p.add_argument('--walk', action='store_true',
                   help='both sides start in walk mode: the standing walk avatar is drawn (put the '
                        'camera 0.62 above the floor with --set target/yaw/pitch/distance)')
    p.add_argument('--blur-frame', type=int, help='frame of a timeline export, motion-blurred')
    p.add_argument('--fps', type=int, default=24, help="the scene's timeline rate (--blur-frame)")
    p.add_argument('--size', default='320x240')
    p.add_argument('--gpu-spp', type=int, default=1024)
    p.add_argument('--cpu-spp', type=int, default=512)
    p.add_argument('--firefly', type=float, default=0.,
                   help='firefly clamp on both sides; biased by sample count, so the GPU then '
                        'renders one sample a frame and --gpu-spp should equal --cpu-spp')
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
    if a.integrator != 0 and not a.keep_filter:
        size += ['--set', 'pixel_filter=0', '--set', 'filter_width=1']
    for kv in a.set:
        size += ['--set', kv]
    if a.time is not None:
        size += ['--time', a.time]
    if a.walk:
        size += ['--walk']
    cpu_integrator = a.integrator if a.cpu_integrator == 'same' else int(a.cpu_integrator)
    failed = 0
    print(f'{"scene":28} {"GPU/CPU":>8} {"z":>7}  worst block (ratio, z, row/col)')
    for name in [s for s in a.scenes.split(',') if s]:
        stem = name.replace(':', '-')
        cpu = os.path.join(a.out, stem + f'.cpu{cpu_integrator}.exr')
        gpu = os.path.join(a.out, stem + f'.gpu{a.integrator}.exr')
        # The CPU reference path-traces by default, the judge for every GPU integrator.
        when = [] if a.blur_frame is None else ['--reference-time', str(a.blur_frame / a.fps)]
        per = 1 if a.firefly > 0 else 4
        # Photon mapping's bias depends on the frame (one photon map per --spp samples, a radius
        # shrinking per frame), so the CPU takes the GPU's frames: same --spp, and the same
        # frame count needs --cpu-spp equal to --gpu-spp.
        frames_cpu = ['--spp', str(per)] if cpu_integrator == 2 else []
        code, out = run([a.exe, *scene_args(name), *size, *when, *frames_cpu,
                         '--set', f'integrator={cpu_integrator}', '--set', f'firefly={a.firefly}', '--reference', str(a.cpu_spp), '--capture', cpu])
        if code != 0:
            why = out.strip().splitlines()[-1] if out.strip() else f'exit {code}'
            print(f'{name:28} SKIP  {why[:110]}')
            continue
        frames = max(1, a.gpu_spp // per)
        if a.blur_frame is not None:
            # No dot: the export would take what follows it for an extension.
            folder = os.path.join(a.out, stem + '-frames')
            os.makedirs(folder, exist_ok=True)
            code, out = run([a.exe, '--headless', *scene_args(name), *size, '--no-rr', '--raw',
                             '--scale', '1', '--spp', str(per), '--set', f'firefly={a.firefly}',
                             '--set', f'integrator={a.integrator}', '--turntable', folder,
                             '--turntable-timeline', '--turntable-format', 'exr',
                             '--turntable-spp', str(frames * per)])
            frame = os.path.join(folder, f'{a.blur_frame + 1:05}.exr')
            if code == 0 and os.path.exists(frame):
                os.replace(frame, gpu)
            elif code == 0:
                code, out = 1, 'no frame ' + frame
        else:
            code, out = run([a.exe, '--headless', *scene_args(name), *size, '--no-rr', '--raw',
                             '--scale', '1', '--spp', str(per), '--samples', str(frames * per),
                             '--frames', str(frames), '--set', f'firefly={a.firefly}',
                             '--set', f'integrator={a.integrator}', '--capture', gpu])
        if code != 0:
            print(f'{name:28} GPU FAILED  {out.strip()[-200:]}')
            failed += 1
            continue
        # The GPU's capture clamps each pixel's mean at 0 (post.hlsl `beauty`); a spectral mean
        # can be negative (hero wavelengths outside the gamut), so the CPU's is clamped the same
        # way to compare like with like. RGB means are never negative.
        c = exr.read(cpu)
        c[..., :3] = np.maximum(c[..., :3], 0)
        ratio, z, (bz, br, at), bad = test(exr.read(gpu), c, 8, a.z, a.floor)
        failed += bad
        print(f'{name:28} {ratio:8.4f} {z:7.2f}  {br:.4f} {bz:6.2f} {at}  {"FAIL" if bad else "ok"}')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
