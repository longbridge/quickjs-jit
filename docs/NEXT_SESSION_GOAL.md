# QuickJS JIT implementation goal

## Objective

Implement a production-quality JIT for QuickJS in the independent
`rquickjs-jit` crate. JavaScript semantics must remain identical to the
interpreter, and unsupported or unstable code must fall back automatically.

The primary remaining objective is runtime-feedback-driven speculative type
specialization. Generic helper-based native execution is not sufficient.

## Required architecture

```text
runtime execution
    -> collect argument, return, operation, call-target, and shape feedback
    -> identify stable monomorphic or bounded-polymorphic behavior
    -> compile a bounded specialized function version
    -> guard assumptions at entry or the earliest dominating point
    -> keep specialized SSA values unboxed
    -> execute direct machine instructions
    -> deoptimize exactly when an assumption fails
    -> widen feedback, compile another version, or stop optimizing
```

Keep QuickJS's tagged `JSValue` representation at interpreter, heap, helper,
and public API boundaries. Inside optimized native code, use raw `Int32`,
`Float64`, and other proven representations in registers and SSA values.

## First mandatory end-to-end case

```js
function add(a, b) {
    return a + b;
}
```

When runtime feedback repeatedly observes `(Int32, Int32) -> Int32`, the JIT
must generate a specialized version with this conceptual hot path:

```asm
guard_int32 a, deopt
guard_int32 b, deopt
add result, a, b
overflow deopt
return_int32 result
```

The hot path must contain a native integer add and must not call the generic
add helper. Arguments are checked and unboxed once, not at every arithmetic
operation. Boxing is permitted only at a required boundary.

If either argument changes type, the result overflows, or JavaScript semantics
require a different path, execution must reconstruct the exact interpreter
state and continue correctly.

## Implementation stages

### 1. Complete feedback model

Collect and retain feedback for:

- every actual function argument;
- every return site and returned value;
- operands and results of arithmetic, comparison, conversion, and branch
  operations;
- call target identity and argument/return signature;
- object shape, prototype dependency, and property location;
- guard failures, overflow, exceptions, and deoptimization reasons.

Feedback belongs to an exact function identity and generation. Its lattice
widens monotonically from unseen to monomorphic, bounded polymorphic, and
generic or megamorphic.

### 2. Bounded specialized function versions

Key optimized code by a bounded signature containing the function generation,
arity, argument representations, and feedback epoch.

Initially support:

- `(Int32...) -> Int32`;
- `(Float64...) -> Float64`;
- generic fallback.

Bound the number of versions, compilation attempts, and deoptimization retries.
Repeated instability must widen or stop optimization rather than creating a
compile/deopt loop.

### 3. Real representation-aware SSA

Make representation an SSA property:

- `Tagged`;
- `Int32`;
- `Float64`;
- `Bool`;
- later, proven heap references and shapes.

Implement explicit guard, unbox, box, and conversion nodes. Propagate known
representations through locals, operand-stack values, branches, and Phi nodes.
Eliminate redundant guards and conversions through dominance and known-value
facts.

### 4. Numeric lowering

Lower stable numeric operations directly:

- Int32 add/subtract/multiply with overflow deoptimization;
- Float64 arithmetic with exact NaN and infinity behavior;
- comparisons and truthiness without generic helpers when types are proven;
- division with explicit zero, negative-zero, NaN, and infinity handling.

Keep loop induction variables and accumulators unboxed across iterations.
Hoist invariant parameter guards outside loops. Add range analysis only after
the guarded Int32 path is correct; remove overflow or bounds checks only when a
proof exists.

### 5. Exact deoptimization

Every speculative exit must record:

- bytecode PC and side-effect phase;
- arguments, locals, operand stack, and exception state;
- the physical register or stack location of every live value;
- each value's native representation and materialization recipe.

Deoptimization must reconstruct an ordinary QuickJS interpreter frame without
creating a JavaScript exception. It must preserve ownership, reference counts,
GC roots, observable coercions, and side-effect ordering.

### 6. Calls and return specialization

Use return feedback in specialized callers. Add monomorphic call-target guards
and a compiled-to-compiled call path that passes compatible Int32 or Float64
values without boxing. Target or signature mismatch deoptimizes or uses the
generic call path.

### 7. Object shape specialization

After numeric and call specialization is correct and profitable, implement:

- stable shape and prototype dependency tokens;
- monomorphic shape guard plus fixed-offset property load/store;
- bounded polymorphic inline caches;
- megamorphic generic fallback;
- invalidation on shape, prototype, class, or property-layout changes.

Accessors, Proxy objects, deletion, redefinition, and observable prototype
effects must retain exact interpreter behavior.

### 8. Native execution overhead

Keep JIT-disabled and no-code execution close to interpreter cost. Avoid a
C/Rust callback on every loop iteration. Amortize hotness and poll checks.

Once specialized code is installed, avoid per-call allocation, full-frame
validation, full-frame materialization, and profiling bookkeeping on the hot
path. Synchronize tagged state only at helpers, GC safepoints, calls, stores,
returns, and deoptimization points.

## Correctness requirements

Test specialization with:

- stable Int32, Float64, String, BigInt, and alternating argument types;
- actual argument count different from declared parameter count;
- multiple return sites and thrown exits;
- Int32 minimum, maximum, and overflow;
- negative zero, NaN, infinities, and division by zero;
- conversion side effects and throwing coercions;
- objects, accessors, Proxy, Symbol, and BigInt mixing;
- branches, loops, Phi nodes, nested calls, recursion, async re-entry, and GC;
- eager deoptimization at every guarded phase;
- hot reload and function-generation retirement;
- randomized differential execution against the interpreter;
- the applicable Test262 corpus.

Automatic fallback must always return the same value, exception, side effects,
and ownership behavior as the interpreter.

## Required machine-code evidence

Timing alone is not proof of specialization. Tests must inspect Cranelift IR or
disassembly and prove that:

- the Int32 `add` hot path contains a native integer add;
- parameter guards dominate uses and are outside the hot loop;
- the loop body does not call generic add, compare, truthiness, duplicate, or
  free helpers;
- loop-carried Int32 Phi values remain unboxed;
- boxing occurs only at a required boundary;
- overflow and wrong-type paths reach exact deoptimization metadata.

## Performance requirements

Measure native-entry overhead separately from the optimized body. First prove
that a long batched `add` workload and an Int32 numeric loop are faster than the
interpreter. Record checksums, native entries, helper counts, fallback counts,
deoptimizations, and generated-code evidence.

After the focused path is profitable, run the complete benchmark matrix with
paired fresh processes and confidence intervals. Report startup, compilation,
installation, steady state, throughput, tail latency, memory, and code size.

The acceptance targets remain:

- at least 10x for a representative hot compute kernel;
- at least 2x for suitable `gpui-shell` steady-state JavaScript workloads;
- no material regression in startup, reload, or tail latency;
- exact JavaScript semantics with automatic fallback.

## Platform and integration requirements

- Native JIT: macOS, Windows, and Linux on x86-64 and AArch64.
- WebAssembly: interpreter only; no JIT or Cranelift dependency.
- Keep the implementation primarily in the independent `rquickjs-jit` crate.
- Keep the QuickJS integration ABI small, versioned, fail-closed, and easy to
  rebase when QuickJS changes.
- Integrate with `gpui-shell` and verify rendering, events, async continuations,
  reload, GC, and runtime teardown.

## Completion definition

The implementation is complete only when speculative argument and return type
specialization, unboxed numeric SSA, exact deoptimization, bounded versioning,
automatic fallback, cross-platform behavior, general JavaScript correctness,
`gpui-shell` integration, and reproducible performance evidence all pass
independent review.

## Post-merge optimization backlog

The initial PR deliberately remains fail-closed for JavaScript surfaces that
have not yet received exact native semantics. Merge readiness does not imply
that Tier 1 or the full benchmark matrix is complete. Follow-up work should be
prioritized in this order:

1. Extend Tier 1 reachability for strings and JSON, beginning with
   `push_atom_value`, then the remaining string/property construction
   operations. Require native-entry evidence and GC/exception differential
   tests before publishing performance numbers.
2. Complete array and typed-array loop optimization. Hoist stable class,
   shape, bounds, and detached-buffer guards where valid, and add Tier 2
   element-loop SSA so the current exact-but-slower guarded baseline path is
   profitable.
3. Add closure and captured-variable support (`fclosure` and the var-ref
   family), followed by multi-level calls and bounded recursion. Preserve
   var-ref lifetime, reference counts, reload generation isolation, and GC
   roots.
4. Extend collections and integer coverage: Map/Set iteration and mutation,
   iterator opcodes, `push_i16`, `push_bigint_i32`, and BigInt arithmetic.
   BigInt remains helper-backed unless a representation proof justifies a
   specialized path.
5. Design and implement an interpreter-compatible exception and continuation
   ABI before admitting protected regions, Promise jobs, generators, or async
   functions to native execution. Throw/catch state, pending exceptions,
   suspension, resume PCs, ownership, and job ordering must remain exact.
6. Re-run the complete benchmark matrix against QuickJS and the pinned Bun
   version with paired fresh processes. Cover Float64 computation,
   strings/RegExp, arrays/growth/typed arrays, object shapes, calls/recursion/
   closures, JSON, Map/Set/BigInt, and exceptions/Promise/async. Keep checksum,
   native-entry, fallback, deoptimization, memory, startup, and tail-latency
   evidence in the tracked report.
7. Investigate V8 and JavaScriptCore/Bun implementation techniques only as
   design references. Any adopted technique still requires a QuickJS-specific
   ownership, exception, GC, invalidation, and deoptimization proof.

Known current limitations must remain visible in reports: focused numeric and
Fibonacci kernels are profitable, while broad strings/JSON/collections/
closures/async workers may still be interpreter-only, and the guarded Tier 1
array traversal is not yet a demonstrated speedup.

After M2 (`docs/M2.md`), the core comparison/bitwise/`%`/unary/tail-call
opcode set is native in both tiers. The next backlog items in priority order
are: caching native entry handles on the C side so generic native-to-native
calls stop paying per-call Rust callbacks; closures (`fclosure`, var-refs);
`typeof`; `for-of`/iterator opcodes; exception regions and async.

The `generic-call-entry` benchmark now isolates the first item with a short
`(Int32, Bool)` callee, avoiding the numeric direct-call specialization in
`call-heavy`. Its Tier 1 integration test proves 1,000 generic `CALL` helper
invocations and at least 1,001 native entries for a 1,000-iteration batch.
The four-mode smoke run agrees on checksums; forced Tier 2 currently executes
Tier 1 for this probe. The in-progress C entry cache is described in `docs/M3.md`.
Its production test also requires fewer than 100 handle acquisitions per batch.
The paired diagnostic run records 1.144x the M2 binary's speed in Tier 1 and
1.228x in automatic mode; see `benchmarks/results/m3-entry-cache-paired.json`.
The pinned follow-up below resolves the focused interpreter variance and
completes the three control workloads; clean-source acceptance remains open.

The next incremental change removes per-native-return queue allocation using a
single synchronous handoff slot. It passes 509 release runtime tests and 39
AddressSanitizer tests. The pinned three-version generic-call comparison records
1.319x M2's speed in automatic mode (1.076x the cache-only version); the focused
interpreter interval is now narrow and tied against cache-only. See
`benchmarks/results/m3-native-return-generic-paired.json` and `docs/M3.md`.
All control measurements are complete and archived under
`benchmarks/results/m3-native-return-*-paired.json`; no benchmark process remains
running. Direct-call modes are statistically tied with M2. Recursive Tier 1 is
1.112x M2 speed; automatic recursion has zero native entries and is only an
interpreter-path control. Scalar-loop automatic mode is 1.037x M2 speed. No
material steady-state regression appeared in these controls. Startup, reload,
tail latency, and clean-source acceptance remain open. CALL feedback now reuses its argument/result buffer while preserving arbitrary
arity, exact order, and rejection of an entire invalid type event. Both warmed
CALL event forms allocate zero times in the regression test. The final code
passes 511 release tests, 39 AddressSanitizer tests, and Clippy. Earlier paired
reports are explicitly tied to the pre-buffer executable hash.

Entry caching must preserve the existing `Coordinator::pin` admission checks:
installed artifact identity, invalidation, and the optimizing feedback epoch.
An `ExecutionPin` alone proves code lifetime, not permission for a new entry.
Also account for `prepare_baseline_direct_refresh`, optimized demotion,
generation retirement, suspension, and runtime detach. Reentrant active frames
must retain their code and metadata even after future entries are invalidated.
Cache hits now reuse a `ProductionEntryPin`; per-call hot/enter/exit callbacks
remain in place. Do not treat this first cache step as completion of the whole
native-call overhead target.


PR #18 is open on `perf/m3-native-entry-overhead`. Final CALL-buffer diagnostics
are archived in `benchmarks/results/m3-call-feedback-generic-paired.json`:
automatic mode is 1.359x M2 speed, but statistically tied with the pre-buffer
version. Coverage CI exposed a bundled/fresh bindgen formatting mismatch,
reproduced locally and corrected across all ten bundles. Follow the current PR
checks and run the clean-source benchmark matrix before broader acceptance.


The clean-source 22-workload/four-mode matrix is now complete and archived in
`benchmarks/results/m3-clean-matrix.json` and `.md`, with the binary hash and
CPU affinity in `m3-clean-matrix-launch.json`. No benchmark process remains
running. All 2,640 retained samples agree on checksums and balance entries/exits.
The formal performance gate fails: compute geomean is 3.17x–3.23x interpreter
speed (target 5x); startup/reload regress; real gpui-shell evidence is absent.
The 10x-kernel, native-tier, profitability-policy and checksum gates pass.
Generic-call automatic mode is still about 0.12x interpreter speed, so remaining
per-call callback/validation/profiling cost remains the active overhead target.
PR #18's latest observed CI has 40 successful checks and sanitizer still active;
re-query its current head and job status before making any CI completion claim.
