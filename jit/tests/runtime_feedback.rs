use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rquickjs::{Context, Runtime};
use rquickjs_core::{
    qjs,
    runtime::{JitBackend, RuntimeJitGuard},
};
use rquickjs_jit::runtime::{
    BinaryFeedbackFlags, FeedbackRepresentation, FeedbackState, FeedbackTable, FunctionKey,
    ObservedType, MAX_SPECIALIZED_ARGUMENTS,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Captured {
    kind: u32,
    function: (u64, u64),
    callee: (u64, u64),
    pc: u32,
    slot: u32,
    types: Vec<u32>,
    flags: u32,
    shape: (u64, u64),
    prototype: (u64, u64),
    property_offset: u32,
    property_attributes: u32,
}

struct CaptureBackend(Arc<Mutex<Vec<Captured>>>);

unsafe impl JitBackend for CaptureBackend {
    fn record_feedback(&mut self, event: &qjs::JSJitFeedbackEvent) {
        let types = if event.type_count == 0 {
            Vec::new()
        } else {
            assert!(!event.types.is_null());
            unsafe { std::slice::from_raw_parts(event.types, event.type_count as usize) }.to_vec()
        };
        self.0.lock().unwrap().push(Captured {
            kind: event.kind,
            function: (event.function.id, event.function.generation),
            callee: (event.callee.id, event.callee.generation),
            pc: event.pc,
            slot: event.slot,
            types,
            flags: event.flags,
            shape: (event.shape_identity, event.shape_generation),
            prototype: (event.prototype_identity, event.prototype_generation),
            property_offset: event.property_offset,
            property_attributes: event.property_attributes,
        });
    }
}

struct DisableFeedbackBackend {
    hot: Arc<AtomicU64>,
    feedback: Arc<AtomicU64>,
    acquire: Arc<AtomicU64>,
}

unsafe impl JitBackend for DisableFeedbackBackend {
    fn record_hot(&mut self, _event: &qjs::JSJitHotEvent) -> u32 {
        self.hot.fetch_add(1, Ordering::Relaxed);
        2
    }

    fn record_feedback(&mut self, _event: &qjs::JSJitFeedbackEvent) {
        self.feedback.fetch_add(1, Ordering::Relaxed);
    }

    fn acquire_entry(&mut self, _id: u64, _generation: u64, _pc: u32) -> qjs::JSJitEntryHandle {
        self.acquire.fetch_add(1, Ordering::Relaxed);
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: None,
            pin: std::ptr::null_mut(),
            stack_map_count: 0,
            helper_abi_version: 0,
        }
    }
}

#[test]
fn terminal_backend_response_disables_hot_and_feedback_callbacks_in_quickjs() {
    let runtime = Runtime::new().unwrap();
    let hot = Arc::new(AtomicU64::new(0));
    let feedback = Arc::new(AtomicU64::new(0));
    let acquire = Arc::new(AtomicU64::new(0));
    let _guard = RuntimeJitGuard::attach(
        &runtime,
        DisableFeedbackBackend {
            hot: Arc::clone(&hot),
            feedback: Arc::clone(&feedback),
            acquire: Arc::clone(&acquire),
        },
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "function terminal(a,b){return a+b} terminal(20,22); for(let i=0;i<100;i++)terminal(20,22)",
            )
        })
        .unwrap();

    assert_eq!(
        hot.load(Ordering::Relaxed),
        2,
        "top-level and terminal function"
    );
    assert_eq!(
        acquire.load(Ordering::Relaxed),
        0,
        "terminal response must suppress native acquisition in the same and later invocations"
    );
    assert!(
        feedback.load(Ordering::Relaxed) <= 6,
        "only the first invocation of each bytecode function may report feedback before its hot response"
    );
}

#[test]
fn property_feedback_records_real_shape_prototype_and_location_only_for_fast_data_properties() {
    let (value, events) = execute(
        "const proto={inherited:7}; const own=Object.create(proto); own.value=40; \
         function load(o){return o.value+o.inherited} function store(o,v){o.value=v;return o.value} \
         const proxy=new Proxy(own,{get(t,k){return Reflect.get(t,k)}}); \
         `${load(own)}:${store(own,41)}:${load(proxy)}`",
    );
    assert_eq!(value, "47:41:48");

    let properties = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_PROPERTY)
        .collect::<Vec<_>>();
    assert!(
        !properties.is_empty(),
        "ordinary bytecode property feedback"
    );
    assert!(properties
        .iter()
        .all(|event| event.shape.0 != 0 && event.shape.1 != 0));
    assert!(properties.iter().any(|event| event.prototype.0 == 0));
    assert!(properties
        .iter()
        .any(|event| event.prototype.0 != 0 && event.prototype.1 != 0));
    assert!(properties.iter().any(|event| {
        event.flags & qjs::JS_JIT_FEEDBACK_PROPERTY_STORE != 0 && event.property_attributes & 1 != 0
    }));
    assert!(properties
        .iter()
        .any(|event| event.property_offset != u32::MAX));
}

#[test]
fn accessor_and_proxy_property_paths_do_not_claim_fixed_locations() {
    let (value, events) = execute(
        "let hits=0; const target={get value(){hits++;return 9}}; \
         const proxy=new Proxy({value:11},{get(t,k){hits++;return Reflect.get(t,k)}}); \
         function read(o){return o.value} `${read(target)}:${read(proxy)}:${hits}`",
    );
    assert_eq!(value, "9:11:2");
    assert!(
        events
            .iter()
            .all(|event| event.kind != qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_PROPERTY),
        "accessor and Proxy slow paths must remain generic"
    );
}

fn execute(source: &str) -> (String, Vec<Captured>) {
    let runtime = Runtime::new().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let _guard = RuntimeJitGuard::attach(&runtime, CaptureBackend(Arc::clone(&events))).unwrap();
    let context = Context::full(&runtime).unwrap();
    let value = context.with(|ctx| ctx.eval::<String, _>(source).unwrap());
    let captured = events.lock().unwrap().clone();
    (value, captured)
}

#[test]
fn call_feedback_records_argc_and_every_argument_type_once() {
    let (value, events) = execute("function add(a){return a+arguments[1]} add(20,22,'tail'); 'ok'");
    assert_eq!(value, "ok");
    let call = events
        .iter()
        .find(|event| {
            event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_CALL && event.types.len() == 3
        })
        .expect("call feedback");
    assert_eq!(
        call.types,
        [
            qjs::JSJitValueType_JS_JIT_VALUE_INT32,
            qjs::JSJitValueType_JS_JIT_VALUE_INT32,
            qjs::JSJitValueType_JS_JIT_VALUE_STRING,
        ]
    );
}

#[test]
fn callsite_feedback_records_caller_target_arguments_and_result_at_exact_pc() {
    let (_, events) = execute(
        "function target(x){return x+1} function caller(x){return target(x)} caller(41); 'ok'",
    );
    let callsite = events
        .iter()
        .find(|event| {
            event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_CALL
                && event.flags & qjs::JS_JIT_FEEDBACK_CALL_SITE != 0
                && event.types
                    == [
                        qjs::JSJitValueType_JS_JIT_VALUE_INT32,
                        qjs::JSJitValueType_JS_JIT_VALUE_INT32,
                    ]
        })
        .expect("bytecode call-site feedback");
    assert_ne!(callsite.function, (0, 0));
    assert_ne!(callsite.callee, (0, 0));
    assert_ne!(callsite.function, callsite.callee);
    assert_ne!(callsite.pc, 0);
    assert_eq!(callsite.slot, 1, "slot carries exact argc");
}

#[test]
fn return_feedback_is_keyed_by_exact_site_and_preserves_dynamic_types() {
    let (_, events) =
        execute("function choose(x){if(x)return 1;return 's'} choose(true);choose(false); 'ok'");
    let returns: Vec<_> = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_RETURN)
        .collect();
    assert!(returns
        .iter()
        .any(|event| event.types == [qjs::JSJitValueType_JS_JIT_VALUE_INT32]));
    assert!(returns
        .iter()
        .any(|event| event.types == [qjs::JSJitValueType_JS_JIT_VALUE_STRING]));
    assert_ne!(returns[0].pc, returns[1].pc);
}

#[test]
fn binary_feedback_records_operands_result_and_numeric_edge_flags() {
    let (_, events) = execute(
        "function ops(a,b){return [a+b,a-b,a*b,a/b,a<b]} ops(2147483647,1);ops(-1,0);ops(0,0); 'ok'",
    );
    let binary: Vec<_> = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_BINARY)
        .collect();
    assert!(binary.iter().any(|event| event.types
        == [
            qjs::JSJitValueType_JS_JIT_VALUE_INT32,
            qjs::JSJitValueType_JS_JIT_VALUE_INT32,
            qjs::JSJitValueType_JS_JIT_VALUE_FLOAT64,
        ]
        && event.flags & qjs::JS_JIT_FEEDBACK_OVERFLOW != 0));
    assert!(binary
        .iter()
        .any(|event| event.flags & qjs::JS_JIT_FEEDBACK_NEGATIVE_ZERO != 0));
    assert!(binary
        .iter()
        .any(|event| event.flags & qjs::JS_JIT_FEEDBACK_NAN != 0));
    assert!(binary
        .iter()
        .any(|event| { event.types.last() == Some(&qjs::JSJitValueType_JS_JIT_VALUE_BOOL) }));
    let distinct_sites = binary
        .iter()
        .map(|event| event.pc)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(distinct_sites.len() >= 5, "add/sub/mul/div/compare slots");
}

#[test]
fn dynamic_add_records_int_float_and_string_without_changing_results() {
    let (value, events) =
        execute("function add(a,b){return a+b} `${add(20,22)}:${add(.5,1.25)}:${add('a','b')}`");
    assert_eq!(value, "42:1.75:ab");
    let adds: Vec<_> = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_BINARY)
        .collect();
    assert!(adds.iter().any(|event| event.types == [1, 1, 1]));
    assert!(adds.iter().any(|event| event.types == [2, 2, 2]));
    assert!(adds.iter().any(|event| event.types == [6, 6, 6]));
}

#[test]
fn interpreter_reports_conversion_and_branch_feedback_at_exact_sites() {
    let (value, events) =
        execute("function probe(x){const n=+x;return n?1:0} `${probe('2')}:${probe('0')}`");
    assert_eq!(value, "1:0");

    let conversions = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_CONVERSION)
        .collect::<Vec<_>>();
    assert!(conversions.iter().any(|event| event.types
        == [
            qjs::JSJitValueType_JS_JIT_VALUE_STRING,
            qjs::JSJitValueType_JS_JIT_VALUE_INT32,
        ]));

    let branches = events
        .iter()
        .filter(|event| event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_BRANCH)
        .collect::<Vec<_>>();
    assert!(branches.iter().any(|event| {
        event.types == [qjs::JSJitValueType_JS_JIT_VALUE_INT32]
            && event.flags & qjs::JS_JIT_FEEDBACK_BRANCH_TAKEN != 0
    }));
    assert!(branches.iter().any(|event| {
        event.types == [qjs::JSJitValueType_JS_JIT_VALUE_INT32]
            && event.flags & qjs::JS_JIT_FEEDBACK_BRANCH_TAKEN == 0
    }));
    assert_eq!(
        branches
            .iter()
            .map(|event| event.pc)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1,
        "both outcomes must belong to the same exact branch site"
    );
}

#[test]
fn feedback_survives_nested_calls_and_full_gc_between_executions() {
    let runtime = Runtime::new().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let _guard = RuntimeJitGuard::attach(&runtime, CaptureBackend(Arc::clone(&events))).unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>(
            "function inner(x){return x+1} function outer(x){return inner(x)} outer(1)",
        )
        .unwrap()
    });
    runtime.run_gc();
    context.with(|ctx| assert_eq!(ctx.eval::<i32, _>("outer(41)").unwrap(), 42));
    let calls = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| {
            event.kind == qjs::JSJitFeedbackKind_JS_JIT_FEEDBACK_CALL
                && event.types == [qjs::JSJitValueType_JS_JIT_VALUE_INT32]
        })
        .count();
    assert_eq!(calls, 4, "outer and reentrant inner before and after GC");
}

#[test]
fn exception_and_bigint_do_not_change_interpreter_semantics() {
    let (value, events) =
        execute("function f(a,b){try{return a+b}catch(e){return e.name}} `${f(1n,2n)}:${f(1n,2)}`");
    assert_eq!(value, "3:TypeError");
    assert!(events.iter().any(|event| event
        .types
        .contains(&qjs::JSJitValueType_JS_JIT_VALUE_BIGINT)));
}

#[test]
fn stable_numeric_feedback_produces_a_tier2_signature() {
    let function = FunctionKey::new(71, 4);
    let mut feedback = FeedbackTable::new(32, 2);
    feedback.observe_call(function, &[ObservedType::Int32, ObservedType::Int32]);
    feedback.observe_return(function, 19, ObservedType::Int32);
    feedback.observe_binary(
        function,
        7,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );

    let signature = feedback
        .snapshot(23)
        .bounded_specialization(function)
        .expect("stable numeric signature");
    assert_eq!(signature.function(), function);
    assert_eq!(signature.generation(), 4);
    assert_eq!(signature.arity(), 2);
    assert_eq!(
        signature.arguments(),
        &[FeedbackRepresentation::Int32, FeedbackRepresentation::Int32]
    );
    assert_eq!(signature.result(), FeedbackRepresentation::Int32);
    assert_eq!(signature.feedback_epoch(), 23);
}

#[test]
fn float_feedback_is_supported_but_mixed_or_unstable_feedback_is_not() {
    let stable = FunctionKey::new(72, 1);
    let mixed_binary = FunctionKey::new(73, 1);
    let polymorphic_return = FunctionKey::new(74, 1);
    let mut feedback = FeedbackTable::new(64, 2);

    feedback.observe_call(stable, &[ObservedType::Float64]);
    feedback.observe_return(stable, 11, ObservedType::Float64);
    feedback.observe_binary(
        stable,
        3,
        ObservedType::Float64,
        ObservedType::Float64,
        ObservedType::Float64,
        BinaryFeedbackFlags::NAN,
    );

    feedback.observe_call(mixed_binary, &[ObservedType::Int32]);
    feedback.observe_return(mixed_binary, 11, ObservedType::Int32);
    feedback.observe_binary(
        mixed_binary,
        3,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Float64,
        BinaryFeedbackFlags::OVERFLOW,
    );

    feedback.observe_call(polymorphic_return, &[ObservedType::Int32]);
    feedback.observe_return(polymorphic_return, 11, ObservedType::Int32);
    feedback.observe_return(polymorphic_return, 11, ObservedType::Float64);
    feedback.observe_binary(
        polymorphic_return,
        3,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );

    let snapshot = feedback.snapshot(24);
    assert_eq!(
        snapshot.bounded_specialization(stable).unwrap().result(),
        FeedbackRepresentation::Float64
    );
    assert!(snapshot.bounded_specialization(mixed_binary).is_none());
    assert!(snapshot
        .bounded_specialization(polymorphic_return)
        .is_none());
    assert!(snapshot
        .bounded_specialization(FunctionKey::new(72, 2))
        .is_none());
}

#[test]
fn specialization_rejects_unstable_arity_zero_epoch_and_unbounded_arguments() {
    let changing_arity = FunctionKey::new(75, 1);
    let too_wide = FunctionKey::new(76, 1);
    let mut feedback = FeedbackTable::new(64, 2);
    feedback.observe_call(changing_arity, &[ObservedType::Int32]);
    feedback.observe_call(changing_arity, &[ObservedType::Int32, ObservedType::Int32]);
    feedback.observe_return(changing_arity, 9, ObservedType::Int32);
    feedback.observe_binary(
        changing_arity,
        3,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );

    let wide_arguments = vec![ObservedType::Int32; MAX_SPECIALIZED_ARGUMENTS + 1];
    feedback.observe_call(too_wide, &wide_arguments);
    feedback.observe_return(too_wide, 9, ObservedType::Int32);
    feedback.observe_binary(
        too_wide,
        3,
        ObservedType::Int32,
        ObservedType::Int32,
        ObservedType::Int32,
        BinaryFeedbackFlags::NONE,
    );

    assert!(feedback
        .snapshot(25)
        .bounded_specialization(changing_arity)
        .is_none());
    assert!(feedback
        .snapshot(25)
        .bounded_specialization(too_wide)
        .is_none());
    assert!(feedback
        .snapshot(0)
        .bounded_specialization(too_wide)
        .is_none());
}

#[test]
fn conversion_feedback_is_exact_bounded_and_queryable() {
    let function = FunctionKey::new(81, 3);
    let mut feedback = FeedbackTable::new(32, 2);
    assert_eq!(
        feedback.observe_conversion(function, 17, ObservedType::String, ObservedType::Float64,),
        FeedbackState::Monomorphic
    );
    assert_eq!(
        feedback.observe_conversion(function, 17, ObservedType::Bool, ObservedType::Int32,),
        FeedbackState::Polymorphic
    );
    assert_eq!(
        feedback.observe_conversion(function, 17, ObservedType::Object, ObservedType::Float64,),
        FeedbackState::Megamorphic
    );

    let snapshot = feedback.snapshot(31);
    assert_eq!(snapshot.function(), Some(function));
    let conversion = snapshot
        .conversion_at(function, 17)
        .expect("conversion slot");
    assert_eq!(conversion.state(), FeedbackState::Megamorphic);
    assert_eq!(conversion.operand(), &[]);
    assert_eq!(
        conversion.result(),
        &[ObservedType::Float64, ObservedType::Int32]
    );
    assert!(snapshot
        .conversion_at(FunctionKey::new(81, 4), 17)
        .is_none());
    assert!(snapshot.conversion_at(function, 18).is_none());
}

#[test]
fn branch_feedback_tracks_condition_type_and_both_outcomes_monotonically() {
    let function = FunctionKey::new(82, 5);
    let mut feedback = FeedbackTable::new(32, 2);
    assert_eq!(
        feedback.observe_branch(function, 23, ObservedType::Bool, true),
        FeedbackState::Monomorphic
    );
    assert_eq!(
        feedback.observe_branch(function, 23, ObservedType::Bool, false),
        FeedbackState::Polymorphic
    );
    assert_eq!(
        feedback.observe_branch(function, 23, ObservedType::Int32, true),
        FeedbackState::Polymorphic
    );
    assert_eq!(
        feedback.observe_branch(function, 23, ObservedType::String, false),
        FeedbackState::Megamorphic
    );

    let snapshot = feedback.snapshot(32);
    let branch = snapshot.branch_at(function, 23).expect("branch slot");
    assert_eq!(branch.state(), FeedbackState::Megamorphic);
    assert_eq!(branch.condition_types(), &[]);
    assert!(branch.was_taken());
    assert!(branch.was_not_taken());
    assert!(snapshot.branch_at(FunctionKey::new(82, 6), 23).is_none());
}

#[test]
fn specialization_fingerprint_is_stable_and_sensitive_to_key_fields() {
    fn signature(
        function: FunctionKey,
        epoch: u64,
        observed: ObservedType,
    ) -> rquickjs_jit::runtime::BoundedSpecializationSignature {
        let mut feedback = FeedbackTable::new(16, 2);
        feedback.observe_call(function, &[observed]);
        feedback.observe_return(function, 9, observed);
        feedback.observe_binary(
            function,
            3,
            observed,
            observed,
            observed,
            BinaryFeedbackFlags::NONE,
        );
        feedback
            .snapshot(epoch)
            .bounded_specialization(function)
            .unwrap()
    }

    let base = signature(FunctionKey::new(90, 2), 41, ObservedType::Int32);
    assert_eq!(base.fingerprint(), base.fingerprint());
    assert_ne!(
        base.fingerprint(),
        signature(FunctionKey::new(90, 3), 41, ObservedType::Int32).fingerprint()
    );
    assert_ne!(
        base.fingerprint(),
        signature(FunctionKey::new(90, 2), 42, ObservedType::Int32).fingerprint()
    );
    assert_ne!(
        base.fingerprint(),
        signature(FunctionKey::new(90, 2), 41, ObservedType::Float64).fingerprint()
    );
}

#[test]
fn call_signature_lattice_tracks_exact_targets_arguments_and_results() {
    let caller = FunctionKey::new(101, 7);
    let first = FunctionKey::new(201, 3);
    let second = FunctionKey::new(202, 1);
    let third = FunctionKey::new(203, 1);
    let mut feedback = FeedbackTable::new(32, 2);

    assert_eq!(
        feedback.observe_call_signature(
            caller,
            29,
            first,
            &[ObservedType::Int32, ObservedType::Int32],
            ObservedType::Int32,
        ),
        FeedbackState::Monomorphic
    );
    assert_eq!(
        feedback.observe_call_signature(
            caller,
            29,
            second,
            &[ObservedType::Int32, ObservedType::Int32],
            ObservedType::Int32,
        ),
        FeedbackState::Polymorphic
    );
    assert_eq!(
        feedback.observe_call_signature(
            caller,
            29,
            third,
            &[ObservedType::Int32, ObservedType::Int32],
            ObservedType::Int32,
        ),
        FeedbackState::Megamorphic
    );

    let snapshot = feedback.snapshot(51);
    let call = snapshot.call_signature_at(caller, 29).expect("call site");
    assert_eq!(call.state(), FeedbackState::Megamorphic);
    assert_eq!(call.targets(), &[]);
    assert_eq!(call.argument(0), &[ObservedType::Int32]);
    assert_eq!(call.results(), &[ObservedType::Int32]);
    assert!(snapshot
        .call_signature_at(FunctionKey::new(101, 8), 29)
        .is_none());
    assert!(snapshot.call_specialization_at(caller, 29).is_none());
}

#[test]
fn stable_numeric_call_signature_produces_an_exact_bounded_key() {
    let caller = FunctionKey::new(102, 4);
    let callee = FunctionKey::new(302, 9);
    let mut feedback = FeedbackTable::new(32, 2);
    feedback.observe_call_signature(
        caller,
        37,
        callee,
        &[ObservedType::Float64, ObservedType::Float64],
        ObservedType::Float64,
    );

    let key = feedback
        .snapshot(52)
        .call_specialization_at(caller, 37)
        .expect("stable call specialization");
    assert_eq!(key.caller(), caller);
    assert_eq!(key.callee(), callee);
    assert_eq!(key.arity(), 2);
    assert_eq!(
        key.arguments(),
        &[
            FeedbackRepresentation::Float64,
            FeedbackRepresentation::Float64
        ]
    );
    assert_eq!(key.result(), FeedbackRepresentation::Float64);
    assert_eq!(key.feedback_epoch(), 52);
}

#[test]
fn call_specialization_rejects_unstable_or_unsupported_signatures() {
    let caller = FunctionKey::new(103, 2);
    let callee = FunctionKey::new(303, 6);
    let mut feedback = FeedbackTable::new(32, 2);
    feedback.observe_call_signature(
        caller,
        41,
        callee,
        &[ObservedType::Int32],
        ObservedType::Int32,
    );
    feedback.observe_call_signature(
        caller,
        41,
        callee,
        &[ObservedType::Int32],
        ObservedType::Float64,
    );
    feedback.observe_call_signature(
        caller,
        43,
        callee,
        &[ObservedType::String],
        ObservedType::String,
    );

    let snapshot = feedback.snapshot(53);
    assert_eq!(
        snapshot.call_signature_at(caller, 41).unwrap().state(),
        FeedbackState::Polymorphic
    );
    assert!(snapshot.call_specialization_at(caller, 41).is_none());
    assert!(snapshot.call_specialization_at(caller, 43).is_none());
    assert!(feedback
        .snapshot(0)
        .call_specialization_at(caller, 43)
        .is_none());
}
