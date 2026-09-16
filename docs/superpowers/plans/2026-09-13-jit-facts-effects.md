# V8/JSC facts, effects and recovery implementation plan

> **For agentic workers:** Use superpowers:executing-plans for the dependent
> compiler work. Independent review may use subagents; preserve the user-selected
> current checkout. Execute continuously under the existing approval.

**Goal:** Implement all five approved optimizing-JIT directions and validate the
complete final automatic-JIT/QuickJS/Bun performance matrix.

**Architecture:** CFG SSA carries semantic values and recovery state. Bounded
known facts and heap effects feed loop guard placement, forwarding, sinking,
range proofs and inline recovery. Cranelift remains the machine backend.

**Tech Stack:** Rust, QuickJS C ABI, Cranelift, shared JS benchmark driver.

**Spec:** `docs/superpowers/specs/2026-09-13-jit-facts-effects-design.md`

## Global constraints

- Work in the current checkout; preserve unrelated changes.
- Follow V8/JSC source designs linked in the spec.
- Preserve exact QuickJS effects, exceptions, GC/rooting and reference counts.
- Keep cancellation, compile work/memory limits and conservative fallback.
- Every optimization requires paired previous JIT, same-version QuickJS and
  default Bun evidence; final README includes every scenario and its full CI.
- Keep all five deliverables active until their code and acceptance gates pass.

## Task 1: Freeze baseline and implement known facts/effects

Files: `jit/src/ir/scalar.rs`, new `jit/src/ir/facts.rs`,
`jit/src/ir/optimized.rs`, `jit/src/ir/mod.rs`, `jit/tests/optimized.rs`.

- [ ] Build e174b52 using the current release-version lockfile; freeze binary,
  lockfile, source and version hashes before any compiler edits.
- [ ] Add tests proving frame source identity through Phi/poll aliases and
  invalidation on assignments, mixed predecessors and unknown operations.
- [ ] Add semantic property/element nodes using actual SSA operands; distinguish
  guarded heap effects from potentially reentrant generic operations.
- [ ] Implement bounded fixed-point facts; joins intersect knowledge, and
  seedless cycles or budget exhaustion never establish a speculative proof.
- [ ] Run `cargo test -p quickjs-jit-runtime --release --features compiler,test-support --test optimized`.

## Task 2: Loop analysis and call guard placement

Files: new `jit/src/ir/loops.rs`, `jit/src/ir/facts.rs`,
`jit/src/compiler/optimized.rs`, `jit/tests/optimized.rs`.

- [ ] Write a generated-CFG test that fails while callee identity loads occur
  inside an admitted loop, keeping overflow and cold-exit checks visible.
- [ ] Build bounded natural-loop/preheader analysis from CFG predecessors and
  dominance; use SSA facts to prove the guarded target is invariant.
- [ ] Move target guards with an entry recovery state and reconstruct/recheck
  facts at observable polls. Do not fail a zero-trip loop for an unused target.
- [ ] Execute native target-change, argument mutation, zero-trip, nested-loop,
  unknown-call, interrupt and overflow recovery tests.

## Task 3: Property forwarding and sinking

Files: `jit/src/ir/facts.rs`, `jit/src/ir/scalar.rs`,
`jit/src/compiler/optimized.rs`, `jit/tests/optimized.rs`.

- [ ] Add a property-loop CFG test requiring x/y values in SSA and no repeated
  property loads/stores between materialization boundaries.
- [ ] Use receiver identity + property location facts to forward reads and
  eliminate redundant shape checks; aliasing writes invalidate matching facts.
- [ ] Represent dirty fields and emit ordered materialization at observable
  polls, deopt, calls, exceptions and normal exits before sinking writes.
- [ ] Test overlapping receivers, shape changes, accessor/Proxy fallback,
  overflow after an earlier write, interrupt observation and GC ownership.

## Task 4: Array modes and range proofs

Files: `jit/src/ir/facts.rs`, `jit/src/ir/loops.rs`,
`jit/src/compiler/optimized.rs`, `jit/tests/optimized.rs`,
`benchmarks/scripts/`, `benchmarks/run.rs`.

- [ ] Add preallocated packed/Int32/Float64 traversal diagnostics with identical
  JS scripts for Bun and QuickJS, retaining the original construction workload.
- [ ] Add failing CFG tests for loop-invariant array metadata and removed
  redundant CheckInBounds under `0 <= i < length` proofs.
- [ ] Implement feedback-selected modes, invariant metadata and bounded integer
  relationships; invalidation/unknown ranges retain original checks.
- [ ] Verify empty, boundary, holes, wrong type, overflow, detached/resizable
  buffers and aliasing writes with interpreter comparisons.

## Task 5: Inline recovery and generic call bridge

Files: `jit/src/ir/inlining.rs`, `jit/src/ir/scalar.rs`,
`jit/src/ir/optimized.rs`, `jit/src/compiler/optimized.rs`,
`jit/src/runtime/`, `sys/patches/`, `jit/tests/`.

- [ ] Add a native effectful-callee deopt test with a counter increment before
  a failing speculation; require the counter to increment exactly once.
- [ ] Model inline origins, parent continuations, live locals/stack and ownership
  for recovery; implement the runtime handoff at the precise bytecode position.
- [ ] Extend bounded graph-building inlining to general branches/locals and
  admitted effects using the same facts and recovery, then test nested frames.
- [ ] Profile actual generic calls; implement stable call-site dispatch and
  reduce unnecessary bridge/frame work while retaining non-inline fallback.
- [ ] Test throws, reentry, callee replacement, GC, retiring artifacts, budgets
  and the generic non-inlined path.

## Task 6: Final verification and evidence

Files: `README.md`, `benchmarks/results/`, `docs/SEMANTIC_SSA_PROGRESS.md`.

- [ ] Run full release runtime tests, targeted Test262/differential, Clippy,
  formatting, diff checks and relevant native sanitizers.
- [ ] Freeze final candidate and collect all scenarios using `benchmarks/paired.py`
  with five discarded and thirty retained processes per configuration.
- [ ] Validate checksums/provenance, summarize paired intervals and refresh the
  complete root README matrix, retaining every losing/inconclusive row.
- [ ] Collect throughput/P99/startup/compile/memory and actual host controls
  with equivalent pure-JS Bun kernels separately.
- [ ] Review final implementation and audit all five spec deliverables and
  numerical gates; keep unfinished items and historical regressions explicit.

## Execution record

### Authorized delivery (2026-09-14)

- User explicitly requested push and PR after completion, then shutdown only
  after CI passes. Preserve all five implementation and benchmark gates before
  delivery; push/create PR, monitor required checks, fix failures and revalidate.
- Shutdown is authorized only after final PR required checks are successful;
  incomplete implementation, pending/failed CI or missing evidence does not
  satisfy this condition. Do not shut down while work/CI is outstanding.

- 2026-09-13: clean e174b52 checkout verified. User approved all five directions.
  `--locked` baseline build rejected stale local release-version lock entries;
  offline build is updating local package versions before source changes.
- Baseline frozen at `/home/jason/work/quickjs-jit-evidence-20260913-rtndEA`:
  e174b52 JIT binary/source/lock hashes and native release validation (582 passed,
  0 failed, 1 ignored). Individual agent reports live in that evidence directory.
- Implemented and unit-tested semantic heap operations/effects, known argument
  identities, bounded natural loops, property planning and integer range proofs.
  Range proof production consumption remains pending.
- Call-target LICM structural/native controls pass; resource-expansion review is
  being addressed. Generic call cleanup native tests pass: CALL remains one;
  FREE counts baseline 4->2, Tier2 6->4, unchanged under stress GC.
- Property cache backend and parent boundary hooks are integrated. Repeated-read
  CFG test passes; loop readiness and full overflow reconstruction are under
  investigation. This is not yet step-3 acceptance.
- Three preallocated array benchmark scenarios added. Native tests reproduced
  packed logical-length and typed shadowed-length bugs; correction and pointer
  width addressing tests precede metadata/range optimization.
- Exact C inline recovery patch and ABI minor 22 are in development. Parent
  compiler/artifact/runtime consumption and general effectful inlining are pending.
- All five deliverables remain active. No new performance measurement or final
  three-engine README matrix has been claimed. Final integrated validation,
  paired sampling, production tiering gates and independent review remain pending.
- Subsequent integration: property suite 10/10 passed after preserving cache
  across non-owning local stores and replacing legacy partial arithmetic
  recovery with full-frame materialization. Array suite 9/9 passed after exact
  logical/property length handling, pointer-width offsets, and borrowed-return
  ownership materialization. The latter fixed reproduced string corruption and
  GC crashes; a further stress-mode local-publication review fix was added.
- Broad mutable-tree diagnostics are recorded in `current-runtime-validation-report.md`
  outside the repository, not treated as final frozen-candidate validation.
  Remaining text-order CLIF assertions are being replaced with actual CFG
  checks, including negative controls rather than weakened expectations.
- New ongoing owners: frame-backed effectful inline IR and backend; bounded
  target-only call feedback/candidate retention; real ArrayMode feedback. C
  recovery review requires exact entry poll timing and closing captured shadow
  locals. Runtime invalid post-native deopt metadata must fail terminally.
