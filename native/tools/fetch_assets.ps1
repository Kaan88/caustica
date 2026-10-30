# Downloads the test models (scenes/models/) and HDRIs (scenes/hdri/) used by the meshes and
# environments packs; both folders are gitignored. Every file is pinned to a SHA-256 (and the
# models to a commit), so a changed upstream file fails instead of slipping in.
#   powershell -File tools/fetch_assets.ps1
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
# Alec Jacobson's common-3d-test-models (scans from Stanford, Georgia Tech and others; see its README).
$obj = 'https://raw.githubusercontent.com/alecjacobson/common-3d-test-models/8a4f8642acaf43f9cd7b67858a1502e1055ef202/data'
# Khronos glTF-Sample-Assets (per-model licences in each model's README).
$gltf = 'https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models'
# Poly Haven HDRIs, 2k (CC0).
$hdri = 'https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/2k'
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
$hdris = @(
    @("$hdri/studio_small_09_2k.hdr", '36724313c0fc66dab126ff90f081b85a0b1ef47be65eda1417bd20b033f0573f'),
    @("$hdri/brown_photostudio_02_2k.hdr", '153ca0cb899575c00f5b23065958d250a0f73e8789c8cf5957ccea313baf2bb6'),
    @("$hdri/kloofendal_48d_partly_cloudy_puresky_2k.hdr", '5244534e9cf5b606f2ff513aa00ddb161b0a4826ffd88a0d3bd03ac29247d198'),
    @("$hdri/venice_sunset_2k.hdr", 'cbfac020ee17ab36016ccc7fbe9020b828587b9864af6ecda0a93089914889a6'),
    @("$hdri/rural_asphalt_road_2k.hdr", 'e135ecfe1852e716b202ddb23678b1063aab41e350588eb21b2a102b2ac74d63')
)
$all = @($models | ForEach-Object { , @($_[0], $_[1], 'models') }) + @($hdris | ForEach-Object { , @($_[0], $_[1], 'hdri') })
foreach ($m in $all) {
    $url, $sha256, $folder = $m
    $target = Join-Path $projectRoot "scenes/$folder"
    New-Item -ItemType Directory -Force $target | Out-Null
    $file = Join-Path $target ([IO.Path]::GetFileName($url))
    if ((Test-Path -LiteralPath $file) -and (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant() -eq $sha256) { continue }
    $partial = "$file.partial"
    Invoke-WebRequest -Uri $url -OutFile $partial
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $partial).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $partial; throw "$url checksum mismatch: $actual" }
    Move-Item -Force -LiteralPath $partial -Destination $file
    Write-Output "FETCHED $([IO.Path]::GetFileName($url))"
}
Write-Output "ASSETS $(Join-Path $projectRoot scenes)"
