#!/usr/bin/env python3
"""Regenerate sys/patches and their fingerprints from a patch branch of sys/quickjs.

Usage: python3 scripts/regen-quickjs-patches.py [BRANCH]

The submodule's HEAD (as recorded in the superproject) is the baseline. BRANCH
(default `jit`) is a branch of sys/quickjs whose commits on top of the baseline
are the patches, one per commit, named by the commit subject (`0001-rquickjs-jit`).
The script rewrites sys/patches/*.patch and the fingerprints in
sys/build_support/patch.rs (baseline, patch digests, patched files).
"""
from pathlib import Path
import re
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
qjs = root / "sys" / "quickjs"
branch = sys.argv[1] if len(sys.argv) > 1 else "jit"
patch_rs = root / "sys" / "build_support" / "patch.rs"


def git(*args, cwd=qjs, text=True):
    return subprocess.check_output(["git", *args], cwd=cwd, text=text)


baseline = git("ls-files", "-s", "sys/quickjs", cwd=root).split()[1]
commits = git("rev-list", "--reverse", f"{baseline}..{branch}").split()


def fnv(data, seed=0xCBF29CE484222325):
    for b in data:
        seed = ((seed ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return seed


source = patch_rs.read_text()
baseline_files = re.findall(r'"([^"]+)"', source.split("pub(crate) const BASELINE_FILES")[1].split("];")[0])
h = 0xCBF29CE484222325
for f in baseline_files:
    h = fnv(f.encode(), h)
    h = fnv(b"\0", h)
    h = fnv(git("show", f"{baseline}:{f}", text=False).replace(b"\r\n", b"\n"), h)

patch_dir = root / "sys" / "patches"
for old in patch_dir.glob("*.patch"):
    old.unlink()
patches = []
for c in commits:
    name = git("log", "-1", "--format=%s", c).strip()
    if not re.fullmatch(r"\d{4}-[a-z0-9.-]+", name):
        raise SystemExit(f"commit {c} subject is not a patch name: {name!r}")
    diff = git("diff", "--no-color", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/",
               f"{c}^", c, text=False)
    (patch_dir / f"{name}.patch").write_bytes(diff)
    patches.append((f"{name}.patch", fnv(diff)))

patched = re.findall(r'\("([^"]+)", 0x', source.split("const PATCHED_FILE_FINGERPRINTS")[1].split("];")[0])
files = []
for f in patched:
    files.append((f, fnv(git("show", f"{branch}:{f}", text=False))))


def hexs(v):
    return f"0x{v:016x}"


source = re.sub(r"const QUICKJS_BASELINE_FNV64: u64 = 0x[0-9a-f_]+;",
                f"const QUICKJS_BASELINE_FNV64: u64 = {hexs(h)};", source)
source = re.sub(r"const EXPECTED_PATCHES: \[\(&str, u64\); \d+\] = \[.*?\];",
                f"const EXPECTED_PATCHES: [(&str, u64); {len(patches)}] = [\n"
                + "".join(f'    ("{n}", {hexs(d)}),\n' for n, d in patches) + "];",
                source, flags=re.S)
source = re.sub(r"const PATCHED_FILE_FINGERPRINTS: \[\(&str, u64\); \d+\] = \[.*?\];",
                f"const PATCHED_FILE_FINGERPRINTS: [(&str, u64); {len(files)}] = [\n"
                + "".join(f'    ("{n}", {hexs(d)}),\n' for n, d in files) + "];",
                source, flags=re.S)
source = re.sub(r"expected [0-9a-f]{7} source-set fingerprint", f"expected {baseline[:7]} source-set fingerprint", source)
patch_rs.write_text(source)
print(f"{len(patches)} patches on {baseline[:7]} from {branch}")
