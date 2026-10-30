# Cornell RTX UI redesign plan

Status: plan only, nothing implemented. Written 2026-10-29 against the source as it stood that day.

> **Update 2026-10-30:** add a DLSS Super Resolution preset choice (Default, J, K, L, M) next to the upscaler, and an HDR section (Auto/SDR, detected peak and SDR-white values from Windows). Both are specified in [roadmap.md](roadmap.md), which is now the ordered plan.
Line numbers are a snapshot; `main.rs` and `gpu.rs` were being edited in parallel (upscaler work), so search by the quoted code if a number has drifted.

The complaint: "the UI is just a bunch of sliders packed together." That is accurate. The panel is a flat dump of `Settings` fields in declaration-ish order, grouped by implementation area rather than by what the user is trying to do. This plan regroups it by task, hides internals behind an Advanced switch, makes the viewport itself interactive, and gives the renderer a status bar that says what it is doing.

---

## 1. What is wrong now

### 1.1 Layout and density

- **One floating window holds everything.** `egui::Window::new("CORNELL / RTX LAB")` at `main.rs:698`, 330 pt wide, positioned at (16,16), over the image. At 800×600 (`captures/validation-ui.png`) it covers the left 45% of the render; the Scene objects window (`editor.rs:48`, default x = screen width − 352) covers most of the rest and spills off the right edge. Between them, about 15% of the image is visible.
- **The two windows overlap in purpose.** Objects live in their own window (`editor.rs:39-208`), but area lights are edited inside the main panel's Lighting header (`main.rs:734` → `editor.rs:209`), and the main ceiling light is a third, different control set (`main.rs:732-733`). Three ways to edit "things in the scene", in two places.
- **Two headers are open by default** ("Reconstruction & rendering" `main.rs:715`, "Display & export" `main.rs:742`), so the first screen is 20+ rows of sliders before the user has done anything. The scene picker, the one thing most people want first, is a plain ComboBox squeezed between buttons (`main.rs:707`).
- **Very long single lines of UI code** (`main.rs:724`, `728`, `732`, `738`: four to seven widgets per line) make the panel hard to reorganise and are part of why it grew as a flat list.
- **Label-on-the-right layout.** egui's default `Slider::text` puts the label right of the value, so the eye zig-zags: slider, number, label. Combined with 140 px sliders this is the "packed together" look in all three captures.
- **Important info in `.small()` text**: RR status (`main.rs:703`), scene description (`708`), the object-window hint (`editor.rs:138`). At 100% DPI this is ~10 px and easy to miss.

### 1.2 Grouping does not match tasks

- **The same field appears twice.** `s.scale` is edited by the "DLSS quality" ComboBox (`main.rs:719`) and by the "Render scale" slider (`main.rs:722`). Dragging the slider to 0.6 makes the combo read "Balanced" although no Balanced mode was chosen.
- **Irrelevant controls stay visible.** With RR on, the native denoiser is bypassed, yet "Native à-trous denoiser", "Denoise strength", "Reuse temporal history" and "Owen-scrambled Sobol (native)" (`main.rs:720-725`) remain enabled. With RR off, "RR model" stays enabled.
- **Post effects filed under Lighting.** "Direct intensity / Indirect intensity" (`main.rs:735`) are AOV recomposition in the display shader (excluded from `transport_key`, `main.rs:86-87`), and an approximation on RR output (README "Current limits"). They belong with exposure/tone map.
- **"Glass casts shadows"** (`main.rs:735`) is a transport shortcut, not a lighting control.
- **Picking a preset silently resets render settings.** `Settings::preset()` (`scene.rs:352-357`) keeps only `scale`, `rr`, `vsync`, `rr_preset` and the camera between Cornell presets; `spp`, `bounces`, `max_samples`, `exposure`, `tonemap`, `denoise` all revert to the preset's values. Nothing in the UI says so.
- **Two different "reset" buttons side by side.** "Reset [R]" restarts accumulation (`main.rs:705`); "Reset all" (`main.rs:713`) replaces the whole scene with `Settings::default()` without confirmation or undo.

### 1.3 Labels that leak internals

| Current label (file:line) | Problem | Proposed |
|---|---|---|
| "RUST / DIRECT3D 12 / DXR 1.1" banner (`main.rs:699`) | Build trivia as the first line | Move to About |
| "N objects · N triangles · N emissive triangles" (`main.rs:701`) | Engine counters | Status bar, Advanced only |
| "DLSS Ray Reconstruction" + "RR model: NVIDIA default / Transformer D / E / RR2 / F" (`main.rs:716-718`) | NGX preset letters | "Denoiser: AI (DLSS Ray Reconstruction)"; model under Advanced as "AI model: Latest (recommended) / Transformer D / Transformer E" |
| "DLSS quality" (`main.rs:719`) | Applies to all reconstruction paths, not only DLSS | "Resolution" with render size shown |
| "Reuse temporal history" (`main.rs:720`) | Filter internals | Advanced: "Blend previous frames (native denoiser)" |
| "Native à-trous denoiser [D]" (`main.rs:720`) | Algorithm name | "Denoiser: Built-in" |
| "Owen-scrambled Sobol (native)" (`main.rs:725`) | Algorithm name | Advanced: "Sample pattern: Low-discrepancy / Random" |
| "Firefly clamp (0 = off)" (`main.rs:725`) | Jargon, magic zero | Advanced: checkbox "Suppress fireflies" + "Max sample brightness" |
| "Sample limit" / "Samples / frame" (`main.rs:724`) | OK-ish, no explanation of cost | "Stop after N samples", "Samples per frame (speed vs. smoothness)" |
| "Room / floor" colour = `s.base` (`main.rs:729`) | Name of a struct field | "Back wall, ceiling & floor" (Cornell) / "Ground" (studio) |
| "Base color (linear)" (`editor.rs:14`) | Colour-space detail in the label | "Color"; linear note in tooltip |
| "IOR" (`editor.rs:19`), "Reflectivity" vs "Metallic" (`editor.rs:16-17`) | Terms without explanation | "Index of refraction" with hint "glass 1.5, water 1.33, diamond 2.42"; tooltips |
| "Kelvin", "Gel" (`main.rs:732`) | Lighting-crew jargon, no unit display | "Color temperature 5200 K", "Color filter" |
| "Lift above floor" / "World Y", "Y stretch" (`editor.rs:170-185`) | Field names | "Height above floor", "Height scale" |
| "A/B [C]" (`main.rs:706`) | Unclear what it captures | "Freeze as reference (C)" and a compare toggle |
| "Pattern: Solid/Checker/…" via `format!("{:?}")` (`editor.rs:23`) | Debug formatting of an enum | Proper labels |

No numeric control shows a unit. FOV and rotations are degrees, distances are scene units (the Cornell box is 2 units wide), light intensity is a multiplier. None of this is on screen.

### 1.4 Ranges and silent clamping (a real bug)

egui 0.31's `Slider` defaults to `SliderClamping::Always`, which clamps the existing value every frame the slider is drawn (`egui-0.31/src/widgets/slider.rs:608`). Several main-panel sliders are narrower than `Settings::validate()` (`scene.rs:456`), so **opening a header can silently rewrite a value loaded from a file, `--set`, or a future preset**, which then restarts accumulation:

| Field | UI range (file:line) | `validate()` range |
|---|---|---|
| `light_intensity` | 0–100 (`main.rs:732`) | 0–2000 |
| `light_size` | 0.05–20 (`main.rs:732`) | 0.05–40 |
| `light_aspect` | 0.25–4 (`main.rs:732`) | 0.1–10 |
| `light_position` x/z, y | ±20, −5–20 (`main.rs:733`) | ±50 |
| `firefly` | 0–50 (`main.rs:725`) | 0–100 |
| `max_samples` | 16–65536 (`main.rs:724`) | 1–1,000,000 |
| `rotate_speed` | ±30 (`main.rs:738`) | ±60 |
| area light `intensity` | 0–100 (`editor.rs:219`) | 0–2000 |
| object `elevation` | −10–20 (`editor.rs:170`) | ±40 |
| `pattern_scale` | 0.1–30 (`editor.rs:36`) | 0.1–100 |

CLAUDE.md says editor ranges were aligned with `validate()`; the main panel and the three editor rows above were missed. Fix in phase 1: every slider gets `.clamping(SliderClamping::Edits)` (existing values survive) and a range that is either the validated range or a comfortable sub-range with the DragValue allowed to go further. Exposure 0–10 linear is also a poor slider: 1.0 sits at 10% of the track; show it in EV stops instead (section 7).

### 1.5 Discoverability

- Shortcuts are listed for some buttons (`[R]`, `[C]`, `[F]`, `[D]`, `[A]`, `[1–8]`) and not others. F12, Space and Tab appear only in the README or in the collapsed Camera header (`main.rs:740`). After Tab hides the UI (`main.rs:907`) nothing on screen says how to get it back.
- **Delete only works while the Scene objects window is open** (`main.rs:910-918`), a hidden rule.
- **Escape quits immediately** (`main.rs:906`), losing an unsaved scene. Escape is the key people press to dismiss a popup.
- Ctrl +/− UI zoom works (egui's built-in `zoom_with_keyboard`) but nobody would know.
- The material library is a ComboBox whose selected text is "Apply material preset…" (`editor.rs:5-13`), a command disguised as a field. No previews.
- The 24 presets are one flat dropdown (`main.rs:707`) with no categories, search or preview. Re-selecting the current preset does nothing (`selectable_value` only fires on change), so there is no "revert to preset".
- The export section tells users to use CLI flags "for true high-resolution renders" (`main.rs:753`) two lines above a "Render high-resolution PNG" button (`main.rs:756`). Contradictory.

### 1.6 Feedback

- **One status line** (`self.status`, drawn at `main.rs:758`) at the bottom of a scrolling window, so usually off-screen. It never clears, and errors share it with success messages.
- **High-res export blocks the UI.** The loop in `redraw()` (`main.rs:~575-590`, `while g.samples < export.max_samples { g.render(...) }`) runs synchronously: the window freezes with no progress for as long as 2048 samples at 4K take.
- Convergence is "N spp" plus the word "Converged" (`main.rs:702`); no progress bar, no target, no time estimate.
- GPU time is one number for the whole frame (`gpu.rs:1077`); no split between tracing, denoising and display, so the user cannot see what a setting costs.
- Changes that restart accumulation (spp, sample limit, any scene edit) give no sign beyond the sample counter dropping.
- Save/Load use a fixed `settings.json` in the working directory (`main.rs:416-440`), overwrite without asking, and give no file picker. PNG/EXR exports overwrite `cornell.png` / `cornell.exr`.
- The window title duplicates the stats (`main.rs:~628`), which is fine, but it is the only place the active denoiser is named in plain words ("DLSS RR" / "À-trous" / "Raw").

### 1.7 Mouse and keyboard gaps

- No hover feedback in the viewport; picking only on click release (`main.rs:841-856`), and every pick force-opens the objects window (`self.show_objects = true`).
- No selection highlight in the image. The selected object is only visible as a list row.
- No direct manipulation: moving an object means typing into DragValues (`editor.rs:156-168`).
- Lights cannot be picked (they are not objects; `pick_object` at `geometry.rs:427` only tests objects), and hidden lights (`visible: false`, the default in studio presets, `presets.rs:105`) are invisible and unclickable.
- No right-click menu: right button is pan only. Middle drag does nothing (only Left and Right are handled, `main.rs:874-884`).
- No undo/redo anywhere. Delete, Reset all, preset switch and Load are all destructive.
- No double-click to focus, no keyboard scene switching, no Ctrl+S / Ctrl+O.

---

## 2. Who uses it and for what

| Task | Typical user moment | Needs on screen | Does not need |
|---|---|---|---|
| **A. Quick look around** | Just launched, wants to see pretty scenes | Big image; scene browser with previews; next/prev; orbit hints; a single Quality choice; convergence progress | Any slider |
| **B. Scene building** | Starting from "Build your own studio" or editing a preset | Outliner (objects + lights + stage + camera); click-to-select in viewport with handles; inspector for the selection; material swatches; undo; save/load with names | Transport internals |
| **C. Image-quality comparison** | Comparing RR vs built-in denoiser, models, render scales, spp; the developer's own regression work | Quality/denoiser/upscaler controls with honest availability; render vs output resolution; per-pass GPU time; sample count/target; AOV views; freeze-reference A/B split; stable keyboard toggles | Scene editing |
| **D. Export** | Finished look, wants a file | Output size, sample budget, format (PNG/EXR), file name and folder, progress, "show in folder" | Most of the rest |

Look (exposure/tone map) is used by A, C and D, so it gets its own tab rather than being buried in Export.

---

## 3. Proposed information architecture

### 3.1 Frame

```
┌ Top bar ─────────────────────────────────────────────────────────────┐
│ scene name ▾ ◀ ▶ │ Quality ▾ │ View ▾ │ ⏸ ↻ │ progress │ Export │ ☰ │
├──────────────────────────────────────────────────┬───────────────────┤
│                                                  │ Side panel        │
│                 viewport                         │ tabs:             │
│   (click/hover/handles/context menu)             │ Scene Render Look │
│                                                  │ Export            │
├──────────────────────────────────────────────────┴───────────────────┤
│ Status bar: state · samples · GPU ms · denoiser · resolution         │
└──────────────────────────────────────────────────────────────────────┘
```

- **Top bar** (`TopBottomPanel::top`, ~34 pt): the things used every minute: scene, Quality, view mode, pause/restart, progress, Export, menu.
- **Side panel** (`SidePanel::right`, resizable 260–480 pt, collapsible with Tab or a chevron). Right side so it does not cover the Cornell box's red wall and so the outliner sits near the window's close button like most DCC tools. Four tabs:
  - **Scene**: outliner (Stage, Camera, Main light, Lights, Objects) on top; inspector for the selection below.
  - **Render**: Quality (section 5), samples, light bounces, Advanced block.
  - **Look**: view mode, exposure, tone map, relight (direct/indirect), compare A/B.
  - **Export**: format, size, samples, file, progress.
- **Status bar** (`TopBottomPanel::bottom`, ~22 pt): section 6.
- **Scene browser**: a large overlay window opened from the top-bar scene name (section 4.2).
- **Tab key** hides side panel and status bar but keeps a thin top bar with a "Show panel (Tab)" hint; a second Tab hides everything (screenshot mode) with a 2-second toast "Press Tab to show controls".

### 3.2 Progressive disclosure

One global **"Show advanced settings"** toggle in the ☰ menu (persisted per user, not in `Settings`). Off by default. When off:

| Tab | Simple (always) | Advanced (hidden) |
|---|---|---|
| Render | Quality; Stop after N samples; Samples per frame; Light bounces | AI model; denoiser strength; blend previous frames; sample pattern; firefly suppression; glass shadows; VSync |
| Look | View (Beauty/…); Exposure (EV); Tone map | Gamma; direct/indirect relight; compare split |
| Scene → object | Name, shape, position, height, size, rotation (yaw), material swatch + colour + surface type | Pitch/roll, per-axis stretch, roughness/metallic/reflectivity/transmission/IOR/emission sliders, pattern |
| Scene → light | On, intensity, colour/temperature, size | Rotation, visibility to camera, aspect |
| Status bar | State, samples, ms | Per-pass ms, triangle/emitter counts, adapter |

Within a section, advanced rows also sit under an inline `CollapsingHeader("More")` so a user who wants one advanced field does not have to flip the global switch.

### 3.3 Labels, units, defaults

- Label left, control right, in a two-column `Grid`; controls fill the column width.
- Every number shows its unit via `.suffix()`: `°`, ` K`, ` spp`, ` units`, `×`, ` EV`, ` %`.
- Every row has a tooltip of one plain sentence; rows that cost performance say so ("Each extra sample per frame adds ~0.4 ms here" when measurable).
- A small **"modified" dot** on rows whose value differs from the loaded preset; clicking it reverts that row. Baseline = the `Settings` captured when the preset/file was loaded (UI-only state).
- Defaults stay as `Settings::default()` (RR on, DLAA, 4 spp, 6 bounces, 4096 cap, ACES). No change to stored defaults.

### 3.4 Field map (current → new home)

| `Settings` fields | New home |
|---|---|
| `rr`, `rr_preset`, `scale`, `denoise`, (new upscaler field) | Render → Quality (section 5); top-bar Quality |
| `spp`, `max_samples`, `bounces` | Render (simple) |
| `denoise_strength`, `temporal`, `sobol`, `firefly`, `glass_shadow`, `vsync` | Render → Advanced |
| `view`, `exposure`, `tonemap` | Look (simple); `view` also in top bar |
| `gamma`, `direct`, `indirect`, `compare`, `split` | Look → Advanced / Compare |
| `cornell_room`, `floor_extent`, `sky`, `left`, `right`, `base`, `background`, `walls`, `floor` | Scene → outliner "Stage" |
| `light_*` | Scene → outliner "Main light" (same inspector layout as area lights) |
| `lights[]` | Scene → outliner "Lights" |
| `objects[]` | Scene → outliner "Objects" + viewport |
| `yaw`, `pitch`, `distance`, `target`, `fov`, `rotate`, `rotate_speed` | Scene → outliner "Camera"; viewport |
| `scene_name` | Top bar, browser |

---

## 4. Layout mockups

### 4.1 Default state (task A), 1440×900

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ ▣ Classic Cornell box ▾  ◀ ▶ │ Quality: Best (AI, native res) ▾ │ View: Beauty ▾ │ ⏸  ↻ │
│                                     ▮▮▮▮▮▮▮▱▱▱ 1,204 / 4,096 spp · 9 s left │ ⇩ Export │ ☰ │
├───────────────────────────────────────────────────────────────┬──────────────────────────┤
│                                                               │ [Scene] Render Look Exp. │
│                                                               │ ──────────────────────── │
│                ┌───────────────────────────┐                  │ ▾ Stage   Cornell box    │
│                │▔▔▔▔▔▔▔▔▔▔ light ▔▔▔▔▔▔▔▔▔▔│                  │ ▸ Camera                 │
│                │█                         ▓│                  │ ▸ Main light   12× 5200K │
│                │█      ┌────┐             ▓│                  │ ▾ Objects (2)   + Add ▾  │
│                │█      │    │             ▓│                  │   👁 Tall block          │
│                │█      │    │    ┌──┐     ▓│                  │   👁 Short block         │
│                │█      └────┘    └──┘     ▓│                  │ ▸ Lights (0)    + Add    │
│                └───────────────────────────┘                  │ ──────────────────────── │
│                                                               │ Nothing selected.        │
│   drag: orbit · right-drag: pan · wheel: zoom · click: select │ Click an object in the   │
│   (hint fades after first interaction)                        │ view or in the list.     │
├───────────────────────────────────────────────────────────────┴──────────────────────────┤
│ ● Rendering  1,204 / 4,096 spp · 3.9 ms GPU (trace 1.6 · AI denoise 2.1 · display 0.2)   │
│   AI denoiser (DLSS RR, model F) · 1440×900 native                       RTX 4070 Ti S.  │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Scene browser (scales to hundreds)

Opened by clicking the scene name, **Ctrl+P** or **/** (Ctrl+O stays "open file"). Large centred window, ~80% of the screen, dimmed backdrop.

```
┌ Scenes ───────────────────────────────────────────────────────────────────────── ✕ ┐
│ 🔍 [ glass_______________ ]   Sort: Category ▾   Size: ▪▪ ▾      24 of 240 shown  │
├────────────────────┬────────────────────────────────────────────────────────────────┤
│ ★ Favorites     3  │ Material studies ─────────────────────────────────────────────  │
│ 🕘 Recent        5  │ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐            │
│ 📁 My scenes    12  │ │ [thumb]  │ │ [thumb]  │ │ [thumb]  │ │ [thumb]  │            │
│ ──────────────────  │ │          │ │          │ │          │ │          │            │
│ All             240 │ └──────────┘ └──────────┘ └──────────┘ └──────────┘            │
│ Cornell box      9  │  Glass       Glass lab     Mirror +     Material                │
│ Material studies 31 │  ★           ~4 ms         glass        matrix                  │
│ Classic CG      18  │                                                                 │
│ Lighting        22  │ Cornell box ────────────────────────────────────────────────    │
│ Sculpture       40  │ ┌──────────┐ ┌──────────┐                                       │
│ Architecture    25  │ │ [thumb]  │ │ [thumb]  │                                       │
│ Empty stages     2  │ └──────────┘ └──────────┘                                       │
│                     │                                                                 │
├────────────────────┴────────────────────────────────────────────────────────────────┤
│ Glass laboratory — Refraction, absorption and rough transmission across boxes,       │
│ spheres, cones and a torus. 14 objects · 3 lights · ~4.1 ms at 1440×900               │
│                       [ Open (Enter) ]  [ Open, keep my camera ]  [ Cancel (Esc) ]   │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- **Search** matches name, category, tags and description, case-insensitive, as you type. Arrow keys move the highlight across cards; Enter opens; Esc closes.
- **Categories** for the current 24: Cornell box (0–8); Material studies (9, 14, 16); Classic CG (10 weekend field, 11 Whitted, 12 teapot, 13 bunny); Lighting (15 RGB shadows, 17 mirror corridor, 21 neon); Sculpture & architecture (18, 19, 20, 22); Empty stages (23).
- **My scenes**: `.json` files in a `scenes/` folder next to the exe (plus whatever the user saves), each with a sibling `.png` thumbnail written at save time.
- **Virtualised grid**: `ScrollArea::vertical().show_rows(...)` over rows of cards, so only visible thumbnails are loaded; thumbnails off screen for a while are dropped (LRU, ~64 resident).
- **Hover** shows the description, object/light counts and a measured cost hint (from a generated table, see phase 3).
- **Unsaved edits**: opening a scene pushes an undo snapshot, so Ctrl+Z gets the old scene back. No modal "are you sure?" needed.
- "Open, keep my camera" exposes the camera-keeping rule that is now implicit in `scene.rs:355`.

Data model needed: `PRESETS: [&str; 24]` plus two `match` statements (`presets.rs:138`, `:180`) will not scale. Replace with a registry: `struct PresetInfo { id: &'static str /*stable slug*/, name, category, tags: &[&str], description, build: fn(&mut Settings) }`. Keep numeric `--preset N` working through the slice order and add `--preset slug`. This touches `scene.rs`/`presets.rs` and must be coordinated with the main session (phase 3).

### 4.3 Object selected (task B)

```
┌──────────────────────────────────────────────────────────────┬──────────────────────────┐
│                                                              │ [Scene] Render Look Exp. │
│                  ┌ Tall block ┐                              │ ▾ Objects (2)   + Add ▾  │
│                  ╔═══════════╗   ← selection outline         │   👁 Tall block       ◀  │
│                  ║     ↑     ║     (accent colour)           │   👁 Short block         │
│                  ║     │     ║                               │ ──────────────────────── │
│                  ║   ◯─┼─◯   ║   ← yaw ring on the floor     │ Tall block          ⋯    │
│                  ║  ←──●──→  ║   ● drag: move on floor       │ Shape     [Box      ▾]   │
│                  ╚═══╤═══════╝   ↑ drag: height              │ ─ Placement ──────────── │
│                      ◢ size     ◢ drag: size                 │ Position  X [ 0.38]  Z [-0.38] units
│        hovered → ┌ ─ ─ ┐ (thin outline)                      │ On floor  [✓]  Height 0.00 │
│                  └ ─ ─ ┘                                     │ Size      ━━━●━━  0.62   │
│                                                              │ Rotation  ━━●━━━  -14°   │
│                                                              │ ▸ More (tilt, stretch)   │
│                                                              │ ─ Material ───────────── │
│                                                              │ ▢▢▢▢▢▢ ▢▢▢▢▢▢  swatches  │
│                                                              │ Type  (Matte|Plastic|    │
│                                                              │        Metal|Glass|Light)│
│                                                              │ Color [■]  Roughness ━●━ │
│                                                              │ ▸ More (IOR, pattern…)   │
├──────────────────────────────────────────────────────────────┴──────────────────────────┤
│ ● Restarted (object moved) · 12 / 4,096 spp · 4.1 ms     Ctrl+Z undo · Del delete · H hide │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

Handle behaviour maps one-to-one onto the existing object model, so no new `Settings` fields are needed:

| Handle | Drag does | Field |
|---|---|---|
| Body / centre disk | Move on the floor plane under the cursor (ray–plane hit at the object's base height) | `position[0]`, `position[1]` (x, z) |
| Vertical arrow | Height | `elevation` (with `grounded` → "height above floor") |
| Floor ring | Yaw, snaps to 15° with Shift | `rotation` |
| Corner square | Uniform size, log-scaled | `size` |
| Alt + drag body | Duplicate, then move the copy | push clone |

Lights get the same treatment: their quad outline is drawn as an overlay even when `visible = false`, clickable, with move/height handles writing `position`, and the corner handle writing `size`.

**Right-click without drag** (< 3 px, the same threshold as pick, `main.rs:843`) opens a context menu:

```
┌──────────────────────────┐
│ Tall block               │
│ Focus              F     │
│ Duplicate          Ctrl+D│
│ Hide               H     │
│ Delete             Del   │
│ ─────────────────────────│
│ Material         ▸ Porcelain, Chrome, Gold, …
│ Rest on floor      ✓     │
│ ─────────────────────────│
│ Add object here  ▸ Box, Sphere, …     (on empty floor)
│ Add light here           │
└──────────────────────────┘
```

Right-drag stays pan; middle-drag also pans.

### 4.4 Render tab and Look tab

```
┌ [Render] ───────────────────────┐   ┌ [Look] ───────────────────────────┐
│ ─ Quality ───────────────────── │   │ View      [Beauty         ▾] 1–8  │
│ Denoiser  (AI | Built-in | Off) │   │ Exposure  ━━━━●━━━━  +0.0 EV      │
│ Upscaler  [— native res — ▾]    │   │ Tone map  [ACES filmic    ▾]      │
│ Resolution[DLAA 100%      ▾]    │   │ ▸ More                             │
│   renders 1440×900 → 1440×900   │   │   Gamma         2.20               │
│ ─ Sampling ──────────────────── │   │   Direct light  1.00×  (approx. on │
│ Stop after  [4096] spp  ∞ □     │   │   Indirect      1.00×   AI output) │
│ Per frame   ━━●━━━  4 spp       │   │ ─ Compare ──────────────────────── │
│   faster ◀────────▶ smoother    │   │ [Freeze reference (C)]            │
│ Light bounces ━━●━━  6          │   │ Show split [✓]  ━━━●━━━  50 %      │
│ ▸ Advanced                      │   │   (drag the split line in view)   │
└─────────────────────────────────┘   └───────────────────────────────────┘
```

### 4.5 Export tab (task D)

```
┌ [Export] ──────────────────────────────────┐
│ Format   (PNG 8-bit | EXR linear HDR)       │
│ Size     (Window | 2× | 4× | Custom)        │
│          2880 × 1800  (max 4096 long side)  │
│ Quality  [256 spp ▾]  ~14 s on this GPU     │
│ File     [cornell-2026-10-29-1432.png] [📁] │
│ □ Include UI                                │
│ [ Render & save ]                           │
│ ▮▮▮▮▮▮▱▱▱▱ 61 %  157/256 spp  [Cancel]      │
│ Last: cornell-…-1432.png  [Show in folder]  │
└─────────────────────────────────────────────┘
```

"Window" size with the current accumulation saves instantly (today's PNG/EXR buttons). Any other size runs the high-res path (today's `actions.high_res`), made non-blocking in phase 4. The misleading CLI hint (`main.rs:753`) goes into a tooltip: "For sizes above 4096, use the command line: …".

---

## 5. Quality and upscaler section

### 5.1 One control for simple mode

The top bar and the top of the Render tab show a single **Quality** dropdown. Each entry is a bundle; the dropdown lists them with the resulting render resolution and an honest availability note:

```
Quality ▾
 ● Best          AI denoiser, native resolution (DLAA)        1440×900   RTX
 ○ Balanced      AI denoiser, 67 % resolution                  960×600    RTX
 ○ Fast          AI denoiser, 50 % resolution                  720×450    RTX
 ○ Fastest       AI denoiser, 33 % resolution                  480×300    RTX
 ─────────────────────────────────────────────────────────────────────────
 ○ Built-in      Built-in denoiser, native resolution         1440×900   any GPU
 ○ Built-in fast Built-in denoiser + FSR, 67 %                 960×600    any GPU
 ○ Reference     No denoiser, native resolution (unbiased)    1440×900   any GPU
 ─────────────────────────────────────────────────────────────────────────
   Custom…       (shown when Advanced values don't match a bundle)
```

On a non-RTX GPU the RR rows are disabled (greyed, still listed) with hover text giving the real reason from capability detection (e.g. "DLSS Ray Reconstruction needs an NVIDIA RTX GPU. This GPU: AMD Radeon …", or the NGX error code if init failed on an RTX card), and "Built-in" becomes the default selection. "Built-in fast" picks the best available temporal upscaler in order DLSS SR → XeSS → FSR, and its label names the one chosen.

### 5.2 Advanced: three explicit choices

```
Denoiser    (AI (DLSS RR) | Built-in | Off)
Upscaler    [None (native) | DLSS Super Resolution | AMD FSR | Intel XeSS | Bilinear | Nearest ▾]
              └ hidden when Denoiser = AI, because RR upscales itself
Resolution  [Native 100% | Quality 67% | Balanced 58% | Performance 50% | Ultra 33% | Custom ▾]
              Custom → slider 25–100 %, snapped labels at the named modes
              Shows: "renders 960×600 → outputs 1440×900"
AI model    [Latest (F, recommended) | Transformer D | Transformer E | NVIDIA default]   (AI only)
```

Rules the UI enforces (so impossible combinations cannot be chosen, rather than validated afterwards):

| Denoiser | Upscaler list | Resolution | Notes shown |
|---|---|---|---|
| AI (RR) | hidden (RR does it) | all modes; 100% = DLAA | "AI model" visible |
| Built-in | None, DLSS SR, FSR, XeSS, Bilinear, Nearest | 100% forces Upscaler = None | Built-in strength under More |
| Off | same as Built-in | same | "Unbiased reference: use for comparisons" |

Availability per backend comes from the GPU layer, not guessed in the UI. The UI wants one struct from `Gpu`, filled at startup and refreshed if a backend fails at runtime:

```rust
pub struct Backend { pub available: bool, pub reason: String /* "", "needs NVIDIA RTX", "nvngx_dlss.dll missing", "init failed 0xBAD00005" */ }
pub struct Capabilities { pub rr: Backend, pub dlss_sr: Backend, pub fsr: Backend, pub xess: Backend }
```

Today only `rr_status: String` exists (`gpu.rs:115`, set at `:400`, `:795-816`, `:924`). If RR fails mid-session (evaluation error at `gpu.rs:924`), the status bar turns amber: "AI denoiser failed (0x…), using built-in" and the Quality dropdown shows the fallback actually in use, not the requested one.

**Mapping to `Settings`.** The Quality bundles only write existing fields (`rr`, `scale`, `denoise`, `rr_preset`) plus whatever upscaler field the main session adds. The bundle shown in the dropdown is derived from the fields each frame (a pure function `quality_bundle(&Settings) -> Option<Bundle>`), never stored, so loaded files and `--set` show correctly and nothing new enters `transport_key`. The duplicate-control bug (combo + slider both writing `scale`, `main.rs:719/722`) goes away because there is exactly one Resolution control with a Custom sub-slider.

Two honest notes to keep in tooltips:

- With a still camera, the built-in path accumulates at render resolution, so below 100% the converged image is capped at that resolution; temporal upscalers recover some detail only through jitter. Export always renders at 100% (`main.rs:575`, `export.scale = 1.`), which the Export tab should state.
- Direct/indirect relighting is an approximation on AI output (README limits); the Look tab says so next to those sliders.

---

## 6. Status and feedback

### 6.1 Status bar (always visible unless in screenshot mode)

```
● Rendering  1,204 / 4,096 spp  ▮▮▮▱  ~9 s │ 3.9 ms GPU  ·  250 fps │ AI denoiser · DLAA · 1440×900 │ 16 obj · 194k tris
```

| Element | Source | Notes |
|---|---|---|
| State dot + word | `paused`, `g.samples`, `max_samples`, `rotate`, reset events | Rendering (green), Converged ✓ (blue), Paused ⏸ (grey), Restarted (yellow flash 1 s with cause: "camera moved", "object moved", "quality changed"), Exporting (purple), Fallback (amber) |
| Progress | `g.samples / s.max_samples` | Hidden when the cap is "∞"; ETA = remaining / (spp per frame × frames per second), smoothed |
| GPU ms | `g.gpu_ms` | Advanced: per pass. Needs more timestamp pairs in `gpu.rs` (today one pair, `gpu.rs:1077`): trace / denoise-or-RR / upscale / display / UI |
| Denoiser | `g.rr_active`, `s.denoise`, upscaler | Plain words: "AI denoiser (DLSS RR, model F)", "Built-in denoiser + FSR 67 %", "No denoiser (raw samples)" |
| Resolution | `g.rw×g.rh → g.width×g.height` | Identical when native |
| Scene size | `objects.len()`, `g.triangle_count`, `g.emitter_count` | Advanced only |

What the denoiser is doing deserves one more line when Advanced is on, using the flags that already decide pass execution (`camera_moved`, `history_valid`, `integrated_valid` in `Gpu`): "Built-in: reprojecting history (camera moving)" / "integrating, 37 frames of history" / "AI: history reset (scene edited)". These need to be exposed read-only from `Gpu`.

### 6.2 Toasts

Bottom-right stack, newest on top, drawn in an `egui::Area` with `Order::Foreground`:

- Success: "Saved cornell-2026-10-29-1432.png (2880×1800, 256 spp)  [Show in folder]" — 4 s.
- Info: "Native denoiser off (D)", "View: Normals (2)", "Auto orbit on (A)" for every single-key toggle, so key presses are never silent — 1.5 s.
- Error: "Load rejected: Invalid light" — stays until dismissed, with [Details].
- Undoable destructive actions: "Deleted Tall block  [Undo]" — 5 s.

Implementation: replace `self.status: String` with `toasts: Vec<Toast { text, kind, action: Option<ToastAction>, until: Instant }>`. Existing writers (`main.rs:420-440`, `:563-574`, `:592`) push toasts instead. "Show in folder" = `std::process::Command::new("explorer").arg(format!("/select,{}", path))`, no new crate.

---

## 7. egui implementation notes

### 7.1 Structure

- New module `src/ui/` with `mod.rs` (`UiState`, entry `fn ui(app, ctx)`), `topbar.rs`, `panel.rs` (tabs), `scene_tab.rs` (outliner + inspector; absorbs `editor.rs`), `render_tab.rs`, `look_tab.rs`, `export_tab.rs`, `browser.rs`, `viewport.rs` (hover, selection overlay, handles, context menu), `status.rs` (status bar, toasts), `theme.rs`, `widgets.rs` (property row, segmented control, unit sliders).
- Add the new files to `tests::ui_strings_have_glyphs` (`main.rs:984-1004`); it lists files explicitly. Every icon glyph chosen (⏸ ▶ ↻ ☰ ★ 👁 📁 ⇩ …) must pass it. If an emoji is missing from egui's default fonts, use a plain symbol (egui's default includes many from the emoji-icon-font subset; the test decides).
- Rewrite the long one-liners as normal formatted code while moving them.

### 7.2 Panels instead of windows

- `TopBottomPanel::top("topbar")`, `TopBottomPanel::bottom("status")`, `SidePanel::right("panel").resizable(true).default_width(320.).width_range(260. ..=480.)` must be added before any `Area`/`Window` each frame.
- The 3D image is still drawn to the whole backbuffer (texture 10 copied to the backbuffer, then egui on top), so the side panel covers part of the render. Phase 1 accepts that (panel fill slightly translucent, ~92% alpha). Phase 3 option: shift the image so the subject centres in the free area, either by an off-centre projection (lens shift in `Settings::camera()`, which is used by picking, motion vectors and RR, so all stay consistent) or by rendering to the central rect size and copying with an x offset. Both touch `scene.rs`/`gpu.rs`; coordinate.
- `egui_winit` already reports `consumed` for events over panels, and `is_pointer_over_area()` is true over panels, so the existing orbit/pick code in `window_event` (`main.rs:841-893`) keeps working in phase 1 unchanged.
- Scene browser: `egui::Window::new("Scenes").collapsible(false).resizable(true).anchor(Align2::CENTER_CENTER, …)` plus a full-screen dim `Area` behind it with `Order::Middle`.

### 7.3 Widgets and patterns

- **Property rows**: `egui::Grid::new(id).num_columns(2).spacing([12., 6.]).striped(true)`; a helper `row(ui, label, tooltip, |ui| widget)` adds the label with `on_hover_text`, the widget, and the "modified" dot. Set `ui.spacing_mut().slider_width = ui.available_width() - 60.` inside the grid so sliders fill the column.
- **Sliders**: always `.clamping(egui::SliderClamping::Edits)`, `.suffix(...)`, `.logarithmic(true)` for sizes/intensities/sample counts. Exposure as EV: `Slider::new(&mut ev, -6. ..=3.32)` on a local `ev = exposure.log2()`, written back as `2f32.powf(ev)` only when `changed()`; zero exposure displays "off".
- **Segmented controls** (Denoiser, Format, Surface type): a horizontal row of `ui.selectable_value` inside `egui::Frame::group`. No extra crate.
- **ComboBox with disabled entries**: inside `show_ui`, `ui.add_enabled(backend.available, egui::SelectableLabel::new(sel, label)).on_disabled_hover_text(&backend.reason)`.
- **CollapsingHeader** for "More" blocks, with `id_salt` per section so open state persists in egui memory.
- **Outliner**: `ScrollArea::vertical().show_rows(ui, row_h, n, …)` for up to 256 objects + 32 lights; each row = visibility toggle + `SelectableLabel`; `response.context_menu(...)` for the same menu as the viewport; double-click focuses (`s.focus_object`, `geometry.rs:418`).
- **Material swatches**: a `Grid` of 12 small buttons, each a filled rect painted with the library colour and a tiny gloss highlight (no textures needed); hover shows the name. Pre-rendered material ball thumbnails can replace them once textures work.
- **Keyboard**: read shortcuts with `ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)))` inside `ui()`, so egui text fields get first refusal. Keep the bare-key handlers in `window_event` for now (they already skip when `wants_keyboard_input()`), then migrate.

### 7.4 Viewport overlay and picking

- Draw overlays (selection outline, hover outline, handles, light quads, split line for A/B) with `ctx.layer_painter(LayerId::new(Order::Background, Id::new("viewport")))`: above the 3D image, below panels and windows.
- Projection: the same `s.camera(aspect)` + `project_point3` used by the self-test pick check (`main.rs:538-550`). Outline = projected convex hull of the object's world AABB from `geometry::object_geometry` (cache per object, invalidate with `geometry_key`). Cheap and exact enough; a pixel-accurate GPU outline from the object-id buffer (u19) is a later option that needs a `selected_id` in `Constants` (must mirror `common.hlsli`).
- Hover: running `pick_object` (CPU Möller–Trumbore over all triangles, `geometry.rs:427`) on every mouse move is too slow for the 194k-triangle chess scene. Options: (a) throttle to 20 Hz and only when the pointer is still; (b) read back one texel of the object-id texture (u19) at the cursor, one frame late, via a 4-byte readback added to `render()`. (b) is exact and essentially free; it touches `gpu.rs`.
- Handle dragging: hit-test handles in screen space first; if a handle is under the pointer on press, the drag edits the object instead of orbiting. In phase 2 this lives in `window_event` (check before `self.drag = Some(button)`, `main.rs:858`). Phase 3 moves all viewport input into an egui `Area` covering the central rect with `Sense::click_and_drag()`, which gives `dragged_by`, `double_clicked`, `context_menu` and `hovered` for free and removes the hand-written drag distance logic.
- Every drag frame changes `geometry_key` → `rebuild_scene` (new BLAS/TLAS). Measure on the chess and bunny scenes; if it hitches, update geometry at most every other frame during a drag, or move the object via a TLAS instance transform later (a renderer change, out of scope here).

### 7.5 Thumbnails need a texture path in `gpu.rs`

Today `update_atlas` rejects any texture except the font atlas (`gpu.rs:1091-1094`, `ensure!(*id == egui::TextureId::Managed(0))`) and `draw_gui` binds heap slot 32 once for every mesh (`gpu.rs:1190`), ignoring `mesh.texture_id`. Calling `ctx.load_texture` or `egui::Image` with any other id would fail the frame.

Two ways to add textures:

1. **egui-managed textures (recommended for PNG thumbnails).** UI code calls `ctx.load_texture(name, ColorImage, TextureOptions::LINEAR)`, egui emits `TextureId::Managed(n)` in `textures_delta.set` and later in `.free`. `gpu.rs` changes:
   - `atlas: Option<Texture>` → `textures: HashMap<egui::TextureId, (Texture, u32 /*heap slot*/, Vec<u8>, [usize;2])>`; generalise the existing patch-and-upload code per id; honour `delta.free` (release after the fence, i.e. push to `keep`).
   - `draw_gui`: record `mesh.texture_id` per draw and call `SetGraphicsRootDescriptorTable(4, self.gpu(slot))` when it changes.
   - Heap slots: the shader-visible heap has 64 descriptors (`gpu.rs:242`); 0–24 are UAVs, 32 is the atlas, so 25–31 and 33–63 are free (38). Raise `NumDescriptors` to 512 and use a free list for SRV slots ≥ 32. No root signature change: slot 4 is already a 1-descriptor SRV table.
   - `gui.hlsl` is unchanged; egui `ColorImage` pixels are premultiplied sRGB like the font atlas, so blending matches.
2. **`TextureId::User(n)` (for renderer-owned images).** Use when the GPU already has the image, e.g. a live small copy of texture 10 for "save current view as thumbnail" or showing the frozen A/B reference (u13) in a picture-in-picture. `gpu.rs` creates the SRV itself and registers the id in the same map; UI just draws `egui::Image::new((TextureId::User(n), size))`. Same draw-path change as above.

Thumbnail source: a `tools/thumbnails.ps1` that runs `cornell-rtx.exe --headless --preset N --width 320 --height 200 --frames 64 --capture assets/thumbs/<slug>.png` per preset (the CLI already does all of this). Load with the `image` crate (already a dependency with `png`). Missing thumbnail → placeholder card painted with the category colour and the scene's initial. User scenes: on save, downscale the current output (`capture()` readback path) to 320×200 and write `<name>.png` next to `<name>.json`. Generate thumbnails lazily on a worker thread? Not needed: 320×200 PNG decode is ~1 ms; decode on demand for visible cards, a few per frame.

### 7.6 Theming and DPI

- `theme.rs::apply(ctx)` once at startup, replacing `egui.set_visuals(egui::Visuals::dark())` (`main.rs:260`): dark base; one accent colour constant (today two different cyans: `Color32::from_rgb(93,194,211)` at `main.rs:699` and `(100,210,220)` at `editor.rs:57`); `spacing.item_spacing = [8,6]`, `interact_size.y = 24`, `button_padding = [8,4]`; text styles Body 14, Small 12 (minimum for anything informative), Heading 16, Monospace 13 for numbers in the status bar; panel fill with slight transparency over the render.
- DPI: `egui_winit::State::new(..., Some(window.scale_factor()))` (`main.rs:806`) and `draw_gui` multiplying by `ppp` already handle per-monitor scale. Add a "UI size" setting (90/100/125/150 %) in ☰ that calls `ctx.set_zoom_factor`, and mention Ctrl +/− in the shortcuts sheet. Persist it with the other UI prefs.
- Persist UI prefs (advanced toggle, panel width, tab, UI size, favorites, recent) in `%APPDATA%\CornellRTX\ui.json`, separate from `settings.json`.

### 7.7 Keep `Settings` and dirty tracking untouched

- **No UI state in `Settings`.** `transport_key` (`main.rs:80-108`) serialises *every* field except an explicit exclusion list, so any new field added to `Settings` restarts accumulation and resets RR history whenever it changes. A `ui_tab` or `advanced` field in `Settings` would restart the render on every tab click. All new state goes in `UiState` inside `App` (selection, hover, tab, browser open/search/scroll, toasts, undo stack, baseline, export job, prefs). Move `selected_object`, `add_shape`, `object_search`, `show_objects`, `export_scale`, `export_samples` (`main.rs:71-78`) into it.
- The UI keeps writing the same fields in the same way, so `geometry_key` / `material_key` / `transport_key`, `reconfigure_rr` on `rr`/`rr_preset` change (`main.rs:474-478`), and the camera-only reset rule are unchanged.
- **Undo** is a UI concern: `Vec<Settings>` snapshots (≤ 100, a `Settings` with 256 objects is a few tens of KB). Push on edit *end* (`Response::drag_stopped()`, `lost_focus()`, button clicks, preset/file load, delete), not every frame. Undo assigns `app.settings = snapshot`; the dirty keys then do the right GPU work. Skip camera-only changes so orbiting does not flood the stack.
- The self-test script mutates `self.settings` directly in `redraw()` and does not go through the UI, so it is unaffected; `--capture-ui` will capture the new layout. Keep `ui()` free of side effects beyond `Settings`/`Actions` writes so headless runs (which skip nothing in `ui()` except when `show_ui` is false) stay deterministic.
- The new upscaler field the main session is adding should stay *in* `transport_key` (a change must reset history) and needs its own reconfigure hook next to the `rr` one; that is renderer work, not UI.

---

## 8. Phased roadmap

### Phase 1: Regroup and relabel (1 day, shippable) — done 2026-10-30

Built as `src/ui.rs` plus inspectors in `editor.rs`, not the multi-file `src/ui/` folder of section 7.1. The modified-row dots of 3.3 and the persisted UI prefs of 7.6 are left for later phases.

Scope, `main.rs` `ui()` and `editor.rs` only, no `gpu.rs`/`scene.rs` changes:
- Replace the floating window with top bar + right `SidePanel` with Scene / Render / Look / Export tabs; status bar with state, samples/cap progress bar, GPU ms, denoiser in words, render→output resolution.
- Move the Scene objects window and light editor into the Scene tab (outliner + inspector). Main light and area lights in one "Lights" list.
- Fix the clamping bug: `SliderClamping::Edits` everywhere; ranges from the table in 1.4.
- Single Resolution control (remove the duplicate scale slider); hide/disable controls that do not apply (native-denoiser rows when RR is on, RR model when off).
- Plain-language labels and units per 1.3; tooltips; "Show advanced settings" toggle.
- Grouped preset ComboBox (category separators) and prev/next buttons in the top bar.
- Toasts replacing `self.status`; toasts for single-key toggles.
- Escape no longer quits (closes popups/deselects); Delete works whenever something is selected.
- Theme constants, Body/Small sizes.
Risks: low. Long one-line UI code makes the move tedious but mechanical. Merge conflicts with the main session's Quality work in `ui()`: agree that phase 1 leaves a stub `render_tab::quality(ui, s, caps)` the main session fills, or land phase 1 after the upscaler UI lands. Verify with `cargo test` (glyph test), `--self-test`, `--capture-ui` at 800×600 and 1440×900, and 150% DPI.

### Phase 2: Viewport interaction and undo (2–3 days) — done 2026-10-30

Undo / redo, gizmos and G / R / S came with roadmap item 3b; the rest with the phase 2 batch (CLAUDE.md "Viewport interaction"). Not done: Alt-drag duplicate (Shift+D instead), hover outlines for lights, silhouette outlines (phase 5).

- Undo/redo stack (Ctrl+Z / Ctrl+Y) and Ctrl+D, H, Del, F / double-click focus, Home reset camera, PageUp/PageDown scenes, middle-drag pan, F1 shortcuts sheet.
- Selection outline and hover outline (projected AABB hull, throttled CPU hover pick).
- Drag handles for move / height / yaw / size; Alt-drag duplicate; light quads drawn and pickable with move handles. **Superseded by roadmap item 3b: full Blender-style move / rotate / scale gizmos and G / R / S modal keys.**
- Right-click context menu (viewport and outliner).
- Picking no longer force-opens anything; it selects and switches the panel to Scene.
Risks: medium. Handle vs orbit input conflicts in `window_event`; BLAS rebuild cost while dragging on heavy scenes (measure chess/bunny; throttle if needed); self-test frame 138 pick check must still pass.

### Phase 3: Scene browser with thumbnails (3–4 days) — done 2026-10-30

The browser and thumbnails came with roadmap item 4; favourites, recent, Ctrl+S / Ctrl+Shift+S / Ctrl+O and `--preset` slugs with UI phases 3–5 (CLAUDE.md). Not done: the `PresetInfo` registry refactor (slugs instead).

- `gpu.rs` multi-texture egui path (managed + user ids, heap to 512, per-draw binding, `delta.free`).
- Preset registry (`PresetInfo` with slug, category, tags, description, builder) replacing `PRESETS`/`description()`/`apply()` matches; `--preset slug`; numeric indices kept.
- `tools/thumbnails.ps1`; `assets/thumbs/`; browser window with search, categories, favorites, recent, My scenes folder, virtualised grid, keyboard navigation; per-scene cost hints from a generated JSON.
- Named Save / Save as / Open (Ctrl+S / Ctrl+Shift+S / Ctrl+O) into `scenes/`, thumbnail on save. A native file dialog via `windows` crate `IFileOpenDialog` (needs `Win32_UI_Shell` + `Win32_System_Com` features) or the `rfd` crate.
Risks: medium. Descriptor-heap and resource-lifetime bugs in `gpu.rs` (run `--debug`); registry refactor touches `scene.rs`/`presets.rs` and the self-test's preset cycling (frames 10–125) and randomize (presets 0–22); coordinate with the main session.

### Phase 4: Quality UX, export and diagnostics (2–3 days, after the upscaler backends land) — done 2026-10-30

Quality bundles in the top bar, per-pass GPU timestamps and a time-to-cap estimate, the high-res still as a non-blocking job (rendered at its own size, as the video export does), Show in folder, a draggable A/B split. Not done: a `Capabilities` struct with per-backend reasons, the "what the denoiser is doing" line, file name templates.

- Quality bundles and the Advanced Denoiser/Upscaler/Resolution/Model controls of section 5, driven by a `Capabilities` struct from `Gpu` with per-backend reasons; runtime fallback shown honestly.
- Per-pass GPU timestamps; "what the denoiser is doing" line from `Gpu` flags.
- Non-blocking high-res export: turn the `while` loop (`main.rs:~575`) into a job advanced a few frames per `redraw()`, with progress and Cancel; file name templates and Show in folder.
- A/B split line draggable in the viewport.
Risks: medium–high. Export resizes the swapchain (`g.resize(ew, eh, …)`), so a non-blocking export either shows a scaled preview during export or needs an off-screen target set; the latter is renderer work. Upscaler availability semantics depend on the main session's API.

### Phase 5 (optional): Polish (2 days) — partly done 2026-10-30

Done: persisted prefs (favourites, recent, UI size, advanced), first-run hint. Not done: lens shift to centre the subject, GPU silhouette outlines, material / light thumbnails, a UI-size setting beyond Ctrl +/−.

- Centre the subject in the free viewport area (lens shift or offset render rect).
- GPU selection outline from the object-id buffer; object-id texel readback for exact hover.
- Material ball thumbnails for the library; pre-rendered light/HDRI previews if added.
- UI-size setting, persisted prefs, first-run hint overlay.
Risks: low–medium; the lens-shift change touches `Settings::camera()`, which feeds RR motion vectors, so rerun `tools/rr_stability.py`.
