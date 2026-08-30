# Task 11 implementation checkpoint

## Implemented and verified

- Production hot-call and hot-loop feedback uses saturating per-generation state,
  base thresholds 32/56, and a queued bit that makes snapshot requests
  single-shot.
- The preliminary adaptive interface is deterministic and integer-only. With
  no measured invocation work it returns the neutral 32/56 base and an
  auditable `NeutralBase` rationale; Task 14 remains responsible for measured
  calibration.
- QuickJS emits a call event once per invocation after function identity is
  available and before native acquisition. A taken backedge emits a loop event
  with delta 1 and target PC only after its interrupt poll succeeds. Untaken
  branches emit no loop event.
- Verification now retains the exact verifier-derived slot kinds at every real
  reachable loop header. `OsrKey` and `OsrMap` bind function generation, PC,
  stack depth, live kinds, and a nonzero independent entry offset. Offset zero
  is rejected so the ordinary function entry cannot masquerade as OSR.
- Existing production lifecycle tests were updated to deliberately reach the
  new call threshold instead of relying on the former one-call Task 10 policy.

## Verification evidence

- `cargo test -p rquickjs-jit --test osr --features compiler,test-support`:
  5/5 passed, including exact 32/56 boundaries, one snapshot request, untaken
  loop, real verifier maps, and rejection of entry offset zero.
- `cargo test -p rquickjs-jit --test background --features compiler,test-support`:
  14/14 passed, including short callbacks, asynchronous install, quotas,
  generation isolation, and two-runtime execution/drop isolation.
- `cargo test -p rquickjs-jit --test osr --test background --features
  compiler,test-support --release`: 19/19 passed.
- `cargo test -p rquickjs-jit --features compiler,test-support`: 234/234 passed
  across all unit and integration suites on the Linux host.

## Required continuation before Task 11 completion

The production compiler does not yet publish callable independent OSR entries.
Task 10's `CompiledArtifact` owns one `PublishedBaselineCode`, and Task 7's
Cranelift lowering has one ABI prologue; only that prologue receives the hidden
`JSJitExit` sret and frame arguments. Frame-state marker offsets are internal
basic-block addresses, not ABI entry points. They must not be returned from
`acquire_entry`.

The safe continuation is to make a Tier 1 artifact own separately compiled and
published ABI functions for each verified loop header (including independent
relocation, W^X indirect-target declaration, unwind registration, pinning, and
entry address), then teach nonzero-PC acquisition to require the exact `OsrMap`.
Each variant must validate every frame invariant and import args/locals/live
stack before jumping to the post-poll header. This checkpoint intentionally
keeps nonzero-PC production acquisition disabled rather than replaying the
function prefix or calling an internal marker with the wrong ABI.
