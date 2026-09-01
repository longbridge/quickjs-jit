use rquickjs_jit::bytecode::{
    audited_opcode_policy_table, linked_opcode_table, tier1_policy, FallbackReason, HelperId,
    Tier1Policy, GENERATED_OPCODE_COUNT, GENERATED_OPCODE_FINGERPRINT,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[test]
fn synthetic_function_identity_cannot_bypass_the_closed_policy() {
    let mut bytecode = vec![linked_opcode_table()
        .find(|opcode| opcode.name() == "push_minus1")
        .expect("linked push_minus1 opcode")
        .id()];
    bytecode.push(rquickjs_jit::bytecode::opcode::RETURN);
    let verified = rquickjs_jit::test_support::verified_bytecode(bytecode, 0, 0);
    let rejection = verified
        .tier1_eligibility()
        .expect_err("ID-zero synthetic bytecode remains subject to production policy");
    assert_eq!(rejection.reason(), FallbackReason::UnsupportedOpcode);
}

#[derive(Deserialize)]
struct Manifest {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    opcode: String,
}

#[test]
fn every_authoritative_opcode_has_exactly_one_generated_policy() {
    let linked: Vec<_> = linked_opcode_table().collect();
    assert_eq!(linked.len(), 252);
    assert_eq!(GENERATED_OPCODE_COUNT, linked.len());
    assert_eq!(GENERATED_OPCODE_FINGERPRINT, 0x05d5_c086_7521_c077);

    for (expected_id, opcode) in linked.into_iter().enumerate() {
        assert_eq!(opcode.id() as usize, expected_id);
        let policy = tier1_policy(opcode.id()).unwrap_or_else(|| {
            panic!(
                "missing Tier 1 policy for {} ({})",
                opcode.name(),
                opcode.id()
            )
        });
        assert!(matches!(
            policy,
            Tier1Policy::Native | Tier1Policy::Helper(_) | Tier1Policy::Reject(_)
        ));
    }
    assert_eq!(tier1_policy(252), None);
}

#[test]
fn audited_policy_is_a_closed_per_id_table_not_a_name_default() {
    let linked: Vec<_> = linked_opcode_table().collect();
    let audited = audited_opcode_policy_table();
    assert_eq!(audited.len(), linked.len());
    for (id, (opcode, audited_entry)) in linked.iter().zip(audited).enumerate() {
        assert_eq!(audited_entry.id as usize, id);
        assert_eq!(audited_entry.name, opcode.name());
        assert_eq!(tier1_policy(opcode.id()), Some(audited_entry.policy));
    }
}

#[test]
fn manifest_has_exactly_one_case_for_every_advertised_opcode() {
    let manifest: Manifest =
        serde_json::from_str(include_str!("fixtures/opcode-cases.json")).unwrap();
    let cases: BTreeSet<_> = manifest
        .cases
        .iter()
        .map(|case| case.opcode.as_str())
        .collect();
    assert_eq!(
        cases.len(),
        manifest.cases.len(),
        "duplicate opcode manifest case"
    );
    let advertised: BTreeSet<_> = audited_opcode_policy_table()
        .iter()
        .filter(|entry| !matches!(entry.policy, Tier1Policy::Reject(_)))
        .map(|entry| entry.name)
        .collect();
    assert_eq!(cases, advertised);
}

#[test]
fn policies_are_semantic_and_categorized() {
    let by_name = |name| {
        let opcode = linked_opcode_table()
            .find(|opcode| opcode.name() == name)
            .unwrap();
        tier1_policy(opcode.id()).unwrap()
    };

    assert_eq!(by_name("push_0"), Tier1Policy::Native);
    assert_eq!(by_name("add"), Tier1Policy::Helper(HelperId::AddSlow));
    assert_eq!(
        by_name("get_field"),
        Tier1Policy::Helper(HelperId::GetProperty)
    );
    assert_eq!(by_name("get_var"), Tier1Policy::Helper(HelperId::GetGlobal));
    assert_eq!(by_name("call1"), Tier1Policy::Helper(HelperId::Call));
    assert_eq!(
        by_name("call_constructor"),
        Tier1Policy::Helper(HelperId::CallConstructor)
    );
    assert_eq!(by_name("regexp"), Tier1Policy::Helper(HelperId::Regexp));
    assert_eq!(
        by_name("eval"),
        Tier1Policy::Reject(FallbackReason::DirectEval)
    );
    assert_eq!(
        by_name("with_get_var"),
        Tier1Policy::Reject(FallbackReason::WithScope)
    );
    assert_eq!(
        by_name("yield"),
        Tier1Policy::Reject(FallbackReason::Generator)
    );
    assert_eq!(by_name("await"), Tier1Policy::Reject(FallbackReason::Async));
}

#[test]
fn unsupported_frame_and_exception_families_are_stable_rejects() {
    let by_name = |name| {
        let opcode = linked_opcode_table()
            .find(|opcode| opcode.name() == name)
            .unwrap();
        tier1_policy(opcode.id()).unwrap()
    };

    assert_eq!(
        by_name("get_var_ref"),
        Tier1Policy::Reject(FallbackReason::ClosureFrame)
    );
    assert_eq!(
        by_name("push_this"),
        Tier1Policy::Reject(FallbackReason::ExtendedFrame)
    );
    assert_eq!(
        by_name("catch"),
        Tier1Policy::Reject(FallbackReason::ExceptionRegion)
    );
    assert_eq!(
        by_name("tail_call"),
        Tier1Policy::Reject(FallbackReason::UnsupportedOpcode)
    );
}
