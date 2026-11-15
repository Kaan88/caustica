## GPU efficiency (track 20a, 2026-11-02)

Owner rule: performance work never lowers quality (no fewer samples or bounces, no lower render scale, no performance mode). See roadmap 20a.

**Tools.** `RENDER_OK` now also prints `trace_ms_mean`, `reconstruct_ms_mean` (RR, or the native filter with `--no-rr`), `display_ms_mean` and `ui_ms_mean`, from the `Gpu::pass_ms` timestamps. `tools/perf_baseline.ps1` runs the sweep below into `captures/perf/baseline.csv` (the 2026-11-02 run is kept as `baseline-2026-11-02.csv`). `CAUSTICA_SHADER_DEBUG=1` at build time adds `-Zi -Qembed_debug` to DXC (same `-O3` code) so Nsight's shader profiler can map stalls to HLSL lines. Build it into its own target dir (`target/profile`).

**Baseline, 2026-11-02** (RTX 4070 Ti SUPER, 1440×900, 4 spp, each scene's own bounce count (Classic and bunny 6, neon 12, mirror corridor 24, Sponza 12, glass dragon 20), 300 frames, no VSync; ms):

| Scene | Mode | GPU | Frame | Trace | Reconstruct |
|---|---|---|---|---|---|
| Classic (0) | RR | 5.23 | 5.77 | 2.85 | 2.29 |
| Classic (0) | native | 5.13 | 5.53 | 3.52 | 1.51 |
| Bunny (13) | RR | 5.53 | 6.04 | 3.18 | 2.26 |
| Mirror corridor (17) | RR | 7.81 | 8.40 | 5.45 | 2.27 |
| Mirror corridor (17) | native | 8.64 | 9.13 | 6.41 | 2.14 |
| Neon (21) | RR | 5.83 | 6.36 | 3.48 | 2.26 |
| Glass dragon | RR | 11.01 | 11.60 | 8.68 | 2.24 |
| Glass dragon | native | 12.77 | 13.26 | 10.68 | 2.00 |
| Stained-glass hall | RR | 7.96 | 8.53 | 5.62 | 2.25 |
| Glass stack | RR | 7.56 | 8.12 | 5.23 | 2.24 |
| Sponza | RR | 25.43 | 26.11 | 23.00 | 2.33 |
| Sponza | native | 30.74 | 31.36 | 28.05 | 2.60 |

Sweeps (native): Classic trace 1.03 / 1.55 / 3.52 ms at 1 / 2 / 6 bounces and 0.99 ms at 1 spp; mirror corridor 2.02 / 3.32 / 6.41 ms at 1 / 2 / 24 bounces and 1.85 ms at 1 spp.

**Reading:**
- **Trace scales linearly with samples** (Classic: about 0.84 ms per spp, a fixed part of about 0.15 ms for the guide ray). There is no large per-pixel overhead to remove. The cost is in the paths themselves.
- **Per-bounce cost grows with divergence:** Classic adds about 0.5 ms per bounce after the first. The mirror corridor's second bounce adds 1.3 ms, and its remaining 22 bounces only 3.1 ms together (Russian roulette and escaping paths end most of them). Glass scenes cost 2–3× Classic. These are the scenes where traversal returns to the shader for non-opaque candidates (glass, hidden lights) and where SER would help: fixes (a) and later track 20.
- **Sponza is the heaviest by far** (262k triangles, 69 textures, alpha-tested plants and curtains, an atrium lit mostly by bounce light from the sun through its open roof): 23 ms of trace. It is the main target for fixes (a), (b) and (e).
- **Fixed costs are a big share of light scenes:** DLSS RR is about 2.25 ms whatever the scene (40% of Classic). The native filter costs 1.5–2.3 ms, which is high for six passes at 1440×900 (fix (f)).
- **CPU/GPU bubble:** frame − GPU is 0.35–0.6 ms in every scene (5–10% on light scenes), and the fence wait is about the whole GPU time: the CPU records nothing while the GPU works (fix (c)).
- The low power draw fits these numbers: short frames with an idle gap each, RR's tensor-core work, and traversal that waits on memory. Nsight (occupancy, stall reasons) is still needed to split the trace time further.

**Nsight GPU Trace, 2026-11-03** (`tools/nsight_capture.ps1`: `ngfx.exe` GPU Trace, 3 frames after 6 s, clocks at base, windowed app, metrics auto-exported to `captures/perf/nsight/<scene>/BASE/GPUTRACE_FRAME.xls`; frame-level averages in %, since our passes have no markers yet):

| | Sponza | Mirror (17) | Classic (0) | Glass dragon |
|---|---|---|---|---|
| GPU busy (`gr__cycles_active`) | 91 | 74 | 50 | 67 |
| SM throughput | 14.5 | 18.7 | 15.9 | 13.2 |
| Warps active (compute) | 21 | 16 | 11 | 14 |
| SM active, warp slots empty | 66 | 48 | 31 | 46 |
| SM idle | 13 | 36 | 58 | 40 |
| Register file allocated | 80 | 56 | 37 | 49 |
| L1 / L2 hit | 71 / 98 | 77 / 90 | 83 / 81 | 75 / 90 |
| VRAM throughput | 6 | 12 | 12 | 9 |

- **Occupancy is register-limited everywhere.** Registers allocated / warps active is about 3.5 in every scene: about 150–160 registers per thread, which caps residency near 12–13 of 48 warps per SM. Sponza runs at that cap. The SMs are mostly waiting with too few warps to hide traversal and memory latency, while DRAM bandwidth stays at 12% or below. Fix (b) is first.
- **The windowed app idles the GPU much more than headless runs show.** GPU busy is 50% in Classic and 67–74% in the mirror corridor and the dragon. The headless bubble was 0.5 ms, but the windowed frame adds Present and egui on the CPU while the GPU waits. Fix (c) is second; confirm it with timestamps in a windowed run first.
- Pass markers (`BeginEvent`/`EndEvent` around trace, reconstruction, display, UI) would split these metrics per pass in `GPUTRACE_REGIMES.xls`.
- Pitfall: don't pass `--platform` to `ngfx.exe`. Qt takes it as its own platform-plugin option and fails with "Could not find the Qt platform plugin".
- Pitfall: the table above came from runs in which the shader profiler never ran. `--metric-set-name`, `--multi-pass-metrics` and `--real-time-shader-profiler` are per-architecture options and are ignored silently without `--architecture` (`REPRO_INFO.xls` then says "Warp State Sampling Disabled" and the metric set falls back to Async Compute Triage). Those traces are kept as `captures/perf/nsight/<scene>-asynctriage/`.

**Nsight GPU Trace with the shader profiler, 2026-11-03** (same script, now `--architecture Ada --metric-set-name "Top-Level Triage" --multi-pass-metrics --real-time-shader-profiler`; traces in `captures/perf/nsight/<scene>/`, read through the `ngfx-mcp` server). Frame averages in % of peak:

| | Sponza | Mirror (17) | Classic (0) | Glass dragon |
|---|---|---|---|---|
| RT core busy (`rtcore__cycles_executed`) | 31.0 | 18.6 | 14.6 | 14.5 |
| SM throughput | 14.6 | 22.5 | 27.9 | 18.1 |
| Warps active (compute) | 19.2 | 12.7 | 16.0 | 14.3 |
| Warp launch blocked: registers | 20.7 | 19.5 | 16.9 | 22.7 |
| Warp launch blocked: shared memory | 4.9 | 10.0 | 9.7 | 4.8 |
| Stall: no instruction (I-cache) | 5.8 | 2.2 | 2.1 | 6.1 |
| Stall: long scoreboard, L1/tex | 5.0 | 4.6 | 3.1 | 2.0 |
| Stall: long scoreboard, RT core | 4.5 | 2.5 | 1.7 | 3.1 |
| Stall: wait | 2.1 | 3.3 | 3.8 | 2.5 |

Top `trace` hotspots by HLSL line (% of the trace's shader samples; `trace` is 91% of all samples in Sponza and 58% in Classic, where DLSS RR takes most of the rest):

| | Sponza | Mirror | Classic | Dragon |
|---|---|---|---|---|
| `intersect` `Proceed` loop (pathtrace.hlsl:192) | 22.4 | 14.6 | 14.0 | 23.1 |
| `occluded` `Proceed` loop (:264) | 10.9 | 11.1 | 7.6 | 10.1 |
| `cdf_search`, env map (:298) | 11.7 | – | – | – |
| `select_index`, emitter CDF (:590) | – | 14.6 | 0.7 | 1.0 |
| `random` / `hash` (:29, :20, :3) | 5.5 | 4.2 | 5.2 | 5.1 |
| `env_bilinear` (:319, :315) | 3.6 | – | – | – |
| `sample_maps` texture reads (:107–115) | 4.0 | – | – | – |

- **RT cores are not the limit.** They are busy 15–31% of the frame. Waiting on them is under 5% of issue stalls. Traversal lines (192, 264) are 22–34% of samples, but they wait for the RT core to return candidates and on memory, with too few warps to hide it.
- **Registers block warp launch 17–23% of the time** in every scene, and this confirms fix (b). The "no instruction" stalls (6% in Sponza and the dragon) are instruction-cache misses in the large kernel. Smaller variants help here too.
- **Binary searches are a new, cheap target.** The env-map CDF search costs 11.7% of Sponza's trace samples (two searches per env sample, about 20 dependent loads). The emitter CDF search in plain NEE costs 14.6% in the mirror corridor. Both are chains of dependent memory loads ("long scoreboard"). A guide table (Fishman's cutpoint method) gives exactly the same index from one load plus a short search. Sampling and Sobol stratification stay bit-identical, which an alias table would not (it is why plain NEE keeps its CDF, see `restir_presample`). This is fix (g) in roadmap 20a.
- `trace` declares no `groupshared`, yet shared-memory allocation blocks launch 5–10% of the time. Probably the driver's RayQuery state; not confirmed.
- The multi-pass runs report GPU busy 85–93% even in Classic, against 50% single-pass: multi-pass replays a frame back to back, which hides the CPU/GPU gap. Use the single-pass numbers above for the bubble (fix (c)).
- **Fix (g), cutpoint guide tables, done 2026-11-03.** Both CDFs keep a table of K entries (K = the largest power of two ≤ the entry count, so `u·K` and `k/K` are exact in f32): entry k is the search's result for u = k/K (`lighting::cutpoints`). `cdf_search` and `select_index` search only between entries k and k + 1, which always bracket the old answer because it never decreases as u grows (the predicate `u < cdf[i]` is monotone in i for u < 1, including the emitter table's forced final 1). Layouts: the env CDF texture doubles to width × 2(height + 1), with the guide rows after the CDF rows as f32 indices; the emitter guide follows the alias records in the emitter buffer, sixteen u32 per record (data-layouts.md). `environment::build` now rejects maps taller than wide: the marginal row never fit them, and the doubled texture would pass D3D12's 16384 rows. Renders are bit-identical where the build is deterministic (see roadmap 20a (g)); the first timings are within noise and need repeated runs.
- Fix (g) timing, 5 interleaved runs each (trace ms, before → after): Sponza 22.63 → 21.94 (−3.1%), mirror corridor 5.46 → 5.21 (−4.6%), Classic 2.85 → 2.98 (+4.3%). Classic's loss came from the two guide loads on its 2-entry table, so `select_index` now uses the guide from 64 emitters only (mirror corridor 25,602, neon 15,362; Classic 2, bunny 6). A smaller loss on Classic and the bunny then remained without any guide load (+1.8%, +4.9%): code size in the full kernel. LEAN (below) removed it.
- **Fix (b) step 1, LEAN variants, done 2026-11-03.** `trace_lean` / `trace_maps_lean` compile `pathtrace.hlsl` with `LEAN`: `fog_on()` / `restir_on()` are constant false, RayQuery skips procedurals (`RQ_BASE`), procedural-candidate code goes, and `lean_strip` zeroes the five feature gates (coat `layers.x`, anisotropy `layers.w`, sheen `sheen.rgb`, thin film `surface_scale.z`, extinction `medium.w`) in `material_at` and `medium_material`, so their branches fold. `Gpu::lean_scene` (set in `update_materials`, the only material upload path) holds when no visible SDF object exists and every material record has those gates ≤ 0; `integrate` uses LEAN when it holds and the frame's `fog[0]` and `restir[0] & 1` are off, the exact values the shader reads. Results, 4 interleaved runs each at 1440×900 with RR (trace ms, before → after): Sponza 23.61 → 14.30 (−39%), Classic 2.91 → 2.22 (−24%), bunny 3.23 → 2.12 (−35%), mirror corridor 5.68 → 3.72 (−35%), neon 3.68 → 2.48 (−33%), courtyard 4.19 → 2.88 (−31%); frame time −12% to −35%. Output: scenes with fog, ReSTIR or anisotropy keep the full kernel and are bit-identical; LEAN scenes differ only through float rounding (Sponza and Classic by ≤ 6e-7 per pixel; scenes with more lobe choices let a rounding change flip a random choice, so single paths diverge, but image means agree to ≤ 1.5e-5 relative after 16 frames). So the register cost of features a scene doesn't use was about a third of the trace pass. Next in (b): split the remaining kernel further (glass and the media stack, emissive meshes, portals) and re-profile Sponza's occupancy.
- **Re-profile with LEAN (2026-11-03, `captures/perf/nsight/<scene>/`, the full-kernel traces in `<scene>-full/`).** Full → LEAN: RT core busy Sponza 31.0 → 43.3, mirror corridor 18.6 → 23.9, Classic 14.6 → 16.2, glass dragon 14.5 → 20.4; compute warps active +12% to +25%; SM throughput +4 to +6 points. Warp launches blocked by register allocation barely moved (Sponza 20.9 → 19.7, dragon 23.1 → 21.1, Classic 17.8 → 13.7): occupancy is still register-limited, and the gain came from the dropped code, not from many more warps. Stall shares: "no instruction" fell (Sponza 28 → 23%, dragon 33 → 19%: a smaller kernel fits the instruction cache), and waiting on the RT cores rose (Sponza 22 → 28%, dragon 17 → 22%) to Sponza's largest stall. So traversal is now the larger share, which points at (a); the per-candidate shader work for glass and camera-hidden objects was the part of (a) still open.
- **Fix (a) step 1, instance masks, done 2026-11-03.** Ordinary objects were already FORCE_OPAQUE; glass and camera-hidden objects were FORCE_NON_OPAQUE, so every candidate triangle of theirs ran the shader (a material load and two tests) for every ray. Now the two skip rules are instance masks (`gpu.rs` `instance_mask`, pathtrace.hlsl `MASK_HIDDEN` / `MASK_CLEAR_GLASS`): camera rays leave out bit 2, shadow rays bit 4 when glass doesn't block, and an object under one rule goes FORCE_OPAQUE with just that bit. Glass below 0.5 transmission (which the shader committed every time anyway) is opaque with 0xff. Still non-opaque: alpha-tested objects, objects under both rules, the static BLAS's hidden-light range and SDFs. Renders bit-identical on 11 scenes (glass dragon, glass of water, glass stack, hourglass, stained-glass hall, the caustic Cornell boxes, Sponza, presets 0/13/17; the photon-mapped box differs by ≤ 5e-7 from accumulation order). Trace ms, 4 interleaved runs at 1440×900 with RR: glass dragon 5.20 → 4.62 (−11%), glass of water 9.38 → 7.94 (−15%), stained-glass hall 3.24 → 2.70 (−17%), hourglass 2.44 → 2.26 (−8%); Sponza and Classic unchanged (no glass).
- Classic's multi-pass trace ran out of frames at `--frames 900` (about 5 ms frames); the script now passes 20000, and ngfx ends the app when the trace is done.
- **Fix (a) rest, opacity micromaps and the static split, done 2026-11-03.**
  - *Opacity micromaps* (DXR 1.2, tier 1.2; `src/omm.rs`, `Gpu::omm_blas`). Each alpha-tested mesh with a base-colour map gets a 4-state OMM array: micro-triangles in bird-curve order, subdivision up to level 7 at about 16 texels per micro-triangle, 64 MiB cap. Classification is conservative: a min/max alpha pyramid over the texel range a micro-triangle touches (wrap addressing, one texel margin for bilinear, cutoff ± 1/512), so only micro-triangles that are opaque or transparent at every sample are marked so; the rest stay unknown and run the any-hit test as before. The image is bit-identical with micromaps on and off. The BLAS key includes the map and cutoff; such instances get no FORCE flag. Sponza trace 13.45 → 12.98 ms (−3.6%, 3 rounds, 1440×900 RR). `CAUSTICA_NO_OMM=1` turns them off.
  - *Static split.* The static geometry was one BLAS with a non-opaque range for hidden lights and glass, so every ray paid the any-hit path there. It is grouped by `instance_mask` now, one BLAS and instance per group with the same masks and FORCE flags as objects. Single runs at 960×600 native: abacus 1.55 → 1.39 ms, stained-glass hall 1.78 → 1.43, mirror corridor 2.18 → 2.05; bit-identical.
  - *Pass markers.* `Gpu::stamp` wraps trace, reconstruction, display and UI in `BeginEvent`/`EndEvent`, so PIX and Nsight split the frame.
  - *Tried and reverted: (f) groupshared tiles* for the à-trous steps 1–2. The filter got slower (preset 0 1.70 → 1.83 ms) and the result was no longer bit-identical; the taps are L1 hits already.

