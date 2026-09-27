//! Scalar-loop code quality: per-loop poll amortization, exit
//! rematerialization, constant-branch folding and constant unsigned shifts.
//! Generated-code assertions establish the shape; runtime tests establish
//! that the cheaper code keeps exact interpreter semantics, deopts and
//! interrupt service.
#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_endian = "little",
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use rquickjs::{Context, Function, Runtime};
use rquickjs_jit::{
    bytecode::VerifyLimits, compiler::optimized::Tier2Compiler, test_support::SnapshotFixture, Jit,
    JitConfig,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

/// Calls before and after the loops rule out whole-function poll
/// amortization; the loop nest itself is pure Int32 arithmetic.
const INT_ARITH: &str = "function arith(iterations){\
     let total=0;\
     const outer=Math.max(1,iterations>>7);\
     for(let j=0;j<outer;j++){let sum=0;for(let i=0;i<1000;i++)sum+=i*i;total+=sum;}\
     return String(total)}";

fn expected_arith(iterations: i64) -> String {
    let outer = (iterations >> 7).max(1);
    let sum: i64 = (0..1000).map(|i| i * i).sum();
    (outer * sum).to_string()
}

fn tier2_clif(source: &str) -> String {
    let fixture = SnapshotFixture::compile(source);
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    Tier2Compiler::host(91)
        .lower_for_test(&verified, 91)
        .unwrap_or_else(|error| panic!("{source} did not lower: {error:?}"))
}

fn countdowns(clif: &str) -> usize {
    clif.lines()
        .filter(|line| {
            let line = line.split("  ;").next().unwrap_or(line).trim();
            line.contains("iadd_imm") && line.ends_with(", -1")
        })
        .count()
}

#[test]
fn a_pure_loop_counts_down_its_poll_even_when_the_function_calls() {
    let clif = tier2_clif(&format!(
        "({})",
        INT_ARITH.replace("function arith", "function")
    ));
    assert!(
        clif.contains("call_indirect"),
        "fixture must keep generic calls outside the loops: {clif}"
    );
    // Both the outer and the inner loop are call-free, so each header polls
    // through its own countdown instead of calling the runtime every trip.
    assert!(countdowns(&clif) >= 2, "{clif}");
}

#[test]
fn a_loop_containing_a_call_keeps_its_per_iteration_poll() {
    let clif = tier2_clif("(function(n){let s=0;for(let i=0;i<n;i++)s=s+Math.abs(i);return s})");
    assert_eq!(countdowns(&clif), 0, "{clif}");
}

struct Warm {
    runtime: Runtime,
    jit: Jit,
    context: Context,
}

fn warm_tier2(source: &str, name: &str, argument: i32, expected: &str) -> Warm {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(16)
            .force_optimized_for_test(true)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| ctx.eval::<(), _>(source)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let before = jit.metrics();
        let result = context.with(|ctx| {
            let function: Function = ctx.globals().get(name).unwrap();
            function
                .call::<_, rquickjs::Value>((argument,))
                .map(|value| {
                    value
                        .as_string()
                        .map(|string| string.to_string().unwrap())
                        .or_else(|| value.as_number().map(|number| number.to_string()))
                        .unwrap()
                })
        });
        assert_eq!(result.unwrap(), expected);
        jit.poll();
        if jit.metrics().tier2_entries > before.tier2_entries {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{name} never entered Tier 2: {:?}",
            jit.metrics()
        );
        std::thread::sleep(Duration::from_micros(200));
    }
    Warm {
        runtime,
        jit,
        context,
    }
}

#[test]
fn per_loop_amortized_poll_keeps_results_and_interrupt_service() {
    let warm = warm_tier2(INT_ARITH, "arith", 1 << 12, &expected_arith(1 << 12));
    // Steady state: exact results across trip counts, all in Tier 2.
    for iterations in [0, 127, 128, 5000, 1 << 14] {
        let before = warm.jit.metrics();
        let actual = warm.context.with(|ctx| {
            let function: Function = ctx.globals().get("arith").unwrap();
            function.call::<_, String>((iterations,)).unwrap()
        });
        assert_eq!(actual, expected_arith(i64::from(iterations)));
        assert!(warm.jit.metrics().tier2_entries > before.tier2_entries);
    }
    // A (practically) unbounded loop nest must still reach the runtime poll
    // and unwind through the uncatchable interrupt.
    let interrupts = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&interrupts);
    warm.runtime.set_interrupt_handler(Some(Box::new(move || {
        seen.fetch_add(1, Ordering::SeqCst) >= 2
    })));
    let before = warm.jit.metrics();
    warm.context.with(|ctx| {
        let function: Function = ctx.globals().get("arith").unwrap();
        assert!(function.call::<_, String>((i32::MAX,)).is_err());
        drop(ctx.catch());
    });
    warm.runtime.set_interrupt_handler(None);
    let after = warm.jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries);
    assert!(interrupts.load(Ordering::SeqCst) >= 3);
    assert_eq!(after.native_entries, after.native_exits);
    let again = warm.context.with(|ctx| {
        let function: Function = ctx.globals().get("arith").unwrap();
        function.call::<_, String>((300,)).unwrap()
    });
    assert_eq!(again, expected_arith(300));
}

#[test]
fn scalar_loop_overflow_deopt_restores_the_exact_frame() {
    // The overflow exit reloads the frame buffers and exit record instead of
    // keeping the prologue values live; the interpreter must resume with the
    // exact pre-overflow sum and induction variable.
    let warm = warm_tier2(
        "function sum(n){let s=0;for(let i=0;i<n;i++)s=s+i;return s}",
        "sum",
        1000,
        "499500",
    );
    let before = warm.jit.metrics();
    let overflowing = warm.context.with(|ctx| {
        let function: Function = ctx.globals().get("sum").unwrap();
        function.call::<_, f64>((100_000,)).unwrap()
    });
    assert_eq!(overflowing, 4_999_950_000.0);
    let after = warm.jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries);
    assert!(after.deopts > before.deopts, "{after:?}");
    assert_eq!(after.native_entries, after.native_exits);
}

#[test]
fn constant_unsigned_shift_stays_int32_in_a_mixed_loop() {
    // `v >>> 3` of a negative Int32 is a large positive Int32; the xor must
    // consume it exactly without a Float64 detour.
    let source = "function bits(n){let v=-123456789;\
                  for(let i=0;i<n;i++)v=((v<<5)^(v>>>3)^i)|0;\
                  return v}";
    let expected = |n: i32| {
        let mut v: i32 = -123_456_789;
        for i in 0..n {
            v = v.wrapping_shl(5) ^ ((v as u32 >> 3) as i32) ^ i;
        }
        v
    };
    let warm = warm_tier2(source, "bits", 300, &expected(300).to_string());
    for n in [0, 1, 7, 1000, 4097] {
        let before = warm.jit.metrics();
        let actual = warm.context.with(|ctx| {
            let function: Function = ctx.globals().get("bits").unwrap();
            function.call::<_, i32>((n,)).unwrap()
        });
        assert_eq!(actual, expected(n));
        assert!(warm.jit.metrics().tier2_entries > before.tier2_entries);
    }
    assert_eq!(
        warm.jit.metrics().native_entries,
        warm.jit.metrics().native_exits
    );
}
