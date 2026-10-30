# Downloads the test models used by scenes/meshes into scenes/models/ (gitignored). Every file is
# pinned to a commit and a SHA-256, so a changed upstream file fails instead of slipping in.
#   powershell -File tools/fetch_models.ps1
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = Join-Path $projectRoot 'scenes/models'
New-Item -ItemType Directory -Force $target | Out-Null
# Alec Jacobson's common-3d-test-models (scans from Stanford, Georgia Tech and others; see its README).
$obj = 'https://raw.githubusercontent.com/alecjacobson/common-3d-test-models/8a4f8642acaf43f9cd7b67858a1502e1055ef202/data'
# Khronos glTF-Sample-Assets (per-model licences in each model's README).
$gltf = 'https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models'
$models = @(
    @("$obj/xyzrgb_dragon.obj", '5d7106b58f2ac0d017758480fe607c5b6bf7d6201d930d2dd1ba7022c4d281c0'),
    @("$obj/happy.obj", '8ce5a4a7161efd97dabc8eeb281824fac1b17c9ba2cb0c2c6f350cd05982058c'),
    @("$obj/armadillo.obj", 'd2fd040334c12b16804fbcb9962391232cc24708244617f931baaa871210563e'),
    @("$obj/lucy.obj", 'ea54611a3466bc903a107fd825409344562b8c4a8f191bbdc88ce380bebd0911'),
    @("$obj/spot.obj", '0738b5e8608fed74e5e8c7aa8dd0af97b4b74f9f6cbf7aac84cd7e40b2e44a75'),
    @("$obj/suzanne.obj", 'd8684326f9bd8cfc24d3d302c1042fa16f63d2e66e49ed56b413fa20bed271e6'),
    @("$gltf/DragonAttenuation/glTF-Binary/DragonAttenuation.glb", 'ec97df715d4207c0e39b60c329db43032872794df60e1279b7652314cf0a999e'),
    @("$gltf/Duck/glTF-Binary/Duck.glb", '65bf938f54d6073e619e76e007820bbf980cdc3dc0daec0d94830ffc4ae54ab5'),
    @("$gltf/Lantern/glTF-Binary/Lantern.glb", 'a79458c4b02d695187a952f23a63b8bf278e7bc3d316a3c2a314f2d6974181f1'),
    @("$gltf/ABeautifulGame/glTF-Binary/ABeautifulGame.glb", 'bd7133b4b322aae97c589b8839dae8155ad2546acb35ae32a127e722a959d007'),
    @("$gltf/ToyCar/glTF-Binary/ToyCar.glb", '01a60862de55cd4b9f3acfab0b0def86451800f9c42467fcd61052c16cb9838c')
)
foreach ($m in $models) {
    $url, $sha256 = $m
    $file = Join-Path $target ([IO.Path]::GetFileName($url))
    if ((Test-Path -LiteralPath $file) -and (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant() -eq $sha256) { continue }
    $partial = "$file.partial"
    Invoke-WebRequest -Uri $url -OutFile $partial
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $partial).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $partial; throw "$url checksum mismatch: $actual" }
    Move-Item -Force -LiteralPath $partial -Destination $file
    Write-Output "FETCHED $([IO.Path]::GetFileName($url))"
}
Write-Output "MODELS $target"
