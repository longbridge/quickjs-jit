#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::{Jit, JitConfig, JitTierPolicy};
use std::time::{Duration, Instant};

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
        checksum = context
            .with(|ctx| ctx.eval("elementKernel(a,t,p,1,40)").unwrap());
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
    let events = context
        .with(|ctx| ctx.eval::<String, _>("events.slice(-2).join(',')").unwrap());
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
    assert_eq!(accessor_and_exception, r#"[41,["set:42","get"],"element boom"]"#);
}
