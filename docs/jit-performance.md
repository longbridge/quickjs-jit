# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `39115d29825a6796baf89b92ae64bdc623e341cb` (dirty: false), QuickJS `ddf43e5e9872538a2c1ef693b84424beb0163195`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`.

Command: `target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output benchmarks/results/task14-comparison.json --report docs/jit-performance.md`. Schema SHA-256 `4aae51171ce8cd996cde9fe280681a91edfe58058ccb75525dd0cc9ef00fba01`; suites lock SHA-256 `0c45036ee8c49342f60a0d10ccf90f9f3df8466a9db6d87285220bb46189f1ae`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 5599725 | 0.12× | 0.12× | 0.14× | 0/0 | 0/0 | `4992502500` |
| numeric (rquickjs-jit) | 650300 | 0.07× | 0.10× | 0.08× | 74/359 | 300/0 | `1999000` |
| collections (rquickjs-jit) | 1704114 | 0.14× | 0.14× | 0.16× | 0/0 | 0/0 | `1018392` |
| strings-json (rquickjs-jit) | 1923093 | 0.26× | 0.26× | 0.28× | 0/0 | 0/0 | `{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 3105706 | 0.12× | 0.12× | 0.13× | 0/0 | 0/0 | `30872` |
| adversarial (rquickjs-jit) | 892872 | 0.14× | 0.14× | 0.16× | 0/0 | 0/0 | `7995` |

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 0.14×..0.15×
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([0.11560755277183496, 0.1253486972412485])), ("numeric", Some([0.07898171085948481, 0.08929660102920169]))]
- FAIL/INCONCLUSIVE — **Every strict sample has required native tier**: FAIL: missing per-sample entry
- PASS — **Automatic uses production profitability policy**: all samples evaluated
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([4.243997944799534, 4.541897310164966]), reload=Some([1.0272354527938348, 1.1269212045760253]), p99=Some([5.695681706778523, 6.112147377886635])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, compile, install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler memory; native entry/exit, PC/OSR, helper-exit, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
