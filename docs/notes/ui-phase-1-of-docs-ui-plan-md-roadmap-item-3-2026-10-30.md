## UI (phase 1 of docs/ui-plan.md, roadmap item 3, 2026-10-30)

- Layout: `TopBottomPanel` top bar and status bar, `SidePanel::right` (370 pt default, 320–560). Panel fill is 250/255 alpha over the render, which still fills the window (lens shift is phase 3).
- **Fixed slider width (130 pt)** inside `row`. Sizing sliders from `available_width` made the panel grow to its maximum on some tabs.
- Picking selects `Selection::Object`, switches to the Scene tab and shows the panel. Escape deselects (it no longer quits). Delete removes the selected object or area light. Tab cycles all → top bar only → nothing.
- Every single-key toggle and every save/load result is a toast; `self.status` is gone.
- **Render tab (reworked after owner feedback: the first version's "AI / Built-in / Off" split and "Upscaler: not needed at 100%" read wrong):** one Reconstruction section in the old menu's terms. "DLSS Ray Reconstruction" checkbox (disabled with `rr_status` when `Gpu::rr_available()` is false), RR model, Quality mode, Upscaler (disabled while RR is on), DLSS preset, "Native denoiser (D)", Render scale (advanced or off-list).
  - Quality mode entries come from `quality_modes`: with RR, DLAA (100%); without, Native (100%), plus DLAA / FSR Native AA / XeSS Native AA (100%) for a temporal upscaler, then 67 / 58 / 50 / 33%.
  - **`Settings.native_aa`** (default false): at 100% `prepare_upscaler` runs the temporal upscaler as anti-aliasing. Off by default so `--no-rr` at 100% stays the progressive path that unbiased references rely on. It is forced off for high-res export (DLAA restarts every frame) and is in the reconfigure trigger list. Self-test 196 checks that DLAA starts.
- Exposure is shown in EV (log2), written back only on change.
- egui's default fonts lack `→` and `●`: the glyph unit test caught both. The status dot is painted.
- CLI for captures: `--ui-tab`, `--ui-advanced`, `--ui-select`.
- Self-test: 178 opens the Render tab for the slider-clamp guard; 190–195 draw the object, light, main light, camera, Look and Export views with advanced rows, then delete the added light through `delete_selected`.
- Later phases are done; see "Viewport interaction" and "UI phases 3–5".

