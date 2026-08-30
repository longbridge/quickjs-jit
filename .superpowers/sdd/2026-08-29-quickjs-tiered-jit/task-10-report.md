# Task 10 report: background compilation and installation

## Outcome

- QuickJS now reports function-entry hot events and transfers an owned snapshot
  to the backend when requested. Rust copies and verifies the snapshot during
  the callback and frees the C allocation exactly once with an RAII guard.
- Compiler-enabled native targets attach `ProductionBackend`, not `NoopBackend`.
  It owns the coordinator, bounded workers, resource configuration, generation
  state, and live metrics.
- Workers receive only owned `CompileRequest` data. They produce relocatable
  artifacts and never access QuickJS, executable mappings, or the code cache.
- Runtime-thread completion drain revalidates artifact identity, publishes W^X
  executable memory, resolves relocations, registers unwind metadata, inserts
  the cache entry, and publishes only bytecode PC zero. `Jit::poll` uses the
  runtime lock and the same installation path as automatic callbacks.
- Entry handles retain a cache `ExecutionPin`; release drops it exactly once.
  Generation retirement immediately prevents new acquisition while active pins
  keep old code alive until reclamation.
- Shutdown cooperatively cancels compilation, closes submission, joins workers,
  drains nonblocking completions, retires functions, and reclaims cache state.

## Resource boundaries

- Bounded worker and completion channels use nonblocking `try_send`; saturation
  rolls back dispatch or moves a completion into a bounded worker-result overflow
  queue drained only by the runtime thread, without losing state or deadlocking.
- Pending snapshot bytes, estimated IR bytes, and worker job counts use shared
  atomic accounting with a worker-side RAII release guard.
- Compiler cancellation and deadlines are checked before/after translation,
  lowering, Cranelift compilation, and publication handoff. Timeouts have a
  distinct metric/category from cancellation and general resource rejection.
- Code bytes and metadata bytes have separate aggregate cache quotas, accounting,
  eviction requirements, and live metrics.
- Snapshot, IR, code, metadata, queue, completion, attempt, and wall-clock limits
  fall back to interpretation rather than invalidating JavaScript execution.

## Verification

- `cargo test -p rquickjs-jit --features compiler,test-support --test background --test lifecycle --release -- --test-threads=8`
  - 10 background tests passed.
  - 45 lifecycle tests passed.
- `cargo test -p rquickjs-jit --all-targets --features compiler,test-support`
  - full JIT crate suite passed, including 28 baseline, 26 native-boundary,
    25 verifier, differential, helpers, semantics, platform, ABI, and production
    runtime tests.
- `cargo clippy -p rquickjs-jit --all-targets --features compiler,test-support -- -D warnings`
  passed.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Production-path coverage executes JavaScript once to trigger compilation,
  waits for runtime-thread installation, then calls the retained function again
  through its real native entry and verifies result `42`.
- Concurrency coverage includes blocked compilation, queue/completion saturation,
  cooperative shutdown without external release, reload races, pin-safe
  invalidation, and 1,000 generation-isolated hot/reload/install/retire cycles.

## Commits

- Nested QuickJS: `67b5093` (`feat(jit): submit hot function snapshots`)
- Root implementation: `6caf241`
- Saturation/shutdown correction: `7e09be8`
- Independent cache/compiler resource limits: `86b4474`
- RAII gauges and timeout categorization: `b7915b5`
- Saturated completion preservation: `8e8caf5`
- Nested review fixes: `063f517`
- Root review fixes: `7aa8842`

Task 10 intentionally publishes entry PC zero only. Hot thresholds, loop events,
and nonzero-PC OSR policy remain Task 11 work.

## Independent review corrections

- Runtime maintenance no longer invokes metrics observers. It only updates the
  shared snapshot; `Jit::poll` notifies after releasing the runtime lock, catches
  observer panics, and permits recursive polling without deadlock.
- Production coordinators now use the engine-assigned runtime ID, the complete
  `JSJitABIInfo` build/layout/helper fingerprint, source/opcode identities, host
  ISA and detected CPU features, and a stable numeric configuration fingerprint.
  Worker requests, completions, artifacts, installation, and acquisition retain
  and revalidate the exact key.
- `RawRuntime::execute_pending_job` performs bounded backend maintenance before
  and after the QuickJS job while the same runtime lock is held. A production
  test proves a worker completion installs without another eligible JS call.
- Production native execution is selected only on little-endian macOS, Windows,
  and Linux for x86-64/AArch64. Other targets attach the interpreter-only no-op
  backend even when the compiler feature is selected.
- Generator/async resume frames now allocate and initialize the two helper
  scratch slots, place var-ref storage after them, and derive a defined native
  stack capacity before entry.
- ABI minor 1.4 appends exact `native_enter`/`native_exit` callbacks. QuickJS
  calls them immediately around the published entry trampoline; production
  metrics therefore prove native entry/return rather than inferring it from JS
  result `42`. The production test observes entries/exits above zero and zero
  deopt/retry fallbacks.

## Review-fix verification

- Focused debug: API 7/7, background 12/12, ABI 12/12.
- Full debug: `cargo test -p rquickjs-jit --all-targets --features compiler,test-support`
  passed (all JIT unit/integration targets, including baseline 28/28,
  native-boundary 26/26, lifecycle 45/45, verifier 25/25).
- Focused release: ABI 12/12, API 7/7, background 12/12, native-boundary 26/26.
- Workspace: `cargo check --workspace --all-targets --features
  rquickjs-jit/compiler,rquickjs-jit/test-support` passed.
- C warning regression: a fresh target directory with `CFLAGS=-Werror` passed
  `cargo check -p rquickjs-jit --features compiler,test-support`.
- `cargo clippy -p rquickjs-jit --all-targets --features
  compiler,test-support -- -D warnings`, formatting, and diff checks passed.
- All nine bundled native JIT bindings share the regenerated ABI 1.4
  declarations. A local `wasm32-unknown-unknown` cross-check could not compile
  QuickJS because that installed target lacks a C sysroot (`stdlib.h`); the
  Rust target gating is covered structurally and WASM remains interpreter-only.
