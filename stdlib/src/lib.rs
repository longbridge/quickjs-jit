#![doc = include_str!("../README.md")]

// BEGIN IMPORTED MODULES
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_abort/lib.rs"]
pub mod abort;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_async_hooks/lib.rs"]
pub mod async_hooks;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_buffer/lib.rs"]
pub mod buffer;
#[cfg(feature = "zlib")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_compression/lib.rs"]
pub mod compression;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_context/lib.rs"]
pub mod context;
#[cfg(feature = "crypto")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_crypto/lib.rs"]
pub mod crypto;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_encoding/lib.rs"]
pub mod encoding;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_events/lib.rs"]
pub mod events;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_exceptions/lib.rs"]
pub mod exceptions;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_hooking/lib.rs"]
pub mod hooking;
#[cfg(feature = "crypto")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_json/lib.rs"]
pub mod json;
#[cfg(feature = "path")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_path/lib.rs"]
pub mod path;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_stream_web/lib.rs"]
pub mod stream_web;
#[cfg(test)]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "llrt/llrt_test/lib.rs"]
mod test_support;
#[cfg(feature = "buffer")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_timers/lib.rs"]
pub mod timers;
#[cfg(feature = "url")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_url/lib.rs"]
pub mod url;
#[cfg(any(feature = "buffer", feature = "path", feature = "url"))]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_utils/lib.rs"]
pub mod utils;
#[cfg(feature = "zlib")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_zlib/lib.rs"]
pub mod zlib;
// END IMPORTED MODULES

#[cfg(all(
    feature = "buffer",
    feature = "crypto",
    feature = "path",
    feature = "url",
    feature = "zlib"
))]
mod facade {
    use rquickjs::{
        loader::{BuiltinResolver, ModuleLoader},
        Ctx, Result,
    };

    use crate::{buffer, crypto, path, url, zlib};

    /// Module names `resolver` and `loader` provide.
    pub const MODULE_NAMES: &[&str] = &["buffer", "crypto", "path", "url", "zlib"];

    /// A resolver for the five standard modules. Compose it with host resolvers.
    pub fn resolver() -> BuiltinResolver {
        MODULE_NAMES
            .iter()
            .fold(BuiltinResolver::default(), |resolver, name| {
                resolver.with_module(*name)
            })
    }

    /// A loader for the five standard modules. Compose it with host loaders.
    pub fn loader() -> ModuleLoader {
        ModuleLoader::default()
            .with_module("buffer", buffer::BufferModule)
            .with_module("crypto", crypto::CryptoModule)
            .with_module("path", path::PathModule)
            .with_module("url", url::UrlModule)
            .with_module("zlib", zlib::ZlibModule)
    }

    /// Install the Buffer, URL and Crypto globals into a fresh context, in order.
    ///
    /// Call once per context before evaluating a module using this library.
    /// This does not install host I/O modules, start Tokio, or drive pending jobs.
    pub fn init(ctx: &Ctx<'_>) -> Result<()> {
        buffer::init(ctx)?;
        url::init(ctx)?;
        crypto::init(ctx)?;
        Ok(())
    }
}

#[cfg(all(
    feature = "buffer",
    feature = "crypto",
    feature = "path",
    feature = "url",
    feature = "zlib"
))]
pub use facade::{init, loader, resolver, MODULE_NAMES};
