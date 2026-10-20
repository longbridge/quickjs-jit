# quickjs-jit-stdlib

LLRT's `buffer`, `crypto`, `path`, `url` and `zlib` modules for quickjs-jit, vendored as
source into one crate (no Git dependencies). The sources are LLRT revision
`d9fd873e793c6551c5d92b2e31480f3279e3cd8d`, imported by `scripts/import-stdlib.py`
(provenance and file hashes in `UPSTREAM.json`). Only those modules and the LLRT
libraries they depend on are vendored; each LLRT crate `llrt_<name>` is the module
`quickjs_jit_stdlib::<name>`.

Features, one per module, with LLRT's pure-Rust backends (RustCrypto for `crypto`,
Rust brotli / flate2 for `zlib`):

| Feature | Module |
|---|---|
| `buffer` | `Buffer`, `Blob`, `File` (and the libraries it needs) |
| `crypto` | `crypto`, `crypto.subtle` |
| `path` | `path` |
| `url` | `url` |
| `zlib` | `zlib` without zstd |
| `zlib-zstd` | `zlib`'s `zstd*` functions (the zstd C library: there is no Rust implementation) |

The default features are `buffer`, `crypto`, `path`, `url` and `zlib`, which also give
`init`, `loader` and `resolver`:

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

The host owns Tokio execution and QuickJS job polling for asynchronous APIs. Rust
1.89 or newer is required. The `parallel` feature forwards to `rquickjs`.

Local patches (listed in `UPSTREAM.json`):

- Buffer storage, zlib output and every other byte array the modules hand to JS are
  allocated by QuickJS, so the runtime's memory limit bounds them (LLRT keeps Rust
  allocations, outside the limit).
- PBKDF2 (`crypto.subtle.deriveBits` / `deriveKey`) runs synchronously on the JS thread,
  where the interrupt budget cannot stop it: its work, iterations times output hash
  blocks, is capped at 10,000,000. HKDF is bounded by its 255-block maximum.
- A host without LLRT's `DOMException` global gets the creation error thrown instead of
  a panic.
- zstd is optional; signals have a fallback off Unix and Windows (WebAssembly).

The LLRT code is licensed under the Apache License 2.0 (`LICENSE-APACHE`, `NOTICE`).
