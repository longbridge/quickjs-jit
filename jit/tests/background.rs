use std::{sync::Arc, time::Instant};

use rquickjs::{Context, Runtime};
use rquickjs_jit::{
    bytecode::{opcode, CompileSnapshot, VerifyLimits},
    code_cache::CompiledArtifact,
    compiler::mock::FakeCompiler,
    runtime::{BackgroundCompiler, CompileState, Coordinator, FunctionKey, Tier},
};

fn snapshot(id: u64, generation: u64) -> rquickjs_jit::bytecode::VerifiedFunction {
    let _ = (id, generation);
    CompileSnapshot::from_untrusted_bytecode(vec![opcode::RETURN_UNDEF], 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .unwrap()
}

#[test]
fn foreground_submission_never_waits_for_blocked_compiler() {
    let (compiler, control) = FakeCompiler::new(1);
    let mut workers = BackgroundCompiler::new(Arc::new(compiler), 1, 1).unwrap();
    let mut coordinator = Coordinator::with_limits(1, 1, 4, 1024);
    let key = FunctionKey::new(7, 1);
    coordinator
        .queue(key, Tier::Baseline, snapshot(7, 1))
        .unwrap();

    let started = Instant::now();
    assert!(workers.dispatch_next(&mut coordinator).unwrap());
    assert!(started.elapsed() < std::time::Duration::from_millis(100));
    assert_eq!(control.next_request().unwrap().key(), key);
    assert_eq!(coordinator.drain_completions().drained(), 0);

    control.complete(CompiledArtifact::fake(Tier::Baseline));
    workers.shutdown(&mut coordinator);
    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
}

#[test]
fn stale_completion_after_reload_is_discarded() {
    let (compiler, control) = FakeCompiler::new(1);
    let mut workers = BackgroundCompiler::new(Arc::new(compiler), 1, 1).unwrap();
    let mut coordinator = Coordinator::with_limits(2, 2, 4, 1024);
    let old = FunctionKey::new(11, 1);
    coordinator
        .queue(old, Tier::Baseline, snapshot(11, 1))
        .unwrap();
    workers.dispatch_next(&mut coordinator).unwrap();
    assert_eq!(control.next_request().unwrap().key(), old);

    let new = FunctionKey::new(11, 2);
    coordinator.retire(old);
    coordinator
        .queue(new, Tier::Baseline, snapshot(11, 2))
        .unwrap();
    control.complete(CompiledArtifact::fake(Tier::Baseline));
    workers.shutdown(&mut coordinator);

    assert_eq!(coordinator.state(old), CompileState::Retired);
    assert_eq!(coordinator.state(new), CompileState::Queued(Tier::Baseline));
    assert_eq!(coordinator.metrics().stale_results, 1);
}

#[test]
fn saturated_worker_mailbox_rolls_request_back_to_queue() {
    let (compiler, control) = FakeCompiler::new(2);
    let mut workers = BackgroundCompiler::new(Arc::new(compiler), 1, 1).unwrap();
    let mut coordinator = Coordinator::with_limits(3, 3, 4, 1024);
    for id in 1..=3 {
        coordinator
            .queue(FunctionKey::new(id, 1), Tier::Baseline, snapshot(id, 1))
            .unwrap();
    }
    assert!(workers.dispatch_next(&mut coordinator).unwrap());
    let _first = control.next_request().unwrap();
    assert!(workers.dispatch_next(&mut coordinator).unwrap());
    assert!(!workers.dispatch_next(&mut coordinator).unwrap());
    assert_eq!(coordinator.metrics().worker_queue_saturated, 1);

    control.complete(CompiledArtifact::fake(Tier::Baseline));
    let _ = control.next_request().unwrap();
    control.complete(CompiledArtifact::fake(Tier::Baseline));
    workers.shutdown(&mut coordinator);
    assert_eq!(
        coordinator.state(FunctionKey::new(3, 1)),
        CompileState::Queued(Tier::Baseline)
    );
}

#[test]
fn shutdown_cooperatively_cancels_a_blocked_compiler() {
    let (compiler, control) = FakeCompiler::new(1);
    let mut workers = BackgroundCompiler::new(Arc::new(compiler), 1, 1).unwrap();
    let mut coordinator = Coordinator::with_limits(1, 1, 4, 1024);
    coordinator
        .queue(FunctionKey::new(19, 1), Tier::Baseline, snapshot(19, 1))
        .unwrap();
    workers.dispatch_next(&mut coordinator).unwrap();
    assert!(control.next_request().is_some());
    workers.shutdown(&mut coordinator);
    assert_eq!(coordinator.metrics().compile_failures, 1);
}

#[test]
fn saturated_completion_mailbox_does_not_deadlock_shutdown() {
    let (compiler, control) = FakeCompiler::new(2);
    let mut workers = BackgroundCompiler::new(Arc::new(compiler), 1, 2).unwrap();
    let mut coordinator = Coordinator::with_limits(2, 1, 4, 1024);
    for id in 31..=32 {
        coordinator
            .queue(FunctionKey::new(id, 1), Tier::Baseline, snapshot(id, 1))
            .unwrap();
        workers.dispatch_next(&mut coordinator).unwrap();
        assert!(control.next_request().is_some());
        control.complete(CompiledArtifact::fake(Tier::Baseline));
    }
    workers.shutdown(&mut coordinator);
    assert_eq!(coordinator.metrics().completion_queue_saturated, 1);
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
#[test]
fn production_backend_receives_owned_snapshots_automatically() {
    let runtime = Runtime::new().unwrap();
    let jit = rquickjs_jit::Jit::attach(&runtime, rquickjs_jit::JitConfig::default()).unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let value: i32 = ctx
            .eval("globalThis.add = function add(a, b) { return a + b; }; add(20, 22)")
            .unwrap();
        assert_eq!(value, 42);
    });
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while Instant::now() < deadline {
        jit.poll();
        if jit.metrics().installed >= 1 {
            break;
        }
        std::thread::yield_now();
    }
    assert!(jit.metrics().queued >= 1);
    assert!(jit.metrics().installed >= 1, "{:?}", jit.metrics());
    context.with(|ctx| {
        let value: i32 = ctx.eval("add(20, 22)").unwrap();
        assert_eq!(value, 42);
    });
}
