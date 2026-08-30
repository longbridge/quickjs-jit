# SDD ledger — plan: docs/superpowers/plans/2026-08-29-quickjs-tiered-jit.md

Execution context: current `jit` branch, explicitly retained per repository AGENTS.md and user direction; no worktree.

## Preflight conflict scan

| Tasks | Shared file/interface | Finding |
|---|---|---|
| 1 | crate manifests and public API | Internally consistent; direct `rquickjs-core` dependency avoids facade cycle. |
| 2 | ABI header, bindings, runtime guard | Tests match versioned attach/detach contract; all checked-in bindings named. |
| 3 | snapshots, verifier, opcode metadata | Owned `CompileSnapshot` and runtime-only constants are separated consistently. |
| 4 | native boundary, function registry, exits | C-local exception handling and exact resume contract agree with tests. |
| 5 | coordinator, cache, mock compiler | Artifact keys and completion types agree with later compiler tasks. |
| 6 | executable allocator | Platform behavior and test API agree; no RWX path specified. |
| 7 | baseline compiler | Entry signature matches Task 4 and artifact publishing from Tasks 5-6. |
| 8 | helpers and ownership | Helper ABI consumes Task 4 frames and Task 7 entries consistently. |
| 9 | opcode eligibility | Generated policy consumes Task 3 metadata and Task 8 helpers. |
| 10 | background install/invalidation | Snapshot ownership, artifact generation, and runtime-thread install match Tasks 3-5. |
| 11 | hotness and OSR | OSR maps consume Task 7 frame state and Task 10 installation safely. |
| 12 | Tier 2/deopt | Deopt maps consume Task 11 OSR/frame state and retain Tier 1 targets. |
| 13 | correctness corpus | Forced-tier coverage prevents fallback from masking Tasks 7-12. |
| 14 | benchmarks/profitability | Metrics and decisions consume Tasks 5, 10-12; schema contains required evidence. |
| 15 | gpui-shell, CI, release | Consumes public API, platform fallback, corpus, and benchmark evidence. |
| 1 ↔ 2 | `Cargo.toml`, `jit/src/lib.rs` | Task 1 establishes `jit-abi`; Task 2 adds attachment without a dependency cycle. |
| 2 ↔ 3 ↔ 4 | `quickjs-jit.h`, `quickjs.c`, `jit/src/abi.rs` | ABI grows only at versioned tails; snapshot and execution contracts use fixed-width C types. |
| 4 ↔ 5 ↔ 10 | function generations and coordinator | Retirement, stale completion rejection, and install are keyed by the same identity. |
| 5 ↔ 6 ↔ 7 | `CompiledArtifact` and executable code | Mock artifacts precede allocator-backed Cranelift artifacts; entry signature is stable. |
| 7 ↔ 8 ↔ 9 | baseline compiler and helpers | Pure lowering precedes throwing/allocating helpers and full eligible opcode policy. |
| 8 ↔ 11 ↔ 12 | C-visible frame slots and maps | OSR/deopt/GC rules preserve owning values and exact interpreter state. |
| 10 ↔ 11 ↔ 12 | coordinator and tier state | Async publication, OSR, Tier 2 dependencies, and invalidation share generations. |
| 13 ↔ 14 ↔ 15 | evidence/reporting | Correctness coverage gates benchmarks; release audit requires both plus shell evidence. |

Preflight result: no unresolved contradiction with the spec or Global Constraints.

Task 1: fix round 1/5 (2 addressed, 0 open — native target capability and WASM-family dependency gating; commits 90f7b1c..f5b78df)
Task 1: complete (commits 0a63a86..f5b78df, review clean)
Task 2: fix round 1/5 (2 addressed, 0 open — fresh-bindgen equivalence and production test-hook isolation; commits e20e21a..cd1be16)
Task 2: complete (commits f5b78df..cd1be16, review clean; nested QuickJS commit 5ed9097)
Task 3: Ruling: retain the function/constant pool transitively in runtime-thread-only `RuntimeConstants` and keep worker snapshots index/descriptors-only; add the per-index JSValue resolver as part of Task 8's versioned helper table — this preserves the spec's no-heap-pointer worker boundary without prematurely extending ABI 1.0 — cost if wrong: Task 8 may require reshaping the opaque runtime constants handle and regenerating bindings.
Task 3: fix round 1/5 (6 addressed, 0 open — bounded proof memory, CFG soundness, slot kinds, compound indices, feature-only rejection, strict flags; commits 81c5f16..17a05fe)
Task 3: complete (commits cd1be16..17a05fe, review clean; nested QuickJS commits 00dec3a and 59a015a)
Task 4: fix round 1/5 (4 addressed, 2 open after new breakage — registration reentrancy, primitive interrupt, malformed exits, acyclic registry fixed; registry synchronization and OOM fail-closed remained; commits 79c3a4e..1c87faf)
Task 4: fix round 2/5 (2 addressed, 0 open — synchronized registry and allocation-independent uncatchable sentinel; commits 1c87faf..6046cef)
Task 4: complete (commits 17a05fe..6046cef, review clean; nested QuickJS commits c66abfe, 23a04e6, 067064e)
Task 5: fix round 1/5 (7 addressed, 3 open after dependency/budget review — attempt identity, byte quota, tier state, bounded drain, reclamation signal, generation retirement, saturation metrics; commits 5202d65..0ec520c)
Task 5: fix round 2/5 (4 addressed, 0 open — dependency-aware transactional eviction, pre-admission watermark, shared bounded poll, conservative remaining signal; commits 0ec520c..a00ce66)
Task 5: complete (commits 6046cef..a00ce66, review clean)

Final-delivery directive (2026-08-30): autonomously run full correctness/performance verification and final self-review; test `../gpui-component`/gpui-shell only in its own worktree; place the complete performance report and evidence in the PR description. Superseding directive: do not shut down the machine because other tasks are running.
Correctness/documentation directive (2026-08-30): final evidence must establish general JavaScript correctness, not only gpui-shell compatibility. Run QuickJS suite, Rust-hosted Test262, interpreter/Tier1/Tier2 differential tests, semantic edge cases, fuzz/random regressions, platform gates, and repeated release audits; publish architecture, ABI, semantic, platform, upgrade, performance, and known-limit documentation.
Task 6: Ruling: do not install missing Rust targets/SDKs/toolchains during implementation; complete native Linux verification locally and leave macOS/Windows execution as explicit required CI evidence — avoids mutating shared machine toolchains and prevents a stalled install from masquerading as validation — cost if wrong: target-gated API errors may be found only by CI and require a follow-up fix.
Task 6: fix round 1/5 (8 addressed, 1 documentation gap open — macOS W^X/process policy, disable epoch, shared owners, same-address AArch64 test, LE gating, page-size and mmap-zero cleanup; commits ddb201d..a6ef1fe)
Task 6: fix round 2/5 (unsafe macOS callback safety/ordering documentation addressed; commits a6ef1fe..f0a1d21)
Task 6: complete (commits a00ce66..f0a1d21, review clean locally; non-host execution remains required CI evidence)
Task 7: Ruling: use the platform ABI's hidden struct-return convention for `JSJitExit`, `OwnedTargetIsa`, full 16-byte JSValue slot copies, and extend artifacts with Cranelift relocation kind/symbolic target; introduce the minimal versioned interrupt-poll runtime helper now and append the remaining helper table in Task 8; unsupported dynamic addition returns RETRY before frame mutation — required to keep Task 7 executable and semantically exact without inventing generic coercion semantics — cost if wrong: Task 8 may need an ABI-minor table-layout adjustment and regenerated bindings.
Task 7: fix round 1/5 (9 findings addressed, exact post-poll retry remained — remainder/refcount, CFG polls, metadata+unwind lifetime, safepoint identity, relocations, slot bounds, deep snapshots, ABI prefixes; commits 45d1993..af802b9)
Task 7: fix round 2/5 (exact call-return safepoints, PollState snapshots, bounded stack scan, Windows ARM64 fallback, Linux real unwind; one post-poll tag RETRY remained; commits af802b9..f66c8c6)
Task 7: fix round 3/5 (all RETRY guards hoisted pre-poll via CFG provenance/domain fixpoint while preserving native numeric loops; commits f66c8c6..ea2f544)
Task 7: complete (commits f0a1d21..ea2f544, review clean; nested QuickJS commits e4680d2 and 85aaac8; non-host execution remains CI evidence)
Task 8: Ruling: append `runtime_api`, runtime ID, frame cookie, stack-map count, and helper ABI version through ABI-minor tails; define the canonical helper set in one X-macro table with status/out-value ownership; make the RawRuntime-owned function registry the sole installed-function constant retainer and resolve constants from the validated active bytecode frame — prevents RawRuntime→artifact→RuntimeConstants→Runtime cycles and rejects invalid frames before touching JSValue — cost if wrong: ABI minor and all bundled bindings must be regenerated again.
Task 8: fix round 1/5 (1 finding addressed, 0 open — native semantic integration tests now declare required features, missing-feature commands fail explicitly, structured metadata prevents zero-test gates, and CI runs the explicit suites; commit c3d729e)
Task 8: complete (commits ea2f544..c3d729e, review clean; nested QuickJS commit fc3fdd2; 192/192 JIT tests plus focused leak/ASan-without-LSan evidence; environment limits recorded in task-8-report.md)
Task 9: fix round 1/5 (5 Important and 1 Minor addressed in two safe checkpoints — closed 252-row policy, exception-map eligibility split, strict native/no-RETRY harness, exact PC/opcode/helper telemetry, honest 26-opcode advertised set, and release CI; commits abff2a0 and 0d400e1; nested c7126aa)
Task 9: fix round 2/5 (2 Important and 2 Minor addressed — helper events correlated to the same PC/opcode, typed applicable semantic dimensions with real stress-GC/coercion/reentrancy gates, public ID-zero bypass removed, macOS/Windows compiler/native-smoke CI added; commits 64e5bf0 and b32b77d; nested 5c44cd3)
Task 9: complete (commits c3d729e..b32b77d, review clean; 252/252 closed policy, 26/26 advertised dynamic cases, release re-review 37/37; unsupported families retain exact interpreter fallback)
Task 10: fix round 1/5 (2 High and 4 Medium addressed — observer moved lock/ABI-outside with panic containment, exact runtime/ABI/config/target environment, pending-job polling, strict native cfg/Noop fallback, generator scratch initialization, production native entry/exit proof; commits 7aa8842, fecd190, 557a1f2, d47b12a; nested 063f517)
Task 10: fix round 2/5 (1 High and 1 Medium addressed — artifact identity now hashes the exact compiler OwnedTargetIsa triple plus every shared/ISA flag, and real dual-runtime production compile/install/native/drop isolation is proved; commits 9436c67 and 21056fc)
Task 10: complete (commits b32b77d..21056fc, review clean; nested QuickJS 67b5093 and 063f517; debug/release JIT 191/191 plus focused final identity 2/2 and background 13/13)
Task 11: implementation checkpoint (hotness, exact C feedback, verifier maps; commits 9c280bb..294b271; nested QuickJS 38f6fff)
Task 11: complete pending independent review (independent per-header ABI functions, recursive artifact/cache ownership, exact validation/import/post-poll OSR, cadence/reload/fallback metrics; commit 44d2f72; debug JIT 240/240, focused release 32/32, feature-enabled workspace and clippy clean)
Task 11: fix round 2 (review follow-up: retry ownership retained until exact coordinator deadline, invalid stress frames remain byte-identical, explicit entry-invocation retry origin separates validation from generated guards; commit aeb560b; debug JIT 248/248, release OSR 14/14, feature-enabled workspace, fmt and clippy clean)
Task 12: fix round 3 (final review blockers addressed — production CSE is keyed by ordered exact SSA argument/local identity, local version, representation and effect epoch with frame/effect barriers; stable exits retain the installed target while an owned generation/epoch/GuardId/PC/type profile compiles a distinct alternate-float side block; specialization identity is independent of the baseline deopt key; commit 5c3833c)
Task 12: verification checkpoint (real production sequence proves 10 identical Float64 guard exits produce exactly 10 deopts, installs the side-path artifact, and the 11th equivalent call increments `side_path_entries` without another deopt; debug optimized 24/24 and lifecycle 47/47, release optimized 24/24 and lifecycle 47/47; feature-enabled workspace passes except the pre-existing bundled-bindings equivalence gate regenerated only for the host target)
Task 12: hardening checkpoint (self-review found artifact-entry counting could overstate actual alternate-block execution; a red production assertion proved it, then dedicated generated-code hit marking made `side_path_entries` exact and restored bundled-target equivalence; commit 4dd0c64, nested QuickJS ddf43e5; debug/release ABI 12/12, lifecycle 47/47, optimized 24/24, fmt and feature clippy clean)
Task 13: fix round 1/5 (2 Critical and 3 Important findings addressed — manifest-scoped real production forced Tier1/Tier2 with native trace evidence; exact raw/source ranges and parse/resolution/runtime/TLA phases; side-effect-free capability-marked observation; three-mode seeded differential and byte-driven fuzzing; bounded and 256-shard CI completeness/platform gates; commit 728e95f)
Task 13: verification checkpoint (raw 14.1-4gs 1/1, parse 2/2, resolution 1/1, TLA 1/1; release correctness 18/18, differential 8/8, regressions 1/1, bounded Test262 4/4 groups and 44 variants; core context 17/17; six fuzz binaries, fmt, diff, and feature all-target clippy clean; scheduled cross-platform/full-corpus execution remains CI evidence)
Task 13: fix round 2/5 (final semantic review — exact raw source/Annex B execution, pre-source immutable observers, generated-function native Tier1/Tier2 evidence, byte-hashed closed grammar, versioned seeds, and bounded randomized CI with artifacts; release correctness 21/21, differential 9/9, regressions 1/1, six binaries and local bounded runs pass)
