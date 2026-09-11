"""Interleave immutable runtime variants with the same benchmark harness/Bun.

Adapted from the archived candidate4 experiments/paired.py. Omit --cpu on
macOS; explicitly pass a CPU on Linux when reproducing pinned measurements.
All discarded and retained processes are saved. Do not overlap collection
with builds or another benchmark. Forced tiers are optional diagnostics.
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--binaries', required=True, help='JSON map from variant to binary')
p.add_argument('--scripts', required=True)
p.add_argument('--workloads', required=True)
p.add_argument('--output', required=True)
p.add_argument('--cpu', help='Linux taskset CPU; omitted means no affinity')
p.add_argument('--pairs', type=int, default=30)
p.add_argument('--diagnostic-tiers', action='store_true')
args = p.parse_args()
if args.pairs < 1:
    p.error('--pairs must be positive')
if args.cpu is not None and platform.system() != 'Linux':
    p.error('--cpu requires Linux taskset')
import fcntl
collection_lock = (Path(__file__).parent / 'sampling.lock').open('a')
fcntl.flock(collection_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
bins = json.loads(Path(args.binaries).read_text())
out = Path(args.output)
out.mkdir(parents=True, exist_ok=False)
configs = [(name, 'automatic') for name in bins]
configs += [(next(iter(bins)), 'interpreter'), (next(iter(bins)), 'bun')]
if 'candidate' in bins and next(iter(bins)) != 'candidate':
    configs += [('candidate', 'interpreter')]
if args.diagnostic_tiers:
    configs += [(next(iter(bins)), 'tier1'), (next(iter(bins)), 'tier2')]
metadata = dict(cpu=args.cpu, status='collecting', pairs=args.pairs, discarded_processes=5,
    binary_sha256={k: hashlib.sha256(Path(v).read_bytes()).hexdigest() for k,v in bins.items()},
    binaries=bins, configs=configs, scripts={}, command=list(os.sys.argv))
import shutil
bun = Path(shutil.which(os.environ.get('JIT_BENCH_BUN', 'bun'))).resolve()
governor_path = Path(f'/sys/devices/system/cpu/cpu{args.cpu}/cpufreq/scaling_governor')
cpu_command = ['sysctl', '-n', 'machdep.cpu.brand_string'] if platform.system() == 'Darwin' else ['lscpu']
metadata.update(collector_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    cpu_description=subprocess.check_output(cpu_command,text=True).strip(),
    governor=governor_path.read_text().strip() if governor_path.is_file() else 'not recorded',
    platform=platform.platform(), bun_path=str(bun),
    bun_version=subprocess.check_output([str(bun), '--version'],text=True).strip(),
    bun_sha256=hashlib.sha256(bun.read_bytes()).hexdigest(), bun_flags=[],
    rustc=subprocess.check_output(['rustc','-Vv'],text=True),
    protocol='shared-js-fixed-warmup-v2', throughput_windows=0,
    note='Latency-only paired candidate diagnostic. Fixed 10-call window; counters include shared driver. Readiness diagnostic is separately conditioned.')
(out / 'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
for workload in args.workloads.split(','):
    script = Path(args.scripts) / (workload + '.js')
    metadata['scripts'][workload] = hashlib.sha256(script.read_bytes()).hexdigest()
    for pair in range(-5, args.pairs):
        order = list(configs)
        if pair % 2:
            order.reverse()
        for variant, mode in order:
            target = out / f'{workload}-{pair}-{variant}-{mode}.json'
            if target.exists():
                raise RuntimeError(f'Refusing to overwrite existing evidence: {target}')
            command = [bins[variant], 'worker', '--mode', mode, '--script', str(script)]
            if args.cpu is not None:
                command = ['taskset', '-c', args.cpu] + command
            started = time.monotonic()
            result = subprocess.run(command, capture_output=True, text=True, timeout=180)
            if result.returncode:
                (out / 'failure.json').write_text(json.dumps(dict(command=command, stdout=result.stdout, stderr=result.stderr),indent=2))
                raise RuntimeError(f'Worker failed: {command}: {result.stderr}')
            value = json.loads(result.stdout)
            protocol = value['protocol']
            assert protocol['name'] == metadata['protocol']
            assert protocol['script_sha256'] == metadata['scripts'][workload]
            assert protocol['warmup_batches'] == len(protocol['warmup_batch_ns']) == 64
            assert protocol['calls_per_batch'] == 10
            assert value['elapsed_ns'] > 0
            if mode != 'bun':
                assert value['native_exits'] == value['native_entries']
            before, after = protocol['fixed_metrics_before'], protocol['fixed_metrics_after']
            if before is not None:
                cumulative = ['native_entries','native_exits','tier2_entries','deopts','native_fallbacks','installed','compile_ns','install_ns']
                value['fixed_deltas'] = {k:after[k]-before[k] for k in cumulative}
                assert all(v >= 0 for v in value['fixed_deltas'].values())
                assert value['fixed_deltas']['native_entries'] == value['fixed_deltas']['native_exits']
                value['fixed_compilation_quiet'] = (after['pending_worker_jobs'] == before['pending_worker_jobs'] == 0 and after['pending_snapshot_bytes'] == before['pending_snapshot_bytes'] == 0 and value['fixed_deltas']['installed'] == 0 and value['fixed_deltas']['compile_ns'] == 0)
            value.update(variant=variant, requested_mode=mode, pair_index=pair, process_wall_seconds=time.monotonic()-started)
            target.write_text(json.dumps(value,indent=2)+'\n')
        print(f'{workload}: pair {pair} complete',flush=True)
    values = [json.loads(path.read_text()) for path in out.glob(f'{workload}-*.json')]
    assert len({v['checksum'] for v in values}) == 1, workload
    assert len({v['protocol']['driver_sha256'] for v in values}) == 1, workload
    (out / 'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
for variant, binary in bins.items():
    assert hashlib.sha256(Path(binary).read_bytes()).hexdigest() == metadata['binary_sha256'][variant]
assert hashlib.sha256(bun.read_bytes()).hexdigest() == metadata['bun_sha256']
metadata['status'] = 'complete'
(out / 'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
