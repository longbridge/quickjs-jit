# P1 item 6: typed-array receivers held in locals

Branch `perf/p1d-typedarray-receivers`, based on `82d3808`. This note covers
roadmap item P1-6 (B5 in
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md)).

## Problem

`float64array-traversal` reads both views from `buffers.ints` and
`buffers.floats` into `const` locals. `ArrayPlan` accepted only receivers
whose SSA identity was an entry argument. As a result, the 0020 typed-array
leaf query was never used for this loop. Every iteration ran `ints.length`
through `JS_JitHelperGetProperty` and the intrinsic getter, and ran a leaf
query for each `floats[i]` store. Poll amortization was also disabled, so
the loop called the poll helper on every iteration.

## What changed

- **Receiver identity.** A candidate receiver is now
  `ArrayReceiver::{Argument, Local}`. A local is accepted only when both of
  these hold:
  - It is the single frame slot whose SSA value matches the operand at the
    access (`KnownFacts::same_value`). Value identity is followed through
    Phis that agree and through frame reads kept by the guarded-site
    contract.
  - No node in the loop writes that local.

  Aliased receivers, receivers also held in an argument, and rebound
  receivers are not planned.
- **Typed `get_length`.** A planned Int32/Float64 length is lowered to the
  versioned leaf query with `JS_JIT_ARRAY_QUERY_LENGTH`. The query checks
  the intrinsic getter chain and a fixed, attached backing store. If the
  check fails, the code takes an exact deoptimization exit before the
  bytecode runs. It never falls back to the bridge that can run an
  accessor.
- **Several live metadata tuples.** Lowering keeps up to four guarded
  tuples, keyed by frame slot. The old code kept a single slot.
  - Every reentrant or frame-writing boundary still clears all tuples.
  - Tuples survive forward joins only when every predecessor ends with the
    same tuple. The shared Cranelift values then dominate the join.
  - This lets the `floats[i]` store and the following load share one
    tuple, while the `ints` tuple stays live for the same iteration.
- **Storage-only hoists.** Consider an Int32/Float64 receiver that is used
  in a loop, has no hoisted length, and passes the existing loop admission
  (no uncertified reentry and no write to its slot). An example is a store
  destination. For such a receiver:
  - The preheader runs one storage query, without the length flag.
  - The query is revalidated on every poll, both amortized and regular.
  - Each access keeps its own index check.
- **Leaf certification.**
  - Numeric `push_const`/`push_const8` sites are treated as non-reentrant.
    Lowering can only emit immediates for them; any other constant fails
    compilation.
  - Guarded `to_propkey` sites are also accepted for poll amortization.
- **Poll amortization.** Two narrow extensions:
  - A store into an *owned* (helper-derived heap) local outside every
    natural loop is accepted. The loop-header representation guard already
    treats owned locals as `Any`.
  - The numeric proof used only for this decision keeps frame reads across
    shape-guarded `get_field` and other certified leaf sites. Lowering still
    uses the original proof.

All exits remain exact deoptimizations at the original bytecode.

## Tests

`jit/tests/typed_receivers.rs`:

- CLIF test for the traversal:
  - no `length` bridge call;
  - exactly two preheader queries and two cold poll revalidations;
  - no bounds check on the range-proven `ints` load.
- Rebound and aliased receivers are not hoisted.
- Production Tier 2 run with stress GC: results match the interpreter, and
  `GET_PROPERTY` is no longer called on every iteration.
- 17 changed-receiver probes, compared with the same-version interpreter:
  - wrong mode, and packed or plain-array receivers;
  - a destination shorter than the source;
  - own `length` override and prototype getter override;
  - detached source or destination;
  - one view over a shared buffer aliasing the other;
  - the same object as both receivers;
  - an accessor on `buffers`;
  - a string seed.
- The source or destination buffer is detached from an interrupt handler
  during the native loop. This covers poll revalidation of local receivers.

`array_cache` unit tests cover:

- a local candidate with its length hoist;
- rejection of rebound, aliased and argument-aliased locals;
- a storage-only hoist for a store destination.

## Noisy diagnostic (not publishable evidence)

This was measured on a shared, loaded 24-core x86_64 host. Each row is 7
alternating runs of the `82d3808` baseline `jit-bench` and this branch,
using `worker --mode automatic`. Each value is the median over runs of
`median(protocol.warmup_batch_ns[-16:])`, in ms per 10 workload calls. Speed
is baseline / new, so values above 1.00x are faster. Checksums match the
interpreter mode.

| Workload | Baseline ms | Branch ms | Speed |
| --- | ---: | ---: | ---: |
| float64array-traversal | 1.0922 | 0.1574 | 6.94x |
| arrays-typed | 2.9833 | 2.0123 | 1.48x |
| int32array-traversal (control) | 0.0497 | 0.0495 | tied (bimodal E/P-core samples) |
| float64-dense (control) | 0.1296 | 0.1309 | tied |

In a 3-run spot check, property-heavy, call-heavy, numeric, scalar-loop,
objects-polymorphic, mixed-quotes, quickjs-int-arith, fibonacci-iterative,
generic-call-entry and scalar-expressions were all within ±7%. Reruns of
quickjs-bitops and host-compute with 7 runs were bimodal ties. The
publishable three-engine matrix, with Bun and confidence intervals, is still
pending and belongs to the integrator.

## Remaining work

- float64array-traversal takes about 7.9 ns per iteration against about
  4.1 ns for Bun (roadmap section 3). What is left:
  - the `floats` store and the reload after it each keep a bounds check;
  - the amortized poll counter;
  - the boxed 16-byte `sum` value.

  These belong to P4 bounds-check elimination and store-to-load forwarding.
- arrays-typed is now limited by its `workload` orchestration: `push`,
  `new Int32Array(values)` and `toFixed` stay in the interpreter or in
  helpers.
- Pre-existing case: `const a = arg; a[i] = v` plans the receiver as
  `Argument` because its SSA identity is the entry argument, while the
  operand provenance is `Local`. The typed-store provenance check therefore
  fails compilation, which is fail-closed. Loads and lengths no longer
  depend on provenance matching. Stores could use the same
  operand-guarded query.
- Packed-array local receivers go through the same planner path. One
  interpreter-differential test covers them: snapshot and live length,
  holes, a typed receiver, overflow, an extended length and non-Int32
  elements. None of their CLIF shape is asserted yet.
