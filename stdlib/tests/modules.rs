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
