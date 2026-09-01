#![no_main]

use libfuzzer_sys::fuzz_target;
use rquickjs_core::{Context, Runtime};
use rquickjs_jit::{
    correctness::{canonical_observation_call_source, canonical_observer_prelude},
    JitConfig, JitRuntime,
};

fn eligible_program(data: &[u8]) -> Option<(String, String)> {
    if data.len() < 3 {
        return None;
    }
    let hash = data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |state, byte| {
        (state ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    });
    let a = i16::from(hash as u8) - 128;
    let b = i16::from((hash >> 17) as u8) - 128;
    let definition = match (hash >> 32) % 3 {
        0 => "function f(a,b){return a+b}",
        1 => "function f(a,b){let x=a+b;x++;return x}",
        2 => "function f(a,b){let x=a+b;if(x)return x;return b}",
        _ => unreachable!(),
    };
    Some((definition.into(), format!("f({a},{b})")))
}

fuzz_target!(|data: &[u8]| {
    // Inputs outside this deliberately closed grammar are discarded, never
    // compared through fallback (which would create false differential cover).
    let Some((definition, invocation)) = eligible_program(data) else {
        return;
    };
    let observation = canonical_observation_call_source(&invocation);
    let Ok(interpreter) = Runtime::new() else {
        return;
    };
    let expected = Context::full(&interpreter).ok().and_then(|context| {
        context.with(|ctx| {
            ctx.eval::<(), _>(canonical_observer_prelude()).ok()?;
            ctx.eval::<(), _>(definition.as_str()).ok()?;
            ctx.eval::<String, _>(observation.as_str()).ok()
        })
    });

    let config = JitConfig::builder()
        .call_threshold(1)
        .loop_threshold(1)
        .build()
        .unwrap();
    let Ok(jit) = JitRuntime::builder().config(config).build() else {
        return;
    };
    let Some(context) = Context::full(&jit).ok() else {
        return;
    };
    if context
        .with(|ctx| ctx.eval::<(), _>(canonical_observer_prelude()))
        .is_err()
    {
        return;
    }
    if context
        .with(|ctx| ctx.eval::<(), _>(definition.as_str()))
        .is_err()
    {
        return;
    }
    let warm = format!("for(let i=0;i<256;i++){{{};}}", invocation);
    for _ in 0..128 {
        if context
            .with(|ctx| ctx.eval::<(), _>(warm.as_str()))
            .is_err()
        {
            return;
        }
        jit.jit().poll();
        if jit.metrics().native_entries > 0 {
            break;
        }
    }
    // Replay the exact generated program after warming.
    let actual = context.with(|ctx| ctx.eval::<String, _>(observation.as_str()).ok());
    let metrics = jit.metrics();
    assert!(
        metrics.installed > 0,
        "eligible program was not installed: {metrics:?}"
    );
    assert!(
        metrics.native_entries > 0,
        "eligible program never entered Tier 1: {metrics:?}"
    );
    assert_eq!(
        metrics.native_fallbacks, 0,
        "fallback masked coverage: {metrics:?}"
    );
    assert_eq!(
        metrics.native_retries, 0,
        "retry masked coverage: {metrics:?}"
    );
    assert_eq!(
        actual, expected,
        "definition={definition}; invocation={invocation}"
    );
});
