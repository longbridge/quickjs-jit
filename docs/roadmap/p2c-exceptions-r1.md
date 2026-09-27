# P2c exception regions: review fixes (r1)

Branch `perf/p2c-exceptions-r1`, based on `perf/p2c-exceptions` (`d5b40f4`).
This note follows [p2c-exceptions.md](p2c-exceptions.md).

## Fixed

- **Blocking: handlers with no exceptional edge failed Tier 1 as
  `InvalidArtifact`.** QuickJS wraps every catch body in an inner
  `catch Lx ... Lx: throw` region. When nothing in that region can raise
  (for example `catch (e) { return -1; }`), its landing pad and handler are
  unreachable CLIF. Cranelift removes them together with their `Poll` and
  `Marker` safepoints, and the frame-state publisher rejected the missing
  states. The compiler now walks the CLIF control flow graph from the entry
  block before code generation. A frame state is dropped only when its
  source location occurs in no reachable block. This is the same graph
  Cranelift's unreachable-code elimination uses, so the rule accepts only
  states that no emitted code can reference. A missing state in a reachable
  block is still `InvalidArtifact`.
- **Test harness stack-map bound.** The forced-baseline harness advertised
  `stack_maps().len()` as the stack-map count, while production uses
  `required_stack_map_count()` (maximum source location + 1). Once states
  are dropped, the harness rejected valid ids as an uncatchable
  "invalid JIT helper stack map". The harness now uses the production bound.
  That unblocks forced-baseline coverage of rethrow from a handler with no
  enclosing region, and of recursive catch-and-rethrow.
- **Stress-GC stale local alias on a FREE edge.** `JS_JitHelperFree` consumes
  its slot before its stress collection can fail. For a local or argument
  store inside a try region, that edge now exits to the interpreter instead
  of entering the native landing pad. The interpreter unwinds from the frame,
  whose slot already holds `undefined`. Before this change, the native handler
  resumed with the SSA variable still naming the released value. Operand-slot
  FREEs keep native dispatch because the landing pad resets every operand
  above the catch offset.
- **Stress-GC catch after mutation.** In patch 0027, `JS_JitCatchException`
  now runs its stress collection before it releases operands or overwrites
  the catch offset. A failure now leaves the frame untouched, and the
  interpreter's own exception label handles it. Before, the frame had already
  been rewritten, so unwinding skipped the handler. The patch digest and the
  patched `quickjs.c` fingerprint are updated. There is no ABI or header
  change.

## New regression tests (`jit/tests/tier1_exceptions.rs`)

- `handlers_without_helper_edges_still_compile_natively`: failed with
  "forced baseline compilation failed: InvalidArtifact" before the fix.
- `production_tiering_installs_common_try_catch_shapes`: Tier 1 only. The
  reviewer's recursive `f` plus `o.x.y` shapes must install (`installed >= 3`)
  with `invalid_artifacts == 0`. It failed before the fix.
- `throws_from_handlers_without_an_enclosing_region_leave_exactly`: failed on
  the harness stack-map bound before that fix.
- `refcounted_local_stores_inside_try_regions_keep_exact_ownership`: FREE of
  heap locals inside a region, with and without stress GC. This test does not
  force an allocation failure, and no deterministic OOM-injection hook exists.

## Remaining

- Under default tiering, a caller with no try region that calls
  try/catch functions from a loop (the `run` driver in the production test)
  is offered to Tier 2. Tier 2 then refuses it at admission ("no stable
  feedback"), which is counted as `InvalidArtifact` (4 attempts with
  backoff). The refusal is in Tier 2 candidate selection, not in exception
  lowering. It is fail-closed, and the caller keeps its Tier 1 code. A clean
  fix would stop offering such callers, or count the refusal as a rejection
  rather than an invalid artifact.
- Skipping the discarded backtrace can avoid an OOM that the interpreter
  could raise inside `build_backtrace`. This only matters under allocation
  failure.
- Coverage gaps carried over from r0: indirect `nip_catch` forms, labeled
  break across two finally blocks, some `let`-in-try shapes (all fail closed
  with `UnsupportedOpcode`), and Tier 2 for functions with exception regions.
- The publishable three-engine README matrix is still pending, for the
  integrator.

## Noisy diagnostic (not publishable evidence)

These numbers come from the shared, loaded 24-core host. Each value is the
median over 7 alternating worker runs of `median(warmup_batch_ns[-16:])`,
automatic tiering. Speed is `82d3808` time / r1 time, and above 1.00x is
faster. Checksums matched interpreter mode in every run.

| workload | 82d3808 ms | r1 ms | speed | interpreter ms |
| --- | ---: | ---: | ---: | ---: |
| exceptions-sync | 2.9201 | 0.8108 | 3.60x | 2.9582 |
| exceptions-promises-async | 3.6806 | 2.4597 | 1.50x | 2.4182 |
| adversarial | 0.9522 | 0.9512 | 1.00x | 0.9371 |

In this run, exceptions-promises-async measured about 2% slower than the
interpreter (0.98x the interpreter speed). There is no confidence interval
for these numbers, and the gap is within host noise. Adversarial is
unchanged.
