# P2d: Tier 1 iteration opcodes (first slice)

Branch `perf/p2d-iteration`, based on `82d3808`. Roadmap item P2.5
(`for_of_*`/`for_in_*`/`iterator_*`) from
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md).

## What landed

- QuickJS patch `0028-tier1-iteration.patch` (ABI 1.25, runtime API 1.10):
  - Append-only helper `ITERATOR_OP` (helper ID 24). It runs the interpreter's
    own stack effect (`js_for_of_start`, `js_for_of_next`, `js_for_in_start`,
    `js_for_in_next`, and the `iterator_close` sequence) directly on the
    materialized native frame. Iterator objects, `next` methods and the for-of
    catch offset stay C-visible across user iterator code and GC. On an
    exception, the helper leaves exactly the stack that the interpreter's
    unwinder expects. That includes the catch offset, which closes the
    iterator with a throw completion. `iterator_close` reports its exception
    at `depth - 2`, as the interpreter does.
  - Versioned leaf table `JS_JitGetIteratorAPI(1)` with `array_values_next`.
    On every step it revalidates four conditions: the live iterator is an
    Array values iterator, its `next` is the exact built-in C function, the
    target is a fast `Array`, and the index is below the dense count. When all
    four hold, it advances the index and returns a duplicated element. It
    performs no allocation, throw, finalization or reentry. Any other state
    misses without touching anything.
- Tier 1 admits `for_of_start`, `for_of_next`, `for_in_start`, `for_in_next`
  and `iterator_close` (`HelperId::IteratorOp`). `for_of_next` calls the leaf
  first and takes the helper only on a miss, which includes completion. A
  dense packed-array loop therefore makes three helper calls in total:
  start, completion and close.
- Because the leaf revalidates on every step, no pristine
  `Array.prototype[Symbol.iterator]` or `%ArrayIteratorPrototype%.next`
  watchpoint is needed. A replaced protocol is observed on the next step, and
  no deopt path exists to get wrong.
- Verifier:
  - `for_of_next`/`for_in_next` keep the enumeration record's slot kinds,
    including `CatchOffset`.
  - A loop-scoped lexical local now joins `Uninitialized` with
    `Tagged`/`Int32`/`Float64` as `Tagged`. Every read of such a local is a
    checked `get_loc_check`.
  - OSR validation of a `CatchOffset` slot now checks `JS_TAG_CATCH_OFFSET`.
    The old code checked `JS_TAG_INT`, which was a latent bug that never ran
    because for-of was rejected.
- New scenario `for-of-array` (for-of sum over a cached packed array) in
  `benchmarks/run.rs`. The README matrix is not updated; that is left to the
  integrator.

## Deliberately not in this slice

- `iterator_next`, `iterator_call` and `iterator_get_value_done` are only
  emitted in generator (`yield*`) and async (`for await`) functions. Those
  functions are already rejected with `Generator`/`Async`, so these opcodes
  stay policy rejects.
- `return` inside a for-of emits `nip_catch` (`ExceptionRegion`), so such
  functions stay in the interpreter. Supporting it needs static tracking of
  catch-offset positions, which fits naturally with P2.3 (try/catch).
- Array destructuring with `...rest`, or with a numeric default (`[a=7]`),
  still fails the verifier's exact operand-stack merge (Int32 vs Tagged).
  This limitation predates this work.
- Tier 2 rejects any loop header with a non-empty operand stack, so for-of
  and for-in loops stay in Tier 1.

## Remaining work and measured limits

- `collections` shows no steady-state change. Its for-of loop alone is about
  3x faster (see `for-of-array`), but the same function's `values.push` loop
  is slower in Tier 1 than in the interpreter: every Tier 1 generic call pays
  the helper-frame validation and call bridge (roadmap B1/B2). In the current
  profitability model, the automatic tier then demotes the function to the
  interpreter after five rejected trials. That demotion happens to be the
  right call today: forcing it off measured `collections` at 0.65x.
  `collections` gains once P1 (O(1) helper validation, cached native entry)
  or P3 lands, or once Tier 2 accepts stack-carrying loop headers. The
  enumeration record could be kept in SSA values with lazy materialization.
- `map-set-bigint` iterates `map.values()`, a Map iterator. Only Array values
  iterators have a leaf; Map/Set leaves would follow the same pattern
  (`js_map_iterator_next`).
- The JSON and string workloads do not use for-of in their kernels.
- Merge risk: helper IDs and runtime API fields are append-only, and 0028
  hunks use exact line numbers, so parallel ABI patches conflict. Rebase by
  regenerating 0028 on top of the merged patch set, renumbering the helper ID
  and the `JSJitRuntimeAPI` offset (currently 200, size 208), and updating
  the digests in `sys/build_support/patch.rs`.
