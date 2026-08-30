#![no_main]
use libfuzzer_sys::fuzz_target;
use rquickjs_core::{Context, Runtime};
use rquickjs_jit::{correctness::{canonical_observation_source, StructuredProgram}, JitRuntime};
fuzz_target!(|data: &[u8]| {
    let mut seed_bytes = [0; 8];
    for (slot, byte) in seed_bytes.iter_mut().zip(data.iter().copied()) { *slot = byte; }
    let program = StructuredProgram::generate(u64::from_le_bytes(seed_bytes), data.len().min(256) as u16);
    let source = canonical_observation_source(program.source());
    if let (Ok(left), Ok(right)) = (Runtime::new(), JitRuntime::builder().build()) {
        let eval = |runtime: &Runtime| {
            Context::full(runtime)
                .ok()
                .and_then(|ctx| ctx.with(|cx| cx.eval::<String, _>(source.as_str()).ok()))
        };
        assert_eq!(eval(&left), eval(&right));
    }
});
