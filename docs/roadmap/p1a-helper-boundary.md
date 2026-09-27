# P1a: O(1) helper-boundary validation

Branch `perf/p1a-helper-boundary`, based on `82d3808`. This covers roadmap P1
items 1, 2 and 5. The C changes are in `sys/patches/0021-helper-boundary.patch`.

## What changed

1. **O(1) PC validation (B1).** `qjsjit_valid_pc` used to scan the bytecode
   linearly. Its cursor only helped forward queries, so the first helper call
   after a loop back-edge rescanned from offset 0. Each `JSFunctionBytecode`
   now builds an instruction-boundary bitmap on the first query, and every
   later query is a range check plus one bit test.
   - The bitmap decodes instructions with exactly the old scan's rules.
     Offsets after an undecodable instruction stay clear.
   - Bit `byte_code_len` is set only when the decode ends exactly at the end
     of the bytecode, so `allow_end` behaves as before.
   - The bitmap takes `len/8 + 1` bytes from the runtime allocator. It is
     freed with the bytecode and counted in `JS_ComputeMemoryUsage`.
   - If the allocation fails, the old cursor scan answers. Nothing raises an
     exception.
2. **Cheaper per-helper validation (partial item 2).** Every identity, ABI,
   stack and PC check stays. Two changes make the fast path cheaper:
   - The pending-exception test is inlined; it was an out-of-line
     `JS_HasException` call on every helper.
   - `qjsjit_reject_helper` is `no_inline`. The fast path no longer carries
     spills or stack-protector setup.
3. **`JS_JitInlineEnter`.**
   - The redundant `call_pc`/`continuation_pc` scans are gone. Frame
     validation already proves that `caller->pc == call_pc` is an instruction
     boundary. The continuation is checked as
     `call_pc + size(call opcode) < len`, which makes it the next boundary.
   - Shadow frames come from a LIFO arena owned by the runtime. The arena
     starts at 32 KiB and grows only while empty, up to the 1 MiB inline
     budget plus alignment. It is released in `JS_FreeRuntime`.
   - A frame that does not fit, or an arena allocation failure, falls back to
     the heap. Declined entries release their storage after the depth
     decrement, so depth 0 always resets the arena.
   - ASan and MSan builds always use the heap, so use-after-free detection
     still works.

No public ABI, binding layout or ABI version changes.

## Tests

- `jit/tests/helpers.rs`: `invalid_identity_map_index_and_slot_are_rejected_before_touching_values`
  now also covers a frame PC inside an instruction and a frame PC past the end.
  Both must be rejected without touching any values.
- `sys/tests/inline_recovery.rs`:
  - Nested inline frames (scenario 16) must be stacked contiguously in the
    arena and must unwind it in LIFO order.
  - Every scenario must leave depth, bytes and arena use at zero.
  - The test also passes with `QUICKJS_INLINE_TEST_SANITIZER=address`.

## Noisy diagnostic (loaded 24-core host, not publishable)

Method: 7 alternating runs of `jit-bench worker --mode automatic`. Each run's
value is the median of the last 16 `protocol.warmup_batch_ns`, and the table
shows the median across runs. The baseline is the 82d3808 binary. Speed is
base/new. On every workload, the checksums from base, new and the interpreter
were identical.

| workload | base ms | new ms | speed |
| --- | ---: | ---: | ---: |
| objects-polymorphic | 23.934 | 13.742 | 1.74x |
| generic-call-fallback | 9.126 | 6.205 | 1.47x |
| property-heavy | 0.0986 | 0.0979 | 1.01x |
| arrays-typed | 2.979 | 2.892 | 1.03x |
| float64array-traversal | 1.102 | 1.053 | 1.05x |

These match or exceed the roadmap's unsafe "skip PC validation" upper bounds
(1.71x and 1.29x). There are no confidence intervals, so read the results
near 1.0x as ties.

## Remaining work

- **Cookie-only per-helper validation (the rest of item 2).** Not done.
  - `helpers.rs` requires every helper to reject runtime-id, runtime-API,
    helper-ABI-version, stack-capacity, stack-top, generation and cookie
    tampering.
  - The benchmark binary is built with `test-support`, so moving those checks
    behind a CONFIG flag would either break those tests or not be measured.
  - After this patch, a gdb-sampled profile shows the remaining validation is
    a straight-line set of about 25 L1-hot compares. The next step is to
    record a validated-entry epoch at native entry/OSR/resume, and to move the
    immutable-field checks under a build flag that the tamper tests enable
    explicitly.
- **Stack-map PCs.** Stack maps could carry their compile-time PC so that
  helpers compare `frame->pc` exactly. The bitmap already makes this O(1), so
  the only remaining gain is removing one load and one bit test.
- **Publishable evidence.** The three-engine README matrix (QuickJS, Bun,
  quickjs-jit) is pending and is left to the integrator.
