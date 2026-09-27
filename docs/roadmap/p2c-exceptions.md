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
automatic tiering, on the shared 24-core host while ~16 agents were building
and testing. Baseline is the `82d3808` `jit-bench`; speed is baseline time /
new time. Checksums matched the interpreter in every run.

| workload | 82d3808 ms | p2c ms | speed |
| --- | ---: | ---: | ---: |
| exceptions-sync (new) | 2.9537 | 1.6643 | 1.78x |
| exceptions-promises-async | 3.7634 | 2.4500 | 1.54x |
| adversarial | 0.9545 | 0.9501 | 1.00x (tied) |
| scalar-loop (control) | 0.0293 | 0.0290 | 1.01x (tied) |
| call-heavy (control) | 0.0599 | 0.0605 | 0.99x (tied) |
| mixed-quotes (control) | 2.5143 | 2.5564 | 0.98x (tied) |

`exceptions-promises-async` is now at interpreter speed (its `workload` and
`asyncStep` are async and stay interpreted; the gain is the removed probe
overhead). `adversarial` has no exception region; its Tier 1 code is rejected
by the profitability gate, which this item does not change. The publishable
three-engine matrix, including `exceptions-sync`, is left to the integrator.

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
