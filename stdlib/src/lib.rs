#![doc = include_str!("../README.md")]

// BEGIN IMPORTED MODULES
#[cfg(feature = "_abort")]
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
#[cfg(feature = "_assert")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_assert/lib.rs"]
pub mod assert;
#[cfg(feature = "_async_hooks")]
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
#[cfg(feature = "_buffer")]
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
#[cfg(feature = "_child_process")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_child_process/lib.rs"]
pub mod child_process;
#[cfg(feature = "_compression")]
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
#[cfg(feature = "_console")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_console/lib.rs"]
pub mod console;
#[cfg(feature = "_context")]
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
#[cfg(feature = "_crypto")]
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
#[cfg(feature = "_dgram")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_dgram/lib.rs"]
pub mod dgram;
#[cfg(feature = "_dns")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_dns/lib.rs"]
pub mod dns;
#[cfg(feature = "_dns_cache")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_dns_cache/lib.rs"]
pub mod dns_cache;
#[cfg(feature = "_encoding")]
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
#[cfg(feature = "_events")]
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
#[cfg(feature = "_exceptions")]
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
#[cfg(feature = "_fetch")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_fetch/lib.rs"]
pub mod fetch;
#[cfg(feature = "_fs")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_fs/lib.rs"]
pub mod fs;
#[cfg(feature = "_hooking")]
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
#[cfg(feature = "_http")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_http/lib.rs"]
pub mod http;
#[cfg(feature = "_intl")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_intl/lib.rs"]
pub mod intl;
#[cfg(feature = "_json")]
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
#[cfg(feature = "_logging")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_logging/lib.rs"]
pub mod logging;
#[cfg(feature = "_navigator")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_navigator/lib.rs"]
pub mod navigator;
#[cfg(feature = "_net")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_net/lib.rs"]
pub mod net;
#[cfg(feature = "_numbers")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_numbers/lib.rs"]
pub mod numbers;
#[cfg(feature = "_os")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_os/lib.rs"]
pub mod os;
#[cfg(feature = "_path")]
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
#[cfg(feature = "_perf_hooks")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_perf_hooks/lib.rs"]
pub mod perf_hooks;
#[cfg(feature = "_process")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_process/lib.rs"]
pub mod process;
#[cfg(feature = "_stream")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_stream/lib.rs"]
pub mod stream;
#[cfg(feature = "_stream_web")]
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
#[cfg(feature = "_string_decoder")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_string_decoder/lib.rs"]
pub mod string_decoder;
#[cfg(feature = "_temporal")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_temporal/lib.rs"]
pub mod temporal;
#[cfg(test)]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "llrt/llrt_test/lib.rs"]
mod test_support;
#[cfg(test)]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "llrt/llrt_test_tls/lib.rs"]
mod test_tls_support;
#[cfg(feature = "_timers")]
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
#[cfg(feature = "_tls")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_tls/lib.rs"]
pub mod tls;
#[cfg(feature = "_tty")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_tty/lib.rs"]
pub mod tty;
#[cfg(feature = "_url")]
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
#[cfg(feature = "_util")]
#[allow(
    dead_code,
    unused_imports,
    unused_unsafe,
    unused_variables,
    private_interfaces,
    clippy::all
)]
#[path = "llrt/llrt_util/lib.rs"]
pub mod util;
#[cfg(feature = "_utils")]
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
#[cfg(feature = "_zlib")]
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
    feature = "_buffer",
    feature = "_crypto",
    feature = "_path",
    feature = "_url",
    feature = "_zlib"
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
    feature = "_buffer",
    feature = "_crypto",
    feature = "_path",
    feature = "_url",
    feature = "_zlib"
))]
pub use facade::{init, loader, resolver, MODULE_NAMES};
