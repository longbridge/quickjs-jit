# P4d: scalar loop code quality (first slice)

Branch `perf/p4d-scalar-codegen`, based on `82d3808`. Roadmap item: P4,
"降低 poll 和溢出检查的密度" plus frame syncs, tag checks and Float64/Int32
representation churn in scalar kernels
([roadmap](../PERFORMANCE_ROADMAP_V8_BUN.md)).

This slice changes only Tier 2 (optimizing) code generation. No QuickJS C
patch, ABI version, bytecode fingerprint or README matrix changes.

## How the code was inspected

Benchmark binaries are built with `test-support`, which already enables
Cranelift disassembly. A new test-support-only diagnostic writes every
finalized function's CLIF and machine code:

```sh
QJSJIT_DUMP_CODE=/some/dir ./target/release/jit-bench worker --mode automatic \
    --script benchmarks/scripts/scalar-loop.js
```

Files are named `<pid>-<seq>-tier1|tier1-osr|tier2.txt`. Tier 2 files contain
the CLIF after the post-lowering cleanups below, followed by the VCode
disassembly. Without the variable the hook does nothing, and it does not exist
in builds without `test-support`.

## Findings on `82d3808`

`for (let i = z; i < n; i++) sum = sum + i` ran a Tier 2 loop of about 27
instructions per iteration, including 2 loads and 4 stores:

- the amortized poll countdown lived in a stack slot (load, decrement, store
  every iteration, plus a register holding the slot address);
- every Int32 tag was a fresh `iconst` per block, so SSA construction turned
  loop-invariant tags into block parameters (four constant-zero phis in this
  loop). Cranelift's constant-phi removal compares value identity and runs
  before GVN, so they survived into register allocation;
- `arg_buf`, `var_buf` and `stack_base` loaded in the prologue were kept live
  across the loop only for the deopt/exit blocks;
- the `sret` exit-record pointer was kept live (and spilled/reloaded inside
  the loop) because Cranelift's `StructReturn` parameter must be returned in
  `rax` at every `return`;
- guards the specializer had already proven compiled to `movl $1; test; jnz`
  because Cranelift does not fold `brif` on constants;
- `x >>> 3` always produced a Float64 alternative plus a select, and Float64
  arithmetic round-tripped every intermediate through a GPR (`bitcast` pairs);
- functions with any call (for example `quickjs-int-arith`: `Math.max` before
  the loops and `String` after them) failed the whole-function amortized-poll
  proof, so every iteration of their pure inner loops called the poll helper
  through the full helper-frame validation.

## Changes

All rewrites are semantics-preserving CLIF transforms or narrower proofs;
failures fall back to the previous code shape.

1. **Poll countdown in a register.** The amortized poll budget is a Cranelift
   `Variable` instead of a stack slot (`emit_opt_amortized_poll`).
2. **Per-loop poll amortization.** When whole-function amortization is
   rejected, each natural loop whose own nodes pass the same
   `permits_amortized_poll_with_guarded_heap` proof (no reentrant or unknown
   calls, no unguarded heap operations, primitive frame writes) counts down its
   header poll (`emit_opt_countdown_poll`). Unlike whole-function mode, frame
   locals keep their immediate stores, and the header numeric guard,
   hoisted-call revalidation and packed-array revalidation still run on every
   iteration: only the poll call frequency changes, exactly as it already does
   for raw Int32 loops. The poll block keeps its layout position (moving call
   sites changes which call return publishes the helper stack map).
3. **Canonical integer constants** (`canonicalize_integer_constants`): every
   `iconst` is replaced by one definition per (type, value) at the top of the
   entry block, so constant phis collapse.
4. **Constant-branch folding** (`fold_constant_branches`): after constant-phi
   removal, a `brif` whose condition evaluates to a constant (constants, width
   conversions, bitwise ops, integer compares) becomes a `jump`, and the dead
   successors are removed. If folding removes every call, the artifact no
   longer publishes a helper stack map.
5. **Bitcast round trips** (`fold_bitcast_round_trips`): `bitcast.T(bitcast.U
   x)` with `x: T` forwards `x` (scalar types only).
6. **Exit rematerialization** (`rematerialize_invariants_in_exits`, functions
   with loops only): blocks ending in `return` reload the root frame's buffer
   pointers from the frame, and `sret` from a prologue stash slot. On x86-64
   System V the exit record pointer is passed as an ordinary pointer argument
   and each exit returns the reloaded pointer explicitly; the machine-level
   contract of `JSJitExit (*)(JSJitExecFrame *)` (hidden pointer in `rdi`,
   returned in `rax`) is unchanged. Other targets keep `StructReturn`.
7. **Constant unsigned shifts**: `x >>> c` with `c & 31 != 0` is always below
   2^31 and lowers directly to an Int32 (no Float64 alternative).

`RelocatableCode::clif()` still returns the lowering output (before items 3-6)
so existing CLIF-structure tests keep describing the lowering; the dump shows
the final CLIF.

After the change the same scalar loop is 14 instructions per iteration with no
memory operations:

```text
lea -1(%rcx),%ecx; test; jz poll      ; countdown
cmp %r15d,%ebx; jl body               ; i < n
mov %r12,%r8; add %ebx,%r8d; seto; test; jnz deopt   ; checked sum + i
lea 1(%rbx),%ebx; mov %r8,%r12; jmp header           ; i++ (range-proven)
```

## Noisy diagnostic measurements

Not publishable evidence: the i7-13700KF was shared with about 16 concurrent
agents, there is no Bun column, and runs were not pinned. Each run alternates
the `82d3808` baseline binary and this branch
(`jit-bench worker --mode automatic --script benchmarks/scripts/W.js`); a
run's value is the median of its last 16 warmup batches (ms per 10 workload
calls), and the table reports the median over runs. Hybrid P/E cores make runs
bimodal, so the "P-core mode" columns repeat the medians over runs within 1.4x
of each engine's fastest run. Speed is baseline time / new time (above 1.00x is
faster). Checksums of both binaries matched the interpreter in every run. Raw
per-run values:
[`p4d-scalar-codegen-noisy-diagnostic-x86_64.json`](../../benchmarks/results/p4d-scalar-codegen-noisy-diagnostic-x86_64.json).

| Workload | Role | Base ms | New ms | Speed | Base ms (P-core mode) | New ms (P-core mode) | Speed (P-core mode) | Runs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| numeric | target | 0.0323 | 0.0215 | 1.50x | 0.0295 | 0.0214 | 1.38x | 11 |
| scalar-loop | target | 0.0294 | 0.0214 | 1.37x | 0.0293 | 0.0214 | 1.37x | 11 |
| scalar-control-flow | target | 0.0546 | 0.0439 | 1.24x | 0.0546 | 0.0439 | 1.24x | 11 |
| scalar-expressions | target | 0.0505 | 0.0456 | 1.11x | 0.0505 | 0.0456 | 1.11x | 11 |
| quickjs-bitops | target | 0.0637 | 0.0598 | 1.06x | 0.0625 | 0.0317 | 1.97x | 11 |
| quickjs-int-arith | target | 1.3378 | 0.3289 | 4.07x | 1.3378 | 0.3289 | 4.07x | 11 |
| fibonacci-iterative | target | 0.9952 | 0.7446 | 1.34x | 0.9952 | 0.7446 | 1.34x | 11 |
| host-compute | target | 0.0713 | 0.0514 | 1.39x | 0.0711 | 0.0514 | 1.38x | 11 |
| float64-dense | target | 0.1291 | 0.1011 | 1.28x | 0.1291 | 0.1011 | 1.28x | 11 |
| property-heavy | control | 0.0989 | 0.0965 | 1.03x | 0.0988 | 0.0964 | 1.03x | 9 |
| int32array-traversal | control | 0.0495 | 0.0375 | 1.32x | 0.0495 | 0.0374 | 1.32x | 9 |
| packed-array-traversal | control | 0.0483 | 0.0401 | 1.20x | 0.0481 | 0.0399 | 1.21x | 9 |
| call-heavy | control | 0.0598 | 0.0416 | 1.44x | 0.0597 | 0.0416 | 1.43x | 9 |
| float64array-traversal | control | 1.0961 | 1.0633 | 1.03x | 1.0961 | 1.0574 | 1.04x | 9 |
| arrays-typed | control | 2.9775 | 2.9485 | 1.01x | 2.9775 | 2.9485 | 1.01x | 9 |
| generic-call-entry | control | 0.0326 | 0.0613 | 0.53x | 0.0323 | 0.0301 | 1.07x | 9 |
| generic-call-fallback | control | 9.2188 | 9.1138 | 1.01x | 9.2188 | 9.1138 | 1.01x | 9 |
| objects-polymorphic | control | 24.1729 | 23.7835 | 1.02x | 24.1729 | 23.7835 | 1.02x | 9 |
| mixed-quotes | control | 2.5350 | 2.6146 | 0.97x | 2.5317 | 2.6146 | 0.97x | 9 |
| fibonacci-recursive | control | 9.8571 | 9.8478 | 1.00x | 9.8571 | 9.8478 | 1.00x | 9 |
| calls-closures | control | 3.3353 | 3.3997 | 0.98x | 3.3272 | 3.3809 | 0.98x | 9 |

Notes:

- `quickjs-bitops` and `generic-call-entry` medians are dominated by core
  placement. Pinned to E-core 20, `quickjs-bitops` measured about 0.101 ms
  (base) and 0.061 ms (new), 1.66x; on an uncontended P-core, 0.062 ms and
  0.032 ms. In `generic-call-entry`, this branch was faster within each mode
  (about 0.030 ms vs 0.032 ms, and 0.062 ms vs 0.071 ms). More of its runs
  landed on slow cores, which pulls the overall median to 0.53x.
- A 13-run rerun of `mixed-quotes` (2.537 ms vs 2.547 ms, 1.00x) and
  `calls-closures` (3.359 ms vs 3.246 ms, 1.03x) found both statistically
  tied. These controls do not execute rewritten loops.

## Tests

- `jit/tests/scalar_codegen.rs` (new): per-loop countdown appears in a
  function with calls outside the loop and not in a loop containing a call;
  exact results across trip counts, uncatchable interrupt service in an
  effectively unbounded amortized loop nest, exact overflow deopt of a scalar
  loop (rematerialized exit), and constant `>>>` in a Tier 2 bit-mixing loop.
- `compiler::baseline::post_lowering_tests`: constant sharing, constant-branch
  folding (including narrow unsigned/signed compares and runtime conditions
  left alone), and bitcast forwarding, each verified with the Cranelift
  verifier.
- `m2_core_opcodes::shifts_mask_the_count_and_unsigned_shift_renormalizes`
  now also covers `-1 >>> 32` (a constant count masking to zero keeps its
  Float64 result).
- Updated evidence tests: the property-loop countdown test now requires an
  SSA countdown (block parameter, no stack slot); the bitops CLIF test
  requires no `fcvt_from_uint` for constant counts and keeps it for a
  zero-masked count; the float side-path test counts the side-path-hit flag
  write instead of raw `brif` counts (proven guards now fold in both
  versions).

## Remaining work

- Fold constant branches that only become constant after Cranelift's egraph
  (for example in `float64-dense`); this needs a second optimize pass or a
  custom GVN before folding.
- Overflow checks still lower to `seto; test; jnz` because Cranelift does not
  fuse `sadd_overflow` with `brif`. Accumulators such as `sum + i` need the
  check; range facts already drop it for bounded induction variables.
- Values needed only by deopt state (for example the per-iteration `next`
  local and its tag in `fibonacci-iterative`) are spilled every iteration.
  Recomputing them at the exit from other live values would remove the stores.
- The root frame pointer still moves between an argument register and a
  callee-saved register around the cold poll call; a preserve-all poll
  trampoline would avoid that but is not available in Cranelift 0.116.
- Extend per-loop amortization to deferred local stores (currently only the
  poll call is amortized per loop), with a flush at loop exit.
- Publishable three-engine measurements (the numbers below are noisy
  diagnostics on a loaded machine).
