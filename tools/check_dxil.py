#!/usr/bin/env python3
"""Compile every D3D12 shader entry build.rs compiles on Windows with the Linux DXC, validate with
dxv, print a pass/fail table. The entry list and the DXC flags are parsed from build.rs (the
`entries` array and `dxil_command`), so there is no second copy to drift.

  tools/check_dxil.py [--jobs N] [--out DIR] [--only NAME ...]

Run it through target/safe.sh in this repo. Exit 0 only if every entry compiles and validates.
Not covered: CAUSTICA_SER_BITS / CAUSTICA_SER_AT / CAUSTICA_SHADER_DEBUG variants, and the
Windows SDK's own (older) dxc: this is the 1.9 release from vendor/dxc-linux.
"""
import argparse, os, re, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DXC_DIR = os.path.join(ROOT, "vendor", "dxc-linux")


def strip_comments(s):
    return re.sub(r"//[^\n]*", "", s)


def parse_entries(src):
    m = re.search(r"let entries: &\[\(.*?\)\] = &\[", src, re.S)
    if not m:
        sys.exit("check_dxil: cannot find the entries array in build.rs")
    body = strip_comments(src[m.end():])
    entries, depth, start = [], 0, None
    for i, ch in enumerate(body):
        if ch == "(":
            if depth == 0:
                start = i
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                strs = re.findall(r'"([^"]*)"', body[start:i])
                if len(strs) < 4:
                    sys.exit("check_dxil: bad entry tuple: " + body[start:i])
                entries.append(tuple(strs[:4]) + (strs[4:],))
        elif ch == "]" and depth == 0:
            break
    return entries


def tokens(arr):
    out = []
    for t in arr.split(","):
        t = t.strip()
        if not t:
            continue
        out.append(t[1:-1] if t.startswith('"') else t)  # bare identifiers: entry / profile
    return out


def parse_flags(src):
    fn = src[src.index("fn dxil_command"):]
    fn = strip_comments(fn[: fn.index("\n}\n")])
    m = re.search(r"let entry_args = if [^{]*\{\s*\[([^\]]*)\]\s*\}\s*else\s*\{\s*\[([^\]]*)\]\s*\}", fn)
    common = re.search(r'\.args\(\["(-HV)"([^\]]*)\]\)', fn)
    if not (m and common):
        sys.exit("check_dxil: cannot parse dxil_command in build.rs")
    return tokens(m.group(1)), tokens(m.group(2)), tokens('"-HV"' + common.group(2))


def command(e, flags, outdir):
    file, entry, profile, name, defines = e
    lib, comp, common = flags
    ea = [entry if t == "entry" else t for t in (lib if profile.startswith("lib_") else comp)]
    return (["-T", profile] + ea + common + defines
            + ["-Fo", os.path.join(outdir, name + ".dxil"), "shaders/" + file])


def run(e, flags, outdir):
    env = dict(os.environ, LD_LIBRARY_PATH=os.path.join(DXC_DIR, "lib"))
    name = e[3]
    out = os.path.join(outdir, name + ".dxil")
    if os.path.exists(out):
        os.remove(out)
    t0 = time.time()
    p = subprocess.run([os.path.join(DXC_DIR, "bin", "dxc")] + command(e, flags, outdir),
                       cwd=ROOT, env=env, capture_output=True, text=True)
    dt = time.time() - t0
    if p.returncode != 0 or not os.path.exists(out):
        err = [l for l in (p.stderr + p.stdout).splitlines() if "error" in l] or (p.stderr + p.stdout).splitlines()
        return name, e[2], "FAIL compile", dt, 0, "\n".join(err[:6])
    v = subprocess.run([os.path.join(DXC_DIR, "bin", "dxv"), out], cwd=ROOT, env=env,
                       capture_output=True, text=True)
    ok = v.returncode == 0 and "Validation succeeded" in (v.stdout + v.stderr)
    return (name, e[2], "ok" if ok else "FAIL dxv", dt, os.path.getsize(out),
            "" if ok else (v.stdout + v.stderr).strip()[:600])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--out", default="/home/kaan/caustica-tmp/d3d12check/dxil")
    ap.add_argument("--only", nargs="*", default=[])
    a = ap.parse_args()
    for t in ("dxc", "dxv"):
        if not os.path.exists(os.path.join(DXC_DIR, "bin", t)):
            sys.exit(f"check_dxil: vendor/dxc-linux/bin/{t} missing (sh tools/setup.sh)")
    src = open(os.path.join(ROOT, "build.rs")).read()
    entries, flags = parse_entries(src), parse_flags(src)
    if a.only:
        entries = [e for e in entries if e[3] in a.only]
    os.makedirs(a.out, exist_ok=True)
    t0 = time.time()
    with ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda e: run(e, flags, a.out), entries))
    print(f"{'entry':26} {'profile':8} {'result':13} {'time':>6} {'size':>8}")
    for name, prof, res, dt, size, _ in rows:
        print(f"{name:26} {prof:8} {res:13} {dt:5.1f}s {size/1024:7.0f}K")
    for name, _, res, _, _, msg in rows:
        if msg:
            print(f"\n--- {name} ({res}) ---\n{msg}")
    bad = sum(r[2] != "ok" for r in rows)
    print(f"\n{len(rows) - bad}/{len(rows)} ok, wall {time.time() - t0:.1f}s, {a.jobs} jobs")
    sys.exit(1 if bad else 0)


main()
