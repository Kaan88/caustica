## Built-in scenes and auto exposure (E8, 2026-11-10)

Plan E8, revised after a plan review. The first formula in the plan (`ev_bias = log2(old exposure)`) was wrong: under auto the shown exposure is the metered one times 2^ev_bias, so it double-counts the meter. The start value is **`ev_bias = log2(old exposure / settled metered exposure)`**, which reproduces the old image whatever the meter decides.
- **What changed in the files:** every built-in except white-furnace, layered-furnace and measured-cornell (the reference scenes stay manual and linear on purpose) now carries `auto_exposure: true`, `ev_bias` and `eyes: true` (94 files; `exposure` stays as the manual fallback, so switching auto off gives the pre-E8 look). Written by `tools/build_scenes.py` from `tools/scene_exposure.json` (slug to stops): scripted scenes get the keys when regenerated, and `retune_exposure` patches every file in `scenes/builtin` by a JSON round trip (all three layouts in the folder: indent 1, indent 2, with or without the final newline), so the diff is only those keys. Run `python tools/build_scenes.py` (no `--overwrite`: four scenes edited by hand after the script wrote them, flashlight-crypt, measured-cornell, newtons-cradle and underwater, are kept as they are and only get the keys). Edit the table and rerun to change a bias.
- **Procedure:** (a) one headless sweep, release build, 640 x 360, 120 frames (RR on, as shipped), `--set auto_exposure=true --set ev_bias=0 --set eyes=false`, reading `meter_ev` from the RENDER_OK line; bias = log2(old exposure) - meter_ev, rounded to 0.01. (b) every scene again from its file (auto, Eyes on, 60 frames) against the same file with `--set auto_exposure=false --set eyes=false` (the old look); mean display luminance (sRGB decoded, Rec. 709) compared. No scene differs by more than 1/3 EV (largest veach-mis -0.24, pipe-organ -0.14, dark-side +0.06; the other 90 within 0.05), so nothing was hand-tuned for that. The 26 scenes on the plan's retest list were looked at side by side (old | new, 640 x 360) and judged by eye: bright emitters do not crush the rest (street-bokeh, disco-floor, stage-beams, jack-o-lantern, candle-shelf), black backgrounds are not lifted into noise (fairy-lights, bunny-museum, gem-display), night scenes stay as dark as authored.
- **Meter noise:** the meter reads a 4 spp image through RR; a scene lit by a few tiny very bright lights moves by 0.2-0.3 EV between runs (veach-mis 14.53 then 14.21 at 60 frames). The Eyes look makes the darkest corners a little darker and bluer than before. Both are expected.
- **Owner default, changeable:** hand-lit night scenes (nighthawks, jack-o-lantern, lighthouse-fog, candle-shelf, colonnade-night, night-skyline, fairy-lights, flashlight-crypt) keep their authored light levels, so they are photopic and the Eyes look hardly touches them; the large negative biases (-3 to -6 EV, dark-side -5.8, veach-mis -13) are the "this scene is meant to be dark" of the old exposure written as a bias. Dimming those lights to real levels (so they go scotopic and the meter does the work) is a per-scene edit of the light intensities; if wanted, set the scene's bias to 0 afterwards.
- **Day cycle:** a plain auto exposure holds the room at one brightness all day (the meter brightens dusk by 28 EV), so the cycle stopped reading as an evening. `sunlit-room-day-cycle` keys `ev_bias` along the same 12 s as the sun: -2.05, -2.6, -3.6, -4.6, -6.0 at 0, 3, 6, 9, 12 s (linear), chosen by eye. Mean display luminance at nine times between 0 and 12 s falls monotonically (0.069, 0.049, 0.040, 0.016, 0.0096, 0.0059, 0.0034, 0.0016, 0.0007). The old fade fell about 0, -0.85, -2.8, -4.8 EV at 0, 3, 6, 9 s; the keys are gentler. The scene file has no `exposure` track (the plan expected one: it was a constant 2.0).
- **starlight-field and clear-night:** starlight-field goes auto at the default sun intensity 1 (it was intensity 1000 and exposure 10, still dark), manual fallback exposure 10000, `ev_bias` -2. The new built-in **clear-night** (scripted in `build_scenes.py`): physical sky at latitude 33 S, day 172, solar time 23.7, `moon` off, the galactic centre about 80 degrees up, a camera looking up from the ground (pitch -32, fov 80) over dark ground and four rocks; auto, `ev_bias` -2. At bias 0 the meter makes the whole frame mid grey (the sky is most of the image and is as dark as the ground), which is a long exposure, not the naked eye; -2 leaves the Milky Way and stars readable on a dim sky. Changeable.
- **A GPU fix this needed (sky map in half floats):** at intensity 1 a moonless sky is about 1e-8 scene units, under half-float's smallest value, so the zenith stored as black and the band edges as jagged steps (the N4 note's "fp16" item; E5 pre-exposure only covers the denoiser inputs). `environment::half_scale(peak)`, used by both backends where the map is uploaded: a map that peaks below 0.01 is divided by a power of two that puts its peak in 1024..2048 (the shader already multiplies the strength and the pdf normalisation by `env_scale`); every other map keeps the factor of 1 or the old over-range division, so their stored bits do not change. Test `half_scale_lifts_a_night_map_and_leaves_the_rest`. Oracle (`tools/oracle.py --scenes starlight-field,clear-night --floor 1e-13`, 320 x 240, 1024 / 512 spp, intensity 1, no workaround): starlight-field GPU/CPU 1.0000, z 0.58; clear-night 1.0000, z -0.30; worst blocks 1.0015 and 1.0002. Before the fix the same scene rendered the black zenith above. The D3D12 side is the same one-line change, not compiled on Linux.
- **Thumbnails:** `scenes::RENDER_VERSION` 2 to 3 (the display and the files changed), so every thumbnail made before has a stale name; `remove_stale` deletes them and `--make-thumbnails` re-renders all. Checked with a scratch folder holding a stale-hash dark-side thumbnail and one for a scene that does not exist: both removed, every scene rendered (counts under "Measured" below).
- **Measured (Vulkan, Linux, RTX):** `--self-test` 484 frames (308 scripted, 176 scene files), RENDER_OK, exit 0; `cargo test` 383 passed, 6 ignored; clippy `-D warnings` clean; `--make-thumbnails --thumbnails-out` rendered 176 scene thumbnails and 12 material balls with no failure (98 of them built-in, clear-night and starlight-field not black), and the two planted stale files were removed.
- **Not done / open:** the table's biases come from one Linux (Vulkan, RTX) sweep at 640 x 360; the D3D12 thumbnails of a release come from the Windows build. Dimming the night scenes' lights to real levels (above). The meter's reading of tiny-light scenes is noisy by 0.3 EV, so a re-run of the sweep can move a bias by that much.

Table (old exposure; settled meter at bias 0 from the 120-frame sweep; the bias written; hand-tuned; verdict):

| scene | old exposure | settled meter (EV) | ev_bias | hand-tuned | verdict |
|---|---|---|---|---|---|
| abacus | 1.0 | 0.62 | -0.62 | no | old look kept (-0.00 EV) |
| aquarium | 1.0 | 0.81 | -0.81 | no | old look kept (+0.01 EV) |
| armillary-sphere | 1.0 | 1.6 | -1.6 | no | old look kept (-0.00 EV) |
| atom | 1.0 | 11.8 | -11.8 | no | old look kept (+0.00 EV) |
| audio-equalizer | 1.0 | 6.05 | -6.05 | no | old look kept (-0.00 EV) |
| bigger-on-the-inside | 1.0 | -0.04 | 0.04 | no | old look kept (+0.00 EV) |
| birthday-cake | 1.0 | 2.57 | -2.57 | no | old look kept (+0.00 EV) |
| blackbody-forge | 1.2 | 6.9 | -6.64 | no | old look kept (-0.00 EV) |
| bowling-lane | 1.3 | 2.74 | -2.36 | no | old look kept (+0.00 EV) |
| brushed-metal | 1.0 | 2.0 | -2.0 | no | old look kept (+0.00 EV) |
| bulb-workshop | 1.3 | 3.09 | -2.71 | no | old look kept (+0.00 EV) |
| bunny-museum | 1.0 | 6.92 | -6.92 | no | old look kept (-0.07 EV); looked at |
| candle-shelf | 1.0 | 3.83 | -3.83 | no | old look kept (-0.00 EV); looked at |
| castle | 1.0 | 0.79 | -0.79 | no | old look kept (-0.00 EV) |
| clear-night | - | - | -2.0 | yes (new) | new: moonless southern night, galactic centre high; bias -2 by eye |
| colonnade-night | 1.0 | 3.03 | -3.03 | no | old look kept (+0.01 EV); looked at |
| cornell-bunny | 1.0 | 2.07 | -2.07 | no | old look kept (+0.00 EV) |
| cornell-caustic | 1.0 | 2.07 | -2.07 | no | old look kept (+0.00 EV) |
| cornell-dark-room | 1.0 | 2.09 | -2.09 | no | old look kept (+0.00 EV) |
| cornell-fog | 1.0 | 2.18 | -2.18 | no | old look kept (+0.00 EV) |
| cornell-water-tank | 1.0 | 2.62 | -2.62 | no | old look kept (+0.00 EV) |
| dark-side | 9.0 | 8.98 | -5.81 | no | old look kept (+0.06 EV); looked at |
| desert-monolith | 1.3 | 2.02 | -1.64 | no | old look kept (+0.00 EV); looked at |
| diamond-fire | 1.3 | -2.39 | 2.77 | no | old look kept (-0.00 EV) |
| disco-floor | 1.4 | 5.66 | -5.17 | no | old look kept (+0.02 EV); looked at |
| domino-spiral | 1.0 | -0.46 | 0.46 | no | old look kept (-0.00 EV) |
| fairy-lights | 1.0 | 5.73 | -5.73 | no | old look kept (+0.00 EV); looked at |
| festival-banners | 1.3 | -0.54 | 0.92 | no | old look kept (-0.00 EV) |
| flashlight-crypt | 1.4 | 4.87 | -4.39 | no | old look kept (-0.00 EV) |
| flavin-fluorescent | 1.0 | 1.4 | -1.4 | no | old look kept (+0.00 EV) |
| gem-display | 1.4 | 6.02 | -5.54 | no | old look kept (-0.04 EV); looked at |
| glass-of-water | 1.0 | 0.37 | -0.37 | no | old look kept (-0.00 EV) |
| glass-stack | 1.0 | -0.45 | 0.45 | no | old look kept (+0.00 EV) |
| glossy-floor-tiles | 1.3 | 2.63 | -2.26 | no | old look kept (-0.00 EV) |
| hourglass | 1.0 | 3.19 | -3.19 | no | old look kept (-0.01 EV) |
| ice-cubes | 1.0 | 1.7 | -1.7 | no | old look kept (+0.01 EV) |
| ies-wall | 1.0 | 1.48 | -1.48 | no | old look kept (-0.00 EV) |
| infinite-corridor | 1.0 | -0.2 | 0.2 | no | old look kept (-0.00 EV) |
| isometric-workshop | 1.0 | -2.1 | 2.1 | no | old look kept (-0.00 EV) |
| jack-o-lantern | 3.0 | 6.11 | -4.53 | no | old look kept (-0.00 EV); looked at |
| jewellery-box | 1.0 | 2.43 | -2.43 | no | old look kept (-0.01 EV) |
| kusama-infinity-room | 0.8 | -0.51 | 0.19 | no | old look kept (+0.02 EV); looked at |
| lava-lamp | 1.0 | 6.15 | -6.15 | no | old look kept (+0.00 EV) |
| layer-sweeps | 1.0 | 0.67 | -0.67 | no | old look kept (+0.00 EV) |
| layered-cocktail | 1.0 | 1.56 | -1.56 | no | old look kept (-0.00 EV) |
| layered-materials | 1.0 | 1.55 | -1.55 | no | old look kept (-0.00 EV) |
| light-temperature-row | 1.0 | 2.33 | -2.33 | no | old look kept (+0.01 EV) |
| lighthouse-fog | 2.6 | 4.54 | -3.17 | no | old look kept (+0.00 EV); looked at |
| little-planet | 1.3 | 1.26 | -0.88 | no | old look kept (+0.01 EV) |
| mirror-maze | 1.6 | 10.12 | -9.44 | no | old look kept (-0.04 EV) |
| mirror-room | 1.0 | -1.69 | 1.69 | no | old look kept (+0.03 EV) |
| neon-diner | 1.2 | 2.6 | -2.33 | no | old look kept (-0.00 EV) |
| newtons-cradle | 1.0 | 0.97 | -0.97 | no | old look kept (-0.00 EV) |
| night-skyline | 1.5 | 3.97 | -3.39 | no | old look kept (+0.01 EV); looked at |
| nighthawks | 1.0 | 4.9 | -4.9 | no | old look kept (+0.01 EV); looked at |
| optical-discs | 1.0 | 3.87 | -3.87 | no | old look kept (-0.00 EV) |
| optics-bench | 2.2 | 3.92 | -2.79 | no | old look kept (+0.00 EV); looked at |
| orrery | 1.0 | 9.78 | -9.78 | no | old look kept (-0.00 EV) |
| overcast-still-life | 0.6 | -2.17 | 1.44 | no | old look kept (+0.00 EV); looked at |
| panorama-gallery | 0.35 | -2.55 | 1.03 | no | old look kept (-0.00 EV); looked at |
| paperweights | 1.0 | 0.8 | -0.8 | no | old look kept (+0.01 EV) |
| penumbra-ladder | 1.0 | 0.38 | -0.38 | no | old look kept (+0.00 EV) |
| perfume-vanity | 1.0 | 2.67 | -2.67 | no | old look kept (-0.00 EV) |
| pipe-organ | 1.0 | 1.51 | -1.51 | no | old look kept (-0.14 EV) |
| pool-break-1984 | 1.0 | 1.24 | -1.24 | no | old look kept (-0.00 EV) |
| scattering-materials | 1.0 | 1.89 | -1.89 | no | old look kept (+0.00 EV) |
| sdf-gallery | 1.0 | 0.78 | -0.78 | no | old look kept (+0.01 EV) |
| shape-material-matrix | 1.0 | -0.36 | 0.36 | no | old look kept (+0.00 EV) |
| sky-garden-dusk | 1.6 | 1.04 | -0.36 | no | old look kept (-0.00 EV); looked at |
| snow-globe | 1.0 | 3.39 | -3.39 | no | old look kept (+0.01 EV) |
| snowmen | 0.75 | -2.42 | 2.0 | no | old look kept (-0.00 EV) |
| spiral-staircase | 1.0 | 1.9 | -1.9 | no | old look kept (-0.01 EV) |
| stage-beams | 2.0 | 4.61 | -3.61 | no | old look kept (+0.05 EV); looked at |
| stained-glass-hall | 1.0 | 2.4 | -2.4 | no | old look kept (-0.00 EV) |
| starlight-field | 10.0 | 13.71 | -2.0 | yes | auto at sun intensity 1, bias -2 by eye: stars, Milky Way, airglow visible; was black |
| stonehenge-dusk | 1.0 | 0.74 | -0.74 | no | old look kept (-0.00 EV); looked at |
| street-bokeh | 1.6 | 6.7 | -6.03 | no | old look kept (+0.02 EV); looked at |
| stylised-forest | 1.0 | -0.84 | 0.84 | no | old look kept (+0.00 EV) |
| sun-greenhouse | 0.45 | -2.39 | 1.24 | no | old look kept (+0.00 EV); looked at |
| sunlit-room-day-cycle | 2.0 | 3.05 | -2.05 | yes (keys) | bias keyed -2.05 to -6.0 over the 12 s; displayed mean falls monotonically (9 times sampled) |
| sunset-tank | 1.5 | 9.03 | -8.45 | no | old look kept (+0.04 EV) |
| swinging-lantern | 1.8 | 7.01 | -6.16 | no | old look kept (+0.03 EV) |
| tea-set | 1.0 | 1.98 | -1.98 | no | old look kept (+0.00 EV) |
| thin-film | 1.0 | 0.05 | -0.05 | no | old look kept (-0.00 EV) |
| tilt-shift-village | 1.3 | 0.63 | -0.25 | no | old look kept (-0.00 EV) |
| torii-corridor | 1.0 | 1.25 | -1.25 | no | old look kept (+0.00 EV) |
| turrell-ganzfeld | 1.0 | 0.21 | -0.21 | no | old look kept (-0.00 EV) |
| underwater | 1.5 | 3.31 | -2.73 | no | old look kept (-0.00 EV); looked at |
| veach-ajar | 1.6 | 4.17 | -3.49 | no | old look kept (-0.00 EV) |
| veach-mis | 2.5 | 14.53 | -13.0 | yes | meter reads 14.5 / 14.2 on two runs (tiny bright lights); bias set to -13.0, looked at; matches |
| velvet-lounge | 1.0 | 0.44 | -0.44 | no | old look kept (+0.00 EV) |
| vermeer-room | 2.5 | 2.04 | -0.72 | no | old look kept (-0.00 EV); looked at |
| weather-project | 1.5 | 2.17 | -1.58 | no | old look kept (+0.00 EV) |
| wrecking-ball | 1.2 | 0.62 | -0.36 | no | old look kept (+0.05 EV) |
| zen-garden | 1.4 | 0.26 | 0.22 | no | old look kept (-0.00 EV) |

