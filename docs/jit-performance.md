# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `b098247336f0b5a0e103a1fb87b2d960ee8fcc83` (dirty: true), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`.

Command: `target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output benchmarks/results/task14-comparison.json --report docs/jit-performance.md`. Schema SHA-256 `b651315afdad2f981e697c1a7c482f800326ffc4c8894579c5052052a92eb773`; suites lock SHA-256 `0c45036ee8c49342f60a0d10ccf90f9f3df8466a9db6d87285220bb46189f1ae`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 5479144 | 0.12× | 0.12× | 0.13× | 0/0 | 0/0 | `4992502500` |
| quickjs-bitops (QuickJS microbench) | 1150243 | 0.18× | 0.18× | 0.20× | 0/0 | 0/0 | `-829819784` |
| quickjs-fibonacci (QuickJS microbench) | 795340 | 0.13× | 0.13× | 0.14× | 0/0 | 0/0 | `1392522469` |
| numeric (rquickjs-jit) | 621699 | 0.07× | 0.10× | 0.07× | 75/360 | 300/0 | `1999000` |
| collections (rquickjs-jit) | 1630876 | 0.13× | 0.13× | 0.14× | 0/0 | 0/0 | `1018392` |
| strings-json (rquickjs-jit) | 1907008 | 0.26× | 0.25× | 0.28× | 0/0 | 0/0 | `{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 3191377 | 0.12× | 0.12× | 0.13× | 0/0 | 0/0 | `30872` |
| adversarial (rquickjs-jit) | 887285 | 0.14× | 0.14× | 0.16× | 0/0 | 0/0 | `7995` |

Stripped binary evidence: non-JIT 1702688 bytes; JIT 5409096 bytes; delta +3706408 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 0.15×..0.15×
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([0.12179644749929219, 0.12621314888086782])), ("numeric", Some([0.08182547475960807, 0.0907064277160687]))]
- FAIL/INCONCLUSIVE — **Every strict sample has required native tier**: FAIL: missing per-sample entry
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([4.055209015155123, 4.354200229726842]), reload=Some([1.0787346631985315, 1.163021125143585]), p99=Some([5.511568102991292, 5.939146506105211])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
