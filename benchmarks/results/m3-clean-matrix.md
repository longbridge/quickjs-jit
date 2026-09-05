# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `4d0a35bafa321766c5f4be44fd2258309ac384f5` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `/home/jason/work/quickjs-jit/target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output /tmp/jit-m3-clean-matrix.json --report /tmp/jit-m3-clean-matrix.md`. Schema SHA-256 `01abae6a1b1a5b0492897612693493b19ee011d7e4dfcd7108616012f28ecaab`; suites lock SHA-256 `03b8db038b7633f44046b7912af1d79e38616e20ea431613d0c64344b17e1732`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

Tier1/Tier2 columns require an entry in the requested tier; automatic accepts either native tier. Without that evidence, timing is shown as `N/A (no qualifying native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 6757998 | 2.54× | 1.72× | 1.72× | — | 124/360 | 0/0 | `string:4992502500` |
| quickjs-bitops (rquickjs-jit local) | 1201916 | 2.84× | 5.71× | 5.71× | — | 370/360 | 0/0 | `number:c1c8bb05c4000000` |
| quickjs-fibonacci (rquickjs-jit local) | 936029 | 2.56× | N/A (no qualifying native entry) | 2.49× | — | 360/0 | 0/0 | `number:41d4c00d39400000` |
| numeric (rquickjs-jit) | 635495 | 2.97× | 23.48× | 23.46× | — | 120/360 | 0/0 | `number:413e809800000000` |
| scalar-loop (rquickjs-jit focused) | 626370 | 2.92× | 23.15× | 23.04× | — | 121/360 | 0/0 | `number:413e809800000000` |
| call-heavy (rquickjs-jit focused) | 1396931 | 1.31× | 4.40× | 4.44× | — | 46052/24348 | 0/0 | `number:40bb580000000000` |
| generic-call-entry (rquickjs-jit focused) | 1079480 | 0.09× | N/A (no qualifying native entry) | 0.12× | — | 776468/0 | 0/0 | `number:409f400000000000` |
| property-heavy (rquickjs-jit focused) | 1399874 | 0.13× | 0.06× | 0.06× | — | 62/341 | 0/0 | `number:412e942200000000` |
| fibonacci-iterative (rquickjs-jit focused) | 33627394 | 4.41× | 38.57× | 40.06× | — | 33/346 | 0/0 | `number:419865fb2c000000` |
| fibonacci-recursive (rquickjs-jit focused) | 12071443 | 0.09× | 0.98× | N/A (no qualifying native entry) | — | 11826/360 | 0/0 | `number:40ba6d0000000000` |
| collections (rquickjs-jit) | 1798369 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | — | 0/0 | 0/0 | `string:1018392` |
| strings-json (rquickjs-jit) | 2176841 | 0.20× | N/A (no qualifying native entry) | 0.95× | — | 152/0 | 0/0 | `string:{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 3528477 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | — | 0/0 | 0/0 | `string:30872` |
| adversarial (rquickjs-jit) | 1065007 | 0.56× | N/A (no qualifying native entry) | 1.00× | — | 393/0 | 0/0 | `string:7995` |
| float64-dense (rquickjs-jit matrix) | 2916985 | 1.78× | 1.16× | 8.07× | — | 1080/360 | 0/0 | `number:40a2d29809876024` |
| strings-regexp (rquickjs-jit matrix) | 19216126 | 0.59× | N/A (no qualifying native entry) | 0.97× | — | 120/0 | 0/0 | `string:483:10249:|8@57|9@58|10@59` |
| arrays-typed (rquickjs-jit matrix) | 4597863 | 0.17× | 1.17× | 0.23× | — | 840/665 | 0/0 | `string:33983000:8496750.000:2000` |
| objects-polymorphic (rquickjs-jit matrix) | 6495011 | 0.04× | N/A (no qualifying native entry) | 0.97× | — | 121/0 | 0/0 | `number:413e8c5000000000` |
| calls-recursion-closures (rquickjs-jit matrix) | 7394307 | 0.06× | 0.18× | N/A (no qualifying native entry) | — | 219172/2205755 | 0/0 | `number:40d4820000000000` |
| json-codec (rquickjs-jit matrix) | 79308218 | 0.36× | N/A (no qualifying native entry) | 1.01× | — | 120/0 | 0/0 | `string:2013000:123` |
| map-set-bigint (rquickjs-jit matrix) | 15610783 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | — | 0/0 | 0/0 | `string:2000:256:e2edbb6504d6fce8` |
| exceptions-promises-async (rquickjs-jit matrix) | 1957181 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | — | 0/0 | 0/0 | `string:124000:72576` |

Stripped binary evidence: non-JIT 1721728 bytes; JIT 5833784 bytes; delta +4112056 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 3.17×..3.23×
- PASS — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([1.6925780777654968, 1.7230420843495353])), ("numeric", Some([21.010683361303883, 23.073149999358023])), ("scalar-loop", Some([21.843498075511842, 23.06072876227219])), ("call-heavy", Some([4.366629169758435, 4.422353567849018])), ("property-heavy", Some([0.05486690438084945, 0.0590410159298336])), ("fibonacci-iterative", Some([38.34990073982953, 39.29056656703675]))]
- PASS — **Every strict sample has required native tier**: all samples
- PASS — **Automatic uses production profitability policy**: all samples evaluated
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([2.4660676962693215, 2.545014081651479]), reload=Some([1.1400838808374003, 1.1886414638018348]), p99=Some([0.6503710555762211, 0.7296092905197097])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
