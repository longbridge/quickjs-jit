# Task 13 correctness evidence

Date: 2026-08-30. Host: Linux x86-64. Test262 revision:
`d5e73fc8d2c663554fb72e2380a8c2bc1a318a33`. Opcode fingerprint:
`0x05d5c0867521c077` (252 total, 7 native, 19 helper-backed, 226 rejected).

## Suite boundaries

- The C `run-test262` runner is an interpreter reference and is never counted
  as JIT coverage. A direct build was needed because this host has no CMake:
  `cc -O2 -D_GNU_SOURCE -o /tmp/qjsjit-run-test262 run-test262.c quickjs-libc.c quickjs.c dtoa.c libregexp.c libunicode.c -lm -ldl -lpthread`.
- C QuickJS suite: `/tmp/qjsjit-run-test262 -c tests.conf` produced
  `0/63 errors, 5 excluded`.
- C fast Test262 reference: `/tmp/qjsjit-run-test262 -m -c test262.conf -c
  test262-fast.conf -a` exited successfully with `92/79986` pinned known errors,
  5,450 excluded, and 5,804 skipped.
- The Rust host deliberately has a separate checked compatibility/exclusion
  manifest. It does not claim that C-host-only `std`, `os`, agent, realm,
  detach-buffer, Intl, or unresolved module-graph facilities execute in the
  Rust host.

## Rust-host Test262

The runner discovered 53,169 runnable non-fixture JavaScript files and records
that count, shard identity, SHA, strict/raw/module/async variant, negative
phase/type, duration, skip reason, and native evidence in per-case JSON.

- Focused first 100 paths: interpreter and automatic each produced 142 pass,
  58 checked skips, 0 failures, and identical case sets.
- Deterministic universe shard 0/256: interpreter and automatic each produced
  294 pass, 106 checked skips, 0 failures, and identical case sets. Reports are
  `target/jit-test262/{interpreter,automatic}-shard-0-of-256.json`.
- Focused behavior: module 1/1, async `$DONE` 2/2, parse-negative 2/2, and
  runtime-negative 2/2 passed.
- Automatic native entries were zero in this shard; it is interpreter-fallback
  semantic evidence, not advertised native coverage.
- Forced native coverage remains the release opcode/differential matrix: all
  26 advertised opcode PCs execute native code with exact helper correlation,
  no generated retry, and matching canonical observations.

## Differential and bounded randomized validation

- The canonical observation retains special numbers, `-0`, BigInt, Symbol,
  cycles/aliases, own keys and descriptors, prototypes, sparse arrays, Map/Set,
  typed-array bytes, normalized errors/stacks, and event-visible objects.
- 64 deterministic structured programs cover bounded loops, closures, arrays,
  holes, objects, getters, proxies, coercion events, throws, and `finally`; all
  interpreter/automatic observations matched. A checked special-value
  regression replays from JSON.
- Six versioned input programs exist for snapshots, verifier, differential,
  frame state, lowering, and relocations. Snapshot validation proves complete
  deterministic decode boundaries; frame-state validation exercises real
  `DeoptMap` shape/duplicate rejection; verifier/lowering capture real QuickJS
  functions; relocation validation asserts rejected batches leave writable
  bytes unchanged.
- Direct bounded runs completed: snapshot 100, verifier 100, differential 50,
  frame-state 100, lowering 50, relocation 100. The host lacks the cargo-fuzz
  wrapper and sanitizer callbacks, so these direct libFuzzer runs are smoke
  evidence; instrumented scheduled CI remains required.

## Final local commands

- `cargo test -p rquickjs-jit --all-targets --features compiler,test-support --release`: pass.
- `cargo clippy -p rquickjs-jit --all-targets --features compiler,test-support -- -D warnings`: pass.
- `cargo check --manifest-path jit/fuzz/Cargo.toml --bins`: pass before cleaning its 1.7 GiB build directory.
- `cargo fmt --all`, nested fuzz formatting, and `git diff --check`: pass.

The full 256-shard Rust-host matrix and platform/sanitizer executions are CI
evidence requirements; this report does not present shard 0 as the full corpus.
