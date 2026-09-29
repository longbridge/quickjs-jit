# P0 benchmark protocol: `shared-js-multibatch-v3`

Branch `perf/p0-bench-protocol`, based on `82d3808`. This is a harness-only
change: the runtime, patches, ABI, and README matrix are unchanged.

## What changed

- Every engine (interpreter, Tier 1, forced Tier 2, automatic, Bun) runs one
  first call, 64 warmup batches, then `K` consecutive timed batches of ten calls
  (`K = 16`, `JIT_BENCH_TIMED_BATCHES` overrides it). `elapsed_ns` is the upper
  median of the K batches. All raw values are kept in `protocol.timed_batch_ns`.
- `protocol.fixed_batch_ns` keeps the old v2 single-batch value (the first timed
  batch) as a diagnostic.
- `protocol.outlier_batches` counts timed batches above 5x the median
  (`protocol.outlier_ratio`). Outliers are never dropped.
- Bun runs the wrapper from a temporary `.mjs` file (`protocol.bun_launch =
  "file"`, `protocol.bun_wrapper_sha256`). `JIT_BENCH_BUN_LAUNCH=eval` reproduces
  `bun -e` as a control only, and `paired.py` rejects it.
- QuickJS records `timed_metrics_before/after` around the whole timed window, in
  addition to the v2 `fixed_metrics_before/after` around the first timed batch.
- `jit-bench-report` still rejects all protocol-tagged samples (v2 and v3) and
  names the protocol in the error. `paired.py` and `summarize_paired.py` share
  `benchmarks/protocol_check.py`, which validates v3 invariants, refuses mixed
  v2/v3 evidence, and reports outlier counts and the fixed-batch diagnostic.

## Noisy diagnostic (loaded 24-core host, 7 alternating processes)

Bun, `numeric`: the v2 fixed batch was 0.129 ms per ten calls. The v3 median is
0.0045 ms, and the fixed batch under the file launch is also about 0.005 ms.
Under `-e` (control), batches 64 and 68 still stall for 0.13–0.40 ms and are
flagged as outliers, while the median stays at about 0.005 ms.

## Open issue for the integrator: JSC reaches its top tier later under the file launch

The raw sequences show that Bun reaches its fastest tier later when started from
a file than when started with `-e`. The wrapper text is identical, and
`bun build --no-bundle` shows no rewrite. Per-process medians, in µs per ten
calls:

| workload | launch | batches 48–63 | 64–79 (v3 window) | 144+ |
| --- | --- | ---: | ---: | ---: |
| numeric | file | 4.6–11.4 | 4.5–6.3 | 4.2–10.0 |
| numeric | eval | 4.6 | 4.6 | 4.1 |
| call-heavy | file | 16.7–19.0 | 6.3–18.4 | 6.1 |
| call-heavy | eval | 6.3–14.2 | 6.3 | 6.1 |
| scalar-control-flow | file | 20.9–23.8 | 10.4–20.6 | 9.9–19.8 |
| scalar-control-flow | eval | 10.3–10.4 | 10.1–10.2 | 10.0 |
| host-compute | file | 20.9–27.1 | 9.8–20.6 | 9.2–19.4 |
| host-compute | eval | 16.1–20.2 | 9.6–19.5 | 9.3–18.9 |

With a fixed 64-batch warmup under the file launch, the v3 window can therefore
measure some Bun processes before JSC's top tier lands. For these workloads, v3
Bun latency can be up to about 3x slower than Bun's eventual steady state. This
is the reverse of the v2 bias (which made Bun look up to about 30x slower on a
single batch), and much smaller. It is the same fixed warmup policy for every
engine, so the comparison remains equal-policy. Still, it is not a
"fully settled Bun" number. Some processes also settle into two different
steady states (host-compute: 9.2 µs or about 19 µs in both launch modes).

Options before publishing the S0 matrix (not implemented here):

1. Keep v3 as is, and report `outlier_batches` together with each engine's
   `timed_batch_ns` spread.
2. Add a longer common warmup for every engine (for example 256 batches),
   which needs a new protocol name because `warmup_batches` is pinned to 64
   in `protocol_check.py` and the schema.
3. Add a settle-based warmup rule, applied identically to all engines.

## Remaining P0 work (integrator)

- Re-run the full 30-scenario three-engine matrix under v3 (`paired.py` or
  `jit-bench compare`) to establish baseline `S0`. Replace the README numbers and
  label the old matrix as historical data affected by the Bun fixed-batch stall.
- Upgrade roadmap section 3 to a paired table with confidence intervals.
- Decide on the JSC late-tier-up issue above.
