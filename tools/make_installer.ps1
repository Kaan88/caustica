# Builds the installer locally, the way CI's Package and Installer steps do (.github/workflows/ci.yml),
# without waiting for GitHub. Needs Inno Setup 6.3+ or 7, a release build, and the assets
# (tools/fetch_assets.ps1).
#   powershell -File tools/make_installer.ps1 [-Source target\release] [-Version 0.3.1-local]
# Stages dist\caustica-v<version>\ (exe, DLLs, ffmpeg, README, the tracked scene files plus the
# fetched models, HDRIs and textures, their prebuilt thumbnails; not scenes\user or the asset browser's assets.json, which
# CI doesn't have) and writes dist\caustica-v<version>-setup.exe.
param([string]$Source = "target\release", [string]$Version = "")
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$release = if ([IO.Path]::IsPathRooted($Source)) { $Source } else { Join-Path $root $Source }
if (-not (Test-Path (Join-Path $release 'caustica.exe'))) { throw "No caustica.exe in $release (cargo build --release)" }
if (-not $Version) {
    $base = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $Version = "$base-local"
}
$iscc = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -like 'Inno Setup*' -and $_.InstallLocation } |
    ForEach-Object { Join-Path $_.InstallLocation 'ISCC.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup (ISCC.exe) not found" }
foreach ($d in 'models', 'hdri', 'textures') {
    if (-not (Test-Path (Join-Path $root "scenes\$d"))) { throw "scenes\$d is missing: run tools/fetch_assets.ps1" }
}
$dist = Join-Path $root "dist\caustica-v$Version"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force $dist | Out-Null
Get-ChildItem $release -File | Where-Object { $_.Name -eq 'caustica.exe' -or $_.Name -eq 'ffmpeg.exe' -or $_.Extension -eq '.dll' } |
    Copy-Item -Destination $dist
# The D3D12 Agility SDK runtime (Shader Model 6.9, SER), which the exe loads from .\D3D12\.
if (Test-Path (Join-Path $release 'D3D12')) { Copy-Item -Recurse (Join-Path $release 'D3D12') (Join-Path $dist 'D3D12') }
Copy-Item (Join-Path $root 'README.md') $dist
Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.md') $dist
# The licence texts THIRD-PARTY-NOTICES.md points at. A missing one stops the build.
$licenses = Join-Path $dist 'licenses'
New-Item -ItemType Directory -Force $licenses | Out-Null
foreach ($l in @(
        @('vendor\dlss\LICENSE.txt', 'NVIDIA-DLSS.txt'),
        @('vendor\fsr\Kits\FidelityFX\docs\license.md', 'AMD-FSR.md'),
        @('vendor\xess\LICENSE.txt', 'Intel-XeSS.txt'),
        @('vendor\oidn\doc\LICENSE.txt', 'OIDN-Apache-2.0.txt'),
        @('vendor\oidn\doc\third-party-programs.txt', 'OIDN-third-party-programs.txt'),
        @('vendor\oidn\doc\third-party-programs-oneTBB.txt', 'OIDN-oneTBB-third-party-programs.txt'),
        @('vendor\agility\LICENSE.txt', 'Microsoft-Agility.txt'),
        @('vendor\ffmpeg\LICENSE', 'FFmpeg.txt'),
        @('LICENSE', 'Caustica-MIT.txt'),
        @('assets\THREE-LICENSE.txt', 'three.js-MIT.txt'),
        @('assets\fonts\IBM-Plex-LICENSE.txt', 'IBM-Plex-OFL.txt'),
        @('assets\CREDITS.md', 'Mesh-credits.md'))) {
    Copy-Item -LiteralPath (Join-Path $root $l[0]) -Destination (Join-Path $licenses $l[1])
}
Push-Location $root
try {
    $files = @(git ls-files scenes) + @(Get-ChildItem scenes\models, scenes\hdri, scenes\textures -Recurse -File |
        Where-Object { $_.Extension -ne '.partial' } |
        ForEach-Object { (Resolve-Path -Relative $_.FullName).TrimStart('.', '\') })
} finally { Pop-Location }
foreach ($f in ($files | Sort-Object -Unique)) {
    $target = Join-Path $dist $f
    New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
    Copy-Item -LiteralPath (Join-Path $root $f) -Destination $target
}
Write-Output "Staged $($files.Count) scene files into $dist"
# The thumbnails of those scenes and of the material presets, rendered by the staged exe itself
# (so from the staged scene files) into dist\...\thumbnails, which ships beside the exe: a fresh
# install copies them into its cache instead of rendering for minutes. Each is named by the hash
# of its scene file / material, so one that is edited later is ignored and re-rendered (src/scenes.rs).
# Needs a GPU on this machine; a failure stops the build rather than ship without them.
$thumbs = Join-Path $dist 'thumbnails'
# caustica.exe is a GUI-subsystem exe: `&` returns at once without waiting, so the check below
# raced the renders. Wait for that process alone: Start-Process -Wait also waits for every
# descendant, and each DLSS init leaves the driver's nvngx_update.exe running, so it never returns.
$p = Start-Process (Join-Path $dist 'caustica.exe') -ArgumentList '--make-thumbnails', '--thumbnails-out', "`"$thumbs`"" -PassThru
$null = $p.Handle # keeps the handle open, or ExitCode reads empty after the exit
$p.WaitForExit()
if ($p.ExitCode) { throw "--make-thumbnails failed with exit code $($p.ExitCode)" }
$thumbCount = @(Get-ChildItem $thumbs -Recurse -File -Filter *.png).Count
if ($thumbCount -eq 0) { throw "--make-thumbnails made no thumbnails" }
Write-Output "Bundled $thumbCount thumbnails into $thumbs"
& $iscc /Qp "/DAppVersion=$Version" "/DSourceDir=$dist" (Join-Path $root 'installer\caustica.iss')
if ($LASTEXITCODE) { throw "ISCC failed with exit code $LASTEXITCODE" }
$setup = Join-Path $root "dist\caustica-v$Version-setup.exe"
Write-Output "INSTALLER $setup $([math]::Round((Get-Item $setup).Length / 1MB)) MB"
