use std::sync::{Arc, Mutex};

use rquickjs::{Context, Runtime};
use rquickjs_core::{
    qjs,
    runtime::{JitBackend, RuntimeJitGuard},
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Captured {
    kind: u32,
    function: (u64, u64),
    pc: u32,
    slot: u32,
    types: Vec<u32>,
    flags: u32,
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
            pc: event.pc,
            slot: event.slot,
            types,
            flags: event.flags,
        });
    }
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
