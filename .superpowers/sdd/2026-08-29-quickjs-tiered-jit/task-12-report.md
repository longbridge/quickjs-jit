# Task 12 implementation report

## Scope delivered

The first optimizing tier is intentionally narrow. It accepts verifier-proven
numeric/local functions and rejects property, allocation, call, global, shape,
cell, and inlining specialization. Accepted artifacts use a distinct Cranelift
guard-exit mode: Tier 1 domain failures return `RETRY_INTERPRETER`, while Tier 2
domain failures return `DEOPT` with the exact C-visible frame PC. The numeric
hot path continues to use the audited unboxed payload/tag representation and
native local arithmetic.

Production dispatch now compiles `Tier::Baseline` and `Tier::Optimizing` on the
existing bounded Task 10 workers. Installation remains runtime-thread-only,
Tier 2 retains the exact Tier 1 deopt target, entry acquisition prefers the
optimizing artifact, and generation dependencies are validated before publish.

## Correctness contracts

- Fixed-capacity feedback entries are keyed by function generation, PC, and
  feedback kind. Diversity transitions `Monomorphic -> Polymorphic ->
  Megamorphic` monotonically and snapshots contain only stable tokens plus an
  epoch.
- `DeoptMap` validation requires every argument/local/stack slot exactly once.
  Materialization validates and plans before touching a destination.
- Owning tagged values use an explicit two-phase duplication contract. A failed
  duplication releases every earlier duplicate in reverse order and publishes
  no partial frame.
- Entry type guards run before native mutation. Arguments and locals are
  already materialized in QuickJS-owned C-visible slots, so the exact entry
  deopt does not depend on machine locations or Cranelift stack maps.
- `BeforeEffect` and `AfterEffect` retain distinct side-effect epochs. Property,
  getter, proxy, setter, call, and coercion specialization is rejected in this
  tier, preventing replay of effects without stable shape/cell identities.
- Numeric folding preserves negative zero and NaN and widens overflowing int32
  arithmetic to float64. Local CSE/DCE operate only on the separate pure
  optimizing IR and never cross an effect.
- Dependency invalidation is generation-exact, reverse-indexed, transitive,
  and cycle-safe. Optimizing artifacts also retain the Task 5 Tier 1 deopt pin.

## TDD evidence

RED failures were observed for the missing feedback API, deopt API, optimizing
IR/compiler API, dependency graph, ownership rollback, CSE/DCE metrics, and the
production Tier 2 entry. A production test then proved all of the following in
one runtime:

1. Task 10 worker compilation and runtime-thread installation of Tier 2.
2. Acquisition of an artifact whose key is `Tier::Optimizing`.
3. At least one real native Tier 2 entry with `boxes_elided > 0`.
4. Correct numeric-loop output under stress-GC mode.
5. A real generated type guard returning `DEOPT` for a string argument.
6. Exact QuickJS resume at entry PC producing the interpreter result `"x"`.
7. Non-zero deopt and Tier 2 guard-failure metrics.

## Verification

- `cargo test -p rquickjs-jit --features compiler,test-support --test optimized --test deopt --test lifecycle`
  - optimized 7/7, deopt 4/4, lifecycle 45/45
- `cargo test -p rquickjs-jit --features compiler,test-support --release`
  - exit 0; the complete release JIT suite passed, including baseline,
    background, lifecycle, native-boundary, opcode, OSR, platform, semantics,
    snapshot, verifier, optimized, and deopt suites
- `cargo fmt --all -- --check`
  - exit 0
- `cargo clippy -p rquickjs-jit --features compiler,test-support --all-targets -- -D warnings`
  - exit 0

The release build repeats existing upstream QuickJS C compiler warnings about
`buf2`; no new Rust warning or clippy finding is present.

## Explicit limits

There are no property/shape/global/callee-specialized mid-instruction guards in
this first tier. Those paths reject Tier 2 and retain Tier 1/interpreter
semantics. The supported numeric/local subset now has both entry and
loop-header guards; unsupported effectful specialization still rejects Tier 2.

## Critical follow-up: independent machine lowering and mid-loop deopt

The production optimizing compiler no longer translates, accepts, or lowers
`BaselineIr`. `OptimizedIr::translate` builds its own CFG blocks, typed value
representations, effects, guard identities, live frame shapes, and rewritten
machine plan directly from verified bytecode. A separate Cranelift builder
consumes only that optimizing IR. Tier 1 and Tier 2 share only target encoding,
unwind packaging, and the stable frame ABI.

The native numeric loop keeps int32 additions unboxed with Cranelift's checked
`sadd_overflow`, widens overflow to float64, and checks the selected int32
representation at the next real loop header. Guard zero is the entry map;
loop guards use their deterministic optimized-IR map identities. At loop
boundaries the operand stack is empty, every live local has already been
committed to QuickJS's `var_buf`, and arguments remain in `arg_buf`, so the
deopt transaction has no owning temporary roots and cannot partially publish.
The exit writes the exact loop-header PC and resumes after the completed
overflowing add, avoiding side-effect replay.

Production verification executes `f(100000, 0)`: the accumulator crosses the
int32 domain during native Tier 2 execution, the next loop guard deoptimizes,
QuickJS resumes at that header, and the final result is exactly
`4_999_950_000`. The same run separately verifies entry deopt for a string
argument. The optimized test suite is now 15/15 and the deopt suite 4/4.

Loop headers also call the canonical POLL helper after all live locals are in
C-visible storage and before evaluating the representation guard. This keeps
interrupt, stress-GC, and reentry handling on the existing root-safe runtime
boundary; the generated CLIF retains the indirect call and its unwind site.

Side exits are counted per function generation and guard. A stable guard emits
the side-path compilation action on exactly hit 10. Seeing a different guard
marks the profile unstable, atomically demotes future entry selection to the
pinned baseline artifact, and installs exponential optimizing-tier retry
backoff. Guard identities are captured inside the pinned trampoline before the
stable ABI reserved field is cleared, then consumed by the runtime callback.

Final follow-up verification:

- optimized 16/16, deopt 4/4, semantics 8/8, lifecycle 47/47 in debug;
- the complete `rquickjs-jit` compiler/test-support release suite passed;
- the non-JIT workspace suite passed (including core 173/173 and doctests);
- fmt and all-target clippy with `-D warnings` passed.

The bare `cargo test --workspace --all-targets` command remains invalid for the
repository's pre-existing feature-gated JIT integration tests, while
`--all-features` enables nightly-only `doc-cfg` on stable. The JIT suite and the
remaining workspace were therefore run with their valid feature matrices.
