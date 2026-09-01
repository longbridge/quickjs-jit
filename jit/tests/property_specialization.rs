#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use rquickjs_jit::{
    bytecode::VerifyLimits,
    compiler::optimized::Tier2Compiler,
    runtime::{
        FeedbackSnapshot, FunctionKey, ObservedType, PropertyAttributes, PrototypeDependencyToken,
        ShapeFeedbackTable, ShapeObservation, ShapeToken,
    },
    test_support::SnapshotFixture,
};

#[test]
fn production_property_specialization_survives_shape_mutation_and_stress_gc() {
    use rquickjs::{Context, Function, Object, Runtime};
    use rquickjs_jit::{Jit, JitConfig};
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .force_optimized_for_test(true)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>("globalThis.shared={answer:41};function f(o){return o.answer}")
        })
        .unwrap();
    for _ in 0..128 {
        context.with(|ctx| {
            let f: Function = ctx.globals().get("f").unwrap();
            let o: Object = ctx.globals().get("shared").unwrap();
            assert_eq!(f.call::<_, i32>((o,)).unwrap(), 41);
        });
        jit.poll();
        std::thread::yield_now();
    }
    for _ in 0..10_000 {
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    context.with(|ctx| {
        let f: Function = ctx.globals().get("f").unwrap();
        let o: Object = ctx.globals().get("shared").unwrap();
        assert_eq!(f.call::<_, i32>((o,)).unwrap(), 41);
    });
    jit.poll();
    assert!(jit.metrics().tier2_entries > 0, "{:?}", jit.metrics());
    let before = jit.metrics();
    context
        .with(|ctx| ctx.eval::<(), _>("shared.extra=1;shared.answer=42"))
        .unwrap();
    for _ in 0..1_024 {
        context.with(|ctx| {
            let f: Function = ctx.globals().get("f").unwrap();
            let o: Object = ctx.globals().get("shared").unwrap();
            assert_eq!(f.call::<_, i32>((o,)).unwrap(), 42);
        });
        jit.poll();
    }
    assert!(jit.metrics().deopts > before.deopts, "{:?}", jit.metrics());
}

#[test]
fn production_bounded_polymorphic_property_hits_each_layout_without_deopt() {
    use rquickjs::{Context, Function, Object, Runtime};
    use rquickjs_jit::{Jit, JitConfig};
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(64)
            .force_optimized_for_test(true)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.left={answer:41};\
                 globalThis.right={padding:1,answer:42};\
                 function poly(o){return o.answer}",
            )
        })
        .unwrap();
    // First install only the baseline. Property feedback is emitted by that
    // native tier, so collect both layouts before the next maintenance poll is
    // allowed to snapshot and queue Tier2.
    for index in 0..10_000 {
        context.with(|ctx| {
            let f: Function = ctx.globals().get("poly").unwrap();
            let o: Object = ctx
                .globals()
                .get(if index & 1 == 0 { "left" } else { "right" })
                .unwrap();
            let result = f.call::<_, i32>((o,));
            assert!(result.is_ok(), "{result:?}; catch={:?}", ctx.catch());
            assert_eq!(result.unwrap(), 41 + (index & 1));
        });
        jit.poll();
        if jit.metrics().installed > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    assert!(jit.metrics().installed > 0, "{:?}", jit.metrics());
    assert_eq!(jit.metrics().tier2_entries, 0, "{:?}", jit.metrics());
    for index in 0..128 {
        context.with(|ctx| {
            let f: Function = ctx.globals().get("poly").unwrap();
            let o: Object = ctx
                .globals()
                .get(if index & 1 == 0 { "left" } else { "right" })
                .unwrap();
            let result = f.call::<_, i32>((o,));
            assert!(result.is_ok(), "{result:?}; catch={:?}", ctx.catch());
            assert_eq!(result.unwrap(), 41 + (index & 1));
        });
    }
    for index in 0..10_000 {
        jit.poll();
        context.with(|ctx| {
            let f: Function = ctx.globals().get("poly").unwrap();
            let o: Object = ctx
                .globals()
                .get(if index & 1 == 0 { "left" } else { "right" })
                .unwrap();
            let result = f.call::<_, i32>((o,));
            assert!(result.is_ok(), "{result:?}; catch={:?}", ctx.catch());
            assert_eq!(result.unwrap(), 41 + (index & 1));
        });
        if jit.metrics().tier2_entries > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    let before = jit.metrics();
    assert!(before.tier2_entries > 0, "{before:?}");
    for index in 0..1_024 {
        context.with(|ctx| {
            let f: Function = ctx.globals().get("poly").unwrap();
            let o: Object = ctx
                .globals()
                .get(if index & 1 == 0 { "left" } else { "right" })
                .unwrap();
            assert_eq!(f.call::<_, i32>((o,)).unwrap(), 41 + (index & 1));
        });
        jit.poll();
    }
    let after = jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries, "{after:?}");
    assert_eq!(after.deopts, before.deopts, "{before:?} -> {after:?}");
}

#[test]
fn guarded_own_primitive_property_lowers_with_owned_deopt_bridge() {
    let fixture = SnapshotFixture::compile("(function(o){return o.answer})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let pc = verified
        .instructions()
        .iter()
        .find(|i| i.opcode().name() == "get_field")
        .unwrap()
        .pc();
    let key = FunctionKey::new(1, 1);
    let mut table = ShapeFeedbackTable::new(3);
    table.observe(
        key,
        pc,
        ShapeObservation::new(
            ShapeToken::new(0x1000, 7),
            PrototypeDependencyToken::new(0, 0),
            1,
            PropertyAttributes::WRITABLE,
            ObservedType::Int32,
        ),
    );
    let feedback = FeedbackSnapshot::empty(1).with_properties(table.snapshot(key));
    let clif = Tier2Compiler::host(1)
        .lower_with_feedback_for_test(&verified, key, &feedback)
        .expect("owned property bridge");
    assert!(clif.contains("call_indirect"), "{clif}");
}

#[test]
fn bounded_polymorphic_property_emits_one_shape_guard_per_layout() {
    let fixture = SnapshotFixture::compile("(function(o){return o.answer})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let pc = verified
        .instructions()
        .iter()
        .find(|i| i.opcode().name() == "get_field")
        .unwrap()
        .pc();
    let key = FunctionKey::new(1, 1);
    let compile = |observations: &[(u64, u32)]| {
        let mut table = ShapeFeedbackTable::new(3);
        for &(identity, offset) in observations {
            table.observe(
                key,
                pc,
                ShapeObservation::new(
                    ShapeToken::new(identity, 7),
                    PrototypeDependencyToken::new(0, 0),
                    offset,
                    PropertyAttributes::WRITABLE,
                    ObservedType::Int32,
                ),
            );
        }
        Tier2Compiler::host(1)
            .lower_with_feedback_for_test(
                &verified,
                key,
                &FeedbackSnapshot::empty(1).with_properties(table.snapshot(key)),
            )
            .unwrap()
    };
    let monomorphic = compile(&[(0x1000, 1)]);
    let polymorphic = compile(&[(0x1000, 1), (0x2000, 3), (0x3000, 2)]);
    let call_width = if cfg!(rquickjs_memory_sanitizer) {
        4
    } else {
        1
    };
    assert_eq!(
        polymorphic.matches("call_indirect").count(),
        monomorphic.matches("call_indirect").count() + 2 * call_width,
        "{polymorphic}"
    );
    assert!(polymorphic.contains("iconst.i32 8192"), "{polymorphic}");
    assert!(polymorphic.contains("iconst.i32 0x3000"), "{polymorphic}");
}

#[test]
fn bounded_polymorphic_primitive_store_emits_a_guard_chain_and_raw_stores() {
    let fixture = SnapshotFixture::compile("(function(o,v){o.answer=v;return v})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let pc = verified
        .instructions()
        .iter()
        .find(|i| i.opcode().name() == "put_field")
        .unwrap()
        .pc();
    let key = FunctionKey::new(1, 1);
    let mut table = ShapeFeedbackTable::new(3);
    for (identity, offset) in [(0x1000, 0), (0x2000, 2)] {
        table.observe(
            key,
            pc,
            ShapeObservation::new(
                ShapeToken::new(identity, 7),
                PrototypeDependencyToken::new(0, 0),
                offset,
                PropertyAttributes::WRITABLE,
                ObservedType::Int32,
            ),
        );
    }
    let clif = Tier2Compiler::host(1)
        .lower_with_feedback_for_test(
            &verified,
            key,
            &FeedbackSnapshot::empty(1).with_properties(table.snapshot(key)),
        )
        .expect("bounded primitive store PIC");
    // Two owner materializations + two SHAPE_GUARD calls + two balanced FREEs.
    let call_width = if cfg!(rquickjs_memory_sanitizer) {
        4
    } else {
        1
    };
    assert_eq!(
        clif.matches("call_indirect").count(),
        6 * call_width,
        "{clif}"
    );
    assert!(clif.matches("store.i64").count() >= 4, "{clif}");
    assert!(clif.contains("iconst.i32 8192"), "{clif}");
}

#[test]
fn megamorphic_property_site_fails_closed_to_the_generic_tier() {
    let fixture = SnapshotFixture::compile("(function(o){return o.answer})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let pc = verified
        .instructions()
        .iter()
        .find(|i| i.opcode().name() == "get_field")
        .unwrap()
        .pc();
    let key = FunctionKey::new(1, 1);
    let mut table = ShapeFeedbackTable::new(3);
    for identity in 1..=4 {
        table.observe(
            key,
            pc,
            ShapeObservation::new(
                ShapeToken::new(identity, 1),
                PrototypeDependencyToken::new(0, 0),
                identity as u32,
                PropertyAttributes::WRITABLE,
                ObservedType::Int32,
            ),
        );
    }
    assert!(Tier2Compiler::host(1)
        .lower_with_feedback_for_test(
            &verified,
            key,
            &FeedbackSnapshot::empty(1).with_properties(table.snapshot(key)),
        )
        .is_err());
}

#[test]
fn inherited_accessor_and_refcounted_values_fail_closed() {
    let fixture = SnapshotFixture::compile("(function(o){return o.answer})");
    let verified = fixture.snapshot().verify(VerifyLimits::default()).unwrap();
    let pc = verified
        .instructions()
        .iter()
        .find(|i| i.opcode().name() == "get_field")
        .unwrap()
        .pc();
    for (prototype, attrs, value) in [
        (
            PrototypeDependencyToken::new(2, 1),
            PropertyAttributes::WRITABLE,
            ObservedType::Int32,
        ),
        (
            PrototypeDependencyToken::new(0, 0),
            PropertyAttributes::ACCESSOR,
            ObservedType::Int32,
        ),
        (
            PrototypeDependencyToken::new(0, 0),
            PropertyAttributes::WRITABLE,
            ObservedType::Object,
        ),
    ] {
        let key = FunctionKey::new(1, 1);
        let mut table = ShapeFeedbackTable::new(3);
        table.observe(
            key,
            pc,
            ShapeObservation::new(ShapeToken::new(0x1000, 7), prototype, 1, attrs, value),
        );
        let feedback = FeedbackSnapshot::empty(1).with_properties(table.snapshot(key));
        assert!(Tier2Compiler::host(1)
            .lower_with_feedback_for_test(&verified, key, &feedback)
            .is_err());
    }
}
