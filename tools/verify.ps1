# -Backend d3d12|vulkan picks the renderer for every run (default: the exe's own, D3D12 on Windows).
param([switch]$Release, [ValidateSet("", "d3d12", "vulkan")][string]$Backend = "")
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot
$previousTarget = $env:CARGO_TARGET_DIR
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target/validation'
# The repo-local Vulkan validation layer (tools/setup.ps1 -VulkanLayer), for the --vulkan --debug runs.
$previousLayerPath = $env:VK_ADD_LAYER_PATH
$layerBin = Get-ChildItem (Join-Path $projectRoot 'vendor/vulkan-sdk') -Directory -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
if ($layerBin) { $env:VK_ADD_LAYER_PATH = Join-Path $layerBin.FullName 'Bin' }
try {
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) {throw 'Formatting failed'}
    & cargo test --locked
    if ($LASTEXITCODE -ne 0) {throw 'Unit tests failed'}
    & cargo clippy --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) {throw 'Clippy failed'}
    $profile = if ($Release) {'release'} else {'debug'}
    if ($Release) {& cargo build --release --locked} else {& cargo build --locked}
    if ($LASTEXITCODE -ne 0) {throw 'Build failed'}
    $exe = Join-Path $env:CARGO_TARGET_DIR "$profile/caustica.exe"
    New-Item -ItemType Directory -Force captures | Out-Null
    function Invoke-Renderer([string]$runName,[string[]]$options) {
        # Release is a GUI-subsystem executable. Explicitly wait and inspect its exit code.
        $stdout=Join-Path $projectRoot "captures/$runName.stdout.txt"
        $stderr=Join-Path $projectRoot "captures/$runName.stderr.txt"
        # Wait for the renderer itself, not -Wait's whole process tree: NVIDIA's NGX runtime starts
        # nvngx_update.exe children that outlive the app by minutes.
        if ($Backend) {$options = @("--$Backend") + $options}
        $process=Start-Process -FilePath $exe -ArgumentList $options -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        $null=$process.Handle # keeps the exit code readable after exit
        $process.WaitForExit()
        Get-Content -LiteralPath $stderr | ForEach-Object {Write-Host $_}
        Get-Content -LiteralPath $stdout | ForEach-Object {Write-Host $_}
        if ($process.ExitCode -ne 0) {throw "$runName failed with exit code $($process.ExitCode)"}
    }
    Invoke-Renderer 'integration' @('--self-test','--width','960','--height','720','--capture-ui','--capture','captures/validation-ui.png')
    Invoke-Renderer 'rr-quality' @('--headless','--frames','32','--width','960','--height','720','--scale','0.666667','--require-rr','--debug','--capture','captures/rr-quality.png')
    Invoke-Renderer 'rr-dlaa' @('--headless','--frames','16','--width','640','--height','480','--scale','1','--require-rr','--debug','--capture','captures/rr-dlaa.exr')
    Invoke-Renderer 'raw' @('--headless','--frames','32','--width','640','--height','480','--no-rr','--raw','--debug','--capture','captures/raw.png')
    # GPU against the CPU reference on three scenes at low sample counts (roadmap Plan step 4).
    # The test is statistical (tools/oracle.py), so few samples widen its tolerance but keep it
    # valid: Cornell (diffuse, area light), veach-mis (glossy, several lights, MIS) and
    # gem-display (dielectrics).
    $oracleBackend = if ($Backend) {@('--backend', $Backend)} else {@()}
    & python tools/oracle.py --exe $exe @oracleBackend --scenes 'preset:0,veach-mis,gem-display' --size 160x120 --gpu-spp 256 --cpu-spp 128 --strict --out (Join-Path $projectRoot 'captures/oracle-verify')
    if ($LASTEXITCODE -ne 0) {throw 'GPU-vs-CPU oracle failed'}
    Write-Host 'PASS: formatting, unit tests, Clippy, DXR, all 24 presets, live scene edits, native denoise, RR D/E/F, DLAA, resize, UI, AOVs, pause, A/B, PNG and EXR, GPU-vs-CPU oracle.'
} finally { $env:CARGO_TARGET_DIR=$previousTarget; $env:VK_ADD_LAYER_PATH=$previousLayerPath; Pop-Location }
