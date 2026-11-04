"""Writes shaders/layers.hlsli: the directional albedo tables the layered materials (track 11)
need, as constant arrays (no GPU resource).

Run from the repo root:  python tools/fit_layers.py

Sheen: Charlie distribution (Estevez and Kulla 2017) with alpha = roughness^2 (roughness
clamped to SHEEN_MIN, as pathtrace.hlsl does) and their fitted Lambda visibility
1 / ((1 + L(nv) + L(nl)) 4 nv nl), as the glTF sample viewer has it (Neubelt's simpler
1 / (4 (nl + nv - nl nv)) reaches an albedo of 5.7 at grazing), white, no Fresnel. E(nv, roughness) = integral of D V nl over the hemisphere of light
directions. Rows: roughness 0..1, columns: nv 0..1, both in N steps (endpoints included).

GGX: the single-scattering albedo of pathtrace.hlsl's specular lobe with F = 1 (GGX D, separable
Smith G1 G1, alpha = max(0.001, roughness^2)), for the energy compensation (Turquin 2019, the
single-table form of Kulla and Conty 2017). Importance-sampled on a stratified grid of half
vectors drawn from D nh: each sample weighs G1(v) G1(l) vh / (nv nh). Rows roughness 0..1,
columns nv 0..1, GGX_N steps each.

Coat: the albedo of the clear coat's lobe (GGX with Schlick F, f0 0.04) after its compensation
(divided by the GGX albedo above), which is what the layers below lose to it: dimming them by
this instead of F(nv) keeps a white base under a rough coat at 1 (F(nv) nears 1 at grazing,
where a rough coat's facets mostly see far smaller F(vh)). Same grid as GGX.

Anisotropic GGX: the same albedo for pathtrace.hlsl's anisotropic lobe (alpha a = max(0.001,
roughness^2) along the brushing direction T, lerp(a, 1, strength^2) across it), which also
depends on the view's azimuth about the normal: E(nv, azimuth, roughness, strength), the
azimuth folded to 0..90 degrees from T (the lobe is symmetric about T and B). No single
effective roughness fits the isotropic table: the albedo varies by 0.5-0.66 with the azimuth.
"""
from pathlib import Path

import numpy as np

N = 16
GGX_N = 32
ANISO = (12, 4, 8, 6)  # nv, azimuth, roughness, strength
SHEEN_MIN = 0.07
OUT = Path(__file__).resolve().parent.parent / "shaders" / "layers.hlsli"


def lambda_sheen(c, a):
    def helper(x):
        t = (1 - a) ** 2
        k = [21.5473 + (25.3245 - 21.5473) * t, 3.82987 + (3.32435 - 3.82987) * t,
             0.19823 + (0.16801 - 0.19823) * t, -1.97760 + (-1.27393 + 1.97760) * t,
             -4.32054 + (-4.85967 + 4.32054) * t]
        return k[0] / (1 + k[1] * np.power(x, k[2])) + k[3] * x + k[4]
    c = np.abs(c)
    return np.where(c < 0.5, np.exp(helper(c)), np.exp(2 * helper(0.5) - helper(1 - c)))


def sheen_albedo(nv, roughness, steps=(384, 768)):
    a = max(roughness, SHEEN_MIN) ** 2
    nv = max(nv, 1e-4)
    v = np.array([np.sqrt(1 - nv * nv), 0.0, nv])
    t = (np.arange(steps[0]) + 0.5) / steps[0] * (np.pi / 2)
    p = (np.arange(steps[1]) + 0.5) / steps[1] * (2 * np.pi)
    t, p = np.meshgrid(t, p, indexing="ij")
    l = np.stack([np.sin(t) * np.cos(p), np.sin(t) * np.sin(p), np.cos(t)], -1)
    h = l + v
    h /= np.linalg.norm(h, axis=-1, keepdims=True)
    nh = np.clip(h[..., 2], 0, 1)
    sin_h = np.sqrt(np.clip(1 - nh * nh, 0, 1))
    d = (2 + 1 / a) * sin_h ** (1 / a) / (2 * np.pi)
    nl = l[..., 2]
    vis = np.clip(1 / ((1 + lambda_sheen(nv, a) + lambda_sheen(nl, a)) * 4 * nv * nl), 0, 1)
    dw = (np.pi / 2 / steps[0]) * (2 * np.pi / steps[1]) * np.sin(t)
    return float(np.sum(d * vis * nl * dw))


def g1(c, a):
    return 2 * c / np.maximum(1e-6, c + np.sqrt(a * a + (1 - a * a) * c * c))


def ggx_albedo(nv, roughness, steps=512, f0=None):
    a = max(0.001, roughness * roughness)
    nv = max(nv, 1e-3)
    v = np.array([np.sqrt(1 - nv * nv), 0.0, nv])
    u = (np.arange(steps) + 0.5) / steps
    u, phi = np.meshgrid(u, u * 2 * np.pi, indexing="ij")
    c = np.sqrt((1 - u) / (1 + (a * a - 1) * u))
    s = np.sqrt(np.maximum(0, 1 - c * c))
    h = np.stack([s * np.cos(phi), s * np.sin(phi), c], -1)
    vh = h @ v
    l = 2 * vh[..., None] * h - v
    nl = l[..., 2]
    w = np.where((nl > 0) & (vh > 0), g1(nv, a) * g1(np.maximum(nl, 0), a) * vh / (nv * c), 0)
    if f0 is not None:
        w = w * (f0 + (1 - f0) * (1 - np.clip(vh, 0, 1)) ** 5)
    return float(w.mean())


def aniso_albedo(nv, azimuth, roughness, strength, steps=256):
    """Half vectors drawn from D nh by the slope distribution, as ggx_sample_aniso does."""
    ax = max(0.001, roughness * roughness)
    ay = ax + (1 - ax) * strength * strength
    nv = max(nv, 1e-3)
    st = np.sqrt(1 - nv * nv)
    v = np.array([st * np.cos(azimuth), st * np.sin(azimuth), nv])
    u = (np.arange(steps) + 0.5) / steps
    u, phi = np.meshgrid(u, u * 2 * np.pi, indexing="ij")
    s = np.sqrt(u / (1 - u))
    h = np.stack([ax * s * np.cos(phi), ay * s * np.sin(phi), np.ones_like(s)], -1)
    h /= np.linalg.norm(h, axis=-1, keepdims=True)
    vh = h @ v
    l = 2 * vh[..., None] * h - v

    def g1a(w):
        return 2 * w[..., 2] / np.maximum(1e-6, w[..., 2] + np.sqrt(
            w[..., 0] ** 2 * ax * ax + w[..., 1] ** 2 * ay * ay + w[..., 2] ** 2))
    ok = (l[..., 2] > 0) & (vh > 0)
    w = np.where(ok, g1a(v) * g1a(np.where(ok[..., None], l, h)) * vh / (nv * h[..., 2]), 0)
    return float(w.mean())


# Vulkan registers of the tables (src/vulkan/gpu.rs `TABLES`, t10-t13).
REGISTERS = {"SHEEN_ALBEDO": 10, "GGX_ALBEDO": 11, "COAT_ALBEDO": 12, "ANISO_ALBEDO": 13}
VULKAN_NOTE = ("// Vulkan: a buffer the backend fills from this file (NVIDIA's SPIR-V compiler expands large "
               "constant arrays at every use).\n")


def table(name, rows):
    """A table packed four values to a float4 (a static const float array takes a 16-byte
    constant slot per element), with an accessor `<name>_at(i)` by flat index."""
    flat = [v for row in rows for v in row]
    assert len(flat) % 4 == 0
    quads = [flat[i:i + 4] for i in range(0, len(flat), 4)]
    body = ",\n    ".join(", ".join("float4(" + ", ".join(f"{v:.5f}" for v in q) + ")" for q in quads[i:i + 4])
                           for i in range(0, len(quads), 4))
    # Vulkan reads the table from a buffer the backend fills from this file (NVIDIA's SPIR-V
    # compiler expands large constant arrays at every use: 40 s and 10 GB for the trace pass).
    return (f"#ifdef __spirv__\n{VULKAN_NOTE}StructuredBuffer<float4> {name}:register(t{REGISTERS[name]});\n#else\n"
            f"static const float4 {name}[{len(quads)}] = {{\n    {body}\n}};\n#endif\n"
            f"float {name}_at(uint i) {{ return {name}[i >> 2][i & 3]; }}\n")


def lookup(fn, name, n):
    """Bilinear lookup of a table at (nv, roughness), both 0..1."""
    return (f"float {fn}(float nv, float roughness) {{\n"
            f"    float2 x = saturate(float2(nv, roughness)) * {n - 1};\n"
            f"    uint2 i = min(uint2(x), {n - 2}); float2 f = x - i;\n"
            f"    #define S(c, r) {name}_at((i.y + r) * {n} + i.x + c)\n"
            "    float e = lerp(lerp(S(0, 0), S(1, 0), f.x), lerp(S(0, 1), S(1, 1), f.x), f.y);\n"
            "    #undef S\n"
            "    return e;\n"
            "}\n")


def aniso_lookup(name, dims):
    """Quadrilinear lookup at (nv, azimuth / 90 degrees, roughness, strength), all 0..1."""
    n, a, r, s = dims
    return (f"float aniso_albedo(float nv, float azimuth, float roughness, float strength) {{\n"
            f"    float4 x = saturate(float4(nv, azimuth, roughness, strength)) * float4({n - 1}, {a - 1}, {r - 1}, {s - 1});\n"
            f"    uint4 i = min(uint4(x), uint4({n - 2}, {a - 2}, {r - 2}, {s - 2})); float4 f = x - i;\n"
            "    float e = 0;\n"
            "    [unroll] for (uint k = 0; k < 16; k++) {\n"
            "        uint4 c = uint4(k & 1, (k >> 1) & 1, (k >> 2) & 1, k >> 3);\n"
            "        float4 w = lerp(1 - f, f, float4(c));\n"
            f"        uint4 j = i + c; e += w.x * w.y * w.z * w.w * {name}_at(((j.w * {r} + j.z) * {a} + j.y) * {n} + j.x);\n"
            "    }\n"
            "    return e;\n"
            "}\n")


def main():
    sheen = [[sheen_albedo(j / (N - 1), i / (N - 1)) for j in range(N)] for i in range(N)]
    ggx = [[ggx_albedo(j / (GGX_N - 1), i / (GGX_N - 1)) for j in range(GGX_N)] for i in range(GGX_N)]
    coat = [[ggx_albedo(j / (GGX_N - 1), i / (GGX_N - 1), f0=0.04) / ggx[i][j] for j in range(GGX_N)]
            for i in range(GGX_N)]
    n, a, r, s = ANISO
    aniso = [[aniso_albedo(x / (n - 1), y / (a - 1) * np.pi / 2, z / (r - 1), w / (s - 1)) for x in range(n)]
             for w in range(s) for z in range(r) for y in range(a)]
    OUT.write_text(
        "// Generated by tools/fit_layers.py: do not edit.\n"
        f"// Sheen directional albedo E(nv, roughness): {N} rows of roughness 0..1, {N} columns of nv 0..1.\n"
        + table("SHEEN_ALBEDO", sheen)
        + f"static const float SHEEN_MIN_ROUGHNESS = {SHEEN_MIN};\n"
        + lookup("sheen_albedo", "SHEEN_ALBEDO", N)
        + f"// GGX single-scattering albedo with F = 1: {GGX_N} rows of roughness 0..1, {GGX_N} columns of nv 0..1.\n"
        + table("GGX_ALBEDO", ggx)
        + lookup("ggx_albedo", "GGX_ALBEDO", GGX_N)
        + f"// The clear coat's compensated albedo (f0 0.04): {GGX_N} rows of roughness 0..1, {GGX_N} columns of nv 0..1.\n"
        + table("COAT_ALBEDO", coat)
        + lookup("coat_albedo", "COAT_ALBEDO", GGX_N)
        + f"// Anisotropic GGX albedo with F = 1: rows of {n} nv 0..1, for {a} azimuths 0..90 degrees from the\n"
        f"// brushing direction, {r} roughnesses 0..1 and {s} strengths 0..1 (innermost first).\n"
        + table("ANISO_ALBEDO", aniso)
        + aniso_lookup("ANISO_ALBEDO", ANISO),
        encoding="utf-8",
    )
    print(f"wrote {OUT}; sheen E range {min(map(min, sheen)):.4f}..{max(map(max, sheen)):.4f}, "
          f"GGX E range {min(map(min, ggx)):.4f}..{max(map(max, ggx)):.4f}, "
          f"coat E range {min(map(min, coat)):.4f}..{max(map(max, coat)):.4f}, "
          f"anisotropic E range {min(map(min, aniso)):.4f}..{max(map(max, aniso)):.4f}")
    for i in (0, 5, 10, 15):
        print(f"sheen roughness {i / (N - 1):.2f}:", " ".join(f"{v:.3f}" for v in sheen[i][::3]))
    for i in (0, 8, 16, 24, 31):
        print(f"GGX roughness {i / (GGX_N - 1):.2f}:", " ".join(f"{v:.3f}" for v in ggx[i][::4]))


if __name__ == "__main__":
    main()
