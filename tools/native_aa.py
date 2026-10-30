"""Edge anti-aliasing of the native (non-RR) path: raw samples and the native denoiser.

Renders at 640x480 with RR off, 4 spp per frame, and scores the last frame against a
16,384-spp white-noise reference with the same pixel filter:

- still: one frame (4 spp) on a still camera
- orbit: the last frame of --orbit-test (still for 64 frames, then 0.25 degrees per frame)

Two scenes: "edges" (rr_stability.py's unlit scene: emissive walls, black blocks, so the only
noise is where a pixel's samples land: pure anti-aliasing error) and "classic" (preset 0, lit).
Scores are RMSE x1000 of compressed luminance l/(1+l) over all pixels and over edge pixels
(reference gradient > 0.05), plus the edge error after a 3x3 blur (stair-steps survive it,
noise mostly does not).

Usage, from the repo root:
    python tools/native_aa.py --exe target/validation/debug/cornell-rtx.exe --tag new --filter bh
    python tools/native_aa.py --exe target/baseline/debug/cornell-rtx.exe --tag main --filter none
--filter none passes no pixel_filter (builds before the setting, which are box filtered);
box / gauss / bh set it. --width sets the filter width. References are rendered once per
scene, filter and width, and reused. Edge crops go to <out>/<tag>_<scene>_<case>.png.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import exr  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
FILTERS = {"none": None, "box": 0, "gauss": 1, "bh": 2}
BLACK = {"color": [0, 0, 0], "reflectivity": 0, "unlit": True}
EDGES = {
    "bounces": 1, "light_on": False,
    "walls": {"roughness": 1, "reflectivity": 0, "emission": 1, "unlit": True},
    "floor": {"roughness": 1, "reflectivity": 0, "emission": 1, "unlit": True},
    "objects": [
        {"name": "Tall block", "material": BLACK},
        {"name": "Short block", "position": [0.38, -0.38], "size": 0.62,
         "stretch": 0.62, "rotation": -14, "material": BLACK},
    ],
}


def render(exe, out, scene, extra):
    command = [str(exe), "--headless", "--width", "640", "--height", "480", "--no-rr", *scene, *extra,
               "--capture", str(out)]
    subprocess.run(command, cwd=ROOT, check=True, stdout=subprocess.DEVNULL,
                   creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))


def compressed(a):
    lum = a @ np.array([0.2126, 0.7152, 0.0722], np.float32)
    return lum / (1 + lum)


def blur3(x):
    p = np.pad(x, 1, mode="edge")
    return sum(p[i:i + x.shape[0], j:j + x.shape[1]] for i in range(3) for j in range(3)) / 9


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/release/cornell-rtx.exe")
    parser.add_argument("--out", type=Path, default=ROOT / "captures/native-aa")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--filter", choices=FILTERS, default="none")
    parser.add_argument("--width", type=float, default=1.5)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    exe = args.exe.resolve()
    edges = out / "edges.json"
    edges.write_text(json.dumps(EDGES), encoding="utf-8")
    scenes = {"edges": ["--settings", str(edges)], "classic": ["--preset", "0"]}
    kind = FILTERS[args.filter]
    pixel = [] if kind is None else ["--set", f"pixel_filter={kind}", "--set", f"filter_width={args.width}"]
    ref_name = "box1" if kind in (None, 0) and (kind is None or args.width == 1) else f"{args.filter}{args.width:g}"
    results = {}
    for scene, flags in scenes.items():
        for case, yaw, frames in [("still", 0, ["--frames", "1"]),
                                  ("orbit", 14.75, ["--frames", "124", "--orbit-test"])]:
            reference = out / f"ref_{scene}_{case}_{ref_name}.exr"
            if not reference.exists():
                render(exe, reference, flags, ["--frames", "1024", "--raw", "--spp", "16", "--samples", "16384",
                                               "--set", "sobol=false", "--set", f"yaw={yaw}", *pixel])
            ref = compressed(exr.read(reference))
            gy, gx = np.gradient(ref)
            mask = np.hypot(gx, gy) > 0.05
            for config, extra in [("raw", ["--raw"]), ("denoised", [])]:
                path = out / f"{args.tag}_{scene}_{case}_{config}.exr"
                render(exe, path, flags, [*frames, "--spp", "4", *extra, *pixel])
                image = exr.read(path)
                e = compressed(image) - ref
                b = blur3(e)
                row = {
                    "all": float(np.sqrt((e ** 2).mean()) * 1000),
                    "edges": float(np.sqrt((e[mask] ** 2).mean()) * 1000),
                    "edges_blurred": float(np.sqrt((b[mask] ** 2).mean()) * 1000),
                }
                results[f"{scene} {case} {config}"] = row
                print(f"{args.tag:10} {scene:8} {case:6} {config:9} all {row['all']:6.2f}  "
                      f"edges {row['edges']:6.2f}  edges 3x3 {row['edges_blurred']:6.2f}", flush=True)
                crop = image[200:330, 300:470]
                tile = Image.fromarray(np.uint8(np.clip((crop / (1 + crop)) ** (1 / 2.2), 0, 1) * 255))
                tile.resize((crop.shape[1] * 4, crop.shape[0] * 4), Image.NEAREST).save(
                    out / f"{args.tag}_{scene}_{case}_{config}.png")
    (out / f"metrics_{args.tag}.json").write_text(json.dumps(results, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
