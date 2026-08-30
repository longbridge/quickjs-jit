# Task 9 report: closed Tier 1 opcode policy and differential matrix

Status: **DONE_WITH_CONCERNS**

## Result

- `jit/build.rs` directly consumes `rquickjs-sys` generated opcode metadata and
  emits the Tier 1 identity table. The build hard-fails unless the authoritative
  table remains 252 dense opcode IDs with fingerprint
  `0x05d5c0867521c077`; a QuickJS opcode update therefore requires an explicit
  policy audit.
- Every linked opcode maps to exactly one closed `Tier1Policy`: `Native`, a
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

## Explicit concern / next instrumentation dependency

The forced-baseline harness proves that the captured function contains the
tested opcode family and that the published native function is entered, but it
does not yet expose a per-bytecode-PC execution trace or per-helper invocation
counter. Consequently this task does **not** claim per-PC dynamic coverage for
all eligible opcodes. That instrumentation belongs on the production backend
and metrics path established by Task 10; Task 13 must use it to turn the family
matrix into a no-fallback per-opcode execution gate. The static 252-entry policy
is complete, but it is intentionally not presented as dynamic semantic
coverage.
