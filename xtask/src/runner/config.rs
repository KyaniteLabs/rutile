pub(crate) const RUNNERS: [&str; 5] = [
    "fm-macos-arm64-v1",
    "fm-macos-x86_64-v1",
    "fm-ubuntu-x11-v1",
    "fm-ubuntu-wayland-v1",
    "fm-fedora-wayland-v1",
];

#[derive(Clone, Copy)]
#[allow(dead_code)] // The provisioned variant is generated only when reviewed manifests exist.
#[allow(clippy::large_enum_variant)] // Build-time constants deliberately avoid heap/runtime indirection.
pub(crate) enum ProductionRunnerConfig {
    Unprovisioned,
    Provisioned(ProvisionedRunnerConfig),
}

#[derive(Clone, Copy)]
pub(crate) struct ProvisionedRunnerConfig {
    pub trust_manifest_sha256: [u8; 32],
    pub dispatch_manifest_sha256: [u8; 32],
    pub roots: [TrustRootConfig; 5],
    pub dispatch: [RunnerDispatchConfig; 5],
}

#[derive(Clone, Copy)]
pub(crate) struct TrustRootConfig {
    pub runner_id: &'static str,
    #[allow(dead_code)] // Consumed by native service installation/smoke evidence.
    pub key_id: &'static str,
    pub public_key: [u8; 32],
}

#[derive(Clone, Copy)]
pub(crate) struct RunnerDispatchConfig {
    pub runner_id: &'static str,
    pub endpoint: &'static str,
    pub ssh_host_ed25519_public_key: [u8; 32],
    pub launcher_protocol_version: u32,
    #[allow(dead_code)] // Consumed by the root measured-probe launcher, not the coordinator.
    pub probe_path: &'static str,
    pub probe_sha256: [u8; 32],
    pub enrollment_snapshot_id: &'static str,
    pub snapshot_provider: &'static str,
    pub enrollment_image_sha256: [u8; 32],
    pub identity: PinnedRunnerIdentityConfig,
    #[allow(dead_code)] // Consumed by the macOS root launcher acceptance path.
    pub macos_designated_requirement: Option<&'static str>,
    #[allow(dead_code)] // Consumed by the macOS root launcher acceptance path.
    pub macos_cdhash: Option<&'static str>,
}

#[derive(Clone, Copy)]
pub(crate) struct PinnedRunnerIdentityConfig {
    pub machine_id_sha256: [u8; 32],
    pub hardware_model: &'static str,
    pub cpu_model: &'static str,
    pub cpu_cores: u16,
    pub ram_bytes: u64,
    pub arch: &'static str,
    pub os_product: &'static str,
    pub os_version: &'static str,
    pub os_build: &'static str,
    pub os_image: &'static str,
    pub kernel: &'static str,
    pub display_session: &'static str,
    pub display_socket: Option<&'static str>,
    pub monitor_width_px: u32,
    pub monitor_height_px: u32,
    pub monitor_scale_milli: u32,
    pub monitor_refresh_millihz: u32,
    pub gtk_version: Option<&'static str>,
    pub webkitgtk_version: Option<&'static str>,
    pub wkwebview_version: Option<&'static str>,
    pub virtualized: bool,
    pub virtualization_image_sha256: Option<[u8; 32]>,
}

include!(concat!(env!("OUT_DIR"), "/production_runner_config.rs"));

pub(crate) fn production_config() -> ProductionRunnerConfig {
    PRODUCTION_RUNNER_CONFIG
}
