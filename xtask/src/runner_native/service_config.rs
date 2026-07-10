use serde::Deserialize;

use crate::runner::EXPECTED_RUNNERS;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LauncherConfigV1 {
    pub runner_id: String,
    pub key_id: String,
    pub probe_sha256: [u8; 32],
    pub macos_designated_requirement: Option<String>,
    pub macos_cdhash: Option<[u8; 20]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherConfigWireV1 {
    schema: String,
    runner_id: String,
    key_id: String,
    probe_sha256: String,
    macos_designated_requirement: Option<String>,
    macos_cdhash: Option<String>,
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
    let probe_sha256 = decode_hash::<32>(&wire.probe_sha256)?;
    let macos_cdhash = wire
        .macos_cdhash
        .as_deref()
        .map(decode_hash::<20>)
        .transpose()?;
    let macos = wire.runner_id.starts_with("fm-macos-");
    if probe_sha256 == [0; 32]
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
        probe_sha256,
        macos_designated_requirement: wire.macos_designated_requirement,
        macos_cdhash,
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
