#![allow(clippy::disallowed_methods)] // Integration test launches only the built xtask binary.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;
use xtask::artifact_inspector::{ArtifactInspector, FindingCode, InspectionMode, PolicyPaths};
use xtask::local_package_cli::enforce_inspection;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[test]
fn artifact_inspect_cli_emits_json_and_rejects_forbidden_input() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"RUTILE_TEST_CONTROL").unwrap();
    let paths = write_policy(root.path(), &[]);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["artifact", "inspect", "--artifact"])
        .arg(&artifact)
        .arg("--quarantine")
        .arg(&paths.quarantine)
        .arg("--policy")
        .arg(&paths.policy)
        .args(["--mode", "candidate"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "rutile.artifact-inspection.v1");
    assert_eq!(report["inspector_version"], "artifact-inspect-v1");
    assert_eq!(report["inspection_mode"], "candidate");
    assert!(report["policy_sha256"].as_str().unwrap().len() == 64);
    assert!(report["quarantine_sha256"].as_str().unwrap().len() == 64);
    assert_eq!(report["publication_authorized"], false);
    assert_eq!(report["accepted"], false);
    assert_eq!(report["findings"][0]["code"], "test_control_marker");
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_policy(root: &std::path::Path, quarantined: &[(&str, &str)]) -> PolicyPaths {
    let quarantine = root.join("quarantine.json");
    let entries: Vec<_> = quarantined
        .iter()
        .map(|(hash, name)| {
            serde_json::json!({
                "sha256": hash,
                "artifact": name,
                "reason": "synthetic test quarantine",
                "discovered_at": "2026-07-12"
            })
        })
        .collect();
    fs::write(
        &quarantine,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "rutile.artifact-quarantine.v1",
            "version": 1,
            "entries": entries
        }))
        .unwrap(),
    )
    .unwrap();

    let policy = root.join("policy.toml");
    fs::write(
        &policy,
        r#"schema = "rutile.artifact-inspector-policy.v1"
version = 1
max_entries = 256
max_uncompressed_bytes = 67108864
expected_license = "MIT"
expected_version = "0.2.0"
expected_source_commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
forbidden_patterns = [
  "RUTILE_TEST_CONTROL",
  "FEATHERMARK_TEST_CONTROL",
  "/home/build-user/",
  "token=synthetic-secret"
]
test_control_environment = ["RUTILE_TEST_CONTROL", "FEATHERMARK_TEST_CONTROL"]
"#,
    )
    .unwrap();
    PolicyPaths { quarantine, policy }
}

/// Write a zip archive at `path` containing the given `(entry-name, bytes)` pairs.
fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    for (name, data) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

/// A minimal clean `.app`-layout zip matching the inspector policy's expected
/// license/version/source-commit. Baseline for the zip-reader tests.
fn clean_app_zip_entries() -> Vec<(&'static str, Vec<u8>)> {
    let manifest = serde_json::json!({
        "license": "MIT",
        "version": "0.2.0",
        "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    })
    .to_string()
    .into_bytes();
    let info_plist = b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>
<key>CFBundleIdentifier</key><string>x</string>
<key>CFBundleDocumentTypes</key><array/>
<key>UTTypeConformsTo</key><array/>
</dict></plist>"
        .to_vec();
    vec![
        ("Rutile.app/Contents/MacOS/FeatherMark", b"Mach-O".to_vec()),
        ("Rutile.app/Contents/Info.plist", info_plist),
        (
            "Rutile.app/Contents/Resources/package-manifest-v1.json",
            manifest,
        ),
        (
            "Rutile.app/Contents/Resources/sbom.spdx.json",
            b"{}".to_vec(),
        ),
    ]
}

#[test]
fn zip_reader_accepts_clean_app_layout_without_unsupported_archive() {
    let root = tempdir().unwrap();
    let zip_path = root.path().join("Rutile-0.2.0-macos-arm64.app.zip");
    write_zip(&zip_path, &clean_app_zip_entries());
    let paths = write_policy(root.path(), &[]);

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&zip_path, InspectionMode::Package, None);

    assert_eq!(report.artifact_kind, "zip_archive");
    assert!(!report.has(FindingCode::UnsupportedArchive));
    assert!(report.has(FindingCode::ProvenanceMissing));
    // Clean bounded scan: provenance findings do not block acceptance.
    assert!(report.accepted);
}

#[test]
fn zip_reader_rejects_forbidden_pattern_in_a_zipped_entry() {
    let root = tempdir().unwrap();
    let zip_path = root.path().join("Rutile-0.2.0-macos-arm64.app.zip");
    let mut entries = clean_app_zip_entries();
    entries.push((
        "Rutile.app/Contents/Resources/leaked.txt",
        b"RUTILE_TEST_CONTROL leak".to_vec(),
    ));
    write_zip(&zip_path, &entries);
    let paths = write_policy(root.path(), &[]);

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&zip_path, InspectionMode::Package, None);

    assert!(!report.has(FindingCode::UnsupportedArchive));
    assert!(report.has(FindingCode::TestControlMarker));
    assert!(!report.accepted);
}

#[test]
fn zip_reader_skips_macosx_appledouble_metadata() {
    let root = tempdir().unwrap();
    let zip_path = root.path().join("Rutile-0.2.0-macos-arm64.app.zip");
    let mut entries = clean_app_zip_entries();
    // ditto emits __MACOSX/._* AppleDouble metadata; a forbidden pattern there
    // must NOT be flagged (archive-level metadata, not app content).
    entries.push((
        "__MACOSX/Rutile.app/Contents/Resources/._leaked.txt",
        b"RUTILE_TEST_CONTROL".to_vec(),
    ));
    write_zip(&zip_path, &entries);
    let paths = write_policy(root.path(), &[]);

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&zip_path, InspectionMode::Package, None);

    assert!(!report.has(FindingCode::TestControlMarker));
    assert!(report.has(FindingCode::ProvenanceMissing));
    assert!(report.accepted);
}

#[test]
fn quarantined_hash_is_rejected_before_content_is_trusted() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("old-package.bin");
    fs::write(&artifact, b"opaque historical package").unwrap();
    let paths = write_policy(
        root.path(),
        &[(&sha256(b"opaque historical package"), "old-package.bin")],
    );

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&artifact, InspectionMode::Package, None);

    assert!(!report.accepted);
    assert_eq!(report.findings[0].code, FindingCode::QuarantinedHash);
}

#[test]
fn binary_scan_reports_test_control_and_private_build_markers() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(
        &artifact,
        b"ELF RUTILE_TEST_CONTROL /home/build-user/private token=synthetic-secret",
    )
    .unwrap();
    let paths = write_policy(root.path(), &[]);

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        None,
    );

    assert!(!report.accepted);
    assert!(report.has(FindingCode::TestControlMarker));
    assert!(report.has(FindingCode::PrivateBuildPath));
    assert!(report.has(FindingCode::CredentialMarker));
}

#[test]
fn binary_scan_rejects_email_and_common_token_shapes_without_echoing_them() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(
        &artifact,
        b"contact=builder@example.invalid credential=ghp_SYNTHETIC_PLACEHOLDER",
    )
    .unwrap();
    let paths = write_policy(root.path(), &[]);

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        None,
    );

    assert!(!report.accepted);
    assert!(report.has(FindingCode::CredentialMarker));
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("builder@example.invalid"));
    assert!(!json.contains("ghp_SYNTHETIC_PLACEHOLDER"));
}

#[test]
fn binary_scan_ignores_short_token_prefixes_but_rejects_credential_length_tokens() {
    let root = tempdir().unwrap();
    let short = root.path().join("short-prefix");
    fs::write(&short, b"normal text containing sk- only").unwrap();
    let paths = write_policy(root.path(), &[]);

    let short_report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&short, InspectionMode::Candidate, None);
    // Scan is clean; provenance is missing (fail-closed finding, recorded but
    // does not block scan acceptance — publication is gated separately).
    assert!(short_report.accepted);
    assert!(short_report.has(FindingCode::ProvenanceMissing));

    let token = root.path().join("credential-shaped");
    fs::write(&token, b"sk-1234567890abcdefghijklmnopqrstuvwxyz").unwrap();
    let token_report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&token, InspectionMode::Candidate, None);
    assert!(token_report.has(FindingCode::CredentialMarker));
}

#[test]
fn package_directory_requires_one_expected_executable_and_matching_metadata() {
    let root = tempdir().unwrap();
    let package = root.path().join("Rutile-linux-x86_64");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(package.join("bin/feathermark"), b"\x7fELF clean production").unwrap();
    fs::write(
        package.join("package-manifest-v1.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "feathermark-local-package-v1",
            "architecture": "x86_64-unknown-linux-gnu",
            "source_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "version": "9.9.9",
            "license": "Proprietary",
            "wayland_verified": false,
            "rpm_runtime_verified": false
        }))
        .unwrap(),
    )
    .unwrap();
    let paths = write_policy(root.path(), &[]);

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&package, InspectionMode::Package, None);

    assert!(!report.accepted);
    assert!(report.has(FindingCode::VersionMismatch));
    assert!(report.has(FindingCode::LicenseMismatch));
    assert!(report.has(FindingCode::SourceCommitMismatch));
    assert!(report.has(FindingCode::PlatformMetadataMissing));
}

#[test]
fn packaging_boundary_rejects_clean_scan_without_publication_authorization() {
    let root = tempdir().unwrap();
    let package = root.path().join("Rutile-linux-x86_64");
    for directory in ["bin", "share/applications", "share/mime/packages"] {
        fs::create_dir_all(package.join(directory)).unwrap();
    }
    fs::write(package.join("bin/feathermark"), b"\x7fELF production").unwrap();
    fs::write(
        package.join("share/applications/feathermark.desktop"),
        b"desktop",
    )
    .unwrap();
    fs::write(
        package.join("share/mime/packages/feathermark-markdown.xml"),
        b"mime",
    )
    .unwrap();
    fs::write(
        package.join("package-manifest-v1.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "feathermark-local-package-v1",
            "architecture": "x86_64-unknown-linux-gnu",
            "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "version": "0.2.0",
            "license": "MIT",
            "wayland_verified": true,
            "rpm_runtime_verified": true
        }))
        .unwrap(),
    )
    .unwrap();
    let inspector = ArtifactInspector::load(&write_policy(root.path(), &[])).unwrap();
    // Provide a valid provenance file so the scan can be accepted.
    let provenance = root.path().join("package.provenance.json");
    fs::write(
        &provenance,
        serde_json::to_vec(&serde_json::json!({
            "schema": "rutile.production-provenance.v1",
            "version": 1,
            "product": "feathermark",
            "product_version": "0.2.0",
            "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "source_tree_clean": true,
            "toolchain": {
                "rustc_version": "1.88.0",
                "host_triple": "x86_64-unknown-linux-gnu",
                "target_triple": "x86_64-unknown-linux-gnu"
            },
            "features": [],
            "candidate_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "reproducibility": {
                "source_date_epoch": 1720915200,
                "remap_path_prefix": true,
                "target_root": "target-prod"
            },
            "built_at": "2024-07-14T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let report = inspector.inspect(&package, InspectionMode::Package, Some(&provenance));
    assert!(report.accepted);
    assert!(!report.publication_authorized);
    assert!(
        report.production_provenance_sha256.is_some(),
        "provenance SHA-256 must be bound when a valid provenance file is provided"
    );

    let error = enforce_inspection(
        &inspector,
        &package,
        InspectionMode::Package,
        Some(&provenance),
    )
    .unwrap_err();
    assert!(error.to_string().contains("publication_not_authorized"));
}

#[test]
fn traversal_is_bounded_by_entry_count_and_total_bytes() {
    let root = tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    for index in 0..257 {
        fs::write(package.join(format!("entry-{index}")), b"x").unwrap();
    }
    let paths = write_policy(root.path(), &[]);

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&package, InspectionMode::Package, None);

    assert!(!report.accepted);
    assert!(report.has(FindingCode::EntryLimitExceeded));
    assert_eq!(report.entries_scanned, 256);
}

#[test]
fn traversal_reports_never_claim_more_bytes_than_the_policy_allowed_to_scan() {
    let root = tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("first"), b"123456").unwrap();
    fs::write(package.join("second"), b"789012").unwrap();
    let paths = write_policy(root.path(), &[]);
    let policy = fs::read_to_string(&paths.policy).unwrap().replace(
        "max_uncompressed_bytes = 67108864",
        "max_uncompressed_bytes = 8",
    );
    fs::write(&paths.policy, policy).unwrap();

    let report =
        ArtifactInspector::load(&paths)
            .unwrap()
            .inspect(&package, InspectionMode::Package, None);

    assert!(report.has(FindingCode::ByteLimitExceeded));
    assert_eq!(report.uncompressed_bytes_scanned, 8);
}

#[test]
fn repository_quarantine_exactly_tracks_the_five_0_2_0_artifacts() {
    let defaults = PolicyPaths::repository_defaults();
    let quarantine: serde_json::Value =
        serde_json::from_slice(&fs::read(defaults.quarantine).unwrap()).unwrap();
    let evidence: serde_json::Value = serde_json::from_slice(
        &fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("docs/evidence/local-beta-0.2.0/manifest-index.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let quarantined: std::collections::BTreeSet<_> = quarantine["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["sha256"].as_str().unwrap())
        .collect();
    let evidenced: std::collections::BTreeSet<_> = evidence["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["artifact_sha256"].as_str().unwrap())
        .collect();

    assert_eq!(quarantined.len(), 5);
    assert_eq!(quarantined, evidenced);

    let policy: toml::Value =
        toml::from_str(&fs::read_to_string(PolicyPaths::repository_defaults().policy).unwrap())
            .unwrap();
    let patterns: std::collections::BTreeSet<_> = policy["forbidden_patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    for required in [
        "--native-smoke",
        "FEATHERMARK_LIFECYCLE_CYCLE",
        "FEATHERMARK_PRODUCT_FUNCTIONAL_PATH",
        "FEATHERMARK_SMOKE_AUTOCLOSE_MS",
        "FEATHERMARK_STARTUP_TRACE",
    ] {
        assert!(
            patterns.contains(required),
            "missing production hook {required}"
        );
    }
}

#[test]
fn production_policy_classifies_native_smoke_argument_as_test_control() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"program --native-smoke").unwrap();

    let report = ArtifactInspector::load(&PolicyPaths::repository_defaults())
        .unwrap()
        .inspect(&artifact, InspectionMode::Candidate, None);

    assert!(report.has(FindingCode::TestControlMarker));
}

#[test]
fn malformed_or_duplicate_quarantine_entries_fail_closed() {
    let root = tempdir().unwrap();
    let paths = write_policy(
        root.path(),
        &[
            (
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "one.bin",
            ),
            (
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "two.bin",
            ),
        ],
    );

    let error = ArtifactInspector::load(&paths).unwrap_err();
    assert!(error.to_string().contains("duplicate quarantine SHA-256"));
}

#[test]
fn provenance_missing_pushes_finding_when_not_provided() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"clean production binary").unwrap();
    let paths = write_policy(root.path(), &[]);

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        None,
    );

    // Scan is clean (accepted), but provenance is missing — the finding is
    // recorded (fail-closed evidence) but does not block scan acceptance.
    // Publication is gated separately by publication_authorized.
    assert!(report.accepted);
    assert!(report.has(FindingCode::ProvenanceMissing));
    assert!(
        report.production_provenance_sha256.is_none(),
        "provenance SHA-256 must be None when no provenance is provided"
    );
}

#[test]
fn provenance_binds_when_valid_file_provided() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"clean production binary").unwrap();
    let paths = write_policy(root.path(), &[]);

    let provenance = root.path().join("candidate.provenance.json");
    let provenance_json = serde_json::json!({
        "schema": "rutile.production-provenance.v1",
        "version": 1,
        "product": "feathermark",
        "product_version": "0.2.0",
        "source_commit": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
        "source_tree_clean": true,
        "toolchain": {
            "rustc_version": "1.88.0",
            "host_triple": "aarch64-apple-darwin",
            "target_triple": "aarch64-apple-darwin"
        },
        "features": [],
        "candidate_sha256": hex::encode(Sha256::digest(b"clean production binary")),
        "reproducibility": {
            "source_date_epoch": 1720915200,
            "remap_path_prefix": true,
            "target_root": "target-prod"
        },
        "built_at": "2024-07-14T00:00:00Z"
    });
    fs::write(
        &provenance,
        serde_json::to_vec_pretty(&provenance_json).unwrap(),
    )
    .unwrap();

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        Some(&provenance),
    );

    assert!(
        report.production_provenance_sha256.is_some(),
        "provenance SHA-256 must be bound when a valid provenance file is provided"
    );
    let expected_sha = hex::encode(Sha256::digest(
        serde_json::to_vec_pretty(&provenance_json).unwrap(),
    ));
    assert_eq!(
        report.production_provenance_sha256.as_deref(),
        Some(expected_sha.as_str()),
        "provenance SHA-256 must match the hash of the provenance file bytes"
    );
    assert!(!report.has(FindingCode::ProvenanceMissing));
    assert!(!report.has(FindingCode::ProvenanceInvalid));
}

#[test]
fn provenance_invalid_pushes_finding_for_malformed_file() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"clean production binary").unwrap();
    let paths = write_policy(root.path(), &[]);

    let provenance = root.path().join("candidate.provenance.json");
    fs::write(&provenance, b"not valid json at all {{{").unwrap();

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        Some(&provenance),
    );

    // Scan is clean (accepted), but provenance is invalid — the finding is
    // recorded (fail-closed evidence) but does not block scan acceptance.
    assert!(report.accepted);
    assert!(report.has(FindingCode::ProvenanceInvalid));
    assert!(report.production_provenance_sha256.is_none());
}

#[test]
fn provenance_invalid_pushes_finding_for_wrong_schema() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"clean production binary").unwrap();
    let paths = write_policy(root.path(), &[]);

    let provenance = root.path().join("candidate.provenance.json");
    fs::write(
        &provenance,
        serde_json::to_vec(&serde_json::json!({
            "schema": "some.other.schema.v2",
            "version": 2
        }))
        .unwrap(),
    )
    .unwrap();

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        Some(&provenance),
    );

    // Scan is clean (accepted), but provenance has wrong schema — the finding
    // is recorded (fail-closed evidence) but does not block scan acceptance.
    assert!(report.accepted);
    assert!(report.has(FindingCode::ProvenanceInvalid));
    assert!(report.production_provenance_sha256.is_none());
}

#[test]
fn provenance_sibling_file_is_discovered_when_not_explicitly_provided() {
    let root = tempdir().unwrap();
    let artifact = root.path().join("candidate");
    fs::write(&artifact, b"clean production binary").unwrap();
    let paths = write_policy(root.path(), &[]);

    // Write a sibling provenance file following the naming convention.
    let provenance = root.path().join("candidate.provenance.json");
    fs::write(
        &provenance,
        serde_json::to_vec(&serde_json::json!({
            "schema": "rutile.production-provenance.v1",
            "version": 1,
            "product": "feathermark",
            "product_version": "0.2.0",
            "source_commit": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            "source_tree_clean": true,
            "toolchain": {
                "rustc_version": "1.88.0",
                "host_triple": "aarch64-apple-darwin",
                "target_triple": "aarch64-apple-darwin"
            },
            "features": [],
            "candidate_sha256": "0".repeat(64),
            "reproducibility": {
                "source_date_epoch": 1720915200,
                "remap_path_prefix": true,
                "target_root": "target-prod"
            },
            "built_at": "2024-07-14T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();

    let report = ArtifactInspector::load(&paths).unwrap().inspect(
        &artifact,
        InspectionMode::Candidate,
        None,
    );

    // The sibling file is discovered and bound — no ProvenanceMissing finding.
    assert!(
        report.production_provenance_sha256.is_some(),
        "sibling provenance file should be discovered and bound"
    );
    assert!(!report.has(FindingCode::ProvenanceMissing));
    assert!(!report.has(FindingCode::ProvenanceInvalid));
}
