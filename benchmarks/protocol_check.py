"""Shared worker-protocol validation for paired.py and summarize_paired.py.

`shared-js-fixed-warmup-v2` timed one batch after 64 warmups. It is kept
readable so historical evidence can still be summarized, but it must never be
mixed with `shared-js-multibatch-v3`, where `elapsed_ns` is the upper median
of `timed_batches` consecutive timed batches.
"""
import os

PROTOCOL_V2 = 'shared-js-fixed-warmup-v2'
PROTOCOL_V3 = 'shared-js-multibatch-v3'
WARMUP_BATCHES = 64
CALLS_PER_BATCH = 10
OUTLIER_RATIO = 5
DEFAULT_TIMED_BATCHES = 16


def timed_batches_from_env():
    raw = os.environ.get('JIT_BENCH_TIMED_BATCHES')
    if raw is None:
        return DEFAULT_TIMED_BATCHES
    if not raw.isdigit() or int(raw) <= 0:
        raise ValueError('JIT_BENCH_TIMED_BATCHES must be a positive integer')
    return int(raw)


def upper_median(values):
    ordered = sorted(values)
    return ordered[-(-(len(ordered) - 1) // 2)] if ordered else 0


def timed_summary(batches):
    median = upper_median(batches)
    return median, sum(ns > median * OUTLIER_RATIO for ns in batches)


def validate_protocol(sample, metadata, workload, mode):
    """Reject incompatible or internally inconsistent worker evidence."""
    protocol = sample['protocol']
    name = protocol['name']
    if name not in (PROTOCOL_V2, PROTOCOL_V3):
        raise AssertionError(f'unsupported timing protocol {name!r}')
    if name != metadata['protocol']:
        raise AssertionError(f'protocol {name!r} mixed into {metadata["protocol"]!r} evidence')
    assert protocol['script_sha256'] == metadata['scripts'][workload]
    assert protocol['calls_per_batch'] == CALLS_PER_BATCH
    assert protocol['warmup_batches'] == len(protocol['warmup_batch_ns']) == WARMUP_BATCHES
    assert sample['elapsed_ns'] > 0
    if name == PROTOCOL_V2:
        return
    timed = protocol['timed_batch_ns']
    expected = metadata.get('timed_batches', DEFAULT_TIMED_BATCHES)
    assert protocol['timed_batches'] == len(timed) == expected, (protocol['timed_batches'], len(timed), expected)
    assert protocol['fixed_batch_ns'] == timed[0]
    assert protocol['outlier_ratio'] == OUTLIER_RATIO
    assert (sample['elapsed_ns'], protocol['outlier_batches']) == timed_summary(timed)
    if mode == 'bun':
        assert protocol['bun_launch'] == 'file', 'publishable Bun evidence must use the file wrapper'
        assert len(protocol['bun_wrapper_sha256']) == 64


if __name__ == '__main__':
    # Self-test: python3 benchmarks/protocol_check.py
    assert upper_median([1, 2, 3, 4]) == 3 and upper_median([1, 2, 3]) == 2
    assert timed_summary([5] * 15 + [187]) == (5, 1)
    assert timed_summary([5] * 15 + [25]) == (5, 0)
    print('protocol_check self-test ok')
