use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EXPECTED_RUNNERS: [&str; 5] = [
    "fm-macos-arm64-v1",
    "fm-macos-x86_64-v1",
    "fm-ubuntu-x11-v1",
    "fm-ubuntu-wayland-v1",
    "fm-fedora-wayland-v1",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerCapturePayload {
    pub schema: String,
    pub runner_id: String,
    pub cpu_model: String,
    pub cpu_cores: u16,
    pub ram_bytes: u64,
    pub arch: String,
    pub os_name: String,
    pub os_version: String,
    pub os_build: String,
    pub kernel: String,
    pub display_session: String,
    pub xdg_session_type: Option<String>,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
    pub monitor_width_px: u32,
    pub monitor_height_px: u32,
    pub monitor_scale_milli: u32,
    pub monitor_refresh_millihz: u32,
    pub gtk_version: Option<String>,
    pub webkitgtk_version: Option<String>,
    pub wkwebview_version: Option<String>,
    pub virtualized: bool,
    pub vm_image_digest: Option<String>,
    pub snapshot_provider: String,
    pub snapshot_id: String,
    pub captured_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRunnerCapture {
    pub payload: RunnerCapturePayload,
    pub signature_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedRunnerKey {
    pub runner_id: String,
    pub public_key_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedRunnerKeys {
    pub schema: String,
    pub keys: Vec<TrustedRunnerKey>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerLock {
    pub schema: String,
    pub trusted_keys: Vec<TrustedRunnerKey>,
    pub runners: Vec<SignedRunnerCapture>,
}

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("runner capture I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runner capture JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("runner set must be exactly the closed five-row matrix")]
    RunnerSet,
    #[error("capture directory contains an unexpected entry: {0}")]
    UnexpectedEntry(PathBuf),
    #[error("trusted key set is invalid")]
    TrustedKeys,
    #[error("runner capture signature is invalid: {0}")]
    Signature(String),
    #[error("runner capture violates the locked platform contract: {0}")]
    Platform(String),
    #[error("snapshot id is reused: {0}")]
    ReusedSnapshot(String),
}

pub fn sign_capture(payload: &RunnerCapturePayload, secret_key: &[u8; 32]) -> SignedRunnerCapture {
    let signing_key = SigningKey::from_bytes(secret_key);
    let message = serde_json::to_vec(payload).expect("runner payload serialization cannot fail");
    SignedRunnerCapture {
        payload: payload.clone(),
        signature_hex: hex::encode(signing_key.sign(&message).to_bytes()),
    }
}

pub fn capture_verify_matrix(
    runners: &[String],
    capture_dir: &Path,
    out: &Path,
) -> Result<RunnerLock, RunnerError> {
    if runners.iter().map(String::as_str).ne(EXPECTED_RUNNERS) {
        return Err(RunnerError::RunnerSet);
    }
    reject_unexpected_entries(capture_dir)?;
    let trusted: TrustedRunnerKeys = read_json(&capture_dir.join("trusted-runner-keys-v1.json"))?;
    if trusted.schema != "feathermark.trusted-runner-keys.v1" {
        return Err(RunnerError::TrustedKeys);
    }
    let captures = EXPECTED_RUNNERS
        .iter()
        .map(|runner_id| read_json(&capture_dir.join(format!("{runner_id}.capture.json"))))
        .collect::<Result<Vec<_>, _>>()?;
    let lock = RunnerLock {
        schema: "feathermark.runner-lock.v1".into(),
        trusted_keys: trusted.keys,
        runners: captures,
    };
    verify_lock(&lock)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut encoded = serde_json::to_vec_pretty(&lock)?;
    encoded.push(b'\n');
    fs::write(out, encoded)?;
    verify_runner_lock(out)
}

pub fn verify_runner_lock(path: &Path) -> Result<RunnerLock, RunnerError> {
    let lock: RunnerLock = read_json(path)?;
    verify_lock(&lock)?;
    Ok(lock)
}

fn verify_lock(lock: &RunnerLock) -> Result<(), RunnerError> {
    if lock.schema != "feathermark.runner-lock.v1" {
        return Err(RunnerError::RunnerSet);
    }
    let keys = trusted_key_map(&lock.trusted_keys)?;
    if lock.runners.len() != EXPECTED_RUNNERS.len() {
        return Err(RunnerError::RunnerSet);
    }
    let mut snapshots = BTreeSet::new();
    for (expected_id, capture) in EXPECTED_RUNNERS.iter().zip(&lock.runners) {
        if capture.payload.runner_id != *expected_id {
            return Err(RunnerError::RunnerSet);
        }
        verify_signature(capture, &keys[*expected_id])?;
        validate_platform(&capture.payload)?;
        if !snapshots.insert(capture.payload.snapshot_id.clone()) {
            return Err(RunnerError::ReusedSnapshot(
                capture.payload.snapshot_id.clone(),
            ));
        }
    }
    Ok(())
}

fn trusted_key_map(keys: &[TrustedRunnerKey]) -> Result<BTreeMap<String, [u8; 32]>, RunnerError> {
    if keys.len() != EXPECTED_RUNNERS.len() {
        return Err(RunnerError::TrustedKeys);
    }
    let mut decoded = BTreeMap::new();
    for key in keys {
        let bytes: [u8; 32] = hex::decode(&key.public_key_hex)
            .ok()
            .and_then(|value| value.try_into().ok())
            .ok_or(RunnerError::TrustedKeys)?;
        if decoded.insert(key.runner_id.clone(), bytes).is_some() {
            return Err(RunnerError::TrustedKeys);
        }
    }
    if decoded.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != EXPECTED_RUNNERS.into_iter().collect()
    {
        return Err(RunnerError::TrustedKeys);
    }
    Ok(decoded)
}

fn verify_signature(
    capture: &SignedRunnerCapture,
    public_key: &[u8; 32],
) -> Result<(), RunnerError> {
    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|_| RunnerError::Signature(capture.payload.runner_id.clone()))?;
    let signature_bytes: [u8; 64] = hex::decode(&capture.signature_hex)
        .ok()
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| RunnerError::Signature(capture.payload.runner_id.clone()))?;
    let signature = Signature::from_bytes(&signature_bytes);
    let message = serde_json::to_vec(&capture.payload)?;
    verifying_key
        .verify(&message, &signature)
        .map_err(|_| RunnerError::Signature(capture.payload.runner_id.clone()))
}

fn validate_platform(payload: &RunnerCapturePayload) -> Result<(), RunnerError> {
    if payload.schema != "feathermark.runner-capture.v1"
        || payload.ram_bytes != 16 * 1024 * 1024 * 1024
        || payload.monitor_scale_milli != 1000
        || payload.monitor_refresh_millihz != 60_000
        || required(&payload.os_version).is_err()
        || required(&payload.os_build).is_err()
        || required(&payload.kernel).is_err()
        || required(&payload.snapshot_provider).is_err()
        || required(&payload.snapshot_id).is_err()
        || required(&payload.captured_at).is_err()
        || payload.virtualized != payload.vm_image_digest.is_some()
        || payload
            .vm_image_digest
            .as_deref()
            .is_some_and(|digest| !valid_sha256_digest(digest))
    {
        return Err(RunnerError::Platform(payload.runner_id.clone()));
    }
    let actual = (
        payload.cpu_model.as_str(),
        payload.cpu_cores,
        payload.arch.as_str(),
        payload.os_name.as_str(),
        payload.os_version.as_str(),
        payload.display_session.as_str(),
        payload.xdg_session_type.as_deref(),
        payload.display.is_some(),
        payload.wayland_display.is_some(),
        payload.monitor_width_px,
        payload.monitor_height_px,
    );
    let (expected, expects_wkwebview, expects_gtk) = match payload.runner_id.as_str() {
        "fm-macos-arm64-v1" => (
            (
                "Apple M1",
                8,
                "aarch64",
                "macOS",
                payload.os_version.as_str(),
                "native",
                None,
                false,
                false,
                2560,
                1600,
            ),
            true,
            false,
        ),
        "fm-macos-x86_64-v1" => (
            (
                "Intel Core i7-9750H",
                6,
                "x86_64",
                "macOS",
                payload.os_version.as_str(),
                "native",
                None,
                false,
                false,
                1920,
                1080,
            ),
            true,
            false,
        ),
        "fm-ubuntu-x11-v1" => (
            (
                "Intel Core i5-8500",
                6,
                "x86_64",
                "Ubuntu",
                "24.04",
                "x11",
                Some("x11"),
                true,
                false,
                1920,
                1080,
            ),
            false,
            true,
        ),
        "fm-ubuntu-wayland-v1" => (
            (
                "Intel Core i5-8500",
                6,
                "x86_64",
                "Ubuntu",
                "24.04",
                "wayland",
                Some("wayland"),
                false,
                true,
                1920,
                1080,
            ),
            false,
            true,
        ),
        "fm-fedora-wayland-v1" => (
            (
                "Intel Core i5-8500",
                6,
                "x86_64",
                "Fedora",
                "43",
                "wayland",
                Some("wayland"),
                false,
                true,
                1920,
                1080,
            ),
            false,
            true,
        ),
        _ => return Err(RunnerError::RunnerSet),
    };
    if actual != expected
        || payload.wkwebview_version.is_some() != expects_wkwebview
        || payload.gtk_version.is_some() != expects_gtk
        || payload.webkitgtk_version.is_some() != expects_gtk
        || payload
            .wkwebview_version
            .iter()
            .chain(payload.gtk_version.iter())
            .chain(payload.webkitgtk_version.iter())
            .any(|version| required(version).is_err())
    {
        return Err(RunnerError::Platform(payload.runner_id.clone()));
    }
    Ok(())
}

fn required(value: &str) -> Result<(), ()> {
    if value.is_empty() || value.contains('+') {
        Err(())
    } else {
        Ok(())
    }
}

fn valid_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn reject_unexpected_entries(capture_dir: &Path) -> Result<(), RunnerError> {
    let mut allowed: BTreeSet<String> = EXPECTED_RUNNERS
        .iter()
        .map(|id| format!("{id}.capture.json"))
        .collect();
    allowed.insert("trusted-runner-keys-v1.json".into());
    for entry in fs::read_dir(capture_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_file() || !allowed.contains(&name) {
            return Err(RunnerError::UnexpectedEntry(entry.path()));
        }
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, RunnerError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
