#![doc = include_str!("../README.md")]

// BEGIN IMPORTED MODULES
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_abort/lib.rs"]
mod llrt_abort;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_async_hooks/lib.rs"]
mod llrt_async_hooks;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_buffer/lib.rs"]
mod llrt_buffer;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_compression/lib.rs"]
mod llrt_compression;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_context/lib.rs"]
mod llrt_context;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_crypto/lib.rs"]
mod llrt_crypto;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_encoding/lib.rs"]
mod llrt_encoding;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_events/lib.rs"]
mod llrt_events;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_exceptions/lib.rs"]
mod llrt_exceptions;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_hooking/lib.rs"]
mod llrt_hooking;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_json/lib.rs"]
mod llrt_json;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_path/lib.rs"]
mod llrt_path;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_stream_web/lib.rs"]
mod llrt_stream_web;
#[cfg(test)]
#[allow(dead_code)]
#[path = "llrt/llrt_test/lib.rs"]
mod llrt_test;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_timers/lib.rs"]
mod llrt_timers;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_url/lib.rs"]
mod llrt_url;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_utils/lib.rs"]
mod llrt_utils;
#[allow(dead_code, unused_imports, private_interfaces, clippy::non_minimal_cfg)]
#[path = "llrt/llrt_zlib/lib.rs"]
mod llrt_zlib;
// END IMPORTED MODULES

pub mod buffer {
    pub use crate::llrt_buffer::*;
}
pub mod crypto {
    pub use crate::llrt_crypto::*;
}
pub mod path {
    pub use crate::llrt_path::*;
}
pub mod url {
    pub use crate::llrt_url::*;
}
pub mod zlib {
    pub use crate::llrt_zlib::*;
}

use rquickjs::{
    loader::{BuiltinResolver, ModuleLoader},
    Ctx, Result,
};

/// Module names provided by this redistribution.
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
