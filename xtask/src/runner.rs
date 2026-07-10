//! Closed production runner enrollment, verification, and launch authority.

use std::path::Path;

use thiserror::Error;

mod config;
#[cfg(test)]
#[allow(dead_code)] // Shared build-script parser is only partially exercised by library tests.
mod config_manifest;
#[allow(dead_code)] // Task 1C consumes fresh pre-spawn capabilities through app_launch.
pub(crate) mod current;
pub(crate) mod encoding;
mod engine;
pub(crate) mod protocol;
mod transaction;
mod transport;
pub(crate) mod verification;

pub const EXPECTED_RUNNERS: [&str; 5] = config::RUNNERS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSummary {
    pub runners: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OfflineLockSummary {
    pub runners: usize,
    pub lock_sha256: [u8; 32],
}

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("production runner configuration is unprovisioned")]
    Unprovisioned,
    #[error("production runner trust is unprovisioned")]
    UnprovisionedTrust,
    #[error("runner set must be exactly the closed five-row matrix")]
    RunnerSet,
    #[error("provisioned runner transport is unavailable: {0}")]
    Transport(String),
    #[error("runner protocol is invalid: {0}")]
    Protocol(String),
    #[error("runner lock publication failed: {0}")]
    Publication(String),
    #[error("filesystem durability contract was lost")]
    FilesystemContractLost,
    #[error("runner I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runner JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn capture_verify_matrix(
    runners: &[String],
    capture_dir: &Path,
    out: &Path,
) -> Result<CaptureSummary, RunnerError> {
    let config = config::production_config();
    let provisioned = match config {
        config::ProductionRunnerConfig::Unprovisioned => {
            return Err(RunnerError::Unprovisioned);
        }
        config::ProductionRunnerConfig::Provisioned(provisioned) => provisioned,
    };
    if runners.iter().map(String::as_str).ne(EXPECTED_RUNNERS) {
        return Err(RunnerError::RunnerSet);
    }
    engine::capture_with(
        &provisioned,
        &transport::ProductionLauncherTransport,
        capture_dir,
        out,
    )
}

pub fn verify_runner_lock_bytes(bytes: &[u8]) -> Result<OfflineLockSummary, RunnerError> {
    let provisioned = match config::production_config() {
        config::ProductionRunnerConfig::Unprovisioned => {
            return Err(RunnerError::UnprovisionedTrust);
        }
        config::ProductionRunnerConfig::Provisioned(provisioned) => provisioned,
    };
    let verified = verification::verify_runner_lock_bytes_with(bytes, &provisioned)?;
    Ok(OfflineLockSummary {
        runners: verified.identities.len(),
        lock_sha256: verified.lock_sha256,
    })
}

pub fn open_committed_runner_lock(out: &Path) -> Result<OfflineLockSummary, RunnerError> {
    let provisioned = match config::production_config() {
        config::ProductionRunnerConfig::Unprovisioned => {
            return Err(RunnerError::UnprovisionedTrust);
        }
        config::ProductionRunnerConfig::Provisioned(provisioned) => provisioned,
    };
    Ok(transaction::open_committed_runner_lock_with(out, &provisioned)?.summary())
}

#[cfg(test)]
mod tests {
    use super::encoding::{decode_probe_request, encode_probe_request};
    use super::protocol::{ProbePurpose, ProbeRequestV1};
    use super::test_support::{build_valid_lock, fake_transport};
    use super::transaction::{
        FailurePoint, open_committed_runner_lock_with, publish_runner_lock_with,
    };
    use super::verification::verify_runner_lock_bytes_with;

    #[test]
    fn request_cbor_is_deterministic_and_rejects_noncanonical_integers() {
        let request = ProbeRequestV1 {
            purpose: ProbePurpose::Enroll,
            run_id: [0; 32],
            runner_id: "fm-macos-arm64-v1".into(),
            challenge: [1; 32],
            issued_at_unix_ms: 1_000,
            not_after_unix_ms: 31_000,
            expected_snapshot_id: "snapshot".into(),
            expected_snapshot_provider: "provider".into(),
            expected_image_sha256: [2; 32],
            expected_probe_sha256: [3; 32],
            enrollment_commitment: None,
            final_lock_sha256: None,
            candidate_manifest_sha256: None,
        };
        let encoded = encode_probe_request(&request);
        assert_eq!(encoded.first(), Some(&0x8e));
        assert_eq!(decode_probe_request(&encoded).unwrap(), request);
        assert_eq!(
            encode_probe_request(&decode_probe_request(&encoded).unwrap()),
            encoded
        );

        let mut noncanonical = encoded;
        noncanonical.splice(1..2, [0x18, 0x01]);
        assert!(decode_probe_request(&noncanonical).is_err());
    }

    #[test]
    fn ten_exchange_lock_verifies_offline_and_rejects_enrollment_only() {
        let fixture = build_valid_lock();
        let verified = verify_runner_lock_bytes_with(&fixture.bytes, &fixture.config).unwrap();
        assert_eq!(verified.identities.len(), 5);

        let mut enrollment_only: serde_json::Value =
            serde_json::from_slice(&fixture.bytes).unwrap();
        enrollment_only["post_lock_exchanges"] = serde_json::json!([]);
        assert!(
            verify_runner_lock_bytes_with(
                &serde_json::to_vec(&enrollment_only).unwrap(),
                &fixture.config
            )
            .is_err()
        );
    }

    #[test]
    fn offline_verifier_rejects_replay_stale_commitment_and_identity_mutations() {
        let fixture = build_valid_lock();
        let original: serde_json::Value = serde_json::from_slice(&fixture.bytes).unwrap();
        let mut mutations = Vec::new();

        let mut replay = original.clone();
        replay["post_lock_exchanges"][0]["request"]["challenge"] =
            replay["enrollment_exchanges"][0]["request"]["challenge"].clone();
        mutations.push(replay);

        let mut stale = original.clone();
        stale["enrollment_exchanges"][0]["receipt"]["payload_cbor_hex"] =
            serde_json::Value::String("00".into());
        mutations.push(stale);

        let mut commitment = original.clone();
        commitment["enrollment_commitment"][0] = serde_json::json!(255);
        mutations.push(commitment);

        let mut identity = original;
        identity["identities"][0]["cpu_cores"] = serde_json::json!(7);
        mutations.push(identity);

        for mutated in mutations {
            assert!(
                verify_runner_lock_bytes_with(
                    &serde_json::to_vec(&mutated).unwrap(),
                    &fixture.config,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn production_offline_verifier_never_imports_caller_trust() {
        let error =
            super::verify_runner_lock_bytes(br#"{"trusted_keys":["attacker"]}"#).unwrap_err();
        assert!(matches!(error, super::RunnerError::UnprovisionedTrust));
    }

    #[test]
    fn private_fake_transport_drives_exact_ten_exchange_capture() {
        let fake = fake_transport();
        let directory = tempfile::tempdir().unwrap();
        let captures = directory.path().join("captures");
        let out = directory.path().join("runner-lock-v1.json");
        let summary = super::engine::capture_with(&fake.config, &fake, &captures, &out).unwrap();
        assert_eq!(summary.runners, 5);
        assert_eq!(std::fs::read_dir(captures).unwrap().count(), 10);
        assert!(open_committed_runner_lock_with(&out, &fake.config).is_ok());
    }

    #[test]
    fn committed_pair_is_authoritative_and_precommit_failure_never_authorizes() {
        let fixture = build_valid_lock();
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("runner-lock-v1.json");

        publish_runner_lock_with(&fixture.bytes, &out, &fixture.config, FailurePoint::None)
            .unwrap();
        let committed = open_committed_runner_lock_with(&out, &fixture.config).unwrap();
        assert_eq!(committed.summary().runners, 5);

        let second_out = directory.path().join("failed-lock.json");
        assert!(
            publish_runner_lock_with(
                &fixture.bytes,
                &second_out,
                &fixture.config,
                FailurePoint::BeforeCommittedRename,
            )
            .is_err()
        );
        assert!(open_committed_runner_lock_with(&second_out, &fixture.config).is_err());

        for (index, failure) in [
            FailurePoint::AfterIncompleteFsync,
            FailurePoint::AfterLockFsync,
            FailurePoint::AfterLockRename,
            FailurePoint::AfterLockParentFsync,
            FailurePoint::AfterJournalRewriteFsync,
            FailurePoint::AfterCommittedRename,
        ]
        .into_iter()
        .enumerate()
        {
            let staged_out = directory.path().join(format!("failed-stage-{index}.json"));
            assert!(
                publish_runner_lock_with(&fixture.bytes, &staged_out, &fixture.config, failure,)
                    .is_err()
            );
            assert!(open_committed_runner_lock_with(&staged_out, &fixture.config).is_err());
        }

        let cleanup_out = directory.path().join("cleanup-lock.json");
        publish_runner_lock_with(
            &fixture.bytes,
            &cleanup_out,
            &fixture.config,
            FailurePoint::Cleanup,
        )
        .unwrap();
        assert!(open_committed_runner_lock_with(&cleanup_out, &fixture.config).is_ok());

        let durable_out = directory.path().join("durable-lock.json");
        publish_runner_lock_with(
            &fixture.bytes,
            &durable_out,
            &fixture.config,
            FailurePoint::AfterCommittedParentFsync,
        )
        .unwrap();
        assert!(open_committed_runner_lock_with(&durable_out, &fixture.config).is_ok());
    }

    #[test]
    fn publication_parent_handle_survives_namespace_substitution() {
        let fixture = build_valid_lock();
        let outer = tempfile::tempdir().unwrap();
        let parent = outer.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let out = parent.join("runner-lock.json");
        let error = publish_runner_lock_with(
            &fixture.bytes,
            &out,
            &fixture.config,
            FailurePoint::AfterParentSwap,
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected parent namespace swap"));
        assert!(parent.is_dir());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        let retained = outer.path().join("parent.retained");
        assert!(retained.is_dir());
        assert!(std::fs::read_dir(retained).unwrap().count() > 0);

        let symlink_parent = outer.path().join("symlink-parent");
        std::os::unix::fs::symlink(&parent, &symlink_parent).unwrap();
        assert!(
            publish_runner_lock_with(
                &fixture.bytes,
                &symlink_parent.join("lock.json"),
                &fixture.config,
                FailurePoint::None,
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod test_support;
