use std::array;

use ed25519_dalek::{Signer, SigningKey};

use super::config::{ProvisionedRunnerConfig, RUNNERS, RunnerDispatchConfig, TrustRootConfig};
use super::encoding::encode_probe_payload;
use super::protocol::{
    ProbeExchangeV1, ProbePayloadV1, ProbePurpose, ProbeRequestV1, RunnerIdentityV1, RunnerLockV1,
    SignedRunnerProbeV1,
};
use super::verification::{enrollment_commitment, probe_signature_message};

pub(crate) struct LockFixture {
    pub config: ProvisionedRunnerConfig,
    pub bytes: Vec<u8>,
}

pub(crate) struct FakeLauncherTransport {
    pub(crate) config: ProvisionedRunnerConfig,
    signing_keys: [SigningKey; 5],
}

impl FakeLauncherTransport {
    pub(crate) fn exchange_impl(
        &self,
        request: &ProbeRequestV1,
    ) -> Result<SignedRunnerProbeV1, super::RunnerError> {
        let index = RUNNERS
            .iter()
            .position(|runner| *runner == request.runner_id)
            .ok_or(super::RunnerError::RunnerSet)?;
        Ok(exchange(
            request.clone(),
            identity(index),
            &self.signing_keys[index],
            index,
        )
        .receipt)
    }
}

pub(crate) fn fake_transport() -> FakeLauncherTransport {
    let (config, signing_keys) = provisioned_test_material();
    FakeLauncherTransport {
        config,
        signing_keys,
    }
}

pub(crate) fn build_valid_lock() -> LockFixture {
    let (config, signing_keys) = provisioned_test_material();
    let run_id = [73; 32];
    let mut enrollment_exchanges = Vec::new();
    let mut identities = Vec::new();
    for (index, signing_key) in signing_keys.iter().enumerate() {
        let request = request(
            &config,
            index,
            ProbePurpose::Enroll,
            run_id,
            [index as u8 + 80; 32],
            None,
        );
        let identity = identity(index);
        enrollment_exchanges.push(exchange(request, identity.clone(), signing_key, index));
        identities.push(identity);
    }
    let mut lock = RunnerLockV1 {
        schema: "feathermark.runner-lock.v1".into(),
        runner_ids: RUNNERS.iter().map(|id| (*id).into()).collect(),
        trust_manifest_sha256: config.trust_manifest_sha256,
        dispatch_manifest_sha256: config.dispatch_manifest_sha256,
        matrix_run_id: run_id,
        enrollment_exchanges,
        identities,
        enrollment_commitment: [0; 32],
        post_lock_exchanges: Vec::new(),
    };
    lock.enrollment_commitment = enrollment_commitment(&lock);
    for (index, signing_key) in signing_keys.iter().enumerate() {
        let request = request(
            &config,
            index,
            ProbePurpose::PostLock,
            run_id,
            [index as u8 + 90; 32],
            Some(lock.enrollment_commitment),
        );
        lock.post_lock_exchanges.push(exchange(
            request,
            lock.identities[index].clone(),
            signing_key,
            index,
        ));
    }
    let mut bytes = serde_json::to_vec_pretty(&lock).unwrap();
    bytes.push(b'\n');
    LockFixture { config, bytes }
}

fn provisioned_test_material() -> (ProvisionedRunnerConfig, [SigningKey; 5]) {
    let signing_keys: [SigningKey; 5] =
        array::from_fn(|index| SigningKey::from_bytes(&[index as u8 + 1; 32]));
    let roots = array::from_fn(|index| TrustRootConfig {
        runner_id: RUNNERS[index],
        key_id: TEST_KEY_IDS[index],
        public_key: signing_keys[index].verifying_key().to_bytes(),
    });
    let dispatch = array::from_fn(|index| RunnerDispatchConfig {
        runner_id: RUNNERS[index],
        endpoint: TEST_ENDPOINTS[index],
        transport_fingerprint: [index as u8 + 20; 32],
        launcher_protocol_version: 1,
        probe_path: TEST_PROBE_PATHS[index],
        probe_sha256: [index as u8 + 30; 32],
        enrollment_snapshot_id: TEST_SNAPSHOTS[index],
        snapshot_provider: "test-sealed-snapshot-provider",
        enrollment_image_sha256: [index as u8 + 40; 32],
        macos_designated_requirement: (index < 2).then_some("anchor apple generic"),
        macos_cdhash: (index < 2).then_some("00112233445566778899aabbccddeeff00112233"),
    });
    let config = ProvisionedRunnerConfig {
        trust_manifest_sha256: [71; 32],
        dispatch_manifest_sha256: [72; 32],
        roots,
        dispatch,
    };
    (config, signing_keys)
}

fn request(
    config: &ProvisionedRunnerConfig,
    index: usize,
    purpose: ProbePurpose,
    run_id: [u8; 32],
    challenge: [u8; 32],
    commitment: Option<[u8; 32]>,
) -> ProbeRequestV1 {
    let row = &config.dispatch[index];
    ProbeRequestV1 {
        purpose,
        run_id,
        runner_id: row.runner_id.into(),
        challenge,
        issued_at_unix_ms: 1_000,
        not_after_unix_ms: 31_000,
        expected_snapshot_id: row.enrollment_snapshot_id.into(),
        expected_snapshot_provider: row.snapshot_provider.into(),
        expected_image_sha256: row.enrollment_image_sha256,
        expected_probe_sha256: row.probe_sha256,
        enrollment_commitment: commitment,
        final_lock_sha256: None,
        candidate_manifest_sha256: None,
    }
}

fn exchange(
    request: ProbeRequestV1,
    identity: RunnerIdentityV1,
    signing_key: &SigningKey,
    index: usize,
) -> ProbeExchangeV1 {
    let payload = ProbePayloadV1 {
        request: request.clone(),
        identity,
        boot_id_sha256: [index as u8 + 100; 32],
        graphical_session_id_sha256: [index as u8 + 110; 32],
        captured_at_unix_ms: request.issued_at_unix_ms + 100,
        elapsed_ms: 10,
        launcher_protocol_version: 1,
        measured_probe_sha256: request.expected_probe_sha256,
    };
    let payload_cbor = encode_probe_payload(&payload);
    let signature = signing_key.sign(&probe_signature_message(&payload_cbor).unwrap());
    ProbeExchangeV1 {
        request,
        receipt: SignedRunnerProbeV1 {
            payload_cbor_hex: hex::encode(payload_cbor),
            signature_hex: hex::encode(signature.to_bytes()),
        },
    }
}

fn identity(index: usize) -> RunnerIdentityV1 {
    let mac = index < 2;
    RunnerIdentityV1 {
        runner_id: RUNNERS[index].into(),
        machine_id_sha256: [index as u8 + 120; 32],
        hardware_model: if mac { "Mac" } else { "Reference PC" }.into(),
        cpu_model: if index == 0 {
            "Apple M1"
        } else if index == 1 {
            "Intel Core i7-9750H"
        } else {
            "Intel Core i5-8500"
        }
        .into(),
        cpu_cores: if index == 0 { 8 } else { 6 },
        ram_bytes: 16 * 1024 * 1024 * 1024,
        arch: if index == 0 { "aarch64" } else { "x86_64" }.into(),
        os_product: if mac {
            "macOS"
        } else if index == 4 {
            "Fedora"
        } else {
            "Ubuntu"
        }
        .into(),
        os_version: if mac {
            "15.5"
        } else if index == 4 {
            "43"
        } else {
            "24.04"
        }
        .into(),
        os_build: "exact-build".into(),
        os_image: "exact-image".into(),
        kernel: "exact-kernel".into(),
        display_session: if mac {
            "native"
        } else if index == 2 {
            "x11"
        } else {
            "wayland"
        }
        .into(),
        display_socket: (!mac).then(|| if index == 2 { ":0" } else { "wayland-0" }.into()),
        monitor_width_px: if index == 0 { 2560 } else { 1920 },
        monitor_height_px: if index == 0 { 1600 } else { 1080 },
        monitor_scale_milli: 1000,
        monitor_refresh_millihz: 60_000,
        gtk_version: (!mac).then(|| "3.24.41".into()),
        webkitgtk_version: (!mac).then(|| "2.44.3".into()),
        wkwebview_version: mac.then(|| "620.2.4".into()),
        virtualized: true,
        virtualization_image_sha256: Some([index as u8 + 125; 32]),
        snapshot_provider: "test-sealed-snapshot-provider".into(),
    }
}

const TEST_KEY_IDS: [&str; 5] = ["key-0", "key-1", "key-2", "key-3", "key-4"];
const TEST_ENDPOINTS: [&str; 5] = ["sealed-0", "sealed-1", "sealed-2", "sealed-3", "sealed-4"];
const TEST_SNAPSHOTS: [&str; 5] = ["snap-0", "snap-1", "snap-2", "snap-3", "snap-4"];
const TEST_PROBE_PATHS: [&str; 5] = ["/probe-0", "/probe-1", "/probe-2", "/probe-3", "/probe-4"];
