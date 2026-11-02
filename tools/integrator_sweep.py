"""Renders every scene file with path tracing, BDPT and photon mapping and lists the scenes
where the integrators' mean colours disagree (a quick way to find what BDPT or photon
mapping get wrong). Usage: python tools/integrator_sweep.py EXE OUTDIR [frames]"""
import glob, os, subprocess, sys

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
    means = []
    for i in range(3):
        path = os.path.join(out, f"{name}_{i}.exr")
        r = subprocess.run([exe, "--headless", "--frames", frames, "--width", "240", "--height", "135",
                            "--scene", scene, "--no-rr", "--raw", "--set", f"integrator={i}",
                            "--capture", path], capture_output=True, text=True)
        if "RENDER_OK" not in r.stdout:
            means.append(None)
            continue
        img = exr.read(path)[..., :3]
        means.append(img.mean(axis=(0, 1)))
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
