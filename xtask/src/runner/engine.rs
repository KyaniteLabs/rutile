//! Ten-exchange coordinator engine shared only with private test fakes.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::config::{ProvisionedRunnerConfig, RUNNERS};
use super::protocol::{ProbeExchangeV1, ProbePurpose, ProbeRequestV1, RunnerLockV1};
use super::transaction::{FailurePoint, publish_runner_lock_with};
use super::transport::LauncherTransport;
use super::verification::{enrollment_commitment, verify_signed_probe};
use super::{CaptureSummary, RunnerError};

pub(crate) fn capture_with<T: LauncherTransport>(
    config: &ProvisionedRunnerConfig,
    transport: &T,
    capture_dir: &Path,
    out: &Path,
) -> Result<CaptureSummary, RunnerError> {
    let run_id = random_hash()?;
    let mut challenges = Vec::with_capacity(10);
    let mut enrollment_exchanges = Vec::with_capacity(5);
    let mut identities = Vec::with_capacity(5);
    let mut enrollment_payloads = Vec::with_capacity(5);
    for index in 0..5 {
        let request = request(
            config,
            index,
            ProbePurpose::Enroll,
            run_id,
            None,
            &mut challenges,
        )?;
        let exchange = dispatch(config, transport, index, request)?;
        let payload = verify_signed_probe(&exchange.receipt, config.roots[index].public_key)?;
        if payload.request != exchange.request {
            return Err(RunnerError::Protocol(
                "launcher did not echo enrollment request".into(),
            ));
        }
        identities.push(payload.identity.clone());
        enrollment_payloads.push(payload);
        enrollment_exchanges.push(exchange);
    }
    let mut lock = RunnerLockV1 {
        schema: "rutile.runner-lock.v1".into(),
        runner_ids: RUNNERS.iter().map(|value| (*value).into()).collect(),
        trust_manifest_sha256: config.trust_manifest_sha256,
        dispatch_manifest_sha256: config.dispatch_manifest_sha256,
        matrix_run_id: run_id,
        enrollment_exchanges,
        identities,
        enrollment_commitment: [0; 32],
        post_lock_exchanges: Vec::with_capacity(5),
    };
    lock.enrollment_commitment = enrollment_commitment(&lock);
    for (index, enrolled) in enrollment_payloads.iter().enumerate() {
        let request = request(
            config,
            index,
            ProbePurpose::PostLock,
            run_id,
            Some(lock.enrollment_commitment),
            &mut challenges,
        )?;
        let exchange = dispatch(config, transport, index, request)?;
        let payload = verify_signed_probe(&exchange.receipt, config.roots[index].public_key)?;
        if payload.request != exchange.request
            || payload.identity != enrolled.identity
            || payload.boot_id_sha256 != enrolled.boot_id_sha256
            || payload.graphical_session_id_sha256 != enrolled.graphical_session_id_sha256
        {
            return Err(RunnerError::Protocol(
                "post-lock receipt changed enrolled runner".into(),
            ));
        }
        lock.post_lock_exchanges.push(exchange);
    }

    let mut bytes = serde_json::to_vec_pretty(&lock)?;
    bytes.push(b'\n');
    write_diagnostic_receipts(capture_dir, &lock)?;
    publish_runner_lock_with(&bytes, out, config, FailurePoint::None)?;
    Ok(CaptureSummary { runners: 5 })
}

fn request(
    config: &ProvisionedRunnerConfig,
    index: usize,
    purpose: ProbePurpose,
    run_id: [u8; 32],
    commitment: Option<[u8; 32]>,
    previous_challenges: &mut Vec<[u8; 32]>,
) -> Result<ProbeRequestV1, RunnerError> {
    let challenge = loop {
        let candidate = random_hash()?;
        if candidate != [0; 32] && !previous_challenges.contains(&candidate) {
            break candidate;
        }
    };
    previous_challenges.push(challenge);
    let issued_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RunnerError::Protocol("system clock precedes Unix epoch".into()))?
        .as_millis()
        .try_into()
        .map_err(|_| RunnerError::Protocol("system clock exceeds u64 milliseconds".into()))?;
    let row = &config.dispatch[index];
    Ok(ProbeRequestV1 {
        purpose,
        run_id,
        runner_id: row.runner_id.into(),
        challenge,
        issued_at_unix_ms,
        not_after_unix_ms: issued_at_unix_ms
            .checked_add(30_000)
            .ok_or_else(|| RunnerError::Protocol("deadline overflow".into()))?,
        expected_snapshot_id: row.enrollment_snapshot_id.into(),
        expected_snapshot_provider: row.snapshot_provider.into(),
        expected_image_sha256: row.enrollment_image_sha256,
        expected_probe_sha256: row.probe_sha256,
        enrollment_commitment: commitment,
        final_lock_sha256: None,
        candidate_manifest_sha256: None,
    })
}

fn dispatch<T: LauncherTransport>(
    config: &ProvisionedRunnerConfig,
    transport: &T,
    index: usize,
    request: ProbeRequestV1,
) -> Result<ProbeExchangeV1, RunnerError> {
    let started = Instant::now();
    let receipt = transport.exchange(&config.dispatch[index], &request)?;
    if started.elapsed().as_millis() > 30_000 {
        return Err(RunnerError::Protocol(
            "launcher exchange exceeded monotonic timeout".into(),
        ));
    }
    Ok(ProbeExchangeV1 { request, receipt })
}

fn random_hash() -> Result<[u8; 32], RunnerError> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| RunnerError::Protocol(format!("OS CSPRNG failed: {error}")))?;
    if bytes == [0; 32] {
        return Err(RunnerError::Protocol(
            "OS CSPRNG returned an all-zero value".into(),
        ));
    }
    Ok(bytes)
}

fn write_diagnostic_receipts(capture_dir: &Path, lock: &RunnerLockV1) -> Result<(), RunnerError> {
    if capture_dir.exists() {
        let metadata = fs::symlink_metadata(capture_dir)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(RunnerError::Publication(
                "capture directory is not a real directory".into(),
            ));
        }
    } else {
        fs::create_dir(capture_dir)?;
    }
    for (phase, exchanges) in [
        ("enroll", &lock.enrollment_exchanges),
        ("post-lock", &lock.post_lock_exchanges),
    ] {
        for (index, exchange) in exchanges.iter().enumerate() {
            let path = capture_dir.join(format!("{phase}-{}.json", RUNNERS[index]));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            let mut bytes = serde_json::to_vec_pretty(exchange)?;
            bytes.push(b'\n');
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
    }
    std::fs::File::open(capture_dir)?.sync_all()?;
    Ok(())
}
