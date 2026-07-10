//! Fresh current-runner verification and five-second single-use capability.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::candidate::VerifiedCandidateManifest;

use super::RunnerError;
use super::config::{ProductionRunnerConfig, ProvisionedRunnerConfig, RUNNERS, production_config};
use super::protocol::{ProbePurpose, ProbeRequestV1, RunnerIdentityV1};
use super::transaction::CommittedRunnerLock;
use super::transport::{LauncherTransport, ProductionLauncherTransport};
use super::verification::verify_signed_probe;

pub(crate) struct VerifiedRunner {
    pub(crate) runner_id: String,
    pub(crate) matrix_run_id: [u8; 32],
    pub(crate) identity: RunnerIdentityV1,
    pub(crate) lock_sha256: [u8; 32],
    pub(crate) manifest_sha256: [u8; 32],
    pub(crate) snapshot_id: String,
    pub(crate) snapshot_provider: String,
    pub(crate) snapshot_image_sha256: [u8; 32],
    pub(crate) executable_sha256: [u8; 32],
    pub(crate) boot_id_sha256: [u8; 32],
    pub(crate) graphical_session_id_sha256: [u8; 32],
    pub(crate) expires_at: Instant,
}

pub(crate) fn verify_current_runner_with<T: LauncherTransport>(
    committed: &CommittedRunnerLock,
    manifest: &VerifiedCandidateManifest,
    runner_id: &str,
    config: &ProvisionedRunnerConfig,
    transport: &T,
) -> Result<VerifiedRunner, RunnerError> {
    let index = RUNNERS
        .iter()
        .position(|expected| *expected == runner_id)
        .ok_or(RunnerError::RunnerSet)?;
    let expectation = manifest
        .expectation(runner_id)
        .map_err(|error| RunnerError::Protocol(error.to_string()))?;
    let row = &config.dispatch[index];
    let issued_at_unix_ms: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RunnerError::Protocol("system clock precedes Unix epoch".into()))?
        .as_millis()
        .try_into()
        .map_err(|_| RunnerError::Protocol("system clock exceeds u64 milliseconds".into()))?;
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge)
        .map_err(|error| RunnerError::Protocol(format!("OS CSPRNG failed: {error}")))?;
    if challenge == [0; 32] {
        return Err(RunnerError::Protocol(
            "OS CSPRNG returned all-zero challenge".into(),
        ));
    }
    let request = ProbeRequestV1 {
        purpose: ProbePurpose::PreSpawn,
        run_id: committed.matrix_run_id(),
        runner_id: expectation.runner_id.clone(),
        challenge,
        issued_at_unix_ms,
        not_after_unix_ms: issued_at_unix_ms
            .checked_add(30_000)
            .ok_or_else(|| RunnerError::Protocol("deadline overflow".into()))?,
        expected_snapshot_id: expectation.snapshot_id.clone(),
        expected_snapshot_provider: expectation.snapshot_provider.clone(),
        expected_image_sha256: expectation.snapshot_image_sha256,
        expected_probe_sha256: row.probe_sha256,
        enrollment_commitment: None,
        final_lock_sha256: Some(committed.lock_sha256()),
        candidate_manifest_sha256: Some(manifest.sha256),
    };
    let started = Instant::now();
    let receipt = transport.exchange(row, &request)?;
    if started.elapsed() > Duration::from_secs(30) {
        return Err(RunnerError::Protocol(
            "pre-spawn exchange exceeded timeout".into(),
        ));
    }
    let payload = verify_signed_probe(&receipt, config.roots[index].public_key)?;
    let enrolled = committed.identity(index);
    let earliest = issued_at_unix_ms.saturating_sub(5_000);
    let latest = request.not_after_unix_ms.saturating_add(5_000);
    if payload.request != request
        || payload.captured_at_unix_ms < earliest
        || payload.captured_at_unix_ms > latest
        || payload.elapsed_ms > 30_000
        || payload.measured_probe_sha256 != row.probe_sha256
        || payload.identity != *enrolled
        || payload.identity.snapshot_provider != expectation.snapshot_provider
        || payload.boot_id_sha256 == [0; 32]
        || payload.graphical_session_id_sha256 == [0; 32]
    {
        return Err(RunnerError::Protocol(
            "fresh current-runner binding mismatch".into(),
        ));
    }
    Ok(VerifiedRunner {
        runner_id: runner_id.into(),
        matrix_run_id: committed.matrix_run_id(),
        identity: payload.identity,
        lock_sha256: committed.lock_sha256(),
        manifest_sha256: manifest.sha256,
        snapshot_id: expectation.snapshot_id,
        snapshot_provider: expectation.snapshot_provider,
        snapshot_image_sha256: expectation.snapshot_image_sha256,
        executable_sha256: expectation.executable_sha256,
        boot_id_sha256: payload.boot_id_sha256,
        graphical_session_id_sha256: payload.graphical_session_id_sha256,
        expires_at: Instant::now() + Duration::from_secs(5),
    })
}

pub(crate) fn recheck_current_session(capability: &VerifiedRunner) -> Result<(), RunnerError> {
    let config = match production_config() {
        ProductionRunnerConfig::Unprovisioned => return Err(RunnerError::UnprovisionedTrust),
        ProductionRunnerConfig::Provisioned(config) => config,
    };
    let index = RUNNERS
        .iter()
        .position(|expected| *expected == capability.runner_id)
        .ok_or(RunnerError::RunnerSet)?;
    let issued_at_unix_ms: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RunnerError::Protocol("system clock precedes Unix epoch".into()))?
        .as_millis()
        .try_into()
        .map_err(|_| RunnerError::Protocol("system clock exceeds u64 milliseconds".into()))?;
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge)
        .map_err(|error| RunnerError::Protocol(format!("OS CSPRNG failed: {error}")))?;
    if challenge == [0; 32] {
        return Err(RunnerError::Protocol(
            "OS CSPRNG returned all-zero challenge".into(),
        ));
    }
    let row = &config.dispatch[index];
    let request = ProbeRequestV1 {
        purpose: ProbePurpose::PreSpawn,
        run_id: capability.matrix_run_id,
        runner_id: capability.runner_id.clone(),
        challenge,
        issued_at_unix_ms,
        not_after_unix_ms: issued_at_unix_ms
            .checked_add(30_000)
            .ok_or_else(|| RunnerError::Protocol("deadline overflow".into()))?,
        expected_snapshot_id: capability.snapshot_id.clone(),
        expected_snapshot_provider: capability.snapshot_provider.clone(),
        expected_image_sha256: capability.snapshot_image_sha256,
        expected_probe_sha256: row.probe_sha256,
        enrollment_commitment: None,
        final_lock_sha256: Some(capability.lock_sha256),
        candidate_manifest_sha256: Some(capability.manifest_sha256),
    };
    let receipt = ProductionLauncherTransport.exchange(row, &request)?;
    let payload = verify_signed_probe(&receipt, config.roots[index].public_key)?;
    if payload.request != request
        || payload.identity != capability.identity
        || payload.boot_id_sha256 != capability.boot_id_sha256
        || payload.graphical_session_id_sha256 != capability.graphical_session_id_sha256
    {
        return Err(RunnerError::Protocol(
            "boot/session changed before capability consumption".into(),
        ));
    }
    Ok(())
}
