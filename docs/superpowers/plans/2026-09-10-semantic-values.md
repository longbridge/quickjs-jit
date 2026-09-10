# Semantic value construction implementation plan

**Goal:** Implement the macOS-first architecture objective, starting by making
numeric expression operands explicit and using those identities in production
optimization. This is the first part of Phase B, not its completion gate.

**Architecture:** Keep the existing deopt and ownership bridge while replacing
the adjacency-only numeric rewrite with semantic value construction. Build
numeric operations from explicit ValueIds; production CSE consumes those IDs
and machine lowering consumes the resulting canonical-value reuse. Expand that
representation to CFG Phi and FrameState next, then migrate lowering and add
the facts/effects, inlining, property and array passes in the supplied design.

**Spec:** `docs/SEMANTIC_SSA_PROGRESS.md` and the user's full attached design.
The user's 2026-09-10 clarification makes macOS the current implementation and
acceptance platform. Native Linux regression verification is deferred.

**Constraints:** Preserve JS semantics, ordered numeric operands, mutation and
reentrant boundaries, deopt stack depth, and bounded compiler memory. Keep the
complete three-engine matrix, paired previous-revision controls, and all losing
rows. Do not claim full Semantic SSA until CFG Phi, FrameState and lowering use
explicit values end-to-end, or claim the performance goal before its Bun gates.

## Numeric value construction

- [x] Add a failing production IR/machine-code test for duplicated nested
  `(a+b)*(c+d)` expressions; current adjacency CSE cannot eliminate the second
  multiply. Require one emitted `fmul`, not just a metric increment.
- [x] Add `jit/src/ir/scalar.rs`: typed `ScalarValueId`, `ScalarBinaryOp`,
  `ScalarValue::{Int32, Input, Binary, Opaque}`, and a `ScalarGraph` mapping original
  IR node IDs to values. Build stack value identities for each basic block;
  use fresh identities at unknown frame writes and reentrant boundaries.
- [x] Connect the graph to `OptimizedIr::translate`, after existing local
  rewrites. CSE keys must include operation, ordered operand IDs and result
  representation. Record graph memory in the compile budget.
- [x] Let machine lowering of `Reuse` consume its declared original operands
  before pushing the canonical result. Preserve legacy zero-pop reuse.
- [x] Verify nested expressions, operand reversal, argument/local mutation,
  intervening calls, overflow and negative zero. Run the existing optimized,
  deopt and semantic suites on macOS. Add runtime execution checks on macOS
  rather than relying on existing Linux-only test configurations.

## Performance and subsequent implementation

- [x] Add a representative repeated-expression workload with identical inputs
  and consumption for Bun and QuickJS. Preserve an immutable pre-change binary
  and run paired controls. Refresh the full matrix for this candidate.
- [ ] Extend values to CFG Phi and exact pre-effect FrameState; verify diamond
  joins, loop backedges, stack joins and deopt materialization.
- [ ] Feed numeric machine lowering directly from the graph, with feedback
  selecting Int32/Float64 and explicit checks. Remove superseded adjacency
  matching after coverage and the no-regression gate pass.

The remaining whole-goal sequence is unchanged: known facts/effects, bounded
inlining and call IC/trampoline, property guard hoisting/forwarding/store sinking,
array modes/LICM/ranges/bounds elimination, lazy inline-frame reconstruction and
ownership. Each still requires the structural and three-engine performance
gates recorded in the progress document.


### CFG migration details established during the macOS run

- Preserve block-entry values separately from frame reads after a write or
  reentrant boundary. The current `Input { block_pc, slot }` is sufficient for
  local value numbering but cannot by itself identify the correct definition
  at every program point. Add explicit write transfer and boundary reload
  definitions before using it for cross-block lowering.
- Create predecessor-indexed Phi inputs for arguments, locals and live stack
  slots, including loop backedges. Model stack permutations with the existing
  audited `opt_stack_permutation` contract, shared with graph construction.
- Attach exact pre-effect SSA frame mappings to active guards. A reused numeric
  operation does not execute its original guard; distinguish its dead guard
  state from the live guards that must retain exact bytecode stack depth.
- Introduce a numeric operation kind with explicit operands in the production
  node stream. Lower its operands from the value graph; retain the audited
  ownership/deopt bridge for operations not yet migrated. Profile-selected
  numeric representations must become graph properties rather than being
  inferred from bytecode names during final lowering.
- Thread `CompileControl` through graph construction and charge before growth,
  including Phi edges, frame maps and scratch state. Existing retained-size
  accounting alone is not a peak memory bound.
- Required first tests: diamond assignments, loop-carried local updates,
  nonempty stack joins, exact pre-effect frame values, mutation/reentrant
  invalidation and cancellation/resource limits. Follow with native execution
  and exceptional deopt checks on macOS before the Phase B performance gate.
