"""Writes shaders/spectral_tables.hlsli for the SPECTRAL tracer (engine-notes "Spectral rendering")
and src/spectral_tables.rs, the same tables for the CPU.

Tables over 380-780 nm in 5 nm steps (81 entries, linearly interpolated in the shader):
- SPECTRAL_BASIS: Mallett and Yuksel's (2019) sRGB spectral primaries. A linear RGB value c
  becomes the spectrum dot(basis(lambda), c); the three sum to 1, so white is flat, and under
  D65 every reflectance in [0, 1] returns its own RGB.
- SPECTRAL_OUT: linear sRGB per unit radiance at lambda, XYZ->sRGB · CIE 1931 2° CMF · D65,
  normalized so a flat spectrum integrates to (1, 1, 1) (the trapezoid rule of the shader's
  linear interpolation). D65 is folded in here: every path picks up emission exactly once, so
  emitters are uplifted like reflectances and the illuminant scales the result instead.
- SPECTRAL_MEASURED: the measured Cornell box (white, green, red reflectances; the light's
  emission over D65, scaled so its RGB peaks at 1).
- SPECTRAL_D65: D65 over its value at 560 nm (blackbody emitters divide by it).

Needs colour-science (pip install colour-science). Run: python tools/spectral_tables.py
"""

import pathlib

import colour
import numpy as np

LO, HI, STEP = 380, 780, 5
grid = np.arange(LO, HI + STEP, STEP, dtype=np.float64)
shape = colour.SpectralShape(LO, HI, STEP)

basis = colour.recovery.MSDS_BASIS_FUNCTIONS_sRGB_MALLETT2019.copy().align(shape).values
cmfs = colour.MSDS_CMFS["CIE 1931 2 Degree Standard Observer"].copy().align(shape).values
d65 = colour.SDS_ILLUMINANTS["D65"].copy().align(shape).values
to_rgb = np.array(colour.RGB_COLOURSPACES["sRGB"].matrix_XYZ_to_RGB)

out = (cmfs * d65[:, None]) @ to_rgb.T  # rgb per nm at each grid wavelength
weights = np.full(len(grid), float(STEP))  # trapezoid rule
weights[0] = weights[-1] = STEP / 2
white = weights @ out
out /= white  # a flat spectrum integrates to exactly (1, 1, 1)


def to_srgb(spectrum):
    return weights @ (out * spectrum[:, None])


# Round trip: reflectances under D65 come back as their own RGB.
worst = 0.0
for c in np.random.default_rng(1).random((2000, 3)):
    worst = max(worst, np.abs(to_srgb(basis @ c) - c).max())
assert worst < 2e-3, worst
print(f"white normalization {white}, worst round trip {worst:.2e}")

# Measured Cornell box spectra (Cornell Program of Computer Graphics, "Cornell Box Data",
# www.graphics.cornell.edu/online/box/data.html): Lambertian reflectances at 400-700 nm in 4 nm
# steps, and the light's emission at 100 nm steps; both held flat outside the measured range.
CORNELL_NM = np.arange(400, 704, 4)
CORNELL_WHITE = [0.343, 0.445, 0.551, 0.624, 0.665, 0.687, 0.708, 0.723, 0.715, 0.71, 0.745, 0.758, 0.739, 0.767, 0.777, 0.765, 0.751, 0.745, 0.748, 0.729, 0.745, 0.757, 0.753, 0.75, 0.746, 0.747, 0.735, 0.732, 0.739, 0.734, 0.725, 0.721, 0.733, 0.725, 0.732, 0.743, 0.744, 0.748, 0.728, 0.716, 0.733, 0.726, 0.713, 0.74, 0.754, 0.764, 0.752, 0.736, 0.734, 0.741, 0.74, 0.732, 0.745, 0.755, 0.751, 0.744, 0.731, 0.733, 0.744, 0.731, 0.712, 0.708, 0.729, 0.73, 0.727, 0.707, 0.703, 0.729, 0.75, 0.76, 0.751, 0.739, 0.724, 0.73, 0.74, 0.737]
CORNELL_GREEN = [0.092, 0.096, 0.098, 0.097, 0.098, 0.095, 0.095, 0.097, 0.095, 0.094, 0.097, 0.098, 0.096, 0.101, 0.103, 0.104, 0.107, 0.109, 0.112, 0.115, 0.125, 0.14, 0.16, 0.187, 0.229, 0.285, 0.343, 0.39, 0.435, 0.464, 0.472, 0.476, 0.481, 0.462, 0.447, 0.441, 0.426, 0.406, 0.373, 0.347, 0.337, 0.314, 0.285, 0.277, 0.266, 0.25, 0.23, 0.207, 0.186, 0.171, 0.16, 0.148, 0.141, 0.136, 0.13, 0.126, 0.123, 0.121, 0.122, 0.119, 0.114, 0.115, 0.117, 0.117, 0.118, 0.12, 0.122, 0.128, 0.132, 0.139, 0.144, 0.146, 0.15, 0.152, 0.157, 0.159]
CORNELL_RED = [0.04, 0.046, 0.048, 0.053, 0.049, 0.05, 0.053, 0.055, 0.057, 0.056, 0.059, 0.057, 0.061, 0.061, 0.06, 0.062, 0.062, 0.062, 0.061, 0.062, 0.06, 0.059, 0.057, 0.058, 0.058, 0.058, 0.056, 0.055, 0.056, 0.059, 0.057, 0.055, 0.059, 0.059, 0.058, 0.059, 0.061, 0.061, 0.063, 0.063, 0.067, 0.068, 0.072, 0.08, 0.09, 0.099, 0.124, 0.154, 0.192, 0.255, 0.287, 0.349, 0.402, 0.443, 0.487, 0.513, 0.558, 0.584, 0.62, 0.606, 0.609, 0.651, 0.612, 0.61, 0.65, 0.638, 0.627, 0.62, 0.63, 0.628, 0.642, 0.639, 0.657, 0.639, 0.635, 0.642]
CORNELL_LIGHT = ([400, 500, 600, 700], [0.0, 8.0, 15.6, 18.4])
assert len(CORNELL_WHITE) == len(CORNELL_GREEN) == len(CORNELL_RED) == len(CORNELL_NM) == 76

d65_rel = d65 / np.interp(560, grid, d65)
light = np.interp(grid, *CORNELL_LIGHT) / d65_rel
light /= to_srgb(light).max()
measured = np.stack(
    [np.interp(grid, CORNELL_NM, c) for c in (CORNELL_WHITE, CORNELL_GREEN, CORNELL_RED)] + [light],
    axis=1,
)
for name, col in zip(["white", "green", "red", "light"], measured.T):
    print(name, to_srgb(col))


def rows(table, n=3):
    return ",\n".join(
        f"    float{n}(" + ",".join(f"{v:.7g}" for v in row) + ")" for row in table
    )


def rust_rows(table):
    return "\n".join("    [" + ", ".join(f"{v:.7e}" for v in row) + "]," for row in table)


text = f"""// Generated by tools/spectral_tables.py; do not edit. 380-780 nm in 5 nm steps.
// SPECTRAL_BASIS: Mallett-Yuksel sRGB spectral primaries; SPECTRAL_OUT: linear sRGB per unit
// radiance at that wavelength (XYZ->sRGB, CIE 1931 2°, D65; a flat spectrum integrates to 1).
static const float SPECTRAL_LO={LO}.0,SPECTRAL_HI={HI}.0,SPECTRAL_STEP={STEP}.0;
static const uint SPECTRAL_COUNT={len(grid)};
#ifdef __spirv__
// Vulkan: a buffer the backend fills from this file (NVIDIA's SPIR-V compiler expands large constant arrays at every use).
StructuredBuffer<float3> SPECTRAL_BASIS:register(t14);
#else
static const float3 SPECTRAL_BASIS[{len(grid)}]={{
{rows(basis)}
}};
#endif
#ifdef __spirv__
// Vulkan: a buffer the backend fills from this file (NVIDIA's SPIR-V compiler expands large constant arrays at every use).
StructuredBuffer<float3> SPECTRAL_OUT:register(t15);
#else
static const float3 SPECTRAL_OUT[{len(grid)}]={{
{rows(out)}
}};
#endif
// SPECTRAL_MEASURED: the Cornell box's white, green and red reflectances and its light (over
// D65, RGB peak 1); SPECTRAL_D65: D65 over its 560 nm value (for blackbody emitters).
#ifdef __spirv__
// Vulkan: a buffer the backend fills from this file (NVIDIA's SPIR-V compiler expands large constant arrays at every use).
StructuredBuffer<float4> SPECTRAL_MEASURED:register(t16);
#else
static const float4 SPECTRAL_MEASURED[{len(grid)}]={{
{rows(measured, 4)}
}};
#endif
#ifdef __spirv__
// Vulkan: a buffer the backend fills from this file (NVIDIA's SPIR-V compiler expands large constant arrays at every use).
StructuredBuffer<float> SPECTRAL_D65:register(t17);
#else
static const float SPECTRAL_D65[{len(grid)}]={{{",".join(f"{v:.7g}" for v in d65_rel)}}};
#endif
"""
path = pathlib.Path(__file__).resolve().parent.parent / "shaders" / "spectral_tables.hlsli"
path.write_text(text, encoding="utf-8", newline="\n")
print(f"wrote {path}")

rust = f"""// Generated by tools/spectral_tables.py; do not edit. 380-780 nm in 5 nm steps, the same
// tables as shaders/spectral_tables.hlsli (see there).
#![allow(clippy::excessive_precision)]
pub const LO: f32 = {LO}.0;
pub const STEP: f32 = {STEP}.0;
pub const COUNT: usize = {len(grid)};
pub const OUT: [[f32; 3]; COUNT] = [
{rust_rows(out)}
];
pub const MEASURED: [[f32; 4]; COUNT] = [
{rust_rows(measured)}
];
pub const D65: [f32; COUNT] = [
{chr(10).join(f"    {v:.7e}," for v in d65_rel)}
];
"""
rpath = path.parent.parent / "src" / "spectral_tables.rs"
rpath.write_text(rust, encoding="utf-8", newline="\n")
print(f"wrote {rpath}")
