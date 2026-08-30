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
    Unknown,
}

#[derive(Clone, Copy, Default)]
enum EntryRepresentation {
    #[default]
    Any,
    Numeric,
    Int32,
    Float64,
}

#[derive(Default)]
struct NumericSpecialization {
    entry: EntryRepresentation,
    int_pcs: std::collections::BTreeSet<u32>,
    float_pcs: std::collections::BTreeSet<u32>,
    calls: std::collections::BTreeMap<u32, crate::runtime::CallSpecializationKey>,
    properties: std::collections::BTreeMap<u32, Box<[crate::runtime::ShapeObservation]>>,
    direct_calls: std::collections::BTreeMap<u32, DirectCallSite>,
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
            .filter(|instruction| instruction.opcode().name().starts_with("call"))
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
        let argument_count = usize::from(function.snapshot().arg_count());
        let Some(representation) = feedback.bounded_specialization(key).and_then(|signature| {
            (signature.arity() == argument_count
                && signature
                    .arguments()
                    .iter()
                    .all(|argument| *argument == signature.result()))
            .then_some(signature.result())
        }) else {
            return Self {
                calls,
                properties,
                ..Self::default()
            };
        };
        let observed = match representation {
            FeedbackRepresentation::Int32 => ObservedType::Int32,
            FeedbackRepresentation::Float64 => ObservedType::Float64,
        };
        let int_pcs = function
            .instructions()
            .iter()
            .filter(|instruction| {
                matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
            })
            .filter_map(|instruction| {
                let site = feedback.binary_at(key, instruction.pc())?;
                (representation == FeedbackRepresentation::Int32
                    && site.state() == FeedbackState::Monomorphic
                    && site.lhs() == [observed]
                    && site.rhs() == [observed]
                    && site.result() == [observed])
                .then_some(instruction.pc())
            })
            .collect();
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
            entry: match representation {
                FeedbackRepresentation::Int32 => EntryRepresentation::Int32,
                FeedbackRepresentation::Float64 => EntryRepresentation::Float64,
            },
            int_pcs,
            float_pcs,
            calls,
            properties,
            direct_calls: Default::default(),
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
            .iter()
            .cloned()
            .nth(rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_POLL as usize)
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
        let payload_type = if int32_loop { types::I32 } else { types::I64 };
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
        );
        for block in ir.blocks() {
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
                        match name {
                            "set_loc_uninitialized" => {
                                let index = opt_u16(node.bytes())?;
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
                            n if opt_index(n, node.bytes(), "get_loc")?.is_some()
                                || n == "get_loc_check" =>
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
                                let index = opt_index(n, node.bytes(), "put_loc")?
                                    .map_or_else(|| opt_u16(node.bytes()), Ok)?;
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let pair = opt_use(&mut builder, stack[depth]);
                                opt_define(&mut builder, locals[index], pair);
                                if !int32_loop {
                                    opt_store(&mut builder, var_buf, index, pair);
                                }
                            }
                            "dup" => {
                                let source = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let pair = opt_use(&mut builder, stack[source]);
                                opt_define(&mut builder, stack[depth], pair);
                                stack_provenance[depth] = stack_provenance[source];
                                depth += 1;
                            }
                            "drop" => {
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                            }
                            "get_field" | "put_field" => {
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
                            "call" | "call0" | "call1" | "call2" | "call3" | "call_method" => {
                                let Some(call) = specialization.calls.get(&node.pc()) else {
                                    return Err(CompileFailure::InvalidArtifact);
                                };
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
                                if argc != call.arguments().len() {
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
                                    reusable_values.insert(*node_id, pair);
                                    depth += 1;
                                    continue;
                                }
                                if specialization.int_pcs.contains(&node.pc()) {
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
                                    let deopt = builder.create_block();
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
                                    reusable_values.insert(*node_id, pair);
                                    depth += 1;
                                    continue;
                                }
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
                                reusable_values.insert(*node_id, pair);
                                depth += 1;
                            }
                            "lt" | "lte" | "gt" | "gte" => {
                                use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
                                depth = depth
                                    .checked_sub(2)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs_pair = opt_use(&mut builder, stack[depth]);
                                let rhs_pair = opt_use(&mut builder, stack[depth + 1]);
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
                                let payload = builder.ins().uextend(payload_type, value);
                                let pair = OptPair {
                                    payload,
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_BOOL)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            "post_inc" | "inc" => {
                                let index = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let old = opt_use(&mut builder, stack[index]);
                                let pair = if int32_loop {
                                    OptPair {
                                        payload: builder.ins().iadd_imm(old.payload, 1),
                                        tag: old.tag,
                                    }
                                } else {
                                    let one = builder.ins().f64const(1.0);
                                    let old = opt_f64(&mut builder, old);
                                    let result = builder.ins().fadd(old, one);
                                    OptPair {
                                        payload: builder.ins().bitcast(
                                            types::I64,
                                            MemFlags::new(),
                                            result,
                                        ),
                                        tag: builder
                                            .ins()
                                            .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64)),
                                    }
                                };
                                if name == "post_inc" {
                                    opt_define(&mut builder, stack[index + 1], pair);
                                    depth += 1;
                                } else {
                                    opt_define(&mut builder, stack[index], pair);
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
    super::baseline::finalize_optimized_machine(isa, clif, control, false)
}

/// Builds the secondary, scalar-only entry used by monomorphic native call
/// edges. Its ABI is `(unboxed arguments...) -> (status:i32, unboxed result)`;
/// status zero is success and a non-zero status asks the caller to deopt at
/// the CALL bytecode.  The entry deliberately has no `JSValue`, frame, or
/// helper parameters, so neither arguments nor the result can be boxed on the
/// compiled-to-compiled fast path.
fn lower_direct_call_machine(
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
                    });
                };
            match name {
                "nop" => {}
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
                    };
                    stack.push(value);
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
        let call = builder.ins().call_indirect(signature, helper, &params);
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
        if provenance.get(index) == Some(&OptProvenance::ImmediatePrimitive) {
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
        let call = builder.ins().call_indirect(signature, helper, &params);
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
        let mut matches =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, function.tag, i64::from(qjs::JS_TAG_OBJECT));
        let identity_matches = builder.ins().icmp_imm(
            IntCC::Equal,
            function.payload,
            direct.call.callee_identity() as i64,
        );
        matches = builder.ins().band(matches, identity_matches);
        // JSObject::u.func.function_bytecode is at byte offset 48 on the
        // validated 64-bit QuickJS layout. Checking both the rooted object
        // and its bytecode identity prevents allocator address reuse from
        // aliasing a different function target.
        let bytecode = builder
            .ins()
            .load(pointer_type, MemFlags::new(), function.payload, 48);
        let bytecode_matches = builder.ins().icmp_imm(
            IntCC::Equal,
            bytecode,
            direct.call.callee_bytecode_identity() as i64,
        );
        matches = builder.ins().band(matches, bytecode_matches);
        for (index, representation) in direct.call.arguments().iter().enumerate() {
            let value = opt_use(builder, stack[argv_index + index]);
            let tag = match representation {
                FeedbackRepresentation::Int32 => qjs::JS_TAG_INT,
                FeedbackRepresentation::Float64 => qjs::JS_TAG_FLOAT64,
            };
            let typed = builder
                .ins()
                .icmp_imm(IntCC::Equal, value.tag, i64::from(tag));
            matches = builder.ins().band(matches, typed);
        }
        let invoke = builder.create_block();
        let deopt = builder.create_block();
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
        };
        let mut signature = Signature::new(builder.func.signature.call_conv);
        signature.params.push(AbiParam::new(pointer_type));
        for argument in direct.call.arguments() {
            signature.params.push(AbiParam::new(match argument {
                FeedbackRepresentation::Int32 => types::I32,
                FeedbackRepresentation::Float64 => types::F64,
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
            });
        }
        let call = builder.ins().call_indirect(signature, target, &params);
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
    for index in (base + 1)..=output_index {
        opt_define(builder, stack[index], undefined);
        opt_store(builder, stack_base, index, undefined);
    }
    stack_provenance[base] = OptProvenance::ImmediatePrimitive;
    for provenance in &mut stack_provenance[(base + 1)..=output_index] {
        *provenance = OptProvenance::Unknown;
    }
    opt_set_stack_top(builder, frame, stack_base, base + 1, pointer_type, layout);
    Ok(base + 1)
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
    let call = builder.ins().call_indirect(signature, helper, &params);
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
) {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let mut numeric = builder.ins().iconst(types::I8, 1);
    let mut alternate_numeric = builder.ins().iconst(types::I8, 1);
    let mut alternate_seen = builder.ins().iconst(types::I8, 0);
    for vars in arguments.iter().chain(locals) {
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
        let valid = match representation {
            EntryRepresentation::Any => builder.ins().iconst(types::I8, 1),
            EntryRepresentation::Numeric => builder.ins().bor(int, float),
            EntryRepresentation::Int32 => int,
            EntryRepresentation::Float64 => float,
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
    let call = builder.ins().call_indirect(signature, poll, &[frame]);
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

fn has_stable_compiled_call(request: &CompileRequest) -> bool {
    let mut found = false;
    for instruction in request.snapshot().instructions() {
        if !instruction.opcode().name().starts_with("call") {
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
