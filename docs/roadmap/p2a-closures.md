# P2a closures: Tier 1 coverage slice

Branch `perf/p2a-closures`, based on `82d3808` (0.12.9). This is the first
slice of roadmap item P2.1 in
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md): the closure
opcodes are now covered in Tier 1. Tier 2 and native var-ref access are not
part of this slice.

## What is admitted

| Opcode family | Tier 1 policy | Lowering |
| --- | --- | --- |
| `fclosure`, `fclosure8` | `Helper(FClosure)` | `JS_JitHelperFClosure` → `js_closure` |
| `get_var_ref0-3`, `get_var_ref`, `get_var_ref_check` | `Helper(GetVarRef)` | `JS_JitHelperGetVarRef` (plain or TDZ-checked) |
| `put_var_ref0-3`, `put_var_ref`, `set_var_ref0-3`, `set_var_ref`, `put_var_ref_check` | `Helper(PutVarRef)` | `JS_JitHelperPutVarRef` (consumes its slot); `set_*` duplicates the value into the scratch slot first |
| `close_loc` | `Helper(CloseLocal)` | `JS_JitHelperCloseLoc` → `close_lexical_var` |
| `set_name` | `Helper(SetName)` | `JS_JitHelperSetName` → `JS_DefineObjectName` |

`set_name` belongs to roadmap item P2.2, but QuickJS emits it right after
`fclosure` for every anonymous arrow or function expression that is assigned to a
binding, so neither target workload is eligible without it.

The following remain rejected (`ClosureFrame`):

- `put_var_ref_check_init`: QuickJS emits it only for a derived constructor's
  `this` that an arrow initializes through `super()`, and those arrows also
  contain `get_super`, which is still rejected. No Tier 1 program can run it,
  so it has no opcode-case manifest entry.
- `make_var_ref`, `make_var_ref_ref`: these only feed
  `get_ref_value`/`put_ref_value`, which are still rejected.

The C side is `sys/patches/0025-tier1-closures.patch`: ABI 1.25 and runtime
API 1.10, with five helpers appended to the end of the helper table (IDs
24-28).

## Semantic safety argument

- **Captured slots alias the frame.** An attached `JSVarRef.pvalue` points into
  `sf->var_buf` or `sf->arg_buf`. Tier 1 keeps arguments and locals in SSA
  variables and publishes all of them before every helper call and poll. In a
  function that contains `fclosure`, it also reloads them after every helper
  and poll, so a write that a closure made during a call becomes visible to
  later native code, and a stale copy never overwrites it at the next publish.
  Only `fclosure` can capture this frame's slots: `eval`, `with`, `arguments`,
  `make_*_ref` and `define_class` are all still rejected. Native arithmetic
  keeps its use-site tag guards, so a closure that changes a slot's type
  deoptimizes; the regression test for this passes.
- **TDZ proofs.** Tier 1 proves `get_loc_check`, `put_loc_check` and
  `put_loc_check_init` statically. A closure can only move a lexical slot from
  uninitialized to initialized (through `put_var_ref_check_init`), never back.
  The proofs for `get_loc_check` and `put_loc_check` therefore stay sound: a
  slot the analysis believes uninitialized can only make the function retry.
  A function that combines `fclosure` with `put_loc_check_init` is rejected
  with `ClosureFrame`.
- **Ownership.** FCLOSURE and GET_VAR_REF write an owned value to an empty
  output slot. PUT_VAR_REF consumes its input on success and on a TDZ
  exception. After a failed PUT_VAR_REF, the exception exit publishes only the
  bytecode-visible stack below the consumed operand. SET_NAME borrows. Every
  helper runs the stress-GC points.
- **Other tiers.** Tier 2's opcode classifier still rejects all of these
  opcodes. Frame inlining rejects callees that contain `fclosure` or
  `close_loc`. Scalar or tagged leaf entries and effect-free inlining still
  require `closure_count == 0` and a closed operation set. OSR is still refused
  when `closure_count != 0`.

Evidence:

- `jit/tests/tier1_closures.rs` has 13 tests.
- `opcode-cases.json` has 21 new cases, each executed at its native PC under
  stress GC.
- The helper-family cases are in `differential.rs`.
- With the reload turned off, the three reload and type-change tests fail, and
  the stress-GC heap test and the per-iteration binding test do not report.

## Noisy diagnostic (not publishable)

Setup:

- Host: i7-13700KF, 24 logical CPUs, shared with about 15 other build and
  bench agents.
- Baseline binary: `82d3808` `target/release/jit-bench`.
- New binary: this branch.
- Command: `jit-bench worker --mode automatic --script
  benchmarks/scripts/W.js`.
- Sampling: 7 alternating base/new processes. Each process contributes the
  median of the last 16 `warmup_batch_ns` values; the table reports the
  median over processes.
- Checksums match interpreter mode for every run.

Speed is base/new, so values above 1.00x mean this branch is faster.

| Workload | Base ms | New ms | Speed vs base | New vs interpreter |
| --- | ---: | ---: | ---: | ---: |
| calls-closures | 3.304 | 3.275 | 1.01x (tied) | 1.00x |
| calls-recursion-closures | 6.452 | 8.723 | **0.74x** | 0.75x |
| exceptions-promises-async | 3.702 | 3.623 | 1.02x (tied) | 0.65x (unchanged `catch` blocker) |
| call-heavy (control) | 0.060 | 0.062 | 0.97x (bimodal load noise) | 25.6x |

`calls-closures`: `workload` is now Tier 1 eligible. Its Tier 2 attempts fail
on the closure opcodes, and the function then runs in the interpreter, so the
result is neutral.

`calls-recursion-closures`: `workload` now runs its loop in Tier 1. Every
iteration crosses the generic helper bridge about 9 times: GET_GLOBAL ×2, DUP,
CALL ×2, and FREE on the callee operands. Each crossing pays the full
`qjsjit_validate_helper_frame_state` check, which took about 19% of samples in
a gdb poor-man's profile. This is not specific to closures. The equivalent
closure-free program below runs its loop in Tier 1 on unmodified `82d3808`, at
**0.78x** the interpreter's speed (automatic 8.50 ms vs interpreter 6.65 ms,
7 pairs). Admitting closures only exposes this target to that existing Tier 1
call-bridge cost. The fixes are roadmap P1 (O(1) helper validation, cheaper
entry bookkeeping) and P3 (native call convention). Re-measure after those
land.

```js
function recursiveSum(n) { return n <= 0 ? 0 : n + recursiveSum(n - 1); }
function call4(fn, value) { return fn(value); }
function call3(fn, value) { return call4(fn, value); }
function call2(fn, value) { return call3(fn, value); }
function call1(fn, value) { return call2(fn, value); }
globalThis.state = { captured: 0 };
function update(value) { return (state.captured = (state.captured + value) | 0); }
function workload(iterations, seed) {
  state.captured = seed;
  for (let i = 0; i < iterations; i++) call1(update, recursiveSum(i & 7));
  return state.captured;
}
```

## Remaining work

1. **Tier 2 var refs.** Classify `get_var_ref*` as a tagged heap load and
   `put_var_ref*` as a heap store with exact effects, lower them through
   `emit_opt_owned_helper_push` or a native load, and model a closure-creating
   function's captured locals as memory (reload after every reentrant node)
   instead of SSA values. Tier 2 has no scalar path for this yet.
2. **Native var-ref access.** Add a versioned layout descriptor for
   `JSObject.u.func.var_refs` and `JSVarRef.pvalue` so loads and stores need
   no helper, and GET_VAR_REF becomes one guarded dependent load.
3. **Reload only captured slots.** Expose `vardefs[].is_captured` in the
   snapshot and reload only captured slots, instead of every argument and
   local.
4. `put_var_ref_check_init`, once `get_super` and derived constructors are
   admitted.
5. Make the generic call bridge (P1/P3) cheaper before expecting the ≥1.5x
   interpreter-speed P2 acceptance on the closure workloads.
