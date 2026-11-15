## Debug views and meters (roadmap 5b, 2026-10-30)

- `Constants.tools` (mirrored in common.hlsli): zebras, scopes, heat scale for bounces (min(bounces, 8)), and BVH steps per sample at the top of the log map (256).
- Trace counts `proceed_steps` in both `RayQuery` loops and the bounces used, written to u27. Views 8–11 are in `display`: 8 is `(bounces − 1) / (max − 1)`; 9 is `log2(1 + steps/spp) / log2(257)`; 10 is the variance of the accumulation (log); 11 is false colour in EV around 0.18 of the exposed image.
- Scopes: `bin_scopes` atomics in `display` on the final SDR value. Before display, `clear_scopes` copies a zero upload buffer into u28; after display, `queue_readbacks` copies u28 and the light-meter texels (u18 at the output pixel, u3 at the render pixel) into one readback buffer. `finish_readbacks` maps it after Present into `Gpu::scopes` / `probe_result`.
- App: `g.zebras` / `scopes_on` / `probe` are set from `UiState` before `render`. The convergence graph reads u18 with `read_floats` at sample counts growing by 1.25× and computes RMSE ×1000 of l/(1+l) on the CPU (a 20 MB readback each time, so a hitch). The graph clears when samples drop.
- `stats.rs`: loaded only when the window is visible; saved every 30 s and on close. The achievements that were checked each frame in `redraw` were removed 2026-11-15 (roadmap Plan step 3); the statistics stay.
- Self-test 204–207 cycle views 8–11. 208 turns on zebras, scopes and the meter (cursor at the center). 209 asserts the luma histogram counts exactly width×height and the meter texels are finite. 210 sets the current image as the reference and restarts. 215 asserts ≥ 2 finite graph points and turns the tools off. Scene files start at frame 220.
- Capture CLI: `--ui-scopes` (scopes + zebras).

