use std::{path::Path, process::Command};

#[test]
#[ignore = "runs the initialized pinned Test262 corpus through the Rust host"]
fn interpreter() {
    let status = Command::new(env!("CARGO_BIN_EXE_jit-test262"))
        .args([
            "--mode",
            "interpreter",
            "--limit",
            "100",
            "--output",
            "../target/jit-test262/interpreter.json",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(Path::new("../target/jit-test262/interpreter.json").is_file());
}

#[test]
#[ignore = "runs the initialized pinned Test262 corpus through automatic tiering"]
fn automatic() {
    let status = Command::new(env!("CARGO_BIN_EXE_jit-test262"))
        .args([
            "--mode",
            "automatic",
            "--limit",
            "100",
            "--output",
            "../target/jit-test262/automatic.json",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(Path::new("../target/jit-test262/automatic.json").is_file());
}
