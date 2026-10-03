#![allow(clippy::disallowed_methods)] // Audited CLI argument rejection tests; no shell or native launch.

use std::process::Command;

#[test]
fn help_and_version_exit_without_launching_the_native_shell() {
    for flag in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rutile"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().contains("Rutile"));
    }
}

#[test]
fn malformed_flags_and_extra_paths_fail_before_launch() {
    for args in [vec!["--definitely-invalid"], vec!["one.md", "two.md"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_rutile"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!output.stderr.is_empty());
    }
}

#[cfg(not(feature = "test-control"))]
#[test]
fn production_build_rejects_smoke_flag_instead_of_opening_user_state() {
    let output = Command::new(env!("CARGO_BIN_EXE_rutile"))
        .arg("--native-smoke")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
