# Task 9 report: closed Tier 1 opcode policy and differential matrix

Status: **FIX ROUND 3 COMPLETE**

## Review fix round 3

- Replaced independent opcode trace and global helper-counter assertions with
  test-only correlated events. Each helper event carries its exact bytecode
  `pc`, opcode ID, and helper ID, and the manifest gate requires a matching
  opcode event at the same PC. All storage and event emission remain excluded
  from production builds.
- Made coverage dimensions a closed typed schema. Required dimensions are
  derived from the linked opcode policy and explicit opcode semantics; missing,
  invented, inapplicable, or policy-mismatched evidence fails validation.
  Ownership cases execute with helper stress-GC, numeric cases include an
  int32 overflow edge, and coercing/property/call cases return observable
  re-entrancy event order for interpreter/Tier 1 comparison.
- Removed the `function_id == 0` policy exception. The public verifier and
  compiler always enforce the advertised policy; compiler-only synthetic tests
  use a private compile-policy variant through the feature-gated test harness.
- Added macOS and Windows CI jobs that check the complete compiler/test graph
  and run a published-native differential smoke on the actual host. Linux
  retains the complete release opcode/differential dynamic gate.

## Review fix round 1

- Replaced the name classifier and wildcard rejection with a checked-in,
  explicit 252-row `(id, name, policy)` audit. The build script now rejects a
  missing, duplicate, reordered, or renamed row in addition to count and
  fingerprint drift.
- Reclassified the advertised set to match the helper calls actually emitted
  by current baseline lowering (50 native, 75 helper, 127 categorized reject).
- A non-empty exception map is no longer a verifier error. Structurally valid
  bytecode verifies and receives stable `ExceptionRegion` eligibility fallback.
- The forced-baseline test backend now counts entry in a trampoline around the
  real published machine-code body, rather than counting acquisition, and
  fails differential tests if the body returns `RETRY_INTERPRETER`.

## Review fix round 2

- Added test-support-only native execution telemetry. The test compiler inserts
  a valid poll/frame-state immediately before each bytecode lowering and the C
  adapter records the actually executed `(pc, opcode)` pair. All helper entry
  points have canonical-ID counters. C storage/APIs and IR instrumentation are
  excluded from production builds, so the production ABI query, generated code,
  and hot path are unchanged.
- Reduced the production advertised set to the 26 opcodes for which a real
  emitted-and-executed JS fixture exists: 7 `Native`, 19 `Helper`, and 226
  categorized rejects. Implemented but unproven lowerings remain available only
  to ID-zero synthetic compiler tests under `test-support`; runtime snapshots
  can never use that escape hatch.
- `opcode-cases.json` schema 2 contains exactly one case per advertised opcode.
  The test reads the manifest, compares its set with the closed audit, compiles
  each captured function, enters the real machine body, observes the target at
  its actual PC, checks the expected helper counter, rejects any retry, and
  compares canonical interpreter semantics including ownership/coercion cases.
- Rejected runtime fixtures prove exact fallback reason, compiler rejection,
  zero native attachment/entry, and normal interpreter semantics.
- CI now runs the opcode and differential gates in release mode.

## Result

- `jit/build.rs` directly consumes `rquickjs-sys` generated opcode metadata and
  emits the Tier 1 identity table. The build hard-fails unless the authoritative
  table remains 252 dense opcode IDs with fingerprint
  `0x05d5c0867521c077`; a QuickJS opcode update therefore requires an explicit
  policy audit.
- Every linked opcode maps to exactly one explicit `Tier1Policy`: `Native`, a
  concrete `Helper(HelperId)`, or a categorized `Reject(FallbackReason)`.
  There is no generic execute-one-opcode helper.
- The policy advertises only opcode families already implemented by the Task 8
  baseline lowering. Closure/extended-frame state, exception regions,
  tail-calls, eval, with, generator, async, dynamic import, and resource
  management remain stable interpreter fallbacks.
- Bytecode well-formedness is separate from Tier 1 eligibility.
  `VerifiedFunction::tier1_eligibility` returns the first exact bytecode PC and
  reason, and `BaselineIr` refuses it before lowering or native publication.
- Forced-baseline differential cases exercise immediates, locals, branches,
  numeric/coercing addition, properties, arrays/objects, ownership, and
  exception/coercion order. Every forced case asserts interpreter equality and
  at least one published native entry.

## TDD evidence

Observed RED failures before implementation:

- `jit/tests/opcodes.rs` failed because the generated policy API did not exist.
- verifier split tests failed because feature opcodes were still rejected as
  malformed/unsupported during well-formedness verification.
- baseline eligibility failed because `CompileFailure::Tier1Rejected` did not
  exist.
- the initial ordinary-program differential matrix rejected QuickJS
  `tail_call`; this exposed a real unsupported frame/control transfer. The
  policy keeps `tail_call` as `Reject(UnsupportedOpcode)` instead of claiming
  false helper coverage.

All corresponding focused tests were then observed GREEN.

## Verification

- `cargo test -p rquickjs-jit --features compiler,test-support`: PASS (all JIT
  tests, including 252-opcode policy, helpers, native boundary, semantics, and
  verifier split).
- `cargo test -p rquickjs-jit --test differential --features compiler,test-support --release`:
  PASS (2 matrix tests; every case enters published native code and matches the
  interpreter).
- `cargo clippy -p rquickjs-jit --all-targets --features compiler,test-support -- -D warnings`:
  PASS.
- `cargo test --workspace -- --test-threads=1`: PASS (workspace unit,
  integration, UI, and doc tests).
- `cargo fmt --all -- --check`: PASS.

Fix round 3 additionally verified:

- `cargo test -p rquickjs-jit --release --features compiler,test-support --test opcodes --test differential --test baseline -- --test-threads=1`: PASS (6 opcode, 6 differential, 28 baseline tests).
- `cargo test -p rquickjs-jit --features compiler,test-support -- --test-threads=1`: PASS (complete JIT crate).
- `cargo check -p rquickjs-jit --all-targets --features compiler`: PASS (production graph without test telemetry).
- `cargo test --workspace -- --test-threads=1`: PASS.
- `cargo clippy -p rquickjs-jit --all-targets --features compiler,test-support -- -D warnings`: PASS.
- `cargo fmt --all -- --check` and `git diff --check`: PASS.

The macOS and Windows jobs were added to CI but were not run on this Linux
host; their results must be taken from CI rather than inferred locally.

## Coverage statement

The advertised set is deliberately smaller than the set of internal lowerings.
No opcode is advertised merely because it appeared in captured bytecode or has
a translator match arm; every advertised opcode has dynamic native-PC evidence.
