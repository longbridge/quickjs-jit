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
  - Some access of the receiver must dominate every latch of the loop, so
    a conditionally used receiver (`if (c) out[i] = v`) is not guarded on
    loop entry, where a mismatch would deopt on every entry.
  - The preheader runs one storage query, without the length flag.
  - Each access keeps its own index check.
- **Poll revalidation of hoisted tuples (review fix r1).** Every loop poll
  that reaches the runtime revalidates the hoisted tuples (length and
  storage) of *every* loop that contains it, not only those of its own
  header. An outer header re-seeds its preheader tuple on each backedge
  and, with amortized polling, an inner loop consumes nearly every
  countdown expiry. Before this fix, an interrupt handler that detached an
  outer receiver's buffer at an inner poll let the outer loop keep reading
  and writing the freed backing store. Loop admission now also rejects any
  safepoint other than a loop poll, and lowering fails closed when an
  enclosing header's tuples are not seeded exactly as planned. The length
  hoist of an argument receiver had the same flaw at `82d3808`, and this
  fix covers it too.
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
    shape-guarded `get_field` and other certified leaf sites. An
    Object-observed `get_field` goes through the generic bridge, which can
    run a getter. It counts as frame-preserving only when the function
    contains no `fclosure`, `special_object` or direct `eval`: the only ways
    script could write this frame. Lowering still
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

## Review fixes (r1)

- The hoist-cap check now runs before any access is marked covered. When a
  loop exceeds `MAX_LIVE_SOURCES` receivers, the extra receivers keep every
  index guard; the rest of the plan is not discarded.
- A pre-existing bug, reproduced at `82d3808`, is fixed. The numeric guard
  at the loop header and at entry exited without republishing the stack top.
  After `const out = o.out` had published a one-slot top through the owned
  property bridge, a failing header guard (for example on a `null` local)
  produced an exit that the runtime rejected as "invalid native
  deoptimization metadata". That is an uncatchable error for the script.
  The guard exit now republishes an empty stack. Lowering rejects a loop
  poll at a non-empty depth.
- New tests, in `typed_receivers.rs` unless noted:
  - `outer_loop_hoists_are_revalidated_by_inner_loop_polls`: the
    reviewer's store/UAF case with a detached destination or source, plus
    the argument-receiver length hoist.
  - `conditional_and_wide_receiver_loops_match_the_interpreter`: no
    deopts on entry for an unused conditional destination, and more than
    four receivers.
  - `loop_header_guard_exit_republishes_an_empty_stack`.
  - `array_cache` unit tests for enclosing-poll revalidation, conditional
    storage hoists and the live-tuple cap.

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

### After the r1 review fixes

The r1 run used the same method, with 9 rotated-order rounds. `prev` is
the implementer's commit `e243f15`.

| Workload | 82d3808 ms | e243f15 ms | r1 ms | Speed vs 82d3808 | Speed vs e243f15 |
| --- | ---: | ---: | ---: | ---: | ---: |
| float64array-traversal | 1.0940 | 0.1642 | 0.1690 | 6.47x | 0.97x |
| arrays-typed | 2.9781 | 1.9978 | 1.9953 | 1.49x | 1.00x |
| int32array-traversal (control) | 0.0498 | 0.0497 | 0.0500 | 1.00x | 0.99x |
| float64-dense (control) | 0.1291 | 0.1305 | 0.1288 | 1.00x | 1.01x |

float64array-traversal is sensitive to code layout. Across r1 builds that
differ only in cold deopt blocks, it measured between 0.93x and 1.19x the
speed of e243f15. One early build made the entry and header numeric-guard
exit use the entry `stack_base` SSA value. That build was about 7% slower
on int32array-traversal, because it kept the value live across the loop.
The exit now reloads the base from the frame and is marked cold.

### r1 follow-up (fail-closed bounds hoist use)

- A `get_array_el` skips its bounds check for a hoist only when the operand's
  provenance names the planned receiver, which is when `expected_mode` is
  kept. When the provenances differ, a cached tuple for the operand's slot
  belongs to another receiver, so the load keeps its own bounds check. Test:
  `argument_aliased_local_receiver_loads_match_the_interpreter`. Today Tier 2
  rejects the `const a = arg` shape as an unsupported opcode and fails
  closed, so the test checks interpreter equality in whichever tier runs it.
- `outer_loop_hoists_are_revalidated_by_inner_loop_polls` was checked
  against a build that revalidated only the innermost loop's tuples. That
  build fails the test, so the test covers the blocking review defect.
- Noisy diagnostic after this change, 7 alternating runs against the `82d3808`
  `jit-bench`: float64array-traversal 1.0985 ms to 0.1693 ms, 6.49x the
  baseline speed; arrays-typed 2.9716 ms to 2.0081 ms, 1.48x; the
  int32array-traversal control 0.0497 ms to 0.0501 ms, 0.99x, which is a tie.
  Checksums match the interpreter mode.
- `mixed_direct_calls::tier2_mixed_edge_eliminates_helper_and_preserves_guard_misses`
  is flaky under load. It failed 6 of 30 runs on an `82d3808` build and 5 of
  30 on this branch. The failure is timing dependent and predates this
  branch.

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
