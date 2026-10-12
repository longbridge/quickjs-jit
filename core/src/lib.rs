//! # High-level bindings to QuickJS
//!
//! The `rquickjs` crate provides safe high-level bindings to the [QuickJS](https://bellard.org/quickjs/) JavaScript engine.
//! This crate is heavily inspired by the [rlua](https://crates.io/crates/rlua) crate.

#![allow(unknown_lints)]
#![allow(clippy::needless_lifetimes)]
#![allow(clippy::uninlined_format_args)]
#![allow(mismatched_lifetime_syntaxes)]
#![cfg_attr(feature = "doc-cfg", feature(doc_cfg))]
#![allow(clippy::doc_lazy_continuation)]
#![cfg_attr(not(test), no_std)]

#[doc(hidden)]
pub extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

pub(crate) use alloc::string::String as StdString;
pub(crate) use core::result::Result as StdResult;

mod js_lifetime;
pub mod markers;
mod persistent;
mod result;
mod safe_ref;
mod util;
mod value;
pub(crate) use safe_ref::*;
pub mod runtime;
pub use runtime::Runtime;
pub mod context;
pub use context::{Context, Ctx};
pub mod class;
pub use class::Class;
pub use js_lifetime::JsLifetime;
pub use persistent::Persistent;
pub use result::{CatchResultExt, CaughtError, CaughtResult, Error, Result, ThrowResultExt};
pub use value::{
    array, atom, convert, function, module, object, promise, proxy, Array, Atom, BigInt, CString,
    Coerced, Constructor, Exception, Filter, FromAtom, FromIteratorJs, FromJs, Function, IntoAtom,
    IntoJs, IteratorJs, Module, Null, Object, Promise, Proxy, String, Symbol, Type, Undefined,
    Value, WriteOptions, WriteOptionsEndianness,
};

pub mod allocator;
#[cfg(feature = "loader")]
#[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "loader")))]
pub mod loader;

#[cfg(feature = "futures")]
#[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "futures")))]
pub use context::AsyncContext;
#[cfg(feature = "multi-ctx")]
pub use context::MultiWith;
#[cfg(feature = "futures")]
#[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "futures")))]
pub use runtime::AsyncRuntime;
pub use value::{
    ArrayBuffer, ArrayBufferSource, Iterable, IterableFn, JsIterator, TypedArray, U8Clamped,
};

//#[doc(hidden)]
pub mod qjs {
    //! Native low-level bindings
    pub use rquickjs_sys::*;
}

#[cfg(feature = "phf")]
#[doc(hidden)]
pub mod phf {
    pub use phf::*;
}

pub mod prelude {
    //! A group of often used types.
    #[cfg(feature = "multi-ctx")]
    pub use crate::context::MultiWith;
    pub use crate::{
        context::Ctx,
        convert::{Coerced, FromAtom, FromIteratorJs, FromJs, IntoAtom, IntoJs, IteratorJs, List},
        function::{
            Exhaustive, Flat, Func, FuncArg, IntoArg, IntoArgs, MutFn, OnceFn, Opt, Rest, This,
        },
        result::{CatchResultExt, ThrowResultExt},
        JsLifetime,
    };
    #[cfg(feature = "futures")]
    #[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "futures")))]
    pub use crate::{
        function::Async,
        promise::{Promise, Promised},
    };
}

#[cfg(test)]
pub(crate) fn test_with<F, R>(func: F) -> R
where
    F: FnOnce(Ctx) -> R,
{
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    ctx.with(func)
}

mod deprecated_features {
    #[cfg(feature = "properties")]
    #[allow(unused_imports)]
    use properties as _;
    #[cfg(feature = "properties")]
    #[deprecated(
        note = "The rquickjs crate feature `properties` is deprecated, the functionality it provided is now enabled by default.
To remove this warning remove the use of the feature when specifying the dependency."
    )]
    mod properties {}

    #[cfg(feature = "array-buffer")]
    #[allow(unused_imports)]
    use array_buffer as _;
    #[cfg(feature = "array-buffer")]
    #[deprecated(
        note = "The rquickjs crate feature `array-buffer` is deprecated, the functionality it provided is now enabled by default.
To remove this warning remove the use of the feature when specifying the dependency."
    )]
    mod array_buffer {}

    #[cfg(feature = "classes")]
    #[allow(unused_imports)]
    use classes as _;
    #[cfg(feature = "classes")]
    #[deprecated(
        note = "The rquickjs crate feature `classes` is deprecated, the functionality it provided is now enabled by default.
To remove this warning remove the use of the feature when specifying the dependency."
    )]
    mod classes {}

    #[cfg(feature = "allocator")]
    #[allow(unused_imports)]
    use allocator as _;
    #[cfg(feature = "allocator")]
    #[deprecated(
        note = "The rquickjs crate feature `allocator` is deprecated, the functionality it provided is now enabled by default.
To remove this warning remove the use of the feature when specifying the dependency."
    )]
    mod allocator {}
}

#[cfg(test)]
mod map_keys {
    use crate::test_with;

    // `0033-map-hash-mix.patch` changes only which bucket a key lands in.
    // Every key family must still be found, equal keys must still meet, and
    // deletes must still unlink the right record.
    #[test]
    fn every_key_family_is_found_after_mixing() {
        test_with(|ctx| {
            let ok: bool = ctx
                .eval(
                    r#"
                    const check = (what, cond) => { if (!cond) throw new Error(what); };
                    const n = 5000;
                    const ints = new Map();
                    for (let i = 0; i < n; i++) ints.set(i, i);
                    for (let i = 0; i < n; i++) check("int " + i, ints.get(i) === i);
                    check("-0", ints.get(-0) === 0);
                    check("1.0", ints.get(1.0) === 1);
                    ints.set(NaN, "nan");
                    check("NaN", ints.get(0 / 0) === "nan");
                    check("size", ints.size === n + 1);
                    for (let i = 0; i < n; i += 2) ints.delete(i);
                    for (let i = 0; i < n; i++) check("deleted " + i, ints.has(i) === (i % 2 === 1));

                    const doubles = new Set();
                    for (let i = 0; i < n; i++) doubles.add(i + 0.5);
                    for (let i = 0; i < n; i++) check("double " + i, doubles.has(i + 0.5));
                    check("big", new Map([[2 ** 53 + 2, 1]]).get(2 ** 53 + 2) === 1);
                    check("bigint", new Map([[10n ** 30n, 1]]).get(10n ** 30n) === 1);

                    const objects = [];
                    const byObject = new Map();
                    const weak = new WeakMap();
                    for (let i = 0; i < n; i++) {
                        const o = { i };
                        objects.push(o);
                        byObject.set(o, i);
                        weak.set(o, i);
                    }
                    for (let i = 0; i < n; i++) {
                        check("object " + i, byObject.get(objects[i]) === i);
                        check("weak " + i, weak.get(objects[i]) === i);
                    }
                    check("fresh object", !byObject.has({ i: 0 }));
                    for (let i = 0; i < n; i += 3) byObject.delete(objects[i]);
                    for (let i = 0; i < n; i++) check("object deleted " + i, byObject.has(objects[i]) === (i % 3 !== 0));

                    const symbols = [];
                    const bySymbol = new Map();
                    for (let i = 0; i < n; i++) {
                        const s = Symbol(i);
                        symbols.push(s);
                        bySymbol.set(s, i);
                    }
                    for (let i = 0; i < n; i++) check("symbol " + i, bySymbol.get(symbols[i]) === i);
                    check("registered symbol", new Map([[Symbol.for("k"), 1]]).get(Symbol.for("k")) === 1);

                    const order = [...new Map([[3, 0], [1, 0], [2, 0]]).keys()].join();
                    check("insertion order", order === "3,1,2");
                    true
                    "#,
                )
                .unwrap();
            assert!(ok);
        });
    }
}
