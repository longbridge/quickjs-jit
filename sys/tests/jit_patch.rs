#![cfg(not(target_os = "wasi"))]

#[path = "../build_support/patch.rs"]
mod patch;

use std::{fs, path::PathBuf, time::SystemTime};

fn scratch_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("rquickjs-jit-patch-{}-{nonce}", std::process::id()))
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
    assert!(jit_header.contains("#define QJSJIT_ABI_MINOR 19u"));
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
    assert!(jit_header.contains("QJSJIT_RUNTIME_API_MINOR 9u"));
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
    assert!(helper_header.contains("#define QJSJIT_DECLARE_MAP_OUT_IN_OP(name)"));
    assert!(quickjs.contains("JS_JitHelperBinaryArithSlow"));
    assert!(quickjs.contains("JS_JitHelperUnaryArithSlow"));
    assert!(quickjs.contains("#define QJSJIT_ABI_COUNT_MAP_OUT_IN_OP 6"));
    assert!(jit_header.contains("JSJitFeedbackEvent"));
    assert!(destination.join("quickjs-jit-helpers.h").is_file());
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
            binding.contains("pub const QJSJIT_ABI_MINOR: u32 = 19;"),
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
                && binding.contains("pub unary_arith_slow: ::core::option::Option"),
            "{target}"
        );
        assert!(
            binding.contains("JS_JIT_HELPER_BINARY_ARITH_SLOW: JSJitHelperId = 22")
                && binding.contains("JS_JIT_HELPER_UNARY_ARITH_SLOW: JSJitHelperId = 23")
                && binding.contains("JS_JIT_HELPER_COUNT: JSJitHelperId = 24"),
            "{target}"
        );
        assert!(
            binding.contains("pub const QJSJIT_RUNTIME_API_MINOR: u32 = 9;"),
            "{target}"
        );
        assert!(
            binding.contains("size_of::<JSJitRuntimeAPI>() - 200usize"),
            "{target}"
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
