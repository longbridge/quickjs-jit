use rquickjs_jit::bytecode::{CompileSnapshot, VerifyLimits};
use rquickjs_jit::code_cache::CompiledArtifact;
use rquickjs_jit::compiler::optimized::{
    NumericBinaryOp, OptimizedCompiler, OptimizedInput, Tier2Compiler,
};
use rquickjs_jit::ir::{OptimizedEffect, OptimizedIr, OptimizedNodeKind, ValueRepresentation};
use rquickjs_jit::runtime::{
    BinaryFeedbackFlags, CompileCompletion, Coordinator, DependencyGraph, DependencyKey,
    FeedbackKind, FeedbackSnapshot, FeedbackState, FeedbackTable, FunctionKey, ObservedType,
    SideExitAction, Tier,
};
use rquickjs_jit::test_support::SnapshotFixture;

#[test]
fn feedback_is_bounded_and_transitions_monotonically() {
    let function = FunctionKey::new(7, 3);
    let mut table = FeedbackTable::new(2, 2);

    assert_eq!(
        table.observe_type(function, 11, FeedbackKind::Value, ObservedType::Int32),
        FeedbackState::Monomorphic
    );
    assert_eq!(
        table.observe_type(function, 11, FeedbackKind::Value, ObservedType::Float64),
        FeedbackState::Polymorphic
    );
    assert_eq!(
        table.observe_type(function, 11, FeedbackKind::Value, ObservedType::String),
        FeedbackState::Megamorphic
    );
    assert_eq!(
        table.observe_type(function, 11, FeedbackKind::Value, ObservedType::Int32),
        FeedbackState::Megamorphic
    );

    table.observe_type(function, 12, FeedbackKind::Value, ObservedType::Int32);
    table.observe_type(function, 13, FeedbackKind::Value, ObservedType::Int32);
    assert_eq!(table.len(), 2);
    assert_eq!(table.dropped_observations(), 1);
}

#[test]
fn feedback_models_all_arguments_return_sites_and_binary_operands() {
    let function = FunctionKey::new(17, 9);
    let mut table = FeedbackTable::new(32, 3);
    table.observe_call(
        function,
        &[
            ObservedType::Int32,
            ObservedType::Float64,
            ObservedType::String,
        ],
    );
    table.observe_return(function, 41, ObservedType::Int32);
    table.observe_return(function, 57, ObservedType::String);
    table.observe_binary(
        function,
        23,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Float64,
        BinaryFeedbackFlags::OVERFLOW,
    );

    let snapshot = table.snapshot(11);
    assert_eq!(
        snapshot.call_argument_types(function),
        Some(
            &[
                ObservedType::Int32,
                ObservedType::Float64,
                ObservedType::String,
            ][..]
        )
    );
    assert_eq!(
        snapshot.stable_return_at(function, 41),
        Some(ObservedType::Int32)
    );
    assert_eq!(
        snapshot.stable_return_at(function, 57),
        Some(ObservedType::String)
    );
    let binary = snapshot.binary_at(function, 23).expect("binary slot");
    assert_eq!(binary.lhs(), &[ObservedType::Int32]);
    assert_eq!(binary.rhs(), &[ObservedType::Int32]);
    assert_eq!(binary.result(), &[ObservedType::Float64]);
    assert!(binary.flags().contains(BinaryFeedbackFlags::OVERFLOW));
}

#[test]
fn feedback_lattice_only_widens_and_flags_accumulate() {
    let function = FunctionKey::new(23, 4);
    let mut table = FeedbackTable::new(32, 2);
    table.observe_binary(
        function,
        7,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );
    table.observe_binary(
        function,
        7,
        ObservedType::Float64,
        ObservedType::Int32,
        ObservedType::Float64,
        BinaryFeedbackFlags::NEGATIVE_ZERO,
    );
    table.observe_binary(
        function,
        7,
        ObservedType::String,
        ObservedType::Object,
        ObservedType::String,
        BinaryFeedbackFlags::NAN,
    );
    table.observe_binary(
        function,
        7,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );

    let snapshot = table.snapshot(12);
    let binary = snapshot.binary_at(function, 7).unwrap();
    assert_eq!(binary.state(), FeedbackState::Megamorphic);
    assert!(binary.flags().contains(BinaryFeedbackFlags::NEGATIVE_ZERO));
    assert!(binary.flags().contains(BinaryFeedbackFlags::NAN));
    assert_eq!(snapshot.function(), Some(function));
    assert_eq!(snapshot.epoch(), 12);
    assert!(snapshot.binary_at(FunctionKey::new(23, 5), 7).is_none());
}

#[test]
fn alternating_argument_types_widen_each_call_slot_without_shifting_positions() {
    let function = FunctionKey::new(31, 2);
    let mut table = FeedbackTable::new(32, 2);
    table.observe_call(function, &[ObservedType::Int32, ObservedType::String]);
    table.observe_call(function, &[ObservedType::Float64, ObservedType::String]);
    table.observe_call(function, &[ObservedType::Object, ObservedType::String]);

    let snapshot = table.snapshot(19);
    let call = snapshot.call_at(function).unwrap();
    assert_eq!(call.state(), FeedbackState::Megamorphic);
    assert_eq!(call.argument(0), &[]);
    assert_eq!(call.argument(1), &[ObservedType::String]);
    assert_eq!(call.argc(), 2);
    assert_eq!(snapshot.call_argument_types(function), None);
}

#[test]
fn compile_request_carries_the_runtime_epoch_to_the_worker() {
    let fixture = SnapshotFixture::compile("(function(){return 1})");
    let snapshot = fixture.snapshot();
    let key = FunctionKey::new(snapshot.function_id(), snapshot.generation());
    let verified = snapshot.verify(VerifyLimits::default()).unwrap();
    let mut coordinator = Coordinator::with_limits(2, 2, 2, 1 << 20);
    coordinator.advance_clock(77);
    coordinator.queue(key, Tier::Baseline, verified).unwrap();
    assert_eq!(coordinator.begin_next().unwrap().feedback_epoch(), 77);
}

#[test]
fn compile_request_owns_an_immutable_feedback_snapshot() {
    let fixture = SnapshotFixture::compile("(function(n){return n+1})");
    let snapshot = fixture.snapshot();
    let key = FunctionKey::new(snapshot.function_id(), snapshot.generation());
    let verified = snapshot.verify(VerifyLimits::default()).unwrap();
    let mut feedback = FeedbackTable::new(8, 2);
    feedback.observe_type(key, 0, FeedbackKind::Value, ObservedType::Int32);
    let frozen = feedback.snapshot(9);
    let mut coordinator = Coordinator::with_limits(2, 2, 2, 1 << 20);

    coordinator
        .queue_with_feedback(key, Tier::Baseline, verified, frozen)
        .unwrap();
    feedback.observe_type(key, 0, FeedbackKind::Value, ObservedType::String);

    let request = coordinator.begin_next().unwrap();
    assert_eq!(request.feedback().epoch(), 9);
    assert_eq!(
        request.feedback().entries()[0].observations(),
        &[ObservedType::Int32]
    );
}

#[test]
fn tier2_feedback_requires_exact_generation_stable_value_and_nonzero_epoch() {
    let key = FunctionKey::new(41, 7);
    let other_generation = FunctionKey::new(41, 8);
    let mut table = FeedbackTable::new(8, 2);
    table.observe_type(key, 0, FeedbackKind::Value, ObservedType::Int32);
    assert!(!table.snapshot(0).has_stable_value_for(key));
    assert!(!table.snapshot(1).has_stable_value_for(other_generation));
    assert!(table.snapshot(1).has_stable_value_for(key));
    table.observe_type(key, 0, FeedbackKind::Value, ObservedType::String);
    assert!(!table.snapshot(2).has_stable_value_for(key));
}

#[test]
fn narrow_optimizer_preserves_javascript_numeric_edges() {
    let mut compiler = OptimizedCompiler;
    let negative_zero = compiler
        .compile(&[
            OptimizedInput::constant_f64(-0.0),
            OptimizedInput::constant_f64(1.0),
            OptimizedInput::binary(NumericBinaryOp::Mul, 0, 1),
            OptimizedInput::ret(2),
        ])
        .expect("numeric input is supported");
    assert!(negative_zero.constant(2).unwrap().is_negative_zero());

    let overflow = compiler
        .compile(&[
            OptimizedInput::constant_i32(i32::MAX),
            OptimizedInput::constant_i32(1),
            OptimizedInput::binary(NumericBinaryOp::Add, 0, 1),
            OptimizedInput::ret(2),
        ])
        .expect("overflow widens exactly");
    assert_eq!(
        overflow.constant(2).unwrap().as_f64(),
        Some(2_147_483_648.0)
    );

    let nan = compiler
        .compile(&[
            OptimizedInput::constant_f64(0.0),
            OptimizedInput::constant_f64(0.0),
            OptimizedInput::binary(NumericBinaryOp::Div, 0, 1),
            OptimizedInput::ret(2),
        ])
        .unwrap();
    assert!(nan.constant(2).unwrap().as_f64().unwrap().is_nan());
}

#[test]
fn integer_zero_times_negative_one_folds_to_negative_zero() {
    let mut compiler = OptimizedCompiler;
    let function = compiler
        .compile(&[
            OptimizedInput::constant_i32(0),
            OptimizedInput::constant_i32(-1),
            OptimizedInput::binary(NumericBinaryOp::Mul, 0, 1),
            OptimizedInput::ret(2),
        ])
        .unwrap();

    assert!(function.constant(2).unwrap().is_negative_zero());
}

#[test]
fn local_cse_and_dce_are_effect_free_and_bounded() {
    let mut compiler = OptimizedCompiler;
    let function = compiler
        .compile(&[
            OptimizedInput::constant_i32(4),
            OptimizedInput::constant_i32(5),
            OptimizedInput::binary(NumericBinaryOp::Add, 0, 1),
            OptimizedInput::binary(NumericBinaryOp::Add, 0, 1),
            OptimizedInput::binary(NumericBinaryOp::Mul, 2, 3),
            OptimizedInput::constant_i32(99),
            OptimizedInput::ret(4),
        ])
        .unwrap();
    assert_eq!(function.cse_eliminated(), 1);
    assert_eq!(function.dead_nodes_eliminated(), 2);
    assert_eq!(function.constant(4).unwrap().as_f64(), Some(81.0));
}

#[test]
fn cse_rewrites_uses_to_the_canonical_ssa_value() {
    let mut compiler = OptimizedCompiler;
    let function = compiler
        .compile(&[
            OptimizedInput::constant_i32(4),
            OptimizedInput::constant_i32(5),
            OptimizedInput::binary(NumericBinaryOp::Add, 0, 1),
            OptimizedInput::binary(NumericBinaryOp::Add, 0, 1),
            OptimizedInput::binary(NumericBinaryOp::Mul, 3, 3),
            OptimizedInput::ret(4),
        ])
        .unwrap();

    assert_eq!(function.cse_eliminated(), 1);
    assert_eq!(function.representative(3), Some(2));
    assert_eq!(function.operands(4), Some((2, 2)));
    assert_eq!(function.constant(4).unwrap().as_f64(), Some(81.0));
}

#[test]
fn mixed_numeric_folding_preserves_negative_zero_nan_and_overflow() {
    let mut compiler = OptimizedCompiler;
    let function = compiler
        .compile(&[
            OptimizedInput::constant_i32(i32::MIN),
            OptimizedInput::constant_i32(-1),
            OptimizedInput::binary(NumericBinaryOp::Mul, 0, 1),
            OptimizedInput::constant_f64(-0.0),
            OptimizedInput::constant_i32(1),
            OptimizedInput::binary(NumericBinaryOp::Mul, 3, 4),
            OptimizedInput::constant_f64(f64::NAN),
            OptimizedInput::binary(NumericBinaryOp::Add, 6, 4),
            OptimizedInput::ret(7),
        ])
        .unwrap();

    assert_eq!(
        function.constant(2).unwrap().as_f64(),
        Some(2_147_483_648.0)
    );
    assert!(function.constant(5).unwrap().is_negative_zero());
    assert!(function.constant(7).unwrap().as_f64().unwrap().is_nan());
}

#[test]
fn dependency_invalidation_is_transitive_and_generation_exact() {
    let a = DependencyKey::function(FunctionKey::new(1, 1));
    let b = DependencyKey::function(FunctionKey::new(2, 1));
    let c = DependencyKey::function(FunctionKey::new(3, 1));
    let mut graph = DependencyGraph::default();
    graph.install(c, 1, []).unwrap();
    graph.install(b, 1, [c]).unwrap();
    graph.install(a, 1, [b]).unwrap();

    let invalidated = graph.invalidate(c);
    assert_eq!(invalidated.len(), 3);
    assert!(!graph.validate_install(a, 1, &[(b, 1)]));

    let b2 = DependencyKey::function(FunctionKey::new(2, 2));
    graph.install(b2, 1, []).unwrap();
    assert!(graph.validate_install(b2, 1, &[]));

    assert!(graph
        .invalidate(DependencyKey::function(FunctionKey::new(99, 1)))
        .is_empty());
}

#[test]
fn tier2_plan_is_exact_for_numeric_locals_and_rejects_property_semantics() {
    let numeric = SnapshotFixture::compile(
        "(function(n,zero){let s=zero; for(let i=zero;i<n;i++) s=s+i; return s})",
    );
    let numeric = numeric.snapshot().verify(VerifyLimits::default()).unwrap();
    let metadata = Tier2Compiler::plan(&numeric, 23).expect("local numeric loop is narrow Tier2");
    assert_eq!(metadata.feedback_epoch(), 23);
    assert!(metadata.boxes_elided() > 0);
    assert!(!metadata.deopt_sites().is_empty());
    assert!(metadata
        .deopt_sites()
        .iter()
        .all(|(shape, map)| map.validate(*shape).is_ok()));

    let property = SnapshotFixture::compile("(function(){return globalThis.answer})");
    let property = property.snapshot().verify(VerifyLimits::default()).unwrap();
    assert!(Tier2Compiler::plan(&property, 24).is_err());
}

#[test]
fn production_optimized_ir_is_independent_ssa_with_loop_guards() {
    let fixture = SnapshotFixture::compile(
        "(function(n,zero){let dead=40+2;let s=zero;for(let i=zero;i<n;i++)s=s+i;return s})",
    );
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let ir = OptimizedIr::translate(&verified, 31).expect("numeric loop is optimized directly");

    assert!(ir.blocks().iter().any(|block| block.is_loop_header()));
    assert!(ir
        .nodes()
        .iter()
        .any(|node| matches!(node.kind(), OptimizedNodeKind::GuardNumeric { .. })));
    assert!(ir
        .nodes()
        .iter()
        .any(|node| node.representation() == ValueRepresentation::Float64));
    assert!(ir
        .nodes()
        .iter()
        .all(|node| node.effect() != OptimizedEffect::Reentrant));
    assert!(
        ir.guard_maps().len() >= 2,
        "entry and mid-loop maps are required"
    );
    assert!(ir
        .guard_maps()
        .iter()
        .all(|site| site.map().guard() == site.guard()));
    assert!(ir
        .guard_maps()
        .iter()
        .all(|site| site.map().validate(site.shape()).is_ok()));
}

#[test]
fn iterative_fibonacci_multi_local_loop_translates_to_optimized_ir() {
    let fixture = SnapshotFixture::compile(
        "(function(_iterations,seed){let a=seed;let b=1;for(let i=seed;i<40;i++){const next=a+b;a=b;b=next;}return a})",
    );
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    verified.tier1_eligibility().unwrap_or_else(|error| {
        panic!(
            "iterative Fibonacci is not advertised for Tier1: {error:?}; opcodes={:?}",
            verified
                .instructions()
                .iter()
                .map(|instruction| (instruction.pc(), instruction.opcode().name()))
                .collect::<Vec<_>>()
        )
    });
    let ir = OptimizedIr::translate(&verified, 32)
        .unwrap_or_else(|error| panic!("iterative Fibonacci did not translate: {error:?}"));
    assert!(ir.blocks().iter().any(|block| block.is_loop_header()));
    rquickjs_jit::test_support::compile_implemented_fixture(
        &rquickjs_jit::compiler::baseline::BaselineCompiler::host(),
        &verified,
    )
    .unwrap_or_else(|error| panic!("iterative Fibonacci did not lower in Tier1: {error:?}"));
    Tier2Compiler::host(32)
        .lower_for_test(&verified, 32)
        .unwrap_or_else(|error| panic!("iterative Fibonacci did not lower: {error:?}"));

    let batched = SnapshotFixture::compile(
        "(function(iterations,seed){let result=seed;for(let batch=seed;batch<iterations;batch++){let a=seed;let b=1;for(let i=seed;i<40;i++){const next=a+b;a=b;b=next;}result=a;}return result})",
    );
    let batched = batched.snapshot().verify(VerifyLimits::default()).unwrap();
    batched.tier1_eligibility().unwrap_or_else(|error| {
        panic!(
            "batched iterative Fibonacci is not Tier1: {error:?}; opcodes={:?}",
            batched
                .instructions()
                .iter()
                .map(|instruction| (instruction.pc(), instruction.opcode().name()))
                .collect::<Vec<_>>()
        )
    });
    let key = FunctionKey::new(
        batched.snapshot().function_id(),
        batched.snapshot().generation(),
    );
    let return_pc = batched
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(64, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
        for instruction in batched
            .instructions()
            .iter()
            .filter(|instruction| instruction.opcode().name() == "add")
        {
            feedback.observe_binary(
                key,
                instruction.pc(),
                ObservedType::Int32,
                ObservedType::Int32,
                ObservedType::Int32,
                Default::default(),
            );
        }
        feedback.observe_return(key, return_pc, ObservedType::Int32);
    }
    let clif = Tier2Compiler::host(33)
        .lower_with_feedback_for_test(&batched, key, &feedback.snapshot(33))
        .unwrap_or_else(|error| panic!("batched iterative Fibonacci did not lower: {error:?}"));
    assert!(
        clif.lines()
            .any(|line| line.starts_with("block") && line.matches(": i32").count() >= 4),
        "the a/b/i/batch loop phis must remain raw i32: {clif}"
    );
    assert!(clif.contains("sadd_overflow"), "{clif}");
    assert!(!clif.contains("fadd"), "{clif}");
    assert!(!clif.contains("fcvt_from_sint"), "{clif}");
}

#[test]
fn tier2_rejects_captured_loop_headers_with_live_operand_stack() {
    use rquickjs_core::qjs;
    let bytecode = vec![
        qjs::QJS_JIT_OP_PUSH_TRUE,
        qjs::QJS_JIT_OP_DUP,
        qjs::QJS_JIT_OP_IF_TRUE8,
        (-2i8) as u8,
        qjs::QJS_JIT_OP_RETURN,
    ];
    let verified = CompileSnapshot::from_untrusted_bytecode(bytecode, 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .expect("captured loop with a live stack value is well formed");
    assert!(OptimizedIr::translate(&verified, 1).is_err());
}

#[test]
fn independent_optimized_machine_lowers_numeric_loop() {
    let fixture = SnapshotFixture::compile(
        "(function(n,zero){let s=zero;for(let i=zero;i<n;i++)s=s+i;return s})",
    );
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let clif = Tier2Compiler::host(33)
        .lower_for_test(&verified, 33)
        .unwrap();
    assert!(clif.contains("fadd"));
    assert!(clif.contains("brif"));
    let fixture = SnapshotFixture::compile(
        "(function(n,zero){let unused;let s=zero;for(let i=zero;i<n;i++)s=s+i;return s})",
    );
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    Tier2Compiler::host(34)
        .lower_for_test(&verified, 34)
        .unwrap();
}

#[test]
fn stable_int32_add_feedback_selects_a_guarded_integer_only_hot_path() {
    let fixture = SnapshotFixture::compile("(function add(a,b){return a+b})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let add_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "add")
        .unwrap()
        .pc();
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(16, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
        feedback.observe_binary(
            key,
            add_pc,
            ObservedType::Int32,
            ObservedType::Int32,
            ObservedType::Int32,
            Default::default(),
        );
        feedback.observe_return(key, return_pc, ObservedType::Int32);
    }

    let clif = Tier2Compiler::host(61)
        .lower_with_feedback_for_test(&verified, key, &feedback.snapshot(61))
        .expect("stable Int32 add feedback is specialized");

    assert!(clif.contains("sadd_overflow"), "{clif}");
    assert!(
        !clif.contains("fadd"),
        "the Int32 hot path must not compute a float add: {clif}"
    );
    assert!(
        !clif
            .lines()
            .any(|line| line.trim_start().starts_with("call ")),
        "the Int32 add hot path must not call a helper: {clif}"
    );
}

#[test]
fn stable_float64_feedback_selects_strict_direct_float_arithmetic() {
    let fixture =
        SnapshotFixture::compile("(function arithmetic(a,b){return (a+b)+(a-b)+(a*b)+(a/b)})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(32, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Float64, ObservedType::Float64]);
        for instruction in verified.instructions().iter().filter(|instruction| {
            matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
        }) {
            feedback.observe_binary(
                key,
                instruction.pc(),
                ObservedType::Float64,
                ObservedType::Float64,
                ObservedType::Float64,
                Default::default(),
            );
        }
        feedback.observe_return(key, return_pc, ObservedType::Float64);
    }

    let clif = Tier2Compiler::host(62)
        .lower_with_feedback_for_test(&verified, key, &feedback.snapshot(62))
        .expect("stable Float64 arithmetic feedback is specialized");

    for operation in ["fadd", "fsub", "fmul", "fdiv"] {
        assert!(clif.contains(operation), "missing {operation}: {clif}");
    }
    assert!(!clif.contains("fcvt_from_sint"), "{clif}");
    assert!(!clif.contains("sadd_overflow"), "{clif}");
    assert!(
        !clif
            .lines()
            .any(|line| line.trim_start().starts_with("call ")),
        "the Float64 hot path must not call a helper: {clif}"
    );
}

#[test]
fn stable_int32_loop_carries_unboxed_i32_ssa_through_the_header() {
    let fixture =
        SnapshotFixture::compile("(function sum(n,z){let s=z;for(let i=z;i<n;i++)s=s+i;return s})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(64, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
        for instruction in verified.instructions().iter().filter(|instruction| {
            matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
        }) {
            feedback.observe_binary(
                key,
                instruction.pc(),
                ObservedType::Int32,
                ObservedType::Int32,
                ObservedType::Int32,
                Default::default(),
            );
        }
        feedback.observe_return(key, return_pc, ObservedType::Int32);
    }
    let opcodes = verified
        .instructions()
        .iter()
        .map(|instruction| instruction.opcode().name())
        .collect::<Vec<_>>();
    let clif = Tier2Compiler::host(63)
        .lower_with_feedback_for_test(&verified, key, &feedback.snapshot(63))
        .unwrap();

    assert!(
        clif.lines()
            .any(|line| { line.starts_with("block") && line.matches(": i32").count() >= 2 }),
        "opcodes={opcodes:?}\n{clif}"
    );
    assert!(clif.contains("sadd_overflow"), "{clif}");
    assert!(!clif.contains("fcvt_from_sint"), "{clif}");
    assert_eq!(
        clif.matches("ireduce.i32").count(),
        2,
        "only the two entry arguments may be unboxed: {clif}"
    );
    assert_eq!(
        clif.matches("call_indirect").count(),
        1,
        "only the amortized interrupt poll helper is permitted: {clif}"
    );
}

#[test]
fn stable_int32_sub_mul_div_use_checked_native_operations() {
    let fixture =
        SnapshotFixture::compile("(function arithmetic(a,b){return ((a-b)*(a+b))/(b+1)})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(64, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
        for instruction in verified.instructions().iter().filter(|instruction| {
            matches!(instruction.opcode().name(), "add" | "sub" | "mul" | "div")
        }) {
            feedback.observe_binary(
                key,
                instruction.pc(),
                ObservedType::Int32,
                ObservedType::Int32,
                ObservedType::Int32,
                Default::default(),
            );
        }
        feedback.observe_return(key, return_pc, ObservedType::Int32);
    }
    let clif = Tier2Compiler::host(64)
        .lower_with_feedback_for_test(&verified, key, &feedback.snapshot(64))
        .unwrap();

    for operation in ["ssub_overflow", "smul_overflow", "sdiv", "srem"] {
        assert!(clif.contains(operation), "missing {operation}: {clif}");
    }
    assert!(!clif.contains("fsub"), "{clif}");
    assert!(!clif.contains("fmul"), "{clif}");
    assert!(!clif.contains("fdiv"), "{clif}");
    assert!(
        clif.matches("brif").count() >= 7,
        "division guards missing: {clif}"
    );
    assert!(clif.contains("-2147483648"), "MIN/-1 guard missing: {clif}");
    assert!(clif.contains(", -1"), "MIN/-1 guard missing: {clif}");
    assert!(
        clif.contains("slt") && clif.contains(", 0"),
        "negative-zero guard missing: {clif}"
    );
}

#[test]
fn stable_monomorphic_call_lowers_with_visible_owner_provenance() {
    let fixture = SnapshotFixture::compile("(function invoke(f,a){let x=f(a);return x+0})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let caller = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let call_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name().starts_with("call"))
        .expect("call opcode")
        .pc();
    let callee = FunctionKey::new(caller.id + 100, 1);
    let mut feedback = FeedbackTable::new(32, 2);
    for _ in 0..32 {
        feedback.observe_call_signature(
            caller,
            call_pc,
            callee,
            &[ObservedType::Int32],
            ObservedType::Int32,
        );
    }

    let clif = Tier2Compiler::host(65)
        .lower_with_feedback_for_test(&verified, caller, &feedback.snapshot(65))
        .expect("owned call bridge");
    assert!(clif.contains("call_indirect"), "{clif}");
}

#[test]
fn monomorphic_call_emits_pointer_guard_and_unboxed_native_abi() {
    let fixture = SnapshotFixture::compile("(function invoke(f,a){let x=f(a);return x+0})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let caller = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let call_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name().starts_with("call"))
        .unwrap()
        .pc();
    let callee = FunctionKey::new(caller.id + 100, 9);
    let mut feedback = FeedbackTable::new(32, 2);
    for _ in 0..32 {
        feedback.observe_call_signature_with_identity(
            caller,
            call_pc,
            callee,
            0x1234_5678,
            0x2234_5678,
            &[ObservedType::Int32],
            ObservedType::Int32,
        );
    }
    let clif = Tier2Compiler::host(68)
        .lower_with_direct_target_for_test(
            &verified,
            caller,
            &feedback.snapshot(68),
            call_pc,
            0x7654_3210,
        )
        .expect("direct native caller");
    assert!(clif.contains("(i64, i32) -> i32"), "{clif}");
    assert!(
        clif.contains("0x1234_5678"),
        "callee identity guard absent: {clif}"
    );
    assert!(
        clif.contains("0x2234_5678"),
        "callee bytecode guard absent: {clif}"
    );
    assert!(
        clif.contains("0x7654_3210"),
        "direct entry address absent: {clif}"
    );
    assert!(clif.contains("call_indirect"), "{clif}");
    assert!(
        clif.contains("brif") && clif.contains("return"),
        "guard/status mismatch must have exact deopt edge: {clif}"
    );
}

#[test]
fn direct_call_entry_uses_only_unboxed_int32_abi_and_checked_arithmetic() {
    let fixture = SnapshotFixture::compile("(function(a,b){return a+b})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let add_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "add")
        .unwrap()
        .pc();
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(32, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
        feedback.observe_binary(
            key,
            add_pc,
            ObservedType::Int32,
            ObservedType::Int32,
            ObservedType::Int32,
            Default::default(),
        );
        feedback.observe_return(key, return_pc, ObservedType::Int32);
    }
    let clif = Tier2Compiler::host(66)
        .lower_direct_call_with_feedback_for_test(&verified, key, &feedback.snapshot(66))
        .expect("typed direct-call entry");
    assert!(clif.contains("(i64, i32, i32) -> i32"), "{clif}");
    assert!(clif.contains("sadd_overflow"), "{clif}");
    assert!(
        !clif.contains("iconst.i64"),
        "tagged JSValue leaked into scalar body: {clif}"
    );
    assert!(
        !clif.contains("call_indirect"),
        "helper leaked into direct entry: {clif}"
    );
    assert_eq!(
        Tier2Compiler::host(66)
            .execute_direct_i32_for_test(&verified, key, &feedback.snapshot(66), &[20, 22])
            .unwrap(),
        (0, 42)
    );
    assert_eq!(
        Tier2Compiler::host(66)
            .execute_direct_i32_for_test(&verified, key, &feedback.snapshot(66), &[i32::MAX, 1])
            .unwrap()
            .0,
        1,
        "overflow must request exact CALL-site deopt"
    );
}

#[test]
fn direct_call_entry_uses_only_unboxed_float64_abi() {
    let fixture = SnapshotFixture::compile("(function(a,b){return a+b})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let add_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "add")
        .unwrap()
        .pc();
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(32, 2);
    for _ in 0..32 {
        feedback.observe_call(key, &[ObservedType::Float64, ObservedType::Float64]);
        feedback.observe_binary(
            key,
            add_pc,
            ObservedType::Float64,
            ObservedType::Float64,
            ObservedType::Float64,
            Default::default(),
        );
        feedback.observe_return(key, return_pc, ObservedType::Float64);
    }
    let clif = Tier2Compiler::host(67)
        .lower_direct_call_with_feedback_for_test(&verified, key, &feedback.snapshot(67))
        .expect("typed direct-call entry");
    assert!(clif.contains("(i64, f64, f64) -> i32"), "{clif}");
    assert!(clif.contains("fadd"), "{clif}");
    assert!(
        !clif.contains("call_indirect"),
        "helper leaked into direct entry: {clif}"
    );
}

#[test]
fn optimized_passes_rewrite_the_emitted_machine_plan() {
    let fixture = SnapshotFixture::compile("(function(a,b){a+b;return (a+b)+(a+b)})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let ir = OptimizedIr::translate(&verified, 32).expect("pure numeric function");

    assert!(ir.metrics().cse_eliminated > 0, "{:#?}", ir.nodes());
    assert!(ir.machine_plan().iter().all(|node| !node.eliminated()));
    assert!(ir.machine_plan().len() < ir.nodes().len());

    let clif = Tier2Compiler::host(32)
        .lower_for_test(&verified, 32)
        .expect("rewritten plan lowers");
    assert_eq!(clif.matches("fadd").count(), 2, "{clif}");
}

#[test]
fn production_cse_keys_exact_ssa_operands_and_respects_frame_writes() {
    let distinct = SnapshotFixture::compile("(function(a,b,c,d,e,f){return (e-f)+(f-e)})");
    let distinct = distinct.snapshot().verify(VerifyLimits::default()).unwrap();
    let distinct_ir = OptimizedIr::translate(&distinct, 101).unwrap();
    assert_eq!(
        distinct_ir.metrics().cse_eliminated,
        0,
        "{:#?}",
        distinct_ir.nodes()
    );

    let mutation =
        SnapshotFixture::compile("(function(a,b){let x=a;let first=x-b;x=b;return first+(x-b)})");
    let mutation = mutation.snapshot().verify(VerifyLimits::default()).unwrap();
    let mutation_ir = OptimizedIr::translate(&mutation, 102).unwrap();
    assert_eq!(
        mutation_ir.metrics().cse_eliminated,
        0,
        "{:#?}",
        mutation_ir.nodes()
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn production_tier2_truthiness_preserves_negative_zero_and_nan_without_fallback() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let captured = SnapshotFixture::compile("(function truth(v){return v?2:1})");
    let verified = captured.snapshot().verify(VerifyLimits::default()).unwrap();
    Tier2Compiler::host(91)
        .lower_for_test(&verified, 91)
        .expect("truthiness fixture must reach the production optimizing lowerer");

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(4)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| ctx.eval::<(), _>("function truth(v){return v?2:1}"))
        .unwrap();
    for _ in 0..8 {
        assert_eq!(
            context.with(|ctx| ctx.eval::<i32, _>("truth(-0)")).unwrap(),
            1
        );
    }
    for _ in 0..10_000 {
        jit.poll();
        if jit.metrics().installed > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().installed > 0, "{:?}", jit.metrics());
    for _ in 0..8 {
        assert_eq!(
            context.with(|ctx| ctx.eval::<i32, _>("truth(-0)")).unwrap(),
            1
        );
    }
    for _ in 0..10_000 {
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().installed >= 2, "{:?}", jit.metrics());
    let before = jit.metrics();
    assert_eq!(
        context.with(|ctx| ctx.eval::<i32, _>("truth(-0)")).unwrap(),
        1
    );
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<i32, _>("truth(NaN)"))
            .unwrap(),
        1
    );
    jit.poll();
    let after = jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries, "{after:?}");
    assert_eq!(after.native_fallbacks, before.native_fallbacks, "{after:?}");
    assert_eq!(after.native_retries, before.native_retries, "{after:?}");
    let before_object = jit.metrics();
    for _ in 0..1_024 {
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<i32, _>("truth(globalThis.truthy ||= {})"))
                .unwrap(),
            2
        );
        jit.poll();
    }
    assert!(
        jit.metrics().deopts > before_object.deopts,
        "{:?}",
        jit.metrics()
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn production_feedback_installs_and_executes_the_int32_add_specialization() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(64)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| ctx.eval::<(), _>("function add(a,b){return a+b}"))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        assert_eq!(
            context.with(|ctx| {
                let add: rquickjs::Function<'_> = ctx.globals().get("add").unwrap();
                add.call::<_, i32>((20, 22)).unwrap()
            }),
            42
        );
        jit.poll();
        if jit.metrics().tier2_entries > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());
    let artifact = jit.test_last_acquired_artifact_key().unwrap();
    assert_eq!(artifact.tier, Tier::Optimizing);
    assert_ne!(artifact.specialization_fingerprint, 0);

    let before = jit.metrics();
    assert_eq!(
        context.with(|ctx| {
            let add: rquickjs::Function<'_> = ctx.globals().get("add").unwrap();
            add.call::<_, i32>((7, 8)).unwrap()
        }),
        15
    );
    assert_eq!(
        context.with(|ctx| {
            let add: rquickjs::Function<'_> = ctx.globals().get("add").unwrap();
            add.call::<_, f64>((2_147_483_647_i32, 1_i32)).unwrap()
        }),
        2_147_483_648.0
    );
    assert_eq!(
        context.with(|ctx| {
            let add: rquickjs::Function<'_> = ctx.globals().get("add").unwrap();
            add.call::<_, String>(("a", "b")).unwrap()
        }),
        "ab"
    );
    jit.poll();
    let after = jit.metrics();
    assert!(after.tier2_entries >= before.tier2_entries + 3, "{after:?}");
    assert!(after.deopts >= before.deopts + 2, "{after:?}");
    assert!(
        after.deopt_materializations >= before.deopt_materializations + 2,
        "{after:?}"
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn production_tier2_caller_executes_a_monomorphic_compiled_callee() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(64)
            .stress_gc(true)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function directAdd(a){return a+1}\n\
                 function invoke(f,a){let result=f(a);return result+0}\n\
                 function invokeTwice(f,a){let x=f(a);let y=f(x);return y+0}\n\
                 function recursiveCount(n){if(n<=0)return 0;return recursiveCount(n-1)+1}",
            )
        })
        .unwrap();

    for _ in 0..512 {
        assert_eq!(
            context.with(|ctx| {
                let add: rquickjs::Function<'_> = ctx.globals().get("directAdd").unwrap();
                let invoke: rquickjs::Function<'_> = ctx.globals().get("invoke").unwrap();
                invoke.call::<_, i32>((add, 41)).unwrap()
            }),
            42
        );
        assert_eq!(
            context.with(|ctx| {
                let add: rquickjs::Function<'_> = ctx.globals().get("directAdd").unwrap();
                let invoke: rquickjs::Function<'_> = ctx.globals().get("invokeTwice").unwrap();
                invoke.call::<_, i32>((add, 40)).unwrap()
            }),
            42
        );
        assert_eq!(
            context.with(|ctx| {
                let recursive: rquickjs::Function<'_> =
                    ctx.globals().get("recursiveCount").unwrap();
                recursive.call::<_, i32>((12,)).unwrap()
            }),
            12
        );
        jit.poll();
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    let before = jit.metrics();
    assert!(before.installed >= 3, "{before:?}");
    assert_eq!(
        context.with(|ctx| {
            let add: rquickjs::Function<'_> = ctx.globals().get("directAdd").unwrap();
            let invoke: rquickjs::Function<'_> = ctx.globals().get("invoke").unwrap();
            invoke.call::<_, i32>((add, 99)).unwrap()
        }),
        100
    );
    for value in 0..2_048 {
        assert_eq!(
            context.with(|ctx| {
                let add: rquickjs::Function<'_> = ctx.globals().get("directAdd").unwrap();
                let invoke: rquickjs::Function<'_> = ctx.globals().get("invokeTwice").unwrap();
                invoke.call::<_, i32>((add, value)).unwrap()
            }),
            value + 2
        );
        assert_eq!(
            context.with(|ctx| {
                let recursive: rquickjs::Function<'_> =
                    ctx.globals().get("recursiveCount").unwrap();
                recursive.call::<_, i32>((8,)).unwrap()
            }),
            8
        );
    }
    jit.poll();
    let after = jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries, "{after:?}");
    assert_eq!(after.native_fallbacks, before.native_fallbacks, "{after:?}");
    assert_eq!(after.native_retries, before.native_retries, "{after:?}");
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn production_unboxed_call_deopts_exactly_on_target_type_and_overflow_mismatch() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(64)
            .stress_gc(true)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function add1(a){return a+1}\n\
         function sub1(a){return a-1}\n\
         function addf(a,b){return a+b}\n\
         function invokeTarget(f,a){let x=f(a);return x+0}\n\
         function invokeType(f,a){let x=f(a);return x+0}\n\
         function invokeOverflow(f,a){let x=f(a);return x+0}\n\
         function invokef(f,a,b){let x=f(a,b);return x+0}",
            )
        })
        .unwrap();

    // Publish both typed callees before collecting/compiling their callers.
    for _ in 0..512 {
        context.with(|ctx| {
            let add1: rquickjs::Function<'_> = ctx.globals().get("add1").unwrap();
            assert_eq!(add1.call::<_, i32>((40,)).unwrap(), 41);
            let addf: rquickjs::Function<'_> = ctx.globals().get("addf").unwrap();
            assert_eq!(addf.call::<_, f64>((1.25, 2.5)).unwrap(), 3.75);
        });
        jit.poll();
    }
    for _ in 0..512 {
        context.with(|ctx| {
            let add1: rquickjs::Function<'_> = ctx.globals().get("add1").unwrap();
            for name in ["invokeTarget", "invokeType", "invokeOverflow"] {
                let invoke: rquickjs::Function<'_> = ctx.globals().get(name).unwrap();
                assert_eq!(invoke.call::<_, i32>((add1.clone(), 40)).unwrap(), 41);
            }
            let addf: rquickjs::Function<'_> = ctx.globals().get("addf").unwrap();
            let invokef: rquickjs::Function<'_> = ctx.globals().get("invokef").unwrap();
            assert_eq!(invokef.call::<_, f64>((addf, 1.25, 2.5)).unwrap(), 3.75);
        });
        jit.poll();
    }
    let before = jit.metrics();
    context.with(|ctx| {
        let invoke: rquickjs::Function<'_> = ctx.globals().get("invokeTarget").unwrap();
        let sub1: rquickjs::Function<'_> = ctx.globals().get("sub1").unwrap();
        assert_eq!(invoke.call::<_, i32>((sub1, 40)).unwrap(), 39);
        let invoke: rquickjs::Function<'_> = ctx.globals().get("invokeType").unwrap();
        let add1: rquickjs::Function<'_> = ctx.globals().get("add1").unwrap();
        assert_eq!(invoke.call::<_, f64>((add1, 1.5)).unwrap(), 2.5);
        let invoke: rquickjs::Function<'_> = ctx.globals().get("invokeOverflow").unwrap();
        let add1: rquickjs::Function<'_> = ctx.globals().get("add1").unwrap();
        assert_eq!(
            invoke.call::<_, f64>((add1, i32::MAX)).unwrap(),
            2_147_483_648.0
        );
    });
    jit.poll();
    let after = jit.metrics();
    assert!(after.deopts >= before.deopts + 2, "{before:?} -> {after:?}");
    assert!(
        after.deopt_materializations >= before.deopt_materializations + 2,
        "{before:?} -> {after:?}"
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn production_worker_installs_and_enters_narrow_tier2_native_code() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let config = JitConfig::builder()
        .call_threshold(2)
        .loop_threshold(4)
        .stress_gc(true)
        .force_optimized_for_test(true)
        .build()
        .unwrap();
    let jit = Jit::attach(&runtime, config).unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| ctx.eval::<(), _>("function f(n,z){let s=z;for(let i=z;i<n;i++)s+=i;return s}"))
        .unwrap();
    let first = context
        .with(|ctx| ctx.eval::<f64, _>("f(50000,0)"))
        .unwrap();
    assert_eq!(first, 1_249_975_000.0);
    for _ in 0..10_000 {
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    let last = context.with(|ctx| ctx.eval::<f64, _>("f(2000,0)")).unwrap();
    assert_eq!(last, 1_999_000.0);
    jit.poll();
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());
    assert!(jit.metrics().boxes_elided > 0, "{:?}", jit.metrics());
    assert_eq!(
        jit.test_last_acquired_artifact_key().unwrap().tier,
        rquickjs_jit::runtime::Tier::Optimizing
    );

    let before_strict_exits = jit.metrics();
    let overflowed = context
        .with(|ctx| ctx.eval::<f64, _>("f(100000,0)"))
        .unwrap();
    assert_eq!(overflowed, 4_999_950_000.0);
    jit.poll();
    let mixed = context.with(|ctx| ctx.eval::<f64, _>("f(2,0.5)")).unwrap();
    assert_eq!(mixed, 2.5);
    jit.poll();
    let after_strict_exits = jit.metrics();
    assert!(
        after_strict_exits.deopts >= before_strict_exits.deopts + 2,
        "{after_strict_exits:?}"
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn stable_int32_loop_waits_for_and_installs_a_bounded_raw_i32_version() {
    use rquickjs::{Context, Function, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function genericLoop(n,z){let s=z;for(let i=z;i<n;i++)s=s+i;return s}",
            )
        })
        .unwrap();

    for _ in 0..10_000 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("genericLoop").unwrap();
            function.call::<_, f64>((2_000, 0)).unwrap()
        });
        assert_eq!(result, 1_999_000.0);
        jit.poll();
        if jit.metrics().tier2_entries > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());
    assert_ne!(
        jit.test_last_acquired_artifact_key()
            .unwrap()
            .specialization_fingerprint,
        0,
        "production numeric Tier2 must carry its bounded feedback signature"
    );

    let before = jit.metrics();
    for _ in 0..10 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("genericLoop").unwrap();
            function.call::<_, f64>((2_000, 0)).unwrap()
        });
        assert_eq!(result, 1_999_000.0);
        jit.poll();
    }
    let after = jit.metrics();
    assert_eq!(after.deopts, before.deopts, "{after:?}");
    assert!(
        after.tier2_entries >= before.tier2_entries + 10,
        "{after:?}"
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn iterative_fibonacci_enters_tier2_with_multi_local_loop_phis() {
    use rquickjs::{Context, Function, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function fib(_iterations,seed){let a=seed;let b=1;for(let i=seed;i<40;i++){const next=a+b;a=b;b=next;}return a}",
            )
        })
        .unwrap();

    for _ in 0..10_000 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("fib").unwrap();
            function.call::<_, i32>((2_000, 0)).unwrap()
        });
        assert_eq!(result, 102_334_155);
        jit.poll();
        if jit.metrics().tier2_entries > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());

    let before = jit.metrics();
    for _ in 0..10 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("fib").unwrap();
            function.call::<_, i32>((2_000, 0)).unwrap()
        });
        assert_eq!(result, 102_334_155);
        jit.poll();
    }
    let after = jit.metrics();
    assert_eq!(after.deopts, before.deopts, "{after:?}");
    assert!(
        after.tier2_entries >= before.tier2_entries + 10,
        "{after:?}"
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn automatic_profitability_blacklist_unpublishes_harmful_baseline() {
    use rquickjs::{Context, Function, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| ctx.eval::<(), _>("function tiny(a,b){return a+b}"))
        .unwrap();

    for _ in 0..20_000 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("tiny").unwrap();
            function.call::<_, i32>((20, 22)).unwrap()
        });
        assert_eq!(result, 42);
        jit.poll();
        if jit.metrics().interpreter_demotions > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    let demoted = jit.metrics();
    assert_eq!(demoted.interpreter_demotions, 1, "{demoted:?}");
    assert!(demoted.profitability_rejected >= 5, "{demoted:?}");

    let native_before = demoted.native_entries;
    for _ in 0..20 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("tiny").unwrap();
            function.call::<_, i32>((20, 22)).unwrap()
        });
        assert_eq!(result, 42);
        jit.poll();
    }
    assert_eq!(
        jit.metrics().native_entries,
        native_before,
        "{:?}",
        jit.metrics()
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn automatic_gpui_layout_kernel_enters_tier2_after_harmful_baseline_demotion() {
    use rquickjs::{Context, Function, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(1)
            .loop_threshold(1)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function layoutKernel(batches,seed){\
                 let checksum=seed;\
                 for(let batch=0;batch<batches;batch+=1){\
                   let a=0;let b=1;\
                   for(let i=0;i<40;i+=1){const next=a+b;a=b;b=next;}\
                   checksum=b;\
                 }\
                 return checksum;\
                 }\
                 function terminalHost(value){return JSON.stringify({value:value,kind:'panel'});}",
            )
        })
        .unwrap();

    let mut saw_demotion = false;
    for _ in 0..64 {
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get("layoutKernel").unwrap();
            let result = function.call::<_, i32>((2_000, 0)).unwrap();
            let terminal: Function = ctx.globals().get("terminalHost").unwrap();
            assert_eq!(
                terminal.call::<_, String>((42,)).unwrap(),
                "{\"value\":42,\"kind\":\"panel\"}"
            );
            result
        });
        assert_eq!(result, 165_580_141);
        jit.poll();
        let metrics = jit.metrics();
        saw_demotion |= metrics.interpreter_demotions > 0;
        if saw_demotion && metrics.tier2_entries > 0 {
            assert!(metrics.profitability_rejected >= 5, "{metrics:?}");
            assert_eq!(metrics.deopts, 0, "{metrics:?}");
            assert!(
                metrics.queued >= 3,
                "Tier2 trial was not queued: {metrics:?}"
            );
            assert!(
                metrics.compile_failures > 0,
                "terminal host function must remain unsupported without blocking the profitable kernel: {metrics:?}"
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    panic!(
        "harmful Tier1 never reached bounded Tier2 trial: {:?}",
        jit.metrics()
    );
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn stable_float64_loop_stays_native_without_a_side_path() {
    use rquickjs::{Context, Runtime};
    use rquickjs_jit::{Jit, JitConfig};

    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(4)
            .force_optimized_for_test(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function stableFloat(n,z){let s=z;for(let i=z;i<n;i++)s=s+i;return s}",
            )
        })
        .unwrap();
    for _ in 0..8 {
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<f64, _>("stableFloat(100.5,0.5)"))
                .unwrap(),
            5000.5
        );
    }
    for _ in 0..10_000 {
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().installed >= 2, "{:?}", jit.metrics());
    let before = jit.metrics();
    for _ in 0..10 {
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<f64, _>("stableFloat(100000.5,0.5)"))
                .unwrap(),
            5_000_000_000.5
        );
        jit.poll();
    }
    let after_large = jit.metrics();
    assert_eq!(after_large.deopts, before.deopts, "{after_large:?}");
    assert_eq!(
        after_large.stable_path_compile_requests, before.stable_path_compile_requests,
        "{after_large:?}"
    );
    assert_eq!(after_large.side_path_entries, before.side_path_entries);
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<f64, _>("stableFloat(0.5,0.5)"))
            .unwrap(),
        0.5
    );
    jit.poll();
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<f64, _>("stableFloat(100000.5,0.5)"))
            .unwrap(),
        5_000_000_000.5
    );
    jit.poll();
    let after = jit.metrics();
    assert_eq!(after.deopts, before.deopts, "{after:?}");
    assert_eq!(
        after.side_path_entries, before.side_path_entries,
        "{after:?}"
    );
}

#[test]
fn worker_snapshot_contains_stable_tokens_not_runtime_pointers() {
    let function = FunctionKey::new(9, 4);
    let mut table = FeedbackTable::new(8, 3);
    table.observe_type(function, 21, FeedbackKind::Value, ObservedType::Int32);
    let snapshot = table.snapshot(17);

    assert_eq!(snapshot.epoch(), 17);
    assert_eq!(snapshot.entries()[0].function(), function);
    assert_eq!(snapshot.entries()[0].pc(), 21);
    assert_eq!(snapshot.entries()[0].observations(), &[ObservedType::Int32]);
}

#[test]
fn stable_side_exit_reaches_recompile_threshold_and_unstable_exit_demotes_with_backoff() {
    let key = FunctionKey::new(88, 1);
    let mut coordinator = Coordinator::with_limits(4, 4, 4, 1 << 20);
    for _ in 0..9 {
        assert_eq!(
            coordinator.record_optimized_side_exit(key, 7),
            SideExitAction::Counted
        );
    }
    assert_eq!(
        coordinator.record_optimized_side_exit(key, 7),
        SideExitAction::StablePathThreshold
    );
    coordinator.advance_clock(50);
    let SideExitAction::Demote { retry_after } = coordinator.record_optimized_side_exit(key, 8)
    else {
        panic!("a second guard is unstable")
    };
    assert!(retry_after > 50);
    assert_eq!(coordinator.metrics().optimized_demotions, 1);
}

#[test]
fn stable_side_path_request_owns_exact_guard_profile_without_unloading_target() {
    use rquickjs_jit::runtime::{GuardId, SidePathProfile};
    let fixture = SnapshotFixture::compile("(function(a,b){return a-b})");
    let snapshot = fixture.snapshot();
    let key = FunctionKey::new(snapshot.function_id(), snapshot.generation());
    let verified = snapshot.verify(VerifyLimits::default()).unwrap();
    let mut feedback = FeedbackTable::new(8, 2);
    feedback.observe_type(key, 7, FeedbackKind::Exit, ObservedType::Float64);
    let frozen = feedback.snapshot(44);
    let mut coordinator = Coordinator::with_limits(4, 4, 4, 1 << 20);

    // A side-path request is a specialization of an already installed target.
    // The test helper installs the baseline and optimizing generations without
    // involving a worker; queuing must leave the optimizing pin available.
    coordinator
        .queue(key, Tier::Baseline, verified.clone())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: baseline.artifact_key(),
        attempt_id: baseline.attempt_id(),
        result: Ok(CompiledArtifact::empty(baseline.artifact_key())),
    });
    coordinator
        .queue_with_feedback(key, Tier::Optimizing, verified.clone(), frozen.clone())
        .unwrap();
    let optimizing = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Optimizing,
        artifact_key: optimizing.artifact_key(),
        attempt_id: optimizing.attempt_id(),
        result: Ok(CompiledArtifact::empty(optimizing.artifact_key())),
    });
    let before = coordinator.pin(key, Tier::Optimizing).unwrap();
    let profile = SidePathProfile::new(key, GuardId::new(7), 7, ObservedType::Float64, 44);
    coordinator
        .queue_side_path(key, verified, frozen, profile)
        .unwrap();
    assert!(coordinator.pin(key, Tier::Optimizing).is_some());
    assert_eq!(before.key().generation, key.generation);

    let request = coordinator.begin_next().unwrap();
    assert_eq!(request.side_path_profile(), Some(profile));
    assert_ne!(request.artifact_key().specialization_fingerprint, 0);
}

#[test]
fn primary_optimized_artifact_is_keyed_by_the_bounded_feedback_signature() {
    let fixture = SnapshotFixture::compile("(function(a,b){return a+b})");
    let snapshot = fixture.snapshot();
    let key = FunctionKey::new(snapshot.function_id(), snapshot.generation());
    let verified = snapshot.verify(VerifyLimits::default()).unwrap();
    let add_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "add")
        .unwrap()
        .pc();
    let return_pc = verified
        .instructions()
        .iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .unwrap()
        .pc();
    let mut feedback = FeedbackTable::new(8, 2);
    feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
    feedback.observe_binary(
        key,
        add_pc,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        Default::default(),
    );
    feedback.observe_return(key, return_pc, ObservedType::Int32);
    let frozen = feedback.snapshot(77);
    let expected = frozen.bounded_specialization(key).unwrap().fingerprint();
    let mut coordinator = Coordinator::with_limits(4, 4, 4, 1 << 20);
    coordinator
        .queue(key, Tier::Baseline, verified.clone())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: baseline.artifact_key(),
        attempt_id: baseline.attempt_id(),
        result: Ok(CompiledArtifact::empty(baseline.artifact_key())),
    });

    coordinator
        .queue_with_feedback(key, Tier::Optimizing, verified, frozen)
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    assert_eq!(request.artifact_key().specialization_fingerprint, expected);
}

#[test]
fn side_path_queue_rejects_stale_or_mismatched_profiles() {
    use rquickjs_jit::runtime::{GuardId, SidePathProfile};
    let fixture = SnapshotFixture::compile("(function(a,b){return a-b})");
    let snapshot = fixture.snapshot();
    let key = FunctionKey::new(snapshot.function_id(), snapshot.generation());
    let verified = snapshot.verify(VerifyLimits::default()).unwrap();
    let mut coordinator = Coordinator::with_limits(4, 4, 4, 1 << 20);
    coordinator
        .queue(key, Tier::Baseline, verified.clone())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: baseline.artifact_key(),
        attempt_id: baseline.attempt_id(),
        result: Ok(CompiledArtifact::empty(baseline.artifact_key())),
    });
    let mut initial = FeedbackTable::new(8, 2);
    initial.observe_type(key, 0, FeedbackKind::Value, ObservedType::Int32);
    coordinator
        .queue_with_feedback(key, Tier::Optimizing, verified.clone(), initial.snapshot(3))
        .unwrap();
    let optimizing = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Optimizing,
        artifact_key: optimizing.artifact_key(),
        attempt_id: optimizing.attempt_id(),
        result: Ok(CompiledArtifact::empty(optimizing.artifact_key())),
    });
    let feedback = FeedbackSnapshot::empty(5);
    let stale = SidePathProfile::new(
        FunctionKey::new(key.id, key.generation + 1),
        GuardId::new(0),
        0,
        ObservedType::String,
        5,
    );
    assert!(coordinator
        .queue_side_path(key, verified, feedback, stale)
        .is_err());
}

#[test]
fn guard_specific_float_side_path_changes_machine_guard_and_preserves_profile() {
    use rquickjs_jit::runtime::{GuardId, SidePathProfile};
    let fixture =
        SnapshotFixture::compile("(function(n,z){let s=z;for(let i=z;i<n;i++)s=s+i;return s})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let key = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let ir = OptimizedIr::translate(&verified, 55).unwrap();
    let loop_guard = ir
        .guard_maps()
        .iter()
        .find(|site| site.guard() != 0)
        .unwrap();
    let profile = SidePathProfile::new(
        key,
        GuardId::new(loop_guard.guard()),
        loop_guard.map().resume_pc(),
        ObservedType::Float64,
        55,
    );
    let compiler = Tier2Compiler::host(55);
    let generic = compiler.lower_for_test(&verified, 55).unwrap();
    let specialized = compiler
        .lower_side_path_for_test(&verified, 55, profile)
        .unwrap();
    assert_ne!(
        generic, specialized,
        "side path must alter the selected guard block"
    );
    assert!(specialized.matches("brif").count() > generic.matches("brif").count());
}
