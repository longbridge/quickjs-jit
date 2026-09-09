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

**Last complete matrix: 2026-09-06, revision `47aeb11`, Bun 1.4.0.**
This is historical evidence, before PR #24. The current runtime checkpoint
`07535b1` has **not** been remeasured against Bun across this matrix. The
[separate gpui-shell mixed regression](docs/MIXED_REGRESSION_20260909.md)
records current-runtime host evidence; it does not update these Bun results.

QuickJS is the repository-pinned QuickJS-ng interpreter without an attached
JIT backend. quickjs-jit uses **production automatic tiering**, including
fallbacks. Forced Tier 1/2 results remain in the linked diagnostic report.
Absolute values below are median **milliseconds per batch of 10 workload
calls**, not per individual function call. Speed ratios are quickjs-jit relative
to the named baseline: above 1x is faster; below 1x is slower. Brackets contain
paired 95% confidence intervals.

| Scenario | QuickJS ms | Bun ms | quickjs-jit ms | JIT / QuickJS speed [95%] | JIT / Bun speed [95%] |
| --- | ---: | ---: | ---: | --- | --- |
| quickjs-int-arith | 6.8045 | 0.9142 | 1.6478 | 4.129x [4.078, 4.147] | 0.555x [0.552, 0.559] |
| quickjs-bitops | 1.2027 | 0.0179 | 0.2090 | 5.753x [5.735, 5.800] | 0.086x [0.085, 0.086] |
| quickjs-fibonacci | 0.9308 | 0.0143 | 0.3721 | 2.501x [2.481, 2.569] | 0.039x [0.038, 0.040] |
| numeric | 0.6200 | 0.0133 | 0.0263 | 23.581x [23.293, 24.160] | 0.506x [0.501, 0.529] |
| scalar-loop | 0.6319 | 0.0134 | 0.0263 | 24.024x [23.382, 24.316] | 0.508x [0.504, 0.520] |
| call-heavy | 1.3983 | 0.0243 | 0.3162 | 4.422x [4.392, 4.460] | 0.077x [0.070, 0.080] |
| generic-call-entry | 1.0784 | 0.0186 | 7.9852 | 0.135x [0.134, 0.136] | 0.002x [0.002, 0.003] |
| property-heavy | 1.3814 | 0.3567 | 5.1528 | 0.268x [0.267, 0.271] | 0.069x [0.069, 0.070] |
| fibonacci-iterative | 33.8036 | 1.1388 | 0.8569 | 39.448x [38.684, 40.356] | 1.329x [1.305, 1.361] |
| fibonacci-recursive | 12.0757 | 0.2824 | 12.2149 | 0.989x [0.984, 0.993] | 0.023x [0.023, 0.024] |
| collections | 1.7849 | 0.7020 | 1.7973 | 0.993x [0.988, 0.998] | 0.391x [0.388, 0.394] |
| strings-json | 2.1580 | 0.5935 | 2.2679 | 0.952x [0.949, 0.954] | 0.262x [0.261, 0.264] |
| calls-closures | 3.5944 | 0.5733 | 3.6513 | 0.984x [0.983, 0.987] | 0.157x [0.156, 0.158] |
| adversarial | 1.0617 | 0.3792 | 1.0624 | 0.999x [0.991, 1.012] | 0.357x [0.351, 0.362] |
| float64-dense | 2.9079 | 0.4635 | 0.3719 | 7.819x [7.708, 7.895] | 1.246x [1.229, 1.261] |
| strings-regexp | 19.3842 | 1.8874 | 19.9433 | 0.972x [0.971, 0.974] | 0.095x [0.094, 0.095] |
| arrays-typed | 4.5986 | 0.4417 | 7.0111 | 0.656x [0.654, 0.658] | 0.063x [0.062, 0.064] |
| objects-polymorphic | 6.5235 | 0.6081 | 6.7252 | 0.970x [0.966, 0.973] | 0.090x [0.090, 0.091] |
| calls-recursion-closures | 7.4095 | 1.6019 | 7.6641 | 0.967x [0.964, 0.970] | 0.209x [0.208, 0.210] |
| json-codec | 78.4781 | 10.0503 | 78.9305 | 0.994x [0.990, 0.997] | 0.127x [0.127, 0.128] |
| map-set-bigint | 15.4696 | 2.2103 | 16.3837 | 0.944x [0.941, 0.947] | 0.135x [0.134, 0.136] |
| exceptions-promises-async | 1.9750 | 0.7497 | 2.8484 | 0.693x [0.690, 0.697] | 0.263x [0.261, 0.267] |

The adversarial interpreter comparison is statistically tied: between 0.9%
slower and 1.2% faster. All other displayed intervals exclude parity, but the
cross-engine protocol limitations below preclude claims of peak engine speed.
Fallback-only and slower scenarios are retained in the table.

Environment: Linux x86_64, Intel i7-13700KF, CPU 0 affinity, powersave,
Rust 1.98.0 release; QuickJS-ng `fd0a0210b7be00957751871e7e01b8291268fc29`.
Each scenario/mode has five discarded warmup processes, 30 interleaved fresh
processes, and ten one-second throughput windows. All 3,300 latency samples
have matching checksums across engines. The displayed speed ratios use ratios
of medians with 10,000 paired bootstrap resamples, rather than geometric means.

**Historical protocol limitations:** Bun had one process-internal warmup call;
QuickJS JIT used adaptive readiness/settling. QuickJS timings include Rust-side
lookup/call, checksum conversion and polling; Bun computes its checksum after
timing. A recorded launcher removed the runner's `--smol` flag to use Bun
defaults. These are measurements of that embedding/protocol, not equivalent
peak-throughput measurements. The next matrix must align those boundaries and
warmup policies before setting new Bun-relative optimization targets.

[Full analysis and tier diagnostics](benchmarks/results/main-47aeb11-engines.md),
[raw samples](benchmarks/results/main-47aeb11-engines.json),
[versions, hashes, flags and reproduction](benchmarks/results/main-47aeb11-methodology.json),
[derived data and intervals](benchmarks/results/main-47aeb11-summary.json), and
[benchmark instructions](benchmarks/README.md).

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
