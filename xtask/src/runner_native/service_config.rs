use serde::Deserialize;

use crate::runner::EXPECTED_RUNNERS;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LauncherConfigV1 {
    pub runner_id: String,
    pub key_id: String,
    pub transport_fingerprint: [u8; 32],
    pub probe_sha256: [u8; 32],
    pub macos_designated_requirement: Option<String>,
    pub macos_cdhash: Option<[u8; 20]>,
    linux_probe_environment: Option<LinuxProbeEnvironment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LinuxProbeEnvironment {
    pub display_session: String,
    pub display_socket: String,
    pub monitor_scale_milli: u32,
    pub monitor_refresh_millihz: u32,
}

impl LauncherConfigV1 {
    #[cfg(any(target_os = "linux", test))]
    pub(super) fn linux_probe_environment(&self) -> Result<&LinuxProbeEnvironment, String> {
        self.linux_probe_environment
            .as_ref()
            .ok_or_else(|| "launcher config has no Linux probe environment".into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherConfigWireV1 {
    schema: String,
    runner_id: String,
    key_id: String,
    transport_fingerprint_sha256: String,
    probe_sha256: String,
    macos_designated_requirement: Option<String>,
    macos_cdhash: Option<String>,
    linux_display_session: Option<String>,
    linux_display_socket: Option<String>,
    linux_monitor_scale_milli: Option<u32>,
    linux_monitor_refresh_millihz: Option<u32>,
}

pub(super) fn parse_launcher_config(bytes: &[u8]) -> Result<LauncherConfigV1, String> {
    if bytes.len() > 16 * 1024 {
        return Err("launcher config exceeds 16 KiB".into());
    }
    let wire: LauncherConfigWireV1 =
        serde_json::from_slice(bytes).map_err(|error| format!("launcher config JSON: {error}"))?;
    if wire.schema != "feathermark.runner-launcher-config.v1"
        || !EXPECTED_RUNNERS.contains(&wire.runner_id.as_str())
        || wire.key_id.trim().is_empty()
    {
        return Err("launcher config schema/runner/key id is invalid".into());
    }
    let transport_fingerprint = decode_hash::<32>(&wire.transport_fingerprint_sha256)?;
    let probe_sha256 = decode_hash::<32>(&wire.probe_sha256)?;
    let macos_cdhash = wire
        .macos_cdhash
        .as_deref()
        .map(decode_hash::<20>)
        .transpose()?;
    let macos = wire.runner_id.starts_with("fm-macos-");
    let linux_environment = match (
        wire.linux_display_session,
        wire.linux_display_socket,
        wire.linux_monitor_scale_milli,
        wire.linux_monitor_refresh_millihz,
    ) {
        (Some(display_session), Some(display_socket), Some(scale), Some(refresh)) if !macos => {
            let expected_session = if wire.runner_id == "fm-ubuntu-x11-v1" {
                "x11"
            } else {
                "wayland"
            };
            let socket_valid = display_socket.len() <= 128
                && !display_socket.is_empty()
                && !display_socket.chars().any(char::is_whitespace)
                && if expected_session == "x11" {
                    display_socket.starts_with(':')
                } else {
                    display_socket.starts_with("wayland-")
                };
            if display_session != expected_session
                || !socket_valid
                || scale != 1000
                || refresh != 60_000
            {
                return Err(
                    "launcher config Linux display pins do not match the fixed runner matrix"
                        .into(),
                );
            }
            Some(LinuxProbeEnvironment {
                display_session,
                display_socket,
                monitor_scale_milli: scale,
                monitor_refresh_millihz: refresh,
            })
        }
        (None, None, None, None) if macos => None,
        _ => return Err("launcher config contains inconsistent Linux display pins".into()),
    };
    if transport_fingerprint == [0; 32]
        || probe_sha256 == [0; 32]
        || macos
            != (wire
                .macos_designated_requirement
                .as_deref()
                .is_some_and(|requirement| !requirement.trim().is_empty())
                && macos_cdhash.is_some())
    {
        return Err("launcher config contains an invalid platform pin".into());
    }
    Ok(LauncherConfigV1 {
        runner_id: wire.runner_id,
        key_id: wire.key_id,
        transport_fingerprint,
        probe_sha256,
        macos_designated_requirement: wire.macos_designated_requirement,
        macos_cdhash,
        linux_probe_environment: linux_environment,
    })
}

fn decode_hash<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err("launcher config hash is not canonical lowercase hex".into());
    }
    hex::decode(value)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "launcher config hash is malformed".into())
}
