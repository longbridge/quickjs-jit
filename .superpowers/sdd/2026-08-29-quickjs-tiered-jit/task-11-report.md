# Task 11 completion report

## Implemented

- Per-generation hotness uses saturating counters, exact base thresholds of 32
  calls and 56 taken loop backedges, and a queued bit that requests one
  snapshot. Eight short callbacks remain cold.
- The preliminary adaptive policy is deterministic and integer-only. Without
  measurements it returns the neutral 32/56 base and `NeutralBase`; Task 14
  owns coefficient calibration from benchmark evidence.
- QuickJS reports a call once per invocation after establishing function
  identity and before native acquisition. A loop event `(delta=1,target_pc)` is
  reported only after a taken backedge's interrupt poll succeeds. An untaken
  branch reports nothing.
- The verifier derives exact live `SlotKind` state for every reachable loop
  header. `OsrKey`/`OsrMap` retain function generation, PC, argument and local
  counts, exact stack depth, and live kinds.
- Tier 1 emits a separate complete Cranelift ABI function for every eligible
  loop header. Each has its own relocatable output, W^X RX allocation, indirect
  target declaration, relocations, unwind registration, maps, and entry. No
  internal marker or ordinary function entry is exposed as OSR.
- One installed generation artifact owns the ordinary entry and all OSR
  children. Cache admission recursively charges all child code and metadata;
  its `ExecutionPin` retains every entry and unwind registration across reload.
- Nonzero-PC acquisition requires an exact generation+PC map. Not-ready,
  missing-map, and validation fallback classes are separately counted.
- The ABI trampoline validates struct/runtime/context/API presence, runtime ID,
  cookie, function ID/generation, exact PC, helper ABI, map count, stack
  bounds/depth, argument/local buffers, cardinality, and specialized slot kinds
  before native code. Rejection returns `RETRY` without changing frame bytes or
  value owners.
- OSR imports current arguments, locals, and live interpreter stack into SSA
  and jumps directly to a post-poll continuation. Native backedges return to
  the real header poll. OSR variants suppress generic CFG-entry duplicate polls
  while retaining periodic and return polls.

## Correctness evidence

- A five-million-iteration first invocation enters production OSR, returns
  `12_499_997_500_000`, records native OSR entry, and has zero retry.
- Interpreter and OSR runs of that loop have identical interrupt-handler poll
  counts, proving the transfer neither skips nor doubles its first backedge.
- Multi-loop compilation produces one independent entry per verified header.
- Two hot-reloaded generations both enter OSR, return distinct correct results,
  retire old code safely, and record zero cross-generation retries.
- The malformed-frame matrix covers wrong runtime, ID, generation, PC, cookie,
  map count, helper version, depth, and slot kind. Every rejection leaves the
  frame byte-identical and the slot owner/tag unchanged.
- Existing native-boundary tests prove retry resumes at the polled PC without
  replaying prefix side effects. Existing forced Tier 1 tests prove AddSlow
  `Symbol.toPrimitive`, forced GC, calls, and reentry semantics. Candidate
  AddSlow loop shapes rejected by the closed opcode policy or merge verifier
  remain stable interpreter fallback and are not claimed as OSR coverage.

## Commands and results

- `cargo test -p rquickjs-jit --features compiler,test-support`: 240/240 pass.
- `cargo test -p rquickjs-jit --test osr --test semantics --test background
  --release --features compiler,test-support`: 32/32 pass.
- `cargo clippy -p rquickjs-jit --all-targets --features
  compiler,test-support -- -D warnings`: pass.
- `cargo fmt --all -- --check`: pass.
- `cargo test --workspace --all-targets --features
  rquickjs-jit/compiler,rquickjs-jit/test-support`: pass, including 177 core
  tests, 240 JIT tests, macro trybuild tests, examples, and workspace crates.
- `cargo test --workspace --all-targets` without features does not compile the
  pre-existing JIT integration sources because they import feature-gated test
  APIs; the explicit-feature workspace command above is the valid gate.

Host execution was Linux x86_64. macOS and Windows execution remains required
CI evidence under the existing Task 6 platform ruling.

`jit/benches/tiering.rs` does not exist yet, so Task 11 makes no cold-dispatch,
first-invocation-time, or speedup claim. Task 14 must add the reproducible bench
and record this implementation's code-size/compile-cost tradeoff.
