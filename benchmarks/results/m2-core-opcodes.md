# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `7ce069fff24c84cc1beacbcedd6fbf3d03cffb64` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `./target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic --output target/bench/m2-core-opcodes.json --report target/bench/m2-core-opcodes.md`. Schema SHA-256 `ae70459701c9799fdd367fe3b720ae2fab457f4dab700e54498ad1f27a13c82c`; suites lock SHA-256 `cfa0056fe9ebd94e16b64a9d74b1f94d3f5570d6c5d6388c4126f3ccb8980be3`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

A JIT ratio is reported only when that mode actually entered native code; fallback-only timing is shown as `N/A (no native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 6830704 | 2.58× | 1.63× | 1.76× | — | 279/340 | 0/0 | `string:4992502500` |
| quickjs-bitops (rquickjs-jit local) | 1335396 | 3.05× | 6.38× | 6.39× | — | 331/352 | 0/0 | `number:c1c8bb05c4000000` |
| quickjs-fibonacci (rquickjs-jit local) | 1313867 | 3.56× | N/A (no native entry) | 3.55× | — | 360/0 | 0/0 | `number:41d4c00d39400000` |
| numeric (rquickjs-jit) | 796807 | 3.70× | 29.38× | 29.20× | — | 119/351 | 0/0 | `number:413e809800000000` |
| scalar-loop (rquickjs-jit focused) | 798609 | 3.74× | 29.98× | 29.42× | — | 118/343 | 0/0 | `number:413e809800000000` |
| call-heavy (rquickjs-jit focused) | 1892590 | 1.76× | 5.97× | 6.05× | — | 11700/113968 | 0/0 | `number:40bb580000000000` |
| property-heavy (rquickjs-jit focused) | 1443539 | 0.13× | 0.06× | 0.06× | — | 89/333 | 0/0 | `number:412e942200000000` |
| fibonacci-iterative (rquickjs-jit focused) | 36129513 | 4.78× | 40.89× | 41.05× | — | 54/332 | 0/0 | `number:419865fb2c000000` |
| fibonacci-recursive (rquickjs-jit focused) | 12019176 | 0.09× | 0.99× | N/A (no native entry) | — | 15199/330 | 0/0 | `number:40ba6d0000000000` |
| collections (rquickjs-jit) | 2065018 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | — | 0/0 | 0/0 | `string:1018392` |
| strings-json (rquickjs-jit) | 3019679 | 0.28× | N/A (no native entry) | 1.28× | — | 120/0 | 0/0 | `string:{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 3560698 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | — | 0/0 | 0/0 | `string:30872` |
| adversarial (rquickjs-jit) | 1123329 | 0.57× | N/A (no native entry) | 1.08× | — | 132/0 | 0/0 | `string:7995` |
| float64-dense (rquickjs-jit matrix) | 3886800 | 2.39× | 2.38× | 10.75× | — | 1554/360 | 0/0 | `number:40a2d29809876024` |
| strings-regexp (rquickjs-jit matrix) | 19343102 | 0.60× | N/A (no native entry) | 0.98× | — | 120/0 | 0/0 | `string:483:10249:|8@57|9@58|10@59` |
| arrays-typed (rquickjs-jit matrix) | 5184735 | 0.39× | 1.34× | 1.90× | — | 988/679 | 0/0 | `string:33983000:8496750.000:2000` |
| objects-polymorphic (rquickjs-jit matrix) | 6657980 | 0.04× | N/A (no native entry) | 0.99× | — | 120/0 | 0/0 | `number:413e8c5000000000` |
| calls-recursion-closures (rquickjs-jit matrix) | 7376264 | 0.07× | 0.18× | N/A (no native entry) | — | 305948/2160700 | 0/0 | `number:40d4820000000000` |
| json-codec (rquickjs-jit matrix) | 78730349 | 0.36× | N/A (no native entry) | 1.00× | — | 120/0 | 0/0 | `string:2013000:123` |
| map-set-bigint (rquickjs-jit matrix) | 15209423 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | — | 0/0 | 0/0 | `string:2000:256:e2edbb6504d6fce8` |
| exceptions-promises-async (rquickjs-jit matrix) | 1961669 | N/A (no native entry) | N/A (no native entry) | N/A (no native entry) | — | 0/0 | 0/0 | `string:124000:72576` |

Stripped binary evidence: non-JIT 1718976 bytes; JIT 5832840 bytes; delta +4113864 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 3.27×..3.44×
- PASS — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([1.6053661622552229, 1.7131290124038567])), ("numeric", Some([19.134002720630228, 25.04497499785137])), ("scalar-loop", Some([20.52084292746775, 26.76202922548727])), ("call-heavy", Some([2.031223363348475, 3.6730467493572396])), ("property-heavy", Some([0.05884823909995908, 0.06444019184836176])), ("fibonacci-iterative", Some([41.12069509974009, 42.13550035548807]))]
- PASS — **Every strict sample has required native tier**: all samples
- PASS — **Automatic uses production profitability policy**: all samples evaluated
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([1.888397333671122, 1.968132654806714]), reload=Some([1.2215236162768106, 1.2652389981083583]), p99=Some([0.4452916919428781, 0.4623661655595627])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
