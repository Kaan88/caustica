"""Upscaler comparison on the Classic preset at 1280x720 output, render scale 0.5.

For each upscaler (and native 100% and DLSS RR as references) it renders a still camera
(160 frames) and the last frame of a 124-frame orbit (2 spp per frame, see motion_error.py),
then scores the linear HDR output against a 16,384-spp full-resolution reference:

- RMSE x1000 of compressed luminance l/(1+l)
- the same RMSE without a 4-pixel rim around the light (luminance > 2). DLSS and XeSS blend
  such extreme HDR edges differently from a linear box filter; that rim (under 1% of the
  image) otherwise dominates their score
- RMSE after a 5x5 blur (bias, smearing)
- edge sharpness: gradient energy relative to the reference (1.0 = as sharp, <1 blurry)

Usage, from native/:  python tools/upscale_quality.py --exe target/validation/release/cornell-rtx.exe
"""
import argparse
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import exr  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CASES = {
    "native 100% (no upscale)": ["--no-rr", "--scale", "1"],
    "bilinear": ["--no-rr", "--set", 'upscaler="bilinear"'],
    "bicubic": ["--no-rr", "--set", 'upscaler="bicubic"'],
    "DLSS SR": ["--no-rr", "--set", 'upscaler="dlss"'],
    "FSR": ["--no-rr", "--set", 'upscaler="fsr"'],
    "XeSS": ["--no-rr", "--set", 'upscaler="xess"'],
    "DLSS RR": ["--rr"],
}


def render(exe, out, extra):
    command = [str(exe), "--preset", "0", "--headless", "--width", "1280", "--height", "720", "--scale", "0.5", *extra, "--capture", str(out)]
    subprocess.run(command, cwd=ROOT, check=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))


def compressed(a):
    lum = a @ np.array([0.2126, 0.7152, 0.0722], np.float32)
    return lum / (1 + lum)


def blur(x):
    p = np.pad(x, 2, mode="edge")
    return sum(p[i:i + x.shape[0], j:j + x.shape[1]] for i in range(5) for j in range(5)) / 25


def gradient_energy(x):
    return np.abs(np.diff(x, axis=0)).mean() + np.abs(np.diff(x, axis=1)).mean()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/release/cornell-rtx.exe")
    parser.add_argument("--out", type=Path, default=ROOT / "captures/upscale")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    exe = args.exe.resolve()
    references = {}
    for name, yaw in [("still", "0"), ("orbit", "14.75")]:
        path = out / f"ref_{name}.exr"
        if not path.exists():
            subprocess.run([str(exe), "--preset", "0", "--headless", "--width", "1280", "--height", "720", "--scale", "1", "--no-rr", "--raw",
                            "--frames", "1024", "--spp", "16", "--samples", "16384", "--set", "sobol=false", "--set", f"yaw={yaw}",
                            "--capture", str(path)], cwd=ROOT, check=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        reference = exr.read(path)
        rim = reference @ np.array([0.2126, 0.7152, 0.0722], np.float32) > 2
        for _ in range(4):
            rim = rim | np.roll(rim, 1, 0) | np.roll(rim, -1, 0) | np.roll(rim, 1, 1) | np.roll(rim, -1, 1)
        references[name] = (compressed(reference), ~rim)
    print(f"{'':26} {'still: RMSE / no light rim / blurred / sharpness':50} orbit: same")
    for case, extra in CASES.items():
        line = f"{case:26}"
        for name, motion in [("still", ["--frames", "160"]), ("orbit", ["--frames", "124", "--orbit-test", "--spp", "2"])]:
            slug = case.split(" ")[0].lower() + ("_rr" if case == "DLSS RR" else "")
            path = out / f"{name}_{slug}.exr"
            render(exe, path, [*extra, *motion])
            r, keep = references[name]
            c = compressed(exr.read(path))
            e = c - r
            line += (f" {np.sqrt((e ** 2).mean()) * 1000:7.2f} / {np.sqrt((e[keep] ** 2).mean()) * 1000:6.2f} / "
                     f"{np.sqrt((blur(e) ** 2).mean()) * 1000:6.2f} / {gradient_energy(c) / gradient_energy(r):5.2f}         ")
        print(line)


if __name__ == "__main__":
    main()
