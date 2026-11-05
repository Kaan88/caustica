#!/bin/sh
# Linux counterpart of tools/setup.ps1: the pinned DXC release (SPIR-V output) into
# vendor/dxc-linux, where build.rs and hotreload.rs look for it (same release as Get-DxcSpirv),
# and the SDKs bridge/vk.cpp builds against for DLSS on Linux (engine-notes "Linux DLSS"): the
# DLSS SDK's headers, static NGX library and Linux runtimes, the XeSS headers (Intel ships no
# Linux libxess.so) and the Vulkan headers, at the same commits as setup.ps1.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)

# get_sdk <name> <url> <folder> <commit> <sparse path>...: as Get-Sdk in setup.ps1.
get_sdk() {
    name=$1 url=$2 dir="$root/vendor/$3" commit=$4
    shift 4
    if [ ! -d "$dir/.git" ]; then
        git clone --filter=blob:none --sparse --depth 1 --no-checkout "$url" "$dir"
    fi
    git -C "$dir" diff --quiet || { echo "The $name checkout has local edits; preserve them first." >&2; exit 1; }
    git -C "$dir" sparse-checkout set --no-cone "$@"
    git -C "$dir" cat-file -e "$commit^{commit}" 2>/dev/null || git -C "$dir" fetch --depth 1 origin "$commit"
    git -C "$dir" checkout -q --detach "$commit"
    echo "$name $commit"
}

get_dxc() {
    tag=v1.9.2609
    file=linux_dxc_2026_09_28.x86_x64.tar.gz
    sha256=96faadc7f5c282d2ffda49804beb4c3ee38127bc252b723234e3c5cdf7aa39a1
    target="$root/vendor/dxc-linux"
    if [ -x "$target/bin/dxc" ] && [ -f "$target/VERSION-$tag" ]; then
        echo "DXC $tag present"
        return
    fi
    tmp=$(mktemp -d)
    curl -fsSL -o "$tmp/$file" "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$tag/$file"
    echo "$sha256  $tmp/$file" | sha256sum -c -
    rm -rf "$target"
    mkdir -p "$target"
    tar -xzf "$tmp/$file" -C "$target"
    rm -rf "$tmp"
    echo "$sha256" > "$target/VERSION-$tag"
    "$target/bin/dxc" --version
}

get_dxc
get_sdk 'NVIDIA DLSS SDK' https://github.com/NVIDIA/DLSS.git dlss 9981c6350dc58c0ce21f024fd65e31e851c31689 \
    /include/ /lib/Linux_x86_64/libnvsdk_ngx.a '/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.*' \
    '/lib/Linux_x86_64/rel/libnvidia-ngx-dlssd.so.*' /LICENSE.txt /README.md
get_sdk 'Intel XeSS SDK' https://github.com/intel/xess.git xess 8fe81bdbbaf00b3c1b733fd0d830c333dc84e6f0 \
    /inc/xess/ /LICENSE.txt
get_sdk 'Vulkan headers' https://github.com/KhronosGroup/Vulkan-Headers.git vulkan-headers 9fbc2a782ee3f8a4db64e7d991e23da75e893f44 \
    /include/vulkan/ /include/vk_video/ /LICENSE.md
