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
    representatives: Box<[u32]>,
    operands: Box<[Option<(u32, u32)>]>,
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
    pub fn representative(&self, value: u32) -> Option<u32> {
        self.representatives.get(value as usize).copied()
    }
    pub fn operands(&self, value: u32) -> Option<(u32, u32)> {
        self.operands.get(value as usize).copied().flatten()
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
        let mut representatives = Vec::<u32>::with_capacity(inputs.len());
        for input in inputs {
            let value_id =
                u32::try_from(constants.len()).map_err(|_| OptimizedCompileError::InvalidValue)?;
            let mut representative = value_id;
            let folded = match *input {
                OptimizedInput::Constant(value) => Some(value),
                OptimizedInput::Binary { op, lhs, rhs } => {
                    let lhs = *representatives
                        .get(lhs as usize)
                        .ok_or(OptimizedCompileError::InvalidValue)?;
                    let rhs = *representatives
                        .get(rhs as usize)
                        .ok_or(OptimizedCompileError::InvalidValue)?;
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
                    if let Some(existing) = expressions.get(&(opcode, lhs, rhs)).copied() {
                        representative = existing;
                        cse_eliminated = cse_eliminated.saturating_add(1);
                    } else {
                        expressions.insert((opcode, lhs, rhs), value_id);
                    }
                    Some(fold(op, lhs_value, rhs_value))
                }
                OptimizedInput::Return(value) => {
                    return_value = Some(
                        *representatives
                            .get(value as usize)
                            .ok_or(OptimizedCompileError::InvalidValue)?,
                    );
                    None
                }
            };
            operands.push(match *input {
                OptimizedInput::Binary { lhs, rhs, .. } => Some((
                    *representatives
                        .get(lhs as usize)
                        .ok_or(OptimizedCompileError::InvalidValue)?,
                    *representatives
                        .get(rhs as usize)
                        .ok_or(OptimizedCompileError::InvalidValue)?,
                )),
                _ => None,
            });
            constants.push(folded);
            representatives.push(representative);
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
            representatives: representatives.into_boxed_slice(),
            operands: operands.into_boxed_slice(),
            return_value,
            boxes_elided,
            cse_eliminated,
            dead_nodes_eliminated,
        })
    }
}

fn fold(op: NumericBinaryOp, lhs: NumericConstant, rhs: NumericConstant) -> NumericConstant {
    if let (NumericConstant::Int32(lhs), NumericConstant::Int32(rhs)) = (lhs, rhs) {
        if matches!(op, NumericBinaryOp::Mul) && (lhs == 0 || rhs == 0) && (lhs < 0 || rhs < 0) {
            return NumericConstant::Float64(-0.0);
        }
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
    isa: cranelift_codegen::isa::OwnedTargetIsa,
    feedback_epoch: u64,
}

#[derive(Clone, Copy)]
struct OptPair {
    payload: cranelift_codegen::ir::Value,
    tag: cranelift_codegen::ir::Value,
}

#[derive(Clone, Copy)]
struct OptVars {
    payload: cranelift_frontend::Variable,
    tag: cranelift_frontend::Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OptProvenance {
    Argument(usize),
    Local(usize),
    ImmediatePrimitive,
    /// The interpreter stack slot at this index owns the value (a helper
    /// wrote it there). Exits and the call bridge leave it in place; it may
    /// be consumed by a call, returned, freed by `drop`, or moved into a
    /// local, but never copied.
    OwnedSlot,
    Unknown,
}

#[derive(Clone, Copy)]
struct GuardedElementSource {
    provenance: OptProvenance,
    block_pc: u32,
    data: cranelift_codegen::ir::Value,
    count: cranelift_codegen::ir::Value,
    kind: cranelift_codegen::ir::Value,
}

#[derive(Clone, Copy, Default)]
enum EntryRepresentation {
    #[default]
    Any,
    Numeric,
    Int32,
    Float64,
    HeapRef,
}

#[derive(Default)]
struct NumericSpecialization {
    entry: EntryRepresentation,
    arguments: Box<[EntryRepresentation]>,
    int_pcs: std::collections::BTreeSet<u32>,
    float_pcs: std::collections::BTreeSet<u32>,
    calls: std::collections::BTreeMap<u32, crate::runtime::CallSpecializationKey>,
    properties: std::collections::BTreeMap<u32, Box<[crate::runtime::ShapeObservation]>>,
    direct_calls: std::collections::BTreeMap<u32, DirectCallSite>,
    numeric_constants: std::collections::BTreeMap<u32, crate::ir::TaggedValue>,
}

#[derive(Clone)]
struct DirectCallSite {
    call: crate::runtime::CallSpecializationKey,
    entry: usize,
}

impl NumericSpecialization {
    fn from_feedback(
        function: &VerifiedFunction,
        key: crate::runtime::FunctionKey,
        feedback: &crate::runtime::FeedbackSnapshot,
    ) -> Self {
        use crate::runtime::{FeedbackRepresentation, FeedbackState, ObservedType};
        let calls = function
            .instructions()
            .iter()
            .filter(|instruction| is_call_site(instruction.opcode().name()))
            .filter_map(|instruction| {
                feedback
                    .call_specialization_at(key, instruction.pc())
                    .map(|call| (instruction.pc(), call))
            })
            .collect();
        let properties = function
            .instructions()
            .iter()
            .filter_map(|instruction| {
                let name = instruction.opcode().name();
                if !matches!(name, "get_field" | "put_field") {
                    return None;
                }
                let site = feedback.property_at(instruction.pc())?;
                let observations = site.observations();
                let safe = site.state() != crate::runtime::ShapeFeedbackState::Megamorphic
                    && !observations.is_empty()
                    && observations.len() <= 3
                    && observations.iter().all(|observation| {
                        let primitive = matches!(
                            observation.value(),
                            ObservedType::Int32
                                | ObservedType::Float64
                                | ObservedType::Bool
                                | ObservedType::Null
                                | ObservedType::Undefined
                        );
                        observation.prototype().identity() == 0
                            && observation.prototype().generation() == 0
                            && !observation
                                .attributes()
                                .contains(crate::runtime::PropertyAttributes::ACCESSOR)
                            && (name != "put_field"
                                || observation
                                    .attributes()
                                    .contains(crate::runtime::PropertyAttributes::WRITABLE))
                            && primitive
                    });
                safe.then_some((instruction.pc(), observations.to_vec().into_boxed_slice()))
            })
            .collect();
        let numeric_constants = function
            .snapshot()
            .constants()
            .iter()
            .filter_map(|constant| {
                matches!(
                    constant.tag(),
                    rquickjs_core::qjs::JS_TAG_INT | rquickjs_core::qjs::JS_TAG_FLOAT64
                )
                .then_some((
                    constant.index(),
                    crate::ir::TaggedValue::new(constant.payload(), i64::from(constant.tag())),
                ))
            })
            .collect();
        let argument_count = usize::from(function.snapshot().arg_count());
        let entry_arguments = feedback
            .call_argument_types(key)
            .filter(|arguments| arguments.len() == argument_count)
            .map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| match argument {
                        ObservedType::Int32 => EntryRepresentation::Int32,
                        ObservedType::Float64 => EntryRepresentation::Float64,
                        ObservedType::Object => EntryRepresentation::HeapRef,
                        _ => EntryRepresentation::Any,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .unwrap_or_default();
        // The checked Int32 arithmetic path guards both operand tags at
        // run time, so stable per-site Int32 feedback selects it whatever the
        // function's own signature is (a kernel may well return a string).
        let int_pcs = function
            .instructions()
            .iter()
            .filter(|instruction| {
                matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
            })
            .filter_map(|instruction| {
                let site = feedback.binary_at(key, instruction.pc())?;
                (site.state() == FeedbackState::Monomorphic
                    && site.lhs() == [ObservedType::Int32]
                    && site.rhs() == [ObservedType::Int32]
                    && site.result() == [ObservedType::Int32])
                .then_some(instruction.pc())
            })
            .collect();
        let Some(signature) = feedback
            .bounded_specialization(key)
            .filter(|signature| signature.arity() == argument_count)
        else {
            return Self {
                arguments: entry_arguments,
                int_pcs,
                calls,
                properties,
                numeric_constants,
                ..Self::default()
            };
        };
        let representation = signature.result();
        let observed = match representation {
            FeedbackRepresentation::Int32 => ObservedType::Int32,
            FeedbackRepresentation::Float64 => ObservedType::Float64,
            FeedbackRepresentation::HeapRef => {
                return Self {
                    int_pcs,
                    calls,
                    properties,
                    numeric_constants,
                    ..Self::default()
                }
            }
        };
        let float_pcs = function
            .instructions()
            .iter()
            .filter(|instruction| {
                matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
            })
            .filter_map(|instruction| {
                let site = feedback.binary_at(key, instruction.pc())?;
                (representation == FeedbackRepresentation::Float64
                    && site.state() == FeedbackState::Monomorphic
                    && site.lhs() == [observed]
                    && site.rhs() == [observed]
                    && site.result() == [observed])
                .then_some(instruction.pc())
            })
            .collect();
        Self {
            entry: if signature
                .arguments()
                .iter()
                .all(|argument| *argument == representation)
            {
                match representation {
                    FeedbackRepresentation::Int32 => EntryRepresentation::Int32,
                    FeedbackRepresentation::Float64 => EntryRepresentation::Float64,
                    FeedbackRepresentation::HeapRef => EntryRepresentation::Any,
                }
            } else {
                EntryRepresentation::Any
            },
            arguments: signature
                .arguments()
                .iter()
                .map(|argument| match argument {
                    FeedbackRepresentation::Int32 => EntryRepresentation::Int32,
                    FeedbackRepresentation::Float64 => EntryRepresentation::Float64,
                    FeedbackRepresentation::HeapRef => EntryRepresentation::HeapRef,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            int_pcs,
            float_pcs,
            calls,
            properties,
            direct_calls: Default::default(),
            numeric_constants,
        }
    }
}

fn lower_optimized_machine(
    isa: &cranelift_codegen::isa::OwnedTargetIsa,
    ir: &OptimizedIr,
    control: Option<&CompileControl>,
    side_path: Option<crate::runtime::SidePathProfile>,
    specialization: &NumericSpecialization,
) -> Result<super::baseline::RelocatableCode, CompileFailure> {
    use cranelift_codegen::ir::{
        types, AbiParam, ArgumentPurpose, Function, InstBuilder, MemFlags, Signature,
    };
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
    use rquickjs_core::qjs;
    let pointer_type = isa.pointer_type();
    if pointer_type.bytes() != 8 {
        return Err(CompileFailure::InvalidArtifact);
    }
    let layout = super::helpers::FrameLayout::validated(8)?;
    let element_layout = crate::abi::AbiInfo::linked()
        .map_err(|_| CompileFailure::InvalidArtifact)?
        .element_layout();
    let Some(entry_site) = ir.guard_maps().first() else {
        return Err(CompileFailure::InvalidArtifact);
    };
    let shape = entry_site.shape();
    let int32_loop = matches!(specialization.entry, EntryRepresentation::Int32)
        && specialization.calls.is_empty()
        && ir.blocks().iter().any(|block| block.is_loop_header());
    let mut signature = Signature::new(isa.default_call_conv());
    signature.params.push(AbiParam::special(
        pointer_type,
        ArgumentPurpose::StructReturn,
    ));
    signature.params.push(AbiParam::new(pointer_type));
    let mut clif = Function::with_name_signature(Default::default(), signature);
    let mut context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut clif, &mut context);
        let generated_signatures = super::helpers::generated_signatures(&**isa)?;
        let poll_signature = generated_signatures
            .get(rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_POLL as usize)
            .cloned()
            .ok_or(CompileFailure::InvalidArtifact)?;
        let poll_signature = builder.import_signature(poll_signature);
        let shape_guard_signature = generated_signatures
            .get(qjs::JSJitHelperId_JS_JIT_HELPER_SHAPE_GUARD as usize)
            .cloned()
            .ok_or(CompileFailure::InvalidArtifact)?;
        let shape_guard_signature = builder.import_signature(shape_guard_signature);
        let helper_signatures = generated_signatures
            .into_iter()
            .map(|signature| builder.import_signature(signature))
            .collect::<Vec<_>>();
        let prologue = builder.create_block();
        builder.append_block_params_for_function_params(prologue);
        builder.switch_to_block(prologue);
        let params = builder.block_params(prologue);
        let sret = params[0];
        let frame = params[1];
        let flags = MemFlags::new();
        let arg_buf = builder
            .ins()
            .load(pointer_type, flags, frame, layout.arg_buf);
        let var_buf = builder
            .ins()
            .load(pointer_type, flags, frame, layout.var_buf);
        let stack_base = builder
            .ins()
            .load(pointer_type, flags, frame, layout.stack_base);
        let mut next_var = 0u32;
        let mut alloc = || {
            let pair = OptVars {
                payload: Variable::from_u32(next_var),
                tag: Variable::from_u32(next_var + 1),
            };
            next_var += 2;
            pair
        };
        let arguments = (0..shape.arguments()).map(|_| alloc()).collect::<Vec<_>>();
        let locals = (0..shape.locals()).map(|_| alloc()).collect::<Vec<_>>();
        let stack_slots = usize::from(ir.max_stack())
            .checked_add(crate::ir::MAX_HELPER_SCRATCH_SLOTS)
            .ok_or(CompileFailure::ResourceLimit)?;
        let stack = (0..stack_slots).map(|_| alloc()).collect::<Vec<_>>();
        let mut stack_provenance = vec![OptProvenance::Unknown; stack_slots];
        let mut guarded_element_source: Option<GuardedElementSource> = None;
        let owned_locals = owned_local_targets(ir, specialization)?;
        let bounded_increments = provably_bounded_increments(ir);
        let payload_type = if int32_loop { types::I32 } else { types::I64 };
        let env = OptEnv {
            frame,
            sret,
            arg_buf,
            var_buf,
            stack_base,
            pointer_type,
            payload_type,
            layout,
            int32_loop,
            arguments: &arguments,
            locals: &locals,
            stack: &stack,
            helper_signatures: &helper_signatures,
        };
        for vars in arguments.iter().chain(&locals).chain(&stack) {
            builder.declare_var(vars.payload, payload_type);
            builder.declare_var(vars.tag, types::I64);
        }
        let poll_budget = Variable::from_u32(next_var);
        builder.declare_var(poll_budget, types::I64);
        let initial_poll_budget = builder.ins().iconst(types::I64, 64);
        builder.def_var(poll_budget, initial_poll_budget);
        for (index, vars) in arguments.iter().enumerate() {
            let mut pair = opt_load(&mut builder, arg_buf, index);
            if int32_loop {
                pair.payload = builder.ins().ireduce(types::I32, pair.payload);
            }
            opt_define(&mut builder, *vars, pair);
        }
        for (index, vars) in locals.iter().enumerate() {
            let pair = if int32_loop {
                OptPair {
                    payload: builder.ins().iconst(types::I32, 0),
                    tag: builder
                        .ins()
                        .iconst(types::I64, i64::from(qjs::JS_TAG_UNDEFINED)),
                }
            } else {
                opt_load(&mut builder, var_buf, index)
            };
            opt_define(&mut builder, *vars, pair);
        }
        let undefined = OptPair {
            payload: builder.ins().iconst(payload_type, 0),
            tag: builder
                .ins()
                .iconst(types::I64, i64::from(qjs::JS_TAG_UNDEFINED)),
        };
        for vars in &stack {
            opt_define(&mut builder, *vars, undefined);
        }
        let blocks = ir
            .blocks()
            .iter()
            .map(|block| (block.start_pc(), builder.create_block()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut predecessors = std::collections::BTreeMap::<u32, Vec<u32>>::new();
        for block in ir.blocks() {
            for successor in block.successors() {
                predecessors
                    .entry(*successor)
                    .or_default()
                    .push(block.start_pc());
            }
        }
        let entry = *blocks.get(&0).ok_or(CompileFailure::InvalidArtifact)?;
        emit_opt_numeric_guard(
            &mut builder,
            frame,
            sret,
            &arguments,
            &[],
            pointer_type,
            layout,
            entry_site.guard(),
            0,
            entry,
            side_path.filter(|profile| profile.guard().get() == entry_site.guard()),
            specialization.entry,
            &specialization.arguments,
        );
        for block in ir.blocks() {
            guarded_element_source = guarded_element_source.filter(|source| {
                source.block_pc == block.start_pc()
                    || (matches!(source.provenance, OptProvenance::Argument(_))
                        && predecessors
                            .get(&block.start_pc())
                            .is_some_and(|incoming| incoming.as_slice() == [source.block_pc]))
            });
            let clif_block = blocks[&block.start_pc()];
            builder.switch_to_block(clif_block);
            let mut depth = usize::from(block.stack_depth());
            let mut terminated = false;
            let mut reusable_values = std::collections::BTreeMap::<u32, OptPair>::new();
            for node_id in block.nodes() {
                let node = ir
                    .nodes()
                    .get(*node_id as usize)
                    .ok_or(CompileFailure::InvalidArtifact)?;
                if node.eliminated() {
                    if node.pops() == 1
                        && node.pushes() == 0
                        && depth
                            .checked_sub(1)
                            .is_some_and(|top| stack_provenance[top] == OptProvenance::OwnedSlot)
                    {
                        emit_opt_free_stack_slot(&mut builder, &env, depth - 1)?;
                    }
                    depth = depth
                        .checked_sub(usize::from(node.pops()))
                        .and_then(|value| value.checked_add(usize::from(node.pushes())))
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    continue;
                }
                match node.kind() {
                    crate::ir::OptimizedNodeKind::GuardNumeric { guard, mid_loop } => {
                        if *mid_loop && int32_loop {
                            emit_opt_amortized_poll(
                                &mut builder,
                                frame,
                                sret,
                                poll_signature,
                                pointer_type,
                                layout,
                                node.pc(),
                                poll_budget,
                            );
                        } else if *mid_loop {
                            emit_opt_poll(
                                &mut builder,
                                frame,
                                sret,
                                poll_signature,
                                pointer_type,
                                layout,
                                node.pc(),
                            );
                            let pass = builder.create_block();
                            emit_opt_numeric_guard(
                                &mut builder,
                                frame,
                                sret,
                                &arguments,
                                &locals,
                                pointer_type,
                                layout,
                                *guard,
                                node.pc(),
                                pass,
                                side_path.filter(|profile| profile.guard().get() == *guard),
                                EntryRepresentation::Numeric,
                                &specialization.arguments,
                            );
                            builder.switch_to_block(pass);
                        }
                    }
                    crate::ir::OptimizedNodeKind::Reuse { source } => {
                        let pair = *reusable_values
                            .get(source)
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        opt_define(&mut builder, stack[depth], pair);
                        stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                        reusable_values.insert(*node_id, pair);
                        depth += 1;
                    }
                    crate::ir::OptimizedNodeKind::Bytecode { opcode } => {
                        let name = opcode.as_ref();
                        if node.effect() == crate::ir::OptimizedEffect::Reentrant {
                            guarded_element_source = None;
                        }
                        match name {
                            "set_loc_uninitialized" => {
                                guarded_element_source = None;
                                let index = opt_u16(node.bytes())?;
                                if owned_locals[index] {
                                    emit_opt_free_local_slot(&mut builder, &env, index)?;
                                }
                                opt_define(&mut builder, locals[index], undefined);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, undefined);
                                }
                            }
                            "push_i8" => {
                                let value = i64::from(
                                    node.bytes()
                                        .get(1)
                                        .copied()
                                        .ok_or(CompileFailure::InvalidArtifact)?
                                        as i8,
                                );
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                reusable_values.insert(*node_id, pair);
                                depth += 1;
                            }
                            "undefined" => {
                                opt_define(&mut builder, stack[depth], undefined);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                depth += 1;
                            }
                            "push_i16" => {
                                let value = i64::from(i16::from_le_bytes([
                                    node.bytes()[1],
                                    node.bytes()[2],
                                ]));
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                reusable_values.insert(*node_id, pair);
                                depth += 1;
                            }
                            "push_i32" => {
                                let value = i64::from(i32::from_le_bytes(
                                    node.bytes()[1..5]
                                        .try_into()
                                        .map_err(|_| CompileFailure::InvalidArtifact)?,
                                ));
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                depth += 1;
                            }
                            "push_const" | "push_const8" => {
                                let constant = if name == "push_const8" {
                                    u32::from(
                                        *node
                                            .bytes()
                                            .get(1)
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                    )
                                } else {
                                    u32::from_le_bytes(
                                        node.bytes()
                                            .get(1..5)
                                            .ok_or(CompileFailure::InvalidArtifact)?
                                            .try_into()
                                            .map_err(|_| CompileFailure::InvalidArtifact)?,
                                    )
                                };
                                let constant = specialization
                                    .numeric_constants
                                    .get(&constant)
                                    .copied()
                                    .ok_or(CompileFailure::UnsupportedOpcode)?;
                                let pair = OptPair {
                                    payload: builder
                                        .ins()
                                        .iconst(types::I64, constant.payload as i64),
                                    tag: builder.ins().iconst(types::I64, constant.tag),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                depth += 1;
                            }
                            "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5"
                            | "push_6" | "push_7" => {
                                let value = i64::from(name.as_bytes()[5] - b'0');
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                depth += 1;
                            }
                            n if opt_index(n, node.bytes(), "get_arg")?.is_some() => {
                                let index = opt_index(n, node.bytes(), "get_arg")?.unwrap();
                                let pair = opt_use(&mut builder, arguments[index]);
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::Argument(index);
                                depth += 1;
                            }
                            n if n != "get_loc0_loc1"
                                && (opt_index(n, node.bytes(), "get_loc")?.is_some()
                                    || n == "get_loc_check") =>
                            {
                                let index = opt_index(n, node.bytes(), "get_loc")?
                                    .map_or_else(|| opt_u16(node.bytes()), Ok)?;
                                let pair = opt_use(&mut builder, locals[index]);
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::Local(index);
                                depth += 1;
                            }
                            n if opt_index(n, node.bytes(), "put_loc")?.is_some()
                                || matches!(n, "put_loc_check" | "put_loc_check_init") =>
                            {
                                guarded_element_source = None;
                                let index = opt_index(n, node.bytes(), "put_loc")?
                                    .map_or_else(|| opt_u16(node.bytes()), Ok)?;
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                emit_opt_alias_store_guard(
                                    &mut builder,
                                    &env,
                                    specialization,
                                    &stack_provenance,
                                    depth + 1,
                                    depth,
                                    OptProvenance::Local(index),
                                    node.pc(),
                                    node.deopt_guard(),
                                )?;
                                let pair = opt_use(&mut builder, stack[depth]);
                                // A value owned by its stack slot moves into
                                // the local exactly like the interpreter's
                                // set_value: release what the local owned,
                                // then let var_buf own the new value.
                                if owned_locals[index] {
                                    emit_opt_free_local_slot(&mut builder, &env, index)?;
                                }
                                opt_define(&mut builder, locals[index], pair);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, pair);
                                }
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    depth,
                                    OptProvenance::Local(index),
                                );
                            }
                            "get_var" => {
                                let atom = opt_u32(node.bytes())?;
                                depth = emit_opt_owned_helper_push(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    qjs::JSJitHelperId_JS_JIT_HELPER_GET_GLOBAL as usize,
                                    &[atom],
                                )?;
                            }
                            "get_field2" => {
                                let atom = opt_u32(node.bytes())?;
                                let receiver = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let receiver_slot = opt_flat_stack_slot(&env, receiver)?;
                                depth = emit_opt_owned_helper_push(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    qjs::JSJitHelperId_JS_JIT_HELPER_GET_PROPERTY as usize,
                                    &[receiver_slot, atom],
                                )?;
                            }
                            "is_undefined_or_null" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let index = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let value = opt_use(&mut builder, stack[index]);
                                let undefined = builder.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                                    value.tag,
                                    i64::from(qjs::JS_TAG_UNDEFINED),
                                );
                                let null = builder.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                                    value.tag,
                                    i64::from(qjs::JS_TAG_NULL),
                                );
                                let truth = builder.ins().bor(undefined, null);
                                let result = opt_bool_pair(&mut builder, &env, truth);
                                opt_define(&mut builder, stack[index], result);
                                stack_provenance[index] = OptProvenance::ImmediatePrimitive;
                            }
                            "to_propkey" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                depth = emit_opt_guarded_propkey(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                )?;
                            }
                            "drop" => {
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                            }
                            "get_field" | "put_field" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(2)..depth,
                                )?;
                                let property = specialization
                                    .properties
                                    .get(&node.pc())
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                depth = emit_opt_guarded_property(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    name == "put_field",
                                    property,
                                    node.pc(),
                                    node.deopt_guard().unwrap_or(entry_site.guard()),
                                    shape_guard_signature,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                )?;
                            }
                            "get_array_el" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(2)..depth,
                                )?;
                                depth = emit_opt_element_get(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                    element_layout,
                                    guarded_element_source,
                                )?;
                            }
                            "get_length" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let source_provenance = stack_provenance[depth - 1];
                                depth = emit_opt_array_length(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                    element_layout,
                                    block.start_pc(),
                                    source_provenance,
                                    &mut guarded_element_source,
                                )?;
                            }
                            "put_array_el" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(3)..depth,
                                )?;
                                let source_provenance = stack_provenance[depth
                                    .checked_sub(3)
                                    .ok_or(CompileFailure::InvalidArtifact)?];
                                depth = emit_opt_element_put(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                    element_layout,
                                    block.start_pc(),
                                    source_provenance,
                                    &mut guarded_element_source,
                                )?;
                            }
                            "call" | "call0" | "call1" | "call2" | "call3" | "call_method" => {
                                // Native callees (Math.max, String, ...) never
                                // record call-site feedback; they take the
                                // generic CALL bridge with an owned result.
                                let call = specialization.calls.get(&node.pc());
                                if call.is_none() && int32_loop {
                                    return Err(CompileFailure::InvalidArtifact);
                                }
                                let argc = if name == "call" || name == "call_method" {
                                    opt_u16(node.bytes())?
                                } else {
                                    usize::from(
                                        *name
                                            .as_bytes()
                                            .last()
                                            .ok_or(CompileFailure::InvalidArtifact)?
                                            - b'0',
                                    )
                                };
                                if call.is_some_and(|call| argc != call.arguments().len()) {
                                    return Err(CompileFailure::InvalidArtifact);
                                }
                                let has_this = name == "call_method";
                                depth = emit_opt_specialized_call(
                                    &mut builder,
                                    frame,
                                    sret,
                                    arg_buf,
                                    var_buf,
                                    stack_base,
                                    &arguments,
                                    &locals,
                                    &stack,
                                    &mut stack_provenance,
                                    depth,
                                    argc,
                                    has_this,
                                    node.pc(),
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                    specialization.direct_calls.get(&node.pc()),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    call.is_some(),
                                )?;
                            }
                            "add" | "sub" | "mul" | "div" => {
                                depth = depth
                                    .checked_sub(2)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs = opt_use(&mut builder, stack[depth]);
                                let rhs = opt_use(&mut builder, stack[depth + 1]);
                                if specialization.float_pcs.contains(&node.pc()) {
                                    let lhs = builder.ins().bitcast(
                                        types::F64,
                                        MemFlags::new(),
                                        lhs.payload,
                                    );
                                    let rhs = builder.ins().bitcast(
                                        types::F64,
                                        MemFlags::new(),
                                        rhs.payload,
                                    );
                                    let result = match name {
                                        "add" => builder.ins().fadd(lhs, rhs),
                                        "sub" => builder.ins().fsub(lhs, rhs),
                                        "mul" => builder.ins().fmul(lhs, rhs),
                                        _ => builder.ins().fdiv(lhs, rhs),
                                    };
                                    let pair = OptPair {
                                        payload: builder.ins().bitcast(
                                            types::I64,
                                            MemFlags::new(),
                                            result,
                                        ),
                                        tag: builder
                                            .ins()
                                            .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64)),
                                    };
                                    opt_define(&mut builder, stack[depth], pair);
                                    stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                    reusable_values.insert(*node_id, pair);
                                    depth += 1;
                                    continue;
                                }
                                if specialization.int_pcs.contains(&node.pc()) {
                                    let deopt = builder.create_block();
                                    if !int32_loop {
                                        use cranelift_codegen::ir::condcodes::IntCC;
                                        let lhs_int = builder.ins().icmp_imm(
                                            IntCC::Equal,
                                            lhs.tag,
                                            i64::from(qjs::JS_TAG_INT),
                                        );
                                        let rhs_int = builder.ins().icmp_imm(
                                            IntCC::Equal,
                                            rhs.tag,
                                            i64::from(qjs::JS_TAG_INT),
                                        );
                                        let both_int = builder.ins().band(lhs_int, rhs_int);
                                        let arithmetic = builder.create_block();
                                        builder.ins().brif(both_int, arithmetic, &[], deopt, &[]);
                                        builder.switch_to_block(arithmetic);
                                    }
                                    let li = if int32_loop {
                                        lhs.payload
                                    } else {
                                        builder.ins().ireduce(types::I32, lhs.payload)
                                    };
                                    let ri = if int32_loop {
                                        rhs.payload
                                    } else {
                                        builder.ins().ireduce(types::I32, rhs.payload)
                                    };
                                    let pass = builder.create_block();
                                    builder.append_block_param(pass, types::I32);
                                    if name == "div" {
                                        use cranelift_codegen::ir::condcodes::IntCC;
                                        let zero = builder.ins().icmp_imm(IntCC::Equal, ri, 0);
                                        let min = builder.ins().icmp_imm(
                                            IntCC::Equal,
                                            li,
                                            i64::from(i32::MIN),
                                        );
                                        let negative_one =
                                            builder.ins().icmp_imm(IntCC::Equal, ri, -1);
                                        let min_overflow = builder.ins().band(min, negative_one);
                                        let zero_lhs = builder.ins().icmp_imm(IntCC::Equal, li, 0);
                                        let negative_rhs =
                                            builder.ins().icmp_imm(IntCC::SignedLessThan, ri, 0);
                                        let negative_zero =
                                            builder.ins().band(zero_lhs, negative_rhs);
                                        let exceptional = builder.ins().bor(zero, min_overflow);
                                        let exceptional =
                                            builder.ins().bor(exceptional, negative_zero);
                                        let safe = builder.create_block();
                                        builder.ins().brif(exceptional, deopt, &[], safe, &[]);
                                        builder.switch_to_block(safe);
                                        let remainder = builder.ins().srem(li, ri);
                                        let exact =
                                            builder.ins().icmp_imm(IntCC::Equal, remainder, 0);
                                        let quotient = builder.create_block();
                                        builder.ins().brif(exact, quotient, &[], deopt, &[]);
                                        builder.switch_to_block(quotient);
                                        let result = builder.ins().sdiv(li, ri);
                                        builder.ins().jump(pass, &[result]);
                                    } else {
                                        let (result, mut failure) = match name {
                                            "add" => builder.ins().sadd_overflow(li, ri),
                                            "sub" => builder.ins().ssub_overflow(li, ri),
                                            _ => builder.ins().smul_overflow(li, ri),
                                        };
                                        if name == "mul" {
                                            use cranelift_codegen::ir::condcodes::IntCC;
                                            let lhs_zero =
                                                builder.ins().icmp_imm(IntCC::Equal, li, 0);
                                            let rhs_zero =
                                                builder.ins().icmp_imm(IntCC::Equal, ri, 0);
                                            let lhs_negative = builder.ins().icmp_imm(
                                                IntCC::SignedLessThan,
                                                li,
                                                0,
                                            );
                                            let rhs_negative = builder.ins().icmp_imm(
                                                IntCC::SignedLessThan,
                                                ri,
                                                0,
                                            );
                                            let lhs_zero_negative_rhs =
                                                builder.ins().band(lhs_zero, rhs_negative);
                                            let rhs_zero_negative_lhs =
                                                builder.ins().band(rhs_zero, lhs_negative);
                                            let negative_zero = builder
                                                .ins()
                                                .bor(lhs_zero_negative_rhs, rhs_zero_negative_lhs);
                                            failure = builder.ins().bor(failure, negative_zero);
                                        }
                                        builder.ins().brif(failure, deopt, &[], pass, &[result]);
                                    }
                                    builder.switch_to_block(deopt);
                                    for (index, vars) in locals.iter().enumerate() {
                                        let local = opt_use(&mut builder, *vars);
                                        opt_store(&mut builder, var_buf, index, local);
                                    }
                                    opt_store(&mut builder, stack_base, depth, lhs);
                                    opt_store(&mut builder, stack_base, depth + 1, rhs);
                                    opt_set_stack_top(
                                        &mut builder,
                                        frame,
                                        stack_base,
                                        depth + 2,
                                        pointer_type,
                                        layout,
                                    );
                                    let start = builder.ins().load(
                                        pointer_type,
                                        MemFlags::new(),
                                        frame,
                                        layout.bytecode_start,
                                    );
                                    let resume =
                                        builder.ins().iadd_imm(start, i64::from(node.pc()));
                                    builder
                                        .ins()
                                        .store(MemFlags::new(), resume, frame, layout.pc);
                                    emit_opt_exit(
                                        &mut builder,
                                        sret,
                                        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
                                        Some(resume),
                                        pointer_type,
                                        node.deopt_guard()
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                    );
                                    builder.switch_to_block(pass);
                                    let result = builder.block_params(pass)[0];
                                    let pair = OptPair {
                                        payload: if int32_loop {
                                            result
                                        } else {
                                            builder.ins().sextend(types::I64, result)
                                        },
                                        tag: builder
                                            .ins()
                                            .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                    };
                                    opt_define(&mut builder, stack[depth], pair);
                                    stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                    reusable_values.insert(*node_id, pair);
                                    depth += 1;
                                    continue;
                                }
                                // Without stable feedback the Float64 path
                                // still requires two numbers; string
                                // concatenation and object coercion deopt.
                                let lhs_int = opt_tag_is(&mut builder, lhs.tag, qjs::JS_TAG_INT);
                                let rhs_int = opt_tag_is(&mut builder, rhs.tag, qjs::JS_TAG_INT);
                                let lhs_float =
                                    opt_tag_is(&mut builder, lhs.tag, qjs::JS_TAG_FLOAT64);
                                let rhs_float =
                                    opt_tag_is(&mut builder, rhs.tag, qjs::JS_TAG_FLOAT64);
                                let lhs_numeric = builder.ins().bor(lhs_int, lhs_float);
                                let rhs_numeric = builder.ins().bor(rhs_int, rhs_float);
                                let numeric = builder.ins().band(lhs_numeric, rhs_numeric);
                                emit_opt_guard_branch(
                                    &mut builder,
                                    &env,
                                    &stack_provenance,
                                    depth + 2,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                    numeric,
                                )?;
                                let lf = opt_f64(&mut builder, lhs);
                                let rf = opt_f64(&mut builder, rhs);
                                let result = match name {
                                    "add" => builder.ins().fadd(lf, rf),
                                    "sub" => builder.ins().fsub(lf, rf),
                                    "mul" => builder.ins().fmul(lf, rf),
                                    _ => builder.ins().fdiv(lf, rf),
                                };
                                let float_payload =
                                    builder.ins().bitcast(types::I64, MemFlags::new(), result);
                                let float_tag = builder
                                    .ins()
                                    .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
                                let pair = if name == "add" {
                                    use cranelift_codegen::ir::condcodes::IntCC;
                                    let lhs_int = builder.ins().icmp_imm(
                                        IntCC::Equal,
                                        lhs.tag,
                                        i64::from(qjs::JS_TAG_INT),
                                    );
                                    let rhs_int = builder.ins().icmp_imm(
                                        IntCC::Equal,
                                        rhs.tag,
                                        i64::from(qjs::JS_TAG_INT),
                                    );
                                    let both_int = builder.ins().band(lhs_int, rhs_int);
                                    let li = builder.ins().ireduce(types::I32, lhs.payload);
                                    let ri = builder.ins().ireduce(types::I32, rhs.payload);
                                    let (sum, overflow) = builder.ins().sadd_overflow(li, ri);
                                    let no_overflow = builder.ins().bnot(overflow);
                                    let keep_int = builder.ins().band(both_int, no_overflow);
                                    let int_payload = builder.ins().sextend(types::I64, sum);
                                    let int_tag = builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT));
                                    OptPair {
                                        payload: builder.ins().select(
                                            keep_int,
                                            int_payload,
                                            float_payload,
                                        ),
                                        tag: builder.ins().select(keep_int, int_tag, float_tag),
                                    }
                                } else {
                                    OptPair {
                                        payload: float_payload,
                                        tag: float_tag,
                                    }
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                reusable_values.insert(*node_id, pair);
                                depth += 1;
                            }
                            "or" | "and" | "xor" | "shl" | "sar" | "shr" => {
                                depth = emit_opt_guarded_int_binary(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    name,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                            }
                            "mod" => {
                                depth = emit_opt_mod(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                            }
                            "eq" | "neq" | "strict_eq" | "strict_neq" => {
                                depth = emit_opt_equality(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    name,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                            }
                            "neg" | "plus" | "not" | "lnot" => {
                                emit_opt_unary(
                                    &mut builder,
                                    &env,
                                    &mut stack_provenance,
                                    depth,
                                    name,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                            }
                            "is_undefined" | "is_null" => {
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let index = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let value = opt_use(&mut builder, stack[index]);
                                let expected = if name == "is_undefined" {
                                    qjs::JS_TAG_UNDEFINED
                                } else {
                                    qjs::JS_TAG_NULL
                                };
                                let truth = opt_tag_is(&mut builder, value.tag, expected);
                                let pair = opt_bool_pair(&mut builder, &env, truth);
                                opt_define(&mut builder, stack[index], pair);
                                stack_provenance[index] = OptProvenance::ImmediatePrimitive;
                            }
                            "inc_loc" | "dec_loc" => {
                                guarded_element_source = None;
                                let index = opt_u8(node.bytes())?;
                                let old = opt_use(&mut builder, locals[index]);
                                let delta = OptPair {
                                    payload: builder.ins().iconst(
                                        payload_type,
                                        if name == "inc_loc" { 1 } else { -1 },
                                    ),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                let pair = emit_opt_checked_add(
                                    &mut builder,
                                    &env,
                                    &stack_provenance,
                                    depth,
                                    old,
                                    delta,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                                opt_define(&mut builder, locals[index], pair);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, pair);
                                }
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    depth,
                                    OptProvenance::Local(index),
                                );
                            }
                            "add_loc" => {
                                guarded_element_source = None;
                                let index = opt_u8(node.bytes())?;
                                let rhs_index = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs = opt_use(&mut builder, locals[index]);
                                let rhs = opt_use(&mut builder, stack[rhs_index]);
                                let pair = emit_opt_checked_add(
                                    &mut builder,
                                    &env,
                                    &stack_provenance,
                                    depth,
                                    lhs,
                                    rhs,
                                    node.pc(),
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                )?;
                                opt_define(&mut builder, locals[index], pair);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, pair);
                                }
                                depth = rhs_index;
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    depth,
                                    OptProvenance::Local(index),
                                );
                            }
                            "push_minus1" => {
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, -1),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                reusable_values.insert(*node_id, pair);
                                depth += 1;
                            }
                            "null" | "push_true" | "push_false" => {
                                let (payload, tag) = match name {
                                    "null" => (0, qjs::JS_TAG_NULL),
                                    "push_true" => (1, qjs::JS_TAG_BOOL),
                                    _ => (0, qjs::JS_TAG_BOOL),
                                };
                                let pair = OptPair {
                                    payload: builder.ins().iconst(payload_type, payload),
                                    tag: builder.ins().iconst(types::I64, i64::from(tag)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                depth += 1;
                            }
                            "get_loc0_loc1" => {
                                if locals.len() < 2 {
                                    return Err(CompileFailure::InvalidArtifact);
                                }
                                for (index, vars) in locals.iter().take(2).enumerate() {
                                    let pair = opt_use(&mut builder, *vars);
                                    opt_define(&mut builder, stack[depth], pair);
                                    stack_provenance[depth] = OptProvenance::Local(index);
                                    depth += 1;
                                }
                            }
                            n if opt_index(n, node.bytes(), "put_arg")?.is_some() => {
                                guarded_element_source = None;
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let index = opt_index(n, node.bytes(), "put_arg")?.unwrap();
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                emit_opt_alias_store_guard(
                                    &mut builder,
                                    &env,
                                    specialization,
                                    &stack_provenance,
                                    depth + 1,
                                    depth,
                                    OptProvenance::Argument(index),
                                    node.pc(),
                                    node.deopt_guard(),
                                )?;
                                let pair = opt_use(&mut builder, stack[depth]);
                                opt_define(&mut builder, arguments[index], pair);
                                opt_store(&mut builder, arg_buf, index, pair);
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    depth,
                                    OptProvenance::Argument(index),
                                );
                            }
                            n if opt_index(n, node.bytes(), "set_arg")?.is_some() => {
                                guarded_element_source = None;
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let index = opt_index(n, node.bytes(), "set_arg")?.unwrap();
                                let top = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                emit_opt_alias_store_guard(
                                    &mut builder,
                                    &env,
                                    specialization,
                                    &stack_provenance,
                                    depth,
                                    top,
                                    OptProvenance::Argument(index),
                                    node.pc(),
                                    node.deopt_guard(),
                                )?;
                                let pair = opt_use(&mut builder, stack[top]);
                                opt_define(&mut builder, arguments[index], pair);
                                opt_store(&mut builder, arg_buf, index, pair);
                                // The value stays on the stack; only older
                                // aliases of the slot go stale.
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    top,
                                    OptProvenance::Argument(index),
                                );
                            }
                            n if opt_index(n, node.bytes(), "set_loc")?.is_some() => {
                                guarded_element_source = None;
                                opt_reject_owned(
                                    &stack_provenance,
                                    depth.saturating_sub(1)..depth,
                                )?;
                                let index = opt_index(n, node.bytes(), "set_loc")?.unwrap();
                                let top = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                emit_opt_alias_store_guard(
                                    &mut builder,
                                    &env,
                                    specialization,
                                    &stack_provenance,
                                    depth,
                                    top,
                                    OptProvenance::Local(index),
                                    node.pc(),
                                    node.deopt_guard(),
                                )?;
                                let pair = opt_use(&mut builder, stack[top]);
                                opt_define(&mut builder, locals[index], pair);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, pair);
                                }
                                opt_invalidate_provenance(
                                    &mut stack_provenance,
                                    top,
                                    OptProvenance::Local(index),
                                );
                            }
                            n if opt_stack_permutation(n).is_some() => {
                                let (take, order) = opt_stack_permutation(n).unwrap();
                                let start = depth
                                    .checked_sub(take)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                opt_reject_owned(&stack_provenance, start..depth)?;
                                if start + order.len() > stack.len() {
                                    return Err(CompileFailure::ResourceLimit);
                                }
                                // Shuffles move SSA values together with the
                                // frame-slot aliases they carry, so a later
                                // exit still materializes every copy of a
                                // borrowed value as its own owner.
                                let values = (0..take)
                                    .map(|offset| {
                                        (
                                            opt_use(&mut builder, stack[start + offset]),
                                            stack_provenance[start + offset],
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                for (destination, source) in order.iter().enumerate() {
                                    let (pair, provenance) = values[*source];
                                    opt_define(&mut builder, stack[start + destination], pair);
                                    stack_provenance[start + destination] = provenance;
                                }
                                depth = start + order.len();
                            }
                            "lt" | "lte" | "gt" | "gte" => {
                                use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
                                depth = depth
                                    .checked_sub(2)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs_pair = opt_use(&mut builder, stack[depth]);
                                let rhs_pair = opt_use(&mut builder, stack[depth + 1]);
                                // Only numbers compare natively; strings,
                                // objects and nullish operands coerce in the
                                // interpreter.
                                // The raw-i32 loop shape admits only unboxed
                                // Int32 values (entry and header guards), so
                                // its operands need no per-iteration tag check.
                                if !int32_loop {
                                    let lhs_int =
                                        opt_tag_is(&mut builder, lhs_pair.tag, qjs::JS_TAG_INT);
                                    let rhs_int =
                                        opt_tag_is(&mut builder, rhs_pair.tag, qjs::JS_TAG_INT);
                                    let lhs_float =
                                        opt_tag_is(&mut builder, lhs_pair.tag, qjs::JS_TAG_FLOAT64);
                                    let rhs_float =
                                        opt_tag_is(&mut builder, rhs_pair.tag, qjs::JS_TAG_FLOAT64);
                                    let lhs_numeric = builder.ins().bor(lhs_int, lhs_float);
                                    let rhs_numeric = builder.ins().bor(rhs_int, rhs_float);
                                    let numeric = builder.ins().band(lhs_numeric, rhs_numeric);
                                    emit_opt_guard_branch(
                                        &mut builder,
                                        &env,
                                        &stack_provenance,
                                        depth + 2,
                                        node.pc(),
                                        node.deopt_guard()
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                        numeric,
                                    )?;
                                }
                                let value = if int32_loop {
                                    let cc = match name {
                                        "lt" => IntCC::SignedLessThan,
                                        "lte" => IntCC::SignedLessThanOrEqual,
                                        "gt" => IntCC::SignedGreaterThan,
                                        _ => IntCC::SignedGreaterThanOrEqual,
                                    };
                                    builder.ins().icmp(cc, lhs_pair.payload, rhs_pair.payload)
                                } else {
                                    let lhs = opt_f64(&mut builder, lhs_pair);
                                    let rhs = opt_f64(&mut builder, rhs_pair);
                                    let cc = match name {
                                        "lt" => FloatCC::LessThan,
                                        "lte" => FloatCC::LessThanOrEqual,
                                        "gt" => FloatCC::GreaterThan,
                                        _ => FloatCC::GreaterThanOrEqual,
                                    };
                                    builder.ins().fcmp(cc, lhs, rhs)
                                };
                                let pair = opt_bool_pair(&mut builder, &env, value);
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = OptProvenance::ImmediatePrimitive;
                                stack_provenance[depth + 1] = OptProvenance::Unknown;
                                depth += 1;
                            }
                            "post_inc" | "inc" | "post_dec" | "dec" => {
                                let index = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let old = opt_use(&mut builder, stack[index]);
                                let delta = OptPair {
                                    payload: builder.ins().iconst(
                                        payload_type,
                                        if name.ends_with("inc") { 1 } else { -1 },
                                    ),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                let pair = if int32_loop
                                    && name.ends_with("inc")
                                    && bounded_increments.contains(&node.pc())
                                {
                                    // Proven `k < X` at the loop header with no
                                    // other write to `k`: the increment fits.
                                    let sum = builder.ins().iadd(old.payload, delta.payload);
                                    opt_int_pair(&mut builder, &env, sum)
                                } else {
                                    emit_opt_checked_add(
                                        &mut builder,
                                        &env,
                                        &stack_provenance,
                                        depth,
                                        old,
                                        delta,
                                        node.pc(),
                                        node.deopt_guard()
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                    )?
                                };
                                if name.starts_with("post_") {
                                    // ToNumeric of a proven number is the
                                    // number itself, so the old value stays.
                                    opt_define(&mut builder, stack[index + 1], pair);
                                    stack_provenance[index + 1] = OptProvenance::ImmediatePrimitive;
                                    depth += 1;
                                } else {
                                    opt_define(&mut builder, stack[index], pair);
                                    stack_provenance[index] = OptProvenance::ImmediatePrimitive;
                                }
                            }
                            "if_false8" | "if_true8" | "if_false" | "if_true" => {
                                use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let condition = opt_use(&mut builder, stack[depth]);
                                if int32_loop {
                                    let truth = builder.ins().icmp_imm(
                                        IntCC::NotEqual,
                                        condition.payload,
                                        0,
                                    );
                                    let target = *blocks
                                        .get(
                                            &node
                                                .branch_target()
                                                .ok_or(CompileFailure::InvalidArtifact)?,
                                        )
                                        .ok_or(CompileFailure::InvalidArtifact)?;
                                    let fallthrough = *blocks
                                        .get(&next_block_pc(ir, block.start_pc())?)
                                        .ok_or(CompileFailure::InvalidArtifact)?;
                                    if name.starts_with("if_false") {
                                        builder.ins().brif(truth, fallthrough, &[], target, &[]);
                                    } else {
                                        builder.ins().brif(truth, target, &[], fallthrough, &[]);
                                    }
                                    terminated = true;
                                    continue;
                                }
                                let is_int = builder.ins().icmp_imm(
                                    IntCC::Equal,
                                    condition.tag,
                                    i64::from(qjs::JS_TAG_INT),
                                );
                                let is_bool = builder.ins().icmp_imm(
                                    IntCC::Equal,
                                    condition.tag,
                                    i64::from(qjs::JS_TAG_BOOL),
                                );
                                let is_float = builder.ins().icmp_imm(
                                    IntCC::Equal,
                                    condition.tag,
                                    i64::from(qjs::JS_TAG_FLOAT64),
                                );
                                let is_null = builder.ins().icmp_imm(
                                    IntCC::Equal,
                                    condition.tag,
                                    i64::from(qjs::JS_TAG_NULL),
                                );
                                let is_undefined = builder.ins().icmp_imm(
                                    IntCC::Equal,
                                    condition.tag,
                                    i64::from(qjs::JS_TAG_UNDEFINED),
                                );
                                let scalar = builder.ins().bor(is_int, is_bool);
                                let empty = builder.ins().bor(is_null, is_undefined);
                                let numeric = builder.ins().bor(scalar, is_float);
                                let allowed = builder.ins().bor(numeric, empty);
                                let truth_block = builder.create_block();
                                let deopt_block = builder.create_block();
                                builder
                                    .ins()
                                    .brif(allowed, truth_block, &[], deopt_block, &[]);
                                builder.switch_to_block(deopt_block);
                                for (index, vars) in arguments.iter().enumerate() {
                                    let value = opt_use(&mut builder, *vars);
                                    opt_store(&mut builder, arg_buf, index, value);
                                }
                                for (index, vars) in locals.iter().enumerate() {
                                    let value = opt_use(&mut builder, *vars);
                                    opt_store(&mut builder, var_buf, index, value);
                                }
                                for (index, vars) in stack.iter().take(depth).enumerate() {
                                    let value = opt_use(&mut builder, *vars);
                                    opt_store(&mut builder, stack_base, index, value);
                                }
                                // Resume before the branch opcode, so restore
                                // its popped condition as an owned interpreter
                                // stack value. This is the important generic
                                // refcounted deopt case (objects/strings).
                                opt_store(&mut builder, stack_base, depth, condition);
                                let start = builder.ins().load(
                                    pointer_type,
                                    MemFlags::new(),
                                    frame,
                                    layout.bytecode_start,
                                );
                                let resume = builder.ins().iadd_imm(start, i64::from(node.pc()));
                                builder
                                    .ins()
                                    .store(MemFlags::new(), resume, frame, layout.pc);
                                opt_own_stack_for_exit(
                                    &mut builder,
                                    frame,
                                    sret,
                                    stack_base,
                                    depth + 1,
                                    arguments.len() + locals.len(),
                                    &stack_provenance,
                                    &helper_signatures,
                                    pointer_type,
                                    layout,
                                )?;
                                emit_opt_exit(
                                    &mut builder,
                                    sret,
                                    qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
                                    Some(resume),
                                    pointer_type,
                                    node.deopt_guard().ok_or(CompileFailure::InvalidArtifact)?,
                                );
                                builder.switch_to_block(truth_block);
                                let integer_truth =
                                    builder
                                        .ins()
                                        .icmp_imm(IntCC::NotEqual, condition.payload, 0);
                                let float = builder.ins().bitcast(
                                    types::F64,
                                    MemFlags::new(),
                                    condition.payload,
                                );
                                let zero = builder.ins().f64const(0.0);
                                let float_truth =
                                    builder.ins().fcmp(FloatCC::OrderedNotEqual, float, zero);
                                let numeric_truth =
                                    builder.ins().select(is_float, float_truth, integer_truth);
                                let false_value = builder.ins().iconst(types::I8, 0);
                                let truth = builder.ins().select(empty, false_value, numeric_truth);
                                let target = *blocks
                                    .get(
                                        &node
                                            .branch_target()
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                    )
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let fallthrough = *blocks
                                    .get(&next_block_pc(ir, block.start_pc())?)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                if name.starts_with("if_false") {
                                    builder.ins().brif(truth, fallthrough, &[], target, &[]);
                                } else {
                                    builder.ins().brif(truth, target, &[], fallthrough, &[]);
                                }
                                terminated = true;
                            }
                            "goto" | "goto8" | "goto16" => {
                                let target = *blocks
                                    .get(
                                        &node
                                            .branch_target()
                                            .ok_or(CompileFailure::InvalidArtifact)?,
                                    )
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                builder.ins().jump(target, &[]);
                                terminated = true;
                            }
                            "return" => {
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let result = opt_use(&mut builder, stack[depth]);
                                opt_store_at(&mut builder, frame, layout.result, result);
                                opt_set_stack_top(
                                    &mut builder,
                                    frame,
                                    stack_base,
                                    depth,
                                    pointer_type,
                                    layout,
                                );
                                emit_opt_exit(
                                    &mut builder,
                                    sret,
                                    qjs::JSJitExitKind_JS_JIT_EXIT_DONE,
                                    None,
                                    pointer_type,
                                    0,
                                );
                                terminated = true;
                            }
                            "return_undef" => {
                                opt_store_at(&mut builder, frame, layout.result, undefined);
                                emit_opt_exit(
                                    &mut builder,
                                    sret,
                                    qjs::JSJitExitKind_JS_JIT_EXIT_DONE,
                                    None,
                                    pointer_type,
                                    0,
                                );
                                terminated = true;
                            }
                            "nop" => {}
                            _ => return Err(CompileFailure::UnsupportedOpcode),
                        }
                    }
                }
                if terminated {
                    break;
                }
            }
            if !terminated {
                let next = next_block_pc(ir, block.start_pc())?;
                builder.ins().jump(blocks[&next], &[]);
            }
        }
        builder.seal_all_blocks();
        builder.finalize();
    }
    // Helpers that validate a stack map (CALL, GET_GLOBAL, GET_PROPERTY, ...)
    // are always invoked with map 0, so an artifact that calls anything must
    // publish the single helper stack map the runtime checks that id against.
    let calls_helpers = clif.layout.blocks().any(|block| {
        clif.layout
            .block_insts(block)
            .any(|inst| clif.dfg.insts[inst].opcode().is_call())
    });
    super::baseline::finalize_optimized_machine(isa, clif, control, calls_helpers)
}

/// Builds the secondary, scalar-only entry used by monomorphic native call
/// edges. Its ABI is `(unboxed arguments...) -> (status:i32, unboxed result)`;
/// status zero is success and a non-zero status asks the caller to deopt at
/// the CALL bytecode.  The entry deliberately has no `JSValue`, frame, or
/// helper parameters, so neither arguments nor the result can be boxed on the
/// compiled-to-compiled fast path.
pub(crate) fn lower_direct_call_machine(
    isa: &cranelift_codegen::isa::OwnedTargetIsa,
    function: &VerifiedFunction,
    signature: &crate::runtime::BoundedSpecializationSignature,
    control: Option<&CompileControl>,
) -> Result<super::baseline::RelocatableCode, CompileFailure> {
    use crate::runtime::FeedbackRepresentation;
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, AbiParam, Function, InstBuilder, Signature};
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    if signature.function().id != function.snapshot().function_id()
        || signature.function().generation != function.snapshot().generation()
        || signature.arity() != usize::from(function.snapshot().arg_count())
    {
        return Err(CompileFailure::InvalidArtifact);
    }
    let representation = signature.result();
    if signature
        .arguments()
        .iter()
        .any(|arg| *arg != representation)
    {
        return Err(CompileFailure::InvalidArtifact);
    }
    let scalar = match representation {
        FeedbackRepresentation::Int32 => types::I32,
        FeedbackRepresentation::Float64 => types::F64,
        FeedbackRepresentation::HeapRef => return Err(CompileFailure::InvalidArtifact),
    };
    let mut abi = Signature::new(isa.default_call_conv());
    abi.params.push(AbiParam::new(isa.pointer_type()));
    abi.params
        .extend((0..signature.arity()).map(|_| AbiParam::new(scalar)));
    abi.returns.push(AbiParam::new(types::I32));
    let mut clif = Function::with_name_signature(Default::default(), abi);
    let mut context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut clif, &mut context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let output = builder.block_params(entry)[0];
        let arguments = builder.block_params(entry)[1..].to_vec();
        let mut stack = Vec::new();
        for instruction in function.instructions() {
            let name = instruction.opcode().name();
            let bytes = instruction.bytes();
            let push_int =
                |value: i64,
                 builder: &mut FunctionBuilder<'_>,
                 stack: &mut Vec<cranelift_codegen::ir::Value>| {
                    stack.push(match representation {
                        FeedbackRepresentation::Int32 => builder.ins().iconst(types::I32, value),
                        FeedbackRepresentation::Float64 => builder.ins().f64const(value as f64),
                        FeedbackRepresentation::HeapRef => {
                            unreachable!("direct calls are scalar-only")
                        }
                    });
                };
            match name {
                "nop" => {}
                "push_minus1" => push_int(-1, &mut builder, &mut stack),
                "push_i8" => push_int(i64::from(bytes[1] as i8), &mut builder, &mut stack),
                "push_i16" => push_int(
                    i64::from(i16::from_le_bytes([bytes[1], bytes[2]])),
                    &mut builder,
                    &mut stack,
                ),
                "push_i32" => push_int(
                    i64::from(i32::from_le_bytes(
                        bytes[1..5]
                            .try_into()
                            .map_err(|_| CompileFailure::InvalidArtifact)?,
                    )),
                    &mut builder,
                    &mut stack,
                ),
                "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5" | "push_6"
                | "push_7" => push_int(
                    i64::from(name.as_bytes()[5] - b'0'),
                    &mut builder,
                    &mut stack,
                ),
                n if opt_index(n, bytes, "get_arg")?.is_some() => {
                    let index = opt_index(n, bytes, "get_arg")?.unwrap();
                    let parameter = arguments
                        .get(index)
                        .copied()
                        .ok_or(CompileFailure::UnsupportedOpcode)?;
                    stack.push(parameter);
                }
                "add" | "sub" | "mul" | "div" => {
                    let rhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let lhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let value = match representation {
                        FeedbackRepresentation::Float64 => match name {
                            "add" => builder.ins().fadd(lhs, rhs),
                            "sub" => builder.ins().fsub(lhs, rhs),
                            "mul" => builder.ins().fmul(lhs, rhs),
                            _ => builder.ins().fdiv(lhs, rhs),
                        },
                        FeedbackRepresentation::Int32 => {
                            if name == "div" {
                                return Err(CompileFailure::UnsupportedOpcode);
                            }
                            let pair = match name {
                                "add" => builder.ins().sadd_overflow(lhs, rhs),
                                "sub" => builder.ins().ssub_overflow(lhs, rhs),
                                _ => builder.ins().smul_overflow(lhs, rhs),
                            };
                            let (result, overflow_flag) = pair;
                            let ok = builder.create_block();
                            builder.append_block_param(ok, types::I32);
                            let overflow =
                                builder.ins().icmp_imm(IntCC::NotEqual, overflow_flag, 0);
                            let fail = builder.create_block();
                            builder.ins().brif(overflow, fail, &[], ok, &[result]);
                            builder.switch_to_block(fail);
                            let status = builder.ins().iconst(types::I32, 1);
                            builder.ins().return_(&[status]);
                            builder.switch_to_block(ok);
                            builder.block_params(ok)[0]
                        }
                        FeedbackRepresentation::HeapRef => {
                            unreachable!("direct calls are scalar-only")
                        }
                    };
                    stack.push(value);
                }
                "neg" => {
                    let value = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let negated = match representation {
                        FeedbackRepresentation::Float64 => builder.ins().fneg(value),
                        FeedbackRepresentation::Int32 => {
                            // `-0` and `-INT32_MIN` are Float64 results the
                            // scalar Int32 ABI cannot return.
                            let zero = builder.ins().icmp_imm(IntCC::Equal, value, 0);
                            let min =
                                builder
                                    .ins()
                                    .icmp_imm(IntCC::Equal, value, i64::from(i32::MIN));
                            let unrepresentable = builder.ins().bor(zero, min);
                            let ok = builder.create_block();
                            let fail = builder.create_block();
                            builder.ins().brif(unrepresentable, fail, &[], ok, &[]);
                            builder.switch_to_block(fail);
                            let status = builder.ins().iconst(types::I32, 1);
                            builder.ins().return_(&[status]);
                            builder.switch_to_block(ok);
                            builder.ins().ineg(value)
                        }
                        FeedbackRepresentation::HeapRef => {
                            unreachable!("direct calls are scalar-only")
                        }
                    };
                    stack.push(negated);
                }
                "mod" => {
                    if representation != FeedbackRepresentation::Int32 {
                        return Err(CompileFailure::UnsupportedOpcode);
                    }
                    let rhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let lhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    // A non-negative dividend and non-zero divisor make the
                    // Int32 remainder exact (no `-0`, no trap).
                    let non_negative =
                        builder
                            .ins()
                            .icmp_imm(IntCC::SignedGreaterThanOrEqual, lhs, 0);
                    let non_zero = builder.ins().icmp_imm(IntCC::NotEqual, rhs, 0);
                    let exact = builder.ins().band(non_negative, non_zero);
                    let ok = builder.create_block();
                    let fail = builder.create_block();
                    builder.ins().brif(exact, ok, &[], fail, &[]);
                    builder.switch_to_block(fail);
                    let status = builder.ins().iconst(types::I32, 1);
                    builder.ins().return_(&[status]);
                    builder.switch_to_block(ok);
                    let remainder = builder.ins().srem(lhs, rhs);
                    stack.push(remainder);
                }
                "return" => {
                    let result = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    if !stack.is_empty() {
                        return Err(CompileFailure::InvalidArtifact);
                    }
                    let status = builder.ins().iconst(types::I32, 0);
                    builder
                        .ins()
                        .store(cranelift_codegen::ir::MemFlags::new(), result, output, 0);
                    builder.ins().return_(&[status]);
                }
                _ => return Err(CompileFailure::UnsupportedOpcode),
            }
        }
        builder.seal_all_blocks();
        builder.finalize();
    }
    super::baseline::finalize_optimized_machine(isa, clif, control, false)
}

fn opt_define(builder: &mut cranelift_frontend::FunctionBuilder<'_>, vars: OptVars, pair: OptPair) {
    builder.def_var(vars.payload, pair.payload);
    builder.def_var(vars.tag, pair.tag);
}
fn opt_use(builder: &mut cranelift_frontend::FunctionBuilder<'_>, vars: OptVars) -> OptPair {
    OptPair {
        payload: builder.use_var(vars.payload),
        tag: builder.use_var(vars.tag),
    }
}
fn opt_load(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    index: usize,
) -> OptPair {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let offset = i32::try_from(index * 16).expect("verified frame");
    OptPair {
        payload: builder
            .ins()
            .load(types::I64, MemFlags::new(), base, offset),
        tag: builder
            .ins()
            .load(types::I64, MemFlags::new(), base, offset + 8),
    }
}
fn opt_store(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    index: usize,
    pair: OptPair,
) {
    opt_store_at(
        builder,
        base,
        i32::try_from(index * 16).expect("verified frame"),
        pair,
    )
}
fn opt_store_at(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    offset: i32,
    pair: OptPair,
) {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let payload = if builder.func.dfg.value_type(pair.payload) == types::I32 {
        builder.ins().sextend(types::I64, pair.payload)
    } else {
        pair.payload
    };
    builder.ins().store(MemFlags::new(), payload, base, offset);
    builder
        .ins()
        .store(MemFlags::new(), pair.tag, base, offset + 8);
}
fn opt_f64(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    pair: OptPair,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let is_int = builder.ins().icmp_imm(
        IntCC::Equal,
        pair.tag,
        i64::from(rquickjs_core::qjs::JS_TAG_INT),
    );
    let int = builder.ins().ireduce(types::I32, pair.payload);
    let intf = builder.ins().fcvt_from_sint(types::F64, int);
    let float = builder
        .ins()
        .bitcast(types::F64, MemFlags::new(), pair.payload);
    builder.ins().select(is_int, intf, float)
}
fn opt_u16(bytes: &[u8]) -> Result<usize, CompileFailure> {
    let raw = bytes.get(1..3).ok_or(CompileFailure::InvalidArtifact)?;
    Ok(usize::from(u16::from_le_bytes([raw[0], raw[1]])))
}
fn opt_index(name: &str, bytes: &[u8], prefix: &str) -> Result<Option<usize>, CompileFailure> {
    if !name.starts_with(prefix) {
        return Ok(None);
    }
    // QuickJS's `*_loc8` opcodes carry an 8-bit operand; the trailing 8 is
    // the operand width, not the fixed local index used by `*_loc0..3`.
    if name.strip_prefix(prefix) == Some("8") {
        return bytes
            .get(1)
            .copied()
            .map(usize::from)
            .map(Some)
            .ok_or(CompileFailure::InvalidArtifact);
    }
    if let Some(last) = name.as_bytes().last().filter(|last| last.is_ascii_digit()) {
        return Ok(Some(usize::from(*last - b'0')));
    }
    Ok(Some(opt_u16(bytes)?))
}

/// Frame-level values and variable tables every guarded lowering arm needs to
/// spill state and leave through an exact deoptimization exit.
#[derive(Clone, Copy)]
struct OptEnv<'a> {
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    pointer_type: cranelift_codegen::ir::Type,
    payload_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    int32_loop: bool,
    arguments: &'a [OptVars],
    locals: &'a [OptVars],
    stack: &'a [OptVars],
    helper_signatures: &'a [cranelift_codegen::ir::SigRef],
}

/// Spills the complete frame, records the resume pc and leaves through the
/// exact deoptimization exit of `guard`. `depth` is the operand-stack depth
/// before the deoptimizing instruction pops its inputs, so the interpreter
/// re-executes it with every operand materialized and no effect applied.
fn emit_opt_deopt(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &[OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
) -> Result<(), CompileFailure> {
    use cranelift_codegen::ir::{InstBuilder, MemFlags};
    for (index, vars) in env.arguments.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.arg_buf, index, value);
    }
    for (index, vars) in env.locals.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.var_buf, index, value);
    }
    for (index, vars) in env.stack.iter().take(depth).enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.stack_base, index, value);
    }
    opt_set_stack_top(
        builder,
        env.frame,
        env.stack_base,
        depth,
        env.pointer_type,
        env.layout,
    );
    let start = builder.ins().load(
        env.pointer_type,
        MemFlags::new(),
        env.frame,
        env.layout.bytecode_start,
    );
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, env.frame, env.layout.pc);
    // The raw-i32 loop shape admits only unboxed scalars (Int32 entry guard,
    // numeric constants, no calls), so the values just stored are already
    // exact interpreter stack owners. Every other shape may alias borrowed
    // arguments or locals and must materialize an owner per slot.
    if !env.int32_loop {
        opt_own_stack_for_exit(
            builder,
            env.frame,
            env.sret,
            env.stack_base,
            depth,
            env.arguments.len() + env.locals.len(),
            provenance,
            env.helper_signatures,
            env.pointer_type,
            env.layout,
        )?;
    }
    emit_opt_exit(
        builder,
        env.sret,
        rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        env.pointer_type,
        guard,
    );
    Ok(())
}

/// Branches to a fresh pass block when `condition` holds and otherwise to an
/// exact deoptimization exit; the builder is left positioned on the pass
/// block. The returned deopt block may be reused by later checks of the same
/// instruction.
fn emit_opt_guard_branch(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &[OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
    condition: cranelift_codegen::ir::Value,
) -> Result<cranelift_codegen::ir::Block, CompileFailure> {
    use cranelift_codegen::ir::InstBuilder;
    let pass = builder.create_block();
    let deopt = builder.create_block();
    // Deoptimization exits are cold: keep them out of the hot fallthrough
    // path so a guarded loop body stays straight-line machine code.
    builder.set_cold_block(deopt);
    builder.ins().brif(condition, pass, &[], deopt, &[]);
    builder.switch_to_block(deopt);
    emit_opt_deopt(builder, env, provenance, depth, pc, guard)?;
    builder.switch_to_block(pass);
    Ok(deopt)
}

fn opt_tag_is(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    tag: cranelift_codegen::ir::Value,
    expected: i32,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::{condcodes::IntCC, InstBuilder};
    builder
        .ins()
        .icmp_imm(IntCC::Equal, tag, i64::from(expected))
}

/// The Int32 payload of a pair whose tag was (or is about to be) checked.
fn opt_i32(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    pair: OptPair,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::{types, InstBuilder};
    if env.int32_loop {
        pair.payload
    } else {
        builder.ins().ireduce(types::I32, pair.payload)
    }
}

fn opt_int_pair(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    value: cranelift_codegen::ir::Value,
) -> OptPair {
    use cranelift_codegen::ir::{types, InstBuilder};
    OptPair {
        payload: if env.int32_loop {
            value
        } else {
            builder.ins().sextend(types::I64, value)
        },
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(rquickjs_core::qjs::JS_TAG_INT)),
    }
}

fn opt_bool_pair(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    truth: cranelift_codegen::ir::Value,
) -> OptPair {
    use cranelift_codegen::ir::{types, InstBuilder};
    OptPair {
        payload: builder.ins().uextend(env.payload_type, truth),
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(rquickjs_core::qjs::JS_TAG_BOOL)),
    }
}

/// ECMAScript ToInt32 of a value already proven Int32 or Float64: exact
/// modulo 2^32 truncation, NaN and infinities map to zero.
fn opt_to_i32(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    pair: OptPair,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let is_int = opt_tag_is(builder, pair.tag, rquickjs_core::qjs::JS_TAG_INT);
    let direct = builder.ins().ireduce(types::I32, pair.payload);
    let number = builder
        .ins()
        .bitcast(types::F64, MemFlags::new(), pair.payload);
    let modulus = builder.ins().f64const(4_294_967_296.0);
    let quotient = builder.ins().fdiv(number, modulus);
    let quotient = builder.ins().trunc(quotient);
    let multiple = builder.ins().fmul(quotient, modulus);
    let remainder = builder.ins().fsub(number, multiple);
    let converted = builder.ins().fcvt_to_sint_sat(types::I64, remainder);
    let converted = builder.ins().ireduce(types::I32, converted);
    builder.ins().select(is_int, direct, converted)
}

/// Stack slots that alias a frame slot stop describing it once the slot is
/// redefined; their SSA value is the primitive the slot held before.
fn opt_invalidate_provenance(provenance: &mut [OptProvenance], depth: usize, stale: OptProvenance) {
    for slot in provenance.iter_mut().take(depth) {
        if *slot == stale {
            *slot = OptProvenance::ImmediatePrimitive;
        }
    }
}

fn opt_u8(bytes: &[u8]) -> Result<usize, CompileFailure> {
    bytes
        .get(1)
        .copied()
        .map(usize::from)
        .ok_or(CompileFailure::InvalidArtifact)
}

/// QuickJS stack shuffles as (values consumed, sources of the values pushed
/// back), identical to the interpreter and the Tier 1 lowering.
fn opt_stack_permutation(name: &str) -> Option<(usize, &'static [usize])> {
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

/// `lhs + rhs` with exact JavaScript numeric semantics for two operands whose
/// tags are checked here: Int32 + Int32 stays Int32 unless it overflows into
/// Float64, any other numeric mix is a Float64 add, and non-numeric operands
/// deoptimize before any effect. In the raw-i32 loop shape a Float64 result
/// cannot be represented, so overflow deoptimizes instead.
#[allow(clippy::too_many_arguments)]
fn emit_opt_checked_add(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &[OptProvenance],
    depth: usize,
    lhs: OptPair,
    rhs: OptPair,
    pc: u32,
    guard: u32,
) -> Result<OptPair, CompileFailure> {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;
    if env.int32_loop {
        // Every value in the raw-i32 loop shape is a proven Int32 (entry and
        // header guards), so only the overflow exit remains.
        let (sum, overflow) = builder.ins().sadd_overflow(lhs.payload, rhs.payload);
        let pass = builder.create_block();
        let deopt = builder.create_block();
        builder.set_cold_block(deopt);
        builder.append_block_param(pass, types::I32);
        builder.ins().brif(overflow, deopt, &[], pass, &[sum]);
        builder.switch_to_block(deopt);
        emit_opt_deopt(builder, env, provenance, depth, pc, guard)?;
        builder.switch_to_block(pass);
        let sum = builder.block_params(pass)[0];
        return Ok(opt_int_pair(builder, env, sum));
    }
    let lhs_int = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_INT);
    let rhs_int = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_INT);
    let both_int = builder.ins().band(lhs_int, rhs_int);
    let lhs_float = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_FLOAT64);
    let rhs_float = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_FLOAT64);
    let lhs_numeric = builder.ins().bor(lhs_int, lhs_float);
    let rhs_numeric = builder.ins().bor(rhs_int, rhs_float);
    let numeric = builder.ins().band(lhs_numeric, rhs_numeric);
    emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, numeric)?;
    let li = builder.ins().ireduce(types::I32, lhs.payload);
    let ri = builder.ins().ireduce(types::I32, rhs.payload);
    let (sum, overflow) = builder.ins().sadd_overflow(li, ri);
    let no_overflow = builder.ins().bxor_imm(overflow, 1);
    let keep_int = builder.ins().band(both_int, no_overflow);
    let lf = opt_f64(builder, lhs);
    let rf = opt_f64(builder, rhs);
    let float_sum = builder.ins().fadd(lf, rf);
    let int_payload = builder.ins().sextend(types::I64, sum);
    let float_payload = builder
        .ins()
        .bitcast(types::I64, MemFlags::new(), float_sum);
    let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
    let float_tag = builder
        .ins()
        .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
    Ok(OptPair {
        payload: builder.ins().select(keep_int, int_payload, float_payload),
        tag: builder.ins().select(keep_int, int_tag, float_tag),
    })
}

/// Unary numeric opcodes (`neg`, `plus`, `not`, `lnot`) on the stack top.
/// Numeric (and, for `lnot`, boolean/nullish) inputs are computed natively
/// with exact JavaScript results; every other tag deoptimizes to the
/// interpreter before any effect.
#[allow(clippy::too_many_arguments)]
fn emit_opt_unary(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &mut [OptProvenance],
    depth: usize,
    operation: &str,
    pc: u32,
    guard: u32,
) -> Result<(), CompileFailure> {
    use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;
    let index = depth
        .checked_sub(1)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let value = opt_use(builder, env.stack[index]);
    let is_int = opt_tag_is(builder, value.tag, qjs::JS_TAG_INT);
    let is_float = opt_tag_is(builder, value.tag, qjs::JS_TAG_FLOAT64);
    let numeric = if env.int32_loop {
        is_int
    } else {
        builder.ins().bor(is_int, is_float)
    };
    let result = match operation {
        "plus" => {
            emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, numeric)?;
            value
        }
        "neg" => {
            if env.int32_loop {
                // Zero and INT32_MIN negate to Float64 values that the raw
                // i32 loop shape cannot hold.
                let nonzero = builder.ins().icmp_imm(IntCC::NotEqual, value.payload, 0);
                let not_min =
                    builder
                        .ins()
                        .icmp_imm(IntCC::NotEqual, value.payload, i64::from(i32::MIN));
                let representable = builder.ins().band(nonzero, not_min);
                let ok = builder.ins().band(is_int, representable);
                emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, ok)?;
                let negated = builder.ins().ineg(value.payload);
                opt_int_pair(builder, env, negated)
            } else {
                emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, numeric)?;
                let iv = builder.ins().ireduce(types::I32, value.payload);
                let int_result = builder.ins().ineg(iv);
                let fv = opt_f64(builder, value);
                let float_result = builder.ins().fneg(fv);
                let nonzero = builder.ins().icmp_imm(IntCC::NotEqual, iv, 0);
                let not_min = builder
                    .ins()
                    .icmp_imm(IntCC::NotEqual, iv, i64::from(i32::MIN));
                let representable = builder.ins().band(nonzero, not_min);
                let keep_int = builder.ins().band(is_int, representable);
                let int_payload = builder.ins().sextend(types::I64, int_result);
                let float_payload =
                    builder
                        .ins()
                        .bitcast(types::I64, MemFlags::new(), float_result);
                let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
                let float_tag = builder
                    .ins()
                    .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
                OptPair {
                    payload: builder.ins().select(keep_int, int_payload, float_payload),
                    tag: builder.ins().select(keep_int, int_tag, float_tag),
                }
            }
        }
        "not" => {
            emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, numeric)?;
            let iv = if env.int32_loop {
                value.payload
            } else {
                opt_to_i32(builder, value)
            };
            let inverted = builder.ins().bnot(iv);
            opt_int_pair(builder, env, inverted)
        }
        "lnot" => {
            let is_bool = opt_tag_is(builder, value.tag, qjs::JS_TAG_BOOL);
            let is_null = opt_tag_is(builder, value.tag, qjs::JS_TAG_NULL);
            let is_undefined = opt_tag_is(builder, value.tag, qjs::JS_TAG_UNDEFINED);
            let scalar = builder.ins().bor(is_int, is_bool);
            let empty = builder.ins().bor(is_null, is_undefined);
            let numeric = if env.int32_loop {
                scalar
            } else {
                builder.ins().bor(scalar, is_float)
            };
            let allowed = builder.ins().bor(numeric, empty);
            emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, allowed)?;
            let iv = opt_i32(builder, env, value);
            let integer_truth = builder.ins().icmp_imm(IntCC::NotEqual, iv, 0);
            let numeric_truth = if env.int32_loop {
                integer_truth
            } else {
                let float = builder
                    .ins()
                    .bitcast(types::F64, MemFlags::new(), value.payload);
                let zero = builder.ins().f64const(0.0);
                let float_truth = builder.ins().fcmp(FloatCC::OrderedNotEqual, float, zero);
                builder.ins().select(is_float, float_truth, integer_truth)
            };
            let false_value = builder.ins().iconst(types::I8, 0);
            let truth = builder.ins().select(empty, false_value, numeric_truth);
            let negated = builder.ins().bxor_imm(truth, 1);
            opt_bool_pair(builder, env, negated)
        }
        _ => return Err(CompileFailure::UnsupportedOpcode),
    };
    opt_define(builder, env.stack[index], result);
    provenance[index] = OptProvenance::ImmediatePrimitive;
    Ok(())
}

/// `%` on two Int32 operands whose result is provably an Int32: a non-negative
/// dividend never yields `-0` and rules out `INT32_MIN % -1`, and a non-zero
/// divisor cannot trap. Everything else (Float64 operands, negative
/// dividends, zero divisors, non-numeric tags) deoptimizes.
fn emit_opt_mod(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::{condcodes::IntCC, InstBuilder};
    use rquickjs_core::qjs;
    let output = depth
        .checked_sub(2)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let lhs = opt_use(builder, env.stack[output]);
    let rhs = opt_use(builder, env.stack[output + 1]);
    let lhs_int = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_INT);
    let rhs_int = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_INT);
    let both_int = builder.ins().band(lhs_int, rhs_int);
    let li = opt_i32(builder, env, lhs);
    let ri = opt_i32(builder, env, rhs);
    let non_negative = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, li, 0);
    let non_zero = builder.ins().icmp_imm(IntCC::NotEqual, ri, 0);
    let exact = builder.ins().band(non_negative, non_zero);
    let ok = builder.ins().band(both_int, exact);
    emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, ok)?;
    let remainder = builder.ins().srem(li, ri);
    let pair = opt_int_pair(builder, env, remainder);
    opt_define(builder, env.stack[output], pair);
    provenance[output] = OptProvenance::ImmediatePrimitive;
    provenance[output + 1] = OptProvenance::Unknown;
    Ok(output + 1)
}

/// `==`, `!=`, `===` and `!==` with exact results for operands that need no
/// coercion, i.e. both tagged Int32, Float64, Bool, `undefined` or `null`:
/// two numbers compare as Float64 (NaN is unequal to itself, `-0` equals
/// `0`), two booleans compare payloads, `undefined`/`null` pairs are equal
/// under `==` and equal under `===` only with matching tags, and any other
/// combination of these tags is unequal. Loose equality of a boolean with a
/// number coerces and deoptimizes, as does every string, object, symbol or
/// BigInt operand, so those run in the interpreter.
#[allow(clippy::too_many_arguments)]
fn emit_opt_equality(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &mut [OptProvenance],
    depth: usize,
    operation: &str,
    pc: u32,
    guard: u32,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
    use cranelift_codegen::ir::{types, InstBuilder};
    use rquickjs_core::qjs;
    let output = depth
        .checked_sub(2)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let lhs = opt_use(builder, env.stack[output]);
    let rhs = opt_use(builder, env.stack[output + 1]);
    let strict = matches!(operation, "strict_eq" | "strict_neq");
    let lhs_int = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_INT);
    let rhs_int = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_INT);
    let (lhs_numeric, rhs_numeric) = if env.int32_loop {
        (lhs_int, rhs_int)
    } else {
        let lhs_float = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_FLOAT64);
        let rhs_float = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_FLOAT64);
        (
            builder.ins().bor(lhs_int, lhs_float),
            builder.ins().bor(rhs_int, rhs_float),
        )
    };
    let both_numeric = builder.ins().band(lhs_numeric, rhs_numeric);
    let lhs_bool = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_BOOL);
    let rhs_bool = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_BOOL);
    let lhs_undefined = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_UNDEFINED);
    let rhs_undefined = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_UNDEFINED);
    let lhs_null = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_NULL);
    let rhs_null = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_NULL);
    let lhs_nullish = builder.ins().bor(lhs_undefined, lhs_null);
    let rhs_nullish = builder.ins().bor(rhs_undefined, rhs_null);
    let lhs_primitive = builder.ins().bor(lhs_numeric, lhs_bool);
    let lhs_primitive = builder.ins().bor(lhs_primitive, lhs_nullish);
    let rhs_primitive = builder.ins().bor(rhs_numeric, rhs_bool);
    let rhs_primitive = builder.ins().bor(rhs_primitive, rhs_nullish);
    let mut allowed = builder.ins().band(lhs_primitive, rhs_primitive);
    if !strict {
        let lhs_coerces = builder.ins().band(lhs_bool, rhs_numeric);
        let rhs_coerces = builder.ins().band(rhs_bool, lhs_numeric);
        let coerces = builder.ins().bor(lhs_coerces, rhs_coerces);
        let pure = builder.ins().bxor_imm(coerces, 1);
        allowed = builder.ins().band(allowed, pure);
    }
    emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, allowed)?;
    let numeric_equal = if env.int32_loop {
        builder.ins().icmp(IntCC::Equal, lhs.payload, rhs.payload)
    } else {
        let lf = opt_f64(builder, lhs);
        let rf = opt_f64(builder, rhs);
        builder.ins().fcmp(FloatCC::Equal, lf, rf)
    };
    let li = opt_i32(builder, env, lhs);
    let ri = opt_i32(builder, env, rhs);
    let payload_equal = builder.ins().icmp(IntCC::Equal, li, ri);
    let both_bool = builder.ins().band(lhs_bool, rhs_bool);
    let both_nullish = builder.ins().band(lhs_nullish, rhs_nullish);
    let nullish_equal = if strict {
        builder.ins().icmp(IntCC::Equal, lhs.tag, rhs.tag)
    } else {
        builder.ins().iconst(types::I8, 1)
    };
    let falsehood = builder.ins().iconst(types::I8, 0);
    let simple_equal = builder.ins().select(both_nullish, nullish_equal, falsehood);
    let simple_equal = builder.ins().select(both_bool, payload_equal, simple_equal);
    let equal = builder
        .ins()
        .select(both_numeric, numeric_equal, simple_equal);
    let result = if matches!(operation, "neq" | "strict_neq") {
        builder.ins().bxor_imm(equal, 1)
    } else {
        equal
    };
    let pair = opt_bool_pair(builder, env, result);
    opt_define(builder, env.stack[output], pair);
    provenance[output] = OptProvenance::ImmediatePrimitive;
    provenance[output + 1] = OptProvenance::Unknown;
    Ok(output + 1)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_guarded_propkey(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{InstBuilder, MemFlags};
    use rquickjs_core::qjs;
    let index = depth
        .checked_sub(1)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let value = opt_use(builder, stack[index]);
    let int = builder
        .ins()
        .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_INT));
    let string = builder
        .ins()
        .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_STRING));
    let symbol = builder
        .ins()
        .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_SYMBOL));
    let valid = builder.ins().bor(int, string);
    let valid = builder.ins().bor(valid, symbol);
    let continuation = builder.create_block();
    let deopt = builder.create_block();
    builder.ins().brif(valid, continuation, &[], deopt, &[]);
    builder.switch_to_block(deopt);
    for (slot, vars) in arguments.iter().enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, arg_buf, slot, current);
    }
    for (slot, vars) in locals.iter().enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, var_buf, slot, current);
    }
    for (slot, vars) in stack.iter().take(depth).enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, stack_base, slot, current);
    }
    opt_set_stack_top(builder, frame, stack_base, depth, pointer_type, layout);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        arguments.len() + locals.len(),
        stack_provenance,
        helper_signatures,
        pointer_type,
        layout,
    )?;
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );
    builder.switch_to_block(continuation);
    Ok(depth)
}

/// Bitwise and shift opcodes on two Int32 operands. Shift counts are masked
/// to five bits exactly like ToUint32(count) & 31 in the specification, and
/// `>>>` renormalizes results at or above 2^31 to Float64 (or deoptimizes in
/// the raw-i32 loop shape, which cannot hold a Float64). Non-Int32 operands
/// deoptimize so the interpreter performs ToInt32/ToNumeric with effects.
fn emit_opt_guarded_int_binary(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &mut [OptProvenance],
    depth: usize,
    operation: &str,
    pc: u32,
    guard: u32,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;

    let output = depth
        .checked_sub(2)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let lhs = opt_use(builder, env.stack[output]);
    let rhs = opt_use(builder, env.stack[output + 1]);
    let lhs_int = opt_tag_is(builder, lhs.tag, qjs::JS_TAG_INT);
    let rhs_int = opt_tag_is(builder, rhs.tag, qjs::JS_TAG_INT);
    let both_int = builder.ins().band(lhs_int, rhs_int);
    let deopt = emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, both_int)?;
    let li = opt_i32(builder, env, lhs);
    let ri = opt_i32(builder, env, rhs);
    let value = match operation {
        "or" => builder.ins().bor(li, ri),
        "and" => builder.ins().band(li, ri),
        "xor" => builder.ins().bxor(li, ri),
        "shl" | "sar" | "shr" => {
            let count = builder.ins().band_imm(ri, 31);
            match operation {
                "shl" => builder.ins().ishl(li, count),
                "sar" => builder.ins().sshr(li, count),
                _ => builder.ins().ushr(li, count),
            }
        }
        _ => return Err(CompileFailure::UnsupportedOpcode),
    };
    let result = if operation == "shr" {
        let fits_int32 = builder
            .ins()
            .icmp_imm(IntCC::SignedGreaterThanOrEqual, value, 0);
        if env.int32_loop {
            let int_block = builder.create_block();
            builder.ins().brif(fits_int32, int_block, &[], deopt, &[]);
            builder.switch_to_block(int_block);
            opt_int_pair(builder, env, value)
        } else {
            let int_payload = builder.ins().sextend(types::I64, value);
            let uint_float = builder.ins().fcvt_from_uint(types::F64, value);
            let float_payload = builder
                .ins()
                .bitcast(types::I64, MemFlags::new(), uint_float);
            let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
            let float_tag = builder
                .ins()
                .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
            OptPair {
                payload: builder.ins().select(fits_int32, int_payload, float_payload),
                tag: builder.ins().select(fits_int32, int_tag, float_tag),
            }
        }
    } else {
        opt_int_pair(builder, env, value)
    };
    opt_define(builder, env.stack[output], result);
    provenance[output] = OptProvenance::ImmediatePrimitive;
    provenance[output + 1] = OptProvenance::Unknown;
    Ok(output + 1)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_array_length(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    element_layout: crate::abi::ElementLayout,
    block_pc: u32,
    source_provenance: OptProvenance,
    guarded_source: &mut Option<GuardedElementSource>,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;

    let index = depth
        .checked_sub(1)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let object = opt_use(builder, stack[index]);
    let classify = builder.create_block();
    let deopt = builder.create_block();
    let packed = builder.create_block();
    let typed = builder.create_block();
    let continuation = builder.create_block();
    builder.append_block_param(typed, types::I8);
    builder.append_block_param(continuation, types::I32);
    builder.append_block_param(continuation, pointer_type);
    builder.append_block_param(continuation, types::I8);
    let object_ok = builder
        .ins()
        .icmp_imm(IntCC::Equal, object.tag, i64::from(qjs::JS_TAG_OBJECT));
    builder.ins().brif(object_ok, classify, &[], deopt, &[]);

    builder.switch_to_block(classify);
    let flags = builder.ins().load(
        types::I8,
        MemFlags::new(),
        object.payload,
        element_layout.object_flags_offset,
    );
    let fast = builder
        .ins()
        .band_imm(flags, element_layout.object_fast_array_mask);
    let fast = builder.ins().icmp_imm(IntCC::NotEqual, fast, 0);
    let class_check = builder.create_block();
    builder.ins().brif(fast, class_check, &[], deopt, &[]);
    builder.switch_to_block(class_check);
    let class = builder.ins().load(
        types::I16,
        MemFlags::new(),
        object.payload,
        element_layout.object_class_id_offset,
    );
    let class = builder.ins().uextend(types::I64, class);
    let is_array = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.array_class_id);
    let typed_check = builder.create_block();
    builder.ins().brif(is_array, packed, &[], typed_check, &[]);
    builder.switch_to_block(typed_check);
    let is_i32 = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.int32_array_class_id);
    let is_f64 = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.float64_array_class_id);
    let is_typed = builder.ins().bor(is_i32, is_f64);
    let int_kind = builder.ins().iconst(types::I8, 1);
    let float_kind = builder.ins().iconst(types::I8, 2);
    let typed_kind = builder.ins().select(is_i32, int_kind, float_kind);
    let typed_accepted = builder.create_block();
    builder
        .ins()
        .brif(is_typed, typed_accepted, &[], deopt, &[]);
    builder.switch_to_block(typed_accepted);
    builder.ins().jump(typed, &[typed_kind]);

    builder.switch_to_block(packed);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let packed_ready = builder.create_block();
    builder.ins().brif(has_data, packed_ready, &[], deopt, &[]);
    builder.switch_to_block(packed_ready);
    let kind = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(continuation, &[count, data, kind]);

    builder.switch_to_block(typed);
    let typed_data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.typed_array_ptr_offset,
    );
    let has_typed = builder.ins().icmp_imm(IntCC::NotEqual, typed_data, 0);
    let stable_check = builder.create_block();
    builder.ins().brif(has_typed, stable_check, &[], deopt, &[]);
    builder.switch_to_block(stable_check);
    let tracks_resizable = builder.ins().load(
        types::I8,
        MemFlags::new(),
        typed_data,
        element_layout.typed_array_track_rab_offset,
    );
    let stable = builder.ins().icmp_imm(IntCC::Equal, tracks_resizable, 0);
    let buffer_check = builder.create_block();
    builder.ins().brif(stable, buffer_check, &[], deopt, &[]);
    builder.switch_to_block(buffer_check);
    let buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        typed_data,
        element_layout.typed_array_buffer_offset,
    );
    let has_buffer = builder.ins().icmp_imm(IntCC::NotEqual, buffer, 0);
    let array_buffer_check = builder.create_block();
    builder
        .ins()
        .brif(has_buffer, array_buffer_check, &[], deopt, &[]);
    builder.switch_to_block(array_buffer_check);
    let array_buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        buffer,
        element_layout.object_union_offset,
    );
    let has_array_buffer = builder.ins().icmp_imm(IntCC::NotEqual, array_buffer, 0);
    let detach_check = builder.create_block();
    builder
        .ins()
        .brif(has_array_buffer, detach_check, &[], deopt, &[]);
    builder.switch_to_block(detach_check);
    let detached = builder.ins().load(
        types::I8,
        MemFlags::new(),
        array_buffer,
        element_layout.array_buffer_detached_offset,
    );
    let attached = builder.ins().icmp_imm(IntCC::Equal, detached, 0);
    let load_count = builder.create_block();
    builder.ins().brif(attached, load_count, &[], deopt, &[]);
    builder.switch_to_block(load_count);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let typed_ready = builder.create_block();
    builder.ins().brif(has_data, typed_ready, &[], deopt, &[]);
    builder.switch_to_block(typed_ready);
    let kind = builder.block_params(typed)[0];
    builder.ins().jump(continuation, &[count, data, kind]);

    builder.switch_to_block(deopt);
    for (slot, vars) in arguments.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, arg_buf, slot, value);
    }
    for (slot, vars) in locals.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, var_buf, slot, value);
    }
    for (slot, vars) in stack.iter().take(depth).enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, stack_base, slot, value);
    }
    opt_set_stack_top(builder, frame, stack_base, depth, pointer_type, layout);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        arguments.len() + locals.len(),
        stack_provenance,
        helper_signatures,
        pointer_type,
        layout,
    )?;
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );

    builder.switch_to_block(continuation);
    let count = builder.block_params(continuation)[0];
    let data = builder.block_params(continuation)[1];
    let kind = builder.block_params(continuation)[2];
    let result = OptPair {
        payload: builder.ins().sextend(types::I64, count),
        tag: builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
    };
    opt_define(builder, stack[index], result);
    stack_provenance[index] = OptProvenance::ImmediatePrimitive;
    *guarded_source = Some(GuardedElementSource {
        provenance: source_provenance,
        block_pc,
        data,
        count,
        kind,
    });
    Ok(depth)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_element_get(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    element_layout: crate::abi::ElementLayout,
    guarded_source: Option<GuardedElementSource>,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;

    let object_index = depth
        .checked_sub(2)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let object = opt_use(builder, stack[object_index]);
    let key = opt_use(builder, stack[object_index + 1]);
    let direct = builder.create_block();
    let deopt = builder.create_block();
    let classify = builder.create_block();
    let packed = builder.create_block();
    let int32 = builder.create_block();
    let float64 = builder.create_block();
    let typed_common = builder.create_block();
    let continuation = builder.create_block();
    builder.append_block_param(typed_common, types::I8);
    builder.append_block_param(continuation, types::I64);
    builder.append_block_param(continuation, types::I64);

    let object_ok = builder
        .ins()
        .icmp_imm(IntCC::Equal, object.tag, i64::from(qjs::JS_TAG_OBJECT));
    let key_ok = builder
        .ins()
        .icmp_imm(IntCC::Equal, key.tag, i64::from(qjs::JS_TAG_INT));
    let tags_ok = builder.ins().band(object_ok, key_ok);
    let cached = guarded_source.filter(|source| {
        source.provenance == stack_provenance[object_index]
            && matches!(
                source.provenance,
                OptProvenance::Argument(_) | OptProvenance::Local(_)
            )
    });
    let cached_index = cached.map(|_| builder.create_block());
    builder
        .ins()
        .brif(tags_ok, cached_index.unwrap_or(direct), &[], deopt, &[]);

    if let (Some(source), Some(cached_index)) = (cached, cached_index) {
        builder.switch_to_block(cached_index);
        let index = builder.ins().ireduce(types::I32, key.payload);
        let in_bounds = builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, source.count);
        let cached_dispatch = builder.create_block();
        builder
            .ins()
            .brif(in_bounds, cached_dispatch, &[], deopt, &[]);
        builder.switch_to_block(cached_dispatch);
        let packed_kind = builder.ins().icmp_imm(IntCC::Equal, source.kind, 0);
        let cached_packed = builder.create_block();
        let cached_typed = builder.create_block();
        builder
            .ins()
            .brif(packed_kind, cached_packed, &[], cached_typed, &[]);
        builder.switch_to_block(cached_packed);
        let offset = builder.ins().imul_imm(index, 16);
        let offset = builder.ins().uextend(pointer_type, offset);
        let address = builder.ins().iadd(source.data, offset);
        let payload = builder.ins().load(types::I64, MemFlags::new(), address, 0);
        let tag = builder
            .ins()
            .load(types::I64, MemFlags::new(), address, layout.value_tag);
        let primitive = builder
            .ins()
            .icmp_imm(IntCC::SignedGreaterThanOrEqual, tag, 0);
        let cached_packed_done = builder.create_block();
        builder
            .ins()
            .brif(primitive, cached_packed_done, &[], deopt, &[]);
        builder.switch_to_block(cached_packed_done);
        builder.ins().jump(continuation, &[payload, tag]);
        builder.switch_to_block(cached_typed);
        let int_kind = builder.ins().icmp_imm(IntCC::Equal, source.kind, 1);
        let cached_int = builder.create_block();
        let cached_float = builder.create_block();
        builder
            .ins()
            .brif(int_kind, cached_int, &[], cached_float, &[]);
        builder.switch_to_block(cached_int);
        let offset = builder.ins().imul_imm(index, 4);
        let offset = builder.ins().uextend(pointer_type, offset);
        let address = builder.ins().iadd(source.data, offset);
        let value = builder.ins().load(types::I32, MemFlags::new(), address, 0);
        let value = builder.ins().sextend(types::I64, value);
        let tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
        builder.ins().jump(continuation, &[value, tag]);
        builder.switch_to_block(cached_float);
        let offset = builder.ins().imul_imm(index, 8);
        let offset = builder.ins().uextend(pointer_type, offset);
        let address = builder.ins().iadd(source.data, offset);
        let value = builder.ins().load(types::F64, MemFlags::new(), address, 0);
        let value = builder.ins().bitcast(types::I64, MemFlags::new(), value);
        let tag = builder
            .ins()
            .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
        builder.ins().jump(continuation, &[value, tag]);
    }

    builder.switch_to_block(direct);
    let index = builder.ins().ireduce(types::I32, key.payload);
    let non_negative = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, index, 0);
    builder.ins().brif(non_negative, classify, &[], deopt, &[]);

    builder.switch_to_block(classify);
    let flags = builder.ins().load(
        types::I8,
        MemFlags::new(),
        object.payload,
        element_layout.object_flags_offset,
    );
    let fast = builder
        .ins()
        .band_imm(flags, element_layout.object_fast_array_mask);
    let fast = builder.ins().icmp_imm(IntCC::NotEqual, fast, 0);
    let class_check = builder.create_block();
    builder.ins().brif(fast, class_check, &[], deopt, &[]);
    builder.switch_to_block(class_check);
    let class = builder.ins().load(
        types::I16,
        MemFlags::new(),
        object.payload,
        element_layout.object_class_id_offset,
    );
    let class = builder.ins().uextend(types::I64, class);
    let is_packed = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.array_class_id);
    let not_packed = builder.create_block();
    builder.ins().brif(is_packed, packed, &[], not_packed, &[]);
    builder.switch_to_block(not_packed);
    let is_int32 = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.int32_array_class_id);
    let not_int32 = builder.create_block();
    builder.ins().brif(is_int32, int32, &[], not_int32, &[]);
    builder.switch_to_block(not_int32);
    let is_float64 =
        builder
            .ins()
            .icmp_imm(IntCC::Equal, class, element_layout.float64_array_class_id);
    builder.ins().brif(is_float64, float64, &[], deopt, &[]);

    builder.switch_to_block(packed);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, count);
    let packed_load = builder.create_block();
    builder.ins().brif(in_bounds, packed_load, &[], deopt, &[]);
    builder.switch_to_block(packed_load);
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let packed_value = builder.create_block();
    builder.ins().brif(has_data, packed_value, &[], deopt, &[]);
    builder.switch_to_block(packed_value);
    let scaled = builder.ins().imul_imm(index, 16);
    let scaled = builder.ins().uextend(pointer_type, scaled);
    let address = builder.ins().iadd(data, scaled);
    let payload = builder.ins().load(types::I64, MemFlags::new(), address, 0);
    let tag = builder
        .ins()
        .load(types::I64, MemFlags::new(), address, layout.value_tag);
    let primitive = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, tag, 0);
    let packed_done = builder.create_block();
    builder.ins().brif(primitive, packed_done, &[], deopt, &[]);
    builder.switch_to_block(packed_done);
    builder.ins().jump(continuation, &[payload, tag]);

    builder.switch_to_block(int32);
    let kind = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(typed_common, &[kind]);
    builder.switch_to_block(float64);
    let kind = builder.ins().iconst(types::I8, 1);
    builder.ins().jump(typed_common, &[kind]);

    builder.switch_to_block(typed_common);
    let typed = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.typed_array_ptr_offset,
    );
    let has_typed = builder.ins().icmp_imm(IntCC::NotEqual, typed, 0);
    let typed_guard = builder.create_block();
    builder.ins().brif(has_typed, typed_guard, &[], deopt, &[]);
    builder.switch_to_block(typed_guard);
    let tracks_resizable = builder.ins().load(
        types::I8,
        MemFlags::new(),
        typed,
        element_layout.typed_array_track_rab_offset,
    );
    let stable = builder.ins().icmp_imm(IntCC::Equal, tracks_resizable, 0);
    let stable_buffer = builder.create_block();
    builder.ins().brif(stable, stable_buffer, &[], deopt, &[]);
    builder.switch_to_block(stable_buffer);
    let buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        typed,
        element_layout.typed_array_buffer_offset,
    );
    let has_buffer = builder.ins().icmp_imm(IntCC::NotEqual, buffer, 0);
    let buffer_object = builder.create_block();
    builder
        .ins()
        .brif(has_buffer, buffer_object, &[], deopt, &[]);
    builder.switch_to_block(buffer_object);
    let array_buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        buffer,
        element_layout.object_union_offset,
    );
    let has_array_buffer = builder.ins().icmp_imm(IntCC::NotEqual, array_buffer, 0);
    let detach_guard = builder.create_block();
    builder
        .ins()
        .brif(has_array_buffer, detach_guard, &[], deopt, &[]);
    builder.switch_to_block(detach_guard);
    let detached = builder.ins().load(
        types::I8,
        MemFlags::new(),
        array_buffer,
        element_layout.array_buffer_detached_offset,
    );
    let attached = builder.ins().icmp_imm(IntCC::Equal, detached, 0);
    let typed_bounds = builder.create_block();
    builder.ins().brif(attached, typed_bounds, &[], deopt, &[]);
    builder.switch_to_block(typed_bounds);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, count);
    let typed_data = builder.create_block();
    builder.ins().brif(in_bounds, typed_data, &[], deopt, &[]);
    builder.switch_to_block(typed_data);
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let typed_load = builder.create_block();
    builder.ins().brif(has_data, typed_load, &[], deopt, &[]);
    builder.switch_to_block(typed_load);
    let kind = builder.block_params(typed_common)[0];
    let is_float = builder.ins().icmp_imm(IntCC::NotEqual, kind, 0);
    let load_i32 = builder.create_block();
    let load_f64 = builder.create_block();
    builder.ins().brif(is_float, load_f64, &[], load_i32, &[]);
    builder.switch_to_block(load_i32);
    let offset = builder.ins().imul_imm(index, 4);
    let offset = builder.ins().uextend(pointer_type, offset);
    let address = builder.ins().iadd(data, offset);
    let value = builder.ins().load(types::I32, MemFlags::new(), address, 0);
    let value = builder.ins().sextend(types::I64, value);
    let tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
    builder.ins().jump(continuation, &[value, tag]);
    builder.switch_to_block(load_f64);
    let offset = builder.ins().imul_imm(index, 8);
    let offset = builder.ins().uextend(pointer_type, offset);
    let address = builder.ins().iadd(data, offset);
    let value = builder.ins().load(types::F64, MemFlags::new(), address, 0);
    let value = builder.ins().bitcast(types::I64, MemFlags::new(), value);
    let tag = builder
        .ins()
        .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
    builder.ins().jump(continuation, &[value, tag]);

    builder.switch_to_block(deopt);
    for (index, vars) in arguments.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, arg_buf, index, value);
    }
    for (index, vars) in locals.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, var_buf, index, value);
    }
    for (index, vars) in stack.iter().take(depth).enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, stack_base, index, value);
    }
    opt_set_stack_top(builder, frame, stack_base, depth, pointer_type, layout);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        arguments.len() + locals.len(),
        stack_provenance,
        helper_signatures,
        pointer_type,
        layout,
    )?;
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );

    builder.switch_to_block(continuation);
    let result = OptPair {
        payload: builder.block_params(continuation)[0],
        tag: builder.block_params(continuation)[1],
    };
    opt_define(builder, stack[object_index], result);
    stack_provenance[object_index] = OptProvenance::ImmediatePrimitive;
    stack_provenance[object_index + 1] = OptProvenance::Unknown;
    Ok(object_index + 1)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_element_put(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    guard: u32,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    element_layout: crate::abi::ElementLayout,
    block_pc: u32,
    source_provenance: OptProvenance,
    guarded_source: &mut Option<GuardedElementSource>,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;

    let object_index = depth
        .checked_sub(3)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let object = opt_use(builder, stack[object_index]);
    let key = opt_use(builder, stack[object_index + 1]);
    let value = opt_use(builder, stack[object_index + 2]);
    let direct = builder.create_block();
    let deopt = builder.create_block();
    let classify = builder.create_block();
    let packed = builder.create_block();
    let int32 = builder.create_block();
    let float64 = builder.create_block();
    let typed_common = builder.create_block();
    let continuation = builder.create_block();
    builder.append_block_param(typed_common, types::I8);
    builder.append_block_param(continuation, types::I32);
    builder.append_block_param(continuation, pointer_type);
    builder.append_block_param(continuation, types::I8);

    let object_ok = builder
        .ins()
        .icmp_imm(IntCC::Equal, object.tag, i64::from(qjs::JS_TAG_OBJECT));
    let key_ok = builder
        .ins()
        .icmp_imm(IntCC::Equal, key.tag, i64::from(qjs::JS_TAG_INT));
    let tags_ok = builder.ins().band(object_ok, key_ok);
    builder.ins().brif(tags_ok, direct, &[], deopt, &[]);
    builder.switch_to_block(direct);
    let index = builder.ins().ireduce(types::I32, key.payload);
    let non_negative = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, index, 0);
    builder.ins().brif(non_negative, classify, &[], deopt, &[]);

    builder.switch_to_block(classify);
    let flags = builder.ins().load(
        types::I8,
        MemFlags::new(),
        object.payload,
        element_layout.object_flags_offset,
    );
    let fast = builder
        .ins()
        .band_imm(flags, element_layout.object_fast_array_mask);
    let fast = builder.ins().icmp_imm(IntCC::NotEqual, fast, 0);
    let class_check = builder.create_block();
    builder.ins().brif(fast, class_check, &[], deopt, &[]);
    builder.switch_to_block(class_check);
    let class = builder.ins().load(
        types::I16,
        MemFlags::new(),
        object.payload,
        element_layout.object_class_id_offset,
    );
    let class = builder.ins().uextend(types::I64, class);
    let is_packed = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.array_class_id);
    let not_packed = builder.create_block();
    builder.ins().brif(is_packed, packed, &[], not_packed, &[]);
    builder.switch_to_block(not_packed);
    let is_int32 = builder
        .ins()
        .icmp_imm(IntCC::Equal, class, element_layout.int32_array_class_id);
    let not_int32 = builder.create_block();
    builder.ins().brif(is_int32, int32, &[], not_int32, &[]);
    builder.switch_to_block(not_int32);
    let is_float64 =
        builder
            .ins()
            .icmp_imm(IntCC::Equal, class, element_layout.float64_array_class_id);
    builder.ins().brif(is_float64, float64, &[], deopt, &[]);

    builder.switch_to_block(packed);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, count);
    let primitive = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, value.tag, 0);
    let packed_ok = builder.ins().band(in_bounds, primitive);
    let packed_data = builder.create_block();
    builder.ins().brif(packed_ok, packed_data, &[], deopt, &[]);
    builder.switch_to_block(packed_data);
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let packed_store = builder.create_block();
    builder.ins().brif(has_data, packed_store, &[], deopt, &[]);
    builder.switch_to_block(packed_store);
    let offset = builder.ins().imul_imm(index, 16);
    let offset = builder.ins().uextend(pointer_type, offset);
    let address = builder.ins().iadd(data, offset);
    let old_tag = builder
        .ins()
        .load(types::I64, MemFlags::new(), address, layout.value_tag);
    let old_primitive = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, old_tag, 0);
    let do_packed_store = builder.create_block();
    builder
        .ins()
        .brif(old_primitive, do_packed_store, &[], deopt, &[]);
    builder.switch_to_block(do_packed_store);
    builder
        .ins()
        .store(MemFlags::new(), value.payload, address, 0);
    builder
        .ins()
        .store(MemFlags::new(), value.tag, address, layout.value_tag);
    let kind = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(continuation, &[count, data, kind]);

    builder.switch_to_block(int32);
    let int_value = builder
        .ins()
        .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_INT));
    let int_typed = builder.create_block();
    builder.ins().brif(int_value, int_typed, &[], deopt, &[]);
    builder.switch_to_block(int_typed);
    let kind = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(typed_common, &[kind]);
    builder.switch_to_block(float64);
    let value_int = builder
        .ins()
        .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_INT));
    let value_float =
        builder
            .ins()
            .icmp_imm(IntCC::Equal, value.tag, i64::from(qjs::JS_TAG_FLOAT64));
    let numeric = builder.ins().bor(value_int, value_float);
    let float_typed = builder.create_block();
    builder.ins().brif(numeric, float_typed, &[], deopt, &[]);
    builder.switch_to_block(float_typed);
    let kind = builder.ins().iconst(types::I8, 1);
    builder.ins().jump(typed_common, &[kind]);

    builder.switch_to_block(typed_common);
    let typed_data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.typed_array_ptr_offset,
    );
    let has_typed = builder.ins().icmp_imm(IntCC::NotEqual, typed_data, 0);
    let stable_check = builder.create_block();
    builder.ins().brif(has_typed, stable_check, &[], deopt, &[]);
    builder.switch_to_block(stable_check);
    let tracks_resizable = builder.ins().load(
        types::I8,
        MemFlags::new(),
        typed_data,
        element_layout.typed_array_track_rab_offset,
    );
    let stable = builder.ins().icmp_imm(IntCC::Equal, tracks_resizable, 0);
    let buffer_check = builder.create_block();
    builder.ins().brif(stable, buffer_check, &[], deopt, &[]);
    builder.switch_to_block(buffer_check);
    let buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        typed_data,
        element_layout.typed_array_buffer_offset,
    );
    let has_buffer = builder.ins().icmp_imm(IntCC::NotEqual, buffer, 0);
    let array_buffer_check = builder.create_block();
    builder
        .ins()
        .brif(has_buffer, array_buffer_check, &[], deopt, &[]);
    builder.switch_to_block(array_buffer_check);
    let array_buffer = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        buffer,
        element_layout.object_union_offset,
    );
    let has_array_buffer = builder.ins().icmp_imm(IntCC::NotEqual, array_buffer, 0);
    let buffer_state = builder.create_block();
    builder
        .ins()
        .brif(has_array_buffer, buffer_state, &[], deopt, &[]);
    builder.switch_to_block(buffer_state);
    let detached = builder.ins().load(
        types::I8,
        MemFlags::new(),
        array_buffer,
        element_layout.array_buffer_detached_offset,
    );
    let immutable = builder.ins().load(
        types::I8,
        MemFlags::new(),
        array_buffer,
        element_layout.array_buffer_immutable_offset,
    );
    let attached = builder.ins().icmp_imm(IntCC::Equal, detached, 0);
    let mutable = builder.ins().icmp_imm(IntCC::Equal, immutable, 0);
    let usable = builder.ins().band(attached, mutable);
    let bounds_check = builder.create_block();
    builder.ins().brif(usable, bounds_check, &[], deopt, &[]);
    builder.switch_to_block(bounds_check);
    let count = builder.ins().load(
        types::I32,
        MemFlags::new(),
        object.payload,
        element_layout.array_count_offset,
    );
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, count);
    let data_check = builder.create_block();
    builder.ins().brif(in_bounds, data_check, &[], deopt, &[]);
    builder.switch_to_block(data_check);
    let data = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        object.payload,
        element_layout.array_data_offset,
    );
    let has_data = builder.ins().icmp_imm(IntCC::NotEqual, data, 0);
    let typed_store = builder.create_block();
    builder.ins().brif(has_data, typed_store, &[], deopt, &[]);
    builder.switch_to_block(typed_store);
    let kind = builder.block_params(typed_common)[0];
    let is_float = builder.ins().icmp_imm(IntCC::NotEqual, kind, 0);
    let store_i32 = builder.create_block();
    let store_f64 = builder.create_block();
    builder.ins().brif(is_float, store_f64, &[], store_i32, &[]);
    builder.switch_to_block(store_i32);
    let offset = builder.ins().imul_imm(index, 4);
    let offset = builder.ins().uextend(pointer_type, offset);
    let address = builder.ins().iadd(data, offset);
    let scalar = builder.ins().ireduce(types::I32, value.payload);
    builder.ins().store(MemFlags::new(), scalar, address, 0);
    let cached_kind = builder.ins().iconst(types::I8, 1);
    builder
        .ins()
        .jump(continuation, &[count, data, cached_kind]);
    builder.switch_to_block(store_f64);
    let offset = builder.ins().imul_imm(index, 8);
    let offset = builder.ins().uextend(pointer_type, offset);
    let address = builder.ins().iadd(data, offset);
    let scalar = opt_f64(builder, value);
    builder.ins().store(MemFlags::new(), scalar, address, 0);
    let cached_kind = builder.ins().iconst(types::I8, 2);
    builder
        .ins()
        .jump(continuation, &[count, data, cached_kind]);

    builder.switch_to_block(deopt);
    for (slot, vars) in arguments.iter().enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, arg_buf, slot, current);
    }
    for (slot, vars) in locals.iter().enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, var_buf, slot, current);
    }
    for (slot, vars) in stack.iter().take(depth).enumerate() {
        let current = opt_use(builder, *vars);
        opt_store(builder, stack_base, slot, current);
    }
    opt_set_stack_top(builder, frame, stack_base, depth, pointer_type, layout);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        arguments.len() + locals.len(),
        stack_provenance,
        helper_signatures,
        pointer_type,
        layout,
    )?;
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );

    builder.switch_to_block(continuation);
    let params = builder.block_params(continuation);
    *guarded_source = Some(GuardedElementSource {
        provenance: source_provenance,
        block_pc,
        count: params[0],
        data: params[1],
        kind: params[2],
    });
    for provenance in &mut stack_provenance[object_index..depth] {
        *provenance = OptProvenance::Unknown;
    }
    Ok(object_index)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_guarded_property(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    store: bool,
    properties: &[crate::runtime::ShapeObservation],
    pc: u32,
    guard: u32,
    signature: cranelift_codegen::ir::SigRef,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;
    let object_index = depth
        .checked_sub(if store { 2 } else { 1 })
        .ok_or(CompileFailure::InvalidArtifact)?;
    for (index, vars) in arguments.iter().enumerate() {
        let v = opt_use(builder, *vars);
        opt_store(builder, arg_buf, index, v);
    }
    for (index, vars) in locals.iter().enumerate() {
        let v = opt_use(builder, *vars);
        opt_store(builder, var_buf, index, v);
    }
    for (index, vars) in stack.iter().take(depth).enumerate() {
        let v = opt_use(builder, *vars);
        opt_store(builder, stack_base, index, v);
    }
    let bytecode = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let current_pc = builder.ins().iadd_imm(bytecode, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), current_pc, frame, layout.pc);
    // SHAPE_GUARD validates its operand against stack_top. Materialize the
    // borrowed SSA aliases as real interpreter owners before exposing them to
    // the helper; incremental stack_top updates make DUP OOM cleanup exact.
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        arguments.len() + locals.len(),
        stack_provenance,
        helper_signatures,
        pointer_type,
        layout,
    )?;
    if properties.is_empty() || properties.len() > 3 {
        return Err(CompileFailure::InvalidArtifact);
    }
    let flat = arguments
        .len()
        .checked_add(locals.len())
        .and_then(|n| n.checked_add(object_index))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(CompileFailure::ResourceLimit)?;
    let api = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.runtime_api);
    let helper = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        api,
        layout.helper_offsets[qjs::JSJitHelperId_JS_JIT_HELPER_SHAPE_GUARD as usize],
    );
    let deopt = builder.create_block();
    let exception = builder.create_block();
    let continuation = builder.create_block();
    if !store {
        builder.append_block_param(continuation, types::I64);
        builder.append_block_param(continuation, types::I64);
    }
    for (index, property) in properties.iter().copied().enumerate() {
        let id = property.shape().identity();
        let generation = property.shape().generation();
        let params = [
            frame,
            builder.ins().iconst(types::I32, 0),
            builder.ins().iconst(types::I32, i64::from(flat)),
            builder.ins().iconst(types::I32, i64::from(id as u32)),
            builder
                .ins()
                .iconst(types::I32, i64::from((id >> 32) as u32)),
            builder
                .ins()
                .iconst(types::I32, i64::from(generation as u32)),
            builder
                .ins()
                .iconst(types::I32, i64::from((generation >> 32) as u32)),
        ];
        let call = super::emit_external_call(
            builder,
            signature,
            helper,
            &params,
            pointer_type,
            Some(frame),
            None,
        );
        let status = builder.inst_results(call)[0];
        let ok = builder
            .ins()
            .icmp_imm(IntCC::Equal, status, i64::from(qjs::JS_JIT_HELPER_OK));
        let access = builder.create_block();
        let miss_or_exception = builder.create_block();
        builder.ins().brif(ok, access, &[], miss_or_exception, &[]);
        builder.switch_to_block(miss_or_exception);
        let miss = builder.ins().icmp_imm(
            IntCC::Equal,
            status,
            i64::from(qjs::JS_JIT_HELPER_GUARD_MISS),
        );
        let next = if index + 1 == properties.len() {
            deopt
        } else {
            builder.create_block()
        };
        builder.ins().brif(miss, next, &[], exception, &[]);
        builder.switch_to_block(access);
        let object = opt_use(builder, stack[object_index]);
        // JSObject's 64-bit layout is: 24-byte GC/header+flags prefix,
        // `shape` at +24, then the JSProperty array pointer at +32.
        let props = builder
            .ins()
            .load(pointer_type, MemFlags::new(), object.payload, 32);
        let offset = i32::try_from(
            usize::try_from(property.offset())
                .map_err(|_| CompileFailure::ResourceLimit)?
                .checked_mul(16)
                .ok_or(CompileFailure::ResourceLimit)?,
        )
        .map_err(|_| CompileFailure::ResourceLimit)?;
        let current = OptPair {
            payload: builder
                .ins()
                .load(types::I64, MemFlags::new(), props, offset),
            tag: builder
                .ins()
                .load(types::I64, MemFlags::new(), props, offset + 8),
        };
        let expected_tag = match property.value() {
            crate::runtime::ObservedType::Int32 => qjs::JS_TAG_INT,
            crate::runtime::ObservedType::Float64 => qjs::JS_TAG_FLOAT64,
            crate::runtime::ObservedType::Bool => qjs::JS_TAG_BOOL,
            crate::runtime::ObservedType::Null => qjs::JS_TAG_NULL,
            crate::runtime::ObservedType::Undefined => qjs::JS_TAG_UNDEFINED,
            _ => return Err(CompileFailure::InvalidArtifact),
        };
        let mut tags_ok =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, current.tag, i64::from(expected_tag));
        if store {
            let value = opt_use(builder, stack[depth - 1]);
            let input_ok = builder
                .ins()
                .icmp_imm(IntCC::Equal, value.tag, i64::from(expected_tag));
            tags_ok = builder.ins().band(tags_ok, input_ok);
            let do_access = builder.create_block();
            builder.ins().brif(tags_ok, do_access, &[], deopt, &[]);
            builder.switch_to_block(do_access);
            opt_store_at(builder, props, offset, value);
            builder.ins().jump(continuation, &[]);
        } else {
            let do_access = builder.create_block();
            builder.ins().brif(tags_ok, do_access, &[], deopt, &[]);
            builder.switch_to_block(do_access);
            builder
                .ins()
                .jump(continuation, &[current.payload, current.tag]);
        }
        if index + 1 != properties.len() {
            builder.switch_to_block(next);
        }
    }
    builder.switch_to_block(exception);
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_EXCEPTION,
        None,
        pointer_type,
        0,
    );
    builder.switch_to_block(deopt);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    emit_opt_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );
    builder.switch_to_block(continuation);
    // Every borrowed alias below the operands was materialized as an owner
    // for the guard's exception path; hand those references back before the
    // operand slots are released, or each guarded access leaks one.
    opt_release_materialized_aliases(
        builder,
        frame,
        sret,
        stack_base,
        stack_provenance,
        0..object_index,
        depth,
        arguments.len() + locals.len(),
        helper_signatures,
        pointer_type,
        layout,
    )?;
    opt_release_owned_stack(
        builder,
        frame,
        sret,
        stack_base,
        object_index,
        depth,
        arguments.len() + locals.len(),
        helper_signatures,
        pointer_type,
        layout,
    )?;
    if store {
        for provenance in &mut stack_provenance[object_index..depth] {
            *provenance = OptProvenance::Unknown;
        }
        Ok(depth - 2)
    } else {
        let params = builder.block_params(continuation);
        let current = OptPair {
            payload: params[0],
            tag: params[1],
        };
        opt_define(builder, stack[object_index], current);
        stack_provenance[object_index] = OptProvenance::ImmediatePrimitive;
        opt_store(builder, stack_base, object_index, current);
        opt_set_stack_top(builder, frame, stack_base, depth, pointer_type, layout);
        Ok(depth)
    }
}

/// Releases the owned duplicates that `opt_own_stack_for_exit` created for
/// borrowed argument/local aliases *below* an operation's operands, once
/// that operation continues natively. The aliases keep their provenance:
/// the SSA value still borrows from the argument or local buffer, and the
/// interpreter slot must not own a reference that nobody consumes.
#[allow(clippy::too_many_arguments)]
fn opt_release_materialized_aliases(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    provenance: &[OptProvenance],
    range: core::ops::Range<usize>,
    exception_depth: usize,
    flat_stack_base: usize,
    signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<(), CompileFailure> {
    use rquickjs_core::qjs;
    for index in range {
        if !matches!(
            provenance.get(index),
            Some(OptProvenance::Argument(_) | OptProvenance::Local(_))
        ) {
            continue;
        }
        let slot = flat_stack_base
            .checked_add(index)
            .and_then(|slot| u32::try_from(slot).ok())
            .ok_or(CompileFailure::ResourceLimit)?;
        emit_opt_helper(
            builder,
            frame,
            sret,
            stack_base,
            exception_depth,
            signatures,
            qjs::JSJitHelperId_JS_JIT_HELPER_FREE as usize,
            &[0, slot],
            pointer_type,
            layout,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn opt_release_owned_stack(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    start: usize,
    end: usize,
    flat_stack_base: usize,
    signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<(), CompileFailure> {
    use rquickjs_core::qjs;
    for index in start..end {
        let slot = flat_stack_base
            .checked_add(index)
            .and_then(|slot| u32::try_from(slot).ok())
            .ok_or(CompileFailure::ResourceLimit)?;
        emit_opt_helper(
            builder,
            frame,
            sret,
            stack_base,
            end,
            signatures,
            qjs::JSJitHelperId_JS_JIT_HELPER_FREE as usize,
            &[0, slot],
            pointer_type,
            layout,
        )?;
    }
    opt_set_stack_top(builder, frame, stack_base, start, pointer_type, layout);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn opt_own_stack_for_exit(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    depth: usize,
    _flat_stack_base: usize,
    provenance: &[OptProvenance],
    signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<(), CompileFailure> {
    use cranelift_codegen::ir::{condcodes::IntCC, types, InstBuilder, MemFlags};
    const MATERIALIZE_OWNER_HELPER: usize =
        rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_MATERIALIZE_OWNER as usize;
    const MATERIALIZED: i64 = rquickjs_core::qjs::JS_JIT_HELPER_MATERIALIZED as i64;
    const SOURCE_ARGUMENT: u32 =
        rquickjs_core::qjs::JSJitOwnerSourceKind_JS_JIT_OWNER_SOURCE_ARGUMENT;
    const SOURCE_LOCAL: u32 = rquickjs_core::qjs::JSJitOwnerSourceKind_JS_JIT_OWNER_SOURCE_LOCAL;
    let undefined = OptPair {
        payload: builder.ins().iconst(types::I64, 0),
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(rquickjs_core::qjs::JS_TAG_UNDEFINED)),
    };
    opt_set_stack_top(builder, frame, stack_base, 0, pointer_type, layout);
    for index in 0..depth {
        if matches!(
            provenance.get(index),
            Some(OptProvenance::ImmediatePrimitive | OptProvenance::OwnedSlot)
        ) {
            opt_set_stack_top(builder, frame, stack_base, index + 1, pointer_type, layout);
            continue;
        }
        let (source_kind, source_index) = match provenance.get(index).copied() {
            Some(OptProvenance::Argument(slot)) => (SOURCE_ARGUMENT, slot),
            Some(OptProvenance::Local(slot)) => (SOURCE_LOCAL, slot),
            _ => return Err(CompileFailure::InvalidArtifact),
        };
        opt_store(builder, stack_base, index, undefined);
        let signature = *signatures
            .get(MATERIALIZE_OWNER_HELPER)
            .ok_or(CompileFailure::InvalidArtifact)?;
        let api = builder
            .ins()
            .load(pointer_type, MemFlags::new(), frame, layout.runtime_api);
        let helper = builder.ins().load(
            pointer_type,
            MemFlags::new(),
            api,
            layout.helper_offsets[MATERIALIZE_OWNER_HELPER],
        );
        let params = [
            frame,
            builder.ins().iconst(types::I32, 0),
            builder.ins().iconst(types::I32, index as i64),
            builder.ins().iconst(types::I32, i64::from(source_kind)),
            builder.ins().iconst(
                types::I32,
                i64::try_from(source_index).map_err(|_| CompileFailure::ResourceLimit)?,
            ),
        ];
        let call = super::emit_external_call(
            builder,
            signature,
            helper,
            &params,
            pointer_type,
            Some(frame),
            None,
        );
        let status = builder.inst_results(call)[0];
        let ok = builder.ins().icmp_imm(IntCC::Equal, status, MATERIALIZED);
        let continuation = builder.create_block();
        let exception = builder.create_block();
        builder.ins().brif(ok, continuation, &[], exception, &[]);
        builder.switch_to_block(exception);
        emit_opt_exit(
            builder,
            sret,
            rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_EXCEPTION,
            None,
            pointer_type,
            0,
        );
        builder.switch_to_block(continuation);
    }
    Ok(())
}
fn next_block_pc(ir: &OptimizedIr, pc: u32) -> Result<u32, CompileFailure> {
    ir.blocks()
        .iter()
        .position(|block| block.start_pc() == pc)
        .and_then(|index| ir.blocks().get(index + 1))
        .map(|block| block.start_pc())
        .ok_or(CompileFailure::InvalidArtifact)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_specialized_call(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arg_buf: cranelift_codegen::ir::Value,
    var_buf: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    stack: &[OptVars],
    stack_provenance: &mut [OptProvenance],
    depth: usize,
    argc: usize,
    has_this: bool,
    pc: u32,
    signatures: &[cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    direct: Option<&DirectCallSite>,
    guard: u32,
    scalar_result: bool,
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;

    let pop = argc + 1 + usize::from(has_this);
    let base = depth
        .checked_sub(pop)
        .ok_or(CompileFailure::InvalidArtifact)?;
    // Until ownership is represented on control-flow phis, keep the helper
    // bridge to the common expression-stack shape where CALL consumes the
    // complete live stack. This makes every temporary owner a contiguous
    // prefix that exception cleanup can describe exactly.
    if base != 0 {
        return Err(CompileFailure::InvalidArtifact);
    }
    let this_index = if has_this { base } else { depth };
    let function_index = if has_this { base + 1 } else { base };
    let argv_index = function_index + 1;
    let output_index = if has_this { depth } else { depth + 1 };
    if output_index >= stack.len() || (has_this && output_index + 1 >= stack.len()) {
        return Err(CompileFailure::ResourceLimit);
    }
    if let Some(direct) = direct.filter(|direct| {
        !has_this
            && direct.call.callee_identity() != 0
            && direct.call.callee_bytecode_identity() != 0
            && matches!(
                stack_provenance[function_index],
                OptProvenance::Argument(_) | OptProvenance::Local(_)
            )
    }) {
        use crate::runtime::FeedbackRepresentation;
        use cranelift_codegen::ir::condcodes::IntCC;
        use cranelift_codegen::ir::{AbiParam, Signature, StackSlotData, StackSlotKind};
        let function = opt_use(builder, stack[function_index]);
        let signature = builder.create_block();
        let invoke = builder.create_block();
        let deopt = builder.create_block();
        super::emit_guarded_direct_callee_identity(
            builder,
            function.tag,
            function.payload,
            super::DirectCalleeIdentity {
                object: direct.call.callee_identity(),
                bytecode: direct.call.callee_bytecode_identity(),
            },
            pointer_type,
            signature,
            deopt,
        );
        builder.switch_to_block(signature);
        let mut matches = builder.ins().iconst(types::I8, 1);
        for (index, representation) in direct.call.arguments().iter().enumerate() {
            let value = opt_use(builder, stack[argv_index + index]);
            let tag = match representation {
                FeedbackRepresentation::Int32 => qjs::JS_TAG_INT,
                FeedbackRepresentation::Float64 => qjs::JS_TAG_FLOAT64,
                FeedbackRepresentation::HeapRef => unreachable!("direct calls are scalar-only"),
            };
            let typed = builder
                .ins()
                .icmp_imm(IntCC::Equal, value.tag, i64::from(tag));
            matches = builder.ins().band(matches, typed);
        }
        builder.ins().brif(matches, invoke, &[], deopt, &[]);
        builder.switch_to_block(deopt);
        for (index, vars) in arguments.iter().enumerate() {
            let value = opt_use(builder, *vars);
            opt_store(builder, arg_buf, index, value);
        }
        for (index, vars) in locals.iter().enumerate() {
            let value = opt_use(builder, *vars);
            opt_store(builder, var_buf, index, value);
        }
        for (index, vars) in stack.iter().take(depth).enumerate() {
            let value = opt_use(builder, *vars);
            opt_store(builder, stack_base, index, value);
        }
        opt_set_stack_top(builder, frame, stack_base, 0, pointer_type, layout);
        let start = builder
            .ins()
            .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
        let resume = builder.ins().iadd_imm(start, i64::from(pc));
        builder
            .ins()
            .store(MemFlags::new(), resume, frame, layout.pc);
        opt_own_stack_for_exit(
            builder,
            frame,
            sret,
            stack_base,
            depth,
            arguments.len() + locals.len(),
            stack_provenance,
            signatures,
            pointer_type,
            layout,
        )?;
        emit_opt_exit(
            builder,
            sret,
            qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
            Some(resume),
            pointer_type,
            guard,
        );
        builder.switch_to_block(invoke);
        let scalar = match direct.call.result() {
            FeedbackRepresentation::Int32 => types::I32,
            FeedbackRepresentation::Float64 => types::F64,
            FeedbackRepresentation::HeapRef => unreachable!("direct calls are scalar-only"),
        };
        let mut signature = Signature::new(builder.func.signature.call_conv);
        signature.params.push(AbiParam::new(pointer_type));
        for argument in direct.call.arguments() {
            signature.params.push(AbiParam::new(match argument {
                FeedbackRepresentation::Int32 => types::I32,
                FeedbackRepresentation::Float64 => types::F64,
                FeedbackRepresentation::HeapRef => unreachable!("direct calls are scalar-only"),
            }));
        }
        signature.returns.push(AbiParam::new(types::I32));
        let signature = builder.import_signature(signature);
        let target = builder.ins().iconst(pointer_type, direct.entry as i64);
        let output = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            scalar.bytes(),
            0,
        ));
        let output_address = builder.ins().stack_addr(pointer_type, output, 0);
        let mut params = Vec::with_capacity(argc + 1);
        params.push(output_address);
        for (index, representation) in direct.call.arguments().iter().enumerate() {
            let value = opt_use(builder, stack[argv_index + index]);
            params.push(match representation {
                FeedbackRepresentation::Int32 => builder.ins().ireduce(types::I32, value.payload),
                FeedbackRepresentation::Float64 => {
                    builder
                        .ins()
                        .bitcast(types::F64, MemFlags::new(), value.payload)
                }
                FeedbackRepresentation::HeapRef => unreachable!("direct calls are scalar-only"),
            });
        }
        let call = super::emit_external_call(
            builder,
            signature,
            target,
            &params,
            pointer_type,
            None,
            None,
        );
        let results = builder.inst_results(call);
        let status = results[0];
        let success = builder.ins().icmp_imm(IntCC::Equal, status, 0);
        let done = builder.create_block();
        builder.ins().brif(success, done, &[], deopt, &[]);
        builder.switch_to_block(done);
        let raw_result = builder
            .ins()
            .load(scalar, MemFlags::new(), output_address, 0);
        let result = match direct.call.result() {
            FeedbackRepresentation::Int32 => OptPair {
                payload: builder.ins().sextend(types::I64, raw_result),
                tag: builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
            },
            FeedbackRepresentation::Float64 => OptPair {
                payload: builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), raw_result),
                tag: builder
                    .ins()
                    .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64)),
            },
            FeedbackRepresentation::HeapRef => unreachable!("direct calls are scalar-only"),
        };
        opt_define(builder, stack[base], result);
        stack_provenance[base] = OptProvenance::ImmediatePrimitive;
        return Ok(base + 1);
    }
    let undefined = OptPair {
        payload: builder.ins().iconst(types::I64, 0),
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(qjs::JS_TAG_UNDEFINED)),
    };
    let mut call_ownership = vec![crate::ir::SsaValueOwnership::Borrowed; pop];
    if !has_this {
        opt_define(builder, stack[this_index], undefined);
    }
    opt_define(builder, stack[output_index], undefined);
    for (index, vars) in arguments.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, arg_buf, index, value);
    }
    for (index, vars) in locals.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, var_buf, index, value);
    }
    for (index, vars) in stack.iter().take(output_index + 1).enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, stack_base, index, value);
    }
    opt_set_stack_top(builder, frame, stack_base, 0, pointer_type, layout);
    let bytecode = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let current_pc = builder.ins().iadd_imm(bytecode, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), current_pc, frame, layout.pc);
    let flat_base = arguments.len() + locals.len();
    let slot = |index: usize| -> Result<u32, CompileFailure> {
        flat_base
            .checked_add(index)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(CompileFailure::ResourceLimit)
    };
    opt_own_stack_for_exit(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        flat_base,
        stack_provenance,
        signatures,
        pointer_type,
        layout,
    )?;
    for owner in &mut call_ownership {
        *owner = owner
            .duplicate()
            .map_err(|_| CompileFailure::InvalidArtifact)?;
    }
    opt_set_stack_top(
        builder,
        frame,
        stack_base,
        output_index + 1,
        pointer_type,
        layout,
    );
    emit_opt_helper(
        builder,
        frame,
        sret,
        stack_base,
        depth,
        signatures,
        qjs::JSJitHelperId_JS_JIT_HELPER_CALL as usize,
        &[
            0,
            slot(output_index)?,
            slot(function_index)?,
            slot(this_index)?,
            if argc == 0 {
                u32::MAX
            } else {
                slot(argv_index)?
            },
            u32::try_from(argc).map_err(|_| CompileFailure::ResourceLimit)?,
        ],
        pointer_type,
        layout,
    )?;
    let result = opt_load(builder, stack_base, output_index);
    let displaced_index = if has_this {
        output_index + 1
    } else {
        this_index
    };
    let displaced = opt_use(builder, stack[base]);
    opt_define(builder, stack[displaced_index], displaced);
    opt_store(builder, stack_base, displaced_index, displaced);
    opt_define(builder, stack[base], result);
    opt_store(builder, stack_base, base, result);
    let free_depth = core::cmp::max(displaced_index, output_index) + 1;
    opt_set_stack_top(builder, frame, stack_base, free_depth, pointer_type, layout);
    for index in core::iter::once(displaced_index).chain((base + 1)..(base + pop)) {
        let input = if index == displaced_index {
            0
        } else {
            index - base
        };
        call_ownership[input]
            .consume()
            .map_err(|_| CompileFailure::InvalidArtifact)?;
        emit_opt_helper(
            builder,
            frame,
            sret,
            stack_base,
            depth + 2,
            signatures,
            qjs::JSJitHelperId_JS_JIT_HELPER_FREE as usize,
            &[0, slot(index)?],
            pointer_type,
            layout,
        )?;
        let value = opt_load(builder, stack_base, index);
        opt_define(builder, stack[index], value);
    }
    for (index, &stack_slot) in stack
        .iter()
        .enumerate()
        .take(output_index + 1)
        .skip(base + 1)
    {
        opt_define(builder, stack_slot, undefined);
        opt_store(builder, stack_base, index, undefined);
    }
    // The CALL bridge materialized every borrowed alias below the callee as
    // an owner for its exception path; release those duplicates now that
    // the call continued natively.
    opt_release_materialized_aliases(
        builder,
        frame,
        sret,
        stack_base,
        stack_provenance,
        0..base,
        free_depth,
        flat_base,
        signatures,
        pointer_type,
        layout,
    )?;
    // Feedback-specialized results are scalars; anything else stays owned
    // by the interpreter stack slot the CALL helper wrote it to.
    stack_provenance[base] = if scalar_result {
        OptProvenance::ImmediatePrimitive
    } else {
        OptProvenance::OwnedSlot
    };
    for provenance in &mut stack_provenance[(base + 1)..=output_index] {
        *provenance = OptProvenance::Unknown;
    }
    opt_set_stack_top(builder, frame, stack_base, base + 1, pointer_type, layout);
    Ok(base + 1)
}

/// Locals that ever receive a value owned by its interpreter stack slot
/// (helper results: global lookups, kept-receiver property loads and
/// non-specialized call results). Stores into such a local must first
/// release whatever it currently owns, exactly like the interpreter's
/// `set_value`. The walk mirrors the lowering's linear stack model, so the
/// set over-approximates every path; releasing a primitive is a no-op.
fn owned_local_targets(
    ir: &OptimizedIr,
    specialization: &NumericSpecialization,
) -> Result<Vec<bool>, CompileFailure> {
    let Some(entry) = ir.guard_maps().first() else {
        return Err(CompileFailure::InvalidArtifact);
    };
    let mut owned_locals = vec![false; usize::from(entry.shape().locals())];
    let mut owned = vec![false; usize::from(ir.max_stack()) + crate::ir::MAX_HELPER_SCRATCH_SLOTS];
    for block in ir.blocks() {
        let mut depth = usize::from(block.stack_depth());
        for node_id in block.nodes() {
            let node = ir
                .nodes()
                .get(*node_id as usize)
                .ok_or(CompileFailure::InvalidArtifact)?;
            let name = match node.kind() {
                crate::ir::OptimizedNodeKind::Bytecode { opcode } => opcode.as_ref(),
                _ => "",
            };
            let pops = usize::from(node.pops());
            let pushes = usize::from(node.pushes());
            let base = depth
                .checked_sub(pops)
                .ok_or(CompileFailure::InvalidArtifact)?;
            if base + pushes > owned.len() {
                return Err(CompileFailure::ResourceLimit);
            }
            let top_owned = pops > 0 && owned[depth - 1];
            let produces_owned = match name {
                "get_var" => true,
                "get_field2" => true,
                n if n.starts_with("call") => !specialization.calls.contains_key(&node.pc()),
                _ => false,
            };
            if top_owned {
                let target = if name.starts_with("put_loc") {
                    opt_index(name, node.bytes(), "put_loc")?
                } else if name.starts_with("set_loc") && name != "set_loc_uninitialized" {
                    opt_index(name, node.bytes(), "set_loc")?
                } else {
                    None
                };
                if let Some(slot) = target.and_then(|local| owned_locals.get_mut(local)) {
                    *slot = true;
                }
            }
            match name {
                "get_field2" => {
                    // The receiver stays; the loaded property is owned.
                    owned[base + 1] = true;
                }
                _ => {
                    for slot in &mut owned[base..base + pushes] {
                        *slot = produces_owned;
                    }
                }
            }
            depth = base + pushes;
        }
    }
    Ok(owned_locals)
}

fn opt_u32(bytes: &[u8]) -> Result<u32, CompileFailure> {
    bytes
        .get(1..5)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(CompileFailure::InvalidArtifact)
}

/// Flat frame slot index (arguments, locals, then stack) of a stack slot.
fn opt_flat_stack_slot(env: &OptEnv<'_>, index: usize) -> Result<u32, CompileFailure> {
    env.arguments
        .len()
        .checked_add(env.locals.len())
        .and_then(|base| base.checked_add(index))
        .and_then(|slot| u32::try_from(slot).ok())
        .ok_or(CompileFailure::ResourceLimit)
}

fn opt_flat_local_slot(env: &OptEnv<'_>, index: usize) -> Result<u32, CompileFailure> {
    env.arguments
        .len()
        .checked_add(index)
        .and_then(|slot| u32::try_from(slot).ok())
        .ok_or(CompileFailure::ResourceLimit)
}

/// Local index written or read by a `get_loc*`/`put_loc*`/`set_loc*`
/// family opcode name, or `None` for other opcodes.
fn opt_local_slot(name: &str, bytes: &[u8]) -> Option<usize> {
    for prefix in [
        "get_loc", "put_loc", "set_loc", "inc_loc", "dec_loc", "add_loc",
    ] {
        if name == "get_loc0_loc1" || name == "set_loc_uninitialized" {
            return None;
        }
        if name.starts_with(prefix) {
            if let Ok(Some(index)) = opt_index(name, bytes, prefix) {
                return Some(index);
            }
            return opt_u16(bytes).ok();
        }
    }
    None
}

/// Increments whose Int32 result provably cannot overflow, so the raw-i32
/// loop shape may add without an overflow exit.
///
/// The proof is the canonical counted loop: inside a natural loop whose
/// header block ends with `get_loc k; <Int32 operand>; lt; if_false -> exit`,
/// the sequence `get_loc k; inc|post_inc; put_loc k` is the only write to
/// local `k` anywhere in the loop. Then `k < X <= INT32_MAX` holds at the
/// increment on every iteration, so `k + 1` fits. Every value in the raw-i32
/// shape is Int32 by its entry and header guards, which is what makes the
/// header comparison a numeric bound.
fn provably_bounded_increments(ir: &OptimizedIr) -> std::collections::BTreeSet<u32> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut bounded = BTreeSet::new();
    let blocks = ir.blocks();
    let index_of: BTreeMap<u32, usize> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.start_pc(), index))
        .collect();
    let mut predecessors: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for block in blocks {
        for successor in block.successors() {
            predecessors
                .entry(*successor)
                .or_default()
                .push(block.start_pc());
        }
    }
    fn name_of(node: &crate::ir::OptimizedNode) -> Option<&str> {
        match node.kind() {
            crate::ir::OptimizedNodeKind::Bytecode { opcode } => Some(opcode.as_ref()),
            _ => None,
        }
    }
    for header in blocks.iter().filter(|block| block.is_loop_header()) {
        // Natural loop: the header plus everything that reaches a latch
        // without passing through the header.
        let latches: Vec<u32> = blocks
            .iter()
            .filter(|block| {
                block.start_pc() >= header.start_pc()
                    && block.successors().contains(&header.start_pc())
            })
            .map(|block| block.start_pc())
            .collect();
        let mut members = BTreeSet::from([header.start_pc()]);
        let mut pending = latches;
        while let Some(pc) = pending.pop() {
            if members.insert(pc) {
                if let Some(preds) = predecessors.get(&pc) {
                    pending.extend(preds.iter().copied());
                }
            }
        }
        // Header pattern: `get_loc k; operand; lt; if_false -> outside`.
        let header_nodes: Vec<&crate::ir::OptimizedNode> = header
            .nodes()
            .iter()
            .filter_map(|id| ir.nodes().get(*id as usize))
            .filter(|node| name_of(node).is_some() && !node.eliminated())
            .collect();
        let count = header_nodes.len();
        if count < 4 {
            continue;
        }
        let [load, operand, compare, branch] = [
            header_nodes[count - 4],
            header_nodes[count - 3],
            header_nodes[count - 2],
            header_nodes[count - 1],
        ];
        let Some(counter) = name_of(load).and_then(|name| {
            name.starts_with("get_loc")
                .then(|| opt_local_slot(name, load.bytes()))
                .flatten()
        }) else {
            continue;
        };
        let operand_ok = name_of(operand).is_some_and(|name| {
            name.starts_with("get_loc") || name.starts_with("get_arg") || name.starts_with("push_")
        });
        let exits_loop = name_of(branch).is_some_and(|name| name.starts_with("if_false"))
            && branch
                .branch_target()
                .is_some_and(|target| !members.contains(&target));
        if !operand_ok || name_of(compare) != Some("lt") || !exits_loop {
            continue;
        }
        // Every write to `counter` inside the loop must be the increment's
        // own `get_loc k; inc|post_inc; put_loc k` store.
        let mut increments = Vec::new();
        let mut foreign_write = false;
        for pc in &members {
            let Some(block) = index_of.get(pc).and_then(|index| blocks.get(*index)) else {
                continue;
            };
            let nodes: Vec<&crate::ir::OptimizedNode> = block
                .nodes()
                .iter()
                .filter_map(|id| ir.nodes().get(*id as usize))
                .filter(|node| name_of(node).is_some())
                .collect();
            for (position, node) in nodes.iter().enumerate() {
                let name = name_of(node).unwrap_or_default();
                let writes_counter = (name.starts_with("put_loc")
                    || name.starts_with("set_loc")
                    || name.starts_with("inc_loc")
                    || name.starts_with("dec_loc")
                    || name.starts_with("add_loc"))
                    && opt_local_slot(name, node.bytes()) == Some(counter);
                if !writes_counter {
                    continue;
                }
                let from_increment = name.starts_with("put_loc")
                    && position >= 2
                    && matches!(name_of(nodes[position - 1]), Some("inc" | "post_inc"))
                    && name_of(nodes[position - 2]).is_some_and(|name| {
                        name.starts_with("get_loc")
                            && opt_local_slot(name, nodes[position - 2].bytes()) == Some(counter)
                    });
                if from_increment {
                    increments.push(nodes[position - 1].pc());
                } else {
                    foreign_write = true;
                }
            }
        }
        if !foreign_write && increments.len() == 1 {
            bounded.extend(increments);
        }
    }
    bounded
}

/// How a value may be stored into an interpreter-owned argument or local
/// buffer. Tier 2 spills borrowed SSA aliases into those buffers at every
/// exit without transferring ownership (deoptimization maps carry identity
/// recipes only), so an alias that is a heap reference must never be copied
/// into a *different* slot: the interpreter would inherit an unowned pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AliasStore {
    /// Proven scalar, slot-owned (moved by the caller), or the slot's own value.
    Safe,
    /// Unproven representation: check the tag at run time and deoptimize
    /// before the store when the value is reference counted.
    GuardHeap,
    /// Proven or unknowable heap alias: fail closed.
    Reject,
}

fn opt_alias_store(
    specialization: &NumericSpecialization,
    argument_count: usize,
    source: OptProvenance,
    destination: OptProvenance,
) -> AliasStore {
    if source == destination {
        return AliasStore::Safe;
    }
    let representation = |slot: usize| {
        specialization
            .arguments
            .get(slot)
            .copied()
            .unwrap_or(specialization.entry)
    };
    let classify = |representation: EntryRepresentation| match representation {
        EntryRepresentation::Numeric
        | EntryRepresentation::Int32
        | EntryRepresentation::Float64 => AliasStore::Safe,
        EntryRepresentation::Any => AliasStore::GuardHeap,
        EntryRepresentation::HeapRef => AliasStore::Reject,
    };
    match source {
        OptProvenance::ImmediatePrimitive | OptProvenance::OwnedSlot => AliasStore::Safe,
        OptProvenance::Argument(slot) => classify(representation(slot)),
        OptProvenance::Local(slot) => classify(representation(argument_count + slot)),
        OptProvenance::Unknown => AliasStore::Reject,
    }
}

/// Applies [`opt_alias_store`] for the value at stack index `source_index`
/// before it is stored into `destination`; `depth` is the operand-stack
/// depth before the storing instruction pops anything.
#[allow(clippy::too_many_arguments)]
fn emit_opt_alias_store_guard(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    specialization: &NumericSpecialization,
    provenance: &[OptProvenance],
    depth: usize,
    source_index: usize,
    destination: OptProvenance,
    pc: u32,
    guard: Option<u32>,
) -> Result<(), CompileFailure> {
    use cranelift_codegen::ir::{condcodes::IntCC, InstBuilder};
    if env.int32_loop {
        return Ok(());
    }
    match opt_alias_store(
        specialization,
        env.arguments.len(),
        provenance[source_index],
        destination,
    ) {
        AliasStore::Safe => Ok(()),
        AliasStore::Reject => Err(CompileFailure::UnsupportedOpcode),
        AliasStore::GuardHeap => {
            // QuickJS reference-counted tags are the negative ones.
            let pair = opt_use(builder, env.stack[source_index]);
            let not_refcounted =
                builder
                    .ins()
                    .icmp_imm(IntCC::SignedGreaterThanOrEqual, pair.tag, 0);
            let guard = guard.ok_or(CompileFailure::InvalidArtifact)?;
            emit_opt_guard_branch(builder, env, provenance, depth, pc, guard, not_refcounted)?;
            Ok(())
        }
    }
}

/// Compilation fails closed when an opcode would copy or silently discard a
/// value owned by its interpreter stack slot.
fn opt_reject_owned(
    provenance: &[OptProvenance],
    range: core::ops::Range<usize>,
) -> Result<(), CompileFailure> {
    if provenance[range].contains(&OptProvenance::OwnedSlot) {
        return Err(CompileFailure::UnsupportedOpcode);
    }
    Ok(())
}

/// Releases the value an interpreter stack slot owns (the slot must hold
/// the current SSA value, which every owned slot does by construction).
fn emit_opt_free_stack_slot(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    index: usize,
) -> Result<(), CompileFailure> {
    let pair = opt_use(builder, env.stack[index]);
    opt_store(builder, env.stack_base, index, pair);
    opt_set_stack_top(
        builder,
        env.frame,
        env.stack_base,
        index + 1,
        env.pointer_type,
        env.layout,
    );
    emit_opt_helper(
        builder,
        env.frame,
        env.sret,
        env.stack_base,
        index + 1,
        env.helper_signatures,
        rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_FREE as usize,
        &[0, opt_flat_stack_slot(env, index)?],
        env.pointer_type,
        env.layout,
    )
}

/// Releases the value a local owns before it is redefined.
fn emit_opt_free_local_slot(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    index: usize,
) -> Result<(), CompileFailure> {
    if env.int32_loop {
        return Err(CompileFailure::UnsupportedOpcode);
    }
    let pair = opt_use(builder, env.locals[index]);
    opt_store(builder, env.var_buf, index, pair);
    emit_opt_helper(
        builder,
        env.frame,
        env.sret,
        env.stack_base,
        0,
        env.helper_signatures,
        rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_FREE as usize,
        &[0, opt_flat_local_slot(env, index)?],
        env.pointer_type,
        env.layout,
    )
}

/// Invokes a helper that writes an owned value into the pushed stack slot
/// (GET_GLOBAL, GET_PROPERTY with the receiver kept). Every live stack slot
/// is turned into a real interpreter owner first, exactly as the CALL bridge
/// does, so an exception unwinds the frame correctly and the helper sees a
/// consistent stack; `helper_arguments` follow the output slot.
fn emit_opt_owned_helper_push(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    env: &OptEnv<'_>,
    provenance: &mut [OptProvenance],
    depth: usize,
    pc: u32,
    helper_id: usize,
    helper_arguments: &[u32],
) -> Result<usize, CompileFailure> {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    use rquickjs_core::qjs;
    if env.int32_loop {
        return Err(CompileFailure::UnsupportedOpcode);
    }
    if depth >= env.stack.len() {
        return Err(CompileFailure::ResourceLimit);
    }
    let undefined = OptPair {
        payload: builder.ins().iconst(types::I64, 0),
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(qjs::JS_TAG_UNDEFINED)),
    };
    opt_define(builder, env.stack[depth], undefined);
    for (index, vars) in env.arguments.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.arg_buf, index, value);
    }
    for (index, vars) in env.locals.iter().enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.var_buf, index, value);
    }
    for (index, vars) in env.stack.iter().take(depth + 1).enumerate() {
        let value = opt_use(builder, *vars);
        opt_store(builder, env.stack_base, index, value);
    }
    let bytecode = builder.ins().load(
        env.pointer_type,
        MemFlags::new(),
        env.frame,
        env.layout.bytecode_start,
    );
    let current_pc = builder.ins().iadd_imm(bytecode, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), current_pc, env.frame, env.layout.pc);
    opt_own_stack_for_exit(
        builder,
        env.frame,
        env.sret,
        env.stack_base,
        depth,
        env.arguments.len() + env.locals.len(),
        provenance,
        env.helper_signatures,
        env.pointer_type,
        env.layout,
    )?;
    for slot in provenance.iter_mut().take(depth) {
        if matches!(slot, OptProvenance::Argument(_) | OptProvenance::Local(_)) {
            *slot = OptProvenance::OwnedSlot;
        }
    }
    opt_set_stack_top(
        builder,
        env.frame,
        env.stack_base,
        depth + 1,
        env.pointer_type,
        env.layout,
    );
    let mut arguments = Vec::with_capacity(helper_arguments.len() + 2);
    arguments.push(0);
    arguments.push(opt_flat_stack_slot(env, depth)?);
    arguments.extend_from_slice(helper_arguments);
    emit_opt_helper(
        builder,
        env.frame,
        env.sret,
        env.stack_base,
        depth,
        env.helper_signatures,
        helper_id,
        &arguments,
        env.pointer_type,
        env.layout,
    )?;
    let result = opt_load(builder, env.stack_base, depth);
    opt_define(builder, env.stack[depth], result);
    provenance[depth] = OptProvenance::OwnedSlot;
    Ok(depth + 1)
}

#[allow(clippy::too_many_arguments)]
fn emit_opt_helper(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    exception_depth: usize,
    signatures: &[cranelift_codegen::ir::SigRef],
    helper_id: usize,
    arguments: &[u32],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) -> Result<(), CompileFailure> {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let signature = *signatures
        .get(helper_id)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let api = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.runtime_api);
    let helper = builder.ins().load(
        pointer_type,
        MemFlags::new(),
        api,
        layout.helper_offsets[helper_id],
    );
    let mut params = Vec::with_capacity(arguments.len() + 1);
    params.push(frame);
    params.extend(
        arguments
            .iter()
            .map(|value| builder.ins().iconst(types::I32, i64::from(*value))),
    );
    let call = super::emit_external_call(
        builder,
        signature,
        helper,
        &params,
        pointer_type,
        Some(frame),
        None,
    );
    let status = builder.inst_results(call)[0];
    let succeeded = builder.ins().icmp_imm(IntCC::Equal, status, 0);
    let continuation = builder.create_block();
    let exception = builder.create_block();
    builder
        .ins()
        .brif(succeeded, continuation, &[], exception, &[]);
    builder.switch_to_block(exception);
    opt_set_stack_top(
        builder,
        frame,
        stack_base,
        exception_depth,
        pointer_type,
        layout,
    );
    emit_opt_exit(
        builder,
        sret,
        rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_EXCEPTION,
        None,
        pointer_type,
        0,
    );
    builder.switch_to_block(continuation);
    Ok(())
}

fn opt_set_stack_top(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    stack_base: cranelift_codegen::ir::Value,
    depth: usize,
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
) {
    use cranelift_codegen::ir::{InstBuilder, MemFlags};
    let top = builder.ins().iadd_imm(
        stack_base,
        i64::try_from(depth * 16).expect("verified frame"),
    );
    builder
        .ins()
        .store(MemFlags::new(), top, frame, layout.stack_top);
    let _ = pointer_type;
}
fn emit_opt_exit(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    sret: cranelift_codegen::ir::Value,
    kind: u32,
    resume: Option<cranelift_codegen::ir::Value>,
    pointer_type: cranelift_codegen::ir::Type,
    guard: u32,
) {
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let zero = builder.ins().iconst(pointer_type, 0);
    let map_identity = if kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT {
        guard.saturating_add(1)
    } else {
        0
    };
    let kind = builder.ins().iconst(types::I32, i64::from(kind));
    let map = builder.ins().iconst(types::I32, i64::from(map_identity));
    builder.ins().store(MemFlags::new(), kind, sret, 0);
    builder.ins().store(MemFlags::new(), map, sret, 4);
    builder
        .ins()
        .store(MemFlags::new(), resume.unwrap_or(zero), sret, 8);
    builder.ins().store(MemFlags::new(), zero, sret, 16);
    builder.ins().return_(&[]);
}
#[allow(clippy::too_many_arguments)]
fn emit_opt_numeric_guard(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    arguments: &[OptVars],
    locals: &[OptVars],
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    guard: u32,
    pc: u32,
    pass: cranelift_codegen::ir::Block,
    side_path: Option<crate::runtime::SidePathProfile>,
    representation: EntryRepresentation,
    argument_representations: &[EntryRepresentation],
) {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let mut numeric = builder.ins().iconst(types::I8, 1);
    let mut alternate_numeric = builder.ins().iconst(types::I8, 1);
    let mut alternate_seen = builder.ins().iconst(types::I8, 0);
    for (index, vars) in arguments.iter().chain(locals).enumerate() {
        let pair = opt_use(builder, *vars);
        let int = builder.ins().icmp_imm(
            IntCC::Equal,
            pair.tag,
            i64::from(rquickjs_core::qjs::JS_TAG_INT),
        );
        let float = builder.ins().icmp_imm(
            IntCC::Equal,
            pair.tag,
            i64::from(rquickjs_core::qjs::JS_TAG_FLOAT64),
        );
        let required = argument_representations
            .get(index)
            .copied()
            .unwrap_or(representation);
        let valid = match required {
            EntryRepresentation::Any => builder.ins().iconst(types::I8, 1),
            EntryRepresentation::Numeric if index >= arguments.len() => {
                // A `let` declared inside the loop body is still
                // uninitialized or undefined at the header. Every numeric
                // consumer guards its operand tags itself, so such locals
                // need no proof here.
                let numeric = builder.ins().bor(int, float);
                let undefined = builder.ins().icmp_imm(
                    IntCC::Equal,
                    pair.tag,
                    i64::from(rquickjs_core::qjs::JS_TAG_UNDEFINED),
                );
                let uninitialized = builder.ins().icmp_imm(
                    IntCC::Equal,
                    pair.tag,
                    i64::from(rquickjs_core::qjs::JS_TAG_UNINITIALIZED),
                );
                let unset = builder.ins().bor(undefined, uninitialized);
                builder.ins().bor(numeric, unset)
            }
            EntryRepresentation::Numeric => builder.ins().bor(int, float),
            EntryRepresentation::Int32 => int,
            EntryRepresentation::Float64 => float,
            EntryRepresentation::HeapRef => builder.ins().icmp_imm(
                IntCC::Equal,
                pair.tag,
                i64::from(rquickjs_core::qjs::JS_TAG_OBJECT),
            ),
        };
        numeric = builder.ins().band(numeric, valid);
        let either_numeric = builder.ins().bor(int, float);
        alternate_numeric = builder.ins().band(alternate_numeric, either_numeric);
        alternate_seen = builder.ins().bor(alternate_seen, float);
    }
    let deopt = builder.create_block();
    if side_path.is_some_and(|profile| profile.observed() == crate::runtime::ObservedType::Float64)
    {
        let side_check = builder.create_block();
        let side_block = builder.create_block();
        builder.ins().brif(numeric, pass, &[], side_check, &[]);
        builder.switch_to_block(side_check);
        let matches_profile = builder.ins().band(alternate_numeric, alternate_seen);
        builder
            .ins()
            .brif(matches_profile, side_block, &[], deopt, &[]);
        builder.switch_to_block(side_block);
        let flags = builder
            .ins()
            .load(types::I32, MemFlags::new(), frame, layout.flags);
        let flags = builder.ins().bor_imm(
            flags,
            i64::from(rquickjs_core::qjs::JS_JIT_FRAME_SIDE_PATH_HIT),
        );
        builder
            .ins()
            .store(MemFlags::new(), flags, frame, layout.flags);
        builder.ins().jump(pass, &[]);
    } else {
        builder.ins().brif(numeric, pass, &[], deopt, &[]);
    }
    builder.switch_to_block(deopt);
    let start = builder
        .ins()
        .load(pointer_type, MemFlags::new(), frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    builder
        .ins()
        .store(MemFlags::new(), resume, frame, layout.pc);
    emit_opt_exit(
        builder,
        sret,
        rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT,
        Some(resume),
        pointer_type,
        guard,
    );
}

fn emit_opt_poll(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    signature: cranelift_codegen::ir::SigRef,
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    pc: u32,
) {
    use cranelift_codegen::ir::{condcodes::IntCC, InstBuilder, MemFlags};
    let flags = MemFlags::new();
    let api = builder
        .ins()
        .load(pointer_type, flags, frame, layout.runtime_api);
    let poll = builder.ins().load(
        pointer_type,
        flags,
        api,
        layout.helper_offsets[rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_POLL as usize],
    );
    let call = super::emit_external_call(
        builder,
        signature,
        poll,
        &[frame],
        pointer_type,
        Some(frame),
        None,
    );
    let interrupted = builder.inst_results(call)[0];
    let interrupted = builder.ins().icmp_imm(IntCC::NotEqual, interrupted, 0);
    let interrupt = builder.create_block();
    let continuation = builder.create_block();
    builder
        .ins()
        .brif(interrupted, interrupt, &[], continuation, &[]);
    builder.switch_to_block(interrupt);
    let start = builder
        .ins()
        .load(pointer_type, flags, frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(start, i64::from(pc));
    emit_opt_exit(
        builder,
        sret,
        rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_INTERRUPT,
        Some(resume),
        pointer_type,
        0,
    );
    builder.switch_to_block(continuation);
}

#[allow(clippy::too_many_arguments)] // Poll ABI parameters mirror the generated helper signature.
fn emit_opt_amortized_poll(
    builder: &mut cranelift_frontend::FunctionBuilder<'_>,
    frame: cranelift_codegen::ir::Value,
    sret: cranelift_codegen::ir::Value,
    signature: cranelift_codegen::ir::SigRef,
    pointer_type: cranelift_codegen::ir::Type,
    layout: super::helpers::FrameLayout,
    pc: u32,
    budget: cranelift_frontend::Variable,
) {
    use cranelift_codegen::ir::{condcodes::IntCC, types, InstBuilder};
    let remaining = builder.use_var(budget);
    let remaining = builder.ins().iadd_imm(remaining, -1);
    builder.def_var(budget, remaining);
    let due = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
    let poll = builder.create_block();
    let continuation = builder.create_block();
    builder.ins().brif(due, poll, &[], continuation, &[]);
    builder.switch_to_block(poll);
    emit_opt_poll(builder, frame, sret, signature, pointer_type, layout, pc);
    let reset = builder.ins().iconst(types::I64, 64);
    builder.def_var(budget, reset);
    builder.ins().jump(continuation, &[]);
    builder.switch_to_block(continuation);
}

impl Tier2Compiler {
    pub fn host(feedback_epoch: u64) -> Self {
        use cranelift_codegen::settings::{self, Configurable};
        let mut settings = settings::builder();
        settings
            .set("opt_level", "speed")
            .expect("Cranelift opt_level setting");
        let isa = cranelift_native::builder()
            .expect("host architecture is supported by Cranelift")
            .finish(settings::Flags::new(settings))
            .expect("host ISA settings are valid");
        Self {
            isa,
            feedback_epoch,
        }
    }

    #[cfg(feature = "test-support")]
    pub fn lower_for_test(
        &self,
        function: &VerifiedFunction,
        epoch: u64,
    ) -> Result<String, CompileFailure> {
        let ir = OptimizedIr::translate(function, epoch)?;
        lower_optimized_machine(
            &self.isa,
            &ir,
            None,
            None,
            &NumericSpecialization::default(),
        )
        .map(|code| code.clif().to_owned())
    }

    /// Compiles and publishes the feedback-free Tier 2 machine code for a
    /// verified function so tests can execute it on a synthetic frame and
    /// observe exact results and exits.
    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    pub fn publish_for_test(
        &self,
        function: &VerifiedFunction,
        epoch: u64,
    ) -> Result<super::baseline::PublishedBaselineCode, CompileFailure> {
        let ir = OptimizedIr::translate(function, epoch)?;
        lower_optimized_machine(
            &self.isa,
            &ir,
            None,
            None,
            &NumericSpecialization::default(),
        )?
        .publish()
        .map_err(|_| CompileFailure::InvalidArtifact)
    }

    #[cfg(feature = "test-support")]
    pub fn lower_with_feedback_for_test(
        &self,
        function: &VerifiedFunction,
        key: crate::runtime::FunctionKey,
        feedback: &crate::runtime::FeedbackSnapshot,
    ) -> Result<String, CompileFailure> {
        let ir = OptimizedIr::translate(function, feedback.epoch())?;
        let specialization = NumericSpecialization::from_feedback(function, key, feedback);
        lower_optimized_machine(&self.isa, &ir, None, None, &specialization)
            .map(|code| code.clif().to_owned())
    }

    #[cfg(feature = "test-support")]
    pub fn lower_direct_call_with_feedback_for_test(
        &self,
        function: &VerifiedFunction,
        key: crate::runtime::FunctionKey,
        feedback: &crate::runtime::FeedbackSnapshot,
    ) -> Result<String, CompileFailure> {
        let signature = feedback
            .bounded_specialization(key)
            .ok_or(CompileFailure::InvalidArtifact)?;
        lower_direct_call_machine(&self.isa, function, &signature, None)
            .map(|code| code.clif().to_owned())
    }

    #[cfg(feature = "test-support")]
    pub fn lower_with_direct_target_for_test(
        &self,
        function: &VerifiedFunction,
        key: crate::runtime::FunctionKey,
        feedback: &crate::runtime::FeedbackSnapshot,
        call_pc: u32,
        entry: usize,
    ) -> Result<String, CompileFailure> {
        let ir = OptimizedIr::translate(function, feedback.epoch())?;
        let mut specialization = NumericSpecialization::from_feedback(function, key, feedback);
        let call = feedback
            .call_specialization_at(key, call_pc)
            .ok_or(CompileFailure::InvalidArtifact)?;
        specialization
            .direct_calls
            .insert(call_pc, DirectCallSite { call, entry });
        lower_optimized_machine(&self.isa, &ir, None, None, &specialization)
            .map(|code| code.clif().to_owned())
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    pub fn execute_direct_i32_for_test(
        &self,
        function: &VerifiedFunction,
        key: crate::runtime::FunctionKey,
        feedback: &crate::runtime::FeedbackSnapshot,
        arguments: &[i32],
    ) -> Result<(i32, i32), CompileFailure> {
        let signature = feedback
            .bounded_specialization(key)
            .ok_or(CompileFailure::InvalidArtifact)?;
        if signature
            .arguments()
            .iter()
            .any(|representation| *representation != crate::runtime::FeedbackRepresentation::Int32)
            || arguments.len() != signature.arity()
            || arguments.len() > 2
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let published = lower_direct_call_machine(&self.isa, function, &signature, None)?
            .publish()
            .map_err(|_| CompileFailure::InvalidArtifact)?;
        // The match above proves the exact arity and representation of the
        // scalar-only ABI before converting the executable entry.
        let mut output = 0i32;
        let status = unsafe {
            match arguments {
                [a] => core::mem::transmute::<*const u8, extern "C" fn(*mut i32, i32) -> i32>(
                    published.as_ptr(),
                )(&mut output, *a),
                [a, b] => {
                    core::mem::transmute::<*const u8, extern "C" fn(*mut i32, i32, i32) -> i32>(
                        published.as_ptr(),
                    )(&mut output, *a, *b)
                }
                _ => return Err(CompileFailure::InvalidArtifact),
            }
        };
        Ok((status, output))
    }

    #[cfg(feature = "test-support")]
    pub fn lower_side_path_for_test(
        &self,
        function: &VerifiedFunction,
        epoch: u64,
        profile: crate::runtime::SidePathProfile,
    ) -> Result<String, CompileFailure> {
        let ir = OptimizedIr::translate(function, epoch)?;
        lower_optimized_machine(
            &self.isa,
            &ir,
            None,
            Some(profile),
            &NumericSpecialization::default(),
        )
        .map(|code| code.clif().to_owned())
    }

    pub fn plan(
        function: &VerifiedFunction,
        feedback_epoch: u64,
    ) -> Result<OptimizedArtifactMetadata, CompileFailure> {
        let ir = OptimizedIr::translate(function, feedback_epoch)?;
        let sites = ir
            .guard_maps()
            .iter()
            .map(|site| (site.shape(), site.map().clone()))
            .collect();
        let metrics = ir.metrics();
        Ok(OptimizedArtifactMetadata::new(
            feedback_epoch,
            sites,
            metrics.boxes_elided,
            metrics.cse_eliminated,
            metrics.dead_nodes_eliminated,
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

/// Call opcodes whose call-site feedback the runtime records at the
/// instruction pc; tail calls are lowered as the equivalent call plus return.
fn is_call_site(name: &str) -> bool {
    name.starts_with("call") || name.starts_with("tail_call")
}

fn has_stable_compiled_call(request: &CompileRequest) -> bool {
    let mut found = false;
    for instruction in request.snapshot().instructions() {
        if !is_call_site(instruction.opcode().name()) {
            continue;
        }
        found = true;
        if request
            .feedback()
            .call_specialization_at(request.key(), instruction.pc())
            .is_none()
        {
            return false;
        }
    }
    found
}

impl Compiler for Tier2Compiler {
    fn compile(
        &self,
        request: CompileRequest,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        if request.tier() != Tier::Optimizing {
            return Err(CompileFailure::InvalidArtifact);
        }
        if request.side_path_profile().is_none()
            && !request.feedback().has_stable_value_for(request.key())
            && request
                .feedback()
                .bounded_specialization(request.key())
                .is_none()
            && !has_stable_compiled_call(&request)
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let metadata = Self::plan(
            request.snapshot(),
            request.feedback_epoch().max(self.feedback_epoch),
        )?;
        let ir = OptimizedIr::translate(request.snapshot(), metadata.feedback_epoch())?;
        let profile = request.side_path_profile();
        if let Some(profile) = profile {
            validate_side_path_profile(&request, profile)?;
        }
        let mut specialization = NumericSpecialization::from_feedback(
            request.snapshot(),
            request.key(),
            request.feedback(),
        );
        let mut direct_dependencies = Vec::new();
        for instruction in request.snapshot().instructions() {
            if let Some(target) = request.direct_call_target(instruction.pc()) {
                direct_dependencies.push(target.publication());
                specialization.direct_calls.insert(
                    instruction.pc(),
                    DirectCallSite {
                        call: target.call().clone(),
                        entry: target.entry() as usize,
                    },
                );
            }
        }
        let code = lower_optimized_machine(&self.isa, &ir, None, profile, &specialization)?;
        let direct_signature = (profile.is_none())
            .then(|| request.feedback().bounded_specialization(request.key()))
            .flatten();
        let direct_code = direct_signature.as_ref().and_then(|signature| {
            lower_direct_call_machine(&self.isa, request.snapshot(), signature, None).ok()
        });
        let mut dependencies = vec![crate::code_cache::ArtifactDependency::new(request.key())];
        dependencies.extend(
            specialization
                .calls
                .values()
                .map(|call| crate::code_cache::ArtifactDependency::new(call.callee())),
        );
        let mut artifact = artifact_from_relocatable(request, code)
            .with_dependencies(dependencies)
            .with_optimized_metadata(profile.map_or(metadata.clone(), |profile| {
                metadata.with_side_path_profile(profile)
            }));
        if let (Some(signature), Some(direct_code)) = (direct_signature, direct_code) {
            let optimized = artifact
                .optimized_metadata()
                .cloned()
                .ok_or(CompileFailure::InvalidArtifact)?
                .with_direct_call_signature(signature);
            artifact = artifact
                .with_optimized_metadata(optimized)
                .with_direct_call_relocatable(direct_code);
        }
        Ok(artifact.with_direct_call_dependencies(direct_dependencies))
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
        if request.side_path_profile().is_none()
            && !request.feedback().has_stable_value_for(request.key())
            && request
                .feedback()
                .bounded_specialization(request.key())
                .is_none()
            && !has_stable_compiled_call(&request)
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let metadata = Self::plan(
            request.snapshot(),
            request.feedback_epoch().max(self.feedback_epoch),
        )?;
        control.check_ir_bytes(
            metadata
                .deopt_sites()
                .len()
                .saturating_mul(core::mem::size_of::<DeoptMap>()),
        )?;
        let ir = OptimizedIr::translate(request.snapshot(), metadata.feedback_epoch())?;
        let profile = request.side_path_profile();
        if let Some(profile) = profile {
            validate_side_path_profile(&request, profile)?;
        }
        let mut specialization = NumericSpecialization::from_feedback(
            request.snapshot(),
            request.key(),
            request.feedback(),
        );
        let mut direct_dependencies = Vec::new();
        for instruction in request.snapshot().instructions() {
            if let Some(target) = request.direct_call_target(instruction.pc()) {
                direct_dependencies.push(target.publication());
                specialization.direct_calls.insert(
                    instruction.pc(),
                    DirectCallSite {
                        call: target.call().clone(),
                        entry: target.entry() as usize,
                    },
                );
            }
        }
        let code =
            lower_optimized_machine(&self.isa, &ir, Some(control), profile, &specialization)?;
        let direct_signature = (profile.is_none())
            .then(|| request.feedback().bounded_specialization(request.key()))
            .flatten();
        let direct_code = direct_signature.as_ref().and_then(|signature| {
            lower_direct_call_machine(&self.isa, request.snapshot(), signature, Some(control)).ok()
        });
        control.check()?;
        let mut dependencies = vec![crate::code_cache::ArtifactDependency::new(request.key())];
        dependencies.extend(
            specialization
                .calls
                .values()
                .map(|call| crate::code_cache::ArtifactDependency::new(call.callee())),
        );
        let mut artifact = artifact_from_relocatable(request, code)
            .with_dependencies(dependencies)
            .with_optimized_metadata(profile.map_or(metadata.clone(), |profile| {
                metadata.with_side_path_profile(profile)
            }));
        if let (Some(signature), Some(direct_code)) = (direct_signature, direct_code) {
            let optimized = artifact
                .optimized_metadata()
                .cloned()
                .ok_or(CompileFailure::InvalidArtifact)?
                .with_direct_call_signature(signature);
            artifact = artifact
                .with_optimized_metadata(optimized)
                .with_direct_call_relocatable(direct_code);
        }
        Ok(artifact.with_direct_call_dependencies(direct_dependencies))
    }
}

fn validate_side_path_profile(
    request: &CompileRequest,
    profile: crate::runtime::SidePathProfile,
) -> Result<(), CompileFailure> {
    if profile.function() != request.key()
        || profile.feedback_epoch() != request.feedback_epoch()
        || !request.feedback().contains_stable_observation(
            request.key(),
            profile.pc(),
            profile.observed(),
        )
        || !matches!(
            profile.observed(),
            crate::runtime::ObservedType::Int32 | crate::runtime::ObservedType::Float64
        )
    {
        return Err(CompileFailure::InvalidArtifact);
    }
    Ok(())
}
use crate::{
    bytecode::VerifiedFunction,
    code_cache::OptimizedArtifactMetadata,
    ir::{DeoptMap, OptimizedIr},
    runtime::{CompileRequest, Tier},
};

use super::{
    baseline::{artifact_from_relocatable, BaselineCompiler},
    CompileControl, CompileFailure, Compiler,
};
