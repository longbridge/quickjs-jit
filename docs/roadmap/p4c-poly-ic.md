# P4c: bounded polymorphic property inline caches (first slice)

Branch `perf/p4c-poly-ic`, based on `82d3808`. Roadmap item: P4 "Tier 2
polymorphic property IC" in
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md).

## What landed

- **Feedback**: `POLYMORPHIC_PROPERTY_LIMIT = 4`
  (`jit/src/runtime/shape_feedback.rs`). A get/put site records up to four
  own-data shapes (prototype-less receivers included, since an own property's
  prototype token is 0) before it turns megamorphic. Both tiers read the same
  constant.
- **Tier 1 (baseline)**: property sites now dispatch inline on the receiver's
  shape pointer and its monotonic layout generation (the exact
  `JS_JitHelperShapeGuard` predicate), up to four shapes, each with its own
  slot offset. The per-site stack cache and the `SHAPE_GUARD` helper crossing
  (frame materialization + O(bytecode) PC validation) are gone. Misses still
  take the exact `GET_PROPERTY`/`SET_PROPERTY` helper edge, never a deopt.
- **Automatic tiering, terminal baseline refresh**: when a function contains a
  reachable opcode outside the Tier-2 vocabulary
  (`ir::optimized_vocabulary_admits`, e.g. `object`/`define_field`/
  `array_from`), Tier 2 can never translate it. Its baseline artifact is then
  refreshed once with the recorded property feedback, as the baseline-only
  policy already did. This is what gives `objects-polymorphic` and the
  `mixed-quotes` workload function property ICs at all under production
  automatic tiering. It never races a queued/compiling/installed Tier-2 job.
- **Tier 2**:
  - Chains accept four shapes. The shape/generation compare is inline for
    both borrowed and frame-materialized receivers (the materialized path used
    to call `SHAPE_GUARD` once per observed shape).
  - Megamorphic `get_field`/`put_field` sites lower to the owning generic
    helper (`GET_PROPERTY` via the audited owned-replace path; new
    `emit_opt_owned_property_store` for `SET_PROPERTY` + `FREE`) instead of
    failing the whole function. A shape miss therefore cannot become a
    deopt/recompile loop. Reentrant getters/setters and a throwing setter are
    covered under stress GC.
  - A shape-guarded site whose receiver is exactly an optimized element load
    is rejected at compile time: optimized `get_array_el` publishes only
    primitives, so such an artifact deopts on every execution (and would then
    strand the function in the slower baseline). Generic sites stay
    admissible.
- **Benchmark**: `benchmarks/scripts/property-polymorphic.js` (registered as a
  non-designated `rquickjs-jit property diagnostics` workload) isolates two
  four-shape read sites on the Tier-2 path.

## Tests

`jit/tests/polymorphic_property_ic.rs` (feedback lattice at four shapes,
Tier-2 CLIF chain without extra helper calls, megamorphic helper lowering,
element-receiver rejection, production Tier 2 four-shape run without deopts,
megamorphic reentrancy/exception run, baseline inline dispatch with zero
`SHAPE_GUARD`/`GET_PROPERTY`/`SET_PROPERTY` on hits, automatic terminal
baseline refresh). Two existing tests were updated to the new contract:
`tier1_calls_properties::baseline_property_cache_hits_inline_and_mutation_misses_exactly`
and `property_specialization::megamorphic_property_site_uses_the_generic_helper_instead_of_guards`.

## Noisy diagnostics (not publishable evidence)

Machine shared by ~16 concurrent agents (i7-13700KF, hybrid cores make
samples bimodal). Automatic mode, median over 7–9 alternating runs of
`median(protocol.warmup_batch_ns[-16:])`; speed = 82d3808 / this branch.
Checksums match the interpreter in every run.

| workload | 82d3808 ms | branch ms | speed | note |
| --- | ---: | ---: | ---: | --- |
| objects-polymorphic | 23.88 | 23.03 | 1.04x | baseline refresh + 4-shape inline IC; still ~0.27x the interpreter (helper-bound, see below) |
| mixed-quotes | 2.60 | 2.65 | 0.98x | tied within noise; cost is the sort-callback entry path (B2) |
| property-polymorphic (new) | 1.88 | 1.95 | 0.96x | 82d3808 rejects the 4-shape site and stays in the interpreter; the branch runs Tier 2 at the monomorphic speed (all-same-shape control: 1.95 ms on both binaries), limited by per-iteration owned `shapes.a` loads |
| property-heavy (control) | 0.099 | 0.098 | 1.01x | unchanged monomorphic Tier-2 path |

## Remaining work

1. **Helper-bound baseline**: `objects-polymorphic` stays ~4x slower than the
   interpreter because every other operation in its loop (object literal,
   `define_field`, `Dup`/`Free` of locals, `call_method`, `get_array_el`) is a
   helper call paying the B1 frame validation. The IC removes 5 of ~20 helper
   crossings per iteration; P1 (O(1) helper validation) is the lever.
2. **Tier-2 vocabulary**: `object`, `define_field`, `array_from`,
   `push_atom_value`, `push_empty_string` keep both target workloads' hot
   functions out of Tier 2 entirely (P2).
3. **Object-valued element loads in Tier 2**: `objects[i].x` needs an owned
   (or borrowed-from-array) heap element result; today it is rejected up front.
4. **Transition stores**: `put_field` that adds a property never records
   feedback (slow path), so Tier 2 queueing waits on it forever
   (`property_feedback_pending`). Recording a transition/generic marker needs a
   C feedback change (reserved patch number 0031) plus a transition IC.
5. **Deopt-to-baseline trap (pre-existing)**: a Tier-2 artifact that deopts
   repeatedly for a non-property reason (e.g. a heap alias copied between
   locals) leaves the function in a baseline that can be slower than the
   interpreter. Raising the limit to four shapes exposes one more shape count
   to this existing behavior (82d3808 shows it with three shapes: an
   array-receiver kernel runs at 8.46 ms vs 1.83 ms interpreted). The
   element-receiver rejection above removes the most common instance.
6. **Megamorphic probe cache**: megamorphic sites use the generic helper; a
   runtime-shared (shape, atom) → slot probe cache would make them cheaper.
