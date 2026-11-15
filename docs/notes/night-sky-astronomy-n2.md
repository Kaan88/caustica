## Night sky: astronomy (N2)

`src/astro.rs` turns a date, time and place into the rotation from the celestial frame (J2000 RA/Dec, which the star catalogue `scenes/sky/stars.bin` uses) to the sky's local frame (east +x, up +y, north −z, the frame of `sun_local()`; the `north` turn is applied by the shader as for the sun). The chain: Julian date (Meeus), Greenwich mean sidereal time (IAU 1982), precession J2000 to date (IAU 1976), then hour angle and latitude. No nutation, aberration or refraction: errors are arc minutes. `PhysicalSky::local_from_celestial()` is the entry point; `PhysicalSky::julian_date()` takes the instant from `year`, `day`, `solar_time()` minus the longitude's hours and the equation of time (with `clock` that is `time − utc_offset`). `PhysicalSky::year` (default 2026, 1900..2100, serde default so old scenes load unchanged) is in `map_key`, and the Year slider sits under Date.
- `parse_stars` / `load_stars` read `stars.bin` ("CSTR", version 1, count, then RA f32, Dec f32, V f16, B−V f16 per star) and fail with a clear error on a missing file, bad magic, wrong version or a length mismatch.
- Superseded by N2b below: `sun_local()` is now the exact chain.
- Tests: Polaris within 1° of the latitude at every hour, Sirius transit altitude 90° − |lat − dec| (within 0.1°), JD and GMST against Meeus' worked examples, precession rates, round trip, frame handedness.


