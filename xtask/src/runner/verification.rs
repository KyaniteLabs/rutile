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

const PROBE_DOMAIN: &[u8] = b"Rutile Runner Probe\0v1\0";
const COMMITMENT_DOMAIN: &[u8] = b"Rutile Runner Enrollment Commitment\0v1\0";

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
    if lock.schema != "rutile.runner-lock.v1"
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
    validate_identity_contract(
        &payload.identity,
        row.runner_id,
        row.snapshot_provider,
        &row.identity,
    )?;
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

pub(crate) fn validate_identity_contract(
    identity: &RunnerIdentityV1,
    runner_id: &str,
    snapshot_provider: &str,
    expected: &super::config::PinnedRunnerIdentityConfig,
) -> Result<(), RunnerError> {
    let exact = identity.runner_id == runner_id
        && identity.machine_id_sha256 == expected.machine_id_sha256
        && identity.hardware_model == expected.hardware_model
        && identity.cpu_model == expected.cpu_model
        && identity.cpu_cores == expected.cpu_cores
        && identity.ram_bytes == expected.ram_bytes
        && identity.arch == expected.arch
        && identity.os_product == expected.os_product
        && identity.os_version == expected.os_version
        && identity.os_build == expected.os_build
        && identity.os_image == expected.os_image
        && identity.kernel == expected.kernel
        && identity.display_session == expected.display_session
        && identity.display_socket.as_deref() == expected.display_socket
        && identity.monitor_width_px == expected.monitor_width_px
        && identity.monitor_height_px == expected.monitor_height_px
        && identity.monitor_scale_milli == expected.monitor_scale_milli
        && identity.monitor_refresh_millihz == expected.monitor_refresh_millihz
        && identity.gtk_version.as_deref() == expected.gtk_version
        && identity.webkitgtk_version.as_deref() == expected.webkitgtk_version
        && identity.wkwebview_version.as_deref() == expected.wkwebview_version
        && identity.virtualized == expected.virtualized
        && identity.virtualization_image_sha256 == expected.virtualization_image_sha256
        && identity.snapshot_provider == snapshot_provider;
    if exact {
        Ok(())
    } else {
        Err(protocol(
            "identity does not exactly match the independently provisioned runner row",
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::config::PinnedRunnerIdentityConfig;

    fn identity(runner_id: &str) -> RunnerIdentityV1 {
        let mac = runner_id.starts_with("fm-macos-");
        RunnerIdentityV1 {
            runner_id: runner_id.into(),
            machine_id_sha256: [1; 32],
            hardware_model: if mac {
                "MacBookPro16,1"
            } else {
                "Reference PC"
            }
            .into(),
            cpu_model: match runner_id {
                "fm-macos-arm64-v1" => "Apple M1",
                "fm-macos-x86_64-v1" => "Intel(R) Core(TM) i7-9750H CPU",
                _ => "Intel(R) Core(TM) i5-8500 CPU",
            }
            .into(),
            cpu_cores: if runner_id == "fm-macos-arm64-v1" {
                8
            } else {
                6
            },
            ram_bytes: 16 * 1024 * 1024 * 1024,
            arch: if runner_id == "fm-macos-arm64-v1" {
                "aarch64"
            } else {
                "x86_64"
            }
            .into(),
            os_product: if mac {
                "macOS"
            } else if runner_id == "fm-fedora-wayland-v1" {
                "Fedora Linux"
            } else {
                "Ubuntu"
            }
            .into(),
            os_version: if mac {
                "15.5"
            } else if runner_id == "fm-fedora-wayland-v1" {
                "43"
            } else {
                "24.04"
            }
            .into(),
            os_build: "exact-build".into(),
            os_image: "exact-image".into(),
            kernel: if mac { "Darwin 24.5.0" } else { "Linux 6.8.0" }.into(),
            display_session: if mac {
                "aqua"
            } else if runner_id == "fm-ubuntu-x11-v1" {
                "x11"
            } else {
                "wayland"
            }
            .into(),
            display_socket: (!mac).then(|| {
                if runner_id == "fm-ubuntu-x11-v1" {
                    ":0"
                } else {
                    "wayland-0"
                }
                .into()
            }),
            monitor_width_px: if runner_id == "fm-macos-arm64-v1" {
                2560
            } else {
                1920
            },
            monitor_height_px: if runner_id == "fm-macos-arm64-v1" {
                1600
            } else {
                1080
            },
            monitor_scale_milli: 1000,
            monitor_refresh_millihz: 60_000,
            gtk_version: (!mac).then(|| "3.24.41".into()),
            webkitgtk_version: (!mac).then(|| "2.44.3".into()),
            wkwebview_version: mac.then(|| "620.2.4".into()),
            virtualized: true,
            virtualization_image_sha256: Some([2; 32]),
            snapshot_provider: "provider".into(),
        }
    }

    fn pinned(runner_id: &str) -> PinnedRunnerIdentityConfig {
        let mac = runner_id.starts_with("fm-macos-");
        PinnedRunnerIdentityConfig {
            machine_id_sha256: [1; 32],
            hardware_model: if mac {
                "MacBookPro16,1"
            } else {
                "Reference PC"
            },
            cpu_model: match runner_id {
                "fm-macos-arm64-v1" => "Apple M1",
                "fm-macos-x86_64-v1" => "Intel(R) Core(TM) i7-9750H CPU",
                _ => "Intel(R) Core(TM) i5-8500 CPU",
            },
            cpu_cores: if runner_id == "fm-macos-arm64-v1" {
                8
            } else {
                6
            },
            ram_bytes: 16 * 1024 * 1024 * 1024,
            arch: if runner_id == "fm-macos-arm64-v1" {
                "aarch64"
            } else {
                "x86_64"
            },
            os_product: if mac {
                "macOS"
            } else if runner_id == "fm-fedora-wayland-v1" {
                "Fedora Linux"
            } else {
                "Ubuntu"
            },
            os_version: if mac {
                "15.5"
            } else if runner_id == "fm-fedora-wayland-v1" {
                "43"
            } else {
                "24.04"
            },
            os_build: "exact-build",
            os_image: "exact-image",
            kernel: if mac { "Darwin 24.5.0" } else { "Linux 6.8.0" },
            display_session: if mac {
                "aqua"
            } else if runner_id == "fm-ubuntu-x11-v1" {
                "x11"
            } else {
                "wayland"
            },
            display_socket: (!mac).then_some(if runner_id == "fm-ubuntu-x11-v1" {
                ":0"
            } else {
                "wayland-0"
            }),
            monitor_width_px: if runner_id == "fm-macos-arm64-v1" {
                2560
            } else {
                1920
            },
            monitor_height_px: if runner_id == "fm-macos-arm64-v1" {
                1600
            } else {
                1080
            },
            monitor_scale_milli: 1000,
            monitor_refresh_millihz: 60_000,
            gtk_version: (!mac).then_some("3.24.41"),
            webkitgtk_version: (!mac).then_some("2.44.3"),
            wkwebview_version: mac.then_some("620.2.4"),
            virtualized: true,
            virtualization_image_sha256: Some([2; 32]),
        }
    }

    fn validate(identity: &RunnerIdentityV1) -> Result<(), RunnerError> {
        validate_identity_contract(
            identity,
            &identity.runner_id,
            "provider",
            &pinned(&identity.runner_id),
        )
    }

    #[test]
    fn exact_runner_contract_rejects_wrong_platform_family_and_values() {
        for runner in RUNNERS {
            assert!(validate(&identity(runner)).is_ok());
        }
        let mut wrong_family = identity("fm-ubuntu-wayland-v1");
        wrong_family.os_product = "Fedora Linux".into();
        assert!(validate(&wrong_family).is_err());

        let mut wrong_runtime = identity("fm-fedora-wayland-v1");
        wrong_runtime.webkitgtk_version = Some("6.0.0".into());
        assert!(validate(&wrong_runtime).is_err());

        let mut wrong_socket_shape = identity("fm-ubuntu-x11-v1");
        wrong_socket_shape.display_socket = Some("wayland-0".into());
        assert!(validate(&wrong_socket_shape).is_err());
    }

    #[test]
    fn macos_contract_requires_distinct_real_wkwebview_runtime() {
        let mut mac = identity("fm-macos-arm64-v1");
        mac.wkwebview_version = Some(mac.os_build.clone());
        assert!(validate(&mac).is_err());
        mac.wkwebview_version = None;
        assert!(validate(&mac).is_err());
    }

    #[test]
    fn every_identity_field_is_exact_for_every_runner_row() {
        for runner in RUNNERS {
            let expected = identity(runner);
            let mut mutations = Vec::new();
            macro_rules! mutate {
                ($field:ident, $value:expr) => {{
                    let mut changed = expected.clone();
                    changed.$field = $value;
                    mutations.push((stringify!($field), changed));
                }};
            }
            mutate!(runner_id, format!("{runner}-changed"));
            mutate!(machine_id_sha256, [9; 32]);
            mutate!(hardware_model, "changed-hardware".into());
            mutate!(cpu_model, "changed-cpu".into());
            mutate!(cpu_cores, expected.cpu_cores + 1);
            mutate!(ram_bytes, expected.ram_bytes - 1);
            mutate!(arch, "changed-arch".into());
            mutate!(os_product, "changed-product".into());
            mutate!(os_version, "changed-version".into());
            mutate!(os_build, "changed-build".into());
            mutate!(os_image, "changed-image".into());
            mutate!(kernel, "changed-kernel".into());
            mutate!(display_session, "changed-session".into());
            mutate!(
                display_socket,
                expected
                    .display_socket
                    .as_ref()
                    .map_or_else(|| Some("unexpected".into()), |_| None)
            );
            mutate!(monitor_width_px, expected.monitor_width_px + 1);
            mutate!(monitor_height_px, expected.monitor_height_px + 1);
            mutate!(monitor_scale_milli, expected.monitor_scale_milli + 1);
            mutate!(
                monitor_refresh_millihz,
                expected.monitor_refresh_millihz + 1
            );
            mutate!(
                gtk_version,
                expected
                    .gtk_version
                    .as_ref()
                    .map_or_else(|| Some("unexpected".into()), |_| None)
            );
            mutate!(
                webkitgtk_version,
                expected
                    .webkitgtk_version
                    .as_ref()
                    .map_or_else(|| Some("unexpected".into()), |_| None)
            );
            mutate!(
                wkwebview_version,
                expected
                    .wkwebview_version
                    .as_ref()
                    .map_or_else(|| Some("unexpected".into()), |_| None)
            );
            mutate!(virtualized, !expected.virtualized);
            mutate!(virtualization_image_sha256, Some([8; 32]));
            mutate!(snapshot_provider, "changed-provider".into());

            for (field, changed) in mutations {
                assert!(
                    validate_identity_contract(&changed, runner, "provider", &pinned(runner))
                        .is_err(),
                    "{runner} accepted mutated {field}"
                );
            }
        }
    }
}
