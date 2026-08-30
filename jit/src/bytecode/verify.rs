use super::{
    cfg,
    decode::{decode_bounded, BoundedDecodeError},
    stack::{self, SlotKind, StateProof},
    CompileSnapshot, ControlFlowGraph, DecodeError, Instruction, OperandFormat, SnapshotStatus,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    SnapshotBytes,
    DecodedInstructions,
    BasicBlocks,
    MetadataBytes,
    WorkUnits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyLimits {
    pub max_snapshot_bytes: usize,
    pub max_instructions: usize,
    pub max_basic_blocks: usize,
    pub max_metadata_bytes: usize,
    pub max_work_units: usize,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        Self {
            max_snapshot_bytes: 16 * 1024 * 1024,
            max_instructions: 1_000_000,
            max_basic_blocks: 65_536,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_work_units: 10_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyError {
    pub pc: u32,
    pub kind: VerifyErrorKind,
}

impl VerifyError {
    pub(crate) const fn new(pc: u32, kind: VerifyErrorKind) -> Self {
        Self { pc, kind }
    }

    pub const fn pc(&self) -> u32 {
        self.pc
    }

    pub const fn kind(&self) -> &VerifyErrorKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerifyErrorKind {
    Decode(DecodeError),
    IncompatibleSnapshot,
    EmptyBytecode,
    MissingTerminator,
    UnreachableInstruction,
    BranchTargetOutOfRange {
        target: i64,
    },
    BranchTargetInsideInstruction {
        target: u32,
    },
    StackUnderflow {
        needed: usize,
        available: usize,
    },
    StackSizeExceeded {
        declared: u16,
        actual: usize,
    },
    InconsistentMergeHeight {
        expected: usize,
        actual: usize,
    },
    IncompatibleMergeKind {
        slot: usize,
        expected: SlotKind,
        actual: SlotKind,
    },
    LocalIndexOutOfRange {
        index: u32,
        count: u32,
    },
    ArgumentIndexOutOfRange {
        index: u32,
        count: u32,
    },
    ClosureIndexOutOfRange {
        index: u32,
        count: u32,
    },
    ConstantIndexOutOfRange {
        index: u32,
        count: u32,
    },
    UnsupportedExceptionRegion,
    UnsupportedFunctionKind(SnapshotStatus),
    OsrPointNotInstructionBoundary,
    OsrPointNotLoopHeader,
    IncompleteOsrState,
    DeoptPointNotInstructionBoundary,
    IncompleteDeoptState,
    ResourceLimit {
        resource: Resource,
    },
}

#[derive(Clone, Debug)]
pub struct VerifiedFunction {
    snapshot: CompileSnapshot,
    instructions: Vec<Instruction>,
    cfg: ControlFlowGraph,
}

impl VerifiedFunction {
    pub fn snapshot(&self) -> &CompileSnapshot {
        &self.snapshot
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub const fn control_flow_graph(&self) -> &ControlFlowGraph {
        &self.cfg
    }

    pub fn tier1_eligibility(&self) -> Result<(), super::Tier1Rejection> {
        if !self.snapshot.exception_map().is_empty() {
            return Err(super::Tier1Rejection::new(
                0,
                super::FallbackReason::ExceptionRegion,
            ));
        }
        let mut unsupported = None;
        for instruction in &self.instructions {
            if let super::Tier1Policy::Reject(reason) =
                super::tier1_policy(instruction.opcode().id())
                    .expect("verified opcode belongs to the generated table")
            {
                #[cfg(feature = "test-support")]
                if self.snapshot.function_id() == 0
                    && reason == super::FallbackReason::UnsupportedOpcode
                {
                    // Synthetic compiler unit fixtures exercise implemented
                    // lowerings independently of the production advertised
                    // policy. Captured/runtime functions never have ID zero.
                    continue;
                }
                let rejection = super::Tier1Rejection::new(instruction.pc(), reason);
                if reason == super::FallbackReason::UnsupportedOpcode {
                    unsupported.get_or_insert(rejection);
                } else {
                    return Err(rejection);
                }
            }
        }
        unsupported.map_or(Ok(()), Err)
    }
}

fn decode_error_pc(error: &DecodeError) -> u32 {
    match error {
        DecodeError::UnknownOpcode { pc, .. }
        | DecodeError::InvalidOpcode { pc }
        | DecodeError::Truncated { pc, .. } => *pc,
    }
}

fn short_index(instruction: &Instruction) -> Option<u32> {
    instruction
        .opcode()
        .name()
        .as_bytes()
        .last()
        .and_then(|value| value.is_ascii_digit().then_some(u32::from(value - b'0')))
}

fn indexed_operand(instruction: &Instruction) -> Option<(IndexSpace, u32, u32)> {
    let format = instruction.opcode().format();
    let name = instruction.opcode().name();
    if format == OperandFormat::AtomU16 {
        let space = match name {
            "make_loc_ref" => IndexSpace::Local,
            "make_arg_ref" => IndexSpace::Argument,
            "make_var_ref_ref" => IndexSpace::Closure,
            _ => return None,
        };
        return Some((space, u32::from(instruction.operand_u16(5)), 1));
    }
    let space = match format {
        OperandFormat::Local | OperandFormat::Local8 | OperandFormat::NoneLocal => {
            IndexSpace::Local
        }
        OperandFormat::Argument | OperandFormat::NoneArgument => IndexSpace::Argument,
        OperandFormat::Closure | OperandFormat::NoneClosure => IndexSpace::Closure,
        OperandFormat::Constant | OperandFormat::Constant8 => IndexSpace::Constant,
        _ => return None,
    };
    let index = match format {
        OperandFormat::Local | OperandFormat::Argument | OperandFormat::Closure => {
            u32::from(instruction.operand_u16(1))
        }
        OperandFormat::Local8 | OperandFormat::Constant8 => u32::from(instruction.operand_u8(1)),
        OperandFormat::Constant => instruction.operand_u32(1),
        OperandFormat::NoneLocal | OperandFormat::NoneArgument | OperandFormat::NoneClosure => {
            short_index(instruction).expect("compact index opcode ends in a digit")
        }
        _ => unreachable!(),
    };
    let width = u32::from(matches!(name, "using_dispose" | "using_dispose_async")) + 1;
    Some((space, index, width))
}

#[derive(Clone, Copy)]
enum IndexSpace {
    Local,
    Argument,
    Closure,
    Constant,
}

fn validate_indices(
    snapshot: &CompileSnapshot,
    instructions: &[Instruction],
) -> Result<(), VerifyError> {
    for instruction in instructions {
        let Some((space, index, width)) = indexed_operand(instruction) else {
            continue;
        };
        let count = match space {
            IndexSpace::Local => u32::from(snapshot.local_count()),
            IndexSpace::Argument => u32::from(snapshot.arg_count()),
            IndexSpace::Closure => u32::from(snapshot.closure_count()),
            IndexSpace::Constant => snapshot.constant_count(),
        };
        let checked_index = index.saturating_add(width - 1);
        if checked_index >= count {
            let kind = match space {
                IndexSpace::Local => VerifyErrorKind::LocalIndexOutOfRange {
                    index: checked_index,
                    count,
                },
                IndexSpace::Argument => VerifyErrorKind::ArgumentIndexOutOfRange {
                    index: checked_index,
                    count,
                },
                IndexSpace::Closure => VerifyErrorKind::ClosureIndexOutOfRange {
                    index: checked_index,
                    count,
                },
                IndexSpace::Constant => VerifyErrorKind::ConstantIndexOutOfRange {
                    index: checked_index,
                    count,
                },
            };
            return Err(VerifyError::new(instruction.pc(), kind));
        }
    }
    Ok(())
}

fn verify_metadata(
    snapshot: &CompileSnapshot,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
    proof: &StateProof,
) -> Result<(), VerifyError> {
    let instruction_pcs: std::collections::BTreeSet<_> =
        instructions.iter().map(Instruction::pc).collect();
    for point in &snapshot.data.metadata.osr_points {
        if !instruction_pcs.contains(&point.pc) {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::OsrPointNotInstructionBoundary,
            ));
        }
        if !cfg.is_loop_header(point.pc) {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::OsrPointNotLoopHeader,
            ));
        }
        let Some(state) = proof.before.get(&point.pc) else {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::IncompleteOsrState,
            ));
        };
        if point.live_slots != state.live_slots(snapshot) {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::IncompleteOsrState,
            ));
        }
    }
    for point in &snapshot.data.metadata.deopt_points {
        if !instruction_pcs.contains(&point.pc) {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::DeoptPointNotInstructionBoundary,
            ));
        }
        let Some(before) = proof.before.get(&point.pc) else {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::IncompleteDeoptState,
            ));
        };
        let Some(after) = proof.after.get(&point.pc) else {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::IncompleteDeoptState,
            ));
        };
        if point.before != before.live_slots(snapshot) || point.after != after.live_slots(snapshot)
        {
            return Err(VerifyError::new(
                point.pc,
                VerifyErrorKind::IncompleteDeoptState,
            ));
        }
    }
    Ok(())
}

pub(crate) fn verify(
    snapshot: CompileSnapshot,
    limits: VerifyLimits,
) -> Result<VerifiedFunction, VerifyError> {
    if snapshot.source_revision() != crate::abi::SOURCE_REVISION
        || snapshot.opcode_fingerprint() != crate::abi::OPCODE_FINGERPRINT
    {
        return Err(VerifyError::new(0, VerifyErrorKind::IncompatibleSnapshot));
    }
    let snapshot_bytes = snapshot
        .bytecode()
        .len()
        .saturating_add(snapshot.exception_map().len())
        .saturating_add(snapshot.source_map().len())
        .saturating_add(
            (snapshot.constant_count() as usize)
                .saturating_mul(std::mem::size_of::<super::ConstantDescriptor>()),
        );
    if snapshot_bytes > limits.max_snapshot_bytes {
        return Err(VerifyError::new(
            0,
            VerifyErrorKind::ResourceLimit {
                resource: Resource::SnapshotBytes,
            },
        ));
    }
    let metadata_bytes = 32usize
        .saturating_add(snapshot.data.metadata.byte_len())
        .saturating_add(snapshot.exception_map().len())
        .saturating_add(snapshot.source_map().len());
    if metadata_bytes > limits.max_metadata_bytes {
        return Err(VerifyError::new(
            0,
            VerifyErrorKind::ResourceLimit {
                resource: Resource::MetadataBytes,
            },
        ));
    }
    let instructions = decode_bounded(snapshot.bytecode(), limits.max_instructions).map_err(
        |error| match error {
            BoundedDecodeError::Decode(error) => {
                VerifyError::new(decode_error_pc(&error), VerifyErrorKind::Decode(error))
            }
            BoundedDecodeError::InstructionLimit { pc } => VerifyError::new(
                pc,
                VerifyErrorKind::ResourceLimit {
                    resource: Resource::DecodedInstructions,
                },
            ),
        },
    )?;
    validate_indices(&snapshot, &instructions)?;
    let cfg = cfg::build(
        &instructions,
        snapshot.bytecode().len(),
        limits.max_basic_blocks,
    )?;
    let proof = stack::prove(&snapshot, &instructions, &cfg, limits.max_work_units)?;
    if let Some(instruction) = instructions
        .iter()
        .find(|instruction| !proof.visited.contains(&instruction.pc()))
    {
        return Err(VerifyError::new(
            instruction.pc(),
            VerifyErrorKind::UnreachableInstruction,
        ));
    }
    let tail = instructions.last().expect("CFG rejected empty bytecode");
    if !cfg::has_valid_exit(tail) {
        return Err(VerifyError::new(
            tail.pc(),
            VerifyErrorKind::MissingTerminator,
        ));
    }
    verify_metadata(&snapshot, &instructions, &cfg, &proof)?;
    Ok(VerifiedFunction {
        snapshot,
        instructions,
        cfg,
    })
}
