## ffmpeg (2026-10-30, owner allowed pulling the binary)

- `tools/setup.ps1` `Get-Ffmpeg '9.0.2' '<sha256>'` downloads gyan.dev's essentials build from its GitHub release mirror (`GyanD/codexffmpeg`). The SHA-256 is pinned (it matched gyan.dev's published checksum on 2026-10-30). Only `ffmpeg.exe` and `LICENSE` are kept, in `vendor/ffmpeg` (gitignored with `vendor/`). A `VERSION-<ver>` marker skips repeat downloads.
- build.rs copies it next to the exe when present (optional; builds work without it). `video::ffmpeg()` prefers that copy, then PATH. The snapshot script copies it too.
- Linux port notes are listed in docs/roadmap.md under "Vulkan backend".

