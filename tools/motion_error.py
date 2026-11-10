"""Motion-error benchmark for the native filter (Classic preset, --orbit-test).

Renders the last frame of a 124-frame orbit (the camera stays still for 64 frames, then
turns 0.25 degrees per frame and ends at yaw 14.75; 2 spp per frame, native path) for:
raw samples, spatial filter only, and filter + temporal history. Each is scored against a
16,384-spp white-noise reference at the final camera:

- RMSE x1000 of compressed luminance l/(1+l)
- the same after a 5x5 blur, which exposes bias, over-blur and ghosting
- the mean blurred error in a band around the ceiling light

Usage, from native/:
    python tools/motion_error.py --exe target/validation/debug/caustica.exe --tag mytest [--tag older]
The reference is rendered once and reused. Light-region crops go to <out>/crop_<tags>.png.

--static 4,16,64,256 instead scores a still camera (yaw 0) at those sample counts, raw against
denoised, to check that the filter converges to the reference instead of adding bias.
"""
import argparse
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
import exr  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CONFIGS = {"raw": ["--raw"], "filter": ["--set", "temporal=false"], "temporal": []}
BAND = (slice(60, 140), slice(250, 550))  # ceiling around the light at 800x600


def render(exe, out, extra):
    command = [str(exe), "--headless", "--width", "800", "--height", "600", "--no-rr", *extra,
               "--set", "auto_exposure=false", "--set", "eyes=false", "--capture", str(out)]
    subprocess.run(command, cwd=ROOT, check=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))


def compressed(a):
    lum = a @ np.array([0.2126, 0.7152, 0.0722], np.float32)
    return lum / (1 + lum)


def blur(x):
    p = np.pad(x, 2, mode="edge")
    return sum(p[i:i + x.shape[0], j:j + x.shape[1]] for i in range(5) for j in range(5)) / 25


def score(image, r):
    e = compressed(image) - r
    return np.sqrt((e ** 2).mean()) * 1000, np.sqrt((blur(e) ** 2).mean()) * 1000


def still(exe, out, tag, counts):
    reference = out / "ref_still.exr"
    if not reference.exists():
        render(exe, reference, ["--frames", "1024", "--raw", "--spp", "16", "--samples", "16384", "--set", "sobol=false"])
    r = compressed(exr.read(reference))
    for spp in counts:
        line = f"{tag:12} {spp:5} spp"
        for config, extra in [("raw", ["--raw"]), ("denoised", [])]:
            path = out / f"{tag}_still{spp}_{config}.exr"
            render(exe, path, ["--frames", str(max(1, spp // 4)), "--spp", str(min(spp, 4)), *extra])
            rmse, blurred = score(exr.read(path), r)
            line += f"  {config} {rmse:6.2f} / {blurred:5.2f}"
        print(line)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/release/caustica.exe")
    parser.add_argument("--out", type=Path, default=ROOT / "captures/motion")
    parser.add_argument("--static", help="Comma-separated sample counts for the still-camera check")
    parser.add_argument("--tag", action="append", required=True,
                        help="Name for this run. Repeat to also score earlier runs already in --out; the first tag is rendered.")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.static:
        still(args.exe.resolve(), out, args.tag[0], [int(v) for v in args.static.split(",")])
        return
    reference = out / "ref.exr"
    if not reference.exists():
        render(args.exe.resolve(), reference, ["--frames", "1024", "--raw", "--spp", "16", "--samples", "16384",
                                               "--set", "sobol=false", "--set", "yaw=14.75"])
    for config, extra in CONFIGS.items():
        render(args.exe.resolve(), out / f"{args.tag[0]}_{config}.exr",
               ["--frames", "124", "--orbit-test", "--spp", "2", *extra])
    ref = exr.read(reference)
    r = compressed(ref)
    crops = [("reference", ref)]
    for tag in args.tag:
        for config in CONFIGS:
            path = out / f"{tag}_{config}.exr"
            if not path.exists():
                continue
            image = exr.read(path)
            e = compressed(image) - r
            b = blur(e)
            print(f"{tag:12} {config:9} RMSE {np.sqrt((e ** 2).mean()) * 1000:6.2f}  "
                  f"blurred {np.sqrt((b ** 2).mean()) * 1000:6.2f}  light band {np.abs(b[BAND]).mean() * 1000:6.2f}")
            crops.append((f"{tag} {config}", image))
    tiles = []
    for name, image in crops:
        a = image[40:160, 200:600]
        tile = Image.fromarray(np.uint8(np.clip((a / (1 + a)) ** (1 / 2.2), 0, 1) * 255)).resize((800, 240), Image.NEAREST)
        ImageDraw.Draw(tile).text((4, 4), name, fill=(255, 255, 0))
        tiles.append(tile)
    sheet = Image.new("RGB", (1600, 240 * ((len(tiles) + 1) // 2)))
    for i, tile in enumerate(tiles):
        sheet.paste(tile, ((i % 2) * 800, (i // 2) * 240))
    sheet.save(out / f"crop_{'_'.join(args.tag)}.png")


if __name__ == "__main__":
    main()
