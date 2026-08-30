# Task 8 report: QuickJS helper ABI and exact value ownership

Status: **DONE_WITH_CONCERNS**

Root implementation commit: `6465a12 feat(jit): execute baseline code through QuickJS helpers`

Nested QuickJS commit: `fc3fdd2 feat(jit): add versioned helper ABI`

## Result

Task 8 connects published Tier 1 code to a generated, versioned QuickJS helper
table and makes ownership transfers explicit at every native/runtime boundary.
The implementation provides:

- one canonical `quickjs-jit-helpers.h` X-macro table for 13 stable helper
  IDs, C declarations, runtime table fields, C metadata, generated Rust
  metadata, and generated Cranelift signatures;
- the Task 7 interrupt poll unchanged as helper ID 0, with no second polling
  ABI;
- slot-based adapters that validate the complete execution identity before
  reading, duplicating, consuming, or freeing a `JSValue`;
- uniform `JSJitHelperStatus` returns, undefined failed outputs, and explicit
  borrowed/consumed/owned contracts;
- compiler lowering that materializes all owners into C-visible frame slots,
  updates bytecode PC and visible stack top, calls helpers indirectly, and
  transfers exceptions through the existing QuickJS catch/finally path;
- an explicit two-slot scratch tail with checked capacity, initialization,
  cleanup, compiler proof, and exit invariants;
- runtime constant resolution from the currently active
  `JSFunctionBytecode`, retained only by the RawRuntime-owned function
  registry while installed code exists;
- forced-baseline differential tests for ownership, constants, coercion event
  order, getters, reentrant calls, interrupts, and unary-plus BigInt failure;
- direct helper tests for validation order, refcount balance, setters,
  proxies, allocations, calls, forced cycle collection, and scratch layout;
- a read-only gpui-shell API inventory and compile-and-run compatibility
  fixture.

No helper failure returns `RETRY_INTERPRETER`. The only retry predecessors in
generated code remain entry-prologue guards that dominate the first poll and
precede every frame mutation.

## ABI result

The QuickJS JIT ABI is now 1.3. The runtime helper API is 1.1, the helper ABI
version is 1, the helper count is 13, and the generated table fingerprint is
`0x5eec58b76e7715d8`.

Append-only tails are:

- `JSJitEntryHandle`: `stack_map_count`, `helper_abi_version`;
- `JSJitExecFrame`: Task 7's `runtime_api`, followed by `runtime_id`,
  `frame_cookie`, and explicit end-exclusive `stack_capacity`;
- `JSJitABIInfo`: `helper_table_fingerprint`.

C and Rust layout fingerprints include every new field. The build fingerprint
also includes the helper-table fingerprint. Runtime-table field offsets and
Cranelift signatures are generated from the same X-macro source as the C
metadata. ABI tests compare ID, name, ABI type sequence, ownership sequence,
output ownership, behavior flags, count, version, and fingerprint across the
C table, bindgen declarations, and generated Rust view.

All nine bundled target bindings were regenerated:

- Linux GNU and musl on x86-64 and AArch64;
- macOS on x86-64 and AArch64;
- Windows GNU x86-64;
- Windows MSVC on x86-64 and AArch64.

Fresh host bindgen output matches the bundled host binding exactly, and all
nine bundles expose the same JIT declarations.

## Helper table and ownership matrix

`atom`, `index`, `operation`, `argc`, and `stack_map_id` are scalar arguments,
not values. `argv` below means a contiguous borrowed slot range.

| ID | Helper | Value inputs | Output | Behavior flags |
|---:|---|---|---|---|
| 0 | `POLL` | none | none | throwing, reentrant |
| 1 | `DUP` | input borrowed | owned | none |
| 2 | `FREE` | input consumed | none | reentrant, finalizing |
| 3 | `RESOLVE_CONST` | none | owned | none |
| 4 | `TO_NUMERIC` | input consumed | owned | throwing, allocating, reentrant, finalizing |
| 5 | `TO_BOOL` | input consumed | owned | reentrant, finalizing |
| 6 | `ADD_SLOW` | left consumed, right consumed | owned | throwing, allocating, reentrant, finalizing |
| 7 | `COMPARE_SLOW` | left consumed, right consumed | owned | throwing, allocating, reentrant, finalizing |
| 8 | `GET_PROPERTY` | object borrowed | owned | throwing, allocating, reentrant |
| 9 | `SET_PROPERTY` | object borrowed, value consumed | none | throwing, allocating, reentrant, finalizing |
| 10 | `CALL` | function borrowed, `this` borrowed, `argv` borrowed | owned | throwing, allocating, reentrant |
| 11 | `NEW_ARRAY` | none | owned | throwing, allocating, reentrant |
| 12 | `NEW_OBJECT` | none | owned | throwing, allocating, reentrant |

Helper ownership is not bytecode stack effect. For example,
`GET_PROPERTY` borrows its receiver, `SET_PROPERTY` borrows its receiver and
consumes only the value, and `CALL` borrows every input. Lowering performs the
opcode-specific receiver/function/argument pops with explicit `FREE` calls in
QuickJS interpreter order.

The numeric conversion adapter uses QuickJS's exact `OP_plus` slow path. This
includes ToNumeric coercion and the required unary-plus rejection for both
short and heap BigInts. It still consumes its source and leaves an owned
result or an undefined output on exception.

## Validation and exception transfer

Before any value slot is resolved, every adapter validates:

1. non-null and mutually matching frame/runtime/context;
2. exact frame size/flags and the canonical table pointer, size, major, and
   minor;
3. runtime ID, active-frame pointer, nonzero active frame cookie, and cookie
   equality;
4. entry size/reserved fields, non-null entry/pin, helper ABI version, and
   active stack-map count;
5. stack-map ID when the helper has one;
6. undefined frame result and absence of a pre-existing exception;
7. the current stack frame's active bytecode function, realm, function ID,
   generation, bytecode start, argument/local/stack bases, capacity, PC,
   visible stack range, and strictness;
8. helper-specific slot ranges, atom/index/operation ranges, non-aliasing, and
   undefined output preconditions.

An invalid validation path raises an uncatchable internal error before the
sentinel value used by adversarial tests is read, duplicated, consumed, or
freed. Valid language failures preserve the original QuickJS exception,
consume every argument promised as consumed, and leave every valid output
undefined. Before a throwing, allocating, reentrant, or finalizing operation,
lowering stores `frame.pc`, `frame.stack_top`, and all live values; native
validation then publishes the same PC to `sf->cur_pc`.

Helper failure exits directly as `JS_JIT_EXIT_EXCEPTION`. QuickJS validates
the logical top and PC, releases the execution pin, and enters its existing
local exception-search labels. Interrupts use the same helper ID 0 contract
and remain uncatchable.

## Scratch and GC ownership proof

QuickJS reserves exactly two `JSValue` slots after the logical operand stack
when `CONFIG_JIT_ABI` is enabled. `frame.stack_capacity` names the end of that
allocation. Both slots start as `JS_UNDEFINED`, and `sf->var_refs` begins only
after them; a non-JIT build retains the original layout.

The compiler computes capacity in widened `u32` arithmetic, rejects overflow,
and exhaustively assigns each helper family a simultaneous scratch requirement
of at most two. A synthetic logical capacity of `u16::MAX` proves the addition
does not wrap. Zero-stack, captured-var-ref, overflow, and non-JIT builds are
covered directly.

Owned helper results first live in a frame-visible scratch slot. Lowering then
moves the result to its logical destination, clears the source to undefined,
and performs any borrowed-input frees explicitly. On helper exception,
interrupt, deopt, retry, and done exits, the scratch tail must be entirely
undefined. QuickJS checks this independently after native return; if generated
code violates it, QuickJS frees the known C allocation's scratch owners and
raises an uncatchable internal error.

Stack maps remain deopt/debug metadata and are never treated as GC roots.
Stress mode allocates a cycle and runs cycle collection before and after every
relevant helper operation while live owners remain in argument, local,
operand, or scratch slots. The post-exception stress path temporarily removes
the language exception, runs allocation/collection, discards any artificial
stress exception, and restores the original exception exactly. Reentrant
`CALL`, `Symbol.toPrimitive`, getters/proxies, setters, and finalizing `FREE`
paths are all exercised under these rules.

## Constants and runtime teardown

Installed artifacts contain only pointer-free function ID/generation keys and
copied metadata. Workers never receive raw bytecode or `JSValue` pointers.
The Task 3 `RuntimeConstants` value remains a short-lived snapshot-thread
capture aid, but installed code has no dependency on it and does not retain
its strong `Runtime` clone.

The RawRuntime-owned `FunctionRegistryOwner` is the sole installed-code
retainer. It duplicates the function/context by `(function_id, generation)`,
thereby retaining the constant pool transitively, and clears those values
under runtime ownership before the runtime is freed. `RESOLVE_CONST` validates
that this exact active bytecode function is current and duplicates
`b->cpool[index]` into the requested output slot.

Tests prove a heap BigInt constant remains valid after the worker snapshot and
its `RuntimeConstants` are dropped, that the registry detaches when the guard
is dropped, and that RawRuntime teardown releases retained functions even when
a public guard survives. The weak public registry handle cannot create a
runtime/artifact/registry cycle.

## Compiler lowering

Tier 1 now lowers the Task 8 helper-bearing subset for constant-pool loads,
object/array allocation, field get/set, calls, borrowed argument/local loads,
owned stack permutations and drops, unary truth conversion/plus, dynamic
addition, and comparisons.

Every helper instruction owns one or more exact helper frame states. The
logical state carries a complete live-slot map, while its materialized depth
may include the two explicit scratch slots. Generated code loads the table and
helper pointer from `frame.runtime_api`, calls the generated C signature via
`call_indirect`, and associates the call return with that helper state.

Consumed values are cleared in their physical frame slots. Borrowed values are
duplicated or explicitly freed according to the surrounding opcode. Owned
returns are transferred without an extra refcount operation. Dynamic helper
operations therefore do not use a post-mutation `RETRY_INTERPRETER`; a helper
exception exits immediately with the materialized logical stack depth.

## TDD evidence

### RED: canonical helper ABI

The first ABI tests failed because the linked C API exposed only Task 7's poll
field, no helper metadata/table existed, the Rust bindings had no helper IDs or
tail fields, and bundled/fresh bindgen output diverged. The generator tests
also had no source from which to derive helper signatures or offsets.

### RED: forced native semantics

The initial forced-baseline cases failed compilation for helper-bearing
bytecodes: there was no constant resolver, owned dup/free path, slow addition,
property access, allocation, or call lowering. The tests require an installed
native entry and compare canonical result/exception/event order; simply
executing the interpreter was not accepted as the implementation.

### RED: explicit scratch capacity

The first real semantic execution exposed that placing owned helper outputs
above the logical operand stack was not backed by an explicit QuickJS frame
allocation. The resulting invalid access was replaced with an ABI-visible
capacity and two initialized slots. A later maximum-capacity test failed with
`CompileFailure::ResourceLimit` because scratch arithmetic passed through
`u16`; widening the proof to `u32` made `u16::MAX + 2` safe and checked.

### RED: exact unary-plus failure

The final self-review differential demonstrated that plain ToNumeric returned
`1n` for `+1n`, causing canonicalization to fail instead of producing
QuickJS's `TypeError:bigint argument with unary +`. The adapter was changed to
QuickJS's exact unary arithmetic slow path. The same forced native test then
passed without a retry/deopt fallback.

### GREEN: focused and full tests

```text
$ cargo test -p rquickjs-jit --all-features -- --test-threads=1
192 passed; 0 failed across all rquickjs-jit test binaries

$ cargo test -p rquickjs-jit --test semantics --all-features -- --test-threads=1
8 passed; 0 failed

$ cargo test -p rquickjs-jit --test helpers --all-features -- --test-threads=1
11 passed; 0 failed

$ cargo +nightly test --workspace --all-features --exclude rquickjs -- --test-threads=1
All included unit, integration, and doc tests passed
(including core 186, JIT helper 11, lifecycle 44, and native-boundary 26;
the then-current semantics suite had 7 tests, followed by the final 8-test
focused/full-JIT reruns above after the BigInt regression was added)

$ cargo test --workspace -- --test-threads=1
All default-feature workspace tests passed

$ cargo test -p rquickjs-jit --test semantics \
    --features compiler,test-support,bindgen,rquickjs-core/dump-leaks \
    -- --test-threads=1
8 passed; 0 failed; no QuickJS leak dump

$ ASAN_OPTIONS=detect_leaks=0 RUSTFLAGS=-Zsanitizer=address \
    cargo +nightly test -p rquickjs-jit --test semantics --all-features \
    --target x86_64-unknown-linux-gnu -- --test-threads=1
8 passed; 0 failed; no AddressSanitizer finding

$ cargo test -p rquickjs-jit --test abi \
    --features test-support,bindgen \
    bundled_targets_match_fresh_bindgen_output -- --exact --test-threads=1
1 passed; 0 failed

$ cargo check -p rquickjs-sys --features jit-abi,update-bindings
Finished successfully

$ cargo check -p rquickjs-sys --no-default-features
$ cargo check -p rquickjs-core --no-default-features
Both finished successfully

$ cargo clippy -p rquickjs-jit --all-targets --all-features -- -D warnings
Finished successfully on the repository's stable toolchain

$ cargo fmt --all -- --check
$ git diff --check
Both finished successfully
```

## gpui-shell compatibility spike

The required sibling inventory was read-only. Seven direct production call
sites were found under `../gpui-component/crates/shell/src/engine/quickjs`:
runtime construction, full context construction, tuple module loader setup,
interrupt handler setup, and three pending-job drain sites. The named fixture
also covers the shell's memory/stack limits, module evaluation, promise job
driving, `is_job_pending`, context drop, and runtime teardown. It compiles and
runs against `JitRuntime`; the sibling checkout was not modified.

## Environmental limitations and remaining scope

1. The brief's exact `-Zbuild-std` ASan command cannot start because the
   installed nightly toolchain lacks `rust-src`. No toolchain component was
   installed. The cached-standard-library ASan variant executes all eight
   bodies successfully; LeakSanitizer then cannot attach under this
   environment's `ptrace_scope`. Running with `ASAN_OPTIONS=detect_leaks=0`
   is fully green, while QuickJS's independent dump-leaks run is also green.
2. `cargo +nightly test --workspace --all-features` reaches the root
   `rquickjs` trybuild suite and reports two of fourteen checked-in diagnostic
   snapshots as mismatches:
   `async_compile_fail/async_nested_contexts.rs` and
   `async_parallel_compile_fail/capture_rc.rs`. The newer nightly emits
   different `AsyncFn`/`ParallelSend` diagnostics. Excluding only the root
   package makes the complete remaining all-features workspace green.
3. Nightly `clippy -D warnings` rejects pre-existing uses of the newly
   deprecated atomic `fetch_update` name and the root all-features deprecated
   feature shims. Stable strict Clippy for the Task 8 crate is green; these
   unrelated migrations were not folded into this task.
4. Only Linux x86-64 native code was executed locally. Cross-target bindings
   and generated declarations have parity coverage, but macOS, Windows, and
   AArch64 runtime behavior still needs their native CI runners.
5. This task intentionally does not broaden into Task 9's full opcode set.
   Unsupported operations such as modulo retain their proven pre-mutation
   retry behavior until an exact helper/lowering is added.

## Commits

- Nested QuickJS: `fc3fdd2 feat(jit): add versioned helper ABI`
- Root implementation: `6465a12 feat(jit): execute baseline code through QuickJS helpers`

## Independent-review correction: semantic test feature gates

The independent review found that `helpers`, `semantics`, and
`gpui_shell_surface` used crate-level `cfg` attributes. A command that omitted
`compiler,test-support` could therefore report success after running zero
tests. Each target now declares Cargo-native `required-features`, so an
under-featured targeted command exits with an error instead of succeeding. A
metadata-based regression test checks the structured Cargo target contract,
without parsing human-oriented test output, and the Linux CI job runs all three
targets with `--features compiler,test-support`.

TDD evidence:

```text
$ cargo test -p rquickjs-jit --test required_features \
    -- --exact native_semantics_targets_require_their_execution_features
RED: required-features array was absent

$ cargo test -p rquickjs-jit --test semantics
error: target `semantics` ... requires the features: `compiler`, `test-support`
exit status 101 (intentional missing-feature gate)

$ cargo test -p rquickjs-jit --features compiler,test-support \
    --test required_features --test helpers --test semantics \
    --test gpui_shell_surface -- --test-threads=1
21 passed; 0 failed; every named target executed at least one test

$ cargo test -p rquickjs-jit --all-targets \
    --features compiler,test-support -- --test-threads=1
192 passed; 0 failed; only target/platform-inapplicable binaries reported zero
```
- This report is committed separately so it can record both implementation
  commit IDs.
