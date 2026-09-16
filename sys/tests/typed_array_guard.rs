#![cfg(all(feature = "jit-abi", target_os = "linux"))]

#[path = "../build_support/patch.rs"]
mod patch;

use std::{fs, path::PathBuf, process::Command};

#[test]
fn typed_array_leaf_guard_revalidates_live_semantics_without_effects() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("mktemp")
        .args(["-d", "/tmp/quickjs-array-guard-XXXXXX"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let scratch = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    for file in patch::BASELINE_FILES {
        fs::copy(manifest.join("quickjs").join(file), scratch.join(file)).unwrap();
    }
    patch::apply_patch_set(&scratch, &manifest.join("patches")).unwrap();
    for file in [
        "quickjs-jit-opcodes.generated.h",
        "quickjs-jit-helpers.generated.h",
    ] {
        fs::copy(
            PathBuf::from(env!("OUT_DIR")).join(file),
            scratch.join(file),
        )
        .unwrap();
    }
    fs::write(scratch.join("array-guard-test.c"), NATIVE_FIXTURE).unwrap();
    let compile = Command::new("cc")
        .current_dir(&scratch)
        .args([
            "-std=gnu11",
            "-O1",
            "-g",
            "-D_GNU_SOURCE",
            "-DCONFIG_JIT_ABI=1",
            "array-guard-test.c",
            "libregexp.c",
            "libunicode.c",
            "dtoa.c",
            "-lm",
            "-lpthread",
            "-o",
            "array-guard-test",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(scratch.join("array-guard-test"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "typed-array guard failed in {}:\n{}\n{}",
        scratch.display(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    fs::remove_dir_all(scratch).unwrap();
}

const NATIVE_FIXTURE: &str = r#"
#include "quickjs.h"
#include "quickjs-jit.h"
#include <assert.h>
#include <string.h>
/* The fixture includes the implementation to perturb otherwise-private
 * backing state after observing a candidate. The exported guard remains the
 * only operation used to accept a continuing native path. */
#include "quickjs.c"

static int query(const JSJitArrayAPI *api, JSContext *ctx, JSValueConst value,
                 uint32_t mode, JSJitArrayMetadata *out)
{
    memset(out, 0xa5, sizeof(*out));
    out->struct_size = sizeof(*out);
    return api->query(ctx, &value, mode, JS_JIT_ARRAY_QUERY_LENGTH, out);
}

static JSValue eval(JSContext *ctx, const char *source)
{
    JSValue value = JS_Eval(ctx, source, strlen(source), "array-guard-test.js",
                            JS_EVAL_TYPE_GLOBAL);
    assert(!JS_IsException(value));
    return value;
}

int main(void)
{
    JSRuntime *rt = JS_NewRuntime();
    JSContext *ctx = JS_NewContext(rt);
    const JSJitArrayAPI *api = JS_JitGetArrayAPI(QJSJIT_ARRAY_API_VERSION);
    JSJitArrayMetadata out;
    JSValue value;
    JSObject *object;
    JSTypedArray *typed;
    JSArrayBuffer *buffer;
    void *expected_data;
    uint32_t expected_count;

    assert(QJSJIT_ABI_MAJOR == 1 && QJSJIT_ABI_MINOR == 24);
    assert(QJSJIT_ARRAY_API_VERSION == 1);
    assert(ctx && api);
    assert(api->struct_size == sizeof(*api));
    assert(api->version == QJSJIT_ARRAY_API_VERSION);
    assert(api->effects == 0);
    assert(!JS_JitGetArrayAPI(QJSJIT_ARRAY_API_VERSION + 1));

    value = eval(ctx, "new Int32Array([3, 5, 8])");
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_OK);
    assert(out.mode == JS_JIT_ARRAY_MODE_INT32 && out.count == 3 && out.data);
    expected_data = out.data;
    expected_count = out.count;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_FLOAT64, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    assert(!out.data && !out.count && !out.mode);

    JS_FreeValue(ctx, value);
    value = eval(ctx,
        "(() => { let a = new Int32Array(3);"
        "Object.defineProperty(a, 'length', { value: 99 }); return a; })()");
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    JS_FreeValue(ctx, value);

    value = eval(ctx,
        "(() => { let a = new Int32Array(3);"
        "Object.setPrototypeOf(a, { get length() { return 3; } }); return a; })()");
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    JS_FreeValue(ctx, value);

    value = eval(ctx,
        "Object.defineProperty(Object.getPrototypeOf(Int32Array.prototype),"
        " 'length', { configurable: true, get() { return 7; } });"
        "new Int32Array(3)");
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    JS_FreeValue(ctx, value);

    /* Use a fresh realm for backing-state checks because the prior scenario
     * deliberately replaced that realm's intrinsic length getter. */
    JS_FreeContext(ctx);
    ctx = JS_NewContext(rt);
    value = eval(ctx, "new Int32Array([1, 2, 3])");
    object = JS_VALUE_GET_OBJ(value);
    typed = object->u.typed_array;
    buffer = typed->buffer->u.array_buffer;
    expected_data = object->u.array.u.ptr;
    expected_count = object->u.array.count;

    buffer->shared = 1;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    buffer->shared = 0;
    buffer->max_byte_length = buffer->byte_length;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    buffer->max_byte_length = -1;
    typed->track_rab = 1;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    typed->track_rab = 0;
    buffer->detached = 1;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    buffer->detached = 0;
    buffer->immutable = 1;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    buffer->immutable = 0;
    object->u.array.u.ptr = (uint8_t *)expected_data + sizeof(int32_t);
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    object->u.array.u.ptr = expected_data;
    object->u.array.count = expected_count + 1;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_MISS);
    object->u.array.count = expected_count;
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_INT32, &out) ==
           JS_JIT_ARRAY_QUERY_OK);

    JS_FreeValue(ctx, value);
    value = eval(ctx, "new Float64Array([1.5, 2.5])");
    assert(query(api, ctx, value, JS_JIT_ARRAY_MODE_FLOAT64, &out) ==
           JS_JIT_ARRAY_QUERY_OK);
    assert(out.mode == JS_JIT_ARRAY_MODE_FLOAT64 && out.count == 2 && out.data);
    JS_FreeValue(ctx, value);
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
    return 0;
}
"#;
