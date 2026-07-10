//! Verified candidate manifest input to runner authorization. Task 1C wires its producers.

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::runner::EXPECTED_RUNNERS;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CandidateError {
    #[error("candidate manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("candidate manifest is not the exact closed five-row schema")]
    Invalid,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateManifestV1 {
    schema: String,
    rows: Vec<CandidateRowV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateRowV1 {
    runner_id: String,
    snapshot_provider: String,
    snapshot_image_sha256: [u8; 32],
    executable_sha256: [u8; 32],
}

pub(crate) struct VerifiedCandidateManifest {
    pub(crate) sha256: [u8; 32],
    rows: Vec<CandidateRowV1>,
}

pub(crate) struct CandidateSnapshotExpectation {
    pub(crate) runner_id: String,
    pub(crate) snapshot_id: String,
    pub(crate) snapshot_provider: String,
    pub(crate) snapshot_image_sha256: [u8; 32],
    pub(crate) executable_sha256: [u8; 32],
}

pub(crate) fn verify_manifest(bytes: &[u8]) -> Result<VerifiedCandidateManifest, CandidateError> {
    let manifest: CandidateManifestV1 = serde_json::from_slice(bytes)?;
    if manifest.schema != "feathermark.candidate-manifest.v1"
        || manifest.rows.len() != EXPECTED_RUNNERS.len()
    {
        return Err(CandidateError::Invalid);
    }
    for (index, row) in manifest.rows.iter().enumerate() {
        if row.runner_id != EXPECTED_RUNNERS[index]
            || row.snapshot_provider.trim().is_empty()
            || row.snapshot_image_sha256 == [0; 32]
            || row.executable_sha256 == [0; 32]
        {
            return Err(CandidateError::Invalid);
        }
    }
    Ok(VerifiedCandidateManifest {
        sha256: Sha256::digest(bytes).into(),
        rows: manifest.rows,
    })
}

impl VerifiedCandidateManifest {
    pub(crate) fn expectation(
        &self,
        runner_id: &str,
    ) -> Result<CandidateSnapshotExpectation, CandidateError> {
        let row = self
            .rows
            .iter()
            .find(|row| row.runner_id == runner_id)
            .ok_or(CandidateError::Invalid)?;
        Ok(CandidateSnapshotExpectation {
            runner_id: row.runner_id.clone(),
            snapshot_id: format!(
                "fm-{}-pristine-{}",
                row.runner_id
                    .strip_prefix("fm-")
                    .and_then(|value| value.strip_suffix("-v1"))
                    .ok_or(CandidateError::Invalid)?,
                &hex::encode(self.sha256)[..12]
            ),
            snapshot_provider: row.snapshot_provider.clone(),
            snapshot_image_sha256: row.snapshot_image_sha256,
            executable_sha256: row.executable_sha256,
        })
    }
}
