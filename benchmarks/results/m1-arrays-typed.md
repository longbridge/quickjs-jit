# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `63dc71666aa564bd11429709f8e1a255719b1777` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `./target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output benchmarks/results/m1-arrays-typed.json --report benchmarks/results/m1-arrays-typed.md`. Schema SHA-256 `ae70459701c9799fdd367fe3b720ae2fab457f4dab700e54498ad1f27a13c82c`; suites lock SHA-256 `cfa0056fe9ebd94e16b64a9d74b1f94d3f5570d6c5d6388c4126f3ccb8980be3`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

A JIT ratio is reported only when that mode actually entered native code; fallback-only timing is shown as `N/A (no native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| arrays-typed (rquickjs-jit matrix) | 4654366 | 0.24× | 1.38× | 0.60× | — | 927/687 | 0/0 | `string:33983000:8496750.000:2000` |

Stripped binary evidence: non-JIT 1716448 bytes; JIT 5734600 bytes; delta +4018152 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: INCONCLUSIVE
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: []
- PASS — **Every strict sample has required native tier**: all samples
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([1.7934970220707684, 2.1592576218967166]), reload=Some([1.027930205959548, 1.2204582380766065]), p99=Some([1.3579201071988791, 1.9208872413572584])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
