#!/usr/bin/env python3
"""Vendor LLRT's standard modules and libraries into the `quickjs-jit-stdlib` crate.

Usage: python3 scripts/import-stdlib.py /path/to/llrt

The checkout must be at REVISION with clean `libs/` and `modules/`; the script never
downloads code. Every LLRT crate becomes one public module of the package (`llrt_url`
-> `quickjs_jit_stdlib::url`) behind its own Cargo feature:

- `<module>` enables the module with its default backends (pure Rust where LLRT
  offers a choice, see DEFAULTS),
- `_<module>` enables the module alone (what LLRT's `default-features = false` is),
- `<module>-<feature>` (or the LLRT feature name itself when it already starts with
  `<module>-`) is one of LLRT's features of that crate.

Dependencies between LLRT crates become feature dependencies, so enabling `zlib`
enables `_buffer`, `_compression`, `_context` and `_utils` and nothing else. External
dependencies are declared once, optional, without default features; each feature
enables what its LLRT crate asked for.
"""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

if sys.version_info < (3, 11):
    raise SystemExit("Python 3.11 or newer is required")
import tomllib

REVISION = "d9fd873e793c6551c5d92b2e31480f3279e3cd8d"
VERSION = "0.14.0"
# `llrt_build` is replaced by this package's `[lints]` (it only declares `rust_nightly`).
SKIP = {"llrt_build"}
# The test helpers (`llrt_test_tls`: certificates for fetch / tls / http) are compiled
# into the package's own unit tests only.
TEST_ONLY = {"llrt_test", "llrt_test_tls"}
# Default backends where LLRT's default is a C library and a Rust one exists.
DEFAULTS = {
    "llrt_compression": ["brotli-rust", "flate2-rust"],
    "llrt_zlib": ["compression-rust"],
    "llrt_fetch": ["http1", "http2", "compression-rust", "webpki-roots", "tls-ring"],
}
# Facade defaults: what the old facade re-exported.
PACKAGE_DEFAULT = ["buffer", "crypto", "path", "url", "zlib"]

root = Path(sys.argv[1]).resolve()
head = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
if head != REVISION:
    raise SystemExit("LLRT checkout must be at " + REVISION)
subprocess.run(["git", "-C", str(root), "diff", "--exit-code", "HEAD", "--", "libs", "modules"],
               check=True, stdout=subprocess.DEVNULL)
dest = Path(__file__).resolve().parents[1] / "stdlib"

crates = {}
for p in sorted(root.glob("*/*/Cargo.toml")):
    if p.parent.parent.name not in ("libs", "modules"):
        continue
    data = tomllib.loads(p.read_text())
    name = data["package"]["name"]
    if name not in SKIP:
        crates[name] = (p.parent, data)


def short(name):
    return {"llrt_test": "test_support", "llrt_test_tls": "test_tls_support"}.get(
        name, name.removeprefix("llrt_"))


# --- Local patches, applied to the parsed manifests and the sources ---------------
# zstd has no Rust implementation (LLRT's `zstd-rust` is the C library too): keep it
# behind zlib's own `zstd` feature so the default build is C-free.
zlib = crates["llrt_zlib"][1]["features"]
zlib["compression-rust"] = [f for f in zlib["compression-rust"] if "zstd" not in f]
zlib["compression-c"].append("zstd")
zlib["zstd"] = ["llrt_compression/zstd-rust"]

SOURCE_PATCHES = {
    # (file, old, new): every `old` must occur exactly once.
    "modules/llrt_zlib/src/lib.rs": [
        ("mod zstd;", '#[cfg(feature = "zstd")]\nmod zstd;'),
        ("use self::zstd::", '#[cfg(feature = "zstd")]\nuse self::zstd::'),
        ('        declare.declare("zstdCompress")?;\n        declare.declare("zstdCompressSync")?;\n\n'
         '        declare.declare("zstdDecompress")?;\n        declare.declare("zstdDecompressSync")?;\n',
         '        #[cfg(feature = "zstd")]\n        {\n'
         '            declare.declare("zstdCompress")?;\n            declare.declare("zstdCompressSync")?;\n'
         '            declare.declare("zstdDecompress")?;\n            declare.declare("zstdDecompressSync")?;\n'
         '        }\n'),
        ('            default.set("zstdCompress", Func::from(zstd_comp))?;\n'
         '            default.set("zstdCompressSync", Func::from(zstd_comp_sync))?;\n\n'
         '            default.set("zstdDecompress", Func::from(zstd_decomp))?;\n'
         '            default.set("zstdDecompressSync", Func::from(zstd_decomp_sync))?;\n',
         '            #[cfg(feature = "zstd")]\n            {\n'
         '                default.set("zstdCompress", Func::from(zstd_comp))?;\n'
         '                default.set("zstdCompressSync", Func::from(zstd_comp_sync))?;\n'
         '                default.set("zstdDecompress", Func::from(zstd_decomp))?;\n'
         '                default.set("zstdDecompressSync", Func::from(zstd_decomp_sync))?;\n'
         '            }\n'),
    ],
    # Buffer storage counts against the runtime's memory limit: QuickJS allocates it
    # (JS_NewArrayBufferCopy) instead of Rust's global allocator, and Buffer.alloc*
    # fill it in place instead of building a Rust Vec of the full size first.
    "modules/llrt_buffer/src/buffer.rs": [
        ("use std::{mem::MaybeUninit, slice};", "use std::{ptr, slice};"),
        ("impl<'js> IntoJs<'js> for Buffer {\n    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {\n"
         "        let array_buffer = ArrayBuffer::new(ctx.clone(), self.0)?;\n",
         "/// A new `ArrayBuffer` of `length` bytes that QuickJS allocates, so it counts against\n"
         "/// the runtime's memory limit (an out-of-memory exception past it), zeroed and then\n"
         "/// filled by `fill`.\n"
         "pub fn new_array_buffer<'js>(\n    ctx: &Ctx<'js>,\n    length: usize,\n"
         "    fill: impl FnOnce(&mut [u8]),\n) -> Result<ArrayBuffer<'js>> {\n"
         "    // SAFETY: a null source makes QuickJS allocate `length` zeroed bytes itself.\n"
         "    let value = unsafe {\n"
         "        let raw = rquickjs::qjs::JS_NewArrayBufferCopy(ctx.as_raw().as_ptr(), ptr::null(), length as _);\n"
         "        if rquickjs::qjs::JS_IsException(raw) {\n            return Err(rquickjs::Error::Exception);\n        }\n"
         "        Value::from_raw(ctx.clone(), raw)\n    };\n"
         "    let array_buffer = ArrayBuffer::from_value(value).or_throw(ctx)?;\n"
         "    if length > 0 {\n"
         "        let mut raw = array_buffer.as_raw().or_throw_msg(ctx, ERROR_MSG_ARRAY_BUFFER_DETACHED)?;\n"
         "        // SAFETY: the buffer is new, so nothing else can observe it while `fill` runs.\n"
         "        fill(unsafe { raw.as_mut() });\n    }\n"
         "    Ok(array_buffer)\n}\n\n"
         "impl<'js> IntoJs<'js> for Buffer {\n    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {\n"
         "        let array_buffer = new_array_buffer(ctx, self.0.len(), |bytes| bytes.copy_from_slice(&self.0))?;\n"),
        ("        if let Some(value) = value.as_int() {\n            let bytes = vec![value as u8; length];\n"
         "            return Buffer(bytes).into_js(&ctx);\n        }\n",
         "        if let Some(value) = value.as_int() {\n"
         "            let array_buffer = new_array_buffer(&ctx, length, |bytes| bytes.fill(value as u8))?;\n"
         "            return Buffer::from_array_buffer(&ctx, array_buffer);\n        }\n"),
        ("    Buffer(vec![0; length]).into_js(&ctx)\n}\n",
         "    Buffer::from_array_buffer(&ctx, new_array_buffer(&ctx, length, |_| {})?)\n}\n"),
        ("    let mut bytes = vec![0; length];\n    let byte_ref_length = byte_ref.len();\n"
         "    for i in 0..length {\n        bytes[i] = byte_ref[i % byte_ref_length];\n    }\n"
         "    Buffer(bytes).into_js(ctx)\n",
         "    let byte_ref_length = byte_ref.len();\n"
         "    let array_buffer = new_array_buffer(ctx, length, |bytes| {\n"
         "        for (i, byte) in bytes.iter_mut().enumerate() {\n"
         "            *byte = byte_ref[i % byte_ref_length];\n        }\n    })?;\n"
         "    Buffer::from_array_buffer(ctx, array_buffer)\n"),
        # QuickJS zeroes what it allocates: allocUnsafe is alloc without the fill.
        ("    let mut bytes: Vec<MaybeUninit<u8>> = Vec::with_capacity(size);\n    unsafe {\n"
         "        bytes.set_len(size);\n    }\n\n    Buffer(maybeuninit_to_u8(bytes)).into_js(&ctx)\n",
         "    Buffer::from_array_buffer(&ctx, new_array_buffer(&ctx, size, |_| {})?)\n"),
        ("fn maybeuninit_to_u8(vec: Vec<MaybeUninit<u8>>) -> Vec<u8> {", "#[cfg(any())]\nfn maybeuninit_to_u8(vec: Vec<std::mem::MaybeUninit<u8>>) -> Vec<u8> {"),
        ("    let layout = std::alloc::Layout::array::<u8>(size).or_throw(&ctx)?;\n\n    let bytes = unsafe {\n"
         "        let ptr = std::alloc::alloc(layout);\n        if ptr.is_null() {\n"
         "            return Err(Exception::throw_internal(&ctx, \"Memory allocation failed\"));\n        }\n"
         "        Vec::from_raw_parts(ptr, size, size)\n    };\n    Buffer(bytes).into_js(&ctx)\n",
         "    Buffer::from_array_buffer(&ctx, new_array_buffer(&ctx, size, |_| {})?)\n"),
    ],
    # PBKDF2 runs synchronously on the JS thread, where the VM's interrupt budget
    # cannot stop it: cap its work (iterations x output hash blocks) at 10,000,000,
    # about 16 times OWASP's 600,000 SHA-256 iterations for one block.
    "modules/llrt_crypto/src/subtle/derive_bits.rs": [
        ("            let out_length = (length / 8).try_into().or_throw(ctx)?;\n"
         "            CRYPTO_PROVIDER\n                .pbkdf2_derive_key(",
         "            let out_length: usize = (length / 8).try_into().or_throw(ctx)?;\n"
         "            let blocks = out_length.div_ceil(hash.digest_len().max(1)) as u64;\n"
         "            if u64::from(*iterations).saturating_mul(blocks) > PBKDF2_MAX_WORK {\n"
         "                return Err(DOMException::operation_error(\n"
         "                    ctx,\n"
         "                    \"PBKDF2 iterations times output blocks exceeds 10000000\",\n"
         "                ));\n            }\n"
         "            CRYPTO_PROVIDER\n                .pbkdf2_derive_key("),
        ("fn truncate_derived_bits(",
         "/// The most PBKDF2 work (iterations times output hash blocks) one call may do.\n"
         "pub const PBKDF2_MAX_WORK: u64 = 10_000_000;\n\nfn truncate_derived_bits("),
    ],
    # A host without LLRT's DOMException global (one with its own, or none) gets the
    # creation error thrown instead of a panic across the JS boundary.
    "modules/llrt_exceptions/src/lib.rs": [
        ('        let value = Self::create(ctx, name, message).expect("failed to create DOMException");\n'
         '        Self::throw_value(ctx, value)\n',
         '        match Self::create(ctx, name, message) {\n'
         '            Ok(value) => Self::throw_value(ctx, value),\n'
         '            Err(error) => error,\n        }\n'),
    ],
    # One crate's tests see every dev-dependency: serde_json's `PartialEq<Value> for usize`
    # makes `vec![]` ambiguous.
    "libs/llrt_utils/src/reuse_list.rs": [
        ("assert_eq!(list.slots, vec![]);", "assert_eq!(list.slots, Vec::<usize>::new());", 5),
    ],
    # The signal helpers define OS implementations for Unix and Windows only.
    "libs/llrt_utils/src/signals.rs": [
        ("pub fn kill(ctx: &Ctx<'_>",
         "// Other targets (WebAssembly) cannot send operating-system signals.\n"
         "#[cfg(not(any(unix, windows)))]\n"
         "pub fn kill_process_raw(_pid: u32, _signal: i32) -> io::Result<()> {\n"
         "    Err(io::Error::new(\n        io::ErrorKind::Unsupported,\n"
         "        \"process signals require a native host\",\n    ))\n}\n\n"
         "pub fn kill(ctx: &Ctx<'_>"),
        ("            #[cfg(windows)]\n            let is_not_found = true;",
         "            #[cfg(not(any(unix, windows)))]\n            let is_not_found = false;\n\n"
         "            #[cfg(windows)]\n            let is_not_found = true;"),
    ],
}


# --- Features ---------------------------------------------------------------------
def deps_of(data, kind="dependencies"):
    out = [(None, k, v) for k, v in data.get(kind, {}).items()]
    for target, section in data.get("target", {}).items():
        out += [(target, k, v) for k, v in section.get(kind, {}).items()]
    return [(t, k, {"version": v} if isinstance(v, str) else dict(v)) for t, k, v in out]


def feature_name(crate, feat):
    s = short(crate)
    return feat if feat.startswith(s + "-") else f"{s}-{feat}"


def implicit_features(data):
    """Optional dependencies without `dep:` anywhere are features named after them."""
    listed = {e[4:] for fs in data.get("features", {}).values() for e in fs if e.startswith("dep:")}
    return {k for _, k, v in deps_of(data) if v.get("optional") and k not in listed}


def enable_dep(crate, dep, spec):
    """Cargo feature entries that enable dependency `dep` of `crate` as LLRT declared it."""
    if dep in crates:
        entries = [short(dep) if spec.get("default-features", True) else "_" + short(dep)]
        entries += [feature_name(dep, f) for f in spec.get("features", [])]
        return entries
    if dep == "rquickjs":
        return [f"rquickjs/{f}" for f in spec.get("features", [])]
    entries = [f"dep:{dep}"]
    if spec.get("default-features", True) and not defaults_always(dep) and dep not in NO_DEFAULT:
        entries.append(f"{dep}/default")
    entries += [f"{dep}/{f}" for f in spec.get("features", [])]
    return entries


# Dependencies without a `default` feature, which some LLRT crates still disable.
NO_DEFAULT = {"itoa"}


def defaults_always(dep):
    """Whether every LLRT crate keeps `dep`'s default features (then the package does too)."""
    return all(spec.get("default-features", True)
               for c in crates for _, d, spec in deps_of(crates[c][1]) if d == dep)


def translate(crate, entry):
    data = crates[crate][1]
    specs = {k: v for _, k, v in deps_of(data)}
    if entry.startswith("dep:"):
        return enable_dep(crate, entry[4:], specs[entry[4:]])
    if "/" in entry:
        dep, feat = entry.split("/", 1)
        weak = dep.endswith("?")
        dep = dep.rstrip("?")
        if dep in SKIP or dep in TEST_ONLY:
            return []
        if dep in crates:
            # Weak or not, a sibling module's feature enables that module.
            return [feature_name(dep, feat)]
        if dep == "rquickjs":
            return [f"rquickjs/{feat}"]
        return ([] if weak else enable_dep(crate, dep, specs[dep])) + [f"{dep}{'?' if weak else ''}/{feat}"]
    if entry in data.get("features", {}):
        return [feature_name(crate, entry)]
    if entry in implicit_features(data):
        return enable_dep(crate, entry, specs[entry])
    raise SystemExit(f"{crate}: unknown feature entry {entry}")


runtime = [c for c in crates if c not in TEST_ONLY]
features = {}
local_features = {}  # crate -> {LLRT feature: package feature}
for crate in runtime:
    data = crates[crate][1]
    s = short(crate)
    base = []
    for _, dep, spec in deps_of(data):
        if not spec.get("optional"):
            base += enable_dep(crate, dep, spec)
    features["_" + s] = sorted(set(base))
    names = {}
    for feat, entries in data.get("features", {}).items():
        if feat == "default":
            continue
        names[feat] = feature_name(crate, feat)
        out = ["_" + s]
        for e in entries:
            out += translate(crate, e)
        features[names[feat]] = sorted(set(out) - {names[feat]})
    for dep in implicit_features(data):
        names[dep] = feature_name(crate, dep)
        features[names[dep]] = sorted({"_" + s, *enable_dep(crate, dep, {k: v for _, k, v in deps_of(data)}[dep])})
    local_features[crate] = names
    defaults = DEFAULTS.get(crate, data.get("features", {}).get("default", []))
    features[s] = sorted({"_" + s, *(names[f] for f in defaults)})
if len(features) != len(set(features)):
    raise SystemExit("feature names collide")

# --- External dependencies ----------------------------------------------------------
merged = {}
for crate in runtime:
    for target, dep, spec in deps_of(crates[crate][1]):
        if dep in crates or dep in SKIP or dep == "rquickjs":
            continue
        decl = {k: v for k, v in spec.items() if k in ("version", "package")}
        key = (target, dep)
        if key in merged:
            prior = merged[key]
            if prior.get("package") != decl.get("package"):
                raise SystemExit(f"incompatible declarations of {dep}: {prior} / {decl} ({crate})")
            # Crates asking for different versions get all of their requirements at once.
            reqs = {r.strip() for r in (prior["version"] + "," + decl["version"]).split(",")}
            prior["version"] = ", ".join(sorted(reqs))
        else:
            merged[key] = decl
# Development dependencies: the runtime crates' own, and what the test helpers use
# (with the features of their defaults, which are the ring backend for TLS).
dev = {}
uses = [(d, s) for c in runtime for t, d, s in deps_of(crates[c][1], "dev-dependencies") if t is None]
uses += [(d, s) for c in TEST_ONLY for t, d, s in deps_of(crates[c][1]) if t is None and not s.get("optional")]
uses += [(d, {"version": crates[c][1]["dependencies"][d]["version"], "features": ["ring"], "default-features": False})
         for c in TEST_ONLY for d in ("rustls",) if d in crates[c][1].get("dependencies", {})]
for dep, spec in uses:
    if dep in crates or dep in SKIP or dep == "rquickjs":
        continue
    spec = {k: v for k, v in spec.items() if k in ("version", "package", "features", "default-features")}
    if dep in dev:
        dev[dep]["features"] = sorted(set(dev[dep].get("features", [])) | set(spec.get("features", [])))
        dev[dep]["default-features"] = dev[dep].get("default-features", True) or spec.get("default-features", True)
    else:
        dev[dep] = spec
rquickjs_features = sorted({"std", "loader"} | {
    f for c in runtime for _, d, s in deps_of(crates[c][1]) if d == "rquickjs" and not s.get("optional")
    for f in s.get("features", [])})

def fold_constant_cfgs(text):
    """Folds cfg predicates made only of `all()` / `any()` (the frozen backend features)."""
    leaf = re.compile(r"\b(all|any|not)\(\s*((?:(?:all|any)\(\)\s*,?\s*)*)\)")

    def fold(m):
        values = [v == "all" for v in re.findall(r"(all|any)\(\)", m[2])]
        result = {"all": all(values), "any": any(values), "not": not (values and values[0])}[m[1]]
        return "all()" if result else "any()"

    while True:
        folded = leaf.sub(lambda m: m[0] if m[0] in ("all()", "any()") else fold(m), text)
        if folded == text:
            break
        text = folded
    return re.sub(r"[ \t]*#\[cfg\(all\(\)\)\]\n", "", text)


# --- Sources ------------------------------------------------------------------------
vendor = dest / "src" / "llrt"
if vendor.exists():
    shutil.rmtree(vendor)
vendor.mkdir(parents=True)
provenance = {"repository": "https://github.com/awslabs/llrt", "revision": REVISION,
              "patches": sorted(SOURCE_PATCHES), "files": {}}
rewritten = set()
macro_names = {}
for crate in crates:
    for p in (crates[crate][0] / "src").rglob("*.rs"):
        macro_names.setdefault(crate, []).extend(
            re.findall(r"#\[macro_export\]\s*macro_rules!\s+(\w+)", p.read_text()))
all_macros = [m for ms in macro_names.values() for m in ms]
if len(all_macros) != len(set(all_macros)):
    raise SystemExit(f"exported macros collide: {sorted(all_macros)}")
names_by_len = sorted(crates, key=len, reverse=True)
for crate in TEST_ONLY:
    if (crates[crate][0] / "data").is_dir():
        shutil.copytree(crates[crate][0] / "data", vendor / crate / "data")
for crate, (path, data) in crates.items():
    src = path / "src"
    s = short(crate)
    for p in sorted(src.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(root)
        out = vendor / crate / p.relative_to(src)
        out.parent.mkdir(parents=True, exist_ok=True)
        raw = p.read_bytes()
        provenance["files"][str(rel)] = hashlib.sha256(raw).hexdigest()
        if p.suffix != ".rs":
            out.write_bytes(raw)
            continue
        text = raw.decode()
        for old, new, *count in SOURCE_PATCHES.get(str(rel), []):
            if text.count(old) != (count[0] if count else 1):
                raise SystemExit(f"patch of {rel} does not apply: {old[:40]!r}")
            text = text.replace(old, new)
        # Every byte array handed to JS counts against the runtime's memory limit:
        # QuickJS copies it into its own allocation (`ArrayBuffer::new` would keep
        # Rust's allocation, outside the limit).
        text, accounted = re.subn(r"\bArrayBuffer::new\(", "ArrayBuffer::new_copy(", text)
        if accounted:
            rewritten.add(str(rel))
        text = text.replace('env!("CARGO_PKG_VERSION")', json.dumps(data["package"]["version"]))
        # Crate-relative paths (also `$crate::` in exported macros) now start at the module.
        text = re.sub(r"\bcrate::", f"crate::{s}::", text)
        for other in names_by_len:
            text = re.sub(r"(?<![\w:$])" + other + r"\b", f"crate::{short(other)}", text)
        text = text.replace('env!("CARGO_MANIFEST_DIR")', f'concat!(env!("CARGO_MANIFEST_DIR"), "/src/llrt/{crate}")')
        # `use crate::x::{self}` written as `use crate::x;` is fine; `extern crate` is not used.
        local = local_features.get(crate, {})

        defaults = data.get("features", {}).get("default", [])

        def cfg(m):
            f = m[1]
            if crate in TEST_ONLY:
                return "all()" if f in defaults else "any()"
            return f'feature = "{local[f]}"' if f in local else "any()"

        text = re.sub(r'feature\s*=\s*"([^"]+)"', cfg, text)
        if crate in TEST_ONLY:
            text = fold_constant_cfgs(text)
        if p.name == "lib.rs" and p.parent == src and macro_names.get(crate):
            text += "\n// Exported macros live at the package root.\n"
            text += "#[allow(unused_imports)]\npub use crate::{" + ", ".join(macro_names[crate]) + "};\n"
        header = "// Vendored from LLRT " + REVISION[:12] + " (" + str(rel) + "); module paths and\n"
        header += "// feature names adapted by scripts/import-stdlib.py"
        header += ", with local patches.\n" if str(rel) in SOURCE_PATCHES or str(rel) in rewritten else ".\n"
        out.write_text(header + text)

shutil.copy2(root / "LICENSE", dest / "LICENSE-APACHE")
notice = (root / "NOTICE").read_text().rstrip() + "\n"
(dest / "NOTICE").write_text(
    "quickjs-jit-stdlib\n\nThis package redistributes source code of LLRT (https://github.com/awslabs/llrt),\n"
    f"revision {REVISION}, under the Apache License 2.0, with modifications\n"
    "(module paths, feature names, and the patches listed in UPSTREAM.json).\n\n"
    "The NOTICE file of LLRT follows.\n\n" + notice)
provenance["accounted_array_buffers"] = sorted(rewritten)
(dest / "UPSTREAM.json").write_text(json.dumps(provenance, indent=2, sort_keys=True) + "\n")


# --- Manifest -----------------------------------------------------------------------
def value(v):
    if isinstance(v, bool):
        return str(v).lower()
    if isinstance(v, list):
        return "[" + ", ".join(value(x) for x in v) + "]"
    return json.dumps(v)


def entry(name, spec):
    return f"{name} = {{ " + ", ".join(f"{k} = {value(v)}" for k, v in spec.items()) + " }"


def quoted(name):
    return name if re.fullmatch(r"[A-Za-z0-9_-]+", name) else json.dumps(name)


lines = [
    "# Generated by scripts/import-stdlib.py; edit the script, not this file.",
    "[package]", 'name = "quickjs-jit-stdlib"', f'version = "{VERSION}"', 'edition = "2021"',
    'rust-version = "1.89"', 'license = "Apache-2.0"',
    'description = "LLRT\'s standard modules for quickjs-jit, vendored into one crate"',
    'repository = "https://github.com/longbridge/quickjs-jit"', 'readme = "README.md"',
    'include = ["src/**", "tests/**", "README.md", "LICENSE-APACHE", "NOTICE", "UPSTREAM.json", "Cargo.toml"]',
    "", "[package.metadata.docs.rs]", "all-features = false", f"features = {value(PACKAGE_DEFAULT)}",
    "", "[features]", f"default = {value(PACKAGE_DEFAULT)}", 'parallel = ["rquickjs/parallel"]',
]
for name in sorted(features):
    lines.append(f"{quoted(name)} = {value(features[name])}")
lines += ["", "[dependencies]",
          entry("rquickjs", {"package": "quickjs-jit", "version": f"={VERSION}", "path": "..",
                             "default-features": False, "features": rquickjs_features})]
for (target, name), spec in sorted(merged.items(), key=lambda x: (str(x[0][0]), x[0][1])):
    if target is None:
        lines.append(entry(name, {**spec, "default-features": defaults_always(name), "optional": True}))
for target in sorted({t for t, _ in merged if t is not None}):
    lines += ["", f"[target.'{target}'.dependencies]"]
    lines += [entry(n, {**s, "default-features": defaults_always(n), "optional": True})
              for (t, n), s in sorted(merged.items(), key=lambda x: x[0][1]) if t == target]
lines += ["", "[dev-dependencies]"]
lines += [entry(n, s) for n, s in sorted(dev.items()) if n not in ("rquickjs",)]
# The integration tests use the facade, which needs the default modules (workspace
# runs with `--no-default-features` build the package without them).
lines += ["", "[[test]]", 'name = "modules"', f"required-features = {value(PACKAGE_DEFAULT)}"]
lines += ["", "[lints.rust]", 'unexpected_cfgs = { level = "warn", check-cfg = ["cfg(rust_nightly)"] }']
(dest / "Cargo.toml").write_text("\n".join(lines) + "\n")

modules = []
for crate in sorted(runtime, key=short):
    modules += [f'#[cfg(feature = "_{short(crate)}")]',
                "#[allow(dead_code, unused_imports, unused_unsafe, unused_variables, private_interfaces, clippy::all)]",
                f'#[path = "llrt/{crate}/lib.rs"]', f"pub mod {short(crate)};"]
for crate in sorted(TEST_ONLY):
    modules += ["#[cfg(test)]", "#[allow(dead_code, unused_imports, clippy::all)]", f'#[path = "llrt/{crate}/lib.rs"]',
                f"mod {short(crate)};"]
lib = dest / "src" / "lib.rs"
text = lib.read_text()
start, end = "// BEGIN IMPORTED MODULES", "// END IMPORTED MODULES"
prefix, tail = text.split(start, 1)
_, suffix = tail.split(end, 1)
lib.write_text(prefix + start + "\n" + "\n".join(modules) + "\n" + end + suffix)
# The workspace formats every member (CI runs `cargo fmt --all -- --check`).
subprocess.run(["cargo", "fmt", "-p", "quickjs-jit-stdlib"], cwd=dest.parent, check=True)
print(f"Vendored {len(runtime)} LLRT crates and {len(TEST_ONLY)} test helpers into {dest}")
