# Downloads the test models (scenes/models/), HDRIs (scenes/hdri/) and texture sets
# (scenes/textures/) used by the meshes, environments and materials packs; all three folders
# are gitignored. Every file is pinned to a SHA-256 (and the
# models to a commit), so a changed upstream file fails instead of slipping in.
#   powershell -File tools/fetch_assets.ps1
# CI runs it before packaging a release, so the installer ships these files (.github/workflows/ci.yml).
$ErrorActionPreference = 'Stop'
# Windows PowerShell's progress bar slows Invoke-WebRequest by an order of magnitude.
$ProgressPreference = 'SilentlyContinue'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
# Alec Jacobson's common-3d-test-models (scans from Stanford, Georgia Tech and others; see its README).
$obj = 'https://raw.githubusercontent.com/alecjacobson/common-3d-test-models/8a4f8642acaf43f9cd7b67858a1502e1055ef202/data'
# Khronos glTF-Sample-Assets (per-model licences in each model's README).
$gltf = 'https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models'
# Poly Haven HDRIs (CC0), 4k, and 8k for the skies seen large. Checked by eye as backgrounds at
# 1440x900; satara_night (sensor noise) and moonless_golf (motion blur) were dropped.
$hdri = 'https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr'
# Poly Haven texture sets (CC0), 2k: colour, OpenGL normals, and AO / roughness / metalness packed.
$phtex = 'https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k'
# ambientCG materials (CC0), 2k JPG zips; only the colour, NormalGL, roughness and metalness
# maps are extracted, into scenes/textures/<id>/.
$ambientcg = @(
    @('Bricks097', '0c4ea4f9ba8373ba9a6f12b0f92a2b1859686e8c670f44616b38be095f65b71f'),
    @('Fabric062', 'af2f1e0db747b69a2c9a3916b6d4b0187c89c7e557fc7f056ab4714a2ae140e5'),
    @('Marble016', 'da551007da2ad2ccd4f18b5b28b93b52e853651b826347729f313c42b06357c3'),
    @('Metal032', '4b8884843c490963d5734be036c35639d064e7a817eb61145322f69c6c19895a'),
    @('Tiles074', '8c9439bd30113c9ab4e223cf4e1fb08265d0a34978d02b5b61d7754c68cfc74f'),
    @('WoodFloor041', 'f80d5875cfd72a2607d1fc2b8a8dc1272f22d4ef1d1111758b2a0727cc58cdae')
)
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
    @("$hdri/4k/brown_photostudio_02_4k.hdr", '368251c254a47dc7aaf532a515c0bd9ad3708ce5a3c5bb520b1628bcb9a37bc0'),
    @("$hdri/4k/canary_wharf_4k.hdr", 'e9ef3c82901dcf50294797c046a05b184be0c0f3b2a4e5c5235ccb01adba5ef5'),
    @("$hdri/4k/cayley_interior_4k.hdr", 'e85c2f3095aeb0e98817c20c8dda052f67e9f48dde7b962ded94cdb83a0539bb'),
    @("$hdri/4k/fireplace_4k.hdr", '93d90f9dd39c8c1321b2d1f393b82feb777eace32b96ad7a600eb2fe98faf163'),
    @("$hdri/8k/kloofendal_48d_partly_cloudy_puresky_8k.hdr", '30ff6bb1eec58c11b52254e89b5dc39aaf756dd332c9a14a4976d68f23feaa71'),
    @("$hdri/4k/lebombo_4k.hdr", '23a59b56482e804985ae8eae404a0d15a24ec924ad2f250e6f81371760ae2a48'),
    @("$hdri/4k/lilienstein_4k.hdr", '5ff50a6e4e0a1cab05cbc349c18f4c73a2d25f09c676ae00cabf213945f29f49'),
    @("$hdri/4k/meadow_2_4k.hdr", '69d2ff4b090f0da4e4cd4b5a83ff7e898f085db8b18e1cbe9c624e5760fad964'),
    @("$hdri/4k/noon_grass_4k.hdr", '7b8be38b8de57d7803c78e0dd2eff460c0a1d2e0145bdd809add8fe93a69c249'),
    @("$hdri/4k/overcast_soil_puresky_4k.hdr", '717958e9dfd092fcead82ff4691f3d615088803dd1888bb638ba948669021140'),
    @("$hdri/4k/photo_studio_loft_hall_4k.hdr", '6b1e43955efa60a24e8021994d3ab6e630ba142fbd02d1530a0cc0ab53ec41a2'),
    @("$hdri/4k/potsdamer_platz_4k.hdr", 'ab4d2ddaf06cae4ee2809236732befb052eb94523e946a777f821bbb76b498dc'),
    @("$hdri/4k/rural_asphalt_road_4k.hdr", '325e8626aefa9cfdf66026bdfd669209d44cba1a32536e172db215e0e21a22d7'),
    @("$hdri/4k/shanghai_bund_4k.hdr", '2e72825fd2502ae649f59035ecc9887c7d217dfe1a32e7915146d587fa2816e3'),
    @("$hdri/4k/snowy_park_01_4k.hdr", 'fd481fc511dac7d5ae1762491886f68ced1ad40409add5a62082aedb0d8613bb'),
    @("$hdri/4k/spruit_sunrise_4k.hdr", 'b5654f184ffa7f6e4f08d6ac5c59b46556f384dfdc2c9e8f3b982c8f4aff5b46'),
    @("$hdri/4k/studio_small_09_4k.hdr", '71a876b4315e691cc095e7340228dc9bc48417c26a681b4fc30938161fc85e39'),
    @("$hdri/8k/the_sky_is_on_fire_8k.hdr", '1aa2fb96a87be9578e4b328675bb9baca8b3e5de9ee38544c2189909c09fb303'),
    @("$hdri/8k/venice_sunset_8k.hdr", 'aeb30638c4793cab176b615e9ca09ae11f96b112d4a89cc3b3e6f23b805c7362')
)
$textures = @(
    @("$phtex/laminate_floor_02/laminate_floor_02_diff_2k.jpg", '2a1e6519309b369f749f06aaf241ac67ea762c686b6a848c1a79331b18a30521'),
    @("$phtex/laminate_floor_02/laminate_floor_02_nor_gl_2k.jpg", '49aa58c34c6770d209e63f9bdb4d16326f5dab24474cfeb18433e63fc54456de'),
    @("$phtex/laminate_floor_02/laminate_floor_02_arm_2k.jpg", 'd646acf8dede6ee1dc0c8a6bc3436f1742cfa65468bf8ad8985aea9edbb91ecb'),
    @("$phtex/rock_wall_08/rock_wall_08_diff_2k.jpg", 'd01a1cbadd402ca039a76d5905b0334ed5fb3e471b0547fe7ba2bb185c7c5f9f'),
    @("$phtex/rock_wall_08/rock_wall_08_nor_gl_2k.jpg", '0bd5dd743149ecccdf5ff79abdcc72426d4b4eb97f431fde5171f8e6d82d6259'),
    @("$phtex/rock_wall_08/rock_wall_08_arm_2k.jpg", '91489a16465996beefdeb868d66a56e9f780458f60950a659bd2376ac35d0edb')
)
# Crytek Sponza (Khronos glTF copy, same commit): a .gltf, its .bin and 69 textures, pinned in
# tools/sponza.sha256 ("file sha256" per line). Track 20a's heavy benchmark scene.
$sponza = Get-Content (Join-Path $PSScriptRoot 'sponza.sha256') | Where-Object { $_ } | ForEach-Object {
    $name, $sha256 = $_ -split ' '
    , @("$gltf/Sponza/glTF/$name", $sha256, 'models/Sponza')
}
$all = @($models | ForEach-Object { , @($_[0], $_[1], 'models') }) + @($hdris | ForEach-Object { , @($_[0], $_[1], 'hdri') }) +
    @($textures | ForEach-Object { , @($_[0], $_[1], "textures/$(Split-Path -Leaf (Split-Path -Parent $_[0]))") }) + @($sponza)
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
foreach ($m in $ambientcg) {
    $id, $sha256 = $m
    $target = Join-Path $projectRoot "scenes/textures/$id"
    if (Test-Path -LiteralPath (Join-Path $target "$($id)_2K-JPG_Color.jpg")) { continue }
    $zip = Join-Path ([IO.Path]::GetTempPath()) "$id-cornell.zip"
    Invoke-WebRequest -Uri "https://ambientcg.com/get?file=$($id)_2K-JPG.zip" -OutFile $zip -UserAgent 'caustica'
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { Remove-Item -LiteralPath $zip; throw "$id checksum mismatch: $actual" }
    New-Item -ItemType Directory -Force $target | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($entry in $archive.Entries) {
            if ($entry.Name -match '_(Color|NormalGL|Roughness|Metalness)\.jpg$') {
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $target $entry.Name), $true)
            }
        }
    } finally { $archive.Dispose() }
    Remove-Item -LiteralPath $zip
    Write-Output "FETCHED $id"
}
Write-Output "ASSETS $(Join-Path $projectRoot scenes)"
