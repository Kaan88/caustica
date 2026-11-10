# Night sky redesign (plan, 2026-11-10)

Owner's decisions (2026-11-10, from mockups A naked eye, B camera 20 s, C half moon):
- A realistic starry night, with **accurate constellations** for the date, time and place, from a real star catalogue.
- A **real Milky Way map**.
- **Night exposure:** automatic, like eyes adapting or a camera's auto exposure, plus manual settings.
- **Looks:** A (naked eye) is the default, and B (camera) is a one-click preset. Both have tweaks, with good defaults.
- Later and separate: **volumetric clouds with their shadows**.

## What is wrong today
- Stars are single texels of the baked equirectangular sky map (`sky.rs`, about 1 texel in 150). A texel is about a third of a degree, so a star covers several screen pixels as a soft square: the "big dots".
- Their energy is physical (about 1e-9 of daylight), but nothing adapts the exposure, so they need Exposure 10 and Sun intensity 1000 to show.
- They are random: no catalogue, no Milky Way, no realistic count by magnitude, no airglow.

## Design
1. **Stars as directional point emitters, not map texels.**
   - A table of unit directions with RGB radiant intensity, from a catalogue (RA, Dec, V magnitude, B−V colour) rotated into the local sky by date, time, latitude and longitude.
   - `sky.rs` already does the sun and moon from the clock.
   - The tracer finds stars with a small cone test per miss, through a spatial grid on the sphere, and samples them like suns (NEE by power, MIS).
   - On screen a star is a point: sharp at any resolution and zoom.
   - Atmospheric extinction comes from the existing transmittance, and twinkling stays out (stills).
2. **Milky Way:** an HDR equirectangular map in galactic or equatorial coordinates, rotated the same way, added to the sky radiance and dimmed by extinction. It must hold no bright stars, or they would be counted twice.
3. **Airglow and the natural night sky:** a faint emission layer, brightest about 10° above the horizon. Greenish in the camera look, near-neutral to the eye.
4. **Night exposure:**
   - Auto mode adapts to the scene's luminance with a speed setting (eye-like or camera-like).
   - Manual mode takes ISO, shutter and f-number (camera) or a fixed EV.
   - The "Eyes" look adds scotopic vision: less colour and a slight blue shift (Purkinje) below about 0.01 cd/m². It is a display step, so it doesn't change the rendered data.
5. **Tweaks with defaults:**
   - star brightness
   - limiting magnitude (6.5 eye, 9 camera)
   - star size (point spread in pixels, display only)
   - Milky Way strength
   - airglow strength and colour
   - light pollution (sky floor)
   - the look preset
6. **CPU renderer:** the same star table and map in `src/reference/`, so parity holds and the oracle can measure it.

## Data and licences
To be settled by research before downloading anything; ships only under licences that allow it, listed in THIRD-PARTY-NOTICES.md.

## Volumetric clouds (separate, after this)
- **Approach:** a cloud layer 1 to 3 km up as a heterogeneous medium (Perlin–Worley density as in Guerrilla's Nubis), lit by the physical sky and sun.
- **Shadows:** come free from path-traced transmittance.
- **Multiple scattering:** exact but slow. Wrenninge's approximation (2013) is the usual speed-up.
- **Reuses:** the delta and ratio tracking of heterogeneous fog.
- **Test asset:** Disney's cloud dataset (CC BY-SA 3.0).
