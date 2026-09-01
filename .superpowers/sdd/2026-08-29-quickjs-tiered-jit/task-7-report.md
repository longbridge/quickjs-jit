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
- add/subtract/multiply/divide, bitwise operations and shifts;
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

---

## Review fix round 1 (2026-08-30)

Status: **DONE**

Root review-fix commit: `d7da0de fix(jit): harden pure baseline publication`

Nested QuickJS review-fix commit:
`85aaac8 fix(jit): support ABI info prefixes`

This section supersedes the earlier claims that modulo was native, that a
shallow frame-struct comparison proved retry atomicity, and that merely
retaining serialized unwind bytes was sufficient publication ownership.

### Review findings resolved with RED/GREEN evidence

1. **Exact remainder semantics.** The new `5 % Infinity`, `-4 % 2`, and
   `1e308 % 1e-308` differential fixture initially returned `DONE` through the
   inexact `trunc(a / b)` lowering. `%` now unconditionally reaches the common
   retry exit before frame mutation. It is not part of the native subset until
   Task 8 provides the exact runtime helper.
2. **QuickJS ownership timing.** Object argument overwrite, drop, duplicate,
   local, and live-stack fixtures initially executed natively. Tier 1 now
   rejects every entry argument, local, or active stack value whose tag lies
   in QuickJS's refcounted range before the first poll or store. The test
   observes unchanged object refcount and `JS_IsLiveObject` liveness.
3. **CFG-sound polling.** A 2,050-diamond forward-control-flow fixture initially
   reached its second poll at bytecode offset 14,350. Every non-entry CFG block
   now begins with a poll, while the 1,024-operation straight-line bound and
   backedge/return polls remain. The same skipped path now interrupts below
   offset 4,096.
4. **Published ownership and unwind lifetime.** The publication-lifetime test
   was RED at compile time because the returned executable exposed no frame
   states, stack maps, unwind metadata, or lifetime pin. `PublishedBaselineCode`
   now uses one `Arc` allocation to own the RX mapping, immutable metadata, and
   native registration. A cloned execution pin retains all metadata. Final
   drop deregisters unwind information before releasing the executable mapping.
5. **Exact safe-point offsets.** Entry poll, return poll, and return state at PC
   zero initially all mapped to native offset 81. Each `FrameStateId` now gets
   a unique non-default Cranelift `SourceLoc`; compilation requires a native
   range for every state and rejects missing, out-of-range, or duplicate native
   offsets. The same-PC states now have distinct in-range offsets with no zero
   substitution.
6. **Section-relative relocations.** `X86SecRel` initially applied as an
   absolute 32-bit relocation despite having no section base. It is now
   unsupported until `ResolvedRelocation` can carry that base. Batch validation
   rejects it before changing any byte, including preceding otherwise-valid
   relocations.
7. **Flattened slot width.** A state with 65,536 flattened slots initially
   translated successfully through saturating `u16` arithmetic. State creation
   now uses checked `usize` arithmetic and returns `ResourceLimit` before
   metadata creation when the total exceeds `u16::MAX`.
8. **Deep retry atomicity.** The synthetic snapshot now copies the complete
   `JSJitExecFrame` and every pointed argument, local, active-stack backing, and
   bytecode buffer. All unsupported coercion, modulo, and refcount retry tests
   compare this deep snapshot rather than only the frame struct.
9. **Older ABI-info callers.** Rust and C 1.0/1.1 prefix queries initially
   returned `JS_JIT_BACKEND_INVALID_ARGUMENT`. `JS_GetJitABIInfo` now reads the
   caller size with `memcpy`, accepts the minimum known 1.0 prefix, constructs
   the full current 1.2 record, and copies only `min(caller_size, current_size)`.
   The prefix receives the current full size/version while adjacent canaries
   remain unchanged.

### Corrected pure Tier 1 boundary and atomicity proof

The native subset accepts only immediate, non-owning values at entry. Before
the generated function can poll, write `frame.result`, or otherwise make a
C-visible change, it:

- validates every argument and local tag;
- walks `[stack_base, stack_top)` in 16-byte `JSValue` increments and validates
  every active stack tag;
- branches directly to the shared retry exit if any tag is refcounted.

Local and operand-stack transformations remain SSA-only. Unsupported dynamic
`+`, `%`, refcount-requiring return, or ownership-sensitive frame operations
therefore retry with the frame and all pointed buffers byte-for-byte unchanged.
Task 8 remains responsible for duplication/free/coercion helpers and exact
remainder semantics.

### Exact safe-point and poll proof

`FrameStateId(n)` is encoded as Cranelift source location `n`. Zero is a valid
source location; Cranelift's default/no-location sentinel is `u32::MAX`, which
is rejected for frame-state IDs.
Polls, exits, and OSR states therefore cannot collapse merely because they
share a bytecode PC. OSR labels emit a source-tagged non-null frame branch so
their state has an observable non-call machine range. After code generation,
compilation requires each required source location to exist and requires every
selected offset to be both in the machine-code range and distinct.

Polling is conservative at every non-entry basic-block entry in addition to
function entry, backward branches, immediately before returns, and each 1,024
straight-line source operations. Thus a forward branch cannot jump around a
lexically placed poll; every runtime path remains below the required 4,096
operation ceiling.

### Publication, relocation, and native unwind proof

The corrected direct publication path is:

```text
host triple + feature validation
  -> prepare native unwind record
  -> allocate RW memory
  -> write code and any in-allocation unwind bytes
  -> resolve all symbolic/function-relative targets
  -> validate the complete relocation batch
  -> apply only validated relocations
  -> declare entry target
  -> transition RW to RX
  -> register native unwind information
  -> return one metadata-owning execution pin
```

Relocation kind and symbolic target preservation remain unchanged. The added
`X86SecRel` test proves that a section-relative relocation without a section
base is rejected atomically and never mutates writable bytes.

On Linux x86-64/AArch64, the owner retains a complete zero-terminated
`.eh_frame` section registered with `__register_frame`. On macOS x86-64/AArch64,
the registration pointer is advanced past the CIE to the individual FDE, as
required by LLVM libunwind's `__register_frame` implementation. On Windows
x86-64, Cranelift's `UNWIND_INFO` is DWORD-aligned and appended to the code
allocation, and a pinned `RUNTIME_FUNCTION` is registered with
`RtlAddFunctionTable`; final drop calls `RtlDeleteFunctionTable` before unmap.
The platform contracts are documented by
[LLVM libunwind](https://github.com/llvm/llvm-project/blob/main/libunwind/src/libunwind.cpp),
[Microsoft's x64 exception-handling guide](https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64),
and the
[dynamic function-table API](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-rtladdfunctiontable).

Windows ARM64 publication explicitly returns
`UnwindRegistrationUnsupported`: this round does not synthesize the complete
ARM64 `.pdata`/`.xdata` record from Cranelift's unwind-code bytes, so it does
not claim registration. Cross-target compilation remains allowed; only native
publication falls back. The required format is described by
[Microsoft's ARM64 exception-handling guide](https://learn.microsoft.com/en-us/cpp/build/arm64-exception-handling).

### ABI compatibility and binding proof

The ABI remains version 1.2; no header layout changed in this review round.
The minimum accepted legacy prefix ends immediately before the 1.1 execution
frame/exit fingerprint tail. Both C `_Static_assert`s and Rust `offset_of!`
assertions prove the 1.0 and 1.1 prefix sizes. Each language surrounds the
prefix with a 16-byte canary and verifies that the current full struct size,
major, and minor are reported without an out-of-prefix write.

Fresh bindgen parity and `update-bindings` both passed without changing any of
the nine bundled target bindings, which is the expected result for a behavior-
only C implementation fix.

### Final verification

```text
$ cargo test -p rquickjs-jit --features compiler,test-support --test baseline
19 passed; 0 failed

$ cargo test -p rquickjs-jit --test baseline --release \
    --features compiler,test-support
19 passed; 0 failed

$ cargo test -p rquickjs-jit --features compiler,test-support \
    --test platform --test abi
25 passed; 0 failed

$ cargo test -p rquickjs-jit --test abi --features test-support,bindgen
11 passed; 0 failed (including fresh-bindgen parity)

$ cargo check -p rquickjs-sys --features jit-abi,update-bindings
Finished successfully; no bundled-binding diff

$ cc -std=c11 -D_GNU_SOURCE -DCONFIG_JIT_ABI=1 \
    -I target/debug/build/rquickjs-sys-a74c8fe3324e0b1d/out \
    -I sys/quickjs sys/quickjs/api-test.c \
    target/debug/build/rquickjs-sys-a74c8fe3324e0b1d/out/libquickjs.a \
    -lm -ldl -lpthread -o /tmp/rquickjs-task7-api-test
$ /tmp/rquickjs-task7-api-test
Exited successfully

$ cargo test -p rquickjs-jit --all-targets \
    --features compiler,test-support
159 passed; 0 failed

$ cargo test --workspace --all-targets
All workspace tests passed

$ cargo clippy -p rquickjs-jit --all-targets \
    --features compiler,test-support -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
Finished successfully

$ git diff --check
Finished successfully in root and nested QuickJS repositories
```

The release build still reports QuickJS's pre-existing `buf2` GCC warning, and
the workspace build still reports the pre-existing unused `Command` import in
`sys/build.rs`. Neither warning is introduced by this round.

### Cross-target limitations and final self-review

Local native execution and unwind registration were verified only on Linux
x86-64. AArch64 Linux and macOS System V registration paths, macOS FDE pointer
selection, and Windows x86-64 dynamic function-table registration require their
native CI runners. No additional target or toolchain was installed. Windows
ARM64 is an intentional publication fallback as described above.

Final self-review confirmed:

- no `ExecutableCode` clone can escape the metadata-owning
  `PublishedBaselineCode` pin;
- registration is created only after successful RX publication, and failed
  registration drops/unmaps the executable rather than returning unregistered
  code;
- registration is destroyed before the mapping on final pin drop;
- state offsets cannot use a missing-location zero fallback or alias another
  required state;
- all retry assertions use deep snapshots;
- `%` is absent from the claimed native semantics; and
- no general Task 8 runtime helper was introduced.

Review-fix commits:

- Nested QuickJS: `85aaac8 fix(jit): support ABI info prefixes`
- Root implementation: `d7da0de fix(jit): harden pure baseline publication`
- This appended report is committed separately.

---

## Review fix round 2 (2026-08-30)

Status: **DONE**

Root review-fix commit:
`0f14d3e fix(jit): make baseline safepoints exact`

Nested QuickJS change: none; the nested repository remains clean at
`85aaac8 fix(jit): support ABI info prefixes`.

This section supersedes the round-1 safe-point claim that a source-range start
was sufficient for a poll, the statement that zero was Cranelift's default
source location, and the assumption that pointer equality alone made live
stack scanning safe.

### RED/GREEN evidence

1. **Exact poll return addresses.** The new safe-point audit was initially RED
   at compile time because artifacts exposed neither a location kind nor the
   Cranelift call-return table. Poll states now carry a unique `SourceLoc`
   only on their `call_indirect`. Compilation reads
   `compiled.buffer.call_sites()` and accepts exactly one return address inside
   exactly one matching source range. Runtime-API loads keep the default
   source location. Entry poll, return poll, and return marker at bytecode PC
   zero have distinct native offsets.
2. **Exact non-call markers.** Applying the initial range rule to loop/OSR
   fixtures exposed a second RED: an unused marker load could disappear, and a
   `trapz` marker produced separate hot and cold ranges. Non-call states now
   emit one source-tagged branch that reasserts the non-null entry-frame ABI
   invariant and otherwise continues normally; an invalid null frame takes
   the existing retry edge. Missing, duplicate, or multiple source ranges,
   call sites, and native offsets are rejected as `InvalidArtifact`.
3. **Deep retry atomicity.** Extending `SyntheticFrameSnapshot` with the poll
   counter and interrupt threshold made the modulo fixture RED: the former
   stub polled once before retrying. Any function containing `%` now emits its
   retry exit immediately after the sret/frame parameters, before reading the
   frame or invoking the entry poll. Tag-dependent refcount/dynamic guards
   likewise remain before polling. Snapshots also include the test-only
   backtrace-capture flag and captured PCs, so every atomic-RETRY fixture
   compares all pointed test state as well as frame and value buffers.
4. **Validated bounded stack scanning.** A zero-length but misaligned range was
   the safe RED reproducer: the old equality loop accepted it and reached the
   poll. Generated code now checks both pointers non-null, both 16-byte
   aligned, unsigned `top >= base`, a whole-slot checked difference, and slot
   count no greater than the verified maximum before dereferencing. It scans
   with a bounded `(cursor, remaining)` loop. Null, reversed, misaligned,
   oversize, and address-wrap fixtures all return retry without polling,
   faulting, hanging, or changing the deep snapshot. Synthetic empty stacks
   now own an explicitly 16-byte-aligned backing slot while retaining
   `top == base`.
5. **Windows ARM64 fallback.** Tests that unwrap native publication are
   excluded on Windows ARM64. A dedicated target test compiles Tier 1 code
   successfully and requires publication to return
   `UnwindRegistrationUnsupported`, rather than failing the suite or claiming
   registration for an unsupported format.
6. **Real System V unwind.** On GNU/Linux, the existing test-only interrupt
   poll callback captures a libc backtrace on its next invocation. The test
   requires the trace to contain the exact published poll return PC retained
   from Cranelift and then continue into a non-inlined Rust caller. It passes
   in debug and release, exercising the registered `.eh_frame` with a real
   unwinder rather than checking registration lifetime alone. No production
   runtime helper or ABI field was added.

### Safe-point representation and rejection rules

Artifact frame states now retain:

- `CallReturn` or `Marker` as an explicit location kind;
- the unique Cranelift source-location value;
- the complete source range; and
- the exact call return address or marker-range start as `code_offset`.

`SourceLoc::default()` is the all-ones sentinel (`u32::MAX`), while source
location zero is valid and belongs to `FrameStateId(0)`. For a poll, API table
and function-pointer loads are untagged and only `call_indirect` receives the
state location. The accepted call return satisfies
`range.start < return_address <= range.end`. For a marker, the exact location
is `range.start`, and any call return within its range is an artifact error.
Every required state must have one nonempty range, exactly one kind-appropriate
native location, an in-bounds offset, and an offset distinct from every other
state.

### Retry and stack safety proof

The unconditional modulo check occurs before frame buffer loads, refcount
guards, and the entry poll. A modulo execution can therefore change only its
hidden caller-owned `JSJitExit` result, never the `JSJitExecFrame`, poll state,
or any pointed frame buffer. Dynamic/refcounted input guards read but do not
mutate their buffers and reach retry before the first poll.

For live stack validation, the verified maximum stack depth is embedded as a
machine constant. Only after pointer and length checks pass can the counted
loop read a tag at `cursor + value_tag`; `remaining` decreases on each edge,
so malformed pointer equality cannot create an unbounded scan. The generated
code continues to use the selected ISA's pointer type and 16-byte `JSValue`
stride.

### Verification

```text
$ cargo test -p rquickjs-jit --features compiler,test-support --test baseline
21 passed; 0 failed

$ cargo test -p rquickjs-jit --test baseline --release \
    --features compiler,test-support
21 passed; 0 failed

$ cargo test -p rquickjs-jit --features compiler,test-support \
    --test platform --test abi
25 passed; 0 failed

$ cargo test -p rquickjs-jit --test abi --features test-support,bindgen
11 passed; 0 failed, including fresh-bindgen parity

$ cargo check -p rquickjs-sys --features jit-abi,update-bindings
Finished successfully; no bundled-binding or nested-repository diff

$ cc -std=c11 -D_GNU_SOURCE -DCONFIG_JIT_ABI=1 ... \
    -o /tmp/rquickjs-task7-round2-api-test
$ /tmp/rquickjs-task7-round2-api-test
Exited successfully, including ABI 1.0/1.1 prefix canaries

$ cargo test -p rquickjs-jit --all-targets \
    --features compiler,test-support
161 passed; 0 failed (Windows ARM64 target test cfg-disabled locally)

$ cargo test --workspace --all-targets
All workspace tests passed

$ cargo clippy -p rquickjs-jit --all-targets \
    --features compiler,test-support -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
Finished successfully

$ git diff --check
Finished successfully in root and nested QuickJS repositories
```

The release build still emits QuickJS's pre-existing `buf2` GCC warning. The
workspace build still emits the pre-existing unused `Command` import warning
from `sys/build.rs`. Neither warning originates in this round.

### Cross-target limitations and self-review

The host has only `x86_64-unknown-linux-gnu` and
`wasm32-unknown-unknown` Rust targets installed; no target or toolchain was
installed for this task. AArch64 machine-code generation remains covered by
Cranelift cross-target compilation and host-publication rejection, but native
execution still belongs to platform CI. The Windows ARM64 fallback test must
run on its native CI runner because the unsupported unwind path is selected by
the build target.

Final self-review confirmed:

- every poll offset is an actual Cranelift call return, never an API-load or
  first-range approximation;
- every marker has one explicit non-call range and cannot silently disappear;
- retry fixtures compare poll/backtrace state and all frame buffers;
- no malformed stack range is dereferenced before validation, and the scan is
  count-bounded by verified metadata;
- Windows ARM64 never returns an executable with unregistered unwind data;
- GNU/Linux debug and optimized code both unwind through an exact generated
  poll PC; and
- no QuickJS header, ABI version, bundled binding, general Task 8 helper, or
  nested repository file changed in this round.

Review-fix commits:

- Root implementation: `0f14d3e fix(jit): make baseline safepoints exact`
- Nested QuickJS: unchanged at
  `85aaac8 fix(jit): support ABI info prefixes`
- This report is committed separately.

---

## Review fix round 3 (2026-08-30)

Status: **DONE**

Root review-fix commit:
`263b4c8 fix(jit): hoist baseline retry guards`

Nested QuickJS change: none; the nested repository remains clean at
`85aaac8 fix(jit): support ABI info prefixes`.

This section supersedes the round-2 statements that operational dynamic guards
already ran before polling and that a failed frame-state marker invariant could
take the shared retry edge. All interpreter retries are now decided in the
pre-poll entry prologue. A marker invariant failure traps instead of returning
`RETRY_INTERPRETER`.

### RED/GREEN evidence

1. **Deep retry atomicity.** The new undefined-unary-plus fixture was RED with
   an otherwise identical deep snapshot except for `PollState.poll_count`,
   which changed from zero to one. Null arithmetic and an uninitialized
   `get_loc_check` reproduced the same post-poll retry class. After guard
   hoisting, undefined plus becomes a no-read entry retry stub, while dynamic
   null arithmetic and entry-local initialization checks fail in the prologue.
   An explicit `set_loc_uninitialized` followed by `get_loc_check` is detected
   statically and also becomes an entry stub. Every fixture now preserves the
   frame bytes, argument/local/stack buffers, poll counter, interrupt threshold,
   and backtrace-capture state byte for byte.
2. **Structural retry audit.** The initial CLIF audit was RED because a
   source-tagged marker after two polls still branched to the shared retry
   block. Generated CLIF now has one retry-result block, and every branch to it
   appears in the entry-validation region before the first `call_indirect`.
   Static entry stubs contain no load, poll, or frame state. Marker branches use
   a separate `trap user1` block. The audit covers frame/refcount/stack guards,
   numeric domain guards, and unconditional entry stubs.
3. **Immediate truthiness.** A short-bigint branch was RED with exit kind
   `RETRY_INTERPRETER`. The entry domain now accepts the complete supported
   non-owning immediate set: int, bool, null, undefined, short bigint, and
   float. Native truthiness tests cover zero/nonzero integers and booleans,
   null, undefined, zero/nonzero short bigints, positive and negative float
   zero, NaN, and a nonzero float. All return `DONE` with exact QuickJS boolean
   behavior.
4. **Native numeric loops retained.** The existing integer loop, overflowing
   add, floating arithmetic, bit-coercion, and captured QuickJS loop fixtures
   remained GREEN throughout. In particular, both loop fixtures still execute
   natively and return 4,950; this round does not replace arithmetic or branch
   functions with blanket retry stubs.
5. **Unsupported tables/properties.** A captured property-read function is
   rejected as `UnsupportedOpcode` before publication. Object, string, symbol,
   and bigint entry values exercise the refcount guard and deep-retry unchanged
   before the first poll.
6. **Exhaustive IR handling.** A compiler unit test constructs every `IrOp`
   variant, every `StackOp`, every `UnaryOp`, and every `BinaryOp`, then runs the
   entry-domain analysis. Its variant classifier has no wildcard arm, so adding
   an `IrOp` without explicitly extending the audit is a compile error. Modulo
   is the sole supported-IR case expected to request an unconditional entry
   retry in those otherwise valid synthetic functions.

### Provenance and entry-domain proof

The compiler runs a monotone CFG worklist before Cranelift lowering. Each
abstract value is a set of alternatives drawn from:

- immutable entry roots (`Argument(index)` and `Local(index)`); and
- statically produced kinds (`Number`, `Boolean`, `Null`, `Undefined`,
  `ShortBigInt`, `Uninitialized`, or `Other`).

Arguments, locals, and the abstract operand stack are propagated through every
IR operation. CFG joins union their alternatives; loop backedges are iterated
to a fixpoint. Constants have a known kind, numeric operations produce a known
number, and comparisons/logical-not produce a known boolean. Consequently,
numeric values derived from guarded entry roots and constants stay native
without repeated runtime checks.

Each consuming operation asks for an exact domain:

- numeric unary/binary/local operations require `int | float`;
- branch/logical-not/return require the supported immediate whitelist;
- checked local reads and non-initializing writes require initialized; and
- initializing checked writes require uninitialized.

A requirement on an entry root is accumulated and emitted once in the entry
prologue. Numeric implies initialized and is a subset of immediate. Conflicting
initialized/uninitialized requirements, or a statically known incompatible
alternative on any reachable path, select the no-read entry retry stub. Thus a
checked entry local receives an initialized guard, while control flow that can
explicitly make that local uninitialized before the check cannot enter native
execution until the exact Task 8 throw helper exists.

The existing non-refcount guards still cover every entry argument/local and
the validated bounded live-stack scan. The new domain guards execute after
those loads but before the first poll and before any C-visible frame mutation.
Operational lowering contains no retry guard. Return stores occur only after
the analysis has proved or hoisted the immediate domain.

### Retry, marker, and lowering invariants

There are exactly two source-level emissions of the retry result: the
unconditional entry stub and the shared entry-validation target. Every call to
the shared `guard` helper occurs before the IR lowering loop, while the first
possible poll is inside that loop. The CLIF test independently enumerates all
generated retry predecessors in representative normal/stub functions and
requires them to precede the first indirect poll.

Modulo and statically incompatible functions write only the hidden sret exit;
they do not load the execution frame, call the poll helper, or retain frame
states. Dynamic domain/refcount/stack failures may read validated entry data but
cannot mutate it and cannot reach a poll. Interrupt exits remain separate and
are not interpreter retries.

Frame-state markers no longer target retry. Their source-tagged non-null-frame
branch targets a dedicated trap on invariant failure, preserving the exact
non-call native range without falsely presenting an ABI violation as an atomic
interpreter fallback. If lowering ever reaches the end of the final IR block
without a terminator, compilation returns `InvalidArtifact` instead of
manufacturing a retry exit.

### Verification

```text
$ cargo test -p rquickjs-jit --features compiler,test-support --test baseline
25 passed; 0 failed

$ cargo test -p rquickjs-jit --release \
    --features compiler,test-support --test baseline
25 passed; 0 failed

$ cargo test -p rquickjs-jit --all-targets \
    --features compiler,test-support
166 passed; 0 failed (Windows ARM64 target test cfg-disabled locally)

$ cargo test --workspace --all-targets
All workspace tests passed

$ cargo clippy -p rquickjs-jit --all-targets \
    --features compiler,test-support -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
Finished successfully

$ git diff --check
Finished successfully in root and nested QuickJS repositories
```

The release build still emits QuickJS's pre-existing `buf2` GCC warning. The
workspace build still emits the pre-existing unused `Command` import warning
from `sys/build.rs`. Neither warning originates in this round.

### Cross-target limitations and final self-review

No target, toolchain, runtime helper, QuickJS header, ABI field, or bundled
binding changed in this round. Cranelift cross-target AArch64 generation and
host-publication rejection remain covered locally; native AArch64, macOS, and
Windows behavior remains assigned to the existing platform CI runners.

Final self-review confirmed:

- all interpreter-retry decisions precede the first poll and visible frame
  mutation;
- the full deep snapshot stays unchanged for undefined plus, null arithmetic,
  checked-local failures, modulo, malformed stack bounds, and refcounted entry
  values;
- refcounted or unsupported immediate values cannot reach raw SSA
  drop/dup/overwrite/return paths;
- numeric provenance across CFG joins and loop backedges keeps the Task 7
  arithmetic/branch loop deliverable native;
- supported immediate truthiness is exact, including short bigint, NaN, and
  negative zero;
- property/table bytecode remains rejected before publication;
- marker invariant failure traps and unterminated lowering rejects; and
- exact hidden sret, 16-byte `JSValue` copies, interrupt-safe points,
  relocation flow, unwind ownership, and target feature gating are unchanged.

Review-fix commits:

- Root implementation: `263b4c8 fix(jit): hoist baseline retry guards`
- Nested QuickJS: unchanged at
  `85aaac8 fix(jit): support ABI info prefixes`
- This appended report is committed separately.
