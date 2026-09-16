#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::{Jit, JitConfig, JitTierPolicy};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn JS_JitGetHelperCount(
        rt: *mut rquickjs_core::qjs::JSRuntime,
        helper_id: u32,
        count: *mut u64,
    ) -> i32;
}

fn call_count(context: &Context) -> u64 {
    context.with(|ctx| {
        let rt = unsafe { rquickjs_core::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let mut count = 0;
        assert_eq!(
            unsafe {
                JS_JitGetHelperCount(
                    rt,
                    rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_CALL,
                    &mut count,
                )
            },
            0
        );
        count
    })
}

fn exercise(policy: JitTierPolicy, force_tier2: bool, miss: (&str, &str, &str, &str)) {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(policy)
            .call_threshold(2)
            .loop_threshold(16)
            .force_optimized_for_test(force_tier2)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.marker={stable:true};
                 function linkedLeaf(object,value){return value+1}
                 function otherLeaf(object,value){return value+20}
                 function linkedCaller(target,object,value,n){
                   for(let i=0;i<n;i++)value=target(object,value);
                   return value;
                 }",
            )
        })
        .unwrap();
    let invoke = |target: &str, object: &str, value: &str, n: usize| -> String {
        context.with(|ctx| {
            ctx.eval::<String, _>(format!(
                "String(linkedCaller({target},{object},{value},{n}))"
            ))
            .unwrap()
        })
    };

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let before = call_count(&context);
        let entries = jit.metrics().native_entries;
        assert_eq!(invoke("linkedLeaf", "marker", "7", 128), "135");
        jit.poll();
        let after = jit.metrics();
        if after.pending_worker_jobs == 0
            && after.native_entries > entries
            && (!force_tier2 || after.tier2_entries > 0)
            && call_count(&context) == before
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "linked edge not ready: {after:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let before = call_count(&context);
    assert_eq!(invoke("linkedLeaf", "marker", "7", 128), "135");
    assert_eq!(
        call_count(&context),
        before,
        "stable HeapRef edge still used generic CALL"
    );

    // The miss must resume the untouched original CALL. Each case gets a
    // fresh runtime because a Tier2 miss may deliberately demote the caller.
    // A Tier2 CALL deopt may be followed by a callee-entry representation
    // retry, so runtime transition counters are not an invocation count.
    let before = call_count(&context);
    let deopts = jit.metrics().deopts;
    assert_eq!(invoke(miss.0, miss.1, miss.2, 1), miss.3);
    assert!(
        call_count(&context) - before + jit.metrics().deopts - deopts >= 1,
        "miss did not take a compiled helper or exact CALL deopt: {miss:?}"
    );
    assert_eq!(jit.metrics().native_entries, jit.metrics().native_exits);
}

#[test]
fn baseline_target_only_heapref_link_preserves_generic_misses() {
    for miss in [
        ("otherLeaf", "marker", "7", "27"),
        ("linkedLeaf", "7", "7", "8"),
        ("linkedLeaf", "marker", "2147483647", "2147483648"),
    ] {
        exercise(JitTierPolicy::BaselineOnly, false, miss);
    }
}

#[test]
fn tier2_target_only_heapref_link_preserves_generic_misses() {
    for miss in [
        ("otherLeaf", "marker", "7", "27"),
        ("linkedLeaf", "7", "7", "8"),
        ("linkedLeaf", "marker", "2147483647", "2147483648"),
    ] {
        exercise(JitTierPolicy::Automatic, true, miss);
    }
}

fn exercise_bool_result(policy: JitTierPolicy, force_tier2: bool) {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(policy)
            .call_threshold(2)
            .loop_threshold(16)
            .force_optimized_for_test(force_tier2)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.boolMarker={stable:true};
                 function linkedBool(object,enabled){return enabled}
                 function boolCaller(target,object,enabled,n){
                   let result=false;
                   for(let i=0;i<n;i++)result=target(object,enabled);
                   return result;
                 }",
            )
        })
        .unwrap();
    let invoke = |target: &str, object: &str, enabled: &str, n: usize| -> bool {
        context.with(|ctx| {
            ctx.eval::<bool, _>(format!("boolCaller({target},{object},{enabled},{n})"))
                .unwrap()
        })
    };

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let before = call_count(&context);
        let entries = jit.metrics().native_entries;
        assert!(invoke("linkedBool", "boolMarker", "true", 128));
        jit.poll();
        let after = jit.metrics();
        if after.pending_worker_jobs == 0
            && after.native_entries > entries
            && (!force_tier2 || after.tier2_entries > 0)
            && call_count(&context) == before
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "linked bool edge not ready: {after:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let before = call_count(&context);
    assert!(!invoke("linkedBool", "boolMarker", "false", 128));
    assert_eq!(
        call_count(&context),
        before,
        "stable Bool result still used the generic CALL frame"
    );
    let before = call_count(&context);
    let deopts = jit.metrics().deopts;
    assert!(invoke("linkedBool", "7", "true", 1));
    assert!(
        call_count(&context) > before || jit.metrics().deopts > deopts,
        "HeapRef tag miss did not retain the generic fallback"
    );
    assert_eq!(jit.metrics().native_entries, jit.metrics().native_exits);
}

#[test]
fn baseline_target_only_heapref_link_returns_bool_without_a_call_frame() {
    exercise_bool_result(JitTierPolicy::BaselineOnly, false);
}

#[test]
fn tier2_target_only_heapref_link_returns_bool_without_a_call_frame() {
    exercise_bool_result(JitTierPolicy::Automatic, true);
}
