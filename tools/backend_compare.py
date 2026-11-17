"""D3D12 against Vulkan from one exe: one table of image difference and timings per scene
(roadmap "D3D12 phase-out", phase 0).

    python tools/backend_compare.py [--exe PATH] [--scenes a,b,...] [--size 960x600]
                                    [--frames 256] [--spp 4] [--rr] [--out DIR]

Each scene renders on both renderers with the same switches: raw accumulation (no denoiser,
scale 1), `--no-vsync`, the display pinned (auto exposure and the Eyes look off) and the clock
frozen at t = 0 (`--time 0`, so physics and animated scenes match). The EXR captures are
compared with the oracle's statistical test (tools/oracle.py: the image means within --z
standard errors or --floor, and no 8 x 8 block off), plus the RMSE x1000 of compressed
luminance l/(1+l) between the two. Timings are the RENDER_OK means (frames 10 on), so --frames
should stay >= 200 for them to mean anything. `backend=` on RENDER_OK confirms which renderer
ran; a run that reports the wrong one fails.

--rr adds a second run per backend with DLSS Ray Reconstruction at the scene's own settings, for
its timings only (RR output is not compared: its noise is not independent between runs). It uses
--require-rr, so an RR that fails to start shows as nan instead of the native denoiser's time.

Default list: perf-history presets 0, 13, 17 and 21, Sponza, stylised-forest, glass-of-water,
fairy-lights, one BDPT scene (veach-ajar), one photon-mapping scene (cornell-caustic with
integrator 2), one spectral scene (blackbody-forge) and one HDRI scene (hdri-noon-spheres).
Sponza and the HDRI scene need tools/fetch_assets.ps1. Exits 1 if any scene fails.
"""
import argparse
import os
import re
import subprocess
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exr  # noqa: E402
from oracle import LUM, test  # noqa: E402

BACKENDS = ('d3d12', 'vulkan')
# name: extra switches (a scene name, or a preset, plus anything that picks the integrator)
DEFAULT = {
    'preset:0': [],
    'preset:13': [],
    'preset:17': [],
    'preset:21': [],
    'sponza': [],
    'stylised-forest': [],
    'glass-of-water': [],
    'fairy-lights': [],
    'veach-ajar': [],                                  # BDPT (the scene's own integrator 1)
    'cornell-caustic': ['--set', 'integrator=2'],      # photon mapping
    'blackbody-forge': [],                             # spectral path tracing
    'hdri-noon-spheres': [],                           # HDRI environment
}
FIELDS = ('gpu_ms_mean', 'trace_ms_mean', 'reconstruct_ms_mean', 'frame_ms_mean')


def scene_args(name):
    return ['--preset', name[7:]] if name.startswith('preset:') else ['--scene', name]


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True,
                       creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    return r.returncode, r.stdout + r.stderr


def timings(out, backend):
    line = next((l for l in out.splitlines() if l.startswith('RENDER_OK')), None)
    if line is None:
        return None, 'no RENDER_OK: ' + (out.strip().splitlines() or ['(no output)'])[-1][:120]
    ran = re.search(r'backend=(\w+)', line)
    if not ran or ran.group(1) != backend:
        return None, f'asked for {backend}, RENDER_OK says {ran.group(1) if ran else "nothing"}'
    return {k: float(m.group(1)) for k in FIELDS if (m := re.search(k + r'=([\d.]+)', line))}, ''


def compressed(a):
    lum = a[..., :3] @ LUM
    return lum / (1 + lum)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--exe', default=os.path.join('target', 'validation', 'release', 'caustica.exe'))
    p.add_argument('--scenes', default=','.join(DEFAULT), help='comma-separated; preset:N or a scene name')
    p.add_argument('--size', default='960x600')
    p.add_argument('--frames', type=int, default=256)
    p.add_argument('--spp', type=int, default=4)
    p.add_argument('--rr', action='store_true', help='also time each backend with Ray Reconstruction')
    p.add_argument('--z', type=float, default=4.0)
    p.add_argument('--floor', type=float, default=0.002)
    p.add_argument('--out', default=os.path.join('captures', 'backend-compare'))
    a = p.parse_args()
    # A relative path fails in subprocess on Windows unless the cwd is the repo root.
    a.exe = os.path.abspath(a.exe)
    w, h = a.size.split('x')
    os.makedirs(a.out, exist_ok=True)
    common = ['--headless', '--no-vsync', '--width', w, '--height', h, '--time', '0',
              '--set', 'auto_exposure=false', '--set', 'eyes=false']
    header = f'{"scene":20} {"D3D12/VK":>8} {"z":>6} {"worst blk":>9} {"rmse":>6}  ' + \
             '  '.join(f'{b + " gpu/trace":>17}' for b in BACKENDS)
    if a.rr:
        header += '  ' + '  '.join(f'{b + " RR gpu":>12}' for b in BACKENDS)
    print(header)
    failed = 0
    for name in [s for s in a.scenes.split(',') if s]:
        extra = DEFAULT.get(name, [])
        stem = name.replace(':', '-')
        images, times, why = {}, {}, ''
        for b in BACKENDS:
            path = os.path.join(a.out, f'{stem}.{b}.exr')
            code, out = run([a.exe, f'--{b}', *scene_args(name), *common, *extra, '--no-rr', '--raw',
                             '--scale', '1', '--spp', str(a.spp), '--samples', str(a.frames * a.spp),
                             '--frames', str(a.frames), '--capture', path])
            t, err = timings(out, b) if code == 0 else (None, f'exit {code}: ' + (out.strip().splitlines() or [''])[-1][:110])
            if t is None:
                why = f'{b}: {err}'
                break
            times[b] = t
            images[b] = exr.read(path)
            if a.rr:
                code, out = run([a.exe, f'--{b}', *scene_args(name), *common, *extra, '--require-rr',
                                 '--frames', str(a.frames), '--capture', os.path.join(a.out, f'{stem}.{b}.rr.png')])
                t, err = timings(out, b) if code == 0 else (None, f'exit {code}')
                times[b]['rr_gpu_ms_mean'] = t['gpu_ms_mean'] if t else float('nan')
        if why:
            print(f'{name:20} FAILED  {why}')
            failed += 1
            continue
        g, v = images['d3d12'], images['vulkan']
        if g.shape != v.shape:
            print(f'{name:20} FAILED  image sizes differ: {g.shape} against {v.shape}')
            failed += 1
            continue
        ratio, z, (bz, br, _at), bad = test(g[..., :3], v[..., :3], 8, a.z, a.floor)
        rmse = np.sqrt(((compressed(g) - compressed(v)) ** 2).mean()) * 1000
        failed += bad
        row = f'{name:20} {ratio:8.4f} {z:6.2f} {br:9.4f} {rmse:6.2f}  ' + '  '.join(
            f'{times[b]["gpu_ms_mean"]:8.2f}/{times[b].get("trace_ms_mean", float("nan")):<8.2f}' for b in BACKENDS)
        if a.rr:
            row += '  ' + '  '.join(f'{times[b]["rr_gpu_ms_mean"]:12.2f}' for b in BACKENDS)
        print(row + ('  FAIL' if bad else '  ok'))
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
