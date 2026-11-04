"""Vulkan groundwork: compiles every shader entry build.rs compiles (same files, entries and
defines) to SPIR-V with the DXC release build (vendor/dxc-spirv, fetched by tools/setup.ps1)
and lists what fails, first error line per entry.

    python tools/spirv_check.py [--dxc PATH] [--only NAME] [--full]

Register spaces map to descriptor sets one-to-one (-fvk-auto-shift-bindings would renumber
them); lib_6_9 SER tracers are skipped (Vulkan reorders with VK_NV_ray_tracing_invocation_reorder
in a raygen shader, a separate port).
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def entries():
    src = open(os.path.join(ROOT, 'build.rs'), encoding='utf-8').read()
    start = src.index('for (file, entry, profile, name, defines) in [')
    body = src[start:src.index('] {', start)]
    out = []
    for m in re.finditer(r'\(\s*"([\w.]+)",\s*"(\w+)",\s*"(\w+)",\s*"(\w+)",\s*&\[(.*?)\]', body, re.S):
        defines = re.findall(r'"([^"]+)"', m.group(5))
        out.append((m.group(1), m.group(2), m.group(3), m.group(4), [d for d in defines if d != '-D']))
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--dxc', default=os.path.join(ROOT, 'vendor', 'dxc-spirv', 'bin', 'x64', 'dxc.exe'))
    p.add_argument('--only')
    p.add_argument('--full', action='store_true', help='print every error line')
    a = p.parse_args()
    if not os.path.isfile(a.dxc):
        sys.exit(f'{a.dxc} missing: run tools/setup.ps1 (Get-DxcSpirv)')
    failed = 0
    tmp = tempfile.mkdtemp()
    for file, entry, profile, name, defines in entries():
        if a.only and name != a.only:
            continue
        if profile.startswith('lib_'):
            print(f'{name:28} skip (SER library)')
            continue
        cmd = [a.dxc, '-spirv', '-fspv-target-env=vulkan1.3', '-T', profile, '-E', entry,
               '-HV', '2021', '-O3', '-I', os.path.join(ROOT, 'shaders')]  # DXC enables the SPIR-V extensions it needs by default
        for d in defines:
            cmd += ['-D', d]
        cmd += ['-Fo', os.path.join(tmp, name + '.spv'), os.path.join(ROOT, 'shaders', file)]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode == 0:
            size = os.path.getsize(os.path.join(tmp, name + '.spv'))
            print(f'{name:28} ok ({size // 1024} KB)')
            continue
        failed += 1
        errors = [l for l in (r.stderr + r.stdout).splitlines() if 'error' in l]
        print(f'{name:28} FAIL  {len(errors)} errors')
        for l in errors if a.full else errors[:1]:
            print('    ' + l.replace(ROOT, '').strip()[:220])
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
