# Linux packages and release CI (2026-11-15)

`.github/workflows/ci.yml` has three jobs. `windows` (windows-latest) and `linux` (ubuntu-22.04, glibc 2.35, so the AppImage and tar.gz run on newer distros) run `cargo fmt --check`, `clippy -D warnings` and `cargo test --locked` on every push to main and every PR; the tests are CPU-only, the runners have no GPU. On a `v*` tag, or a manual run, they also package; `release` (tags only) downloads both artifacts, writes one `SHA256SUMS` (sha256sum format, which `src/update.rs` parses) and creates the release with the commits since the previous tag as notes. A tag that differs from `Cargo.toml`'s version fails both jobs early. A manual run keeps the packages as workflow artifacts and releases nothing.

Assets (the names `src/update.rs` matches, `caustica-v*-setup.exe` and `Caustica-*-x86_64.AppImage`, are unchanged):

- `caustica-vX.Y.Z-setup.exe`, `caustica-vX.Y.Z.zip`
- `Caustica-X.Y.Z-x86_64.AppImage`, `caustica-X.Y.Z-x86_64.tar.gz`
- `caustica_X.Y.Z_amd64.deb`, `caustica-X.Y.Z-1-x86_64.pkg.tar.zst`, `caustica-X.Y.Z-1.x86_64.rpm`
- `SHA256SUMS`

Flow on Linux: `tools/appimage.sh` (with `STAGE_OUT=dist/stage CAUSTICA_NO_THUMBNAILS=1`) builds the release exe and the AppImage and leaves its `usr/bin` tree (exe, NGX and OIDN libraries, scenes with the fetched assets, licences). `tools/package_linux.sh <stage> <version> <out>` makes the tar.gz (one folder plus `RUN.txt`) and runs nfpm over `packaging/nfpm.yaml` for the three native packages. The packages install the tree to `/opt/caustica` (the app treats it as an installed, read-only copy: settings and user scenes go to the XDG folders), a `/usr/bin/caustica` symlink, a `.desktop` file and a 256 px icon under `/usr/share`. Self-update only replaces an AppImage; packages update through the package manager or by installing the next one.

Dependencies (ldd of a release build plus the dlopen'd libraries: Vulkan loader via ash, X11/Wayland/xkbcommon via winit, libudev via gilrs, OpenSSL 3 via native-tls): the Vulkan loader, libstdc++, OpenSSL 3, libudev, libxkbcommon, libwayland-client, libX11, under each distro's names; ffmpeg is recommended (video capture finds it on PATH). The GPU driver supplies the Vulkan ICD. Re-check with `ldd` when a dependency is added.

nfpm is pinned in the workflow env (`NFPM_VERSION`, `NFPM_SHA256`, the Linux x86_64 tarball). To bump, take the hash from the release's `checksums.txt`. nfpm does not expand env vars in content paths, so the script puts the stage path into a copy of the config. No thumbnails are prebuilt on Linux CI (no GPU): the first open renders them.

Local dry run: `NFPM=/path/to/nfpm sh tools/package_linux.sh <stage dir> 0.13.0 /tmp/out`, then list with `bsdtar -tf`.
