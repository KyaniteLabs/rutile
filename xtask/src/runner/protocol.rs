use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbePurpose {
    Enroll = 1,
    PostLock = 2,
    PreSpawn = 3,
}

impl TryFrom<u64> for ProbePurpose {
    type Error = ();

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Enroll),
            2 => Ok(Self::PostLock),
            3 => Ok(Self::PreSpawn),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeRequestV1 {
    pub purpose: ProbePurpose,
    pub run_id: [u8; 32],
    pub runner_id: String,
    pub challenge: [u8; 32],
    pub issued_at_unix_ms: u64,
    pub not_after_unix_ms: u64,
    pub expected_snapshot_id: String,
    pub expected_snapshot_provider: String,
    pub expected_image_sha256: [u8; 32],
    pub expected_probe_sha256: [u8; 32],
    pub enrollment_commitment: Option<[u8; 32]>,
    pub final_lock_sha256: Option<[u8; 32]>,
    pub candidate_manifest_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunnerIdentityV1 {
    pub runner_id: String,
    pub machine_id_sha256: [u8; 32],
    pub hardware_model: String,
    pub cpu_model: String,
    pub cpu_cores: u16,
    pub ram_bytes: u64,
    pub arch: String,
    pub os_product: String,
    pub os_version: String,
    pub os_build: String,
    pub os_image: String,
    pub kernel: String,
    pub display_session: String,
    pub display_socket: Option<String>,
    pub monitor_width_px: u32,
    pub monitor_height_px: u32,
    pub monitor_scale_milli: u32,
    pub monitor_refresh_millihz: u32,
    pub gtk_version: Option<String>,
    pub webkitgtk_version: Option<String>,
    pub wkwebview_version: Option<String>,
    pub virtualized: bool,
    pub virtualization_image_sha256: Option<[u8; 32]>,
    pub snapshot_provider: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbePayloadV1 {
    pub request: ProbeRequestV1,
    pub identity: RunnerIdentityV1,
    pub boot_id_sha256: [u8; 32],
    pub graphical_session_id_sha256: [u8; 32],
    pub captured_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub launcher_protocol_version: u32,
    pub measured_probe_sha256: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedRunnerProbeV1 {
    pub payload_cbor_hex: String,
    pub signature_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeExchangeV1 {
    pub request: ProbeRequestV1,
    pub receipt: SignedRunnerProbeV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunnerLockV1 {
    pub schema: String,
    pub runner_ids: Vec<String>,
    pub trust_manifest_sha256: [u8; 32],
    pub dispatch_manifest_sha256: [u8; 32],
    pub matrix_run_id: [u8; 32],
    pub enrollment_exchanges: Vec<ProbeExchangeV1>,
    pub identities: Vec<RunnerIdentityV1>,
    pub enrollment_commitment: [u8; 32],
    pub post_lock_exchanges: Vec<ProbeExchangeV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeProbeChallengeV1 {
    pub challenge: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeProbeReportV1 {
    pub challenge: [u8; 32],
    pub identity: RunnerIdentityV1,
    pub boot_id_sha256: [u8; 32],
    pub graphical_session_id_sha256: [u8; 32],
    pub snapshot_id: String,
    pub snapshot_provider: String,
    pub snapshot_image_sha256: [u8; 32],
    pub captured_at_unix_ms: u64,
}
