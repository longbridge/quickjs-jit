use rquickjs_jit::bytecode::{
    audited_opcode_policy_table, linked_opcode_table, tier1_policy, FallbackReason, HelperId,
    Tier1Policy, GENERATED_OPCODE_COUNT, GENERATED_OPCODE_FINGERPRINT,
};

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
fn policies_are_semantic_and_categorized() {
    let by_name = |name| {
        let opcode = linked_opcode_table()
            .find(|opcode| opcode.name() == name)
            .unwrap();
        tier1_policy(opcode.id()).unwrap()
    };

    assert_eq!(by_name("push_i32"), Tier1Policy::Native);
    assert_eq!(by_name("add"), Tier1Policy::Helper(HelperId::AddSlow));
    assert_eq!(
        by_name("get_field"),
        Tier1Policy::Helper(HelperId::GetProperty)
    );
    assert_eq!(by_name("call"), Tier1Policy::Helper(HelperId::Call));
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
