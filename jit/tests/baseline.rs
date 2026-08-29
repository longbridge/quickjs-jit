#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]

use cranelift_codegen::{isa, settings};
use rquickjs_core::qjs;
use rquickjs_jit::{
    bytecode::{linked_opcode_table, opcode, VerifyLimits},
    code_cache::RelocationTarget,
    compiler::baseline::BaselineCompiler,
    platform::CodeMemoryError,
    test_support::{verified_bytecode, JSValueRepr, SnapshotFixture, SyntheticFrame},
};

fn named_opcode(name: &str) -> u8 {
    linked_opcode_table()
        .find(|opcode| opcode.name() == name)
        .unwrap_or_else(|| panic!("linked QuickJS opcode {name}"))
        .id()
}

fn compile(
    bytecode: Vec<u8>,
    args: u16,
    locals: u16,
) -> rquickjs_jit::compiler::baseline::RelocatableCode {
    let function = verified_bytecode(bytecode, args, locals);
    BaselineCompiler::host().compile(&function).unwrap()
}

#[test]
fn machine_entry_executes_the_exact_aggregate_return_abi() {
    let executable = compile(vec![opcode::RETURN_UNDEF], 0, 0).publish().unwrap();
    let mut frame = SyntheticFrame::new(&[], 0, 0);

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_DONE);
    assert_eq!(outcome.exit.reserved, 0);
    assert!(outcome.exit.resume_pc.is_null());
    assert!(outcome.exit.resume_stack_top.is_null());
    assert_eq!(outcome.result, JSValueRepr::undefined());
}

#[test]
fn cranelift_ir_has_hidden_sret_and_frame_indirect_poll() {
    let code = compile(vec![opcode::RETURN_UNDEF], 0, 0);
    let signature = code
        .clif()
        .lines()
        .find(|line| line.starts_with("function "))
        .expect("CLIF function signature");

    assert!(signature.contains("sret"), "{signature}");
    assert!(!signature.contains(" -> "), "{signature}");
    assert!(code.clif().contains("call_indirect"));
    assert!(code
        .relocations()
        .iter()
        .all(|relocation| !matches!(relocation.target, RelocationTarget::Symbol(_))));
}

#[test]
fn cranelift_output_retains_unwind_stack_and_frame_metadata() {
    let code = compile(vec![opcode::RETURN_UNDEF], 0, 0);

    assert!(code.unwind_metadata().is_some());
    assert_eq!(code.stack_maps().len(), code.frame_states().len());
    assert!(!code.frame_states().is_empty());
}

#[test]
fn result_copy_preserves_all_sixteen_jsvalue_bytes() {
    let mut bytecode = vec![opcode::GET_ARG];
    bytecode.extend_from_slice(&0_u16.to_le_bytes());
    bytecode.push(opcode::RETURN);
    let executable = compile(bytecode, 1, 0).publish().unwrap();
    let sentinel = JSValueRepr::new(0x0123_4567_89ab_cdef, qjs::JS_TAG_FLOAT64 as i64);
    let mut frame = SyntheticFrame::new(&[sentinel], 0, 1);

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_DONE);
    assert_eq!(outcome.result, sentinel);
}

#[test]
fn unsupported_dynamic_add_retries_before_mutating_the_frame() {
    let mut bytecode = vec![opcode::GET_ARG];
    bytecode.extend_from_slice(&0_u16.to_le_bytes());
    bytecode.push(opcode::GET_ARG);
    bytecode.extend_from_slice(&1_u16.to_le_bytes());
    bytecode.extend([opcode::ADD, opcode::RETURN]);
    let executable = compile(bytecode, 2, 0).publish().unwrap();
    let left = JSValueRepr::new(0x1111_2222_3333_4444, qjs::JS_TAG_OBJECT as i64);
    let right = JSValueRepr::new(0xaaaa_bbbb_cccc_dddd, qjs::JS_TAG_STRING as i64);
    let mut frame = SyntheticFrame::new(&[left, right], 0, 2);
    let before = frame.frame_bytes();

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(
        outcome.exit.kind,
        qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER
    );
    assert_eq!(frame.frame_bytes(), before);
}

#[test]
fn returning_a_borrowed_refcounted_value_retries_without_mutating_the_frame() {
    let mut bytecode = vec![opcode::GET_ARG];
    bytecode.extend_from_slice(&0_u16.to_le_bytes());
    bytecode.push(opcode::RETURN);
    let executable = compile(bytecode, 1, 0).publish().unwrap();
    let object = JSValueRepr::new(0x1111_2222_3333_4444, qjs::JS_TAG_OBJECT as i64);
    let mut frame = SyntheticFrame::new(&[object], 0, 1);
    let before = frame.frame_bytes();

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(
        outcome.exit.kind,
        qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER
    );
    assert_eq!(frame.frame_bytes(), before);
}

#[test]
fn interrupt_is_polled_immediately_before_return() {
    let executable = compile(vec![opcode::RETURN_UNDEF], 0, 0).publish().unwrap();
    let mut frame = SyntheticFrame::new(&[], 0, 0);
    frame.interrupt_on_poll(2);

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_INTERRUPT);
    assert_eq!(frame.poll_count(), 2);
}

#[test]
fn straight_line_code_polls_within_four_thousand_ninety_six_operations() {
    let mut bytecode = vec![opcode::NOP; 4_097];
    bytecode.push(opcode::RETURN_UNDEF);
    let executable = compile(bytecode.clone(), 0, 0).publish().unwrap();
    let mut frame = SyntheticFrame::new(&[], 0, 0);
    frame.set_bytecode(&bytecode);
    let bytecode_start = frame.bytecode_start();
    frame.interrupt_on_poll(2);

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_INTERRUPT);
    assert_eq!(frame.poll_count(), 2);
    let resume_offset = unsafe { outcome.exit.resume_pc.offset_from(bytecode_start) };
    assert!(
        (0..=4_096).contains(&resume_offset),
        "second poll resumed at bytecode offset {resume_offset}"
    );
}

#[test]
fn cross_target_compilation_can_never_publish_on_the_host() {
    let target = if cfg!(target_arch = "x86_64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let flags = settings::Flags::new(settings::builder());
    let isa = isa::lookup(target.parse().unwrap())
        .unwrap()
        .finish(flags)
        .unwrap();
    let function = verified_bytecode(vec![opcode::RETURN_UNDEF], 0, 0);
    let code = BaselineCompiler::new(isa).compile(&function).unwrap();

    assert!(matches!(
        code.publish(),
        Err(CodeMemoryError::TargetIsaMismatch)
    ));
}

#[test]
fn overflowing_add_uses_the_number_path_without_runtime_helpers() {
    let get_arg0 = named_opcode("get_arg0");
    let push_i32 = opcode::PUSH_I32;
    let add = opcode::ADD;
    let return_ = opcode::RETURN;

    let mut bytecode = vec![get_arg0, push_i32];
    bytecode.extend_from_slice(&1_i32.to_le_bytes());
    bytecode.extend([add, return_]);
    let executable = compile(bytecode, 1, 0).publish().unwrap();

    let mut ordinary = SyntheticFrame::new(&[JSValueRepr::int32(41)], 0, 2);
    let ordinary = unsafe { ordinary.call(&executable) };
    assert_eq!(ordinary.result, JSValueRepr::int32(42));

    let mut overflow = SyntheticFrame::new(&[JSValueRepr::int32(i32::MAX)], 0, 2);
    let overflow = unsafe { overflow.call(&executable) };
    assert_eq!(overflow.result.as_f64(), Some(i32::MAX as f64 + 1.0));
}

#[test]
fn compiles_loop_and_integer_arithmetic() {
    let push_0 = named_opcode("push_0");
    let push_1 = named_opcode("push_1");
    let plus = named_opcode("plus");
    let put_loc0 = named_opcode("put_loc0");
    let put_loc1 = named_opcode("put_loc1");
    let get_loc0 = named_opcode("get_loc0");
    let get_loc1 = named_opcode("get_loc1");
    let get_arg0 = named_opcode("get_arg0");
    let lt = named_opcode("lt");
    let if_false8 = named_opcode("if_false8");
    let goto8 = named_opcode("goto8");

    // s = 0; i = 0; while (i < n) { s += i; i += 1; } return s
    let bytecode = vec![
        push_0,
        plus,
        put_loc0,
        push_0,
        plus,
        put_loc1,
        get_loc1,
        get_arg0,
        lt,
        if_false8,
        11,
        get_loc0,
        get_loc1,
        opcode::ADD,
        put_loc0,
        get_loc1,
        push_1,
        opcode::ADD,
        put_loc1,
        goto8,
        (-14_i8) as u8,
        get_loc0,
        opcode::RETURN,
    ];
    let executable = compile(bytecode, 1, 2).publish().unwrap();
    let mut frame = SyntheticFrame::new(&[JSValueRepr::int32(100)], 2, 2);

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_DONE);
    assert_eq!(outcome.result, JSValueRepr::int32(4_950));
    assert!(frame.poll_count() >= 102);
}

#[test]
fn floating_bit_coercion_unsigned_shift_and_negative_zero_match_quickjs_values() {
    let get_arg0 = named_opcode("get_arg0");
    let get_arg1 = named_opcode("get_arg1");

    let bit_not = compile(vec![get_arg0, named_opcode("not"), opcode::RETURN], 1, 0)
        .publish()
        .unwrap();
    let mut frame = SyntheticFrame::new(&[JSValueRepr::float64(1.5)], 0, 1);
    assert_eq!(
        unsafe { frame.call(&bit_not) }.result,
        JSValueRepr::int32(-2)
    );

    let unsigned_shift = compile(
        vec![get_arg0, get_arg1, named_opcode("shr"), opcode::RETURN],
        2,
        0,
    )
    .publish()
    .unwrap();
    let mut frame = SyntheticFrame::new(
        &[JSValueRepr::float64(4_294_967_295.0), JSValueRepr::int32(0)],
        0,
        2,
    );
    assert_eq!(
        unsafe { frame.call(&unsigned_shift) }.result,
        JSValueRepr::float64(4_294_967_295.0)
    );

    let multiply = compile(
        vec![get_arg0, get_arg1, named_opcode("mul"), opcode::RETURN],
        2,
        0,
    )
    .publish()
    .unwrap();
    let mut frame = SyntheticFrame::new(&[JSValueRepr::int32(0), JSValueRepr::int32(-1)], 0, 2);
    assert_eq!(
        unsafe { frame.call(&multiply) }.result,
        JSValueRepr::float64(-0.0)
    );
}

#[test]
fn compiles_quickjs_loop_snapshot_without_rewriting_its_opcodes() {
    let fixture = SnapshotFixture::compile(
        "(function sum(n, zero) { let s = zero; for (let i = zero; i < n; i++) s += i; return s; })",
    );
    let verified = fixture
        .snapshot()
        .verify(VerifyLimits::default())
        .expect("captured loop verifies");
    let names: Vec<_> = verified
        .instructions()
        .iter()
        .map(|instruction| instruction.opcode().name())
        .collect();
    let code = BaselineCompiler::host()
        .compile(&verified)
        .unwrap_or_else(|error| panic!("{error:?} lowering {names:?}"));
    assert!(code
        .frame_states()
        .iter()
        .any(|state| state.code_offset > 0 && !state.slots.is_empty()));
    assert_eq!(code.stack_maps().len(), code.frame_states().len());
    assert!(code
        .stack_maps()
        .iter()
        .any(|map| map.code_offset > 0 && !map.live_slots.is_empty()));
    let executable = code.publish().unwrap();
    let mut frame = SyntheticFrame::new(
        &[JSValueRepr::int32(100), JSValueRepr::int32(0)],
        verified.snapshot().local_count() as usize,
        verified.snapshot().stack_size() as usize,
    );

    let outcome = unsafe { frame.call(&executable) };

    assert_eq!(outcome.exit.kind, qjs::JSJitExitKind_JS_JIT_EXIT_DONE);
    assert_eq!(outcome.result, JSValueRepr::int32(4_950));
}
