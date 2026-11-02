# Packages the release build as a Windows screensaver in builds\screensaver\:
# Caustica.scr (the same exe), the upscaler DLLs it loads from its own folder, and the scenes.
# Install: right-click Caustica.scr > Install (keep the folder; the .scr needs the DLLs beside it).
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root "target\release"
if (-not (Test-Path (Join-Path $release "caustica.exe"))) { throw "Build the release exe first (cargo build --release)" }
$dest = Join-Path $root "builds\screensaver"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $release "caustica.exe") (Join-Path $dest "Caustica.scr") -Force
Get-ChildItem $release -Filter *.dll | Copy-Item -Destination $dest -Force
if (Test-Path (Join-Path $release "ffmpeg.exe")) { Copy-Item (Join-Path $release "ffmpeg.exe") $dest -Force }
Get-ChildItem (Join-Path $root "scenes") -Directory | Where-Object { $_.Name -ne "thumbnails" } | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $dest "scenes\$($_.Name)") -Recurse -Force
}
Write-Output "SCREENSAVER $dest\Caustica.scr (right-click > Install)"
