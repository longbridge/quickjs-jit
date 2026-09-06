# JIT performance evidence

The benchmark runner compares fresh-process interpreter, Tier 1, forced Tier 2,
and automatic-tiering runs in round-robin order. It records raw paired latency
samples, throughput windows, phase timings, native-entry counters, memory/code
sizes, checksums, build fingerprints, and host provenance.

Add `bun` to `--modes` for an external comparison using the same scripts,
arguments, warmups, fresh-process pairs, samples, throughput windows, and
checksums. Bun is excluded from rquickjs JIT-entry and ABI/opcode identity
validation. Its native counters are JSON `null` (reported as N/A), while timing
ratios remain comparable. Provenance records Bun's version, resolved path, and
executable SHA-256; `JIT_BENCH_BUN` overrides discovery.

For publishable evidence, build and run the default matrix:

```sh
cargo build --release --manifest-path benchmarks/Cargo.toml --bins
./target/release/jit-bench compare \
  --modes interpreter,tier1,tier2,automatic \
  --output benchmarks/results/comparison.json \
  --report benchmarks/results/comparison.md
```

The default policy is five warmups, 30 fresh-process latency samples, and ten
one-second throughput windows per workload and mode. The JSON records those
actual values. Do not use reduced runs for performance claims.


Forced Tier 2 probes can declare `globalThis.tier2ReadyInstalls` to require a
known publication count before timing. The direct-call probe requires four:
two baseline and two optimizing artifacts. An unrelated blacklisted function
cannot bypass that requirement. Both optimized modes execute quiet warmup
iterations while draining compilation; an explicit forced-Tier2 requirement
also rejects unsettled startup or new compilation during the timed batch.
Unsupported probes without the declaration retain bounded interpreter/Tier 1
fallback. Use the same harness revision for both sides of a runtime comparison.

For a fast functional smoke run of the focused scalar-loop, call-heavy, and
property-heavy cases:

```sh
JIT_BENCH_WORKLOADS=scalar-loop,call-heavy,property-heavy \
JIT_BENCH_WARMUPS=1 JIT_BENCH_SAMPLES=1 \
JIT_BENCH_WINDOWS=1 JIT_BENCH_WINDOW_MS=20 \
./target/release/jit-bench compare \
  --modes interpreter,tier1,tier2,automatic \
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

`generic-call-entry` isolates the generic native-call boundary with a batched
short callee taking `(Int32, Bool)`. The Bool argument excludes the numeric
direct-call signature used by `call-heavy`. The Tier 1 integration test verifies
one `CALL` helper invocation per iteration and native callee entry, so a future
specialization change cannot silently turn this into a direct-call benchmark.
Its checksum is `seed + iterations`. It is non-designated because automatic
mode may demote this call-only workload. Compare it with `call-heavy` and
`fibonacci-recursive` when optimizing entry overhead; zero native entries are
fallback evidence, not an improvement in native entry cost.

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

## Focused engine comparison

Lower latency is better. These are the medians from the publishable sampling
run in [`results/focused-5mode.json`](results/focused-5mode.json).

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
