use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rquickjs_core::qjs;

use super::{
    cfg::ControlFlowGraph, CompileSnapshot, Instruction, OperandFormat, Resource, VerifyError,
    VerifyErrorKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotKind {
    Tagged,
    Int32,
    Float64,
    CatchOffset,
    Uninitialized,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AbstractState {
    pub(crate) locals: Vec<SlotKind>,
    pub(crate) stack: Vec<SlotKind>,
}

impl AbstractState {
    fn cell_count(&self) -> usize {
        self.locals.len().saturating_add(self.stack.len())
    }

    pub(crate) fn live_slots(&self, snapshot: &CompileSnapshot) -> Vec<SlotKind> {
        let mut result = Vec::with_capacity(
            snapshot.arg_count() as usize
                + self.locals.len()
                + snapshot.closure_count() as usize
                + self.stack.len(),
        );
        result.resize(snapshot.arg_count() as usize, SlotKind::Tagged);
        result.extend_from_slice(&self.locals);
        result.resize(
            result.len() + snapshot.closure_count() as usize,
            SlotKind::Tagged,
        );
        result.extend_from_slice(&self.stack);
        result
    }
}

struct WorkBudget {
    used: usize,
    limit: usize,
}

impl WorkBudget {
    fn charge(&mut self, pc: u32, units: usize) -> Result<(), VerifyError> {
        self.used = self.used.saturating_add(units);
        if self.used > self.limit {
            return Err(VerifyError::new(
                pc,
                VerifyErrorKind::ResourceLimit {
                    resource: Resource::WorkUnits,
                },
            ));
        }
        Ok(())
    }
}

pub(crate) struct StateProof {
    pub(crate) before: BTreeMap<u32, AbstractState>,
    pub(crate) after: BTreeMap<u32, AbstractState>,
    pub(crate) visited: BTreeSet<u32>,
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
            .map_or(base, |arguments| base.saturating_add(arguments)),
        _ => base,
    }
}

fn local_index(instruction: &Instruction) -> Option<usize> {
    match instruction.opcode().format() {
        OperandFormat::Local => Some(instruction.operand_u16(1) as usize),
        OperandFormat::Local8 => Some(instruction.operand_u8(1) as usize),
        OperandFormat::NoneLocal => instruction
            .opcode()
            .name()
            .as_bytes()
            .last()
            .and_then(|value| value.is_ascii_digit().then_some((value - b'0') as usize)),
        _ => None,
    }
}

fn pushed_kind(snapshot: &CompileSnapshot, instruction: &Instruction) -> SlotKind {
    let name = instruction.opcode().name();
    if matches!(
        name,
        "push_i32"
            | "push_i8"
            | "push_i16"
            | "push_minus1"
            | "push_0"
            | "push_1"
            | "push_2"
            | "push_3"
            | "push_4"
            | "push_5"
            | "push_6"
            | "push_7"
    ) {
        SlotKind::Int32
    } else if matches!(name, "push_const" | "push_const8") {
        let index = if name == "push_const" {
            instruction.operand_u32(1)
        } else {
            u32::from(instruction.operand_u8(1))
        };
        match snapshot
            .constants()
            .get(index as usize)
            .map(|value| value.tag())
        {
            Some(tag) if tag == qjs::JS_TAG_INT => SlotKind::Int32,
            Some(tag) if tag == qjs::JS_TAG_FLOAT64 => SlotKind::Float64,
            _ => SlotKind::Tagged,
        }
    } else if name == "catch" {
        SlotKind::CatchOffset
    } else {
        SlotKind::Tagged
    }
}

fn copied_stack_values(name: &str, popped: &[SlotKind]) -> Option<Vec<SlotKind>> {
    let values = match (name, popped) {
        ("nip", &[_, b]) => vec![b],
        ("nip1", &[_, b, c]) => vec![b, c],
        ("dup", &[a]) => vec![a, a],
        ("dup1", &[a, b]) => vec![a, a, b],
        ("dup2", &[a, b]) => vec![a, b, a, b],
        ("dup3", &[a, b, c]) => vec![a, b, c, a, b, c],
        ("insert2", &[object, a]) => vec![a, object, a],
        ("insert3", &[object, property, a]) => vec![a, object, property, a],
        ("insert4", &[this, object, property, a]) => {
            vec![a, this, object, property, a]
        }
        ("perm3", &[object, a, b]) => vec![a, object, b],
        ("perm4", &[object, property, a, b]) => vec![a, object, property, b],
        ("perm5", &[this, object, property, a, b]) => vec![a, this, object, property, b],
        ("swap", &[a, b]) => vec![b, a],
        ("swap2", &[a, b, c, d]) => vec![c, d, a, b],
        ("rot3l", &[a, b, c]) => vec![b, c, a],
        ("rot3r", &[a, b, c]) => vec![c, a, b],
        ("rot4l", &[a, b, c, d]) => vec![b, c, d, a],
        ("rot5l", &[a, b, c, d, e]) => vec![b, c, d, e, a],
        _ => return None,
    };
    Some(values)
}

fn check_stack_size(
    snapshot: &CompileSnapshot,
    instruction: &Instruction,
    state: &AbstractState,
) -> Result<(), VerifyError> {
    if state.stack.len() > snapshot.stack_size() as usize {
        return Err(VerifyError::new(
            instruction.pc(),
            VerifyErrorKind::StackSizeExceeded {
                declared: snapshot.stack_size(),
                actual: state.stack.len(),
            },
        ));
    }
    Ok(())
}

fn transfer(
    snapshot: &CompileSnapshot,
    instruction: &Instruction,
    state: &mut AbstractState,
) -> Result<(), VerifyError> {
    let pop = effective_pop(instruction);
    if state.stack.len() < pop {
        return Err(VerifyError::new(
            instruction.pc(),
            VerifyErrorKind::StackUnderflow {
                needed: pop,
                available: state.stack.len(),
            },
        ));
    }

    let name = instruction.opcode().name();
    let popped = state.stack.split_off(state.stack.len() - pop);
    let popped_top = popped.last().copied().unwrap_or(SlotKind::Tagged);

    if name == "get_loc0_loc1" {
        state.stack.push(state.locals[0]);
        state.stack.push(state.locals[1]);
        return check_stack_size(snapshot, instruction, state);
    }
    if name.starts_with("get_loc") {
        let index = local_index(instruction).expect("local format was validated");
        state.stack.push(state.locals[index]);
        return check_stack_size(snapshot, instruction, state);
    }
    if name == "set_loc_uninitialized" {
        let index = local_index(instruction).expect("local format was validated");
        state.locals[index] = SlotKind::Uninitialized;
    } else if name.starts_with("put_loc") || name.starts_with("set_loc") {
        let index = local_index(instruction).expect("local format was validated");
        state.locals[index] = popped_top;
    }

    if matches!(name, "inc_loc" | "dec_loc" | "add_loc") {
        let index = local_index(instruction).expect("local format was validated");
        state.locals[index] = SlotKind::Tagged;
    }

    if matches!(name, "for_of_start" | "for_await_of_start") {
        state
            .stack
            .extend([SlotKind::Tagged, SlotKind::Tagged, SlotKind::CatchOffset]);
        return check_stack_size(snapshot, instruction, state);
    }

    if name == "using_dispose_init" {
        state.stack.push(SlotKind::Uninitialized);
        return check_stack_size(snapshot, instruction, state);
    }

    if let Some(values) = copied_stack_values(name, &popped) {
        state.stack.extend(values);
        return check_stack_size(snapshot, instruction, state);
    }

    if instruction.opcode().n_push() == 1
        && (name.starts_with("set_loc")
            || name.starts_with("set_arg")
            || name.starts_with("set_var_ref"))
    {
        state.stack.push(popped_top);
        return check_stack_size(snapshot, instruction, state);
    }

    let push = instruction.opcode().n_push() as usize;
    state.stack.extend(std::iter::repeat_n(
        pushed_kind(snapshot, instruction),
        push,
    ));
    check_stack_size(snapshot, instruction, state)
}

fn merge_state(
    pc: u32,
    expected: &mut AbstractState,
    actual: &AbstractState,
) -> Result<bool, VerifyError> {
    if expected.stack.len() != actual.stack.len() {
        return Err(VerifyError::new(
            pc,
            VerifyErrorKind::InconsistentMergeHeight {
                expected: expected.stack.len(),
                actual: actual.stack.len(),
            },
        ));
    }
    for (slot, (expected, actual)) in expected
        .stack
        .iter()
        .zip(&actual.stack)
        .chain(expected.locals.iter().zip(&actual.locals))
        .enumerate()
    {
        if expected != actual {
            return Err(VerifyError::new(
                pc,
                VerifyErrorKind::IncompatibleMergeKind {
                    slot,
                    expected: *expected,
                    actual: *actual,
                },
            ));
        }
    }
    Ok(false)
}

pub(crate) fn prove(
    snapshot: &CompileSnapshot,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
    max_work_units: usize,
) -> Result<StateProof, VerifyError> {
    if instructions.is_empty() {
        return Ok(StateProof {
            before: BTreeMap::new(),
            after: BTreeMap::new(),
            visited: BTreeSet::new(),
        });
    }
    let before_points: BTreeSet<u32> = snapshot
        .data
        .metadata
        .osr_points
        .iter()
        .map(|point| point.pc)
        .chain(
            snapshot
                .data
                .metadata
                .deopt_points
                .iter()
                .map(|point| point.pc),
        )
        .chain(
            cfg.blocks()
                .iter()
                .map(|block| block.start_pc())
                .filter(|pc| cfg.is_loop_header(*pc)),
        )
        .collect();
    let after_points: BTreeSet<u32> = snapshot
        .data
        .metadata
        .deopt_points
        .iter()
        .map(|point| point.pc)
        .collect();
    let mut budget = WorkBudget {
        used: 0,
        limit: max_work_units,
    };
    budget.charge(0, snapshot.local_count() as usize)?;
    let mut block_entries = BTreeMap::new();
    block_entries.insert(
        0,
        AbstractState {
            locals: vec![SlotKind::Tagged; snapshot.local_count() as usize],
            stack: Vec::new(),
        },
    );
    let mut queue = VecDeque::from([0_u32]);
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    let mut visited = BTreeSet::new();

    while let Some(block_pc) = queue.pop_front() {
        let block = cfg.block(block_pc).expect("CFG successor names a block");
        let entry = &block_entries[&block_pc];
        budget.charge(block_pc, entry.cell_count())?;
        let mut state = entry.clone();
        for instruction in &instructions[block.instruction_range()] {
            budget.charge(instruction.pc(), 1usize.saturating_add(state.cell_count()))?;
            if before_points.contains(&instruction.pc()) {
                budget.charge(instruction.pc(), state.cell_count())?;
                before.insert(instruction.pc(), state.clone());
            }
            visited.insert(instruction.pc());
            transfer(snapshot, instruction, &mut state)?;
            if after_points.contains(&instruction.pc()) {
                budget.charge(instruction.pc(), state.cell_count())?;
                after.insert(instruction.pc(), state.clone());
            }
        }
        for successor in block.successors() {
            if let Some(expected) = block_entries.get_mut(successor) {
                budget.charge(*successor, state.cell_count())?;
                merge_state(*successor, expected, &state)?;
            } else {
                budget.charge(*successor, state.cell_count())?;
                block_entries.insert(*successor, state.clone());
                queue.push_back(*successor);
            }
        }
    }
    Ok(StateProof {
        before,
        after,
        visited,
    })
}
