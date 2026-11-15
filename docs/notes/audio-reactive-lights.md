## Audio-reactive lights

Removed 2026-11-15 (roadmap Plan step 3).

- 2026-11-02, track 19. Scene → Audio → "Lights follow the music". `audio::Audio::start` opens the default output device's mix as an input stream: cpal's WASAPI host sets `AUDCLNT_STREAMFLAGS_LOOPBACK` when an input stream is built on an output device. Only f32 shared-mode output is handled (the Windows default). The callback mixes to mono into a 2048-sample ring.
- Each frame `update` takes the window, applies a Hann window and an FFT, and takes the RMS bin magnitude in 20–250 Hz, 250–2000 Hz and 2–12 kHz. Each band is divided by its own peak, which halves every 4 s (automatic gain, so quiet and loud music both swing the lights), then smoothed per frame time (`smoothing` at 60 fps).
- WASAPI loopback delivers no packets during silence, so a window older than 200 ms counts as silence (otherwise the lights freeze at the last level).
- `audio::apply` scales the frame's copy of the settings in `redraw` (the same clone the renderer already used), never `App::settings`: saving, undo and the inspector keep the scene's values. Multiplier = `floor + gain × level`; main light = bass, area light i and emitter i = band i mod 3. The copy changes every frame, so accumulation restarts every frame (as when the timeline plays) and `update_materials` rebuilds the emitter table; use RR.
- Off in headless runs and video exports (deterministic output). cpal reports a change of the default output device as a stream error but keeps capturing the old device, and some drivers and audio tools send that notification as soon as a capture opens. So a stream error is not fatal: `Audio::recover` reopens the capture on the current default device (at most once a second, levels and gains kept). Only a reopen that fails shows an error and switches the option off.
- Tests: tones at 80 Hz, 900 Hz and 5 kHz land in their bands; `apply` scales by `floor + gain`. `loopback_capture_opens_and_delivers_samples` is `#[ignore]` (needs an output device and something playing).

