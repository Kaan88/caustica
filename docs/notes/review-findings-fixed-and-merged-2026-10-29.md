## Review findings: fixed and merged (2026-10-29)

Done on branch `review-findings` and fast-forwarded into `main` (commit c72f163). The pre-fix state is commit 1fc032d. All results were measured.

**Native denoiser** (see "Rendering behaviour" for the design). Scores are ×1000, measured with `tools/motion_error.py`.

| Case | Before | After |
|---|---|---|
| Orbit, 2 spp: error / blurred / light band | 20.65 / 16.94 / 36.9 | 9.63 / 4.40 / 7.0 |
| Still camera, 16 spp | 12.96 | 6.31 |
| Still camera, 1024 spp | 4.79 (raw 2.43) | 1.56 |

- The old filter was biased: its blurred output was fed back as history, so it was worse than raw above 64 spp.
- The white specks came from per-pixel variance estimates that were near zero by chance. The 3×3 variance prefilter fixed them, so no separate firefly clamp was added.
- The light "ghost" was the spatial and temporal blur crossing from the light to the ceiling. The object-id stop fixed it.

**Sobol** decorrelates per reset and is now 4× cheaper. See "Rendering behaviour".

**Other fixes:**
- Pan follows the screen axes, and the target is clamped to ±20.
- The Pause glyph is ⏸, and the resolution label reads "Render … · output …".
- The `tests::ui_strings_have_glyphs` unit test checks every non-ASCII character in UI strings against egui's fonts.
- The Delete key only works while the Scene objects window is open.
- The UI capture is copied before Present.
- Marble veins are thinner and softer.
- The CLI is order-independent.
- RENDER_OK prints means.

**Performance:** 1440×900, 4 spp, mean of 200 frames, before → after on this branch.

| Case | Before | After |
|---|---|---|
| Classic, raw | 2.87 | 1.64 |
| Classic, native denoiser | 7.48 | 3.10 |
| Bunny (13), native | 7.53 | 3.86 |
| Mirror corridor (17), native | 10.04 | 5.75 |
| RR | 3.95 | 3.88 |
| With UI drawn, wall clock | 11.8 | 4.2 |

The gains come from:
- First-hit shadow rays.
- The Sobol pair cache.
- The packed filter guide with demodulated filtering.
- One egui upload per frame.
- Skipping dirty-key serialization when settings are unchanged.

**Not a bug:** windows going off-screen after a resize only happened because the self-test resizes the swapchain without resizing the window.

**Still open (not started):**
- Faint low-frequency blotches at very low spp, which is typical of SVGF.

