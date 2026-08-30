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

fn lower_optimized_machine(
    isa: &cranelift_codegen::isa::OwnedTargetIsa,
    ir: &OptimizedIr,
    control: Option<&CompileControl>,
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
        let stack = (0..ir.max_stack()).map(|_| alloc()).collect::<Vec<_>>();
        for vars in arguments.iter().chain(&locals).chain(&stack) {
            builder.declare_var(vars.payload, types::I64);
            builder.declare_var(vars.tag, types::I64);
        }
        for (index, vars) in arguments.iter().enumerate() {
            let pair = opt_load(&mut builder, arg_buf, index);
            opt_define(&mut builder, *vars, pair);
        }
        for (index, vars) in locals.iter().enumerate() {
            let pair = opt_load(&mut builder, var_buf, index);
            opt_define(&mut builder, *vars, pair);
        }
        let undefined = OptPair {
            payload: builder.ins().iconst(types::I64, 0),
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
        );
        for block in ir.blocks() {
            let clif_block = blocks[&block.start_pc()];
            builder.switch_to_block(clif_block);
            let mut depth = usize::from(block.stack_depth());
            let mut terminated = false;
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
                        if *mid_loop {
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
                            );
                            builder.switch_to_block(pass);
                        }
                    }
                    crate::ir::OptimizedNodeKind::Bytecode { opcode } => {
                        let name = opcode.as_ref();
                        match name {
                            "set_loc_uninitialized" => {
                                let index = opt_u16(node.bytes())?;
                                opt_define(&mut builder, locals[index], undefined);
                                opt_store(&mut builder, var_buf, index, undefined);
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
                                    payload: builder.ins().iconst(types::I64, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            "undefined" => {
                                opt_define(&mut builder, stack[depth], undefined);
                                depth += 1;
                            }
                            "push_i16" => {
                                let value = i64::from(i16::from_le_bytes([
                                    node.bytes()[1],
                                    node.bytes()[2],
                                ]));
                                let pair = OptPair {
                                    payload: builder.ins().iconst(types::I64, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            "push_i32" => {
                                let value = i64::from(i32::from_le_bytes(
                                    node.bytes()[1..5]
                                        .try_into()
                                        .map_err(|_| CompileFailure::InvalidArtifact)?,
                                ));
                                let pair = OptPair {
                                    payload: builder.ins().iconst(types::I64, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5"
                            | "push_6" | "push_7" => {
                                let value = i64::from(name.as_bytes()[5] - b'0');
                                let pair = OptPair {
                                    payload: builder.ins().iconst(types::I64, value),
                                    tag: builder
                                        .ins()
                                        .iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
                                };
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            n if opt_index(n, node.bytes(), "get_arg")?.is_some() => {
                                let index = opt_index(n, node.bytes(), "get_arg")?.unwrap();
                                let pair = opt_use(&mut builder, arguments[index]);
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            n if opt_index(n, node.bytes(), "get_loc")?.is_some()
                                || n == "get_loc_check" =>
                            {
                                let index = opt_index(n, node.bytes(), "get_loc")?
                                    .map_or_else(|| opt_u16(node.bytes()), Ok)?;
                                let pair = opt_use(&mut builder, locals[index]);
                                opt_define(&mut builder, stack[depth], pair);
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
                                opt_store(&mut builder, var_buf, index, pair);
                            }
                            "dup" => {
                                let pair = opt_use(
                                    &mut builder,
                                    stack[depth
                                        .checked_sub(1)
                                        .ok_or(CompileFailure::InvalidArtifact)?],
                                );
                                opt_define(&mut builder, stack[depth], pair);
                                depth += 1;
                            }
                            "drop" => {
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                            }
                            "add" | "sub" | "mul" | "div" => {
                                depth = depth
                                    .checked_sub(2)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs = opt_use(&mut builder, stack[depth]);
                                let rhs = opt_use(&mut builder, stack[depth + 1]);
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
                                depth += 1;
                            }
                            "lt" | "lte" | "gt" | "gte" => {
                                use cranelift_codegen::ir::condcodes::FloatCC;
                                depth = depth
                                    .checked_sub(2)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let lhs_pair = opt_use(&mut builder, stack[depth]);
                                let rhs_pair = opt_use(&mut builder, stack[depth + 1]);
                                let lhs = opt_f64(&mut builder, lhs_pair);
                                let rhs = opt_f64(&mut builder, rhs_pair);
                                let cc = match name {
                                    "lt" => FloatCC::LessThan,
                                    "lte" => FloatCC::LessThanOrEqual,
                                    "gt" => FloatCC::GreaterThan,
                                    _ => FloatCC::GreaterThanOrEqual,
                                };
                                let value = builder.ins().fcmp(cc, lhs, rhs);
                                let payload = builder.ins().uextend(types::I64, value);
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
                                let one = builder.ins().f64const(1.0);
                                let old = opt_f64(&mut builder, old);
                                let result = builder.ins().fadd(old, one);
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
                                if name == "post_inc" {
                                    opt_define(&mut builder, stack[index + 1], pair);
                                    depth += 1;
                                } else {
                                    opt_define(&mut builder, stack[index], pair);
                                }
                            }
                            "if_false8" | "if_true8" | "if_false" | "if_true" => {
                                depth = depth
                                    .checked_sub(1)
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let condition = opt_use(&mut builder, stack[depth]);
                                let truth = builder.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
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
    super::baseline::finalize_optimized_machine(isa, clif, control)
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
    use cranelift_codegen::ir::{InstBuilder, MemFlags};
    builder
        .ins()
        .store(MemFlags::new(), pair.payload, base, offset);
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
    if let Some(last) = name.as_bytes().last().filter(|last| last.is_ascii_digit()) {
        return Ok(Some(usize::from(*last - b'0')));
    }
    Ok(Some(opt_u16(bytes)?))
}
fn next_block_pc(ir: &OptimizedIr, pc: u32) -> Result<u32, CompileFailure> {
    ir.blocks()
        .iter()
        .position(|block| block.start_pc() == pc)
        .and_then(|index| ir.blocks().get(index + 1))
        .map(|block| block.start_pc())
        .ok_or(CompileFailure::InvalidArtifact)
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
) {
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
    let mut numeric = builder.ins().iconst(types::I8, 1);
    let require_int32 = !locals.is_empty();
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
        let valid = if require_int32 {
            int
        } else {
            builder.ins().bor(int, float)
        };
        numeric = builder.ins().band(numeric, valid);
    }
    let deopt = builder.create_block();
    builder.ins().brif(numeric, pass, &[], deopt, &[]);
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
        lower_optimized_machine(&self.isa, &ir, None).map(|code| code.clif().to_owned())
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

impl Compiler for Tier2Compiler {
    fn compile(
        &self,
        request: CompileRequest,
    ) -> Result<crate::code_cache::CompiledArtifact, CompileFailure> {
        if request.tier() != Tier::Optimizing {
            return Err(CompileFailure::InvalidArtifact);
        }
        if !request.feedback().has_stable_value_for(request.key()) {
            return Err(CompileFailure::InvalidArtifact);
        }
        let metadata = Self::plan(
            request.snapshot(),
            request.feedback_epoch().max(self.feedback_epoch),
        )?;
        let ir = OptimizedIr::translate(request.snapshot(), metadata.feedback_epoch())?;
        let code = lower_optimized_machine(&self.isa, &ir, None)?;
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
        if !request.feedback().has_stable_value_for(request.key()) {
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
        let code = lower_optimized_machine(&self.isa, &ir, Some(control))?;
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
    ir::{DeoptMap, OptimizedIr},
    runtime::{CompileRequest, Tier},
};

use super::{
    baseline::{artifact_from_relocatable, BaselineCompiler},
    CompileControl, CompileFailure, Compiler,
};
