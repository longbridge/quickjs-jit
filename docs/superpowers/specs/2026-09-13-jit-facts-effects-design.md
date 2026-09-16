# V8/JSC-derived optimizing JIT completion

The user approved implementation of all five directions on 2026-09-13.
This specification preserves that full scope, including performance evidence.
The implementation stays in the current checkout as requested by AGENTS.md.

## Architecture and references

Keep QuickJS tagged values and ownership at runtime boundaries, CFG semantic
SSA in the optimizing frontend, and Cranelift as the machine backend.
Use Maglev's known-node information and representation selection to carry facts
through SSA joins. Use JSC's abstract heaps and clobberize discipline to describe
reads, writes and invalidation, and its LICM safety conditions to move checks.
Neither bytecode-pattern special cases nor disabling optimization to improve a
benchmark replaces the requested general analysis.

Reference implementations and explanations:

- https://v8.dev/blog/maglev (Known Node Information, representation, deopt)
- https://v8.dev/blog/leaving-the-sea-of-nodes (CFG architecture)
- https://webkit.org/blog/10308/speculation-in-javascriptcore/ (abstract
  interpretation, clobberize, inline caches and OSR exits)
- https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/dfg/DFGClobberize.h
- https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/dfg/DFGLICMPhase.cpp
- https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/dfg/DFGIntegerRangeOptimizationPhase.cpp
- https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/dfg/DFGByteCodeParser.cpp

## Required deliverables

1. Explicit property/element semantics and a bounded, flow-sensitive known-facts
   analysis. Model distinct heap locations, alias invalidation, frame writes,
   unknown calls and observable safepoints. Merge only facts valid on all
   incoming paths. Unknown or exhausted proofs retain existing checks.
2. General loop analysis and call-target guard LICM. A guard moved to a loop
   entry must dominate its uses and have a valid recovery state. Preserve zero
   iteration behavior, nested-loop and branch behavior, and invalidation across
   polls. Existing pure inlining remains the first consumer, not the only one.
3. Property check deduplication, load/store forwarding and store sinking.
   Stable data-property loops carry numeric fields in SSA. Dirty fields must be
   written before any boundary that can observe them and on normal exits.
   Aliases, getters/setters, shape transitions, Proxies, exceptions, GC and
   reentrant calls preserve QuickJS semantics and reference ownership.
4. Feedback-driven array modes, loop-invariant buffer/data/length checks and
   range-based bounds elimination. Prove nonnegative induction and upper bounds
   before deleting checks; handle overflow, empty inputs, wrong classes,
   out-of-bounds accesses, holes and detached/resizable buffers conservatively.
   Add preallocated traversal diagnostics without replacing arrays-typed.
5. Exact inline-frame recovery before effectful/general inlining. Resume the
   correct callee bytecode position and caller continuation without replaying
   completed effects. Improve stable call-site dispatch and the actual generic
   bridge; retain a representative non-inlined fallback benchmark.

## Validation and acceptance

Every production change needs failing-first semantic or generated-code tests,
then relevant native execution, deopt, ownership and resource-limit tests.
Tests inspect emitted control flow and operations, not only optimization counts.
Compilation remains cancellable, bounded, and fail-closed under budget pressure.

Before declaring completion run the full release runtime suite with
compiler,test-support, Clippy with warnings denied, format/diff checks, relevant
Test262/differential tests, and native sanitizer/CI coverage for changed runtime
boundaries. Record actual executed commands and their scope.

Freeze e174b52 and its build inputs as the before baseline. Compare final
automatic JIT with that baseline, same-version detached QuickJS, and default
Bun, using equivalent inputs, warmup, timed boundaries and result consumption.
Refresh all README scenarios, including slower/unsupported/fallback rows, with
absolute times, speed ratios and confidence intervals. Retain paired evidence,
revisions, hashes, raw samples, toolchains, host and flags. Use at least five
discarded and thirty retained processes per configuration for the final matrix.
No Linux timing is inferred from the historical Apple M3 archive.

Retain the existing 0.5x Bun gates for call-entry/property/arrays and the
same-version QuickJS profitability gate for generic fallback. Also report
startup, compilation, memory, P99, throughput and actual gpui-shell regression
controls separately from equivalent pure-JS Bun kernels. Historical regressions
and incomplete gates remain explicit; partial infrastructure is not completion.
