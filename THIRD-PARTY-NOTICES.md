# Third-party notices

Caustica includes or downloads the following. Each stays under its own terms; nothing here changes them. Full texts are in `licenses/` beside this file in release packages.

## Upscaler and runtime components

- **NVIDIA DLSS SDK** (`nvngx_dlss*`, `libnvidia-ngx-dlss*`): Copyright NVIDIA Corporation, under the NVIDIA RTX SDKs license (`licenses/NVIDIA-DLSS.txt`). You may not reverse engineer, decompile or disassemble these files, or redistribute them on their own. Caustica is not sponsored or endorsed by NVIDIA.
- **AMD FSR SDK / FidelityFX** (`amd_fidelityfx_*`, Windows): Copyright Advanced Micro Devices, Inc. (`licenses/AMD-FSR.md`).
- **Intel XeSS SDK** (`libxess.dll`, Windows): Copyright Intel Corporation, under the Intel Simplified Software License (`licenses/Intel-XeSS.txt`).
- **Microsoft D3D12 Agility SDK** (`D3D12\`, Windows): Copyright Microsoft Corporation (`licenses/Microsoft-Agility.txt`).
- **FFmpeg** (`ffmpeg.exe`, Windows): a separate program the app runs for video export, from the gyan.dev "essentials" build (`licenses/FFmpeg.txt`). Source: https://ffmpeg.org and https://github.com/GyanD/codexffmpeg

## Models

- **Utah teapot:** Martin Newell's patches, by way of three.js (MIT, `assets/THREE-LICENSE.txt`).
- **Stanford bunny, Happy Buddha, Armadillo, Lucy, XYZ RGB dragon, and the dragon in DragonAttenuation:** Stanford University Computer Graphics Laboratory (https://graphics.stanford.edu/data/3Dscanrep/). Free to use for research and to redistribute for free, with credit; not for commercial use without Stanford's permission. Taken from Alec Jacobson's common-3d-test-models, and for DragonAttenuation through Morgan McGuire's Computer Graphics Archive and Khronos glTF-Sample-Assets.
- **Spot:** Keenan Crane (CC0 1.0). **Suzanne:** the Blender Foundation's test model, through common-3d-test-models.
- **ABeautifulGame (chess set):** ASWF / MaterialX Project, glTF conversion by Ed Mackey (CC BY 4.0, https://creativecommons.org/licenses/by/4.0/).
- **Duck:** Sony (SCEA Shared Source License 1.0), through Khronos glTF-Sample-Assets.
- **Lantern, ToyCar:** CC0 1.0, through Khronos glTF-Sample-Assets.
- **Sponza:** Crytek, under the CRYENGINE Limited License Agreement (https://www.cryengine.com/ce-terms), through Khronos glTF-Sample-Assets.

## Environments and textures

Poly Haven (https://polyhaven.com) and ambientCG (https://ambientcg.com): CC0 1.0. Scene files name their sources in `credits`.

## Rust crates

Built with crates under MIT, Apache-2.0, BSD, Zlib, Unicode-3.0 and MPL-2.0 licences. `cargo metadata` lists each crate and its licence; `Cargo.lock` pins the versions.
