use std::sync::{Arc, Mutex};

use rquickjs::{Context, Function, Runtime};
use rquickjs_core::{
    qjs,
    runtime::{JitBackend, JitFunctionRegistry},
};
use rquickjs_jit::{abi::JitExitExt, bytecode::CompileSnapshot};

unsafe extern "C" fn native_done(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe {
        (*frame).result = qjs::JS_MKVAL(qjs::JS_TAG_INT, 42);
    }
    qjs::JSJitExit::done()
}

struct AlwaysNativeBackend;

unsafe impl JitBackend for AlwaysNativeBackend {
    fn acquire_entry(&mut self, _id: u64, _generation: u64, pc: u32) -> qjs::JSJitEntryHandle {
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: (pc == 0).then_some(native_done),
            pin: Box::into_raw(Box::new(0_u8)).cast(),
        }
    }

    fn release_entry(&mut self, entry: qjs::JSJitEntryHandle) {
        unsafe { drop(Box::from_raw(entry.pin.cast::<u8>())) };
    }
}

#[test]
fn guard_drop_leaves_runtime_clones_and_contexts_interpreter_only() {
    let runtime = Runtime::new().unwrap();
    let runtime_clone = runtime.clone();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
    });
    let guard = runtime
        .attach_jit_backend(AlwaysNativeBackend)
        .expect("attach native fixture");

    let native = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });
    assert_eq!(native, 42);

    drop(guard);
    drop(runtime);
    runtime_clone.run_gc();
    let interpreted = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });
    assert_eq!(interpreted, 1);
}

struct PendingWork {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for PendingWork {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("queued_work_drop");
    }
}

struct OrderedBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    pending: Option<PendingWork>,
}

unsafe impl JitBackend for OrderedBackend {
    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push("detach");
        drop(self.pending.take());
    }
}

impl Drop for OrderedBackend {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("backend_drop");
    }
}

#[test]
fn raw_runtime_forces_detach_and_drains_work_while_guard_survives() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let guard = runtime
        .attach_jit_backend(OrderedBackend {
            events: Arc::clone(&events),
            pending: Some(PendingWork {
                events: Arc::clone(&events),
            }),
        })
        .unwrap();

    drop(runtime);
    assert!(events.lock().unwrap().is_empty());
    drop(context);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["detach", "queued_work_drop", "backend_drop"]
    );

    drop(guard);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["detach", "queued_work_drop", "backend_drop"]
    );
}

struct RegistryValueDrop {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for RegistryValueDrop {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("registry_value_drop");
    }
}

struct RegistryBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    pending: Option<PendingWork>,
    registry: Arc<Mutex<Option<JitFunctionRegistry>>>,
}

unsafe impl JitBackend for RegistryBackend {
    fn runtime_attached(&mut self, registry: JitFunctionRegistry) {
        *self.registry.lock().unwrap() = Some(registry);
    }

    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push("detach");
        drop(self.pending.take());
    }
}

impl Drop for RegistryBackend {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("backend_drop");
    }
}

#[test]
fn runtime_owned_registry_releases_functions_before_free_while_guard_survives() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let registry_slot = Arc::new(Mutex::new(None));
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let guard = runtime
        .attach_jit_backend(RegistryBackend {
            events: Arc::clone(&events),
            pending: Some(PendingWork {
                events: Arc::clone(&events),
            }),
            registry: Arc::clone(&registry_slot),
        })
        .unwrap();
    let registry = registry_slot
        .lock()
        .unwrap()
        .clone()
        .expect("runtime_attached supplied a registry handle");

    context.with(|ctx| {
        let value_drop = RegistryValueDrop {
            events: Arc::clone(&events),
        };
        let host = Function::new(ctx.clone(), move || {
            let _ = &value_drop;
        })
        .unwrap();
        ctx.globals().set("__jitRegistryValue", host).unwrap();
        ctx.eval::<(), _>(
            r#"
            globalThis.target = (function make(value) {
                return function target() { return value };
            })(globalThis.__jitRegistryValue);
            delete globalThis.__jitRegistryValue;
            "#,
        )
        .unwrap();

        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let snapshot = unsafe {
            CompileSnapshot::capture_raw(ctx.as_raw().as_ptr(), function.as_value().as_raw())
        }
        .unwrap();
        registry
            .retain_function(
                &ctx,
                &function,
                snapshot.function_id(),
                snapshot.generation(),
            )
            .unwrap();
        assert_eq!(registry.retained_len(&ctx).unwrap(), 1);
        ctx.eval::<(), _>("delete globalThis.target").unwrap();
    });
    assert!(events.lock().unwrap().is_empty());

    drop(runtime);
    assert!(events.lock().unwrap().is_empty());
    drop(context);
    events.lock().unwrap().push("runtime_drop_returned");

    assert!(!registry.is_attached());
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "detach",
            "queued_work_drop",
            "registry_value_drop",
            "backend_drop",
            "runtime_drop_returned",
        ]
    );
    drop(guard);
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&"runtime_drop_returned")
    );
}
