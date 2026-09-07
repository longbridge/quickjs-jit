# quickjs-jit-stdlib

One distributable crate containing the LLRT-derived `buffer`, `crypto`, `path`,
`url` and `zlib` modules used by GPUI Shell, plus their internal implementations.
No LLRT crate, Git dependency, or `rquickjs` Cargo patch is needed by consumers.
This is a selected standard-library distribution, not the full LLRT runtime or
all Node.js APIs.

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
`zlib::ZlibModule`. Call `init` once on each fresh context to install globals
before evaluating scripts. Hosts still own Tokio execution and QuickJS job
polling for asynchronous operations. No filesystem, process, network or fetch
modules are installed by this package.

This crate requires Rust 1.89 or newer because of the selected cryptography
dependencies. Bindings are `quickjs-jit` 0.12.7. Development uses a versioned local dependency;
publishing requires that binding release and its dependencies on crates.io
first. The `parallel` feature forwards the binding's parallel runtime support.
The imported configuration uses RustCrypto, Rust brotli/flate2, and zstd's native
backend, matching the current Shell integration.

Upstream: LLRT revision `7b95c82a9b15e7ddfb2778eca4b5a63111e74f51`.
Sources retain their copyright headers. See `NOTICE`, `LICENSE-APACHE` and
`UPSTREAM.json`. To reproduce the import from a checkout at that revision, run
`python3 scripts/import-stdlib.py /path/to/llrt` from the repository (Python 3.11+).


## Migrating a GPUI Shell host

Replace the five `llrt_*` dependency entries with `quickjs-jit-stdlib`. Change
`llrt_buffer::BufferModule` to `quickjs_jit_stdlib::buffer::BufferModule`, and
likewise for `crypto`, `path`, `url` and `zlib`. Replace the three global
initializers with `quickjs_jit_stdlib::init(ctx)` or retain individual module
initializers in the existing order. Remove the local `rquickjs` compatibility
crate and its `[patch.crates-io]` entry. Host-specific loaders and permission
checks stay in the host; the standard modules can be composed with them.

## Validation and publication

Run `cargo test -p quickjs-jit-stdlib --all-features`. To verify all unpublished
local binding packages together, Cargo can stage them into a temporary registry:

```sh
cargo package --allow-dirty -p quickjs-jit-sys -p quickjs-jit-core \
  -p quickjs-jit-macro -p quickjs-jit -p quickjs-jit-stdlib
```

This builds the extracted packages without publishing anything. Once the
binding packages are available on crates.io, stdlib is the only additional
package to publish. Its archive contains the adapted LLRT modules, original
license and notice, and provenance; it does not contain nested LLRT manifests.
