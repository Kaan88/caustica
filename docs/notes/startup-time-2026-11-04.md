## Startup time (2026-11-04)

New builds sat on a white window for seconds (warm driver shader cache: 5.9 s to the first headless frame; the first launch after a shader change 20–67 s, the driver compiling every pipeline). stderr now prints `STARTUP phase ms` (since `main`: window, device, pipelines, gpu, first-frame). Measured warm, release, 320 × 240 headless: device 0.36 s, the ~22 compute pipelines and 7 SER state objects 3.65 s one after another, NGX init 1.45 s, the rest (FSR, XeSS, resize, scene) 0.2 s.

- The compute pipelines (`create_compute_pipelines`) and the SER state objects (`make_ser`) are created on one thread each (`in_parallel`; the device is free-threaded, `Threaded` wraps the COM pointers): 3.65 → 0.7 s.
- NGX init runs on its own thread from right after the root signature, holding a device reference, and is joined where it used to be called.
- `present_blank` clears the first back buffer to dark grey and presents it right after the swap chain exists (0.36 s), so the window is never white for long; a failure there is only logged.
- Result: first frame at 2.1–2.3 s (headless and windowed), the window dark from 0.36 s.

