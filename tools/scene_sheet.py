"""Renders every scene file headless and tiles them into one labelled contact sheet.

    python tools/scene_sheet.py [--exe PATH] [--out DIR] [--only SLUG ...] [--frames N]

The sheet is a quick visual check after writing scenes, and a before/after diff for renderer
changes. Each tile is a PNG capture at 480 x 300 (RR on, as the app starts).
"""
import argparse
import json
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "target" / "release" / "caustica.exe"))
    ap.add_argument("--out", default=str(ROOT / "captures" / "scenes"))
    ap.add_argument("--only", nargs="*")
    ap.add_argument("--frames", type=int, default=48)
    ap.add_argument("--pack", default="builtin")
    ap.add_argument("--width", type=int, default=480)
    ap.add_argument("--height", type=int, default=300)
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    pack = json.loads((ROOT / "scenes" / a.pack / "pack.json").read_text(encoding="utf-8"))
    slugs = [Path(s).stem for s in pack["scenes"]]
    if a.only:
        slugs = [s for s in slugs if s in a.only]
    tiles = []
    for slug in slugs:
        png = out / f"{slug}.png"
        r = subprocess.run([a.exe, "--headless", "--frames", str(a.frames), "--width", str(a.width), "--height", str(a.height),
                            "--scene", slug, "--capture", str(png)], capture_output=True, text=True, cwd=ROOT)
        ok = "RENDER_OK" in r.stdout
        print(f"{slug:28} {'ok' if ok else 'FAILED ' + (r.stderr.strip().splitlines() or [''])[-1]}")
        title = json.loads((ROOT / "scenes" / a.pack / f"{slug}.json").read_text(encoding="utf-8"))["title"]
        tiles.append((title, png if ok else None))
    cols = 6
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * a.width, rows * (a.height + 18)), (20, 20, 22))
    draw = ImageDraw.Draw(sheet)
    for i, (title, png) in enumerate(tiles):
        x, y = (i % cols) * a.width, (i // cols) * (a.height + 18)
        if png:
            sheet.paste(Image.open(png).convert("RGB"), (x, y + 18))
        draw.text((x + 4, y + 3), f"{i + 1}. {title}", fill=(230, 230, 230))
    path = out / f"sheet-{a.pack}.png"
    sheet.save(path)
    print(f"sheet: {path}")


if __name__ == "__main__":
    main()
