# JIT performance report

Status: generated from raw `jit-benchmark-v1` evidence. Source `c9a6df7b6e1a35cd5bbc824d03f8abbd5a43c707`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`.

Sampling: 5 discarded warmup processes, 30 fresh measured processes, 10 one-second throughput windows, 10,000 deterministic bootstrap resamples.

## Workloads

| workload | interpreter median ns | Tier1 speedup | Tier2 speedup | automatic speedup | native T1/T2 | fallback | checksum |
|---|---:|---:|---:|---:|---:|---:|---|
| numeric | 4485479 | 0.07× | 0.28× | 0.30× | 3/71 | 10 | `1999000` |
| collections | 13598210 | 0.15× | 0.15× | 0.17× | 0/0 | 0 | `1018392` |
| strings-json | 16572479 | 0.29× | 0.30× | 0.32× | 0/0 | 0 | `{"length":2000,"first":"abcdefgh"}` |
| calls-closures | 25304675 | 0.13× | 0.13× | 0.14× | 0/0 | 0 | `30872` |
| adversarial | 7713614 | 0.17× | 0.17× | 0.18× | 0/0 | 0 | `7995` |

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute geometric mean ≥5×**: 0.20×
- FAIL/INCONCLUSIVE — **Designated hot kernel ≥10×**: FAIL/INCONCLUSIVE
- PASS — **Native Tier1 and Tier2 evidence**: native entries observed
- PASS — **Checksums identical**: all compared checksums match
- FAIL/INCONCLUSIVE — **gpui-shell steady-state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not yet supplied
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 regression ≤5%**: INCONCLUSIVE: Task 15 worktree evidence not yet supplied

## Break-even and memory

Raw JSON retains per-workload compile/install time, break-even execution, peak RSS, native code, metadata, compiler memory, entry/exit/retry/fallback counters, and every latency/throughput sample. A zero or null field means the runtime did not expose that measurement; it is not estimated. Binary size is provenance metadata and is kept separate from RSS.

## Exclusions

- SunSpider/JetStream / external corpora: not imported; suites.lock records explicit status

This report never converts missing evidence into a pass. Failed targets remain visible and must not be hidden by fallback or workload removal.
