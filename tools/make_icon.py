"""Draws the Caustica icon: the coffee-cup caustic (a nephroid), made by tracing parallel
light rays that reflect off the inside of a ring, as the renderer itself would.

    python tools/make_icon.py        # writes assets/caustica.ico and assets/caustica.png

Needs numpy and Pillow. The .ico holds 16-256 px; build.rs embeds it in the exe.
"""
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
N = 1024  # working size; downsampled for every icon size


def caustic(size: int, radius: float) -> np.ndarray:
    """Light energy of rays entering from the left and reflecting once off the far wall."""
    acc = np.zeros((size, size), np.float64)
    c = size / 2
    rays = 6000
    # Rays at heights h = R sin(a) hit the right half of the ring at angle a.
    a = np.arcsin(np.linspace(-0.995, 0.995, rays))
    n = np.stack([np.cos(a), np.sin(a)], 1)  # outward normal at the hit
    d = np.array([1.0, 0.0])
    r = d - 2 * (n @ d)[:, None] * n  # reflected direction, back into the ring
    length = -2 * radius * np.einsum('ij,ij->i', r, n)  # chord to the far side
    p = c + radius * n
    t = np.linspace(0, 1, 900)
    pts = p[:, None, :] + r[:, None, :] * (length[:, None, None] * t[None, :, None])
    # Equal samples per unit length: every ray carries the same light per step.
    w = np.repeat(np.full(rays, 1.0), t.size) * np.repeat(length / length.max(), t.size)
    xy = pts.reshape(-1, 2)
    ix = np.clip(xy[:, 0].astype(int), 0, size - 1)
    iy = np.clip(xy[:, 1].astype(int), 0, size - 1)
    np.add.at(acc, (iy, ix), w)
    return acc


def blur(a: np.ndarray, sigma: float) -> np.ndarray:
    """Gaussian blur by FFT (Pillow cannot blur float images)."""
    f = np.fft.fftfreq(a.shape[0])
    g = np.exp(-2 * (np.pi * sigma * f) ** 2)
    return np.real(np.fft.ifft2(np.fft.fft2(a) * g[:, None] * g[None, :]))


def main() -> None:
    radius = N * 0.40
    c = N / 2
    yy, xx = np.mgrid[0:N, 0:N]
    dist = np.hypot(xx - c, yy - c)
    inside = dist < radius

    light = caustic(N, radius)
    light = blur(light, N / 300)
    light = light / np.percentile(light[inside], 98.5)
    glow = blur(light, N / 40)
    e = np.clip(light, 0, None) + 0.6 * glow
    e = 1 - np.exp(-2.2 * e)  # tone map

    # Background: a deep blue rounded square; the ring's floor a little lighter.
    img = np.zeros((N, N, 4), np.float64)
    shade = 0.75 + 0.25 * (1 - yy / N)
    img[..., 0] = 0.035 * shade
    img[..., 1] = 0.06 * shade
    img[..., 2] = 0.13 * shade
    floor = inside * (0.5 + 0.5 * (1 - dist / radius))
    img[..., 0] += 0.03 * floor
    img[..., 1] += 0.06 * floor
    img[..., 2] += 0.10 * floor
    # Caustic colour: amber in the tails, white-hot at the cusp.
    warm = np.array([1.0, 0.62, 0.18])
    hot = np.array([1.0, 0.97, 0.88])
    k = np.clip((e - 0.55) / 0.45, 0, 1)[..., None]
    col = warm * (1 - k) + hot * k
    img[..., :3] += col * e[..., None] * inside[..., None]
    # The ring itself: a thin metallic rim, brighter where the light reflects (right side).
    rim = np.exp(-((dist - radius) / (N * 0.012)) ** 2)
    lit = 0.35 + 0.65 * np.clip((xx - c) / radius, 0, 1)
    img[..., :3] += 0.45 * rim[..., None] * lit[..., None] * np.array([0.95, 0.8, 0.55])
    img[..., 3] = 1
    img = np.clip(img, 0, 1)

    rgba = Image.fromarray((img * 255 + 0.5).astype(np.uint8), 'RGBA')
    # Rounded-square mask.
    mask = Image.new('L', (N, N), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, N - 1, N - 1], radius=int(N * 0.2), fill=255)
    rgba.putalpha(mask)

    out = ROOT / 'assets'
    out.mkdir(exist_ok=True)
    big = rgba.resize((256, 256), Image.LANCZOS)
    big.save(out / 'caustica.png')
    big.save(out / 'caustica.ico', sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)])
    print('wrote', out / 'caustica.ico')


if __name__ == '__main__':
    main()
