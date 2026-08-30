#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::bytecode::{
    linked_opcode_table, tier1_policy, FallbackReason, HelperId, Tier1Policy,
};
use rquickjs_jit::test_support::{assert_tier1_rejected, differential};
use rquickjs_jit::{
    correctness::{canonical_observation_source, StructuredProgram},
    JitRuntime,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct OpcodeManifest {
    cases: Vec<OpcodeCase>,
}

#[derive(Deserialize)]
struct OpcodeCase {
    opcode: String,
    definition: String,
    expression: String,
    helper: Option<ManifestHelper>,
    dimensions: BTreeSet<Dimension>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
enum Dimension {
    Normal,
    NumericTagEdge,
    ExceptionOrNonthrow,
    OwnershipGc,
    CoercionReentrancy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum ManifestHelper {
    Dup,
    Free,
    ResolveConst,
    ToNumeric,
    ToBool,
    AddSlow,
    CompareSlow,
    GetProperty,
    SetProperty,
    Call,
    NewArray,
    NewObject,
}

impl ManifestHelper {
    const fn id(self) -> HelperId {
        match self {
            Self::Dup => HelperId::Dup,
            Self::Free => HelperId::Free,
            Self::ResolveConst => HelperId::ResolveConst,
            Self::ToNumeric => HelperId::ToNumeric,
            Self::ToBool => HelperId::ToBool,
            Self::AddSlow => HelperId::AddSlow,
            Self::CompareSlow => HelperId::CompareSlow,
            Self::GetProperty => HelperId::GetProperty,
            Self::SetProperty => HelperId::SetProperty,
            Self::Call => HelperId::Call,
            Self::NewArray => HelperId::NewArray,
            Self::NewObject => HelperId::NewObject,
        }
    }
}

fn required_dimensions(case: &OpcodeCase) -> BTreeSet<Dimension> {
    let mut required = BTreeSet::from([Dimension::Normal, Dimension::ExceptionOrNonthrow]);
    let opcode = linked_opcode_table()
        .find(|opcode| opcode.name() == case.opcode)
        .expect("manifest opcode is linked");
    if matches!(tier1_policy(opcode.id()), Some(Tier1Policy::Helper(_))) {
        required.insert(Dimension::OwnershipGc);
    }
    if matches!(case.opcode.as_str(), "plus" | "post_inc" | "add" | "lt") {
        required.insert(Dimension::NumericTagEdge);
    }
    if matches!(
        case.helper,
        Some(
            ManifestHelper::ToNumeric
                | ManifestHelper::AddSlow
                | ManifestHelper::CompareSlow
                | ManifestHelper::GetProperty
                | ManifestHelper::SetProperty
                | ManifestHelper::Call
        )
    ) {
        required.insert(Dimension::CoercionReentrancy);
    }
    required
}

fn validate_dimensions(manifest: &OpcodeManifest) -> Result<(), String> {
    for case in &manifest.cases {
        let opcode = linked_opcode_table()
            .find(|opcode| opcode.name() == case.opcode)
            .ok_or_else(|| format!("unknown opcode {}", case.opcode))?;
        let expected_helper = match tier1_policy(opcode.id()) {
            Some(Tier1Policy::Helper(helper)) => Some(helper),
            Some(Tier1Policy::Native) => None,
            _ => return Err(format!("{} is not advertised", case.opcode)),
        };
        if case.helper.map(ManifestHelper::id) != expected_helper {
            return Err(format!("{} helper does not match policy", case.opcode));
        }
        let missing: Vec<_> = required_dimensions(case)
            .difference(&case.dimensions)
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(format!("{} missing {missing:?}", case.opcode));
        }
        let extra: Vec<_> = case
            .dimensions
            .difference(&required_dimensions(case))
            .copied()
            .collect();
        if !extra.is_empty() {
            return Err(format!("{} has inapplicable {extra:?}", case.opcode));
        }
    }
    Ok(())
}

#[test]
fn manifest_executes_every_advertised_opcode_at_its_native_pc() {
    let manifest: OpcodeManifest =
        serde_json::from_str(include_str!("fixtures/opcode-cases.json")).unwrap();
    validate_dimensions(&manifest).expect("manifest has semantic dimension evidence");
    for case in manifest.cases {
        let mut run = differential(&case.definition, &case.expression)
            .force_baseline()
            .expect_executed_opcode(&case.opcode);
        if case.dimensions.contains(&Dimension::OwnershipGc) {
            run = run.stress_gc();
        }
        if let Some(expected) = case.helper {
            run = run.expect_helper(expected.id());
        }
        run.assert_same();
    }
}

#[test]
fn manifest_dimension_schema_is_closed_and_required_dimensions_are_mechanical() {
    let source = include_str!("fixtures/opcode-cases.json");
    let manifest: OpcodeManifest = serde_json::from_str(source).unwrap();
    validate_dimensions(&manifest).unwrap();

    let mut value: serde_json::Value = serde_json::from_str(source).unwrap();
    value["cases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|case| case["opcode"] == "add")
        .unwrap()["dimensions"]
        .as_array_mut()
        .unwrap()
        .retain(|dimension| dimension != "coercion-reentrancy");
    let missing: OpcodeManifest = serde_json::from_value(value.clone()).unwrap();
    assert!(validate_dimensions(&missing).unwrap_err().contains("add"));

    value["cases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|case| case["opcode"] == "add")
        .unwrap()["dimensions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("invented-dimension"));
    assert!(serde_json::from_value::<OpcodeManifest>(value).is_err());
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

#[test]
fn seeded_structured_programs_match_interpreter_and_automatic_modes() {
    for seed in 0..64 {
        let program = StructuredProgram::generate(seed, 32);
        let definition = "function f(g,x){return g(x)+0}";
        let invocation = format!("f(()=>{},0)", program.source());
        differential(definition, &invocation)
            .force_baseline()
            .expect_executed_opcode("call1")
            .expect_helper(HelperId::Call)
            .assert_same();
        let interpreter = Runtime::new().unwrap();
        let expected = Context::full(&interpreter).unwrap().with(|ctx| {
            ctx.eval::<(), _>(definition).unwrap();
            ctx.eval::<String, _>(canonical_observation_source(&invocation))
                .unwrap()
        });
        let automatic = JitRuntime::builder()
            .config(
                rquickjs_jit::JitConfig::builder()
                    .call_threshold(1)
                    .loop_threshold(1)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        let automatic_context = Context::full(&automatic).unwrap();
        automatic_context.with(|ctx| ctx.eval::<(), _>(definition).unwrap());
        let warm = format!("for(let i=0;i<256;i++){{{invocation};}}");
        for _ in 0..128 {
            automatic_context.with(|ctx| ctx.eval::<(), _>(warm.as_str()).unwrap());
            automatic.jit().poll();
            if automatic.metrics().native_entries > 0 {
                break;
            }
        }
        let actual = automatic_context.with(|ctx| {
            ctx.eval::<String, _>(canonical_observation_source(&invocation))
                .unwrap()
        });
        let metrics = automatic.metrics();
        assert!(
            metrics.native_entries > 0,
            "seed {seed} never entered automatic native code: {metrics:?}"
        );
        assert_eq!(
            metrics.native_fallbacks, 0,
            "seed {seed} automatic fallback: {metrics:?}"
        );
        assert_eq!(
            actual,
            expected,
            "seed {seed}; minimize with fuel {}",
            program.fuel()
        );
    }
}

#[test]
fn seeded_structured_programs_enter_optimized_mode_with_native_evidence() {
    for seed in 0..16 {
        let program = StructuredProgram::generate(seed, 32);
        let definition = "function f(){return 2}";
        let invocation = format!("(()=>{{f();return {};}})()", program.source());
        let interpreter = Runtime::new().unwrap();
        let expected = Context::full(&interpreter).unwrap().with(|ctx| {
            ctx.eval::<(), _>(definition).unwrap();
            ctx.eval::<String, _>(canonical_observation_source(&invocation))
                .unwrap()
        });
        let config = rquickjs_jit::JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .force_optimized_for_test(true)
            .build()
            .unwrap();
        let optimized = JitRuntime::builder().config(config).build().unwrap();
        let context = Context::full(&optimized).unwrap();
        context.with(|ctx| ctx.eval::<(), _>(definition).unwrap());
        let warm = "for(let i=0;i<256;i++){f();}";
        for _ in 0..128 {
            context.with(|ctx| ctx.eval::<(), _>(warm).unwrap());
            optimized.jit().poll();
            if optimized.metrics().tier2_entries > 0 {
                break;
            }
        }
        let actual = context.with(|ctx| {
            ctx.eval::<String, _>(canonical_observation_source(&invocation))
                .unwrap()
        });
        let metrics = optimized.metrics();
        assert!(
            metrics.tier2_entries > 0,
            "seed {seed} never entered Tier2: {metrics:?}"
        );
        assert_eq!(
            metrics.native_fallbacks, 0,
            "seed {seed} optimized fallback: {metrics:?}"
        );
        assert_eq!(actual, expected, "optimized seed {seed}");
    }
}
