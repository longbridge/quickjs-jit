# P4b: array bounds-check elimination and loop metadata (first slice)

Branch `perf/p4b-bce`, based on `82d3808`. Roadmap item: P4 bounds-check
elimination and length/data hoisting for packed Array, Int32Array and
Float64Array loops ([roadmap](../PERFORMANCE_ROADMAP_V8_BUN.md), section 6).

## What 82d3808 already had

`ArrayPlan` + `IntegerRangeAnalysis` already proved `0 <= i < length` for
canonical induction loops whose single `length` read is guarded in the
preheader (packed `logical_length == dense_count`, typed intrinsic-getter
query), deleted the per-access comparison, and revalidated data/count after
every poll. The remaining per-iteration cost in the target loops was not the
bounds compare itself but everything around it.

## What this slice changes (Tier 2 only, fail-closed)

1. **Literal-tag folding.** `opt_known_const` folds already-emitted literal
   IR (`iconst` through `icmp`/`band`/`bor`/extends). Guards whose condition
   folds to true emit no branch; `lt/lte/gt/gte` use an integer compare when
   both operand tags are literal Int32 (for example the hoisted exact length);
   `if_*` on a literal Bool/Int32 condition branches on the payload, or jumps
   directly when the truth itself is literal; `to_propkey` on a literal Int32
   key is the identity.
2. **Static storage kind.** Cached element sources record their kind when it
   is statically known; cached loads emit only that kind's load and skip the
   redundant receiver-tag check (the source already guarded that frame slot).
3. **Block sealing.** IR blocks whose predecessors are all lowered, guard
   diamonds and the amortized-poll join are sealed immediately, and literal
   tags are restated after the poll join (the cold path never redefines a
   frame variable), so literal definitions stay visible instead of hiding
   behind merge parameters. `opt_ir_edge` rejects any branch to an IR block
   that is not a declared IR successor, which keeps early sealing sound.
4. **Multiple live sources.** Guarded element metadata is tracked per frame
   slot provenance (`ElementSources`). Loads, cached lengths and typed stores
   keep the other sources; generic lengths, generic element stores, frame
   writes and every plan invalidation clear all of them.
5. **Metadata-only typed hoists.** A typed candidate accessed in a loop with no
   `length` read hoists its storage query to the preheader (`length: None`).
   Bounds checks stay; polls revalidate data/count exactly like length hoists.
   Packed candidates never get such a hoist (their guard would demand
   `length == dense_count`, which an element-only loop never needed).
6. **Hoisted sources across in-loop diamonds.** A block inherits a hoisted
   source only when every (already lowered) predecessor's exit state holds the
   identical source; the preheader dominates every such block. This carries
   metadata across the `a[i] = v` nullish-destination diamond.
7. **Numeric constants.** `push_const*` of a numeric constant-pool entry is
   lowered only as a literal, so it is non-reentrant for array plans and the
   amortized-poll proof, preserves frame identities, and seeds representation
   proofs (`proven_numeric_values_with_constants`). This is what admits
   `x * 0.25 + 0.5` loops to the array plan at all.
8. **Receiver tag at the header.** A hoisted argument receiver that no loop
   node rebinds has its object tag restated at the loop header.

## Tests

- `jit/tests/array_bce.rs`: two typed receivers in one loop (exact-length and
  metadata-only hoists, aliased views over one buffer, short destinations,
  wrong modes, zero-trip loops); a detach at an observable poll with a
  metadata-only hoist (the test segfaults if revalidation is removed);
  an argument rebound inside a loop that reads through a hoisted alias;
  literal folding with Float64 overflow and NaN.
- `array_cache` unit test: element-only typed loops hoist metadata but keep
  every bounds check; packed element-only loops do not hoist.
- `automatic_call_heavy_promotes_the_direct_edge_caller` now waits for the
  leaf's five clock-paced profitability retries before asserting their exact
  count. That race already failed on 82d3808 (3/60 local runs); the faster
  native caller loop made it more frequent (11/40) until de-raced. A second,
  pre-existing race remains in the same test: the post-warmup
  `tier2_entries == 11` assertion occasionally sees the leaf's generic entries
  (for example `6008`). With the de-raced test it fails at the same rate on
  both revisions (82d3808: 1/60, this branch: 2/60, alternating runs).

## New diagnostic scenario

`typed-convert-traversal` (arrays-typed group, diagnostics suite): the
`convertAndSum` kernel with both typed arrays passed as arguments to one
callee, so the float64 destination is a second live receiver with no length
read. It isolates the path that `arrays-typed` covers but buries under its
interpreter-only `push`/construction work. It needs a Bun row in the next
publishable matrix.

## Noisy diagnostics (not publishable)

Loaded 24-core host (16 agents), `taskset -c 0-15`, alternating baseline
(`82d3808` `target/release/jit-bench`) and branch binaries, median over runs
of the median of the last 16 warmup batches, µs per 10 calls, automatic mode.
Checksums match the interpreter for every run. Speed = baseline / branch.

| Workload | runs | 82d3808 | branch | speed |
| --- | ---: | ---: | ---: | ---: |
| int32array-traversal | 7 | 49.77 | 31.00 | 1.61x |
| packed-array-traversal | 7 | 48.15 | 31.09 | 1.55x |
| arrays-typed | 7 | 2973.81 | 1890.80 | 1.57x |
| float64array-traversal | 7 | 1091.95 | 1069.62 | 1.02x |
| typed-convert-traversal (new) | 7 | 1069.40 | 58.83 | 18.2x |
| call-heavy (control) | 3 | 60.24 | 43.87 | 1.37x |
| numeric (control) | 3 | 29.19 | 26.55 | 1.10x |
| scalar-loop (control) | 3 | 29.26 | 26.46 | 1.11x |
| property-heavy (control) | 3 | 98.21 | 100.22 | 0.98x |
| other scalar/call controls | 3 | — | — | 0.99x–1.02x |

`property-heavy` stays about 2% slower even with every new lowering change
disabled through temporary switches, so it is attributed to host-binary layout
noise, not to generated code; the publishable matrix must confirm it.

## Remaining work

- `float64array-traversal` is unchanged: its receivers come from
  `buffers.ints`/`buffers.floats` locals, and `ArrayPlan` still admits only
  entry-argument identities (roadmap P1 item 6). Once non-argument receivers
  are admitted, items 4–6 above apply to it directly.
- `arrays-typed` is now dominated by its interpreter-only orchestration
  (`values.push`, `new Int32Array(values)`, `toFixed`, concatenation), not by
  the typed traversals.
- The amortized poll countdown lives in a stack slot; keeping it in an SSA
  variable measured roughly a further 1.1x on the traversals (int32 loop to
  about 27 µs) but conflicts with the structural property-cache tests that
  pin the stack-slot form. It belongs with the poll-density item.
- `(sum + a[i]) | 0` still performs a checked add; a wrapping add when the
  sum only feeds `| 0` would remove the overflow exit.
- Bounds checks are still per access for loops bounded by anything other than
  the receiver's own single `length` read (for example `i < n` or a second
  array's length); a range fact relating two lengths needs a guarded
  `n <= length` check in the preheader.
- Register pressure spills hoisted `data`/`count` in the packed loop.
