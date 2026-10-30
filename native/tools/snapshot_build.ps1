# Copies the current release build into builds/<date>-<name>/ so it can be run later on its own:
# the exe, the upscaler DLLs, the scene packs (without the thumbnail cache) and a NOTES.md.
#   powershell -File tools/snapshot_build.ps1 -Name scene-files -Notes "What changed"
param(
    [Parameter(Mandatory = $true)][string]$Name,
    [string]$Notes = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root "target\release"
if (-not (Test-Path (Join-Path $release "cornell-rtx.exe"))) { throw "Build the release exe first (cargo build --release)" }
$date = Get-Date -Format "yyyy-MM-dd"
$dest = Join-Path $root "builds\$date-$Name"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $release "cornell-rtx.exe") $dest -Force
Get-ChildItem $release -Filter *.dll | Copy-Item -Destination $dest -Force
if (Test-Path (Join-Path $release "ffmpeg.exe")) { Copy-Item (Join-Path $release "ffmpeg.exe") $dest -Force }
$scenes = Join-Path $root "scenes"
if (Test-Path $scenes) {
    Get-ChildItem $scenes -Directory | Where-Object { $_.Name -ne "thumbnails" } | ForEach-Object {
        Copy-Item $_.FullName (Join-Path $dest "scenes\$($_.Name)") -Recurse -Force
    }
}
$commit = (git -C $root rev-parse --short HEAD).Trim()
$body = @"
# $Name

- Built: $date, commit $commit
- Run: ``cornell-rtx.exe`` (double-click). Scenes are read from ``scenes\`` next to the exe; thumbnails render on first use.

$Notes
"@
Set-Content -Path (Join-Path $dest "NOTES.md") -Value $body -Encoding utf8
Write-Output "SNAPSHOT $dest"
