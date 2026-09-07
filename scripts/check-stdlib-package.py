#!/usr/bin/env python3
"""Check that a Cargo archive is one self-contained LLRT redistribution."""
import sys
import tarfile
if sys.version_info < (3, 11):
    raise SystemExit("Python 3.11 or newer is required")
import tomllib

with tarfile.open(sys.argv[1], 'r:gz') as archive:
    names = archive.getnames()
    root = names[0].split('/')[0]
    def read(name):
        return archive.extractfile(f'{root}/{name}').read()
    manifest = tomllib.loads(read('Cargo.toml').decode())
    assert manifest['package']['name'] == 'quickjs-jit-stdlib'
    assert not manifest.get('patch'), 'consumer patches are not a publication strategy'
    sections = [manifest]
    sections.extend(manifest.get('target', {}).values())
    for section in sections:
        for kind in ['dependencies', 'dev-dependencies', 'build-dependencies']:
            for name, spec in section.get(kind, {}).items():
                spec = {'version': spec} if isinstance(spec, str) else spec
                package = spec.get('package', name)
                assert not package.startswith('llrt_'), (kind, package)
                assert not {'git', 'path', 'registry'} & spec.keys(), (kind, package, spec)
                assert spec.get('version'), (kind, package)
    assert manifest['dependencies']['rquickjs']['package'] == 'quickjs-jit'
    for name in names:
        assert not (name.endswith('/Cargo.toml') and name != f'{root}/Cargo.toml'), name
    lock = tomllib.loads(read('Cargo.lock').decode())
    for package in lock['package']:
        assert not package['name'].startswith('llrt_'), package
        assert package['name'] not in {'rquickjs', 'rquickjs-core', 'rquickjs-sys', 'rquickjs-macro'}, package
        assert not package.get('source', '').startswith('git+'), package
    for required in ['LICENSE-APACHE', 'NOTICE', 'NOTICE-LLRT', 'UPSTREAM.json']:
        assert read(required), required
    for module in ['buffer', 'crypto', 'path', 'url', 'zlib']:
        assert read(f'src/llrt/llrt_{module}/lib.rs'), module
print('PASS: one stdlib archive, registry-only dependencies, no LLRT packages or patches')
