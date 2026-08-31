# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `9e7e48686659869abc56096470892aaf5a18f589` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `./target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic,bun --output benchmarks/results/broad-5mode.json --report benchmarks/results/broad-5mode.md`. Schema SHA-256 `ae70459701c9799fdd367fe3b720ae2fab457f4dab700e54498ad1f27a13c82c`; suites lock SHA-256 `cfa0056fe9ebd94e16b64a9d74b1f94d3f5570d6c5d6388c4126f3ccb8980be3`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

A JIT ratio is reported only when that mode actually entered native code; fallback-only timing is shown as `N/A (no native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| float64-dense (rquickjs-jit matrix) | 2828805 | 1.39× | 8.76× | 7.30× | 6.12× | 171/360 | 0/0 | `number:40a2d29809876024` |
| strings-regexp (rquickjs-jit matrix) | 19379043 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 7.96× | 0/0 | 0/0 | `string:483:10249:|8@57|9@58|10@59` |
| arrays-typed (rquickjs-jit matrix) | 4676664 | 0.36× | N/A (no native entry) | 0.52× | 6.07× | 99/0 | 0/0 | `string:33983000:8496750.000:2000` |
| objects-polymorphic (rquickjs-jit matrix) | 6687248 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 8.07× | 0/0 | 0/0 | `number:413e8c5000000000` |
| calls-recursion-closures (rquickjs-jit matrix) | 7389416 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 4.63× | 0/0 | 0/0 | `number:40d4820000000000` |
| json-codec (rquickjs-jit matrix) | 79052892 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 8.94× | 0/0 | 0/0 | `string:2013000:123` |
| map-set-bigint (rquickjs-jit matrix) | 15308001 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 7.05× | 0/0 | 0/0 | `string:2000:256:e2edbb6504d6fce8` |
| exceptions-promises-async (rquickjs-jit matrix) | 2482627 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | 3.35× | 0/0 | 0/0 | `string:124000:72576` |

Stripped binary evidence: non-JIT 1715488 bytes; JIT 5688504 bytes; delta +3973016 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: INCONCLUSIVE
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: []
- PASS — **Every strict sample has required native tier**: all samples
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([1.4885431099246005, 1.6075640646242761]), reload=Some([0.9957780523708666, 1.0665108300832447]), p99=Some([0.7529494025720349, 0.8724170761092236])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
