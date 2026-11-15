## egui 0.33 (step 1 of docs/egui-upgrade-plan.md, 2026-11-10)
- egui, egui-winit, epaint, emath, ecolor bumped 0.31.1 → 0.33.3 together (`cargo update -p egui -p egui-winit`).
- The font atlas is a colour image: the `ImageData::Font` arms are gone from the Vulkan (`vulkan/gui.rs`) and D3D12 (`gpu/frame.rs`, edited by reading) uploaders. Glyph edges no longer go through the old gamma coverage curve.
- Menus close on any click inside by default now. `ui::sticky_menu` keeps the old close-on-click-outside behaviour for the top-bar Menu (its Renderer, Text-to-scene model and About submenus inherit it: `ui.menu_button` inside a menu is a `SubMenuButton`, and `SubMenu` takes the parent's `MenuConfig` when it has none, egui menu.rs:445), LPE Presets and Levels; plain items still close them with `ui.close()` (was `close_menu`). Default choice, changeable per menu.
- `SelectableLabel` → `Button::selectable`; `screen_rect` → `content_rect`; `ColorImage` literals → `ColorImage::new`; the glyph self-test uses `fonts_mut`. `ui.disable()` now also dims (export dialog rows): accepted.
- Visible in SDR captures (1280/1920/2560, render/look/scene tabs): slider handles are rounded rectangles instead of circles, panel text is a little narrower and crisper, combo arrows point down, text weight about the same, top bar rows unchanged.
- Owner checks on Windows: D3D12 compile and UI, and text/translucent-panel brightness in scRGB HDR and HDR10/PQ (not testable headless). No capture or self-test frame opens the Menu submenus; click a slider in Menu → Renderer to confirm it stays open.

