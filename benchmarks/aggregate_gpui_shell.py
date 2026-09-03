#!/usr/bin/env python3
"""Aggregate gpui-shell's one-process samples into gpui-shell-jit-v1 evidence."""

import json
import os
import subprocess
import sys
from pathlib import Path


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", root, *args], text=True).strip()


def source_revision(root: Path) -> tuple[str, bool]:
    """Return provenance for a checkout or an explicitly marked clean source copy."""
    try:
        return git(root, "rev-parse", "HEAD"), bool(git(root, "status", "--porcelain"))
    except subprocess.CalledProcessError:
        marker = root / ".source-revision"
        if not marker.is_file():
            raise SystemExit(f"{root} is not a git checkout and has no .source-revision marker")
        revision = marker.read_text().strip()
        if len(revision) != 40 or any(char not in "0123456789abcdef" for char in revision):
            raise SystemExit(f"invalid source revision marker: {marker}")
        # The integration patch intentionally changes the marked source copy.
        return revision, True


def samples(directory: Path, workload: str, mode: str) -> list[dict]:
    result = []
    for pair in range(30):
        path = directory / f"pair-{pair}-{workload}-{mode}.json"
        value = json.loads(path.read_text())
        if value["mode"] != mode or value["workload"] != workload or value["pair_index"] != pair:
            raise SystemExit(f"invalid mode/pair in {path}")
        result.append(value)
    return result


def main() -> None:
    if len(sys.argv) != 5:
        raise SystemExit("usage: aggregate_gpui_shell.py SAMPLE_DIR SHELL_ROOT RQUICKJS_ROOT OUTPUT")
    directory, shell_root, rquickjs_root, output = map(Path, sys.argv[1:])
    workload_keys = (
        "pair_index", "steady_state_ns", "p99_script_render_ns", "checksum",
        "snapshot_sha256", "script_renders", "native_enabled", "native_entries",
        "fallback_count", "installed", "compile_failures",
        "unsupported_opcode_failures", "tier1_rejections",
        "resource_limit_failures", "cancelled_compilations", "compiler_panics",
        "invalid_artifacts", "install_failures", "native_exits", "osr_entries",
        "deopts",
    )
    lifecycle_keys = (
        "pair_index", "first_window_ns", "hot_reload_ns", "snapshot_sha256",
        "script_renders", "reload_observations",
    )
    select = lambda rows, keys: [{key: row[key] for key in keys} for row in rows]
    shell_revision, shell_dirty = source_revision(shell_root)
    rquickjs_revision, rquickjs_dirty = source_revision(rquickjs_root)
    document = {
        "schema": "gpui-shell-jit-v1",
        "provenance": {
            "shell_revision": shell_revision,
            "rquickjs_revision": rquickjs_revision,
            "shell_dirty": shell_dirty,
            "rquickjs_dirty": rquickjs_dirty,
            "test_binary_sha256": os.environ.get("GPUI_SHELL_TEST_BINARY_SHA256", ""),
            "integration_patch_sha256": os.environ.get("GPUI_SHELL_INTEGRATION_PATCH_SHA256", ""),
            "cpu_affinity": os.environ.get("GPUI_SHELL_JIT_CPU", "0"),
            "target_triple": subprocess.check_output(
                ["rustc", "-vV"], text=True
            ).split("host: ", 1)[1].splitlines()[0],
            "command": ["scripts/bench-gpui-shell.sh", str(shell_root), str(output)],
        },
        "policy": {
            "warmup_processes": 5,
            "paired_processes": 30,
            "bootstrap_resamples": 10000,
        },
        "workloads": [],
        "lifecycle": {
            "interpreter": select(samples(directory, "panel", "interpreter"), lifecycle_keys),
            "automatic": select(samples(directory, "panel", "automatic"), lifecycle_keys),
        },
    }
    for workload, name, suitable, regression_guard, native_required in (
        ("panel", "realistic 443-node host-heavy panel", False, True, False),
        ("compute", "render-driven numeric layout checksum", True, False, True),
        ("mixed", "market compute, sort, aggregate, and visible list", False, False, True),
    ):
        document["workloads"].append({
            "name": name,
            "suitable_for_jit": suitable,
            "regression_guard": regression_guard,
            "native_required": native_required,
            "interpreter": select(samples(directory, workload, "interpreter"), workload_keys),
            "automatic": select(samples(directory, workload, "automatic"), workload_keys),
        })
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
