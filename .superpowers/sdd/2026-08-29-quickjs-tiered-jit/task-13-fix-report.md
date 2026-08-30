# Task 13 correctness hardening report

Implementation commit: `728e95f`

## Review findings closed

- Test262 forced modes use the production hotness, snapshot, background compile,
  install, and replay path in one live context. Only cases named in
  `test262-jit-eligible.json` are forced; every other selection is an explicit
  ineligible skip. Tier 2 sets the optimizing policy and requires an observed
  Tier 2 entry. Native entries/exits, exact executed opcode/helper IDs, and zero
  unexpected fallback are recorded per variant.
- The parser retains the exact source and raw lexemes plus byte ranges for the
  YAML metadata and executable body. Raw cases receive no harness or strict
  injection. Script parse negatives use compile-only evaluation; modules are
  declared, linked/evaluated, and their TLA promise is finished as distinct
  phase boundaries. Negative phase and error type must both match.
- Arbitrary object observation no longer invokes `toString`, `instanceof`,
  constructors, prototypes, property descriptors, or Proxy traps. It reports
  a `primitive-only` capability and opaque object tag. Proxy-free plain graph
  snapshots require an explicit fixture API. A real Proxy event-order
  regression proves observation adds no events.
- Seeded differential programs return an explicit primitive snapshot containing
  arithmetic, loop, closure, alias, getter/Proxy, coercion, exception/finally,
  and event-order results. Sixty-four seeds run interpreter, automatic native,
  and strict forced baseline with PC/helper evidence; sixteen seeds additionally
  require optimized native entry. Automatic/optimized runs reject unexpected
  fallback.
- The differential fuzz target now consumes input bytes as a structured-program
  seed/fuel and compares interpreter with the real automatic JIT. All six fuzz
  targets compile together; their snapshot/verifier/lowering/frame/deopt/
  relocation inputs remain byte-driven and versioned by the opcode fingerprint.
- PR CI runs bounded Test262 interpreter/automatic/forced Tier 1/forced Tier 2,
  random differential tests, regressions, and all fuzz target builds. The
  scheduled workflow runs 256 complete Test262 shards in interpreter and
  automatic modes, rejects missing/duplicate/zero/different outcomes, runs the
  pinned QuickJS C reference, native Linux/macOS/Windows jobs, and a WASM
  interpreter-only build.

## Local evidence

- `14.1-4gs.js` raw parse negative: 1/1 variants pass; compile-only execution
  proves the preceding throw is never evaluated.
- Parse fixture `S12.9_A1_T1.js`: 2/2 variants pass.
- Resolution fixture `instn-iee-err-not-found.js`: 1/1 variant passes.
- Module/TLA fixture `void-await-expr.js`: 1/1 variant passes.
- Forced eligible release suite: Tier 1 2/2 and Tier 2 2/2 variants pass;
  native entries/exits and opcode IDs are nonzero, Tier 2 entries are nonzero,
  and unexpected fallback is zero.
- Release changed suites: correctness 18/18, differential 8/8, regressions 1/1,
  bounded Test262 4/4 test groups (44 total variants) pass.
- `rquickjs-core` context tests: 17/17 pass.
- `cargo check --manifest-path jit/fuzz/Cargo.toml --bins`: pass.
- Feature-enabled all-target clippy with `-D warnings`: pass.
- Workspace formatting and `git diff --check`: pass.

The scheduled cross-platform, C-reference, WASM, and full 256-shard corpus jobs
are CI evidence requirements; they were not represented as local execution.
