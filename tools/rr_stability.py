"""Image-based RR regression: a stationary, noise-free scene must stay still.

Optional test dependencies: python -m pip install numpy Pillow
Run from any directory: python native/tools/rr_stability.py
"""
import argparse
import json
from pathlib import Path
import subprocess
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]


def analyze(directory):
    paths = sorted(directory.glob("*.png"))
    if len(paths) != 32:
        raise AssertionError(f"Expected 32 frames in {directory}, got {len(paths)}")
    rgb = np.stack([np.asarray(Image.open(p).convert("RGB"), dtype=np.float32) / 255 for p in paths])
    gray = rgb.mean(axis=-1)
    edges = np.hypot(*np.gradient(gray.mean(axis=0))) > 0.05
    if edges.sum() < 1000:
        raise AssertionError("Test scene did not produce enough high-contrast edges")
    return {
        "edge_pixels": int(edges.sum()),
        "edge_temporal_std": float(gray.std(axis=0)[edges].mean()),
        "mean_frame_difference": float(np.abs(np.diff(gray, axis=0)).mean()),
    }, rgb.mean(axis=0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/release/caustica.exe")
    parser.add_argument("--output", type=Path, default=ROOT / "captures/rr-stability/regression")
    parser.add_argument("--before", type=Path, help="Optional existing baseline sequence for comparison")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # Every surface is unlit, so paths end at the first hit: emissive walls
    # and black blocks with no light sampling and no Monte Carlo noise.
    # Remaining frame-to-frame movement is a reconstruction error.
    black = {"color": [0, 0, 0], "reflectivity": 0, "unlit": True}
    scene = {
        "bounces": 1, "spp": 1, "scale": 0.5, "light_on": False,
        "walls": {"roughness": 1, "reflectivity": 0, "emission": 1, "unlit": True},
        "floor": {"roughness": 1, "reflectivity": 0, "emission": 1, "unlit": True},
        # Same placement as the default tall and short blocks.
        "objects": [
            {"name": "Tall block", "material": black},
            {"name": "Short block", "position": [0.38, -0.38], "size": 0.62,
             "stretch": 0.62, "rotation": -14, "material": black},
        ],
    }
    settings = output / "edges.json"
    settings.write_text(json.dumps(scene), encoding="utf-8")
    sequence = output / "stationary"
    command = [str(args.exe.resolve()), "--headless", "--frames", "96",
               "--width", "640", "--height", "480", "--require-rr", "--debug",
               "--settings", str(settings), "--capture-sequence", str(sequence)]
    subprocess.run(command, cwd=ROOT, check=True, creationflags=subprocess.CREATE_NO_WINDOW)
    metrics, mean = analyze(sequence)
    Image.fromarray(np.uint8(np.clip(mean, 0, 1) * 255)).save(output / "mean.png")
    report = {"after": metrics}
    if args.before:
        baseline, _ = analyze(args.before.resolve())
        report["before"] = baseline
        report["edge_shimmer_reduction_percent"] = 100 * (1 - metrics["edge_temporal_std"] / baseline["edge_temporal_std"])
    (output / "metrics.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
    assert metrics["edge_temporal_std"] < 0.02, "RR edges shimmer: check jitter sign/scale and history"
    assert metrics["mean_frame_difference"] < 0.0015, "Stationary RR output is unstable"
    print("PASS: stationary RR edges remain stable at 50% input resolution")


if __name__ == "__main__":
    main()
