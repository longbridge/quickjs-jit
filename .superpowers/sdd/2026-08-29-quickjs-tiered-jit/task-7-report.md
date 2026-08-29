# Task 7 report: Tier 1 Cranelift compiler for pure frame operations

Status: **DONE_WITH_CONCERNS**

Root implementation commit: `e909718 feat(jit): compile baseline QuickJS control flow`

Nested QuickJS commit: `e4680d2 feat(jit): add interrupt poll runtime API`

## Result

Task 7 adds a Cranelift 0.116 Tier 1 compiler for the verified, pure-frame
QuickJS subset. It provides:

- `BaselineCompiler::host().compile(&VerifiedFunction)` and explicit
  `BaselineCompiler::new(OwnedTargetIsa)` cross-target compilation;
- a compact, PC-annotated baseline IR with frame states, stack maps, OSR
  labels, polls, constants, frame loads/stores, stack permutations, branches,
  numeric operations, comparisons, and returns;
- direct Cranelift code generation without `cranelift-jit` or `JITModule`;
- W^X publication through the Task 6 allocator with exact relocation kinds,
  symbolic targets, explicit resolution, validation, and one-way publication;
- a versioned frame-owned interrupt-poll API, added as an ABI-minor tail;
- retained Cranelift unwind metadata, logical stack maps, and frame states in
  compiler artifacts;
- conservative interpreter retry for unsupported coercion or refcounted return
  cases, without first mutating the execution frame.

The existing Task 5 `Compiler` trait remains unchanged. `BaselineCompiler`
has the required inherent compile method and a separate trait adapter that
produces `CompiledArtifact`.

## TDD evidence

### RED: compiler contract

The initial baseline test did not compile because the production compiler and
synthetic-frame support did not exist. Representative failures were missing
`compiler::baseline`, `verified_bytecode`, `SyntheticFrame`, and
`CodeMemoryError::TargetIsaMismatch`.

### RED: relocation preservation

Tests added before the relocation extension failed for missing
`RelocationKind`, `RelocationTarget`, and `ResolvedRelocation`. The former
publisher accepted the Task 6 absolute-only `Relocation` directly, so it could
not prove that Cranelift's encoding or symbolic target survived publication.

### RED: interrupt ABI tail

ABI tests added first failed for the missing runtime API constants, type,
execution-frame tail, ABI-info fingerprint tail, and regenerated bindings.

### RED: retained unwind metadata

The final metadata test failed at the intended missing interface:

```text
error[E0599]: no method named `unwind_metadata` found for struct `RelocatableCode`
```

It passed after retaining the complete Cranelift 0.116 unwind plan as a
version-pinned opaque encoding and carrying it into the Task 5 artifact.

### Final GREEN

```text
$ cargo test -p rquickjs-jit --features compiler,test-support
151 passed; 0 failed across all rquickjs-jit test binaries

$ cargo test -p rquickjs-jit --test baseline --release \
    --features compiler,test-support
test result: ok. 13 passed; 0 failed

$ cargo test -p rquickjs-jit --test abi --features test-support,bindgen
test result: ok. 10 passed; 0 failed

$ cargo check -p rquickjs-sys --features jit-abi,update-bindings
Finished successfully

$ cargo clippy -p rquickjs-jit --all-targets \
    --features compiler,test-support -- -D warnings
Finished successfully

$ cargo check -p rquickjs-jit
Finished successfully

$ cargo test --workspace --all-targets
All workspace tests passed

$ cargo fmt --all -- --check
Finished successfully

$ git diff --check
Finished successfully
```

Release compilation emits the existing QuickJS `buf2` maybe-uninitialized C
warning. The workspace build also emits the existing unused `Command` import
warning from `sys/build.rs`; neither warning originates in Task 7.

## Machine ABI and value-layout proof

The Cranelift signature is constructed from the selected ISA's default C call
convention with exactly two parameters:

1. `isa.pointer_type()` with `ArgumentPurpose::StructReturn`;
2. `isa.pointer_type()` for `JSJitExecFrame *`.

There are no ordinary return values. Every exit path stores all 24 bytes of
`JSJitExit`: kind at byte 0, reserved zero at byte 4, resume PC at byte 8, and
resume stack-top at byte 16, followed by a void machine return. The retained
CLIF audit test checks `sret`, absence of a normal result signature, and the
indirect poll call.

`machine_entry_executes_the_exact_aggregate_return_abi` publishes real host
machine code and invokes it through the declared
`extern "C" fn(*mut JSJitExecFrame) -> JSJitExit` type. It verifies all exit
fields and the returned frame result. This exercises the host compiler's
aggregate-return lowering rather than transmuting to a two-argument test-only
signature.

`FrameLayout::validated` derives every frame/helper/value offset with
`offset_of!` from the generated bindings. Compilation rejects anything other
than an eight-byte target pointer, a 16-byte/eight-aligned `JSValue`, and a
24-byte `JSJitExit`. A value is always represented as two I64 value words
(payload and tag); only pointer operations use `isa.pointer_type()`.
`result_copy_preserves_all_sixteen_jsvalue_bytes` returns a sentinel whose two
words are independently nontrivial and confirms an exact 16-byte copy.

## IR, supported semantics, and retry atomicity

Every `IrInstruction` carries its source bytecode PC. Polls, guards, exits, and
OSR labels reference a `FrameStateId`; each state names live argument, local,
and stack slots. Cranelift source locations are translated into native code
offsets, and the trait adapter retains both frame states and corresponding
logical stack maps. The complete Cranelift unwind plan is serialized with its
format and frame size and charged as owned artifact metadata.

The supported subset includes:

- undefined, null, boolean, compact i32, and explicit i32 constants;
- argument/local loads and stores, checked locals, uninitialized locals,
  drops, and QuickJS stack permutations;
- goto, true/false branches, loop headers, return, and return-undefined;
- unary plus, negation, increment/decrement, bit-not, and logical-not;
- add/subtract/multiply/divide/modulo, bitwise operations and shifts;
- numeric relational, loose numeric equality, and strict numeric equality.

Overflowing i32 arithmetic changes to the JavaScript Number path. Tests cover
`i32::MAX + 1`, unsigned shift results above `i32::MAX`, floating ToInt32, and
negative zero from multiplication.
An actual QuickJS-captured for-loop snapshot compiles without rewriting its
opcodes and returns 4,950 for 100 iterations.

Potentially coercive dynamic arithmetic guards both operands before any store
to the execution frame. Unsupported object/string `+` returns
`JS_JIT_EXIT_RETRY_INTERPRETER`, and a byte-for-byte frame snapshot remains
unchanged. Values that would need `JS_DupValue`, such as a borrowed object
argument returned directly, also retry unchanged rather than creating an
unowned result. Pure local/stack changes remain in SSA, so a later retry safely
restarts the interpreter at the entry PC with the original frame.

## Interrupt and safe-point proof

ABI minor 1.2 appends `runtime_api` to `JSJitExecFrame` and appends the runtime
API layout fingerprint to `JSJitABIInfo`. `JSJitRuntimeAPI` v1.0 currently
contains only `interrupt_poll`. QuickJS initializes a static table whose poll
delegates to the existing `js_poll_interrupts` path. All nine bundled
JIT-capable target bindings were regenerated, and fresh bindgen parity passes.

Generated code loads the table and function pointer from the frame and uses
Cranelift `call_indirect`. It therefore embeds no process helper address and
emits no external poll relocation.

Poll insertion occurs:

- at function entry;
- before every backward branch;
- immediately before every return;
- after at most 1,024 straight-line source operations.

There are no other helper calls or allocation points in this pure Tier 1
subset. Later helper-bearing lowering must place a poll immediately before
each such call/allocation.

The entry/return test arms interruption on poll 2 and proves interruption wins
before the return result is written. The 4,097-NOP test arms the second poll,
observes an interrupt, and verifies that its resume PC is no later than source
offset 4,096. The loop test observes entry plus backedge polling across 100
iterations.

## Relocation and publication path

The artifact model now preserves every Cranelift 0.116 relocation variant and
one of three target forms: absolute address, function-relative offset, or
symbol. External names are rendered with the compiled function's retained
Cranelift parameters rather than discarded or guessed.

Publication is:

```text
validate target triple + required ISA features
  -> allocate writable memory
  -> write code bytes
  -> resolve every relocation target
  -> validate/apply the complete resolved relocation batch
  -> declare indirect target 0
  -> publish once as executable
```

`WritableCode::apply_relocations` accepts only `ResolvedRelocation`; unresolved
symbolic targets cannot reach byte application. The publisher implements and
range-checks the kinds needed by the supported desktop backends, including
absolute, x86 PC-relative/call, and AArch64 call encodings, and rejects other
kinds explicitly. Batch validation completes before the first staged byte is
changed.

The platform test preserves an exact `Abs8` kind and the symbolic
`qjsjit_interrupt_poll` name, resolves it, applies the addend, declares the
entry, and publishes. Actual poll lowering deliberately uses the frame API
indirectly, so ordinary Tier 1 code has no PC-relative host-helper range risk.

`BaselineCompiler::new` may generate non-host bytes but `publish` rejects them
before allocator creation. Same-triple publication also verifies every
required ISA boolean/enum flag against a freshly detected native ISA; merely
matching the architecture string is insufficient.

## Cross-target limitations

This Linux x86-64 host generated AArch64 Linux code and proved that it cannot
publish locally. It did not execute AArch64 code. Linux AArch64, macOS
x86-64/AArch64, and Windows x86-64/AArch64 still require their native release
CI jobs to prove aggregate-return ABI execution, unwind generation, cache
synchronization, and numeric behavior on each platform.

No target SDK or Rust target was installed. WASM remains outside the compiler
feature graph by cfg and was not claimed as a local Tier 1 target.

## Self-review findings and remaining concerns

The direct adversarial review found and fixed three issues before commit:

1. returning borrowed refcounted values had to retry because Tier 1 has no
   duplication helper yet;
2. target-triple equality alone was insufficient for publication, so required
   ISA flags are now checked against native host capabilities;
3. frame states were retained but logical stack maps and Cranelift unwind data
   were not yet carried into the Task 5 artifact; both are now retained and
   cache-charged.

Remaining concerns are intentionally bounded:

1. Object/string coercion, general runtime helpers, exception-producing calls,
   allocations, and refcount helpers are unsupported and retry atomically;
   Task 8 owns that helper table extension and lowering.
2. Symbolic relocation resolution has no global host-symbol registry yet.
   Pure Tier 1 poll calls avoid the issue through `call_indirect`; any future
   external symbol safely fails publication until an installer resolver is
   supplied.
3. Unwind metadata is retained with artifacts but OS-specific registration is
   not performed by this direct `RelocatableCode::publish` convenience path.
4. Only native x86-64 Linux execution was available locally; all other native
   runner claims remain CI requirements.

## Commits

- Nested QuickJS: `e4680d2 feat(jit): add interrupt poll runtime API`
- Root implementation: `e909718 feat(jit): compile baseline QuickJS control flow`
- This report is committed separately so it can record the implementation
  commit IDs.
