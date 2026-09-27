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

use rquickjs::{Context, Runtime};
use rquickjs_jit::test_support::{differential, forced_baseline};
use rquickjs_jit::{Jit, JitConfig, JitMetrics};

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
fn observable_backtrace_hooks_run_for_natively_caught_primitives() {
    // The interpreter builds a backtrace for every caught primitive. It is
    // skipped natively only when building it cannot run user code; these
    // hooks make it observable and must fire exactly as in the interpreter.
    same(
        "var calls = 0; Error.prepareStackTrace = function(e, s) { calls++; return 'trace'; };\nfunction f(n){ let c = 0; for (let i = 0; i < n; i++) { try { if (i % 4 === 0) throw i; } catch (v) { c -= -v; } } return c + ':' + calls; }",
        "f(40)",
        "neg",
    );
    same(
        "var calls = 0; Error.stackTraceLimit = { valueOf() { calls++; return 3; } };\nfunction f(n){ let c = 0; for (let i = 0; i < n; i++) { try { if (i % 4 === 0) throw {i}; } catch (v) { c -= -v.i; } } return c + ':' + calls; }",
        "f(40)",
        "neg",
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

/// Runs `source` then evaluates `expression` `rounds` times with and without
/// the production JIT, returning both results and the JIT metrics.
fn production(source: &str, expression: &str, rounds: usize) -> (String, String, JitMetrics) {
    let run = |jit: bool| {
        let runtime = Runtime::new().unwrap();
        let attached = jit.then(|| Jit::attach(&runtime, JitConfig::default()).unwrap());
        let context = Context::full(&runtime).unwrap();
        let mut result = String::new();
        context.with(|ctx| ctx.eval::<(), _>(source).unwrap());
        for _ in 0..rounds {
            result = context.with(|ctx| ctx.eval::<String, _>(expression).unwrap());
            while runtime.is_job_pending() {
                runtime.execute_pending_job().unwrap();
            }
            if let Some(jit) = &attached {
                jit.poll();
            }
        }
        (
            result,
            attached.map(|jit| jit.metrics()).unwrap_or_default(),
        )
    };
    let (interpreted, _) = run(false);
    let (compiled, metrics) = run(true);
    (interpreted, compiled, metrics)
}

#[test]
fn production_tiering_runs_hot_exception_loops_natively() {
    let (interpreted, compiled, metrics) = production(
        "function f(n){ let caught=0, finals=0; for(let i=0;i<n;i++){ try { if((i&15)===0) throw i; caught-=1; } catch (value) { caught+=value; } finally { finals++; } } return caught + ':' + finals; }",
        "f(4000)",
        40,
    );
    assert_eq!(compiled, interpreted);
    assert!(metrics.native_entries > 0, "{metrics:?}");
    assert_eq!(metrics.native_retries, 0, "{metrics:?}");
    assert_eq!(metrics.invalid_artifacts, 0, "{metrics:?}");
}

#[test]
fn osr_enters_a_loop_header_inside_a_try_region() {
    // The first invocation is long enough to enter the loop through OSR with
    // the catch offset live on the interpreter operand stack.
    let (interpreted, compiled, metrics) = production(
        "function f(n){ let s = 0; try { for (let i = 0; i < n; i++) { s = (s + i) | 0; if (i === n - 3) throw s; } } catch (e) { return 'caught:' + e; } return 'done:' + s; }",
        "f(3000000)",
        1,
    );
    assert_eq!(compiled, interpreted);
    assert!(metrics.osr_entries >= 1, "{metrics:?}");
    assert_eq!(metrics.native_retries, 0, "{metrics:?}");
}

#[test]
fn async_functions_stop_requesting_refused_snapshots() {
    // Async and generator frames can never compile. Their refusal is local to
    // the bytecode, so hot probes must stop instead of re-requesting a
    // snapshot at every call, resume and loop poll.
    let (interpreted, compiled, metrics) = production(
        "async function step(v){ return (await Promise.resolve(v + 1)) * 3; }\n\
         async function g(n){ let s = 0; for (let i = 0; i < n; i++) { try { if ((i & 15) === 0) throw i; } catch (e) { s += e; } } for (let i = 0; i < 8; i++) s = await step(s & 0xffff); globalThis.out = String(s); }\n\
         function* gen(n){ for (let i = 0; i < n; i++) yield i; }\n\
         function h(){ let t = 0; for (const v of gen(500)) t += v; return t; }",
        "(g(2000), String(h()))",
        60,
    );
    assert_eq!(compiled, interpreted);
    assert!(
        metrics.snapshot_requests <= 8,
        "refused async/generator snapshots were re-requested: {metrics:?}"
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
