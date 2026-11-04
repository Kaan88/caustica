param([switch]$DebugBuild, [switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot
# Sparse, pinned checkout of a vendor SDK: only the listed paths are downloaded.
function Get-Sdk([string]$name, [string]$url, [string]$folder, [string]$commit, [string[]]$paths) {
    $sdkPath = Join-Path $projectRoot "vendor/$folder"
    if (-not (Test-Path -LiteralPath (Join-Path $sdkPath '.git'))) {
        & git clone --filter=blob:none --sparse --depth 1 --no-checkout $url $sdkPath
        if ($LASTEXITCODE -ne 0) { throw "Could not fetch $name" }
    }
    & git -C $sdkPath diff --quiet
    if ($LASTEXITCODE -ne 0) { throw "The $name checkout has local edits; preserve them before rerunning setup." }
    & git -C $sdkPath sparse-checkout set --no-cone @paths
    if ($LASTEXITCODE -ne 0) { throw "$name sparse checkout failed" }
    & git -C $sdkPath cat-file -e "$commit`^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) {
        & git -C $sdkPath fetch --depth 1 origin $commit
        if ($LASTEXITCODE -ne 0) { throw "Could not fetch the pinned $name revision" }
    }
    & git -C $sdkPath checkout --detach $commit
    if ($LASTEXITCODE -ne 0) { throw "Could not select the pinned $name revision" }
}
# ffmpeg for the turntable video export: a pinned gyan.dev "essentials" build (GitHub release
# mirror), checked against its SHA-256. Only ffmpeg.exe and its LICENSE are kept.
function Get-Ffmpeg([string]$version, [string]$sha256) {
    $target = Join-Path $projectRoot 'vendor/ffmpeg'
    $exe = Join-Path $target 'ffmpeg.exe'
    if ((Test-Path -LiteralPath $exe) -and (Test-Path -LiteralPath (Join-Path $target "VERSION-$version"))) { return }
    New-Item -ItemType Directory -Force $target | Out-Null
    $zip = Join-Path ([IO.Path]::GetTempPath()) "ffmpeg-$version-essentials_build.zip"
    Invoke-WebRequest -Uri "https://github.com/GyanD/codexffmpeg/releases/download/$version/ffmpeg-$version-essentials_build.zip" -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $zip; throw "ffmpeg $version checksum mismatch: $actual" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($entry in $archive.Entries) {
            if ($entry.FullName -like '*/bin/ffmpeg.exe') { [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $exe, $true) }
            if ($entry.FullName -like '*/LICENSE') { [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $target 'LICENSE'), $true) }
        }
    } finally { $archive.Dispose() }
    Remove-Item -LiteralPath $zip
    Get-ChildItem $target -Filter 'VERSION-*' | Remove-Item
    Set-Content -LiteralPath (Join-Path $target "VERSION-$version") -Value $sha256
}
# D3D12 Agility SDK (NuGet Microsoft.Direct3D.D3D12): D3D12Core.dll for Shader Model 6.9 and
# Shader Execution Reordering (track 20). The exe exports D3D12SDKVersion / D3D12SDKPath
# (src/main.rs) and the runtime loads it from D3D12\ next to the exe; build.rs copies it there.
function Get-Agility([string]$version, [string]$sha256) {
    $target = Join-Path $projectRoot 'vendor/agility'
    if ((Test-Path -LiteralPath (Join-Path $target 'D3D12Core.dll')) -and (Test-Path -LiteralPath (Join-Path $target "VERSION-$version"))) { return }
    New-Item -ItemType Directory -Force $target | Out-Null
    $zip = Join-Path ([IO.Path]::GetTempPath()) "agility-$version.zip"
    Invoke-WebRequest -Uri "https://api.nuget.org/v3-flatcontainer/microsoft.direct3d.d3d12/$version/microsoft.direct3d.d3d12.$version.nupkg" -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $zip; throw "Agility SDK $version checksum mismatch: $actual" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($entry in $archive.Entries) {
            if ($entry.FullName -in 'build/native/bin/x64/D3D12Core.dll', 'build/native/bin/x64/d3d12SDKLayers.dll', 'LICENSE.txt') {
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $target $entry.Name), $true)
            }
        }
    } finally { $archive.Dispose() }
    Remove-Item -LiteralPath $zip
    Get-ChildItem $target -Filter 'VERSION-*' | Remove-Item
    Set-Content -LiteralPath (Join-Path $target "VERSION-$version") -Value $sha256
}
# DirectXShaderCompiler release build (GitHub): the Windows SDK's dxc has no SPIR-V code
# generation, which the Vulkan backend needs (`dxc -spirv`). Kept apart from the SDK dxc that
# build.rs uses for DXIL.
function Get-DxcSpirv([string]$tag, [string]$file, [string]$sha256) {
    $target = Join-Path $projectRoot 'vendor/dxc-spirv'
    if ((Test-Path -LiteralPath (Join-Path $target 'bin/x64/dxc.exe')) -and (Test-Path -LiteralPath (Join-Path $target "VERSION-$tag"))) { return }
    New-Item -ItemType Directory -Force $target | Out-Null
    $zip = Join-Path ([IO.Path]::GetTempPath()) $file
    Invoke-WebRequest -Uri "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$tag/$file" -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $zip; throw "DXC $tag checksum mismatch: $actual" }
    Expand-Archive -LiteralPath $zip -DestinationPath $target -Force
    Remove-Item -LiteralPath $zip
    Get-ChildItem $target -Filter 'VERSION-*' | Remove-Item
    Set-Content -LiteralPath (Join-Path $target "VERSION-$tag") -Value $sha256
}
try {
    foreach ($command in @('git','cargo')) { if (-not (Get-Command $command -ErrorAction SilentlyContinue)) { throw "$command is required. Install Rust (MSVC) and Git first." } }
    # NVIDIA DLSS 310.9.16: Ray Reconstruction and Super Resolution.
    Get-Sdk 'NVIDIA DLSS SDK' 'https://github.com/NVIDIA/DLSS.git' 'dlss' '9981c6350dc58c0ce21f024fd65e31e851c31689' `
        @('/include/', '/lib/Windows_x86_64/x64/', '/lib/Windows_x86_64/rel/', '/doc/', '/LICENSE.txt', '/README.md')
    # AMD FSR SDK 2.3.0: FidelityFX API headers and the signed loader/upscaler DLLs.
    Get-Sdk 'AMD FSR SDK' 'https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK.git' 'fsr' '60f4ea81909200d8542eca14dccb2628b763a9a3' `
        @('/Kits/FidelityFX/api/include/', '/Kits/FidelityFX/upscalers/include/', '/Kits/FidelityFX/docs/license.md',
          '/Kits/FidelityFX/signedbin/amd_fidelityfx_loader_dx12.dll', '/Kits/FidelityFX/signedbin/amd_fidelityfx_upscaler_dx12.dll')
    # Intel XeSS SDK 3.0.2: headers and the super-resolution DLL.
    Get-Sdk 'Intel XeSS SDK' 'https://github.com/intel/xess.git' 'xess' '8fe81bdbbaf00b3c1b733fd0d830c333dc84e6f0' `
        @('/inc/xess/', '/bin/libxess.dll', '/LICENSE.txt', '/third-party-programs.txt')
    # ffmpeg 9.0.2 (2026-09-19) for MP4 turntables; the app falls back to PNG frames without it.
    # D3D12 Agility SDK 1.620.1 (retail; must match D3D12SDKVersion in src/main.rs).
    Get-Agility '1.620.1' 'a8a494c3e5f687d9be0a465ec8248da70781ab689d888db8806b600422722be6'
    # DXC 1.9.2609 with SPIR-V output (Vulkan backend groundwork).
    Get-DxcSpirv 'v1.9.2609' 'dxc_2026_09_29.zip' 'ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1'
    Get-Ffmpeg '9.0.2' '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba'
    if (-not $SkipBuild) {
        if ($DebugBuild) { & cargo build --locked } else { & cargo build --release --locked }
        if ($LASTEXITCODE -ne 0) { throw 'Build failed. Install Visual Studio C++ Build Tools and Windows SDK (DXC), or set DXC_PATH.' }
        $profile = if ($DebugBuild) {'debug'} else {'release'}
        Write-Host "Ready: $projectRoot\target\$profile\caustica.exe"
    }
} finally { Pop-Location }
