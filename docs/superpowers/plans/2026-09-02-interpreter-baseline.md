# True Interpreter Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a statistically valid gpui-shell JIT comparison against a runtime with no JIT backend attached.

**Architecture:** Add an explicit interpreter constructor to the owning `JitRuntime` builder while preserving the existing JIT-enabled default. Wire gpui-shell's test-only benchmark runtime to that constructor, emit one JSON sample per fresh process, and run the existing paired bootstrap reporter.

**Tech Stack:** Rust, Cargo tests, Bash, Python aggregation, macOS Apple Silicon.

**Spec:** GitHub Issue #3 acceptance test and `benchmarks/gpui_shell_report.rs`.

## Global Constraints

- `JitRuntimeBuilder::build()` remains JIT-enabled and backward compatible.
- Interpreter mode must attach no JIT backend and report `native_enabled == false` with zero JIT counters.
- Interpreter and automatic samples must use identical source, render counts, and snapshot assertions.
- The final run uses 5 discarded warmup processes and 30 interleaved process pairs.
- Preserve unrelated gpui-component `assets/` files.

---

### Task 1: Owning interpreter runtime

**Files:**
- Modify: `jit/src/lib.rs`
- Test: `jit/tests/api.rs`

**Interfaces:**
- Produces: `JitRuntimeBuilder::build_interpreter(self) -> Result<JitRuntime, JitError>`
- Produces: `JitRuntime::metrics() -> JitMetrics` returning disabled metrics when no guard is attached

- [ ] **Step 1: Write the failing API test**

```rust
#[test]
fn owning_interpreter_runtime_has_no_jit_backend() {
    let runtime = JitRuntime::builder().build_interpreter().unwrap();
    assert!(!runtime.metrics().native_enabled());
    assert_eq!(runtime.metrics().queued, 0);
    Context::full(&runtime).unwrap();
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test -p quickjs-jit-runtime --features compiler --test api owning_interpreter_runtime_has_no_jit_backend -- --exact`

Expected: compilation fails because `build_interpreter` does not exist.

- [ ] **Step 3: Implement the minimal constructor**

Construct `Runtime::new()` and return `JitRuntime { jit: None, runtime: Some(runtime) }`. Make `metrics()` return `JitMetrics::disabled()` for `None`; correct the stale `build()` and `jit()` documentation.

- [ ] **Step 4: Verify GREEN and regression coverage**

Run: `cargo test -p quickjs-jit-runtime --features compiler --test api`

Expected: all API tests pass.

### Task 2: Use the true baseline in gpui-shell

**Files:**
- Modify: `/Users/jason/work/gpui-component/crates/shell/src/engine/quickjs/mod.rs`
- Modify: `/Users/jason/work/gpui-component/crates/shell/src/tests/benchmark.rs`
- Modify: `scripts/bench-gpui-shell.sh`

**Interfaces:**
- Consumes: `JitRuntimeBuilder::build_interpreter()`
- Produces: `tests::benchmark::emit_one_jit_acceptance_sample`

- [ ] **Step 1: Preserve the observed invalid-baseline failure**

Run the existing 30-pair report and record that threshold-only interpreter mode falsely reports a 23x panel speedup despite zero native entries.

- [ ] **Step 2: Replace the threshold workaround**

Change `ShellRuntime::new_isolated_interpreter()` to construct its runtime with `JitRuntime::builder().build_interpreter()` through a small internal constructor that accepts an already-built `JitRuntime`.

- [ ] **Step 3: Run one paired compute and panel smoke sample**

Run the emitter once for each workload and mode. Assert equal snapshot hashes and render counts, automatic compute `native_entries > 0`, and interpreter `native_enabled == false` in the emitted diagnostics.

- [ ] **Step 4: Verify formatting and shell portability**

Run: `cargo fmt --all -- --check`, `git diff --check`, and `bash -n scripts/bench-gpui-shell.sh`.

### Task 3: Produce and publish acceptance evidence

**Files:**
- Update: GitHub Issue #3
- Update: quickjs-jit PR #15 and gpui-component PR #2903 descriptions/comments as appropriate

**Interfaces:**
- Consumes: `gpui-shell-jit-v1` JSON samples
- Produces: bootstrap confidence intervals and preserved provenance

- [ ] **Step 1: Run the full paired benchmark**

Run: `scripts/bench-gpui-shell.sh /Users/jason/work/gpui-component /tmp/gpui-shell-issue3-0.12.5.json`

Expected: report exits zero after 5 warmups and 30 interleaved pairs.

- [ ] **Step 2: Audit the generated evidence**

Verify 30 samples per mode/workload, pairwise identical snapshot hashes and render counts, zero interpreter native entries, positive automatic compute native entries, and all bootstrap gates.

- [ ] **Step 3: Commit and push both repositories**

Commit only scoped quickjs-jit and gpui-component files; do not stage gpui-component `assets/`.

- [ ] **Step 4: Update Issue #3 and PR evidence**

Replace the outstanding statistical checklist item with the measured confidence intervals and explicitly state whether CPU affinity was available.
