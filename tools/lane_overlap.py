#!/usr/bin/env python3
"""Where do parallel lane worktrees collide? Read-only (git diff / ls-files / file reads).

    python3 tools/lane_overlap.py [LANE ...]

A LANE is a worktree path, `name=path`, or `name=path@base` (a base other than HEAD).
With no arguments it checks the six lanes (exposure, accuracy, headless, bounces, egui,
oidn). exposure and accuracy branched at a752d0f and carry the 80b14a1 fixes (chrome.rs,
vulkan/layout.rs, vulkan/gpu.rs, dirs.rs) uncommitted, so they are diffed against 80b14a1
too: that is what keeps those four files from showing up as lane edits.

Each lane's edits are `git diff <base>` (working tree, staged or not) plus untracked files,
minus the symlinks vendor, scenes/hdri|models|textures, scenes/sky/raw and target.
Hunk ranges are base-side line numbers (all lanes share a base, so they compare directly).
Overlap means the line ranges intersect, adjacent means they touch (git conflicts on both);
hunks with the same patch text in two lanes are "identical, no conflict".
Python 3 standard library only.
"""
import itertools
import os
import re
import subprocess
import sys

HOME = os.path.expanduser("~")
DEFAULT_LANES = [
    ("exposure", f"{HOME}/caustica-exposure", "80b14a1"),
    ("accuracy", f"{HOME}/caustica-accuracy", "80b14a1"),
    ("headless", f"{HOME}/caustica-headless", "HEAD"),
    ("bounces", f"{HOME}/caustica-bounces", "HEAD"),
    ("egui", f"{HOME}/caustica-egui", "HEAD"),
    ("oidn", f"{HOME}/caustica-oidn", "HEAD"),
]
IGNORE_PREFIXES = ("vendor", "scenes/hdri", "scenes/models", "scenes/textures", "scenes/sky/raw", "target")
HUNK_RE = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def git(wt, *args):
    r = subprocess.run(["git", "-C", wt, *args], capture_output=True, text=True, errors="replace")
    return r.stdout


def ignored(path):
    return any(path == p or path.startswith(p + "/") for p in IGNORE_PREFIXES)


def read_text(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return None


class Hunk:
    def __init__(self, os_, ol, ns, nl):
        self.os, self.ol, self.ns, self.nl = os_, ol, ns, nl
        self.minus, self.plus = [], []  # plus: (new line number, text)

    @property
    def span(self):
        """Base-side interval; a pure insertion sits between two lines (a + 0.5)."""
        if self.ol == 0:
            return (self.os + 0.5, self.os + 0.5)
        return (self.os, self.os + self.ol - 1)

    def key(self):
        return (tuple(self.minus), tuple(t for _, t in self.plus))

    def label(self):
        lo, hi = self.span
        if self.ol == 0:
            return f"+{self.os}"
        return f"{int(lo)}" if lo == hi else f"{int(lo)}-{int(hi)}"


def parse_diff(text):
    hunks, cur, nl_no = [], None, 0
    for ln in text.splitlines():
        m = HUNK_RE.match(ln)
        if m:
            cur = Hunk(int(m[1]), 1 if m[2] is None else int(m[2]), int(m[3]), 1 if m[4] is None else int(m[4]))
            nl_no = cur.ns
            hunks.append(cur)
        elif cur is not None:
            if ln.startswith("+") and not ln.startswith("+++"):
                cur.plus.append((nl_no, ln[1:]))
                nl_no += 1
            elif ln.startswith("-") and not ln.startswith("---"):
                cur.minus.append(ln[1:])
    return hunks


class Lane:
    def __init__(self, name, wt, base):
        self.name, self.wt, self.base = name, wt, base
        self.files = {}  # path -> {"hunks": [...], "new": bool, "text": str|None}
        self.load()

    def load(self):
        tracked = git(self.wt, "diff", "--name-only", self.base).split("\n")
        untracked = git(self.wt, "ls-files", "--others", "--exclude-standard").split("\n")
        for p in sorted(set(x for x in tracked + untracked if x)):
            full = os.path.join(self.wt, p)
            if ignored(p) or os.path.islink(full) or os.path.isdir(full):
                continue
            is_new = p in untracked
            text = read_text(full) if os.path.exists(full) else None
            if is_new:
                if text is None:
                    continue
                lines = text.splitlines()
                h = Hunk(0, 0, 1, len(lines))
                h.plus = [(i + 1, t) for i, t in enumerate(lines)]
                hunks = [h]
            else:
                hunks = parse_diff(git(self.wt, "diff", "-U0", "--no-color", self.base, "--", p))
                if not hunks:
                    # binary or mode-only change
                    hunks = [Hunk(0, 0, 0, 0)]
            self.files[p] = {"hunks": hunks, "new": is_new, "text": text, "deleted": text is None}


def relation(a, b):
    (a0, a1), (b0, b1) = a.span, b.span
    if a.key() == b.key() and (a.os, a.ol) == (b.os, b.ol):
        return "identical"
    if a0 <= b1 and b0 <= a1:
        return "overlap"
    if a0 <= b1 + 1 and b0 <= a1 + 1:
        return "adjacent"
    return "disjoint"


def pair_status(fa, fb):
    """Worst relation between two lanes' edits of one file."""
    if fa["new"] and fb["new"]:
        return "identical" if fa["text"] == fb["text"] else "overlap"
    if fa["text"] == fb["text"]:
        return "identical"
    if fa["new"] or fb["new"]:
        return "overlap"
    rank = {"disjoint": 0, "adjacent": 1, "identical": 0, "overlap": 2}
    worst, saw_ident, hits = "disjoint", False, []
    for ha in fa["hunks"]:
        for hb in fb["hunks"]:
            r = relation(ha, hb)
            if r == "identical":
                saw_ident = True
            elif rank[r] > rank[worst]:
                worst = r
            if r in ("overlap", "adjacent"):
                hits.append((ha.label(), hb.label(), r))
    if worst == "disjoint" and saw_ident:
        return "identical"
    return worst


# ---------------------------------------------------------------- special collisions

def block_range(lines, start_re):
    """1-based (first, last) line numbers of the brace block opened by the first line matching."""
    for i, ln in enumerate(lines):
        if re.search(start_re, ln):
            depth, seen = 0, False
            for j in range(i, len(lines)):
                depth += lines[j].count("{") - lines[j].count("}")
                seen = seen or "{" in lines[j]
                if seen and depth <= 0:
                    return i + 1, j + 1
            return i + 1, len(lines)
    return None


RUST_FIELD = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*[^:]")
HLSL_FIELD = re.compile(r"^\s*(?:float|uint|int|bool|float2|float3|float4|uint2|uint3|uint4|int2|int3|int4|float4x4|float3x3|float4x3)\s+([A-Za-z_][A-Za-z0-9_]*)")


def block_fields(lane, struct_re, field_re, only_paths=None):
    """{path: (added names, removed names)} for added/removed fields inside a struct block."""
    out = {}
    for p, f in lane.files.items():
        if only_paths and p not in only_paths:
            continue
        if f["text"] is None:
            continue
        lines = f["text"].splitlines()
        rng = block_range(lines, struct_re)
        if not rng:
            continue
        added, removed = [], []
        for h in f["hunks"]:
            for no, t in h.plus:
                if rng[0] < no <= rng[1] and not t.strip().startswith(("//", "///", "#")):
                    m = field_re.match(t)
                    if m:
                        added.append(m[1])
            # removed lines: base side, attribute them to this block when the hunk touches it
            if h.os + h.ol >= rng[0] - 5 and h.ns <= rng[1]:
                for t in h.minus:
                    m = field_re.match(t)
                    if m and not t.strip().startswith("//"):
                        removed.append(m[1])
        added = [a for a in added if a not in removed]
        removed = [r for r in removed if r not in [x for x in added]]
        if added or removed:
            out[p] = (added, removed)
    return out


def collect_fields(lanes, struct_re, field_re, paths=None):
    """{lane name: {"added": [...], "removed": [...], "where": [paths]}}"""
    res = {}
    for ln in lanes:
        d = block_fields(ln, struct_re, field_re, paths)
        if d:
            res[ln.name] = {
                "added": [a for v in d.values() for a in v[0]],
                "removed": [r for v in d.values() for r in v[1]],
                "where": sorted(d),
            }
    return res


def print_fields(title, res, note=""):
    print(f"  {title}{note}")
    if not res:
        print("    no lane adds fields")
        return 0
    for n, v in res.items():
        add = ", ".join(v["added"]) or "-"
        rem = f"; removed/renamed: {', '.join(v['removed'])}" if v["removed"] else ""
        print(f"    {n}: +{len(v['added'])} [{add}]{rem}  ({', '.join(v['where'])})")
    appenders = [n for n, v in res.items() if v["added"]]
    names = {}
    for n in appenders:
        for a in res[n]["added"]:
            names.setdefault(a, []).append(n)
    dup = {a: ls for a, ls in names.items() if len(ls) > 1}
    bad = 0
    if len(appenders) > 1:
        print(f"    COLLISION: {len(appenders)} lanes add fields ({', '.join(appenders)}); "
              "appended at the same end they conflict, and the layout order depends on merge order")
        bad = 1
    for a, ls in dup.items():
        print(f"    COLLISION: field `{a}` added by {', '.join(ls)}")
        bad = 1
    return bad


FRAME_ARM = re.compile(r"^\s*((?:\d+(?:\s*\.\.=?\s*\d+)?\s*\|?\s*)+)\s*(?:if\b.*)?=>")
FRAME_EQ = re.compile(r"(?:rendered|frame|n)\s*(?:==|>=|<=|<|>)\s*(\d{2,4})")
FRAME_CONST = re.compile(r"^\s*(?:pub\s+)?const\s+([A-Z_]*FRAME[A-Z_]*|SCENES_FROM)\s*:\s*u32\s*=\s*(\d+)")


def frame_numbers(lane):
    nums = {}  # number -> source text
    consts = []
    for p, f in lane.files.items():
        if not p.endswith(".rs") and not p.endswith("testing.md"):
            continue
        if p.endswith(".rs") and not (p.endswith("selftest.rs") or p.endswith("main.rs")):
            continue
        for h in f["hunks"]:
            for no, t in h.plus:
                if p.endswith("testing.md"):
                    continue
                m = FRAME_CONST.match(t)
                if m:
                    consts.append(f"{m[1]}={m[2]}")
                    continue
                if not p.endswith("selftest.rs"):
                    continue
                m = FRAME_ARM.match(t)
                if m:
                    for part in re.split(r"\|", m[1]):
                        part = part.strip()
                        r = re.match(r"(\d+)\s*\.\.=?\s*(\d+)$", part)
                        inclusive = "..=" in part
                        if r:
                            a, b = int(r[1]), int(r[2]) + (0 if inclusive else -1)
                            for k in range(a, b + 1):
                                nums[k] = t.strip()
                        elif part.isdigit():
                            nums[int(part)] = t.strip()
                for q in FRAME_EQ.findall(t):
                    nums.setdefault(int(q), t.strip())
    return nums, consts


def entries_info(lane):
    """(hotreload ENTRIES count in the working tree, added entry lines, build.rs touched?)"""
    out = {}
    f = lane.files.get("src/hotreload.rs")
    if f and f["text"]:
        m = re.search(r"pub const ENTRIES:[^=]*;\s*(\d+)\]", f["text"])
        rng = block_range(f["text"].splitlines(), r"pub const ENTRIES")
        added = []
        if rng:
            for h in f["hunks"]:
                for no, t in h.plus:
                    if rng[0] < no <= rng[1]:
                        mm = re.match(r'^\s*\("([^"]+)",\s*"([^"]+)"', t)
                        if mm:
                            added.append(f"{mm[1]}:{mm[2]}")
        out["hotreload"] = (int(m[1]) if m else None, added)
    b = lane.files.get("build.rs")
    if b:
        out["build.rs"] = [t.strip() for h in b["hunks"] for _, t in h.plus if re.search(r"\.hlsl|ENTRIES|entry", t)]
    return out


# ---------------------------------------------------------------- main

def parse_lane(arg):
    base = "HEAD"
    if "@" in arg:
        arg, base = arg.rsplit("@", 1)
    if "=" in arg:
        name, path = arg.split("=", 1)
    else:
        path = arg
        name = os.path.basename(path.rstrip("/")).replace("caustica-", "")
    return name, os.path.expanduser(path), base


def main(argv):
    specs = [parse_lane(a) for a in argv] if argv else DEFAULT_LANES
    lanes = []
    for n, p, b in specs:
        if not os.path.isdir(p):
            print(f"skip {n}: {p} is not a directory", file=sys.stderr)
            continue
        lanes.append(Lane(n, p, b))
    if len(lanes) < 2:
        print("need at least two lanes", file=sys.stderr)
        return 2

    print("LANES")
    for ln in lanes:
        head = git(ln.wt, "rev-parse", "--short", "HEAD").strip()
        nh = sum(len(f["hunks"]) for f in ln.files.values())
        nn = sum(1 for f in ln.files.values() if f["new"])
        print(f"  {ln.name:9} HEAD {head}, diffed against {ln.base}: {len(ln.files)} files ({nn} new), {nh} hunks")

    # (1) shared files
    owners = {}
    for ln in lanes:
        for p in ln.files:
            owners.setdefault(p, []).append(ln)
    shared = {p: ls for p, ls in owners.items() if len(ls) > 1}
    print(f"\n(1) FILES TOUCHED BY TWO OR MORE LANES: {len(shared)}")
    summary = {"overlap": [], "adjacent": [], "identical": [], "disjoint": []}
    for p in sorted(shared):
        ls = shared[p]
        pairs = {}
        for a, b in itertools.combinations(ls, 2):
            pairs[(a.name, b.name)] = pair_status(a.files[p], b.files[p])
        order = {"overlap": 3, "adjacent": 2, "identical": 1, "disjoint": 0}
        worst = max(pairs.values(), key=lambda s: order[s])
        summary[worst].append(p)
        verdict = {"overlap": "OVERLAP (merge conflict)", "adjacent": "ADJACENT (git still conflicts)",
                   "identical": "identical, no conflict", "disjoint": "disjoint, merges cleanly"}[worst]
        print(f"  {p}: {', '.join(l.name for l in ls)}  -> {verdict}")
        for l in ls:
            f = l.files[p]
            if f["new"]:
                rs = f"new file, {len(f['hunks'][0].plus)} lines"
            elif f["deleted"]:
                rs = "deleted"
            else:
                rs = ", ".join(h.label() for h in f["hunks"])
            print(f"      {l.name:9} {rs}")
        if len(ls) > 2:
            print("      pairs: " + "; ".join(f"{a} x {b} {s}" for (a, b), s in pairs.items()))
        # name the overlapping ranges
        for a, b in itertools.combinations(ls, 2):
            fa, fb = a.files[p], b.files[p]
            if fa["new"] or fb["new"] or fa["text"] == fb["text"]:
                continue
            hits = []
            for ha in fa["hunks"]:
                for hb in fb["hunks"]:
                    r = relation(ha, hb)
                    if r in ("overlap", "adjacent"):
                      hits.append(f"{ha.label()} ~ {hb.label()} ({r})")
                    elif r == "identical":
                        hits.append(f"{ha.label()} = {hb.label()} (identical patch)")
            if hits:
                print(f"      {a.name} x {b.name}: " + "; ".join(hits[:12]) + (" ..." if len(hits) > 12 else ""))
    print(f"  summary: {len(summary['overlap'])} overlapping, {len(summary['adjacent'])} adjacent only, "
          f"{len(summary['identical'])} identical, {len(summary['disjoint'])} disjoint")

    # (2) special collisions
    print("\n(2) SPECIAL COLLISIONS")
    bad = 0
    hl = [ln for ln in lanes]
    cb = collect_fields(hl, r"^\s*cbuffer\s+Frame\b", HLSL_FIELD, {"shaders/common.hlsli"})
    bad += print_fields("cbuffer Frame (shaders/common.hlsli)", cb)
    cs = collect_fields(hl, r"pub struct Constants\b", RUST_FIELD)
    bad += print_fields("Constants (src/render.rs, src/gpu.rs, ...)", cs)
    for n in sorted(set(cb) | set(cs)):
        a, b = len(cb.get(n, {}).get("added", [])), len(cs.get(n, {}).get("added", []))
        if a != b:
            print(f"    MISMATCH in {n}: {a} cbuffer fields vs {b} Constants fields added (must stay field for field)")
            bad += 1
    st = collect_fields(hl, r"pub struct Settings\b", RUST_FIELD)
    bad += print_fields("Settings fields", st, " (each also needs validate(), UI range, transport_key, keep_output/preset())")

    print("  build.rs / hotreload::ENTRIES")
    counts = {}
    for ln in lanes:
        info = entries_info(ln)
        if not info:
            continue
        hr = info.get("hotreload")
        if hr:
            counts[ln.name] = hr[0]
            print(f"    {ln.name}: ENTRIES now {hr[0]}, added {len(hr[1])} {hr[1] if hr[1] else ''}")
        if "build.rs" in info:
            print(f"    {ln.name}: build.rs touched ({len(info['build.rs'])} entry/hlsl lines added)")
    touchers = [ln.name for ln in lanes if "build.rs" in ln.files or "src/hotreload.rs" in ln.files]
    if len(touchers) > 1:
        print(f"    COLLISION: {', '.join(touchers)} all edit the shader entry list; the array length and the "
              "build.rs / ENTRIES / Shaders / reload_shaders order must be redone by hand after merging")
        bad += 1
    elif not touchers:
        print("    no lane touches them")

    print("  self-test frame numbers (src/selftest.rs match arms and frame tests, SCENES_FROM in main.rs)")
    used = {}
    for ln in lanes:
        nums, consts = frame_numbers(ln)
        if nums or consts:
            ks = sorted(nums)
            print(f"    {ln.name}: frames {compress(ks) if ks else '-'}" + (f"; consts {', '.join(consts)}" if consts else ""))
        for k in nums:
            used.setdefault(k, set()).add(ln.name)
    clash = {k: v for k, v in used.items() if len(v) > 1}
    if clash:
        print(f"    COLLISION: frames used by more than one lane: "
              + "; ".join(f"{k} ({', '.join(sorted(v))})" for k, v in sorted(clash.items())))
        bad += 1
    sf = [ln.name for ln in lanes if any("SCENES_FROM" in c for c in frame_numbers(ln)[1])]
    if len(sf) > 1:
        print(f"    COLLISION: {', '.join(sf)} each move SCENES_FROM; the first scene-file frame depends on merge order")
        bad += 1
    if not used and not sf:
        print("    no lane adds self-test frames")

    print("  docs (docs/, README.md, THIRD-PARTY-NOTICES.md, CLAUDE.md) edited by several lanes")
    doc_owners = {}
    for ln in lanes:
        for p in ln.files:
            if p.startswith("docs/") or p in ("README.md", "THIRD-PARTY-NOTICES.md", "CLAUDE.md"):
                doc_owners.setdefault(p, []).append(ln.name)
    any_doc = False
    for p, ns in sorted(doc_owners.items()):
        if len(ns) > 1:
            any_doc = True
            tag = "EVERY lane" if len(ns) == len(lanes) else f"{len(ns)} of {len(lanes)} lanes"
            st = summary_status(p, shared, lanes)
            print(f"    {p}: {tag} ({', '.join(ns)}) -> {st}")
    if not any_doc:
        print("    none")
    print(f"\nRESULT: {len(summary['overlap']) + len(summary['adjacent'])} files conflict textually, "
          f"{bad} special collision groups")
    return 0


def compress(ks):
    out, i = [], 0
    while i < len(ks):
        j = i
        while j + 1 < len(ks) and ks[j + 1] == ks[j] + 1:
            j += 1
        out.append(str(ks[i]) if i == j else f"{ks[i]}-{ks[j]}")
        i = j + 1
    return ", ".join(out)


def summary_status(p, shared, lanes):
    ls = shared.get(p)
    if not ls:
        return "n/a"
    order = {"overlap": 3, "adjacent": 2, "identical": 1, "disjoint": 0}
    worst = max((pair_status(a.files[p], b.files[p]) for a, b in itertools.combinations(ls, 2)), key=lambda s: order[s])
    return {"overlap": "overlapping hunks", "adjacent": "adjacent hunks", "identical": "identical, no conflict",
            "disjoint": "disjoint hunks, merges cleanly"}[worst]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
