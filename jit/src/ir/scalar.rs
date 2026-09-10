//! Explicit scalar values used by production numeric value numbering.
//!
//! Frame and stack values are connected across CFG edges with explicit Phi
//! inputs. Numeric lowering consumes these value identities, while non-numeric
//! operations retain the audited tagged-frame bridge during migration.
//! Numeric operations carry the feedback-selected arithmetic contract before
//! machine lowering. The backend implements its checks and result semantics.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{
    FrameSlot, OptimizedBlock, OptimizedEffect, OptimizedFrameShape, OptimizedNode,
    OptimizedNodeKind,
};
use crate::compiler::CompileFailure;

#[cfg(test)]
mod representation_tests {
    use super::*;

    fn cyclic_graph(seed: ScalarValue) -> ScalarGraph {
        let phi = |input| ScalarValue::Phi {
            block_pc: 1,
            slot: FrameSlot::Local(0),
            inputs: vec![
                ScalarPhiInput {
                    predecessor: Some(0),
                    value: ScalarValueId(0),
                },
                ScalarPhiInput {
                    predecessor: Some(1),
                    value: ScalarValueId(input),
                },
            ]
            .into(),
        };
        ScalarGraph {
            values: vec![seed, phi(2), phi(1)],
            ..Default::default()
        }
    }

    #[test]
    fn cyclic_representation_requires_real_seed_and_all_edges() {
        let graph = cyclic_graph(ScalarValue::Int32(1));
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 100),
            [Some(ScalarNumericMode::Int32); 3]
        );
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 1),
            [None; 3]
        );
        let mut graph = cyclic_graph(ScalarValue::Opaque);
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 100),
            [None; 3]
        );
        graph.values[0] = ScalarValue::Phi {
            block_pc: 0,
            slot: FrameSlot::Local(0),
            inputs: vec![ScalarPhiInput {
                predecessor: Some(1),
                value: ScalarValueId(1),
            }]
            .into(),
        };
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 100),
            [None; 3]
        );
    }

    #[test]
    fn numeric_phi_preserves_wider_representation_without_proving_int32() {
        for mode in [ScalarNumericMode::Float64, ScalarNumericMode::Number] {
            let mut graph = cyclic_graph(ScalarValue::Update {
                mode,
                input: ScalarValueId(1),
                delta: 1,
            });
            assert_eq!(
                graph.proven_numeric_values(&[], |_| false, false, 100),
                [Some(mode); 3]
            );
            graph.values.push(ScalarValue::Int32(0));
            if let ScalarValue::Phi { inputs, .. } = &mut graph.values[1] {
                let mut extended = inputs.to_vec();
                extended.push(ScalarPhiInput {
                    predecessor: Some(2),
                    value: ScalarValueId(3),
                });
                *inputs = extended.into();
            }
            assert_eq!(
                graph.proven_numeric_values(&[], |_| false, false, 100),
                [
                    Some(mode),
                    Some(ScalarNumericMode::Number),
                    Some(ScalarNumericMode::Number),
                    Some(ScalarNumericMode::Int32)
                ]
            );
        }
    }

    #[test]
    fn unsigned_shift_does_not_seed_an_int32_phi_proof() {
        for op in [
            ScalarBitwiseOp::Or,
            ScalarBitwiseOp::And,
            ScalarBitwiseOp::Xor,
            ScalarBitwiseOp::Shl,
            ScalarBitwiseOp::Sar,
            ScalarBitwiseOp::Shr,
        ] {
            let graph = cyclic_graph(ScalarValue::Bitwise {
                op,
                lhs: ScalarValueId(1),
                rhs: ScalarValueId(2),
            });
            assert_eq!(
                graph.proven_numeric_values(&[], |_| false, false, 100),
                [Some(if op == ScalarBitwiseOp::Shr {
                    ScalarNumericMode::Number
                } else {
                    ScalarNumericMode::Int32
                }); 3]
            );
        }
    }

    #[test]
    fn argument_representation_requires_entry_guard() {
        let graph = cyclic_graph(ScalarValue::Input {
            block_pc: 0,
            slot: FrameSlot::Argument(0),
        });
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 100),
            [None; 3]
        );
        assert_eq!(
            graph.proven_numeric_values(&[], |index| index == 0, false, 100),
            [Some(ScalarNumericMode::Int32); 3]
        );
    }

    #[test]
    fn mixed_argument_proofs_follow_exact_guarded_slots() {
        let graph = ScalarGraph {
            values: (0..4)
                .map(|index| ScalarValue::Input {
                    block_pc: 0,
                    slot: FrameSlot::Argument(index),
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            graph.proven_numeric_values(&[], |index| matches!(index, 0 | 2), false, 100),
            [
                Some(ScalarNumericMode::Int32),
                None,
                Some(ScalarNumericMode::Int32),
                None
            ]
        );
    }

    #[test]
    fn late_unknown_edge_invalidates_a_provisional_loop_fact() {
        let mut graph = cyclic_graph(ScalarValue::Int32(1));
        graph.values.push(ScalarValue::Opaque);
        if let ScalarValue::Phi { inputs, .. } = &mut graph.values[2] {
            let mut extended = inputs.to_vec();
            extended.push(ScalarPhiInput {
                predecessor: Some(2),
                value: ScalarValueId(3),
            });
            *inputs = extended.into();
        }
        assert_eq!(
            graph.proven_numeric_values(&[], |_| false, false, 100),
            [Some(ScalarNumericMode::Int32), None, None, None]
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScalarValueId(u32);

impl ScalarValueId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ScalarBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ScalarBinaryOp {
    fn from_opcode(name: &str) -> Option<Self> {
        match name {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Sub),
            "mul" => Some(Self::Mul),
            "div" => Some(Self::Div),
            _ => None,
        }
    }
}

/// Checked Int32-input bitwise operations. Unsigned right shift may return
/// Float64 outside a raw Int32 loop; its result must not be assumed Int32.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarBitwiseOp {
    Or,
    And,
    Xor,
    Shl,
    Sar,
    Shr,
}

impl ScalarBitwiseOp {
    fn from_opcode(name: &str) -> Option<Self> {
        Some(match name {
            "or" => Self::Or,
            "and" => Self::And,
            "xor" => Self::Xor,
            "shl" => Self::Shl,
            "sar" => Self::Sar,
            "shr" => Self::Shr,
            _ => return None,
        })
    }
}

/// Ordered JavaScript numeric comparisons. NaN makes all four false;
/// non-numeric operands leave native code before any coercion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarCompareOp {
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,
}

impl ScalarCompareOp {
    fn from_opcode(name: &str) -> Option<Self> {
        match name {
            "lt" => Some(Self::LessThan),
            "lte" => Some(Self::LessEqual),
            "gt" => Some(Self::GreaterThan),
            "gte" => Some(Self::GreaterEqual),
            _ => None,
        }
    }
}

/// The checked arithmetic contract selected by frontend feedback.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ScalarNumericMode {
    /// Accept either JS number representation; non-numbers deopt before coercion.
    #[default]
    Number,
    /// Guard Int32 operands and deopt on overflow, negative zero or inexact division.
    Int32,
    /// Operands have the guarded Float64 representation; preserve IEEE semantics.
    Float64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarPhiInput {
    /// None denotes the initial function entry, including a loop at pc zero.
    pub predecessor: Option<u32>,
    pub value: ScalarValueId,
}

/// A JavaScript call before specialization or inlining. Operands refer to the
/// exact pre-call frame, including the receiver of a method call. The generic
/// call may reenter JavaScript; its result has no inferred representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarCall {
    pub target: ScalarValueId,
    pub receiver: Option<ScalarValueId>,
    pub arguments: Box<[ScalarValueId]>,
    pub frame_state_node: u32,
    pub inline: Option<Box<ScalarInlineRegion>>,
}

/// An effect-free callee path expanded in the caller's ValueId namespace.
/// Failed guards replay the original call from its exact pre-call state; this
/// recovery policy cannot be used for regions with observable callee effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarInlineRegion {
    pub artifact: crate::code_cache::ArtifactKey,
    pub object_identity: u64,
    pub bytecode_identity: u64,
    pub arguments: Box<[crate::runtime::FeedbackRepresentation]>,
    pub steps: Box<[ScalarInlineStep]>,
    pub result: ScalarValueId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarInlineStep {
    pub value: ScalarValueId,
    pub frame: Option<ScalarFrameState>,
}

impl ScalarInlineRegion {
    fn allocated_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
            + core::mem::size_of_val(self.arguments.as_ref())
            + core::mem::size_of_val(self.steps.as_ref())
            + self
                .steps
                .iter()
                .filter_map(|step| step.frame.as_ref())
                .map(|frame| {
                    core::mem::size_of_val(frame.arguments.as_ref())
                        + core::mem::size_of_val(frame.locals.as_ref())
                        + core::mem::size_of_val(frame.stack.as_ref())
                })
                .sum::<usize>()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScalarValue {
    Int32(i32),
    Bool(bool),
    Input {
        block_pc: u32,
        slot: FrameSlot,
    },
    Phi {
        block_pc: u32,
        slot: FrameSlot,
        inputs: Box<[ScalarPhiInput]>,
    },
    FrameRead {
        node: u32,
        slot: FrameSlot,
    },
    Binary {
        mode: ScalarNumericMode,
        op: ScalarBinaryOp,
        lhs: ScalarValueId,
        rhs: ScalarValueId,
    },
    /// Increment/decrement after a numeric input check. Postfix bytecodes
    /// separately retain the input value as their first stack output.
    Update {
        mode: ScalarNumericMode,
        input: ScalarValueId,
        delta: i8,
    },
    Bitwise {
        op: ScalarBitwiseOp,
        lhs: ScalarValueId,
        rhs: ScalarValueId,
    },
    Compare {
        op: ScalarCompareOp,
        lhs: ScalarValueId,
        rhs: ScalarValueId,
    },
    Call(ScalarCall),
    /// A producer whose value identity is known, but whose semantics have not
    /// yet been modeled by this pass. Never value-number opaque operations.
    Opaque,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarFrameState {
    pub pc: u32,
    pub arguments: Box<[ScalarValueId]>,
    pub locals: Box<[ScalarValueId]>,
    pub stack: Box<[ScalarValueId]>,
    pub parent: Option<u32>,
}

/// A pure type check with an exact pre-effect recovery state. Eliminated
/// checks retain their proof result for IR inspection, but emit no machine code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarCheck {
    pub value: ScalarValueId,
    pub mode: ScalarNumericMode,
    pub frame_state_node: u32,
    pub eliminated: bool,
}

type FrameBindings = Box<[(FrameSlot, ScalarValueId)]>;

#[derive(Clone, Debug, Default)]
pub struct ScalarGraph {
    values: Vec<ScalarValue>,
    checks: Vec<Box<[ScalarCheck]>>,
    node_values: Vec<Option<ScalarValueId>>,
    node_outputs: Vec<Box<[ScalarValueId]>>,
    frame_definitions: Vec<FrameBindings>,
    block_inputs: Vec<(u32, FrameBindings)>,
    frame_states: Vec<Option<ScalarFrameState>>,
    state_entries: usize,
    value_limit: usize,
}

impl ScalarGraph {
    /// Representation facts valid after the caller-supplied entry guards.
    /// Poll aliases are enabled only when lowering preserves register bindings
    /// across its poll; arbitrary reentrant frame reads remain unknown.
    pub(crate) fn proven_numeric_values(
        &self,
        nodes: &[OptimizedNode],
        guarded_argument: impl Fn(u16) -> bool,
        preserve_poll_bindings: bool,
        mut work: usize,
    ) -> Vec<Option<ScalarNumericMode>> {
        // Zero is not-yet-reached evidence, not an unknown JS type. Union is
        // monotone: an unknown incoming value (OTHER) can never prove Int32.
        const INT: u8 = 1;
        const FLOAT: u8 = 2;
        const NUMBER: u8 = INT | FLOAT;
        const OTHER: u8 = 4;
        let mut types = vec![0u8; self.values.len()];
        loop {
            let mut changed = false;
            for (index, value) in self.values.iter().enumerate() {
                let cost = match value {
                    ScalarValue::Phi { inputs, .. } => inputs.len().max(1),
                    _ => 1,
                };
                let Some(remaining) = work.checked_sub(cost) else {
                    return vec![None; self.values.len()];
                };
                work = remaining;
                let next = match value {
                    ScalarValue::Int32(_) => INT,
                    ScalarValue::Call(call) if call.inline.is_some() => {
                        types[call.inline.as_ref().unwrap().result.index()]
                    }
                    ScalarValue::Input {
                        block_pc: 0,
                        slot: FrameSlot::Argument(argument),
                    } if guarded_argument(*argument) => INT,
                    ScalarValue::Binary { mode, .. } | ScalarValue::Update { mode, .. } => {
                        match mode {
                            ScalarNumericMode::Int32 => INT,
                            ScalarNumericMode::Float64 => FLOAT,
                            ScalarNumericMode::Number => INT | FLOAT,
                        }
                    }
                    ScalarValue::Bitwise { op, .. } => {
                        if *op == ScalarBitwiseOp::Shr {
                            INT | FLOAT
                        } else {
                            INT
                        }
                    }
                    ScalarValue::Phi { inputs, .. } => inputs
                        .iter()
                        .fold(0, |set, input| set | types[input.value.index()]),
                    ScalarValue::FrameRead { node, slot }
                        if preserve_poll_bindings
                            && matches!(
                                nodes[*node as usize].kind(),
                                OptimizedNodeKind::GuardNumeric { .. }
                            ) =>
                    {
                        self.frame_state_for_node(*node)
                            .and_then(|state| match slot {
                                FrameSlot::Argument(index) => {
                                    state.arguments.get(usize::from(*index))
                                }
                                FrameSlot::Local(index) => state.locals.get(usize::from(*index)),
                                FrameSlot::Stack(index) => state.stack.get(usize::from(*index)),
                            })
                            .map_or(OTHER, |source| types[source.index()])
                    }
                    _ => OTHER,
                };
                let next = next | types[index];
                changed |= next != types[index];
                types[index] = next;
            }
            if !changed {
                return types
                    .into_iter()
                    .map(|set| match set {
                        INT => Some(ScalarNumericMode::Int32),
                        FLOAT => Some(ScalarNumericMode::Float64),
                        NUMBER => Some(ScalarNumericMode::Number),
                        _ => None,
                    })
                    .collect();
            }
        }
    }

    pub fn checks_for_node(&self, node: u32) -> &[ScalarCheck] {
        self.checks
            .get(node as usize)
            .map(Box::as_ref)
            .unwrap_or(&[])
    }

    pub fn eliminated_type_checks(&self) -> usize {
        self.checks
            .iter()
            .flatten()
            .filter(|check| check.eliminated)
            .count()
    }

    pub fn values(&self) -> &[ScalarValue] {
        &self.values
    }

    pub fn value_for_node(&self, node: u32) -> Option<ScalarValueId> {
        self.node_values.get(node as usize).copied().flatten()
    }

    pub fn inputs_for_block(&self, pc: u32) -> &[(FrameSlot, ScalarValueId)] {
        self.block_inputs
            .binary_search_by_key(&pc, |(block_pc, _)| *block_pc)
            .ok()
            .map(|index| self.block_inputs[index].1.as_ref())
            .unwrap_or(&[])
    }

    pub fn outputs_for_node(&self, node: u32) -> &[ScalarValueId] {
        self.node_outputs
            .get(node as usize)
            .map(Box::as_ref)
            .unwrap_or(&[])
    }

    pub fn frame_definitions_for_node(&self, node: u32) -> &[(FrameSlot, ScalarValueId)] {
        self.frame_definitions
            .get(node as usize)
            .map(Box::as_ref)
            .unwrap_or(&[])
    }

    pub fn binary_operation(&self, node: u32) -> Option<(ScalarBinaryOp, ScalarNumericMode)> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Binary { op, mode, .. } => Some((*op, *mode)),
            _ => None,
        }
    }

    pub fn update_operation(&self, node: u32) -> Option<(ScalarNumericMode, ScalarValueId, i8)> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Update { mode, input, delta } => Some((*mode, *input, *delta)),
            _ => None,
        }
    }

    pub fn bitwise_operation(
        &self,
        node: u32,
    ) -> Option<(ScalarBitwiseOp, ScalarValueId, ScalarValueId)> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Bitwise { op, lhs, rhs } => Some((*op, *lhs, *rhs)),
            _ => None,
        }
    }

    pub fn inlined_calls(&self) -> u64 {
        self.values
            .iter()
            .filter(|value| matches!(value, ScalarValue::Call(call) if call.inline.is_some()))
            .count() as u64
    }

    pub fn call(&self, node: u32) -> Option<&ScalarCall> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Call(call) => Some(call),
            _ => None,
        }
    }

    pub fn comparison(&self, node: u32) -> Option<(ScalarCompareOp, ScalarValueId, ScalarValueId)> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Compare { op, lhs, rhs } => Some((*op, *lhs, *rhs)),
            _ => None,
        }
    }

    pub fn binary_operands(&self, node: u32) -> Option<(ScalarValueId, ScalarValueId)> {
        match self.values.get(self.value_for_node(node)?.index())? {
            ScalarValue::Binary { lhs, rhs, .. } => Some((*lhs, *rhs)),
            _ => None,
        }
    }

    pub fn frame_state_for_node(&self, node: u32) -> Option<&ScalarFrameState> {
        self.frame_states.get(node as usize)?.as_ref()
    }

    fn build_numeric_checks(
        &mut self,
        nodes: &[OptimizedNode],
        blocks: &[OptimizedBlock],
    ) -> Result<(), CompileFailure> {
        let work = nodes
            .len()
            .saturating_add(self.values.len())
            .saturating_mul(128);
        self.build_numeric_checks_with_budget(nodes, blocks, work)
    }

    pub(super) fn build_numeric_checks_with_budget(
        &mut self,
        nodes: &[OptimizedNode],
        blocks: &[OptimizedBlock],
        mut work: usize,
    ) -> Result<(), CompileFailure> {
        let indices = blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.start_pc(), index))
            .collect::<BTreeMap<_, _>>();
        let mut exits = BTreeMap::<u32, BTreeMap<ScalarValueId, ScalarNumericMode>>::new();
        let mut pending = (0..blocks.len()).collect::<VecDeque<_>>();
        let mut queued = vec![true; blocks.len()];
        // Unknown is the conservative initial state. Facts only become more
        // precise, and a missing predecessor never counts as evidence.
        while let Some(index) = pending.pop_front() {
            queued[index] = false;
            let block = &blocks[index];
            let cost = block.nodes().len()
                + self
                    .inputs_for_block(block.start_pc())
                    .iter()
                    .map(|&(_, value)| match &self.values[value.index()] {
                        ScalarValue::Phi { inputs, .. } => inputs.len(),
                        _ => 1,
                    })
                    .sum::<usize>();
            let Some(remaining) = work.checked_sub(cost.max(1)) else {
                // This optional optimization must not make large functions
                // uncompilable. Restore the proven block-local solution.
                exits.clear();
                for block in blocks {
                    self.numeric_checks_for_block(nodes, block, &exits)?;
                }
                return Ok(());
            };
            work = remaining;
            let facts = self.numeric_checks_for_block(nodes, block, &exits)?;
            if exits.get(&block.start_pc()) == Some(&facts) {
                continue;
            }
            exits.insert(block.start_pc(), facts);
            for successor in block.successors() {
                let &next = indices
                    .get(successor)
                    .ok_or(CompileFailure::InvalidArtifact)?;
                if !queued[next] {
                    queued[next] = true;
                    pending.push_back(next);
                }
            }
        }
        Ok(())
    }

    fn known_numeric(
        &self,
        value: ScalarValueId,
        facts: &BTreeMap<ScalarValueId, ScalarNumericMode>,
    ) -> Option<ScalarNumericMode> {
        facts.get(&value).copied().or_else(|| {
            matches!(self.values[value.index()], ScalarValue::Int32(_))
                .then_some(ScalarNumericMode::Int32)
        })
    }

    fn numeric_checks_for_block(
        &mut self,
        nodes: &[OptimizedNode],
        block: &OptimizedBlock,
        exits: &BTreeMap<u32, BTreeMap<ScalarValueId, ScalarNumericMode>>,
    ) -> Result<BTreeMap<ScalarValueId, ScalarNumericMode>, CompileFailure> {
        let mut facts = BTreeMap::<ScalarValueId, ScalarNumericMode>::new();
        for &(_, value) in self.inputs_for_block(block.start_pc()) {
            let ScalarValue::Phi { inputs, .. } = &self.values[value.index()] else {
                continue;
            };
            let mut incoming = inputs.iter().map(|input| {
                input
                    .predecessor
                    .and_then(|pc| exits.get(&pc))
                    .and_then(|exit| self.known_numeric(input.value, exit))
            });
            let first = incoming.next().flatten();
            let merged = incoming.fold(first, |left, right| match (left, right) {
                (Some(left), Some(right)) if left == right => Some(left),
                (Some(_), Some(_)) => Some(ScalarNumericMode::Number),
                _ => None,
            });
            if let Some(mode) = merged {
                facts.insert(value, mode);
            }
        }
        for &id in block.nodes() {
            let node = &nodes[id as usize];
            if node.eliminated()
                || !matches!(node.kind(), OptimizedNodeKind::Bytecode { opcode } if matches!(opcode.as_ref(), "add" | "sub" | "mul" | "div" | "lt" | "lte" | "gt" | "gte" | "inc" | "dec" | "post_inc" | "post_dec" | "inc_loc" | "dec_loc" | "or" | "and" | "xor" | "shl" | "sar" | "shr"))
            {
                continue;
            }
            let Some(result) = self.value_for_node(id) else {
                continue;
            };
            let (mode, operands, numeric_result) = match self.values[result.index()] {
                ScalarValue::Binary { mode, lhs, rhs, .. } => {
                    (mode, [Some(lhs), Some(rhs)], Some(mode))
                }
                ScalarValue::Update { mode, input, .. } => (mode, [Some(input), None], Some(mode)),
                ScalarValue::Compare { lhs, rhs, .. } => {
                    (ScalarNumericMode::Number, [Some(lhs), Some(rhs)], None)
                }
                ScalarValue::Bitwise { op, lhs, rhs } => (
                    ScalarNumericMode::Int32,
                    [Some(lhs), Some(rhs)],
                    Some(if op == ScalarBitwiseOp::Shr {
                        ScalarNumericMode::Number
                    } else {
                        ScalarNumericMode::Int32
                    }),
                ),
                _ => continue,
            };
            if self.frame_state_for_node(id).is_none() {
                return Err(CompileFailure::InvalidArtifact);
            }
            let mut checks = Vec::with_capacity(2);
            for value in operands.into_iter().flatten() {
                let known = facts.get(&value).copied().or_else(|| {
                    matches!(self.values[value.index()], ScalarValue::Int32(_))
                        .then_some(ScalarNumericMode::Int32)
                });
                let eliminated =
                    known.is_some_and(|known| known == mode || mode == ScalarNumericMode::Number);
                checks.push(ScalarCheck {
                    value,
                    mode,
                    frame_state_node: id,
                    eliminated,
                });
                if !eliminated {
                    facts.insert(value, mode);
                }
            }
            if let Some(result_mode) = numeric_result {
                facts.insert(result, result_mode);
            }
            self.checks[id as usize] = checks.into();
        }
        Ok(facts)
    }

    fn capture_frame(
        &mut self,
        node: u32,
        pc: u32,
        frame: &BTreeMap<(u8, u16), ScalarValueId>,
        stack: &[ScalarValueId],
        shape: OptimizedFrameShape,
    ) -> Result<(), CompileFailure> {
        let count = frame
            .len()
            .checked_add(stack.len())
            .ok_or(CompileFailure::ResourceLimit)?;
        self.state_entries = self
            .state_entries
            .checked_add(count)
            .ok_or(CompileFailure::ResourceLimit)?;
        if self.state_entries > self.node_values.len().saturating_mul(64) {
            return Err(CompileFailure::ResourceLimit);
        }
        self.frame_states[node as usize] = Some(ScalarFrameState {
            pc,
            arguments: (0..shape.arguments())
                .map(|index| frame[&(0, index)])
                .collect(),
            locals: (0..shape.locals())
                .map(|index| frame[&(1, index)])
                .collect(),
            stack: stack.into(),
            parent: None,
        });
        Ok(())
    }

    pub fn allocated_bytes(&self) -> usize {
        let check_bytes = self.checks.capacity() * core::mem::size_of::<Box<[ScalarCheck]>>()
            + self
                .checks
                .iter()
                .map(|checks| checks.len() * core::mem::size_of::<ScalarCheck>())
                .sum::<usize>();
        check_bytes
            + self.frame_states.capacity() * core::mem::size_of::<Option<ScalarFrameState>>()
            + self.state_entries * core::mem::size_of::<ScalarValueId>()
            + self.node_outputs.capacity() * core::mem::size_of::<Box<[ScalarValueId]>>()
            + self
                .node_outputs
                .iter()
                .map(|values| core::mem::size_of_val(values.as_ref()))
                .sum::<usize>()
            + self.frame_definitions.capacity()
                * core::mem::size_of::<Box<[(FrameSlot, ScalarValueId)]>>()
            + self
                .frame_definitions
                .iter()
                .map(|values| core::mem::size_of_val(values.as_ref()))
                .sum::<usize>()
            + self.block_inputs.capacity()
                * core::mem::size_of::<(u32, Box<[(FrameSlot, ScalarValueId)]>)>()
            + self
                .block_inputs
                .iter()
                .map(|(_, values)| core::mem::size_of_val(values.as_ref()))
                .sum::<usize>()
            + self.values.capacity() * core::mem::size_of::<ScalarValue>()
            + self
                .values
                .iter()
                .map(|value| match value {
                    ScalarValue::Phi { inputs, .. } => {
                        inputs.len() * core::mem::size_of::<ScalarPhiInput>()
                    }
                    ScalarValue::Call(call) => {
                        core::mem::size_of_val(call.arguments.as_ref())
                            + call
                                .inline
                                .as_ref()
                                .map_or(0, |region| region.allocated_bytes())
                    }
                    _ => 0,
                })
                .sum::<usize>()
            + self.node_values.capacity() * core::mem::size_of::<Option<ScalarValueId>>()
    }

    fn push(&mut self, value: ScalarValue) -> Result<ScalarValueId, CompileFailure> {
        if self.values.len() >= self.value_limit {
            return Err(CompileFailure::ResourceLimit);
        }
        let id = u32::try_from(self.values.len()).map_err(|_| CompileFailure::ResourceLimit)?;
        self.values.push(value);
        Ok(ScalarValueId(id))
    }

    fn expand_inline(
        &mut self,
        call: &ScalarCall,
        callee: &super::InlineCallee,
    ) -> Result<Option<Box<ScalarInlineRegion>>, CompileFailure> {
        use super::inlining::InlineValue;
        if !matches!(
            self.values[call.target.index()],
            ScalarValue::Input {
                slot: FrameSlot::Argument(_) | FrameSlot::Local(_),
                ..
            } | ScalarValue::Phi {
                slot: FrameSlot::Argument(_) | FrameSlot::Local(_),
                ..
            } | ScalarValue::FrameRead {
                slot: FrameSlot::Argument(_) | FrameSlot::Local(_),
                ..
            }
        ) {
            return Ok(None);
        }

        if call.receiver.is_some() || call.arguments.len() != callee.call.arguments().len() {
            return Ok(None);
        }
        let Some(plan) =
            callee.plan(
                |index| match self.values.get(call.arguments[index].index()) {
                    Some(ScalarValue::Bool(value)) => Some(*value),
                    _ => None,
                },
            )
        else {
            return Ok(None);
        };
        let state_entries = plan
            .values
            .iter()
            .map(|value| match value {
                InlineValue::Binary { stack, .. } => stack.len() + call.arguments.len(),
                _ => 0,
            })
            .sum::<usize>();
        if self.values.len().saturating_add(plan.values.len()) > self.value_limit
            || self.state_entries.saturating_add(state_entries)
                > self.node_values.len().saturating_mul(64)
        {
            return Ok(None);
        }
        let mut mapped = Vec::with_capacity(plan.values.len());
        let mut steps = Vec::new();
        for value in plan.values {
            let (value, frame) = match value {
                InlineValue::Argument(index) => {
                    mapped.push(call.arguments[index]);
                    continue;
                }
                InlineValue::Int32(value) => (ScalarValue::Int32(value), None),
                InlineValue::Bool(value) => (ScalarValue::Bool(value), None),
                InlineValue::Binary {
                    op,
                    lhs,
                    rhs,
                    pc,
                    stack,
                } => {
                    let frame = ScalarFrameState {
                        pc,
                        arguments: call.arguments.clone(),
                        locals: Box::new([]),
                        stack: stack.iter().map(|&id| mapped[id]).collect(),
                        parent: Some(call.frame_state_node),
                    };
                    (
                        ScalarValue::Binary {
                            mode: ScalarNumericMode::Int32,
                            op,
                            lhs: mapped[lhs],
                            rhs: mapped[rhs],
                        },
                        Some(frame),
                    )
                }
            };
            let value = self.push(value)?;
            mapped.push(value);
            steps.push(ScalarInlineStep { value, frame });
        }
        self.state_entries += state_entries;
        Ok(Some(Box::new(ScalarInlineRegion {
            artifact: callee.artifact,
            object_identity: callee.call.callee_identity(),
            bytecode_identity: callee.call.callee_bytecode_identity(),
            arguments: callee.call.arguments().into(),
            steps: steps.into(),
            result: mapped[plan.result],
        })))
    }

    pub(super) fn rewrite(
        nodes: &mut [OptimizedNode],
        blocks: &[OptimizedBlock],
        shape: OptimizedFrameShape,
        numeric_modes: &BTreeMap<u32, ScalarNumericMode>,
        inline_callees: &BTreeMap<u32, super::InlineCallee>,
    ) -> Result<(Self, u64), CompileFailure> {
        let mut graph = Self {
            values: Vec::new(),
            checks: vec![Box::new([]); nodes.len()],
            node_values: vec![None; nodes.len()],
            node_outputs: vec![Box::new([]); nodes.len()],
            frame_definitions: vec![Box::new([]); nodes.len()],
            block_inputs: Vec::new(),
            frame_states: vec![None; nodes.len()],
            state_entries: 0,
            // Block-input stacks can otherwise grow quadratically in CFGs
            // with many joins. Bound the pass independently of lowering.
            value_limit: nodes.len().saturating_mul(16),
        };
        // Allocate block definitions before processing bodies, so forward
        // edges and loop backedges refer to stable value identities.
        let mut slots = BTreeSet::new();
        slots.extend((0..shape.arguments()).map(|index| (0, index)));
        slots.extend((0..shape.locals()).map(|index| (1, index)));
        for node in nodes.iter() {
            if let OptimizedNodeKind::Bytecode { opcode } = node.kind() {
                if super::optimized::is_pure_load(opcode) || opcode.as_ref() == "get_loc_check" {
                    let index = super::optimized::indexed_node_operand(opcode, node.bytes())
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    slots.insert((u8::from(!opcode.starts_with("get_arg")), index));
                }
                if opcode.as_ref() == "get_loc0_loc1" {
                    slots.extend([(1, 0), (1, 1)]);
                }
            }
            if let Some(slot) = super::optimized::frame_write_slot(node) {
                slots.insert(slot);
            }
        }
        let mut entries = BTreeMap::new();
        let mut predecessors = BTreeMap::<u32, Vec<u32>>::new();
        for block in blocks {
            let mut frame = BTreeMap::new();
            for &key in &slots {
                frame.insert(
                    key,
                    graph.push(ScalarValue::Input {
                        block_pc: block.start_pc(),
                        slot: frame_slot(key),
                    })?,
                );
            }
            let mut stack = Vec::new();
            for index in 0..block.stack_depth() {
                stack.push(graph.push(ScalarValue::Input {
                    block_pc: block.start_pc(),
                    slot: FrameSlot::Stack(index),
                })?);
            }
            graph.block_inputs.push((
                block.start_pc(),
                frame
                    .iter()
                    .map(|(&key, &value)| (frame_slot(key), value))
                    .chain(
                        stack
                            .iter()
                            .enumerate()
                            .map(|(index, &value)| (FrameSlot::Stack(index as u16), value)),
                    )
                    .collect(),
            ));
            entries.insert(block.start_pc(), (frame, stack));
            for &successor in block.successors() {
                predecessors
                    .entry(successor)
                    .or_default()
                    .push(block.start_pc());
            }
        }
        graph.block_inputs.sort_unstable_by_key(|(pc, _)| *pc);
        let mut exits = BTreeMap::new();
        let mut eliminated = 0;
        for block in blocks {
            let (mut frame, mut stack) = entries[&block.start_pc()].clone();
            let mut expressions = BTreeMap::new();
            let mut constants = BTreeMap::<i32, ScalarValueId>::new();
            for &id in block.nodes() {
                let node = &mut nodes[id as usize];
                let base = stack
                    .len()
                    .checked_sub(usize::from(node.pops()))
                    .ok_or(CompileFailure::InvalidArtifact)?;
                if node.eliminated() {
                    stack.truncate(base);
                    for _ in 0..node.pushes() {
                        stack.push(graph.push(ScalarValue::Opaque)?);
                    }
                    continue;
                }
                if node.deopt_guard().is_some()
                    && !matches!(node.kind(), OptimizedNodeKind::Reuse { .. })
                {
                    graph.capture_frame(id, node.pc(), &frame, &stack, shape)?;
                }
                let result = match node.kind() {
                    OptimizedNodeKind::Reuse { source } => Some(
                        graph
                            .value_for_node(*source)
                            .ok_or(CompileFailure::InvalidArtifact)?,
                    ),
                    OptimizedNodeKind::Bytecode { opcode } => {
                        if matches!(
                            opcode.as_ref(),
                            "call" | "call0" | "call1" | "call2" | "call3" | "call_method"
                        ) {
                            let receiver_count = usize::from(opcode.as_ref() == "call_method");
                            if node.pushes() != 1
                                || usize::from(node.pops()) < 1 + receiver_count
                                || node.effect() != OptimizedEffect::Reentrant
                                || graph.frame_state_for_node(id).is_none()
                            {
                                return Err(CompileFailure::InvalidArtifact);
                            }
                            let target = base + receiver_count;
                            let mut call = ScalarCall {
                                target: stack[target],
                                receiver: (receiver_count != 0).then(|| stack[base]),
                                arguments: stack[target + 1..].into(),
                                frame_state_node: id,
                                inline: None,
                            };
                            if let Some(callee) = inline_callees.get(&node.pc()) {
                                call.inline = graph.expand_inline(&call, callee)?;
                                if call.inline.is_some() {
                                    node.mark_effect_free_inline();
                                }
                            }
                            Some(graph.push(ScalarValue::Call(call))?)
                        } else if matches!(opcode.as_ref(), "push_true" | "push_false") {
                            Some(graph.push(ScalarValue::Bool(opcode.as_ref() == "push_true"))?)
                        } else if let Some(constant) = integer_constant(opcode, node.bytes()) {
                            let value = if let Some(value) = constants.get(&constant) {
                                *value
                            } else {
                                let value = graph.push(ScalarValue::Int32(constant))?;
                                constants.insert(constant, value);
                                value
                            };
                            Some(value)
                        } else if super::optimized::is_pure_load(opcode)
                            || opcode.as_ref() == "get_loc_check"
                        {
                            let index =
                                super::optimized::indexed_node_operand(opcode, node.bytes())
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                            let argument = opcode.starts_with("get_arg");
                            let key = (u8::from(!argument), index);
                            let value = *frame.get(&key).ok_or(CompileFailure::InvalidArtifact)?;
                            Some(value)
                        } else if opcode.starts_with("set_arg")
                            || opcode.starts_with("set_loc")
                                && opcode.as_ref() != "set_loc_uninitialized"
                        {
                            stack.last().copied()
                        } else if matches!(
                            opcode.as_ref(),
                            "inc" | "dec" | "post_inc" | "post_dec" | "inc_loc" | "dec_loc"
                        ) {
                            let input = if opcode.ends_with("_loc") {
                                let index =
                                    *node.bytes().get(1).ok_or(CompileFailure::InvalidArtifact)?;
                                *frame
                                    .get(&(1, u16::from(index)))
                                    .ok_or(CompileFailure::InvalidArtifact)?
                            } else {
                                *stack.last().ok_or(CompileFailure::InvalidArtifact)?
                            };
                            Some(graph.push(ScalarValue::Update {
                                mode: numeric_modes.get(&node.pc()).copied().unwrap_or_default(),
                                input,
                                delta: if opcode.contains("inc") { 1 } else { -1 },
                            })?)
                        } else if let Some(op) = ScalarBitwiseOp::from_opcode(opcode) {
                            if node.pops() != 2 || node.pushes() != 1 {
                                return Err(CompileFailure::InvalidArtifact);
                            }
                            Some(graph.push(ScalarValue::Bitwise {
                                op,
                                lhs: stack[base],
                                rhs: stack[base + 1],
                            })?)
                        } else if let Some(op) = ScalarCompareOp::from_opcode(opcode) {
                            if node.pops() != 2 || node.pushes() != 1 {
                                return Err(CompileFailure::InvalidArtifact);
                            }
                            Some(graph.push(ScalarValue::Compare {
                                op,
                                lhs: stack[base],
                                rhs: stack[base + 1],
                            })?)
                        } else if let Some(op) = ScalarBinaryOp::from_opcode(opcode) {
                            if node.pops() != 2 || node.pushes() != 1 {
                                return Err(CompileFailure::InvalidArtifact);
                            }
                            let lhs = stack[base];
                            let rhs = stack[base + 1];
                            let mode = numeric_modes.get(&node.pc()).copied().unwrap_or_default();
                            let key = (op, mode, lhs, rhs, node.representation() as u8);
                            if let Some(&(value, source)) = expressions.get(&key) {
                                // Keep the original operand consumption. The
                                // backend's SSA DCE removes now-unused loads;
                                // no adjacency or single-use assumption is made.
                                node.reuse_value(source);
                                eliminated += 1;
                                Some(value)
                            } else {
                                let value =
                                    graph.push(ScalarValue::Binary { mode, op, lhs, rhs })?;
                                expressions.insert(key, (value, id));
                                Some(value)
                            }
                        } else {
                            None
                        }
                    }
                    OptimizedNodeKind::GuardNumeric { .. } => None,
                };
                let permutation = match node.kind() {
                    OptimizedNodeKind::Bytecode { opcode } => stack_permutation(opcode),
                    _ => None,
                };
                let shuffled = permutation
                    .map(|(pops, order)| {
                        if pops != usize::from(node.pops())
                            || order.len() != usize::from(node.pushes())
                        {
                            return Err(CompileFailure::InvalidArtifact);
                        }
                        Ok(order
                            .iter()
                            .map(|&index| stack[base + index])
                            .collect::<Vec<_>>())
                    })
                    .transpose()?;
                let mut definitions = Vec::new();
                if node.effect() != OptimizedEffect::Pure {
                    expressions.clear();
                    if node.effect() == OptimizedEffect::FrameWrite {
                        if let Some(slot) = super::optimized::frame_write_slot(node) {
                            let is_store = matches!(node.kind(), OptimizedNodeKind::Bytecode { opcode }
                                if (opcode.starts_with("put_") || opcode.starts_with("set_"))
                                    && opcode.as_ref() != "set_loc_uninitialized");
                            let value = if matches!(node.kind(), OptimizedNodeKind::Bytecode { opcode } if matches!(opcode.as_ref(), "inc_loc" | "dec_loc"))
                            {
                                result.ok_or(CompileFailure::InvalidArtifact)?
                            } else if is_store {
                                *stack.last().ok_or(CompileFailure::InvalidArtifact)?
                            } else {
                                graph.push(ScalarValue::FrameRead {
                                    node: id,
                                    slot: frame_slot(slot),
                                })?
                            };
                            frame.insert(slot, value);
                            definitions.push((frame_slot(slot), value));
                        } else {
                            for (&slot, value) in &mut frame {
                                *value = graph.push(ScalarValue::FrameRead {
                                    node: id,
                                    slot: frame_slot(slot),
                                })?;
                                definitions.push((frame_slot(slot), *value));
                            }
                        }
                    } else if matches!(
                        node.effect(),
                        OptimizedEffect::Reentrant | OptimizedEffect::Poll
                    ) {
                        for (&slot, value) in &mut frame {
                            *value = graph.push(ScalarValue::FrameRead {
                                node: id,
                                slot: frame_slot(slot),
                            })?;
                            definitions.push((frame_slot(slot), *value));
                        }
                    }
                }
                graph.frame_definitions[id as usize] = definitions.into();
                stack.truncate(base);
                for output in 0..node.pushes() {
                    let value = if matches!(node.kind(), OptimizedNodeKind::Bytecode { opcode } if matches!(opcode.as_ref(), "post_inc" | "post_dec"))
                    {
                        let value = result.ok_or(CompileFailure::InvalidArtifact)?;
                        let ScalarValue::Update { input, .. } = graph.values[value.index()] else {
                            return Err(CompileFailure::InvalidArtifact);
                        };
                        if output == 0 {
                            input
                        } else {
                            value
                        }
                    } else if let Some(values) = &shuffled {
                        values[usize::from(output)]
                    } else if matches!(node.kind(), OptimizedNodeKind::Bytecode { opcode } if opcode.as_ref() == "get_loc0_loc1")
                    {
                        frame[&(1, u16::from(output))]
                    } else if output == 0 {
                        match result {
                            Some(value) => value,
                            None => graph.push(ScalarValue::Opaque)?,
                        }
                    } else {
                        graph.push(ScalarValue::Opaque)?
                    };
                    stack.push(value);
                    if node.pushes() == 1 {
                        graph.node_values[id as usize] = Some(value);
                    }
                }
                graph.node_outputs[id as usize] = stack[base..].into();
                if result.is_some_and(|value| {
                    matches!(graph.values[value.index()], ScalarValue::Update { .. })
                }) {
                    graph.node_values[id as usize] = result;
                }
            }
            exits.insert(block.start_pc(), (frame, stack));
        }
        let mut edge_count = 0usize;
        for (pc, (frame, stack)) in entries {
            let incoming = predecessors.get(&pc).map(Vec::as_slice).unwrap_or(&[]);
            for (slot, value) in frame
                .into_iter()
                .map(|(key, value)| (frame_slot(key), value))
                .chain(
                    stack
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| (FrameSlot::Stack(index as u16), value)),
                )
            {
                if incoming.is_empty() {
                    if pc != 0 {
                        return Err(CompileFailure::InvalidArtifact);
                    }
                    continue;
                }
                edge_count = edge_count
                    .checked_add(incoming.len() + usize::from(pc == 0))
                    .ok_or(CompileFailure::ResourceLimit)?;
                if edge_count > nodes.len().saturating_mul(32) {
                    return Err(CompileFailure::ResourceLimit);
                }
                let mut inputs = Vec::with_capacity(incoming.len() + usize::from(pc == 0));
                if pc == 0 {
                    inputs.push(ScalarPhiInput {
                        predecessor: None,
                        value: graph.push(ScalarValue::Input { block_pc: pc, slot })?,
                    });
                }
                for &predecessor in incoming {
                    let (frame, stack) = exits
                        .get(&predecessor)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let value = match slot {
                        FrameSlot::Argument(index) => frame.get(&(0, index)),
                        FrameSlot::Local(index) => frame.get(&(1, index)),
                        FrameSlot::Stack(index) => stack.get(usize::from(index)),
                    }
                    .copied()
                    .ok_or(CompileFailure::InvalidArtifact)?;
                    inputs.push(ScalarPhiInput {
                        predecessor: Some(predecessor),
                        value,
                    });
                }
                graph.values[value.index()] = ScalarValue::Phi {
                    block_pc: pc,
                    slot,
                    inputs: inputs.into(),
                };
            }
        }
        // Entry guard runs before the loop-header Phi, so use the implicit
        // initial edge rather than a loop-carried definition for that state.
        let mut initial = BTreeMap::new();
        for &(slot, value) in graph.inputs_for_block(0) {
            let value = match &graph.values[value.index()] {
                ScalarValue::Phi { inputs, .. } => {
                    inputs
                        .iter()
                        .find(|input| input.predecessor.is_none())
                        .ok_or(CompileFailure::InvalidArtifact)?
                        .value
                }
                _ => value,
            };
            match slot {
                FrameSlot::Argument(index) => {
                    initial.insert((0, index), value);
                }
                FrameSlot::Local(index) => {
                    initial.insert((1, index), value);
                }
                FrameSlot::Stack(_) => return Err(CompileFailure::InvalidArtifact),
            }
        }
        if let Some(entry) = nodes.first().filter(|node| {
            matches!(
                node.kind(),
                OptimizedNodeKind::GuardNumeric {
                    mid_loop: false,
                    ..
                }
            )
        }) {
            graph.capture_frame(entry.id(), entry.pc(), &initial, &[], shape)?;
        }
        graph.build_numeric_checks(nodes, blocks)?;
        Ok((graph, eliminated))
    }
}

pub(super) fn integer_constant(name: &str, bytes: &[u8]) -> Option<i32> {
    match name {
        "push_minus1" => Some(-1),
        "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5" | "push_6" | "push_7" => {
            Some(i32::from(name.as_bytes()[5] - b'0'))
        }
        "push_i8" => Some(i32::from(*bytes.get(1)? as i8)),
        "push_i16" => Some(i32::from(i16::from_le_bytes(
            bytes.get(1..3)?.try_into().ok()?,
        ))),
        "push_i32" => Some(i32::from_le_bytes(bytes.get(1..5)?.try_into().ok()?)),
        _ => None,
    }
}

fn frame_slot((kind, index): (u8, u16)) -> FrameSlot {
    if kind == 0 {
        FrameSlot::Argument(index)
    } else {
        FrameSlot::Local(index)
    }
}

/// Interpreter-compatible consumed stack size and output source indices.
pub(crate) fn stack_permutation(name: &str) -> Option<(usize, &'static [usize])> {
    Some(match name {
        "nip" => (2, &[1]),
        "nip1" => (3, &[1, 2]),
        "dup" => (1, &[0, 0]),
        "dup1" => (2, &[0, 0, 1]),
        "dup2" => (2, &[0, 1, 0, 1]),
        "dup3" => (3, &[0, 1, 2, 0, 1, 2]),
        "insert2" => (2, &[1, 0, 1]),
        "insert3" => (3, &[2, 0, 1, 2]),
        "insert4" => (4, &[3, 0, 1, 2, 3]),
        "perm3" => (3, &[1, 0, 2]),
        "perm4" => (4, &[2, 0, 1, 3]),
        "perm5" => (5, &[3, 0, 1, 2, 4]),
        "swap" => (2, &[1, 0]),
        "swap2" => (4, &[2, 3, 0, 1]),
        "rot3l" => (3, &[1, 2, 0]),
        "rot3r" => (3, &[2, 0, 1]),
        "rot4l" => (4, &[1, 2, 3, 0]),
        "rot5l" => (5, &[1, 2, 3, 4, 0]),
        _ => return None,
    })
}
