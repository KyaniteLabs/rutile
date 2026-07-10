use std::fs;
use std::process::Command;

use ed25519_dalek::SigningKey;
use tempfile::tempdir;
use xtask::runner::{
    EXPECTED_RUNNERS, RunnerCapturePayload, TrustedRunnerKey, TrustedRunnerKeys,
    capture_verify_matrix, sign_capture, verify_runner_lock,
};

fn payload(runner_id: &str, snapshot_id: &str) -> RunnerCapturePayload {
    let (
        cpu_model,
        cpu_cores,
        arch,
        os_name,
        os_version,
        display_session,
        xdg,
        display,
        wayland,
        width,
        height,
        wk,
        gtk,
        webkit,
    ) = match runner_id {
        "fm-macos-arm64-v1" => (
            "Apple M1",
            8,
            "aarch64",
            "macOS",
            "15.5",
            "native",
            None,
            None,
            None,
            2560,
            1600,
            Some("620.2.4"),
            None,
            None,
        ),
        "fm-macos-x86_64-v1" => (
            "Intel Core i7-9750H",
            6,
            "x86_64",
            "macOS",
            "15.5",
            "native",
            None,
            None,
            None,
            1920,
            1080,
            Some("620.2.4"),
            None,
            None,
        ),
        "fm-ubuntu-x11-v1" => (
            "Intel Core i5-8500",
            6,
            "x86_64",
            "Ubuntu",
            "24.04",
            "x11",
            Some("x11"),
            Some(":0"),
            None,
            1920,
            1080,
            None,
            Some("3.24.41"),
            Some("2.44.3"),
        ),
        "fm-ubuntu-wayland-v1" => (
            "Intel Core i5-8500",
            6,
            "x86_64",
            "Ubuntu",
            "24.04",
            "wayland",
            Some("wayland"),
            None,
            Some("wayland-0"),
            1920,
            1080,
            None,
            Some("3.24.41"),
            Some("2.44.3"),
        ),
        "fm-fedora-wayland-v1" => (
            "Intel Core i5-8500",
            6,
            "x86_64",
            "Fedora",
            "43",
            "wayland",
            Some("wayland"),
            None,
            Some("wayland-0"),
            1920,
            1080,
            None,
            Some("3.24.49"),
            Some("2.48.1"),
        ),
        _ => panic!("unexpected test runner"),
    };
    RunnerCapturePayload {
        schema: "feathermark.runner-capture.v1".into(),
        runner_id: runner_id.into(),
        cpu_model: cpu_model.into(),
        cpu_cores,
        ram_bytes: 16 * 1024 * 1024 * 1024,
        arch: arch.into(),
        os_name: os_name.into(),
        os_version: os_version.into(),
        os_build: "exact-build-1".into(),
        kernel: "exact-kernel-1".into(),
        display_session: display_session.into(),
        xdg_session_type: xdg.map(str::to_owned),
        display: display.map(str::to_owned),
        wayland_display: wayland.map(str::to_owned),
        monitor_width_px: width,
        monitor_height_px: height,
        monitor_scale_milli: 1000,
        monitor_refresh_millihz: 60_000,
        gtk_version: gtk.map(str::to_owned),
        webkitgtk_version: webkit.map(str::to_owned),
        wkwebview_version: wk.map(str::to_owned),
        virtualized: true,
        vm_image_digest: Some(format!("sha256:{:064x}", runner_id.len())),
        snapshot_provider: "native-snapshot-service".into(),
        snapshot_id: snapshot_id.into(),
        captured_at: "2026-07-09T12:00:00Z".into(),
    }
}

fn create_capture_set(root: &std::path::Path) -> Vec<String> {
    let mut keys = Vec::new();
    for (index, runner_id) in EXPECTED_RUNNERS.iter().enumerate() {
        let secret = [index as u8 + 1; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        keys.push(TrustedRunnerKey {
            runner_id: (*runner_id).into(),
            public_key_hex: hex::encode(signing_key.verifying_key().as_bytes()),
        });
        let capture = sign_capture(&payload(runner_id, &format!("snapshot-{index}")), &secret);
        fs::write(
            root.join(format!("{runner_id}.capture.json")),
            serde_json::to_vec_pretty(&capture).unwrap(),
        )
        .unwrap();
    }
    fs::write(
        root.join("trusted-runner-keys-v1.json"),
        serde_json::to_vec_pretty(&TrustedRunnerKeys {
            schema: "feathermark.trusted-runner-keys.v1".into(),
            keys,
        })
        .unwrap(),
    )
    .unwrap();
    EXPECTED_RUNNERS.iter().map(|id| (*id).into()).collect()
}

#[test]
fn capture_matrix_verifies_signatures_and_closed_runner_set() {
    let root = tempdir().unwrap();
    let runners = create_capture_set(root.path());
    let lock_path = root.path().join("runner-lock-v1.json");
    let lock = capture_verify_matrix(&runners, root.path(), &lock_path).unwrap();
    assert_eq!(lock.runners.len(), 5);
    assert_eq!(verify_runner_lock(&lock_path).unwrap(), lock);

    fs::remove_file(root.path().join("fm-fedora-wayland-v1.capture.json")).unwrap();
    assert!(capture_verify_matrix(&runners, root.path(), &lock_path).is_err());
}

#[test]
fn capture_matrix_rejects_extra_substituted_and_tampered_evidence() {
    let root = tempdir().unwrap();
    let runners = create_capture_set(root.path());
    fs::write(root.path().join("extra.capture.json"), b"{}").unwrap();
    assert!(capture_verify_matrix(&runners, root.path(), &root.path().join("lock.json")).is_err());
    fs::remove_file(root.path().join("extra.capture.json")).unwrap();

    let path = root.path().join("fm-ubuntu-x11-v1.capture.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["payload"]["cpu_model"] = "substituted CPU".into();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    assert!(capture_verify_matrix(&runners, root.path(), &root.path().join("lock.json")).is_err());
}

#[test]
fn runner_cli_exposes_the_closed_capture_verify_command() {
    let root = tempdir().unwrap();
    let runners = create_capture_set(root.path());
    let lock = root.path().join("runner-lock-v1.json");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "runner",
            "capture-verify-matrix",
            "--runners",
            &runners.join(","),
            "--capture-dir",
            root.path().to_str().unwrap(),
            "--out",
            lock.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(verify_runner_lock(&lock).unwrap().runners.len(), 5);
}
