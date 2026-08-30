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

#[test]
fn pinned_public_quickjs_baseline_applies_cleanly_without_git() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("quickjs");
    let destination = scratch_dir();
    fs::create_dir(&destination).unwrap();
    for file in patch::BASELINE_FILES {
        fs::copy(source.join(file), destination.join(file)).unwrap();
    }

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
    let source = manifest.join("quickjs");
    let destination = scratch_dir();
    fs::create_dir(&destination).unwrap();
    for file in patch::BASELINE_FILES {
        fs::copy(source.join(file), destination.join(file)).unwrap();
    }
    fs::write(destination.join("quickjs.c"), "not the pinned baseline\n").unwrap();

    let error = patch::apply_patch_set(&destination, &manifest.join("patches")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!destination.join("quickjs-jit.h").exists());

    fs::remove_dir_all(destination).unwrap();
}
