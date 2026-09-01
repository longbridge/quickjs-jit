#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use rquickjs::{Context, Function, Runtime};
use rquickjs_jit::{Jit, JitConfig};

#[test]
fn real_quickjs_numeric_snapshots_lower_with_matching_feedback() {
    use rquickjs_jit::{
        bytecode::VerifyLimits,
        compiler::optimized::Tier2Compiler,
        runtime::{BinaryFeedbackFlags, FeedbackTable, FunctionKey, ObservedType},
        test_support::SnapshotFixture,
    };

    for operator in ["-", "*", "/"] {
        let fixture = SnapshotFixture::compile(&format!("(function(a,b){{return a{operator}b}})"));
        let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
        let arithmetic_pc = verified
            .instructions()
            .iter()
            .find(|instruction| matches!(instruction.opcode().name(), "sub" | "mul" | "div"))
            .unwrap()
            .pc();
        let key = FunctionKey::new(1, 1);
        let mut feedback = FeedbackTable::new(4, 4);
        for _ in 0..8 {
            feedback.observe_call(key, &[ObservedType::Int32, ObservedType::Int32]);
            feedback.observe_binary(
                key,
                arithmetic_pc,
                ObservedType::Int32,
                ObservedType::Int32,
                ObservedType::Int32,
                BinaryFeedbackFlags::NONE,
            );
            feedback.observe_return(key, arithmetic_pc + 1, ObservedType::Int32);
        }
        let snapshot = feedback.snapshot(1);
        Tier2Compiler::host(snapshot.epoch())
            .lower_with_feedback_for_test(&verified, key, &snapshot)
            .unwrap_or_else(|error| panic!("real {operator} snapshot did not lower: {error:?}"));
    }
}

fn with_optimized_binary<T>(
    body: &str,
    warm_lhs: T,
    warm_rhs: T,
    check: impl FnOnce(&Context, &Jit),
) where
    T: Copy + for<'js> rquickjs::IntoJs<'js>,
{
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
        .with(|ctx| ctx.eval::<(), _>(format!("function f(a,b){{return {body}}}")))
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            let _: f64 = function.call((warm_lhs, warm_rhs)).unwrap();
        });
        jit.poll();
        if jit.metrics().tier2_entries > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());
    check(&context, &jit);
}

#[test]
fn checked_int32_subtraction_deopts_to_the_exact_overflow_result() {
    with_optimized_binary("a-b", 20_i32, 3_i32, |context, jit| {
        let before = jit.metrics();
        let result = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((i32::MIN, 1_i32)).unwrap()
        });
        assert_eq!(result, -2_147_483_649.0);
        assert!(jit.metrics().deopts > before.deopts);
    });
}

#[test]
fn checked_int32_multiply_deopts_for_javascript_negative_zero() {
    with_optimized_binary("a*b", 6_i32, 7_i32, |context, jit| {
        let before = jit.metrics();
        let result = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((0_i32, -1_i32)).unwrap()
        });
        assert_eq!(result.to_bits(), (-0.0_f64).to_bits());
        assert!(jit.metrics().deopts > before.deopts);
    });
}

#[test]
fn checked_int32_division_deopts_for_fraction_and_zero_divisor() {
    with_optimized_binary("a/b", 84_i32, 2_i32, |context, jit| {
        let before = jit.metrics();
        let fraction = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((7_i32, 2_i32)).unwrap()
        });
        let infinity = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((1_i32, 0_i32)).unwrap()
        });
        assert_eq!(fraction, 3.5);
        assert_eq!(infinity, f64::INFINITY);
        assert!(jit.metrics().deopts >= before.deopts + 2);
    });
}

#[test]
fn stable_float64_add_executes_natively_and_preserves_nan_and_infinity() {
    with_optimized_binary("a+b", 1.25_f64, 2.5_f64, |context, jit| {
        let before = jit.metrics();
        let infinity = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((f64::INFINITY, 1.5_f64)).unwrap()
        });
        let nan = context.with(|ctx| {
            let function: Function<'_> = ctx.globals().get("f").unwrap();
            function.call::<_, f64>((f64::NAN, 1.5_f64)).unwrap()
        });
        assert_eq!(infinity, f64::INFINITY);
        assert!(nan.is_nan());
        let after = jit.metrics();
        assert!(after.tier2_entries >= before.tier2_entries + 2, "{after:?}");
        assert_eq!(after.deopts, before.deopts, "{after:?}");
    });
}
