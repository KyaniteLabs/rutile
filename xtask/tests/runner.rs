#![allow(clippy::disallowed_methods)] // Integration harness launches only the built xtask binary.

use std::process::Command;

use tempfile::tempdir;

const CLOSED_RUNNERS: &str = "fm-macos-arm64-v1,fm-macos-x86_64-v1,fm-ubuntu-x11-v1,fm-ubuntu-wayland-v1,fm-fedora-wayland-v1";

#[test]
fn normal_binary_is_unprovisioned_and_fails_before_io() {
    let root = tempdir().unwrap();
    let untouched = root.path().join("must-not-exist");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "runner",
            "capture-verify-matrix",
            "--runners",
            CLOSED_RUNNERS,
            "--capture-dir",
            untouched.join("captures").to_str().unwrap(),
            "--out",
            untouched.join("runner-lock-v1.json").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "xtask: production runner configuration is unprovisioned\n"
    );
    assert!(!untouched.exists());
}
