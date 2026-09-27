# P4e: object allocation and `Array.prototype.push` fast paths (first slice)

Branch `perf/p4e-alloc-fastpath`, based on `82d3808`. Roadmap item: P4
"对象分配快路径" in [PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md).

## What landed

- `sys/patches/0032-object-fast-paths.patch` (ABI 1.25): a versioned
  `JSJitObjectAPI` table of leaf functions. Every leaf either completes the
  interpreter's exact effect or misses with no observable effect; none throws,
  calls user code or reenters the VM, so generated code always keeps the
  generic helper sequence as its fallback.
  - `literal(ctx, atoms, values, count, out)`: builds `{a: v0, ...}`
    (OP_object + one OP_define_field per atom, `JS_PROP_C_W_E`). The final
    hashed shape is a pure function of `(Object.prototype, atom list)`, so it
    is looked up directly in the runtime shape hash (no intermediate
    transition shapes, which QuickJS mutates in place and does not retain).
    Allocation uses the non-throwing `js_malloc_rt`; a miss (no shape, duplicate
    atom, uninitialized value, OOM) has no effect. It keeps the
    `JS_NewObjectFromShape` GC trigger point, so GC accounting is unchanged.
  - `retain_shape(ctx, object)`: after a literal is built generically, keeps
    its final shape in the bounded, GC-traced per-context JIT shape cache from
    0012, so literals whose receivers are later extended (`o = {x, kind};
    o.y = ...`) still find their shape next time.
  - `array_method(ctx, receiver, atom, out)`: exact `get_field2` lookup on a
    fast Array when it resolves to a plain data property of the realm's
    `Array.prototype` (no own property, no exotic handler, getter or autoinit).
  - `array_push(ctx, fn, receiver, value, out)` and
    `array_push_method(ctx, receiver, atom, value, out)`: a one-argument call
    of the built-in `Array.prototype.push` (identity: C function,
    `js_array_push`, magic 0, same runtime realm), mirroring
    `js_call_c_function`'s stack-overflow check and `js_array_push`'s
    fast-array path; element-buffer growth uses `expand_fast_array`'s policy
    with non-throwing `js_realloc_rt`.
  - Upstream fix: `js_array_push`'s fast path appended at `length` even when
    a fast array kept `count < length` (after `arr.length = n` grew it),
    leaving uninitialized element slots. The unmodified 82d3808 interpreter
    crashes (`SIGSEGV`) on `a=[1,2,3]; a.length=6; a.push(7,8)` in a loop
    (`jit-bench worker --mode interpreter` on such a script dumps core). The
    fast path now requires `length == count`; otherwise the generic path runs.
- Tier 1 lowering (`jit/src/compiler/baseline.rs`):
  - Complete object literals whose values are side-effect-free reads
    (constants, arguments, locals) are fused into one `literal` leaf call; the
    unchanged `object`/`define_field` sequence is the fallback and retains the
    resulting shape. Prefixes of longer literals are not fused (their shape is
    never alive), checked by a bounded stack-effect scan.
  - `x.push(v)` statements with side-effect-free receiver and argument reads
    are fused into one `array_push_method` leaf call: no DUP/FREE,
    GET_PROPERTY or CALL helpers on a hit.
  - Other `get_field2` sites on fast Arrays try `array_method` before the
    GET_PROPERTY helper, and other one-argument method calls on Arrays try
    `array_push` before the CALL helper (all inputs stay borrowed, so the
    interpreter-order cleanup is shared by both paths).

Tests: `jit/tests/object_fast_paths.rs` (interpreter differential checks,
stress GC, every push miss shape: frozen/sealed/read-only length, own `push`,
subclass, holey and grown-length arrays, array-likes, patched or accessor
`Array.prototype.push`, `unshift` aliasing, TDZ bindings, non-array receivers;
helper-count proofs that hits skip NEW_OBJECT/SET_PROPERTY/GET_PROPERTY/CALL),
plus ABI table validation tests in `jit/src/abi.rs`.

## Noisy diagnostic (not publishable evidence)

Shared, loaded host (i7-13700KF, load average about 11–40 from 16
concurrent agents, no CPU pinning). `jit-bench worker --mode automatic`,
alternating the 82d3808 binary (baseline) and this branch. Each value is the
median over processes of `median(protocol.warmup_batch_ns[-16:])`, in ms per
10 workload calls. Speed = baseline / new (above 1.00x is faster). Checksums
are identical across both binaries and the interpreter. There are no
confidence intervals; the three-engine README matrix still has to be
measured by the integrator.

| Workload | Runs | Baseline ms | New ms | Speed |
| --- | ---: | ---: | ---: | ---: |
| objects-polymorphic | 9 | 24.151 | 19.951 | 1.21x |
| mixed-quotes | 9 | 2.520 | 2.501 | 1.01x (tied, noise) |
| collections | 9 | 1.651 | 1.645 | 1.00x (tied, noise; not native) |
| property-heavy (control) | 5 | 0.1030 | 0.0996 | 1.03x (bimodal samples) |
| call-heavy (control) | 5 | 0.1136 | 0.0604 | inconclusive: samples are bimodal (0.06 and 0.115 ms in both binaries) |
| generic-call-fallback (control) | 5 | 9.123 | 9.089 | 1.00x |
| arrays-typed (control) | 5 | 2.991 | 3.032 | 0.99x |
| calls-closures (control) | 5 | 3.379 | 3.255 | 1.04x (bimodal samples) |

Remaining helper calls per `objects-polymorphic` iteration on this branch
(Tier 1, `JS_JitGetHelperCount`): NEW_OBJECT 0, SET_PROPERTY 2.0 (`o.y =`,
`o.x +=` and the `Object.create(null)` case), GET_PROPERTY 5.25, GET_ELEMENT
2, DUP 7, FREE 8.5, CALL 0.25 (`Object.create`). Every literal and every
`objects.push(object)` is handled by a leaf.

## Remaining work

1. **Pre-existing bug, not fixed here:** Tier 1 lowers `define_field` (and
   `define_array_el`) with *set* semantics (`JS_JIT_HELPER_SET_PROPERTY` /
   `SET_ELEMENT`). An inherited setter on `Object.prototype` runs and no own
   property is created for literals that are not fused (for example
   `{x: i, y: f()}`). Reproduced on this branch's generic path, which is
   unchanged from 82d3808; recorded as the ignored test
   `generic_define_field_ignores_inherited_setters`. The fix needs an exact
   define helper (new helper-table entry, ABI bump) or a non-throwing define
   leaf with a deopt fallback.
2. **Tier 2:** `object`/`define_field` are not admitted by the optimized
   tier, so functions building literals stay in Tier 1. Admitting them needs
   an allocation node in the semantic SSA (fresh object: no aliasing with
   existing heap locations), ownership/materialization for deopt, and then
   scalar replacement/allocation sinking.
3. **Non-simple push arguments** (for example `quotes.push({index, score})` in
   `mixed-quotes`) still use the separate `array_method` + `array_push`
   leaves plus DUP/FREE helpers; fusing a literal value into the push leaf
   needs an owned-temporary release path.
4. **Empty literals without live empty objects** (`const e = {}` that is
   dropped) miss because no hashed empty shape survives; creating the shape
   non-throwingly in the leaf would remove the NEW_OBJECT helper there too.
5. `collections` is unaffected: its workload is not natively compiled
   (`for_of` is rejected by the Tier 1 policy; roadmap P2).
6. The remaining cost in `objects-polymorphic` is DUP/FREE helpers (≈7 and
   ≈8.5 per iteration) and polymorphic property reads (≈5 GET_PROPERTY per
   iteration); those are roadmap P1 (inline Dup/Free, O(1) helper checks) and
   P4 polymorphic property ICs.
