use std::collections::{BTreeMap, VecDeque};

use rquickjs_core::qjs;

use crate::{
    bytecode::{Instruction, OperandFormat, VerifiedFunction},
    compiler::CompileFailure,
};

use super::{
    BinaryOp, FrameSlot, FrameState, FrameStateId, FrameStateTable, IrOp, StackOp, TaggedValue,
    UnaryOp,
};

const POLL_INTERVAL: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrInstruction {
    pub pc: u32,
    pub frame_state: Option<FrameStateId>,
    pub op: IrOp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrBlock {
    pub start_pc: u32,
    pub stack_depth: u16,
    pub instructions: Vec<IrInstruction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaselineIr {
    pub blocks: Vec<IrBlock>,
    pub frame_states: FrameStateTable,
    pub max_stack_depth: u16,
    pub argument_count: u16,
    pub local_count: u16,
}

impl BaselineIr {
    pub fn translate(function: &VerifiedFunction) -> Result<Self, CompileFailure> {
        let block_depths = block_depths(function)?;
        let snapshot = function.snapshot();
        let mut states = FrameStateTable::default();
        let mut blocks = Vec::with_capacity(function.control_flow_graph().blocks().len());
        let mut max_stack_depth = 0_usize;
        let mut emitted_since_poll = 0_usize;

        for (block_index, block) in function.control_flow_graph().blocks().iter().enumerate() {
            let mut depth = *block_depths
                .get(&block.start_pc())
                .ok_or(CompileFailure::InvalidArtifact)?;
            let mut instructions = Vec::new();
            if function
                .control_flow_graph()
                .is_loop_header(block.start_pc())
            {
                let state = record_state(
                    &mut states,
                    snapshot.arg_count(),
                    snapshot.local_count(),
                    depth,
                    block.start_pc(),
                );
                instructions.push(IrInstruction {
                    pc: block.start_pc(),
                    frame_state: Some(state),
                    op: IrOp::OsrLabel { state },
                });
            }
            for instruction in &function.instructions()[block.instruction_range()] {
                let pc = instruction.pc();
                let needs_entry_poll = block_index == 0 && instruction.pc() == 0;
                let needs_periodic_poll = emitted_since_poll >= POLL_INTERVAL;
                let needs_return_poll =
                    matches!(instruction.opcode().name(), "return" | "return_undef");
                if needs_entry_poll {
                    let state = record_state(
                        &mut states,
                        snapshot.arg_count(),
                        snapshot.local_count(),
                        depth,
                        pc,
                    );
                    instructions.push(IrInstruction {
                        pc,
                        frame_state: Some(state),
                        op: IrOp::Poll { state },
                    });
                    emitted_since_poll = 0;
                }
                if needs_periodic_poll {
                    let state = record_state(
                        &mut states,
                        snapshot.arg_count(),
                        snapshot.local_count(),
                        depth,
                        pc,
                    );
                    instructions.push(IrInstruction {
                        pc,
                        frame_state: Some(state),
                        op: IrOp::Poll { state },
                    });
                    emitted_since_poll = 0;
                }
                if needs_return_poll {
                    let state = record_state(
                        &mut states,
                        snapshot.arg_count(),
                        snapshot.local_count(),
                        depth,
                        pc,
                    );
                    instructions.push(IrInstruction {
                        pc,
                        frame_state: Some(state),
                        op: IrOp::Poll { state },
                    });
                    emitted_since_poll = 0;
                }

                let op = translate_instruction(instruction)?;
                let depth_before = depth;
                let frame_state = operation_may_exit(&op).then(|| {
                    record_state(
                        &mut states,
                        snapshot.arg_count(),
                        snapshot.local_count(),
                        depth,
                        pc,
                    )
                });
                let pop = effective_pop(instruction);
                if depth < pop {
                    return Err(CompileFailure::InvalidArtifact);
                }
                let next_depth = depth - pop + instruction.opcode().n_push() as usize;
                max_stack_depth = max_stack_depth.max(next_depth).max(depth);
                instructions.push(IrInstruction {
                    pc,
                    frame_state,
                    op,
                });
                emitted_since_poll = emitted_since_poll.saturating_add(1);
                depth = next_depth;

                if let Some(target) = instruction.branch_target() {
                    if target <= i64::from(pc) {
                        let state = record_state(
                            &mut states,
                            snapshot.arg_count(),
                            snapshot.local_count(),
                            depth_before,
                            pc,
                        );
                        let insert_at = instructions.len() - 1;
                        instructions.insert(
                            insert_at,
                            IrInstruction {
                                pc,
                                frame_state: Some(state),
                                op: IrOp::Poll { state },
                            },
                        );
                        emitted_since_poll = 0;
                    }
                }
            }
            blocks.push(IrBlock {
                start_pc: block.start_pc(),
                stack_depth: *block_depths
                    .get(&block.start_pc())
                    .ok_or(CompileFailure::InvalidArtifact)? as u16,
                instructions,
            });
        }

        Ok(Self {
            blocks,
            frame_states: states,
            max_stack_depth: u16::try_from(max_stack_depth)
                .map_err(|_| CompileFailure::ResourceLimit)?,
            argument_count: snapshot.arg_count(),
            local_count: snapshot.local_count(),
        })
    }
}

fn operation_may_exit(operation: &IrOp) -> bool {
    matches!(
        operation,
        IrOp::Unary(_)
            | IrOp::PostUnary(_)
            | IrOp::LocalUnary { .. }
            | IrOp::AddLocal(_)
            | IrOp::Binary(_)
            | IrOp::GetLocalChecked(_)
            | IrOp::PutLocalChecked { .. }
            | IrOp::Branch { .. }
            | IrOp::Return
            | IrOp::ReturnUndefined
    )
}

fn record_state(
    table: &mut FrameStateTable,
    arguments: u16,
    locals: u16,
    stack_depth: usize,
    pc: u32,
) -> FrameStateId {
    let mut slots = Vec::with_capacity(arguments as usize + locals as usize + stack_depth);
    slots.extend((0..arguments).map(FrameSlot::Argument));
    slots.extend((0..locals).map(FrameSlot::Local));
    slots.extend((0..stack_depth as u16).map(FrameSlot::Stack));
    table.push(FrameState {
        pc,
        slots: slots.into_boxed_slice(),
    })
}

fn effective_pop(instruction: &Instruction) -> usize {
    let base = instruction.opcode().n_pop() as usize;
    match instruction.opcode().format() {
        OperandFormat::NPop | OperandFormat::NPopU16 => {
            base.saturating_add(instruction.operand_u16(1) as usize)
        }
        OperandFormat::NPopFixed => instruction
            .opcode()
            .name()
            .as_bytes()
            .last()
            .and_then(|value| value.is_ascii_digit().then_some((value - b'0') as usize))
            .map_or(base, |count| base.saturating_add(count)),
        _ => base,
    }
}

fn block_depths(function: &VerifiedFunction) -> Result<BTreeMap<u32, usize>, CompileFailure> {
    let cfg = function.control_flow_graph();
    let mut depths = BTreeMap::from([(0_u32, 0_usize)]);
    let mut queue = VecDeque::from([0_u32]);
    while let Some(pc) = queue.pop_front() {
        let block = cfg.block(pc).ok_or(CompileFailure::InvalidArtifact)?;
        let mut depth = depths[&pc];
        for instruction in &function.instructions()[block.instruction_range()] {
            let pop = effective_pop(instruction);
            if depth < pop {
                return Err(CompileFailure::InvalidArtifact);
            }
            depth = depth - pop + instruction.opcode().n_push() as usize;
        }
        for &successor in block.successors() {
            match depths.get(&successor) {
                Some(existing) if *existing != depth => {
                    return Err(CompileFailure::InvalidArtifact)
                }
                Some(_) => {}
                None => {
                    depths.insert(successor, depth);
                    queue.push_back(successor);
                }
            }
        }
    }
    Ok(depths)
}

fn short_index(instruction: &Instruction) -> Option<u16> {
    instruction
        .opcode()
        .name()
        .as_bytes()
        .last()
        .and_then(|value| value.is_ascii_digit().then_some(u16::from(value - b'0')))
}

fn indexed_operand(instruction: &Instruction) -> Option<u16> {
    match instruction.opcode().format() {
        OperandFormat::Local | OperandFormat::Argument => Some(instruction.operand_u16(1)),
        OperandFormat::Local8 => Some(u16::from(instruction.operand_u8(1))),
        OperandFormat::NoneLocal | OperandFormat::NoneArgument => short_index(instruction),
        _ => None,
    }
}

fn translate_instruction(instruction: &Instruction) -> Result<IrOp, CompileFailure> {
    let name = instruction.opcode().name();
    let int = |value: i32| TaggedValue::new(value as i64 as u64, qjs::JS_TAG_INT as i64);
    let operation = match name {
        "nop" => IrOp::Nop,
        "undefined" | "push_undefined" => {
            IrOp::Push(TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64))
        }
        "null" | "push_null" => IrOp::Push(TaggedValue::new(0, qjs::JS_TAG_NULL as i64)),
        "push_false" => IrOp::Push(TaggedValue::new(0, qjs::JS_TAG_BOOL as i64)),
        "push_true" => IrOp::Push(TaggedValue::new(1, qjs::JS_TAG_BOOL as i64)),
        "push_minus1" => IrOp::Push(int(-1)),
        "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5" | "push_6" | "push_7" => {
            IrOp::Push(int(
                short_index(instruction).expect("numeric compact opcode") as i32,
            ))
        }
        "push_i8" => IrOp::Push(int(instruction.operand_u8(1) as i8 as i32)),
        "push_i16" => IrOp::Push(int(instruction.operand_i16(1) as i32)),
        "push_i32" => IrOp::Push(int(instruction.operand_i32(1))),
        "get_arg" | "get_arg0" | "get_arg1" | "get_arg2" | "get_arg3" => {
            IrOp::GetArgument(indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?)
        }
        "get_loc" | "get_loc8" | "get_loc0" | "get_loc1" | "get_loc2" | "get_loc3" => {
            IrOp::GetLocal(indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?)
        }
        "get_loc_check" => IrOp::GetLocalChecked(
            indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
        ),
        "get_loc0_loc1" => IrOp::GetLocalPair,
        "put_arg" | "put_arg0" | "put_arg1" | "put_arg2" | "put_arg3" => IrOp::PutArgument {
            index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
            keep: false,
        },
        "set_arg" | "set_arg0" | "set_arg1" | "set_arg2" | "set_arg3" => IrOp::PutArgument {
            index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
            keep: true,
        },
        "put_loc" | "put_loc8" | "put_loc0" | "put_loc1" | "put_loc2" | "put_loc3" => {
            IrOp::PutLocal {
                index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
                keep: false,
            }
        }
        "set_loc" | "set_loc8" | "set_loc0" | "set_loc1" | "set_loc2" | "set_loc3" => {
            IrOp::PutLocal {
                index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
                keep: true,
            }
        }
        "put_loc_check" | "put_loc_check_init" => IrOp::PutLocalChecked {
            index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
            initialize: name == "put_loc_check_init",
        },
        "set_loc_uninitialized" => IrOp::SetLocalUninitialized(
            indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
        ),
        "drop" => IrOp::Drop,
        "nip" => IrOp::Stack(StackOp::Nip),
        "nip1" => IrOp::Stack(StackOp::Nip1),
        "dup" => IrOp::Stack(StackOp::Dup),
        "dup1" => IrOp::Stack(StackOp::Dup1),
        "dup2" => IrOp::Stack(StackOp::Dup2),
        "dup3" => IrOp::Stack(StackOp::Dup3),
        "insert2" => IrOp::Stack(StackOp::Insert2),
        "insert3" => IrOp::Stack(StackOp::Insert3),
        "insert4" => IrOp::Stack(StackOp::Insert4),
        "perm3" => IrOp::Stack(StackOp::Perm3),
        "perm4" => IrOp::Stack(StackOp::Perm4),
        "perm5" => IrOp::Stack(StackOp::Perm5),
        "swap" => IrOp::Stack(StackOp::Swap),
        "swap2" => IrOp::Stack(StackOp::Swap2),
        "rot3l" => IrOp::Stack(StackOp::Rot3Left),
        "rot3r" => IrOp::Stack(StackOp::Rot3Right),
        "rot4l" => IrOp::Stack(StackOp::Rot4Left),
        "rot5l" => IrOp::Stack(StackOp::Rot5Left),
        "plus" => IrOp::Unary(UnaryOp::Plus),
        "neg" => IrOp::Unary(UnaryOp::Neg),
        "inc" => IrOp::Unary(UnaryOp::Increment),
        "dec" => IrOp::Unary(UnaryOp::Decrement),
        "post_inc" => IrOp::PostUnary(UnaryOp::Increment),
        "post_dec" => IrOp::PostUnary(UnaryOp::Decrement),
        "inc_loc" => IrOp::LocalUnary {
            index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
            op: UnaryOp::Increment,
        },
        "dec_loc" => IrOp::LocalUnary {
            index: indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?,
            op: UnaryOp::Decrement,
        },
        "add_loc" => {
            IrOp::AddLocal(indexed_operand(instruction).ok_or(CompileFailure::InvalidArtifact)?)
        }
        "lnot" => IrOp::Unary(UnaryOp::LogicalNot),
        "not" => IrOp::Unary(UnaryOp::BitNot),
        "add" => IrOp::Binary(BinaryOp::Add),
        "sub" => IrOp::Binary(BinaryOp::Sub),
        "mul" => IrOp::Binary(BinaryOp::Mul),
        "div" => IrOp::Binary(BinaryOp::Div),
        "mod" => IrOp::Binary(BinaryOp::Mod),
        "and" => IrOp::Binary(BinaryOp::BitAnd),
        "or" => IrOp::Binary(BinaryOp::BitOr),
        "xor" => IrOp::Binary(BinaryOp::BitXor),
        "shl" => IrOp::Binary(BinaryOp::ShiftLeft),
        "sar" => IrOp::Binary(BinaryOp::ShiftRight),
        "shr" => IrOp::Binary(BinaryOp::ShiftRightUnsigned),
        "lt" => IrOp::Binary(BinaryOp::LessThan),
        "lte" => IrOp::Binary(BinaryOp::LessThanOrEqual),
        "gt" => IrOp::Binary(BinaryOp::GreaterThan),
        "gte" => IrOp::Binary(BinaryOp::GreaterThanOrEqual),
        "eq" => IrOp::Binary(BinaryOp::Equal),
        "neq" => IrOp::Binary(BinaryOp::NotEqual),
        "strict_eq" => IrOp::Binary(BinaryOp::StrictEqual),
        "strict_neq" => IrOp::Binary(BinaryOp::StrictNotEqual),
        "goto" | "goto8" | "goto16" => IrOp::Jump(
            instruction
                .branch_target()
                .ok_or(CompileFailure::InvalidArtifact)? as u32,
        ),
        "if_true" | "if_true8" => IrOp::Branch {
            target: instruction
                .branch_target()
                .ok_or(CompileFailure::InvalidArtifact)? as u32,
            when_true: true,
        },
        "if_false" | "if_false8" => IrOp::Branch {
            target: instruction
                .branch_target()
                .ok_or(CompileFailure::InvalidArtifact)? as u32,
            when_true: false,
        },
        "return" => IrOp::Return,
        "return_undef" => IrOp::ReturnUndefined,
        _ => return Err(CompileFailure::UnsupportedOpcode),
    };
    Ok(operation)
}
