use std::collections::BTreeSet;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use super::RunnerError;
use super::config::{ProvisionedRunnerConfig, RUNNERS};
use super::encoding::{
    array, bytes, decode_probe_payload, encode_identity, encode_probe_request, text, uint,
};
use super::protocol::{
    ProbeExchangeV1, ProbePayloadV1, ProbePurpose, RunnerIdentityV1, RunnerLockV1,
    SignedRunnerProbeV1,
};

const PROBE_DOMAIN: &[u8] = b"FeatherMark Runner Probe\0v1\0";
const COMMITMENT_DOMAIN: &[u8] = b"FeatherMark Runner Enrollment Commitment\0v1\0";

#[derive(Clone, Debug)]
pub(crate) struct VerifiedRunnerLock {
    pub lock_sha256: [u8; 32],
    pub matrix_run_id: [u8; 32],
    pub identities: Vec<RunnerIdentityV1>,
}

pub(crate) fn verify_runner_lock_bytes_with(
    bytes: &[u8],
    config: &ProvisionedRunnerConfig,
) -> Result<VerifiedRunnerLock, RunnerError> {
    let lock: RunnerLockV1 = serde_json::from_slice(bytes)?;
    if lock.schema != "feathermark.runner-lock.v1"
        || lock.runner_ids.iter().map(String::as_str).ne(RUNNERS)
        || lock.trust_manifest_sha256 != config.trust_manifest_sha256
        || lock.dispatch_manifest_sha256 != config.dispatch_manifest_sha256
        || lock.enrollment_exchanges.len() != 5
        || lock.identities.len() != 5
        || lock.post_lock_exchanges.len() != 5
        || lock.matrix_run_id == [0; 32]
    {
        return Err(protocol("lock header or closed row count mismatch"));
    }

    let mut challenges = BTreeSet::new();
    let mut enrollment_payloads = Vec::with_capacity(5);
    for index in 0..5 {
        let payload = verify_exchange(
            &lock.enrollment_exchanges[index],
            ProbePurpose::Enroll,
            index,
            &lock,
            config,
            &mut challenges,
        )?;
        if payload.identity != lock.identities[index]
            || payload.request.enrollment_commitment.is_some()
            || payload.request.final_lock_sha256.is_some()
            || payload.request.candidate_manifest_sha256.is_some()
        {
            return Err(protocol("enrollment binding mismatch"));
        }
        enrollment_payloads.push(payload);
    }
    let commitment = enrollment_commitment(&lock);
    if commitment != lock.enrollment_commitment {
        return Err(protocol("enrollment commitment mismatch"));
    }
    for (index, enrolled) in enrollment_payloads.iter().enumerate() {
        let payload = verify_exchange(
            &lock.post_lock_exchanges[index],
            ProbePurpose::PostLock,
            index,
            &lock,
            config,
            &mut challenges,
        )?;
        if payload.request.enrollment_commitment != Some(commitment)
            || payload.request.final_lock_sha256.is_some()
            || payload.request.candidate_manifest_sha256.is_some()
            || payload.identity != enrolled.identity
            || payload.boot_id_sha256 != enrolled.boot_id_sha256
            || payload.graphical_session_id_sha256 != enrolled.graphical_session_id_sha256
        {
            return Err(protocol("post-lock binding mismatch"));
        }
    }
    Ok(VerifiedRunnerLock {
        lock_sha256: Sha256::digest(bytes).into(),
        matrix_run_id: lock.matrix_run_id,
        identities: lock.identities,
    })
}

fn verify_exchange(
    exchange: &ProbeExchangeV1,
    purpose: ProbePurpose,
    index: usize,
    lock: &RunnerLockV1,
    config: &ProvisionedRunnerConfig,
    challenges: &mut BTreeSet<[u8; 32]>,
) -> Result<ProbePayloadV1, RunnerError> {
    let request = &exchange.request;
    let row = &config.dispatch[index];
    let root = &config.roots[index];
    if request.purpose != purpose
        || request.run_id != lock.matrix_run_id
        || request.runner_id != RUNNERS[index]
        || row.runner_id != RUNNERS[index]
        || root.runner_id != RUNNERS[index]
        || request.challenge == [0; 32]
        || !challenges.insert(request.challenge)
        || request.not_after_unix_ms
            != request
                .issued_at_unix_ms
                .checked_add(30_000)
                .ok_or_else(|| protocol("deadline overflow"))?
        || request.expected_snapshot_id != row.enrollment_snapshot_id
        || request.expected_snapshot_provider != row.snapshot_provider
        || request.expected_image_sha256 != row.enrollment_image_sha256
        || request.expected_probe_sha256 != row.probe_sha256
    {
        return Err(protocol("request binding mismatch"));
    }
    let payload = verify_signed_probe(&exchange.receipt, root.public_key)?;
    let earliest = request.issued_at_unix_ms.saturating_sub(5_000);
    let latest = request.not_after_unix_ms.saturating_add(5_000);
    if payload.request != *request
        || payload.captured_at_unix_ms < earliest
        || payload.captured_at_unix_ms > latest
        || payload.elapsed_ms > 30_000
        || payload.launcher_protocol_version != row.launcher_protocol_version
        || payload.measured_probe_sha256 != row.probe_sha256
        || payload.boot_id_sha256 == [0; 32]
        || payload.graphical_session_id_sha256 == [0; 32]
        || payload.identity.runner_id != request.runner_id
        || payload.identity.snapshot_provider != request.expected_snapshot_provider
    {
        return Err(protocol("signed payload mismatch"));
    }
    validate_identity_contract(&payload.identity)?;
    Ok(payload)
}

pub(crate) fn verify_signed_probe(
    signed: &SignedRunnerProbeV1,
    public_key: [u8; 32],
) -> Result<ProbePayloadV1, RunnerError> {
    if !lower_hex(&signed.payload_cbor_hex) || !lower_hex(&signed.signature_hex) {
        return Err(protocol("signed probe hex is not lowercase canonical"));
    }
    let payload_bytes =
        hex::decode(&signed.payload_cbor_hex).map_err(|_| protocol("invalid payload hex"))?;
    let payload = decode_probe_payload(&payload_bytes)?;
    let signature_bytes: [u8; 64] = hex::decode(&signed.signature_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| protocol("invalid signature hex"))?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| protocol("invalid trust root"))?;
    verifying_key
        .verify(
            &probe_signature_message(&payload_bytes)?,
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| protocol("signature verification failed"))?;
    Ok(payload)
}

pub(crate) fn probe_signature_message(payload_cbor: &[u8]) -> Result<Vec<u8>, RunnerError> {
    let len: u32 = payload_cbor
        .len()
        .try_into()
        .map_err(|_| protocol("probe payload too large"))?;
    let mut message = Vec::with_capacity(PROBE_DOMAIN.len() + 4 + payload_cbor.len());
    message.extend_from_slice(PROBE_DOMAIN);
    message.extend_from_slice(&len.to_be_bytes());
    message.extend_from_slice(payload_cbor);
    Ok(message)
}

pub(crate) fn enrollment_commitment(lock: &RunnerLockV1) -> [u8; 32] {
    let mut body = Vec::new();
    array(&mut body, 7);
    uint(&mut body, 1);
    array(&mut body, lock.runner_ids.len());
    for runner in &lock.runner_ids {
        text(&mut body, runner);
    }
    bytes(&mut body, &lock.trust_manifest_sha256);
    bytes(&mut body, &lock.dispatch_manifest_sha256);
    bytes(&mut body, &lock.matrix_run_id);
    array(&mut body, lock.enrollment_exchanges.len());
    for exchange in &lock.enrollment_exchanges {
        array(&mut body, 3);
        body.extend_from_slice(&encode_probe_request(&exchange.request));
        text(&mut body, &exchange.receipt.payload_cbor_hex);
        text(&mut body, &exchange.receipt.signature_hex);
    }
    array(&mut body, lock.identities.len());
    for identity in &lock.identities {
        body.extend_from_slice(&encode_identity(identity));
    }
    let mut message = Vec::with_capacity(COMMITMENT_DOMAIN.len() + 8 + body.len());
    message.extend_from_slice(COMMITMENT_DOMAIN);
    message.extend_from_slice(&(body.len() as u64).to_be_bytes());
    message.extend_from_slice(&body);
    Sha256::digest(message).into()
}

fn validate_identity_contract(identity: &RunnerIdentityV1) -> Result<(), RunnerError> {
    let nonempty = [
        identity.hardware_model.as_str(),
        identity.cpu_model.as_str(),
        identity.arch.as_str(),
        identity.os_product.as_str(),
        identity.os_version.as_str(),
        identity.os_build.as_str(),
        identity.os_image.as_str(),
        identity.kernel.as_str(),
        identity.display_session.as_str(),
        identity.snapshot_provider.as_str(),
    ]
    .into_iter()
    .all(|value| !value.is_empty() && !value.contains('+'));
    if !nonempty
        || identity.machine_id_sha256 == [0; 32]
        || identity.cpu_cores == 0
        || identity.ram_bytes == 0
        || identity.monitor_width_px == 0
        || identity.monitor_height_px == 0
        || identity.monitor_scale_milli == 0
        || identity.monitor_refresh_millihz == 0
        || identity.virtualized != identity.virtualization_image_sha256.is_some()
    {
        return Err(protocol("identity contains an empty/presence-only field"));
    }
    Ok(())
}

fn lower_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn protocol(message: &str) -> RunnerError {
    RunnerError::Protocol(message.into())
}
