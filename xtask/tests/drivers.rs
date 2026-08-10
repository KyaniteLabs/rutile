use std::fs;

use sha2::{Digest, Sha256};
use tempfile::tempdir;
use xtask::gui::validate_transcript;
use xtask::metrics::{MetricAssertion, assert_metric_record};
use xtask::package::assert_file;

#[test]
fn gui_driver_parses_bounded_correlated_command_and_event_streams() {
    let commands = b"{\"type\":\"focus_editor\",\"v\":1,\"request_id\":1}\n{\"type\":\"close\",\"v\":1,\"request_id\":2}\n";
    let events = b"{\"type\":\"focus_changed\",\"v\":1,\"request_id\":1,\"surface\":\"editor\"}\n{\"type\":\"closed\",\"v\":1,\"request_id\":2}\n";
    let summary = validate_transcript(commands, events).unwrap();
    assert_eq!(summary.commands, 2);
    assert_eq!(summary.events, 2);

    let uncorrelated = b"{\"type\":\"closed\",\"v\":1,\"request_id\":3}\n";
    assert!(validate_transcript(commands, uncorrelated).is_err());
}

#[test]
fn metric_driver_applies_nearest_rank_without_dropping_samples() {
    let record = metric_record(&[1, 7, 3, 5]);
    let result = assert_metric_record(
        &record,
        &MetricAssertion {
            minimum_samples: 4,
            maximum_p95: 7,
        },
    )
    .unwrap();
    assert_eq!(result.p95, 7);
    assert!(
        assert_metric_record(
            &record,
            &MetricAssertion {
                minimum_samples: 5,
                maximum_p95: 7,
            }
        )
        .is_err()
    );
}

#[test]
fn package_driver_hashes_the_exact_file_and_applies_a_byte_cap() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("artifact.bin");
    fs::write(&path, b"artifact").unwrap();
    let hash: String = Sha256::digest(b"artifact")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let result = assert_file(&path, &hash, 8).unwrap();
    assert_eq!(result.bytes, 8);
    assert!(assert_file(&path, &hash, 7).is_err());
    assert!(assert_file(&path, &"0".repeat(64), 8).is_err());
}

fn metric_record(samples: &[u64]) -> Vec<u8> {
    let mut record = serde_json::to_vec(&serde_json::json!({
        "schema": "rutile.metric.v1",
        "v": 1,
        "scenario": "paced-latency",
        "git_commit": "0123456789012345678901234567890123456789",
        "dirty": false,
        "rustc_version": "rustc 1.88.0",
        "toolchain": "1.88.0",
        "target_triple": "aarch64-apple-darwin",
        "release_profile": "release",
        "features": ["test-control"],
        "build_kind": "instrumented",
        "candidate_executable_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "package_sha256": null,
        "runner_id": "fm-macos-arm64-v1",
        "runner_lock_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "pristine_snapshot_id": "snapshot",
        "cpu_model": "Apple M1",
        "cpu_cores": 8,
        "ram_bytes": 17179869184_u64,
        "os": "macOS",
        "kernel": "Darwin",
        "display_session": "native",
        "display_environment": {},
        "webview_version": "WKWebView",
        "monitor_scale_milli": 1000,
        "monitor_refresh_millihz": 60000,
        "fixture_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "fixture_bytes": 1048576,
        "captured_at_utc": "2026-07-09T00:00:00Z",
        "monotonic_clock": "mach_continuous_time",
        "warmups": 5,
        "samples": samples,
        "skipped": 0,
        "stale": 0,
        "pid_rss_samples": []
    })).unwrap();
    record.push(b'\n');
    record
}
