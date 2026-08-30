#![no_main]
use libfuzzer_sys::fuzz_target;
use rquickjs_core::{Context, Runtime};
use rquickjs_jit::correctness::canonical_observation_source;
fuzz_target!(|data: &[u8]| {
    let a = data.first().copied().unwrap_or(0);
    let b = data.get(1).copied().unwrap_or(0);
    let expression = format!(
        "(()=>{{let x={a};for(let i=0;i<{};i++)x=(x+{b})|0;return x}})()",
        data.len().min(32)
    );
    let source = canonical_observation_source(&expression);
    if let (Ok(left), Ok(right)) = (Runtime::new(), Runtime::new()) {
        let eval = |runtime: &Runtime| {
            Context::full(runtime)
                .ok()
                .and_then(|ctx| ctx.with(|cx| cx.eval::<String, _>(source.as_str()).ok()))
        };
        assert_eq!(eval(&left), eval(&right));
    }
});
