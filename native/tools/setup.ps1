param([switch]$DebugBuild, [switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot
try {
    foreach ($command in @('git','cargo')) { if (-not (Get-Command $command -ErrorAction SilentlyContinue)) { throw "$command is required. Install Rust (MSVC) and Git first." } }
    $sdkCommit = '9981c6350dc58c0ce21f024fd65e31e851c31689'
    $sdkPath = Join-Path $projectRoot 'vendor/dlss'
    if (-not (Test-Path -LiteralPath (Join-Path $sdkPath '.git'))) {
        & git clone --filter=blob:none --sparse --depth 1 https://github.com/NVIDIA/DLSS.git $sdkPath
        if ($LASTEXITCODE -ne 0) { throw 'Could not fetch NVIDIA DLSS SDK' }
    }
    & git -C $sdkPath diff --quiet
    if ($LASTEXITCODE -ne 0) { throw 'The vendor SDK has local edits; preserve them before rerunning setup.' }
    & git -C $sdkPath sparse-checkout set include lib/Windows_x86_64/x64 lib/Windows_x86_64/rel doc
    if ($LASTEXITCODE -ne 0) { throw 'SDK sparse checkout failed' }
    & git -C $sdkPath cat-file -e "$sdkCommit`^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) {
        & git -C $sdkPath fetch --depth 1 origin $sdkCommit
        if ($LASTEXITCODE -ne 0) { throw 'Could not fetch pinned SDK revision' }
    }
    & git -C $sdkPath checkout --detach $sdkCommit
    if ($LASTEXITCODE -ne 0) { throw 'Could not select pinned SDK revision' }
    if (-not $SkipBuild) {
        if ($DebugBuild) { & cargo build --locked } else { & cargo build --release --locked }
        if ($LASTEXITCODE -ne 0) { throw 'Build failed. Install Visual Studio C++ Build Tools and Windows SDK (DXC), or set DXC_PATH.' }
        $profile = if ($DebugBuild) {'debug'} else {'release'}
        Write-Host "Ready: $projectRoot\target\$profile\cornell-rtx.exe"
    }
} finally { Pop-Location }
