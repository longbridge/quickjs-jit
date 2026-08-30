# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `8111d666ff2caa5d406051d34d525ccf4f75c296` (dirty: false), QuickJS `ddf43e5e9872538a2c1ef693b84424beb0163195`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`.

Command: `target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output benchmarks/results/task14-comparison.json --report docs/jit-performance.md`. Schema SHA-256 `4aae51171ce8cd996cde9fe280681a91edfe58058ccb75525dd0cc9ef00fba01`; suites lock SHA-256 `0c45036ee8c49342f60a0d10ccf90f9f3df8466a9db6d87285220bb46189f1ae`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 7749707 | 0.03× | 0.03× | 0.03× | 0/0 | 0/0 | `4992502500` |
| numeric (rquickjs-jit) | 1084088 | 0.07× | 0.07× | 0.07× | 83/359 | 300/0 | `1999000` |
| collections (rquickjs-jit) | 2640357 | 0.03× | 0.03× | 0.03× | 0/0 | 0/0 | `1018392` |
| strings-json (rquickjs-jit) | 3331490 | 0.06× | 0.06× | 0.07× | 0/0 | 0/0 | `{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 4465543 | 0.14× | 0.14× | 0.15× | 0/0 | 0/0 | `30872` |
| adversarial (rquickjs-jit) | 1468475 | 0.03× | 0.03× | 0.04× | 0/0 | 0/0 | `7995` |

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 0.05×..0.06×
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([0.1388457037777315, 0.14530695580011033])), ("numeric", Some([0.0922134424837995, 0.10129494477455192]))]
- FAIL/INCONCLUSIVE — **Every strict sample has required native tier**: FAIL: missing per-sample entry
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([3.9263900259669007, 4.261018447851962]), reload=Some([1.0753286961579183, 1.1538867999930817]), p99=Some([17.24515556785233, 18.141673106428023])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, compile, install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler memory; native entry/exit, PC/OSR, helper-exit, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
