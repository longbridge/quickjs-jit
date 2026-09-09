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

**Complete 24-scenario matrix: candidate4, measured 2026-09-10.**
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

<!-- END JIT_MATRIX -->

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
