# Third-party notices

Caustica's own source code is under the MIT license (`LICENSE`, or `licenses/Caustica-MIT.txt` in release packages). That covers the code only, not the components, models, textures and environments below. Each of those stays under its own terms; nothing here changes them. Full texts are in `licenses/` beside this file in release packages.

## End-user terms

By installing or using Caustica you agree that the third-party components it includes are used under their own terms, including these: you may not reverse engineer, decompile or disassemble them, remove their proprietary notices, or redistribute them on their own. The NVIDIA, Microsoft, AMD and Intel components are provided by their owners under the licences in `licenses/`, and those owners are not responsible for Caustica.

## NVIDIA DLSS

This application uses the NVIDIA DLSS SDK. NVIDIA RTX(TM). NVIDIA, NVIDIA RTX and DLSS are trademarks of NVIDIA Corporation. DLSS needs an NVIDIA GPU.

## Logos

The logos in the About box (`assets/nvidia-logo.png`, `assets/amd-fsr-logo.png`, `assets/intel-logo.jpg`) belong to NVIDIA Corporation, Advanced Micro Devices, Inc. and Intel Corporation. The AMD and Intel logos show on Windows only, where those components ship. They are not covered by Caustica's MIT license and don't imply that any of them endorses Caustica.

## Upscaler and runtime components

- **NVIDIA DLSS SDK** (`nvngx_dlss*`, `libnvidia-ngx-dlss*`): Copyright NVIDIA Corporation, under the NVIDIA RTX SDKs license (`licenses/NVIDIA-DLSS.txt`). You may not reverse engineer, decompile or disassemble these files, or redistribute them on their own. Caustica is not sponsored or endorsed by NVIDIA.
- **AMD FSR SDK / FidelityFX** (`amd_fidelityfx_*`, Windows): Copyright Advanced Micro Devices, Inc. (`licenses/AMD-FSR.md`).
- **Intel XeSS SDK** (`libxess.dll`, Windows): Copyright Intel Corporation, under the Intel Simplified Software License (`licenses/Intel-XeSS.txt`).
- **Intel Open Image Denoise 2.5.1** (`OpenImageDenoise*.dll` and `tbb12.dll` on Windows, `libOpenImageDenoise*.so` and `libtbb.so*` on Linux; CPU device only): Copyright Intel Corporation, under the Apache License 2.0 (`licenses/OIDN-Apache-2.0.txt`). Source: https://github.com/RenderKit/oidn. Its third-party notices are in `licenses/OIDN-third-party-programs.txt`.
- **Intel oneAPI Threading Building Blocks (oneTBB)** (`tbb12.dll`, `libtbb.so*`, shipped with Open Image Denoise): Copyright Intel Corporation, under the Apache License 2.0 (`licenses/OIDN-Apache-2.0.txt`); notices in `licenses/OIDN-oneTBB-third-party-programs.txt`.
- **Microsoft D3D12 Agility SDK** (`D3D12\`, Windows): Copyright Microsoft Corporation (`licenses/Microsoft-Agility.txt`).
- **FFmpeg** (`ffmpeg.exe`, Windows): a separate program the app runs for video export, from the gyan.dev "essentials" build, licensed under the GNU GPL version 3 (`licenses/FFmpeg.txt`). Source: https://ffmpeg.org and https://github.com/GyanD/codexffmpeg

## Models

- **Utah teapot:** Martin Newell's patches, by way of three.js (MIT, `licenses/three.js-MIT.txt`).
- **Stanford bunny, Happy Buddha, Armadillo, Lucy, XYZ RGB dragon, and the dragon in DragonAttenuation:** Stanford University Computer Graphics Laboratory (https://graphics.stanford.edu/data/3Dscanrep/). Free to use for research and to redistribute for free, with credit; not for commercial use without Stanford's permission. Credits are in `licenses/Mesh-credits.md`. Taken from Alec Jacobson's common-3d-test-models, and for DragonAttenuation through Morgan McGuire's Computer Graphics Archive and Khronos glTF-Sample-Assets.
- **Spot:** Keenan Crane (CC0 1.0). **Suzanne:** the Blender Foundation's test model, through common-3d-test-models.
- **ABeautifulGame (chess set):** ASWF / MaterialX Project, glTF conversion by Ed Mackey (CC BY 4.0, https://creativecommons.org/licenses/by/4.0/).
- **Duck:** Sony (SCEA Shared Source License 1.0), through Khronos glTF-Sample-Assets.
- **Lantern, ToyCar:** CC0 1.0, through Khronos glTF-Sample-Assets.
- **Sponza:** Crytek, under the CRYENGINE Limited License Agreement (https://www.cryengine.com/ce-terms), through Khronos glTF-Sample-Assets.

## Environments and textures

Poly Haven (https://polyhaven.com) and ambientCG (https://ambientcg.com): CC0 1.0. Scene files name their sources in `credits`.

## Night sky data

- **Milky Way map** (`scenes/sky/milkyway.exr`): derived from the Deep Star Maps 2020 by NASA/Goddard Space Flight Center Scientific Visualization Studio (https://svs.gsfc.nasa.gov/4851), based on data from ESA/Gaia/DPAC, Hipparcos-2 and Tycho-2. Downsampled to 2048 x 1024. NASA media are not copyrighted and are free to use with credit; NASA does not endorse Caustica.
- **Star table** (`scenes/sky/stars.bin`): derived from the HYG database v4.2 by David Nash (https://astronexus.com/hyg), under CC BY-SA 4.0 (https://creativecommons.org/licenses/by-sa/4.0/). The stars with V magnitude 9 or brighter, in a binary layout. `scenes/sky/stars.bin` is under CC BY-SA 4.0 as well; `tools/prep_sky_data.py` regenerates it.

## Rust crates

Built with crates under MIT, Apache-2.0, BSD, Zlib, Unicode-3.0 and MPL-2.0 licences. `cargo metadata` lists each crate and its licence; `Cargo.lock` pins the versions.
