//! Production provenance keystone — fail-closed build-evidence measurement.
//!
//! Source identity (commit, tree-cleanliness, toolchain, features) and the
//! candidate hash are MEASURED from the build environment, never caller-asserted.
//! The reproducibility controls (SOURCE_DATE_EPOCH + --remap-path-prefix) are an
//! exception: [`generate_with_reproducible_env`] re-derives them from the repo so
//! a separate `provenance generate`/bless invocation (which does not inherit
//! `reproducible-build`'s subprocess env) can record them. This means the
//! reproducibility *controls* are operator-asserted (the operator must have run
//! `reproducible-build`); the **candidate_sha256 is genuinely measured** and is
//! the verification anchor — a reviewer rebuilds via `reproducible-build` and
//! byte-compares candidate_sha256 to confirm reproducibility independently.
//! This assertion boundary is made explicit and schema-enforced by the
//! [`ReproducibilityControlsOrigin`] field on every record: it records whether
//! the controls were measured from the ambient build environment or re-derived
//! by an operator assertion, so a self-injected re-derivation cannot masquerade
//! as an independent build-environment measurement.
//! A production candidate must originate from a clean git tree, must not enable
//! the test-control feature, and must use reproducible-build controls
//! (SOURCE_DATE_EPOCH + --remap-path-prefix + a separate prod target root).
//! The generator returns [`ProvenanceError`] for any violation; a production
//! gate must never emit a provenance record for a candidate that fails these
//! measurements.

use crate::tool_process;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

const SCHEMA: &str = "rutile.production-provenance.v1";
const VERSION: u32 = 1;
const TEST_CONTROL_FEATURE: &str = "test-control";

/// Measured production provenance binding a candidate artifact to its source
/// tree, toolchain, features, reproducibility controls, and artifact hash.
///
/// Serialization order does not matter: canonical JSON (sorted keys) is used
/// for the provenance SHA-256 computation.
#[derive(Clone, Debug, Serialize)]
pub struct ProductionProvenance {
    pub schema: String,
    pub version: u32,
    pub product: String,
    pub product_version: String,
    pub source_commit: String,
    pub source_tree_clean: bool,
    pub toolchain: Toolchain,
    pub features: Vec<String>,
    pub candidate_sha256: String,
    pub reproducibility: Reproducibility,
    pub built_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_tag: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Toolchain {
    pub rustc_version: String,
    pub host_triple: String,
    pub target_triple: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Reproducibility {
    pub source_date_epoch: u64,
    pub remap_path_prefix: bool,
    pub target_root: String,
    pub controls_origin: ReproducibilityControlsOrigin,
}

/// Explicit derivation boundary for the reproducibility controls
/// (`source_date_epoch` + `--remap-path-prefix` + `target_root`).
///
/// The `candidate_sha256` is always measured from the candidate artifact file.
/// The reproducibility *controls* have a derivation boundary that this enum
/// records, so a self-injected re-derivation cannot be mistaken for an
/// independent build-environment measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproducibilityControlsOrigin {
    /// `SOURCE_DATE_EPOCH` + `--remap-path-prefix` RUSTFLAGS were read from the
    /// ambient build environment (i.e. inherited from the `reproducible-build`
    /// subprocess that produced the candidate). The operator asserts the
    /// candidate was built under these controls; the generator measured them.
    AmbientBuildEnv,
    /// The controls were re-derived from the repository by
    /// [`generate_with_reproducible_env`] (commit-date `SOURCE_DATE_EPOCH` and
    /// repo-derived RUSTFLAGS) because the generating process did not inherit
    /// `reproducible-build`'s subprocess environment. This is an operator
    /// re-derivation assertion, not an independent build-environment measurement.
    OperatorReDerivation,
}

/// Inputs the generator cannot measure on its own (paths and caller-provided
/// feature list).  The generator VALIDATES every field — it never trusts the
/// caller for anything measurable from git, the toolchain, or the environment.
#[derive(Clone, Debug)]
pub struct ProvenanceRequest {
    /// Candidate artifact file whose SHA-256 anchors the provenance record.
    pub candidate: PathBuf,
    /// Repository root for git measurements (commit, tree-clean, tag).
    pub repo_root: PathBuf,
    /// Cargo features enabled on the build; will be canonicalized and checked.
    pub features: Vec<String>,
    /// Production target root (relative path, must differ from "target").
    pub target_root: String,
}

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("CARGO_PKG_NAME environment variable is required for provenance")]
    MissingProductName,
    #[error("CARGO_PKG_VERSION environment variable is required for provenance")]
    MissingProductVersion,
    #[error("SOURCE_DATE_EPOCH environment variable is required for reproducible provenance")]
    MissingSourceDateEpoch,
    #[error("SOURCE_DATE_EPOCH is not a valid non-negative integer: {0}")]
    InvalidSourceDateEpoch(String),
    #[error("source tree is dirty; provenance requires a clean working tree")]
    DirtyTree,
    #[error("test-control feature is forbidden in production artifacts")]
    TestControlFeature,
    #[error("--remap-path-prefix is required in RUSTFLAGS or CARGO_ENCODED_RUSTFLAGS")]
    MissingRemapPathPrefix,
    #[error("production target root must differ from the default \"target\" directory")]
    DefaultTargetRoot,
    #[error("production target root must be a relative path without host-local segments")]
    UnsafeTargetRoot,
    #[error("failed to measure git state: {0}")]
    GitMeasurement(String),
    #[error("failed to measure toolchain via `rustc -vV`: {0}")]
    ToolchainMeasurement(String),
    #[error("failed to hash candidate artifact {path}: {error}")]
    CandidateHash { path: PathBuf, error: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Generate a production provenance record by measuring the build environment.
///
/// Fail-closed: returns [`ProvenanceError`] for dirty trees, test-control
/// features, missing reproducibility env, or any measurement failure.  Never
/// returns a partial record.
pub fn generate(request: &ProvenanceRequest) -> Result<ProductionProvenance, ProvenanceError> {
    // Product identity — measured from the cargo environment.
    let product =
        std::env::var("CARGO_PKG_NAME").map_err(|_| ProvenanceError::MissingProductName)?;
    let product_version =
        std::env::var("CARGO_PKG_VERSION").map_err(|_| ProvenanceError::MissingProductVersion)?;

    // Source commit + tree cleanliness — measured from git.
    let (source_commit, source_tree_clean) = measure_git_state(&request.repo_root)?;
    if !source_tree_clean {
        return Err(ProvenanceError::DirtyTree);
    }

    // Toolchain — measured from rustc -vV.
    let toolchain = measure_toolchain()?;

    // Features — caller-provided but validated and canonicalized.
    let features = canonicalize_features(&request.features)?;
    if features.iter().any(|f| f == TEST_CONTROL_FEATURE) {
        return Err(ProvenanceError::TestControlFeature);
    }

    // Candidate artifact hash — measured by streaming the file.
    let candidate_sha256 = hash_candidate(&request.candidate)?;

    // Reproducibility — measured from the build environment.
    let source_date_epoch = measure_source_date_epoch()?;
    if !remap_path_prefix_active() {
        return Err(ProvenanceError::MissingRemapPathPrefix);
    }
    validate_target_root(&request.target_root)?;

    // built_at — deterministic from SOURCE_DATE_EPOCH (never wall clock).
    let built_at = epoch_to_iso8601(source_date_epoch);

    // source_tag — optional, measured from git describe.
    let source_tag = measure_source_tag(&request.repo_root);

    Ok(ProductionProvenance {
        schema: SCHEMA.to_string(),
        version: VERSION,
        product,
        product_version,
        source_commit,
        source_tree_clean: true, // always true: dirty => early Err above
        toolchain,
        features,
        candidate_sha256,
        reproducibility: Reproducibility {
            source_date_epoch,
            remap_path_prefix: true,
            target_root: request.target_root.clone(),
            controls_origin: ReproducibilityControlsOrigin::AmbientBuildEnv,
        },
        built_at,
        source_tag,
    })
}

/// Generate provenance with the reproducibility controls re-derived from the
/// repository (`SOURCE_DATE_EPOCH` from the commit date, `--remap-path-prefix`
/// RUSTFLAGS) and product identity set by the caller. Shared by the `provenance
/// generate` CLI and the local-packaging bless step. The operator asserts the
/// candidate was produced by `reproducible-build`.
pub fn generate_with_reproducible_env(
    request: &ProvenanceRequest,
    product: &str,
    product_version: &str,
) -> Result<ProductionProvenance, ProvenanceError> {
    let source_date_epoch = crate::reproducible_build::git_commit_date(&request.repo_root)
        .map_err(|error| ProvenanceError::GitMeasurement(error.to_string()))?;
    let rustflags = crate::reproducible_build::reproducible_rustflags(&request.repo_root);
    // SAFETY: single-threaded xtask CLI/packaging path; `generate` reads these
    // env vars synchronously on this thread, with no spawn in between.
    unsafe {
        std::env::set_var("CARGO_PKG_NAME", product);
        std::env::set_var("CARGO_PKG_VERSION", product_version);
        std::env::set_var("SOURCE_DATE_EPOCH", &source_date_epoch);
        std::env::set_var("RUSTFLAGS", &rustflags);
    }
    let mut provenance = generate(request)?;
    // The controls were re-derived from the repository (commit-date
    // SOURCE_DATE_EPOCH + repo-derived RUSTFLAGS) rather than measured from
    // the candidate build subprocess env. Mark the boundary explicitly so the
    // record cannot be mistaken for an independent build-environment measurement.
    provenance.reproducibility.controls_origin =
        ReproducibilityControlsOrigin::OperatorReDerivation;
    Ok(provenance)
}

impl ProductionProvenance {
    /// Canonical JSON serialization with deterministic (alphabetically sorted)
    /// key order at every nesting level.  This is the canonical form over which
    /// the provenance SHA-256 is computed.
    pub fn canonical_json(&self) -> serde_json::Result<String> {
        // serde_json::Map is backed by BTreeMap by default (no preserve_order
        // feature), so converting through Value yields sorted keys at every
        // level — true canonical JSON.
        let value = serde_json::to_value(self)?;
        serde_json::to_string(&value)
    }

    /// SHA-256 of the canonical JSON representation.  This is the value bound
    /// into artifact inspection reports and evidence indices.
    pub fn provenance_sha256(&self) -> Result<String, serde_json::Error> {
        let json = self.canonical_json()?;
        Ok(hex::encode(Sha256::digest(json.as_bytes())))
    }
}

// ---------------------------------------------------------------------------
// Measurement primitives — every field is measured, never caller-asserted.
// ---------------------------------------------------------------------------

fn measure_git_state(repo: &Path) -> Result<(String, bool), ProvenanceError> {
    let rev = tool_process::git_isolated(repo, &["--no-replace-objects", "rev-parse", "HEAD"], &[])
        .map_err(|e| ProvenanceError::GitMeasurement(e.to_string()))?;
    if !rev.status.success() {
        return Err(ProvenanceError::GitMeasurement(format!(
            "git rev-parse HEAD exited {}: {}",
            rev.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&rev.stderr).trim()
        )));
    }
    let commit = String::from_utf8_lossy(&rev.stdout).trim().to_string();
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ProvenanceError::GitMeasurement(format!(
            "invalid commit hash: {commit}"
        )));
    }

    let status = tool_process::git_isolated(repo, &["--no-replace-objects", "status", "--porcelain"], &[])
        .map_err(|e| ProvenanceError::GitMeasurement(e.to_string()))?;
    if !status.status.success() {
        return Err(ProvenanceError::GitMeasurement(format!(
            "git status --porcelain exited {}",
            status.status.code().unwrap_or(-1)
        )));
    }
    let clean = String::from_utf8_lossy(&status.stdout).trim().is_empty();

    Ok((commit, clean))
}

fn measure_toolchain() -> Result<Toolchain, ProvenanceError> {
    let output = {
        #[allow(clippy::disallowed_methods)]
        {
            Command::new("rustc")
                .arg("-vV")
                .output()
                .map_err(|e| ProvenanceError::ToolchainMeasurement(e.to_string()))?
        }
    };
    if !output.status.success() {
        return Err(ProvenanceError::ToolchainMeasurement(format!(
            "rustc -vV exited {}",
            output.status.code().unwrap_or(-1)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);

    let rustc_version = text
        .lines()
        .find(|line| line.starts_with("release: "))
        .map(|line| line.trim_start_matches("release: ").trim().to_string())
        .or_else(|| text.lines().next().map(|line| line.trim().to_string()))
        .ok_or_else(|| ProvenanceError::ToolchainMeasurement("empty rustc output".into()))?;

    let host_triple = text
        .lines()
        .find(|line| line.starts_with("host: "))
        .map(|line| line.trim_start_matches("host: ").trim().to_string())
        .ok_or_else(|| {
            ProvenanceError::ToolchainMeasurement("host triple not found in rustc -vV".into())
        })?;

    // Target triple: prefer CARGO_BUILD_TARGET (set when --target is passed);
    // fall back to the host triple (native build).
    let target_triple = std::env::var("CARGO_BUILD_TARGET").unwrap_or_else(|_| host_triple.clone());

    Ok(Toolchain {
        rustc_version,
        host_triple,
        target_triple,
    })
}

fn canonicalize_features(raw: &[String]) -> Result<Vec<String>, ProvenanceError> {
    let mut features: Vec<String> = raw
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    features.sort();
    features.dedup();
    Ok(features)
}

fn hash_candidate(path: &Path) -> Result<String, ProvenanceError> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| ProvenanceError::CandidateHash {
        path: path.to_owned(),
        error: e.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| ProvenanceError::CandidateHash {
                path: path.to_owned(),
                error: e.to_string(),
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn measure_source_date_epoch() -> Result<u64, ProvenanceError> {
    let raw =
        std::env::var("SOURCE_DATE_EPOCH").map_err(|_| ProvenanceError::MissingSourceDateEpoch)?;
    raw.parse::<u64>()
        .map_err(|_| ProvenanceError::InvalidSourceDateEpoch(raw))
}

fn remap_path_prefix_active() -> bool {
    // CARGO_ENCODED_RUSTFLAGS uses unit separator (\x1f) between args.
    if let Ok(encoded) = std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        if encoded
            .split('\x1f')
            .any(|arg| arg.starts_with("--remap-path-prefix"))
        {
            return true;
        }
    }
    // Fall back to RUSTFLAGS (whitespace-separated).
    if let Ok(flags) = std::env::var("RUSTFLAGS") {
        if flags
            .split_whitespace()
            .any(|arg| arg.starts_with("--remap-path-prefix"))
        {
            return true;
        }
    }
    false
}

fn validate_target_root(target_root: &str) -> Result<(), ProvenanceError> {
    // Must not be the default cargo target directory.
    if target_root == "target" || target_root == "target/" {
        return Err(ProvenanceError::DefaultTargetRoot);
    }
    // Must not be absolute or contain host-local path segments.
    if target_root.starts_with('/')
        || target_root.contains("..")
        || target_root.contains("/Users/")
        || target_root.contains("/home/")
        || target_root.contains("/private/")
        || target_root.contains("/var/folders/")
    {
        return Err(ProvenanceError::UnsafeTargetRoot);
    }
    Ok(())
}

fn measure_source_tag(repo: &Path) -> Option<String> {
    let output =
        tool_process::git_isolated(repo, &["--no-replace-objects", "describe", "--tags", "--exact-match", "HEAD"], &[])
            .ok()?;
    if !output.status.success() {
        return None;
    }
    let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if tag.is_empty() {
        return None;
    }
    // Basic validation against the schema's gitTag constraints.
    if tag.len() > 200
        || tag.contains('\0')
        || tag.to_lowercase().contains("token")
        || tag.to_lowercase().contains("secret")
        || tag.to_lowercase().contains("password")
        || tag.to_lowercase().contains("credentials")
        || tag.contains("/Users/")
        || tag.contains("/home/")
        || tag.contains("/private/")
    {
        return None;
    }
    Some(tag)
}

/// Convert a Unix epoch timestamp to an ISO-8601 UTC string.
///
/// Deterministic, no external time crate needed.  Uses the well-known
/// civil-from-days algorithm from Howard Hinnant.
fn epoch_to_iso8601(epoch: u64) -> String {
    let days_since_epoch = (epoch / 86400) as i64;
    let remainder = epoch % 86400;
    let hour = remainder / 3600;
    let minute = (remainder % 3600) / 60;
    let second = remainder % 60;

    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's civil-from-days algorithm.
/// Input: days since 1970-01-01.  Output: (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = if z >= 0 { z } else { z - 145_969 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Serialises tests that mutate process-wide environment variables.
    /// Without this, parallel `cargo test` threads race on env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: initialise a hermetic git repo with a single committed file so
    /// that `rev-parse HEAD` and `status --porcelain` succeed.
    fn init_clean_git_repo(dir: &Path) {
        let _ = tool_process::git_isolated(dir, &["init", "--quiet"], &[])
            .unwrap()
            .status
            .success();
        tool_process::git_isolated(
            dir,
            &["config", "user.email", "test@rutile.local"],
            &[],
        )
        .unwrap();
        tool_process::git_isolated(dir, &["config", "user.name", "Test"], &[]).unwrap();
        fs::write(dir.join("README"), b"test").unwrap();
        tool_process::git_isolated(dir, &["add", "."], &[]).unwrap();
        tool_process::git_isolated(dir, &["commit", "--quiet", "-m", "init"], &[]).unwrap();
    }

    /// Helper: set the required env vars for a happy-path measurement.
    fn set_provenance_env() {
        unsafe {
            std::env::set_var("CARGO_PKG_NAME", "rutile");
            std::env::set_var("CARGO_PKG_VERSION", "0.2.0");
            std::env::set_var("SOURCE_DATE_EPOCH", "1720915200"); // 2024-07-14T00:00:00Z
            std::env::set_var("RUSTFLAGS", "--remap-path-prefix=/Users/x=src");
            std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");
            std::env::remove_var("CARGO_BUILD_TARGET");
        }
    }

    fn clear_provenance_env() {
        unsafe {
            std::env::remove_var("CARGO_PKG_NAME");
            std::env::remove_var("CARGO_PKG_VERSION");
            std::env::remove_var("SOURCE_DATE_EPOCH");
            std::env::remove_var("RUSTFLAGS");
            std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");
            std::env::remove_var("CARGO_BUILD_TARGET");
        }
    }

    fn make_request(repo: &Path, candidate: &Path) -> ProvenanceRequest {
        ProvenanceRequest {
            candidate: candidate.to_owned(),
            repo_root: repo.to_owned(),
            features: vec!["wayland".into(), "appimage".into()],
            target_root: "target-prod".into(),
        }
    }

    /// Create a candidate file in a SEPARATE temp directory from the git repo
    /// so that the candidate file does not make the git tree dirty.
    fn make_candidate(content: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let candidate = dir.path().join("candidate.bin");
        fs::write(&candidate, content).unwrap();
        (dir, candidate)
    }

    // --- RLS-004: rejection cases ---

    #[test]
    fn dirty_tree_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        set_provenance_env();
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());
        // Dirty the tree.
        fs::write(repo.path().join("uncommitted"), b"dirty").unwrap();

        let (_cand_dir, candidate) = make_candidate(b"artifact bytes");

        let result = generate(&make_request(repo.path(), &candidate));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("dirty"),
            "expected dirty-tree error, got: {err}"
        );
        clear_provenance_env();
    }

    #[test]
    fn test_control_feature_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        set_provenance_env();
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());

        let (_cand_dir, candidate) = make_candidate(b"artifact bytes");

        let mut request = make_request(repo.path(), &candidate);
        request.features = vec!["wayland".into(), "test-control".into()];

        let result = generate(&request);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("test-control"),
            "expected test-control error, got: {err}"
        );
        clear_provenance_env();
    }

    #[test]
    fn non_reproducible_missing_source_date_epoch_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        // Set everything except SOURCE_DATE_EPOCH.
        unsafe {
            std::env::set_var("CARGO_PKG_NAME", "rutile");
            std::env::set_var("CARGO_PKG_VERSION", "0.2.0");
            std::env::set_var("RUSTFLAGS", "--remap-path-prefix=/x=y");
            std::env::remove_var("SOURCE_DATE_EPOCH");
            std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");
        }
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());

        let (_cand_dir, candidate) = make_candidate(b"artifact bytes");

        let result = generate(&make_request(repo.path(), &candidate));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("SOURCE_DATE_EPOCH"),
            "expected SOURCE_DATE_EPOCH error, got: {err}"
        );
        clear_provenance_env();
    }

    #[test]
    fn non_reproducible_missing_remap_path_prefix_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        unsafe {
            std::env::set_var("CARGO_PKG_NAME", "rutile");
            std::env::set_var("CARGO_PKG_VERSION", "0.2.0");
            std::env::set_var("SOURCE_DATE_EPOCH", "1720915200");
            std::env::remove_var("RUSTFLAGS");
            std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");
        }
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());

        let (_cand_dir, candidate) = make_candidate(b"artifact bytes");

        let result = generate(&make_request(repo.path(), &candidate));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("remap-path-prefix"),
            "expected remap-path-prefix error, got: {err}"
        );
        clear_provenance_env();
    }

    #[test]
    fn default_target_root_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        set_provenance_env();
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());

        let (_cand_dir, candidate) = make_candidate(b"artifact bytes");

        let mut request = make_request(repo.path(), &candidate);
        request.target_root = "target".into();

        let result = generate(&request);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("target"),
            "expected default-target-root error, got: {err}"
        );
        clear_provenance_env();
    }

    // --- RLS-001: negative-env probes ---

    #[test]
    fn missing_product_name_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        unsafe {
            std::env::set_var("CARGO_PKG_VERSION", "0.2.0");
            std::env::set_var("SOURCE_DATE_EPOCH", "1720915200");
            std::env::set_var("RUSTFLAGS", "--remap-path-prefix=/x=y");
            std::env::remove_var("CARGO_PKG_NAME");
        }
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());
        let (_cand_dir, candidate) = make_candidate(b"x");

        let result = generate(&make_request(repo.path(), &candidate));
        assert!(matches!(result, Err(ProvenanceError::MissingProductName)));
        clear_provenance_env();
    }

    #[test]
    fn missing_product_version_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        unsafe {
            std::env::set_var("CARGO_PKG_NAME", "rutile");
            std::env::set_var("SOURCE_DATE_EPOCH", "1720915200");
            std::env::set_var("RUSTFLAGS", "--remap-path-prefix=/x=y");
            std::env::remove_var("CARGO_PKG_VERSION");
        }
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());
        let (_cand_dir, candidate) = make_candidate(b"x");

        let result = generate(&make_request(repo.path(), &candidate));
        assert!(matches!(
            result,
            Err(ProvenanceError::MissingProductVersion)
        ));
        clear_provenance_env();
    }

    // --- Happy path ---

    #[test]
    fn happy_path_clean_tree_valid_env_produces_schema_valid_provenance() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_provenance_env();
        set_provenance_env();
        let repo = tempfile::tempdir().unwrap();
        init_clean_git_repo(repo.path());

        let (_cand_dir, candidate) = make_candidate(b"production artifact bytes");

        let provenance = generate(&make_request(repo.path(), &candidate)).unwrap();

        // Core fields measured correctly.
        assert_eq!(provenance.schema, "rutile.production-provenance.v1");
        assert_eq!(provenance.version, 1);
        assert_eq!(provenance.product, "rutile");
        assert_eq!(provenance.product_version, "0.2.0");
        assert_eq!(provenance.source_commit.len(), 40);
        assert!(
            provenance
                .source_commit
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert!(provenance.source_tree_clean);

        // Toolchain measured from rustc.
        assert!(!provenance.toolchain.rustc_version.is_empty());
        assert!(!provenance.toolchain.host_triple.is_empty());
        assert!(!provenance.toolchain.target_triple.is_empty());

        // Features canonicalized (sorted, no test-control).
        assert_eq!(provenance.features, vec!["appimage", "wayland"]);

        // Candidate hash.
        let expected_hash = hex::encode(Sha256::digest(b"production artifact bytes"));
        assert_eq!(provenance.candidate_sha256, expected_hash);

        // Reproducibility.
        assert_eq!(provenance.reproducibility.source_date_epoch, 1720915200);
        assert!(provenance.reproducibility.remap_path_prefix);
        assert_eq!(provenance.reproducibility.target_root, "target-prod");
        // `generate` reads the controls from the ambient build env: the
        // derivation boundary is explicitly marked as measured, not re-derived.
        assert_eq!(
            provenance.reproducibility.controls_origin,
            ReproducibilityControlsOrigin::AmbientBuildEnv
        );

        // built_at deterministic from SOURCE_DATE_EPOCH.
        assert_eq!(provenance.built_at, "2024-07-14T00:00:00Z");

        // Canonical JSON + SHA256 are deterministic.
        let sha_a = provenance.provenance_sha256().unwrap();
        let sha_b = provenance.provenance_sha256().unwrap();
        assert_eq!(sha_a, sha_b);
        assert_eq!(sha_a.len(), 64);

        // Schema validation against the checked-in schema file.
        let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas/rutile.production-provenance.v1.schema.json");
        let schema = serde_json::from_str(&std::fs::read_to_string(&schema_path).unwrap()).unwrap();
        let instance = serde_json::to_value(&provenance).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(
            errors.is_empty(),
            "provenance must validate against schema: {errors:?}"
        );
        clear_provenance_env();
    }

    // --- Canonical JSON determinism ---

    #[test]
    fn canonical_json_has_sorted_object_keys_and_is_stable() {
        let provenance = ProductionProvenance {
            schema: SCHEMA.into(),
            version: VERSION,
            product: "rutile".into(),
            product_version: "0.2.0".into(),
            source_commit: "a".repeat(40),
            source_tree_clean: true,
            toolchain: Toolchain {
                rustc_version: "1.88.0".into(),
                host_triple: "aarch64-apple-darwin".into(),
                target_triple: "aarch64-apple-darwin".into(),
            },
            features: vec!["zeta".into(), "alpha".into()],
            candidate_sha256: "b".repeat(64),
            reproducibility: Reproducibility {
                source_date_epoch: 0,
                remap_path_prefix: true,
                target_root: "target-prod".into(),
                controls_origin: ReproducibilityControlsOrigin::AmbientBuildEnv,
            },
            built_at: "1970-01-01T00:00:00Z".into(),
            source_tag: None,
        };

        let json = provenance.canonical_json().unwrap();
        // Object keys must be alphabetically sorted at the top level.
        // "built_at" < "candidate_sha256" < "features" < ... < "version"
        let built_pos = json.find("\"built_at\"").unwrap();
        let candidate_pos = json.find("\"candidate_sha256\"").unwrap();
        let features_pos = json.find("\"features\"").unwrap();
        let version_pos = json.find("\"version\"").unwrap();
        assert!(built_pos < candidate_pos);
        assert!(candidate_pos < features_pos);
        assert!(features_pos < version_pos);

        // Nested object keys are also sorted.
        let toolchain_pos = json.find("\"toolchain\"").unwrap();
        let host_pos = json.find("\"host_triple\"").unwrap();
        let rustc_pos = json.find("\"rustc_version\"").unwrap();
        let target_pos = json.find("\"target_triple\"").unwrap();
        assert!(
            host_pos > toolchain_pos && rustc_pos > toolchain_pos && target_pos > toolchain_pos,
            "toolchain children must follow toolchain key"
        );
        assert!(
            host_pos < rustc_pos,
            "nested keys must be sorted: host_triple < rustc_version"
        );
        assert!(
            rustc_pos < target_pos,
            "nested keys must be sorted: rustc_version < target_triple"
        );

        // SHA256 is stable across calls.
        let sha1 = provenance.provenance_sha256().unwrap();
        let sha2 = provenance.provenance_sha256().unwrap();
        assert_eq!(sha1, sha2);
    }

    // --- Epoch to ISO-8601 correctness ---

    #[test]
    fn epoch_to_iso8601_known_values() {
        assert_eq!(epoch_to_iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(epoch_to_iso8601(1720915200), "2024-07-14T00:00:00Z");
        // Leap-day: 2000-02-29 (2000 is divisible by 400 → leap year).
        assert_eq!(epoch_to_iso8601(951782400), "2000-02-29T00:00:00Z");
        // Day after leap-day.
        assert_eq!(epoch_to_iso8601(951868800), "2000-03-01T00:00:00Z");
    }
}
