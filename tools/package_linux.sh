#!/bin/sh
# Linux packages from a staged tree (tools/appimage.sh with STAGE_OUT=<dir> leaves one: the exe,
# the NGX and OIDN libraries, scenes, licences). Writes to dist/:
#   caustica-<V>-x86_64.tar.gz           (the tree plus a run note)
#   caustica_<V>_amd64.deb, caustica-<V>-1-x86_64.pkg.tar.zst, caustica-<V>-1.x86_64.rpm
# The three native packages come from packaging/nfpm.yaml and need `nfpm` on PATH (or $NFPM);
# without it only the tar.gz is made.
#   sh tools/package_linux.sh <stage dir> [version] [out dir]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
stage=$(cd "${1:?usage: package_linux.sh <stage dir> [version] [out dir]}" && pwd)
version=${2:-$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)}
out=${3:-$root/dist}
mkdir -p "$out"
out=$(cd "$out" && pwd)
[ -x "$stage/caustica" ] || { echo "$stage/caustica is missing" >&2; exit 1; }

# tar.gz: one folder, run ./caustica from it. Same files as the AppImage.
name=caustica-$version-x86_64
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/$name"
cp -a "$stage/." "$work/$name/"
cat > "$work/$name/RUN.txt" <<'NOTE'
Caustica, Linux x86_64 (Vulkan).

Run ./caustica from this folder. Needs a GPU with Vulkan ray query support and its driver
(the Vulkan loader, libvulkan.so.1, comes from your distribution), plus OpenSSL 3, libudev and
libxkbcommon. Video capture uses ffmpeg from PATH if installed.

Settings, user scenes and captures go to ~/.config/caustica and ~/.cache/caustica. To keep them
beside the exe instead (a portable copy), create an empty file named "portable" in this folder.
NOTE
tar --owner=0 --group=0 -C "$work" -czf "$out/$name.tar.gz" "$name"
echo "TARGZ $out/$name.tar.gz"

nfpm=${NFPM:-$(command -v nfpm || true)}
if [ -z "$nfpm" ]; then
    echo "nfpm not found: skipping .deb, .pkg.tar.zst and .rpm" >&2
    exit 0
fi
cd "$root"
# nfpm expands env vars in the version but not in content paths: put the stage path in a copy.
export VERSION=$version
sed "s|\${STAGE}|$stage|" packaging/nfpm.yaml > "$work/nfpm.yaml"
"$nfpm" package -f "$work/nfpm.yaml" -p deb -t "$out/caustica_${version}_amd64.deb"
"$nfpm" package -f "$work/nfpm.yaml" -p archlinux -t "$out/caustica-${version}-1-x86_64.pkg.tar.zst"
"$nfpm" package -f "$work/nfpm.yaml" -p rpm -t "$out/caustica-${version}-1.x86_64.rpm"
echo "PACKAGES_OK $out"
