# M1 strings and element-loop optimization summary

## Scope completed

This milestone advances the first two items in the post-merge optimization
backlog.

1. Tier 1 now reaches atom-backed string constants and the remaining object and
   element property-construction paths needed by the string/JSON cases. The
   QuickJS patch exposes `push_atom_value` through the versioned JIT ABI and
   snapshots numeric constant payloads without retaining heap pointers.
   Differential coverage exercises native entry, forced cycle collection,
   exceptions, computed fields, object properties, and JSON global lookup.
2. Tier 2 feedback and SSA now admit stable object arguments alongside numeric
   loop state. Packed arrays, `Int32Array`, and `Float64Array` have native
   length, element-load, and element-store lowering. Stable class, fast-array,
   data, count, and detached/resizable-buffer guards established at a loop
   header are reused only within the producing block or its unique direct
   successor for an immutable argument. Local reassignment, CFG merges,
   reentrant effects, and element stores invalidate the fact. Dynamic bounds
   checks remain where no range proof exists. Wrong classes, Proxy objects,
   unsupported values, and numeric mismatches deoptimize to exact interpreter
   state.

The constant descriptor ABI is extended append-only with a 64-bit payload and
the ABI minor version is 18. Bundled bindings for all supported targets were
regenerated consistently.

## Correctness and machine-code evidence

The optimized-IR tests assert native packed and typed-array loads/stores and
guard placement. Production tests execute packed-array and typed-array loops
with native Tier 2 entries and zero deoptimizations on stable input, then check
exact results after Float64 and Proxy side exits. Tier 1 differential tests
cover string/property ownership, GC, and exception behavior.

The full runtime test command passes:

```sh
cargo fmt --all -- --check
cargo test -p quickjs-jit-runtime --features compiler,test-support --tests
```

This includes 48 optimized-code tests, 21 runtime-feedback tests, bounded
Test262 runs in interpreter/automatic/eligible forced tiers, and the native
array/typed-array integration tests.

## Reproducible performance result

The checked-in [raw evidence](../benchmarks/results/m1-arrays-typed.json) and
[generated report](../benchmarks/results/m1-arrays-typed.md) were collected
from clean source commit `63dc71666aa564bd11429709f8e1a255719b1777` using the
repository's full policy: five warmups, 30 paired fresh-process samples, ten
one-second throughput windows, and 10,000 paired bootstrap resamples.

For `arrays-typed`, forced Tier 2 has a median steady-state latency of
3,379,947 ns versus 4,654,366 ns for the interpreter, a 1.38x speedup. Across
the 30 samples it records 687 Tier 2 entries, zero deoptimizations, zero
fallbacks, and the identical checksum
`string:33983000:8496750.000:2000`.

This is substantive progress for the element-loop path, but it is not the
project-wide performance acceptance result. The report remains
FAIL/INCONCLUSIVE because `arrays-typed` is not a designated 10x compute
kernel, automatic mode does not yet publish a profitability decision for this
workload, startup/tail gates regress, and gpui-shell evidence is outside this
run. The one-second throughput windows also remain worse than the interpreter
(median 18 versus 126 operations), showing that compilation/entry overhead
still dominates repeated whole-workload invocations even though the measured
Tier 2 steady-state body is faster. These limitations are deliberately retained
in the report rather than converted into a passing claim.

## Follow-up

- Teach automatic profitability selection to retain the profitable Tier 2
  array version without the current attached-runtime overhead.
- Amortize compilation and native-entry costs across repeated workload calls;
  then rerun the full matrix and gpui-shell acceptance suite.
- Add range proofs for loop induction variables before eliminating the dynamic
  bounds checks that remain in otherwise-hoisted element loops.
