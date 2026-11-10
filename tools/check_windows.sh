#!/bin/bash
# Type-check or clippy the cfg(windows) side of Caustica on Linux for x86_64-pc-windows-msvc, with
# the official rustup 1.99.1 toolchain in ~/.rustup (the windows-msvc std was added to it; the
# Arch rustc cannot use official std builds). No linking, no C++ bridges, nothing runs. See
# docs/claude/d3d12-on-linux.md and testing.md.
#
#   tools/check_windows.sh              cargo check, shaders as empty stubs (CAUSTICA_CHECK_ONLY=1)
#   tools/check_windows.sh check --all-targets   test code too (extra cargo arguments)
#   tools/check_windows.sh clippy       clippy -- -D warnings for the Windows target
#   CAUSTICA_CHECK_ONLY=dxil tools/check_windows.sh   also compile real DXIL (Linux DXC)
# Extra arguments go to cargo. WIN_TOOLCHAIN overrides the toolchain folder.
set -e
cd "$(dirname "$0")/.."
tc="${WIN_TOOLCHAIN:-$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu}"
mode=check
case "${1:-}" in check) shift ;; clippy) mode=clippy; shift ;; esac
export PATH="$tc/bin:$PATH" RUSTC="$tc/bin/rustc" RUSTUP_TOOLCHAIN=stable
export CARGO_TARGET_DIR="$PWD/target/wincheck" CARGO_NET_OFFLINE=true
export CAUSTICA_CHECK_ONLY="${CAUSTICA_CHECK_ONLY:-1}"
if [ $mode = clippy ]; then
  exec target/safe.sh "$tc/bin/cargo" clippy --locked --target x86_64-pc-windows-msvc "$@" -- -D warnings
fi
exec target/safe.sh "$tc/bin/cargo" check --locked --target x86_64-pc-windows-msvc "$@"
