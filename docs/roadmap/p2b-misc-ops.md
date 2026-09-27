# P2b: common rejected opcodes (first slice)

Branch `perf/p2b-misc-ops`, based on `82d3808`. Roadmap item: P2 coverage for
`push_this`, `special_object`, global variable access, the `typeof` family,
`in`, `instanceof`, `pow`, `delete`, `to_object` and the stack shuffles. See
[the roadmap](../PERFORMANCE_ROADMAP_V8_BUN.md), sections 4 (B3) and 6 (P2).

## What Tier 1 now admits

| Opcode(s) | Lowering |
| --- | --- |
| `push_this`, `special_object` (arguments, this function, `new.target`, home object, variable object, null prototype) | `GENERIC_OP` helper, reading the root entry's call context |
| `get_var_undef`, `delete_var`, `put_var` | `GENERIC_OP` helper with a validated atom |
| `typeof` | `GENERIC_OP` helper |
| `typeof_is_undefined`, `typeof_is_function` | native answer for every non-negative tag (not an object, not reference counted); `GENERIC_OP` otherwise |
| `to_object` | `GENERIC_OP` helper (object destructuring) |
| `to_propkey2` | native no-op when the receiver is object-coercible and the key is Int32/string/symbol; `GENERIC_OP` otherwise (compound element updates, computed destructuring) |
| `in`, `instanceof`, `delete` | `GENERIC_OP` helper; the boolean result feeds branches without `ToBool` |
| `pow` | existing exact `BINARY_ARITH_SLOW` helper, which already accepted `OP_pow` |
| `dup1`, `dup2` | existing `DUP` lowering |
| `perm4`, `swap2`, `rot3l`, `rot3r` | existing native stack permutations |

`GENERIC_OP` is one append-only helper: ID 24, ABI 1.25, runtime API 1.10,
patch `0026-tier1-generic-ops.patch`. Each opcode runs the same C routine the
interpreter uses, under the same slot and ownership contract as the other
exact slow-path helpers. Operands stay in C-visible frame slots. Consumed slots
are cleared on success and on a language exception, and the exception depth is
the pre-instruction depth. A no_inline wrapper around each root native entry
publishes the receiver, `new.target` and the caller's `argc`/`argv`.
`push_this` and `special_object` reject any other frame, such as an inline
view.

Tier 2 is unchanged. Its translator has no classification for these opcodes,
so functions that contain them stay at Tier 1; the stack shuffles were already
classified. The frame-inline and effect-free inliners still resume at, or
reject, these instructions.

## Still rejected, and why

| Opcode(s) | Reason |
| --- | --- |
| `special_object` MAPPED_ARGUMENTS (`ExtendedFrame`) | Sloppy `arguments` aliases argument slots through variable references, but Tier 1 keeps arguments in registers. This needs a frame-aliasing model, shared with the closure work. |
| `special_object` IMPORT_META | Needs module-only state. |
| `set_name`, `set_name_computed` | They only name a fresh closure or class, so they cannot be reached until `fclosure` and `define_class` are admitted. Adding them to `GENERIC_OP` takes a few lines: `JS_DefineObjectName[Computed]`, with the value kept in place. |
| `put_var_init`, `define_var`, `check_define_var`, `define_func` | Only occur in script or eval code, which is never compiled as a Tier 1 function. The differential manifest cannot execute them natively. |
| `nop`, `nip1` | Never emitted after label resolution. |
| `dup3`, `insert4`, `perm5`, `rot4l`, `rot5l` | Only follow `super` references or async iteration. |

Each admitted opcode has an `opcode-cases.json` entry. The entry covers the
interpreter differential, execution at the native PC, the exception path and
stress GC. The focused tests in `jit/tests/tier1_generic_ops.rs` add more
cases.

## Runtime interaction: untranslatable Tier 1 functions

Admitting these opcodes turns many small methods and host-style helpers into
Tier 1 functions that Tier 2 can never translate. At `82d3808` such a function
followed one of two paths:

* It stayed in Tier 1 forever when a property, direct-call or numeric gate
  deferred its Tier 2 trial, and it rebuilt a full feedback snapshot at every
  maintenance. `objects-polymorphic` is on this path; `object` has no Tier 2
  classification.
* Otherwise it queued Tier 2 attempts that could only fail. When the
  optimizing tier was blacklisted, `record_hot` disabled the generation's
  probes and the function returned to the interpreter. `json-codec` is on this
  path.

With the new opcodes, the new `methods-dynamic` kernel took the first path and
ran 5.4x slower than `82d3808`, where its functions never entered native code.
Most samples were in maintenance feedback snapshots, followed by helper-PC
validation (B1).

`ProductionBackend::maintenance` now settles such a candidate once it has
served 16 baseline executions. Before that point, deferring it costs one map
lookup. It keeps the call-only interpreter demotion and that demotion's
metric. Otherwise it moves directly to the terminal state that the failed
bounded trials used to reach: baseline demoted, probes off. Forced Tier 2 test
runs keep the old path. As a result, steady-state automatic tiering runs these
functions in the interpreter, exactly as at `82d3808` or better. The new
coverage matters for warmup and OSR, for forced Tier 1, and for later Tier 2
work. A real gain from Tier 1 alone depends on P1: O(1) helper-PC validation
(B1) and cheaper per-call bookkeeping (B2), followed by a baseline-versus-
interpreter measurement.

## Noisy diagnostics (not publishable evidence)

The host is shared by 16 agents on 24 cores. Each value is the median over
alternating runs of `median(protocol.warmup_batch_ns[-16:])` from
`jit-bench worker --mode automatic`. The baseline binary was built at
`82d3808`; the branch binary was built at `a43c209`. Speed is baseline time
divided by branch time, so values above 1.00x are faster. Every run's checksum
matched the interpreter. For `methods-dynamic`, Bun's checksum also matched
QuickJS.

| Workload | Runs | 82d3808 ms | branch ms | Speed |
| --- | ---: | ---: | ---: | ---: |
| objects-polymorphic | 7 | 24.222 | 6.697 | 3.62x (now at interpreter speed, 6.18 ms) |
| methods-dynamic (new) | 7 | 13.006 | 13.215 | 0.98x |
| adversarial | 7 | 0.954 | 0.949 | 1.01x |
| strings-json | 7 | 2.002 | 1.998 | 1.00x |
| strings-regexp | 7 | 19.888 | 20.158 | 0.99x |
| json-codec | 7 | 78.838 | 78.893 | 1.00x |
| mixed-quotes | 7 | 2.487 | 2.525 | 0.98x |
| generic-call-fallback | 7 | 9.196 | 9.033 | 1.02x |
| map-set-bigint | 7 | 17.228 | 17.703 | 0.97x |
| property-heavy | 7 | 0.0988 | 0.0979 | 1.01x |
| call-heavy | 7 | 0.0600 | 0.0599 | 1.00x |

`map-set-bigint` never enters native code. In pure interpreter mode, with no
JIT attached, it measured 0.98x across 9 runs, while `calls-closures`,
`fibonacci-recursive`, `collections` and `objects-polymorphic` measured
0.99x to 1.00x in the same mode. This looks like C code layout rather than a
JIT effect, and the published matrix should confirm it. A 7-run pass over the
other 20 matrix scenarios, using the binary built just before the
out-of-line wrapper, measured between 0.99x and 1.02x. `numeric` was bimodal
in both binaries (0.029 ms or 0.055 ms per batch), as it was at `82d3808`.
Bun runs `methods-dynamic` in 0.150 ms, against 12.85 ms for the QuickJS
interpreter.

Progression of `methods-dynamic` during development:

| Variant | Speed vs 82d3808 |
| --- | ---: |
| Opcodes admitted, maintenance unchanged | 0.18x |
| Plus settling with the measured profitability trial kept | 0.26x to 0.40x (bimodal); `json-codec` fell to 0.84x |
| Final (terminal settle after 16 baseline executions) | 0.98x |

## Remaining work

1. Land P1 (O(1) helper-PC validation and C-side tier state), then replace the
   terminal settle with a measured baseline-versus-interpreter decision, so
   profitable Tier 1-only code can stay native.
2. Mapped `arguments`, once frame slots can be aliased (closure frames).
3. `set_name` and `set_name_computed`, together with `fclosure` and
   `define_class`.
4. Script-level compilation, or OSR in global code, for the global
   declaration family.
5. Tier 2 classifications for `typeof_is_*`, `in`, `instanceof`,
   `to_propkey2`, `to_object` and `push_this`, so hot loops with these checks
   can reach the optimizer.
