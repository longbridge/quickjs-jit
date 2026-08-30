#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::{Jit, JitConfig, JitTierPolicy};
use std::time::{Duration, Instant};

fn run_until_native(source: &str, expression: &str) -> (String, rquickjs_jit::JitMetrics) {
    let runtime = Runtime::new().unwrap();
    let config = JitConfig::builder()
        .tier_policy(JitTierPolicy::BaselineOnly)
        .call_threshold(1)
        .loop_threshold(1)
        .build()
        .unwrap();
    let jit = Jit::attach(&runtime, config).unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| ctx.eval::<(), _>(source)).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut result = String::new();
    while Instant::now() < deadline {
        result = context.with(|ctx| ctx.eval(expression)).unwrap();
        jit.poll();
        if jit.metrics().native_entries > 0 {
            break;
        }
    }
    (result, jit.metrics())
}

#[test]
fn production_tier1_installs_and_executes_fixed_and_generic_calls() {
    let (result, metrics) = run_until_native(
        "globalThis.g=(a,b,c,d)=>a+b+c+d;\
         globalThis.f=function f(g,a,b,c,d){let value=g(a,b,c,d);return value+0};",
        "String(f(g,5,10,12,15))",
    );
    assert_eq!(result, "42");
    assert!(metrics.installed > 0, "{metrics:?}");
    assert!(metrics.native_entries > 0, "{metrics:?}");
    assert_eq!(metrics.native_entries, metrics.native_exits, "{metrics:?}");
}

#[test]
fn production_tier1_installs_and_executes_method_property_path() {
    let (result, metrics) = run_until_native(
        "globalThis.o={base:20,add(x){return this.base+x}};\
         globalThis.f=function f(o,x){o.answer=x;let value=o.add(o.answer);return value+0};",
        "String(f(o,22))",
    );
    assert_eq!(result, "42");
    assert!(metrics.installed > 0, "{metrics:?}");
    assert!(metrics.native_entries > 0, "{metrics:?}");
    assert_eq!(metrics.native_entries, metrics.native_exits, "{metrics:?}");
}
