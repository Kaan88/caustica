"""Benchmarks a fixed set and appends the means to captures/perf-history.csv, keyed by commit,
then plots the history to captures/perf-history.png. Run it after each change that may cost time:

    python tools/perf_history.py [--exe PATH] [--frames 200]

Set: presets 0 (Classic), 13 (bunny), 17 (mirror corridor), 21 (neon) and the Sponza mesh scene
(textured, alpha-tested foliage; needs tools/fetch_assets.ps1) at 1440 x 900, 4 spp, no VSync,
each with DLSS Ray Reconstruction and with the native denoiser. The "preset" column holds the
preset number or the scene name.
"""
import argparse
import csv
import datetime
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CSV = ROOT / "captures" / "perf-history.csv"
SETS = {"0": ["--preset", "0"], "13": ["--preset", "13"], "17": ["--preset", "17"], "21": ["--preset", "21"],
        "sponza": ["--scene", "sponza"]}
MODES = {"rr": ["--rr"], "native": ["--no-rr"]}


def run(exe, preset, mode, frames):
    r = subprocess.run([exe, "--headless", "--frames", str(frames), "--no-vsync", "--width", "1440", "--height", "900",
                        "--spp", "4", *SETS[preset], *MODES[mode]], capture_output=True, text=True, cwd=ROOT)
    m = re.search(r"gpu_ms_mean=([\d.]+) frame_ms_mean=([\d.]+)", r.stdout)
    if not m:
        raise SystemExit(f"preset {preset} {mode} failed: {r.stdout}{r.stderr}")
    return float(m.group(1)), float(m.group(2))


def plot():
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    rows = list(csv.DictReader(CSV.open(encoding="utf-8")))
    commits = list(dict.fromkeys(r["commit"] for r in rows))
    fig, ax = plt.subplots(figsize=(10, 5))
    for preset in SETS:
        for mode, style in (("rr", "-"), ("native", "--")):
            pts = [(commits.index(r["commit"]), float(r["gpu_ms_mean"])) for r in rows
                   if r["preset"] == preset and r["mode"] == mode]
            if pts:
                name = preset if not preset.isdigit() else f"preset {preset}"
                ax.plot(*zip(*pts), style, marker="o", label=f"{name} {mode}")
    ax.set_xticks(range(len(commits)), commits, rotation=45, ha="right", fontsize=8)
    ax.set_ylabel("GPU ms per frame (mean)")
    ax.set_title("Caustica performance history (1440x900, 4 spp)")
    ax.grid(alpha=0.3)
    ax.legend(fontsize=8, ncol=2)
    fig.tight_layout()
    fig.savefig(CSV.with_suffix(".png"), dpi=110)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "target" / "release" / "caustica.exe"))
    ap.add_argument("--frames", type=int, default=200)
    a = ap.parse_args()
    commit = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(ROOT), "status", "--porcelain", "--", "src", "shaders", "bridge"],
                           capture_output=True, text=True).stdout.strip()
    if dirty:
        commit += "+"
    CSV.parent.mkdir(parents=True, exist_ok=True)
    new = not CSV.exists()
    with CSV.open("a", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        if new:
            w.writerow(["date", "commit", "preset", "mode", "gpu_ms_mean", "frame_ms_mean"])
        for preset in SETS:
            for mode in MODES:
                gpu, frame = run(a.exe, preset, mode, a.frames)
                print(f"{preset:>6} {mode:6} gpu {gpu:6.2f} ms  frame {frame:6.2f} ms")
                w.writerow([datetime.date.today().isoformat(), commit, preset, mode, gpu, frame])
    plot()
    print(f"history: {CSV}  plot: {CSV.with_suffix('.png')}")


if __name__ == "__main__":
    main()
