//! Target-only linked leaf entry for heterogeneous, guarded arguments.

use crate::{
    bytecode::VerifiedFunction,
    compiler::{CompileControl, CompileFailure},
    ir::{BaselineIr, BinaryOp, IrOp},
    runtime::{BoundedSpecializationSignature, FeedbackRepresentation},
};
use cranelift_codegen::{
    ir::{types, AbiParam, Function, InstBuilder, MemFlags, Signature, Value},
    isa::OwnedTargetIsa,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use rquickjs_core::qjs;

#[derive(Clone, Copy)]
struct TypedValue {
    value: Value,
    representation: FeedbackRepresentation,
}

/// Build the first, deliberately narrow stage of a JSC-style linked call.
///
/// Target identity and argument-tag guards live at the call site. This entry
/// consumes only those guarded, borrowed values and performs no allocation,
/// ownership transfer, helper call, or frame recovery. Status `1` means the
/// caller must execute the original generic CALL with its untouched frame.
pub(crate) fn lower_target_only_linked_leaf(
    isa: &OwnedTargetIsa,
    function: &VerifiedFunction,
    signature: &BoundedSpecializationSignature,
    control: Option<&CompileControl>,
) -> Result<super::baseline::RelocatableCode, CompileFailure> {
    let snapshot = function.snapshot();
    let slots = usize::from(snapshot.arg_count())
        .checked_add(usize::from(snapshot.local_count()))
        .and_then(|count| count.checked_add(usize::from(snapshot.stack_size())))
        .ok_or(CompileFailure::ResourceLimit)?;
    // Match the existing bounded inline planner. This admission happens before
    // BaselineIr translation or Cranelift block creation, so rejected input
    // cannot turn a stable call-link observation into unbounded compilation.
    if function.instructions().len() > 128 || slots > 64 || snapshot.retained_bytes() > 16 * 1024 {
        return Err(CompileFailure::ResourceLimit);
    }
    if signature.function().id != snapshot.function_id()
        || signature.function().generation != snapshot.generation()
        || signature.arity() != usize::from(snapshot.arg_count())
        || !snapshot.exception_map().is_empty()
        || function.control_flow_graph().blocks().len() != 1
        || !matches!(
            signature.result(),
            FeedbackRepresentation::Int32
                | FeedbackRepresentation::Float64
                | FeedbackRepresentation::Bool
        )
        || !signature
            .arguments()
            .contains(&FeedbackRepresentation::HeapRef)
    {
        return Err(CompileFailure::UnsupportedOpcode);
    }
    if let Some(control) = control {
        control.check()?;
    }

    let ir = BaselineIr::translate(function)?;
    if ir.blocks.len() != 1 {
        return Err(CompileFailure::UnsupportedOpcode);
    }
    let mut abi = Signature::new(isa.default_call_conv());
    abi.params.push(AbiParam::new(isa.pointer_type()));
    abi.params
        .extend(signature.arguments().iter().map(|representation| {
            AbiParam::new(match representation {
                FeedbackRepresentation::Int32 | FeedbackRepresentation::Bool => types::I32,
                FeedbackRepresentation::Float64 => types::F64,
                FeedbackRepresentation::HeapRef => isa.pointer_type(),
            })
        }));
    abi.returns.push(AbiParam::new(types::I32));

    let mut function_ir = Function::with_name_signature(Default::default(), abi);
    let mut context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut function_ir, &mut context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let output = builder.block_params(entry)[0];
        let arguments = builder.block_params(entry)[1..].to_vec();
        let mut stack = Vec::new();
        let mut returned = false;

        for instruction in &ir.blocks[0].instructions {
            if returned {
                return Err(CompileFailure::InvalidArtifact);
            }
            match &instruction.op {
                IrOp::Poll { .. } | IrOp::OsrLabel { .. } | IrOp::Nop => {}
                IrOp::Push(tagged) if tagged.tag == i64::from(qjs::JS_TAG_INT) => {
                    stack.push(TypedValue {
                        value: builder
                            .ins()
                            .iconst(types::I32, tagged.payload as i32 as i64),
                        representation: FeedbackRepresentation::Int32,
                    });
                }
                IrOp::Push(tagged) if tagged.tag == i64::from(qjs::JS_TAG_FLOAT64) => {
                    stack.push(TypedValue {
                        value: builder.ins().f64const(f64::from_bits(tagged.payload)),
                        representation: FeedbackRepresentation::Float64,
                    });
                }
                IrOp::GetArgument(index) => {
                    let index = usize::from(*index);
                    stack.push(TypedValue {
                        value: *arguments
                            .get(index)
                            .ok_or(CompileFailure::InvalidArtifact)?,
                        representation: *signature
                            .arguments()
                            .get(index)
                            .ok_or(CompileFailure::InvalidArtifact)?,
                    });
                }
                IrOp::Drop => {
                    stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                }
                IrOp::Binary(operation @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)) => {
                    let rhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let lhs = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    if lhs.representation != rhs.representation
                        || !matches!(
                            lhs.representation,
                            FeedbackRepresentation::Int32 | FeedbackRepresentation::Float64
                        )
                    {
                        return Err(CompileFailure::UnsupportedOpcode);
                    }
                    let value = match lhs.representation {
                        FeedbackRepresentation::Int32 => {
                            let (value, overflow) = match operation {
                                BinaryOp::Add => builder.ins().sadd_overflow(lhs.value, rhs.value),
                                BinaryOp::Sub => builder.ins().ssub_overflow(lhs.value, rhs.value),
                                BinaryOp::Mul => builder.ins().smul_overflow(lhs.value, rhs.value),
                                _ => unreachable!(),
                            };
                            let must_miss = if *operation == BinaryOp::Mul {
                                use cranelift_codegen::ir::condcodes::IntCC;
                                let zero = builder.ins().icmp_imm(IntCC::Equal, value, 0);
                                let lhs_negative =
                                    builder.ins().icmp_imm(IntCC::SignedLessThan, lhs.value, 0);
                                let rhs_negative =
                                    builder.ins().icmp_imm(IntCC::SignedLessThan, rhs.value, 0);
                                let opposite_signs = builder.ins().bxor(lhs_negative, rhs_negative);
                                let negative_zero = builder.ins().band(zero, opposite_signs);
                                builder.ins().bor(overflow, negative_zero)
                            } else {
                                overflow
                            };
                            let ok = builder.create_block();
                            let miss = builder.create_block();
                            builder.ins().brif(must_miss, miss, &[], ok, &[]);
                            builder.switch_to_block(miss);
                            let status = builder.ins().iconst(types::I32, 1);
                            builder.ins().return_(&[status]);
                            builder.switch_to_block(ok);
                            value
                        }
                        FeedbackRepresentation::Float64 => match operation {
                            BinaryOp::Add => builder.ins().fadd(lhs.value, rhs.value),
                            BinaryOp::Sub => builder.ins().fsub(lhs.value, rhs.value),
                            BinaryOp::Mul => builder.ins().fmul(lhs.value, rhs.value),
                            _ => unreachable!(),
                        },
                        FeedbackRepresentation::Bool | FeedbackRepresentation::HeapRef => {
                            unreachable!("checked above")
                        }
                    };
                    stack.push(TypedValue {
                        value,
                        representation: lhs.representation,
                    });
                }
                IrOp::Return => {
                    let result = stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    if !stack.is_empty() || result.representation != signature.result() {
                        return Err(CompileFailure::UnsupportedOpcode);
                    }
                    builder
                        .ins()
                        .store(MemFlags::new(), result.value, output, 0);
                    let status = builder.ins().iconst(types::I32, 0);
                    builder.ins().return_(&[status]);
                    returned = true;
                }
                _ => return Err(CompileFailure::UnsupportedOpcode),
            }
        }
        if !returned {
            return Err(CompileFailure::UnsupportedOpcode);
        }
        builder.seal_all_blocks();
        builder.finalize();
    }
    super::baseline::finalize_optimized_machine(isa, function_ir, control, false)
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use crate::{
        bytecode::VerifyLimits,
        runtime::{
            BinaryFeedbackFlags, FeedbackRepresentation, FeedbackTable, FunctionKey, ObservedType,
        },
        test_support::SnapshotFixture,
    };

    fn fixture_and_signature(
        source: &str,
        argument_types: &[ObservedType],
        binary_name: &str,
    ) -> (
        SnapshotFixture,
        crate::bytecode::VerifiedFunction,
        crate::runtime::BoundedSpecializationSignature,
    ) {
        let fixture = SnapshotFixture::compile(source);
        let verified = fixture
            .snapshot()
            .verify(VerifyLimits::default())
            .expect("verified linked leaf");
        let key = FunctionKey::new(
            verified.snapshot().function_id(),
            verified.snapshot().generation(),
        );
        let binary_pc = verified
            .instructions()
            .iter()
            .find(|instruction| instruction.opcode().name() == binary_name)
            .expect("binary opcode")
            .pc();
        let return_pc = verified
            .instructions()
            .iter()
            .find(|instruction| instruction.opcode().name() == "return")
            .expect("return opcode")
            .pc();
        let mut feedback = FeedbackTable::new(32, 2);
        for _ in 0..32 {
            feedback.observe_call(key, argument_types);
            feedback.observe_binary(
                key,
                binary_pc,
                ObservedType::Int32,
                ObservedType::Int32,
                ObservedType::Int32,
                BinaryFeedbackFlags::NONE,
            );
            feedback.observe_return(key, return_pc, ObservedType::Int32);
        }
        let signature = feedback
            .snapshot(91)
            .bounded_specialization(key)
            .expect("heterogeneous bounded signature");
        (fixture, verified, signature)
    }

    #[test]
    fn heapref_argument_leaf_executes_without_a_frame_or_helper() {
        let (_fixture, verified, signature) = fixture_and_signature(
            "(function(object, value){return value+1})",
            &[ObservedType::Object, ObservedType::Int32],
            "add",
        );
        assert_eq!(
            signature.arguments(),
            &[
                FeedbackRepresentation::HeapRef,
                FeedbackRepresentation::Int32
            ]
        );
        let isa = cranelift_native::builder()
            .expect("host ISA")
            .finish(cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            ))
            .expect("host flags");
        let code = super::lower_target_only_linked_leaf(&isa, &verified, &signature, None)
            .expect("target-only linked leaf");
        assert!(
            code.clif().contains("(i64, i64, i32) -> i32"),
            "{}",
            code.clif()
        );
        assert!(!code.clif().contains("call_indirect"), "{}", code.clif());
        let published = code.publish().expect("publish linked leaf");
        let mut output = -1i32;
        let status = unsafe {
            core::mem::transmute::<*const u8, extern "C" fn(*mut i32, usize, i32) -> i32>(
                published.as_ptr(),
            )(&mut output, 0x1234usize, 41)
        };
        assert_eq!((status, output), (0, 42));
    }

    #[test]
    fn overflow_returns_miss_without_committing_output() {
        let (_fixture, verified, signature) = fixture_and_signature(
            "(function(object, value){return value+1})",
            &[ObservedType::Object, ObservedType::Int32],
            "add",
        );
        let isa = cranelift_native::builder()
            .expect("host ISA")
            .finish(cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            ))
            .expect("host flags");
        let code = super::lower_target_only_linked_leaf(&isa, &verified, &signature, None)
            .expect("target-only linked leaf")
            .publish()
            .expect("publish linked leaf");
        let mut output = 77i32;
        let status = unsafe {
            core::mem::transmute::<*const u8, extern "C" fn(*mut i32, usize, i32) -> i32>(
                code.as_ptr(),
            )(&mut output, 0x1234usize, i32::MAX)
        };
        assert_eq!(
            status, 1,
            "miss must select the caller's generic CALL block"
        );
        assert_eq!(output, 77, "a miss must not expose a partial result");
    }

    #[test]
    fn int32_multiply_negative_zero_returns_miss() {
        let (_fixture, verified, signature) = fixture_and_signature(
            "(function(object, left, right){return left*right})",
            &[
                ObservedType::Object,
                ObservedType::Int32,
                ObservedType::Int32,
            ],
            "mul",
        );
        let isa = cranelift_native::builder()
            .expect("host ISA")
            .finish(cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            ))
            .expect("host flags");
        let code = super::lower_target_only_linked_leaf(&isa, &verified, &signature, None)
            .expect("target-only linked leaf")
            .publish()
            .expect("publish linked leaf");
        let mut output = 77i32;
        let status = unsafe {
            core::mem::transmute::<*const u8, extern "C" fn(*mut i32, usize, i32, i32) -> i32>(
                code.as_ptr(),
            )(&mut output, 0x1234usize, -1, 0)
        };
        assert_eq!(status, 1, "-0 needs the generic JavaScript multiply path");
        assert_eq!(output, 77, "a miss must not expose Int32 zero");
    }

    #[test]
    fn heapref_link_can_return_a_guarded_bool_without_a_frame() {
        let fixture = SnapshotFixture::compile("(function(object, enabled){return enabled})");
        let verified = fixture
            .snapshot()
            .verify(VerifyLimits::default())
            .expect("verified linked bool leaf");
        let key = FunctionKey::new(
            verified.snapshot().function_id(),
            verified.snapshot().generation(),
        );
        let return_pc = verified
            .instructions()
            .iter()
            .find(|instruction| instruction.opcode().name() == "return")
            .expect("return opcode")
            .pc();
        let mut feedback = FeedbackTable::new(32, 2);
        for _ in 0..32 {
            feedback.observe_call(key, &[ObservedType::Object, ObservedType::Bool]);
            feedback.observe_return(key, return_pc, ObservedType::Bool);
        }
        let signature = feedback
            .snapshot(92)
            .bounded_specialization(key)
            .expect("heterogeneous bool signature");
        assert_eq!(signature.result(), FeedbackRepresentation::Bool);
        let isa = cranelift_native::builder()
            .expect("host ISA")
            .finish(cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            ))
            .expect("host flags");
        let code = super::lower_target_only_linked_leaf(&isa, &verified, &signature, None)
            .expect("target-only linked bool leaf")
            .publish()
            .expect("publish linked bool leaf");
        let mut output = -1i32;
        let invoke = unsafe {
            core::mem::transmute::<*const u8, extern "C" fn(*mut i32, usize, i32) -> i32>(
                code.as_ptr(),
            )
        };
        assert_eq!(invoke(&mut output, 0x1234usize, 1), 0);
        assert_eq!(output, 1);
        assert_eq!(invoke(&mut output, 0x1234usize, 0), 0);
        assert_eq!(output, 0);
    }

    #[test]
    fn oversized_straight_line_leaf_is_rejected_before_codegen() {
        let mut source = String::from("(function(object, value){return value");
        for _ in 0..130 {
            source.push_str("+1");
        }
        source.push_str("})");
        let (_fixture, verified, signature) =
            fixture_and_signature(&source, &[ObservedType::Object, ObservedType::Int32], "add");
        assert!(verified.instructions().len() > 128);
        let isa = cranelift_native::builder()
            .expect("host ISA")
            .finish(cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            ))
            .expect("host flags");
        assert_eq!(
            super::lower_target_only_linked_leaf(&isa, &verified, &signature, None)
                .expect_err("oversized target-only leaf"),
            crate::compiler::CompileFailure::ResourceLimit
        );
    }
}
