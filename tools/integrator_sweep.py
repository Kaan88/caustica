"""Renders every scene file with path tracing, BDPT and photon mapping and lists the scenes
where the integrators' mean colours disagree (a quick way to find what BDPT or photon
mapping get wrong). Usage: python tools/integrator_sweep.py EXE OUTDIR [frames]

All three render with a 1-pixel box filter (BDPT and photon mapping used only that until
2026-11-05; they now take the path tracer's filter too): the path tracer's default Blackman-Harris weights don't sum to exactly 1 over the pixel grid, which moved
image means at small bright lights and suns by up to 10% (the old dusk "gap"; 2026-11-04,
tools/oracle.py). Means are over the whole image."""
import glob, os, subprocess, sys
import numpy as np

sys.path.insert(0, os.path.dirname(__file__))
import exr  # noqa: E402

exe, out = sys.argv[1], sys.argv[2]
frames = sys.argv[3] if len(sys.argv) > 3 else "100"
os.makedirs(out, exist_ok=True)
rows = []
for scene in sorted(glob.glob("scenes/builtin/*.json")):
    if scene.endswith("pack.json"):
        continue
    name = os.path.splitext(os.path.basename(scene))[0]
    images = []
    for i in range(3):
        path = os.path.join(out, f"{name}_{i}.exr")
        r = subprocess.run([exe, "--headless", "--frames", frames, "--width", "240", "--height", "135",
                            "--scene", scene, "--no-rr", "--raw", "--set", f"integrator={i}",
                            "--set", "pixel_filter=0", "--set", "filter_width=1",
                            "--set", "auto_exposure=false", "--set", "eyes=false",
                            "--capture", path], capture_output=True, text=True)
        if "RENDER_OK" not in r.stdout:
            images.append(None)
            continue
        images.append(exr.read(path)[..., :3])
    means = [None if im is None else im.reshape(-1, 3).mean(axis=0) for im in images]
    pt = means[0]
    for i, label in ((1, "bdpt"), (2, "photon")):
        m = means[i]
        if pt is None or m is None:
            rows.append((9.99, name, label, "render failed"))
            continue
        rel = abs(m - pt).max() / max(pt.max(), 1e-6)
        rows.append((rel, name, label, f"pt {pt.round(4)} {label} {m.round(4)}"))
rows.sort(reverse=True)
for rel, name, label, info in rows:
    if rel > 0.05:
        print(f"{rel*100:6.1f}%  {name:32s} {label:7s} {info}")
print(f"{sum(r[0] > 0.05 for r in rows)} of {len(rows)} comparisons differ by more than 5%")
