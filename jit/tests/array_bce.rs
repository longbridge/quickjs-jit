//! Loop-hoisted array metadata, bounds-check elimination and literal-tag
//! folding in Tier 2 must match the same-version QuickJS interpreter.
#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_endian = "little",
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use rquickjs::{Context, Runtime};
use rquickjs_jit::{Jit, JitConfig};
use std::time::{Duration, Instant};

fn evaluate_without_jit(source: &str, expression: &str) -> String {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>(source).unwrap();
        ctx.eval::<String, _>(expression).unwrap()
    })
}

/// Evaluates `warmup` until it enters a published Tier 2 body.
fn prepared(source: &str, warmup: &str) -> (Runtime, Jit, Context) {
    let runtime = Runtime::new().unwrap();
    let jit = Jit::attach(
        &runtime,
        JitConfig::builder()
            .call_threshold(2)
            .loop_threshold(4)
            .force_optimized_for_test(true)
            .stress_gc(true)
            .build()
            .unwrap(),
    )
    .unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| ctx.eval::<(), _>(source)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let before = jit.metrics();
        context.with(|ctx| ctx.eval::<(), _>(warmup)).unwrap();
        jit.poll();
        let after = jit.metrics();
        if after.tier2_entries > before.tier2_entries
            && after.deopts == before.deopts
            && after.pending_worker_jobs == 0
        {
            return (runtime, jit, context);
        }
        assert!(
            Instant::now() < deadline,
            "{warmup} never reached a stable Tier2 body: {after:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

const CONVERT: &str = r#"
    function convertAndSum(ints, floats) {
      let mixed = 0.0;
      for (let i = 0; i < ints.length; i++) {
        floats[i] = ints[i] * 0.25 + 0.5;
        mixed += floats[i];
      }
      return mixed;
    }
    function copyScaled(src, dst, n) {
      let last = -1;
      for (let i = 0; i < n; i++) {
        dst[i] = src[i] * 2;
        last = dst[i];
      }
      return last;
    }
"#;

#[test]
fn two_typed_arrays_share_one_loop_and_match_the_interpreter() {
    // Both receivers carry loop-hoisted metadata at once (one with the exact
    // length bound, one metadata-only) across the `a[i] = v` nullish-check
    // diamond. Aliased views observe every store, short destinations and
    // out-of-range counts leave the guarded path, and zero-trip loops still
    // return exactly what QuickJS returns.
    const CASES: &str = r#"JSON.stringify([
        convertAndSum(new Int32Array([1, 2, 3, 4]), new Float64Array(4)),
        convertAndSum(new Int32Array(0), new Float64Array(0)),
        convertAndSum(new Int32Array([8, 9, 10]), new Float64Array(2)),
        (() => { const buffer = new ArrayBuffer(64);
                 const ints = new Int32Array(buffer, 0, 4);
                 const floats = new Float64Array(buffer, 0, 4);
                 ints.set([7, 11, 13, 17]);
                 return [convertAndSum(ints, floats), Array.from(ints)]; })(),
        copyScaled(new Int32Array([3, 4, 5]), new Float64Array(3), 3),
        copyScaled(new Int32Array([3, 4, 5]), new Float64Array(3), 5),
        copyScaled(new Int32Array([3, 4, 5]), new Int32Array(2), 3),
        copyScaled(new Int32Array(0), new Float64Array(0), 0),
        copyScaled([1, 2, 3], new Float64Array(3), 3)
    ])"#;
    let expected = evaluate_without_jit(CONVERT, CASES);
    let (_runtime, jit, context) = prepared(
        CONVERT,
        "convertAndSum(new Int32Array([1,2,3,4,5,6,7,8]), new Float64Array(8));\
         copyScaled(new Int32Array([1,2,3,4,5,6,7,8]), new Float64Array(8), 8)",
    );
    let before = jit.metrics();
    let actual = context.with(|ctx| ctx.eval::<String, _>(CASES)).unwrap();
    let after = jit.metrics();
    assert_eq!(actual, expected);
    assert_eq!(after.native_entries, after.native_exits);
    assert!(
        after.tier2_entries > before.tier2_entries,
        "cases never entered Tier2: {before:?} -> {after:?}"
    );
    assert!(
        after.deopts > before.deopts,
        "short destinations and wrong modes must leave the guarded path: {before:?} -> {after:?}"
    );
}

#[test]
fn metadata_only_typed_hoist_revalidates_detach_after_observable_poll() {
    use rquickjs::qjs;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let (runtime, jit, context) = prepared(
        CONVERT,
        "copyScaled(new Int32Array([1,2,3,4,5,6,7,8]), new Float64Array(8), 8)",
    );
    let (raw_ctx, raw_destination, raw_buffer) = context.with(|ctx| {
        ctx.eval::<(), _>(
            "globalThis.src=new Int32Array(1000000).fill(3);\
             globalThis.dst=new Float64Array(1000000);globalThis.buffer=dst.buffer",
        )
        .unwrap();
        let destination: rquickjs::Object = ctx.globals().get("dst").unwrap();
        let buffer: rquickjs::Object = ctx.globals().get("buffer").unwrap();
        (
            ctx.as_raw().as_ptr() as usize,
            unsafe { qjs::JS_VALUE_GET_PTR(destination.as_value().as_raw()) } as usize,
            unsafe { qjs::JS_VALUE_GET_PTR(buffer.as_value().as_raw()) } as usize,
        )
    });
    let detached = Arc::new(AtomicBool::new(false));
    let observed = detached.clone();
    // Detach the destination only after native stores are visible. The
    // interrupt runs at a loop poll, which must revalidate the hoisted
    // data/count before any later iteration writes through them.
    runtime.set_interrupt_handler(Some(Box::new(move || unsafe {
        let ctx = raw_ctx as *mut qjs::JSContext;
        let destination = qjs::JS_MKPTR(
            qjs::JS_TAG_OBJECT,
            raw_destination as *mut core::ffi::c_void,
        );
        let value = qjs::JS_GetPropertyUint32(ctx, destination, 1);
        let tag = qjs::JS_VALUE_GET_TAG(value);
        let started = (tag == qjs::JS_TAG_INT && qjs::JS_VALUE_GET_INT(value) == 6)
            || (tag == qjs::JS_TAG_FLOAT64 && qjs::JS_VALUE_GET_FLOAT64(value) == 6.0);
        qjs::JS_FreeValue(ctx, value);
        if started && !observed.swap(true, Ordering::SeqCst) {
            let buffer = qjs::JS_MKPTR(qjs::JS_TAG_OBJECT, raw_buffer as *mut core::ffi::c_void);
            qjs::JS_DetachArrayBuffer(ctx, buffer);
        }
        false
    })));
    let before = jit.metrics();
    let result = context.with(|ctx| {
        ctx.eval::<bool, _>("copyScaled(src, dst, src.length) === undefined && dst.length === 0")
    });
    runtime.set_interrupt_handler(None);
    assert!(
        detached.load(Ordering::SeqCst),
        "no poll observed completed stores"
    );
    assert!(
        result.unwrap(),
        "a store or load used stale hoisted metadata after detach"
    );
    let after = jit.metrics();
    assert!(after.tier2_entries > before.tier2_entries);
    assert_eq!(after.native_entries, after.native_exits);
}

#[test]
fn rebound_argument_keeps_its_value_when_an_alias_receiver_is_hoisted() {
    // The traversal reads through a local alias of the entry argument while
    // the loop rebinds the argument itself. The argument's representation
    // must come from the rebinding, never from the receiver guard.
    const SOURCE: &str = r#"
        function rebind(a) {
          let b = a, s = 0;
          for (let i = 0; i < b.length; i++) { s = (s + b[i]) | 0; a = i; }
          return a * 1000 + s;
        }
        function keep(a) {
          let s = 0;
          for (let i = 0; i < a.length; i++) s = (s + a[i]) | 0;
          return a.length * 1000 + s;
        }
    "#;
    const CASES: &str = r#"JSON.stringify([
        rebind(new Int32Array([1, 2, 3])), rebind(new Int32Array(0)),
        rebind([4, 5, 6, 7]), rebind([]),
        keep(new Int32Array([1, 2, 3])), keep([9, 8]), keep(new Float64Array([0.5]))
    ])"#;
    let expected = evaluate_without_jit(SOURCE, CASES);
    let (_runtime, jit, context) = prepared(
        SOURCE,
        "rebind(new Int32Array([1,2,3,4,5,6,7,8]));keep(new Int32Array([1,2,3,4,5,6,7,8]))",
    );
    let before = jit.metrics();
    let actual = context.with(|ctx| ctx.eval::<String, _>(CASES)).unwrap();
    let after = jit.metrics();
    assert_eq!(actual, expected);
    assert_eq!(after.native_entries, after.native_exits);
    assert!(after.tier2_entries > before.tier2_entries);
}

#[test]
fn literal_tag_folding_keeps_exact_numeric_semantics() {
    // Numeric constant-pool literals, literal Bool/Int32 branch conditions,
    // hoisted Int32 lengths and checked overflow must all behave exactly as
    // in QuickJS, including the Float64 overflow and NaN comparisons.
    const SOURCE: &str = r#"
        function literals(values, seed) {
          let acc = seed, flag = true;
          for (let i = 0; i < values.length; i++) {
            acc = acc + values[i] * 0.5 + 1.25;
            if (flag) acc = acc - 0.25;
            flag = i < 2;
          }
          return acc;
        }
        function overflow(values) {
          let sum = 2147483000;
          for (let i = 0; i < values.length; i++) sum = sum + values[i];
          return sum;
        }
    "#;
    const CASES: &str = r#"JSON.stringify([
        literals(new Int32Array([1, 2, 3, 4]), 0),
        literals(new Float64Array([0.5, NaN, 2]), 1),
        literals([1, 2.5, -3], 0.75),
        literals(new Int32Array(0), 7),
        overflow(new Int32Array([600, 100])),
        overflow([1, 2, 3]),
        overflow(new Float64Array([1e300, 1e300]))
    ])"#;
    let expected = evaluate_without_jit(SOURCE, CASES);
    let (_runtime, jit, context) = prepared(
        SOURCE,
        "literals(new Int32Array([1,2,3,4,5,6,7,8]),0);overflow(new Int32Array([1,2,3,4,5,6,7,8]))",
    );
    let before = jit.metrics();
    let actual = context.with(|ctx| ctx.eval::<String, _>(CASES)).unwrap();
    let after = jit.metrics();
    assert_eq!(actual, expected);
    assert_eq!(after.native_entries, after.native_exits);
    assert!(after.tier2_entries > before.tier2_entries);
}
