# UI usability review, 2026-11-06

Requested by the owner ("it has become harder to use"). Five independent reviewers read the source and screenshots at 1280 x 720 and 1920 x 1080, by task and lens (first run, structure, expert use, language, space). 60 raw findings merged to 42; each was then challenged by a separate reviewer trying to refute it, and all 42 held, several with corrected fixes.

Root cause worth knowing: in SDR the whole UI was sRGB-decoded and never re-encoded (`eye.w` was 1, `gui.hlsl`), so panels, text and swatches were far too dark (notes at 1.13:1 contrast). An HDR development display hides it. Fixed; `render::tests::sdr_constants_leave_the_gui_in_srgb` guards it.

## Fixed in the working tree

| Severity | Finding |
|---|---|
| high | Flashlight config is too big, always open, misplaced and unlike every other light; enabling it blows out the image |
| low | Physical sky's night controls (stars, Milky Way, airglow, light pollution, moon) were loose rows among the day ones; now grouped under one "Night sky" heading with tooltips, plus limiting magnitude and star size (N5) |
| high | Panel edits have no undo, and Ctrl+Z rewinds everything since the last checkpoint |
| high | Closing the window loses unsaved work with no warning, and nothing comes back on launch |
| high | The whole UI is drawn too dark: SDR colours are sRGB-decoded and never re-encoded |
| high | Notes, hints and explanatory text are close to unreadable, and stay weak after the gamma fix |
| high | Every save overwrites the last one, and the note names the wrong place |
| high | Scene combo, Page Up/Down, Randomize and Today replace the scene with no undo and keep a stale selection |
| high | Tab hands keyboard focus to egui, which kills every bare-key shortcut and breaks typed vectors |
| medium | Failures and refusals are silent or reported inconsistently |
| medium | The selection's inspector sits below unrelated Physics, Audio and Laser sections, and Material comes last |
| medium | Object inspector puts physics rows before position and size |
| medium | The add row widens the panel to about 484 px, covers the image, and makes the tab buttons move between tabs |
| medium | Walk mode swallows F12 and every other global key |
| medium | Saving gives no sign the image is still noisy, and the two save routes are unexplained |
| medium | Scene browser is mouse-only, and typing without clicking first fires global shortcuts |
| medium | G/R/S depend silently on pointer position, and the sheet and tooltips misdescribe keys |
| medium | Most disabled controls give no reason |
| medium | Scroll position is shared across tabs and the outliner nests scroll lists |
| medium | Object search stays active after its box disappears, emptying the outliner |
| medium | Row help only appears on the label, and row labels do not toggle checkboxes |
| medium | The Timeline toggle does nothing on keyed scenes, so at 720p the panel can't be dismissed |
| low | Render tab controls stay enabled when they do nothing |
| low | The same concept appears under different names |
| low | Slider and reset behaviour differs between sections |
| medium | Top bar is clipped at 1920, wraps mid-group at 1280, holds 24 undifferentiated controls, and its labels explain little |
| medium | Quality names contradict each other, and 'Native' has two meanings |
| low | First-run hint covers the mouse only, not how to reach a saved image (parts 1 and 2: the second hint line, and the camera note and Reset camera moved to the top of the Camera section) |
| low | Status bar overflows at 1280 with Advanced on and buries its useful text |
| low | Overlays use fixed offsets and the 'Hide panel' button hides more than the panel |
| medium | The Stage inspector is one flat grid that grows past a screen |
| medium | Stage and Object inspectors show controls that are overridden or inactive |
| low | 'Show advanced' flattens panels instead of organising them |
| low | Tooltips use implementation jargon where plain words would do |
| medium | Look tab offers no starting point, and 'advanced off' still shows expert controls |
| medium | Scene browser gives its space to a tag wall, and three scene pickers disagree (parts 1 to 3; part 4, falling off small screens, only partly: see ui-batch-E) |
| medium | Mesh import freezes the window with no feedback |
| low | Audio tooltip in the Scene tab |
| medium | Look tab mixes grading with diagnostics, leaving Compare off-screen |
| medium | Five save paths; the 'Scene file' section saves settings, and its Load replaces render settings |
| low | Denoiser and resolution are each set in three places, in mixed order |
| medium | No way to find a setting by name |
| medium | Clicking the image pulls you away from Look or Render and adds a gizmo |
| low | WASD does surprising things outside walk mode, and R changes meaning |

Known limits of those fixes: real OS key input and clicks were not run (hidden runs ignore them; logic has unit tests), D3D12 and Windows were not built, and #12 failures is partly done (modal G / R / S under non-perspective cameras needs a decision).

## Open

One row is only partly done:

- **R changes meaning:** R still restarts accumulation with nothing selected and rotates (modal) with a selection. The step's text on R was cut off, so the fix is not known; moving restart to Ctrl+R is the candidate. Not done because it needs an owner choice, and a guess would change a key people use. Everything else in the three former rows (tab kept on pick, bare O orbit, Alt+D denoiser, Ctrl+K palette) is in the Fixed table (see ui-batch-D below).

## Update 2026-11-10: denoiser enum

`Settings.rr` / `Settings.denoise` are now `Settings.denoiser` (Off, Native, Rr). Rows above that name the RR checkbox, the "Native denoiser (D)" row or `settings.rr` are stale: the Render tab's Reconstruction section opens with one Denoiser combo (the reorder in the "set in three places" row is done for the denoiser; Quality mode and the upscaler stay as they were). D now cycles the three choices and its toast names the choice (`Denoiser: ...`), not the RR state, so the "use `g.rr_active`" point does not apply; the cycle now sits on Alt+D (done, see Keys under ui-batch-D).

## Update 2026-11-10: ui-batch-A

Fixed above: quality names (Best 100% / Good 67% / Fast 50% / Fastest 33% / Native denoiser 100% / Native denoiser 67%), top bar clusters with Hide panel reserved, status bar order, overlays constrained to the free area, first-run hint line two (reaching a saved image). Defaults taken and changeable: cluster widths are estimates in `chrome.rs`; Hide panel still hides the status bar and timeline too; parts 2 and 3 of the first-run row (camera_ui mouse note) are not done and the row's remaining text was truncated in this file. Not checked by eye: layouts were built and unit-tested only, so look at 1280 and 1920 wide.

## Update 2026-11-10: ui-batch-B

Fixed above: the Stage inspector split, overridden or inactive controls, 'Show advanced' groups, tooltip jargon, first-run hint part 2. The open rows for these were truncated in this file (after "Sky light: disabled when ..."), so where the text ran out the defaults below were taken. All are changeable.

- **Stage:** three blocks, each its own grid with `min_col_width` 128 px (`ui::LABEL_WIDTH`, also used by the material grids): Room (Cornell box, Measured walls, Left and Right wall, Other walls or Floor colour, Floor size, Shadow catcher, then the Wall and Floor material headers, Wall material only with the box on), Sky and light (Sky light, Physical sky, HDRI, Background), Fog. 'Ground size' is now 'Floor size' and the box-off colour 'Floor colour'.
- **Greyed rows** (`ui::row_if`, stored values kept, the row tooltip leads with the reason; the rules are `stage::overrides`, unit-tested): Left/Right wall with Measured walls or a wall texture; Other walls / Floor colour with Measured walls or textured wall and floor; Sky light with the physical sky or a loaded HDRI; Background while the camera sees the sky or the HDRI; HDRI with the physical sky; Orbit speed with Auto orbit off; material Colour with a Measured spectrum. Left/Right wall are hidden with the box off; Measured walls stays reachable while it is on. An HDRI that failed to load overrides nothing. Area light Rotation is hidden for a sphere.
- **Material groups** (`editor::group`): Surface (Reflection, Index of refraction, Measured spectrum, Beam splitter), Coatings (Clear coat, Sheen, Thin film), Brushed and grating, Volume (only with Glass above 0), Pattern, Other (Priority, Glow only). A group starts open when something in it is in use; after that its open state is kept per header. The switch only decides whether the groups sit inline or under More.
- **Tooltips:** effect first, term second (Audio, Fog, Henyey-Greenstein, blackbody, Abbe, extinction, linear RGB, delta/ratio tracking, importance sampling). Broken whitespace inside two long tooltip strings in editor.rs was removed.
- Not done, from the first-run row: part 3 (truncated in this file). Not checked by eye: 1280 and 1920 wide, and the groups' open state across object selections.



## Update 2026-11-10: ui-batch-C

Fixed above: the Look starting point and expert rows, Compare off-screen, the Export tab's "Scene file" section, denoiser and resolution rows. The open rows for these were truncated in this file (the "Five save paths" row after "Scenes window: do NOT replace its save", the Look and "set in three places" rows mid-list), so where the text ran out the defaults below were taken. All are changeable.

- **Look combo** (Look tab, first row of Image): Neutral, Warm film, Cool, Punchy, Soft (`look_tab::LOOKS`). A bundle writes the tone map, halation, grain, grain size, vignette, lift, gamma and gain, and nothing else (exposure, glare, the LUT, Display gamma stay); there is no monochrome bundle. Neutral is the default state of those fields. The selection is derived from the values (`Look::of`, 0.001 tolerance) and reads Custom after any tweak. Unit test `looks_write_only_their_fields_and_are_recognised`.
- **Advanced off hides:** Grain size (unless grain is on), Glare size and threshold (unless changed), the LUT row (unless one is loaded), Swapchain (unless HDR10 is chosen), Peak brightness and Paper white (unless set by hand). Film stock stays visible. Changeable in `look_tab.rs`.
- **Compare** now sits above Tools, and Light path expressions is a collapsing header at the end, closed unless the view is LPE or an expression exists (then it is forced open so it is never hidden).
- **Save paths:** the Export tab's "Scene file" section is gone (step 1). Step 2 (the Scenes window) is untouched, as its text was cut off. The Menu keeps Save scene, Save scene as, Open scene and the three settings entries, which now carry tooltips that say what they write: a bare settings file with the scene included, against a titled scene file in your scenes. No Save scene shortcut was added to the Export tab.
- **Reconstruction rows:** one note at the top names the other places (the top bar's Quality menu, the D key). Order: Denoiser, RR status, RR model, Denoise strength and Reuse temporal history (shown when Native is the choice, or under Show advanced; moved out of the Advanced group), Quality mode, Render scale, Upscaler, DLSS preset, then Pixel filter and Filter width. Render scale moved from after Filter width to sit under Quality mode. The Quality menu and Quality mode keep their names. The denoiser cycle is on Alt+D (done).
- Not checked by eye: layouts were built, clippy-clean and unit-tested only.

## Update 2026-11-10: ui-batch-E

Fixed above: the Scenes window tag wall and picker wording, mesh import freeze, the audio tooltip. The rows were truncated in this file (the scene-browser row after "clicking the active tag already clears", the audio tooltip row absent), so where the text ran out the defaults below were taken. All are changeable.

- **Tags** (`windows.rs`): one row of the lists (All, Favorites, Recent), `Any tag` (was `all`), ten hard-coded chips (`TOP_TAGS`: material, glass, interior, emissive, night, caustics, physics, still life, outdoor, generated; one that no scene uses is not drawn) and a `More tags (n)` menu with every other tag and its scene count. A tag picked from the menu shows as an active chip until cleared. Search still matches every tag. 'still-life' is now 'still life' (the three generated scene files, `build_scenes.py`, and `scenes::save_generated`); the generator family id stays `still-life`.
- **Pickers:** the top bar scene menu stays the 24 presets and gains `All scenes… (n)` at its end, which opens the Scenes window; its tooltip and the arrows' tooltips say which list they walk (the Scenes list while a scene file is loaded, else the presets). The arrows' behaviour itself is unchanged: not merged into one list, as that text was cut off.
- **Small screens:** the Scenes window is constrained to the screen and its grid height follows the screen (120 to 360 px). Not checked by eye at 1280 x 720.
- **Mesh import:** `mesh::load` no longer holds `CACHE` while parsing (`load_cached`: lookup, release, parse, re-lock, `entry().or_insert`). Two threads that miss together both parse and return the first insert's `Arc`; no in-flight guard (a duplicate parse is the accepted cost), and failures are cached as before. `UiState::import_mesh` parses on a thread (Scene tab + Add > Mesh file… and the Assets window's Use), `land_mesh_imports` adds the result before each frame's settings snapshot with one undo step. Feedback: a toast on start, a spinner row under + Add while running, a toast on landing (an error toast when parts did not fit). Imports landing in the order they finish. Tests: `mesh::tests::cache_is_not_held_across_a_parse_and_inserts_once`, `concurrent_imports_of_one_file_agree`.
- **Audio tooltip:** says it follows the PC's sound, not the microphone, and names the control that settles the picture (Render > Reconstruction > Denoiser).

## Update 2026-11-10: ui-batch-D

Fixed above: the image click, WASD and R, and settings search. The open rows' proposed-fix text was truncated in this file after the first steps, so where it ran out the defaults below were taken. All are changeable.

- **Image click** (`main.rs`, `pick_keeps_tab`): a left click on the image keeps the Look or Render tab (Scene and Export still go to Scene, and a double-click, which frames, opens Scene too). It does not scroll the inspector. Toast "Selected X: open Scene to edit it, Esc to deselect" for objects, area lights and the main light (not for a right click, whose menu already shows what it acts on). Selecting still happens, so the outline, the gizmo and Delete work; the gizmo is the only thing the row's title called "adds", and it stays (it is how a selection is seen). A selection made while Look is open therefore shows a gizmo; Esc removes it.
- **Keys:** auto orbit is O (bare), the denoiser cycle is Alt+D (it still cycles Ray Reconstruction, Native, None; its toast names the choice, as the enum change decided). Bare A and bare D, the old keys, do nothing but toast the new key ("Auto orbit is O", "Cycle the denoiser with Alt+D"), so a WASD user who leaves walk mode is told rather than surprised; remove those two arms when the new keys are known. R keeps restarting accumulation when nothing is selected and rotates (modal) when something is, over the viewport; away from the viewport with a selection it only toasts. That rule is unchanged: the row's text on R was truncated, so a different fix (for example moving restart to Ctrl+R) is open for the owner to choose, not done. Listings updated: the shortcut sheet (`SHORTCUTS`, so `--ui-shortcuts`), the About box's Keys lines, the Auto orbit and denoiser tooltips, README, and the palette's own rows.
- **Command palette** (`src/palette.rs`, `App::palette_window` / `run_palette`): Ctrl+K opens a box at the top centre; type, Up / Down, Enter runs, Esc or Ctrl+K closes, a click runs. `--ui-palette "query"` opens it for a capture. About 80 static entries: settings (a row or section name plus search words, with its tab, the Scene selection that shows it and the section named in the toast), the windows and commands that are hard to find (Script console, Phone remote, Statistics, Scenes, Assets, Video, Scopes, Convergence graph, Path visualizer, shortcuts, Open / Save scene and settings, Show advanced, CPU view, Walk mode, Flashlight, denoiser, A/B reference, update check) and the keys' commands, with their key at the right. Always-visible buttons (Randomize, Frame, Assets in the top bar) are not justified entries; Frame the scene and Reset the camera are in only because they have keys. Every query word must match (label prefix, word start, substring, search word, then letters in order, so "dnoiser" finds Denoiser). Choosing a setting opens its tab, selects Stage / Camera / Main light / Flashlight (or the selected or first object or area light, or a toast when there is none) and toasts "Section: Tab tab"; it does not scroll to or highlight the row. Not in it: Reset everything to defaults (destructive), per-material fields (the Object entries open the inspector). Changeable: the table order is the empty query's order.
- Tests: `palette::tests` (matching, typo, every word, unique labels, keys), `tests::a_pick_keeps_look_and_render_but_not_a_double_click`; self-test frames 241 and 242. Not checked by hand: real OS key input (Ctrl+K, Alt+D, O) and real clicks on the image, which hidden runs ignore; one `--capture-ui --ui-palette denois` render was looked at.

