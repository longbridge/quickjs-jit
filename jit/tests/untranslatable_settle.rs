#![cfg(all(
    any(target_os = "linux", target_os = "macos"),
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

//! Automatic tiering for Tier 1 functions that Tier 2 can never translate.

fn automatic_runtime() -> (rquickjs::Runtime, rquickjs_jit::Jit, rquickjs::Context) {
    let runtime = rquickjs::Runtime::new().unwrap();
    let jit = rquickjs_jit::Jit::attach(
        &runtime,
        rquickjs_jit::JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    (runtime, jit, context)
}

#[test]
fn untranslatable_loop_settles_its_baseline_decision_once() {
    // `typeof` has no optimized classification, so Tier 2 can never compile
    // this function. Its baseline decision must be taken a bounded number of
    // times instead of rebuilding feedback snapshots at every maintenance.
    let (_runtime, jit, context) = automatic_runtime();
    context.with(|ctx| {
        ctx.eval::<(), _>(
            "globalThis.kinds = function(items) { let n = 0; \
               for (let i = 0; i < items.length; i++) { if (typeof items[i] === 'string') n++; } \
               return n; }; \
             globalThis.items = ['a', 1, 'b', {}, 'c'];",
        )
        .unwrap();
    });
    let mut evaluations = Vec::new();
    let mut entries = Vec::new();
    for round in 0..4_000 {
        let value = context.with(|ctx| ctx.eval::<i32, _>("kinds(items)").unwrap());
        assert_eq!(value, 3);
        jit.poll();
        if round % 1_000 == 999 {
            let metrics = jit.metrics();
            evaluations.push(metrics.profitability_evaluations);
            entries.push(metrics.native_entries);
        }
    }
    let metrics = jit.metrics();
    assert!(metrics.native_entries > 0, "{metrics:?}");
    assert_eq!(metrics.tier2_entries, 0, "{metrics:?}");
    assert_eq!(metrics.unsupported_opcode_failures, 0, "{metrics:?}");
    assert!(
        evaluations.windows(2).all(|pair| pair[0] == pair[1]) && evaluations[0] <= 5,
        "untranslatable candidate kept re-evaluating: {evaluations:?} {metrics:?}"
    );
    // The settled generation runs in the interpreter with its probes off,
    // exactly where a failed bounded Tier 2 trial used to leave it.
    assert!(
        entries.windows(2).all(|pair| pair[0] == pair[1]),
        "settled generation kept entering native code: {entries:?} {metrics:?}"
    );
}

#[test]
fn untranslatable_call_only_method_returns_to_the_interpreter() {
    let (_runtime, jit, context) = automatic_runtime();
    context.with(|ctx| {
        ctx.eval::<(), _>(
            "function Box(v) { this.v = v; } \
             Box.prototype.twice = function() { return this.add(this.v); }; \
             Box.prototype.add = function(x) { return this.v + x; }; \
             globalThis.box = new Box(21);",
        )
        .unwrap();
    });
    for _ in 0..4_000 {
        let value = context.with(|ctx| ctx.eval::<i32, _>("box.twice()").unwrap());
        assert_eq!(value, 42);
        jit.poll();
    }
    let metrics = jit.metrics();
    assert!(metrics.generic_call_rejections > 0, "{metrics:?}");
}

/// Runs `call` repeatedly under automatic tiering and returns the
/// `native_entries` metric sampled after each 1,000-round window.
fn native_entry_windows(source: &str, call: &str, expected: &str) -> Vec<u64> {
    let (_runtime, jit, context) = automatic_runtime();
    context.with(|ctx| ctx.eval::<(), _>(source).unwrap());
    let mut entries = Vec::new();
    for round in 0..4_000 {
        let value = context.with(|ctx| ctx.eval::<String, _>(call).unwrap());
        assert_eq!(value, expected);
        jit.poll();
        if round % 1_000 == 999 {
            entries.push(jit.metrics().native_entries);
        }
    }
    entries
}

#[test]
fn untranslatable_loop_without_newly_admitted_opcodes_keeps_its_baseline() {
    // Regression: the settle must not demote Tier 1 code that already ran
    // natively at 82d3808. String literals (`push_atom_value`), object
    // literals (`object`) and `new` (`call_constructor`) have no Tier 2
    // classification, but these loops were Tier 1 functions before the
    // GENERIC_OP opcodes were admitted and must keep entering native code.
    let cases = [
        (
            "globalThis.f = function(n, z) { let sum = z; \
               for (let i = z; i < n; i++) sum = sum + i * 0.5; return 'r' + sum; };",
            "f(64, 0)",
            "r1008",
        ),
        (
            "globalThis.f = function(n) { let sum = 0; \
               for (let i = 0; i < n; i++) { const o = { v: i }; sum = sum + o.v; } \
               return '' + sum; };",
            "f(64)",
            "2016",
        ),
        (
            "function P(v) { this.v = v; } \
             globalThis.f = function(n) { let sum = 0; \
               for (let i = 0; i < n; i++) { const p = new P(i); sum = sum + p.v; } \
               return '' + sum; };",
            "f(64)",
            "2016",
        ),
    ];
    for (source, call, expected) in cases {
        let entries = native_entry_windows(source, call, expected);
        assert!(
            entries.windows(2).all(|pair| pair[1] > pair[0]),
            "Tier 1 function stopped entering native code: {source} {entries:?}"
        );
    }
}
