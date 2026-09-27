#![cfg(not(target_os = "wasi"))]

#[path = "../build_support/patch.rs"]
mod patch;

use std::{fs, path::PathBuf, time::SystemTime};

#[cfg(unix)]
use std::process::Command;

fn scratch_dir() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rquickjs-jit-patch-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn copy_baseline(destination: &std::path::Path) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("quickjs");
    fs::create_dir_all(destination).unwrap();
    for file in patch::BASELINE_FILES {
        fs::copy(source.join(file), destination.join(file)).unwrap();
    }
}

fn copy_patches(destination: &std::path::Path) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("patches");
    fs::create_dir_all(destination).unwrap();
    fs::copy(
        source.join("0001-rquickjs-jit.patch"),
        destination.join("0001-rquickjs-jit.patch"),
    )
    .unwrap();
    for patch in [
        "0002-runtime-feedback.patch",
        "0003-element-layout.patch",
        "0004-tier1-globals.patch",
        "0005-tier1-constructors.patch",
        "0006-tier1-regexp.patch",
        "0007-msan-slot-boundary.patch",
        "0008-tier1-atom-values.patch",
        "0009-tier1-arith-slow.patch",
        "0010-native-entry-cache.patch",
        "0011-helper-pc-cursor.patch",
        "0012-shape-retention.patch",
        "0013-feedback-gate.patch",
        "0014-inactive-probes.patch",
        "0015-cold-object-probes.patch",
        "0017-shape-generation.patch",
        "0018-inline-frame-recovery.patch",
        "0019-array-feedback.patch",
        "0020-typed-array-guard.patch",
        "0021-helper-boundary.patch",
        "0022-inline-refcount.patch",
        "0023-fast-native-entry.patch",
        "0025-tier1-closures.patch",
        "0026-tier1-generic-ops.patch",
        "0027-tier1-exception-regions.patch",
        "0028-tier1-iteration.patch",
        "0029-native-call-convention.patch",
        "0032-object-fast-paths.patch",
    ] {
        fs::copy(source.join(patch), destination.join(patch)).unwrap();
    }
}

fn convert_baseline_to_crlf(destination: &std::path::Path) {
    for file in patch::BASELINE_FILES {
        let source = destination.join(file);
        let bytes = fs::read(&source).unwrap();
        fs::write(source, as_crlf(&bytes)).unwrap();
    }
}

fn convert_patches_to_crlf(destination: &std::path::Path) {
    for entry in fs::read_dir(destination).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("patch") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        fs::write(path, as_crlf(&bytes)).unwrap();
    }
}

fn as_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut crlf = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        if bytes[index] == b'\n' {
            crlf.push(b'\r');
        }
        crlf.push(bytes[index]);
        index += 1;
    }
    crlf
}

#[test]
fn crlf_conversion_is_idempotent() {
    assert_eq!(as_crlf(b"one\ntwo\r\n"), b"one\r\ntwo\r\n");
}

#[test]
fn pinned_public_quickjs_baseline_applies_cleanly_without_git() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);

    patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap();

    let quickjs = fs::read_to_string(destination.join("quickjs.c")).unwrap();
    let jit_header = fs::read_to_string(destination.join("quickjs-jit.h")).unwrap();
    let helper_header = fs::read_to_string(destination.join("quickjs-jit-helpers.h")).unwrap();
    assert!(quickjs.contains("JS_GetJitRuntimeId"));
    assert!(quickjs.contains("JS_JIT_FRAME_SIDE_PATH_HIT"));
    assert!(jit_header.contains("#define QJSJIT_ABI_MINOR 25u"));
    assert!(jit_header.contains("#define JS_JIT_FRAME_FAST_ENTRY (1U << 3)"));
    assert!(jit_header.contains("JSJitFastEntryGrant *grant);\n} JSJitBackendVTable;"));
    assert!(quickjs.contains("JS_JIT_FRAME_SIDE_PATH_HIT |\n        JS_JIT_FRAME_FAST_ENTRY;"));
    assert!(quickjs.contains("static inline bool qjsjit_fast_entry_ready("));
    assert!(jit_header.contains("JS_JitCatchException"));
    assert!(quickjs.contains("JSJitHelperStatus JS_JitThrowValue("));
    assert!(quickjs.contains("JSJitHelperStatus JS_JitThrowError("));
    assert!(quickjs.contains("JSJitHelperStatus JS_JitCatchException("));
    assert!(quickjs.contains("b->func_kind != JS_FUNC_NORMAL ?"));
    assert!(jit_header.contains("JS_JitGetIteratorAPI(uint32_t version)"));
    assert!(quickjs.contains("JSJitHelperStatus JS_JitHelperIteratorOp("));
    assert!(quickjs.contains("qjsjit_array_values_next"));
    assert!(jit_header.contains("JSJitPropertyLayout property_layout;"));
    assert!(quickjs.contains("uint64_t jit_shape_generation;"));
    assert!(!quickjs.contains("QJSJIT_SHAPE_HASH"));
    assert!(jit_header.contains("uint64_t payload;"));
    assert!(quickjs.contains("constants[i].payload = 0;"));
    assert!(quickjs.contains("JS_JitHelperShapeGuard"));
    assert!(quickjs.contains("JS_JitHelperMaterializeOwner"));
    assert!(quickjs.contains(
        "(void)stack_map_id;\n    if (qjsjit_validate_helper_frame(frame, false, 0, &b, &sf) < 0)"
    ));
    assert_eq!(
        quickjs
            .matches(
                "(void)stack_map_id;\n    if (qjsjit_validate_helper_frame(frame, false, 0, &b, &sf) < 0)"
            )
            .count(),
        3
    );
    assert!(jit_header.contains("QJSJIT_RUNTIME_API_MINOR 10u"));
    assert!(helper_header.contains("X(GENERIC_OP, generic_op, JS_JitHelperGenericOp"));
    assert!(quickjs.contains("JSJitHelperStatus JS_JitHelperGenericOp("));
    assert!(quickjs.contains("rt->jit_active_root = root_call.previous;"));
    assert!(jit_header.contains("#define QJSJIT_RUNTIME_FIELD_MAP_OUT_IN_OP(field)"));
    assert!(helper_header.contains("JS_JIT_HELPER_MATERIALIZED = 2"));
    assert!(helper_header.contains("JS_JIT_OWNER_SOURCE_ARGUMENT = 0"));
    assert!(helper_header.contains("JS_JIT_OWNER_SOURCE_LOCAL = 1"));
    assert!(helper_header.contains("JS_JIT_OWNER_SOURCE_OWNED_STACK = 2"));
    assert!(helper_header.contains("X(MATERIALIZE_OWNER, materialize_owner"));
    assert!(helper_header.contains("X(GET_ELEMENT, get_element"));
    assert!(helper_header.contains("X(SET_ELEMENT, set_element"));
    assert!(helper_header.contains("X(TO_PROPKEY, to_propkey"));
    assert!(helper_header.contains("X(GET_GLOBAL, get_global"));
    assert!(helper_header.contains("X(CALL_CONSTRUCTOR, call_constructor"));
    assert!(helper_header.contains("X(REGEXP, regexp"));
    assert!(helper_header.contains("X(ATOM_VALUE, atom_value"));
    assert!(helper_header.contains("X(BINARY_ARITH_SLOW, binary_arith_slow"));
    assert!(helper_header.contains("X(UNARY_ARITH_SLOW, unary_arith_slow"));
    assert!(helper_header.contains("X(FCLOSURE, fclosure"));
    assert!(helper_header.contains("X(GET_VAR_REF, get_var_ref"));
    assert!(helper_header.contains("X(PUT_VAR_REF, put_var_ref"));
    assert!(helper_header.contains("X(CLOSE_LOC, close_loc"));
    assert!(helper_header.contains("X(SET_NAME, set_name"));
    assert!(helper_header.contains("JS_JIT_VAR_REF_CHECK_INIT = 2"));
    assert!(helper_header.contains("X(ITERATOR_OP, iterator_op"));
    assert!(helper_header.contains("#define QJSJIT_DECLARE_MAP_OUT_IN_OP(name)"));
    assert!(quickjs.contains("JS_JitHelperBinaryArithSlow"));
    assert!(quickjs.contains("JS_JitHelperUnaryArithSlow"));
    assert!(quickjs.contains("JS_JitHelperFClosure"));
    assert!(quickjs.contains("JS_JitHelperGetVarRef"));
    assert!(quickjs.contains("JS_JitHelperPutVarRef"));
    assert!(quickjs.contains("JS_JitHelperCloseLoc"));
    assert!(quickjs.contains("JS_JitHelperSetName"));
    assert!(quickjs.contains("#define QJSJIT_ABI_COUNT_MAP_OUT_IN_OP 6"));
    assert!(jit_header.contains("JSJitFeedbackEvent"));
    assert!(jit_header.contains("JS_EXTERN const JSJitObjectAPI *JS_JitGetObjectAPI"));
    assert!(quickjs.contains("static int qjsjit_object_literal("));
    assert!(quickjs.contains("static int qjsjit_array_push("));
    assert!(destination.join("quickjs-jit-helpers.h").is_file());
    fs::remove_dir_all(destination).unwrap();
}

#[cfg(unix)]
#[test]
fn patched_quickjs_compiles_without_the_jit_abi() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);
    patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap();

    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let output = Command::new(compiler)
        .current_dir(&destination)
        .args(["-D_GNU_SOURCE", "-c", "quickjs.c", "-o", "quickjs.o"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "patched QuickJS failed to compile without CONFIG_JIT_ABI:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn pinned_public_quickjs_baseline_with_crlf_applies_cleanly() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);
    convert_baseline_to_crlf(&destination);

    patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap();

    for file in patch::BASELINE_FILES {
        assert!(
            !fs::read(destination.join(file))
                .unwrap()
                .windows(2)
                .any(|pair| pair == b"\r\n"),
            "patched source retains CRLF: {file}"
        );
    }
    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn crlf_patch_checkout_applies_cleanly() {
    let root = scratch_dir();
    let source = root.join("source");
    let patches = root.join("patches");
    copy_baseline(&source);
    copy_patches(&patches);
    convert_patches_to_crlf(&patches);

    patch::apply_patch_set(&source, &patches).unwrap();

    assert!(source.join("quickjs-jit.h").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn crlf_patch_checkout_with_content_tampering_is_rejected() {
    let root = scratch_dir();
    let source = root.join("source");
    let patches = root.join("patches");
    copy_baseline(&source);
    copy_patches(&patches);
    convert_patches_to_crlf(&patches);
    let patch = patches.join("0001-rquickjs-jit.patch");
    let mut bytes = fs::read(&patch).unwrap();
    bytes.extend_from_slice(b"tampered\r\n");
    fs::write(patch, bytes).unwrap();

    let error = patch::apply_patch_set(&source, &patches).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!source.join("quickjs-jit.h").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundled_jit_bindings_include_materialize_owner_tail() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bindings = manifest.join("src/bindings");
    let targets = [
        "aarch64-apple-darwin.rs",
        "aarch64-pc-windows-msvc.rs",
        "aarch64-unknown-linux-gnu.rs",
        "aarch64-unknown-linux-musl.rs",
        "x86_64-apple-darwin.rs",
        "x86_64-pc-windows-gnu.rs",
        "x86_64-pc-windows-msvc.rs",
        "x86_64-unknown-linux-gnu.rs",
        "x86_64-unknown-linux-musl.rs",
    ];
    for target in targets {
        let binding = fs::read_to_string(bindings.join(target)).unwrap();
        assert!(
            binding.contains("pub const QJSJIT_ABI_MINOR: u32 = 25;")
                && binding.contains("pub fn JS_JitThrowValue(")
                && binding.contains("pub fn JS_JitThrowError(")
                && binding.contains("pub fn JS_JitCatchException("),
            "{target}"
        );
        assert!(
            binding.contains("pub struct JSJitArrayMetadata"),
            "{target}"
        );
        assert!(binding.contains("pub struct JSJitArrayAPI"), "{target}");
        assert!(
            binding.contains("pub struct JSJitFastEntryGrant")
                && binding.contains("offset_of!(JSJitBackendVTable, entry_fast_grant) - 96usize")
                && binding.contains("size_of::<JSJitBackendVTable>() - 104usize"),
            "{target}"
        );
        assert!(
            binding.contains("pub struct JSJitObjectAPI")
                && binding.contains("pub fn JS_JitGetObjectAPI")
                && binding.contains("pub const QJSJIT_OBJECT_API_VERSION: u32 = 1;")
                && binding.contains("size_of::<JSJitObjectAPI>() - 56usize"),
            "{target}"
        );
        assert!(
            binding.contains("receiver: *const JSValue")
                && binding.contains("pub fn JS_JitGetArrayAPI"),
            "{target}"
        );
        assert!(
            binding.lines().any(|line| {
                let line = line.trim();
                line.starts_with("pub const JS_JIT_HELPER_MATERIALIZED:") && line.ends_with(" = 2;")
            }),
            "{target}"
        );
        assert!(
            binding.contains("JS_JIT_HELPER_MATERIALIZE_OWNER: JSJitHelperId = 14"),
            "{target}"
        );
        assert!(
            binding.contains("pub materialize_owner: ::core::option::Option"),
            "{target}"
        );
        assert!(binding.contains("pub payload: u64"), "{target}");
        assert!(
            binding.contains("pub get_element: ::core::option::Option")
                && binding.contains("pub set_element: ::core::option::Option")
                && binding.contains("pub to_propkey: ::core::option::Option")
                && binding.contains("pub get_global: ::core::option::Option")
                && binding.contains("pub call_constructor: ::core::option::Option")
                && binding.contains("pub regexp: ::core::option::Option")
                && binding.contains("pub atom_value: ::core::option::Option")
                && binding.contains("pub binary_arith_slow: ::core::option::Option")
                && binding.contains("pub unary_arith_slow: ::core::option::Option")
                && binding.contains("pub iterator_op: ::core::option::Option"),
            "{target}"
        );
        assert!(
            binding.contains("JS_JIT_HELPER_BINARY_ARITH_SLOW: JSJitHelperId = 22")
                && binding.contains("JS_JIT_HELPER_UNARY_ARITH_SLOW: JSJitHelperId = 23")
                && binding.contains("JS_JIT_HELPER_FCLOSURE: JSJitHelperId = 24")
                && binding.contains("JS_JIT_HELPER_GET_VAR_REF: JSJitHelperId = 25")
                && binding.contains("JS_JIT_HELPER_PUT_VAR_REF: JSJitHelperId = 26")
                && binding.contains("JS_JIT_HELPER_CLOSE_LOC: JSJitHelperId = 27")
                && binding.contains("JS_JIT_HELPER_SET_NAME: JSJitHelperId = 28")
                && binding.contains("JS_JIT_HELPER_GENERIC_OP: JSJitHelperId = 29")
                && binding.contains("pub generic_op: ::core::option::Option")
                && binding.contains("pub fn JS_JitHelperGenericOp(")
                && binding.contains("JS_JIT_HELPER_ITERATOR_OP: JSJitHelperId = 30")
                && binding.contains("pub fn JS_JitHelperIteratorOp(")
                && binding.contains("JS_JIT_HELPER_COUNT: JSJitHelperId = 31"),
            "{target}"
        );
        assert!(
            binding.contains("pub const QJSJIT_RUNTIME_API_MINOR: u32 = 10;"),
            "{target}"
        );
        assert!(
            binding.contains("size_of::<JSJitRuntimeAPI>() - 256usize")
                && binding.contains("offset_of!(JSJitRuntimeAPI, iterator_op) - 248usize")
                && binding.contains("pub close_loc: ::core::option::Option")
                && binding.contains(
                    "pub fn JS_JitGetIteratorAPI(version: u32) -> *const JSJitIteratorAPI;"
                ),
            "{target}"
        );
    }
}

#[test]
fn bundled_wasm_jit_binding_matches_iteration_abi_1_25() {
    let binding = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bindings/wasm32-wasip1.rs"),
    )
    .unwrap();

    for declaration in [
        // ABI 1.22: inline recovery.
        "pub const QJSJIT_INLINE_RECOVERY_VERSION: u32 = 1;",
        "pub const JS_JIT_INLINE_CALL: u32 = 0;",
        "pub const JS_JIT_INLINE_CALL_METHOD: u32 = 1;",
        "pub const JS_JIT_INLINE_RESUME_INSTRUCTION: u32 = 0;",
        "pub const JS_JIT_INLINE_RESUME_EXCEPTION: u32 = 1;",
        "pub const JS_JIT_INLINE_MAX_DEPTH: u32 = 16;",
        "pub const JS_JIT_INLINE_MAX_BYTES: u32 = 1048576;",
        "pub fn JS_JitInlineEnter(",
        "pub fn JS_JitInlineLeave(",
        "pub fn JS_JitInlineResume(",
        "pub fn JS_JitInlineCheck(",
        "pub struct JSJitInlineAPI",
        "pub fn JS_JitGetInlineAPI(",
        "size_of::<JSJitInlineAPI>() - 40usize",
        "align_of::<JSJitInlineAPI>() - 4usize",
        "offset_of!(JSJitInlineAPI, struct_size) - 0usize",
        "offset_of!(JSJitInlineAPI, version) - 4usize",
        "offset_of!(JSJitInlineAPI, max_depth) - 8usize",
        "offset_of!(JSJitInlineAPI, max_bytes) - 12usize",
        "offset_of!(JSJitInlineAPI, effects) - 16usize",
        "offset_of!(JSJitInlineAPI, reserved) - 20usize",
        "offset_of!(JSJitInlineAPI, enter) - 24usize",
        "offset_of!(JSJitInlineAPI, leave) - 28usize",
        "offset_of!(JSJitInlineAPI, resume) - 32usize",
        "offset_of!(JSJitInlineAPI, check) - 36usize",
        // ABI 1.23: array feedback.
        "JSJitFeedbackKind_JS_JIT_FEEDBACK_ARRAY: JSJitFeedbackKind = 6;",
        "JSJitArrayMode_JS_JIT_ARRAY_MODE_GENERIC: JSJitArrayMode = 0;",
        "JSJitArrayMode_JS_JIT_ARRAY_MODE_PACKED: JSJitArrayMode = 1;",
        "JSJitArrayMode_JS_JIT_ARRAY_MODE_INT32: JSJitArrayMode = 2;",
        "JSJitArrayMode_JS_JIT_ARRAY_MODE_FLOAT64: JSJitArrayMode = 3;",
        "pub const JS_JIT_FEEDBACK_ARRAY_STORE: u32 = 64;",
        "pub const JS_JIT_FEEDBACK_ARRAY_LENGTH: u32 = 128;",
        "pub const JS_JIT_FEEDBACK_ARRAY_EXOTIC: u32 = 256;",
        "pub const JS_JIT_FEEDBACK_ARRAY_SLOW: u32 = 512;",
        "pub const JS_JIT_FEEDBACK_ARRAY_RESIZABLE: u32 = 1024;",
        "pub const JS_JIT_FEEDBACK_ARRAY_DETACHED: u32 = 2048;",
        "pub const JS_JIT_FEEDBACK_ARRAY_IMMUTABLE: u32 = 4096;",
        "pub const JS_JIT_FEEDBACK_ARRAY_SHARED: u32 = 8192;",
        "pub const JS_JIT_FEEDBACK_ARRAY_INVALID_BACKING: u32 = 16384;",
        // ABI 1.25: callback-free native entry.
        "pub const QJSJIT_ABI_MINOR: u32 = 25;",
        "pub const JS_JIT_FRAME_FAST_ENTRY: u32 = 8;",
        "pub const JS_JIT_FAST_ENTRY_OPTIMIZED: u32 = 1;",
        "pub const JS_JIT_EXIT_FAST_UNPAIRED: u32 = 2147483648;",
        "pub const JS_JIT_FAST_ENTRY_MAX_BUDGET: u32 = 65536;",
        "size_of::<JSJitFastEntryState>() - 40usize",
        "size_of::<JSJitFastEntryGrant>() - 20usize",
        "offset_of!(JSJitFastEntryGrant, state) - 16usize",
        "size_of::<JSJitBackendVTable>() - 52usize",
        "offset_of!(JSJitBackendVTable, entry_fast_grant) - 48usize",
        // ABI 1.25: Tier 1 closure helpers.
        "pub const QJSJIT_RUNTIME_API_MINOR: u32 = 10;",
        "JS_JIT_HELPER_CLOSE_LOC: JSJitHelperId = 27",
        "JSJitVarRefMode_JS_JIT_VAR_REF_CHECK_INIT: JSJitVarRefMode = 2;",
        "size_of::<JSJitRuntimeAPI>() - 132usize",
        "JS_JIT_HELPER_SET_NAME: JSJitHelperId = 28",
        // ABI 1.25: exact generic-opcode helper (appended after the closure tail).
        "JS_JIT_HELPER_GENERIC_OP: JSJitHelperId = 29;",
        "pub fn JS_JitHelperGenericOp(",
        "offset_of!(JSJitRuntimeAPI, generic_op) - 124usize",
        // ABI 1.25: Tier 1 exception regions (exported entry points only).
        "pub fn JS_JitThrowValue(",
        "pub fn JS_JitThrowError(",
        "pub fn JS_JitCatchException(",
        // ABI 1.24: typed-array continuing guard.
        "pub const QJSJIT_ARRAY_API_VERSION: u32 = 1;",
        "pub const JS_JIT_ARRAY_QUERY_LENGTH: u32 = 1;",
        "JSJitArrayQueryStatus_JS_JIT_ARRAY_QUERY_INVALID: JSJitArrayQueryStatus = -1;",
        "JSJitArrayQueryStatus_JS_JIT_ARRAY_QUERY_MISS: JSJitArrayQueryStatus = 0;",
        "JSJitArrayQueryStatus_JS_JIT_ARRAY_QUERY_OK: JSJitArrayQueryStatus = 1;",
        "pub struct JSJitArrayMetadata",
        "pub type JSJitArrayQueryFunc",
        "receiver: *const JSValue",
        "pub struct JSJitArrayAPI",
        "pub fn JS_JitGetArrayAPI(",
        // ABI 1.25: Tier 1 iteration helper (appended after GENERIC_OP) and
        // Array values leaf.
        "pub const QJSJIT_ITERATOR_API_VERSION: u32 = 1;",
        "JSJitHelperId_JS_JIT_HELPER_ITERATOR_OP: JSJitHelperId = 30;",
        "pub fn JS_JitHelperIteratorOp(",
        "pub struct JSJitIteratorAPI",
        "pub fn JS_JitGetIteratorAPI(",
        "size_of::<JSJitIteratorAPI>() - 20usize",
        "offset_of!(JSJitIteratorAPI, array_values_next) - 16usize",
        "offset_of!(JSJitRuntimeAPI, iterator_op) - 128usize",
        "JSJitHelperId_JS_JIT_HELPER_COUNT: JSJitHelperId = 31;",
        // CONFIG_JIT_TEST_SUPPORT declarations retained by the CI generator.
        "pub fn JS_JitGetHelperCount(",
        "pub fn JS_JitSetExecutionTrace(",
        "pub fn JS_JitGetExecutionTraceLength(",
        // wasm32 layout assertions emitted by bindgen.
        "size_of::<JSJitArrayMetadata>() - 20usize",
        "align_of::<JSJitArrayMetadata>() - 4usize",
        "offset_of!(JSJitArrayMetadata, struct_size) - 0usize",
        "offset_of!(JSJitArrayMetadata, mode) - 4usize",
        "offset_of!(JSJitArrayMetadata, count) - 8usize",
        "offset_of!(JSJitArrayMetadata, reserved) - 12usize",
        "offset_of!(JSJitArrayMetadata, data) - 16usize",
        "size_of::<JSJitArrayAPI>() - 20usize",
        "align_of::<JSJitArrayAPI>() - 4usize",
        "offset_of!(JSJitArrayAPI, struct_size) - 0usize",
        "offset_of!(JSJitArrayAPI, version) - 4usize",
        "offset_of!(JSJitArrayAPI, effects) - 8usize",
        "offset_of!(JSJitArrayAPI, reserved) - 12usize",
        "offset_of!(JSJitArrayAPI, query) - 16usize",
        // ABI 1.25: object allocation and array-method fast paths.
        "pub const QJSJIT_OBJECT_API_VERSION: u32 = 1;",
        "pub const JS_JIT_OBJECT_LITERAL_MAX_FIELDS: u32 = 16;",
        "JSJitObjectStatus_JS_JIT_OBJECT_MISS: JSJitObjectStatus = 0;",
        "pub struct JSJitObjectAPI",
        "pub fn JS_JitGetObjectAPI(",
        "size_of::<JSJitObjectAPI>() - 36usize",
        "align_of::<JSJitObjectAPI>() - 4usize",
        "offset_of!(JSJitObjectAPI, literal) - 16usize",
        "offset_of!(JSJitObjectAPI, retain_shape) - 20usize",
        "offset_of!(JSJitObjectAPI, array_method) - 24usize",
        "offset_of!(JSJitObjectAPI, array_push) - 28usize",
        "offset_of!(JSJitObjectAPI, array_push_method) - 32usize",
    ] {
        assert!(
            binding.contains(declaration),
            "wasm32-wasip1 binding is missing `{declaration}`"
        );
    }
}

#[test]
fn wrong_quickjs_baseline_is_rejected_before_writing_outputs() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);
    fs::write(destination.join("quickjs.c"), "not the pinned baseline\n").unwrap();

    let error = patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!destination.join("quickjs-jit.h").exists());

    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn crlf_quickjs_baseline_with_content_tampering_is_rejected() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);
    convert_baseline_to_crlf(&destination);
    let source = destination.join("quickjs.c");
    let mut bytes = fs::read(&source).unwrap();
    bytes.extend_from_slice(b"/* tampered */\r\n");
    fs::write(source, bytes).unwrap();

    let error = patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!destination.join("quickjs-jit.h").exists());

    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn patch_set_rejects_missing_extra_and_modified_patches_before_writing() {
    let root = scratch_dir();
    let source = root.join("source");
    let patches = root.join("patches");
    copy_baseline(&source);
    fs::create_dir(&patches).unwrap();
    assert!(patch::apply_patch_set(&source, &patches).is_err());
    assert!(!source.join("quickjs-jit.h").exists());

    copy_patches(&patches);
    fs::write(patches.join("0003-extra.patch"), "not allowed\n").unwrap();
    assert!(patch::apply_patch_set(&source, &patches).is_err());
    fs::remove_file(patches.join("0003-extra.patch")).unwrap();

    let expected = patches.join("0001-rquickjs-jit.patch");
    let mut contents = fs::read_to_string(&expected).unwrap();
    contents.push('\n');
    fs::write(&expected, contents).unwrap();
    assert!(patch::apply_patch_set(&source, &patches).is_err());
    assert!(!source.join("quickjs-jit.h").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn patch_set_rejects_header_edits_and_unknown_file_creation() {
    let root = scratch_dir();
    for (needle, replacement) in [
        ("+++ b/quickjs.c", "+++ b/quickjs.h"),
        ("+++ b/quickjs-jit.h", "+++ b/unknown-jit.h"),
    ] {
        let source = root.join(if needle.contains("quickjs.c") {
            "header"
        } else {
            "unknown"
        });
        let patches = source.with_extension("patches");
        copy_baseline(&source);
        copy_patches(&patches);
        let file = patches.join("0001-rquickjs-jit.patch");
        let changed = fs::read_to_string(&file)
            .unwrap()
            .replacen(needle, replacement, 1);
        fs::write(file, changed).unwrap();
        assert!(patch::apply_patch_set(&source, &patches).is_err());
        assert!(!source.join("quickjs-jit.h").exists());
        assert!(!source.join("unknown-jit.h").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn patch_directory_read_errors_are_not_ignored() {
    let root = scratch_dir();
    let source = root.join("source");
    let not_a_directory = root.join("patch-file");
    copy_baseline(&source);
    fs::write(&not_a_directory, "x").unwrap();
    assert!(patch::apply_patch_set(&source, &not_a_directory).is_err());
    fs::remove_dir_all(root).unwrap();
}
