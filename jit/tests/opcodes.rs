use rquickjs_jit::bytecode::{
    audited_opcode_policy_table, linked_opcode_table, tier1_policy, FallbackReason, HelperId,
    Tier1Policy, GENERATED_OPCODE_COUNT, GENERATED_OPCODE_FINGERPRINT,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[test]
fn synthetic_function_identity_cannot_bypass_the_closed_policy() {
    // `typeof` has no Tier 1 lowering and stays a policy reject; the synthetic
    // ID-zero function must not bypass that row.
    let mut bytecode = vec![
        rquickjs_jit::bytecode::opcode::PUSH_UNDEFINED,
        linked_opcode_table()
            .find(|opcode| opcode.name() == "typeof")
            .expect("linked typeof opcode")
            .id(),
    ];
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
        by_name("typeof"),
        Tier1Policy::Reject(FallbackReason::UnsupportedOpcode)
    );
}

#[test]
fn m2_core_opcodes_are_advertised_with_their_lowering_family() {
    let by_name = |name| {
        let opcode = linked_opcode_table()
            .find(|opcode| opcode.name() == name)
            .unwrap();
        tier1_policy(opcode.id()).unwrap()
    };

    for native in [
        "push_minus1",
        "push_4",
        "push_5",
        "push_6",
        "push_7",
        "push_i16",
        "null",
        "goto",
        "perm3",
        "post_dec",
        "inc_loc",
        "dec_loc",
        "add_loc",
        "xor",
        "shl",
        "sar",
        "shr",
    ] {
        assert_eq!(by_name(native), Tier1Policy::Native, "{native}");
    }
    assert_eq!(
        by_name("push_const"),
        Tier1Policy::Helper(HelperId::ResolveConst)
    );
    for dup in ["get_loc", "get_loc0_loc1", "insert2", "insert3"] {
        assert_eq!(by_name(dup), Tier1Policy::Helper(HelperId::Dup), "{dup}");
    }
    for free in [
        "put_loc",
        "set_loc",
        "put_arg",
        "put_arg0",
        "put_arg1",
        "put_arg2",
        "put_arg3",
        "set_arg",
        "set_arg0",
        "set_arg1",
        "set_arg2",
        "set_arg3",
        "nip",
        "is_undefined",
        "is_null",
    ] {
        assert_eq!(by_name(free), Tier1Policy::Helper(HelperId::Free), "{free}");
    }
    assert_eq!(by_name("lnot"), Tier1Policy::Helper(HelperId::ToBool));
    assert_eq!(
        by_name("mod"),
        Tier1Policy::Helper(HelperId::BinaryArithSlow)
    );
    for unary in ["neg", "inc", "dec", "not"] {
        assert_eq!(
            by_name(unary),
            Tier1Policy::Helper(HelperId::UnaryArithSlow),
            "{unary}"
        );
    }
    for compare in ["lte", "gt", "gte", "eq", "neq", "strict_eq", "strict_neq"] {
        assert_eq!(
            by_name(compare),
            Tier1Policy::Helper(HelperId::CompareSlow),
            "{compare}"
        );
    }
    assert_eq!(
        by_name("push_empty_string"),
        Tier1Policy::Helper(HelperId::AtomValue)
    );
    assert_eq!(by_name("tail_call"), Tier1Policy::Helper(HelperId::Call));
    assert_eq!(
        by_name("tail_call_method"),
        Tier1Policy::Helper(HelperId::Call)
    );

    // Stack shuffles that ordinary Tier 1 eligible source cannot reach stay
    // rejected: `nop`/`nip1` are never emitted by the QuickJS compiler,
    // `dup1`/`swap2`/`rot3r`/`rot3l` need destructuring or for-in lvalues
    // (`to_object`, `for_in_start`), `dup2`/`perm4` need `to_propkey2`, and
    // `dup3`/`insert4`/`perm5`/`rot4l`/`rot5l` need `super` lvalues.
    for rejected in [
        "nop", "nip1", "dup1", "dup2", "dup3", "insert4", "perm4", "perm5", "swap2", "rot3l",
        "rot3r", "rot4l", "rot5l",
    ] {
        assert_eq!(
            by_name(rejected),
            Tier1Policy::Reject(FallbackReason::UnsupportedOpcode),
            "{rejected}"
        );
    }
}
