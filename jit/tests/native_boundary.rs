use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use rquickjs::{Context, Function, Runtime};
use rquickjs_core::{qjs, runtime::JitBackend};
use rquickjs_jit::{abi::JitExitExt, bytecode::CompileSnapshot};

#[derive(Clone)]
enum EntryKind {
    Done,
    Throw,
    Interrupt,
    Deopt {
        resume_pc: u32,
        counter: Arc<AtomicUsize>,
    },
    InvalidDeopt,
    Retry,
}

impl EntryKind {
    fn entry(&self) -> unsafe extern "C" fn(*mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
        match self {
            Self::Done => native_done,
            Self::Throw => native_throw,
            Self::Interrupt => native_interrupt,
            Self::Deopt { .. } => native_deopt,
            Self::InvalidDeopt => native_invalid_deopt,
            Self::Retry => native_retry,
        }
    }
}

#[derive(Clone)]
struct EntrySpec {
    generation: u64,
    pc: u32,
    kind: EntryKind,
}

struct EntryPin {
    kind: EntryKind,
}

struct NativeBackend {
    entries: HashMap<u64, EntrySpec>,
    releases: Arc<AtomicUsize>,
    retirements: Arc<Mutex<Vec<(u64, u64)>>>,
}

impl NativeBackend {
    fn one(snapshot: &CompileSnapshot, kind: EntryKind, releases: Arc<AtomicUsize>) -> Self {
        Self::one_at(snapshot, 0, kind, releases)
    }

    fn one_at(
        snapshot: &CompileSnapshot,
        pc: u32,
        kind: EntryKind,
        releases: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            entries: HashMap::from([(
                snapshot.function_id(),
                EntrySpec {
                    generation: snapshot.generation(),
                    pc,
                    kind,
                },
            )]),
            releases,
            retirements: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn recording(retirements: Arc<Mutex<Vec<(u64, u64)>>>) -> Self {
        Self {
            entries: HashMap::new(),
            releases: Arc::new(AtomicUsize::new(0)),
            retirements,
        }
    }
}

unsafe impl JitBackend for NativeBackend {
    fn acquire_entry(&mut self, id: u64, generation: u64, pc: u32) -> qjs::JSJitEntryHandle {
        let Some(spec) = self
            .entries
            .get(&id)
            .filter(|spec| spec.generation == generation && spec.pc == pc)
        else {
            return qjs::JSJitEntryHandle {
                struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
                reserved: 0,
                entry: None,
                pin: std::ptr::null_mut(),
            };
        };

        let pin = Arc::new(EntryPin {
            kind: spec.kind.clone(),
        });
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: Some(spec.kind.entry()),
            pin: Arc::into_raw(pin).cast_mut().cast::<c_void>(),
        }
    }

    fn release_entry(&mut self, entry: qjs::JSJitEntryHandle) {
        if !entry.pin.is_null() {
            unsafe { drop(Arc::from_raw(entry.pin.cast::<EntryPin>())) };
            self.releases.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn function_retire(&mut self, id: u64, generation: u64) {
        self.retirements.lock().unwrap().push((id, generation));
    }
}

struct UnpinnedBackend;

unsafe impl JitBackend for UnpinnedBackend {
    fn acquire_entry(&mut self, _id: u64, _generation: u64, _pc: u32) -> qjs::JSJitEntryHandle {
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: Some(native_done),
            pin: std::ptr::null_mut(),
        }
    }
}

unsafe extern "C" fn native_done(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe {
        (*frame).result = qjs::JS_MKVAL(qjs::JS_TAG_INT, 42);
    }
    qjs::JSJitExit::done()
}

unsafe extern "C" fn native_throw(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    const MESSAGE: &[u8] = b"native throw";
    unsafe {
        let error = qjs::JS_NewStringLen(
            (*frame).ctx,
            MESSAGE.as_ptr().cast(),
            MESSAGE.len() as qjs::size_t,
        );
        qjs::JS_Throw((*frame).ctx, error);
    }
    qjs::JSJitExit::exception()
}

unsafe extern "C" fn native_interrupt(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe {
        let error = qjs::JS_NewError((*frame).ctx);
        qjs::JS_SetUncatchableError((*frame).ctx, error);
        qjs::JS_Throw((*frame).ctx, error);
    }
    qjs::JSJitExit::interrupt()
}

unsafe extern "C" fn native_deopt(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe {
        let pin = &*((*frame).entry.pin.cast::<EntryPin>());
        let EntryKind::Deopt { resume_pc, counter } = &pin.kind else {
            return qjs::JSJitExit::retry_interpreter();
        };
        counter.fetch_add(1, Ordering::SeqCst);
        (*(*frame).stack_base) = qjs::JS_MKVAL(qjs::JS_TAG_INT, 7);
        (*frame).stack_top = (*frame).stack_base.add(1);
        qjs::JSJitExit::resume((*frame).bytecode_start.add(*resume_pc as usize))
    }
}

unsafe extern "C" fn native_invalid_deopt(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe { qjs::JSJitExit::resume((*frame).bytecode_start.add(2)) }
}

unsafe extern "C" fn native_retry(_frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    qjs::JSJitExit::retry_interpreter()
}

fn snapshot<'js>(ctx: &rquickjs::Ctx<'js>, function: &Function<'js>) -> CompileSnapshot {
    unsafe { CompileSnapshot::capture_raw(ctx.as_raw().as_ptr(), function.as_value().as_raw()) }
        .expect("supported bytecode function")
}

#[test]
fn native_return_enters_c_done_cleanup_and_releases_the_pin() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::Done,
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 42);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn verified_loop_header_can_enter_native_code_from_an_osr_poll() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.target = function target() {
                globalThis.started = true;
                while (globalThis.keepGoing) globalThis.keepGoing = false;
                return 3;
            };
            globalThis.keepGoing = true;
            "#,
        )
        .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let verified = captured
        .verify(Default::default())
        .expect("loop bytecode verifies before publishing an OSR entry");
    let osr_pc = verified
        .instructions()
        .iter()
        .map(|instruction| instruction.pc())
        .find(|pc| *pc != 0 && verified.control_flow_graph().is_loop_header(*pc))
        .expect("non-entry loop header");
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one_at(
            &captured,
            osr_pc,
            EntryKind::Done,
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 42);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn osr_retry_restarts_at_the_polled_pc_without_replaying_the_prefix() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.started = 0;
            globalThis.keepGoing = true;
            globalThis.target = function target() {
                globalThis.started++;
                while (globalThis.keepGoing) globalThis.keepGoing = false;
                return globalThis.started;
            };
            "#,
        )
        .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let verified = captured.verify(Default::default()).expect("verified loop");
    let osr_pc = verified
        .instructions()
        .iter()
        .map(|instruction| instruction.pc())
        .find(|pc| *pc != 0 && verified.control_flow_graph().is_loop_header(*pc))
        .expect("non-entry loop header");
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one_at(
            &captured,
            osr_pc,
            EntryKind::Retry,
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 1, "RETRY replayed code before the OSR poll");
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn native_entry_without_a_lifetime_pin_is_not_called() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
    });
    let guard = runtime.attach_jit_backend(UnpinnedBackend).unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 1);
    drop(guard);
}

#[test]
fn native_exception_is_caught_by_the_existing_js_catch_path() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.target = function target() { return "interpreted" };
            globalThis.wrapper = function wrapper() {
                try { target(); return "missed"; }
                catch (error) { return String(error); }
            };
            "#,
        )
        .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::Throw,
            Arc::clone(&releases),
        ))
        .unwrap();

    let caught = context.with(|ctx| ctx.eval::<String, _>("wrapper()").unwrap());

    assert_eq!(caught, "native throw");
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn native_interrupt_remains_uncatchable() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.target = function target() { return 1 };
            globalThis.wrapper = function wrapper() {
                try { target(); return false; }
                catch (_) { return true; }
            };
            "#,
        )
        .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::Interrupt,
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| ctx.eval::<bool, _>("wrapper()"));

    assert!(result.is_err(), "an uncatchable interrupt reached JS catch");
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn deopt_resumes_after_the_materialized_side_effect_exactly_once() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let captured = context.with(|ctx| {
        let interpreted_counter = Arc::clone(&counter);
        ctx.globals()
            .set(
                "tick",
                Function::new(ctx.clone(), move || {
                    interpreted_counter.fetch_add(1, Ordering::SeqCst);
                })
                .unwrap(),
            )
            .unwrap();
        ctx.eval::<(), _>("globalThis.target = function target() { tick(); return 7 }")
            .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let return_pc = captured
        .decode()
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode().name() == "return")
        .expect("explicit return opcode")
        .pc();
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::Deopt {
                resume_pc: return_pc,
                counter: Arc::clone(&counter),
            },
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 7);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn deopt_rejects_a_pc_inside_an_instruction_after_releasing_the_pin() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.target = function target() { return 1000 };
            globalThis.wrapper = function wrapper() {
                try { target(); return "accepted"; }
                catch (error) { return String(error); }
            };
            "#,
        )
        .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let captured = snapshot(&ctx, &function);
        assert!(captured
            .decode()
            .unwrap()
            .iter()
            .any(|instruction| instruction.pc() == 0 && instruction.size() > 2));
        captured
    });
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::InvalidDeopt,
            Arc::clone(&releases),
        ))
        .unwrap();

    let caught = context.with(|ctx| ctx.eval::<String, _>("wrapper()").unwrap());

    assert!(
        caught.contains("invalid JIT deopt PC"),
        "mid-instruction resume was not rejected at the boundary: {caught}"
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    drop(guard);
}

#[test]
fn closure_objects_share_bytecode_identity_and_retire_only_at_finalization() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let retirements = Arc::new(Mutex::new(Vec::new()));
    let guard = runtime
        .attach_jit_backend(NativeBackend::recording(Arc::clone(&retirements)))
        .unwrap();

    let key = context.with(|ctx| {
        let make: Function<'_> = ctx
            .eval("(function make() { return function shared() { return 1 } })")
            .unwrap();
        let first: Function<'_> = make.call(()).unwrap();
        let second: Function<'_> = make.call(()).unwrap();
        let first_snapshot = snapshot(&ctx, &first);
        let second_snapshot = snapshot(&ctx, &second);
        assert_eq!(first_snapshot.function_id(), second_snapshot.function_id());
        assert_eq!(first_snapshot.generation(), second_snapshot.generation());
        (first_snapshot.function_id(), first_snapshot.generation())
    });

    runtime.run_gc();
    let matching: Vec<_> = retirements
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|event| *event == key)
        .collect();
    assert_eq!(matching, [key]);
    drop(guard);
}

#[test]
fn distinct_bytecode_allocations_with_identical_bodies_get_distinct_ids() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();

    let (first_id, second_id) = context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.first = function sameBody() { return 1 };
            globalThis.second = function sameBody() { return 1 };
            "#,
        )
        .unwrap();
        let first: Function<'_> = ctx.globals().get("first").unwrap();
        let second: Function<'_> = ctx.globals().get("second").unwrap();
        (
            snapshot(&ctx, &first).function_id(),
            snapshot(&ctx, &second).function_id(),
        )
    });

    assert_ne!(
        first_id, second_id,
        "identity was derived from byte content"
    );
}

#[test]
fn explicit_invalidation_keeps_identity_and_advances_generation() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (function_id, old_generation) = context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let captured = snapshot(&ctx, &function);
        (captured.function_id(), captured.generation())
    });
    let retirements = Arc::new(Mutex::new(Vec::new()));
    let guard = runtime
        .attach_jit_backend(NativeBackend::recording(Arc::clone(&retirements)))
        .unwrap();

    let new_generation = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let status = unsafe {
            qjs::JS_JitInvalidateFunction(ctx.as_raw().as_ptr(), function.as_value().as_raw())
        };
        assert_eq!(status, qjs::JS_JIT_BACKEND_OK);
        let captured = snapshot(&ctx, &function);
        assert_eq!(captured.function_id(), function_id);
        captured.generation()
    });

    assert!(new_generation > old_generation);
    assert_eq!(
        retirements.lock().unwrap().as_slice(),
        [(function_id, old_generation)]
    );
    drop(guard);
}

#[test]
fn invalidation_prevents_acquiring_an_entry_from_the_retired_generation() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let captured = context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        snapshot(&ctx, &function)
    });
    let releases = Arc::new(AtomicUsize::new(0));
    let guard = runtime
        .attach_jit_backend(NativeBackend::one(
            &captured,
            EntryKind::Done,
            Arc::clone(&releases),
        ))
        .unwrap();

    let result = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let status = unsafe {
            qjs::JS_JitInvalidateFunction(ctx.as_raw().as_ptr(), function.as_value().as_raw())
        };
        assert_eq!(status, qjs::JS_JIT_BACKEND_OK);
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(result, 1);
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    drop(guard);
}
