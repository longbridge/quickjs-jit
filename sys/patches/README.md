# QuickJS integration patches

Patches in this directory are applied in lexical order to the public QuickJS
submodule baseline after its sources are copied into Cargo's `OUT_DIR`.

`0001-rquickjs-jit.patch` is generated from quickjs-ng v0.15.1 (`fd0a021`)
through the reviewed rquickjs JIT integration commits ending at `eb389a3`.
`0002-runtime-feedback.patch` adds the versioned runtime type-feedback ABI and
interpreter probes without changing the pinned upstream baseline.
The build accepts exactly this patch name and byte digest, then verifies the
complete patched source manifest. Keeping the baseline and patch separate makes
upstream QuickJS upgrades an explicit rebase instead of requiring an
unpublished submodule commit.
