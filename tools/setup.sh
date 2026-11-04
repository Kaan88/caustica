#!/bin/sh
# Linux counterpart of tools/setup.ps1: fetches the pinned DXC release (SPIR-V output) into
# vendor/dxc-linux, where build.rs and hotreload.rs look for it. Same release as Get-DxcSpirv.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tag=v1.9.2609
file=linux_dxc_2026_09_28.x86_x64.tar.gz
sha256=96faadc7f5c282d2ffda49804beb4c3ee38127bc252b723234e3c5cdf7aa39a1
target="$root/vendor/dxc-linux"
if [ -x "$target/bin/dxc" ] && [ -f "$target/VERSION-$tag" ]; then
    echo "DXC $tag present"
    exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$file" "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$tag/$file"
echo "$sha256  $tmp/$file" | sha256sum -c -
rm -rf "$target"
mkdir -p "$target"
tar -xzf "$tmp/$file" -C "$target"
echo "$sha256" > "$target/VERSION-$tag"
"$target/bin/dxc" --version
