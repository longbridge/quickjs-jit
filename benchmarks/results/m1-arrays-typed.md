# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `fcc1a1db823beb923e9f0262faeb05e7b40077b5` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `./target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output benchmarks/results/m1-arrays-typed.json --report benchmarks/results/m1-arrays-typed.md`. Schema SHA-256 `ae70459701c9799fdd367fe3b720ae2fab457f4dab700e54498ad1f27a13c82c`; suites lock SHA-256 `cfa0056fe9ebd94e16b64a9d74b1f94d3f5570d6c5d6388c4126f3ccb8980be3`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

A JIT ratio is reported only when that mode actually entered native code; fallback-only timing is shown as `N/A (no native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| arrays-typed (rquickjs-jit matrix) | 4885385 | 0.26× | 1.45× | 0.63× | — | 939/687 | 0/0 | `string:33983000:8496750.000:2000` |

Stripped binary evidence: non-JIT 1716448 bytes; JIT 5734600 bytes; delta +4018152 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: INCONCLUSIVE
- FAIL/INCONCLUSIVE — **At least one designated kernel lower CI ≥10×**: []
- PASS — **Every strict sample has required native tier**: all samples
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([1.5735333520276038, 1.9735796433736166]), reload=Some([0.9609864916079588, 1.1762279225149517]), p99=Some([1.1472065909007318, 1.326419267394972])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
