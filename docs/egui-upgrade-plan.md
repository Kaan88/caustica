# egui 0.31 → 0.37 upgrade plan (2026-11-10)

Researched by a workflow (changelogs of egui, epaint, egui-winit 0.32–0.37, an impact map on this tree, a completeness check). For the main session to run, after the exposure and accuracy lanes merge and before any UI redesign (mockups: the "Caustica UI redesign mockups" canvas).

## Verdict: worth it, in three steps

Why:
- **Text:** 0.34 replaces ab_glyph with Skrifa + vello_cpu, adding hinting and harfrust kerning. Text gets visibly crisper, and 0.32 also sharpens icons.
- **Menus and popups:** the new Popup/Tooltip API with `PopupCloseBehavior`, and popups no longer close on mouse release (0.37). These replace our 8 hand-built `Frame::popup` sites and fix stuck submenus (#7589).
- **Focus and keys:** all keys are released when the window loses focus (0.32), Ctrl+arrow no longer moves focus (0.34), and `lost_focus` is fixed (0.35). These hit the ui-review's shortcut and walk-mode complaints.
- **Layout tools the redesign needs:**
  - Atoms / `AtomLayout` (0.32; nesting in 0.35) for icon + label + shortcut buttons, with intrinsic sizes so the top bar can measure itself instead of guessing (`cluster()` widths).
  - `egui_taffy` 0.16 for flex-wrap, and `egui_tiles` 0.18 for docking. Both target egui 0.37. `egui_dock` is stuck on 0.36, so don't use it.
  - Panels that never overflow their width (0.35), and drag to close or reopen a panel (0.35/0.36).
- **Smaller:** faded `ScrollArea` edges, and scroll events consumed by the area instead of leaking to the camera. `CompletionPopup` would serve the console, `RangeSlider` the timeline ranges, and per-widget style classes (0.34/0.35) the hint text.
- **Compatible:**
  - winit stays on 0.30: egui-winit 0.37 uses 0.30.13.
  - The MSRV of 1.95 is under our rustc 1.99, and we're already on edition 2024.
  - No `PaintCallback` is used.
  - The path-traced pixels, Sobol sampling and the RR path are untouched.

Cost:
- About 3 focused days for the port: 1.5–2 days of code, then about 1 day checking both backends in SDR, scRGB HDR and HDR10 and re-baselining the UI captures.
- Adopting the new layout tools is part of the redesign, not of this port.

## Steps (one commit each, reviewer before each, screenshots at 1280 / 1920 / 2560)

### 1. 0.31 → 0.33: the renderer-facing step (opus-low for the renderer part, sonnet for the rest)

**Font atlas (renderer).** 0.32 made the atlas a colour image (#7298). Delete the `ImageData::Font(i) => i.srgba_pixels(None)` arms in `src/gpu/frame.rs:611-612` (D3D12) and `src/vulkan/gui.rs:192-193` (Vulkan), and keep `Color(i) => i.pixels`.
- The old path applied gamma-based coverage-to-alpha, so glyph edge weight changes.
- Recheck text brightness through `shaders/gui.hlsl:19-23` in SDR, HDR and PQ.

**Translucency (renderer).** 0.32 fixed semi-transparent colours that looked too bright, which changes premultiplied edges and fills. Popup shadows, selection highlights and overlays get slightly darker. Recheck the same three output modes.

**Menus:**
- Menus now close on click by default (a silent change), across 13 `menu_button` sites. Menus holding sliders, checkboxes or combos must keep `CloseOnClickOutside`: the Renderer and AI submenus, Presets and Levels.
- About 30 `ui.close_menu()` calls become `ui.close()` or are deleted. Those in non-menu popups (`ui.rs:2034`, `chrome.rs:79`) need `ui.close()`.
- Roughly 1 day.

**Smaller changes:**
- `SelectableLabel` is gone: 33 sites, mostly through `selectable_label` / `selectable_value`. Its framing now follows buttons, so recheck the tab rows in `chrome.rs`.
- `ui.disable()` now dims (`video_ui.rs:147,160`).
- `screen_rect` is deprecated in favour of `content_rect` (`overlay.rs:79,105`, `windows.rs:557`).

### 2. 0.33 → 0.34: the API move while the old names still warn (sonnet)
- `Context::run` → `run_ui`, with panels drawn via `show_inside(ui)`. Sites: `main.rs:1215,3719,3752`, `ui.rs:2319,2731`, `scene_tab.rs:838`.
- `SidePanel` / `TopBottomPanel` → `Panel::top/bottom/right(id)`. Sites: `chrome.rs:84,533,704`, `timeline_ui.rs:80`, and the capture paths' `CentralPanel::show`.
- `ctx.style()` → `global_style` (`ui.rs:1138`). Also check `set_style` (`ui.rs:1160`) and `style_mut` (`ui.rs:2772`).
- `Context::available_rect` is deprecated: the 4 `constrain_to(ctx.available_rect())` sites in `chrome.rs:514,667`, `ui.rs:1920` and `overlay.rs:247`.
- The text backend changes here. Re-baseline the UI captures and the self-test UI frames in this step, and recheck where the top bar wraps, since text widths shift.

### 3. 0.34 → 0.37: removals (sonnet)
- 0.35 removes every deprecated item. Panel methods are renamed (#8192), `impl Into<f32>` arguments are gone, and `AlphaFromCoverage` is renamed to `FontColorTransferFunction`.
- 0.36 moves `Modifiers` out of `RawInput` into an event. Drop the field from the headless builders (`main.rs:3711`, `ui.rs:2330,2420,2744`, `scene_tab.rs:848`). The live path goes through egui-winit.
- 0.36 also fixes `TexturesDelta` being dropped (#8356). Make sure the set/free order and partial updates still match in both uploaders, and that a resized atlas recreates the D3D12 texture.
- 0.37:
  - `default_fonts` is no longer a default feature, so set `egui = { version = "0.37", features = ["default_fonts"] }`. Without it, every glyph renders as tofu.
  - `Id::new` and `From<&str>` are deprecated (20 sites). Migrate them, since clippy `-D warnings` fails on the deprecation.
  - `Ui::id` becomes `scope_id` (0 sites).
  - `Clipboard::new` becomes unsafe (unused).
- The glyph self-test `ctx.fonts(|f| f.has_glyph(..))` (`main.rs:3773`) probably breaks under the 0.37 font rework and `MissingGlyphPolicy`. Port it, and keep it asserting real coverage.

## Risks and checks
- **D3D12 isn't compiled on Linux.** The `src/gpu/` changes (Font arm, texture resize) are cfg(windows). Grep them by hand here, and compile and run them in the next Windows session.
- **Outline hack:** the silhouette outline in `gui.hlsl:8` encodes a negative `uv.x` in egui meshes and relies on UVs passing through unchanged. No tessellator change is listed; confirm on screen.
- **Vertex layout:** (pos, uv, Color32) is unchanged in every changelog; confirm by compiling before touching either input layout.
- **Self-test:** UI captures change, so update them in step 2 and keep the self-test at zero D3D12 and validation warnings.
- **Before starting:** port the uncommitted top-bar fix (`chrome.rs` `top_bar` and `cluster`, on `cpu-playable`). Step 2 changes the panel API around it.

## After the port (the redesign, separate)
Pick a direction from the mockups first. The tools it would use:
- Atoms for the toolbar buttons
- `egui_taffy` for any wrapping row
- `egui_tiles` if docking is chosen (option E)
- the new Popup API for menus and cards
- style classes for hint text.
