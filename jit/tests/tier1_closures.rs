//! Tier 1 closure coverage: `fclosure`, closure-variable access, `close_loc`
//! and `set_name`. Every program runs once in the interpreter and once with
//! the named function `f` forced into published Tier 1 code; results must be
//! identical, including exceptions, TDZ errors and values that a closure
//! writes into the native frame's captured arguments and locals.
#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::bytecode::{FallbackReason, HelperId};
use rquickjs_jit::test_support::{assert_tier1_rejected, differential};
use rquickjs_jit::{Jit, JitConfig, JitTierPolicy};
use std::time::{Duration, Instant};

#[test]
fn closure_writes_to_captured_locals_are_reloaded_after_every_call() {
    // `inc` writes the captured `c` through its attached var ref while the
    // native frame holds a register copy. The following `c++` must observe it.
    differential(
        "function f(n){ let c = 0; const inc = () => { c += 2 }; for (let i = 0; i < n; i++) { inc(); c++ } return c }",
        "f(10)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("fclosure8")
    .expect_helper(HelperId::FClosure)
    .assert_same();
}

#[test]
fn closure_writes_to_captured_arguments_are_reloaded() {
    differential(
        "function f(a){ const set = v => { a = v }; set(5); let r = a + 1; set('x'); return r + ':' + a }",
        "f(1)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("fclosure8")
    .assert_same();
}

#[test]
fn closure_type_change_takes_the_native_numeric_guard_exit() {
    // The closure turns the captured Int32 into a string. Subtraction is a
    // native-only numeric operation, so the reloaded value must fail its
    // tag guard and deoptimize instead of using the stale register copy.
    differential(
        "function f(n){ let x = n; const g = () => { x = 'a' + x }; g(); return x - 1 }",
        "String(f(3))",
    )
    .force_baseline()
    .expect_executed_opcode("fclosure8")
    .expect_deopt()
    .assert_same();
}

#[test]
fn captured_heap_values_keep_exact_ownership_under_stress_gc() {
    differential(
        "function f(n){ let o = { v: 0 }; const g = () => { o = { v: o.v + 1, prev: o } }; for (let i = 0; i < n; i++) g(); return o.v + ':' + o.prev.v }",
        "f(20)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("fclosure8")
    .assert_same();
}

#[test]
fn per_iteration_bindings_are_closed_at_loop_boundaries() {
    differential(
        "function f(n){ const fs = []; for (let i = 0; i < n; i++) { fs.push(() => i * 10) } let s = ''; for (let j = 0; j < n; j++) { const g = fs[j]; s += g() + ',' } return s }",
        "f(6)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("close_loc")
    .expect_helper(HelperId::CloseLocal)
    .assert_same();
}

#[test]
fn closure_variable_reads_raise_the_interpreter_tdz_error() {
    differential(
        "var f = (function(){ if (globalThis) return function f(){ return x }; let x = 1 })()",
        "(() => { try { return f() } catch (e) { return e.name + ':' + e.message } })()",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("get_var_ref_check")
    .expect_helper(HelperId::GetVarRef)
    .assert_same();
}

#[test]
fn closure_variable_writes_raise_the_interpreter_tdz_error() {
    differential(
        "var f = (function(){ if (globalThis) return function f(v){ x = v; return 1 }; let x = 1 })()",
        "(() => { try { return f({}) } catch (e) { return e.name + ':' + e.message } })()",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("put_var_ref_check")
    .expect_helper(HelperId::PutVarRef)
    .assert_same();
}

#[test]
fn closure_variables_are_shared_between_closures_and_the_owner() {
    differential(
        "var peek; var f = (function(){ let c = 1; peek = () => c; return function f(a){ c = c * 2 + a; return c + peek() } })()",
        "[f(1), f(2), peek()].join()",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("put_var_ref_check")
    .assert_same();
}

#[test]
fn set_var_ref_keeps_the_stored_heap_value_on_the_stack() {
    differential(
        "var peek; var f = (function(){ var c = null; peek = () => c; return function f(a){ return (c = { a: a }).a + peek().a } })()",
        "f(21)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("set_var_ref0")
    .assert_same();
}

#[test]
fn anonymous_closure_names_match_the_interpreter() {
    differential(
        "function f(){ const g = () => 1; const h = function(){ return 2 }; return g.name + ',' + h.name + ',' + (g() + h()) }",
        "f()",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("set_name")
    .expect_helper(HelperId::SetName)
    .assert_same();
}

#[test]
fn generator_and_async_closures_are_created_but_run_in_the_interpreter() {
    differential(
        "function f(n){ const gen = function* () { yield n; yield n + 1 }; const it = gen(); const a = it.next().value; const b = it.next().value; return a + b }",
        "f(20)",
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("fclosure8")
    .assert_same();
}

#[test]
fn derived_constructor_this_initialization_through_arrows_stays_rejected() {
    assert_tier1_rejected(
        "var f = (function(){ let r; class A { constructor(){ this.v = 1 } } class C extends A { constructor(){ r = () => super(); r() } } new C(); return r })()",
        "(() => { try { f(); return 'no' } catch (e) { return e.name } })()",
        FallbackReason::ClosureFrame,
    );
}

#[test]
fn automatic_tiering_runs_closure_creating_loops_natively() {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(JitTierPolicy::BaselineOnly)
            .call_threshold(1)
            .loop_threshold(1)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>(
            "globalThis.kernel = function kernel(n){ let c = 0; const add = x => y => (x + y) | 0; for (let i = 0; i < n; i++) { c = add(c)(i & 31) } return c }",
        )
        .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut result = 0;
    while Instant::now() < deadline {
        result = context.with(|ctx| ctx.eval::<i32, _>("kernel(200)").unwrap());
        jit.poll();
        if jit.metrics().native_entries > 0 {
            break;
        }
    }
    let expected = (0..200).fold(0_i32, |c, i| c.wrapping_add(i & 31));
    assert_eq!(result, expected);
    assert!(jit.metrics().native_entries > 0, "{:?}", jit.metrics());
    let after = context.with(|ctx| ctx.eval::<i32, _>("kernel(200)").unwrap());
    assert_eq!(after, expected);
}
