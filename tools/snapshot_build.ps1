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
if (-not (Test-Path (Join-Path $release "caustica.exe"))) { throw "Build the release exe first (cargo build --release)" }
$date = Get-Date -Format "yyyy-MM-dd"
$dest = Join-Path $root "builds\$date-$Name"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $release "caustica.exe") $dest -Force
Get-ChildItem $release -Filter *.dll | Copy-Item -Destination $dest -Force
if (Test-Path (Join-Path $release "ffmpeg.exe")) { Copy-Item (Join-Path $release "ffmpeg.exe") $dest -Force }
$scenes = Join-Path $root "scenes"
if (Test-Path $scenes) {
    Get-ChildItem $scenes -Directory | Where-Object { $_.Name -ne "thumbnails" } | ForEach-Object {
        $target = Join-Path $dest "scenes\$($_.Name)"
        if ($_.Name -in @("models", "hdri", "textures")) {
            # Downloaded assets (hundreds of MB) are hard-linked, not copied: same files, no extra
            # disk per snapshot. They never change in place (fetch_assets.ps1 pins each by hash).
            $source = $_.FullName
            Get-ChildItem $source -File -Recurse | ForEach-Object {
                $link = Join-Path $target $_.FullName.Substring($source.Length + 1)
                New-Item -ItemType Directory -Force (Split-Path -Parent $link) | Out-Null
                if (-not (Test-Path $link)) { New-Item -ItemType HardLink -Path $link -Target $_.FullName | Out-Null }
            }
        } else {
            Copy-Item $_.FullName $target -Recurse -Force
        }
    }
}
$commit = (git -C $root rev-parse --short HEAD).Trim()
$body = @"
# $Name

- Built: $date, commit $commit
- Run: ``caustica.exe`` (double-click). Scenes are read from ``scenes\`` next to the exe; thumbnails render on first use.

$Notes
"@
Set-Content -Path (Join-Path $dest "NOTES.md") -Value $body -Encoding utf8
Write-Output "SNAPSHOT $dest"
