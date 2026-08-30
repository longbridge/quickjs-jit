use super::TaggedValue;
use crate::{bytecode::VerifiedFunction, compiler::CompileFailure};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueRepresentation {
    Tagged,
    Int32,
    Float64,
    Effect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptimizedEffect {
    Pure,
    FrameWrite,
    Control,
    Poll,
    Reentrant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptimizedNodeKind {
    GuardNumeric { guard: u32, mid_loop: bool },
    Bytecode { opcode: Box<str> },
    Reuse { source: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizedNode {
    id: u32,
    pc: u32,
    kind: OptimizedNodeKind,
    representation: ValueRepresentation,
    effect: OptimizedEffect,
    eliminated: bool,
    bytes: Box<[u8]>,
    branch_target: Option<u32>,
    pops: u8,
    pushes: u8,
}

impl OptimizedNode {
    pub const fn id(&self) -> u32 {
        self.id
    }
    pub const fn pc(&self) -> u32 {
        self.pc
    }
    pub fn kind(&self) -> &OptimizedNodeKind {
        &self.kind
    }
    pub const fn representation(&self) -> ValueRepresentation {
        self.representation
    }
    pub const fn effect(&self) -> OptimizedEffect {
        self.effect
    }
    pub const fn eliminated(&self) -> bool {
        self.eliminated
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub const fn branch_target(&self) -> Option<u32> {
        self.branch_target
    }
    pub const fn pops(&self) -> u8 {
        self.pops
    }
    pub const fn pushes(&self) -> u8 {
        self.pushes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizedBlock {
    start_pc: u32,
    stack_depth: u16,
    successors: Box<[u32]>,
    loop_header: bool,
    nodes: Box<[u32]>,
}

impl OptimizedBlock {
    pub const fn start_pc(&self) -> u32 {
        self.start_pc
    }
    pub const fn stack_depth(&self) -> u16 {
        self.stack_depth
    }
    pub fn successors(&self) -> &[u32] {
        &self.successors
    }
    pub const fn is_loop_header(&self) -> bool {
        self.loop_header
    }
    pub fn nodes(&self) -> &[u32] {
        &self.nodes
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizedMetrics {
    pub boxes_elided: u64,
    pub cse_eliminated: u64,
    pub dead_nodes_eliminated: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuardSite {
    guard: u32,
    shape: OptimizedFrameShape,
    map: DeoptMap,
}

impl GuardSite {
    pub const fn guard(&self) -> u32 {
        self.guard
    }
    pub const fn shape(&self) -> OptimizedFrameShape {
        self.shape
    }
    pub const fn map(&self) -> &DeoptMap {
        &self.map
    }
}

/// QuickJS-specific optimizing IR. It is translated directly from verified
/// bytecode and never accepts or contains the baseline IR.
#[derive(Clone, Debug)]
pub struct OptimizedIr {
    blocks: Box<[OptimizedBlock]>,
    nodes: Box<[OptimizedNode]>,
    machine_plan: Box<[OptimizedNode]>,
    guards: Box<[GuardSite]>,
    metrics: OptimizedMetrics,
    feedback_epoch: u64,
    max_stack: u16,
}

impl OptimizedIr {
    pub fn translate(
        function: &VerifiedFunction,
        feedback_epoch: u64,
    ) -> Result<Self, CompileFailure> {
        if feedback_epoch == 0 {
            return Err(CompileFailure::InvalidArtifact);
        }
        let snapshot = function.snapshot();
        // Entry and loop-header guards are instruction boundaries with an
        // empty operand stack. Only live arguments/locals participate in the
        // deopt transaction; capacity-only stack slots are not roots.
        let shape = OptimizedFrameShape::new(snapshot.arg_count(), snapshot.local_count(), 0);
        let mut nodes = Vec::new();
        let mut guards = Vec::new();
        let mut next_guard = 0u32;
        let block_depths = optimized_block_depths(function)?;
        if function.control_flow_graph().blocks().iter().any(|block| {
            function
                .control_flow_graph()
                .is_loop_header(block.start_pc())
                && block_depths.get(&block.start_pc()).copied().unwrap_or(1) != 0
        }) {
            return Err(CompileFailure::UnsupportedOpcode);
        }
        let mut make_guard = |pc: u32,
                              mid_loop: bool,
                              nodes: &mut Vec<OptimizedNode>,
                              guards: &mut Vec<GuardSite>|
         -> Result<u32, CompileFailure> {
            let guard = next_guard;
            next_guard = next_guard
                .checked_add(1)
                .ok_or(CompileFailure::ResourceLimit)?;
            let mut recipes = Vec::with_capacity(shape.slot_count());
            let mut flat = 0u16;
            for index in 0..snapshot.arg_count() {
                recipes.push(Materialization::argument(
                    index,
                    MaterializedValue::TaggedSlot(flat),
                ));
                flat = flat.checked_add(1).ok_or(CompileFailure::ResourceLimit)?;
            }
            for index in 0..snapshot.local_count() {
                recipes.push(Materialization::local(
                    index,
                    MaterializedValue::TaggedSlot(flat),
                ));
                flat = flat.checked_add(1).ok_or(CompileFailure::ResourceLimit)?;
            }
            for index in 0..shape.stack() {
                recipes.push(Materialization::stack(
                    index,
                    MaterializedValue::TaggedSlot(flat),
                ));
                flat = flat.checked_add(1).ok_or(CompileFailure::ResourceLimit)?;
            }
            let map = DeoptMap::new(guard, pc, DeoptPhase::BeforeEffect(0), recipes);
            map.validate(shape)
                .map_err(|_| CompileFailure::InvalidArtifact)?;
            let id = u32::try_from(nodes.len()).map_err(|_| CompileFailure::ResourceLimit)?;
            nodes.push(OptimizedNode {
                id,
                pc,
                kind: OptimizedNodeKind::GuardNumeric { guard, mid_loop },
                representation: ValueRepresentation::Effect,
                effect: OptimizedEffect::Control,
                eliminated: false,
                bytes: Box::new([]),
                branch_target: None,
                pops: 0,
                pushes: 0,
            });
            guards.push(GuardSite { guard, shape, map });
            Ok(id)
        };
        make_guard(0, false, &mut nodes, &mut guards)?;
        let mut blocks = Vec::with_capacity(function.control_flow_graph().blocks().len());
        let mut boxes_elided = 0u64;
        for block in function.control_flow_graph().blocks() {
            let mut block_nodes = Vec::new();
            if function
                .control_flow_graph()
                .is_loop_header(block.start_pc())
            {
                block_nodes.push(make_guard(block.start_pc(), true, &mut nodes, &mut guards)?);
            }
            for instruction in &function.instructions()[block.instruction_range()] {
                let name = instruction.opcode().name();
                let (representation, effect) = classify_optimized_opcode(name)?;
                boxes_elided = boxes_elided.saturating_add(u64::from(matches!(
                    representation,
                    ValueRepresentation::Int32 | ValueRepresentation::Float64
                )));
                let id = u32::try_from(nodes.len()).map_err(|_| CompileFailure::ResourceLimit)?;
                // Stack drops and bytecode nops have no machine operation once
                // their SSA uses are dead; producers are removed by the
                // backwards liveness pass in the lowering stage.
                nodes.push(OptimizedNode {
                    id,
                    pc: instruction.pc(),
                    kind: OptimizedNodeKind::Bytecode {
                        opcode: name.into(),
                    },
                    representation,
                    effect,
                    eliminated: matches!(name, "nop" | "drop"),
                    bytes: instruction.bytes().into(),
                    branch_target: instruction
                        .branch_target()
                        .and_then(|target| u32::try_from(target).ok()),
                    pops: instruction.opcode().n_pop(),
                    pushes: instruction.opcode().n_push(),
                });
                block_nodes.push(id);
            }
            blocks.push(OptimizedBlock {
                start_pc: block.start_pc(),
                stack_depth: *block_depths
                    .get(&block.start_pc())
                    .ok_or(CompileFailure::InvalidArtifact)?,
                successors: block.successors().into(),
                loop_header: function
                    .control_flow_graph()
                    .is_loop_header(block.start_pc()),
                nodes: block_nodes.into(),
            });
        }
        let (cse_eliminated, dead_nodes_eliminated) = rewrite_pure_expressions(&mut nodes, &blocks);
        let machine_plan = nodes
            .iter()
            .filter(|node| !node.eliminated)
            .cloned()
            .collect::<Vec<_>>();
        Ok(Self {
            blocks: blocks.into(),
            nodes: nodes.into(),
            machine_plan: machine_plan.into(),
            guards: guards.into(),
            metrics: OptimizedMetrics {
                boxes_elided,
                cse_eliminated,
                dead_nodes_eliminated,
            },
            feedback_epoch,
            max_stack: snapshot.stack_size(),
        })
    }
    pub fn blocks(&self) -> &[OptimizedBlock] {
        &self.blocks
    }
    pub fn nodes(&self) -> &[OptimizedNode] {
        &self.nodes
    }
    pub fn machine_plan(&self) -> &[OptimizedNode] {
        &self.machine_plan
    }
    pub fn guard_maps(&self) -> &[GuardSite] {
        &self.guards
    }
    pub const fn metrics(&self) -> OptimizedMetrics {
        self.metrics
    }
    pub const fn feedback_epoch(&self) -> u64 {
        self.feedback_epoch
    }
    pub const fn max_stack(&self) -> u16 {
        self.max_stack
    }
}

fn rewrite_pure_expressions(nodes: &mut [OptimizedNode], blocks: &[OptimizedBlock]) -> (u64, u64) {
    use std::collections::BTreeMap;
    let mut cse = 0u64;
    let mut dead = 0u64;
    for block in blocks {
        let ids = block.nodes();
        let mut expressions = BTreeMap::<(Box<str>, Box<str>, Box<str>), u32>::new();
        let mut index = 0usize;
        while index < ids.len() {
            if index + 3 < ids.len() {
                let quartet = [ids[index], ids[index + 1], ids[index + 2], ids[index + 3]];
                let names = quartet.map(|id| opcode_name(&nodes[id as usize]));
                if names[3] == Some("drop")
                    && names[0].is_some_and(is_pure_load)
                    && names[1].is_some_and(is_pure_load)
                    && names[2].is_some_and(is_pure_binary)
                {
                    for id in quartet {
                        let node = &mut nodes[id as usize];
                        node.eliminated = true;
                        node.pops = 0;
                        node.pushes = 0;
                        dead = dead.saturating_add(1);
                    }
                    index += 4;
                    continue;
                }
            }
            if index + 2 < ids.len() {
                let triple = [ids[index], ids[index + 1], ids[index + 2]];
                let names = triple.map(|id| opcode_name(&nodes[id as usize]));
                if names[0].is_some_and(is_pure_load)
                    && names[1].is_some_and(is_pure_load)
                    && names[2].is_some_and(is_pure_binary)
                {
                    let key = (
                        names[2].unwrap().into(),
                        names[0].unwrap().into(),
                        names[1].unwrap().into(),
                    );
                    if let Some(source) = expressions.get(&key).copied() {
                        for id in &triple[..2] {
                            let node = &mut nodes[*id as usize];
                            node.eliminated = true;
                            node.pops = 0;
                            node.pushes = 0;
                        }
                        let node = &mut nodes[triple[2] as usize];
                        node.kind = OptimizedNodeKind::Reuse { source };
                        node.pops = 0;
                        node.pushes = 1;
                        cse = cse.saturating_add(1);
                    } else {
                        expressions.insert(key, triple[2]);
                    }
                    index += 3;
                    continue;
                }
            }
            index += 1;
        }
    }
    (cse, dead)
}

fn opcode_name(node: &OptimizedNode) -> Option<&str> {
    match &node.kind {
        OptimizedNodeKind::Bytecode { opcode } => Some(opcode),
        _ => None,
    }
}

fn is_pure_load(name: &str) -> bool {
    matches!(
        name,
        "get_arg"
            | "get_arg0"
            | "get_arg1"
            | "get_arg2"
            | "get_arg3"
            | "get_loc"
            | "get_loc8"
            | "get_loc0"
            | "get_loc1"
            | "get_loc2"
            | "get_loc3"
    )
}

fn is_pure_binary(name: &str) -> bool {
    matches!(name, "add" | "sub" | "mul" | "div")
}

fn optimized_block_depths(
    function: &VerifiedFunction,
) -> Result<std::collections::BTreeMap<u32, u16>, CompileFailure> {
    use std::collections::{BTreeMap, VecDeque};
    let mut depths = BTreeMap::from([(0u32, 0u16)]);
    let mut queue = VecDeque::from([0u32]);
    while let Some(pc) = queue.pop_front() {
        let block = function
            .control_flow_graph()
            .block(pc)
            .ok_or(CompileFailure::InvalidArtifact)?;
        let mut depth = *depths.get(&pc).ok_or(CompileFailure::InvalidArtifact)?;
        for instruction in &function.instructions()[block.instruction_range()] {
            depth = depth
                .checked_sub(u16::from(instruction.opcode().n_pop()))
                .ok_or(CompileFailure::InvalidArtifact)?;
            depth = depth
                .checked_add(u16::from(instruction.opcode().n_push()))
                .ok_or(CompileFailure::ResourceLimit)?;
        }
        for successor in block.successors() {
            match depths.get(successor) {
                Some(existing) if *existing != depth => {
                    return Err(CompileFailure::InvalidArtifact)
                }
                Some(_) => {}
                None => {
                    depths.insert(*successor, depth);
                    queue.push_back(*successor);
                }
            }
        }
    }
    Ok(depths)
}

fn classify_optimized_opcode(
    name: &str,
) -> Result<(ValueRepresentation, OptimizedEffect), CompileFailure> {
    let result = match name {
        "push_i8" | "push_i16" | "push_i32" | "push_0" | "push_1" | "push_2" | "push_3"
        | "push_4" | "push_5" | "push_6" | "push_7" | "add_loc" | "inc_loc" | "dec_loc"
        | "post_inc" | "post_dec" | "inc" | "dec" | "shl" | "sar" | "shr" | "and" | "or"
        | "xor" => (ValueRepresentation::Int32, OptimizedEffect::Pure),
        "add" | "sub" | "mul" | "div" | "mod" | "plus" | "neg" => {
            (ValueRepresentation::Float64, OptimizedEffect::Pure)
        }
        "get_arg" | "get_arg0" | "get_arg1" | "get_arg2" | "get_arg3" | "get_loc" | "get_loc8"
        | "get_loc0" | "get_loc1" | "get_loc2" | "get_loc3" | "get_loc_check" | "get_loc0_loc1"
        | "undefined" | "null" | "push_true" | "push_false" | "dup" | "dup1" | "dup2" | "dup3" => {
            (ValueRepresentation::Tagged, OptimizedEffect::Pure)
        }
        "put_arg"
        | "put_loc"
        | "put_loc8"
        | "put_loc0"
        | "put_loc1"
        | "put_loc2"
        | "put_loc3"
        | "put_loc_check"
        | "put_loc_check_init"
        | "set_loc_uninitialized"
        | "drop" => (ValueRepresentation::Effect, OptimizedEffect::FrameWrite),
        "if_false" | "if_true" | "if_false8" | "if_true8" | "goto" | "goto8" | "goto16"
        | "return" | "return_undef" | "lt" | "lte" | "gt" | "gte" | "eq" | "neq" | "strict_eq"
        | "strict_neq" | "lnot" | "nop" => (ValueRepresentation::Effect, OptimizedEffect::Control),
        _ => return Err(CompileFailure::UnsupportedOpcode),
    };
    Ok(result)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeoptSlot {
    Argument(u16),
    Local(u16),
    Stack(u16),
}

#[derive(Clone, Copy, Debug)]
pub enum MaterializedValue {
    Poison,
    Undefined,
    Null,
    Bool(bool),
    Int32(i32),
    Float64(f64),
    TaggedSlot(u16),
}

impl PartialEq for MaterializedValue {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Poison, Self::Poison)
            | (Self::Undefined, Self::Undefined)
            | (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int32(a), Self::Int32(b)) => a == b,
            (Self::Float64(a), Self::Float64(b)) => a.to_bits() == b.to_bits(),
            (Self::TaggedSlot(a), Self::TaggedSlot(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptimizedFrameShape {
    arguments: u16,
    locals: u16,
    stack: u16,
}

impl OptimizedFrameShape {
    pub const fn new(arguments: u16, locals: u16, stack: u16) -> Self {
        Self {
            arguments,
            locals,
            stack,
        }
    }
    pub const fn slot_count(self) -> usize {
        self.arguments as usize + self.locals as usize + self.stack as usize
    }
    pub const fn arguments(self) -> u16 {
        self.arguments
    }
    pub const fn locals(self) -> u16 {
        self.locals
    }
    pub const fn stack(self) -> u16 {
        self.stack
    }
    fn index(self, slot: DeoptSlot) -> Option<usize> {
        match slot {
            DeoptSlot::Argument(i) if i < self.arguments => Some(i as usize),
            DeoptSlot::Local(i) if i < self.locals => Some(self.arguments as usize + i as usize),
            DeoptSlot::Stack(i) if i < self.stack => {
                Some(self.arguments as usize + self.locals as usize + i as usize)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeoptPhase {
    BeforeEffect(u64),
    AfterEffect(u64),
}

impl DeoptPhase {
    pub const fn side_effect_epoch(self) -> u64 {
        match self {
            Self::BeforeEffect(epoch) => epoch.saturating_sub(1),
            Self::AfterEffect(epoch) => epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Materialization {
    slot: DeoptSlot,
    value: MaterializedValue,
}

impl Materialization {
    pub const fn argument(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Argument(index),
            value,
        }
    }
    pub const fn local(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Local(index),
            value,
        }
    }
    pub const fn stack(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Stack(index),
            value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeoptValidationError {
    SlotCount,
    DuplicateSlot,
    InvalidSlot,
    DestinationSize,
    UnsupportedRecipe,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeoptMap {
    guard: u32,
    resume_pc: u32,
    phase: DeoptPhase,
    slots: Box<[Materialization]>,
}

impl DeoptMap {
    pub fn new(guard: u32, resume_pc: u32, phase: DeoptPhase, slots: Vec<Materialization>) -> Self {
        Self {
            guard,
            resume_pc,
            phase,
            slots: slots.into_boxed_slice(),
        }
    }
    pub const fn guard(&self) -> u32 {
        self.guard
    }
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn phase(&self) -> DeoptPhase {
        self.phase
    }
    pub const fn materialization_count(&self) -> usize {
        self.slots.len()
    }
    pub fn validate(&self, shape: OptimizedFrameShape) -> Result<(), DeoptValidationError> {
        if self.slots.len() != shape.slot_count() {
            return Err(DeoptValidationError::SlotCount);
        }
        let mut seen = vec![false; shape.slot_count()];
        for recipe in &self.slots {
            let index = shape
                .index(recipe.slot)
                .ok_or(DeoptValidationError::InvalidSlot)?;
            if seen[index] {
                return Err(DeoptValidationError::DuplicateSlot);
            }
            seen[index] = true;
        }
        if seen.iter().any(|present| !present) {
            return Err(DeoptValidationError::SlotCount);
        }
        Ok(())
    }

    /// Validates the narrow Tier 2 in-place deopt transaction. Every current
    /// production recipe aliases its own already-rooted frame slot, so the
    /// two-phase transaction performs complete validation before committing
    /// the resume state and requires no refcount mutation.
    pub fn validate_identity_materialization(
        &self,
        shape: OptimizedFrameShape,
    ) -> Result<(), DeoptValidationError> {
        self.validate(shape)?;
        if shape.stack() != 0 {
            return Err(DeoptValidationError::UnsupportedRecipe);
        }
        for recipe in &self.slots {
            let destination = shape
                .index(recipe.slot)
                .ok_or(DeoptValidationError::InvalidSlot)?;
            if recipe.value != MaterializedValue::TaggedSlot(destination as u16) {
                return Err(DeoptValidationError::UnsupportedRecipe);
            }
        }
        Ok(())
    }
    pub fn materialize(
        &self,
        shape: OptimizedFrameShape,
    ) -> Result<MaterializedFrame, DeoptValidationError> {
        let mut slots = vec![MaterializedValue::Poison; shape.slot_count()];
        self.materialize_into(shape, &mut slots)?;
        Ok(MaterializedFrame {
            resume_pc: self.resume_pc,
            side_effect_epoch: self.phase.side_effect_epoch(),
            slots: slots.into_boxed_slice(),
        })
    }
    pub fn materialize_into(
        &self,
        shape: OptimizedFrameShape,
        destination: &mut [MaterializedValue],
    ) -> Result<(), DeoptValidationError> {
        self.validate(shape)?;
        if destination.len() != shape.slot_count() {
            return Err(DeoptValidationError::DestinationSize);
        }
        let plan = self
            .slots
            .iter()
            .map(|recipe| (shape.index(recipe.slot).expect("validated"), recipe.value))
            .collect::<Vec<_>>();
        for (index, value) in plan {
            destination[index] = value;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedFrame {
    resume_pc: u32,
    side_effect_epoch: u64,
    slots: Box<[MaterializedValue]>,
}
impl MaterializedFrame {
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn side_effect_epoch(&self) -> u64 {
        self.side_effect_epoch
    }
    pub fn slots(&self) -> &[MaterializedValue] {
        &self.slots
    }
}

pub trait DeoptOwnership {
    type Error;
    fn duplicate(&mut self, source_slot: u16) -> Result<TaggedValue, Self::Error>;
    fn release(&mut self, value: TaggedValue);
}

#[derive(Debug)]
pub enum OwnedMaterializeError<E> {
    InvalidMap(DeoptValidationError),
    Ownership(E),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnedMaterializedValue {
    Scalar(MaterializedValue),
    Tagged(TaggedValue),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedMaterializedFrame {
    resume_pc: u32,
    side_effect_epoch: u64,
    slots: Box<[OwnedMaterializedValue]>,
    owned_count: usize,
}
impl OwnedMaterializedFrame {
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn side_effect_epoch(&self) -> u64 {
        self.side_effect_epoch
    }
    pub fn slots(&self) -> &[OwnedMaterializedValue] {
        &self.slots
    }
    pub const fn owned_count(&self) -> usize {
        self.owned_count
    }
}

impl DeoptMap {
    /// Executes the fallible ownership phase into private scratch storage. No
    /// caller-visible frame slot is changed until every duplication succeeds.
    pub fn materialize_owned<O: DeoptOwnership>(
        &self,
        shape: OptimizedFrameShape,
        ownership: &mut O,
    ) -> Result<OwnedMaterializedFrame, OwnedMaterializeError<O::Error>> {
        self.validate(shape)
            .map_err(OwnedMaterializeError::InvalidMap)?;
        let mut planned =
            vec![OwnedMaterializedValue::Scalar(MaterializedValue::Poison); shape.slot_count()];
        let mut owned = Vec::new();
        for recipe in &self.slots {
            let index = shape
                .index(recipe.slot)
                .expect("map validated before ownership");
            planned[index] = match recipe.value {
                MaterializedValue::TaggedSlot(source) => match ownership.duplicate(source) {
                    Ok(value) => {
                        owned.push(value);
                        OwnedMaterializedValue::Tagged(value)
                    }
                    Err(error) => {
                        for value in owned.drain(..).rev() {
                            ownership.release(value);
                        }
                        return Err(OwnedMaterializeError::Ownership(error));
                    }
                },
                scalar => OwnedMaterializedValue::Scalar(scalar),
            };
        }
        Ok(OwnedMaterializedFrame {
            resume_pc: self.resume_pc,
            side_effect_epoch: self.phase.side_effect_epoch(),
            slots: planned.into_boxed_slice(),
            owned_count: owned.len(),
        })
    }
}

impl MaterializedValue {
    pub fn is_negative_zero(self) -> bool {
        matches!(self, Self::Float64(value) if value.to_bits() == (-0.0f64).to_bits())
    }
}
