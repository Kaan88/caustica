## Grazing shadow rays (2026-11-02)

`nee_limit(len, cos)` ends NEE shadow rays 0.6 mm off the emitter's surface along its normal (`len − min(len/2, 0.0006 / max(|cos|, 0.02))`) instead of 0.6 mm along the ray. The fixed margin let grazing samples hit the emitter's own face: light sampling lost 0.57% on night-skyline and MIS 1.6% on the tea set (both now within noise).

