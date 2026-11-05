"""Writes the .cube 3D LUTs shipped in scenes/luts (Look -> Film -> LUT).

The post pass applies a LUT to display values encoded with gamma 2.2 (post.hlsl film_look), so
these map encoded 0..1 RGB to encoded 0..1 RGB, red fastest as the .cube format wants.

warm-film: a print-film look. A gentle S curve on luma (deeper shadows, rolled highlights), teal
in the shadows and amber in the highlights (split toning), saturation eased off in both ends
and slightly raised in the midtones, and a lifted black so nothing goes fully to zero.
"""
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "scenes" / "luts"
N = 33


def smoothstep(a, b, x):
    t = min(max((x - a) / (b - a), 0.0), 1.0)
    return t * t * (3 - 2 * t)


def warm_film(r, g, b):
    luma = 0.2126 * r + 0.7152 * g + 0.0722 * b
    # S curve around mid grey, blended so the ends stay put.
    s = luma + 0.18 * (smoothstep(0.0, 1.0, luma) - luma)
    k = s / luma if luma > 1e-5 else 1.0
    c = [v * k for v in (r, g, b)]
    # Saturation: a touch more in the midtones, less in deep shadows and bright highlights.
    sat = 1.0 + 0.12 * (1 - abs(2 * s - 1)) - 0.18 * (1 - smoothstep(0.0, 0.25, s)) - 0.15 * smoothstep(0.75, 1.0, s)
    c = [s + (v - s) * sat for v in c]
    # Split toning: teal shadows, amber highlights.
    shadow, high = 1 - smoothstep(0.0, 0.45, s), smoothstep(0.5, 1.0, s)
    tint = [0.035 * high - 0.02 * shadow, 0.012 * high + 0.008 * shadow, -0.035 * high + 0.025 * shadow]
    c = [v + t for v, t in zip(c, tint)]
    # Lifted black and a soft white.
    return [min(max(0.018 + v * 0.97, 0.0), 1.0) for v in c]


def write(name, title, f):
    OUT.mkdir(parents=True, exist_ok=True)
    lines = [f'TITLE "{title}"', f"LUT_3D_SIZE {N}", "DOMAIN_MIN 0 0 0", "DOMAIN_MAX 1 1 1"]
    for bi in range(N):
        for gi in range(N):
            for ri in range(N):
                rgb = f(ri / (N - 1), gi / (N - 1), bi / (N - 1))
                lines.append(" ".join(f"{v:.6f}" for v in rgb))
    (OUT / name).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{OUT / name}: {N}^3")


if __name__ == "__main__":
    write("warm-film.cube", "Caustica warm film", warm_film)
