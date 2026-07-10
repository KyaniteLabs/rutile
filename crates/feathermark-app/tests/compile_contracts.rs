use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn compile_fixture(name: &str, source: &str) -> std::process::Output {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "feathermark-compile-contract-{}-{name}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        directory.join("Cargo.toml"),
        format!("[package]\nname='compile-contract-{name}'\nversion='0.0.0'\nedition='2024'\n[dependencies]\nfeathermark-app={{path={app:?}}}\n"),
    )
    .unwrap();
    fs::write(directory.join("src/main.rs"), source).unwrap();
    #[allow(clippy::disallowed_methods)] // This test is the audited compile-fail owner.
    let output = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--quiet"])
        .current_dir(&directory)
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(directory);
    output
}

fn check_features(name: &str, features: &str) -> std::process::Output {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let target = std::env::temp_dir().join(format!(
        "feathermark-feature-contract-{}-{name}",
        std::process::id()
    ));
    #[allow(clippy::disallowed_methods)] // This test is the audited compile-fail owner.
    let output = Command::new(env!("CARGO"))
        .args([
            "check",
            "--quiet",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--no-default-features",
            "--features",
            features,
        ])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(target);
    output
}

#[test]
fn render_execution_permit_is_not_cloneable() {
    let output = compile_fixture(
        "permit-clone",
        r#"
use std::sync::Arc;
use feathermark_app::render_scheduler::{RenderRequest, RenderScheduler, DEBOUNCE_MS};
fn main() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(RenderRequest::new(1, Arc::from("x")), 0);
    let permit = scheduler.start_ready(DEBOUNCE_MS).unwrap();
    let _duplicate = permit.clone();
}
"#,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no method named `clone`"));
}

#[test]
fn mutually_exclusive_platform_features_are_a_compile_error() {
    let output = check_features("both", "linux-gtk,macos-shell");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutually exclusive"));
}

#[test]
fn platform_feature_for_the_wrong_target_is_a_compile_error() {
    let wrong_feature = if cfg!(target_os = "linux") {
        "macos-shell"
    } else {
        "linux-gtk"
    };
    let expected = if cfg!(target_os = "linux") {
        "requires a macOS target"
    } else {
        "requires a Linux target"
    };
    let output = check_features("wrong-target", wrong_feature);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
}
