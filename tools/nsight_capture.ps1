# Track 20a: Nsight Graphics GPU Trace of the benchmark scenes, metrics exported as files.
# Run from the repo root, in a normal (unsandboxed) shell:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/nsight_capture.ps1 [-Only sponza]
# Needs the profiling build (CAUSTICA_SHADER_DEBUG=1 into target/profile) and GPU counter access
# (NVIDIA Control Panel > Developer > Manage GPU Performance Counters > all users).
# No --platform: Qt takes it as its own platform-plugin option and fails to start.
# --metric-set-name, --multi-pass-metrics and --real-time-shader-profiler are per-architecture:
# without --architecture ngfx ignores them silently (REPRO_INFO.xls then says "Warp State
# Sampling Disabled" and the metric set falls back to Async Compute Triage).
# Each scene writes captures/perf/nsight/<name>/ (the trace and the auto-exported metrics).
param([string]$Only = "", [string]$Ngfx = "", [string]$Arch = "Ada", [string]$MetricSet = "Top-Level Triage")
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Ngfx) {
    $Ngfx = Get-ChildItem "C:\Program Files\NVIDIA Corporation\Nsight Graphics*\host\windows-desktop-nomad-x64\ngfx.exe" |
        Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
if (-not $Ngfx) { throw "ngfx.exe not found; pass -Ngfx" }
$exe = Join-Path $root "target\profile\release\caustica.exe"
if (-not (Test-Path $exe)) { throw "Build the profiling exe first: `$env:CARGO_TARGET_DIR='target/profile'; `$env:CAUSTICA_SHADER_DEBUG='1'; cargo build --release" }
$scenes = [ordered]@{
    sponza  = "--scene scenes/meshes/sponza.json"
    mirror  = "--preset 17"
    classic = "--preset 0"
    dragon  = "--scene scenes/meshes/glass-dragon.json"
}
foreach ($name in $scenes.Keys) {
    if ($Only -and $name -ne $Only) { continue }
    $out = Join-Path $root "captures\perf\nsight\$name"
    New-Item -ItemType Directory -Force $out | Out-Null
    Write-Host "== $name =="
    # max_samples keeps the trace running past the 4096-sample cap; --frames ends the app if
    # ngfx doesn't. Multi-pass metrics need extra frames: 900 ran out on Classic (~5 ms frames).
    $appArgs = "$($scenes[$name]) --no-vsync --set max_samples=1000000 --frames 20000"
    Push-Location (Split-Path $Ngfx)
    try {
        & $Ngfx --activity "GPU Trace Profiler" --exe $exe --dir $root --args $appArgs `
            --start-after-ms 6000 --limit-to-frames 3 --auto-export --output-dir $out `
            --architecture $Arch --metric-set-name $MetricSet --multi-pass-metrics --real-time-shader-profiler
        Write-Host "ngfx exit $LASTEXITCODE"
    } finally { Pop-Location }
    Get-ChildItem $out -Recurse -File | ForEach-Object { Write-Host "  $($_.FullName) $($_.Length)" }
}
