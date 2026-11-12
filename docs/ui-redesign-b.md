# UI redesign: layout B (tool rail + inspector)

Owner's choice, 2026-11-10. Mockups: the design canvas at https://claude.ai/artifact/9qAa5yvJStbGphdetrRNL5 (private to the owner). The "Option B in detail" row holds B-TopBar, B-Scenes, B-Assets, B-Render, B-Look, B-Lights, B-Export and B-Timeline.

**Status:** built 2026-11-12 (engine-notes "UI layout B"); the open tails are in the roadmap.

**Order:** do the egui upgrade first (`docs/egui-upgrade-plan.md`, 0.31 → 0.37), then this. B uses 0.32+ features: menus that close on click, Atoms for icon-plus-text buttons, `Panel`, and `content_rect`.

This spec merges four control maps (Chrome, Render/Look/Export, Scene, Windows/overlays) and a completeness critic, all generated from the code in `caustica-parity` on 2026-11-10. Where the maps disagreed, the decision below is final, and the critic's findings are folded in.

## 1. Layout

| Area | Contents |
|---|---|
| Top bar (one row, never wraps) | ☰ Menu · scene combo with ‹ › · promoted buttons (§2) · Pause, Restart, compact spp · promoted buttons · Quality combo · View combo · GPU/CPU toggle |
| Tool rail (left, 44 px) | Select (tooltip mentions B), Move, Rotate, Scale · Walk (right-click: Walk avatar), Frame selection (tooltip: F, Home, double-click), Add (opens the outliner's + Add menu plus "Assets…") · at the bottom: Timeline toggle (tooltip lists I, Shift+I, Alt+I, K, Shift+←/→) |
| Viewport overlay (top left) | Space, Pivot, Snap. Shown **whenever something is selected or a transform runs**, not tied to the rail tool, because G/R/S work under Select too (main.rs:3175, overlay.rs:316–323). The Pivot combo moves here from the top bar. |
| Right column | Outliner (top split) over a vertical tab strip: Render, Look, Lights, Object, Material, Export. Sections are collapsible and their headers show value previews. |
| Timeline panel | Bottom, **under the viewport only** (§6). |
| Status bar | State, spp, ETA, resolution, upscaler, GPU time; "Exporting" while an export runs. |

### Right-column sizing (critic)
- Default width is 370 pt (prefs.rs:69); raise the minimum from 320 to 360 pt (chrome.rs:707). The 36 pt tab strip leaves about 330 pt for properties.
- Lay out the right column and the rail **before** the timeline panel (today ui.rs:1756–1757 runs the timeline full width).
- The outliner is a draggable top split saved in prefs: default 30 % of the column, clamped between 4 rows and 45 %.
- On Render, Look and Export the outliner folds to one line (selection name plus filter) that opens on click. It also folds when the column is shorter than 520 pt.
- Below 400 pt of width the tab strip turns into a horizontal row above the properties.
- All sizes in points so Ctrl +/− zoom (ui.rs:1891) scales them.

## 2. Top-bar buttons (owner asked for more on the top bar)

**Chosen: B3 "Pin your own"** (owner, 2026-11-10). B1 and B2 on the B-TopBar board are the rejected alternatives.

- **Defaults:** the B1 set: Scenes, Assets, Compare, Save image.
- **Pinning:** right-clicking any Menu item or palette entry shows "Pin to top bar", and right-clicking a pinned button shows "Unpin" and "Move left/right".
- **What can be pinned:** commands and windows, which are the palette's `Act::Do(Do)` and `Act::Window(Win)` (palette.rs:26, :41). Pins reuse the palette's labels and `App::run_palette`, so there is no second dispatch path. `Act::Go` (jump to a setting) holds a `&'static str`, so it isn't pinnable, and doesn't need to be.
- **Storage:** derive `Serialize`/`Deserialize` on `Win` and `Do`, and add `enum Pin { Do(Do), Window(Win) }`. Prefs get `top_pins: Vec<Pin>`, defaulting to the B1 set. An unknown name in an old prefs file must be skipped rather than failing the whole prefs load (deserialize each entry leniently, for example as `Vec<serde_json::Value>` filtered through `from_value`).
- **New palette entries the defaults and B2 need:** Compare (toggle the split; `FreezeReference` only freezes), Randomize, Today, and High-res render. Video already exists as `Win::Video`, and Save image as `Do::SavePng`.
- **Look:** toggles show their state with a pressed look; window openers act as toggles.
- **Overflow:** the bar never wraps. When the pins don't fit, the rightmost ones collapse to icons first, then into a "»" overflow menu.
- Randomize and Today stay in the scene combo's footer by default, and can be pinned.
- "Reset everything to defaults" doesn't touch the pins; Menu > Settings gets "Reset top bar".

## 3. Menu (☰)

Merged list:

| Submenu | Items |
|---|---|
| File | Open scene… (Ctrl+O), Save scene (Ctrl+S), Save scene as…, Restore last session (only when edits are live), Generate scene… (opens the Scenes window's Create pane) |
| Edit | Undo, Redo |
| Window | Scenes, Assets, Scopes, Convergence, Path visualizer (P), Statistics and achievements, Phone remote, Script console, Describe a scene (AI), Laser puzzle, Hide panel (Tab) |
| Settings | Show advanced settings, Check for updates at startup (its only home; remove the copy in the Update window), Text-to-scene model ▸, Save settings.json, Save settings as…, Load settings…, ── Reset everything to defaults (add a confirmation) |
| Renderer | (Windows only) D3D12 / Vulkan, as today |
| Help | Keyboard and mouse (F1), Check for updates, About |

- The seven "Keys" hint lines in today's menu go, because the Keyboard and mouse window holds the same content. The ☰ tooltip says "Menu · F1 for keys".
- The footer line "<adapter> · Rust, Direct3D 12, DXR 1.1" must say Vulkan when Vulkan runs.
- The laser puzzle window also opens by itself whenever a scene turns the puzzle on.

## 4. Property tabs

`Tab::Scene` goes away. It splits into Lights, Object and Material.

| Tab | Sections (top to bottom) |
|---|---|
| Render | Reconstruction (Denoiser, RR model, Quality mode, Reuse history), Sampling, Upscaling and filter, CPU renderer, Advanced |
| Look | Look preset on the title · Exposure, Tone, Grade, Film, Compare (Freeze reference, Reference on CPU, Split), Tools (Zebras, Scopes, Light meter, Convergence graph, Path visualizer), Light path expressions (moved from ui.rs:1374; look_tab.rs:458 draws it today), Display |
| Lights | **Always on top:** Sky and sun, HDRI, Fog, Background. **Below:** the selected light (Main, Flashlight, area light) with Delete in its header. **Always at the bottom:** Audio |
| Object | Selected object: name, Duplicate, Delete in the header, transform, physics. Camera selected, or **nothing selected:** Camera and Lens. Stage selected: Room. Always at the bottom: Physics world |
| Material | The selected object's material; Walls and Floor when Stage is selected |
| Export | Progress card while an export runs (§5), Save the current image (one JPEG-quality slider), High-resolution still, Video ("Export video…" opens the dialog, which stays), Recent exports |

Rules:
- **Selection rule (critic):** sky, HDRI, fog and lens never depend on the selection, so picking an object never hides them. Stage selection drives only Room and Walls/Floor.
- **Viewport pick:** a picked object opens Object (or keeps Material); a picked light opens Lights; Render, Look and Export stay put. This replaces `Tab::Scene` at main.rs:1822 and `pick_keeps_tab` (main.rs:3668).
- **Denoiser:** Render > Reconstruction is its home. The top-bar Quality combo stays as the bundle shortcut, and Alt+D cycles it. (The chrome map's "Custom only" idea is dropped.)
- **View:** the top-bar combo is the only home; the Look > Image combo goes.
- **CPU view:** the top-bar toggle is the only home; the Menu checkbox goes.
- **Walk avatar** (st:547) is a separate setting from walk mode and stays, as the right-click option on the rail's Walk icon.
- **Duplicate and Delete:** the Object tab header and the context menu, both. Hide is in the context menu (H) and the outliner's eye.
- **Lights "On":** the outliner checkbox is the home and the Lights tab mirrors it. The Flashlight has only the outliner checkbox and T, so the off note at sc:244 stays.
- **Scene description** (sc:20): shown in the scene combo's tooltip and on the outliner's Stage row.

## 5. Export progress (B-Export)

Today a centred 380 px window covers the image (video_ui.rs:253–350). In B:
- A top-bar chip shows "Exporting video 37 % · bar · ~14 min" from any tab; clicking it opens Export.
- The Export tab's card shows the preview, Frame N of M, spp, elapsed and remaining time, a progress bar, a frame strip (done, current, left), Pause, Cancel, and "Open folder when done".
- The viewport shows "locked while exporting", and the status bar shows Exporting.
- When it finishes, a toast says "Saved <file> · Open folder", and Recent exports keeps a list including failures and cancellations.
- The close-armed warning stays as it is today.
- The CPU variants keep their labels ("Rendering image (CPU)" and "Exporting video (CPU)").
- Pause is new: it needs the export loop to stop submitting frames without dropping state.

## 6. Timeline (B-Timeline)

- The rail's bottom button toggles the timeline; it runs under the viewport only.
- **Toolbar row:**
  - |◀, ◆◀, Play/Stop, ▶◆, ▶|, the time, frame N
  - Loop, Camera path, Curves
  - Auto-key, Key, Keys ▾ (shift, speed, copy to selected), Delete key, Interpolation
  - right-aligned: Clear, Shutter, fps, Length
- Below it come the scrub strip, then the tracks for whatever holds keys (the selected item first), then the curve editor (150 pt, shown when Curves is on).
- Keyed rows in the property tabs show ◆, and clicking it keys at the playhead.
- At 1280×720 with the timeline open, the column keeps its full height because of the layout order above.

## 7. Scenes and Assets browsers (B-Scenes, B-Assets)

- Both open over the viewport, reached from the top bar, the Menu, and Ctrl+P (Scenes).
- **Scenes:**
  - left sidebar: Library (All, Favorites, Recent, My scenes), Collections, tags, and Create (Generate…, Save current scene…)
  - middle: search, sort and a card-size control over a card grid
  - right: details with Load and ★ Favorite
  - The scene combo lists recent and favourite scenes, then "All scenes… (Ctrl+P)".
- **Assets:**
  - left sidebar: Kind (Sky maps, Materials, Models), Source (Poly Haven, ambientCG, Khronos, Other), On this computer
  - right: details with a 1K–8K resolution choice, "Use as sky" (into Lights > HDRI), Apply to floor or walls, and an inline download progress row with Cancel
- New data: favourites and recents in prefs, plus scene tags (from the scene JSON or a sidecar file).

## 8. Things to change with the tab split (critic)

- **Prefs:** `Tab` is serialized and `Tab::Scene` is the default (prefs.rs:64, ui.rs:137). Add `#[serde(alias = "Scene")]` on `Object`, or every old prefs.json fails to load and resets the width and zoom.
- **Text that names the Scene tab or old places:**
  - main.rs:3239: "Physics is off (Scene tab, Simulate)"
  - ai.rs:103: "The critique is in the Scene tab"
  - browser.rs:483 and :513: "Stage → HDRI" should become Lights > HDRI
  - overlay.rs:294: "unlock it under More"
  - the shortcut sheet at ui.rs:2090 and :2139
  - the first-run hint at ui.rs:1927
  - video_ui.rs:67: the HDR10 tooltip says Render, but Display is in Look
- **Code:**
  - the palette's Focus and Go targets (palette.rs:9, :288)
  - `--tab scene` (main.rs:674), which should keep working as an alias
  - selftest.rs:277, 301, 1036, 1049 and 1103
- E and N work with nothing selected, so their toasts point to Object > Physics world, which is always shown.
- Tooltips: Move, Rotate and Scale say that G/R/S need the pointer over the viewport and a selection. Hide panel says whether the rail and the outliner hide too (main.rs:3397).

## 9. Open questions for the owner

Built with these defaults on 2026-11-12, all changeable: 2 Display stays in Look; 3 Lens stays in Object > Camera; 4 Battle mode in Render > Sampling; 5 Tools stays a Look section; 6 both; 7 not yet.


1. (Settled: B3, pin your own.)
2. Should Display (output, swapchain, peak and paper white) go to a Menu submenu, or stay in Look?
3. Should Lens go to Look, next to exposure, or stay in Object > Camera?
4. Battle mode: in Render > Sampling with a header badge, or in the Menu?
5. Should Look > Tools become a View popup on the top bar?
6. spp appears in both the top bar and the status bar: keep both (the top bar compact, the status bar full)?
7. Should the Keyboard and mouse sheet gain a gamepad block?

## 10. Mockup corrections not yet drawn

- The boards show the overlay only for transform tools. Per §1 it shows whenever something is selected.
- B-Lights shows Sky only when Sky is selected. Per §4 the sky sections are always on top of Lights.
- The boards draw the right column at 380 pt; the default is 370.
