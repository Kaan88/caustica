## egui 0.34 port

`Context::run_ui` with `Panel::*::show_inside` (see `docs/archive/egui-upgrade-plan.md` step 2). `ui::central_rect(ctx)` replaces `Context::available_rect` for clamping popups and toasts: it is the root `Ui`'s free rect after the panels, stored each frame in egui temp data (default: `content_rect`). Changeable default: `cluster()` widths in `chrome.rs` stay estimates; measure them if the new Atoms API is adopted in the redesign.

