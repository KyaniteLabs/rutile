#![allow(clippy::disallowed_methods)] // Integration harness launches only the built xtask binary.

use sha2::{Digest, Sha256};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

#[test]
fn fixture_cli_generates_and_verifies_the_closed_fixture_set() {
    let root = tempdir().unwrap();
    let fixtures = root.path().join("fixtures");
    assert!(
        xtask()
            .args(["fixtures", "generate", "--out"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::metadata(fixtures.join("one-mib.md")).unwrap().len(),
        1_048_576
    );
    assert!(
        xtask()
            .args(["fixtures", "verify", "--dir"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
    fs::write(fixtures.join("small.md"), b"drift").unwrap();
    assert!(
        !xtask()
            .args(["fixtures", "verify", "--dir"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn comparator_cli_creates_and_verifies_the_locked_repository() {
    let root = tempdir().unwrap();
    for path in [
        "fixtures/small.md",
        "feathermark-types/src/lib.rs",
        "feathermark-protocol/src/lib.rs",
        "xtask/src/main.rs",
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"deterministic\n").unwrap();
    }
    let contracts = format!(
        "{},{}",
        root.path().join("feathermark-types").display(),
        root.path().join("feathermark-protocol").display()
    );
    let repo = root.path().join("repo");
    let lock = root.path().join("lock.json");
    assert!(
        xtask()
            .args(["comparator", "scaffold", "create", "--fixtures"])
            .arg(root.path().join("fixtures"))
            .args(["--contracts", &contracts, "--xtask"])
            .arg(root.path().join("xtask"))
            .arg("--out")
            .arg(&repo)
            .arg("--lock")
            .arg(&lock)
            .status()
            .unwrap()
            .success()
    );
    assert!(repo.join(".git").is_dir());
    assert!(lock.is_file());
    assert!(
        xtask()
            .args(["comparator", "scaffold", "verify", "--repo"])
            .arg(&repo)
            .arg("--lock")
            .arg(&lock)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn bootstrap_driver_cli_exposes_gui_metric_and_package_assertions() {
    let root = tempdir().unwrap();
    let commands = root.path().join("commands.ndjson");
    let events = root.path().join("events.ndjson");
    fs::write(
        &commands,
        b"{\"type\":\"close\",\"v\":1,\"request_id\":1}\n",
    )
    .unwrap();
    fs::write(&events, b"{\"type\":\"closed\",\"v\":1,\"request_id\":1}\n").unwrap();
    assert!(
        xtask()
            .args(["gui", "validate-transcript", "--commands"])
            .arg(&commands)
            .arg("--events")
            .arg(&events)
            .status()
            .unwrap()
            .success()
    );

    let metric = root.path().join("metric.ndjson");
    fs::write(&metric, metric_record()).unwrap();
    assert!(
        xtask()
            .args(["metrics", "assert-record", "--input"])
            .arg(&metric)
            .args(["--minimum-samples", "3", "--maximum-p95", "3"])
            .status()
            .unwrap()
            .success()
    );

    let artifact = root.path().join("artifact.bin");
    fs::write(&artifact, b"artifact").unwrap();
    let hash: String = Sha256::digest(b"artifact")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(
        xtask()
            .args(["package", "assert-file", "--path"])
            .arg(&artifact)
            .args(["--sha256", &hash, "--maximum-bytes", "8"])
            .status()
            .unwrap()
            .success()
    );
}

fn metric_record() -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "schema":"feathermark.metric.v1","v":1,"scenario":"paced-latency",
        "git_commit":"0123456789012345678901234567890123456789","dirty":false,
        "rustc_version":"rustc 1.88.0","toolchain":"1.88.0","target_triple":"aarch64-apple-darwin",
        "release_profile":"release","features":["test-control"],"build_kind":"instrumented",
        "candidate_executable_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "package_sha256":null,"runner_id":"fm-macos-arm64-v1",
        "runner_lock_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "pristine_snapshot_id":"snapshot","cpu_model":"Apple M1","cpu_cores":8,"ram_bytes":17179869184_u64,
        "os":"macOS","kernel":"Darwin","display_session":"native","display_environment":{},
        "webview_version":"WKWebView","monitor_scale_milli":1000,"monitor_refresh_millihz":60000,
        "fixture_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "fixture_bytes":1048576,"captured_at_utc":"2026-07-09T00:00:00Z",
        "monotonic_clock":"mach_continuous_time","warmups":5,"samples":[1,2,3],
        "skipped":0,"stale":0,"pid_rss_samples":[]
    })).unwrap();
    bytes.push(b'\n');
    bytes
}
