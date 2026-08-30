#![cfg(feature = "test-support")]

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use rquickjs::{
    loader::{BuiltinLoader, BuiltinResolver},
    Context as JsContext, Module,
};
use rquickjs_jit::JitRuntime;

/*
 * Read-only production inventory from
 * ../gpui-component/crates/shell/src/engine/quickjs on 2026-08-30.  Each
 * discovered direct rquickjs call is represented by a named fixture below;
 * test-only shell call sites use the same surfaces and need no extra adapter.
 */
const GPUI_SHELL_QUICKJS_INVENTORY: &[(&str, u32, &str)] = &[
    ("engine/quickjs/mod.rs", 1449, "runtime_creation"),
    ("engine/quickjs/mod.rs", 1450, "full_context_creation"),
    ("engine/quickjs/mod.rs", 1464, "tuple_module_loader"),
    ("engine/quickjs/mod.rs", 1487, "interrupt_handler"),
    ("engine/quickjs/scheduler.rs", 176, "scoped_job_drain"),
    ("engine/quickjs/scheduler.rs", 206, "test_job_drain"),
    (
        "engine/quickjs/scheduler.rs",
        240,
        "transactional_job_drain",
    ),
];

#[test]
fn current_gpui_shell_quickjs_surface_runs_against_jit_runtime() {
    assert_eq!(GPUI_SHELL_QUICKJS_INVENTORY.len(), 7);

    // runtime_creation + runtime_teardown
    let runtime = JitRuntime::builder().build().expect("JIT runtime");
    let runtime_dropped = Arc::new(AtomicBool::new(false));
    rquickjs_core::runtime::test_support::set_runtime_drop_probe(&runtime, {
        let runtime_dropped = Arc::clone(&runtime_dropped);
        move || runtime_dropped.store(true, Ordering::SeqCst)
    });

    // full_context_creation
    let context = JsContext::full(&runtime).expect("full shell context");

    // tuple_module_loader
    runtime.set_loader(
        BuiltinResolver::default().with_module("gpui:surface"),
        BuiltinLoader::default().with_module("gpui:surface", b"export const base = 40;".as_slice()),
    );

    // The shell applies these limits between loader and interrupt setup.
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);

    // interrupt_handler
    let interrupt_polls = Arc::new(AtomicUsize::new(0));
    runtime.set_interrupt_handler({
        let interrupt_polls = Arc::clone(&interrupt_polls);
        Some(Box::new(move || {
            interrupt_polls.fetch_add(1, Ordering::SeqCst);
            false
        }))
    });

    context.with(|ctx| {
        Module::evaluate(
            ctx.clone(),
            "gpui-shell-surface",
            r#"
            import { base } from "gpui:surface";
            globalThis.__gpuiModuleValue = base;
            "#,
        )
        .expect("module declaration")
        .finish::<()>()
        .expect("module evaluation");
        ctx.eval::<(), _>(
            "Promise.resolve(__gpuiModuleValue).then(value => { globalThis.__gpuiAsyncValue = value + 2 })",
        )
        .expect("promise scheduling");
    });

    // scoped_job_drain + test_job_drain + transactional_job_drain all use
    // this exact operation outside Context::with.
    let mut jobs = 0;
    loop {
        match runtime.execute_pending_job() {
            Ok(true) => jobs += 1,
            Ok(false) => break,
            Err(_) => panic!("gpui-shell promise fixture threw"),
        }
        assert!(jobs < 32, "promise fixture failed to quiesce");
    }
    assert!(jobs > 0);
    assert!(!runtime.is_job_pending());
    let value = context.with(|ctx| ctx.globals().get::<_, i32>("__gpuiAsyncValue").unwrap());
    assert_eq!(value, 42);

    drop(context);
    assert!(!runtime_dropped.load(Ordering::SeqCst));
    drop(runtime);
    assert!(runtime_dropped.load(Ordering::SeqCst));
    let _ = interrupt_polls;
}
