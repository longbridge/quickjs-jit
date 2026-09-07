# quickjs-jit-stdlib

A thin facade over the external LLRT `buffer`, `crypto`, `path`, `url` and `zlib`
crates. This package contains dependency declarations, re-exports and context
initialization only; LLRT sources remain in the upstream repository and are
fetched by Cargo. Dependencies are pinned to LLRT revision
`7b95c82a9b15e7ddfb2778eca4b5a63111e74f51`, using RustCrypto and Rust
brotli/flate2 with zstd's native backend.

```rust
use quickjs_jit_stdlib as stdlib;
use rquickjs::{Context, Module, Runtime};

let runtime = Runtime::new()?;
runtime.set_loader(stdlib::resolver(), stdlib::loader());
let context = Context::full(&runtime)?;
context.with(|ctx| {
    stdlib::init(&ctx)?;
    Module::evaluate(ctx, "app", "import { Buffer } from 'buffer';")?
        .finish::<()>()
})?;
# Ok::<(), rquickjs::Error>(())
```

Hosts can compose their own loader with `buffer::BufferModule`,
`crypto::CryptoModule`, `path::PathModule`, `url::UrlModule` and
`zlib::ZlibModule`. Call `init` once per fresh context to install globals.
The host owns Tokio execution and QuickJS job polling for asynchronous APIs.
This package does not install filesystem, process, network or fetch modules.
Rust 1.89 or newer is required. The `parallel` feature forwards to `rquickjs`.

## JIT integration and GPUI Shell migration

The facade uses the same `rquickjs` package identity as LLRT. Without a host
patch it uses upstream rquickjs, not quickjs-jit. For a JIT application, retain
an application-owned compatibility crate named `rquickjs` that re-exports
`quickjs-jit` and forwards the required Cargo features. Its quickjs-jit dependency
must use exactly the same source and revision as the host's binding dependency.

GPUI Shell should replace its five direct `llrt_*` dependencies with this package,
use the corresponding `quickjs_jit_stdlib::{buffer,crypto,path,url,zlib}` paths,
and retain its existing local `rquickjs-compat` crate and root patch:

```toml
# Application workspace root; patches in library manifests do not propagate.
[patch.crates-io]
rquickjs = { path = "crates/shell/rquickjs-compat" }
```

The three global initializers can be replaced by `quickjs_jit_stdlib::init(ctx)`.
Permission checks and host-specific loaders stay in the application. This package
does not ship a compatibility crate or apply a workspace patch.

## Distribution and verification

This package is currently Git-only (`publish = false`). A single public facade
does not remove LLRT's transitive dependencies: crates.io publication still needs
compatible registry releases and a solution for the binding compatibility layer.

Run `cargo test -p quickjs-jit-stdlib --all-features` to test the facade with
upstream rquickjs. With Python 3.11+, run `python3 scripts/check-stdlib-jit.py` to test an isolated
external host against the local quickjs-jit bindings, using a temporary host-owned
compatibility crate. Neither command copies LLRT sources into this repository.
