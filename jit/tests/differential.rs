#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs_jit::bytecode::FallbackReason;
use rquickjs_jit::bytecode::HelperId;
use rquickjs_jit::test_support::{assert_tier1_rejected, differential};
use serde::Deserialize;

#[derive(Deserialize)]
struct OpcodeManifest {
    cases: Vec<OpcodeCase>,
}

#[derive(Deserialize)]
struct OpcodeCase {
    opcode: String,
    definition: String,
    expression: String,
    helper: Option<String>,
}

fn helper(name: &str) -> HelperId {
    match name {
        "Dup" => HelperId::Dup,
        "Free" => HelperId::Free,
        "ResolveConst" => HelperId::ResolveConst,
        "ToNumeric" => HelperId::ToNumeric,
        "ToBool" => HelperId::ToBool,
        "AddSlow" => HelperId::AddSlow,
        "CompareSlow" => HelperId::CompareSlow,
        "GetProperty" => HelperId::GetProperty,
        "SetProperty" => HelperId::SetProperty,
        "Call" => HelperId::Call,
        "NewArray" => HelperId::NewArray,
        "NewObject" => HelperId::NewObject,
        _ => panic!("unknown manifest helper {name}"),
    }
}

#[test]
fn manifest_executes_every_advertised_opcode_at_its_native_pc() {
    let manifest: OpcodeManifest =
        serde_json::from_str(include_str!("fixtures/opcode-cases.json")).unwrap();
    for case in manifest.cases {
        let mut run = differential(&case.definition, &case.expression)
            .force_baseline()
            .expect_executed_opcode(&case.opcode);
        if let Some(expected) = case.helper.as_deref() {
            run = run.expect_helper(helper(expected));
        }
        run.assert_same();
    }
}

#[test]
fn rejected_programs_have_exact_fallback_and_interpreter_semantics() {
    assert_tier1_rejected(
        "function f(a,b){ return a-b }",
        "f(44,2)",
        FallbackReason::UnsupportedOpcode,
    );
}

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
            .expect_executed_opcode(match expression {
                "f(20,22)" => "add",
                "f(41)" => "post_inc",
                "[f(true),f(false)]" => "if_false8",
                "f({answer:42})" => "get_field",
                "f()" => "array_from",
                _ => unreachable!(),
            })
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
    .expect_executed_opcode("add")
    .expect_helper(HelperId::AddSlow)
    .assert_same();

    differential(
        "function f(o){ return [o,o] }",
        "(() => { const o={x:42}; const a=f(o); return a[0]===a[1] && a[0].x })()",
    )
    .force_baseline()
    .assert_same();
}

#[test]
fn every_advertised_helper_family_has_a_real_native_execution_case() {
    let cases = [
        (
            "function f(a){ return +a }",
            "f({valueOf(){return 42}})",
            "plus",
            HelperId::ToNumeric,
        ),
        (
            "function f(a,b){ return a < b }",
            "f({valueOf(){return 20}},22)",
            "lt",
            HelperId::CompareSlow,
        ),
        (
            "function f(o,a){ o.answer=a; return o.answer }",
            "f({},42)",
            "put_field",
            HelperId::SetProperty,
        ),
        (
            "function f(g,a){ let x=g(a); return x+0 }",
            "f(x=>x+1,41)",
            "call1",
            HelperId::Call,
        ),
        (
            "function f(){ return {} }",
            "f()",
            "object",
            HelperId::NewObject,
        ),
    ];
    for (definition, expression, opcode, helper) in cases {
        differential(definition, expression)
            .force_baseline()
            .expect_executed_opcode(opcode)
            .expect_helper(helper)
            .assert_same();
    }
}
