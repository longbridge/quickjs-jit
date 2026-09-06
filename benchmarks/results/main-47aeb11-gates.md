# JIT performance report

Status: generated from tracked raw `jit-benchmark-v1` evidence. Source `47aeb11839a51ac9ae5d1d7c4b30c54ad3d3c3b7` (dirty: false), QuickJS `fd0a0210b7be00957751871e7e01b8291268fc29`; target `x86_64`; CPU `model name	: 13th Gen Intel(R) Core(TM) i7-13700KF`; power `powersave`. Bun: `1.4.0` at `/home/jason/.bun/bin/bun` (SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).

Command: `target/release/jit-bench compare --modes interpreter,tier1,tier2,automatic,bun --output /tmp/jit-main47a-five-engine.json --report /tmp/jit-main47a-five-engine.md`. Schema SHA-256 `01abae6a1b1a5b0492897612693493b19ee011d7e4dfcd7108616012f28ecaab`; suites lock SHA-256 `84c026c6e06a7a2cfd5a91901e7e5a4ff6e7753c187866016947c2dda96d55b4`.

Sampling: 5 discarded warmup processes, 30 interleaved paired fresh processes, 10 interleaved one-second throughput windows, 10000 joint paired bootstrap resamples.

## Workloads

Tier1/Tier2 columns require an entry in the requested tier; automatic accepts either native tier. Without that evidence, timing is shown as `N/A (no qualifying native entry)`. Bun remains an external engine comparison.

| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| quickjs-int-arith (QuickJS microbench) | 6804469 | 2.58× | 4.13× | 4.13× | 7.44× | 192/450 | 0/0 | `string:4992502500` |
| quickjs-bitops (rquickjs-jit local) | 1202750 | 2.82× | 5.74× | 5.75× | 67.28× | 371/450 | 0/0 | `number:c1c8bb05c4000000` |
| quickjs-fibonacci (rquickjs-jit local) | 930783 | 2.57× | N/A (no qualifying native entry) | 2.50× | 64.87× | 450/0 | 0/0 | `number:41d4c00d39400000` |
| numeric (rquickjs-jit) | 620037 | 2.91× | 23.76× | 23.58× | 46.63× | 120/450 | 0/0 | `number:413e809800000000` |
| scalar-loop (rquickjs-jit focused) | 631903 | 2.97× | 24.14× | 24.02× | 47.26× | 120/450 | 0/0 | `number:413e809800000000` |
| call-heavy (rquickjs-jit focused) | 1398314 | 2.95× | 4.43× | 4.42× | 57.49× | 8084/52445 | 0/0 | `number:40bb580000000000` |
| generic-call-entry (rquickjs-jit focused) | 1078400 | 0.12× | N/A (no qualifying native entry) | 0.14× | 57.85× | 1140150/0 | 0/0 | `number:409f400000000000` |
| property-heavy (rquickjs-jit focused) | 1381416 | 0.45× | 0.27× | 0.27× | 3.87× | 153/450 | 0/0 | `number:412e942200000000` |
| fibonacci-iterative (rquickjs-jit focused) | 33803632 | 4.46× | 40.00× | 39.45× | 29.68× | 80/447 | 0/0 | `number:419865fb2c000000` |
| fibonacci-recursive (rquickjs-jit focused) | 12075744 | 0.12× | 0.99× | N/A (no qualifying native entry) | 42.76× | 13885/450 | 0/0 | `number:40ba6d0000000000` |
| collections (rquickjs-jit) | 1784929 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | 2.54× | 0/0 | 0/0 | `string:1018392` |
| strings-json (rquickjs-jit) | 2158027 | 0.49× | N/A (no qualifying native entry) | 0.95× | 3.64× | 159/0 | 0/0 | `string:{"length":2000,"first":"abcdefgh"}` |
| calls-closures (rquickjs-jit) | 3594400 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | 6.27× | 0/0 | 0/0 | `string:30872` |
| adversarial (rquickjs-jit) | 1061652 | 1.38× | N/A (no qualifying native entry) | 1.00× | 2.80× | 185/0 | 0/0 | `string:7995` |
| float64-dense (rquickjs-jit matrix) | 2907874 | 1.78× | 7.69× | 7.82× | 6.27× | 990/1182 | 0/0 | `number:40a2d29809876024` |
| strings-regexp (rquickjs-jit matrix) | 19384228 | 0.86× | N/A (no qualifying native entry) | 0.97× | 10.27× | 121/0 | 0/0 | `string:483:10249:|8@57|9@58|10@59` |
| arrays-typed (rquickjs-jit matrix) | 4598559 | 0.30× | 1.67× | 0.66× | 10.41× | 1349/1183 | 0/0 | `string:33983000:8496750.000:2000` |
| objects-polymorphic (rquickjs-jit matrix) | 6523461 | 0.28× | N/A (no qualifying native entry) | 0.97× | 10.73× | 127/0 | 0/0 | `number:413e8c5000000000` |
| calls-recursion-closures (rquickjs-jit matrix) | 7409529 | 0.08× | 0.20× | N/A (no qualifying native entry) | 4.63× | 280682/2720340 | 0/0 | `number:40d4820000000000` |
| json-codec (rquickjs-jit matrix) | 78478067 | 0.83× | N/A (no qualifying native entry) | 0.99× | 7.81× | 123/0 | 0/0 | `string:2013000:123` |
| map-set-bigint (rquickjs-jit matrix) | 15469565 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | 7.00× | 0/0 | 0/0 | `string:2000:256:e2edbb6504d6fce8` |
| exceptions-promises-async (rquickjs-jit matrix) | 1974985 | N/A (no qualifying native entry) | N/A (no qualifying native entry) | N/A (no qualifying native entry) | 2.63× | 0/0 | 0/0 | `string:124000:72576` |

Stripped binary evidence: non-JIT 1721920 bytes; JIT 5830504 bytes; delta +4108584 bytes.

## Acceptance gates

- FAIL/INCONCLUSIVE — **Compute paired geometric-mean lower CI ≥5×**: 3.87×..3.90×
- PASS — **At least one designated kernel lower CI ≥10×**: [("quickjs-int-arith", Some([4.092106014308881, 4.145302077636263])), ("numeric", Some([23.014539640067884, 23.886960252123927])), ("scalar-loop", Some([22.81823595897274, 23.922565241459903])), ("call-heavy", Some([4.374426351521262, 4.427634358487113])), ("property-heavy", Some([0.26707271956195855, 0.2710266146988435])), ("fibonacci-iterative", Some([38.92088042746747, 39.76947369570452]))]
- PASS — **Every strict sample has required native tier**: all samples
- FAIL/INCONCLUSIVE — **Automatic uses production profitability policy**: FAIL: missing decision
- PASS — **Checksums identical in every sample**: all samples
- FAIL/INCONCLUSIVE — **startup/hot-reload/P99 upper regression CI ≤5%**: startup=Some([2.3795877530632312, 2.408328936955414]), reload=Some([1.1055869106386833, 1.1287512917893832]), p99=Some([0.5556104054793453, 0.5895555266296872])
- FAIL/INCONCLUSIVE — **gpui-shell steady state ≥2×**: INCONCLUSIVE: Task 15 worktree evidence not supplied

## Phase, break-even, and memory evidence

Every raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.

## Exclusions

- SunSpider / all: not vendored; no redistribution/import performed
- JetStream / all: not vendored; no runnable components available locally

QuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.
