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
        "rutile-types/src/lib.rs",
        "rutile-protocol/src/lib.rs",
        "xtask/src/main.rs",
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"deterministic\n").unwrap();
    }
    // Contract manifests are required now that `create_scaffold` resolves a
    // Cargo.lock for the scaffolded xtask (xtask path-deps rutile-protocol).
    fs::write(
        root.path().join("rutile-types/Cargo.toml"),
        "[package]\nname = \"rutile-types\"\nversion = \"0.1.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[dependencies]\nhtml-escape.workspace = true\nthiserror.workspace = true\nurl.workspace = true\n",
    )
    .unwrap();
    fs::write(
        root.path().join("rutile-protocol/Cargo.toml"),
        "[package]\nname = \"rutile-protocol\"\nversion = \"0.1.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[dependencies]\nrutile-types = { path = \"../rutile-types\" }\nserde.workspace = true\nserde_json.workspace = true\nthiserror.workspace = true\n",
    )
    .unwrap();
    let contracts = format!(
        "{},{}",
        root.path().join("rutile-types").display(),
        root.path().join("rutile-protocol").display()
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

#[test]
fn evidence_validate_fails_closed_on_accessibility_source_mismatch() {
    let root = tempdir().unwrap();
    let input = root.path().join("accessibility.json");
    fs::write(
        &input,
        serde_json::to_vec(&serde_json::json!({
            "schema": "rutile.accessibility-attestation.v1",
            "version": 1,
            "source_commit": "0000000000000000000000000000000000000000",
            "platform": "macos",
            "tool": "voiceover",
            "rows": [{
                "action": "file/open",
                "passed": true,
                "evidence_ref": "release/evidence/readiness/file-open.wav"
            }],
            "summary": { "passed": 1, "total": 1, "failed": 0 },
            "unverified_rows": []
        }))
        .unwrap(),
    )
    .unwrap();

    let output = xtask()
        .args(["evidence", "validate", "--input"])
        .arg(&input)
        .args(["--schema", "accessibility-attestation"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("source commit/tree do not match"),
        "source mismatch must be reported: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn quality_probes_emit_writes_unsigned_unattested_bundle() {
    let root = tempdir().unwrap();
    let out = root.path().join("quality-probes.json");
    let output = xtask()
        .args(["quality-probes", "emit", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "well-formed catalog must exit 0 without GUI: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("attested=false"), "{stdout}");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(value["schema"], "rutile.quality-probe-bundle.v1");
    assert_eq!(value["attested"], false);
    assert_eq!(value["probes"].as_array().unwrap().len(), 14);
    assert!(value.get("publication_authorized").is_none());
    for probe in value["probes"].as_array().unwrap() {
        assert_eq!(probe["state"], "unattested");
        assert!(probe.get("passed").is_none());
    }
}

#[test]
fn quality_gate_doc_lists_every_emitted_probe_id() {
    let doc = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/evidence/quality-evidence-gate.md"),
    )
    .unwrap();
    let root = tempdir().unwrap();
    let out = root.path().join("quality-probes.json");
    assert!(
        xtask()
            .args(["quality-probes", "emit", "--out"])
            .arg(&out)
            .status()
            .unwrap()
            .success()
    );
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert!(doc.contains(value["schema"].as_str().unwrap()));
    for probe in value["probes"].as_array().unwrap() {
        let id = probe["id"].as_str().unwrap();
        assert!(doc.contains(id), "quality-evidence-gate.md must list {id}");
    }
}

fn metric_record() -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "schema":"rutile.metric.v1","v":1,"scenario":"paced-latency",
        "git_commit":"0123456789012345678901234567890123456789","dirty":false,
        "rustc_version":"rustc 1.88.0","toolchain":"1.88.0","target_triple":"aarch64-apple-darwin",
        "release_profile":"release","features":["test-control"],"build_kind":"instrumented",
        "candidate_executable_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "package_sha256":null,"runner_id":"rutile-macos-arm64-v1",
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
