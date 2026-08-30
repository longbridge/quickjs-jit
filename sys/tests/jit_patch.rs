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
}

#[test]
fn pinned_public_quickjs_baseline_applies_cleanly_without_git() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let destination = scratch_dir();
    copy_baseline(&destination);

    patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap();

    let quickjs = fs::read_to_string(destination.join("quickjs.c")).unwrap();
    let jit_header = fs::read_to_string(destination.join("quickjs-jit.h")).unwrap();
    assert!(quickjs.contains("JS_GetJitRuntimeId"));
    assert!(quickjs.contains("JS_JIT_FRAME_SIDE_PATH_HIT"));
    assert!(jit_header.contains("#define QJSJIT_ABI_MINOR 4u"));
    assert!(destination.join("quickjs-jit-helpers.h").is_file());
    fs::remove_dir_all(destination).unwrap();
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
fn patch_set_rejects_missing_extra_and_modified_patches_before_writing() {
    let root = scratch_dir();
    let source = root.join("source");
    let patches = root.join("patches");
    copy_baseline(&source);
    fs::create_dir(&patches).unwrap();
    assert!(patch::apply_patch_set(&source, &patches).is_err());
    assert!(!source.join("quickjs-jit.h").exists());

    copy_patches(&patches);
    fs::write(patches.join("0002-extra.patch"), "not allowed\n").unwrap();
    assert!(patch::apply_patch_set(&source, &patches).is_err());
    fs::remove_file(patches.join("0002-extra.patch")).unwrap();

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
