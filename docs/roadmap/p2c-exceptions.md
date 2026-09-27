# P2c: exception regions in Tier 1

Branch `perf/p2c-exceptions`, based on `82d3808`. Roadmap item P2.3 in
[the V8/Bun roadmap](../PERFORMANCE_ROADMAP_V8_BUN.md).

## What landed

- `throw`, `throw_error`, `catch`, `nip_catch`, `gosub` and `ret` are Tier 1
  `Native` opcodes (opcode manifest cases added).
- Verifier: a `catch` handler is entered with the caught value in place of its
  catch offset and with locals joined over every instruction of the try
  region (initialized/uninitialized disagreements widen to `Tagged`). `gosub`
  pushes an Int32 return offset and has only its finally block as successor;
  each `ret` gets the return points of the `gosub` sites of the finally block
  that reaches it (a `ret` owned by two finally entries is rejected). Dead code
  after a finally block's `ret` is tolerated only in functions with
  subroutines and is never lowered. The innermost live catch offset is
  recorded for every PC.
- Native dispatch: every helper exception edge inside a try region publishes
  the exact owned operand stack (as the ordinary exception exit does) and
  enters a per-handler landing pad. The pad calls `JS_JitCatchException`, which
  replays the interpreter's exception label (backtrace at the throwing PC,
  catchability) and, after proving the innermost catch offset is the expected
  slot/handler, releases the operands above it and stores the caught value.
  A caught exception resumes the handler natively; uncatchable errors,
  interrupts and invariant failures leave through the normal exception exit
  so the interpreter unwinds the published frame exactly.
- `JS_JitThrowValue` / `JS_JitThrowError` implement `throw` / `throw_error`.
- The interpreter builds a backtrace string for every caught primitive or
  non-Error throw only to store it in `ctx->error_back_trace`, which the catch
  releases unread. The native catch skips that build when it cannot run user
  code (`Error.prepareStackTrace` unset, numeric `Error.stackTraceLimit`) and
  cannot define `stack` on the caught value; a regression test checks both
  hooks still fire exactly as in the interpreter.
- The entry-domain analysis merges every exceptional edge (before, after and
  mid-store states) into handler inputs, so `get_loc_check` proofs stay sound.
- Tier 2, bounded inlining, tagged direct links and inline snapshot retention
  reject functions with exception regions up front; production maintenance no
  longer offers them to Tier 2.
- OSR validation now checks `CatchOffset` slots against
  `JS_TAG_CATCH_OFFSET` (it compared them with the Int32 tag), so loop headers
  inside try regions can be entered.
- Patch 0027 (ABI 1.25) also fixes the `exceptions-promises-async`
  regression: generator/async resumes pass their state as `func_obj`, the
  snapshot was refused without the backend ever learning why, and maintenance
  re-armed the request, so every call, resume and loop poll paid a `record_hot`
  round trip plus maintenance (2,030 snapshot requests per worker run; now 7).
  Refused generator/async/eval/`with` bytecode now disables its probes locally.
- New benchmark scenario `exceptions-sync` (pure JS, Bun-compatible): a caught
  primitive throw every 16th iteration, a finally block on every iteration and
  a helper-raised TypeError every 256th iteration.

## Noisy diagnostic (not publishable evidence)

Median over 7 alternating worker runs of `median(warmup_batch_ns[-16:])`,
automatic tiering, measured at `354a86c` on the shared 24-core host while ~16
agents were building and testing. Baseline is the `82d3808` `jit-bench`;
speed is baseline time / new time (above 1.00x is faster). Checksums matched
the interpreter in every run. The interpreter column is one same-host run.

| workload | 82d3808 ms | p2c ms | speed | interpreter ms |
| --- | ---: | ---: | ---: | ---: |
| exceptions-sync (new) | 2.9318 | 0.8765 | 3.35x | 2.9442 |
| exceptions-promises-async | 3.6864 | 2.4385 | 1.51x | 2.3155 |
| adversarial | 0.9528 | 0.9526 | 1.00x | 0.9347 |
| mixed-quotes (control) | 2.4885 | 2.5132 | 0.99x | 2.7690 |
| property-heavy (control) | 0.0981 | 0.0968 | 1.01x | 1.0673 |
| generic-call-entry (control) | 0.0323 | 0.0323 | 1.00x | 0.9689 |
| scalar-loop (control) | 0.0540 | 0.0300 | inconclusive | 0.5405 |
| call-heavy (control) | 0.0853 | 0.0598 | inconclusive | 1.2823 |

The scalar-loop and call-heavy baseline medians were inflated by host load:
earlier rounds measured 0.0291/0.0290 ms and 0.0595/0.0608 ms, and an A/A run
of the same `82d3808` binary against itself spread call-heavy between 0.95x
and 1.77x. Neither control executes code this item changes. An earlier
candidate checked every Tier 2 candidate snapshot for exception regions on
each maintenance pass and measured mixed-quotes at 0.98x in three rounds; the
check now runs once at verification and only for ready candidates (0.999x on
11 runs, 0.99x above).

`exceptions-sync` uses Tier 1 with native handlers (3.35x the previous JIT,
about 3.4x the interpreter). `exceptions-promises-async` is now at interpreter
speed: its `workload` and `asyncStep` are async and stay interpreted, and the
gain is the removed probe overhead (2,030 to 7 snapshot requests per worker
run). `adversarial` has no exception region; its Tier 1 code is rejected by
the profitability gate, which this item does not change. The publishable
three-engine matrix, including `exceptions-sync`, is left to the integrator.

## Verification

- `cargo test --release -p quickjs-jit-runtime --features compiler,test-support --tests`:
  797 passed, 0 failed, 1 ignored at `354a86c`.
- `cargo test -p quickjs-jit-sys --test jit_patch`: 13 passed.
- `cargo clippy -p quickjs-jit-runtime --all-targets --features compiler,test-support -- -D warnings`: clean.
- `mixed_direct_calls::tier2_mixed_edge_eliminates_helper_and_preserves_guard_misses`
  failed once in an earlier full run on this branch. It is timing-dependent and
  also fails on an unmodified `82d3808` build on this host (3 of 19 runs).

## Remaining work

- Tier 2 exception regions (landing pads with deopt-map ownership, handler
  blocks in the scalar graph). Tier 2 currently rejects these functions.
- `nip_catch` with operands between the kept value and its catch offset (only
  the direct form is lowered; others reject the compile).
- For-of/for-in iterator close offsets (`catch` offset 0) inside try regions:
  rejected until the iteration opcodes are admitted (P2.5).
- Helper-frame validation (P1 B1) is paid twice per native throw/catch.
- The interpreter could skip the same unread backtrace build; that is a
  runtime-library change outside this item.
