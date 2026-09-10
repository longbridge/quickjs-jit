# Semantic SSA performance work

Status: active, macOS-first implementation and acceptance per the user’s
2026-09-10 clarification. Historical native Linux verification is deferred. Baseline runtime: `023a2206d20d3cc8aa1646f3ab7fa0ad34fb220d`
(merged #26). The previous lint fix was committed and pushed before this
objective resumed; this work starts from the subsequently observed clean main.

## Acceptance scope

Implement feedback-driven CFG semantic SSA, known facts, effects/alias analysis,
speculative inlining, LICM, ranges/bounds elimination, lazy frame reconstruction,
inline frame chains and SSA ownership. Retain Cranelift as the machine backend
and QuickJS's public tagged value representation.

The first performance gate is at least 0.5x default Bun speed for
`generic-call-entry`, `property-heavy`, and `arrays-typed`, followed by 0.8x and
parity as later targets. Generic fallback must first reach same-version QuickJS
speed. An isolated fast-path or passing semantic test is not completion.

Required hot-loop evidence:

- Calls: integer Phi/add/branch, with target guard outside the loop; no generic
  call, helper, frame publication, argv array or per-iteration boxing.
- Properties: shape guard outside the loop, x/y in SSA, stores at exits; no
  property memory access inside the loop. Materialize dirty stores before
  observable poll/interrupt/exception/deopt/reentrant boundaries.
- Typed arrays: class/buffer/data/length guards outside the loop, raw memory
  accesses and induction; no helper, shape or dynamic bounds check, or boxing.

Preserve exact JS semantics, GC/rooting, reference counts, exception order,
invalidation and interpreter fallback. Inline deopt must reconstruct the full
inline frame chain before expanding beyond safe leaf inlining.

Every optimization needs paired previous-runtime controls, same-version
QuickJS, default Bun, production automatic tiering, complete README matrix
including losing/unsupported/inconclusive rows, confidence intervals and raw
provenance. Forced tiers and host snapshots remain separate diagnostics.

## Execution sequence

| Phase | Deliverable and gate | Current state |
| --- | --- | --- |
| A | Isolate and fix host/interpreter regression; freeze call/fallback/property/array/scalar controls | macOS paired baseline archived; historical Linux regression deferred |
| B | Explicit ValueId/operands/Phi/semantic nodes/FrameState/CFG; migrate numeric lowering without regression | In progress: CFG Phi edges drive backend value binding; numeric operands and recovery bindings use SSA IDs. Feedback-specialized semantic nodes, full lazy deopt and current-candidate performance remain pending |
| C | Flow-sensitive type/shape/range/array/call facts and effect/clobber model; prove duplicate checks eliminated | Not implemented |
| D | Bounded monomorphic graph-builder inlining; call loop structural gate and 0.5x Bun | Not implemented |
| E | Stable call-site IC feedback slots and compact generic trampoline; fallback at least QuickJS speed | Not implemented |
| F | Property shape-check deduplication and effect-aware LICM; first exceed 10x QuickJS | Not implemented |
| G | Abstract heap load/store forwarding with alias invalidation | Not implemented |
| H | Store sinking plus dirty-heap materialization; property 0.5x Bun | Not implemented |
| I | Array modes and class/length/data/detach LICM | Not implemented |
| J | Induction/range proofs and bounds elimination; arrays 0.5x Bun | Not implemented |
| K | Profile first, then packed push and selected construction/builtin bridges if dominant | Not started |

Compiler counters must cover generic/direct/inlined calls, call/shape/array
guards, dynamic/eliminated bounds checks, frame materializations/deopts,
helper calls/exits, box/unbox, property loads/stores, forwarded loads/sunk stores,
and refcount dup/free. These counters and generated-code inspection are gates,
not substitutes for end-to-end timing.

## Phase A evidence, 2026-09-10

The [manifest](../benchmarks/results/semantic-ssa-baseline-manifest.json) freezes
the candidate4 archive and binary hashes and verifies all seven archived
focused workload hashes against current scripts. Historical baseline data stays
historical; no new native Linux measurement or causal attribution is claimed.

The final candidate4 host evidence still shows compute automatic reload at
0.9444x old-runtime speed and interpreter steady state at 0.9329x. Reload render
is the implicated component. Candidate3's prior improvement cannot be applied
to candidate4. See [original evidence](BUN_BOUNDARIES_20260909.md).

Added `host-compute` to the benchmark registry because the existing pure-JS
matrix lacked the host compute workload. `layoutKernel` matches the archived
host harness byte-for-byte (harness SHA-256
`62496a5d64fc4966e77cdca8c401bd11d47a5a47a750a11ae50e032d5b47fae1`).
It retains all 100 batches, the 40-iteration Fibonacci loop, the overwritten
checksum, fixed seed and `layout:165580141` result. The argument convention is
the common driver convention, but workload sizing stays fixed to one host
render. GPUI allocation, snapshot materialization and validation are excluded.

Initial Apple M3 measurements: 30 retained pairs/configuration, default Bun,
same-binary QuickJS and automatic, plus forced-tier diagnostics. All 150 retained
samples have matching checksums, script/driver hashes and fixed warmup policy.
The raw initial probe remains archived; the README now shows the newer paired
old/current-runtime experiment described below.
QuickJS/automatic speed is not compared against host snapshot time. The short
throughput probe is not used for any throughput claim. The existing harness
does not serialize the five discarded warmup-process results.

Reproduce on this host:

```sh
cargo build -p rquickjs-jit-benchmarks --release --bins
JIT_BENCH_WORKLOADS=host-compute JIT_BENCH_WINDOWS=1 JIT_BENCH_WINDOW_MS=1 \
  target/release/jit-bench compare --modes interpreter,bun,automatic,tier1,tier2 \
  --output /tmp/host-compute.json
```

Validation passed: exact expected-result test, shared-driver QuickJS/Bun
checksum test (including the new script), benchmark Clippy with `-D warnings`,
and archived source/hash verification. No runtime code changed.

Next: perform same-lock old-runtime/current-runtime compute and host comparisons
on native Linux x86_64, then isolate the remaining C/runtime delta. Docker on
the current ARM machine is available for Linux correctness checks, but x86
emulated timings cannot resolve the historical native x86 performance gate.
The new probe shows profitable native computation on Apple M3; it does not
identify the Linux regression's cause or satisfy Phase A.

## Paired old/current baseline on Apple M3

The previous goal turn added a missing workload and verified fresh samples;
this turn is further progress through an actual old/current-runtime comparison.
No source-performance fix or completed Semantic SSA phase is claimed.

Built `928a8f2` from a separate Git archive with the current Cargo.lock and
exact current benchmark sources. The current binary uses `023a220`; runtime
source has no local modifications. Both immutable binary hashes, all harness
hashes, the complete lockfile and build commands are retained in
[the evidence archive](../benchmarks/results/semantic-ssa-baseline-paired-arm64.tar.gz).
The archive's `sources.json` records the pinned QuickJS revision and compiler.

All eight controls completed: host-compute, generic-call-entry,
generic-call-fallback, call-heavy, property-heavy, arrays-typed, scalar-loop,
and mixed-quotes. For each, five configurations (old/current automatic,
old/current interpreter, Bun default) run five discarded and 30 retained
processes. All 1,400 records are saved. The summarizer verifies their identities,
checksums, driver/script hashes and warmup policies before computing statistics.
No sample is excluded based on native coverage or compilation quietness.

New reproducible Unix tools are `benchmarks/paired.py` and
`benchmarks/summarize_paired.py`, adapted from the candidate4 archive. Collection
uses a lock, refuses an existing output directory, records incomplete status
before sampling, and verifies executable hashes again at completion. Native
Linux can use `--cpu 2`; macOS omits affinity. Negative checks verified refusal
of zero pairs, Linux affinity on macOS, and overwriting existing evidence.
The summarizer was also checked to reject incomplete collection status.

```sh
# binaries.json maps "baseline" and "candidate" to immutable jit-bench paths.
python3 benchmarks/paired.py --binaries /tmp/binaries.json \
  --scripts "$PWD/benchmarks/scripts" \
  --workloads host-compute,generic-call-entry,generic-call-fallback,call-heavy,property-heavy,arrays-typed,scalar-loop,mixed-quotes \
  --output /tmp/new-paired-run
python3 benchmarks/summarize_paired.py /tmp/new-paired-run
```

The [README](../README.md#experimental-jit-performance) contains every focused
row and [the summary](../benchmarks/results/semantic-ssa-baseline-paired-arm64.json)
contains every configuration. Speed is reference latency divided by candidate
latency, using paired geometric means and 10,000 bootstrap draws (seed
20260909; sorted indices 249/9749). This differs from the initial single-probe
bootstrap seed; do not mix estimates from the two experiments.

Host-compute automatic versus old automatic is statistically tied:
1.0184x [0.9965, 1.0416], between 0.35% slower and 4.16% faster. Interpreter
versus old interpreter is also statistically tied: 1.0616x [0.9898, 1.1874],
between 1.02% slower and 18.74% faster. This provides no same-direction
reproduction of the historical native Linux loss on the current Apple host.
Generic-call-entry/property-heavy/arrays-typed are only 0.0219x/0.0347x/0.0766x
Bun speed. Generic fallback is 0.3644x its own interpreter speed. These remain
explicit unmet gates, despite improvements over the old JIT.

Native Linux host availability has been asked asynchronously. That evidence
remains pending; no emulated timing or unrelated host result is substituted for
the required Linux host comparison. The full architecture objective is unfinished.

## Historical native-host prerequisite audit (superseded as a development gate)

The missing native Linux x86_64 environment persisted through three consecutive
goal turns. The first two made concrete progress by adding the equivalent kernel
and completing the locked old/current comparisons; those measurements are done,
and no paired collector or summarizer remains running. The host question remains
unanswered. Current host is Darwin ARM64 and Docker reports AArch64.

The final source check found that the only executable C integration change from
`928a8f2` to `023a220` is `0017-shape-generation.patch`; `sys/build.rs` is
unchanged. Its runtime field is inserted inside the JIT extension after the
original runtime fields, so it does not move those original fields. The patch
also changes shape layout, invalidation stores, feedback, guards and ABI
metadata. This narrows candidate changes but does not identify the cause of the
native Linux timing regression. No runtime fix is justified by the current
evidence, and emulated timings would not resolve that causal question.

The original ordering held SSA implementation pending the host regression fix.
The user superseded that development gate with “我们先在 macOS 上实现完整”
on 2026-09-10. Implementation and acceptance now proceed on macOS; no Linux
result is implied. Resume the deferred native Linux investigation when a host is supplied;
use the archived same-lock build inputs and paired collector, reproduce the
actual host snapshot/reload harness, and isolate the C change before proposing
a fix. All later architecture and performance requirements remain unchanged.


## macOS semantic value construction

The production numeric pass now constructs explicit Int32 constants, frame
inputs and ordered binary operands, and reuses repeated nested expression trees.
Machine lowering consumes each reused operation’s original operands before
pushing its canonical result. This is intra-block value numbering: CFG Phi,
exact SSA FrameState and graph-driven numeric lowering remain the next work.

Independent static review found no concrete production semantic regression.
Its exceptional-path coverage concern was addressed with separately warmed
runtime instances and measured Tier 2 entries for overflow, negative zero, NaN
and object coercion; object coercion also requires a deopt counter increment.
The linear graph value cap prevents quadratic retained graph growth. The
configured byte budget checks retained graph size after construction, not peak
scratch-map allocation during construction.

The full macOS runtime suite passed (531 passed, one existing ignored test)
after extending 15 native execution tests
from Linux to macOS and fixing seven platform tests that assumed 4 KiB pages.
Platform quotas now use the operating system’s actual page size (16 KiB on this
Apple M3), preserving the original quota and fault-injection assertions. The
benchmark checksum test and warnings-as-errors Clippy passed.

Full 26-workload five-configuration sampling completed: 4,550 process records,
all discarded and retained samples, matching script/driver/checksum identities.
The [complete README matrix](../README.md#experimental-jit-performance),
[all comparisons](../benchmarks/results/semantic-values-paired-arm64.json) and
[verified source/raw archive](../benchmarks/results/semantic-values-paired-arm64.tar.gz)
record this candidate. 4,593 archived files passed hash verification.

The no-regression gate remains unmet: collections is 0.9761x previous speed
[0.9708, 0.9811], arrays-typed 0.9863x [0.9827, 0.9905], and
calls-recursion-closures 0.9938x [0.9901, 0.9972]. Collections and closure
recursion have zero timed native entries; interpreter controls also slow for
collections (0.9774x) and arrays (0.9900x). These controls prevent attributing
the differences directly to scalar CSE; the cause remains unresolved.
Scalar-expressions is statistically tied with the previous JIT: 0.9691x
[0.9016, 1.0329], between 9.84% slower and 3.29% faster. Although the structural
unprofiled lowering test eliminates a repeated multiply, an end-to-end gain
has not been demonstrated. The full architecture objective remains active.

[Validation commands and results](../benchmarks/results/semantic-values-validation-arm64.json)
record the macOS checks. The measured benchmark binary remained unchanged after
the test-only portability fixes.

## CFG value flow and recovery bindings, 2026-09-10 (unmeasured)

The next working-tree candidate builds argument, local and operand-stack Phi
inputs from each predecessor's outgoing ValueIds. A loop at bytecode pc zero
has a separate implicit initial edge. Cranelift lowering now defines target
Phi variables from those graph edges; it no longer relies solely on the old
frame-variable merges to connect blocks. Numeric instructions read graph
operands, and their recovery bridge variables are bound from the pre-effect
SSA FrameState. Existing exit ownership materialization is retained.

Graph construction preserves assignment aliases and stack permutations,
records frame definitions after local arithmetic and reentrant effects, and
captures exact argument/local/stack IDs at guards. Retained graph storage is
included in the compile quota; linear value, Phi-edge and captured-state limits
bound expansion. Configured-budget enforcement during scratch construction
still needs work. Full lazy materialization, inline frame chains and
feedback-specialized first-class semantic operations are not yet implemented.

Validation: the complete release runtime suite passed 536 tests with one
existing ignored test. Two subsequently added FrameState tests also passed.
The tests cover diamonds, stack joins, initial/backedge loop values, simultaneous
loop-variable exchange with odd/even iteration counts, parameter/nested
assignments, postfix values, values retained across calls, and arithmetic
overflow deopt after a Phi and parameter assignment with unchanged Int32 input
types. An additional object-coercion probe checks conversion count and result
but may exit at the entry guard. Native tests assert Tier2 entry and
appropriate deopt behavior. Warnings-as-errors Clippy passed. Read-only review
confirmed that backend execution now consumes Phi predecessor identities.

The archived numeric-step matrix above predates this CFG candidate. Its results
must not be attributed to the current working tree. A new complete three-engine
matrix and paired previous-runtime controls remain pending for the completed
Phase B candidate; the no-regression and overall performance gates are open.

## Feedback-selected numeric semantics, 2026-09-10 (unmeasured)

Arithmetic graph construction now selects Number, checked Int32 or checked
Float64 contracts before either value-numbering pass. Both CSE keys include
the mode. Lowering consumes the graph's arithmetic operation and mode rather
than looking up per-PC feedback itself. Both production compiler entry points
build the specialized graph once and derive published optimization metrics
from that graph, replacing the previous separate default-mode planning graph.

A native regression exposed an existing Float64 error across an unobserved
Phi edge: after warming the floating branch, an integer constant on the other
branch was interpreted as floating payload bits, returning 2.5 instead of 3.5.
The test failed before the fix. Float64 lowering now checks both operand tags
before bitcasting and uses the existing SSA recovery/ownership exit on failure.
The reproducer passes with unchanged argument types across warm and probe calls.

The complete release runtime suite passed 540 tests with one existing ignored
test, and warnings-as-errors Clippy passed. The automatic-mode structured
differential test now waits up to ten seconds for actual native entry instead
of assuming 128 warm iterations finish asynchronous compilation. Its native
entry and semantic assertions remain mandatory. Read-only review confirmed
the mode/CSE propagation, Float64 checks and final-graph metric source.

These changes remain unmeasured; the README numeric-step matrix is historical.
Standalone semantic check nodes, comparison migration, flow-sensitive facts,
configured scratch-allocation quotas and full lazy deoptimization remain open
alongside the later call/property/array architecture and performance gates.

## Explicit numeric checks and local facts, 2026-09-10 (unmeasured)

Each surviving arithmetic producer now has explicit operand checks carrying
ValueId, numeric contract and pre-effect FrameState identity. The backend
consumes these checks for all three numeric modes; the separate handwritten
per-mode tag guards have been removed. Arithmetic overflow, negative-zero and
inexact-division exits remain intact. Retained check storage is counted in the
compile quota.

A block-local facts pass eliminates checks on already checked immutable values,
known Int32 constants and successful arithmetic results. Number accepts stronger
Int32/Float64 facts, while stronger checks remain for merely Number values.
Frame reloads after reentrant effects have fresh identities. Facts reset at CFG
boundaries; general flow-sensitive propagation is still pending. The public
graph exposes checks and the static eliminated-check count for inspection.

The full release runtime suite passed 541 tests with one existing ignored test,
and Clippy passed with warnings denied. Structural tests verify that
`(a+b)*(a-b)` retains two of six operand checks in each numeric mode, while
post-call reloads require new checks. A subsequent test also passed for CFG
boundary resets and retaining Float64 checks after Number facts. Read-only review found no correctness
defect in alias reuse, frame reloads, block resets or ownership exits.
Performance remains unmeasured; this is progress toward the semantic foundation
and facts model, not completion of Phase C or the architecture goal.

## CFG numeric facts, 2026-09-10 (unmeasured)

Numeric facts now propagate from predecessor exits through explicit Phi inputs.
All incoming edges must provide evidence; Int32 and Float64 merge to Number.
Missing evidence, reentrant frame reloads and the implicit initial function
edge prevent unjustified elimination. A bounded work list reschedules successors
when exit facts change. On analysis-budget exhaustion it discards cross-block
facts and regenerates block-local checks, preserving compilability.

The full release runtime suite passed 544 tests with one existing ignored test.
Tests cover both-edge proofs, one unchecked edge, a call invalidating one edge,
and preserving the first-iteration check at a pc-zero loop. A native probe keeps
all argument types fixed while selecting an unchecked Float64 Phi edge after
warming the Int32-producing branch; it enters Tier2 and deopts with the correct
result. An attempted object-input fixture did not reach Tier2 and was replaced;
it supplies no evidence about object Phi execution. Read-only review found no
unsound loop inference or budget fallback behavior.
The subsequent forced-budget test also passed: existing cross-block eliminations
are removed when the budget is exhausted and restored when analysis runs with
sufficient budget. All 18 semantic tests and warnings-as-errors Clippy passed
after adding that test hook.

## Semantic comparisons and frozen candidate, 2026-09-10

The four ordered comparisons now have explicit SSA operands and comparison
operators. They share the arithmetic preparation path, consuming graph checks
and pre-effect recovery bindings. Successful comparison checks contribute Number
facts for operands on both outgoing edges; Bool results never contribute Number
facts. The initial loop comparison keeps its guard even when later arithmetic
can reuse that proof. Structural and native tests cover all four operations,
NaN on either side, negative zero and Bool-result coercion requirements.

The complete runtime suite passed 547 tests with one existing ignored test;
warnings-as-errors Clippy and read-only review passed. A new candidate binary
and exact source snapshot are frozen for the complete 27-scenario paired run.
The immutable pre-SSA `023a220` binary is the previous-runtime control; QuickJS
uses the candidate runtime with JIT detached, and Bun uses defaults. Collection
is active under the shared fixed-warmup protocol with 30 retained pairs and
five discarded pairs. No performance gate is claimed until collection,
verification and the full README matrix are complete.

The new `scalar-control-flow` benchmark exercises branch-produced numeric Phi
values in a loop. Its Bun/QuickJS harness checksum comparison passed. The next
complete performance matrix therefore has 27 scenarios; new timings remain
pending, and the archived 26-scenario table remains historical.

## Inlining integration findings while the frozen run collects

Read-only inspection confirms that `CompileRequest` owns a `VerifiedFunction`,
but `artifact_from_relocatable` drops that snapshot when building the cached
artifact. `DirectCallTarget` retains the callee signature and executable
publication only. Consequently the current caller graph builder cannot inspect
the callee body; an existing direct native call is not graph inlining.

The next inlining work must retain a bounded, worker-safe callee snapshot,
expose it to caller graph construction, and charge its retained storage through
`CompiledArtifact::charge_bytes`. `CompileSnapshot::owned_bytes` covers copied
snapshot data but does not account for `VerifiedFunction` instructions, CFG or
OSR metadata, so it cannot alone represent the retained verified body. Callee
generation dependencies must remain explicit alongside the current caller and
call dependencies. Snapshot retention and executable publication lifetimes are
separate concerns. These are identified integration requirements, not implemented
inlining or evidence that its performance/deoptimization gates pass.

## Complete CFG candidate measurements, 2026-09-10

The frozen 27-scenario collection and summarizer both exited successfully.
All 4,725 process records are retained; the evidence archive contains 4,819
verified files. The root README now contains all 27 absolute timing rows,
previous-JIT/QuickJS/default-Bun speed comparisons and confidence intervals,
native entry coverage and paired interpreter controls. The previous 26-scenario
numeric matrix is preserved as historical data.

The numeric-foundation no-regression gate is **not met**:

| Scenario | Candidate / previous JIT speed [95% CI] | Candidate / previous interpreter speed [95% CI] |
| --- | --- | --- |
| host-compute | 0.7316x [0.7016, 0.7868] | 0.9880x [0.9352, 1.0343] |
| numeric | 0.6605x [0.5227, 0.8187] | 1.0096x [0.9950, 1.0258] |
| scalar-loop | 0.6691x [0.6182, 0.7085] | 1.0189x [0.9993, 1.0406] |
| fibonacci-iterative | 0.5540x [0.5345, 0.5801] | 1.0036x [0.9842, 1.0303] |

All four interpreter controls are statistically tied. Their ranges are,
respectively, 6.48% slower to 3.43% faster, 0.50% slower to 2.58% faster,
0.07% slower to 4.06% faster, and 1.58% slower to 3.03% faster.
Each automatic sample entered native code ten times. Compilation was quiet
in 30/30 samples except numeric (29/30). These observations prioritize native
loop code generation for investigation, but do not yet identify the cause.

`scalar-expressions` improved to 1.3634x previous speed [1.2484, 1.5204].
`scalar-control-flow` was statistically tied with previous performance,
0.8383x [0.7571, 1.0136], spanning 24.29% slower to 1.36% faster.
Neither result cancels the four regressions.

The architecture gates also remain open: generic-call-entry, property-heavy
and arrays-typed reach only 0.0253x, 0.0361x and 0.0736x default Bun speed;
generic-call-fallback reaches 0.3779x same-version QuickJS speed. Complete
intervals and absolute timings are in README. Next work is to inspect the
regressed loops' emitted IR and machine code before advancing the foundation.

Evidence: `benchmarks/results/semantic-cfg-paired-arm64.tar.gz`, its summary
JSON and manifest. Candidate binary SHA256:
`a2ea242d6823f57d69258e337f7607cbe55e94996609973d4daf5d403c94f56d`.
These are macOS measurements; no native Linux host-reload conclusion follows.

Loop diagnostics after this run passed 16 existing tests. The representative
sum loop still carries dynamic tags through Phi edges after its Int32 entry
guard; its comparison and addition emit repeated tag checks. A diagnostic
assertion requiring only the poll-budget i64 header parameter failed, exposing
the extra tag parameters. Canonicalizing entry tags and interning constant tags
at edges removed some parameters but did not eliminate the loop-carried tag
cycles; final ARM64 assembly still contained tag tests. That incomplete change
and its diagnostic test were saved outside the worktree, then removed. All
frozen source hashes were verified again, so the measured candidate remains
the current runtime source. This is a code-generation lead, not a measured fix.
The next investigation must propagate justified representation facts through
cyclic SSA values without assuming that an unknown or reloaded value is Int32.

## Cyclic representation proofs, 2026-09-10 (unmeasured)

A bounded monotone type-set analysis now distinguishes unresolved cyclic input
from unknown JavaScript types. It seeds only explicitly guarded entry arguments
and known numeric producers, unions all Phi edges, and discards its entire
result on budget exhaustion. Ordinary frame reloads remain unknown. Raw-loop
poll bindings can preserve their exact prior SSA values because that lowering
does not reload compiler variables. Side paths explicitly disable the new
entry-seeded analysis. Its scratch/result storage is included in the controlled
compile estimate.

The backend uses proven Int32 block inputs to replace dynamic tag bindings with
the constant Int32 tag after the original entry guard. Read-only review found
no new correctness defect. Unit tests cover cyclic seeds, missing entry guards,
unanchored cycles, late unknown edges and budget exhaustion. An ARM64 machine
test verifies that a sum loop with explicitly modeled arithmetic no longer tests
for Float64 tags, while checked integer arithmetic remains. Disabling this
optimization makes that machine test fail; it passes with the optimization.
The full runtime suite passed 551 tests, with one existing ignored test;
warnings-as-errors Clippy passed.

This is not yet a measured resolution of the four regressions. Opaque increment
producers and the full numeric representation pipeline still need attention;
the archived 27-scenario matrix predates this change. No new performance claim,
inlining completion or architecture gate is established by these tests.

## Semantic increment/decrement updates, 2026-09-10 (measurement pending)

Increment/decrement, postfix forms and local update bytecodes now have explicit
SSA Update inputs, deltas and results. Postfix preserves the old input as its
first output; local forms define the destination local with the new ValueId.
The frontend selects the raw-loop Int32 contract before graph construction;
other updates retain Number semantics. Backend lowering consumes the graph's
operand, delta, exact pre-effect state and one explicit numeric check. Overflow
still exits before the update is written back.

The ARM64 machine regression now covers ordinary `i++` and `i--` loops as well
as explicit addition, and passes without Float64 tag tests in those loops.
Structural tests cover all six bytecodes, including verified serialized
inc_loc/dec_loc fixtures. A native probe retains Int32 argument tags while
overflowing a postfix increment, and verifies exact old/new values after deopt.
Read-only review found no correctness defect. Full validation passed 553 tests
with one existing ignored test; Clippy and the release benchmark build passed.

A frozen candidate and its exact source/build evidence are collecting a focused
six-scenario diagnostic with ten retained pairs. It includes immutable pre-SSA
baseline, prior CFG candidate, new candidate, same-version interpreters and
default Bun. This checks the four regressions plus scalar expressions/control
flow before committing to another complete matrix. It is not full acceptance;
the archived 27-scenario table still describes the previous CFG candidate.
New candidate binary SHA256:
`3d8922caf6a059b6deec0add12403f158874bd7fba664b0ad3f56727b611bb53`.

## Focused update diagnostic, 2026-09-10

All 540 process records passed collector/summarizer validation. Six scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior CFG ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior CFG speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| host-compute | 0.062166 | 0.088000 | 1.231646 | 0.227208 | 0.061062 | 0.9363x [0.7594, 1.0572]; statistically tied (24.06% slower to 5.72% faster) | 1.3541x [1.0870, 1.5723] | 19.4018x [15.7197, 22.3019] | 3.6056x [2.8009, 4.4122] |
| numeric | 0.024167 | 0.036125 | 0.394230 | 0.121688 | 0.029625 | 0.8130x [0.8054, 0.8209] | 1.3179x [1.1937, 1.5741] | 13.3509x [13.2721, 13.4397] | 4.4548x [3.9541, 5.4191] |
| scalar-loop | 0.023355 | 0.033854 | 0.382792 | 0.109313 | 0.028667 | 0.8076x [0.7827, 0.8273] | 1.1808x [1.1449, 1.2087] | 13.2273x [12.8785, 13.5077] | 3.7821x [3.6647, 3.9044] |
| fibonacci-iterative | 0.868271 | 1.597438 | 21.583917 | 1.041937 | 1.102729 | 0.8068x [0.7621, 0.8765] | 1.5023x [1.4191, 1.6044] | 19.4465x [18.8695, 19.9637] | 0.9529x [0.9006, 1.0314]; statistically tied (9.94% slower to 3.14% faster) |
| scalar-expressions | 0.072042 | 0.058500 | 0.831375 | 0.074229 | 0.057167 | 1.2648x [1.2198, 1.3083] | 1.0197x [0.9859, 1.0537]; statistically tied (1.41% slower to 5.37% faster) | 14.3376x [13.9781, 14.6539] | 1.4842x [1.2698, 1.8126] |
| scalar-control-flow | 0.047542 | 0.061833 | 1.180230 | 0.078167 | 0.056562 | 0.8551x [0.8297, 0.8893] | 1.0971x [1.0833, 1.1148] | 20.8789x [20.6585, 21.1097] | 1.4031x [1.3541, 1.4676] |

All candidate samples had ten native entries and quiet compilation. Numeric,
scalar-loop, iterative Fibonacci and scalar-control-flow remain significantly
slower than the pre-SSA baseline in this diagnostic. The first three recover
part of the prior CFG loss, but none passes the no-regression gate. Host-compute
is tied with baseline; scalar expressions improves over baseline. The complete
27-scenario README matrix remains the previous frozen CFG run.

The strengthened ARM64 sum-loop test also passes with at most three 64-bit
comparisons (two entry tags and the poll budget), so repeated tag comparisons
are insufficient to explain the remaining slowdown. Next inspect invariant
argument Phi cycles and register moves rather than assuming another tag check.

Evidence: `benchmarks/results/semantic-updates-diagnostic-arm64.tar.gz`, summary JSON and manifest.


### Constant-true numeric guard branches (work in progress)

Current candidate D machine disassembly disproved the assumption that removing
64-bit tag comparisons removes the whole guard cost. The simple sum loop still
contains `movz wN, #1; uxtb wN, wN; cbnz xN, ...` at numeric operations, with cold
materialization blocks retained. This is a stronger immediate lead than invariant
argument Phi traffic; no Phi-origin simplification has been implemented.

Lowering now consumes the existing all-edge Int32 representation proof to omit
Number/Int32 guard branches entirely. The proof is only computed for guarded
Int32 loops without side paths; Float64 checks, exact state validation and
arithmetic overflow checks remain. The machine regression assertion failed before
this change and passed afterward for explicit addition, postfix increment and
decrement loops. A read-only reviewer found no correctness issue. Full runtime
tests passed; benchmark results remain pending and candidate D measurements must
not be attributed to this change.

## Focused guard-branch diagnostic, 2026-09-10

All 540 process records passed collector/summarizer validation. Six scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior updates ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior updates speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| host-compute | 0.060146 | 0.059041 | 1.180541 | 0.190771 | 0.058438 | 1.0221x [1.0003, 1.0368] | 0.9923x [0.9697, 1.0118]; statistically tied (3.03% slower to 1.18% faster) | 19.9374x [19.3998, 20.3926] | 3.3074x [3.1456, 3.5157] |
| numeric | 0.023333 | 0.028583 | 0.378958 | 0.109146 | 0.023667 | 1.0092x [0.9790, 1.0536]; statistically tied (2.10% slower to 5.36% faster) | 1.2235x [1.1845, 1.2764] | 16.1750x [15.7376, 16.6300] | 4.8141x [4.4748, 5.3534] |
| scalar-loop | 0.022895 | 0.028375 | 0.378667 | 0.109896 | 0.023062 | 0.9991x [0.9889, 1.0116]; statistically tied (1.11% slower to 1.16% faster) | 1.2452x [1.2267, 1.2662] | 16.6143x [16.4002, 16.8261] | 4.9199x [4.6893, 5.2355] |
| fibonacci-iterative | 0.882479 | 1.096937 | 21.286334 | 0.987854 | 0.872958 | 1.0101x [0.9915, 1.0307]; statistically tied (0.85% slower to 3.07% faster) | 1.2545x [1.2424, 1.2677] | 24.5107x [24.0115, 25.0084] | 1.1235x [1.0784, 1.1686] |
| scalar-expressions | 0.071063 | 0.056250 | 0.812604 | 0.068479 | 0.048334 | 1.5947x [1.4308, 1.9323] | 1.1607x [1.1280, 1.1871] | 16.6569x [16.2284, 16.9606] | 1.4026x [1.3642, 1.4314] |
| scalar-control-flow | 0.047729 | 0.056563 | 1.163354 | 0.075271 | 0.050833 | 0.9662x [0.9406, 0.9980] | 1.1194x [1.1127, 1.1273] | 22.8817x [22.7745, 22.9904] | 1.4922x [1.4771, 1.5089] |

All candidate samples had ten native entries and quiet compilation. Numeric,
scalar-loop and iterative Fibonacci are statistically tied with the pre-SSA
baseline. Host-compute and scalar expressions improve. Scalar-control-flow
remains significantly slower (0.9662x baseline speed; 95% CI 0.9406–0.9980).
The complete matrix and no-regression acceptance remain pending.

Evidence: `benchmarks/results/semantic-constant-guards-diagnostic-arm64.tar.gz`, summary JSON and manifest.


### Semantic binary bitwise operations (work in progress)

The frozen guard-branch diagnostic completed with 540 validated process records
and 638 verified archive files. Scalar-control-flow was still 0.9662x baseline
speed. Its machine dump retained constant-true branches at legacy bitwise
lowering, and the accumulator's bitwise-produced tag remained a loop variable.

The graph now models Or/And/Xor/Shl/Sar/Shr with explicit operand ValueIds and
Int32 checks. Lowering consumes graph operands and operation, reuses exact
FrameState checks, and omits proven input guards. Result facts seed Int32 for
signed bitwise results; Shr remains Number because the general path can produce
Float64. Raw-loop unsigned overflow still exits before writing the result.
The extra machine regression failed before migration and passed after it.
A representation unit test covers all six result contracts. Read-only review
found no correctness issue. Full runtime validation passed (554 tests, one existing ignored); Clippy with
warnings denied passed. Candidate benchmarking remains pending.

## Focused bitwise diagnostic, 2026-09-10

All 540 process records passed collector/summarizer validation. Six scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior guards ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior guards speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| host-compute | 0.062230 | 0.060750 | 1.259375 | 0.243000 | 0.061104 | 1.0140x [0.7361, 1.4357]; statistically tied (26.39% slower to 43.57% faster) | 0.9480x [0.7276, 1.2070]; statistically tied (27.24% slower to 20.70% faster) | 17.5475x [13.8536, 20.5878] | 3.6621x [2.6861, 4.8057] |
| numeric | 0.023250 | 0.022979 | 0.381854 | 0.108438 | 0.023438 | 1.0039x [0.9802, 1.0325]; statistically tied (1.98% slower to 3.25% faster) | 1.0045x [0.9730, 1.0481]; statistically tied (2.70% slower to 4.81% faster) | 16.4750x [16.1920, 16.7605] | 4.6508x [4.5447, 4.7616] |
| scalar-loop | 0.023312 | 0.023584 | 0.383896 | 0.115896 | 0.023208 | 1.0160x [0.9810, 1.0564]; statistically tied (1.90% slower to 5.64% faster) | 1.0110x [0.9851, 1.0387]; statistically tied (1.49% slower to 3.87% faster) | 16.5569x [16.0982, 16.9570] | 5.2589x [4.8088, 6.0807] |
| fibonacci-iterative | 0.897938 | 0.877104 | 21.350792 | 1.070083 | 0.880938 | 0.8934x [0.6695, 1.0476]; statistically tied (33.05% slower to 4.76% faster) | 0.8674x [0.6498, 1.0197]; statistically tied (35.02% slower to 1.97% faster) | 21.1118x [16.1979, 24.4202] | 1.0675x [0.9143, 1.1855]; statistically tied (8.57% slower to 18.55% faster) |
| scalar-expressions | 0.072521 | 0.050521 | 0.828000 | 0.091750 | 0.045062 | 1.5713x [1.4925, 1.6185] | 1.1230x [1.0629, 1.1873] | 17.9689x [17.0123, 18.6190] | 2.0161x [1.8527, 2.1986] |
| scalar-control-flow | 0.050250 | 0.053041 | 1.219396 | 0.104438 | 0.049313 | 1.3858x [0.9314, 2.3015]; statistically tied (6.86% slower to 130.15% faster) | 1.0421x [0.8746, 1.2178]; statistically tied (12.54% slower to 21.78% faster) | 26.4557x [21.1162, 32.7748] | 2.5052x [1.7699, 4.3845] |

All candidate samples had ten native entries and quiet compilation. Five
scenarios are statistically tied with the pre-SSA baseline; scalar expressions
improves. Host-compute, iterative Fibonacci and scalar-control-flow have wide
intervals, so absence of a significant slowdown is not proof of no regression.
Proceed to the prespecified full 27-scenario, 30-pair matrix with the same frozen
candidate, retaining every sample. The full acceptance gate remains pending.

Evidence: `benchmarks/results/semantic-bitwise-diagnostic-arm64.tar.gz`, summary JSON and manifest.


## Full semantic bitwise matrix, 2026-09-10

The frozen semantic bitwise candidate (binary `081879eab797ed3c5ce3ce7482777b97eedbed3522a69ac4df09027ac71eb00e`)
completed all 27 scenarios, five configurations, five discarded and 30 retained
fresh processes: 4,725 records. The summarizer validated all checksums/protocols;
the archive was read back and every recorded file hash checked. README now
contains the complete timing/speed/CI matrix and native/interpreter controls;
the preceding CFG matrix is explicitly historical.

The no-regression gate remains unmet: call-heavy 0.9621x [0.9414, 0.9877],
generic-call-fallback 0.9818x [0.9631, 0.9988], adversarial 0.8439x
[0.7290, 0.9714]. Corresponding interpreter controls cross parity. Adversarial
has zero timed native entries and wide intervals; this does not establish a
JIT machine-code cause. Numeric, scalar-loop, iterative Fibonacci and control
flow now cross parity; scalar expressions improves 1.5487x [1.3653, 1.7199].
Some early samples overlap compilation (host-compute 22/30 quiet, control flow
27/30, scalar expressions 29/30); all samples are retained and this limitation
is visible in README. No subsequent architecture or Bun gate is complete.

Evidence: `benchmarks/results/semantic-bitwise-paired-arm64.tar.gz`, JSON and
manifest. No collectors remain active. Next inspect entry-derived numeric
facts in non-loop callees before attributing call-path losses to runtime costs.


### Reuse Int32 entry facts outside raw loops (work in progress)

The non-loop `(a,b) => a+b` machine test initially passed a check for at most
two tag comparisons, but failed the stronger check for at most two conditional
branches (entry plus overflow). Comparisons were shared while the operand guard
branch remained. This is the same distinction found in the earlier loop work.

The existing representation analysis now runs for all Int32 entry specializations
without side paths. Poll FrameRead aliases are allowed only for raw loops;
other poll/reentrant reads remain unknown. Per-argument entry guards are all
Int32 whenever the aggregate entry specialization is Int32. Machine regression
passes after this change; the full runtime suite passed (555 tests, one existing
ignored), and Clippy with warnings denied passed. Read-only review found no
correctness defect. This is an
unmeasured candidate and does not close the call-heavy or fallback regressions.


### Per-argument entry proofs (work in progress)

The mixed `(Int32, Int32, Object)` leaf test reproduced a redundant operand
guard even though its entry guard checked each numeric argument. Representation
analysis now takes a guarded-slot predicate matching the exact entry selection:
per-argument specialization, falling back to the aggregate entry contract.
It allocates no extra argument mask. Non-Int32 arguments remain unknown. Derived
constant and successful bitwise/Int32 result proofs can now be used regardless
of the aggregate entry type; side paths still disable the analysis, and poll
aliases remain restricted to raw Int32 loops.

The mixed leaf machine test failed before and passed after the change. A sparse
slot unit test verifies that nonnumeric slots between numeric slots stay unknown.
Full runtime tests passed (557, one existing ignored), Clippy with warnings denied
passed, and read-only review found no correctness issue. The planned paired
diagnostic retains call-heavy, generic entry/fallback, adversarial, host-compute
and numeric/control-flow controls. No performance acceptance is claimed yet.

## Focused entry-facts diagnostic, 2026-09-10

All 720 process records passed collector/summarizer validation. Eight scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior bitwise ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior bitwise speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| call-heavy | 0.306020 | 0.315312 | 0.953375 | 0.012438 | 0.309437 | 0.9913x [0.9833, 0.9989] | 1.0264x [1.0086, 1.0473] | 3.0889x [3.0403, 3.1431] | 0.0403x [0.0393, 0.0413] |
| generic-call-entry | 0.240853 | 0.245292 | 0.785146 | 0.005833 | 0.242063 | 0.9990x [0.9873, 1.0150]; statistically tied (1.27% slower to 1.50% faster) | 1.0088x [0.9997, 1.0141]; statistically tied (0.03% slower to 1.41% faster) | 3.2318x [3.1930, 3.2646] | 0.0239x [0.0236, 0.0241] |
| generic-call-fallback | 2.861209 | 2.883917 | 1.061271 | 0.011770 | 2.891271 | 1.0454x [0.9789, 1.1671]; statistically tied (2.11% slower to 16.71% faster) | 0.9966x [0.9829, 1.0103]; statistically tied (1.71% slower to 1.03% faster) | 0.3663x [0.3615, 0.3704] | 0.0040x [0.0039, 0.0041] |
| adversarial | 0.644896 | 0.647437 | 0.659813 | 0.110542 | 0.644271 | 1.0034x [0.9951, 1.0126]; statistically tied (0.49% slower to 1.26% faster) | 1.0051x [0.9930, 1.0166]; statistically tied (0.70% slower to 1.66% faster) | 1.0206x [1.0108, 1.0298] | 0.1700x [0.1671, 0.1730] |
| host-compute | 0.059812 | 0.059395 | 1.181062 | 0.184104 | 0.058396 | 1.0355x [1.0186, 1.0661] | 1.0121x [1.0041, 1.0211] | 20.1770x [20.0801, 20.2600] | 3.1579x [3.1144, 3.2008] |
| numeric | 0.023291 | 0.023230 | 0.385146 | 0.112500 | 0.023749 | 0.9612x [0.9031, 0.9967] | 0.9657x [0.9031, 1.0080]; statistically tied (9.69% slower to 0.80% faster) | 15.9071x [14.9350, 16.4931] | 4.6109x [4.2995, 4.8093] |
| scalar-loop | 0.023291 | 0.023125 | 0.380271 | 0.107958 | 0.023063 | 0.9961x [0.9758, 1.0128]; statistically tied (2.42% slower to 1.28% faster) | 1.0022x [0.9908, 1.0129]; statistically tied (0.92% slower to 1.29% faster) | 16.5208x [16.3753, 16.6183] | 4.6239x [4.5184, 4.7221] |
| scalar-control-flow | 0.047854 | 0.047229 | 1.163875 | 0.075416 | 0.046999 | 1.0218x [1.0124, 1.0339] | 1.0030x [0.9947, 1.0118]; statistically tied (0.53% slower to 1.18% faster) | 24.8404x [24.4711, 25.2224] | 1.6003x [1.5886, 1.6144] |

All candidate samples had quiet compilation. Native entries were ten per batch
except generic-call-fallback (20,000) and adversarial (zero). Call-heavy remains
slower than pre-SSA (0.9913x [0.9833, 0.9989]); numeric also falls below parity
(0.9612x [0.9031, 0.9967]). Generic entry/fallback, adversarial and scalar-loop
are tied; host-compute and control-flow improve. The prior full-matrix failures
are not closed by this smaller diagnostic. No-regression remains unmet.

Evidence: `benchmarks/results/semantic-entry-facts-diagnostic-arm64.tar.gz`, summary JSON and manifest.


### Numeric representation facts (work in progress)

The cyclic representation solver already tracked Int32, Float64 and their union,
but returned only Int32 proofs. Lowering now receives the full numeric mode.
A Number proof may remove a Number check; it cannot remove a strict Int32 or
Float64 check. Only a definite Int32 proof canonicalizes a Phi tag. Unknown
edges and exhausted analysis budgets remain conservative, and side paths still
disable these proofs. Scratch accounting now uses the actual result element size.

Unit coverage includes Float64 cycles, mixed Float64/Int32 joins becoming Number,
unsigned-shift Number results, unanchored cycles, late unknown edges and budget
exhaustion. An experimental assertion banning every constant-true machine branch
was rejected: mixed-loop and update fixtures also contain independent boolean
and ownership guard branches. Those dumps are diagnostic evidence, not a passing
code-generation acceptance test. Full runtime validation passed (558 tests, one existing ignored), Clippy with
warnings denied passed, and independent static review found no issue. Fresh
performance evidence is pending; the frozen README matrix does not measure this
candidate.

## Focused numeric-facts diagnostic, 2026-09-10

All 720 process records passed collector/summarizer validation. Eight scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior entry facts ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior entry facts speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| call-heavy | 0.309959 | 0.310833 | 0.975105 | 0.012604 | 0.313688 | 0.9892x [0.9668, 1.0100]; statistically tied (3.32% slower to 1.00% faster) | 0.9891x [0.9603, 1.0112]; statistically tied (3.97% slower to 1.12% faster) | 3.1141x [3.0158, 3.2049] | 0.0407x [0.0388, 0.0427] |
| generic-call-entry | 0.244000 | 0.252500 | 0.800000 | 0.005875 | 0.247646 | 0.9823x [0.9640, 0.9977] | 1.0144x [0.9794, 1.0636]; statistically tied (2.06% slower to 6.36% faster) | 3.2222x [3.1288, 3.2985] | 0.0235x [0.0229, 0.0240] |
| generic-call-fallback | 2.996520 | 2.991250 | 1.096958 | 0.012209 | 2.991521 | 1.0207x [0.9836, 1.0596]; statistically tied (1.64% slower to 5.96% faster) | 1.0816x [1.0022, 1.2249] | 0.3703x [0.3605, 0.3814] | 0.0040x [0.0039, 0.0041] |
| adversarial | 0.664021 | 0.669667 | 0.676104 | 0.123958 | 0.670520 | 1.0642x [0.9840, 1.1880]; statistically tied (1.60% slower to 18.80% faster) | 1.0330x [0.9900, 1.0797]; statistically tied (1.00% slower to 7.97% faster) | 1.0158x [0.9976, 1.0328]; statistically tied (0.24% slower to 3.28% faster) | 0.1916x [0.1778, 0.2071] |
| host-compute | 0.059770 | 0.058521 | 1.238979 | 0.219792 | 0.059333 | 0.9827x [0.9312, 1.0270]; statistically tied (6.88% slower to 2.70% faster) | 1.0301x [0.9303, 1.1845]; statistically tied (6.97% slower to 18.45% faster) | 20.4741x [19.2548, 21.6257] | 3.6541x [3.2363, 4.1091] |
| numeric | 0.023708 | 0.023604 | 0.388647 | 0.114813 | 0.023750 | 1.0054x [0.9565, 1.0528]; statistically tied (4.35% slower to 5.28% faster) | 0.9994x [0.9528, 1.0549]; statistically tied (4.72% slower to 5.49% faster) | 16.8246x [15.6448, 18.5148] | 5.3109x [4.5398, 6.7814] |
| scalar-loop | 0.023625 | 0.024334 | 0.393000 | 0.109834 | 0.024001 | 0.9705x [0.9259, 1.0110]; statistically tied (7.41% slower to 1.10% faster) | 1.0942x [0.9556, 1.3339]; statistically tied (4.44% slower to 33.39% faster) | 15.8597x [15.2188, 16.4256] | 4.5695x [4.3444, 4.8706] |
| scalar-control-flow | 0.047730 | 0.047312 | 1.160813 | 0.079021 | 0.047271 | 0.9773x [0.9025, 1.0299]; statistically tied (9.75% slower to 2.99% faster) | 0.9664x [0.9068, 1.0020]; statistically tied (9.32% slower to 0.20% faster) | 25.1876x [24.4534, 26.4803] | 1.6321x [1.5103, 1.7500] |

Native entry ranges and compilation overlap are retained per scenario in the
summary JSON. This focused diagnostic does not close the full no-regression
gate. Full architecture and performance acceptance remain pending.

Evidence: `benchmarks/results/semantic-number-facts-diagnostic-arm64.tar.gz`, summary JSON and manifest.

Numeric-facts diagnostic assessment: generic-call-entry is below the pre-SSA
baseline at 0.9823x speed [0.9640, 0.9977]. The other seven scenarios are
statistically tied. All candidate batches had quiet compilation; native entry
counts were ten except fallback (20,000) and adversarial (zero). This result
does not establish no-regression. The full matrix remains pending.


### Semantic call operands (work in progress)

Generic and method calls now produce a semantic Call value with target, optional
receiver, argument ValueIds and the exact pre-call FrameState identity. Existing
production lowering consumes the graph's argument count and receiver and checks
the complete operand sequence against the recovery state. The shared frame
binding restoration routine writes compiler variables, not interpreter memory.
The existing call bridge still governs ownership and exceptions; reentrant
frame invalidation and unknown call-result representation are preserved.
Call argument storage is charged and bounded by captured-state limits.

The new test failed on the former Opaque call result, then passed with explicit
operand identities for arities zero through four and method receivers. Full
runtime tests passed (559, one existing ignored), Clippy passed, and independent
static review found no issue. This is infrastructure for graph-builder inlining,
not completed inlining or a performance acceptance result. The caller still
lacks a retained callee bytecode body; safe retention must account for both
artifact and queued/background request budgets and version identity.

## Focused semantic-call diagnostic, 2026-09-10

All 720 process records passed collector/summarizer validation. Eight scenarios,
six configurations, five discarded and ten retained fresh-process pairs use
the same fixed-warmup protocol as the full matrix. This smaller diagnostic
is not a replacement for the pending full matrix. All timings below are median
ms per ten calls; ratios are paired geometric speed with 95% bootstrap intervals.

| Scenario | Pre-SSA ms | Prior numeric facts ms | QuickJS ms | Bun ms | New JIT ms | New / pre-SSA speed | New / prior numeric facts speed | New / QuickJS speed | New / Bun speed |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| call-heavy | 0.319916 | 0.333438 | 1.014812 | 0.012833 | 0.320479 | 1.1948x [1.0316, 1.4264] | 1.0463x [0.9705, 1.1245]; statistically tied (2.95% slower to 12.45% faster) | 3.1111x [2.9146, 3.2615] | 0.0416x [0.0374, 0.0463] |
| generic-call-entry | 0.246167 | 0.252750 | 0.804312 | 0.006042 | 0.250271 | 1.0035x [0.9587, 1.0727]; statistically tied (4.13% slower to 7.27% faster) | 1.0099x [0.9866, 1.0409]; statistically tied (1.34% slower to 4.09% faster) | 3.2565x [3.1721, 3.3550] | 0.0238x [0.0233, 0.0243] |
| generic-call-fallback | 3.063458 | 2.974771 | 1.123646 | 0.012083 | 3.028042 | 0.9989x [0.9134, 1.0923]; statistically tied (8.66% slower to 9.23% faster) | 0.9422x [0.8791, 0.9984] | 0.3586x [0.3315, 0.3860] | 0.0041x [0.0033, 0.0053] |
| adversarial | 0.669771 | 0.669813 | 0.682604 | 0.153021 | 0.672312 | 1.0624x [0.8007, 1.5698]; statistically tied (19.93% slower to 56.98% faster) | 1.1497x [0.8712, 1.6274]; statistically tied (12.88% slower to 62.74% faster) | 1.1160x [0.8700, 1.4807]; statistically tied (13.00% slower to 48.07% faster) | 0.2246x [0.1692, 0.3017] |
| host-compute | 0.072750 | 0.060791 | 1.250417 | 0.271083 | 0.060896 | 1.1245x [0.9054, 1.3806]; statistically tied (9.46% slower to 38.06% faster) | 0.9666x [0.7965, 1.1313]; statistically tied (20.35% slower to 13.13% faster) | 18.9283x [15.8010, 22.0319] | 3.9613x [3.1766, 4.8185] |
| numeric | 0.024313 | 0.025771 | 0.485333 | 0.142687 | 0.024501 | 1.1923x [0.9832, 1.4835]; statistically tied (1.68% slower to 48.35% faster) | 1.0801x [0.9625, 1.2347]; statistically tied (3.75% slower to 23.47% faster) | 18.8448x [16.7836, 21.1382] | 6.6619x [5.4193, 8.3478] |
| scalar-loop | 0.024355 | 0.025021 | 0.400958 | 0.121188 | 0.024354 | 0.7473x [0.4129, 1.0392]; statistically tied (58.71% slower to 3.92% faster) | 0.9096x [0.6122, 1.2130]; statistically tied (38.78% slower to 21.30% faster) | 12.4501x [6.7421, 17.8411] | 4.2014x [2.1095, 7.0227] |
| scalar-control-flow | 0.050021 | 0.051333 | 1.340021 | 0.113167 | 0.050417 | 0.9140x [0.7604, 1.0363]; statistically tied (23.96% slower to 3.63% faster) | 1.1043x [0.9817, 1.2916]; statistically tied (1.83% slower to 29.16% faster) | 24.2610x [19.7543, 28.3559] | 2.0921x [1.5272, 2.8860] |

Native entry ranges and compilation overlap are retained per scenario in the
summary JSON. This focused diagnostic does not close the full no-regression
gate. Full architecture and performance acceptance remain pending.

Evidence: `benchmarks/results/semantic-call-ir-diagnostic-arm64.tar.gz`, summary JSON and manifest.

Semantic-call diagnostic assessment: call-heavy measured 1.1948x pre-SSA speed
[1.0316, 1.4264]; the other seven scenarios were statistically tied, with broad
intervals. Numeric, scalar-loop and scalar-control-flow each had only eight of
ten compilation-quiet candidate samples; the other scenarios had ten. All
samples remain in the evidence. These wide intervals and compilation overlap
do not establish stable gains or satisfy the full no-regression gate. Native
entry ranges remained ten, except fallback (20,000) and adversarial (zero).
The collector, summarizer and archive verification all completed successfully.


### Retained callee snapshots for graph construction (work in progress)

Compiled artifacts may retain a worker-safe CompileSnapshot for a small callee:
up to 128 instructions and 16 KiB retained storage, no exception map, and exact
function/generation/source/opcode identity. This does not declare the body
inlineable. Retained-byte accounting includes vector capacities, SnapshotData
and Arc counters, constants, source/exception maps, and verifier metadata.
The artifact metadata charge includes this storage conservatively even if shared.

DirectCallTarget carries the originating artifact key and an optional snapshot;
queued caller requests retain at most 64 KiB of callee snapshot storage. The
background snapshot/IR admission and release charges include retained bodies.
If optional bodies exceed a configured background budget, admission drops them
while preserving direct publications, then retries the ordinary caller charge.
This avoids turning an optional inline opportunity into a compilation rejection.

Tests cover retained capacity/metadata, size and instruction limits, exact source
identity, artifact retirement, original-runtime teardown, request budget charge,
and reduced-budget dispatch/release. The constrained request test failed before
optional-body removal and passed afterward. Independent review found no issue.
Full runtime validation passed (564 tests, one existing ignored), and Clippy with
warnings denied passed. No fresh performance acceptance is claimed for this
retention candidate; the README matrix predates it.

Next: graph-builder inlining must consume these copied bodies under the caller's
SSA context and checked artifact identity, preserve invalidation dependencies,
and provide exact inline recovery states. Current machine lowering still emits
ordinary direct/generic calls. Bounded monomorphic inlining and its >=0.5x Bun
gate, generic fallback profitability, property/array phases and full matrix
acceptance remain incomplete.

### First effect-free inline regions (macOS work in progress)

Graph construction now consumes retained callee bodies in the caller's ValueId
namespace. Admission permits bounded forward paths with Int32 add/subtract,
scalar constants and arguments; literal caller booleans select known branches.
Unknown branches, receiver calls, mutation and runtime operations keep ordinary
calls. Each checked arithmetic step records its callee PC, operand state and
parent caller call-site state. Target identity and argument tags are checked
before executing the expanded path. Successful inline paths have no callee ABI
call or helper call; cold recovery may invoke ownership helpers.

Recovery currently replays the original, effect-free call from the exact caller
pre-call state on identity, type or overflow failure. These diagnostic parent
states are **not general inline-frame reconstruction**. Nested/effectful inlining
must wait for that recovery mechanism. Loop target-guard hoisting, removal of
per-iteration frame synchronization, general call fallback profitability and the
>=0.5x Bun call gate remain unfinished.

Optional expansion falls back to ordinary graph construction on graph resource
limits. Controlled compilation also retries after retained-SSA or Cranelift IR
budget failures, preserving the same cancellation/deadline. Temporary verified
callee bodies are released after construction. A production test with a
20-add callee verifies that the smallest affordable controlled compilation uses
ordinary calls. Artifact `inlined_calls` counts static expanded sites, not
runtime executions. Independent static review found no concrete issue.

macOS release runtime validation passed: 565 tests, zero failures, one existing
ignored test. This includes native target/type/overflow recovery and production
retained-snapshot consumption. Clippy with warnings denied passed. The completed
performance evidence and updated root README matrix are described below.

### Frozen inline-region performance evidence

The full 27-scenario matrix is now the primary root README matrix. It retains
5,670 process records (six configurations, five discarded and 30 retained
processes each) plus the earlier two-call diagnostic. The archive verifies
6,037 files, including current and previous-retention source trees. Candidate
binary SHA256: `30dd7e0ce72e05d2017f6f32335b0b17d8ee9c80715e40198d50e7856dce3682`.

- `call-heavy`: previous-retention speed 1.095152x [1.045685, 1.135405]; 023a220 speed 1.096311x [1.034672, 1.159282]; QuickJS speed 3.378712x; Bun speed 0.043820x. Full CIs and absolute times are in README and the archived JSON.
- `generic-call-entry`: previous-retention speed 1.171867x [1.101183, 1.235798]; 023a220 speed 1.218149x [1.134256, 1.323468]; QuickJS speed 3.795174x; Bun speed 0.027478x. Full CIs and absolute times are in README and the archived JSON.
- `host-compute`: previous-retention speed 0.952072x [0.895147, 0.993705]; 023a220 speed 1.046963x [0.956542, 1.158785]; QuickJS speed 20.892237x; Bun speed 3.245231x. Full CIs and absolute times are in README and the archived JSON.
- `strings-json`: previous-retention speed 0.984647x [0.970950, 0.996030]; 023a220 speed 1.014276x [0.989889, 1.045035]; QuickJS speed 1.102550x; Bun speed 0.080385x. Full CIs and absolute times are in README and the archived JSON.
- `arrays-typed`: previous-retention speed 1.003335x [0.989947, 1.016476]; 023a220 speed 0.964931x [0.954457, 0.975032]; QuickJS speed 1.754206x; Bun speed 0.072994x. Full CIs and absolute times are in README and the archived JSON.
- `generic-call-fallback`: previous-retention speed 0.972374x [0.894561, 1.073133]; 023a220 speed 1.071308x [0.961468, 1.203760]; QuickJS speed 0.351434x; Bun speed 0.003678x. Full CIs and absolute times are in README and the archived JSON.

Intervals crossing 1.00x are statistically tied. The no-regression gate is
still unmet: arrays-typed is slower than 023a220, and host-compute and
strings-json are slower than the previous retention candidate. Strings-json
has zero native entries; that observation does not implicate machine code.
Numeric and scalar-loop have 19/30 and 22/30 compilation-quiet samples; all
samples remain included. All other candidate configurations are quiet in
30/30 samples. General inline recovery, target-guard LICM, lazy frame state,
generic-call profitability and property/array architecture remain unfinished.
