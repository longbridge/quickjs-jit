use rquickjs_jit::bytecode::{
    decode_raw, opcode, CompileSnapshot, DecodeError, DeoptPoint, OsrPoint, Resource, SlotKind,
    VerifierMetadata, VerifyErrorKind, VerifyLimits,
};

fn snapshot_from_parts(
    bytecode: Vec<u8>,
    arg_count: u16,
    local_count: u16,
    closure_count: u16,
    constant_count: u32,
) -> CompileSnapshot {
    CompileSnapshot::from_untrusted_bytecode(
        bytecode,
        arg_count,
        local_count,
        closure_count,
        constant_count,
    )
}

fn verifier_metadata(osr: Vec<OsrPoint>, deopt: Vec<DeoptPoint>) -> VerifierMetadata {
    VerifierMetadata::new(osr, deopt)
}

fn i32_operand(value: i32) -> [u8; 4] {
    value.to_le_bytes()
}

#[test]
fn decoder_rejects_unknown_opcodes_and_never_panics_on_arbitrary_bytes() {
    assert_eq!(
        decode_raw(&[u8::MAX]).unwrap_err(),
        DecodeError::UnknownOpcode {
            pc: 0,
            opcode: u8::MAX
        }
    );

    let mut state = 0x9e37_79b9_u32;
    for len in 0..=256 {
        let mut bytes = vec![0; len];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let result = std::panic::catch_unwind(|| decode_raw(&bytes));
        assert!(result.is_ok(), "decoder panicked for {bytes:02x?}");
        if let Ok(instructions) = result.unwrap() {
            let consumed: usize = instructions
                .iter()
                .map(|instruction| instruction.size())
                .sum();
            assert_eq!(consumed, bytes.len());
        }
    }
}

#[test]
fn verifier_never_panics_on_arbitrary_untrusted_snapshots() {
    let mut state = 0x243f_6a88_u32;
    for len in 0..=256 {
        let mut bytes = vec![0; len];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let snapshot = snapshot_from_parts(
            bytes.clone(),
            state as u16 & 7,
            (state >> 3) as u16 & 7,
            (state >> 6) as u16 & 7,
            (state >> 9) & 7,
        );
        let result = std::panic::catch_unwind(|| snapshot.verify(VerifyLimits::default()));
        assert!(result.is_ok(), "verifier panicked for {bytes:02x?}");
    }
}

#[test]
fn verifier_rejects_a_branch_into_an_operand() {
    let mut bytes = vec![opcode::PUSH_I32];
    bytes.extend(i32_operand(7));
    bytes.push(opcode::GOTO8);
    bytes.push((-5_i8) as u8); // operand PC 6 - 5 = byte 1, inside PUSH_I32
    let error = snapshot_from_parts(bytes, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.pc(), 5);
    assert_eq!(
        error.kind(),
        &VerifyErrorKind::BranchTargetInsideInstruction { target: 1 }
    );
}

#[test]
fn verifier_rejects_stack_underflow_and_inconsistent_merge_height() {
    let underflow = snapshot_from_parts(vec![opcode::ADD, opcode::RETURN], 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(underflow.pc(), 0);
    assert_eq!(
        underflow.kind(),
        &VerifyErrorKind::StackUnderflow {
            needed: 2,
            available: 0
        }
    );

    let bytes = vec![
        opcode::PUSH_TRUE,
        opcode::IF_FALSE8,
        3, // operand PC 2 + 3 = RETURN_UNDEF at 5
        opcode::PUSH_TRUE,
        opcode::NOP,
        opcode::RETURN_UNDEF,
    ];
    let merge = snapshot_from_parts(bytes, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(merge.pc(), 5);
    assert_eq!(
        merge.kind(),
        &VerifyErrorKind::InconsistentMergeHeight {
            expected: 0,
            actual: 1
        }
    );
}

#[test]
fn verifier_rejects_incompatible_merge_kinds_without_a_boxing_join() {
    let bytes = vec![
        opcode::PUSH_TRUE,
        opcode::IF_FALSE8,
        5, // operand PC 2 + 5 = PUSH_TRUE at 7
        opcode::PUSH_I8,
        1,
        opcode::GOTO8,
        3, // operand PC 6 + 3 = RETURN at 9
        opcode::PUSH_TRUE,
        opcode::NOP,
        opcode::RETURN,
    ];
    let error = snapshot_from_parts(bytes, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.pc(), 9);
    assert!(matches!(
        error.kind(),
        VerifyErrorKind::IncompatibleMergeKind { slot: 0, .. }
    ));
}

#[test]
fn explicit_boxing_on_both_paths_allows_a_tagged_join() {
    let bytes = vec![
        opcode::PUSH_TRUE,
        opcode::IF_FALSE8,
        6, // operand PC 2 + 6 = PUSH_TRUE at 8
        opcode::PUSH_I8,
        1,
        opcode::PLUS,
        opcode::GOTO8,
        3, // operand PC 7 + 3 = RETURN at 10
        opcode::PUSH_TRUE,
        opcode::PLUS,
        opcode::RETURN,
    ];
    assert!(snapshot_from_parts(bytes, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .is_ok());
}

#[test]
fn stack_copy_operations_do_not_implicitly_box_specialized_slots() {
    let bytes = vec![
        opcode::PUSH_TRUE,
        opcode::IF_FALSE8,
        7, // operand PC 2 + 7 = PUSH_TRUE at 9
        opcode::PUSH_I8,
        1,
        opcode::DUP,
        opcode::DROP,
        opcode::GOTO8,
        2, // operand PC 8 + 2 = RETURN at 10
        opcode::PUSH_TRUE,
        opcode::RETURN,
    ];
    let error = snapshot_from_parts(bytes, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.pc(), 10);
    assert!(matches!(
        error.kind(),
        VerifyErrorKind::IncompatibleMergeKind { slot: 0, .. }
    ));
}

#[test]
fn verifier_checks_local_arg_closure_and_constant_indices() {
    let fixtures = [
        (
            vec![opcode::GET_LOC, 1, 0, opcode::RETURN],
            VerifyErrorKind::LocalIndexOutOfRange { index: 1, count: 1 },
        ),
        (
            vec![opcode::GET_ARG, 1, 0, opcode::RETURN],
            VerifyErrorKind::ArgumentIndexOutOfRange { index: 1, count: 1 },
        ),
        (
            vec![opcode::GET_VAR_REF, 1, 0, opcode::RETURN],
            VerifyErrorKind::ClosureIndexOutOfRange { index: 1, count: 1 },
        ),
        (
            vec![opcode::PUSH_CONST8, 1, opcode::RETURN],
            VerifyErrorKind::ConstantIndexOutOfRange { index: 1, count: 1 },
        ),
        (
            vec![opcode::GET_LOC0_LOC1, opcode::RETURN],
            VerifyErrorKind::LocalIndexOutOfRange { index: 1, count: 1 },
        ),
    ];

    for (bytes, expected) in fixtures {
        let error = snapshot_from_parts(bytes, 1, 1, 1, 1)
            .verify(VerifyLimits::default())
            .unwrap_err();
        assert_eq!(error.kind(), &expected);
    }
}

#[test]
fn compact_double_local_preserves_its_authoritative_stack_effect() {
    let snapshot = snapshot_from_parts(
        vec![opcode::GET_LOC0_LOC1, opcode::ADD, opcode::RETURN],
        0,
        2,
        0,
        0,
    );
    assert!(snapshot.verify(VerifyLimits::default()).is_ok());
}

#[test]
fn set_local_uninitialized_records_the_conservative_slot_kind() {
    let metadata = verifier_metadata(
        vec![],
        vec![DeoptPoint::new(
            0,
            vec![SlotKind::Uninitialized],
            vec![SlotKind::Uninitialized],
        )],
    );
    let snapshot = snapshot_from_parts(
        vec![opcode::SET_LOC_UNINITIALIZED, 0, 0, opcode::RETURN_UNDEF],
        0,
        1,
        0,
        0,
    )
    .with_metadata(metadata);
    assert!(snapshot.verify(VerifyLimits::default()).is_ok());
}

#[test]
fn verifier_rejects_unsupported_exception_regions_before_ir() {
    let error = snapshot_from_parts(vec![opcode::CATCH, 0, 0, 0, 0, opcode::RETURN], 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.pc(), 0);
    assert_eq!(error.kind(), &VerifyErrorKind::UnsupportedExceptionRegion);
}

#[test]
fn verifier_rejects_invalid_osr_and_deopt_metadata_before_ir() {
    let osr = verifier_metadata(vec![OsrPoint::new(1, vec![SlotKind::Tagged])], vec![]);
    let error = snapshot_from_parts(vec![opcode::NOP, opcode::RETURN_UNDEF], 0, 0, 0, 0)
        .with_metadata(osr)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.kind(), &VerifyErrorKind::OsrPointNotLoopHeader);

    let deopt = verifier_metadata(vec![], vec![DeoptPoint::new(0, vec![], vec![])]);
    let error = snapshot_from_parts(vec![opcode::PUSH_TRUE, opcode::RETURN], 0, 0, 0, 0)
        .with_metadata(deopt)
        .verify(VerifyLimits::default())
        .unwrap_err();
    assert_eq!(error.kind(), &VerifyErrorKind::IncompleteDeoptState);
}

#[test]
fn every_verifier_resource_has_a_distinct_limit_error() {
    let base = snapshot_from_parts(vec![opcode::NOP, opcode::RETURN_UNDEF], 0, 0, 0, 0);
    let cases = [
        (
            VerifyLimits {
                max_snapshot_bytes: 1,
                ..VerifyLimits::default()
            },
            Resource::SnapshotBytes,
        ),
        (
            VerifyLimits {
                max_instructions: 1,
                ..VerifyLimits::default()
            },
            Resource::DecodedInstructions,
        ),
        (
            VerifyLimits {
                max_basic_blocks: 0,
                ..VerifyLimits::default()
            },
            Resource::BasicBlocks,
        ),
        (
            VerifyLimits {
                max_metadata_bytes: 0,
                ..VerifyLimits::default()
            },
            Resource::MetadataBytes,
        ),
        (
            VerifyLimits {
                max_work_units: 1,
                ..VerifyLimits::default()
            },
            Resource::WorkUnits,
        ),
    ];

    for (limits, resource) in cases {
        let error = base.verify(limits).unwrap_err();
        assert_eq!(error.kind(), &VerifyErrorKind::ResourceLimit { resource });
    }
}
