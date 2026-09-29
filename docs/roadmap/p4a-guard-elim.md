# P4a: guard elimination on guarded property loops (first slice)

Branch `perf/p4a-guard-elim`, based on `82d3808`. Roadmap item: P4 in
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md) (redundant
guard elimination, LICM, load/store forwarding on semantic SSA).

## Starting point

`#29` already promoted monomorphic primitive own-data fields of an argument
receiver into SSA (`ir/property.rs`, `compiler/optimized/property_cache.rs`):
the shape+generation guard runs once, later accesses are forwarded from the
cached value, stores are sunk until a flush boundary, and the cold amortized
poll publishes, invalidates and revalidates the cache. The `property-heavy`
loop was therefore already free of field loads and shape guards. The remaining
per-iteration cost came from elsewhere in the same loop:

1. Every guarded property leaf re-published the operand stack into the
   interpreter frame (placeholders, primitives and `stack_top`): about 16
   memory stores per `property-heavy` iteration.
2. Frame reads across property operations were treated as reentrant, so the
   loop counter `i` degraded to a mixed Int32/Float64 value. `i++` computed
   both an Int32 and a Float64 update, and `i < n` converted both operands to
   doubles.
3. Cached field tags, and loop-carried tags and flags in general, were block
   parameters. Cranelift's identity-based constant-phi pass kept them because
   each edge used a different `iconst`, which left tag guards and spills in
   the loop. Tag tests folded to constants still left constant `brif`s.

## What this slice does

- **Guarded property leaves no longer publish stack memory.** Every observer
  of interpreter stack memory (deopt, ownership helpers, generic bridges)
  already republishes the live prefix from SSA and sets `stack_top` itself.
  `can_emit_access` guarantees that no live operand is an owner. Leaving
  memory untouched preserves the invariant that `[stack_base, stack_top)` only
  holds values that the interpreter may release, which is all a terminating
  interrupt unwind can see.
- **Induction proofs across guarded property leaves.** After the final
  property plan is known, lowering specializes Number updates to checked Int32
  updates, using `specialize_integer_updates_with_preserved_frame_reads` with
  exactly the planned guarded sites and polls as preserved frame reads. It then
  recomputes the numeric facts with the same premise. The IR is cloned and
  charged to the IR budget first. Selected updates keep their operand check
  and overflow deopt. A planned site that cannot be emitted still fails
  compilation.
- **Constant representation of cached fields.** Cached values are defined with
  the checked constant tag, and reads and flushes use that constant.
  Proven Int32/Float64 inputs to a cached store skip the input tag check.
- **Static guard folding.** `emit_opt_guard_branch` and the generic `if_*`
  truthiness guard emit no branch when the condition is provably true from the
  emitted constants: `iconst 0/1`, `icmp_imm eq/ne` of a constant, and
  `band`/`bor` of such booleans.
- **Constant canonicalization** (`compiler/optimized/const_canon.rs`). Tier 2
  lowering rewrites every `iconst` to a single canonical definition per type
  and value in the entry block. Cranelift's constant-phi pass then removes
  loop-carried tag and flag parameters, and its e-graph rematerializes the
  constants near their uses.

The resulting `property-heavy` hot loop, after Cranelift optimization, has no
tag guards, no Float64 operations, no stack or field stores, and no constant
branches. What remains is three checked `sadd`/`ssub`, the checked `i++`, an
integer compare and the poll countdown.

## Evidence

Tests (all in `jit/tests/property_cache.rs`):
`guarded_property_loop_keeps_its_induction_variable_int32` (structural: no
`fcvt_from_sint`/`fcmp`/`fadd`, integer compare, field-free continuing loop),
`specialized_property_loop_counter_overflow_and_float_seed_deoptimize_exactly`
(the `i++` Int32 overflow and a Float64 seed both deopt exactly), and
`terminating_interrupt_after_unpublished_property_reads_unwinds_cleanly`
(stress GC, a terminating interrupt with primitives below property reads, then
a clean re-entry). The `const_canon` unit test checks that both loop edges pass
one canonical value. The CLIF-walking helper in `property_cache.rs` now
resolves constant aliases.

Performance: see the branch summary. These are noisy single-host diagnostics,
not publishable matrix data. The integrator must run the three-engine matrix.

## Remaining work (not in this slice)

- **Register pressure in property loops.** The loop still spills `i`, `n` and
  `toggle` because many argument, local and cache variables stay live across
  the loop. Sinking unused argument tags and moving cache data pointers
  out of the loop Phis (they are loop-invariant except on the cold poll edge)
  should remove the spills.
- **Loop-invariant guard hoisting for non-argument receivers.** Property
  candidates are still limited to unmodified arguments (`facts.entry_argument`).
  Local and `this` receivers, and receivers loaded from other objects, need
  identity facts plus a preheader guard with an exact deopt map. This is also
  B5 (`float64array-traversal`).
- **Alias-aware forwarding for distinct receivers with the same atom**
  (`mixed-quotes` `right.score - left.score`). Two different arguments with
  the same atom flush and invalidate each other; a runtime identity check
  (`a != b`) in the preheader could split the paths. `mixed-quotes` is
  dominated by per-call bookkeeping (B2), so this slice does not move it.
- **Store sinking across calls** stays a FlushInvalidate boundary.
- **Inherited, pre-existing nondeterminism:** a 20x-longer `property-heavy`
  loop sometimes never reaches Tier 2 after an OSR attempt. This happens on
  both the `82d3808` baseline and this branch (in 4 of 6 baseline runs), so it
  is a tier-up scheduling issue and not related to this change.
