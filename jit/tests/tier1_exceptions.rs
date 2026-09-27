//! Tier 1 exception regions: `throw`, `throw_error`, try/catch (`catch`,
//! `nip_catch`) and try/finally (`gosub`, `ret`) execute natively and match
//! the interpreter exactly, including exceptions raised by helpers inside a
//! try region, nested regions, rethrow, and uncatchable interrupts.

#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs_jit::test_support::{differential, forced_baseline};

fn same(definition: &str, expression: &str, opcode: &str) {
    differential(definition, expression)
        .force_baseline()
        .expect_executed_opcode(opcode)
        .assert_same();
    differential(definition, expression)
        .force_baseline()
        .expect_executed_opcode(opcode)
        .stress_gc()
        .assert_same();
}

#[test]
fn catch_in_a_hot_loop_resumes_the_handler_natively() {
    // `neg` occurs only in the handler: it must execute in native code.
    same(
        "function f(n){ let caught=0; for(let i=0;i<n;i++){ try { if((i&15)===0) throw i; } catch (value) { caught-=-value; } } return caught; }",
        "f(160)",
        "neg",
    );
    same(
        "function f(n){ let caught=0; for(let i=0;i<n;i++){ try { if((i&15)===0) throw i; } catch (value) { caught+=value; } } return caught; }",
        "f(160)",
        "throw",
    );
}

#[test]
fn uncaught_throw_leaves_with_the_exact_exception() {
    same(
        "function f(a){ if (a > 1) throw {code: a}; return a; }",
        "(function(){ try { return f(3); } catch (e) { return 'outer:' + e.code; } })()",
        "throw",
    );
    same(
        "function f(a){ throw new TypeError('bad ' + a); }",
        "(function(){ try { f(7); } catch (e) { return e.name + ':' + e.message; } })()",
        "throw",
    );
}

#[test]
fn helper_exceptions_inside_try_reach_the_native_handler() {
    // GET_PROPERTY on undefined, CALL of a non-function and a throwing getter.
    same(
        "function f(o){ let r = 0; try { r = o.x.y; } catch (e) { r = -1; } return r; }",
        "[f({x:{y:5}}), f({}), f({x:null})].join()",
        "push_minus1",
    );
    same(
        "function f(g){ try { return g(1); } catch (e) { return 'caught:' + e.name; } }",
        "[f(function(v){return v+1}), f(5), f(null)].join()",
        "nip_catch",
    );
    same(
        "function f(o){ let n = 0; for (let i = 0; i < 20; i++) { try { n += o.v; } catch (e) { n -= -e; } } return n; }",
        "f({ get v() { if (this.k++ % 3 === 0) throw 7; return 2; }, k: 0 })",
        "neg",
    );
    // Pending owned operands above the catch offset are released exactly.
    same(
        "function f(a, o){ let r; try { r = [a, {k: a}, o.x.y]; } catch (e) { r = -1; } return r; }",
        "String(f({}, {})) + String(f([1], {x: {y: 2}}).length)",
        "push_minus1",
    );
}

#[test]
fn error_stack_is_captured_at_the_throwing_pc() {
    same(
        "function f(o){ try { return o.missing.field; } catch (e) { return e.stack; } }",
        "f({})",
        "catch",
    );
    same(
        "function f(a){\n  try {\n    throw new Error('line' + a);\n  } catch (e) {\n    return e.stack;\n  }\n}",
        "f(2)",
        "throw",
    );
}

#[test]
fn rethrow_and_nested_regions_match_the_interpreter() {
    same(
        "function f(a){ let log = ''; try { try { throw a; } catch (e) { log += 'inner' + e; throw e + 1; } } catch (e) { log += ',outer' + e; } return log; }",
        "f(1)",
        "catch",
    );
    same(
        "function f(a){ let log = ''; try { try { throw a; } finally { log += 'finally'; } } catch (e) { log += ',caught' + e; } return log; }",
        "f(4)",
        "gosub",
    );
    same(
        "function f(a){ let log = ''; try { try { log += 'body'; } finally { log += ',f1'; } log += ',after'; } finally { log += ',f2'; } return log; }",
        "f(0)",
        "ret",
    );
    same(
        "function f(a){ try { return a + 1; } finally { a = 100; } }",
        "f(41)",
        "gosub",
    );
    same(
        "function f(a){ try { throw a; } finally { return 'override'; } }",
        "f(1)",
        "throw",
    );
    same(
        "function f(a){ let log = []; try { log.push('t'); throw new RangeError('r' + a); } catch (e) { log.push(e.message); } finally { log.push('f'); } return log.join(); }",
        "f(9)",
        "gosub",
    );
}

#[test]
fn control_transfers_through_finally_blocks() {
    same(
        "function f(n){ let c = 0; for (let i = 0; i < n; i++) { try { if (i % 3 === 0) continue; c += i; } finally { c += 1000; } } return c; }",
        "f(20)",
        "ret",
    );
    same(
        "function f(n){ let c = 0; while (true) { try { c++; if (c > n) break; } finally { c += 10; } } return c; }",
        "f(35)",
        "gosub",
    );
}

#[test]
fn throw_error_raises_the_interpreter_error() {
    same(
        "function f(a){ const k = a; try { k = 2; } catch (e) { return e.name + ':' + e.message; } return 'none'; }",
        "f(1)",
        "throw_error",
    );
    same(
        "function f(a){ const k = a; k = 2; }",
        "(function(){ try { f(1); } catch (e) { return e.name + ':' + e.message; } })()",
        "throw_error",
    );
}

#[test]
fn interrupts_inside_try_regions_stay_uncatchable() {
    forced_baseline(
        "function f(n){ let c = 0; for (;;) { try { c++; } catch (e) { c--; } } } f(1)",
    )
    .interrupt_after(3)
    .assert_uncatchable_interrupt();
    forced_baseline("function f(n){ let c = 0; for (;;) { try { c++; } finally { c--; } } } f(1)")
        .interrupt_after(3)
        .assert_uncatchable_interrupt();
}
