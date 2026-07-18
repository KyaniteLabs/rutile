//! Readiness keystone: verify-and-publish for externally signed attestations.
//!
//! This is the G002 Phase B readiness keystone. It consumes an externally
//! signed `rutile.readiness-attestation.v1` JSON file (produced off-repo by
//! the G004 verifier lane using a separate operator and provisioning host),
//! validates schema and current git source, opens the committed production
//! runner lock through the existing [`crate::runner`] API, checks the lock's
//! SHA-256 equals the attestation's `runner_lock_sha256`, invokes the G001
//! pinned independent-verifier assessment, and — when requested — publishes
//! the verified attestation create-only to a bounded output path.
//!
//! # Authority boundary
//!
//! The keystone NEVER signs, NEVER generates keys, NEVER reads a secret, and
//! NEVER alters a caller's verdict. It is verification and durable
//! retention only. Publication authorization (release-authority signature)
//! is a separate, unimplemented lane here; publishing a verified readiness
//! attestation explicitly does NOT authorize tagging, releasing, or publicly
//! distributing the product, and does not set or imply `publication_authorized`.
//!
//! # Fail-closed posture
//!
//! Every public entrypoint fails closed with [`KeystoneError::UnprovisionedTrust`]
//! until G004 provisions the pinned independent trusted-verifier public key
//! at [`crate::readiness::DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH`] and the
//! production runner trust is provisioned. The keystone never falls back to a
//! weaker verifier, never trusts caller-supplied trust material, and never
//! publishes a partial or unverified attestation.

#[cfg(unix)]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::readiness::{
    DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH, PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT,
    ReadinessAttestationV1, ReadinessError, ReadinessSourceV1,
    assess_readiness_with_trusted_key_file,
};
use crate::runner::{self, RunnerError};
use crate::tool_process;

/// Maximum byte length of a readiness attestation input or output file.
/// Bounds file reads and writes so a concurrently growing input cannot feed
/// unbounded data between `fstat` and `read_to_end`, and so the published
/// output cannot exceed the verified envelope.
pub const MAX_ATTESTATION_BYTES: u64 = 1024 * 1024;

/// Clock-skew tolerance: an attestation whose `signed_at_unix_ms` lies up to
/// this far in the future is still accepted, mirroring the preflight lane.
/// Anything further ahead is rejected as [`KeystoneError::Future`].
const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Typed keystone failure. Every variant is fail-closed: the keystone never
/// publishes a partial or unverified attestation and never silently downgrades
/// a verification failure into a success.
#[derive(Debug, Error)]
pub enum KeystoneError {
    /// Input attestation file does not exist (or cannot be opened).
    #[error("readiness attestation input file is missing")]
    Missing,
    /// Input attestation file exceeds [`MAX_ATTESTATION_BYTES`].
    #[error("readiness attestation input exceeds {MAX_ATTESTATION_BYTES} bytes")]
    Oversized,
    /// Input bytes are not valid JSON or do not match the readiness
    /// attestation v1 shape.
    #[error("readiness attestation JSON is malformed: {0}")]
    Malformed(String),
    /// Input or output path is a symlink, hardlink, or otherwise not a
    /// regular file in a real directory.
    #[error("readiness attestation path must be a regular file (symlink/hardlink rejected)")]
    Symlink,
    /// Output parent path traverses a symlink, is world/group-writable, is
    /// not owned by the current user, or is otherwise unsafe for create-only
    /// publication.
    #[error("output parent path is unsafe: {0}")]
    UnsafeParent(String),
    /// Current git source cannot be derived (e.g. `git rev-parse` failed).
    #[error("cannot derive current repository source: {0}")]
    SourceUnavailable(String),
    /// Attestation `expires_at_unix_ms` is in the past relative to wall-clock
    /// now. The attestation is stale and must not be accepted as readiness
    /// evidence.
    #[error("attestation has already expired (current time past expires_at_unix_ms)")]
    Expired,
    /// Attestation `signed_at_unix_ms` is unreasonably far in the future
    /// (beyond [`CLOCK_SKEW_MS`]).
    #[error("attestation signed_at_unix_ms is unreasonably far in the future")]
    Future,
    /// Attestation source (commit/tree) does not match the current repository
    /// source. Treated as a replay of a stale attestation against a different
    /// commit.
    #[error("attestation source does not match the current repository source (replay)")]
    Replayed,
    /// Attestation was not signed by the pinned independent trusted verifier
    /// (signature verification failed, trusted key mismatch, or the verifier
    /// is the release-authority key and therefore not independent).
    #[error("attestation verifier is not the pinned independent trusted verifier")]
    WrongSigner,
    /// Production runner trust is not provisioned, the trusted-verifier public
    /// key file is missing/invalid, or the production runner configuration is
    /// unprovisioned. Returned for every fail-closed unprovisioned state.
    #[error("production runner trust is unprovisioned")]
    UnprovisionedTrust,
    /// Committed runner lock SHA-256 does not equal the attestation's
    /// `runner_lock_sha256`. The attestation is bound to a different lock.
    #[error("committed runner lock SHA-256 does not match attestation runner_lock_sha256")]
    RunnerLockHashMismatch,
    /// Attestation declares `ready == false` or carries actionable blockers.
    /// The keystone only publishes ready attestations.
    #[error("attestation is not ready (actionable blockers remain)")]
    NotReady,
    /// Output path already exists. Publication is strictly create-only.
    #[error("output path already exists (create-only)")]
    AlreadyExists,
    /// Readiness assessment rejected the attestation for a reason that does
    /// not map to a more specific keystone category.
    #[error("readiness assessment rejected the attestation: {0}")]
    Readiness(String),
    /// Runner lock verification failed for an operational reason that does not
    /// map to a more specific keystone category.
    #[error("runner lock verification failed: {0}")]
    Runner(String),
    /// Underlying I/O failure.
    #[error("keystone I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Canonical JSON (de)serialization failure.
    #[error("keystone JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Receipt
// ---------------------------------------------------------------------------

/// Typed receipt returned by every successful keystone verification. Carries
/// the binding fields a caller needs to confirm what was verified, without
/// exposing any secret material.
///
/// `out_path` is [`Option::None`] for verify-only flows and [`Option::Some`]
/// for verify-and-publish flows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeystoneReceipt {
    /// Attestation source binding (commit + tree SHA-40).
    pub source: ReadinessSourceV1,
    /// SHA-256 of the canonical signed message. Equals
    /// `attestation.authority.canonical_message_sha256` decoded.
    pub canonical_message_sha256: [u8; 32],
    /// SHA-256 of the committed production runner lock that was bound to this
    /// attestation. Equals `attestation.runner_lock_sha256` decoded.
    pub runner_lock_sha256: [u8; 32],
    /// SHA-256 fingerprint (64-char lowercase hex) of the pinned independent
    /// trusted verifier public key that signed this attestation. Distinct from
    /// the pinned release-authority fingerprint.
    pub verifier_key_fingerprint: String,
    /// Whether the attestation declares readiness. The keystone only
    /// publishes attestations with `ready == true`; a not-ready attestation is
    /// rejected with [`KeystoneError::NotReady`] before publication.
    pub ready: bool,
    /// Output path the verified attestation was published to. [`Option::None`]
    /// for verify-only flows.
    pub out_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Dependency-injection seam
// ---------------------------------------------------------------------------

/// Closure that derives the caller's authoritative git source (commit + tree)
/// for replay binding.
pub type CurrentSourceFn<'a> = &'a dyn Fn() -> Result<ReadinessSourceV1, KeystoneError>;

/// Closure that opens the committed production runner lock at a path and
/// returns its SHA-256.
pub type VerifyRunnerLockFn<'a> = &'a dyn Fn(&Path) -> Result<[u8; 32], KeystoneError>;

/// Closure that publishes bytes create-only to a path.
pub type PublishFn<'a> = &'a dyn Fn(&Path, &[u8]) -> Result<(), KeystoneError>;

/// Injected seams for the verify-and-publish pipeline.
///
/// Production callers use [`verify_only`] / [`verify_and_publish`], which
/// construct internal deps bound to the pinned trusted-verifier public key
/// path, the production runner API, the production create-only publisher, and
/// the git-isolated source derivation. Tests inject alternative closures
/// through [`verify_only_with`] / [`verify_and_publish_with`] to bypass
/// provisioning and exercise rejection paths without a real runner lock or
/// trusted-verifier key file.
pub struct KeystoneDeps<'a> {
    /// Filesystem path to the pinned independent trusted-verifier public key
    /// (64-char lowercase hex, optional single trailing newline). Production
    /// uses [`crate::readiness::DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH`].
    pub trusted_verifier_public_key_path: &'a Path,
    /// Derive the caller's authoritative git source (commit + tree) for
    /// replay binding. Production wraps a `git rev-parse` invocation rooted at
    /// the workspace; tests typically return a fixed source matching the
    /// fixture attestation.
    pub current_source: CurrentSourceFn<'a>,
    /// Open the committed production runner lock at the given path and return
    /// its SHA-256. Production wraps [`runner::open_committed_runner_lock`].
    pub verify_runner_lock: VerifyRunnerLockFn<'a>,
    /// Publish the verified attestation bytes to the output path create-only.
    /// Production uses the dirfd-bound create-only publisher; tests typically
    /// write to a temp path. Must fail with [`KeystoneError::AlreadyExists`]
    /// if the path already exists.
    pub publish: PublishFn<'a>,
}

// ---------------------------------------------------------------------------
// Public entrypoints
// ---------------------------------------------------------------------------

/// Verify an externally signed readiness attestation against the current git
/// source and committed production runner lock, WITHOUT publishing.
///
/// Returns Ok for both ready and not-ready attestations so operators can
/// inspect blockers via [`KeystoneReceipt::ready`]. Use [`verify_and_publish`]
/// to apply the publish gate (reject NotReady).
///
/// Fails closed with [`KeystoneError::UnprovisionedTrust`] until G004
/// provisions the trusted-verifier public key at
/// [`crate::readiness::DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH`] and the
/// production runner trust is provisioned.
///
/// On success, returns a [`KeystoneReceipt`] with `out_path == None`.
pub fn verify_only(
    input: &Path,
    runner_lock_path: &Path,
) -> Result<KeystoneReceipt, KeystoneError> {
    verify_only_with(input, runner_lock_path, &production_deps())
}

/// Verify an externally signed readiness attestation against the current git
/// source and committed production runner lock, then publish the verified
/// attestation create-only to `out`.
///
/// Re-runs every verification check immediately before publication; a receipt
/// returned by [`verify_only`] is NEVER implicitly trusted for publication.
/// Fails closed with [`KeystoneError::UnprovisionedTrust`] until G004
/// provisions the trusted-verifier public key and the production runner trust.
/// Fails with [`KeystoneError::AlreadyExists`] if `out` already exists.
///
/// On success, returns a [`KeystoneReceipt`] with `out_path == Some(out)`.
pub fn verify_and_publish(
    input: &Path,
    runner_lock_path: &Path,
    out: &Path,
) -> Result<KeystoneReceipt, KeystoneError> {
    verify_and_publish_with(input, runner_lock_path, out, &production_deps())
}

/// Load and parse a readiness attestation JSON file. Validates that the input
/// is a regular file (no symlink/hardlink), bounded by
/// [`MAX_ATTESTATION_BYTES`], and parses as a
/// `rutile.readiness-attestation.v1` instance. Does NOT verify the signature,
/// source binding, or runner lock — use [`verify_only`] / [`verify_and_publish`]
/// for the full pipeline.
pub fn load_attestation(path: &Path) -> Result<ReadinessAttestationV1, KeystoneError> {
    let bytes = read_regular_file(path)?;
    let claim: ReadinessAttestationV1 =
        serde_json::from_slice(&bytes).map_err(|e| KeystoneError::Malformed(e.to_string()))?;
    Ok(claim)
}

// Note: trusted-verifier public-key loading is the readiness module's
// authoritative responsibility (load_trusted_verifier_public_key is private
// to crate::readiness and consumed internally by assess_readiness_*). The
// keystone does NOT duplicate that loader: it delegates through
// assess_readiness_with_trusted_key_file, which fails closed with
// ReadinessError::TrustedKeyMissing / TrustedKeyInvalid when the pinned key
// file is absent or malformed.

// ---------------------------------------------------------------------------
// pub(crate) seam-driven workers (for tests)
// ---------------------------------------------------------------------------

/// Verify-only worker with explicit dependency seams. Production delegates
/// here via [`verify_only`]; tests call this directly to bypass provisioning.
pub(crate) fn verify_only_with(
    input: &Path,
    runner_lock_path: &Path,
    deps: &KeystoneDeps,
) -> Result<KeystoneReceipt, KeystoneError> {
    let claim = load_attestation(input)?;
    let expected_source = (deps.current_source)()?;
    let lock_sha = (deps.verify_runner_lock)(runner_lock_path)?;
    assess_and_bind(&claim, &expected_source, &lock_sha, deps).map(|receipt| KeystoneReceipt {
        out_path: None,
        ..receipt
    })
}

/// Verify-and-publish worker with explicit dependency seams. Production
/// delegates here via [`verify_and_publish`]; tests call this directly.
pub(crate) fn verify_and_publish_with(
    input: &Path,
    runner_lock_path: &Path,
    out: &Path,
    deps: &KeystoneDeps,
) -> Result<KeystoneReceipt, KeystoneError> {
    let claim = load_attestation(input)?;
    let expected_source = (deps.current_source)()?;
    let lock_sha = (deps.verify_runner_lock)(runner_lock_path)?;
    let receipt = assess_and_bind(&claim, &expected_source, &lock_sha, deps)?;

    // Publish gate: only ready attestations may be retained. verify_only
    // accepts not-ready claims for inspection, but publishing a not-ready
    // attestation would publish a non-passing readiness state.
    if !receipt.ready {
        return Err(KeystoneError::NotReady);
    }

    // Re-serialize the parsed, verified attestation to canonical JSON so the
    // published bytes are exactly what the keystone validated. Round-tripping
    // through the parser defeats any whitespace/key-order tricks in the input.
    let canonical = canonical_attestation_bytes(&claim)?;
    validate_output_path(out)?;
    (deps.publish)(out, &canonical)?;

    Ok(KeystoneReceipt {
        out_path: Some(out.to_path_buf()),
        ..receipt
    })
}

// ---------------------------------------------------------------------------
// Core pipeline (shared by verify-only and verify-and-publish)
// ---------------------------------------------------------------------------

fn assess_and_bind(
    claim: &ReadinessAttestationV1,
    expected_source: &ReadinessSourceV1,
    lock_sha: &[u8; 32],
    deps: &KeystoneDeps,
) -> Result<KeystoneReceipt, KeystoneError> {
    // G001 independent-verifier assessment: schema/version/disclaimer/source/
    // probes/verifier/independence/trusted-binding/authority/freshness/ready/
    // canonical-hash/signature. Fail-closed map: every ReadinessError becomes
    // a specific KeystoneError category; no verdict is silently downgraded.
    let assessed = assess_readiness_with_trusted_key_file(
        claim,
        deps.trusted_verifier_public_key_path,
        PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT,
        expected_source,
    )
    .map_err(map_readiness_error)?;

    // Runner-lock hash binding: the committed production runner lock we just
    // opened must hash to exactly the attestation's `runner_lock_sha256`. The
    // readiness assessment has already validated that the field is 64-char
    // lowercase hex; decode and compare against the opened lock's SHA-256.
    let claimed_lock_sha = hex::decode(&claim.runner_lock_sha256)
        .map_err(|_| KeystoneError::RunnerLockHashMismatch)?;
    if claimed_lock_sha.as_slice() != lock_sha.as_slice() {
        return Err(KeystoneError::RunnerLockHashMismatch);
    }

    // Wall-clock freshness (not covered by the G001 lane, which only checks
    // internal consistency). The keystone refuses stale and far-future
    // attestations against the verifier's wall clock.
    let now = now_unix_ms();
    if now >= claim.authority.expires_at_unix_ms {
        return Err(KeystoneError::Expired);
    }
    if claim.authority.signed_at_unix_ms > now.saturating_add(CLOCK_SKEW_MS) {
        return Err(KeystoneError::Future);
    }

    // verify_only returns Ok for ready AND not-ready claims so operators can
    // inspect blockers; the receipt's `ready` field surfaces the state.
    // verify_and_publish applies the publish-gate policy (reject NotReady).

    Ok(KeystoneReceipt {
        source: claim.source.clone(),
        canonical_message_sha256: assessed.canonical_message_sha256,
        runner_lock_sha256: *lock_sha,
        verifier_key_fingerprint: claim.verifier.key_fingerprint.clone(),
        ready: claim.ready,
        out_path: None,
    })
}

// ---------------------------------------------------------------------------
// Production deps construction
// ---------------------------------------------------------------------------

fn production_deps() -> KeystoneDeps<'static> {
    // The trusted-verifier key path is rooted at the compile-time workspace
    // root, never at the runtime working directory. This prevents a caller
    // from redirecting the pinned trust anchor by changing cwd.
    KeystoneDeps {
        trusted_verifier_public_key_path: production_trusted_verifier_key_path(),
        current_source: &current_source_closure,
        verify_runner_lock: &production_verify_runner_lock,
        publish: &production_publish,
    }
}

/// Compute the workspace-rooted pinned trusted-verifier public key path once
/// and cache it in a `'static` reference. The path is
/// `workspace_root().join(DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH)`, never
/// cwd-relative.
fn production_trusted_verifier_key_path() -> &'static Path {
    static CACHED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| workspace_root().join(DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH))
        .as_path()
}

fn current_source_closure() -> Result<ReadinessSourceV1, KeystoneError> {
    let repo = workspace_root();
    let output = tool_process::git_isolated(
        repo,
        &["--no-replace-objects", "rev-parse", "HEAD", "HEAD^{tree}"],
        &[],
    )
    .map_err(|e| KeystoneError::SourceUnavailable(e.to_string()))?;
    if !output.status.success() || output.stdout.len() > 256 {
        return Err(KeystoneError::SourceUnavailable(
            "git rev-parse failed or produced unexpected output".into(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| KeystoneError::SourceUnavailable("git output is not UTF-8".into()))?;
    let mut lines = text.lines();
    let commit = lines
        .next()
        .ok_or_else(|| KeystoneError::SourceUnavailable("git produced no commit line".into()))?;
    let tree = lines
        .next()
        .ok_or_else(|| KeystoneError::SourceUnavailable("git produced no tree line".into()))?;
    if lines.next().is_some() {
        return Err(KeystoneError::SourceUnavailable(
            "git produced unexpected trailing output".into(),
        ));
    }
    if commit.len() != 40 || tree.len() != 40 {
        return Err(KeystoneError::SourceUnavailable(
            "git produced a non-40-char object id".into(),
        ));
    }
    Ok(ReadinessSourceV1 {
        commit: commit.to_string(),
        tree: tree.to_string(),
    })
}

fn production_verify_runner_lock(path: &Path) -> Result<[u8; 32], KeystoneError> {
    let summary = runner::open_committed_runner_lock(path).map_err(map_runner_error)?;
    Ok(summary.lock_sha256)
}

// ---------------------------------------------------------------------------
// File I/O helpers
// ---------------------------------------------------------------------------

/// Pinned repository root derived at compile time from the `xtask` crate
/// location. Source binding must never follow the runtime working directory or
/// inherited Git environment, so `current_source` always roots at this path.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate lives in the workspace")
}

/// Read a regular file, bounded by [`MAX_ATTESTATION_BYTES`]. Rejects symlinks,
/// hardlinks, and non-regular files via `O_NOFOLLOW` + `fstat` on Unix. A
/// concurrently growing file cannot feed unbounded data between `fstat` and
/// `read_to_end` because of the hard read cap.
///
/// On non-Unix targets the function fails closed: the TOCTOU-vulnerable
/// `symlink_metadata` + `File::open` sequence cannot meet the same
/// symlink/hardlink rejection guarantee, so the keystone refuses to read
/// rather than silently downgrading to a weaker check.
#[cfg(unix)]
fn read_regular_file(path: &Path) -> Result<Vec<u8>, KeystoneError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| KeystoneError::Malformed("input path is not valid".into()))?;
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
            Some(libc::ELOOP) => KeystoneError::Symlink,
            Some(libc::ENOENT) => KeystoneError::Missing,
            _ => KeystoneError::Io(err),
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } < 0 {
        return Err(KeystoneError::Io(std::io::Error::last_os_error()));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(KeystoneError::Symlink);
    }
    if stat.st_nlink != 1 {
        // Reject hardlinks: the input must be a uniquely-owned regular file
        // so a concurrent publisher cannot mutate it through another name.
        return Err(KeystoneError::Symlink);
    }
    if stat.st_size as u64 > MAX_ATTESTATION_BYTES {
        return Err(KeystoneError::Oversized);
    }
    let mut bytes = Vec::new();
    file.take(MAX_ATTESTATION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(KeystoneError::Io)?;
    if bytes.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(KeystoneError::Oversized);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_regular_file(_path: &Path) -> Result<Vec<u8>, KeystoneError> {
    // Fail closed: the non-Unix TOCTOU-vulnerable metadata+open sequence
    // cannot meet the same symlink/hardlink rejection guarantee as the
    // O_NOFOLLOW + fstat approach. The keystone refuses to read rather than
    // silently downgrade to a weaker check.
    Err(KeystoneError::Io(std::io::Error::other(
        "readiness keystone safe file I/O is unix-only (O_NOFOLLOW + fstat)",
    )))
}

/// Validate the output path before publication: must have a file name, must
/// not already exist, and its parent directory must be a real, private
/// directory. The dirfd-bound publisher repeats the existence check under the
/// directory lock, so this is the caller-facing fast path.
fn validate_output_path(out: &Path) -> Result<(), KeystoneError> {
    if out.file_name().map(|n| n.is_empty()).unwrap_or(true) {
        return Err(KeystoneError::UnsafeParent(
            "output requires a file name".into(),
        ));
    }
    if out.exists() {
        return Err(KeystoneError::AlreadyExists);
    }
    Ok(())
}

/// Re-serialize the verified attestation to canonical pretty JSON with a
/// trailing newline. Round-trips through the parser so the published bytes are
/// exactly what the keystone validated, regardless of input whitespace or
/// key order. Bounded by [`MAX_ATTESTATION_BYTES`] since the parsed structure
/// is bounded by the input cap.
fn canonical_attestation_bytes(claim: &ReadinessAttestationV1) -> Result<Vec<u8>, KeystoneError> {
    let mut bytes = serde_json::to_vec_pretty(claim)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(KeystoneError::Oversized);
    }
    Ok(bytes)
}

/// Production create-only publisher. Writes the bytes to a private random
/// temp file in the output parent directory, syncs it, then hard-links it to
/// the final name (rejecting existing). Walks every parent component with
/// `O_NOFOLLOW` so a symlinked ancestor cannot redirect the output, and
/// requires every component to be a real directory owned by the current user
/// with no group/other write bit. Mirrors the audited `release_preflight`
/// publisher contract.
///
/// On non-Unix targets the function fails closed: the TOCTOU-vulnerable
/// `OpenOptions::create_new` + `hard_link` sequence cannot meet the same
/// dirfd-bound symlink-rejection guarantee, so the keystone refuses to publish
/// rather than silently downgrading to a weaker publisher.
#[cfg(unix)]
fn production_publish(out: &Path, bytes: &[u8]) -> Result<(), KeystoneError> {
    use std::os::fd::{AsRawFd, IntoRawFd};

    if bytes.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(KeystoneError::Oversized);
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = out
        .file_name()
        .ok_or_else(|| KeystoneError::UnsafeParent("output requires a file name".into()))?;
    // `OsString` (not `String`): the unix publisher helpers below take
    // `&OsStr`, and `&OsString` derefs to `&OsStr`.
    let temp_name = std::ffi::OsString::from(random_temp_name()?);

    let dirfd = open_private_dirfd(parent)?;
    let mut file = openat_exclusive(&dirfd, &temp_name)?;
    let publication = (|| -> Result<(), KeystoneError> {
        file.write_all(bytes).map_err(KeystoneError::Io)?;
        file.sync_all().map_err(KeystoneError::Io)?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = unlinkat_name(&dirfd, &temp_name);
    }
    publication?;

    // Hard-link the fully-synced temp file to the final name. This fails
    // with EEXIST if the destination already exists, preserving
    // create-only semantics even under concurrent publishers.
    let link_result = linkat_name(&dirfd, &temp_name, file_name);
    let _ = unlinkat_name(&dirfd, &temp_name);
    link_result?;

    let file_fd = file.into_raw_fd();
    // SAFETY: file_fd is the raw fd we just extracted; closing it does not
    // touch any other resource. We treat a parent-dir fsync failure as a
    // durability loss and reject under the keystone's strict fail-closed
    // posture.
    if unsafe { libc::close(file_fd) } < 0 {
        return Err(KeystoneError::Io(std::io::Error::last_os_error()));
    }
    if unsafe { libc::fsync(dirfd.as_raw_fd()) } < 0 {
        return Err(KeystoneError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn production_publish(_out: &Path, _bytes: &[u8]) -> Result<(), KeystoneError> {
    // Fail closed: the non-Unix TOCTOU-vulnerable OpenOptions + hard_link
    // sequence cannot meet the same dirfd-bound symlink-rejection guarantee
    // as the O_NOFOLLOW + openat + linkat approach. The keystone refuses to
    // publish rather than silently downgrade to a weaker publisher.
    Err(KeystoneError::Io(std::io::Error::other(
        "readiness keystone safe publication is unix-only (dirfd + O_NOFOLLOW)",
    )))
}

/// Generate a caller-unpredictable temporary basename for create-only
/// publication. The name is never derived from any caller-controlled field.
#[cfg(unix)]
fn random_temp_name() -> Result<String, KeystoneError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| {
        KeystoneError::UnsafeParent(format!("failed to generate random temp name: {e}"))
    })?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(".readiness.tmp.{hex}"))
}

#[cfg(unix)]
fn open_private_dirfd(parent: &Path) -> Result<std::os::fd::OwnedFd, KeystoneError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

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
        return Err(KeystoneError::UnsafeParent(format!(
            "cannot open output parent root: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut dirfd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

    for component in parent.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(KeystoneError::UnsafeParent(
                    "output parent must not contain parent-directory references".into(),
                ));
            }
            std::path::Component::Normal(name) => {
                let c_name = CString::new(name.as_bytes()).map_err(|_| {
                    KeystoneError::UnsafeParent("output parent path is not valid".into())
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
                    return Err(KeystoneError::UnsafeParent(format!(
                        "output parent must be a real directory: {err}"
                    )));
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
fn validate_dirfd(dirfd: &std::os::fd::OwnedFd, require_owner: bool) -> Result<(), KeystoneError> {
    use std::os::fd::AsRawFd;

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(dirfd.as_raw_fd(), &mut stat) } < 0 {
        return Err(KeystoneError::UnsafeParent(format!(
            "fstat failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(KeystoneError::UnsafeParent(
            "output parent must be a real directory".into(),
        ));
    }
    if (stat.st_mode as u32 & 0o022) != 0 {
        return Err(KeystoneError::UnsafeParent(
            "output parent path traverses a writable directory".into(),
        ));
    }
    if require_owner && stat.st_uid != unsafe { libc::geteuid() } {
        return Err(KeystoneError::UnsafeParent(
            "output parent must be a private directory owned by the current user".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn openat_exclusive(
    dirfd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<std::fs::File, KeystoneError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_name = CString::new(name.as_bytes())
        .map_err(|_| KeystoneError::UnsafeParent("output temp name is not valid".into()))?;
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
            KeystoneError::AlreadyExists
        } else {
            KeystoneError::Io(err)
        });
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn unlinkat_name(dirfd: &std::os::fd::OwnedFd, name: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let c_name = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid temp name"))?;
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
) -> Result<(), KeystoneError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let c_from = CString::new(from.as_bytes())
        .map_err(|_| KeystoneError::UnsafeParent("invalid temporary name".into()))?;
    let c_to = CString::new(to.as_bytes())
        .map_err(|_| KeystoneError::UnsafeParent("invalid output name".into()))?;
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
        let err = std::io::Error::last_os_error();
        return Err(if err.raw_os_error() == Some(libc::EEXIST) {
            KeystoneError::AlreadyExists
        } else {
            KeystoneError::Io(err)
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn map_readiness_error(err: ReadinessError) -> KeystoneError {
    match err {
        // Trust provisioning failures: the keystone treats any absence or
        // invalidity of the pinned trusted-verifier key as unprovisioned,
        // matching the fail-closed-until-G004 contract.
        ReadinessError::TrustedKeyMissing
        | ReadinessError::TrustedKeyInvalid
        | ReadinessError::TrustedVerifierInvalid => KeystoneError::UnprovisionedTrust,
        // Verifier independence and signature failures: the attestation was
        // not signed by the expected independent trusted verifier.
        ReadinessError::VerifierNotIndependent
        | ReadinessError::TrustedVerifierMismatch
        | ReadinessError::SignatureVerification
        | ReadinessError::AuthoritySignature => KeystoneError::WrongSigner,
        // Source binding: replay of an attestation against a different commit.
        ReadinessError::SourceMismatch => KeystoneError::Replayed,
        // Freshness: future evidence or already-expired-at-issuance.
        ReadinessError::FutureEvidence => KeystoneError::Future,
        ReadinessError::ExpiredAtIssuance => KeystoneError::Expired,
        // All other ReadinessError variants surface as a generic Readiness
        // failure with the original message preserved for diagnostics.
        other => KeystoneError::Readiness(other.to_string()),
    }
}

fn map_runner_error(err: RunnerError) -> KeystoneError {
    match err {
        RunnerError::Unprovisioned | RunnerError::UnprovisionedTrust => {
            KeystoneError::UnprovisionedTrust
        }
        RunnerError::Io(e) => KeystoneError::Io(e),
        RunnerError::Json(e) => KeystoneError::Json(e),
        other => KeystoneError::Runner(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Small primitives
// ---------------------------------------------------------------------------

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::{
        PROBE_IDS, READINESS_ATTESTATION_SCHEMA, READINESS_DISCLAIMER, READINESS_DOMAIN_STR,
        READINESS_SCHEMA_VERSION, ReadinessAuthorityV1, ReadinessProbeV1, ReadinessVerifierV1,
        canonical_message,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use std::fs;
    use tempfile::{tempdir, tempdir_in};
    /// Base time for test claims, derived from wall-clock now so the keystone's
    /// wall-clock freshness checks (`now < expires`, `signed_at <= now + skew`)
    /// don't reject every fixture. Computed at test-runtime, not const.
    fn base_time() -> u64 {
        let now = now_unix_ms();
        // Use a deterministic offset within the test run; assert well after
        // unix epoch so `now - 60_000` cannot underflow.
        now.saturating_sub(60_000)
    }

    const SAMPLE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const SAMPLE_TREE: &str = "fedcba9876543210fedcba9876543210fedcba98";
    const SAMPLE_RUNNER_LOCK_SHA256: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_runner_lock_sha() -> [u8; 32] {
        [0xaa; 32]
    }

    /// Build a complete valid attestation signed by `signing_key` with
    /// `ready=true` and no blockers.
    fn build_valid_attestation(signing_key: &SigningKey) -> ReadinessAttestationV1 {
        build_attestation(signing_key, true, &[])
    }

    fn build_attestation(
        signing_key: &SigningKey,
        ready: bool,
        blockers: &[&str],
    ) -> ReadinessAttestationV1 {
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.to_bytes());
        let fingerprint = crate::readiness::sha256_hex(&verifying_key.to_bytes());
        // Base time is wall-clock-derived so the keystone's `now < expires`
        // and `signed_at <= now + CLOCK_SKEW_MS` checks don't reject fixtures.
        let base = base_time();

        let probes: Vec<ReadinessProbeV1> = PROBE_IDS
            .iter()
            .map(|id| ReadinessProbeV1 {
                id: (*id).to_string(),
                state: "attested".to_string(),
                observed_at_unix_ms: base,
                evidence_ref: format!("evidence/readiness/{id}.json"),
                evidence_sha256: hex::encode([0xbb; 32]),
            })
            .collect();

        let mut claim = ReadinessAttestationV1 {
            schema: READINESS_ATTESTATION_SCHEMA.to_string(),
            version: READINESS_SCHEMA_VERSION,
            generated_at_unix_ms: base,
            source: ReadinessSourceV1 {
                commit: SAMPLE_COMMIT.to_string(),
                tree: SAMPLE_TREE.to_string(),
            },
            runner_lock_ref: "locks/runner-lock.json".to_string(),
            runner_lock_sha256: SAMPLE_RUNNER_LOCK_SHA256.to_string(),
            probes,
            actionable_blockers: blockers.iter().map(|s| (*s).to_string()).collect(),
            verifier: ReadinessVerifierV1 {
                identity: "independent-readiness-verifier".to_string(),
                key_fingerprint: fingerprint,
                signing_public_key_hex: pubkey_hex,
                independence_evidence_ref: "evidence/readiness/verifier-independence.md"
                    .to_string(),
            },
            authority: ReadinessAuthorityV1 {
                domain: READINESS_DOMAIN_STR.to_string(),
                canonical_message_sha256: String::new(),
                signature_hex: String::new(),
                signed_at_unix_ms: base + 60_000,
                expires_at_unix_ms: base + 86_400_000,
            },
            ready,
            disclaimer: READINESS_DISCLAIMER.to_string(),
        };

        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        claim
    }

    fn expected_source() -> ReadinessSourceV1 {
        ReadinessSourceV1 {
            commit: SAMPLE_COMMIT.to_string(),
            tree: SAMPLE_TREE.to_string(),
        }
    }

    /// Write a key file in the release-key convention (64 lowercase hex,
    /// optional trailing newline).
    fn write_trusted_key(dir: &Path, signing_key: &SigningKey) -> PathBuf {
        let path = dir.join("trusted-verifier-v1.pub.hex");
        std::fs::write(&path, hex::encode(signing_key.verifying_key().to_bytes())).unwrap();
        path
    }

    fn write_attestation(dir: &Path, claim: &ReadinessAttestationV1) -> PathBuf {
        let path = dir.join("attestation.json");
        std::fs::write(&path, serde_json::to_vec_pretty(claim).unwrap()).unwrap();
        path
    }

    /// Build test deps with the given trusted key and runner-lock hash. The
    /// publisher writes to whatever path it's given (real fs).
    ///
    /// Uses thread-local storage + named functions instead of leaked closures
    /// so the `&dyn Fn(&Path, &[u8])` higher-ranked trait bounds are satisfied
    /// by concrete `fn` items (clippy rejects boxed-closure HRTB inference).
    fn test_deps<'a>(trusted_key_path: &'a Path, lock_sha: [u8; 32]) -> KeystoneDeps<'a> {
        INJECTED_LOCK_SHA.with(|cell| cell.set(lock_sha));
        // Reset the source override so a prior source-mismatch test running on
        // the same pooled thread can't leak its wrong source into this call.
        INJECTED_SOURCE.with(|cell| *cell.borrow_mut() = expected_source());
        KeystoneDeps {
            trusted_verifier_public_key_path: trusted_key_path,
            current_source: &expected_source_fn,
            verify_runner_lock: &injected_lock_sha_fn,
            publish: &publish_create_only_fn,
        }
    }

    thread_local! {
        /// Per-thread override of the current-source closure return value.
        /// `test_deps` resets this to `expected_source()`; the source-mismatch
        /// test sets it to a wrong source to exercise the replay rejection.
        static INJECTED_SOURCE: std::cell::RefCell<ReadinessSourceV1> = const {
            std::cell::RefCell::new(ReadinessSourceV1 {
                commit: String::new(),
                tree: String::new(),
            })
        };
        /// Per-thread runner-lock SHA returned by the verify-runner-lock seam.
        static INJECTED_LOCK_SHA: std::cell::Cell<[u8; 32]> =
            const { std::cell::Cell::new([0; 32]) };
    }

    fn expected_source_fn() -> Result<ReadinessSourceV1, KeystoneError> {
        Ok(expected_source())
    }

    fn wrong_source_fn() -> Result<ReadinessSourceV1, KeystoneError> {
        Ok(INJECTED_SOURCE.with(|cell| cell.borrow().clone()))
    }

    fn injected_lock_sha_fn(_path: &Path) -> Result<[u8; 32], KeystoneError> {
        Ok(INJECTED_LOCK_SHA.with(|cell| cell.get()))
    }

    fn publish_create_only_fn(out: &Path, bytes: &[u8]) -> Result<(), KeystoneError> {
        if out.exists() {
            return Err(KeystoneError::AlreadyExists);
        }
        std::fs::write(out, bytes).map_err(KeystoneError::Io)
    }

    fn publish_noop_fn(_out: &Path, _bytes: &[u8]) -> Result<(), KeystoneError> {
        Ok(())
    }

    /// Tempdir rooted under the workspace's `target/tmp/` so dirfd-walking
    /// publishers never traverse a host symlinked temp root (e.g. macOS
    /// `/var/folders -> /private/var/folders`). Mirrors `release_preflight`.
    fn temp_root() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate lives in the workspace")
            .join("target")
            .join("tmp");
        fs::create_dir_all(&root).unwrap();
        tempdir_in(&root).unwrap()
    }

    // -- Happy path ----------------------------------------------------------

    #[test]
    fn verify_only_with_succeeds_for_valid_attestation() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");

        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let receipt =
            verify_only_with(&input, &runner_lock_path, &deps).expect("valid attestation");
        assert!(receipt.ready);
        assert_eq!(receipt.out_path, None);
        assert_eq!(receipt.runner_lock_sha256, sample_runner_lock_sha());
        assert_eq!(receipt.source, expected_source());
        assert_eq!(
            receipt.verifier_key_fingerprint,
            claim.verifier.key_fingerprint
        );
        assert_eq!(
            &receipt.canonical_message_sha256[..],
            hex::decode(&claim.authority.canonical_message_sha256)
                .unwrap()
                .as_slice()
        );
    }

    #[test]
    fn verify_and_publish_with_writes_canonical_bytes() {
        let signing_key = SigningKey::from_bytes(&[0x02; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let out = dir.path().join("verified.json");

        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let receipt =
            verify_and_publish_with(&input, &runner_lock_path, &out, &deps).expect("publishes");
        assert_eq!(receipt.out_path, Some(out.clone()));

        let written = std::fs::read(&out).unwrap();
        // Trailing newline is part of the canonical envelope.
        assert!(written.ends_with(b"\n"));
        let reparsed: ReadinessAttestationV1 = serde_json::from_slice(&written).unwrap();
        assert_eq!(reparsed, claim);
    }

    // -- Input shape failures ------------------------------------------------

    #[test]
    fn missing_input_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let missing = dir.path().join("absent.json");
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&missing, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Missing), "{err:?}");
    }

    #[test]
    fn oversized_input_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = dir.path().join("huge.json");
        // Write a file that exceeds MAX_ATTESTATION_BYTES.
        let huge = vec![b' '; (MAX_ATTESTATION_BYTES + 1) as usize];
        std::fs::write(&input, &huge).unwrap();
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Oversized), "{err:?}");
    }

    #[test]
    fn malformed_json_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = dir.path().join("broken.json");
        std::fs::write(&input, b"{not json").unwrap();
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn wrong_schema_is_rejected_as_malformed() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.schema = "rutile.readiness-attestation.v2".to_string();
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        // readiness::assess_readiness returns ReadinessError::Schema, which
        // does not map to WrongSigner/Replayed/etc., so it surfaces as the
        // generic Readiness category.
        assert!(
            matches!(err, KeystoneError::Readiness(ref s) if s.contains("schema")),
            "{err:?}"
        );
    }

    // -- Symlink / hardlink rejection ---------------------------------------

    #[cfg(unix)]
    #[test]
    fn symlink_input_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let real = write_attestation(dir.path(), &claim);
        let symlink = dir.path().join("link.json");
        std::os::unix::fs::symlink(&real, &symlink).unwrap();
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&symlink, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Symlink), "{err:?}");
    }

    // -- Source mismatch (replay) -------------------------------------------

    #[test]
    fn source_mismatch_is_rejected_as_replay() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");

        let wrong_source = ReadinessSourceV1 {
            commit: "1111111111111111111111111111111111111111".to_string(),
            tree: SAMPLE_TREE.to_string(),
        };
        // Override the thread-local source so `wrong_source_fn` returns the
        // mismatched commit; `test_deps`-style closures would trip clippy's
        // HRTB-on-boxed-closure diagnostic, so named fns + thread_local are
        // used instead.
        INJECTED_SOURCE.with(|cell| *cell.borrow_mut() = wrong_source);
        INJECTED_LOCK_SHA.with(|cell| cell.set(sample_runner_lock_sha()));
        let deps = KeystoneDeps {
            trusted_verifier_public_key_path: &trusted_path,
            current_source: &wrong_source_fn,
            verify_runner_lock: &injected_lock_sha_fn,
            publish: &publish_noop_fn,
        };

        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Replayed), "{err:?}");
    }

    // -- Runner-lock hash mismatch ------------------------------------------

    #[test]
    fn runner_lock_hash_mismatch_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        // Different sha256 from what the attestation claims.
        let deps = test_deps(&trusted_path, [0xcc; 32]);
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(
            matches!(err, KeystoneError::RunnerLockHashMismatch),
            "{err:?}"
        );
    }

    // -- Wrong signer / verifier independence -------------------------------

    #[test]
    fn trusted_verifier_mismatch_is_wrong_signer() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        // Pinned trusted key is a DIFFERENT key than the one that signed.
        let other_key = SigningKey::from_bytes(&[0x02; 32]);
        let trusted_path = write_trusted_key(dir.path(), &other_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::WrongSigner), "{err:?}");
    }

    // Note: the keystone hard-pins PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT and
    // cannot be configured to treat an arbitrary test key as the release
    // authority. The independence rejection (release-authority key used as a
    // readiness verifier) is therefore covered by readiness.rs's own
    // release_authority_key_is_rejected_as_verifier test, and the keystone's
    // preservation of that category is covered by
    // map_readiness_error_preserves_categories below.

    // -- Freshness -----------------------------------------------------------

    #[test]
    fn expired_attestation_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Expiry in the past relative to wall-clock now.
        claim.authority.expires_at_unix_ms = 1; // 1970-01-01.
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());

        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Expired), "{err:?}");
    }

    #[test]
    fn far_future_signed_at_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Signed centuries in the future — exceeds CLOCK_SKEW_MS.
        claim.authority.signed_at_unix_ms = u64::MAX / 2;
        claim.authority.expires_at_unix_ms = u64::MAX;
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());

        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_only_with(&input, &runner_lock_path, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::Future), "{err:?}");
    }

    // -- Not-ready rejection -------------------------------------------------

    #[test]
    fn not_ready_attestation_passes_verify_only_but_is_rejected_at_publish_gate() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_attestation(
            &signing_key,
            false,
            &["macos-arm64-clean-install: runner offline"],
        );
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let out = dir.path().join("out.json");
        let deps = test_deps(&trusted_path, sample_runner_lock_sha());

        // verify_only accepts a not-ready attestation for inspection; the
        // receipt surfaces ready=false and any blockers.
        let receipt = verify_only_with(&input, &runner_lock_path, &deps).expect("inspect");
        assert!(!receipt.ready);
        assert_eq!(receipt.out_path, None);

        // verify_and_publish rejects: the publish gate requires ready=true.
        let err = verify_and_publish_with(&input, &runner_lock_path, &out, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::NotReady), "{err:?}");
        // Output must not be created on a NotReady rejection.
        assert!(!out.exists());
    }

    // -- Existing output -----------------------------------------------------

    #[test]
    fn existing_output_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let trusted_path = write_trusted_key(dir.path(), &signing_key);
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let out = dir.path().join("out.json");
        std::fs::write(&out, b"preexisting").unwrap();

        let deps = test_deps(&trusted_path, sample_runner_lock_sha());
        let err = verify_and_publish_with(&input, &runner_lock_path, &out, &deps).unwrap_err();
        assert!(matches!(err, KeystoneError::AlreadyExists), "{err:?}");
        // Pre-existing bytes are preserved (create-only).
        assert_eq!(std::fs::read(&out).unwrap(), b"preexisting");
    }

    // -- Production fail-closed ---------------------------------------------

    #[test]
    fn production_verify_only_fails_closed_when_key_unprovisioned() {
        // The pinned trusted-verifier key file does not exist in this
        // repository until G004 provisions it, so verify_only must fail
        // closed with UnprovisionedTrust before reaching the runner API.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        // Note: no trusted key is written at the DEFAULT path.
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");

        let err = verify_only(&input, &runner_lock_path).unwrap_err();
        assert!(matches!(err, KeystoneError::UnprovisionedTrust), "{err:?}");
    }

    #[test]
    fn production_verify_and_publish_fails_closed_when_key_unprovisioned() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let input = write_attestation(dir.path(), &claim);
        let runner_lock_path = dir.path().join("runner-lock.json");
        let out = dir.path().join("out.json");

        let err = verify_and_publish(&input, &runner_lock_path, &out).unwrap_err();
        assert!(matches!(err, KeystoneError::UnprovisionedTrust), "{err:?}");
        assert!(!out.exists());
    }

    // -- load_attestation ----------------------------------------------------

    #[test]
    fn load_attestation_parses_valid_file() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let path = write_attestation(dir.path(), &claim);
        let loaded = load_attestation(&path).expect("parses");
        assert_eq!(loaded, claim);
    }

    #[test]
    fn load_attestation_rejects_unknown_field() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let dir = tempdir().unwrap();
        let path = dir.path().join("att.json");
        let json = serde_json::to_string(&claim).unwrap();
        let tampered = json.trim_end_matches('}').to_string() + ",\"extra\":42}";
        std::fs::write(&path, &tampered).unwrap();
        assert!(load_attestation(&path).is_err());
    }

    // Trusted-verifier public-key loading tests removed: the keystone no
    // longer duplicates the readiness module's authoritative loader
    // (load_trusted_verifier_public_key). Those acceptance paths (hex format,
    // trailing newline, missing file, uppercase rejection) are covered by
    // readiness.rs's own trusted_key_file_* tests, and the keystone exercises
    // the fail-closed mapping via production_verify_only_fails_closed_*.
    // -- Production publisher -----------------------------------------------

    #[cfg(unix)]
    #[test]
    fn production_publish_writes_canonical_bytes_to_real_directory() {
        // Use temp_root (under workspace/target/tmp) so the dirfd parent walk
        // never traverses macOS `/var -> /private/var`.
        let dir = temp_root();
        let out = dir.path().join("published.json");
        let bytes = b"{\"verified\":true}\n";
        production_publish(&out, bytes).expect("publishes");
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn production_publish_rejects_existing_output() {
        let dir = temp_root();
        let out = dir.path().join("published.json");
        std::fs::write(&out, b"preexisting").unwrap();
        let err = production_publish(&out, b"{}").unwrap_err();
        assert!(matches!(err, KeystoneError::AlreadyExists), "{err:?}");
        // Pre-existing content preserved.
        assert_eq!(std::fs::read(&out).unwrap(), b"preexisting");
    }

    #[cfg(unix)]
    #[test]
    fn production_publish_rejects_symlink_parent() {
        let outer = temp_root();
        let real_parent = outer.path().join("real");
        std::fs::create_dir(&real_parent).unwrap();
        let symlink_parent = outer.path().join("link");
        std::os::unix::fs::symlink(&real_parent, &symlink_parent).unwrap();
        let out = symlink_parent.join("out.json");
        let err = production_publish(&out, b"{}").unwrap_err();
        assert!(
            matches!(err, KeystoneError::UnsafeParent(_) | KeystoneError::Io(_)),
            "{err:?}"
        );
        // No file created through the symlink.
        assert!(!real_parent.join("out.json").exists());
    }

    // -- Determinism --------------------------------------------------------

    #[test]
    fn canonical_attestation_bytes_round_trips() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let bytes = canonical_attestation_bytes(&claim).unwrap();
        let reparsed: ReadinessAttestationV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reparsed, claim);
        assert!(bytes.ends_with(b"\n"));
    }

    #[test]
    fn map_readiness_error_preserves_categories() {
        assert!(matches!(
            map_readiness_error(ReadinessError::TrustedKeyMissing),
            KeystoneError::UnprovisionedTrust
        ));
        assert!(matches!(
            map_readiness_error(ReadinessError::VerifierNotIndependent),
            KeystoneError::WrongSigner
        ));
        assert!(matches!(
            map_readiness_error(ReadinessError::SignatureVerification),
            KeystoneError::WrongSigner
        ));
        assert!(matches!(
            map_readiness_error(ReadinessError::SourceMismatch),
            KeystoneError::Replayed
        ));
        assert!(matches!(
            map_readiness_error(ReadinessError::FutureEvidence),
            KeystoneError::Future
        ));
        assert!(matches!(
            map_readiness_error(ReadinessError::ExpiredAtIssuance),
            KeystoneError::Expired
        ));
        // Schema error preserves its message in the generic Readiness bucket.
        let mapped = map_readiness_error(ReadinessError::Schema);
        assert!(matches!(mapped, KeystoneError::Readiness(_)));
    }

    #[test]
    fn map_runner_error_preserves_unprovisioned() {
        assert!(matches!(
            map_runner_error(RunnerError::UnprovisionedTrust),
            KeystoneError::UnprovisionedTrust
        ));
        assert!(matches!(
            map_runner_error(RunnerError::Unprovisioned),
            KeystoneError::UnprovisionedTrust
        ));
    }
}
