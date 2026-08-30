fn main() {
    let runtime = rquickjs::Runtime::new().expect("runtime");
    let jit = rquickjs_jit::Jit::attach(&runtime, rquickjs_jit::JitConfig::default()).expect("jit");
    let _context = rquickjs::Context::full(&runtime).expect("context");
    println!("{}", jit.metrics().native_enabled());
}
