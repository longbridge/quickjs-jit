#![cfg(all(
    feature = "test-support",
    feature = "compiler",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs_jit::test_support::{differential, forced_baseline};

#[test]
fn compiled_values_have_interpreter_ownership() {
    differential("function f(o) { let x=o; return [x,x] }", "f({a:1})").assert_same();
}

#[test]
fn compiled_constant_outlives_the_worker_snapshot_and_registry_detaches() {
    // The forced backend compiles from a pointer-free snapshot, drops it
    // before this call, and retains the active bytecode only through the
    // RawRuntime-owned function registry. DifferentialRun also verifies that
    // detaching the guard makes the weak registry handle report detached.
    differential(
        "function f() { return 123456789012345678901234567890n }",
        "String(f())",
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn thrown_getter_is_caught_at_the_same_handler() {
    differential(
        "function f(o){ return o.x }",
        "(() => { try { return f({get x(){throw new Error('boom')}}) } catch(e) { return e.message } })()",
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn symbol_to_primitive_addition_preserves_event_order() {
    differential(
        r#"
        const events=[];
        function f(left, right){ return left + right }
        "#,
        r#"({
            value: f(
                { [Symbol.toPrimitive]() { events.push('left'); return 20 } },
                { [Symbol.toPrimitive]() { events.push('right'); return 22 } }
            ),
            events
        })"#,
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn thrown_symbol_to_primitive_preserves_left_to_right_event_order() {
    differential(
        r#"
        const events=[];
        function f(left, right){ return left + right }
        "#,
        r#"(() => {
            try {
                f(
                    { [Symbol.toPrimitive]() { events.push('left'); return 20 } },
                    { [Symbol.toPrimitive]() { events.push('right'); throw new Error('boom') } }
                );
            } catch (error) {
                return { message: error.message, events };
            }
        })()"#,
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn unary_plus_rejects_bigint_in_native_code() {
    differential(
        "function f(value){ return +value }",
        "(() => { try { return f(1n) } catch (error) { return `${error.name}:${error.message}` } })()",
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn thrown_reentrant_call_clears_scratch_before_exception_transfer() {
    differential(
        "function f(callback, value){ callback(value); return 1 }",
        "(() => { try { f(() => { throw new Error('call boom') }, {marker: 1}) } catch (error) { return error.message } })()",
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn interrupt_stops_compiled_loop() {
    forced_baseline("function f(){for(;;){}} f()")
        .interrupt_after(100)
        .assert_uncatchable_interrupt();
}
