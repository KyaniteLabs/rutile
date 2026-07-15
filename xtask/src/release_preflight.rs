//! Fail-closed, bounded release-prerequisite attestations.
//!
//! This is deliberately not a release authorization format.  A checked-in
//! record can retain a blocked preflight, but only independently authenticated
//! runner/signer probes may ever clear a prerequisite in a future release job.
use crate::tool_process;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub const SCHEMA: &str = "rutile.release-prerequisite-preflight.v1";
const MAX_INPUT_BYTES: u64 = 1024 * 1024;
const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

/// Generate a random, caller-unpredictable temporary name for create-only
/// publication.  The name is never derived from `run_id` or any other
/// caller-controlled field.
fn random_temp_name() -> Result<String, PreflightError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        PreflightError::Publish(format!("failed to generate random temp name: {error}"))
    })?;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(".tmp.{hex}"))
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preflight {
    pub schema: String,
    pub version: u32,
    pub run_id: String,
    pub generated_at_unix_ms: u64,
    pub source: Source,
    pub verifier: Verifier,
    pub runner_lock: Option<HashBoundLog>,
    pub probes: Probes,
    pub result: ResultState,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub commit: String,
    pub tree: String,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Verifier {
    pub identity: String,
    pub key_fingerprint: Option<String>,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HashBoundLog {
    pub logical_id: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Attested,
    Unavailable,
    Failed,
    NotRequired,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub state: ProbeState,
    pub observed_at_unix_ms: u64,
    pub evidence: Option<HashBoundLog>,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerProbe {
    pub runner_id: String,
    pub os: String,
    pub architecture: String,
    pub display: String,
    pub clean_install_host: Probe,
    pub capability: Probe,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppleProbe {
    pub certificate_sha256: Option<String>,
    pub team_id: Option<String>,
    pub certificate_expires_at_unix_ms: Option<u64>,
    pub private_key_challenge: Probe,
    pub notarization_challenge: Probe,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GpgProbe {
    pub fingerprint: Option<String>,
    pub signing_challenge: Probe,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TagApprovalProbe {
    pub protected_pattern: String,
    pub manual_owner_approval: Probe,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionProbe {
    pub pr_days: u32,
    pub release_days: u32,
    pub maximum_artifact_bytes: u64,
    pub truncation_fails: bool,
    pub policy: Probe,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Probes {
    pub macos_arm64: RunnerProbe,
    pub linux_x86_64_x11: RunnerProbe,
    pub macos_x86_64: Probe,
    pub linux_x86_64_wayland: Probe,
    pub apple: AppleProbe,
    pub linux_gpg: GpgProbe,
    pub protected_tag_and_owner_approval: TagApprovalProbe,
    pub artifact_retention: RetentionProbe,
}
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResultState {
    pub ready: bool,
    pub hard_blockers: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PublishOutcome {
    pub durable: bool,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error("preflight input exceeds {MAX_INPUT_BYTES} bytes")]
    TooLarge,
    #[error("cannot read preflight: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid preflight JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid preflight: {0}")]
    Invalid(String),
    #[error("evidence publication refused: {0}")]
    Publish(String),
}

/// Pinned repository root derived at compile time from the `xtask` crate
/// location. Source binding must never follow the runtime working directory or
/// inherited Git environment.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate lives in the workspace")
}

pub fn load_and_validate(path: &Path) -> Result<Preflight, PreflightError> {
    let bytes = read_regular_file(path)?;
    let report = load_and_validate_bytes(&bytes)?;
    verify_current_source(&report.source)?;
    Ok(report)
}

#[cfg(unix)]
fn read_regular_file(path: &Path) -> Result<Vec<u8>, PreflightError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| PreflightError::Invalid("input path is not valid".into()))?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(match err.raw_os_error() {
            Some(libc::ELOOP) => PreflightError::Invalid("input must be a regular file".into()),
            _ => PreflightError::Read(err),
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } < 0 {
        return Err(PreflightError::Read(std::io::Error::last_os_error()));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(PreflightError::Invalid(
            "input must be a regular file".into(),
        ));
    }
    if stat.st_size as u64 > MAX_INPUT_BYTES {
        return Err(PreflightError::TooLarge);
    }
    // Use a hard read cap so a concurrently growing file cannot feed unbounded
    // data between fstat and read_to_end.
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(PreflightError::Read)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(PreflightError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_regular_file(path: &Path) -> Result<Vec<u8>, PreflightError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PreflightError::Invalid(
            "input must be a regular file".into(),
        ));
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(PreflightError::TooLarge);
    }
    let file = std::fs::File::open(path).map_err(PreflightError::Read)?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(PreflightError::Read)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(PreflightError::TooLarge);
    }
    Ok(bytes)
}

fn load_and_validate_bytes(bytes: &[u8]) -> Result<Preflight, PreflightError> {
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(PreflightError::TooLarge);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(PreflightError::Json)?;
    validate_required_shape(&value)?;
    let mut value: Preflight = serde_json::from_value(value).map_err(PreflightError::Json)?;
    validate(&value)?;
    value.result = assess(&value);
    Ok(value)
}

fn validate_required_shape(value: &serde_json::Value) -> Result<(), PreflightError> {
    let required: &[&[&str]] = &[
        &["schema"],
        &["version"],
        &["run_id"],
        &["generated_at_unix_ms"],
        &["source"],
        &["source", "commit"],
        &["source", "tree"],
        &["verifier"],
        &["verifier", "identity"],
        &["verifier", "key_fingerprint"],
        &["runner_lock"],
        &["probes"],
        &["probes", "macos_arm64"],
        &["probes", "macos_arm64", "runner_id"],
        &["probes", "macos_arm64", "os"],
        &["probes", "macos_arm64", "architecture"],
        &["probes", "macos_arm64", "display"],
        &["probes", "macos_arm64", "clean_install_host"],
        &["probes", "macos_arm64", "clean_install_host", "state"],
        &[
            "probes",
            "macos_arm64",
            "clean_install_host",
            "observed_at_unix_ms",
        ],
        &["probes", "macos_arm64", "clean_install_host", "evidence"],
        &["probes", "macos_arm64", "capability"],
        &["probes", "macos_arm64", "capability", "state"],
        &["probes", "macos_arm64", "capability", "observed_at_unix_ms"],
        &["probes", "macos_arm64", "capability", "evidence"],
        &["probes", "linux_x86_64_x11"],
        &["probes", "linux_x86_64_x11", "runner_id"],
        &["probes", "linux_x86_64_x11", "os"],
        &["probes", "linux_x86_64_x11", "architecture"],
        &["probes", "linux_x86_64_x11", "display"],
        &["probes", "linux_x86_64_x11", "clean_install_host"],
        &["probes", "linux_x86_64_x11", "clean_install_host", "state"],
        &[
            "probes",
            "linux_x86_64_x11",
            "clean_install_host",
            "observed_at_unix_ms",
        ],
        &[
            "probes",
            "linux_x86_64_x11",
            "clean_install_host",
            "evidence",
        ],
        &["probes", "linux_x86_64_x11", "capability"],
        &["probes", "linux_x86_64_x11", "capability", "state"],
        &[
            "probes",
            "linux_x86_64_x11",
            "capability",
            "observed_at_unix_ms",
        ],
        &["probes", "linux_x86_64_x11", "capability", "evidence"],
        &["probes", "macos_x86_64"],
        &["probes", "macos_x86_64", "state"],
        &["probes", "macos_x86_64", "observed_at_unix_ms"],
        &["probes", "macos_x86_64", "evidence"],
        &["probes", "linux_x86_64_wayland"],
        &["probes", "linux_x86_64_wayland", "state"],
        &["probes", "linux_x86_64_wayland", "observed_at_unix_ms"],
        &["probes", "linux_x86_64_wayland", "evidence"],
        &["probes", "apple"],
        &["probes", "apple", "certificate_sha256"],
        &["probes", "apple", "team_id"],
        &["probes", "apple", "certificate_expires_at_unix_ms"],
        &["probes", "apple", "private_key_challenge"],
        &["probes", "apple", "private_key_challenge", "state"],
        &[
            "probes",
            "apple",
            "private_key_challenge",
            "observed_at_unix_ms",
        ],
        &["probes", "apple", "private_key_challenge", "evidence"],
        &["probes", "apple", "notarization_challenge"],
        &["probes", "apple", "notarization_challenge", "state"],
        &[
            "probes",
            "apple",
            "notarization_challenge",
            "observed_at_unix_ms",
        ],
        &["probes", "apple", "notarization_challenge", "evidence"],
        &["probes", "linux_gpg"],
        &["probes", "linux_gpg", "fingerprint"],
        &["probes", "linux_gpg", "signing_challenge"],
        &["probes", "linux_gpg", "signing_challenge", "state"],
        &[
            "probes",
            "linux_gpg",
            "signing_challenge",
            "observed_at_unix_ms",
        ],
        &["probes", "linux_gpg", "signing_challenge", "evidence"],
        &["probes", "protected_tag_and_owner_approval"],
        &[
            "probes",
            "protected_tag_and_owner_approval",
            "protected_pattern",
        ],
        &[
            "probes",
            "protected_tag_and_owner_approval",
            "manual_owner_approval",
        ],
        &[
            "probes",
            "protected_tag_and_owner_approval",
            "manual_owner_approval",
            "state",
        ],
        &[
            "probes",
            "protected_tag_and_owner_approval",
            "manual_owner_approval",
            "observed_at_unix_ms",
        ],
        &[
            "probes",
            "protected_tag_and_owner_approval",
            "manual_owner_approval",
            "evidence",
        ],
        &["probes", "artifact_retention"],
        &["probes", "artifact_retention", "pr_days"],
        &["probes", "artifact_retention", "release_days"],
        &["probes", "artifact_retention", "maximum_artifact_bytes"],
        &["probes", "artifact_retention", "truncation_fails"],
        &["probes", "artifact_retention", "policy"],
        &["probes", "artifact_retention", "policy", "state"],
        &[
            "probes",
            "artifact_retention",
            "policy",
            "observed_at_unix_ms",
        ],
        &["probes", "artifact_retention", "policy", "evidence"],
        &["result"],
        &["result", "ready"],
        &["result", "hard_blockers"],
    ];
    for segments in required {
        let pointer = "/".to_string() + &segments.join("/");
        if value.pointer(&pointer).is_none() {
            return Err(PreflightError::Invalid(format!(
                "missing required field {pointer}"
            )));
        }
    }
    if let Some(blockers) = value
        .pointer("/result/hard_blockers")
        .and_then(|v| v.as_array())
    {
        if blockers.is_empty() || blockers.len() > 32 {
            return Err(PreflightError::Invalid(
                "result.hard_blockers must contain 1-32 items".into(),
            ));
        }
        for (i, item) in blockers.iter().enumerate() {
            let s = item.as_str().ok_or_else(|| {
                PreflightError::Invalid(format!("result.hard_blockers[{i}] must be a string"))
            })?;
            if s.is_empty() || s.len() > 200 {
                return Err(PreflightError::Invalid(format!(
                    "result.hard_blockers[{i}] must be 1-200 characters"
                )));
            }
        }
    } else {
        return Err(PreflightError::Invalid(
            "missing required field /result/hard_blockers".into(),
        ));
    }
    Ok(())
}

pub fn validate(value: &Preflight) -> Result<(), PreflightError> {
    if value.schema != SCHEMA || value.version != 1 {
        return Err(PreflightError::Invalid("schema/version mismatch".into()));
    }
    if value.result.ready {
        return Err(PreflightError::Invalid(
            "blocked inventory cannot assert a ready result".into(),
        ));
    }
    validate_logical_id(&value.run_id)?;
    valid_sha(&value.source.commit, "source commit")?;
    valid_sha(&value.source.tree, "source tree")?;
    validate_logical_id(&value.verifier.identity)?;
    if value.verifier.key_fingerprint.is_some() || value.runner_lock.is_some() {
        return Err(PreflightError::Invalid(
            "blocked inventory cannot assert verifier or runner authority".into(),
        ));
    }
    exact_runner(&value.probes.macos_arm64, "macos", "arm64", "none")?;
    exact_runner(&value.probes.linux_x86_64_x11, "linux", "x86_64", "x11")?;
    validate_probe(&value.probes.macos_x86_64)?;
    validate_probe(&value.probes.linux_x86_64_wayland)?;
    if value.probes.apple.certificate_sha256.is_some()
        || value.probes.apple.team_id.is_some()
        || value.probes.apple.certificate_expires_at_unix_ms.is_some()
        || value.probes.linux_gpg.fingerprint.is_some()
    {
        return Err(PreflightError::Invalid(
            "blocked inventory cannot assert signing authority".into(),
        ));
    }
    validate_probe(&value.probes.apple.private_key_challenge)?;
    validate_probe(&value.probes.apple.notarization_challenge)?;
    validate_probe(&value.probes.linux_gpg.signing_challenge)?;
    if value
        .probes
        .protected_tag_and_owner_approval
        .protected_pattern
        != "v*"
    {
        return Err(PreflightError::Invalid(
            "protected tag pattern must be v*".into(),
        ));
    }
    validate_probe(
        &value
            .probes
            .protected_tag_and_owner_approval
            .manual_owner_approval,
    )?;
    let retention = &value.probes.artifact_retention;
    if retention.pr_days < 30
        || retention.release_days < 365
        || retention.maximum_artifact_bytes < 100 * 1024 * 1024
        || !retention.truncation_fails
    {
        return Err(PreflightError::Invalid(
            "retention policy is below the release minimum".into(),
        ));
    }
    validate_probe(&retention.policy)?;
    Ok(())
}

fn exact_runner(
    runner: &RunnerProbe,
    os: &str,
    architecture: &str,
    display: &str,
) -> Result<(), PreflightError> {
    validate_logical_id(&runner.runner_id)?;
    if runner.os != os || runner.architecture != architecture || runner.display != display {
        return Err(PreflightError::Invalid(format!(
            "runner must be {os}/{architecture}/{display}"
        )));
    }
    validate_probe(&runner.capability)?;
    validate_probe(&runner.clean_install_host)
}
fn validate_probe(probe: &Probe) -> Result<(), PreflightError> {
    if probe.state == ProbeState::Attested || probe.evidence.is_some() {
        return Err(PreflightError::Invalid(
            "blocked inventory cannot contain attested probe evidence".into(),
        ));
    }
    Ok(())
}
fn valid_sha(value: &str, label: &str) -> Result<(), PreflightError> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(PreflightError::Invalid(format!(
            "{label} must be a lower-case git object id or SHA-256"
        )))
    }
}
fn is_non_public_ip(value: &str) -> bool {
    // Logical IDs may use `/`, `-`, `_`, or `.` as separators.  Any segment
    // that is itself a private/loopback/link-local IPv4 address is a leak.
    value
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .any(|token| {
            let Ok(ip) = token.parse::<std::net::Ipv4Addr>() else {
                return false;
            };
            ip.is_private() || ip.is_loopback() || ip.is_link_local()
        })
}

pub fn validate_logical_id(value: &str) -> Result<(), PreflightError> {
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 160
        || lower.starts_with('/')
        || lower.contains("..")
        || lower.contains('\\')
        || lower.contains('=')
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("credentials")
        || lower.contains("password")
        || lower.contains("/users/")
        || lower.contains("/home/")
        || lower.contains("/private/")
        || is_non_public_ip(value)
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'.'))
    {
        Err(PreflightError::Invalid(
            "identifier must be bounded, logical, and non-secret".into(),
        ))
    } else {
        Ok(())
    }
}

fn verify_current_source(source: &Source) -> Result<(), PreflightError> {
    verify_source_in_repo(workspace_root(), source)
}

fn verify_source_in_repo(repo: &Path, source: &Source) -> Result<(), PreflightError> {
    let output = tool_process::git_isolated(
        repo,
        &["--no-replace-objects", "rev-parse", "HEAD", "HEAD^{tree}"],
        &[],
    )
    .map_err(PreflightError::Read)?;
    if !output.status.success() || output.stdout.len() > 256 {
        return Err(PreflightError::Invalid(
            "cannot derive current repository source".into(),
        ));
    }
    let mut lines = std::str::from_utf8(&output.stdout)
        .map_err(|_| PreflightError::Invalid("git source output is not UTF-8".into()))?
        .lines();
    if lines.next() != Some(source.commit.as_str())
        || lines.next() != Some(source.tree.as_str())
        || lines.next().is_some()
    {
        return Err(PreflightError::Invalid(
            "source commit/tree do not match the current repository".into(),
        ));
    }
    Ok(())
}

fn assess(value: &Preflight) -> ResultState {
    let mut blockers = Vec::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0);
    if now.saturating_sub(value.generated_at_unix_ms) > MAX_AGE_MS
        || value.generated_at_unix_ms > now.saturating_add(5 * 60 * 1000)
    {
        blockers.push("preflight evidence is stale or has an invalid timestamp".into());
    }
    if value.verifier.key_fingerprint.is_none() {
        blockers.push("trusted preflight verifier is not provisioned".into());
    }
    if value.runner_lock.is_none() {
        blockers.push("authenticated Forgejo runner lock is unavailable".into());
    }
    if value.probes.apple.certificate_sha256.is_none()
        || value.probes.apple.team_id.is_none()
        || value
            .probes
            .apple
            .certificate_expires_at_unix_ms
            .is_none_or(|expires| expires <= now)
    {
        blockers.push("current Apple Developer ID certificate authority is unavailable".into());
    }
    if value.probes.linux_gpg.fingerprint.is_none() {
        blockers.push("Linux release GPG fingerprint is unavailable".into());
    }
    for (name, probe) in [
        (
            "macos_arm64 runner capability",
            &value.probes.macos_arm64.capability,
        ),
        (
            "macos_arm64 clean install host",
            &value.probes.macos_arm64.clean_install_host,
        ),
        (
            "linux_x86_64_x11 runner capability",
            &value.probes.linux_x86_64_x11.capability,
        ),
        (
            "linux_x86_64_x11 clean install host",
            &value.probes.linux_x86_64_x11.clean_install_host,
        ),
        (
            "Apple private-key signing",
            &value.probes.apple.private_key_challenge,
        ),
        (
            "Apple notarization",
            &value.probes.apple.notarization_challenge,
        ),
        (
            "Linux GPG signing",
            &value.probes.linux_gpg.signing_challenge,
        ),
        (
            "protected tag owner approval",
            &value
                .probes
                .protected_tag_and_owner_approval
                .manual_owner_approval,
        ),
        (
            "artifact retention",
            &value.probes.artifact_retention.policy,
        ),
    ] {
        if now.saturating_sub(probe.observed_at_unix_ms) > MAX_AGE_MS
            || probe.observed_at_unix_ms > now.saturating_add(5 * 60 * 1000)
        {
            blockers.push(format!("{name} evidence is stale or future-dated"));
        }
        if probe.state != ProbeState::Attested {
            blockers.push(format!("{name} is not independently attested"));
        }
    }
    // A retained repository record cannot prove control of a runner or signing
    // key. W3 must replace this inventory command with an authenticated release
    // job before any public release can become ready.
    blockers.push("authenticated release authority has not approved this inventory".into());
    ResultState {
        ready: false,
        hard_blockers: blockers,
    }
}

pub fn publish_create_only(
    path: &Path,
    report: &Preflight,
) -> Result<PublishOutcome, PreflightError> {
    if report.result.ready {
        return Err(PreflightError::Publish(
            "this command retains preflight evidence; it cannot publish release authorization"
                .into(),
        ));
    }
    // Re-validate and re-derive every time we serialize.  A Rust caller must not
    // be able to bypass authority rejection, source binding, or result overwrite
    // by constructing a Preflight directly.
    validate(report)?;
    verify_current_source(&report.source)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| PreflightError::Publish("output requires a file name".into()))?;

    // Create a private, random temporary file in the same directory, then
    // hard-link it to the final name and remove the temporary name.  This
    // keeps the final destination absent until the payload is fully written
    // and synced, and the temporary basename is unpredictable to the caller.
    let temp_name = std::ffi::OsString::from(random_temp_name()?);

    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let dirfd = open_private_dirfd(parent)?;
        let mut file = openat_exclusive(&dirfd, &temp_name)?;
        let publication = (|| -> Result<(), PreflightError> {
            let mut value = serde_json::to_value(report).map_err(PreflightError::Json)?;
            value["result"] = serde_json::to_value(assess(report)).map_err(PreflightError::Json)?;
            let bytes = serde_json::to_vec_pretty(&value).map_err(PreflightError::Json)?;
            file.write_all(&bytes).map_err(PreflightError::Read)?;
            file.write_all(b"\n").map_err(PreflightError::Read)?;
            file.sync_all().map_err(PreflightError::Read)?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = unlinkat_name(&dirfd, &temp_name);
        }
        publication?;

        // Link the fully-synced temporary file to the final name.  This fails
        // if the final name already exists, preserving create-only semantics.
        linkat_name(&dirfd, &temp_name, file_name)?;
        let _ = unlinkat_name(&dirfd, &temp_name);

        let mut warnings = Vec::new();
        let file_fd = file.into_raw_fd();
        if unsafe { libc::close(file_fd) } < 0 {
            warnings.push("file close failed; destination durability is unknown");
        }
        if unsafe { libc::fsync(dirfd.as_raw_fd()) } < 0 {
            warnings.push("parent directory sync failed; destination durability is unknown");
        }
        Ok(PublishOutcome {
            durable: warnings.is_empty(),
            warnings,
        })
    }
    #[cfg(not(unix))]
    {
        let parent_meta = fs::symlink_metadata(parent).map_err(PreflightError::Read)?;
        if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
            return Err(PreflightError::Publish(
                "output parent must be a real directory".into(),
            ));
        }
        let temp_path = parent.join(&temp_name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(PreflightError::Read)?;
        let publication = (|| -> Result<(), PreflightError> {
            let mut value = serde_json::to_value(report).map_err(PreflightError::Json)?;
            value["result"] = serde_json::to_value(assess(report)).map_err(PreflightError::Json)?;
            let bytes = serde_json::to_vec_pretty(&value).map_err(PreflightError::Json)?;
            file.write_all(&bytes).map_err(PreflightError::Read)?;
            file.write_all(b"\n").map_err(PreflightError::Read)?;
            file.sync_all().map_err(PreflightError::Read)?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        publication?;

        fs::hard_link(&temp_path, path).map_err(PreflightError::Read)?;
        let _ = fs::remove_file(&temp_path);

        Ok(PublishOutcome {
            durable: true,
            warnings: Vec::new(),
        })
    }
}

#[cfg(unix)]
fn open_private_dirfd(parent: &Path) -> Result<std::os::fd::OwnedFd, PreflightError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    // Walk every path component with O_NOFOLLOW so that no symlinked ancestor
    // can redirect the output directory. Each intermediate directory must also
    // be owned by the current effective user and have no group/other write bit.
    let start = if parent.is_absolute() {
        CString::new("/").unwrap()
    } else {
        CString::new(".").unwrap()
    };
    let fd = unsafe {
        libc::open(
            start.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(PreflightError::Read(std::io::Error::last_os_error()));
    }
    let mut dirfd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

    for component in parent.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(PreflightError::Publish(
                    "output parent must not contain parent-directory references".into(),
                ));
            }
            std::path::Component::Normal(name) => {
                let c_name = CString::new(name.as_bytes()).map_err(|_| {
                    PreflightError::Publish("output parent path is not valid".into())
                })?;
                let fd = unsafe {
                    libc::openat(
                        dirfd.as_raw_fd(),
                        c_name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        0,
                    )
                };
                if fd < 0 {
                    let err = std::io::Error::last_os_error();
                    return Err(match err.kind() {
                        std::io::ErrorKind::NotADirectory
                        | std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::PermissionDenied => {
                            PreflightError::Publish("output parent must be a real directory".into())
                        }
                        _ => PreflightError::Read(err),
                    });
                }
                dirfd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
                validate_dirfd(&dirfd, false)?;
            }
            _ => unreachable!(),
        }
    }
    validate_dirfd(&dirfd, true)?;
    Ok(dirfd)
}

#[cfg(unix)]
fn validate_dirfd(dirfd: &std::os::fd::OwnedFd, require_owner: bool) -> Result<(), PreflightError> {
    use std::os::fd::AsRawFd;

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(dirfd.as_raw_fd(), &mut stat) } < 0 {
        return Err(PreflightError::Read(std::io::Error::last_os_error()));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(PreflightError::Publish(
            "output parent must be a real directory".into(),
        ));
    }
    if (stat.st_mode as u32 & 0o022) != 0 {
        return Err(PreflightError::Publish(
            "output parent path traverses a writable directory".into(),
        ));
    }
    if require_owner && stat.st_uid != unsafe { libc::geteuid() } {
        return Err(PreflightError::Publish(
            "output parent must be a private directory owned by the current user".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn openat_exclusive(
    dirfd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<std::fs::File, PreflightError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_name = CString::new(name.as_bytes())
        .map_err(|_| PreflightError::Publish("output name is not valid".into()))?;
    let fd = unsafe {
        libc::openat(
            dirfd.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(if err.raw_os_error() == Some(libc::EEXIST) {
            PreflightError::Publish("output already exists".into())
        } else {
            PreflightError::Read(err)
        });
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn unlinkat_name(dirfd: &std::os::fd::OwnedFd, name: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let c_name = CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid output name")
    })?;
    let rc = unsafe { libc::unlinkat(dirfd.as_raw_fd(), c_name.as_ptr(), 0) };
    if rc < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn linkat_name(
    dirfd: &std::os::fd::OwnedFd,
    from: &std::ffi::OsStr,
    to: &std::ffi::OsStr,
) -> Result<(), PreflightError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let c_from = CString::new(from.as_bytes())
        .map_err(|_| PreflightError::Publish("invalid temporary name".into()))?;
    let c_to = CString::new(to.as_bytes())
        .map_err(|_| PreflightError::Publish("invalid output name".into()))?;
    let rc = unsafe {
        libc::linkat(
            dirfd.as_raw_fd(),
            c_from.as_ptr(),
            dirfd.as_raw_fd(),
            c_to.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if rc < 0 {
        return Err(PreflightError::Read(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate lives in the workspace")
            .join("target")
            .join("tmp");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(&root).unwrap()
    }

    fn inventory_json() -> serde_json::Value {
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let unavailable = || {
            serde_json::json!({
                "state": "unavailable",
                "observed_at_unix_ms": observed_at,
                "evidence": null
            })
        };
        serde_json::json!({
            "schema": SCHEMA,
            "version": 1,
            "run_id": "test-blocked-inventory",
            "generated_at_unix_ms": observed_at,
            "source": {
                "commit": "0000000000000000000000000000000000000000",
                "tree": "1111111111111111111111111111111111111111"
            },
            "verifier": { "identity": "test-inventory", "key_fingerprint": null },
            "runner_lock": null,
            "probes": {
                "macos_arm64": {
                    "runner_id": "forgejo-macos-arm64",
                    "os": "macos",
                    "architecture": "arm64",
                    "display": "none",
                    "clean_install_host": unavailable(),
                    "capability": unavailable()
                },
                "linux_x86_64_x11": {
                    "runner_id": "forgejo-linux-x86-64-x11",
                    "os": "linux",
                    "architecture": "x86_64",
                    "display": "x11",
                    "clean_install_host": unavailable(),
                    "capability": unavailable()
                },
                "macos_x86_64": {
                    "state": "not_required",
                    "observed_at_unix_ms": observed_at,
                    "evidence": null
                },
                "linux_x86_64_wayland": {
                    "state": "not_required",
                    "observed_at_unix_ms": observed_at,
                    "evidence": null
                },
                "apple": {
                    "certificate_sha256": null,
                    "team_id": null,
                    "certificate_expires_at_unix_ms": null,
                    "private_key_challenge": unavailable(),
                    "notarization_challenge": unavailable()
                },
                "linux_gpg": { "fingerprint": null, "signing_challenge": unavailable() },
                "protected_tag_and_owner_approval": {
                    "protected_pattern": "v*",
                    "manual_owner_approval": unavailable()
                },
                "artifact_retention": {
                    "pr_days": 30,
                    "release_days": 365,
                    "maximum_artifact_bytes": 104857600,
                    "truncation_fails": true,
                    "policy": unavailable()
                }
            },
            "result": {
                "ready": false,
                "hard_blockers": ["inventory-result-present"]
            }
        })
    }

    fn load_inventory() -> Preflight {
        load_and_validate_bytes(&serde_json::to_vec(&inventory_json()).unwrap()).unwrap()
    }

    fn validated_inventory_with_current_source(run_id: &str) -> Preflight {
        let mut input = inventory_json();
        let output =
            tool_process::git(Path::new("."), &["rev-parse", "HEAD", "HEAD^{tree}"], &[]).unwrap();
        let mut lines = std::str::from_utf8(&output.stdout).unwrap().lines();
        input["source"]["commit"] = serde_json::json!(lines.next().unwrap());
        input["source"]["tree"] = serde_json::json!(lines.next().unwrap());
        input["run_id"] = serde_json::json!(run_id);
        load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap()
    }

    #[test]
    fn caller_controlled_verdict_is_rejected() {
        assert!(load_and_validate_bytes(br#"{"schema":"rutile.release-prerequisite-preflight.v1","version":1,"run_id":"forged","verified":true}"#).is_err());
    }

    #[test]
    fn missing_result_is_rejected_by_shape_check() {
        let mut input = inventory_json();
        input.as_object_mut().unwrap().remove("result");
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(
            error.to_string().contains("missing required field /result"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn missing_required_nullable_runner_lock_is_rejected() {
        let mut input = inventory_json();
        input.as_object_mut().unwrap().remove("runner_lock");
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing required field /runner_lock"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn empty_hard_blockers_is_rejected() {
        let mut input = inventory_json();
        input["result"]["hard_blockers"] = serde_json::json!([]);
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("result.hard_blockers must contain 1-32 items"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn forged_derived_result_is_overwritten_and_real_result_is_blocked() {
        let mut input = inventory_json();
        input["result"] = serde_json::json!({"ready": false, "hard_blockers": ["forged-blocker"]});
        let report = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(!report.result.ready);
        assert!(
            report
                .result
                .hard_blockers
                .iter()
                .any(|blocker| blocker.contains("authenticated release authority"))
        );
        assert!(
            !report
                .result
                .hard_blockers
                .iter()
                .any(|b| b == "forged-blocker")
        );
        let serialized = serde_json::to_value(&report).unwrap();
        assert_eq!(serialized["result"]["ready"], false);
    }

    #[test]
    fn validate_rejects_caller_asserted_ready_result() {
        let mut input = inventory_json();
        input["result"] =
            serde_json::json!({"ready": true, "hard_blockers": ["should-not-matter"]});
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(error.to_string().contains("cannot assert a ready result"));
    }

    #[test]
    fn blocked_inventory_rejects_all_attested_claims() {
        let mut input = inventory_json();
        input["probes"]["macos_arm64"]["capability"]["state"] = serde_json::json!("attested");
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(error.to_string().contains("cannot contain attested"));
    }

    #[test]
    fn blocked_inventory_rejects_caller_asserted_authority() {
        let mut input = inventory_json();
        input["runner_lock"] = serde_json::json!({
            "logical_id": "fake-runner-lock",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        });
        let error = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot assert verifier or runner")
        );
    }

    #[test]
    fn file_loader_rejects_fabricated_repository_source() {
        let directory = temp_root();
        let input = directory.path().join("inventory.json");
        fs::write(&input, serde_json::to_vec(&inventory_json()).unwrap()).unwrap();
        let error = load_and_validate(&input).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("do not match the current repository")
        );
    }

    #[test]
    fn stale_inventory_is_retained_but_cannot_be_ready() {
        let mut input = inventory_json();
        input["generated_at_unix_ms"] = serde_json::json!(0);
        let report = load_and_validate_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(!report.result.ready);
        assert!(
            report
                .result
                .hard_blockers
                .iter()
                .any(|blocker| blocker.contains("stale"))
        );
    }
    #[test]
    fn unknown_fields_and_stale_or_incomplete_records_are_rejected() {
        for input in [br#"{"schema":"rutile.release-prerequisite-preflight.v1","version":1,"unexpected":true}"#.as_slice(), br#"{"schema":"rutile.release-prerequisite-preflight.v1","version":1,"run_id":"x"}"#.as_slice()] { assert!(load_and_validate_bytes(input).is_err()); }
    }
    #[test]
    fn logs_and_identity_fields_reject_absolute_paths_and_secrets() {
        assert!(validate_logical_id("/private/tmp/secret").is_err());
        assert!(validate_logical_id("token=secret").is_err());
        assert!(validate_logical_id("SECRET-VALUE").is_err());
        assert!(validate_logical_id("preflight/logs/macos-arm64-001").is_ok());
        // Case-insensitive path and keyword guards.
        assert!(validate_logical_id("/USERS/bob").is_err());
        assert!(validate_logical_id("/Home/bob").is_err());
        assert!(validate_logical_id("/PRIVATE/tmp").is_err());
        assert!(validate_logical_id("my-credentials").is_err());
        assert!(validate_logical_id("MY-PASSWORD").is_err());
    }

    #[test]
    fn logical_ids_reject_private_loopback_and_link_local_ips() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "172.31.255.255",
            "169.254.0.1",
        ] {
            assert!(validate_logical_id(ip).is_err(), "{ip} should be rejected");
            assert!(
                validate_logical_id(&format!("preflight/logs/{ip}")).is_err(),
                "nested {ip} should be rejected"
            );
            assert!(
                validate_logical_id(&format!("host-{ip}")).is_err(),
                "prefixed {ip} should be rejected"
            );
        }
        // IPv6 literals are already rejected by the allowed-character filter,
        // but the schema still catches loopback/link-local/unique-local shapes.
        assert!(validate_logical_id("::1").is_err());
        assert!(validate_logical_id("fe80::1").is_err());
        assert!(validate_logical_id("fd00::1").is_err());
        assert!(validate_logical_id("fc00::1").is_err());
        assert!(validate_logical_id("8.8.8.8").is_ok());
        assert!(validate_logical_id("v8.8.8.8").is_ok());
        assert!(validate_logical_id("forgejo-runner-001").is_ok());
    }

    #[test]
    fn schema_logical_id_pattern_is_case_insensitive_for_secrets() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate lives in the workspace");
        for path in [
            "schemas/rutile.release-prerequisite-preflight.v1.schema.json",
            "schemas/rutile.w0b-stage0-blocked-receipt.v1.schema.json",
        ] {
            let full = workspace.join(path);
            let schema: serde_json::Value =
                serde_json::from_slice(&fs::read(&full).unwrap()).unwrap();
            let pattern = schema["$defs"]["logicalId"]["allOf"][0]["not"]["pattern"]
                .as_str()
                .unwrap();
            let lower = pattern.to_ascii_lowercase();
            assert!(lower.contains("token"), "{path}: missing token guard");
            assert!(lower.contains("secret"), "{path}: missing secret guard");
            assert!(
                lower.contains("credentials"),
                "{path}: missing credentials guard"
            );
            assert!(lower.contains("password"), "{path}: missing password guard");
            assert!(
                pattern.contains("(?i:"),
                "{path}: pattern must be case-insensitive"
            );
            assert!(lower.contains("/users/"), "{path}: missing /users/ guard");
            assert!(lower.contains("/home/"), "{path}: missing /home/ guard");
            assert!(
                lower.contains("/private/"),
                "{path}: missing /private/ guard"
            );
            assert!(pattern.contains("127\\."), "{path}: missing loopback guard");
            assert!(
                !pattern.contains("^127"),
                "{path}: IP guard must not be start-anchored"
            );
            assert!(
                pattern.contains("\\d"),
                "{path}: IP guard must require octets"
            );
        }
    }

    #[test]
    fn oversized_input_and_existing_or_symlink_outputs_are_rejected() {
        assert!(matches!(
            load_and_validate_bytes(&vec![b' '; MAX_INPUT_BYTES as usize + 1]),
            Err(PreflightError::TooLarge)
        ));
        let report = validated_inventory_with_current_source("nested/logical-id");
        let directory = temp_root();
        let output = directory.path().join("retained.json");
        let outcome = publish_create_only(&output, &report).unwrap();
        assert!(outcome.durable);
        assert!(outcome.warnings.is_empty());
        assert!(publish_create_only(&output, &report).is_err());
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
                .count(),
            0
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&output, directory.path().join("alias.json")).unwrap();
            assert!(publish_create_only(&directory.path().join("alias.json"), &report).is_err());
        }
    }

    #[test]
    fn publish_create_only_uses_random_temp_name_independent_of_run_id() {
        let report = validated_inventory_with_current_source("caller-run-id");
        let directory = temp_root();
        let output_a = directory.path().join("a.json");
        let output_b = directory.path().join("b.json");

        publish_create_only(&output_a, &report).unwrap();
        publish_create_only(&output_b, &report).unwrap();

        // No leftover temporary files; the committed files are the only contents.
        let entries: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&"a.json".to_string()));
        assert!(entries.contains(&"b.json".to_string()));

        // The temporary basename is not derived from the caller-controlled run_id;
        // no file in the directory is named after it.
        for entry in fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains("caller-run-id"),
                "temp name must not contain caller run_id: {name}"
            );
        }
    }

    #[test]
    fn publish_create_only_rejects_fabricated_source_when_called_directly() {
        let mut report = load_inventory();
        report.run_id = "direct-call-fabricated-source".into();
        let directory = temp_root();
        let output = directory.path().join("retained.json");
        assert!(publish_create_only(&output, &report).is_err());
    }

    #[test]
    fn publish_create_only_rejects_caller_asserted_authority_when_called_directly() {
        let mut report = validated_inventory_with_current_source("direct-call-forged-authority");
        // A Rust caller cannot bypass authority rejection by mutating after validation.
        report.verifier.key_fingerprint = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let directory = temp_root();
        let output = directory.path().join("retained.json");
        assert!(publish_create_only(&output, &report).is_err());
    }

    #[test]
    fn verify_source_in_repo_ignores_git_replace_refs() {
        let repo = temp_root();
        let env = [
            ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.com"),
            ("GIT_COMMITTER_NAME", "Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.com"),
        ];
        let git = |args: &[&str]| -> String {
            let out = tool_process::git(repo.path(), args, &env).unwrap();
            assert!(out.status.success(), "git {args:?} failed");
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };

        git(&["init", "-q"]);
        fs::write(repo.path().join("a.txt"), b"a\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "a"]);
        let commit_a = git(&["rev-parse", "HEAD"]);
        let tree_a = git(&["rev-parse", "HEAD^{tree}"]);

        fs::write(repo.path().join("b.txt"), b"b\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "b"]);
        let commit_b = git(&["rev-parse", "HEAD"]);
        let tree_b = git(&["rev-parse", "HEAD^{tree}"]);

        // Build a replacement commit that rewinds the tree to tree_a.
        let replacement_args = vec![
            "commit-tree",
            tree_a.as_str(),
            "-p",
            commit_a.as_str(),
            "-m",
            "replacement",
        ];
        let replacement = git(&replacement_args);
        let replace_args = vec!["replace", commit_b.as_str(), replacement.as_str()];
        git(&replace_args);

        // Without --no-replace-objects the ambient tree would now be tree_a.
        assert_eq!(
            git(&["rev-parse", "HEAD^{tree}"]),
            tree_a,
            "replacement ref should be active in the test repo"
        );

        // Source binding must ignore replacement refs and use the canonical tree.
        assert!(
            verify_source_in_repo(
                repo.path(),
                &Source {
                    commit: commit_b.clone(),
                    tree: tree_b.clone(),
                }
            )
            .is_ok()
        );
        assert!(
            verify_source_in_repo(
                repo.path(),
                &Source {
                    commit: commit_b,
                    tree: tree_a,
                }
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn publish_create_only_rejects_insecure_output_parent() {
        use std::os::unix::fs::PermissionsExt;
        let report = validated_inventory_with_current_source("insecure-parent-test");
        let root = temp_root();
        let parent = root.path().join("group-writable");
        fs::create_dir_all(&parent).unwrap();
        let mut permissions = fs::metadata(&parent).unwrap().permissions();
        permissions.set_mode(0o777);
        fs::set_permissions(&parent, permissions).unwrap();
        let output = parent.join("retained.json");
        let error = publish_create_only(&output, &report).unwrap_err();
        assert!(
            error.to_string().contains("writable directory")
                || error.to_string().contains("private directory"),
            "unexpected error: {error}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn publish_create_only_rejects_symlink_parent() {
        let report = validated_inventory_with_current_source("symlink-parent-test");
        let root = temp_root();
        let attacker = root.path().join("attacker");
        fs::create_dir_all(&attacker).unwrap();
        let link = root.path().join("parent");
        std::os::unix::fs::symlink(&attacker, &link).unwrap();
        let output = link.join("retained.json");
        let error = publish_create_only(&output, &report).unwrap_err();
        assert!(
            error.to_string().contains("real directory")
                || error.to_string().contains("private directory"),
            "unexpected error: {error}"
        );
        assert!(!attacker.join("retained.json").exists());
    }

    #[test]
    #[cfg(unix)]
    fn publish_create_only_rejects_symlinked_ancestor() {
        let report = validated_inventory_with_current_source("symlink-ancestor-test");
        let root = temp_root();
        let base = root.path().join("base");
        let real_child = base.join("child");
        fs::create_dir_all(&real_child).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&base, &link).unwrap();

        // The final parent component is a real directory, but an ancestor is a
        // symlink. A pathname-based open would follow the symlink and land in
        // `base/child`; the component walk must reject the symlinked ancestor.
        let output = link.join("child").join("retained.json");
        let error = publish_create_only(&output, &report).unwrap_err();
        assert!(
            error.to_string().contains("real directory"),
            "unexpected error: {error}"
        );
        assert!(!real_child.join("retained.json").exists());
    }

    #[test]
    #[cfg(unix)]
    fn publish_create_only_survives_parent_path_rename() {
        let report = validated_inventory_with_current_source("parent-rename-test");
        let root = temp_root();
        let parent = root.path().join("parent");
        fs::create_dir_all(&parent).unwrap();
        let attacker = root.path().join("attacker");
        fs::create_dir_all(&attacker).unwrap();

        // Acquire a validated dirfd, then simulate an attacker renaming the
        // parent directory and reusing the path name for a symlink to another
        // directory. A pathname-based second open would follow the symlink.
        let dirfd = open_private_dirfd(&parent).unwrap();
        fs::rename(&parent, root.path().join("parent_original")).unwrap();
        std::os::unix::fs::symlink(&attacker, &parent).unwrap();

        let mut file = openat_exclusive(&dirfd, std::ffi::OsStr::new("retained.json")).unwrap();
        file.write_all(b"{\"stable\":true}\n").unwrap();
        file.sync_all().unwrap();

        assert!(
            root.path()
                .join("parent_original")
                .join("retained.json")
                .exists(),
            "write must land in the original directory, not the renamed path"
        );
        assert!(
            !attacker.join("retained.json").exists(),
            "write must not follow the symlink that now occupies the path name"
        );

        // The public API only sees the path name. Once the path has been
        // replaced by a symlink, the pathname must be rejected rather than
        // followed to the attacker's directory.
        let output = parent.join("retained.json");
        let error = publish_create_only(&output, &report).unwrap_err();
        assert!(
            error.to_string().contains("real directory"),
            "public API must reject the symlinked path name: {error}"
        );
        assert!(!attacker.join("retained.json").exists());
    }
}
