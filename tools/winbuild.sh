#!/bin/bash
# Windows release build of Caustica under Wine, on the owner's Linux box (engine-notes "Windows under
# Wine"). The toolchain (Rust windows-msvc, MSVC, the Windows SDK's rc and dxc) and the Windows SDKs in
# vendor/ (agility, fsr, ffmpeg, ...) are read from the Windows checkout on the C: partition, mounted at
# /mnt/C. The tree is copied to target/win/src (without the ignored asset packs), crates are vendored for an offline
# build, and the exe lands in target/win/out/release/caustica.exe. Wine has no DXR: run it with --vulkan.
#   tools/winbuild.sh [version]          version defaults to Cargo.toml's
#   WINEPREFIX=$PWD/target/win/prefix wine target/win/out/release/caustica.exe --vulkan --self-test
# Run the exe from the repo root: a build in a cargo target folder reads scenes/ from the working
# directory (the sky data and the asset packs; from elsewhere the self-test fails at frame 127).
# Override C, WIN, MSVC, KV or RUSTV when the Windows side changes.
set -u
cd "$(dirname "$0")/.."
R=$PWD
V=${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
W=$R/target/win; S=$W/src
C=${C:-/mnt/C}; WIN=${WIN:-$C/Users/kaank/Desktop/cornell}
MSVC=${MSVC:-14.51.36231}; KV=${KV:-10.0.26100.0}; RUSTV=${RUSTV:-1.99.1}
zpath() { echo "Z:$1" | tr / '\\'; }
for need in "$C/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/$MSVC" \
    "$C/Program Files (x86)/Windows Kits/10/bin/$KV/x64" "$C/Users/kaank/.rustup/toolchains/$RUSTV-x86_64-pc-windows-msvc" "$WIN/vendor"; do
  [ -d "$need" ] || { echo "winbuild: missing $need" >&2; exit 1; }
done
# Tracked and new files only (the ignored asset packs stay out); the exe embeds scenes/builtin.
mkdir -p "$S"; git ls-files -co --exclude-standard | tar -cf - -T - | tar -xf - -C "$S"
rm -rf "$S/vendor"; ln -sfn "$WIN/vendor" "$S/vendor"
[ -d "$W/crates" ] || cargo vendor --locked --versioned-dirs "$W/crates" >/dev/null 2>&1
mkdir -p "$S/.cargo"
printf '[source.crates-io]\nreplace-with = "v"\n[source.v]\ndirectory = "Z:%s"\n[net]\noffline = true\n' "$W/crates" > "$S/.cargo/config.toml"
export WINEPREFIX=$W/prefix WINEDEBUG=-all WINEDLLOVERRIDES='mscoree,mshtml=' WINEARCH=win64
VS="$(zpath "$C/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/$MSVC")"
KIT="$(zpath "$C/Program Files (x86)/Windows Kits/10")"
TC="$(zpath "$C/Users/kaank/.rustup/toolchains/$RUSTV-x86_64-pc-windows-msvc/bin")"
export WINEPATH="$TC;$VS\\bin\\Hostx64\\x64;$KIT\\bin\\$KV\\x64"
export INCLUDE="$VS\\include;$KIT\\Include\\$KV\\ucrt;$KIT\\Include\\$KV\\um;$KIT\\Include\\$KV\\shared;$KIT\\Include\\$KV\\winrt"
export LIB="$VS\\lib\\x64;$KIT\\Lib\\$KV\\ucrt\\x64;$KIT\\Lib\\$KV\\um\\x64"
export "ProgramFiles(x86)=$(zpath "$C/Program Files (x86)")"
export RC_PATH="$KIT\\bin\\$KV\\x64\\rc.exe" DXC_PATH="$KIT\\bin\\$KV\\x64\\dxc.exe"
export CAUSTICA_VERSION=$V CARGO_TARGET_DIR="$(zpath "$W/out")" CARGO_HOME="$(zpath "$W/cargohome")" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12
export RUSTC="$TC\\rustc.exe"
cd "$S" && wine "$C/Users/kaank/.rustup/toolchains/$RUSTV-x86_64-pc-windows-msvc/bin/cargo.exe" build --release --locked 2>&1
echo "WINBUILD_EXIT=$?"
