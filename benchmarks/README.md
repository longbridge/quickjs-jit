# JIT performance evidence

The benchmark runner compares fresh-process interpreter, Tier 1, forced Tier 2,
automatic tiering, and Bun runs in round-robin order. It records raw paired
latencies, process-throughput windows, phase timings, counters, memory/code
sizes, checksums, build fingerprints, and host provenance.

Current samples identify their timing protocol as
`protocol.name = "shared-js-fixed-warmup-v2"`. The JSON envelope remains
`jit-benchmark-v1`, but these timings are **not equivalent to historical v1
measurements**. The legacy `jit-bench-report` gate reporter explicitly rejects
v2 samples as an incompatible timing protocol. Do not pass `--report` when
collecting v2 evidence or interpret that rejection as a runtime test failure.

Build and collect the complete five-mode raw matrix:

```sh
cargo build --release --manifest-path benchmarks/Cargo.toml --bins
./target/release/jit-bench compare \
  --modes interpreter,tier1,tier2,automatic,bun \
  --output benchmarks/results/comparison-v2.json
```

Bun uses its default engine configuration: the runner no longer adds `--smol`.
`JIT_BENCH_BUN` selects an alternative executable. Comparison-file provenance
records its version, resolved path, and executable SHA-256. Bun's native
counters in comparison samples are JSON `null` (N/A). Raw worker output keeps
zero placeholders in the legacy cumulative counter fields; these are not Bun
execution-tier measurements.

For an external paired baseline/candidate collector, each command below emits
one worker JSON object. Build both runtime revisions with the exact same
benchmark harness and scripts, and retain immutable binaries with their hashes:

```sh
./target/release/jit-bench worker --mode automatic --script benchmarks/scripts/mixed-quotes.js
./target/release/jit-bench worker --mode interpreter --script benchmarks/scripts/mixed-quotes.js
./target/release/jit-bench worker --mode bun --script benchmarks/scripts/mixed-quotes.js
```

Worker commands do not run the comparison collector's sample validation.
External collectors must check checksums, protocol identity, script/driver
hashes, sampling counts, and native-counter consistency themselves, and record
engine identities and host/build provenance.

## Fixed-warmup timing and readiness diagnostics

Every engine executes one first call, then exactly 64 warmup batches of ten
calls, then one measured batch of ten calls. The shared `batch-driver.js` uses
identical workload arguments and stores every result. Promise results complete
sequentially before the batch ends. Primitive checksums are generated after
timing, including during warmup, and combine all ten results. The
`protocol.warmup_batch_ns` array records the 64 consecutive warmup timings.

`elapsed_ns` is the **fixed-warmup batch latency**, not a claim that compilation
has settled. `protocol.fixed_metrics_before` and `fixed_metrics_after` surround
that measured batch and exclude checksum execution. Derive native, Tier 2,
deopt, fallback, compilation, and installation deltas from these snapshots.
Counters aggregate the shared driver and workload; they do not identify the
native tier of an individual workload function. Bun has no equivalent internal
metrics and reports these snapshots as `null`; interpreter workers have no
attached JIT and also leave them `null`.

QuickJS still times one outer Rust `Function::call` and, for asynchronous work,
its Promise completion bridge. Bun times the corresponding JS call and await.
Both timings include driver array/closure work. The protocol removes the old
per-workload Rust lookup/checksum/poll loop from the measured region, but does
not eliminate the outer host-call difference. The driver is part of the
measured workload; these are not isolated function-body timings.

After collecting the fixed-window sample, QuickJS runs the existing bounded
JIT readiness and settling checks, then another shared batch. Its timing is
stored in `protocol.readiness_diagnostic_ns` and `phases.steady_state_ns`.
Bun has no separately conditioned readiness diagnostic. Do not divide Bun's
fixed-window latency by QuickJS's post-readiness diagnostic and call that a
matched steady-state comparison. Legacy cumulative counters include first
execution, warmup, the fixed sample, and subsequent diagnostics; they cannot
prove native execution or settled compilation during the fixed window.

Forced Tier 2 probes can declare `globalThis.tier2ReadyInstalls` to require a
known publication count in the readiness diagnostic. The direct-call probe
requires four: two baseline and two optimizing artifacts. An unrelated
blacklisted function cannot bypass that requirement. Explicit forced-Tier2
requirements reject unsettled diagnostic startup or new compilation during
the diagnostic batch. Those checks happen after the fixed-window sample and
do not make its warmup policy variable. Unsupported probes retain bounded
fallback/readiness behavior.

The default process policy is five discarded warmup processes, 30 retained
fresh-process latency samples, and ten one-second process-throughput windows
per workload and mode. `raw_throughput_ops` counts fresh worker completions,
including startup, warmup, and diagnostics; the last worker can finish after
the target window duration. It is **not JavaScript operations per second**.
JS throughput needs a separate in-process measurement. Reduced runs are
functional smoke evidence only.

For a fast functional smoke run of the focused scalar-loop, call-heavy, and
property-heavy cases:

```sh
JIT_BENCH_WORKLOADS=scalar-loop,call-heavy,property-heavy \
JIT_BENCH_WARMUPS=1 JIT_BENCH_SAMPLES=1 \
JIT_BENCH_WINDOWS=1 JIT_BENCH_WINDOW_MS=20 \
./target/release/jit-bench compare \
  --modes interpreter,tier1,tier2,automatic,bun \
  --output /tmp/rquickjs-jit-smoke.json
```

The environment overrides must be positive integers. `JIT_BENCH_WORKLOADS` is
a comma-separated list of exact workload names. A reduced run remains useful
for checking checksums, tier counters, and harness behavior, but is not
statistically meaningful.

Scripts receive exactly `(iterations, seed)` unless they define
`globalThis.workloadArgument`; only then does the harness pass a third
argument. This preserves the declared two-argument signature required for
bounded Int32/Float64 specialization while still supporting focused
call/property workloads that need a stable object or callable input.

The focused Fibonacci pair separates loop optimization from call support:
`fibonacci-iterative` computes bounded-Int32 fib(40) as a loop/Phi probe;
it is a designated compute kernel and must enter its requested native tier.
`fibonacci-recursive` computes fib(20) as a non-designated call-path probe; its
zero-entry samples remain visible as an explicit fail-closed gap instead of
being reported as native performance.

`generic-call-entry` preserves the original short `(Int32, Bool)` branch-leaf
probe and its historical checksum, `seed + iterations`. New mixed-Bool direct
specialization can optimize this callee; its timing no longer proves that the
generic boundary itself became cheaper. The original script stays unchanged.

`generic-call-fallback` separately preserves a genuine generic CALL path. Its
callee accepts `(Int32, Bool, Object)` and increments an observable property,
which excludes the pure direct-leaf ABI. A stable object supplies the target and
counter; each workload invocation resets the counter, then consumes both the
returned values and mutations as `seed + 2 * iterations`. This includes property
access cost and is not interchangeable with the original branch-leaf probe.
Compare this script against itself across baseline, candidate, and default Bun,
using identical inputs and the shared driver. Both call probes are non-designated;
zero native entries remain fallback evidence. Future ABI expansion must retain
an explicitly verified generic-boundary probe and report its actual call path.

New evidence includes `native_acquisitions`: successfully acquired backend
handles, including OSR. Native executions that reuse a C entry handle increase
`native_entries` without another acquisition. Older reports omit this optional
counter; Bun reports it as `null`.

## Representative performance matrix

The default run also covers the broader JavaScript surface below. Every case
returns a deterministic primitive checksum and uses the same fresh-process
sampling in QuickJS interpreter, Tier 1, Tier 2, automatic, and optional Bun
modes. These scenarios are intentionally non-designated: native entries,
fallbacks, retries, and deoptimizations stay visible, without treating a
fallback-only result as proof that a JIT tier supports the feature.

| Workload | JavaScript behavior exercised |
| --- | --- |
| `mixed-quotes` | One pure JS shell kernel: 96 objects, 32-step quote scoring, sorting, and top-12 result consumption |
| `float64-dense` | Dense Float64 arithmetic, `sin`, `cos`, and `sqrt` |
| `strings-regexp` | String construction, slicing, RegExp capture and replacement |
| `arrays-typed` | Packed-array growth/traversal and Int32Array/Float64Array traffic |
| `objects-polymorphic` | Allocation, property reads/writes, and four stable shapes |
| `calls-recursion-closures` | Four-deep calls, bounded recursion, and mutable closure capture |
| `json-codec` | Repeated nested JSON encoding and decoding |
| `map-set-bigint` | Map and Set mutation/iteration plus bounded BigInt arithmetic |
| `exceptions-promises-async` | Throw/catch, Promise jobs, async functions, and continuations |

Select the matrix with `JIT_BENCH_WORKLOADS` for smoke or publishable runs; the
sample/warmup/window rules above remain unchanged.

`mixed-quotes` adapts the scoring and comparator from gpui-kit's
`MIXED_MARKET_TEMPLATE`. Each workload call constructs and sorts one set of 96
quotes and returns the total plus the top 12 indices/scores. It intentionally
excludes GPUI element/snapshot construction and host validation. Compare its
Bun timing only with the same pure JS kernel, and keep the actual shell
snapshot regression test as separate host evidence.

## Historical focused engine comparison

The following table is historical evidence measured at source
`fda642b68b07cc523c0564f1ad51e5f9043331eb` (dirty tree), using the older timing
protocol. It is not current performance or a v2 baseline. The JIT column below
is forced Tier 2; automatic-mode results are discussed separately. Lower
latency is better. These are the medians from [`results/focused-5mode.json`](results/focused-5mode.json).

| Scenario | QuickJS | QuickJS + JIT | Bun | QuickJS vs JIT | JIT vs Bun |
| --- | ---: | ---: | ---: | ---: | ---: |
| Scalar loop | 836.946 us | 25.837 us | 12.983 us | JIT 32.39x faster | JIT 1.99x slower |
| Numeric loop | 823.818 us | 25.715 us | 12.358 us | JIT 32.04x faster | JIT 2.08x slower |
| Iterative Fibonacci | 33.780 ms | 753.907 us | 1.093 ms | JIT 44.81x faster | JIT 1.45x faster |

Automatic tiering rejected Tier 2 for these kernels after five bounded
profitability trials and unpublished the harmful Tier 1 artifact. Its medians
were 1.212 ms (scalar), 1.223 ms (numeric), and 57.692 ms (Fibonacci), versus
the much slower forced-Tier-1 medians of 8.122 ms, 8.144 ms, and 1.132 s.
After demotion, native-entry counters stop increasing and execution remains in
the interpreter; the remaining automatic-mode overhead is the attached
runtime's feedback callback boundary.

Methodology: Intel Core i7-13700KF, Linux 7.1.9-arch1-2 in `powersave` mode,
rustc 1.98.0/LLVM 22.1.8, pinned QuickJS revision
`fd0a0210b7be00957751871e7e01b8291268fc29`, and Bun 1.4.0 (binary SHA-256
`33d56b070be6a9e3da0ab013038b43d1645d0534ca811ecdba4472599117eb4b`).
Each latency is the median of 30 interleaved fresh processes after five warmup
processes; the evidence also contains ten independent one-second throughput
windows per engine and scenario. Iterative Fibonacci computes `fib(40)` 2,000
times inside JavaScript and returns `102334155` (canonical checksum
`number:419865fb2c000000`) in every engine. The recorded source tree is dirty
because the benchmark measures the implementation under review; exact source,
suite, schema, executable, and Bun hashes are retained in the JSON provenance.

## Real gpui-shell acceptance

`jit/tests/gpui_shell_surface.rs` is a compatibility fixture. It exercises a
small mirror of the shell's QuickJS call surface, but it is not evidence that
the real `gpui-shell` runtime is integrated or faster. Run the external
acceptance only against the sibling application's actual `crates/shell`:

```sh
scripts/bench-gpui-shell.sh ../gpui-component \
  target/gpui-shell-jit-report.json
```

The command intentionally fails before running if the shell does not own a
`JitRuntime`, expose the native `quickjs-jit` feature, and emit paired evidence
from its real `#[gpui::test]` panel benchmark. A compliant benchmark writes
`gpui-shell-jit-v1` JSON to `GPUI_SHELL_JIT_REPORT`. The report contains five
or more discarded warmup processes and 30 fresh-process pairs for each mode;
every pair records the real snapshot SHA-256, script-render count, checksum,
steady-state script time, P99 script-render latency, native entries, and
fallbacks for both the host-heavy panel and a render-driven numeric layout
kernel. It also records paired first-window and hot-reload samples.

`jit-gpui-shell-report` rejects mismatched snapshots, render counts or
checksums, dirty/incomplete provenance, missing native execution, a lower 95%
confidence bound below 2x for any workload marked suitable for JIT, or an
upper regression bound above 5% for the host-heavy panel, P99, first-window, or
hot-reload latency.
The script writes the rendered verdict beside the JSON as `.md`; no fixture
result can make this acceptance gate pass.
