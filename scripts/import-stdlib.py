#!/usr/bin/env python3
"""Import the pinned LLRT runtime closure into one distributable Rust crate.

Usage: python3 scripts/import-stdlib.py /path/to/llrt
The source checkout must match REVISION. This script never downloads code.
"""
import copy
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

REVISION = '7b95c82a9b15e7ddfb2778eca4b5a63111e74f51'
ROOTS = {'llrt_buffer': [], 'llrt_crypto': ['crypto-rust'], 'llrt_path': [],
         'llrt_url': [], 'llrt_zlib': ['compression-rust']}
root = Path(sys.argv[1]).resolve()
if subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() != REVISION:
    raise SystemExit('LLRT checkout must be at ' + REVISION)
subprocess.run(['git', '-C', str(root), 'diff', '--exit-code', 'HEAD', '--', 'libs', 'modules'], check=True, stdout=subprocess.DEVNULL)
dest = Path(__file__).resolve().parents[1] / 'stdlib'
crates = {}
for p in root.glob('*/*/Cargo.toml'):
    data = tomllib.loads(p.read_text())
    if 'package' in data:
        crates[data['package']['name']] = (p.parent, data)

def dependencies(data):
    result = [(None, k, v) for k, v in data.get('dependencies', {}).items()]
    for target, section in data.get('target', {}).items():
        result += [(target, k, v) for k, v in section.get('dependencies', {}).items()]
    return [(t, k, {'version': v} if isinstance(v, str) else v) for t, k, v in result]

features = {name: set(fs) for name, fs in ROOTS.items()}
optional = {}
extra = {}
for name in ROOTS:
    if name != 'llrt_zlib': features[name].add('default')
changed = True
while changed:
    before = repr((features, optional, extra))
    for name, enabled in list(features.items()):
        data = crates[name][1]
        optional.setdefault(name, set())
        extra.setdefault(name, {})
        for feature in list(enabled):
            for child in data.get('features', {}).get(feature, []):
                if '/' in child:
                    dep, feat = child.split('/', 1)
                    conditional = dep.endswith('?')
                    dep = dep.rstrip('?')
                    if conditional and dep not in optional[name]: continue
                    optional[name].add(dep)
                    extra[name].setdefault(dep, set()).add(feat)
                elif child.startswith('dep:'):
                    optional[name].add(child[4:])
                elif child in data.get('features', {}):
                    enabled.add(child)
                else:
                    optional[name].add(child)
        for _, dep, spec in dependencies(data):
            if spec.get('optional') and dep not in optional[name]: continue
            if dep.startswith('llrt_'):
                fs = features.setdefault(dep, set())
                fs.update(spec.get('features', []))
                fs.update(extra[name].get(dep, set()))
                if spec.get('default-features', True): fs.add('default')
    changed = before != repr((features, optional, extra))

# Merge only selected external runtime dependencies. Optional backend selection
# is resolved during import: this package ships Shell's existing backend choices.
merged = {}
for name in features:
    for target, dep, spec in dependencies(crates[name][1]):
        if dep.startswith('llrt_'): continue
        if spec.get('optional') and dep not in optional[name]: continue
        spec = copy.deepcopy(spec)
        spec.pop('optional', None)
        spec.pop('path', None)
        spec['features'] = sorted(set(spec.get('features', [])) | extra[name].get(dep, set()))
        key = (target, dep)
        if key in merged:
            prior = merged[key]
            if prior['version'] != spec['version'] and dep != 'rquickjs':
                raise SystemExit(f'incompatible versions for {dep}: {prior} / {spec}')
            prior['features'] = sorted(set(prior['features']) | set(spec['features']))
            prior['default-features'] = prior.get('default-features', True) or spec.get('default-features', True)
        else: merged[key] = spec
binding = merged[(None, 'rquickjs')]
binding.update(package='quickjs-jit', version='=0.12.7', path='..')
binding['features'] = sorted(set(binding['features']) | {'std', 'loader'})

vendor = dest / 'src' / 'llrt'
if vendor.exists(): shutil.rmtree(vendor)
vendor.mkdir(parents=True)
provenance = {'repository': 'https://github.com/awslabs/llrt', 'revision': REVISION,
              'roots': ROOTS, 'features': {n: sorted(f) for n, f in sorted(features.items())}, 'files': {}}
# Keep the upstream unit tests and their helper in the same crate.
names = sorted(features) + ['llrt_test']
macro_names = {}
for name in names:
    src = crates[name][0] / 'src'
    macro_names[name] = []
    for p in src.rglob('*.rs'):
        text = p.read_text()
        macro_names[name] += re.findall(r'#\[macro_export\]\s*macro_rules!\s+(\w+)', text)
for name in names:
    src = crates[name][0] / 'src'
    for p in src.rglob('*'):
        if not p.is_file(): continue
        output = vendor / name / p.relative_to(src)
        output.parent.mkdir(parents=True, exist_ok=True)
        raw = p.read_bytes()
        provenance['files'][str(p.relative_to(root))] = hashlib.sha256(raw).hexdigest()
        if p.suffix != '.rs': output.write_bytes(raw); continue
        text = raw.decode()
        text = text.replace('env!("CARGO_PKG_VERSION")', json.dumps(crates[name][1]['package']['version']))
        # Former crate-local paths are now local to the imported module.
        text = re.sub(r'\bcrate::', f'crate::{name}::', text)
        for other in names:
            text = re.sub(r'(?<![\w:])' + other + r'::', f'crate::{other}::', text)
        # Feature selection is frozen to the documented import configuration.
        text = re.sub(r'feature\s*=\s*"([^"]+)"',
                      lambda m: 'all()' if m[1] in features.get(name, set()) or m[1] in optional.get(name, set()) else 'any()', text)
        if p.name == 'lib.rs' and macro_names[name]:
            text += '\n// Macro exports move to the combined crate root.\n'
            text += 'pub(crate) use crate::{' + ', '.join(macro_names[name]) + '};\n'
        text = '// Redistributed from LLRT; module paths and backend cfgs adapted by scripts/import-stdlib.py.\n' + text
        output.write_text(text)

shutil.copy2(root / 'LICENSE', dest / 'LICENSE-APACHE')
(dest / 'NOTICE-LLRT').write_text('\n'.join(line.rstrip() for line in (root / 'NOTICE').read_text().splitlines()) + '\n')
(dest / 'UPSTREAM.json').write_text(json.dumps(provenance, indent=2, sort_keys=True) + '\n')

def value(v):
    if isinstance(v, bool): return str(v).lower()
    if isinstance(v, list): return '[' + ', '.join(value(x) for x in v) + ']'
    return json.dumps(v)
def entry(name, spec):
    return name + ' = { ' + ', '.join(k + ' = ' + value(v) for k, v in spec.items() if k != 'features' or v) + ' }'
lines = ['[package]', 'name = "quickjs-jit-stdlib"', 'version = "0.12.7"',
         'edition = "2021"', 'rust-version = "1.89"', 'license = "Apache-2.0"',
         'description = "LLRT-derived standard modules for quickjs-jit in one distributable crate"',
         'repository = "https://github.com/longbridge/quickjs-jit"', 'readme = "README.md"',
         'include = ["src/**", "tests/**", "README.md", "LICENSE-APACHE", "NOTICE", "NOTICE-LLRT", "UPSTREAM.json", "Cargo.toml"]',
         '', '[features]', 'parallel = ["rquickjs/parallel"]', '', '[dependencies]']
for (target, name), spec in sorted(merged.items(), key=lambda x: str(x[0])):
    if target is None: lines.append(entry(name, spec))
for target in sorted({t for t, _ in merged if t is not None}):
    lines += ['', "[target.'" + target + "'.dependencies]"]
    lines += [entry(n, s) for (t, n), s in sorted(merged.items(), key=lambda x: str(x[0])) if t == target]
lines += ['', '[dev-dependencies]', 'tokio = { version = "1", features = ["full", "test-util"] }',
          'uuid = { version = "1", features = ["v4"] }', '',
          '[lints.rust]', 'unexpected_cfgs = { level = "warn", check-cfg = ["cfg(rust_nightly)"] }']
(dest / 'Cargo.toml').write_text('\n'.join(lines) + '\n')
modules = []
for name in sorted(features):
    modules += ['#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]', '#[path = "llrt/' + name + '/lib.rs"]', 'mod ' + name + ';']
modules += ['#[cfg(test)]', '#[allow(dead_code)]', '#[path = "llrt/llrt_test/lib.rs"]', 'mod llrt_test;']
lib = dest / 'src' / 'lib.rs'
text = lib.read_text()
start, end = '// BEGIN IMPORTED MODULES', '// END IMPORTED MODULES'
prefix, tail = text.split(start, 1)
_, suffix = tail.split(end, 1)
lib.write_text(prefix + start + '\n' + '\n'.join(modules) + '\n' + end + suffix)
subprocess.run(['cargo', 'fmt', '-p', 'quickjs-jit-stdlib'], cwd=dest.parent, check=True)
print(f'Imported {len(features)} runtime modules and one test helper into {dest}')
