#![doc = include_str!("../README.md")]

pub use llrt_buffer as buffer;
pub use llrt_crypto as crypto;
pub use llrt_path as path;
pub use llrt_url as url;
pub use llrt_zlib as zlib;

use rquickjs::{
    loader::{BuiltinResolver, ModuleLoader},
    Ctx, Result,
};

/// Module names provided by this facade.
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
