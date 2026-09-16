#![cfg(all(feature = "jit-abi", target_os = "linux"))]

#[path = "../build_support/patch.rs"]
mod patch;

use std::{fs, path::PathBuf, process::Command};

// A native entry has already performed the callee's visible store when its
// speculative add fails. Replaying the pending CALL would increment twice.
#[test]
fn native_inline_overflow_resumes_after_completed_effect() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("mktemp")
        .args(["-d", "/tmp/quickjs-inline-test-XXXXXX"])
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
    fs::write(scratch.join("inline-test.c"), NATIVE_FIXTURE).unwrap();
    let sanitizer = std::env::var("QUICKJS_INLINE_TEST_SANITIZER").ok();
    let mut compiler = Command::new(if sanitizer.is_some() { "clang" } else { "cc" });
    compiler.current_dir(&scratch);
    match sanitizer.as_deref() {
        None => {}
        Some("address") => {
            compiler.args([
                "-fsanitize=address,undefined",
                "-fno-sanitize-recover=all",
                "-fno-omit-frame-pointer",
            ]);
        }
        Some("memory") => {
            compiler.args([
                "-fsanitize=memory",
                "-fsanitize-memory-track-origins",
                "-fPIE",
                "-pie",
                "-fno-omit-frame-pointer",
            ]);
        }
        Some(other) => panic!("unsupported native fixture sanitizer {other}"),
    }
    let compile = compiler
        .args([
            "-std=gnu11",
            "-O1",
            "-g",
            "-D_GNU_SOURCE",
            "-DCONFIG_JIT_ABI=1",
            "inline-test.c",
            "libregexp.c",
            "libunicode.c",
            "dtoa.c",
            "-lm",
            "-lpthread",
            "-o",
            "inline-test",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(scratch.join("inline-test")).output().unwrap();
    assert!(
        run.status.success(),
        "native recovery failed in {}:\n{}\n{}",
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
#include <stdio.h>
#include <string.h>
/* Include the unchanged patched implementation in this test translation unit
 * to control the real interrupt countdown and inspect escaped var references;
 * production recovery itself is exercised through its exported API. */
#include "quickjs.c"

static JSJitFunctionSnapshot *outer_snapshot, *callee_snapshot;
static unsigned call_pc, call_next, add_pc, entries, releases, scenario, polls;
static JSJitFunctionSnapshot *middle_snapshot;
static int cancel_entry(JSRuntime *rt, void *opaque) { return 1; }
static int count_entry(JSRuntime *rt, void *opaque)
{
    JSContext *ctx = opaque;
    polls++;
    ctx->interrupt_counter = 1;
    return 0;
}
static int invalidate_entry(JSRuntime *rt, void *opaque)
{
    JSJitExecFrame *root = opaque;
    polls++;
    assert(JS_JitInvalidateFunction(root->ctx, root->arg_buf[0]) == JS_JIT_BACKEND_OK);
    if (scenario == 15)
        assert(JS_JitInvalidateFunction(root->ctx, rt->current_stack_frame->cur_func) == JS_JIT_BACKEND_OK);
    return 0;
}

static unsigned local_named(JSContext *ctx, JSFunctionBytecode *b, const char *name)
{
    unsigned i;
    for (i = 0; i < b->var_count; i++) {
        const char *found = JS_AtomToCString(ctx, b->vardefs[b->arg_count + i].var_name);
        int matches = found && !strcmp(found, name);
        JS_FreeCString(ctx, found);
        if (matches) return i;
    }
    assert(!"missing expected native fixture local");
    return 0;
}

static unsigned find_opcode(JSJitFunctionSnapshot *s, const char *name, int last)
{
    uint32_t count, pc = 0, found = UINT32_MAX;
    const JSJitOpcodeInfo *ops = JS_JitGetOpcodeTable(&count, NULL);
    while (pc < s->bytecode_len) {
        const JSJitOpcodeInfo *op = &ops[s->bytecode[pc]];
        assert(s->bytecode[pc] < count && op->size);
        if (!strcmp(op->name, name)) {
            found = pc;
            if (!last) return found;
        }
        pc += op->size;
    }
    if (found == UINT32_MAX) fprintf(stderr, "scenario%u missing opcode %s\n", scenario, name);
    assert(found != UINT32_MAX);
    return found;
}

static JSJitExit native(JSJitExecFrame *root)
{
    JSJitExecFrame *leaf = NULL;
    JSJitExecFrame *middle = NULL;
    JSJitExit exit = { JS_JIT_EXIT_DEOPT, 0, NULL, NULL };
    JSValue global;
    entries++;
    assert(entries == 1);
    root->stack_base[0] = JS_DupValue(root->ctx, root->arg_buf[0]);
    root->stack_base[1] = JS_DupValue(root->ctx, root->arg_buf[1]);
    root->stack_top = root->stack_base + 2;
    root->pc = root->bytecode_start + call_pc;
#ifdef QJSJIT_INLINE_RECOVERY_VERSION
    if (scenario == 14 || scenario == 15) {
        JS_SetInterruptHandler(root->rt, invalidate_entry, root);
        root->ctx->interrupt_counter = 1;
    }
    if (scenario == 13) {
        JS_SetInterruptHandler(root->rt, count_entry, root->ctx);
        root->ctx->interrupt_counter = 1;
        JS_SetMemoryLimit(root->rt, 1);
        assert(JS_JitInlineEnter(root, 0, call_pc, call_next,
                                JS_JIT_INLINE_CALL, 1, &leaf) == JS_JIT_HELPER_GUARD_MISS);
        JS_SetMemoryLimit(root->rt, 0);
        assert(!leaf && polls == 0);
        exit.resume_pc = root->pc;
        exit.resume_stack_top = root->stack_top;
        return exit;
    }
    if (scenario == 10) {
        JSJitExecFrame saved = *root;
        JS_SetInterruptHandler(root->rt, cancel_entry, NULL);
        root->ctx->interrupt_counter = 1;
        assert(JS_JitInlineEnter(root, 0, call_pc, call_next,
                                JS_JIT_INLINE_CALL, 1, &leaf) == JS_JIT_HELPER_EXCEPTION);
        assert(!leaf && !memcmp(&saved, root, sizeof(saved)));
        JS_SetInterruptHandler(root->rt, NULL, NULL);
        exit.kind = JS_JIT_EXIT_EXCEPTION;
        exit.resume_pc = root->bytecode_start + call_next;
        exit.resume_stack_top = root->stack_top;
        return exit;
    }
    if (scenario == 7) {
        root->stack_base[2] = JS_NewInt32(root->ctx, 7);
        root->stack_base[3] = JS_NewInt32(root->ctx, 9);
        root->stack_top += 2;
    }
    if (scenario == 9) {
        root->stack_base[2] = root->stack_base[1];
        root->stack_base[1] = JS_GetPropertyStr(root->ctx, root->arg_buf[0], "run");
        root->stack_top++;
    }
    if (scenario == 2 || scenario == 16 || scenario == 17) {
        JSValue globals = JS_GetGlobalObject(root->ctx);
        JS_FreeValue(root->ctx, root->stack_base[1]);
        root->stack_base[1] = JS_GetPropertyStr(root->ctx, globals, "inner");
        root->stack_base[2] = JS_DupValue(root->ctx, root->arg_buf[1]);
        root->stack_top++;
        JS_FreeValue(root->ctx, globals);
    }
    if (scenario == 4) {
        JSJitExecFrame saved = *root;
        JS_SetMemoryLimit(root->rt, 1);
        assert(JS_JitInlineEnter(root, 0, call_pc, call_next,
                                JS_JIT_INLINE_CALL, 1, &leaf) == JS_JIT_HELPER_GUARD_MISS);
        JS_SetMemoryLimit(root->rt, 0);
        assert(!leaf && !memcmp(&saved, root, sizeof(saved)) && !JS_HasException(root->ctx));
    }
    assert(JS_JitInlineEnter(root, 0, call_pc, call_next,
                            scenario == 9 ? JS_JIT_INLINE_CALL_METHOD : JS_JIT_INLINE_CALL,
                            scenario == 2 || scenario == 16 || scenario == 17 ? 2 : scenario == 7 ? 3 : 1, &leaf) == JS_JIT_HELPER_OK);
    assert(leaf);
    if (scenario == 2 || scenario == 16 || scenario == 17) {
        unsigned middle_call = find_opcode(middle_snapshot, "call1", 0);
        JSJitExecFrame *inner;
        assert(leaf->function_id == middle_snapshot->function.id);
        middle = leaf;
        leaf->stack_base[0] = JS_DupValue(leaf->ctx, leaf->arg_buf[0]);
        leaf->stack_base[1] = JS_DupValue(leaf->ctx, leaf->arg_buf[1]);
        leaf->stack_top = leaf->stack_base + 2;
        leaf->pc = leaf->bytecode_start + middle_call;
        assert(JS_JitInlineEnter(leaf, 0, middle_call, middle_call + 1,
                                JS_JIT_INLINE_CALL, 1, &inner) == JS_JIT_HELPER_OK);
        leaf = inner;
    }
    assert(leaf->function_id == callee_snapshot->function.id);
    if (scenario == 14 || scenario == 15) {
        JS_SetInterruptHandler(root->rt, NULL, NULL);
        assert(polls == 1 && leaf->generation != callee_snapshot->function.generation);
        assert(JS_JitInlineCheck(leaf) == (scenario == 15 ? JS_JIT_HELPER_GUARD_MISS : JS_JIT_HELPER_OK));
    }
    if (scenario == 11 || scenario == 12) {
        JSStackFrame *sf = leaf->rt->current_stack_frame;
        JSObject *function = JS_VALUE_GET_OBJ(sf->cur_func);
        JSFunctionBytecode *b = function->u.func.function_bytecode;
        JSValue saved, globals = JS_GetGlobalObject(leaf->ctx);
        if (scenario == 11) {
            unsigned slot = local_named(leaf->ctx, b, "arguments");
            saved = js_build_mapped_arguments(leaf->ctx, 1, leaf->arg_buf, sf, 1);
            assert(!JS_IsException(saved));
            leaf->var_buf[slot] = JS_DupValue(leaf->ctx, saved);
            assert(JS_VALUE_GET_OBJ(saved)->u.array.u.var_refs[0]->pvalue == sf->arg_buf);
        } else {
            unsigned slot = local_named(leaf->ctx, b, "y");
            unsigned closure_pc = find_opcode(callee_snapshot, "fclosure8", 0);
            unsigned index = callee_snapshot->bytecode[closure_pc + 1];
            leaf->var_buf[slot] = JS_DupValue(leaf->ctx, leaf->arg_buf[0]);
            saved = js_closure(leaf->ctx, js_dup(b->cpool[index]), function->u.func.var_refs, sf);
            assert(!JS_IsException(saved));
            assert(JS_VALUE_GET_OBJ(saved)->u.func.var_refs[0]->pvalue == leaf->var_buf + slot);
        }
        assert(JS_SetPropertyStr(leaf->ctx, globals, "saved", saved) == 1);
        JS_FreeValue(leaf->ctx, globals);
        JS_RunGC(leaf->rt);
    }
#endif
    /* The completed native store belongs to the inlined callee. */
    global = JS_GetGlobalObject(root->ctx);
    if (scenario != 7 && scenario != 9 && scenario != 14 && scenario != 15)
        assert(JS_SetPropertyStr(root->ctx, global, "hits", JS_NewInt32(root->ctx, 1)) == 1);
    JS_FreeValue(root->ctx, global);
#ifdef QJSJIT_INLINE_RECOVERY_VERSION
    if (scenario == 7 || scenario == 9 || scenario == 14 || scenario == 15) {
        assert(JS_JitInlineResume(leaf, 0, JS_JIT_INLINE_RESUME_INSTRUCTION) == JS_JIT_HELPER_OK);
    } else if (scenario == 16) {
        double value;
        size_t nested_bytes = root->rt->jit_inline_bytes;
        assert(root->rt->jit_inline_depth == 2 && root->rt->jit_inline_top);
        assert(&root->rt->jit_inline_top->view == leaf);
        leaf->stack_base[0] = JS_NewFloat64(leaf->ctx, 2147483648.0);
        leaf->stack_top = leaf->stack_base + 1;
        assert(JS_JitInlineLeave(leaf, callee_snapshot->arg_count + callee_snapshot->local_count) == JS_JIT_HELPER_OK);
        assert(root->rt->jit_inline_depth == 1 && root->rt->jit_inline_bytes < nested_bytes);
        assert(root->rt->jit_inline_top && &root->rt->jit_inline_top->view == middle);
        assert(root->rt->jit_active_frame == middle && middle->stack_top == middle->stack_base + 1);
        assert(middle->pc == middle->bytecode_start + find_opcode(middle_snapshot, "call1", 0) + 1);
        assert(JS_ToFloat64(middle->ctx, &value, middle->stack_base[0]) == 0 && value == 2147483648.0);
        JS_FreeValue(middle->ctx, middle->stack_base[0]);
        middle->stack_base[0] = JS_NewFloat64(middle->ctx, 2147483668.0);
        assert(JS_JitInlineLeave(middle, middle_snapshot->arg_count + middle_snapshot->local_count) == JS_JIT_HELPER_OK);
        assert(!root->rt->jit_inline_top && root->rt->jit_inline_depth == 0 && root->rt->jit_inline_bytes == 0);
        assert(root->rt->jit_active_frame == root);
    } else if (scenario == 17) {
        unsigned throw_pc = find_opcode(callee_snapshot, "throw", 0);
        leaf->stack_top = leaf->stack_base;
        leaf->pc = leaf->bytecode_start + throw_pc + 1;
        JS_Throw(leaf->ctx, JS_NewInt32(leaf->ctx, 7));
        assert(JS_JitInlineResume(leaf, throw_pc + 1, JS_JIT_INLINE_RESUME_EXCEPTION) == JS_JIT_HELPER_EXCEPTION);
        assert(!root->rt->jit_inline_top && root->rt->jit_inline_depth == 0 && root->rt->jit_inline_bytes == 0);
        assert(root->rt->jit_active_frame == root);
        /* Exceptional completion preserves the physical root's original
         * call2 operands; the VM unwinds them instead of replaying CALL. */
        assert(root->stack_top == root->stack_base + 3 && root->pc == root->bytecode_start + call_next);
        exit.kind = JS_JIT_EXIT_EXCEPTION;
        exit.resume_pc = root->pc;
        exit.resume_stack_top = root->stack_top;
        return exit;
    } else if (scenario == 3) {
        unsigned catch_pc = find_opcode(callee_snapshot, "catch", 0);
        unsigned throw_pc = find_opcode(callee_snapshot, "throw", 0);
        const uint8_t *p = callee_snapshot->bytecode + catch_pc + 1;
        uint32_t offset = p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
        leaf->stack_base[0] = JS_MKVAL(JS_TAG_CATCH_OFFSET, catch_pc + 1 + (int32_t)offset);
        leaf->stack_top = leaf->stack_base + 1;
        leaf->pc = leaf->bytecode_start + throw_pc + 1;
        JS_Throw(leaf->ctx, JS_NewInt32(leaf->ctx, 7));
        assert(JS_JitInlineResume(leaf, throw_pc + 1, JS_JIT_INLINE_RESUME_EXCEPTION) == JS_JIT_HELPER_OK);
    } else if (scenario == 1) {
        leaf->stack_base[0] = JS_NewFloat64(leaf->ctx, 2147483648.0);
        leaf->stack_top = leaf->stack_base + 1;
        assert(JS_JitInlineLeave(leaf, callee_snapshot->arg_count + callee_snapshot->local_count) == JS_JIT_HELPER_OK);
        assert(JS_JitInlineCheck(root) == JS_JIT_HELPER_OK);
    } else if (scenario == 5) {
        assert(JS_JitInlineResume(leaf, UINT32_MAX, JS_JIT_INLINE_RESUME_INSTRUCTION) == JS_JIT_HELPER_EXCEPTION);
        exit.kind = JS_JIT_EXIT_EXCEPTION;
        exit.resume_pc = root->pc;
        exit.resume_stack_top = root->stack_top;
        return exit;
    } else {
    if (scenario == 6) {
        assert(JS_JitInvalidateFunction(leaf->ctx, root->arg_buf[0]) == JS_JIT_BACKEND_OK);
        leaf->stack_base[0] = JS_UNDEFINED;
        leaf->stack_base[1] = JS_NewString(leaf->ctx, "completed opcode temporary");
        leaf->stack_top = leaf->stack_base + 2;
        leaf->pc = leaf->bytecode_start + add_pc;
        assert(JS_JitHelperDup(leaf, 0, callee_snapshot->arg_count + callee_snapshot->local_count, 0) == JS_JIT_HELPER_OK);
        assert(JS_JitHelperFree(leaf, 0, callee_snapshot->arg_count + callee_snapshot->local_count + 1) == JS_JIT_HELPER_OK);
        assert(JS_IsUndefined(leaf->stack_base[1]));
        assert(JS_JitInlineCheck(leaf) == JS_JIT_HELPER_GUARD_MISS);
    } else {
        leaf->stack_base[0] = JS_DupValue(leaf->ctx, leaf->arg_buf[0]);
    }
    leaf->stack_base[1] = JS_NewInt32(leaf->ctx, 1);
    leaf->stack_top = leaf->stack_base + 2;
    leaf->pc = leaf->bytecode_start + add_pc;
    assert(JS_JitInlineResume(leaf, add_pc, JS_JIT_INLINE_RESUME_INSTRUCTION) == JS_JIT_HELPER_OK);
    }
    assert(root->stack_top == root->stack_base + 1);
    assert(root->pc == root->bytecode_start + call_next);
#endif
    exit.resume_pc = root->pc;
    exit.resume_stack_top = root->stack_top;
    return exit;
}

static JSJitEntryHandle acquire(void *opaque, uint64_t id, uint64_t generation, uint32_t pc)
{
    JSJitEntryHandle handle = {0};
    handle.struct_size = sizeof(handle);
    if (id == outer_snapshot->function.id && generation == outer_snapshot->function.generation && !pc) {
        handle.entry = native;
        handle.pin = &entries;
        handle.stack_map_count = 1;
        handle.helper_abi_version = QJSJIT_HELPER_ABI_VERSION;
    }
    return handle;
}
static void release(void *opaque, JSJitEntryHandle handle) { releases++; }

static void run_case(unsigned mode)
{
    JSRuntime *rt = JS_NewRuntime();
    JSContext *ctx = JS_NewContext(rt);
    JSJitBackendVTable backend = {0};
    JSValue outer, callee, result, args[2], global, hits;
    double number;
    int count;
    const char *outer_src = "(function(f,x){return f(x)+10})";
    const char *callee_src = "(function(x){globalThis.hits++;return x+1})";
    scenario = mode;
    entries = releases = 0;
    polls = 0;
    middle_snapshot = NULL;
    if (mode == 2 || mode == 16 || mode == 17)
        outer_src = "(function(f,x){return f(globalThis.inner,x)+10})";
    if (mode == 3)
        callee_src = "(function(x){globalThis.hits++;try {throw 7} catch(e) {return x+e}})";
    if (mode == 7) {
        outer_src = "(function(f,x){return f(x,7,9)+10})";
        callee_src = "(function(x){globalThis.hits++;return x+arguments.length})";
    }
    if (mode == 8)
        callee_src = "(function(n){return function(x){globalThis.hits++;return (x+1)+n}})(7)";
    if (mode == 9) {
        outer_src = "(function(f,x){return f.run(x)+10})";
        callee_src = "(function(x){globalThis.hits++;return x+this.bump})";
    }
    if (mode == 11)
        callee_src = "(function(x){globalThis.saved=arguments;globalThis.hits++;x=x+1;return x+globalThis.saved[0]})";
    if (mode == 12)
        callee_src = "(function(x){var y=x;globalThis.saved=function(){return y};globalThis.hits++;y=x+1;return globalThis.saved()})";
    if (mode == 17)
        callee_src = "(function(x){globalThis.hits++;throw 7})";
    outer = JS_Eval(ctx, outer_src, strlen(outer_src), "outer.js", JS_EVAL_TYPE_GLOBAL);
    callee = JS_Eval(ctx, callee_src, strlen(callee_src), "callee.js", JS_EVAL_TYPE_GLOBAL);
    assert(!JS_IsException(outer) && !JS_IsException(callee));
    assert(JS_JitSnapshotFunction(ctx, outer, &outer_snapshot) == 0);
    assert(JS_JitSnapshotFunction(ctx, callee, &callee_snapshot) == 0);
    call_pc = find_opcode(outer_snapshot, mode == 2 || mode == 16 || mode == 17 ? "call2" : mode == 7 ? "call3" : mode == 9 ? "call_method" : "call1", 0);
    call_next = call_pc + (mode == 9 ? 3 : 1);
    add_pc = mode == 17 ? 0 : find_opcode(callee_snapshot, "add", 0);
    backend.struct_size = sizeof(backend);
    backend.acquire_entry = acquire;
    backend.release_entry = release;
    assert(JS_SetJitBackend(rt, &backend, NULL) == JS_JIT_BACKEND_OK);
    global = JS_GetGlobalObject(ctx);
    assert(JS_SetPropertyStr(ctx, global, "hits", JS_NewInt32(ctx, 0)) == 1);
    args[0] = callee;
    if (mode == 9) {
        args[0] = JS_NewObject(ctx);
        assert(JS_SetPropertyStr(ctx, args[0], "run", JS_DupValue(ctx, callee)) == 1);
        assert(JS_SetPropertyStr(ctx, args[0], "bump", JS_NewInt32(ctx, 7)) == 1);
    }
    if (mode == 2 || mode == 16 || mode == 17) {
        const char *source = "(function(f,x){return f(x)+20})";
        args[0] = JS_Eval(ctx, source, strlen(source), "middle.js", JS_EVAL_TYPE_GLOBAL);
        assert(!JS_IsException(args[0]));
        assert(JS_JitSnapshotFunction(ctx, args[0], &middle_snapshot) == 0);
        assert(JS_SetPropertyStr(ctx, global, "inner", JS_DupValue(ctx, callee)) == 1);
    }
    args[1] = JS_NewInt32(ctx, INT32_MAX);
    result = JS_Call(ctx, outer, JS_UNDEFINED, 2, args);
    if (mode == 13) {
        JS_SetInterruptHandler(rt, NULL, NULL);
        assert(polls == 1);
    }
    if (JS_IsException(result)) {
        JSValue exception = JS_GetException(ctx);
        if (mode == 5 || mode == 10 || mode == 17) {
            if (mode == 17) {
                assert(JS_VALUE_GET_TAG(exception) == JS_TAG_INT && JS_VALUE_GET_INT(exception) == 7);
                assert(entries == 1 && releases == 1);
                hits = JS_GetPropertyStr(ctx, global, "hits");
                assert(JS_ToInt32(ctx, &count, hits) == 0 && count == 1);
                JS_FreeValue(ctx, hits);
                JS_FreeValue(ctx, exception);
                goto cleanup;
            }
            assert(JS_IsUncatchableError(exception));
            assert(entries == 1 && releases == 1);
            hits = JS_GetPropertyStr(ctx, global, "hits");
            assert(JS_ToInt32(ctx, &count, hits) == 0 && count == (mode == 10 ? 0 : 1));
            JS_FreeValue(ctx, hits);
            JS_FreeValue(ctx, exception);
            goto cleanup;
        }
        const char *message = JS_ToCString(ctx, exception);
        fprintf(stderr, "exception: %s\n", message);
        JS_FreeCString(ctx, message);
        JS_FreeValue(ctx, exception);
        assert(!"unexpected recovery exception");
    }
    assert(JS_ToFloat64(ctx, &number, result) == 0);
    hits = JS_GetPropertyStr(ctx, global, "hits");
    assert(JS_ToInt32(ctx, &count, hits) == 0);
    if (number != (mode == 2 || mode == 16 ? 2147483678.0 : mode == 3 || mode == 9 ? 2147483664.0 : mode == 7 ? 2147483660.0 : mode == 8 ? 2147483665.0 : mode == 11 ? 4294967306.0 : 2147483658.0) ||
        count != 1 || entries != 1 || releases != 1) {
        fprintf(stderr, "result=%.0f hits=%d native_entries=%u releases=%u; expected 2147483658,1,1,1\n", number, count, entries, releases);
        assert(!"incorrect recovery result or replayed effect");
    }
    JS_FreeValue(ctx, hits);
    JS_FreeValue(ctx, result);
    if (mode == 11 || mode == 12) {
        JSValue saved = JS_GetPropertyStr(ctx, global, "saved"), value;
        JSVarRef *ref = mode == 11 ? JS_VALUE_GET_OBJ(saved)->u.array.u.var_refs[0] : JS_VALUE_GET_OBJ(saved)->u.func.var_refs[0];
        assert(ref->is_detached && ref->pvalue == &ref->value);
        JS_RunGC(rt);
        value = mode == 11 ? JS_GetPropertyUint32(ctx, saved, 0) : JS_Call(ctx, saved, JS_UNDEFINED, 0, NULL);
        assert(!JS_IsException(value));
        assert(JS_ToFloat64(ctx, &number, value) == 0 && number == 2147483648.0);
        JS_FreeValue(ctx, value);
        JS_FreeValue(ctx, saved);
    }
cleanup:
    JS_FreeValue(ctx, global);
    JS_FreeValue(ctx, outer);
    JS_FreeValue(ctx, callee);
    JS_JitFreeSnapshot(outer_snapshot);
    JS_JitFreeSnapshot(callee_snapshot);
    if (middle_snapshot) {
        JS_JitFreeSnapshot(middle_snapshot);
        JS_FreeValue(ctx, args[0]);
    }
    if (mode == 9) JS_FreeValue(ctx, args[0]);
    assert(JS_SetJitBackend(rt, NULL, NULL) == JS_JIT_BACKEND_OK);
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
}

int main(void)
{
    unsigned mode;
#ifdef QJSJIT_INLINE_RECOVERY_VERSION
    const JSJitInlineAPI *api = JS_JitGetInlineAPI(QJSJIT_INLINE_RECOVERY_VERSION);
    assert(api && api->struct_size == sizeof(*api));
    assert(api->enter == JS_JitInlineEnter && api->leave == JS_JitInlineLeave && api->resume == JS_JitInlineResume);
    assert(api->check == JS_JitInlineCheck);
    assert(JS_JitGetInlineAPI(UINT32_MAX) == NULL);
#endif
    for (mode = 0; mode < 18; mode++) run_case(mode);
    return 0;
}
"#;
