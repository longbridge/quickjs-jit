use quickjs_jit_stdlib as stdlib;
use rquickjs::{CatchResultExt, Context, Module, Runtime};

fn evaluate(source: &str) {
    let runtime = Runtime::new().unwrap();
    runtime.set_loader(stdlib::resolver(), stdlib::loader());
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        stdlib::init(&ctx).unwrap();
        Module::evaluate(ctx.clone(), "consumer", source)
            .unwrap()
            .finish::<()>()
            .catch(&ctx)
            .unwrap();
    });
}

#[test]
fn modules_share_buffer_url_and_quickjs_types() {
    evaluate(
        r#"
        import { Buffer as ImportedBuffer } from 'buffer';
        import path from 'path';
        import { URL as ImportedURL } from 'url';
        if (ImportedBuffer !== Buffer || ImportedURL !== URL) throw Error('different globals');
        if (Buffer.from('hello').toString('base64') !== 'aGVsbG8=') throw Error('buffer');
        if (path.normalize('./file') !== 'file') throw Error('path');
        if (new URL('/child?q=1', 'https://example.com/base').hostname !== 'example.com') throw Error('url');
    "#,
    );
}

#[test]
fn crypto_hash_and_compression_roundtrip() {
    evaluate(
        r#"
        import { createHash } from 'crypto';
        import { gzipSync, gunzipSync } from 'zlib';
        const digest = createHash('sha256').update('abc').digest('hex');
        if (digest !== 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad') throw Error(digest);
        const input = Buffer.from('external standard library');
        if (gunzipSync(gzipSync(input)).toString() !== input.toString()) throw Error('compression');
    "#,
    );
}

#[test]
fn module_definitions_can_be_composed_by_a_host() {
    let _: rquickjs::loader::ModuleLoader = rquickjs::loader::ModuleLoader::default()
        .with_module("buffer", stdlib::buffer::BufferModule)
        .with_module("crypto", stdlib::crypto::CryptoModule)
        .with_module("path", stdlib::path::PathModule)
        .with_module("url", stdlib::url::UrlModule)
        .with_module("zlib", stdlib::zlib::ZlibModule);
}

#[tokio::test]
async fn async_blob_crypto_and_compression_use_the_host_executor() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    runtime
        .set_loader(stdlib::resolver(), stdlib::loader())
        .await;
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    context.async_with(async |ctx| {
        stdlib::init(&ctx).unwrap();
        Module::evaluate(ctx.clone(), "async-consumer", r#"
            import { gzip, gunzip } from 'zlib';
            const text = 'async standard modules';
            if (await new Blob([text]).text() !== text) throw Error('Blob.text');
            const digest = await crypto.subtle.digest('SHA-256', Buffer.from('abc'));
            if (Buffer.from(digest).toString('hex') !== 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad') throw Error('subtle.digest');
            const compressed = await new Promise((resolve, reject) => gzip(Buffer.from(text), (error, value) => error ? reject(error) : resolve(value)));
            const plain = await new Promise((resolve, reject) => gunzip(compressed, (error, value) => error ? reject(error) : resolve(value)));
            if (plain.toString() !== text) throw Error('async gzip');
        "#).unwrap().into_future::<()>().await.catch(&ctx).unwrap();
    }).await;
}

/// Buffer bytes come from QuickJS's allocator, so the runtime's memory limit
/// bounds them: a Buffer larger than the limit throws instead of growing the
/// process.
#[test]
fn buffers_count_against_the_memory_limit() {
    let runtime = Runtime::new().unwrap();
    runtime.set_loader(stdlib::resolver(), stdlib::loader());
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| stdlib::init(&ctx).unwrap());
    runtime.set_memory_limit(64 << 20);
    let before = runtime.memory_usage().malloc_size;
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.kept = [Buffer.alloc(8 << 20, 1), Buffer.allocUnsafe(8 << 20), Buffer.from(new Uint8Array(8 << 20))];")
            .catch(&ctx)
            .unwrap();
    });
    let grown = runtime.memory_usage().malloc_size - before;
    assert!(
        grown >= 24 << 20,
        "Buffer storage is not counted: {grown} bytes"
    );
    context.with(|ctx| {
        for source in [
            "Buffer.alloc(256 << 20, 1)",
            "Buffer.alloc(256 << 20)",
            "Buffer.allocUnsafe(256 << 20)",
            "Buffer.allocUnsafeSlow(256 << 20)",
            "Buffer.alloc(256 << 20, 'ab')",
            "Buffer.concat([Buffer.alloc(30 << 20), Buffer.alloc(30 << 20)])",
        ] {
            let result = ctx.eval::<(), _>(source).catch(&ctx);
            assert!(result.is_err(), "{source} exceeded the memory limit");
            ctx.run_gc();
        }
        ctx.eval::<(), _>("if (kept[0][0] !== 1 || kept[0].length !== 8 << 20 || kept[1][5] !== 0) throw Error('contents');")
            .catch(&ctx)
            .unwrap();
    });
}

/// zlib's results are Buffers, so the memory limit bounds them too.
#[test]
fn zlib_output_counts_against_the_memory_limit() {
    let runtime = Runtime::new().unwrap();
    runtime.set_loader(stdlib::resolver(), stdlib::loader());
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| stdlib::init(&ctx).unwrap());
    runtime.set_memory_limit(64 << 20);
    let before = runtime.memory_usage().malloc_size;
    context.with(|ctx| {
        Module::evaluate(
            ctx.clone(),
            "inflate",
            "import { gzipSync, gunzipSync } from 'zlib';
            globalThis.kept = gunzipSync(gzipSync(Buffer.alloc(16 << 20)));",
        )
        .unwrap()
        .finish::<()>()
        .catch(&ctx)
        .unwrap();
    });
    let grown = runtime.memory_usage().malloc_size - before;
    assert!(
        grown >= 16 << 20,
        "zlib output is not counted: {grown} bytes"
    );
    context.with(|ctx| {
        let result = Module::evaluate(
            ctx.clone(),
            "inflate-too-much",
            "import { gzipSync, gunzipSync } from 'zlib';
            gunzipSync(gzipSync(Buffer.alloc(64 << 20)));",
        )
        .and_then(|module| module.finish::<()>())
        .catch(&ctx);
        assert!(result.is_err(), "zlib output exceeded the memory limit");
    });
}

/// PBKDF2 runs on the JS thread; its work is capped so a script cannot hold it.
#[tokio::test]
async fn pbkdf2_work_is_capped() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    runtime
        .set_loader(stdlib::resolver(), stdlib::loader())
        .await;
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    context
        .async_with(async |ctx| {
            stdlib::init(&ctx).unwrap();
            // WebCrypto's errors are DOMExceptions.
            stdlib::exceptions::init(&ctx).unwrap();
            Module::evaluate(
                ctx.clone(),
                "pbkdf2",
                r#"
                const key = await crypto.subtle.importKey('raw', Buffer.from('password'), 'PBKDF2', false, ['deriveBits']);
                const derive = (iterations, length) => crypto.subtle.deriveBits(
                    { name: 'PBKDF2', hash: 'SHA-256', salt: Buffer.from('salt'), iterations }, key, length);
                if ((await derive(1000, 256)).byteLength !== 32) throw Error('PBKDF2');
                for (const [iterations, length] of [[2 ** 32 - 1, 256], [10_000_001, 256], [5_000_001, 512]]) {
                    let failed = false;
                    try { await derive(iterations, length); } catch { failed = true; }
                    if (!failed) throw Error(`PBKDF2 ${iterations} x ${length} bits was not capped`);
                }
            "#,
            )
            .unwrap()
            .into_future::<()>()
            .await
            .catch(&ctx)
            .unwrap();
        })
        .await;
}
