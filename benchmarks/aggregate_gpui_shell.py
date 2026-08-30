#!/usr/bin/env python3
"""Aggregate gpui-shell's one-process samples into gpui-shell-jit-v1 evidence."""

import json
import subprocess
import sys
from pathlib import Path


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", root, *args], text=True).strip()


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
        "snapshot_sha256", "script_renders", "native_entries", "fallback_count",
    )
    lifecycle_keys = (
        "pair_index", "first_window_ns", "hot_reload_ns", "snapshot_sha256",
        "script_renders",
    )
    select = lambda rows, keys: [{key: row[key] for key in keys} for row in rows]
    document = {
        "schema": "gpui-shell-jit-v1",
        "provenance": {
            "shell_revision": git(shell_root, "rev-parse", "HEAD"),
            "rquickjs_revision": git(rquickjs_root, "rev-parse", "HEAD"),
            "shell_dirty": bool(git(shell_root, "status", "--porcelain")),
            "rquickjs_dirty": bool(git(rquickjs_root, "status", "--porcelain")),
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
    for workload, name, suitable, regression_guard in (
        ("panel", "realistic 443-node host-heavy panel", False, True),
        ("compute", "render-driven numeric layout checksum", True, False),
    ):
        document["workloads"].append({
            "name": name,
            "suitable_for_jit": suitable,
            "regression_guard": regression_guard,
            "interpreter": select(samples(directory, workload, "interpreter"), workload_keys),
            "automatic": select(samples(directory, workload, "automatic"), workload_keys),
        })
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
