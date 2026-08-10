use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const RUNNERS: [&str; 5] = [
    "rutile-macos-arm64-v1",
    "rutile-macos-x86_64-v1",
    "rutile-ubuntu-x11-v1",
    "rutile-ubuntu-wayland-v1",
    "rutile-fedora-wayland-v1",
];

#[derive(Clone, Debug)]
pub(crate) enum ManifestState {
    Unprovisioned,
    Provisioned(ProvisioningManifests),
}

#[derive(Clone, Debug)]
pub(crate) struct ProvisioningManifests {
    pub trust: TrustManifest,
    pub dispatch: DispatchManifest,
    pub trust_sha256: [u8; 32],
    pub dispatch_sha256: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustManifest {
    pub schema: String,
    pub roots: Vec<TrustRoot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustRoot {
    pub runner_id: String,
    pub key_id: String,
    pub public_key_hex: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchManifest {
    pub schema: String,
    pub runners: Vec<DispatchRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchRow {
    pub runner_id: String,
    pub endpoint: String,
    pub ssh_host_ed25519_public_key_hex: String,
    pub launcher_protocol_version: u32,
    pub probe_path: String,
    pub probe_sha256: String,
    pub enrollment_snapshot_id: String,
    pub snapshot_provider: String,
    pub enrollment_image_sha256: String,
    pub identity: PinnedIdentityRow,
    pub macos_designated_requirement: Option<String>,
    pub macos_cdhash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinnedIdentityRow {
    pub machine_id_sha256: String,
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
    pub virtualization_image_sha256: Option<String>,
}

pub(crate) fn parse_manifest_state(
    trust_bytes: Option<&[u8]>,
    dispatch_bytes: Option<&[u8]>,
) -> Result<ManifestState, String> {
    match (trust_bytes, dispatch_bytes) {
        (None, None) => Ok(ManifestState::Unprovisioned),
        (Some(_), None) | (None, Some(_)) => {
            Err("both manifests must be present or both absent".into())
        }
        (Some(trust_bytes), Some(dispatch_bytes)) => {
            let trust: TrustManifest = serde_json::from_slice(trust_bytes)
                .map_err(|error| format!("trust JSON: {error}"))?;
            let dispatch_text = std::str::from_utf8(dispatch_bytes)
                .map_err(|_| "dispatch TOML is not UTF-8".to_owned())?;
            let dispatch: DispatchManifest =
                toml::from_str(dispatch_text).map_err(|error| format!("dispatch TOML: {error}"))?;
            validate(&trust, &dispatch)?;
            Ok(ManifestState::Provisioned(ProvisioningManifests {
                trust,
                dispatch,
                trust_sha256: Sha256::digest(trust_bytes).into(),
                dispatch_sha256: Sha256::digest(dispatch_bytes).into(),
            }))
        }
    }
}

fn validate(trust: &TrustManifest, dispatch: &DispatchManifest) -> Result<(), String> {
    if trust.schema != "rutile.runner-trust-roots.v1"
        || dispatch.schema != "rutile.runner-dispatch.v1"
        || trust.roots.len() != RUNNERS.len()
        || dispatch.runners.len() != RUNNERS.len()
    {
        return Err("wrong schema or row count".into());
    }
    let mut keys = BTreeSet::new();
    let mut key_ids = BTreeSet::new();
    let mut endpoints = BTreeSet::new();
    for (index, expected) in RUNNERS.iter().enumerate() {
        let root = &trust.roots[index];
        let row = &dispatch.runners[index];
        if root.runner_id != *expected || row.runner_id != *expected {
            return Err("runner rows are not the exact ordered set".into());
        }
        let key = decode_hash(&root.public_key_hex, "public key")?;
        if key == [0; 32]
            || !keys.insert(key)
            || root.key_id.trim().is_empty()
            || !key_ids.insert(root.key_id.clone())
        {
            return Err("zero/duplicate key or empty key id".into());
        }
        if row.launcher_protocol_version != 1
            || row.endpoint.trim().is_empty()
            || is_reserved_endpoint(&row.endpoint)
            || !valid_endpoint(&row.endpoint)
            || !endpoints.insert(row.endpoint.clone())
            || row.enrollment_snapshot_id.trim().is_empty()
            || row.snapshot_provider.trim().is_empty()
            || row.probe_path.trim().is_empty()
            || !row.probe_path.starts_with('/')
            || expected_probe_path(expected) != Some(row.probe_path.as_str())
        {
            return Err("invalid dispatch identity or endpoint".into());
        }
        for (value, label) in [
            (&row.ssh_host_ed25519_public_key_hex, "SSH host public key"),
            (&row.probe_sha256, "probe hash"),
            (&row.enrollment_image_sha256, "image hash"),
        ] {
            if decode_hash(value, label)? == [0; 32] {
                return Err(format!("zero {label}"));
            }
        }
        validate_pinned_identity(row)?;
        let mac = expected.starts_with("rutile-macos-");
        if mac
            != (row
                .macos_designated_requirement
                .as_deref()
                .is_some_and(|v| !v.is_empty())
                && row
                    .macos_cdhash
                    .as_deref()
                    .is_some_and(|value| value.len() == 40 && valid_lower_hex(value)))
        {
            return Err("macOS signature pins are inconsistent".into());
        }
    }
    Ok(())
}

fn validate_pinned_identity(row: &DispatchRow) -> Result<(), String> {
    let identity = &row.identity;
    let required = [
        identity.hardware_model.as_str(),
        identity.cpu_model.as_str(),
        identity.arch.as_str(),
        identity.os_product.as_str(),
        identity.os_version.as_str(),
        identity.os_build.as_str(),
        identity.os_image.as_str(),
        identity.kernel.as_str(),
        identity.display_session.as_str(),
    ];
    if required.into_iter().any(str::is_empty)
        || identity.cpu_cores == 0
        || identity.ram_bytes == 0
        || identity.monitor_width_px == 0
        || identity.monitor_height_px == 0
        || identity.monitor_scale_milli == 0
        || identity.monitor_refresh_millihz == 0
        || decode_hash(&identity.machine_id_sha256, "machine id")? == [0; 32]
        || identity.virtualized != identity.virtualization_image_sha256.is_some()
    {
        return Err("invalid pinned runner identity".into());
    }
    if let Some(image) = &identity.virtualization_image_sha256 {
        if decode_hash(image, "virtualization image")? == [0; 32] {
            return Err("zero virtualization image".into());
        }
    }
    let mac = row.runner_id.starts_with("rutile-macos-");
    if mac
        != (identity.display_socket.is_none()
            && identity.gtk_version.is_none()
            && identity.webkitgtk_version.is_none()
            && identity
                .wkwebview_version
                .as_deref()
                .is_some_and(|v| !v.is_empty()))
        || (!mac
            && (identity.display_socket.as_deref().is_none_or(str::is_empty)
                || identity.gtk_version.as_deref().is_none_or(str::is_empty)
                || identity
                    .webkitgtk_version
                    .as_deref()
                    .is_none_or(str::is_empty)
                || identity.wkwebview_version.is_some()))
    {
        return Err("pinned platform identity options are inconsistent".into());
    }
    Ok(())
}

fn expected_probe_path(runner_id: &str) -> Option<&'static str> {
    if matches!(
        runner_id,
        "rutile-macos-arm64-v1" | "rutile-macos-x86_64-v1"
    ) {
        Some("/Library/Application Support/Rutile Runner/bin/rutile-runner-probe")
    } else if matches!(
        runner_id,
        "rutile-ubuntu-x11-v1" | "rutile-ubuntu-wayland-v1" | "rutile-fedora-wayland-v1"
    ) {
        Some("/usr/libexec/rutile-runner-probe")
    } else {
        None
    }
}

fn valid_endpoint(value: &str) -> bool {
    let Some((host, port)) = value.rsplit_once(':') else {
        return false;
    };
    !host.trim().is_empty() && port.parse::<u16>().is_ok_and(|port| port != 0)
}

fn decode_hash(value: &str, label: &str) -> Result<[u8; 32], String> {
    if !valid_lower_hex(value) || value.len() != 64 {
        return Err(format!("invalid {label}"));
    }
    hex::decode(value)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| format!("invalid {label}"))
}

fn valid_lower_hex(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_reserved_endpoint(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("example")
        || lower.contains("localhost")
        || lower.contains("127.0.0.1")
        || lower.contains("0.0.0.0")
        || lower.contains("placeholder")
        || lower.contains("invalid")
}

pub(crate) fn render_production_config(manifests: &ProvisioningManifests) -> String {
    let mut roots = String::new();
    let mut rows = String::new();
    for (root, row) in manifests
        .trust
        .roots
        .iter()
        .zip(&manifests.dispatch.runners)
    {
        let key = decode_hash(&root.public_key_hex, "public key").expect("validated key");
        roots.push_str(&format!(
            "TrustRootConfig {{ runner_id: {:?}, key_id: {:?}, public_key: {:?} }},",
            root.runner_id, root.key_id, key
        ));
        rows.push_str(&format!(
            "RunnerDispatchConfig {{ runner_id: {:?}, endpoint: {:?}, ssh_host_ed25519_public_key: {:?}, launcher_protocol_version: 1, probe_path: {:?}, probe_sha256: {:?}, enrollment_snapshot_id: {:?}, snapshot_provider: {:?}, enrollment_image_sha256: {:?}, identity: PinnedRunnerIdentityConfig {{ machine_id_sha256: {:?}, hardware_model: {:?}, cpu_model: {:?}, cpu_cores: {:?}, ram_bytes: {:?}, arch: {:?}, os_product: {:?}, os_version: {:?}, os_build: {:?}, os_image: {:?}, kernel: {:?}, display_session: {:?}, display_socket: {:?}, monitor_width_px: {:?}, monitor_height_px: {:?}, monitor_scale_milli: {:?}, monitor_refresh_millihz: {:?}, gtk_version: {:?}, webkitgtk_version: {:?}, wkwebview_version: {:?}, virtualized: {:?}, virtualization_image_sha256: {:?} }}, macos_designated_requirement: {:?}, macos_cdhash: {:?} }},",
            row.runner_id,
            row.endpoint,
            decode_hash(&row.ssh_host_ed25519_public_key_hex, "SSH host public key")
                .expect("validated"),
            row.probe_path,
            decode_hash(&row.probe_sha256, "probe").expect("validated"),
            row.enrollment_snapshot_id,
            row.snapshot_provider,
            decode_hash(&row.enrollment_image_sha256, "image").expect("validated"),
            decode_hash(&row.identity.machine_id_sha256, "machine id").expect("validated"),
            row.identity.hardware_model,
            row.identity.cpu_model,
            row.identity.cpu_cores,
            row.identity.ram_bytes,
            row.identity.arch,
            row.identity.os_product,
            row.identity.os_version,
            row.identity.os_build,
            row.identity.os_image,
            row.identity.kernel,
            row.identity.display_session,
            row.identity.display_socket,
            row.identity.monitor_width_px,
            row.identity.monitor_height_px,
            row.identity.monitor_scale_milli,
            row.identity.monitor_refresh_millihz,
            row.identity.gtk_version,
            row.identity.webkitgtk_version,
            row.identity.wkwebview_version,
            row.identity.virtualized,
            row.identity.virtualization_image_sha256.as_ref().map(|value| decode_hash(value, "virtualization image").expect("validated")),
            row.macos_designated_requirement,
            row.macos_cdhash,
        ));
    }
    format!(
        "pub(crate) const PRODUCTION_RUNNER_CONFIG: ProductionRunnerConfig = ProductionRunnerConfig::Provisioned(ProvisionedRunnerConfig {{ trust_manifest_sha256: {:?}, dispatch_manifest_sha256: {:?}, roots: [{roots}], dispatch: [{rows}] }});\n",
        manifests.trust_sha256, manifests.dispatch_sha256
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_absent_is_the_only_unprovisioned_state() {
        assert!(matches!(
            parse_manifest_state(None, None).unwrap(),
            ManifestState::Unprovisioned
        ));
        assert!(parse_manifest_state(Some(b"{}"), None).is_err());
        assert!(parse_manifest_state(None, Some(b"")).is_err());
    }

    #[test]
    fn every_runner_id_has_one_fixed_launcher_control_probe_path() {
        for runner in RUNNERS {
            let path = expected_probe_path(runner).unwrap();
            if runner.starts_with("rutile-macos-") {
                assert_eq!(
                    path,
                    "/Library/Application Support/Rutile Runner/bin/rutile-runner-probe"
                );
            } else {
                assert_eq!(path, "/usr/libexec/rutile-runner-probe");
            }
        }
        assert!(expected_probe_path("unknown-runner").is_none());
    }
}
