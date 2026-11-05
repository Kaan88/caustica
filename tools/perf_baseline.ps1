# Track 20a step 1: baseline sweep. Writes captures/perf/baseline.csv (rename it to keep a run).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File tools/perf_baseline.ps1
param([string]$Exe = "target/validation/release/caustica.exe", [int]$Frames = 300)
$out = "captures/perf/baseline.csv"
"case,mode,extra,gpu_ms,frame_ms,fence_wait_ms,trace_ms,reconstruct_ms,display_ms,ui_ms" | Set-Content $out -Encoding utf8
$cases = @(
  @{n="p0-classic"; a="--preset 0"}, @{n="p13-bunny"; a="--preset 13"},
  @{n="p17-mirror-corridor"; a="--preset 17"}, @{n="p21-neon"; a="--preset 21"},
  @{n="glass-dragon"; a="--scene scenes/meshes/glass-dragon.json"},
  @{n="stained-glass-hall"; a="--scene scenes/builtin/stained-glass-hall.json"},
  @{n="glass-stack"; a="--scene scenes/builtin/glass-stack.json"},
  @{n="sponza"; a="--scene scenes/meshes/sponza.json"}
)
$runs = @()
foreach ($c in $cases) { foreach ($m in "--rr","--no-rr") { $runs += ,@($c.n, $c.a, $m, "") } }
# Sweeps on Classic and the mirror corridor: bounces and spp (separate fixed and per-sample costs).
foreach ($c in $cases[0], $cases[2]) {
  foreach ($x in "--set bounces=1", "--set bounces=2", "--spp 1") { $runs += ,@($c.n, $c.a, "--no-rr", $x) }
}
foreach ($r in $runs) {
  $appArgs = "--headless --frames $Frames --no-vsync --width 1440 --height 900 $($r[1]) --spp 4 $($r[2]) $($r[3])"
  $tmp = [IO.Path]::GetTempFileName()
  # Not -Wait: in PowerShell 5.1 it can hang after the process has exited.
  $p = Start-Process $Exe -ArgumentList $appArgs -RedirectStandardOutput $tmp -RedirectStandardError "$tmp.err" -NoNewWindow -PassThru
  if (-not $p.WaitForExit(180000)) { $p.Kill(); Write-Host "TIMEOUT: $appArgs"; continue }
  $line = (Get-Content $tmp | Select-String "RENDER_OK").Line
  if (-not $line) { Write-Host "FAILED: $appArgs"; Get-Content "$tmp.err" | Select-Object -Last 5; continue }
  $v = foreach ($k in "gpu_ms_mean","frame_ms_mean","fence_wait_ms_mean","trace_ms_mean","reconstruct_ms_mean","display_ms_mean","ui_ms_mean") { [regex]::Match($line, "$k=([\d.]+)").Groups[1].Value }
  "$($r[0]),$($r[2].TrimStart('-')),$($r[3] -replace ',',';'),$($v -join ',')" | Add-Content $out -Encoding utf8
}
Get-Content $out
