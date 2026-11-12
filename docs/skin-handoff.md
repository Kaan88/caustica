# UI skin pass: handoff (2026-11-12)

For the next session. It starts cold, on another account, so everything it needs is here or in the files named here.

## Where things stand

- **UI layout B is built and committed** on `main` as `cc9bf00` (not pushed). Tests, clippy and the self-test passed on D3D12 and Vulkan; snapshot in `builds/2026-11-12-ui-redesign-b-complete`. Notes: engine-notes "UI layout B"; open tails: roadmap, "UI and editor".
- **Uncommitted now:** `docs/roadmap.md` (the skin pass as Plan step 1, other steps moved down one), this file, and `docs/mockups/`. `docs/new-scene-prompt.md` is the owner's untracked file: leave it.
- **The owner's complaint:** egui "feels cheap no matter what, like crappy html buttons". The answer agreed on: stay on egui, replace the stock look with the app's own painted widgets. The layout B structure does not change.
- **The pick:** the owner chose direction **B "Instrument"**, "definitely B, with no cutbacks". Direction A "Quiet" is rejected.

## The mockup

- Source of truth: `docs/mockups/skin-b-instrument.dc.html`. Its `<style>` block is the specification (colours, gradients, shadows, radii, sizes, timings) and its markup shows the app at 1280×720 with the Render tab, then a sheet with every control in rest, hover, pressed, focused and disabled states, a menu with a tooltip, a segment, a tab strip, the type scale and three rows.
- The file is a design-canvas board: it loads `./support.js` from the canvas, so it does not render from disk. Read it as text. To see it rendered, ask the owner for a capture, or publish it to a new design canvas on the new account (Artifact type "Design"; name the board file `Main.dc.html`).
- The original canvas, https://claude.ai/artifact/W3Fp5p8GoqgjoaSsV3xDxR, belongs to the account that drew it, so the new account can't open it.
- The boards were written without being looked at rendered. If something in the mockup looks broken (an overlap, a clipped label), it is a drawing slip, not a design choice: ask the owner.
- `docs/mockups/skin-a-quiet.dc.html` is the rejected direction, kept for reference only.

## What "no cutbacks" means

Every visual property in board B is built, not approximated to what egui's stock painter does easily:

- **Raised controls:** a vertical two-stop gradient (`#3b3c41` to `#2f3034`), a 1 px near-black edge (`#0c0c0d`), a 1 px top highlight inside the edge (white at 8 %), a soft 1 px drop shadow. Hover lightens the gradient; pressed inverts it, adds an inner shadow and moves the content down half a pixel.
- **Recessed controls** (text and number fields, slider groove, checkbox, switch track): near-black fill (`#141416`), an inner shadow at the top, a faint light line under the bottom edge.
- **Slider:** 6 px groove, 4 px fill in the accent darkened with black, a 10×16 px metal thumb with its own gradient, edge and highlight.
- **Primary button and checked checkbox:** the accent as a gradient with a bright top highlight and dark text (`#06272c`).
- **Toggle on:** pressed-in look with accent text, not a tinted fill.
- **Flat buttons** (top-bar pins, tool rail, tab strip): no chrome at rest, the raised look on hover, pressed-in when on.
- **Section headers:** full-width bands (`#2b2b2f`, light line above, black line below), 11 px semibold caps with 0.06 em tracking, the preview right-aligned in the mono font.
- **Tab strip:** darker gutter (`#1a1a1c`); the active tab takes the panel colour with a 2 px accent bar on its outer edge.
- **Popups and menus:** `#2a2a2e`, black edge, top highlight, a large soft shadow; the hovered item is filled with the accent and its text goes dark. Separators are a black line over a light line.
- **Top bar:** gradient `#2c2c30` to `#242427` with a top highlight; grouped buttons joined into segments; the spp readout sits in a recessed well.
- **Focus:** a 1.5 px accent ring on every control.
- **Motion:** 100 ms on colour, shadow and position; 140 ms with the eased curve on the switch knob and the section chevron. Nothing may keep the UI repainting at rest.
- **Type:** IBM Plex Sans 12.5 for text (14 semibold for the tab title, 11 semibold caps for sections), IBM Plex Mono 11.5 for numbers and previews. Row height 28, label column 128.
- **Accent:** the app's teal `#60CAD8` (`ui::ACCENT`), unchanged.

If a property turns out to be impossible or too slow in egui, say so to the owner with the evidence before dropping it. Do not quietly simplify.

## How to build it (advice, not yet tried)

- **Gradients and inner shadows:** egui's painter has no gradient fill or inset shadow. Two workable routes:
  1. Nine-slice textures: draw each control skin (raised rest, hover, pressed, recessed, band, thumb, popup) once into a small RGBA image at the current pixels-per-point, upload as egui textures, and paint with a nine-slice mesh. Exact, cheap per frame, and sharp at any zoom if regenerated when the scale changes (Ctrl +/−).
  2. Meshes with per-vertex colours for the gradients plus thin strips for highlights and inner shadows. No textures, but rounded corners with a gradient need care.
  Route 1 is the likelier way to match the board exactly. Decide by trying both on one button.
- **Widgets:** one module (suggested `src/ui/skin.rs`) with the app's own button, flat button, toggle, combo, checkbox, switch, slider with value field, text field, section header and menu item, each taking the same arguments as the call it replaces, each allocating with `ui.allocate_response` and painting itself. Animate with `ctx.animate_bool_with_time`.
- **Existing helpers to restyle, not duplicate:** `ui::fold` (section headers, `src/ui.rs`), `ui::row` / `row_controls` (rows and the key-mark slot), the top bar's `promo` and `pin_width` in `src/ui/chrome.rs` (measured width must equal drawn width, or the bar wraps), `tool_rail`, the tab strip in `side_panel`.
- **Fonts:** egui uses its default fonts today, with Hack appended as a glyph fallback (`ui::theme`). IBM Plex Sans and IBM Plex Mono are under the SIL Open Font License: bundle the TTFs, add a line to `THIRD-PARTY-NOTICES.md`, and keep the fallback so `ui_strings_have_glyphs` still passes.
- **Known limit:** egui has no hinting or subpixel text, so text at 100 % scale stays a little softer than native. The owner has been told.

## Order of work

1. Skin module with the raised button, recessed field and section band; prove the gradient and inner-shadow technique.
2. **Render tab only**, plus the font. Stop there: the owner compares it with the current look in the live app. If it fixes the feel, continue; if not, the toolkit question reopens (a webview UI was weighed and set aside as weeks of work).
3. Then tab by tab: Look, Lights, Object, Material, Export; then the top bar, rail, outliner, timeline, the Scenes and Assets browsers, menus, popups, tooltips, toasts, the Ctrl+K palette and the remaining windows.
4. Re-baseline the `--capture-ui` captures once at the end. The self-test's UI frames must pass throughout.

## Project rules that apply (details in `CLAUDE.md`)

- UI code stays in the main session; long runs (full `cargo test`, `--self-test`, release builds, snapshots) go to the `verifier` agent in one batch; every finished change gets the `reviewer` agent before a commit.
- Build checks into `target/validation`, never `target/release`; never kill the running app.
- Capture the UI with `--headless --capture-ui --ui-tab <name> --capture out.png` (switches near `src/main.rs:687`); check D3D12 and `--vulkan`.
- Each finished roadmap item gets a release build and a snapshot in `builds/`. Commit and push only when the owner asks. Ask before removing any feature, file or doc.
- UI text keeps technical names (DLSS, RR, DLAA); no relabelling.
- Notes for the finished skin go in `docs/engine-notes.md`; open items in `docs/roadmap.md`.

## Still open from layout B (not part of the skin)

- The owner's pass over the click paths in the live app (key marks, sorting, downloads, Icon only).
- Drawing the layout on Linux.
- The owner's confirmation of the spec's section 9 defaults (`docs/ui-redesign-b.md`).
