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

The build accepts only the patch names and byte digests listed in
`build_support/patch.rs`, then verifies the complete patched source manifest.
Keeping the baseline and patch separate makes
upstream QuickJS upgrades an explicit rebase instead of requiring an
unpublished submodule commit.
