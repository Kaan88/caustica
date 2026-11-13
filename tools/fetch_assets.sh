#!/bin/sh
# Linux twin of tools/fetch_assets.ps1: downloads the test models (scenes/models/), HDRIs
# (scenes/hdri/) and texture sets (scenes/textures/). The URLs and SHA-256 pins are read from
# fetch_assets.ps1 and sponza.sha256, so that script stays the one list to edit.
#   sh tools/fetch_assets.sh
# Needs curl, sha256sum and unzip.
set -eu
tools=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$tools")
ps1="$tools/fetch_assets.ps1"

# Base URLs: the `$name = 'https://…'` lines of the .ps1.
base() { sed -n "s/^\$$1 = '\(https[^']*\)'.*/\1/p" "$ps1"; }
obj=$(base obj); gltf=$(base gltf); hdri=$(base hdri); phtex=$(base phtex)

fetch() { # url sha256 folder
    url=$1 sha=$2 dir="$root/scenes/$3"
    mkdir -p "$dir"
    file="$dir/$(basename "$url")"
    if [ -f "$file" ] && [ "$(sha256sum "$file" | cut -d' ' -f1)" = "$sha" ]; then return; fi
    curl -fsSL --retry 3 -o "$file.partial" "$url"
    actual=$(sha256sum "$file.partial" | cut -d' ' -f1)
    if [ "$actual" != "$sha" ]; then rm -f "$file.partial"; echo "$url checksum mismatch: $actual" >&2; exit 1; fi
    mv -f "$file.partial" "$file"
    echo "FETCHED $(basename "$url")"
}

# Pinned files: `@("$var/path", 'sha256'),` lines; the variable picks the folder.
sed -n "s/^ *@(\"\$\([a-z]*\)\/\([^\"]*\)\", '\([0-9a-f]\{64\}\)').*/\1 \2 \3/p" "$ps1" |
while read -r var path sha; do
    case $var in
        obj) fetch "$obj/$path" "$sha" models ;;
        gltf) fetch "$gltf/$path" "$sha" models ;;
        hdri) fetch "$hdri/$path" "$sha" hdri ;;
        phtex) fetch "$phtex/$path" "$sha" "textures/$(basename "$(dirname "$path")")" ;;
        *) echo "unknown base \$$var in $ps1" >&2; exit 1 ;;
    esac
done

# Crytek Sponza: "file sha256" per line.
while read -r name sha; do
    [ -n "$name" ] && fetch "$gltf/Sponza/glTF/$name" "$sha" models/Sponza
done < "$tools/sponza.sha256"

# ambientCG zips: `@('Id', 'sha256'),` lines; only the colour, normal, roughness and metalness maps.
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
sed -n "s/^ *@('\([A-Za-z0-9]*\)', '\([0-9a-f]\{64\}\)').*/\1 \2/p" "$ps1" |
while read -r id sha; do
    dir="$root/scenes/textures/$id"
    [ -f "$dir/${id}_2K-JPG_Color.jpg" ] && continue
    zip="$tmp/$id.zip"
    curl -fsSL --retry 3 -A caustica -o "$zip" "https://ambientcg.com/get?file=${id}_2K-JPG.zip"
    actual=$(sha256sum "$zip" | cut -d' ' -f1)
    if [ "$actual" != "$sha" ]; then echo "$id checksum mismatch: $actual" >&2; exit 1; fi
    mkdir -p "$dir"
    unzip -qjo "$zip" '*_Color.jpg' '*_NormalGL.jpg' '*_Roughness.jpg' '*_Metalness.jpg' -d "$dir" 2>/dev/null || [ -f "$dir/${id}_2K-JPG_Color.jpg" ]
    rm -f "$zip"
    echo "FETCHED $id"
done
echo "ASSETS $root/scenes"
