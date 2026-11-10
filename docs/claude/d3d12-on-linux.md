# D3D12 on Linux: type-checking the Windows side and compiling its HLSL (feasibility, 2026-11-10)

## Outcome (built and run, 2026-11-10, branch lane-tools)

Both checks exist and work. How to run them: `testing.md` "Windows-side checks on Linux"; what they need: `setup.md`.
- `tools/check_dxil.py`: 31 of 31 entries compile and validate, 22 s wall with 8 jobs (the table was identical to
  the serial sweep below). The entry list and flags are parsed from `build.rs`.
- `build.rs`: `CAUSTICA_CHECK_ONLY=1` (empty `.dxil` stubs) or `=dxil` (real DXIL from the Linux DXC, 8 at a time, in
  the build script); `dxil_command()` is now the one place the DXC command line lives (the Windows build uses it too,
  serially, unchanged). SPIR-V for the Windows target uses the Linux DXC in check mode, so `cfg(spirv)` is on.
  A normal Linux `cargo check` is unchanged and still warning-free.
- `tools/check_windows.sh`: `cargo check --locked --target x86_64-pc-windows-msvc` with the user-local official
  1.99.1 toolchain (rustup `stable` in `~/.rustup`, with the windows-msvc std added; see setup.md), the `windows` crate sources already in the registry.
  First run: 2 min 14 s wall (65 crates, build.rs and dependencies included; later runs 48 s to 1.5 min
  depending on what changed). **Zero errors**, 11 warnings (all Windows-only, see below). `--all-targets` (test code)
  also compiles, zero errors.
- `tools/check_windows.sh clippy` runs `clippy -D warnings` for the Windows target with the rustup toolchain, which ships clippy. It passes.
- Why the warnings never showed on Linux: `main.rs` line 5 allows `dead_code` and `unused_imports` off Windows, so only
  the Windows check sees them. All are CPU-reference, avatar, star and test-only items, not D3D12 drift.
- Fixed here (small, plain): unused imports `rgb` (`src/reference/scene.rs:3`), `Shape` (`src/ui.rs:12`), `PI`
  (`src/astro.rs:11`, now imported in its tests) and `CROUCH_EYE` (`src/avatar.rs:17`, likewise); `APP_ID`
  (`src/platform.rs:91`) gated to `target_os = "linux"` like its only user. The rest were test-only or dead: `SHUTTER_MOMENTS` and both `render`s re-exported or defined under `cfg(test)`, `avatar::Part::corners` under `cfg(test)`, the never-read BDPT `Vertex::light` removed, and `stars` `total` allowed as dead outside tests. `src/reference/tests.rs:1246` `samples_reach` only warns in a test build, which this check does not run.
- The C++ bridges, linking and `cargo test` remain Windows-only, as expected.

The sections below are the pre-implementation feasibility study, kept as written.

Question: can the D3D12 half of Caustica be checked on this Linux machine without Windows? Nothing was
downloaded or installed and no source changed. Measured here: the DXC sweep (section 2) and a cargo metadata
resolution for the Windows target (section 1). Not measured: `cargo check --target` itself, because the Windows
`rust-std` is not installed. Estimates are marked as such.

## Short answer

- **HLSL to DXIL: yes, today, with nothing to install.** `vendor/dxc-linux/bin/dxc` (v1.9.2609) compiles all 31
  entries from `build.rs` with the exact Windows flags, including the five `lib_6_9` SER libraries, and
  `dxv` validates the output ("Validation succeeded"). 31/31 ok.
- **Rust type check of the `cfg(windows)` code: yes, with one download and a small `build.rs` change.**
  Needed: the `rust-std` component for `x86_64-pc-windows-msvc` (about 25 to 40 MB, MIT or Apache-2.0, estimated,
  not checked). No Windows SDK, no MSVC and no C compiler are needed because `cargo check` does not link and
  no dependency compiles C for Windows. `build.rs` itself does need a check-only switch (section 3).
- This catches D3D12/Windows compile errors, signature drift against `windows` 0.62 and `Constants`/shader
  mismatches that surface as type errors. It does not run anything, check linking, or exercise the C++ bridges.

Line numbers in the tables are approximate (read from `build.rs` at this checkout).

## 1. cargo check --target x86_64-pc-windows-msvc

Installed now: `rustc`/`cargo` 1.99.1 from pacman (no rustup), sysroot has only `x86_64-unknown-linux-gnu` and
`wasm32-unknown-unknown`. `lld`, `lld-link`, `llvm-rc`, `clang`, `wine` 11.21 are installed; `mingw-w64-gcc`
and `cargo-xwin` are not. No Windows SDK or xwin cache.

**Dependencies.** `cargo metadata --offline --filter-platform x86_64-pc-windows-msvc` succeeds: all 233
crates for that target are already in `~/.cargo/registry` (`windows` 0.62.2, `windows-core`, `windows_x86_64_msvc`
and so on), so no fetch. 37 of them have a build script, and they all run on the host: they print cfg flags or
link-search lines (`windows_x86_64_msvc`, `winit`, `rfd`, `gilrs`, `cpal`, `native-tls` and so on). The only
crates in `Cargo.lock` that depend on `cc` are `caustica` itself, `openssl-sys` and `wayland-backend`, and the
last two are not in the Windows graph (native-tls uses Schannel there). So no crate needs a Windows SDK or
`cl.exe` at check time.

**What `build.rs` does on a Windows target** (`CARGO_CFG_WINDOWS` set), in order, and whether it blocks a check:

| Step | Lines | Blocks a check? |
| --- | --- | --- |
| Find `dxc.exe` (Windows Kits, or `DXC_PATH`); only the path is embedded (`CAUSTICA_DXC`) | 14-33 | No, falls back to `dxc.exe` |
| SPIR-V pass with `vendor/dxc-spirv/bin/x64/dxc.exe` | 277-327 | No, but missing means no `cfg(spirv)`, so the Vulkan-on-Windows code would go unchecked. Fix by pointing `DXC_SPIRV` at the Linux dxc (it runs on the host; the Linux build already uses it for SPIR-V) |
| `assert!(TARGET contains "windows-msvc")` | 338 | No with `--target x86_64-pc-windows-msvc` |
| `assert!` that `vendor/dlss/include/nvsdk_ngx.h` exists | 342 | **Yes**: this checkout has only the Linux DLSS set (`vendor/dlss/lib/Linux_x86_64`, no `nvsdk_ngx.h`) |
| `resources()`: `rc.exe` for the icon and version info | 456-531 | No, only warns without rc.exe (`llvm-rc` is installed but takes different arguments; not needed for a check) |
| 31 DXC invocations writing `OUT_DIR/*.dxil`, which `src/gpu/init.rs` `include_bytes!`s | 345-370 | **Yes**: the files must exist, and `dxc.exe` is not runnable here |
| asserts that `vendor/fsr` and `vendor/xess/inc` exist | 371-381 | **Yes**: `vendor/fsr` is absent |
| `ffmpeg.exe` copy | 383-392 | No, optional |
| `cc::Build` on `bridge/{ngx,fsr,xess,vk}.cpp` with `/std:c++17` | 394-412 | **Yes**: needs `cl.exe` or `clang-cl`, the Windows `d3d12.h` and the FSR/XeSS headers |
| link search, `d3d12`/`dxgi`/`shlwapi`, `/EXPORT` args | 413-417, 438-452 | Ignored by `check` |
| Copy of the DLSS, FSR, XeSS and Agility DLLs next to the exe | 418-450 | **Yes**: `std::fs::read` of files that are not here (`vendor/agility` absent) |

**Can it be skipped behind an env var?** Yes, cleanly: one early block, section 3. `rustc-env`/`rustc-cfg` lines
(`CAUSTICA_VERSION`, `CAUSTICA_SHADERS`, `cfg(upscalers)`) still need printing so the crate sees the same cfgs
as a real Windows build.

**What the check would not cover:** the C++ bridges (`bridge/ngx.cpp`, `fsr.cpp`, `xess.cpp`, `vk.cpp` as built
for MSVC); linking and the missing `extern` symbols (`nvsdk_ngx_d`, FSR loader); `cargo test` (needs a linker,
`--no-run` fails); runtime behaviour. `cargo clippy --target` works since it is a check.

**What would need installing** (decision for the owner, nothing done):

| What | Source | Licence | Size | Notes |
| --- | --- | --- | --- | --- |
| `rust-std` for `x86_64-pc-windows-msvc`, version 1.99.1 exactly | `static.rust-lang.org/dist/rust-std-1.99.1-x86_64-pc-windows-msvc.tar.xz` (standalone installer) | MIT OR Apache-2.0 | about 25-40 MB download (estimate), about 150 MB unpacked | pacman has no such package (`pacman -Ss` found only mingw packages) and there is no rustup. Unpack with its `install.sh --prefix=<dir>`, then build an overlay sysroot of symlinks (system `bin`, `lib`, plus the new `lib/rustlib/x86_64-pc-windows-msvc`) and pass `RUSTFLAGS=--sysroot=<overlay>` together with an explicit `--target` (which keeps host build scripts off that flag). No root needed. Alternative: install rustup user-local with a 1.99.1 minimal profile, about 100 MB |
| Nothing else for the Rust check | | | | `lld-link`, `llvm-rc`, `wine` are already here and are not needed |

For the optional C++ bridge syntax check (not recommended as part of this): `mingw-w64-headers` (extra, small,
custom licence) would provide `d3d12.h`, but `vendor/fsr` is not in this checkout, and `llvm-mingw` is 96.7 MB
download / 621 MB installed, so not worth it.

## 2. DXC on Linux

`vendor/dxc-linux` holds `bin/dxc`, `dxv`, `lib/libdxcompiler.so`, `lib/libdxil.so` (DXC release v1.9.2609,
LLVM and MS licences in the folder). It is what `build.rs` already uses for SPIR-V on Linux
(`vendor/dxc-linux/bin/dxc`, `-spirv ...`). It is a full DXIL back end too, and `libdxil.so` is what signs the
containers, so it is not "unsigned".

**Does `build.rs` already do it?** No. The DXIL loop runs only after the `if !windows { linux_upscalers(); return; }` early return, and uses `dxc.exe`. On Linux only the SPIR-V pass runs.

**Test.** Same flags as `build.rs` 354-368: `-T <profile> (-E <entry> -Ges | -exports <entry>
-Wno-misplaced-attributes for lib_*) -HV 2021 -O3 -I shaders <defines> -Fo x.dxil shaders/<file>`, run with
`LD_LIBRARY_PATH=vendor/dxc-linux/lib` through `target/safe.sh`, all 31 entries parsed from `build.rs` by a
script (`/home/kaan/caustica-tmp/now/sweep.py`, scratch). Result: **31 of 31 compile, exit 0, no errors**
(only the known `image_format` attribute warnings). `dxv` on `trace.dxil` and `trace_ser.dxil`: validation
succeeded.

| Group | Entries | Wall time each | Size |
| --- | --- | --- | --- |
| Small passes (restir, filter, display, glare, photon_clear, vs/ps) | 11 | under 0.2 s | 3-47 KB |
| Path tracer RGB (`trace*`, `lean`, `paths`, `lpe`, `bdpt`, `photon_*`) | 11 | 0.8-7 s | 126-544 KB |
| SER libraries `lib_6_9` (`*_ser`) | 7 | 1.1-10.6 s | 135-583 KB |
| Spectral variants | the 4 others | 1.1-15.4 s | 170-567 KB |

Whole sweep, serial: about 2.5 minutes of wall time (sum of the above, roughly 125 s), single-threaded dxc
each (this run went through `safe.sh`). Running them
in parallel like `src/hotreload.rs` does would bring it under 30 s. Sizes are the same order as the DXIL the
Windows build embeds (not byte-compared: the Windows SDK dxc is an older version, see below).

**Differences worth knowing:**
- The Windows build uses the SDK 10.0.26100 `dxc.exe` (`docs/claude/setup.md`). The Linux one is a newer 1.9
  release. `lib_6_9` compiled fine, so the SER shader model is accepted. Output will not be byte-identical, and
  a compiler difference could in principle appear as a shader that compiles on one and not the other. In this
  sweep no such case showed up. Shader-model 6.9 execution still needs the Agility SDK 1.619 runtime on Windows,
  which is a runtime matter, not a compile one.
- This is compile and validate only. No D3D12 device runs the DXIL here. `wine` 11.21 is installed, but wine's
  D3D12 over vkd3d-proton has no DXR 1.1 / SER support worth relying on, so no run test is proposed.

**Nothing to download for DXC.** Optionally a `tools/check_dxil.sh` (below) that loops the 31 entries.

## 3. The build.rs change

An env var `CAUSTICA_CHECK_ONLY` (not set in normal builds, `cargo:rerun-if-env-changed`), read right after the
`windows` flag at line 277, before the SPIR-V block. Sketch:

```rust
let check_only = env::var_os("CAUSTICA_CHECK_ONLY").is_some();   // =1: stubs, =dxil: real DXIL via DXC_PATH
println!("cargo:rerun-if-env-changed=CAUSTICA_CHECK_ONLY");
// SPIR-V: with check_only the default dxc_spirv becomes vendor/dxc-linux/bin/dxc (host binary),
//   so cfg(spirv) stays on for Windows and the Vulkan code is checked.
...
println!("cargo:rustc-cfg=upscalers");
if check_only {
    for &(_, _, _, name, _) in entries { write OUT_DIR/{name}.dxil (empty, or real dxc output) }
    return;               // skips: asserts on vendor/{dlss,fsr,xess}, resources(), ffmpeg, cc, link, DLL copies
}
```

- `=1`: writes empty `.dxil` stubs (the Rust only embeds them, so a check does not care). Time: seconds.
- `=dxil`: runs the same DXC command with `DXC_PATH=vendor/dxc-linux/bin/dxc` and `LD_LIBRARY_PATH`, in a
  parallel loop, so a cross check also proves the shaders compile. The existing `Command` code works unchanged
  except `-Ges` etc. are fine and the output goes to `OUT_DIR`; it needs only to be moved out of the Windows-only
  tail and ideally run with `std::thread::scope` (about 125 s serial otherwise). Setting `DXC_PATH` makes
  `CAUSTICA_DXC` point at the Linux binary, which only matters to the debug hot reload and is not run in a check.
- The `return` must come after `println!("cargo:rustc-cfg=upscalers")` and must skip the msvc `assert!`
  only if the target is not msvc (with `--target x86_64-pc-windows-msvc` it passes anyway).
- Command, after the one-time `rust-std` setup:
  `CAUSTICA_CHECK_ONLY=dxil DXC_PATH=$PWD/vendor/dxc-linux/bin/dxc RUSTFLAGS=--sysroot=<overlay> target/safe.sh cargo check --locked --target x86_64-pc-windows-msvc`
  with `CARGO_TARGET_DIR` set to a scratch folder, never `target/release`.
- A separate `tools/check_dxil.sh` that only loops the 31 entries (the `sweep.py` logic) is independent of
  the above and could be done first; it needs nothing installed.

## 4. Estimate

| Piece | Effort | Needs the owner |
| --- | --- | --- |
| `tools/check_dxil.sh` (31-entry DXC + `dxv` sweep, parallel) | about 30 min | No |
| `CAUSTICA_CHECK_ONLY` in `build.rs` | about 30-45 min | No |
| `rust-std` download, overlay sysroot, document in `docs/claude/setup.md` | about 30 min | Yes: approval to download (about 25-40 MB) |
| First cross `cargo check`: 233 crates on the build lock (estimate 3-6 min with 10 jobs), then fixing whatever `cfg(windows)` code the Linux-only checks let drift | unknown; 0 to a few hours depending on how much has drifted. The recent worktrees changed several Windows-only files (`gpu.rs`, `platform.rs`, `display.rs`, `update.rs`), so some drift is likely | No |

Total to a working check loop: about 2 hours plus any drift fixes. Value: catches D3D12-side breakage on every
Linux change without the Windows machine. What it will not replace: a Windows build, `--self-test`, and the
D3D12 debug layer.

## Open items

- Whether to approve the `rust-std` download; the exact size and the standalone-installer layout were not
  verified (no network use was allowed).
- Compare the Linux DXC DXIL against the Windows SDK DXC on one entry (bytes, instruction counts) before trusting
  the sweep as a proxy for the shipped shaders.
- The C++ bridges stay unchecked on Linux; they build only on the Windows machine.
