## UI leftovers of item 3 (2026-11-02)

- **Capabilities:** `Gpu::capabilities()` returns a `Backend { available, reason }` for RR, DLSS SR, FSR and XeSS. Startup reasons (`init_reasons`) come from the adapter name (not NVIDIA), the NGX init code, the NGX caps bits, and the bridges' return codes (1 = DLL missing next to the exe, 2 = missing export, XeSS 0x1xx = init result); `rr_failed` / `sr_failed` add "failed while rendering". The RR checkbox, the Upscaler combo and the Quality bundles show the reason on hover. Self-test 231: each backend is available exactly when its reason is empty.
- **Modified dots:** `crate::slider` compares the value with `reset_value` (the same field in the scene as loaded, or the `UiState` default) and draws an accent dot in an 8 px slot it always reserves, so rows don't shift; clicking the dot does what a double-click does. Tolerance 1e-6 relative.
- **File pickers:** LUT and reference EXR get *Browse…*; settings files go through `settings_dialog` (`Actions::save` / `load` now carry the path, the quick buttons pass ./settings.json; loading pushes an undo checkpoint).

