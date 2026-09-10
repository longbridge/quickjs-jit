# quickjs-jit

This is Longbridge's JIT-enabled distribution of `rquickjs`. The published
package is named `quickjs-jit`, while its Rust library name remains `rquickjs`
for source compatibility:

```toml
rquickjs = { package = "quickjs-jit", version = "=0.12.2" }
rquickjs-jit = { package = "quickjs-jit-runtime", version = "=0.12.2", features = ["compiler"] }
```

The distribution and JIT runtime must use the same patch version. The JIT ABI
includes internal QuickJS structure layouts, so mixing runtime, core, or sys
crate patch versions may compile but will be rejected during JIT startup.
`quickjs-jit-runtime` 0.12.3 has not been published yet. Consumers testing the
0.12.3 source must pin the distribution and runtime to the same Git revision
and use Cargo source patches from that revision for both `quickjs-jit-core` and
`quickjs-jit-sys`. Do not combine the published 0.12.2 runtime with 0.12.3
core or sys crates.

[![github](https://img.shields.io/badge/github-longbridge/rquickjs-8da0cb.svg?style=for-the-badge&logo=github)](https://github.com/longbridge/rquickjs)
[![crates](https://img.shields.io/crates/v/quickjs-jit.svg?style=for-the-badge&color=fc8d62&logo=rust)](https://crates.io/crates/quickjs-jit)
[![docs](https://img.shields.io/badge/docs.rs-quickjs--jit-66c2a5?style=for-the-badge&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K)](https://docs.rs/quickjs-jit)
[![status](https://img.shields.io/github/actions/workflow/status/longbridge/rquickjs/ci.yml?branch=main&style=for-the-badge&logo=github-actions&logoColor=white)](https://github.com/longbridge/rquickjs/actions/workflows/ci.yml)

This library is a high level bindings of the [QuickJS-NG](https://quickjs-ng.github.io/quickjs/) JavaScript engine, a fork of the [QuickJS](https://bellard.org/quickjs/) Javascript engine.
Its goal is to be an easy to use, and safe wrapper similar to the rlua library.

**QuickJS** is a small and embeddable JavaScript engine. It supports the _ES2020_ specification including modules, asynchronous generators, proxies and BigInt.
It optionally supports mathematical extensions such as big decimal floating point numbers (BigDecimal), big binary floating point numbers (BigFloat) and operator overloading.

## Main features of QuickJS

- Small and easily embeddable: just a few C files, no external dependency, 210 KiB of x86 code for a simple hello world program.
- Fast interpreter with very low startup time: runs the 75000 tests of the ECMAScript Test Suite in about 100 seconds on a single core of a desktop PC.
  The complete life cycle of a runtime instance completes in less than 300 microseconds.
- Almost complete ES2020 support including modules, asynchronous generators and full Annex B support (legacy web compatibility).
- Passes nearly 100% of the ECMAScript Test Suite tests when selecting the ES2020 features. A summary is available at Test262 Report.
- Can compile JavaScript sources to executables with no external dependency.
- Garbage collection using reference counting (to reduce memory usage and have deterministic behavior) with cycle removal.
- Mathematical extensions: BigDecimal, BigFloat, operator overloading, bigint mode, math mode.
- Command line interpreter with contextual colorization implemented in JavaScript.
- Small built-in standard library with C library wrappers.

## Features provided by this crate

- Full integration with async Rust
  - The ES6 Promises can be handled as Rust futures and vice versa
  - Easy integration with almost any async runtime or executor
- Flexible data conversion between Rust and JS
  - Many widely used Rust types can be converted to JS and vice versa
- Support for user-defined allocators
  - The `Runtime` can be created using custom allocator
  - Using Rust's global allocator is also fully supported
- Support for user-defined module resolvers and loaders which also
  can be combined to get more flexible solution for concrete case
- Support for bundling JS modules as a bytecode using `embed` macro
- Support for deferred calling of JS functions
- Full support of ES6 classes
  - Rust data types can be represented as JS classes
  - Data fields can be accessed via object properties
  - Both static and instance members is also supported
  - The properties can be defined with getters and setters
  - Support for constant static properties
  - Support for holding references to JS objects
    (Data type which holds refs should implement `Trace` trait to get garbage collector works properly)
  - Support for extending defined classes by JS

## Experimental JIT performance

Every JIT optimization must include benchmarks against **QuickJS, Bun, and
quickjs-jit**, with the complete per-scenario comparison in this README.
Bun default is the external performance target; the previous JIT revision
tracks regressions, and the interpreter establishes whether native execution
is profitable. See the [repository rules](AGENTS.md#performance-reporting) and
[next optimization targets](docs/PERFORMANCE_NEXT.md).

<!-- BEGIN JIT_MATRIX -->

**Complete 27-scenario macOS matrix, 2026-09-10: bounded semantic inlining candidate.**
This frozen candidate adds effect-free monomorphic inline regions to semantic
SSA: caller ValueId argument binding, literal-boolean branch folding, checked
Int32 add/subtract, and ordinary-call fallback when compilation budgets are
exhausted. Cold recovery replays the original pure call from its caller state;
this is not general inline-frame reconstruction. Loop guard hoisting, lazy
frame synchronization, heap facts/LICM, property forwarding/sinking, array
range elimination and the required Bun performance gates remain unfinished.

The previous JIT column is the frozen callee-retention candidate
`92e1d5c90f1f30b15f65c8dd41afa6020683bd297c23fa56d4224da681312333`.
The original `023a220` baseline is also measured and retained in the comparison
JSON and interpreter control below. Source snapshots and binary hashes identify
all measured revisions; no later work is implied by these timings.

The numeric-foundation no-regression gate remains **unmet**. Automatic runtime slowdowns relative to `023a220` with intervals entirely below parity occur in: `arrays-typed`.
Interpreter controls are included below; these observations alone do not isolate the cause.
Relative to the immediately preceding retention candidate, `host-compute` and `strings-json` also have intervals entirely below parity. The latter has zero native entries in this window, so its slowdown is not evidence of slower JIT machine code.

Timings are medians in **ms per ten workload calls**. Speed is reference
latency / candidate latency with paired geometric means and 95% confidence
intervals; values above 1x are faster. QuickJS uses the candidate binary with
JIT detached; quickjs-jit uses production automatic tiering; Bun uses defaults.
All scenarios, including losing, fallback-only and inconclusive results, remain
in the matrix.

| Workload | Previous JIT ms | QuickJS ms | Bun ms | quickjs-jit ms | JIT / previous speed [95% CI] | JIT / QuickJS speed [95% CI] | JIT / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| scalar-control-flow | 0.047750 | 1.175792 | 0.078271 | 0.047709 | 0.9853x [0.9280, 1.0232]; statistically tied (7.20% slower to 2.32% faster) | 23.9686x [22.6382, 24.7696] | 1.6098x [1.5093, 1.6789] |
| scalar-expressions | 0.044125 | 0.817542 | 0.068312 | 0.044313 | 0.9884x [0.9727, 1.0035]; statistically tied (2.73% slower to 0.35% faster) | 18.1951x [17.8991, 18.4695] | 1.5752x [1.5340, 1.6255] |
| host-compute | 0.058396 | 1.213834 | 0.197792 | 0.059354 | 0.9521x [0.8951, 0.9937] | 20.8922x [19.3012, 22.4833] | 3.2452x [3.0252, 3.4199] |
| mixed-quotes | 2.142396 | 1.544396 | 0.112334 | 2.142438 | 1.0532x [0.9925, 1.1770]; statistically tied (0.75% slower to 17.70% faster) | 0.7232x [0.7168, 0.7298] | 0.0532x [0.0519, 0.0549] |
| quickjs-int-arith | 1.732520 | 3.391000 | 0.113729 | 1.729000 | 1.0323x [1.0036, 1.0826] | 1.9795x [1.9605, 2.0029] | 0.0669x [0.0657, 0.0684] |
| quickjs-bitops | 0.205167 | 0.540999 | 0.128145 | 0.207167 | 0.9810x [0.9454, 1.0069]; statistically tied (5.46% slower to 0.69% faster) | 2.5906x [2.4972, 2.6658] | 0.6159x [0.5969, 0.6316] |
| quickjs-fibonacci | 0.473542 | 0.645188 | 0.129479 | 0.478375 | 0.9666x [0.9318, 1.0002]; statistically tied (6.82% slower to 0.02% faster) | 1.3202x [1.2791, 1.3646] | 0.2794x [0.2583, 0.3074] |
| numeric | 0.024500 | 0.397479 | 0.129833 | 0.026083 | 0.9638x [0.8708, 1.0600]; statistically tied (12.92% slower to 6.00% faster) | 15.9114x [14.3101, 17.6350] | 4.9788x [4.3793, 5.7124] |
| scalar-loop | 0.024521 | 0.394896 | 0.137479 | 0.024688 | 1.0165x [0.9247, 1.1130]; statistically tied (7.53% slower to 11.30% faster) | 15.8441x [14.9814, 16.7318] | 5.5678x [4.9140, 6.3275] |
| call-heavy | 0.322375 | 0.995979 | 0.012875 | 0.289959 | 1.0952x [1.0457, 1.1354] | 3.3787x [3.2140, 3.5186] | 0.0438x [0.0424, 0.0452] |
| generic-call-entry | 0.251812 | 0.815083 | 0.006041 | 0.214479 | 1.1719x [1.1012, 1.2358] | 3.7952x [3.6470, 3.9572] | 0.0275x [0.0264, 0.0286] |
| generic-call-fallback | 2.972542 | 1.100104 | 0.012291 | 3.122937 | 0.9724x [0.8946, 1.0731]; statistically tied (10.54% slower to 7.31% faster) | 0.3514x [0.3152, 0.3977] | 0.0037x [0.0034, 0.0039] |
| property-heavy | 0.304667 | 0.918938 | 0.011291 | 0.304833 | 0.9619x [0.8574, 1.0320]; statistically tied (14.26% slower to 3.20% faster) | 2.9072x [2.4591, 3.2594] | 0.0350x [0.0306, 0.0382] |
| fibonacci-iterative | 0.867396 | 21.294833 | 0.977417 | 0.874645 | 0.9989x [0.9662, 1.0506]; statistically tied (3.38% slower to 5.06% faster) | 25.3709x [24.0964, 27.2051] | 1.1513x [1.0718, 1.2688] |
| fibonacci-recursive | 5.520396 | 5.882750 | 0.495667 | 5.557437 | 0.9878x [0.9686, 1.0035]; statistically tied (3.14% slower to 0.35% faster) | 1.0625x [1.0391, 1.0865] | 0.0892x [0.0869, 0.0914] |
| collections | 1.198521 | 1.188104 | 0.172125 | 1.166250 | 1.0778x [0.9879, 1.1808]; statistically tied (1.21% slower to 18.08% faster) | 1.0185x [0.9616, 1.0826]; statistically tied (3.84% slower to 8.26% faster) | 0.1446x [0.1290, 0.1620] |
| strings-json | 2.252625 | 2.274958 | 0.176020 | 2.269646 | 0.9846x [0.9709, 0.9960] | 1.1025x [0.9841, 1.3633]; statistically tied (1.59% slower to 36.33% faster) | 0.0804x [0.0766, 0.0850] |
| calls-closures | 3.942312 | 3.968145 | 0.157500 | 3.949771 | 0.9955x [0.9722, 1.0233]; statistically tied (2.78% slower to 2.33% faster) | 1.0272x [0.9742, 1.0975]; statistically tied (2.58% slower to 9.75% faster) | 0.0405x [0.0384, 0.0430] |
| adversarial | 0.671167 | 0.684334 | 0.140166 | 0.669854 | 0.8897x [0.6396, 1.1120]; statistically tied (36.04% slower to 11.20% faster) | 0.9023x [0.6658, 1.1178]; statistically tied (33.42% slower to 11.78% faster) | 0.1728x [0.1282, 0.2163] |
| float64-dense | 0.531980 | 2.376459 | 0.139812 | 0.544709 | 0.9889x [0.9752, 1.0030]; statistically tied (2.48% slower to 0.30% faster) | 4.6261x [4.3361, 5.1820] | 0.2653x [0.2565, 0.2751] |
| strings-regexp | 25.301603 | 25.816605 | 1.342375 | 25.314812 | 1.0096x [0.9462, 1.0836]; statistically tied (5.38% slower to 8.36% faster) | 1.0188x [0.9554, 1.0896]; statistically tied (4.46% slower to 8.96% faster) | 0.0510x [0.0474, 0.0552] |
| arrays-typed | 2.185376 | 3.746750 | 0.154729 | 2.176020 | 1.0033x [0.9899, 1.0165]; statistically tied (1.01% slower to 1.65% faster) | 1.7542x [1.7078, 1.8296] | 0.0730x [0.0702, 0.0759] |
| objects-polymorphic | 6.406084 | 6.442104 | 0.229813 | 6.505584 | 1.0051x [0.9758, 1.0409]; statistically tied (2.42% slower to 4.09% faster) | 1.0272x [0.9834, 1.1009]; statistically tied (1.66% slower to 10.09% faster) | 0.0400x [0.0355, 0.0468] |
| calls-recursion-closures | 4.838770 | 5.132376 | 0.235958 | 4.868229 | 0.9931x [0.9196, 1.0794]; statistically tied (8.04% slower to 7.94% faster) | 1.0073x [0.9431, 1.0643]; statistically tied (5.69% slower to 6.43% faster) | 0.0490x [0.0447, 0.0542] |
| json-codec | 96.847521 | 96.966708 | 7.374313 | 97.100667 | 1.0144x [0.9324, 1.1193]; statistically tied (6.76% slower to 11.93% faster) | 0.9550x [0.8988, 1.0054]; statistically tied (10.12% slower to 0.54% faster) | 0.0724x [0.0681, 0.0762] |
| map-set-bigint | 21.849355 | 21.723250 | 0.902333 | 21.780625 | 0.9832x [0.9535, 1.0077]; statistically tied (4.65% slower to 0.77% faster) | 1.0006x [0.9564, 1.0490]; statistically tied (4.36% slower to 4.90% faster) | 0.0421x [0.0398, 0.0448] |
| exceptions-promises-async | 3.919375 | 2.485375 | 0.264833 | 3.936500 | 0.9973x [0.9868, 1.0084]; statistically tied (1.32% slower to 0.84% faster) | 0.6406x [0.6260, 0.6643] | 0.0683x [0.0668, 0.0699] |

Protocol `shared-js-fixed-warmup-v2`: one initial call, 64 ten-call warmup
batches, then one timed ten-call batch, with result consumption/checksum outside
timing. Scripts, inputs and driver match across engines. Every workload has
five discarded and 30 retained fresh processes per configuration; six
configurations alternate in reversed order. All 5,670 process records are
archived. Bootstrap uses 10,000 paired percentile resamples, seed 20260909,
sorted indices 249/9749. Timing samples are not filtered by native readiness.

Host: Apple M3; macOS-27.0-arm64-arm-64bit-Mach-O. Bun 1.4.0, default flags. Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); LLVM version: 22.1.8.
Release build, no CPU pinning; power policy not recorded. `host-compute`
measures the equivalent pure-JS render kernel, excluding GPUI allocation and
snapshots. No Linux, host-reload or throughput result is inferred.

Automatic native coverage and the paired interpreter control:

| Workload | Fixed-batch native entries (min–max) | Compilation quiet samples / 30 | Candidate / 023a220 interpreter speed [95% CI] |
| --- | ---: | ---: | --- |
| scalar-control-flow | 10–10 | 30 | 1.0005x [0.9945, 1.0071]; statistically tied (0.55% slower to 0.71% faster) |
| scalar-expressions | 10–10 | 30 | 1.0040x [0.9896, 1.0184]; statistically tied (1.04% slower to 1.84% faster) |
| host-compute | 10–10 | 30 | 0.9692x [0.8984, 1.0456]; statistically tied (10.16% slower to 4.56% faster) |
| mixed-quotes | 6990–6990 | 30 | 0.9955x [0.9861, 1.0043]; statistically tied (1.39% slower to 0.43% faster) |
| quickjs-int-arith | 10–10 | 30 | 1.0270x [0.9807, 1.1151]; statistically tied (1.93% slower to 11.51% faster) |
| quickjs-bitops | 10–10 | 30 | 0.9724x [0.9557, 0.9879] |
| quickjs-fibonacci | 10–10 | 30 | 1.0297x [0.9711, 1.1054]; statistically tied (2.89% slower to 10.54% faster) |
| numeric | 10–10 | 19 | 0.9367x [0.8407, 1.0469]; statistically tied (15.93% slower to 4.69% faster) |
| scalar-loop | 10–10 | 22 | 0.9758x [0.8878, 1.0768]; statistically tied (11.22% slower to 7.68% faster) |
| call-heavy | 10–10 | 30 | 1.0221x [0.9890, 1.0588]; statistically tied (1.10% slower to 5.88% faster) |
| generic-call-entry | 10–10 | 30 | 0.9950x [0.9535, 1.0382]; statistically tied (4.65% slower to 3.82% faster) |
| generic-call-fallback | 20000–20000 | 30 | 1.0077x [0.9049, 1.1089]; statistically tied (9.51% slower to 10.89% faster) |
| property-heavy | 10–10 | 30 | 0.9824x [0.9144, 1.0491]; statistically tied (8.56% slower to 4.91% faster) |
| fibonacci-iterative | 10–10 | 30 | 0.9466x [0.8811, 0.9922] |
| fibonacci-recursive | 0–0 | 30 | 0.9802x [0.9616, 0.9925] |
| collections | 0–0 | 30 | 0.9791x [0.9104, 1.0502]; statistically tied (8.96% slower to 5.02% faster) |
| strings-json | 0–0 | 30 | 0.8938x [0.7252, 0.9993] |
| calls-closures | 0–0 | 30 | 0.9806x [0.9359, 1.0146]; statistically tied (6.41% slower to 1.46% faster) |
| adversarial | 0–0 | 30 | 0.9247x [0.8538, 0.9854] |
| float64-dense | 10–10 | 30 | 0.9490x [0.8459, 1.0107]; statistically tied (15.41% slower to 1.07% faster) |
| strings-regexp | 0–0 | 30 | 1.0352x [0.9629, 1.1343]; statistically tied (3.71% slower to 13.43% faster) |
| arrays-typed | 20–20 | 30 | 0.9890x [0.9496, 1.0144]; statistically tied (5.04% slower to 1.44% faster) |
| objects-polymorphic | 0–0 | 30 | 0.9552x [0.8811, 0.9987] |
| calls-recursion-closures | 0–0 | 30 | 1.0085x [0.9822, 1.0356]; statistically tied (1.78% slower to 3.56% faster) |
| json-codec | 0–0 | 30 | 0.9928x [0.9796, 1.0050]; statistically tied (2.04% slower to 0.50% faster) |
| map-set-bigint | 0–0 | 30 | 1.0068x [0.9561, 1.0711]; statistically tied (4.39% slower to 7.11% faster) |
| exceptions-promises-async | 0–0 | 30 | 0.9876x [0.9521, 1.0100]; statistically tied (4.79% slower to 1.00% faster) |

[All raw records and source/build inputs](benchmarks/results/semantic-inline-regions-paired-arm64.tar.gz),
[all configuration comparisons](benchmarks/results/semantic-inline-regions-paired-arm64.json),
[verified hashes, validation and provenance](benchmarks/results/semantic-inline-regions-paired-arm64-manifest.json),
and [remaining architecture and performance gates](docs/SEMANTIC_SSA_PROGRESS.md).

<!-- END JIT_MATRIX -->

<!-- BEGIN HISTORICAL_BITWISE_JIT_MATRIX -->

**Historical 27-scenario macOS matrix, 2026-09-10: semantic scalar SSA candidate.**
These measurements apply only to the archived semantic-bitwise candidate and predate bounded semantic inlining.

This frozen candidate adds CFG ValueIds/Phi, semantic arithmetic, comparisons,
increment/decrement and binary bitwise operations, pre-effect FrameState
bindings, explicit checks, CFG type facts and cyclic Int32 representation proofs to
`023a220`. The archived source snapshot and binary hash identify the measured
candidate. Proven numeric guards are removed before machine lowering.
General inlining, heap facts/LICM, array range elimination, inline
frame chains and full lazy materialization remain unfinished.

The numeric-foundation no-regression gate was **unmet in this historical run**. Automatic runtime slowdowns with intervals entirely below parity occur in: `call-heavy`, `generic-call-fallback`, `adversarial`.
Interpreter controls are included below; these observations alone do not isolate the cause.

Timings are medians in **ms per ten workload calls**. Speed is reference
latency / candidate latency with paired geometric means and 95% confidence
intervals; values above 1x are faster. QuickJS uses the candidate binary with
JIT detached; quickjs-jit uses production automatic tiering; Bun uses defaults.
All scenarios, including losing, fallback-only and inconclusive results, remain
in the matrix.

| Workload | Previous JIT ms | QuickJS ms | Bun ms | quickjs-jit ms | JIT / previous speed [95% CI] | JIT / QuickJS speed [95% CI] | JIT / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| scalar-control-flow | 0.050167 | 1.504479 | 0.130479 | 0.049562 | 1.0330x [0.8129, 1.2728]; statistically tied (18.71% slower to 27.28% faster) | 30.2170x [23.2157, 37.9195] | 2.8112x [2.1330, 3.7216] |
| scalar-expressions | 0.073833 | 0.876730 | 0.088708 | 0.046146 | 1.5487x [1.3653, 1.7199] | 19.2585x [16.6154, 21.9501] | 2.0760x [1.6485, 2.6887] |
| host-compute | 0.062833 | 1.289230 | 0.243125 | 0.063687 | 0.9973x [0.8401, 1.2016]; statistically tied (15.99% slower to 20.16% faster) | 21.5916x [18.5598, 25.1055] | 4.0384x [3.1145, 5.3765] |
| mixed-quotes | 2.326312 | 1.648354 | 0.162833 | 2.479855 | 0.9423x [0.8495, 1.0398]; statistically tied (15.05% slower to 3.98% faster) | 0.6512x [0.5886, 0.7160] | 0.0683x [0.0585, 0.0801] |
| quickjs-int-arith | 1.894625 | 3.452250 | 0.141021 | 1.849104 | 1.0325x [1.0002, 1.0713] | 1.8593x [1.7949, 1.9334] | 0.0766x [0.0704, 0.0845] |
| quickjs-bitops | 0.235437 | 0.535771 | 0.136187 | 0.224750 | 1.0629x [1.0186, 1.1393] | 2.3730x [2.3202, 2.4288] | 0.6024x [0.5853, 0.6224] |
| quickjs-fibonacci | 0.502604 | 0.662750 | 0.137999 | 0.514270 | 0.9747x [0.9247, 1.0275]; statistically tied (7.53% slower to 2.75% faster) | 1.3299x [1.2647, 1.3983] | 0.2802x [0.2564, 0.3108] |
| numeric | 0.023729 | 0.391125 | 0.113834 | 0.023646 | 1.0485x [0.9860, 1.1215]; statistically tied (1.40% slower to 12.15% faster) | 16.4621x [16.2655, 16.6660] | 5.1627x [4.6933, 5.7545] |
| scalar-loop | 0.023230 | 0.388417 | 0.116229 | 0.023500 | 1.0084x [0.9780, 1.0409]; statistically tied (2.20% slower to 4.09% faster) | 16.8610x [16.2424, 17.6919] | 5.3067x [4.9156, 5.8183] |
| call-heavy | 0.310812 | 0.994854 | 0.013000 | 0.325709 | 0.9621x [0.9414, 0.9877] | 3.0668x [3.0105, 3.1245] | 0.0395x [0.0385, 0.0406] |
| generic-call-entry | 0.249541 | 0.814354 | 0.006041 | 0.255167 | 1.0783x [0.9935, 1.1870]; statistically tied (0.65% slower to 18.70% faster) | 3.3439x [3.1984, 3.5209] | 0.0234x [0.0229, 0.0240] |
| generic-call-fallback | 2.981104 | 1.095250 | 0.011813 | 2.985333 | 0.9818x [0.9631, 0.9988] | 0.3670x [0.3592, 0.3748] | 0.0040x [0.0039, 0.0042] |
| property-heavy | 0.327541 | 0.928709 | 0.011291 | 0.323854 | 1.0793x [0.9873, 1.1848]; statistically tied (1.27% slower to 18.48% faster) | 2.7572x [2.5867, 2.9200] | 0.0357x [0.0328, 0.0395] |
| fibonacci-iterative | 0.893228 | 21.526021 | 1.073354 | 0.892687 | 0.9687x [0.9034, 1.0227]; statistically tied (9.66% slower to 2.27% faster) | 23.4207x [21.8899, 24.8480] | 1.1802x [1.0819, 1.2915] |
| fibonacci-recursive | 5.763708 | 6.284833 | 0.578959 | 5.815062 | 0.9946x [0.8774, 1.1087]; statistically tied (12.26% slower to 10.87% faster) | 1.0476x [0.9887, 1.1021]; statistically tied (1.13% slower to 10.21% faster) | 0.1006x [0.0854, 0.1209] |
| collections | 1.163021 | 1.173167 | 0.154584 | 1.148083 | 1.0364x [0.9958, 1.0882]; statistically tied (0.42% slower to 8.82% faster) | 1.0418x [1.0109, 1.0908] | 0.1424x [0.1311, 0.1572] |
| strings-json | 2.308521 | 2.294146 | 0.202604 | 2.335604 | 0.9640x [0.8851, 1.0331]; statistically tied (11.49% slower to 3.31% faster) | 0.9887x [0.9078, 1.0731]; statistically tied (9.22% slower to 7.31% faster) | 0.0801x [0.0733, 0.0867] |
| calls-closures | 4.227520 | 4.224437 | 0.190812 | 4.142583 | 1.0858x [0.9616, 1.2414]; statistically tied (3.84% slower to 24.14% faster) | 1.0233x [0.8892, 1.1814]; statistically tied (11.08% slower to 18.14% faster) | 0.0460x [0.0398, 0.0533] |
| adversarial | 0.696646 | 0.887750 | 0.156479 | 0.821000 | 0.8439x [0.7290, 0.9714] | 1.0235x [0.8320, 1.2758]; statistically tied (16.80% slower to 27.58% faster) | 0.1911x [0.1562, 0.2374] |
| float64-dense | 0.591938 | 3.442521 | 0.213521 | 0.568875 | 0.9878x [0.7831, 1.2651]; statistically tied (21.69% slower to 26.51% faster) | 4.7212x [3.7338, 5.9358] | 0.3296x [0.2668, 0.4035] |
| strings-regexp | 25.614916 | 25.992854 | 1.311438 | 25.341249 | 1.0039x [0.9898, 1.0171]; statistically tied (1.02% slower to 1.71% faster) | 1.0277x [1.0033, 1.0576] | 0.0525x [0.0509, 0.0543] |
| arrays-typed | 2.091146 | 3.797083 | 0.151188 | 2.109708 | 0.9917x [0.9767, 1.0078]; statistically tied (2.33% slower to 0.78% faster) | 1.8014x [1.7752, 1.8270] | 0.0722x [0.0697, 0.0752] |
| objects-polymorphic | 6.257688 | 6.330771 | 0.214750 | 6.266896 | 0.9982x [0.9853, 1.0109]; statistically tied (1.47% slower to 1.09% faster) | 0.9992x [0.9875, 1.0107]; statistically tied (1.25% slower to 1.07% faster) | 0.0356x [0.0338, 0.0380] |
| calls-recursion-closures | 4.708750 | 5.050916 | 0.205042 | 4.725562 | 1.0058x [0.9893, 1.0263]; statistically tied (1.07% slower to 2.63% faster) | 1.0621x [1.0510, 1.0723] | 0.0444x [0.0433, 0.0456] |
| json-codec | 97.112083 | 97.673229 | 7.242417 | 97.531042 | 0.9940x [0.9622, 1.0205]; statistically tied (3.78% slower to 2.05% faster) | 0.9797x [0.9465, 1.0059]; statistically tied (5.35% slower to 0.59% faster) | 0.0736x [0.0710, 0.0758] |
| map-set-bigint | 22.298000 | 21.883353 | 0.908771 | 21.943000 | 1.0591x [1.0037, 1.1629] | 1.0046x [0.9921, 1.0164]; statistically tied (0.79% slower to 1.64% faster) | 0.0446x [0.0419, 0.0485] |
| exceptions-promises-async | 3.930188 | 2.487624 | 0.264230 | 3.936104 | 1.0069x [0.9947, 1.0211]; statistically tied (0.53% slower to 2.11% faster) | 0.6357x [0.6283, 0.6438] | 0.0668x [0.0655, 0.0683] |

Protocol `shared-js-fixed-warmup-v2`: one initial call, 64 ten-call warmup
batches, then one timed ten-call batch, with result consumption/checksum outside
timing. Scripts, inputs and driver match across engines. Every workload has
five discarded and 30 retained fresh processes per configuration; five
configurations alternate in reversed order. All 4,725 process records are
archived. Bootstrap uses 10,000 paired percentile resamples, seed 20260909,
sorted indices 249/9749. Timing samples are not filtered by native readiness.

Host: Apple M3; macOS-27.0-arm64-arm-64bit-Mach-O. Bun 1.4.0, default flags. Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); LLVM version: 22.1.8.
Release build, no CPU pinning; power policy not recorded. `host-compute`
measures the equivalent pure-JS render kernel, excluding GPUI allocation and
snapshots. No Linux, host-reload or throughput result is inferred.

Automatic native coverage and the paired interpreter control:

| Workload | Fixed-batch native entries (min–max) | Compilation quiet samples / 30 | Current / previous interpreter speed [95% CI] |
| --- | ---: | ---: | --- |
| scalar-control-flow | 10–10 | 27 | 1.1283x [0.8911, 1.4263]; statistically tied (10.89% slower to 42.63% faster) |
| scalar-expressions | 10–10 | 29 | 0.9501x [0.8608, 1.0485]; statistically tied (13.92% slower to 4.85% faster) |
| host-compute | 10–10 | 22 | 0.9112x [0.7745, 1.0558]; statistically tied (22.55% slower to 5.58% faster) |
| mixed-quotes | 6990–6990 | 30 | 1.0424x [0.9449, 1.1649]; statistically tied (5.51% slower to 16.49% faster) |
| quickjs-int-arith | 10–10 | 30 | 1.0195x [0.9725, 1.0672]; statistically tied (2.75% slower to 6.72% faster) |
| quickjs-bitops | 10–10 | 30 | 1.0024x [0.9750, 1.0339]; statistically tied (2.50% slower to 3.39% faster) |
| quickjs-fibonacci | 10–10 | 30 | 1.0151x [0.9468, 1.0961]; statistically tied (5.32% slower to 9.61% faster) |
| numeric | 10–10 | 30 | 1.0409x [0.9492, 1.1814]; statistically tied (5.08% slower to 18.14% faster) |
| scalar-loop | 10–10 | 30 | 0.9930x [0.9586, 1.0240]; statistically tied (4.14% slower to 2.40% faster) |
| call-heavy | 10–10 | 30 | 1.0188x [0.9699, 1.0855]; statistically tied (3.01% slower to 8.55% faster) |
| generic-call-entry | 10–10 | 30 | 0.9813x [0.9273, 1.0436]; statistically tied (7.27% slower to 4.36% faster) |
| generic-call-fallback | 20000–20000 | 30 | 1.0151x [0.9784, 1.0656]; statistically tied (2.16% slower to 6.56% faster) |
| property-heavy | 10–10 | 30 | 1.0084x [0.9596, 1.0671]; statistically tied (4.04% slower to 6.71% faster) |
| fibonacci-iterative | 10–10 | 30 | 1.0550x [0.9783, 1.1649]; statistically tied (2.17% slower to 16.49% faster) |
| fibonacci-recursive | 0–0 | 30 | 1.1250x [1.0216, 1.2639] |
| collections | 0–0 | 30 | 0.9737x [0.9281, 1.0096]; statistically tied (7.19% slower to 0.96% faster) |
| strings-json | 0–0 | 30 | 0.9942x [0.9153, 1.0706]; statistically tied (8.47% slower to 7.06% faster) |
| calls-closures | 0–0 | 30 | 0.9399x [0.8396, 1.0190]; statistically tied (16.04% slower to 1.90% faster) |
| adversarial | 0–0 | 30 | 0.9975x [0.8423, 1.1742]; statistically tied (15.77% slower to 17.42% faster) |
| float64-dense | 10–10 | 30 | 0.9836x [0.8690, 1.1273]; statistically tied (13.10% slower to 12.73% faster) |
| strings-regexp | 0–0 | 30 | 0.9765x [0.9506, 0.9961] |
| arrays-typed | 20–20 | 30 | 1.0233x [0.9860, 1.0882]; statistically tied (1.40% slower to 8.82% faster) |
| objects-polymorphic | 0–0 | 30 | 1.0185x [0.9941, 1.0527]; statistically tied (0.59% slower to 5.27% faster) |
| calls-recursion-closures | 0–0 | 30 | 1.0101x [0.9973, 1.0242]; statistically tied (0.27% slower to 2.42% faster) |
| json-codec | 0–0 | 30 | 1.0048x [0.9955, 1.0148]; statistically tied (0.45% slower to 1.48% faster) |
| map-set-bigint | 0–0 | 30 | 1.0439x [0.9959, 1.1260]; statistically tied (0.41% slower to 12.60% faster) |
| exceptions-promises-async | 0–0 | 30 | 0.9865x [0.9741, 0.9981] |

[All raw records and source/build inputs](benchmarks/results/semantic-bitwise-paired-arm64.tar.gz),
[all configuration comparisons](benchmarks/results/semantic-bitwise-paired-arm64.json),
[verified hashes, validation and provenance](benchmarks/results/semantic-bitwise-paired-arm64-manifest.json),
and [remaining architecture and performance gates](docs/SEMANTIC_SSA_PROGRESS.md).

<!-- END HISTORICAL_BITWISE_JIT_MATRIX -->

<!-- BEGIN HISTORICAL_CFG_JIT_MATRIX -->

**Historical 27-scenario macOS matrix, 2026-09-10: CFG semantic SSA candidate.**
These measurements predate cyclic representation proofs, semantic updates,
guard-branch elimination and semantic bitwise operations. Results below apply
only to the archived CFG snapshot.

This frozen candidate adds CFG ValueIds/Phi, numeric and comparison operands,
pre-effect FrameState bindings, explicit numeric checks and CFG type facts to
`023a220`. The archived source snapshot and binary hash identify the measured
candidate. General inlining, heap facts/LICM, array range elimination, inline
frame chains and full lazy materialization remain unfinished.

The numeric-foundation no-regression gate was **unmet in this historical run**. Automatic runtime slowdowns with intervals entirely below parity occur in: `host-compute`, `numeric`, `scalar-loop`, `fibonacci-iterative`.
Interpreter controls are included below; these observations alone do not isolate the cause.

Timings are medians in **ms per ten workload calls**. Speed is reference
latency / candidate latency with paired geometric means and 95% confidence
intervals; values above 1x are faster. QuickJS uses the candidate binary with
JIT detached; quickjs-jit uses production automatic tiering; Bun uses defaults.
All scenarios, including losing, fallback-only and inconclusive results, remain
in the matrix.

| Workload | Previous JIT ms | QuickJS ms | Bun ms | quickjs-jit ms | JIT / previous speed [95% CI] | JIT / QuickJS speed [95% CI] | JIT / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| scalar-control-flow | 0.047625 | 1.160646 | 0.077291 | 0.061937 | 0.8383x [0.7571, 1.0136]; statistically tied (24.29% slower to 1.36% faster) | 19.8512x [18.6734, 22.1377] | 1.2708x [1.2452, 1.2993] |
| scalar-expressions | 0.073875 | 0.839563 | 0.080312 | 0.059875 | 1.3634x [1.2484, 1.5204] | 14.4407x [13.8821, 15.1259] | 1.4089x [1.3345, 1.4956] |
| host-compute | 0.060874 | 1.209938 | 0.211334 | 0.087208 | 0.7316x [0.7016, 0.7868] | 14.4300x [13.9294, 15.1862] | 2.6348x [2.4572, 2.8484] |
| mixed-quotes | 2.239729 | 1.605792 | 0.132916 | 2.249625 | 1.0203x [0.9809, 1.0638]; statistically tied (1.91% slower to 6.38% faster) | 0.7132x [0.6920, 0.7355] | 0.0624x [0.0568, 0.0699] |
| quickjs-int-arith | 1.819750 | 3.496104 | 0.123730 | 1.817584 | 0.9973x [0.9523, 1.0341]; statistically tied (4.77% slower to 3.41% faster) | 1.9082x [1.8223, 1.9880] | 0.0685x [0.0656, 0.0723] |
| quickjs-bitops | 0.242020 | 0.539917 | 0.135667 | 0.247188 | 0.9821x [0.9105, 1.0601]; statistically tied (8.95% slower to 6.01% faster) | 2.1240x [1.9988, 2.2299] | 0.5678x [0.5204, 0.6261] |
| quickjs-fibonacci | 0.484729 | 0.656166 | 0.128876 | 0.486000 | 0.9594x [0.8732, 1.0286]; statistically tied (12.68% slower to 2.86% faster) | 1.3052x [1.1883, 1.3910] | 0.2651x [0.2401, 0.2975] |
| numeric | 0.023416 | 0.385708 | 0.114417 | 0.034146 | 0.6605x [0.5227, 0.8187] | 9.9530x [8.1828, 11.2544] | 3.1892x [2.5819, 3.7439] |
| scalar-loop | 0.023667 | 0.384604 | 0.110750 | 0.034146 | 0.6691x [0.6182, 0.7085] | 10.7932x [10.0249, 11.2975] | 3.2011x [2.9338, 3.4108] |
| call-heavy | 0.313292 | 0.985875 | 0.012562 | 0.322166 | 1.0273x [0.9781, 1.0994]; statistically tied (2.19% slower to 9.94% faster) | 3.1727x [3.0786, 3.3021] | 0.0389x [0.0383, 0.0396] |
| generic-call-entry | 0.249396 | 0.806771 | 0.005959 | 0.245438 | 1.0313x [1.0054, 1.0632] | 3.3086x [3.2103, 3.4208] | 0.0253x [0.0241, 0.0266] |
| generic-call-fallback | 2.904542 | 1.077395 | 0.011813 | 2.968459 | 1.0176x [0.9588, 1.1240]; statistically tied (4.12% slower to 12.40% faster) | 0.3779x [0.3591, 0.4076] | 0.0040x [0.0039, 0.0043] |
| property-heavy | 0.318625 | 0.879250 | 0.010958 | 0.314854 | 0.9904x [0.9238, 1.0522]; statistically tied (7.62% slower to 5.22% faster) | 2.6964x [2.5242, 2.8221] | 0.0361x [0.0318, 0.0432] |
| fibonacci-iterative | 0.876229 | 21.794958 | 1.067980 | 1.623417 | 0.5540x [0.5345, 0.5801] | 13.3025x [13.0494, 13.5606] | 0.6728x [0.6430, 0.7067] |
| fibonacci-recursive | 5.661437 | 6.019083 | 0.540125 | 5.624375 | 0.9973x [0.9667, 1.0316]; statistically tied (3.33% slower to 3.16% faster) | 1.0432x [1.0157, 1.0690] | 0.0933x [0.0901, 0.0968] |
| collections | 1.201938 | 1.164208 | 0.163771 | 1.162292 | 1.0012x [0.9576, 1.0422]; statistically tied (4.24% slower to 4.22% faster) | 0.9842x [0.9332, 1.0363]; statistically tied (6.68% slower to 3.63% faster) | 0.1386x [0.1297, 0.1498] |
| strings-json | 2.305521 | 2.310125 | 0.203896 | 2.283229 | 1.0194x [0.9959, 1.0449]; statistically tied (0.41% slower to 4.49% faster) | 1.0424x [1.0020, 1.0898] | 0.0889x [0.0837, 0.0951] |
| calls-closures | 4.076250 | 4.014083 | 0.164480 | 4.015146 | 1.0245x [0.9965, 1.0565]; statistically tied (0.35% slower to 5.65% faster) | 0.9996x [0.9720, 1.0268]; statistically tied (2.80% slower to 2.68% faster) | 0.0425x [0.0390, 0.0472] |
| adversarial | 0.649042 | 0.656312 | 0.109250 | 0.649541 | 0.9932x [0.9731, 1.0072]; statistically tied (2.69% slower to 0.72% faster) | 1.0122x [0.9882, 1.0313]; statistically tied (1.18% slower to 3.13% faster) | 0.1762x [0.1669, 0.1888] |
| float64-dense | 0.529229 | 2.311041 | 0.129416 | 0.523146 | 1.0148x [0.9957, 1.0365]; statistically tied (0.43% slower to 3.65% faster) | 4.3765x [4.3025, 4.4295] | 0.2502x [0.2460, 0.2557] |
| strings-regexp | 25.365938 | 25.918208 | 1.301354 | 25.483291 | 1.0029x [0.9894, 1.0173]; statistically tied (1.06% slower to 1.73% faster) | 1.0271x [1.0096, 1.0500] | 0.0513x [0.0504, 0.0524] |
| arrays-typed | 2.098750 | 3.706729 | 0.149812 | 2.083333 | 1.0157x [1.0015, 1.0350] | 1.7830x [1.7712, 1.7948] | 0.0736x [0.0712, 0.0765] |
| objects-polymorphic | 6.219521 | 6.236604 | 0.201750 | 6.203583 | 1.0149x [1.0023, 1.0293] | 1.0087x [0.9988, 1.0197]; statistically tied (0.12% slower to 1.97% faster) | 0.0357x [0.0337, 0.0383] |
| calls-recursion-closures | 4.642895 | 4.910125 | 0.197480 | 4.656230 | 0.9969x [0.9905, 1.0034]; statistically tied (0.95% slower to 0.34% faster) | 1.0531x [1.0470, 1.0595] | 0.0423x [0.0421, 0.0426] |
| json-codec | 99.073521 | 97.773729 | 7.214917 | 97.680063 | 1.0483x [0.9737, 1.1425]; statistically tied (2.63% slower to 14.25% faster) | 0.9983x [0.9478, 1.0632]; statistically tied (5.22% slower to 6.32% faster) | 0.0792x [0.0711, 0.0919] |
| map-set-bigint | 21.941520 | 21.835354 | 0.894354 | 21.704854 | 1.0067x [0.9936, 1.0210]; statistically tied (0.64% slower to 2.10% faster) | 1.0157x [0.9979, 1.0387]; statistically tied (0.21% slower to 3.87% faster) | 0.0418x [0.0408, 0.0432] |
| exceptions-promises-async | 3.974105 | 2.495208 | 0.268896 | 4.012416 | 0.9832x [0.9506, 1.0057]; statistically tied (4.94% slower to 0.57% faster) | 0.6541x [0.6250, 0.6984] | 0.0704x [0.0662, 0.0764] |

Protocol `shared-js-fixed-warmup-v2`: one initial call, 64 ten-call warmup
batches, then one timed ten-call batch, with result consumption/checksum outside
timing. Scripts, inputs and driver match across engines. Every workload has
five discarded and 30 retained fresh processes per configuration; five
configurations alternate in reversed order. All 4,725 process records are
archived. Bootstrap uses 10,000 paired percentile resamples, seed 20260909,
sorted indices 249/9749. Timing samples are not filtered by native readiness.

Host: Apple M3; macOS-27.0-arm64-arm-64bit-Mach-O. Bun 1.4.0, default flags. Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); LLVM version: 22.1.8.
Release build, no CPU pinning; power policy not recorded. `host-compute`
measures the equivalent pure-JS render kernel, excluding GPUI allocation and
snapshots. No Linux, host-reload or throughput result is inferred.

Automatic native coverage and the paired interpreter control:

| Workload | Fixed-batch native entries (min–max) | Compilation quiet samples / 30 | Current / previous interpreter speed [95% CI] |
| --- | ---: | ---: | --- |
| scalar-control-flow | 10–10 | 30 | 0.9738x [0.8623, 1.0633]; statistically tied (13.77% slower to 6.33% faster) |
| scalar-expressions | 10–10 | 30 | 0.9997x [0.9665, 1.0348]; statistically tied (3.35% slower to 3.48% faster) |
| host-compute | 10–10 | 30 | 0.9880x [0.9352, 1.0343]; statistically tied (6.48% slower to 3.43% faster) |
| mixed-quotes | 6990–6990 | 30 | 0.9948x [0.9677, 1.0218]; statistically tied (3.23% slower to 2.18% faster) |
| quickjs-int-arith | 10–10 | 30 | 0.9931x [0.9616, 1.0241]; statistically tied (3.84% slower to 2.41% faster) |
| quickjs-bitops | 10–10 | 30 | 0.9887x [0.9621, 1.0115]; statistically tied (3.79% slower to 1.15% faster) |
| quickjs-fibonacci | 10–10 | 30 | 0.9811x [0.9423, 1.0188]; statistically tied (5.77% slower to 1.88% faster) |
| numeric | 10–10 | 29 | 1.0096x [0.9950, 1.0258]; statistically tied (0.50% slower to 2.58% faster) |
| scalar-loop | 10–10 | 30 | 1.0189x [0.9993, 1.0406]; statistically tied (0.07% slower to 4.06% faster) |
| call-heavy | 10–10 | 30 | 0.9806x [0.9449, 1.0138]; statistically tied (5.51% slower to 1.38% faster) |
| generic-call-entry | 10–10 | 30 | 1.0020x [0.9665, 1.0357]; statistically tied (3.35% slower to 3.57% faster) |
| generic-call-fallback | 20000–20000 | 30 | 0.9576x [0.8855, 1.0077]; statistically tied (11.45% slower to 0.77% faster) |
| property-heavy | 10–10 | 30 | 1.0077x [0.9905, 1.0255]; statistically tied (0.95% slower to 2.55% faster) |
| fibonacci-iterative | 10–10 | 30 | 1.0036x [0.9842, 1.0303]; statistically tied (1.58% slower to 3.03% faster) |
| fibonacci-recursive | 0–0 | 30 | 0.9978x [0.9822, 1.0138]; statistically tied (1.78% slower to 1.38% faster) |
| collections | 0–0 | 30 | 1.0072x [0.9731, 1.0381]; statistically tied (2.69% slower to 3.81% faster) |
| strings-json | 0–0 | 30 | 0.9809x [0.9370, 1.0237]; statistically tied (6.30% slower to 2.37% faster) |
| calls-closures | 0–0 | 30 | 0.9989x [0.9817, 1.0161]; statistically tied (1.83% slower to 1.61% faster) |
| adversarial | 0–0 | 30 | 1.0110x [1.0003, 1.0228] |
| float64-dense | 10–10 | 30 | 1.0112x [0.9991, 1.0249]; statistically tied (0.09% slower to 2.49% faster) |
| strings-regexp | 0–0 | 30 | 0.9865x [0.9584, 1.0115]; statistically tied (4.16% slower to 1.15% faster) |
| arrays-typed | 20–20 | 30 | 1.0146x [1.0086, 1.0207] |
| objects-polymorphic | 0–0 | 30 | 0.9998x [0.9875, 1.0137]; statistically tied (1.25% slower to 1.37% faster) |
| calls-recursion-closures | 0–0 | 30 | 1.0044x [0.9984, 1.0101]; statistically tied (0.16% slower to 1.01% faster) |
| json-codec | 0–0 | 30 | 1.0297x [0.9563, 1.1066]; statistically tied (4.37% slower to 10.66% faster) |
| map-set-bigint | 0–0 | 30 | 0.9848x [0.9553, 1.0066]; statistically tied (4.47% slower to 0.66% faster) |
| exceptions-promises-async | 0–0 | 30 | 0.9564x [0.8961, 1.0019]; statistically tied (10.39% slower to 0.19% faster) |

[All raw records and source/build inputs](benchmarks/results/semantic-cfg-paired-arm64.tar.gz),
[all configuration comparisons](benchmarks/results/semantic-cfg-paired-arm64.json),
[verified hashes, validation and provenance](benchmarks/results/semantic-cfg-paired-arm64-manifest.json),
and [remaining architecture and performance gates](docs/SEMANTIC_SSA_PROGRESS.md).

<!-- END HISTORICAL_CFG_JIT_MATRIX -->

<!-- BEGIN HISTORICAL_NUMERIC_JIT_MATRIX -->

**Historical 26-scenario macOS matrix, 2026-09-10: first intra-block numeric step.**
This older candidate contains intra-block expression reuse only; it predates the CFG/Phi, FrameState, numeric-check and comparison changes measured above. Its timings and conclusions apply only to that archived source snapshot.

The no-regression gate was **unmet in that run**. `collections`, `arrays-typed` and
`calls-recursion-closures` are 2.39%, 1.37% and 0.62% slower than the previous
binary in this run; their intervals exclude equal speed. `collections` and
`calls-recursion-closures` have no timed native entries. The interpreter control
also slows for collections (0.9774x [0.9707, 0.9848]) and arrays
(0.9900x [0.9869, 0.9935]); the source of these differences remains unresolved.
The new scalar workload is statistically tied with the previous JIT, between
9.84% slower and 3.29% faster. No end-to-end gain is attributed to this change.
The three main targets remain below 0.5x Bun, and generic fallback remains
slower than its own interpreter.

Timings are medians in **ms per ten workload calls**. Ratios are paired geometric
means of reference latency / candidate latency, with 95% confidence intervals;
values above 1x are faster. QuickJS 0.15.1 runs with JIT detached in the candidate
binary; quickjs-jit uses production automatic tiering. Bun 1.4.0 uses default
flags. Every row, including fallback-only and slower results, is retained.

| Workload | Previous JIT ms | QuickJS ms | Bun ms | quickjs-jit ms | JIT / previous speed [95% CI] | JIT / QuickJS speed [95% CI] | JIT / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| scalar-expressions | 0.072042 | 0.834854 | 0.075979 | 0.071938 | 0.9691x [0.9016, 1.0329]; statistically tied (9.84% slower to 3.29% faster) | 11.4345x [10.5485, 12.3586] | 1.1152x [1.0146, 1.2901] |
| host-compute | 0.059916 | 1.212730 | 0.191792 | 0.059604 | 0.9799x [0.9357, 1.0167]; statistically tied (6.43% slower to 1.67% faster) | 20.4279x [19.3558, 21.6708] | 3.1904x [3.0398, 3.3336] |
| mixed-quotes | 2.213104 | 1.585625 | 0.126750 | 2.199521 | 1.0214x [0.9718, 1.0811]; statistically tied (2.82% slower to 8.11% faster) | 0.7122x [0.6897, 0.7334] | 0.0613x [0.0565, 0.0675] |
| quickjs-int-arith | 1.798854 | 3.426979 | 0.122605 | 1.801084 | 0.9895x [0.9732, 1.0046]; statistically tied (2.68% slower to 0.46% faster) | 1.9273x [1.8698, 1.9943] | 0.0704x [0.0668, 0.0747] |
| quickjs-bitops | 0.237208 | 0.528375 | 0.132729 | 0.233666 | 1.0183x [0.9786, 1.0574]; statistically tied (2.14% slower to 5.74% faster) | 2.2472x [2.1766, 2.3083] | 0.5644x [0.5424, 0.5851] |
| quickjs-fibonacci | 0.471459 | 0.629875 | 0.118792 | 0.471208 | 1.0085x [0.9953, 1.0274]; statistically tied (0.47% slower to 2.74% faster) | 1.3407x [1.3313, 1.3503] | 0.2558x [0.2528, 0.2591] |
| numeric | 0.023625 | 0.385563 | 0.112354 | 0.023688 | 0.9907x [0.9781, 1.0024]; statistically tied (2.19% slower to 0.24% faster) | 16.5390x [16.1446, 17.0668] | 4.8225x [4.6808, 4.9852] |
| scalar-loop | 0.023479 | 0.386605 | 0.111812 | 0.023708 | 1.0013x [0.9841, 1.0226]; statistically tied (1.59% slower to 2.26% faster) | 16.4648x [16.2284, 16.7598] | 4.7744x [4.6781, 4.8888] |
| call-heavy | 0.307042 | 0.968938 | 0.012375 | 0.311208 | 0.9775x [0.9492, 1.0003]; statistically tied (5.08% slower to 0.03% faster) | 3.0648x [2.9831, 3.1319] | 0.0397x [0.0384, 0.0411] |
| generic-call-entry | 0.240458 | 0.790833 | 0.005916 | 0.240541 | 1.0023x [0.9706, 1.0313]; statistically tied (2.94% slower to 3.13% faster) | 3.2422x [3.1513, 3.3091] | 0.0253x [0.0239, 0.0275] |
| generic-call-fallback | 2.966208 | 1.092854 | 0.011792 | 2.924708 | 1.0033x [0.9605, 1.0482]; statistically tied (3.95% slower to 4.82% faster) | 0.4208x [0.3585, 0.5509] | 0.0041x [0.0038, 0.0044] |
| property-heavy | 0.321146 | 0.890709 | 0.011000 | 0.320875 | 0.9950x [0.9660, 1.0225]; statistically tied (3.40% slower to 2.25% faster) | 2.7735x [2.7034, 2.8384] | 0.0342x [0.0331, 0.0351] |
| fibonacci-iterative | 0.864792 | 21.059604 | 0.873542 | 0.856750 | 1.0029x [0.9943, 1.0089]; statistically tied (0.57% slower to 0.89% faster) | 24.7274x [24.2600, 25.4422] | 1.0461x [1.0204, 1.0765] |
| fibonacci-recursive | 5.457667 | 5.750792 | 0.471042 | 5.382834 | 1.0107x [1.0060, 1.0152] | 1.0654x [1.0608, 1.0700] | 0.0882x [0.0873, 0.0895] |
| collections | 1.119354 | 1.170083 | 0.127688 | 1.145250 | 0.9761x [0.9708, 0.9811] | 1.0157x [1.0099, 1.0210] | 0.1142x [0.1098, 0.1201] |
| strings-json | 2.234958 | 2.244709 | 0.163771 | 2.251145 | 0.9979x [0.9894, 1.0075]; statistically tied (1.06% slower to 0.75% faster) | 0.9996x [0.9922, 1.0074]; statistically tied (0.78% slower to 0.74% faster) | 0.0733x [0.0725, 0.0744] |
| calls-closures | 3.897166 | 3.881625 | 0.126375 | 3.870709 | 1.0058x [0.9966, 1.0187]; statistically tied (0.34% slower to 1.87% faster) | 1.0019x [0.9945, 1.0101]; statistically tied (0.55% slower to 1.01% faster) | 0.0333x [0.0326, 0.0340] |
| adversarial | 0.643084 | 0.655646 | 0.107375 | 0.642813 | 1.0001x [0.9966, 1.0036]; statistically tied (0.34% slower to 0.36% faster) | 1.0195x [1.0157, 1.0234] | 0.1673x [0.1662, 0.1686] |
| float64-dense | 0.528292 | 2.292542 | 0.128875 | 0.528438 | 1.0021x [0.9982, 1.0065]; statistically tied (0.18% slower to 0.65% faster) | 4.3521x [4.3326, 4.3728] | 0.2443x [0.2436, 0.2451] |
| strings-regexp | 24.327521 | 24.535333 | 1.233499 | 24.321209 | 0.9985x [0.9929, 1.0038]; statistically tied (0.71% slower to 0.38% faster) | 1.0107x [1.0025, 1.0218] | 0.0510x [0.0503, 0.0522] |
| arrays-typed | 2.076042 | 3.749459 | 0.145646 | 2.111626 | 0.9863x [0.9827, 0.9905] | 1.7787x [1.7737, 1.7844] | 0.0702x [0.0688, 0.0718] |
| objects-polymorphic | 6.137041 | 6.162750 | 0.197020 | 6.173625 | 0.9968x [0.9929, 1.0003]; statistically tied (0.71% slower to 0.03% faster) | 1.0014x [0.9960, 1.0066]; statistically tied (0.40% slower to 0.66% faster) | 0.0321x [0.0319, 0.0325] |
| calls-recursion-closures | 4.644209 | 4.936104 | 0.196146 | 4.659541 | 0.9938x [0.9901, 0.9972] | 1.0616x [1.0537, 1.0730] | 0.0421x [0.0419, 0.0424] |
| json-codec | 93.563875 | 93.337458 | 6.957896 | 93.361750 | 1.0021x [0.9957, 1.0083]; statistically tied (0.43% slower to 0.83% faster) | 1.0005x [0.9947, 1.0060]; statistically tied (0.53% slower to 0.60% faster) | 0.0745x [0.0739, 0.0754] |
| map-set-bigint | 21.420750 | 21.432750 | 0.848958 | 21.434583 | 0.9999x [0.9980, 1.0021]; statistically tied (0.20% slower to 0.21% faster) | 1.0012x [0.9988, 1.0043]; statistically tied (0.12% slower to 0.43% faster) | 0.0398x [0.0395, 0.0403] |
| exceptions-promises-async | 3.866312 | 2.459729 | 0.248438 | 3.865896 | 0.9977x [0.9928, 1.0030]; statistically tied (0.72% slower to 0.30% faster) | 0.6346x [0.6310, 0.6383] | 0.0640x [0.0637, 0.0644] |

Protocol `shared-js-fixed-warmup-v2`: one initial call, 64 ten-call warmup
batches, then one timed ten-call batch, with result consumption/checksum outside
timing. All engines use identical scripts, inputs and driver. Each workload has
five discarded and 30 retained fresh processes in each of five configurations
(previous/current automatic, previous/current interpreter, default Bun), with
alternating reversed order. All 4,550 process records are archived. Bootstrap:
10,000 paired percentile resamples, seed 20260909, sorted indices 249/9749.

Apple M3, macOS ARM64 / Darwin 27.0.0; Rust 1.98.1 / LLVM 22.1.8;
release build, no CPU pinning, power policy not recorded. `host-compute` measures
the equivalent pure-JS render kernel, excluding GPUI allocation and snapshots.
No Linux, host reload or throughput result is inferred from these timings.

Automatic native coverage is reported without conditioning the timing samples:

| Workload | Fixed-batch native entries (min–max) | Compilation quiet samples / 30 |
| --- | ---: | ---: |
| scalar-expressions | 10–10 | 30 |
| host-compute | 10–10 | 30 |
| mixed-quotes | 6990–6990 | 30 |
| quickjs-int-arith | 10–10 | 30 |
| quickjs-bitops | 10–10 | 30 |
| quickjs-fibonacci | 10–10 | 30 |
| numeric | 10–10 | 30 |
| scalar-loop | 10–10 | 30 |
| call-heavy | 10–10 | 30 |
| generic-call-entry | 10–10 | 30 |
| generic-call-fallback | 20000–20000 | 30 |
| property-heavy | 10–10 | 30 |
| fibonacci-iterative | 10–10 | 30 |
| fibonacci-recursive | 0–0 | 30 |
| collections | 0–0 | 30 |
| strings-json | 0–0 | 30 |
| calls-closures | 0–0 | 30 |
| adversarial | 0–0 | 30 |
| float64-dense | 10–10 | 30 |
| strings-regexp | 0–0 | 30 |
| arrays-typed | 20–20 | 30 |
| objects-polymorphic | 0–0 | 30 |
| calls-recursion-closures | 0–0 | 30 |
| json-codec | 0–0 | 30 |
| map-set-bigint | 0–0 | 30 |
| exceptions-promises-async | 0–0 | 30 |

[All raw records and source/build inputs](benchmarks/results/semantic-values-paired-arm64.tar.gz),
[all configuration comparisons](benchmarks/results/semantic-values-paired-arm64.json),
[verified hashes and machine provenance](benchmarks/results/semantic-values-paired-arm64-manifest.json),
and [remaining architecture and performance gates](docs/SEMANTIC_SSA_PROGRESS.md).

<!-- END HISTORICAL_NUMERIC_JIT_MATRIX -->

<!-- BEGIN HISTORICAL_JIT_MATRIX -->

**Historical Linux 24-scenario matrix: candidate4, measured 2026-09-10.**
**Host compute regression remains unresolved:** automatic hot reload is
0.9444x baseline speed [0.9373, 0.9518], and interpreter steady-state is
0.9329x [0.9303, 0.9354]. The host regression budget has not passed; this is
measured candidate evidence, not an acceptance claim. See the
[implementation, paired controls, and host findings](docs/BUN_BOUNDARIES_20260909.md).

Measured source snapshot: `candidate4`, binary SHA-256 `b758218bdd1d746083f34693a78d42607bc663086fa3e64e2d2f98384d17c2a3`.
QuickJS and automatic use that same binary; QuickJS means JIT detached. Bun
1.4.0 uses default flags `[]`, executable `/home/jason/.bun/bin/bun`,
SHA-256 `33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`. Host: 13th Gen Intel(R) Core(TM) i7-13700KF; Linux-7.1.9-arch1-2-x86_64-with-glibc2.44; pinned
CPU 2; governor `powersave`; `rustc 1.98.1 (48a229cea 2026-09-01)`. Exact compiler/build,
source revision/patch manifests, and host details belong to the linked archive;
this renderer does not infer unrecorded source revisions from binary names.

Protocol `shared-js-fixed-warmup-v2`: one initial call, then 64 batches of ten calls, followed
by one measured ten-call batch. All engines run the same driver, input policy,
sequential Promise completion, and result storage; checksums run after timing.
Driver SHA-256 `60408ad6d28c58e7f20161c17a5db11fc142685fdd07ec7dbf35d7bb02a8f7c5`. Each row has five discarded processes/config and
30 interleaved retained fresh-process pairs/config. Absolute timings are
medians in **ms per ten workload calls**. Speed ratios are geometric means of
paired reference-latency/automatic-latency ratios, with deterministic 10,000
paired percentile-bootstrap resamples (seed 20260909); they are not ratios of the
displayed medians. Above 1x is faster than the named baseline. Intervals crossing
1x are statistically tied, with the slower/faster range shown explicitly.

These are fixed-warmup latencies. Samples are not excluded because they lack
native entries, compile during the window, fall back, or run slowly. QuickJS
includes one outer Rust call/Promise bridge; both engines include shared driver
array/closure work. Fixed counters surround the batch before checksum execution
and aggregate driver plus workload, not a named function. The quiet count means
zero pending jobs/snapshot bytes at both endpoints and no observed installation
or compile-time increment, not proof of an engine's peak tier. Native counts do
not establish Bun's tier.

Readiness diagnostics are collected later only for QuickJS; their timings and
cumulative counters must not replace fixed-window evidence or be divided into
Bun's differently conditioned latency. This latency matrix contains no JS
throughput windows, matched settled-state Bun samples, per-call P99, or host
snapshot measurement. Those requirements remain separate; `mixed-quotes` is a
pure JS kernel, not GPUI snapshot timing. Legacy gate reporting rejects this
protocol. [Raw evidence](benchmarks/results/bun-boundaries-20260909-evidence.tar.gz) and [machine-readable summary](benchmarks/results/bun-boundaries-20260909-matrix.json).

| Workload | QuickJS ms | Bun ms | automatic ms | automatic / QuickJS speed [95% CI] | automatic / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | --- | --- |
| mixed-quotes | 2.349774 | 0.147462 | 2.736590 | 0.8595x [0.8568, 0.8627] | 0.0542x [0.0536, 0.0549] |
| quickjs-int-arith | 5.357147 | 0.131634 | 1.634172 | 3.2921x [3.2831, 3.3018] | 0.0831x [0.0811, 0.0855] |
| quickjs-bitops | 1.144004 | 0.152575 | 0.205012 | 5.5814x [5.5525, 5.6111] | 0.7575x [0.7399, 0.7778] |
| quickjs-fibonacci | 0.839358 | 0.144318 | 0.359949 | 2.3896x [2.3486, 2.4372] | 0.4116x [0.4054, 0.4184] |
| numeric | 0.488411 | 0.129795 | 0.021608 | 22.4911x [22.2385, 22.7115] | 6.1432x [5.9995, 6.3035] |
| scalar-loop | 0.489399 | 0.132878 | 0.021566 | 22.2350x [21.4209, 22.7450] | 6.0842x [5.8316, 6.3037] |
| call-heavy | 1.261803 | 0.006315 | 0.312781 | 4.0333x [3.9902, 4.0812] | 0.0203x [0.0202, 0.0205] |
| generic-call-entry | 0.934287 | 0.004954 | 0.231675 | 4.0356x [4.0086, 4.0622] | 0.0225x [0.0213, 0.0251] |
| generic-call-fallback | 1.274439 | 0.008116 | 3.786595 | 0.3376x [0.3360, 0.3391] | 0.0022x [0.0021, 0.0022] |
| property-heavy | 1.029746 | 0.007804 | 0.325760 | 3.1635x [3.1323, 3.1971] | 0.0243x [0.0238, 0.0252] |
| fibonacci-iterative | 29.467614 | 0.936226 | 0.878952 | 33.7586x [33.3822, 34.1502] | 1.0773x [1.0613, 1.0940] |
| fibonacci-recursive | 9.865694 | 0.549300 | 9.895509 | 0.9992x [0.9965, 1.0023]; statistically tied; between 0.3% slower and 0.2% faster | 0.0555x [0.0549, 0.0560] |
| collections | 1.575133 | 0.137655 | 1.576190 | 1.0005x [0.9968, 1.0055]; statistically tied; between 0.3% slower and 0.5% faster | 0.0877x [0.0871, 0.0883] |
| strings-json | 1.972826 | 0.187261 | 2.025572 | 0.9753x [0.9704, 0.9801] | 0.0901x [0.0879, 0.0923] |
| calls-closures | 3.489562 | 0.228020 | 3.519299 | 0.9897x [0.9845, 0.9946] | 0.0619x [0.0596, 0.0640] |
| adversarial | 0.914932 | 0.125981 | 0.916971 | 1.0023x [0.9906, 1.0145]; statistically tied; between 0.9% slower and 1.4% faster | 0.1394x [0.1361, 0.1434] |
| float64-dense | 2.281055 | 0.148484 | 0.362700 | 6.3050x [6.2579, 6.3595] | 0.4153x [0.4097, 0.4221] |
| strings-regexp | 19.591037 | 1.309532 | 20.241909 | 0.9663x [0.9622, 0.9690] | 0.0648x [0.0643, 0.0652] |
| arrays-typed | 4.160535 | 0.138155 | 2.534226 | 1.6407x [1.6359, 1.6455] | 0.0548x [0.0544, 0.0552] |
| objects-polymorphic | 6.268702 | 0.202083 | 6.652939 | 0.9425x [0.9385, 0.9462] | 0.0304x [0.0303, 0.0306] |
| calls-recursion-closures | 6.464855 | 0.206337 | 6.552003 | 0.9888x [0.9854, 0.9927] | 0.0317x [0.0315, 0.0321] |
| json-codec | 77.302303 | 7.727146 | 78.995999 | 0.9843x [0.9758, 0.9981] | 0.0978x [0.0974, 0.0981] |
| map-set-bigint | 17.486868 | 0.971720 | 16.617827 | 1.0430x [1.0259, 1.0590] | 0.0591x [0.0584, 0.0598] |
| exceptions-promises-async | 2.323206 | 0.288878 | 3.487680 | 0.6624x [0.6530, 0.6694] | 0.0825x [0.0811, 0.0838] |

## Separate QuickJS execution diagnostics

| Workload | quiet fixed batches / 30 | native entry delta | Tier2 entry delta | deopt delta | later readiness median ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| mixed-quotes | 30 | 6990 | 6980 | 0 | 2.752615 |
| quickjs-int-arith | 30 | 10 | 10 | 0 | 1.618791 |
| quickjs-bitops | 30 | 10 | 10 | 0 | 0.204897 |
| quickjs-fibonacci | 30 | 10 | 0 | 0 | 0.371977 |
| numeric | 30 | 10 | 10 | 0 | 0.021836 |
| scalar-loop | 30 | 10 | 10 | 0 | 0.021813 |
| call-heavy | 30 | 10 | 10 | 0 | 0.313397 |
| generic-call-entry | 30 | 10 | 10 | 0 | 0.225981 |
| generic-call-fallback | 30 | 20000 | 20000 | 0 | 3.785854 |
| property-heavy | 30 | 10 | 10 | 0 | 0.319246 |
| fibonacci-iterative | 30 | 10 | 10 | 0 | 0.880027 |
| fibonacci-recursive | 30 | 0 | 0 | 0 | 9.921852 |
| collections | 30 | 0 | 0 | 0 | 1.586696 |
| strings-json | 30 | 0 | 0 | 0 | 2.031513 |
| calls-closures | 30 | 0 | 0 | 0 | 3.521440 |
| adversarial | 30 | 0 | 0 | 0 | 0.935643 |
| float64-dense | 30 | 10 | 10 | 0 | 0.364465 |
| strings-regexp | 30 | 0 | 0 | 0 | 20.256235 |
| arrays-typed | 30 | 20 | 20 | 0 | 2.535597 |
| objects-polymorphic | 30 | 0 | 0 | 0 | 6.687356 |
| calls-recursion-closures | 30 | 0 | 0 | 0 | 6.534747 |
| json-codec | 30 | 0 | 0 | 0 | 79.607234 |
| map-set-bigint | 30 | 0 | 0 | 0 | 16.940369 |
| exceptions-promises-async | 30 | 0 | 0 | 0 | 3.538708 |

All 24 scenarios are included. Zero-entry rows measure attached-runtime fallback;
aggregate native/Tier2 counts do not identify the tier of an individual callee.
The original branch-leaf call probe can now use compiled caller plus direct ABI;
`generic-call-fallback` preserves effectful non-direct calls and native entries.
It does not prove a compiled caller invokes the C CALL helper on every iteration.

<!-- END HISTORICAL_JIT_MATRIX -->

**Historical focused baseline comparison, 2026-09-10 (Apple M3, macOS ARM64):**
Eight pure-JS controls compare old runtime `928a8f2` with merged #26 `023a220`,
built with the identical Cargo.lock and benchmark harness. `host-compute`
reproduces the archived host render kernel and text result. That baseline comparison did not change runtime code. This is a different host from the historical
24-scenario matrix above. The complete current macOS matrix is above; native Linux host regression
acceptance remains deferred.

| Workload | Old JIT ms | QuickJS ms | Bun ms | automatic ms | automatic / old JIT speed [95% CI] | automatic / QuickJS speed [95% CI] | automatic / Bun speed [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| host-compute | 0.061167 | 1.217812 | 0.200042 | 0.060688 | 1.0184x [0.9965, 1.0416]; statistically tied (0.35% slower to 4.16% faster) | 20.2439x [19.8452, 20.6762] | 3.7467x [3.3598, 4.2957] |
| generic-call-entry | 5.170479 | 0.788021 | 0.005834 | 0.246062 | 19.4033x [17.4142, 21.0883] | 2.9601x [2.6310, 3.2293] | 0.0219x [0.0195, 0.0239] |
| generic-call-fallback | 3.470271 | 1.067395 | 0.011792 | 2.886375 | 1.2263x [1.1896, 1.2789] | 0.3644x [0.3586, 0.3695] | 0.0041x [0.0039, 0.0045] |
| call-heavy | 0.311833 | 0.967584 | 0.012500 | 0.310917 | 0.9989x [0.9835, 1.0131]; statistically tied (1.65% slower to 1.31% faster) | 3.1262x [3.0649, 3.2045] | 0.0402x [0.0395, 0.0409] |
| property-heavy | 2.457979 | 0.884292 | 0.010937 | 0.318604 | 7.5748x [7.2831, 7.7730] | 2.7268x [2.6169, 2.8091] | 0.0347x [0.0330, 0.0363] |
| arrays-typed | 6.687042 | 3.786563 | 0.159188 | 2.086562 | 3.2116x [3.1561, 3.2903] | 1.8141x [1.7870, 1.8406] | 0.0766x [0.0732, 0.0811] |
| scalar-loop | 0.023959 | 0.385354 | 0.112833 | 0.023645 | 1.0018x [0.9853, 1.0181]; statistically tied (1.47% slower to 1.81% faster) | 16.2007x [15.8834, 16.5010] | 4.8451x [4.6780, 5.0425] |
| mixed-quotes | 2.319021 | 1.574021 | 0.116854 | 2.190834 | 1.0756x [1.0452, 1.1177] | 0.7293x [0.7007, 0.7766] | 0.0551x [0.0530, 0.0575] |

Units are ms per ten calls, using `shared-js-fixed-warmup-v2`: one initial call,
64 ten-call warmup batches, then one timed ten-call batch with checksum outside
timing. Each of five configurations has five discarded and 30 retained fresh
processes per workload; all 1,400 process records are archived, including slower
samples. Order reverses on alternating pairs. Bun 1.4.0 uses default flags;
QuickJS uses the candidate binary with JIT detached, and the JIT column uses
production automatic tiering. Ratios are paired geometric means with 10,000
percentile-bootstrap resamples, seed 20260909 (sorted indices 249 and 9749).
Rust 1.98.1 / LLVM 22.1.8, release build, shared Apple M3 on Darwin 27.0.0,
no CPU pinning, power policy not recorded for this run. No throughput or host
snapshot/reload result is inferred from these kernel latencies.

The `host-compute` interpreter comparison is statistically tied with the old
runtime: 1.0616x [0.9898, 1.1874], between 1.02% slower and 18.74% faster.
This does not resolve the native Linux regression. Generic fallback and mixed
quotes remain slower than their own interpreter; the three target workloads
remain below 0.5x Bun. [All raw samples and build inputs](benchmarks/results/semantic-ssa-baseline-paired-arm64.tar.gz),
[all configuration summaries](benchmarks/results/semantic-ssa-baseline-paired-arm64.json),
[source/binary hashes and initial probe history](benchmarks/results/semantic-ssa-baseline-manifest.json),
and [remaining architecture work](docs/SEMANTIC_SSA_PROGRESS.md).

**Historical matrix:** the 2026-09-06 `47aeb11` results used different warmup
and host timing boundaries and must not be divided into these v2 timings to
claim an optimization gain. The historical [full table and tier diagnostics](benchmarks/results/main-47aeb11-engines.md),
[raw samples](benchmarks/results/main-47aeb11-engines.json),
[versions, flags, and reproduction](benchmarks/results/main-47aeb11-methodology.json),
and [derived intervals](benchmarks/results/main-47aeb11-summary.json) remain
available. [Benchmark commands and protocol details](benchmarks/README.md).

## Community development

This crate doesn't aim to provide system and web APIs. The QuickJS library is close to [V8](https://v8.dev/) in that regard.
If you need APIs from [WinterGC](https://wintercg.org/) or [Node](https://nodejs.org/api/), then you can take a look at the follow community projects:

- [AWS LLRT Modules](https://github.com/awslabs/llrt/tree/main/llrt_modules): Collection of modules that micmic some of the `Node` APIs in pure Rust
- [Rquickjs Extra](https://github.com/rquickjs/rquickjs-extra): Collection of modules that complement `AWS LLRT Modules` in pure Rust

The community has also built various utilities which might be relevant to you:

- [Rquickjs Serde](https://github.com/rquickjs/rquickjs-serde): Serde serializer and deserializer for rquickjs Value

## Development status

This bindings is feature complete, mostly stable and ready to use.
The error handling is only thing which may change in the future.
Some experimental features like `parallel` may not works as expected. Use it for your own risk.

## Supported platforms

Rquickjs needs to compile a C-library which has it's own limitation on supported platforms, furthermore it needs to generate bindings for that platform.
As a result rquickjs might not compile on all platforms which rust supports.
In general you can allways try to compile rquickjs with the `bindgen` feature, this should work for most platforms.
Rquickjs ships bindings for a limited set of platforms, for these platforms you don't have to enable the `bindgen` feature.
See below for a list of supported platforms.

| **platform**                   | **shipped bindings** | **tested** | **supported by quickjs** |
| ------------------------------ | :------------------: | :--------: | :----------------------: |
|                                |                      |            |                          |
| x86_64-unknown-linux-gnu       |          ✅          |     ✅     |            ✅            |
| i686-unknown-linux-gnu         |          ✅          |     ✅     |            ✅            |
| aarch64-unknown-linux-gnu      |          ✅          |     ✅     |            ✅            |
| loongarch64-unknown-linux-gnu  |          ✅          |     ✅     |            ✅            |
| x86_64-unknown-linux-musl      |          ✅          |     ✅     |            ✅            |
| aarch64-unknown-linux-musl     |          ✅          |     ✅     |            ✅            |
| loongarch64-unknown-linux-musl |          ✅          |     ✅     |            ✅            |
| x86_64-pc-windows-gnu          |          ✅          |     ✅     |            ✅            |
| i686-pc-windows-gnu            |          ✅          |     ✅     |            ✅            |
| x86_64-pc-windows-msvc         |          ✅          |     ✅     |     ❌ experimental!     |
| aarch64-pc-windows-msvc        |          ✅          |     ❌     |     ❌ experimental!     |
| x86_64-apple-darwin            |          ✅          |     ✅     |            ✅            |
| aarch64-apple-darwin           |          ✅          |     ❌     |            ✅            |
| wasm32-wasip1                  |          ✅          |     ✅     |            ✅            |
| wasm32-wasip2                  |          ✅          |     ✅     |            ✅            |
| other                          |          ❌          |     ❌     |         Unknown          |

## License

This library is licensed under the [MIT License](LICENSE)
