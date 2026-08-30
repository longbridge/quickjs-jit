#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug)]
pub enum NumericConstant {
    Int32(i32),
    Float64(f64),
}

impl NumericConstant {
    pub fn as_f64(self) -> Option<f64> {
        Some(match self {
            Self::Int32(value) => f64::from(value),
            Self::Float64(value) => value,
        })
    }
    pub fn is_negative_zero(self) -> bool {
        matches!(self, Self::Float64(value) if value.to_bits() == (-0.0f64).to_bits())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OptimizedInput {
    Constant(NumericConstant),
    Binary {
        op: NumericBinaryOp,
        lhs: u32,
        rhs: u32,
    },
    Return(u32),
}

impl OptimizedInput {
    pub const fn constant_i32(value: i32) -> Self {
        Self::Constant(NumericConstant::Int32(value))
    }
    pub const fn constant_f64(value: f64) -> Self {
        Self::Constant(NumericConstant::Float64(value))
    }
    pub const fn binary(op: NumericBinaryOp, lhs: u32, rhs: u32) -> Self {
        Self::Binary { op, lhs, rhs }
    }
    pub const fn ret(value: u32) -> Self {
        Self::Return(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptimizedCompileError {
    InvalidValue,
    MissingReturn,
    Unsupported,
}

#[derive(Debug)]
pub struct OptimizedFunction {
    constants: Box<[Option<NumericConstant>]>,
    return_value: u32,
    boxes_elided: u64,
    cse_eliminated: u64,
    dead_nodes_eliminated: u64,
}

impl OptimizedFunction {
    pub fn constant(&self, value: u32) -> Option<NumericConstant> {
        self.constants.get(value as usize).copied().flatten()
    }
    pub const fn return_value(&self) -> u32 {
        self.return_value
    }
    pub const fn boxes_elided(&self) -> u64 {
        self.boxes_elided
    }
    pub const fn cse_eliminated(&self) -> u64 {
        self.cse_eliminated
    }
    pub const fn dead_nodes_eliminated(&self) -> u64 {
        self.dead_nodes_eliminated
    }
}

#[derive(Debug, Default)]
pub struct OptimizedCompiler;

impl OptimizedCompiler {
    pub fn compile(
        &mut self,
        inputs: &[OptimizedInput],
    ) -> Result<OptimizedFunction, OptimizedCompileError> {
        let mut constants = Vec::with_capacity(inputs.len());
        let mut return_value = None;
        let mut boxes_elided = 0u64;
        let mut cse_eliminated = 0u64;
        let mut expressions = std::collections::BTreeMap::<(u8, u32, u32), u32>::new();
        let mut operands = Vec::<Option<(u32, u32)>>::with_capacity(inputs.len());
        for input in inputs {
            let folded = match *input {
                OptimizedInput::Constant(value) => Some(value),
                OptimizedInput::Binary { op, lhs, rhs } => {
                    let lhs_value = constants
                        .get(lhs as usize)
                        .copied()
                        .flatten()
                        .ok_or(OptimizedCompileError::InvalidValue)?;
                    let rhs_value = constants
                        .get(rhs as usize)
                        .copied()
                        .flatten()
                        .ok_or(OptimizedCompileError::InvalidValue)?;
                    boxes_elided = boxes_elided.saturating_add(1);
                    let opcode = match op {
                        NumericBinaryOp::Add => 0,
                        NumericBinaryOp::Sub => 1,
                        NumericBinaryOp::Mul => 2,
                        NumericBinaryOp::Div => 3,
                    };
                    if expressions
                        .insert((opcode, lhs, rhs), constants.len() as u32)
                        .is_some()
                    {
                        cse_eliminated = cse_eliminated.saturating_add(1);
                    }
                    Some(fold(op, lhs_value, rhs_value))
                }
                OptimizedInput::Return(value) => {
                    if constants.get(value as usize).is_none() {
                        return Err(OptimizedCompileError::InvalidValue);
                    }
                    return_value = Some(value);
                    None
                }
            };
            operands.push(match *input {
                OptimizedInput::Binary { lhs, rhs, .. } => Some((lhs, rhs)),
                _ => None,
            });
            constants.push(folded);
        }
        let return_value = return_value.ok_or(OptimizedCompileError::MissingReturn)?;
        let mut live = vec![false; inputs.len()];
        let mut work = vec![return_value];
        while let Some(value) = work.pop() {
            let Some(slot) = live.get_mut(value as usize) else {
                return Err(OptimizedCompileError::InvalidValue);
            };
            if *slot {
                continue;
            }
            *slot = true;
            if let Some((lhs, rhs)) = operands[value as usize] {
                work.extend([lhs, rhs]);
            }
        }
        let dead_nodes_eliminated = inputs
            .iter()
            .enumerate()
            .filter(|(index, input)| !live[*index] && !matches!(input, OptimizedInput::Return(_)))
            .count() as u64;
        Ok(OptimizedFunction {
            constants: constants.into_boxed_slice(),
            return_value,
            boxes_elided,
            cse_eliminated,
            dead_nodes_eliminated,
        })
    }
}

fn fold(op: NumericBinaryOp, lhs: NumericConstant, rhs: NumericConstant) -> NumericConstant {
    if let (NumericConstant::Int32(lhs), NumericConstant::Int32(rhs)) = (lhs, rhs) {
        let exact = match op {
            NumericBinaryOp::Add => lhs.checked_add(rhs),
            NumericBinaryOp::Sub => lhs.checked_sub(rhs),
            NumericBinaryOp::Mul => lhs.checked_mul(rhs),
            NumericBinaryOp::Div => None,
        };
        if let Some(value) = exact {
            return NumericConstant::Int32(value);
        }
    }
    let lhs = lhs.as_f64().expect("numeric constant");
    let rhs = rhs.as_f64().expect("numeric constant");
    NumericConstant::Float64(match op {
        NumericBinaryOp::Add => lhs + rhs,
        NumericBinaryOp::Sub => lhs - rhs,
        NumericBinaryOp::Mul => lhs * rhs,
        NumericBinaryOp::Div => lhs / rhs,
    })
}

/// Production Tier 2 compiler. Its deliberately narrow first implementation
/// reuses the audited Tier 1 machine lowering after proving a numeric/local
/// subset and attaches exact guard/deopt metadata. Unsupported semantics reject
/// the tier and leave Tier 1 installed.
pub struct Tier2Compiler {
    baseline: BaselineCompiler,
    feedback_epoch: u64,
}

impl Tier2Compiler {
    pub fn host(feedback_epoch: u64) -> Self {
        Self {
            baseline: BaselineCompiler::host(),
            feedback_epoch,
        }
    }

    pub fn plan(
        function: &VerifiedFunction,
        feedback_epoch: u64,
    ) -> Result<OptimizedArtifactMetadata, CompileFailure> {
        let ir = BaselineIr::translate(function)?;
        if ir
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| {
                !matches!(
                    instruction.op,
                    IrOp::Poll { .. }
                        | IrOp::OsrLabel { .. }
                        | IrOp::Nop
                        | IrOp::Push(_)
                        | IrOp::GetArgument(_)
                        | IrOp::GetLocal(_)
                        | IrOp::GetLocalChecked(_)
                        | IrOp::PutArgument { .. }
                        | IrOp::PutLocal { .. }
                        | IrOp::PutLocalChecked { .. }
                        | IrOp::SetLocalUninitialized(_)
                        | IrOp::Drop
                        | IrOp::Stack(_)
                        | IrOp::AddLocal(_)
                        | IrOp::Binary(_)
                        | IrOp::Unary(_)
                        | IrOp::PostUnary(_)
                        | IrOp::LocalUnary { .. }
                        | IrOp::Jump(_)
                        | IrOp::Branch { .. }
                        | IrOp::Return
                        | IrOp::ReturnUndefined
                )
            })
        {
            return Err(CompileFailure::UnsupportedOpcode);
        }
        let mut sites = Vec::new();
        let boxes_elided = ir
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| {
                matches!(
                    instruction.op,
                    IrOp::AddLocal(_)
                        | IrOp::Binary(_)
                        | IrOp::Unary(_)
                        | IrOp::PostUnary(_)
                        | IrOp::LocalUnary { .. }
                )
            })
            .count() as u64;
        for (guard, state) in ir.frame_states.iter().enumerate() {
            let mut arguments = 0u16;
            let mut locals = 0u16;
            let mut stack = 0u16;
            let mut recipes = Vec::with_capacity(state.slots.len());
            for (flat, slot) in state.slots.iter().copied().enumerate() {
                let value = MaterializedValue::TaggedSlot(
                    u16::try_from(flat).map_err(|_| CompileFailure::ResourceLimit)?,
                );
                recipes.push(match slot {
                    FrameSlot::Argument(index) => {
                        arguments = arguments.max(index.saturating_add(1));
                        Materialization::argument(index, value)
                    }
                    FrameSlot::Local(index) => {
                        locals = locals.max(index.saturating_add(1));
                        Materialization::local(index, value)
                    }
                    FrameSlot::Stack(index) => {
                        stack = stack.max(index.saturating_add(1));
                        Materialization::stack(index, value)
                    }
                });
            }
            let shape = OptimizedFrameShape::new(arguments, locals, stack);
            let map = DeoptMap::new(
                u32::try_from(guard).map_err(|_| CompileFailure::ResourceLimit)?,
                state.pc,
                DeoptPhase::BeforeEffect(0),
                recipes,
            );
            map.validate(shape)
                .map_err(|_| CompileFailure::InvalidArtifact)?;
            sites.push((shape, map));
        }
        Ok(OptimizedArtifactMetadata::new(
            feedback_epoch,
            sites,
            boxes_elided,
            0,
            0,
        ))
    }
}

pub struct TieredCompiler {
    baseline: BaselineCompiler,
    optimizing: Tier2Compiler,
}

impl TieredCompiler {
    pub fn host() -> Self {
        Self {
            baseline: BaselineCompiler::host(),
            optimizing: Tier2Compiler::host(0),
        }
    }
    pub fn target_identity(&self) -> crate::compiler::baseline::TargetIdentity {
        self.baseline.target_identity()
    }
}

impl Compiler for TieredCompiler {
    fn compile(
        &self,
        request: CompileRequest,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        match request.tier() {
            Tier::Baseline => Compiler::compile(&self.baseline, request),
            Tier::Optimizing => self.optimizing.compile(request),
        }
    }
    fn compile_controlled(
        &self,
        request: CompileRequest,
        control: &CompileControl,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        match request.tier() {
            Tier::Baseline => self.baseline.compile_controlled(request, control),
            Tier::Optimizing => self.optimizing.compile_controlled(request, control),
        }
    }
}

impl Compiler for Tier2Compiler {
    fn compile(
        &self,
        request: CompileRequest,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        if request.tier() != Tier::Optimizing {
            return Err(CompileFailure::InvalidArtifact);
        }
        let metadata = Self::plan(request.snapshot(), self.feedback_epoch)?;
        let code = self.baseline.compile_optimizing(request.snapshot(), None)?;
        let dependency = crate::code_cache::ArtifactDependency::new(request.key());
        Ok(artifact_from_relocatable(request, code)
            .with_dependencies(vec![dependency])
            .with_optimized_metadata(metadata))
    }

    fn compile_controlled(
        &self,
        request: CompileRequest,
        control: &CompileControl,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        control.check()?;
        if request.tier() != Tier::Optimizing {
            return Err(CompileFailure::InvalidArtifact);
        }
        let metadata = Self::plan(request.snapshot(), self.feedback_epoch)?;
        control.check_ir_bytes(
            metadata
                .deopt_sites()
                .len()
                .saturating_mul(core::mem::size_of::<DeoptMap>()),
        )?;
        let code = self
            .baseline
            .compile_optimizing(request.snapshot(), Some(control))?;
        control.check()?;
        let dependency = crate::code_cache::ArtifactDependency::new(request.key());
        Ok(artifact_from_relocatable(request, code)
            .with_dependencies(vec![dependency])
            .with_optimized_metadata(metadata))
    }
}
use crate::{
    bytecode::VerifiedFunction,
    code_cache::OptimizedArtifactMetadata,
    ir::{
        BaselineIr, DeoptMap, DeoptPhase, FrameSlot, IrOp, Materialization, MaterializedValue,
        OptimizedFrameShape,
    },
    runtime::{CompileRequest, Tier},
};

use super::{
    baseline::{artifact_from_relocatable, BaselineCompiler},
    CompileControl, CompileFailure, Compiler,
};
