# Builds and uploads a release from this machine (CI costs money; .github/workflows/ci.yml is
# manual-only). Run after the release commit is pushed and tagged:
#   git tag v0.10.0; git push origin v0.10.0
#   ./tools/release.ps1 -Version 0.10.0 [-AppImage] [-Upload]
# Writes dist/caustica-v<V>-setup.exe and dist/caustica-v<V>.zip (tools/make_installer.ps1: Inno
# Setup 7 or 6.3+, the assets from tools/fetch_assets.ps1) and, with -AppImage,
# dist/Caustica-<V>-x86_64.AppImage
# built on the Linux VM (ssh caustica-vm) from the tagged commit by tools/appimage.sh. -Upload
# creates the GitHub release with them and notes listing the commits since the previous tag.
# Builds into target/dist, so the launcher's target/release (and a running app) is untouched.
param(
    [string]$Version = (Select-String -Path "$PSScriptRoot/../Cargo.toml" -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value,
    [switch]$AppImage,
    [switch]$Upload,
    [string]$Vm = 'caustica-vm'
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot/.."
Set-Location $root
$tag = "v$Version"
if ($Upload) {
    git rev-parse -q --verify "refs/tags/$tag" | Out-Null
    if ($LASTEXITCODE) { throw "Tag $tag does not exist: tag and push the release commit first." }
}

# The Windows build, staged and packed by tools/make_installer.ps1 (exe, DLLs, D3D12\, ffmpeg,
# README, the tracked scenes plus the fetched assets; finds Inno Setup 6.3+ or 7), then zipped.
$env:CAUSTICA_VERSION = $Version
$env:CARGO_TARGET_DIR = "target/dist"
cargo build --release --locked
if ($LASTEXITCODE) { throw "cargo build failed" }
& "$PSScriptRoot/make_installer.ps1" -Source target/dist/release -Version $Version
$name = "caustica-v$Version"
$zip = "dist/$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path "dist/$name" -DestinationPath $zip
$files = @($zip, "dist/$name-setup.exe")
"ZIP $zip"

if ($AppImage) {
    # The tagged commit (or HEAD) as a clean tree on the VM, built there, copied back.
    $ref = if ($Upload) { $tag } else { 'HEAD' }
    $remote = "caustica-release"
    # vendor/ (the SDKs, DXC, appimagetool) is kept between releases.
    ssh $Vm "mkdir -p ~/$remote && find ~/$remote -mindepth 1 -maxdepth 1 ! -name vendor -exec rm -rf {} +"
    if ($LASTEXITCODE) { throw "ssh $Vm failed" }
    cmd /c "git archive --format=tar $ref | ssh $Vm tar -x -C ~/$remote"
    if ($LASTEXITCODE) { throw "copying the tree to $Vm failed" }
    # The VM keeps cargo under its data disk (not in ~/.cargo).
    ssh $Vm "export PATH=/mnt/2b4bc8c0-2e4e-4fc1-b330-27648dcbbf49/cargo/bin:`$PATH; cd ~/$remote && CAUSTICA_VERSION=$Version sh tools/appimage.sh"
    if ($LASTEXITCODE) { throw "tools/appimage.sh failed on $Vm" }
    $image = "Caustica-$Version-x86_64.AppImage"
    scp "${Vm}:$remote/dist/$image" "dist/$image"
    if ($LASTEXITCODE) { throw "copying the AppImage back failed" }
    $files += "dist/$image"
    "APPIMAGE dist/$image"
}

if ($Upload) {
    # Notes: every commit subject since the previous release tag.
    $previous = git describe --tags --abbrev=0 --match 'v*' "$tag^" 2>$null
    $range = if ($previous) { "$previous..$tag" } else { $tag }
    $log = git log $range --no-merges --format='- %s (%h)'
    $header = if ($previous) { "Changes since $previous`n" } else { "First release`n" }
    Set-Content -Encoding utf8 dist/notes.md ($header + "`n" + ($log -join "`n"))
    gh release create $tag @files --title "Caustica $tag" --notes-file dist/notes.md
    if ($LASTEXITCODE) { throw "gh release create failed" }
    "RELEASED $tag"
}
