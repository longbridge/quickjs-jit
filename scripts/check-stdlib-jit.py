#!/usr/bin/env python3
"""Test the facade from an external JIT host with an application-owned patch."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
bindings = tomllib.loads((root / "Cargo.toml").read_text())
version = bindings["package"]["version"]

with tempfile.TemporaryDirectory(prefix="stdlib-jit-host-") as directory:
    host = Path(directory)
    compat = host / "compat"
    (compat / "src").mkdir(parents=True)
    # The fixture belongs to this temporary application, never to the library.
    features = {name: [f"upstream/{name}"] for name in bindings["features"]}
    (compat / "Cargo.toml").write_text(
        '[package]\nname = "rquickjs"\n'
        f'version = "{version}"\nedition = "2021"\npublish = false\n'
        '[dependencies]\nupstream = { package = "quickjs-jit", '
        f'path = {json.dumps(str(root))}, default-features = false }}\n'
        '[features]\n'
        + ''.join(f'{name} = {json.dumps(values)}\n' for name, values in features.items())
    )
    (compat / "src/lib.rs").write_text('#![no_std]\npub use upstream::*;\n')
    (host / "tests").mkdir()
    shutil.copyfile(root / "stdlib/tests/modules.rs", host / "tests/modules.rs")
    (host / "Cargo.toml").write_text(
        '[package]\nname = "stdlib-jit-host"\nversion = "0.0.0"\n'
        'edition = "2021"\npublish = false\n[workspace]\n'
        '[dependencies]\nrquickjs = { package = "quickjs-jit", '
        f'path = {json.dumps(str(root))}, features = ["futures", "loader", "macro", "half"] }}\n'
        f'quickjs-jit-stdlib = {{ path = {json.dumps(str(root / "stdlib"))} }}\n'
        'tokio = { version = "1", features = ["macros", "rt", "time"] }\n'
        '[features]\nparallel = ["quickjs-jit-stdlib/parallel", "rquickjs/parallel"]\n'
        '[patch.crates-io]\nrquickjs = { path = "compat" }\n'
    )
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(root / "target/stdlib-jit-host"))
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1"], cwd=host, env=env
    ))
    packages = metadata["packages"]
    assert not any(p["name"] in {"rquickjs-core", "rquickjs-sys", "rquickjs-macro"}
                   for p in packages), "Host resolved original bindings alongside JIT"
    assert sum(p["name"] == "quickjs-jit-sys" for p in packages) == 1
    llrt = [p for p in packages if p["name"].startswith("llrt_")]
    assert llrt and all((p["source"] or "").startswith("git+https://github.com/awslabs/llrt")
                        for p in llrt), "LLRT must remain an external Git dependency"
    for flags in ([], ["--all-features"]):
        subprocess.run(["cargo", "test", *flags], cwd=host, env=env, check=True)
