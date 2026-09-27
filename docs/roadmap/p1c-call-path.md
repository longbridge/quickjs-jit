# P1 item 4: callback-free interpreter-to-native calls (first slice)

Branch `perf/p1c-call-path`, based on `82d3808`. Roadmap context:
[PERFORMANCE_ROADMAP_V8_BUN.md](../PERFORMANCE_ROADMAP_V8_BUN.md) B2 and P1.4.

## What changed

- **QuickJS patch 0023, ABI 1.25.** The backend vtable gains the optional
  `entry_fast_grant` callback. After a DONE exit whose pc=0 handle QuickJS
  caches, the backend may grant a bounded budget plus a backend-owned
  `JSJitFastEntryState`. While the budget lasts and `state->epoch` still
  equals the cached entry epoch, calls of that function reuse the cached
  handle and skip call feedback, `record_hot`, `entry_cache_epoch`,
  `native_enter` and `native_exit`. QuickJS counts these executions in the
  shared state and sets `JS_JIT_FRAME_FAST_ENTRY` on their frames. A
  non-DONE exit still calls `native_exit`, tagged
  `JS_JIT_EXIT_FAST_UNPAIRED`, and is never re-cached. C-to-JS callbacks
  (`Array.prototype.sort` comparators, `forEach`/`map` callbacks) use the
  same path because they enter through `JS_CallInternal`.
- **The idle cache is no longer evicted by unrelated calls.** A call of a
  different function that never reaches native code keeps the other
  function's still-admissible handle. A native call replaces it after
  returning, as before.
- **Production grant policy.** `ProductionBackend::entry_fast_grant` grants
  255 calls only to steady entries: an installed Tier 2 artifact whose
  profitability trial is decided (or cannot run without baseline samples),
  or an installed baseline whose optimizing tier is terminally blacklisted
  (not under `BaselineOnly`, which probes on every hot event). Each renewal
  is a full call, so hot counts, call/return feedback, timing, benefit
  accounting and `maintenance_if_due` are sampled once per 256 calls instead
  of paid on every call. The shared epoch mirrors `entry_cache_epoch`, so
  every existing invalidation (maintenance, deopt, demotion, direct refresh,
  detach) also stops granted calls before the next execution.
- **Exact counters.** `Jit::metrics` adds the shared entries, exits and
  optimized entries, so `native_entries == native_exits` and `tier2_entries`
  stay exact. `native_acquisitions` and `benefit_recordings` now count only
  full calls; that is the intended semantic change (acquisitions and timing
  are what the fast path removes).
- **Tier 2 candidate scan cache.** Maintenance remembers candidates it
  deferred for missing property feedback, an unresolved callee, or an
  incomplete numeric/element signature, keyed by the inputs of that
  decision (feedback version, a new shape-feedback version, installed
  artifact count, element-loop sample gate). It skips rebuilding the
  feedback snapshot for them until an input changes. The shape-feedback
  version ignores repeated observations and reordering.

## Verification

- `jit/tests/fast_entry.rs`: C ABI tests for budget expiry, epoch change,
  zero epoch, unpaired non-DONE exits, suspension, malformed grants and
  cache retention across interpreted calls; plus a production test where a
  steady Tier 2 sort comparator runs granted with exact metrics and still
  deoptimizes correctly on string scores.
- `jit_patch` covers the 1.25 header, source and bundled bindings for all
  JIT targets (bindings updated by hand, including wasm32 layouts).

## Noisy diagnostic (not publishable)

Seven alternating processes per engine on a shared, heavily loaded host;
median over runs of the median of the last 16 warmup batches; checksums
matched the interpreter. Speed is baseline time divided by new time.

| Workload | 82d3808 ms | branch ms | speed |
| --- | ---: | ---: | ---: |
| mixed-quotes | 2.5036 | 0.6426 | 3.90x |
| generic-call-entry | 0.0709 | 0.0688 | 1.03x |
| call-heavy | 0.1057 | 0.1041 | 1.02x |
| generic-call-fallback | 9.1043 | 8.9973 | 1.01x |
| calls-closures | 3.3230 | 3.3293 | 1.00x |
| fibonacci-recursive | 9.8675 | 9.9872 | 0.99x |
| calls-recursion-closures | 6.4353 | 6.4962 | 0.99x |
| objects-polymorphic | 23.8856 | 23.8650 | 1.00x |
| property-heavy | 0.0989 | 0.0970 | 1.02x |
| numeric | 0.0294 | 0.0300 | 0.98x |
| arrays-typed | 2.9859 | 3.0208 | 0.99x |
| collections | 1.6589 | 1.6539 | 1.00x |

Only mixed-quotes moves outside the noise band. `generic-call-entry` and
`call-heavy` perform ten native entries per batch (their hot calls are
direct or inlined), and the closure/recursion scenarios never compile, so
they have no steady native entries to grant.

## Remaining work

- Grants still require one full call per 256 executions and per maintenance
  epoch. A per-bytecode handle cache (instead of the single runtime slot)
  would let alternating native callees all stay granted; the slot is still
  replaced when two native functions alternate at the same depth.
- Baseline entries that are still Tier 2 candidates keep the full callback
  path, including per-call timing and a full maintenance pass about every
  32 calls. Moving hot counting into a C-side decrementing counter (V8's
  interrupt budget) would remove that too.
- Maintenance still invalidates the entry epoch on every full pass. Making
  invalidation conditional on an actual admissibility change needs an
  explicit coordinator state generation for installs, demotions, eviction
  and dependency invalidation.
- `calls-closures`, `calls-recursion-closures` and `fibonacci-recursive`
  need P2 coverage (closure opcodes) and P3 (native recursion) before this
  path can help them.
- The publishable three-engine matrix for this change is pending.
