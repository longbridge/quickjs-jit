# P3b: effectful linked direct calls (first slice)

Branch `perf/p3b-effectful-direct`, based on `82d3808`. Roadmap item: P3
(native calling convention), effectful compiled-to-compiled ABI.

## What this slice does

`lower_target_only_linked_leaf` (`jit/src/compiler/tagged_call_link.rs`)
used to accept only single-block, effect-free Int32/Float64/Bool leaves with at
least one borrowed HeapRef argument. It now also accepts bounded, acyclic
callees that read and write guarded own data fields of borrowed HeapRef
arguments, like `generic-call-fallback`'s `incrementAndRecord`:

```js
function incrementAndRecord(value, enabled, state) {
  state.calls = state.calls + 1;
  if (enabled) return value + 1;
  return value;
}
```

The ABI is unchanged: `(output*, unboxed args...) -> status`. Status `0`
stores the scalar result; status `1` asks the caller to run the original
generic CALL with an untouched frame. No C patch, ABI version or fingerprint
changes.

### Exactness model: a transaction, not a frame

The callee is lowered as a transaction over borrowed arguments:

1. Every speculation happens before the first observable write: Int32
   overflow and `-0`, property shape identity + shape generation, and the
   field's current value tag (which also proves the old value is a primitive,
   so replacing it frees nothing).
2. Guarded field stores are buffered in Cranelift variables (`dirty`, address,
   payload) and committed in program order only at `return`, immediately
   before the result is written.
3. Therefore status `1` always means "no effect happened", and the existing
   caller-side recovery (Baseline: generic CALL helper; Tier 2: deopt at the
   CALL PC) remains exact. No callee frame is ever materialized, so there is no
   `JS_JitInlineEnter`, no `calloc`, and no helper/validation call per access.

Admission (fail-closed, before code generation):

- at most 128 instructions, 64 frame slots, 16 KiB retained bytes, 16 blocks,
  8 buffered stores; no exception table;
- forward-only control flow (every jump/branch target is a later block start)
  and an empty operand stack at every block boundary;
- no field read at a PC after any field store PC (on a forward-only CFG this
  is exactly "no path reads a buffered write"; it also makes argument aliasing
  safe, and same-field stores still commit last-writer-wins);
- each property site has monomorphic feedback that passes
  `eligible_property_observation` (own, writable, non-accessor, no prototype
  dependency) with an Int32/Float64/Bool value; a stored value must have the
  observed representation;
- supported operations: typed constants, arguments, `drop`, `dup`, `swap`,
  Int32/Float64 `add`/`sub`/`mul`, Int32 relational and Int32/Bool strict
  equality, `!`, Int32 `inc`/`dec`/`post_inc`/`post_dec`, `get_field`,
  `get_field2`, `put_field`, `goto`, `if_true`/`if_false` on Int32/Bool,
  `return` of the signature's representation. Everything else rejects the
  linked entry, and the call keeps its previous path.

`lower_direct_call_machine` now receives the callee's `FeedbackSnapshot`
so the linked entry can read property feedback (Baseline and Tier 2 compile
paths, plus the test hooks).

### Coordinator preference

`Coordinator` previously skipped frame-inline planning at a call PC only when
the direct target also carried an inline snapshot. It now also skips it when
the Tier 2 caller can actually take the linked entry there: the linked entry
needs no shadow frame, and the old preference would otherwise route
`incrementAndRecord` through `JS_JitInlineEnter`, as it did before this slice.

`emit_opt_specialized_call` takes a linked entry only for a receiver-free
`call` whose function operand and every HeapRef argument come from frame
reads. `tier2_direct_call_site_usable` (coordinator.rs) predicts that
conservatively by replaying the call's own basic block: only `get_arg*`,
`get_loc*` and `get_loc_check` pushes that no later frame write, closure
variable write or nested call could have changed count as frame reads. Every
other site (callee by global name, closure variable, `obj.fn(...)`, or a HeapRef
argument read from a global) keeps frame inlining; an earlier revision of this
branch sent those sites to the generic CALL (review r1, regression tests
`production_effectful_callee_*` in `jit/tests/tagged_call_link.rs`). The
prediction can under-approximate, which only keeps the pre-linking frame
inline.

### Tier 2 queueing order (`jit/src/lib.rs`)

The interpreter records no property feedback, so the callee's Baseline
artifact normally has no linked entry. Only its Tier 2 artifact does. Without
another change, the caller was often queued for Tier 2 in the same scan as the
callee, or before the callee's Tier 2 install. It then captured a frame
inline of the callee's Baseline body and never re-linked. In a noisy check, 2
of 7 processes stayed on the old ~9 ms path. Two changes fix this:

- a Tier 2 scan queues callees before their callers (sorted by the number of
  ready callees);
- a caller whose loop call site is frame-inline-ready but not yet
  direct-ready waits while the callee's optimizing compile is `Queued`,
  `Compiling` or `Ready` and the callee has a bounded signature. The wait
  ends when that compile installs or fails, so it is bounded. The wait also
  requires `tier2_direct_call_site_usable`, because other sites are
  frame-inlined anyway.

After this, 12 of 12 processes linked the edge.

## Tests

- Unit (`tagged_call_link::tests::effectful`), against fake object memory
  using the linked property layout: commit on both branches; late `value + 1`
  overflow and counter overflow miss with the field untouched; shape
  identity/generation and field-tag misses write nothing; aliased arguments
  commit in program order; `state.calls++` read-modify-write; a read after a
  store is rejected.
- Integration (`jit/tests/tagged_call_link.rs`, Baseline, forced Tier 2, and
  forced Tier 2 with GC stress): no generic CALL on the stable edge; exact
  results and effects for result overflow, counter overflow, Float64 and
  string fields, a different receiver shape, a newly added field, a read-only
  field (the generic `put_field` throws `TypeError`, nothing is written), and
  an accessor replacement. A mutation check (dropping the commit store) makes
  all three integration tests fail, so they do exercise the linked entry.

## Remaining work

- **`mixed-quotes` / `compareQuotes`**: the comparator is called by
  `Array.prototype.sort` from C, not from compiled code, so no linked call edge
  exists. `compareQuotes(left, right){return right.score - left.score}` already
  fits this ABI (two HeapRef loads, Int32 result). Using it needs a C-side
  fast entry: a cached per-function linked entry that `js_array_sort` (and
  `map`/`forEach`) can call before `JS_Call`, cleared on artifact retirement.
  That is the P1 "C-side tier state" / P3 "fast entry from C" work and needs a
  new patch (reserved number `0030`), an ABI minor bump and a retirement
  protocol. It was not done in this slice to avoid overlapping the P1 changes
  to the same C call path.
- Tier 2 does not compile a callee containing `get_field2` + `post_inc`
  (`state.calls++`), so under forced Tier 2 such a callee publishes no linked
  entry. In production the callee's Baseline artifact provides it.
- Persistent misses (for example a permanent shape change) cost a Tier 2 caller
  one deopt per call until the existing guard-failure demotion moves it to
  Baseline, where a miss costs only the guard loads before the generic CALL.
  Recompiling the callee's linked entry from newer property feedback would
  remove this.
- Not yet admitted: field loads after stores (needs store-to-load forwarding
  plus alias checks), heap-valued fields and heap results (needs ownership
  transfer), locals, loops (needs polls), nested calls, `this`, Float64
  comparisons, and polymorphic property sites.
- Known divergence (shared with the earlier effect-free leaf): the linked leaf
  skips the interpreter's call-entry stack-overflow and interrupt checks. An
  effectful leaf can therefore commit its field writes where the interpreter
  would have thrown `RangeError` at call entry. The leaf is bounded and
  loop-free, so the window is one call deep.
- The scheduler wait still covers callees that can never publish a linked
  entry (they use `this`, locals, loops or calls); gating it on the linked-leaf
  admission prepass would avoid delaying those callers. Transitive chains
  (A -> B -> leaf) and mutual recursion under the wait are not tested.
- A real native calling convention (P3 proper) with lazily materialized frames
  is still required for recursion and general callees. This slice avoids any
  frame by keeping the callee transactional.
