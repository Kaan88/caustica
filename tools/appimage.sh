#!/bin/sh
# Builds dist/Caustica-<version>-x86_64.AppImage on Linux: the release exe (Vulkan), the shipped
# scenes and the README, packed by a pinned appimagetool. Releases are built and uploaded from
# here, not CI: `gh release upload v<version> dist/Caustica-<version>-x86_64.AppImage`.
#
# The AppImage mounts read-only, which is the app's installed mode (src/dirs.rs): settings, user
# scenes, downloads and captures go to $XDG_CONFIG_HOME/caustica, caches to $XDG_CACHE_HOME. The
# Vulkan loader and the GPU driver come from the system (an AppImage must not bundle them; DLSS's
# NGX runtimes are bundled), and ffmpeg is found on PATH. glibc: it runs on distros with at least the build machine's glibc.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
sh tools/setup.sh
cargo build --release --locked
exe="${CARGO_TARGET_DIR:-$root/target}/release/caustica"

tool_version=1.9.1
tool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
tool="$root/vendor/appimagetool-$tool_version"
if [ ! -x "$tool" ]; then
    mkdir -p "$root/vendor"
    curl -fsSL -o "$tool.part" "https://github.com/AppImage/appimagetool/releases/download/$tool_version/appimagetool-x86_64.AppImage"
    echo "$tool_sha256  $tool.part" | sha256sum -c -
    chmod +x "$tool.part"
    mv "$tool.part" "$tool"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app="$work/Caustica.AppDir"
mkdir -p "$app/usr/bin"
cp "$exe" "$app/usr/bin/caustica"
# DLSS RR and SR (build.rs copies NGX's runtimes beside the exe; bridge/vk.cpp looks there).
for so in "$(dirname "$exe")"/libnvidia-ngx-dlss*.so.*; do
    if [ -f "$so" ]; then cp "$so" "$app/usr/bin/"; fi
done
# dirs::scenes() looks beside the exe. The fetched assets must be there too (release.ps1 copies
# them from Windows; the mesh, environment and material scenes load them).
for d in models hdri textures; do
    [ -d "scenes/$d" ] || { echo "scenes/$d is missing: copy the fetched assets (tools/fetch_assets.ps1)" >&2; exit 1; }
done
cp -r scenes "$app/usr/bin/scenes"
cp README.md THIRD-PARTY-NOTICES.md "$app/usr/bin/"
mkdir -p "$app/usr/bin/licenses"
cp vendor/dlss/LICENSE.txt "$app/usr/bin/licenses/NVIDIA-DLSS.txt"
cp assets/caustica.png "$app/caustica.png"
cat > "$app/caustica.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Caustica
Comment=Path tracer
Exec=caustica
Icon=caustica
Categories=Graphics;3DGraphics;
Terminal=false
EOF
cat > "$app/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/caustica" "$@"
EOF
chmod +x "$app/AppRun"

mkdir -p dist
out="$root/dist/Caustica-$version-x86_64.AppImage"
# Extract-and-run: no FUSE needed on the build machine. appimagetool fetches its runtime.
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$app" "$out"
echo "APPIMAGE_OK $out"
