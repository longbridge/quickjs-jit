#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs_jit::test_support::differential;

#[test]
fn ordinary_synchronous_programs_enter_tier1_and_match_the_interpreter() {
    let programs = [
        ("function f(a,b){ return a+b }", "f(20,22)"),
        ("function f(a){ let x=a; x++; return x }", "f(41)"),
        (
            "function f(a){ if(a) return 1; return 2 }",
            "[f(true),f(false)]",
        ),
        ("function f(o){ return o.answer }", "f({answer:42})"),
        ("function f(){ return [20,22] }", "f()"),
    ];

    for (definition, expression) in programs {
        differential(definition, expression)
            .force_baseline()
            .assert_same();
    }
}

#[test]
fn coercion_exception_and_gc_visible_ownership_match() {
    differential(
        "const events=[]; function f(a,b){ return a+b }",
        "f({[Symbol.toPrimitive](){events.push('a');return 20}}, {[Symbol.toPrimitive](){events.push('b');return 22}}) + ':' + events.join(',')",
    )
    .force_baseline()
    .assert_same();

    differential(
        "function f(o){ return [o,o] }",
        "(() => { const o={x:42}; const a=f(o); return a[0]===a[1] && a[0].x })()",
    )
    .force_baseline()
    .assert_same();
}
