# P1 item 3: inline DUP/FREE reference counting

Branch `perf/p1b-inline-refcount`, based on `82d3808` (0.12.9). This note
covers roadmap item P1.3 in
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md): generated
code updates QuickJS reference counts directly instead of calling
`JS_JitHelperDup` / `JS_JitHelperFree` for every ownership transition.

## What changed

- `jit/src/compiler/refcount.rs` holds the shared Cranelift emitters:
  - `JS_VALUE_HAS_REF_COUNT` compares the tag as `unsigned`, exactly as C
    does.
  - DUP is `ref_count++`.
  - FREE is `ref_count--` only while the count is greater than one.
  - Frame-flag checks keep stress GC on the helpers.
- The FREE helper still runs for the last reference, so finalizers, GC list
  maintenance, and reentrancy stay in C. It also still runs for every value in
  `JS_JIT_FRAME_STRESS_GC` frames, which keeps the stress collection points.
  Sites that used to call the helper for primitives keep doing so in stress
  mode (`lower_free_always_in_stress`, the `GetLocalPair` DUP, and frame-inline
  DUP/FREE).
- Tier 1 (`baseline.rs`):
  - These operations duplicate inline: get/put argument and local, the checked
    local, `dup`/`dup1..3`/`insert*`, and `get_loc0_loc1`.
  - These release inline: put, `drop`, `nip`, the `is_undefined`/`is_null`
    family, property receivers, and consumed generic CALL operands.
  - An inline release writes undefined into the consumed slot's frame memory,
    as the helper does. CALL and GET_PROPERTY outputs are cleared from scratch
    memory after the helper result is reloaded into SSA
    (`clear_reloaded_output`). Before this change, the following FREE helper's
    frame materialization cleared them implicitly. Exits reject non-undefined
    scratch slots, so without the clear the exit fails with "owned JIT scratch
    at exit".
- Tier 2 (`optimized.rs`, `optimized/frame_inline.rs`):
  - FREE of owned stack and local slots goes through `emit_opt_free_slot`.
    This covers consumed operands of specialized CALLs and the receiver of
    owned GetField.
  - `dup` of an `OwnedSlot` goes through `emit_opt_owned_dup`. It is inline
    when no borrowed alias is live, and stress mode keeps the old
    helper-and-materialize path with identical provenance.
  - Shadow-frame DUP/FREE are inline.
  - Borrowed-alias owner materialization
    (`JS_JitHelperMaterializeOwner`, which is a `js_dup`) is inline on paths
    that continue into helper or runtime calls (`opt_own_stack_for_helper`).
    Deopt and exception bridges keep the helper (`opt_own_stack_for_exit`):
    inlining them measurably perturbed hot-loop code
    (`int32array-traversal` was 0.94x).
  - Out-of-line placement:
    - The direct-call signature deopt block is now a cold block.
    - The helper continuation of owner materialization inherits the caller's
      coldness. Stress-only blocks are cold.
- ABI and layout: `sys/patches/0022-inline-refcount.patch` adds static
  assertions that every refcounted payload starts with the C `int` count.
  These cover objects, bytecode, strings, ropes, symbols, BigInt, and modules.
  The patch also hashes the count's offset and width into the value-layout
  fingerprint. `jit/src/abi.rs` pins `REF_COUNT_OFFSET = 0` and
  `REF_COUNT_BYTES = 4` in the same fingerprint, so a library without the patch
  fails ABI validation and the JIT stays closed. No ABI table or struct changed
  and the ABI minor version is unchanged. Bindings are unchanged.

## Helper counter semantics

`JS_JitGetHelperCounters` and `JS_JitGetHelperCount` now count only the
helper calls that still happen: last references, stress GC, and cold bridges.
Tests were updated on purpose:

- `generic_call_cleanup`: shared operands no longer reach FREE outside stress
  mode.
- `frame_inline`: the duplicated owner uses the helper only under stress. The
  surviving marker after GC remains the ownership evidence.
- `property_specialization`: an inline owner reload of `arg_buf`/`var_buf`
  counts as a root load.

`jit/tests/inline_refcount.rs` adds differential checks against the
interpreter in Tier 1, Tier 2, and automatic mode, each with and without
stress GC. Forced tiers must keep running native code. The file also asserts
that shared DUP/FREE skip the helpers while every last reference still
reaches FREE.

## Verification (this branch)

- `cargo test --release -p quickjs-jit-runtime --features compiler,test-support --tests`:
  791 passed, 1 failed, 1 ignored. The single failure is
  `mixed_direct_calls::tier2_mixed_edge_eliminates_helper_and_preserves_guard_misses`.
  That flake depends on wall-clock profitability and also fails on unmodified
  `82d3808` under the shared machine's load (5 of 12 runs failed there).
  `optimized::automatic_call_heavy_promotes_the_direct_edge_caller` is a
  similar timing flake, with 2 of 12 runs failing on `82d3808`. An earlier
  full run on this branch passed all 792 tests.
- The `jit_patch` test in `quickjs-jit-sys` passes 13 of 13. Clippy with
  `-D warnings` is clean, both with and without the compiler features.

## Noisy diagnostic (not publishable)

- Protocol: `jit-bench worker --mode automatic`, pinned to P-cores
  (`taskset -c 0-15`) because E-core placement makes samples bimodal.
- 9 alternating baseline and candidate processes per workload. Each value is
  the median over processes of `median(warmup_batch_ns[-16:])`.
- Baseline: `/home/jason/work/quickjs-jit/target/release/jit-bench` built at
  `82d3808`.
- Host: i7-13700KF, shared with about 15 other agents.
- Checksums matched interpreter mode in every run.
- Speed is baseline time divided by new time. Values above 1.00x are faster.

| Workload | 82d3808 (ms) | Branch (ms) | Speed |
| --- | ---: | ---: | ---: |
| objects-polymorphic | 23.878 | 19.342 | 1.23x |
| generic-call-fallback | 9.119 | 6.611 | 1.38x |
| mixed-quotes | 2.492 | 2.535 | 0.98x |
| arrays-typed | 2.980 | 2.810 | 1.06x |
| property-heavy | 0.099 | 0.099 | 1.00x |
| call-heavy | 0.060 | 0.044 | 1.35x |
| numeric | 0.029 | 0.029 | 1.01x |
| scalar-loop | 0.029 | 0.029 | 1.01x |
| int32array-traversal | 0.050 | 0.050 | 1.00x |
| packed-array-traversal | 0.048 | 0.048 | 1.01x |
| float64array-traversal | 1.091 | 0.858 | 1.27x |
| calls-closures | 3.304 | 3.364 | 0.98x |
| fibonacci-iterative | 0.997 | 0.989 | 1.01x |

The two results near 0.98x are mixed-quotes and calls-closures. Both spend
most of their time in the interpreter and in C: sort-callback bookkeeping and
closure opcodes that are still rejected. Interpreter mode shows a similar
build-to-build spread: mixed-quotes 0.99x and calls-closures 1.02x. Treat them
as unresolved until the paired three-engine matrix runs. The `call-heavy` gain
comes mostly from the cold direct-call deopt layout, not from reference
counting.

## Remaining work

- `lower_define_element` (Tier 1) still calls the DUP helper twice per
  element definition.
- Inlining owner materialization on deopt bridges is possible only once
  those bridges are laid out cold everywhere. That needs every deopt caller to
  mark its fail block cold; `emit_opt_deopt`, the element get/put guards, and
  the property-key guards currently do not.
- An owned `dup` with a live borrowed alias below it still uses the helper
  path with full frame materialization.
- The publishable three-engine matrix and README update are left to the
  integrator.
