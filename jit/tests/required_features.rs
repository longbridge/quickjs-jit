use std::process::Command;

#[test]
fn native_semantics_targets_require_their_execution_features() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
        .expect("run cargo metadata");
    assert!(output.status.success(), "cargo metadata failed");

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse cargo metadata JSON");
    let package = metadata["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .find(|package| package["name"] == "rquickjs-jit")
        .expect("rquickjs-jit package");

    for test_name in ["helpers", "semantics", "gpui_shell_surface"] {
        let target = package["targets"]
            .as_array()
            .expect("targets array")
            .iter()
            .find(|target| target["name"] == test_name)
            .unwrap_or_else(|| panic!("missing {test_name} test target"));
        let mut features = target["required-features"]
            .as_array()
            .expect("required-features array")
            .iter()
            .map(|feature| feature.as_str().expect("feature name"))
            .collect::<Vec<_>>();
        features.sort_unstable();
        assert_eq!(features, ["compiler", "test-support"], "{test_name}");
    }
}
