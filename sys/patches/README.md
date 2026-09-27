# QuickJS integration patches

Patches in this directory are applied in lexical order to the public QuickJS
submodule baseline after its sources are copied into Cargo's `OUT_DIR`.

`0001-rquickjs-jit.patch` is generated from quickjs-ng v0.15.1 (`fd0a021`)
through the reviewed rquickjs JIT integration commits ending at `eb389a3`.
`0002-runtime-feedback.patch` adds the versioned runtime type-feedback ABI and
interpreter probes without changing the pinned upstream baseline.
`0012-shape-retention.patch` retains shared plain-object shapes observed by JIT
feedback so short-lived receivers can reuse their property guards. Each context
has at most 64 entries and a 16 KiB storage budget, including cache metadata,
shape backing storage, and conservatively charged property atoms (excluding
allocator overhead). GC traces the retained shapes; context destruction, JIT
detach, and Object class-prototype replacement release them.
`0013-feedback-gate.patch` skips arithmetic feedback collection when no backend
is listening. Recording rechecks suspension after primitive conversions, which
can reenter JavaScript or host callbacks.
`0014-inactive-probes.patch` also gates call, return, branch, and conversion
feedback, and initializes reserved native helper slots only on native entry.
`0015-cold-object-probes.patch` gates property and call-site probes before
entering their recorders, avoiding calls for functions with disabled feedback.

`0017-shape-generation.patch` replaces per-access descriptor hashing with a
lazy, nonzero runtime-unique shape token. Feedback assigns tokens; descriptor
mutations clear them, including callback windows between preparation and the
actual mutation. New and cloned shapes start unobserved. Copy-on-write leaves
unchanged shared shapes valid. The runtime counter never resets or wraps;
exhaustion suppresses observations for new shapes instead of reusing tokens.
Native guards compare the live shape pointer and token using the append-only,
fingerprinted ABI 1.21 property-layout descriptor. A zero token always misses.
On this 64-bit build the shape header grows from 64 to 72 bytes and the runtime
adds one eight-byte counter; no allocation or retained receiver is introduced.

Patch number 0016 is intentionally absent: the helper-PC cache experiment was
removed after regression measurements. Frozen benchmark evidence retains that
experiment; production uses the prior validated-PC cursor without its extra
eight bytes per bytecode object.

`0018-inline-frame-recovery.patch` adds the ABI 1.22 versioned inline-recovery
table. Native callers can enter bounded runtime-owned shadow frames, check the
callee identity, and either leave normally or resume in the interpreter after
deoptimization or an exception. Recovery preserves QuickJS ownership and
stack effects, supports nested inline frames, and caps each runtime at 16
frames and 1 MiB of shadow-frame storage.

`0019-array-feedback.patch` adds ABI 1.23 pre-effect array-mode observations at
element loads, element stores and length reads. The existing feedback event
layout is unchanged: a finite mode occupies `slot`, and access/hazard masks
occupy `flags`. Observations retain no receiver pointers and do not replace
native class, buffer, storage, bounds or observable-property guards. Fixed
views over resizable buffers are detected from the backing buffer even when
the view does not track its length.

`0020-typed-array-guard.patch` adds the ABI 1.24 versioned typed-array leaf
guard. Feedback only selects an Int32Array or Float64Array candidate; the leaf
then revalidates the live receiver, exact built-in `length` lookup, fixed
non-resizable and non-shared backing, attachment, mutability, byte bounds and
data pointer before returning `{ data, count, mode }`. The table advertises
zero effects and the receiver is passed by pointer so generated calls do not
depend on platform-specific aggregate argument classification.

`0021-helper-boundary.patch` makes helper-boundary PC validation O(1). Each
bytecode lazily builds an instruction-boundary bitmap (`byte_code_len / 8 + 1`
bytes, runtime-allocated, freed with the bytecode and reported in
`JS_ComputeMemoryUsage`) using exactly the linear scan's decoding rules; if
the allocation fails the previous validated-PC scan answers instead. Helper
frame validation keeps every identity, ABI and stack check but inlines the
pending-exception test and moves rejection out of line.
`JS_JitInlineEnter` no longer rescans the call and continuation PCs (the frame
PC is already a proven boundary and the continuation is proven as the next
one) and reserves shadow frames from a runtime-owned LIFO arena (grown only
while empty, at most the 1 MiB inline budget plus alignment, released with the
runtime) with heap fallback; sanitizer builds always use the heap. No public
ABI or struct layout visible to the bindings changes.

`0022-inline-refcount.patch` pins the reference-count header that generated
code now updates in place. Static assertions require every payload a
`JS_VALUE_HAS_REF_COUNT` value can address (objects, bytecode, strings, ropes,
symbols, BigInts and modules) to start with the C `int` count, and the ABI
value-layout fingerprint additionally hashes its offset and width. Native code
increments the count for DUP, decrements it for FREE while other references
remain, and still calls `JS_JitHelperFree` for the last reference and in
stress-GC frames. No ABI table or structure changes, so the ABI minor version
is unchanged; a library without this patch fails the fingerprint check.

`0023-fast-native-entry.patch` adds the ABI 1.25 callback-free native entry.
After a DONE exit whose pc=0 handle QuickJS caches, the optional
`entry_fast_grant` callback may grant a bounded budget (at most 65536) and a
backend-owned `JSJitFastEntryState`. While budget remains and the state epoch
still equals the cached entry epoch, calls of that function reuse the cached
handle without call feedback, `record_hot`, `entry_cache_epoch`,
`native_enter` or `native_exit`; QuickJS counts the executions in the state
and marks their frames `JS_JIT_FRAME_FAST_ENTRY`. A non-DONE exit still calls
`native_exit`, tagged `JS_JIT_EXIT_FAST_UNPAIRED`, and is never re-cached.
Suspension, feedback shutdown, a zero or changed epoch, or a malformed grant
all return to the full callback path. Calls that never reach native code keep
another function's still-admissible idle handle instead of evicting it. The
runtime grows by two 32-bit fields and one pointer; the backend vtable gains
one trailing member.

`0025-tier1-closures.patch` adds the Tier 1 closure helpers under ABI 1.25
(the integration's single minor bump, taken by 0023) and bumps the runtime API
to 1.10, as an append-only helper-table tail (IDs 24-28): `FCLOSURE`, `GET_VAR_REF`,
`PUT_VAR_REF`, `CLOSE_LOC` and `SET_NAME`. Each mirrors its interpreter opcode
on the validated current stack frame (`js_closure`, `*var_refs[idx]->pvalue`
with the exact TDZ `ReferenceError`, `set_value`, `close_lexical_var`,
`JS_DefineObjectName`). `PUT_VAR_REF` consumes its value slot on success and
on exception; `JSJitVarRefMode` selects the plain, checked or
checked-initialization form. Attached var refs alias the interpreter's
argument/local storage, so generated code publishes those slots before every
helper and reloads them afterwards in functions that create closures.

`0026-tier1-generic-ops.patch` adds ABI 1.25 / runtime API 1.10 helper
`GENERIC_OP` (append-only helper ID 29 after the 0025 closure tail,
`MAP_OUT_TWO_OP`; runtime API offset 240, size 248 on 64-bit targets). It executes
`push_this`, `special_object` (arguments, this function, `new.target`, home
object, variable object and null-prototype objects), `get_var_undef`,
`delete_var`, `put_var`, the `typeof` family, `to_object`, `to_propkey2`,
`in`, `instanceof` and `delete` with the interpreter's own C routines. The
operation word packs the opcode and its 8-bit immediate; atoms travel in the
right operand and are validated. Each root native entry publishes a private
stack-allocated call context (receiver, `new.target`, caller `argc`/`argv`) that
is saved and restored around the pinned entry call; `push_this` and
`special_object` reject any frame that is not that root (for example an inline
view). Mapped (sloppy) `arguments` and `import.meta` stay interpreter-only.
On a language exception every named slot is released and cleared.

`0027-tier1-exception-regions.patch` adds the Tier 1 exception entry points
under ABI 1.25 (the integration's single minor bump, taken by 0023; no
helper IDs or runtime API slots are added). `JS_JitThrowValue` and `JS_JitThrowError` raise exactly like
`OP_throw` and `OP_throw_error`. `JS_JitCatchException` replays the
interpreter's `exception:` label for the active root frame (backtrace at the
throwing PC, then catchability) and, only after proving that unwinding would
stop at the named catch-offset slot and handler PC, releases the operands
above it and stores the caught value in that slot. A backtrace that would
only populate `ctx->error_back_trace` (released unread by the catch) is not
built when building it cannot run user code (`Error.prepareStackTrace`, a
non-numeric `Error.stackTraceLimit`) or define `stack` on the caught value.
Uncatchable exceptions are returned with the frame untouched so the ordinary
native exception exit lets the interpreter unwind them. The patch also stops
hot and feedback probes for
generator, async, eval and `with` bytecode when the backend requests a
snapshot the runtime must refuse; previously every later call, async resume
and loop poll re-requested it.

The build accepts only the patch names and byte digests listed in
`build_support/patch.rs`, then verifies the complete patched source manifest.
Keeping the baseline and patch separate makes
upstream QuickJS upgrades an explicit rebase instead of requiring an
unpublished submodule commit.
