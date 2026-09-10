"""Validate and summarize paired.py evidence; speed is reference time / time."""
import argparse
import json
import math
from pathlib import Path
import random
import statistics

p=argparse.ArgumentParser()
p.add_argument('directory')
args=p.parse_args()
root=Path(args.directory)
metadata=json.loads((root/'metadata.json').read_text())
if metadata.get('status') != 'complete':
    raise RuntimeError('Collector has not completed; partial samples are not an acceptance result')
randomizer=random.Random(20260909)
rows=[]
def interval(values):
    logs=[math.log(v) for v in values]
    bootstrap=sorted(math.exp(statistics.mean(randomizer.choices(logs,k=len(logs)))) for _ in range(10000))
    return math.exp(statistics.mean(logs)),[bootstrap[249],bootstrap[9749]]
for workload in metadata['scripts']:
    data={}
    checksums=set()
    drivers=set()
    for variant,mode in metadata['configs']:
        paths=[root/f'{workload}-{i}-{variant}-{mode}.json' for i in range(metadata['pairs'])]
        if not all(p.exists() for p in paths):
            raise RuntimeError(f'Incomplete evidence: {workload}/{variant}/{mode}')
        data[variant,mode]=[json.loads(p.read_text()) for p in paths]
        for i in range(-metadata['discarded_processes'], metadata['pairs']):
            sample=json.loads((root/f'{workload}-{i}-{variant}-{mode}.json').read_text())
            assert (sample['pair_index'],sample['variant'],sample['requested_mode']) == (i,variant,mode)
            protocol=sample['protocol']
            assert protocol['name'] == metadata['protocol']
            assert protocol['script_sha256'] == metadata['scripts'][workload]
            assert protocol['calls_per_batch'] == 10
            assert protocol['warmup_batches'] == len(protocol['warmup_batch_ns']) == 64
            assert sample['elapsed_ns'] > 0
            checksums.add(sample['checksum'])
            drivers.add(protocol['driver_sha256'])
        if len(checksums) != 1 or len(drivers) != 1:
            raise RuntimeError(f'Engine inputs or checksums differ: {workload}')
    else:
        baseline=next(iter(metadata['binaries']))
        for variant,mode in metadata['configs']:
            values=data[variant,mode]
            comparisons={}
            for reference in [(baseline,'automatic'),(baseline,'interpreter'),(baseline,'bun')] + ([('candidate','interpreter')] if ('candidate','interpreter') in data and baseline != 'candidate' else []):
                speed,ci=interval([a['elapsed_ns']/b['elapsed_ns'] for a,b in zip(data[reference],values)])
                comparisons['/'.join(reference)]=dict(speed=speed,ci95=ci)
            deltas=[v.get('fixed_deltas') for v in values]
            rows.append(dict(workload=workload,variant=variant,mode=mode,
                median_ns=statistics.median(v['elapsed_ns'] for v in values),
                comparisons=comparisons,
                compilation_quiet_samples=sum(v.get('fixed_compilation_quiet',False) for v in values),
                native_entry_range=([min(d['native_entries'] for d in deltas),max(d['native_entries'] for d in deltas)] if all(d is not None for d in deltas) else None)))
(root/'summary.json').write_text(json.dumps(rows,indent=2)+'\n')
for row in rows:
    old=row['comparisons'][next(iter(metadata['binaries']))+'/automatic']
    bun=row['comparisons'][next(iter(metadata['binaries']))+'/bun']
    print(f"{row['workload']:20} {row['variant']:10}/{row['mode']:11} {row['median_ns']/1e6:9.4f} ms; baseline {old['speed']:.3f}x {old['ci95']}; Bun {bun['speed']:.4f}x; quiet {row['compilation_quiet_samples']}/{metadata['pairs']}; entries {row['native_entry_range']}")
