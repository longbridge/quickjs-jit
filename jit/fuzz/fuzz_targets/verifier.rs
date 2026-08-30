#![no_main]
use libfuzzer_sys::fuzz_target;
use rquickjs_core::{context::EvalOptions, Context, Function, Runtime};
use rquickjs_jit::bytecode::{decode_raw, tier1_policy, CompileSnapshot, VerifyLimits};
fuzz_target!(|data: &[u8]| {
    if let Ok(instructions) = decode_raw(data) {
        for instruction in instructions {
            let _ = tier1_policy(instruction.opcode().id());
        }
    }
    let a = data.first().copied().unwrap_or(0);
    let n = data.len().min(32);
    let source = format!("function f(x){{let y=x+{a};for(let i=0;i<{n};i++)y=(y+i)|0;return y}}");
    if let (Ok(runtime),) = (Runtime::new(),) {
        if let Ok(context) = Context::full(&runtime) {
            context.with(|ctx| {
                let mut options = EvalOptions::default();
                options.global = true;
                options.strict = false;
                if ctx
                    .eval_with_options::<(), _>(source.as_str(), options)
                    .is_ok()
                {
                    if let Ok(function) = ctx.globals().get::<_, Function<'_>>("f") {
                        if let Ok(snapshot) = unsafe {
                            CompileSnapshot::capture_raw(
                                ctx.as_raw().as_ptr(),
                                function.as_value().as_raw(),
                            )
                        } {
                            let _ = snapshot.verify(VerifyLimits::default());
                        }
                    }
                }
            });
        }
    }
});
