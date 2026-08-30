fn main() {
    let runtime = rquickjs::Runtime::new().expect("runtime");
    let _context = rquickjs::Context::full(&runtime).expect("context");
    println!("rquickjs");
}
