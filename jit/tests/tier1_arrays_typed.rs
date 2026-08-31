#![cfg(all(
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

fn helper_count(rt: *mut rquickjs_core::qjs::JSRuntime, helper_id: u32) -> u64 {
    let mut count = 0;
    assert_eq!(
        unsafe { JS_JitGetHelperCount(rt, helper_id, &mut count) },
        0
    );
    count
}

#[test]
fn arrays_typed_arrays_proxy_and_accessor_enter_native_with_exact_checksum() {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(JitTierPolicy::BaselineOnly)
            .call_threshold(1)
            .loop_threshold(1)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
            globalThis.getElement = function getElement(o, k) { let v = o[k]; return v + 0; };
            globalThis.setElement = function setElement(o, k, v) { o[k] = v; return v + 0; };
            globalThis.elementKernel = function elementKernel(a, t, p, k, v) {
              setElement(a, k, v);
              setElement(t, k, getElement(a, k) + 1);
              setElement(p, k, getElement(t, k) + 1);
              return getElement(a, k) * 10000 + getElement(t, k) * 100 + getElement(p, k);
            };
            globalThis.a = [0, 0, 0];
            globalThis.t = new Float64Array(3);
            globalThis.events = [];
            globalThis.p = new Proxy({}, {
              get(o, k) { events.push('g:' + k); return o[k]; },
              set(o, k, v) { events.push('s:' + k); o[k] = v; return true; }
            });
            "#,
        )
        .unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut checksum = 0_i32;
    while Instant::now() < deadline {
        checksum = context.with(|ctx| ctx.eval("elementKernel(a,t,p,1,40)").unwrap());
        jit.poll();
        if jit.metrics().native_entries > 0 {
            break;
        }
    }
    assert_eq!(checksum, 404_142);
    let metrics = jit.metrics();
    assert!(metrics.installed > 0, "{metrics:?}");
    assert!(metrics.native_entries > 0, "{metrics:?}");
    assert_eq!(metrics.native_entries, metrics.native_exits, "{metrics:?}");
    let events = context.with(|ctx| ctx.eval::<String, _>("events.slice(-2).join(',')").unwrap());
    assert_eq!(events, "s:1,g:1");
    let accessor_and_exception = context.with(|ctx| {
        ctx.eval::<String, _>(
            r#"
            (() => {
              const seen = [];
              const accessor = {
                get x() { seen.push('get'); return 41; },
                set x(v) { seen.push('set:' + v); }
              };
              setElement(accessor, 'x', 42);
              const value = getElement(accessor, 'x');
              const throwing = new Proxy({}, { get() { throw new Error('element boom'); } });
              let error;
              try { getElement(throwing, 'x'); } catch (e) { error = e.message; }
              return JSON.stringify([value, seen, error]);
            })()
            "#,
        )
        .unwrap()
    });
    assert_eq!(
        accessor_and_exception,
        r#"[41,["set:42","get"],"element boom"]"#
    );
}

#[test]
fn packed_and_typed_element_hits_skip_get_set_helpers_and_fallbacks_stay_exact() {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(JitTierPolicy::BaselineOnly)
            .call_threshold(1)
            .loop_threshold(1)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                r#"
                globalThis.packed = function packed(a, i) { a[i] = i + 3; return a[i]; };
                globalThis.i32 = function i32(a, i) { a[i] = i + 5; return a[i]; };
                globalThis.f64 = function f64(a, i) { a[i] = i + 0.5; return a[i]; };
                globalThis.readElement = function readElement(a, i) { return a[i]; };
                globalThis.writeElement = function writeElement(a, i, v) { a[i] = v; return a.length; };
                globalThis.packedArray = Array(32).fill(0);
                globalThis.i32Array = new Int32Array(32);
                globalThis.f64Array = new Float64Array(32);
                "#,
            )
        })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<i32, _>("packed(packedArray,10)"))
                .unwrap(),
            13
        );
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<i32, _>("i32(i32Array,10)"))
                .unwrap(),
            15
        );
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<f64, _>("f64(f64Array,10)"))
                .unwrap(),
            10.5
        );
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
    }
    assert!(jit.metrics().installed >= 2, "{:?}", jit.metrics());
    let rt =
        context.with(|ctx| unsafe { rquickjs_core::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) });
    assert_eq!(
        unsafe { rquickjs_core::qjs::JS_JitResetHelperCounters(rt) },
        0
    );
    let entries_before = jit.metrics().native_entries;
    for _ in 0..64 {
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<i32, _>("packed(packedArray,10)"))
                .unwrap(),
            13
        );
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<i32, _>("i32(i32Array,10)"))
                .unwrap(),
            15
        );
        assert_eq!(
            context
                .with(|ctx| ctx.eval::<f64, _>("f64(f64Array,10)"))
                .unwrap(),
            10.5
        );
    }
    assert!(
        jit.metrics().native_entries > entries_before,
        "{:?}",
        jit.metrics()
    );
    assert_eq!(
        helper_count(
            rt,
            rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_GET_ELEMENT
        ),
        0,
        "direct element loads reached the generic helper"
    );
    assert_eq!(
        helper_count(
            rt,
            rquickjs_core::qjs::JSJitHelperId_JS_JIT_HELPER_SET_ELEMENT
        ),
        0,
        "direct element stores reached the generic helper"
    );

    context
        .with(|ctx| {
            ctx.eval::<(), _>(
                r#"
                globalThis.events = [];
                globalThis.slowArray = [1, 2, 3];
                Object.defineProperty(slowArray, '1', {
                  get() { events.push('get'); return 41; },
                  set(v) { events.push('set:' + v); }
                });
                globalThis.immutableI32 = new Int32Array(
                  new ArrayBuffer(4).transferToImmutable()
                );
                "#,
            )
        })
        .unwrap();
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<i32, _>("readElement(slowArray,1)"))
            .unwrap(),
        41
    );
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<i32, _>("writeElement(slowArray,1,9)"))
            .unwrap(),
        3
    );
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<String, _>("events.join(',')"))
            .unwrap(),
        "get,set:9"
    );
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<i32, _>("writeElement(slowArray,9,7)"))
            .unwrap(),
        10
    );
    assert_eq!(
        context
            .with(|ctx| ctx.eval::<i32, _>("i32(immutableI32,0)"))
            .unwrap(),
        0
    );
}

#[test]
fn arrays_typed_workload_keeps_native_entries_bounded_per_invocation() {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .tier_policy(JitTierPolicy::BaselineOnly)
            .call_threshold(1)
            .loop_threshold(1)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context
        .with(|ctx| ctx.eval::<(), _>(include_str!("../../benchmarks/scripts/arrays-typed.js")))
        .unwrap();

    let run = || {
        context.with(|ctx| {
            ctx.eval::<String, _>("workload(2000, 0)")
                .expect("arrays-typed workload evaluates")
        })
    };
    // Sanitizer instrumentation makes each queued compilation substantially
    // slower, and this workload discovers many hot functions at once. Keep the
    // multi-install precondition while allowing the single compiler worker to
    // make progress through that queue.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        assert_eq!(run(), "33983000:8496750.000:2000");
        jit.poll();
        if jit.metrics().installed >= 2 {
            break;
        }
    }
    assert!(jit.metrics().installed >= 2, "{:?}", jit.metrics());

    let entries_before = jit.metrics().native_entries;
    for _ in 0..8 {
        assert_eq!(run(), "33983000:8496750.000:2000");
    }
    let entries = jit.metrics().native_entries - entries_before;
    assert!(
        entries <= 32,
        "arrays-typed crossed the native boundary {entries} times for eight workload calls: {:?}",
        jit.metrics()
    );
}
