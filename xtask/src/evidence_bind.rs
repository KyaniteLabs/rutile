//! Evidence bind keystone — fail-closed cross-tree evidence index binding.
//!
//! Consumes the plain gate index produced by `scripts/ci/evidence-finalize.sh`,
//! a `rutile.production-provenance.v1` record, the evidence tree root, and a
//! create-only output path, and produces a canonical
//! `rutile.evidence-index.v1` document binding every gated job to its
//! gate-result file, retained log, and artifact hashes, anchored to the
//! production provenance record by SHA-256.
//!
//! This module NEVER signs, NEVER synthesizes a missing row or hash, and NEVER
//! trusts a caller-asserted measurement that can be re-derived from disk or
//! git. Every hash bound into the output is recomputed from the bytes on disk;
//! every source binding is cross-checked against the current repository HEAD
//! via the audited `tool_process::git_isolated` path. A single failed check
//! fails the whole bind closed with a typed [`EvidenceBindError`].
//!
//! # Plain gate index contract (input)
//!
//! `evidence-finalize.sh` writes a plain JSON index with exactly these fields:
//!
//! ```json
//! {
//!   "commit": "<40-hex>",
//!   "gate_count": N,
//!   "passed": N,
//!   "failed": N,
//!   "required_jobs": ["job-a", "job-b"],
//!   "gates": [
//!     {"job": "job-a", "path": "<commit>/job-a/run-.../gate-result.json",
//!      "sha256": "<64-hex>", "status": "passed", "error": ""}
//!   ]
//! }
//! ```
//!
//! Paths in `gates[].path` are relative to the evidence root. Missing required
//! jobs appear with `path: null, sha256: null, status: "failed"`; those fail
//! the "every gate passed" check closed.

use crate::tool_process;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Schema identifier for the evidence index output.
pub const EVIDENCE_INDEX_SCHEMA: &str = "rutile.evidence-index.v1";

/// Schema version for the evidence index output.
pub const EVIDENCE_INDEX_VERSION: u32 = 1;

/// Schema identifier carried by every gate-result document.
const GATE_RESULT_SCHEMA: &str = "rutile.gate-result.v1";

/// Schema identifier carried by every production-provenance document.
const PROVENANCE_SCHEMA: &str = "rutile.production-provenance.v1";

/// Maximum records in an evidence index. Mirrors the schema `records.maxItems`.
const MAX_RECORDS: usize = 256;

/// Maximum byte length of a repo-relative evidence path. Mirrors the schema
/// `relativePath.maxLength` of 256.
const MAX_RELATIVE_PATH_BYTES: usize = 256;

/// Maximum byte length of a logical job id. Mirrors the schema `logicalId.maxLength`.
const MAX_LOGICAL_ID_BYTES: usize = 160;

/// Maximum retained-log size in bytes. Mirrors the gate-result schema
/// `retained_logs.items.bytes.maximum` of 16384 (16 KiB).
const MAX_RETAINED_LOG_BYTES: u64 = 16 * 1024;

/// Bounded input sizes — fail closed on oversize inputs to prevent unbounded
/// reads. The plain index, provenance, and gate-results are all small JSON
/// documents; these ceilings are generous but finite.
const MAX_PLAIN_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_PROVENANCE_BYTES: u64 = 1024 * 1024;
const MAX_GATE_RESULT_BYTES: u64 = 1024 * 1024;

/// Maximum artifact hashes per record. Mirrors the schema
/// `artifact_sha256.maxItems` of 256.
const MAX_ARTIFACT_HASHES: usize = 256;

/// Canonical retained-log stream ordering: stdout is the primary log.
fn stream_rank(stream: &str) -> i64 {
    match stream {
        "stdout" => 0,
        "stderr" => 1,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------
// Typed output (mirrors rutile.evidence-index.v1 exactly)
// ---------------------------------------------------------------------------

/// Canonical evidence index document (`rutile.evidence-index.v1`).
///
/// Serialization order does not matter: canonical JSON (sorted keys) is used
/// for both on-disk output and schema validation, mirroring the provenance
/// keystone's canonical form.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIndex {
    pub schema: String,
    pub version: u32,
    pub source_commit: String,
    pub production_provenance_sha256: String,
    pub records: Vec<EvidenceRecord>,
}

/// A single evidence record binding one gated job to its gate-result file,
/// retained log, and artifact hashes.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub job: String,
    pub gate_result_path: String,
    pub gate_result_sha256: String,
    pub retained_log_path: String,
    pub retained_log_sha256: String,
    pub artifact_sha256: Vec<String>,
}

impl EvidenceIndex {
    /// Canonical JSON serialization with deterministic (alphabetically sorted)
    /// key order at every nesting level. serde_json::Map is backed by BTreeMap
    /// by default (no preserve_order feature), so converting through Value
    /// yields sorted keys at every level.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        let value = serde_json::to_value(self)?;
        serde_json::to_string(&value)
    }

    /// Canonical pretty-printed bytes (sorted keys, trailing newline) for
    /// durable create-only output.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let value = serde_json::to_value(self)?;
        let mut bytes = serde_json::to_vec_pretty(&value)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

// ---------------------------------------------------------------------------
// Plain gate index input (typed parse of evidence-finalize's output)
// ---------------------------------------------------------------------------

/// Typed view of the plain gate index emitted by `evidence-finalize.sh`.
/// `deny_unknown_fields` fails closed on any unexpected key so the contract
/// cannot drift silently.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainGateIndex {
    commit: String,
    gate_count: u32,
    passed: u32,
    failed: u32,
    #[serde(default)]
    required_jobs: Vec<String>,
    gates: Vec<PlainGate>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainGate {
    job: String,
    /// Relative path to the gate-result file; `None` for missing-required jobs.
    #[serde(default)]
    path: Option<String>,
    /// SHA-256 of the gate-result file bytes; `None` for missing-required jobs.
    #[serde(default)]
    sha256: Option<String>,
    status: String,
    #[serde(default)]
    error: String,
}

// ---------------------------------------------------------------------------
// Request / outcome / error
// ---------------------------------------------------------------------------

/// Inputs to [`bind`]. All paths are validated; none are trusted for
/// measurements that can be re-derived from disk or git.
#[derive(Clone, Debug)]
pub struct EvidenceBindRequest {
    /// Path to the plain gate index JSON written by `evidence-finalize.sh`.
    pub plain_index: PathBuf,
    /// Path to a `rutile.production-provenance.v1` JSON record.
    pub provenance: PathBuf,
    /// Evidence tree root (e.g. `target/evidence`). Gate-result paths in the
    /// plain index are relative to this root.
    pub evidence_root: PathBuf,
    /// Create-only output path for the canonical evidence index.
    pub out: PathBuf,
}

/// Successful bind + publish outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceBindOutcome {
    /// Whether the output reached durable storage (no fsync warnings).
    pub durable: bool,
    /// Durability warnings (empty when fully durable).
    pub warnings: Vec<&'static str>,
    /// Bound source commit (current repository HEAD).
    pub source_commit: String,
    /// SHA-256 of the production provenance file bytes.
    pub production_provenance_sha256: String,
    /// Number of records bound.
    pub record_count: usize,
    /// Output path written.
    pub out: PathBuf,
}

/// Typed evidence-bind failure. Every variant is fail-closed.
#[derive(Debug, Error)]
pub enum EvidenceBindError {
    #[error("plain index read failed at {path}: {error}")]
    PlainIndexRead { path: PathBuf, error: String },
    #[error("plain index is too large ({size} bytes); maximum {max}")]
    PlainIndexOversize { size: u64, max: u64 },
    #[error("plain index is not valid JSON at {path}: {error}")]
    PlainIndexParse { path: PathBuf, error: String },
    #[error("plain index commit is not a 40-char lowercase hex SHA: {0}")]
    PlainIndexCommitMalformed(String),
    #[error("plain index gate_count ({declared}) does not match gates.len() ({actual})")]
    PlainIndexGateCountMismatch { declared: u32, actual: usize },
    #[error(
        "plain index passed+failed ({passed}+{failed}) does not equal gate_count ({gate_count})"
    )]
    PlainIndexCountArithmetic {
        passed: u32,
        failed: u32,
        gate_count: u32,
    },
    #[error("plain index has no gates; at least one is required")]
    PlainIndexEmpty,
    #[error("plain index has too many gates ({0}); maximum {1}")]
    PlainIndexTooManyGates(usize, usize),
    #[error(
        "plain index passed count ({declared}) does not match gates with status \"passed\" ({actual})"
    )]
    PlainIndexPassedMismatch { declared: u32, actual: u32 },
    #[error(
        "plain index failed count ({declared}) does not match gates with status \"failed\" ({actual})"
    )]
    PlainIndexFailedMismatch { declared: u32, actual: u32 },
    #[error("plain index gate has unrecognized status \"{status}\" on job \"{job}\"")]
    PlainIndexBadStatus { job: String, status: String },
    #[error("plain index passed gate \"{0}\" carries a non-empty error")]
    PlainIndexPassedHasError(String),
    #[error("cannot derive current repository source: {0}")]
    CurrentSourceIo(String),
    #[error("current repository source output is malformed")]
    CurrentSourceMalformed,
    #[error("plain index commit ({index}) does not match the current repository HEAD ({head})")]
    SourceCommitMismatch { index: String, head: String },
    #[error("evidence root is not a directory: {0}")]
    EvidenceRootNotDir(PathBuf),
    #[error("evidence root is a symlink: {0}")]
    EvidenceRootSymlink(PathBuf),
    #[error("provenance read failed at {path}: {error}")]
    ProvenanceRead { path: PathBuf, error: String },
    #[error("provenance is too large ({size} bytes); maximum {max}")]
    ProvenanceOversize { size: u64, max: u64 },
    #[error("provenance is not valid JSON at {path}: {error}")]
    ProvenanceParse { path: PathBuf, error: String },
    #[error("provenance schema identifier is not rutile.production-provenance.v1")]
    ProvenanceSchema,
    #[error("provenance schema version is not 1")]
    ProvenanceVersion,
    #[error("provenance failed schema validation:\n{0}")]
    ProvenanceSchemaInvalid(String),
    #[error(
        "provenance source_commit ({provenance}) does not match the current HEAD / plain index commit ({head})"
    )]
    ProvenanceSourceMismatch { provenance: String, head: String },
    #[error("cannot resolve schema file for kind \"{0}\"; known kinds: {1}")]
    SchemaUnknown(String, String),
    #[error("cannot read schema file {path}: {error}")]
    SchemaRead { path: PathBuf, error: String },
    #[error("schema file is not valid JSON: {0}")]
    SchemaParse(String),
    #[error("failed to compile schema: {0}")]
    SchemaCompile(String),
    #[error("gate-result read failed at {path}: {error}")]
    GateResultRead { path: PathBuf, error: String },
    #[error("gate-result at {path} is not a regular file (symlink)")]
    GateResultNotRegular { path: PathBuf },
    #[error("gate-result is not valid JSON at {path}: {error}")]
    GateResultParse { path: PathBuf, error: String },
    #[error("gate-result schema identifier is not rutile.gate-result.v1 at {path}")]
    GateResultSchemaId { path: PathBuf },
    #[error("gate-result failed schema validation at {path}:\n{errors}")]
    GateResultSchemaInvalid { path: PathBuf, errors: String },
    #[error("gate-result sha256 at {path} does not match the plain index recorded value")]
    GateResultHashMismatch { path: PathBuf },
    #[error("gate-result source at {path} does not match the current repository HEAD")]
    GateResultSourceMismatch { path: PathBuf },
    #[error("gate-result run_directory does not match its path location at {path}")]
    GateResultRunDirectoryMismatch { path: PathBuf },
    #[error("gate \"{job}\" has status \"{status}\"; every gate must be passed")]
    GateNotPassed { job: String, status: String },
    #[error("gate \"{job}\" is missing a path or sha256 binding in the plain index")]
    GateMissingBinding { job: String },
    #[error("gate \"{job}\" identifier is malformed or unsafe")]
    GateJobUnsafe { job: String },
    #[error("gate-result path is malformed or unsafe: {0}")]
    GateResultPathUnsafe(String),
    #[error("gate-result at {path} has no retained logs; cannot bind a retained log")]
    NoRetainedLog { path: PathBuf },
    #[error("retained log path component is malformed or unsafe: {0}")]
    RetainedLogPathUnsafe(String),
    #[error("retained log read failed at {path}: {error}")]
    RetainedLogRead { path: PathBuf, error: String },
    #[error("retained log at {path} is not a regular file (symlink)")]
    RetainedLogNotRegular { path: PathBuf },
    #[error("retained log at {path} sha256 does not match the gate-result recorded value")]
    RetainedLogHashMismatch { path: PathBuf },
    #[error("retained log at {path} is {size} bytes; maximum {max}")]
    RetainedLogOversize { path: PathBuf, size: u64, max: u64 },
    #[error("gate-result at {path} has no artifact hashes")]
    NoArtifactHashes { path: PathBuf },
    #[error("gate-result at {path} has too many artifact hashes ({count}); maximum {max}")]
    TooManyArtifactHashes {
        path: PathBuf,
        count: usize,
        max: usize,
    },
    #[error("artifact hash at {path} is not a 64-char lowercase hex SHA-256")]
    ArtifactHashMalformed { path: PathBuf },
    #[error("duplicate gate job: {0}")]
    DuplicateJob(String),
    #[error("required job not present or not passed: {0}")]
    RequiredJobMissing(String),
    #[error("gate-result sha256 field is not a 64-char lowercase hex SHA-256 for job \"{0}\"")]
    PlainGateShaMalformed(String),
    #[error("path component is a symlink; refusing: {0}")]
    FileSymlink(PathBuf),
    #[error("output evidence-index failed schema validation:\n{0}")]
    OutputSchemaInvalid(String),
    #[error("publish failed: {0}")]
    Publish(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("canonical serialization failed: {0}")]
    CanonicalSerialization(String),
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load + validate every input and produce a canonical [`EvidenceIndex`].
///
/// Every check fails closed: a single violation returns
/// [`EvidenceBindError`] and no index is produced. This function performs no
/// filesystem writes.
pub fn load_and_bind(request: &EvidenceBindRequest) -> Result<EvidenceIndex, EvidenceBindError> {
    // Current source is the verification anchor for the plain index commit,
    // every gate-result source, and the provenance source_commit.
    let (head_commit, head_tree) = current_repo_source()?;

    // Evidence root must be a real (non-symlink) directory.
    validate_evidence_root(&request.evidence_root)?;

    // Plain gate index: bounded read, typed parse, coherence checks.
    let plain_bytes = read_regular_nofollow(&request.plain_index, MAX_PLAIN_INDEX_BYTES).map_err(
        |e| match e {
            ReadFileError::Oversize { size, max } => {
                EvidenceBindError::PlainIndexOversize { size, max }
            }
            other => EvidenceBindError::PlainIndexRead {
                path: request.plain_index.clone(),
                error: other.to_string(),
            },
        },
    )?;
    let plain: PlainGateIndex =
        serde_json::from_slice(&plain_bytes).map_err(|e| EvidenceBindError::PlainIndexParse {
            path: request.plain_index.clone(),
            error: e.to_string(),
        })?;
    validate_plain_index(&plain, &head_commit)?;

    // Production provenance: bounded read, schema id check, schema validation,
    // source binding. production_provenance_sha256 is the SHA-256 of the raw
    // provenance file bytes (mirrors artifact_inspector's binding).
    let provenance_bytes = read_regular_nofollow(&request.provenance, MAX_PROVENANCE_BYTES)
        .map_err(|e| match e {
            ReadFileError::Oversize { size, max } => {
                EvidenceBindError::ProvenanceOversize { size, max }
            }
            other => EvidenceBindError::ProvenanceRead {
                path: request.provenance.clone(),
                error: other.to_string(),
            },
        })?;
    let provenance_value: serde_json::Value =
        serde_json::from_slice(&provenance_bytes).map_err(|e| {
            EvidenceBindError::ProvenanceParse {
                path: request.provenance.clone(),
                error: e.to_string(),
            }
        })?;
    if provenance_value.get("schema").and_then(|v| v.as_str()) != Some(PROVENANCE_SCHEMA) {
        return Err(EvidenceBindError::ProvenanceSchema);
    }
    if provenance_value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err(EvidenceBindError::ProvenanceVersion);
    }
    validate_against_kind(&provenance_value, "production-provenance")
        .map_err(map_provenance_err)?;
    let provenance_sha256 = hex::encode(Sha256::digest(&provenance_bytes));
    validate_provenance_source(&provenance_value, &head_commit)?;

    // Bind each gate. Records are sorted by job for deterministic output.
    let mut records: Vec<EvidenceRecord> = Vec::with_capacity(plain.gates.len());
    let mut seen_jobs: Vec<String> = Vec::with_capacity(plain.gates.len());
    for gate in &plain.gates {
        let record = bind_gate(gate, &request.evidence_root, &head_commit, &head_tree)?;
        if seen_jobs.iter().any(|j| j == &record.job) {
            return Err(EvidenceBindError::DuplicateJob(record.job));
        }
        seen_jobs.push(record.job.clone());
        records.push(record);
    }
    records.sort_by(|a, b| a.job.cmp(&b.job));
    if records.len() > MAX_RECORDS {
        return Err(EvidenceBindError::PlainIndexTooManyGates(
            records.len(),
            MAX_RECORDS,
        ));
    }

    // Required jobs must all be present (and passed — every gate is passed by
    // construction above, so presence suffices).
    for required in &plain.required_jobs {
        if !records.iter().any(|r| &r.job == required) {
            return Err(EvidenceBindError::RequiredJobMissing(required.clone()));
        }
    }

    let index = EvidenceIndex {
        schema: EVIDENCE_INDEX_SCHEMA.to_string(),
        version: EVIDENCE_INDEX_VERSION,
        source_commit: head_commit,
        production_provenance_sha256: provenance_sha256,
        records,
    };

    // Defense in depth: validate the produced document against the checked-in
    // evidence-index schema before handing it to the publisher.
    let value = serde_json::to_value(&index)
        .map_err(|e| EvidenceBindError::CanonicalSerialization(e.to_string()))?;
    validate_against_kind(&value, "evidence-index")?;
    Ok(index)
}

/// Publish an [`EvidenceIndex`] to `path` with create-only semantics: a private
/// random-named temporary file is written and fsynced in the output parent,
/// then hard-linked to the final name. If the final name already exists the
/// link fails and no bytes reach the destination. Symlinked ancestors and
/// group/world-writable parents are rejected, mirroring the audited
/// `release_preflight::publish_create_only` posture.
pub fn publish_create_only(
    path: &Path,
    index: &EvidenceIndex,
) -> Result<EvidenceBindOutcome, EvidenceBindError> {
    // Re-validate the document before any filesystem write. A Rust caller must
    // not be able to bypass schema validation by constructing an EvidenceIndex
    // directly.
    let value = serde_json::to_value(index)
        .map_err(|e| EvidenceBindError::CanonicalSerialization(e.to_string()))?;
    validate_against_kind(&value, "evidence-index")?;
    let bytes = index
        .canonical_bytes()
        .map_err(|e| EvidenceBindError::CanonicalSerialization(e.to_string()))?;

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| EvidenceBindError::Publish("output requires a file name".into()))?;
    let temp_name = std::ffi::OsString::from(random_temp_name()?);

    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let dirfd = open_private_dirfd(parent)?;
        let mut file = openat_exclusive(&dirfd, &temp_name)?;
        let publication = (|| -> Result<(), EvidenceBindError> {
            file.write_all(&bytes)
                .map_err(|e| EvidenceBindError::Publish(format!("write failed: {e}")))?;
            file.sync_all()
                .map_err(|e| EvidenceBindError::Publish(format!("sync failed: {e}")))?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = unlinkat_name(&dirfd, &temp_name);
        }
        publication?;

        // Hard-link the fully-synced temp file to the final name. Fails if the
        // final name already exists, preserving create-only semantics. Clean up
        // the temp name on link failure so re-runs against an existing output
        // do not leak random-named temp files in the output directory.
        if let Err(error) = linkat_name(&dirfd, &temp_name, file_name) {
            let _ = unlinkat_name(&dirfd, &temp_name);
            return Err(error);
        }
        let _ = unlinkat_name(&dirfd, &temp_name);

        let mut warnings = Vec::new();
        let file_fd = file.into_raw_fd();
        if unsafe { libc::close(file_fd) } < 0 {
            warnings.push("file close failed; destination durability is unknown");
        }
        if unsafe { libc::fsync(dirfd.as_raw_fd()) } < 0 {
            warnings.push("parent directory sync failed; destination durability is unknown");
        }
        Ok(EvidenceBindOutcome {
            durable: warnings.is_empty(),
            warnings,
            source_commit: index.source_commit.clone(),
            production_provenance_sha256: index.production_provenance_sha256.clone(),
            record_count: index.records.len(),
            out: path.to_path_buf(),
        })
    }
    #[cfg(not(unix))]
    {
        let parent_meta =
            std::fs::symlink_metadata(parent).map_err(|e| EvidenceBindError::Io(e.to_string()))?;
        if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
            return Err(EvidenceBindError::Publish(
                "output parent must be a real directory".into(),
            ));
        }
        let temp_path = parent.join(&temp_name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| EvidenceBindError::Io(e.to_string()))?;
        let publication = (|| -> Result<(), EvidenceBindError> {
            file.write_all(&bytes)
                .map_err(|e| EvidenceBindError::Publish(format!("write failed: {e}")))?;
            file.sync_all()
                .map_err(|e| EvidenceBindError::Publish(format!("sync failed: {e}")))?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        publication?;
        std::fs::hard_link(&temp_path, path).map_err(|e| EvidenceBindError::Io(e.to_string()))?;
        let _ = std::fs::remove_file(&temp_path);
        Ok(EvidenceBindOutcome {
            durable: true,
            warnings: Vec::new(),
            source_commit: index.source_commit.clone(),
            production_provenance_sha256: index.production_provenance_sha256.clone(),
            record_count: index.records.len(),
            out: path.to_path_buf(),
        })
    }
}

/// Convenience: [`load_and_bind`] + [`publish_create_only`].
pub fn bind(request: &EvidenceBindRequest) -> Result<EvidenceBindOutcome, EvidenceBindError> {
    let index = load_and_bind(request)?;
    publish_create_only(&request.out, &index)
}

// ---------------------------------------------------------------------------
// Bind a single gate -> EvidenceRecord
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn bind_gate(
    gate: &PlainGate,
    evidence_root: &Path,
    head_commit: &str,
    head_tree: &str,
) -> Result<EvidenceRecord, EvidenceBindError> {
    // Job id is a schema logicalId: bounded, safe character class, denylist.
    if !is_safe_path(&gate.job, MAX_LOGICAL_ID_BYTES) {
        return Err(EvidenceBindError::GateJobUnsafe {
            job: gate.job.clone(),
        });
    }

    // Every gate must be passed.
    if gate.status != "passed" {
        return Err(EvidenceBindError::GateNotPassed {
            job: gate.job.clone(),
            status: gate.status.clone(),
        });
    }
    let path_str = gate
        .path
        .clone()
        .ok_or_else(|| EvidenceBindError::GateMissingBinding {
            job: gate.job.clone(),
        })?;
    let sha_str = gate
        .sha256
        .clone()
        .ok_or_else(|| EvidenceBindError::GateMissingBinding {
            job: gate.job.clone(),
        })?;

    if !is_sha256(&sha_str) {
        return Err(EvidenceBindError::PlainGateShaMalformed(gate.job.clone()));
    }
    if !is_safe_path(&path_str, MAX_RELATIVE_PATH_BYTES) {
        return Err(EvidenceBindError::GateResultPathUnsafe(path_str));
    }

    // Resolve the gate-result file under the evidence root, refusing symlinks
    // and any path that escapes the root.
    let gate_result_path = resolve_under_root(evidence_root, &path_str)?;
    let gate_result_bytes = read_regular_nofollow(&gate_result_path, MAX_GATE_RESULT_BYTES)
        .map_err(|e| match e {
            ReadFileError::Symlink => EvidenceBindError::GateResultNotRegular {
                path: gate_result_path.clone(),
            },
            other => EvidenceBindError::GateResultRead {
                path: gate_result_path.clone(),
                error: other.to_string(),
            },
        })?;

    // Referenced gate-result SHA: re-measure from disk and compare to the plain
    // index's recorded value. Never synthesize.
    let measured_sha = hex::encode(Sha256::digest(&gate_result_bytes));
    if measured_sha != sha_str {
        return Err(EvidenceBindError::GateResultHashMismatch {
            path: gate_result_path,
        });
    }

    // Parse + schema-validate the gate-result.
    let gate_value: serde_json::Value =
        serde_json::from_slice(&gate_result_bytes).map_err(|e| {
            EvidenceBindError::GateResultParse {
                path: gate_result_path.clone(),
                error: e.to_string(),
            }
        })?;
    if gate_value.get("schema").and_then(|v| v.as_str()) != Some(GATE_RESULT_SCHEMA) {
        return Err(EvidenceBindError::GateResultSchemaId {
            path: gate_result_path,
        });
    }
    validate_against_kind(&gate_value, "gate-result").map_err(|e| match e {
        EvidenceBindError::OutputSchemaInvalid(errors) => {
            EvidenceBindError::GateResultSchemaInvalid {
                path: gate_result_path.clone(),
                errors,
            }
        }
        other => other,
    })?;

    // Gate-result source must match the current repository HEAD.
    let src_commit = gate_value
        .get("source")
        .and_then(|v| v.get("commit"))
        .and_then(|v| v.as_str());
    let src_tree = gate_value
        .get("source")
        .and_then(|v| v.get("tree"))
        .and_then(|v| v.as_str());
    if src_commit != Some(head_commit) || src_tree != Some(head_tree) {
        return Err(EvidenceBindError::GateResultSourceMismatch {
            path: gate_result_path,
        });
    }

    // required_row.status must be passed and exit_code == 0 (defense in depth
    // even though the plain index already asserted status == "passed").
    let required_status = gate_value
        .get("required_row")
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str());
    let exit_code = gate_value.get("exit_code").and_then(|v| v.as_i64());
    if required_status != Some("passed") || exit_code != Some(0) {
        return Err(EvidenceBindError::GateNotPassed {
            job: gate.job.clone(),
            status: gate.status.clone(),
        });
    }

    // Cross-check evidence.run_directory against the gate-result path location.
    let run_directory = gate_value
        .get("evidence")
        .and_then(|v| v.get("run_directory"))
        .and_then(|v| v.as_str());
    let path_components: Vec<&str> = path_str.split('/').collect();
    let run_dir_in_path = path_components
        .get(path_components.len().saturating_sub(2))
        .copied();
    if run_directory.is_none() || run_dir_in_path != run_directory {
        return Err(EvidenceBindError::GateResultRunDirectoryMismatch {
            path: gate_result_path,
        });
    }

    // Pick the canonical retained log (lowest run, stdout before stderr).
    let retained_logs = gate_value
        .get("retained_logs")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| EvidenceBindError::NoRetainedLog {
            path: gate_result_path.clone(),
        })?;
    let chosen_idx = pick_canonical_retained_log(retained_logs).ok_or_else(|| {
        EvidenceBindError::NoRetainedLog {
            path: gate_result_path.clone(),
        }
    })?;
    let chosen = &retained_logs[chosen_idx];
    let log_basename = chosen.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
        EvidenceBindError::NoRetainedLog {
            path: gate_result_path.clone(),
        }
    })?;
    let recorded_log_sha = chosen
        .get("sha256")
        .and_then(|v| v.as_str())
        .ok_or_else(|| EvidenceBindError::NoRetainedLog {
            path: gate_result_path.clone(),
        })?;

    // Construct the retained-log path relative to the evidence root: the
    // parent of the gate-result file joined with the retained-log basename.
    let mut log_parts: Vec<&str> = path_str.split('/').collect();
    log_parts.pop(); // drop "gate-result.json"
    log_parts.push(log_basename);
    let retained_log_rel = log_parts.join("/");
    if !is_safe_path(&retained_log_rel, MAX_RELATIVE_PATH_BYTES) {
        return Err(EvidenceBindError::RetainedLogPathUnsafe(retained_log_rel));
    }

    // Independently measure the retained log from disk: refuse symlinks,
    // recompute SHA-256, enforce the 16 KiB size bound.
    let retained_log_path = resolve_under_root(evidence_root, &retained_log_rel)?;
    let log_bytes =
        read_regular_nofollow(&retained_log_path, MAX_RETAINED_LOG_BYTES).map_err(|e| match e {
            ReadFileError::Symlink => EvidenceBindError::RetainedLogNotRegular {
                path: retained_log_path.clone(),
            },
            ReadFileError::Oversize { size, max } => EvidenceBindError::RetainedLogOversize {
                path: retained_log_path.clone(),
                size,
                max,
            },
            other => EvidenceBindError::RetainedLogRead {
                path: retained_log_path.clone(),
                error: other.to_string(),
            },
        })?;
    let measured_log_sha = hex::encode(Sha256::digest(&log_bytes));
    if measured_log_sha != recorded_log_sha {
        return Err(EvidenceBindError::RetainedLogHashMismatch {
            path: retained_log_path,
        });
    }

    // Artifact hashes: collect, validate, dedupe, sort. Max 256 unique.
    let artifact_hashes = gate_value
        .get("artifact_hashes")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| EvidenceBindError::NoArtifactHashes {
            path: gate_result_path.clone(),
        })?;
    let mut hashes: Vec<String> = Vec::with_capacity(artifact_hashes.len());
    for entry in artifact_hashes {
        let sha = entry
            .get("sha256")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EvidenceBindError::ArtifactHashMalformed {
                path: gate_result_path.clone(),
            })?;
        if !is_sha256(sha) {
            return Err(EvidenceBindError::ArtifactHashMalformed {
                path: gate_result_path,
            });
        }
        if !hashes.iter().any(|h| h == sha) {
            hashes.push(sha.to_string());
        }
    }
    if hashes.len() > MAX_ARTIFACT_HASHES {
        return Err(EvidenceBindError::TooManyArtifactHashes {
            path: gate_result_path,
            count: hashes.len(),
            max: MAX_ARTIFACT_HASHES,
        });
    }
    hashes.sort();

    Ok(EvidenceRecord {
        job: gate.job.clone(),
        gate_result_path: path_str,
        gate_result_sha256: sha_str,
        retained_log_path: retained_log_rel,
        retained_log_sha256: measured_log_sha,
        artifact_sha256: hashes,
    })
}

// ---------------------------------------------------------------------------
// Plain-index coherence validation
// ---------------------------------------------------------------------------

fn validate_plain_index(
    plain: &PlainGateIndex,
    head_commit: &str,
) -> Result<(), EvidenceBindError> {
    if !is_commit40(&plain.commit) {
        return Err(EvidenceBindError::PlainIndexCommitMalformed(
            plain.commit.clone(),
        ));
    }
    if plain.commit != head_commit {
        return Err(EvidenceBindError::SourceCommitMismatch {
            index: plain.commit.clone(),
            head: head_commit.to_string(),
        });
    }
    if plain.gates.is_empty() {
        return Err(EvidenceBindError::PlainIndexEmpty);
    }
    if plain.gates.len() > MAX_RECORDS {
        return Err(EvidenceBindError::PlainIndexTooManyGates(
            plain.gates.len(),
            MAX_RECORDS,
        ));
    }
    if plain.gate_count as usize != plain.gates.len() {
        return Err(EvidenceBindError::PlainIndexGateCountMismatch {
            declared: plain.gate_count,
            actual: plain.gates.len(),
        });
    }
    // Validate status values and count coherence.
    for gate in &plain.gates {
        if gate.status != "passed" && gate.status != "failed" {
            return Err(EvidenceBindError::PlainIndexBadStatus {
                job: gate.job.clone(),
                status: gate.status.clone(),
            });
        }
        if gate.status == "passed" && !gate.error.is_empty() {
            return Err(EvidenceBindError::PlainIndexPassedHasError(
                gate.job.clone(),
            ));
        }
    }
    let passed = plain.gates.iter().filter(|g| g.status == "passed").count() as u32;
    let failed = plain.gates.iter().filter(|g| g.status == "failed").count() as u32;
    if passed != plain.passed {
        return Err(EvidenceBindError::PlainIndexPassedMismatch {
            declared: plain.passed,
            actual: passed,
        });
    }
    if failed != plain.failed {
        return Err(EvidenceBindError::PlainIndexFailedMismatch {
            declared: plain.failed,
            actual: failed,
        });
    }
    if passed + failed != plain.gate_count {
        return Err(EvidenceBindError::PlainIndexCountArithmetic {
            passed,
            failed,
            gate_count: plain.gate_count,
        });
    }
    Ok(())
}

fn validate_provenance_source(
    value: &serde_json::Value,
    head_commit: &str,
) -> Result<(), EvidenceBindError> {
    let source_commit = value
        .get("source_commit")
        .and_then(|v| v.as_str())
        .ok_or(EvidenceBindError::ProvenanceSchema)?;
    if source_commit != head_commit {
        return Err(EvidenceBindError::ProvenanceSourceMismatch {
            provenance: source_commit.to_string(),
            head: head_commit.to_string(),
        });
    }
    Ok(())
}

fn validate_evidence_root(root: &Path) -> Result<(), EvidenceBindError> {
    let meta = std::fs::symlink_metadata(root).map_err(|e| EvidenceBindError::Io(e.to_string()))?;
    if meta.is_symlink() {
        return Err(EvidenceBindError::EvidenceRootSymlink(root.to_path_buf()));
    }
    if !meta.is_dir() {
        return Err(EvidenceBindError::EvidenceRootNotDir(root.to_path_buf()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Canonical retained-log selection
// ---------------------------------------------------------------------------

fn pick_canonical_retained_log(logs: &[serde_json::Value]) -> Option<usize> {
    let mut best: Option<(i64, i64, usize)> = None;
    for (i, log) in logs.iter().enumerate() {
        let run = log.get("run").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
        let stream = log.get("stream").and_then(|v| v.as_str()).unwrap_or("");
        let rank = (run, stream_rank(stream));
        match &best {
            None => best = Some((rank.0, rank.1, i)),
            Some((br, bs, _)) if (rank.0, rank.1) < (*br, *bs) => {
                best = Some((rank.0, rank.1, i));
            }
            _ => {}
        }
    }
    best.map(|(_, _, i)| i)
}

// ---------------------------------------------------------------------------
// Schema validation (in-memory, against checked-in schema files)
// ---------------------------------------------------------------------------

/// Validate a JSON value against a checked-in schema kind. The kind maps to
/// `schemas/rutile.<kind>.v1.schema.json` via [`crate::evidence::schema_path`].
/// Infrastructure failures (unknown kind, unreadable/unparseable schema,
/// compile errors) return the shared `Schema*` variants; validation failures
/// return [`EvidenceBindError::OutputSchemaInvalid`] (callers rewrap into a
/// context-specific variant when desired).
fn validate_against_kind(value: &serde_json::Value, kind: &str) -> Result<(), EvidenceBindError> {
    let schema_file = crate::evidence::schema_path(kind).ok_or_else(|| {
        EvidenceBindError::SchemaUnknown(
            kind.to_string(),
            crate::evidence::KNOWN_SCHEMA_KINDS.join(", "),
        )
    })?;
    let schema_str =
        std::fs::read_to_string(&schema_file).map_err(|e| EvidenceBindError::SchemaRead {
            path: schema_file.clone(),
            error: e.to_string(),
        })?;
    let schema_value: serde_json::Value = serde_json::from_str(&schema_str)
        .map_err(|e| EvidenceBindError::SchemaParse(e.to_string()))?;
    let validator = jsonschema::validator_for(&schema_value)
        .map_err(|e| EvidenceBindError::SchemaCompile(e.to_string()))?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("  - {e}"))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(EvidenceBindError::OutputSchemaInvalid(errors.join("\n")))
    }
}

/// Remap the generic output-schema-invalid error into the provenance-specific
/// variant; pass through infrastructure errors unchanged.
fn map_provenance_err(e: EvidenceBindError) -> EvidenceBindError {
    match e {
        EvidenceBindError::OutputSchemaInvalid(msg) => {
            EvidenceBindError::ProvenanceSchemaInvalid(msg)
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// File-read primitive (portable, symlink-refusing, size-bounded)
// ---------------------------------------------------------------------------

/// Internal read-error type for [`read_regular_nofollow`]. Portable: does not
/// depend on libc errno values.
#[derive(Debug)]
enum ReadFileError {
    Symlink,
    NotRegular,
    Oversize { size: u64, max: u64 },
    Io(std::io::Error),
}

impl std::fmt::Display for ReadFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadFileError::Symlink => f.write_str("path is a symlink"),
            ReadFileError::NotRegular => f.write_str("path is not a regular file"),
            ReadFileError::Oversize { size, max } => {
                write!(f, "file size {size} exceeds maximum {max}")
            }
            ReadFileError::Io(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

/// Read a file refusing symlinks, non-regular files, and oversize inputs.
#[cfg(unix)]
fn read_regular_nofollow(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ReadFileError> {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    // Open with O_NOFOLLOW + fstat so a symlink swapped in between a metadata
    // check and the read (TOCTOU) cannot redirect the read. Mirrors
    // readiness_keystone::read_regular_file.
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ReadFileError::Io(std::io::Error::other("input path is not valid")))?;
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
            Some(libc::ELOOP) => ReadFileError::Symlink,
            _ => ReadFileError::Io(err),
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } < 0 {
        return Err(ReadFileError::Io(std::io::Error::last_os_error()));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(ReadFileError::NotRegular);
    }
    if stat.st_size as u64 > max_bytes {
        return Err(ReadFileError::Oversize {
            size: stat.st_size as u64,
            max: max_bytes,
        });
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadFileError::Io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(ReadFileError::Oversize {
            size: bytes.len() as u64,
            max: max_bytes,
        });
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_regular_nofollow(_path: &Path, _max_bytes: u64) -> Result<Vec<u8>, ReadFileError> {
    // Fail closed: the non-Unix metadata+open sequence cannot meet the same
    // symlink-rejection guarantee as O_NOFOLLOW + fstat.
    Err(ReadFileError::Io(std::io::Error::other(
        "evidence safe file read is unix-only (O_NOFOLLOW + fstat)",
    )))
}

/// Resolve a repo-relative path under `root`, refusing symlinked components so
/// no symlinked directory can redirect the read outside the evidence tree.
/// `rel` must already pass [`is_safe_path`] (no `..`, no absolute, safe chars).
fn resolve_under_root(root: &Path, rel: &str) -> Result<PathBuf, EvidenceBindError> {
    let mut current = root.to_path_buf();
    for component in rel.split('/') {
        current.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            if meta.is_symlink() {
                return Err(EvidenceBindError::FileSymlink(current));
            }
        }
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// Path + hash primitives
// ---------------------------------------------------------------------------

/// Validate a repo-relative path or logical id. Mirrors the evidence-index
/// schema's `relativePath`/`logicalId` definitions so the code is never weaker
/// than the published contract: rejects empty, oversize (>max_bytes), absolute
/// paths, any `..` substring, characters outside `[A-Za-z0-9._/-]`, empty or
/// `.` components, secret-looking substrings (token/secret/credentials/
/// password, case-insensitive), host-local path substrings
/// (`/users/`, `/home/`, `/private/`, `/var/folders/`), private/loopback IPv4
/// shapes, and IPv6 local prefixes.
fn is_safe_path(value: &str, max_bytes: usize) -> bool {
    if value.is_empty() || value.len() > max_bytes {
        return false;
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return false;
    }
    if value.starts_with('/') {
        return false;
    }
    if value.contains("..") {
        return false;
    }
    for component in value.split('/') {
        if component.is_empty() {
            return false;
        }
        if component == "." {
            return false;
        }
    }
    !matches_path_denylist(value)
}

fn matches_path_denylist(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    const SECRET_SUBSTRINGS: &[&str] = &["token", "secret", "credentials", "password"];
    const HOST_LOCAL_SUBSTRINGS: &[&str] = &["/users/", "/home/", "/private/", "/var/folders/"];
    for needle in SECRET_SUBSTRINGS.iter().chain(HOST_LOCAL_SUBSTRINGS.iter()) {
        if lower.contains(needle) {
            return true;
        }
    }
    if lower.starts_with("::1")
        || lower.starts_with("fc00:")
        || lower.starts_with("fd00:")
        || lower.starts_with("fe80:")
    {
        return true;
    }
    contains_private_or_loopback_ipv4(&lower)
}

/// Detect the schema's `(127\.|10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|
/// 169\.254\.)\d{1,3}\.\d{1,3}` private/loopback IPv4 shape as a substring.
/// Ported verbatim from `readiness::contains_private_or_loopback_ipv4` so the
/// two lanes cannot diverge: the readiness keystone is the canonical
/// implementation of this schema regex.
fn contains_private_or_loopback_ipv4(lower: &str) -> bool {
    for token in lower.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let octets: Vec<&str> = token.split('.').collect();
        // 1-octet prefix (127. or 10.) needs prefix + two trailing octets.
        for start in 0..octets.len().saturating_sub(2) {
            let o1 = octets[start];
            if (o1 == "127" || o1 == "10")
                && is_schema_octet(octets[start + 1])
                && is_schema_octet(octets[start + 2])
            {
                return true;
            }
        }
        // 2-octet prefix (192.168. / 172.16-31. / 169.254.) needs prefix + two
        // trailing octets.
        for start in 0..octets.len().saturating_sub(3) {
            let o1 = octets[start];
            let o2 = octets[start + 1];
            let prefix_matches = (o1 == "192" && o2 == "168")
                || (o1 == "172" && matches_octet_range(o2, 16, 31))
                || (o1 == "169" && o2 == "254");
            if prefix_matches
                && is_schema_octet(octets[start + 2])
                && is_schema_octet(octets[start + 3])
            {
                return true;
            }
        }
    }
    false
}

/// Mirror the schema's `\d{1,3}` octet shape: 1-3 ASCII digits, value-agnostic
/// (the schema regex does not constrain octets to 0-255, so neither do we).
fn is_schema_octet(field: &str) -> bool {
    matches!(field.len(), 1..=3) && field.bytes().all(|b| b.is_ascii_digit())
}

/// Return `true` when `field` is a 1-3 digit decimal whose numeric value lies
/// in `lo..=hi`. Used for the `172.16-31.x.x` prefix shape.
fn matches_octet_range(field: &str, lo: u16, hi: u16) -> bool {
    if !is_schema_octet(field) {
        return false;
    }
    let value = field.parse::<u16>().expect("1-3 ASCII digits fit in u16");
    (lo..=hi).contains(&value)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_commit40(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------------------
// Current repository source
// ---------------------------------------------------------------------------

/// Pinned repository root derived at compile time from the `xtask` crate
/// location, mirroring `release_preflight::workspace_root` and
/// `evidence::workspace_root`. Source binding must never follow the runtime
/// working directory or inherited Git environment.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a workspace member")
}

fn current_repo_source() -> Result<(String, String), EvidenceBindError> {
    let repo = workspace_root();
    let output = tool_process::git_isolated(
        repo,
        &["--no-replace-objects", "rev-parse", "HEAD", "HEAD^{tree}"],
        &[],
    )
    .map_err(|e| EvidenceBindError::CurrentSourceIo(e.to_string()))?;
    if !output.status.success() || output.stdout.len() > 256 {
        return Err(EvidenceBindError::CurrentSourceMalformed);
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| EvidenceBindError::CurrentSourceMalformed)?;
    let mut lines = text.lines();
    let commit = lines
        .next()
        .ok_or(EvidenceBindError::CurrentSourceMalformed)?
        .to_string();
    let tree = lines
        .next()
        .ok_or(EvidenceBindError::CurrentSourceMalformed)?
        .to_string();
    if lines.next().is_some() || !is_commit40(&commit) || !is_commit40(&tree) {
        return Err(EvidenceBindError::CurrentSourceMalformed);
    }
    Ok((commit, tree))
}

// ---------------------------------------------------------------------------
// Create-only publication primitives (mirror release_preflight's posture)
// ---------------------------------------------------------------------------

fn random_temp_name() -> Result<String, EvidenceBindError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| {
        EvidenceBindError::Publish(format!("failed to generate random temp name: {e}"))
    })?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(".tmp.{hex}"))
}

#[cfg(unix)]
fn open_private_dirfd(parent: &Path) -> Result<std::os::fd::OwnedFd, EvidenceBindError> {
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
        return Err(EvidenceBindError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let mut dirfd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

    for component in parent.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(EvidenceBindError::Publish(
                    "output parent must not contain parent-directory references".into(),
                ));
            }
            std::path::Component::Normal(name) => {
                let c_name = CString::new(name.as_bytes()).map_err(|_| {
                    EvidenceBindError::Publish("output parent path is not valid".into())
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
                        | std::io::ErrorKind::PermissionDenied => EvidenceBindError::Publish(
                            "output parent must be a real directory".into(),
                        ),
                        _ => EvidenceBindError::Io(err.to_string()),
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
fn validate_dirfd(
    dirfd: &std::os::fd::OwnedFd,
    require_owner: bool,
) -> Result<(), EvidenceBindError> {
    use std::os::fd::AsRawFd;

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(dirfd.as_raw_fd(), &mut stat) } < 0 {
        return Err(EvidenceBindError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(EvidenceBindError::Publish(
            "output parent must be a real directory".into(),
        ));
    }
    if (stat.st_mode as u32 & 0o022) != 0 {
        return Err(EvidenceBindError::Publish(
            "output parent path traverses a writable directory".into(),
        ));
    }
    if require_owner && stat.st_uid != unsafe { libc::geteuid() } {
        return Err(EvidenceBindError::Publish(
            "output parent must be a private directory owned by the current user".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn openat_exclusive(
    dirfd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<std::fs::File, EvidenceBindError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_name = CString::new(name.as_bytes())
        .map_err(|_| EvidenceBindError::Publish("output name is not valid".into()))?;
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
            EvidenceBindError::Publish("output already exists".into())
        } else {
            EvidenceBindError::Io(err.to_string())
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
) -> Result<(), EvidenceBindError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let c_from = CString::new(from.as_bytes())
        .map_err(|_| EvidenceBindError::Publish("invalid temporary name".into()))?;
    let c_to = CString::new(to.as_bytes())
        .map_err(|_| EvidenceBindError::Publish("invalid output name".into()))?;
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
            EvidenceBindError::Publish("output already exists".into())
        } else {
            EvidenceBindError::Io(err.to_string())
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    // -----------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// Current repository HEAD (commit, tree) so fixtures can bind the real
    /// source. Mirrors `release_preflight` / `evidence` test convention.
    fn current_head() -> (String, String) {
        current_repo_source().expect("repository HEAD must be derivable in tests")
    }

    fn temp_root() -> TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate lives in the workspace")
            .join("target")
            .join("tmp");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(&root).unwrap()
    }

    /// Build a minimal valid gate-result JSON value for `job` bound to HEAD.
    fn gate_result_value(
        job: &str,
        commit: &str,
        tree: &str,
        stdout_sha: &str,
    ) -> serde_json::Value {
        json!({
            "schema": "rutile.gate-result.v1",
            "command_id": format!("evidence-{job}"),
            "profile": "release",
            "source": {"commit": commit, "tree": tree, "dirty": false},
            "evidence": {"run_directory": "run-1000-1-0"},
            "runner": {"platform": "macos", "architecture": "arm64", "name": "local"},
            "started_unix_ms": 1000_i64,
            "ended_unix_ms": 2000_i64,
            "exit_code": 0,
            "tests": {"total": 1, "passed": 1, "failed": 0, "ignored": 0, "skipped": 0},
            "required_row": {"name": format!("evidence-{job}"), "required": true, "status": "passed"},
            "artifact_hashes": [{
                "path": format!("target/evidence/{commit}/{job}/artifact.bin"),
                "sha256": format!("{:064x}", 1_u128),
                "identity": {"device": 1, "inode": 2, "bytes": 3}
            }],
            "retained_logs": [{
                "run": 1,
                "stream": "stdout",
                "path": "run-0001.stdout.log",
                "bytes": 5_i64,
                "sha256": stdout_sha
            }],
            "runs": [{
                "run": 1,
                "status": "passed",
                "reaped": true,
                "stage_traces": 0,
                "resize_traces": 0,
                "error": null
            }]
        })
    }

    fn write_provenance(path: &Path, commit: &str) {
        let provenance = json!({
            "schema": "rutile.production-provenance.v1",
            "version": 1,
            "product": "rutile-app",
            "product_version": "0.2.2",
            "source_commit": commit,
            "source_tree_clean": true,
            "toolchain": {
                "rustc_version": "rustc 1.80.0",
                "host_triple": "aarch64-apple-darwin",
                "target_triple": "aarch64-apple-darwin"
            },
            "features": [],
            "candidate_sha256": format!("{:064x}", 9_u128),
            "reproducibility": {
                "source_date_epoch": 0,
                "remap_path_prefix": true,
                "target_root": "target/prod",
                "controls_origin": "ambient_build_env"
            },
            "built_at": "2026-01-01T00:00:00Z"
        });
        fs::write(path, serde_json::to_vec_pretty(&provenance).unwrap()).unwrap();
    }

    /// Write a complete evidence tree for one gate and return a request
    /// pointing at it. The output path is `tmp/out.json`.
    fn build_single_gate_tree(
        tmp: &TempDir,
        job: &str,
        required: &[&str],
    ) -> (EvidenceBindRequest, String, String, String) {
        let (commit, tree) = current_head();
        let evidence_root = tmp.path().join("evidence");
        let gate_dir = evidence_root.join(&commit).join(job).join("run-1000-1-0");
        fs::create_dir_all(&gate_dir).unwrap();

        let stdout = b"hello";
        let stdout_sha = sha256_hex(stdout);
        fs::write(gate_dir.join("run-0001.stdout.log"), stdout).unwrap();

        let gate_value = gate_result_value(job, &commit, &tree, &stdout_sha);
        let gate_bytes = serde_json::to_vec_pretty(&gate_value).unwrap();
        fs::write(gate_dir.join("gate-result.json"), &gate_bytes).unwrap();
        let gate_sha = sha256_hex(&gate_bytes);

        let gate_rel = format!("{commit}/{job}/run-1000-1-0/gate-result.json");
        let plain = json!({
            "commit": commit,
            "gate_count": 1,
            "passed": 1,
            "failed": 0,
            "required_jobs": required,
            "gates": [{
                "job": job,
                "path": gate_rel,
                "sha256": gate_sha,
                "status": "passed",
                "error": ""
            }]
        });
        let plain_path = tmp.path().join("plain-index.json");
        fs::write(&plain_path, serde_json::to_vec_pretty(&plain).unwrap()).unwrap();

        let provenance_path = tmp.path().join("provenance.json");
        write_provenance(&provenance_path, &commit);

        let request = EvidenceBindRequest {
            plain_index: plain_path,
            provenance: provenance_path,
            evidence_root,
            out: tmp.path().join("out.json"),
        };
        (request, commit, tree, gate_sha)
    }

    // -----------------------------------------------------------------
    // Happy path
    // -----------------------------------------------------------------

    #[test]
    fn bind_single_gate_produces_canonical_index() {
        let tmp = temp_root();
        let (request, commit, _tree, gate_sha) = build_single_gate_tree(&tmp, "portable", &[]);
        let outcome = bind(&request).expect("happy path must bind");
        assert!(outcome.durable);
        assert!(outcome.warnings.is_empty());
        assert_eq!(outcome.record_count, 1);
        assert_eq!(outcome.source_commit, commit);
        assert!(request.out.is_file());

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.out).unwrap()).unwrap();
        assert_eq!(written["schema"], "rutile.evidence-index.v1");
        assert_eq!(written["version"], 1);
        assert_eq!(written["source_commit"], commit);
        let prov_sha = sha256_hex(&fs::read(&request.provenance).unwrap());
        assert_eq!(written["production_provenance_sha256"], prov_sha);
        let rec = &written["records"][0];
        assert_eq!(rec["job"], "portable");
        assert_eq!(rec["gate_result_sha256"], gate_sha);
        assert_eq!(rec["retained_log_sha256"], sha256_hex(b"hello"));
        assert_eq!(
            rec["retained_log_path"],
            format!("{commit}/portable/run-1000-1-0/run-0001.stdout.log")
        );
        assert_eq!(rec["artifact_sha256"][0], format!("{:064x}", 1_u128));
    }

    #[test]
    fn output_validates_against_evidence_index_schema() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        bind(&request).unwrap();
        crate::evidence::validate_kind(&request.out, "evidence-index")
            .expect("output must validate against the checked-in schema");
    }

    #[test]
    fn canonical_json_has_sorted_keys() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let index = load_and_bind(&request).unwrap();
        let json = index.canonical_json().unwrap();
        let schema_pos = json.find("\"schema\"").unwrap();
        let source_pos = json.find("\"source_commit\"").unwrap();
        assert!(schema_pos < source_pos);
    }

    #[test]
    fn production_provenance_sha_is_raw_file_bytes() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let outcome = bind(&request).unwrap();
        let expected = sha256_hex(&fs::read(&request.provenance).unwrap());
        assert_eq!(outcome.production_provenance_sha256, expected);
    }

    #[test]
    fn retained_log_sha_is_remeasured_from_disk() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let index = load_and_bind(&request).unwrap();
        // The recorded sha in the gate-result fixture was sha256("hello").
        assert_eq!(index.records[0].retained_log_sha256, sha256_hex(b"hello"));
    }

    // -----------------------------------------------------------------
    // Plain-index coherence failures
    // -----------------------------------------------------------------

    #[test]
    fn empty_gates_fail_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": commit, "gate_count": 0, "passed": 0, "failed": 0,
            "required_jobs": [], "gates": []
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::PlainIndexEmpty));
    }

    #[test]
    fn gate_count_mismatch_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": commit, "gate_count": 2, "passed": 1, "failed": 0,
            "required_jobs": [], "gates": [{
                "job": "portable", "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                "sha256": format!("{:064x}", 1_u128), "status": "passed", "error": ""
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::PlainIndexGateCountMismatch {
                declared: 2,
                actual: 1
            }
        ));
    }

    #[test]
    fn passed_count_mismatch_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": commit, "gate_count": 1, "passed": 2, "failed": 0,
            "required_jobs": [], "gates": [{
                "job": "portable", "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                "sha256": format!("{:064x}", 1_u128), "status": "passed", "error": ""
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::PlainIndexPassedMismatch {
                declared: 2,
                actual: 1
            }
        ));
    }

    #[test]
    fn malformed_commit_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": "not-a-sha", "gate_count": 1, "passed": 1, "failed": 0,
            "required_jobs": [], "gates": [{
                "job": "portable", "path": "x/portable/run-1000-1-0/gate-result.json",
                "sha256": format!("{:064x}", 1_u128), "status": "passed", "error": ""
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::PlainIndexCommitMalformed(_)
        ));
    }

    #[test]
    fn unknown_field_in_plain_index_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let raw = fs::read_to_string(&request.plain_index).unwrap();
        let mut with_extra = raw.clone();
        with_extra.insert_str(2, "\"surplus\": true, ");
        fs::write(&request.plain_index, with_extra).unwrap();
        let err = load_and_bind(&request).unwrap_err();
        // deny_unknown_fields => parse error surfaced as PlainIndexParse
        assert!(matches!(err, EvidenceBindError::PlainIndexParse { .. }));
    }

    #[test]
    fn bad_status_value_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": commit, "gate_count": 1, "passed": 1, "failed": 0,
            "required_jobs": [], "gates": [{
                "job": "portable", "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                "sha256": format!("{:064x}", 1_u128), "status": "skipped", "error": ""
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::PlainIndexBadStatus { ref job, ref status } if job == "portable" && status == "skipped"
        ));
    }

    #[test]
    fn passed_gate_with_nonempty_error_fails_closed() {
        // A gate marked "passed" but carrying a non-empty error string is
        // incoherent — evidence-finalize only records an error on failed
        // gates. Reject it closed with PlainIndexPassedHasError before any
        // downstream binding trusts the gate.
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": commit, "gate_count": 1, "passed": 1, "failed": 0,
            "required_jobs": [], "gates": [{
                "job": "portable",
                "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                "sha256": format!("{:064x}", 1_u128),
                "status": "passed",
                "error": "spurious-error-on-passed-gate"
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::PlainIndexPassedHasError(ref job) if job == "portable"
        ));
    }

    // -----------------------------------------------------------------
    // Source binding failures
    // -----------------------------------------------------------------

    #[test]
    fn plain_index_commit_not_head_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let wrong = "0123456789abcdef0123456789abcdef01234567";
        let orig = fs::read_to_string(&request.plain_index).unwrap();
        let mut plain: serde_json::Value = serde_json::from_str(&orig).unwrap();
        plain["commit"] = json!(wrong);
        plain["gates"][0]["path"] =
            json!(format!("{wrong}/portable/run-1000-1-0/gate-result.json"));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::SourceCommitMismatch { .. }
        ));
    }

    #[test]
    fn gate_result_source_not_head_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["source"]["commit"] = json!("0123456789abcdef0123456789abcdef01234567");
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::GateResultSourceMismatch { .. }
        ));
    }

    #[test]
    fn provenance_source_commit_not_head_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut prov: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.provenance).unwrap()).unwrap();
        prov["source_commit"] = json!("0123456789abcdef0123456789abcdef01234567");
        fs::write(
            &request.provenance,
            serde_json::to_vec_pretty(&prov).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::ProvenanceSourceMismatch { .. }
        ));
    }

    // -----------------------------------------------------------------
    // Provenance schema failures
    // -----------------------------------------------------------------

    #[test]
    fn provenance_wrong_schema_id_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut prov: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.provenance).unwrap()).unwrap();
        prov["schema"] = json!("rutile.something-else.v1");
        fs::write(
            &request.provenance,
            serde_json::to_vec_pretty(&prov).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::ProvenanceSchema));
    }

    #[test]
    fn provenance_test_control_feature_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut prov: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.provenance).unwrap()).unwrap();
        prov["features"] = json!(["test-control"]);
        fs::write(
            &request.provenance,
            serde_json::to_vec_pretty(&prov).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::ProvenanceSchemaInvalid(_)));
    }

    // -----------------------------------------------------------------
    // Gate status / binding failures
    // -----------------------------------------------------------------

    #[test]
    fn failed_gate_status_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let plain = json!({
            "commit": current_head().0, "gate_count": 1, "passed": 0, "failed": 1,
            "required_jobs": [], "gates": [{
                "job": "portable", "path": null, "sha256": null,
                "status": "failed", "error": "exit=1"
            }]
        });
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::GateNotPassed { ref job, ref status } if job == "portable" && status == "failed"
        ));
    }

    #[test]
    fn gate_result_hash_tamper_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["command_id"] = json!("tampered");
        fs::write(&gate_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::GateResultHashMismatch { .. }
        ));
    }

    #[test]
    fn gate_result_schema_invalid_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["artifact_hashes"] = json!([]);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::GateResultSchemaInvalid { .. }
                | EvidenceBindError::NoArtifactHashes { .. }
        ));
    }

    #[test]
    fn gate_result_schema_id_mismatch_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["schema"] = json!("rutile.something-else.v1");
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::GateResultSchemaId { .. }));
    }

    #[test]
    fn gate_result_required_row_failed_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["required_row"]["status"] = json!("failed");
        value["exit_code"] = json!(1);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::GateNotPassed { .. }));
    }

    // -----------------------------------------------------------------
    // Retained-log failures
    // -----------------------------------------------------------------

    #[test]
    fn retained_log_hash_tamper_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let log_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("run-0001.stdout.log");
        fs::write(&log_path, b"tampered").unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::RetainedLogHashMismatch { .. }
        ));
    }

    #[test]
    fn retained_log_oversize_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let log_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("run-0001.stdout.log");
        let big = vec![b'A'; (MAX_RETAINED_LOG_BYTES + 1) as usize];
        let big_sha = sha256_hex(&big);
        fs::write(&log_path, &big).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["retained_logs"][0]["sha256"] = json!(big_sha);
        value["retained_logs"][0]["bytes"] = json!(big.len() as i64);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(
            matches!(
                err,
                EvidenceBindError::GateResultSchemaInvalid { .. }
                    | EvidenceBindError::RetainedLogOversize { .. }
                    | EvidenceBindError::RetainedLogRead { .. }
            ),
            "oversize retained log must fail closed: {err}"
        );
    }

    #[test]
    fn no_retained_logs_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["retained_logs"] = json!([]);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::NoRetainedLog { .. }));
    }

    // -----------------------------------------------------------------
    // Duplicate / required jobs
    // -----------------------------------------------------------------

    #[test]
    fn duplicate_jobs_fails_closed() {
        let tmp = temp_root();
        let (commit, tree) = current_head();
        let evidence_root = tmp.path().join("evidence");
        let gate_dir = evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0");
        fs::create_dir_all(&gate_dir).unwrap();
        let stdout = b"hi";
        let stdout_sha = sha256_hex(stdout);
        fs::write(gate_dir.join("run-0001.stdout.log"), stdout).unwrap();
        let gv = gate_result_value("portable", &commit, &tree, &stdout_sha);
        let gb = serde_json::to_vec_pretty(&gv).unwrap();
        fs::write(gate_dir.join("gate-result.json"), &gb).unwrap();
        let gs = sha256_hex(&gb);

        let plain = json!({
            "commit": commit, "gate_count": 2, "passed": 2, "failed": 0,
            "required_jobs": [],
            "gates": [
                {"job": "portable",
                 "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                 "sha256": gs, "status": "passed", "error": ""},
                {"job": "portable",
                 "path": format!("{commit}/portable/run-1000-1-0/gate-result.json"),
                 "sha256": gs, "status": "passed", "error": ""}
            ]
        });
        let plain_path = tmp.path().join("plain.json");
        fs::write(&plain_path, serde_json::to_vec_pretty(&plain).unwrap()).unwrap();
        let provenance_path = tmp.path().join("prov.json");
        write_provenance(&provenance_path, &commit);
        let request = EvidenceBindRequest {
            plain_index: plain_path,
            provenance: provenance_path,
            evidence_root,
            out: tmp.path().join("out.json"),
        };
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::DuplicateJob(_)));
    }

    #[test]
    fn missing_required_job_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) =
            build_single_gate_tree(&tmp, "portable", &["portable", "missing-job"]);
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::RequiredJobMissing(ref job) if job == "missing-job"
        ));
    }

    // -----------------------------------------------------------------
    // Path safety
    // -----------------------------------------------------------------

    #[test]
    fn gate_result_path_traversal_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["path"] = json!(format!(
            "{commit}/portable/../portable/run-1000-1-0/gate-result.json"
        ));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::GateResultPathUnsafe(_)));
    }

    #[test]
    fn gate_result_absolute_path_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["path"] = json!(format!(
            "/Users/leaker/{commit}/portable/run-1000-1-0/gate-result.json"
        ));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::GateResultPathUnsafe(_)));
    }

    #[test]
    fn secret_marker_in_job_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["job"] = json!("secret-token");
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(err, EvidenceBindError::GateJobUnsafe { .. }));
    }

    #[test]
    fn private_ip_in_path_rejected() {
        assert!(!is_safe_path(
            "release/127.0.0.1.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path(
            "release/10.0.0.1.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path(
            "release/192.168.0.1.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path(
            "release/172.16.0.1.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path(
            "release/169.254.0.9.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path(
            "release/172.31.255.255.json",
            MAX_RELATIVE_PATH_BYTES
        ));
    }

    #[test]
    fn safe_paths_accept_valid_shapes() {
        assert!(is_safe_path(
            "abc/portable/run-1000-1-0/gate-result.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(is_safe_path("portable", MAX_LOGICAL_ID_BYTES));
        assert!(is_safe_path("release/v1.2.json", MAX_RELATIVE_PATH_BYTES));
        assert!(is_safe_path(
            "release/8.8.8.8.json",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(is_safe_path(
            "release/172.32.0.1.json",
            MAX_RELATIVE_PATH_BYTES
        ));
    }

    #[test]
    fn unsafe_paths_reject_dangerous_shapes() {
        assert!(!is_safe_path("", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("/abs", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("../up", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("a/../b", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("a//b", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("a/", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("release/token.bin", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("release/SECRET.bin", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("release/home/x", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("release/private/x", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path(
            "release/var/folders/x",
            MAX_RELATIVE_PATH_BYTES
        ));
        assert!(!is_safe_path("::1", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("fc00:dead::1", MAX_RELATIVE_PATH_BYTES));
        assert!(!is_safe_path("fe80::1", MAX_RELATIVE_PATH_BYTES));
    }

    // -----------------------------------------------------------------
    // Create-only publication
    // -----------------------------------------------------------------

    #[test]
    fn existing_output_fails_closed() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        fs::write(&request.out, b"pre-existing").unwrap();
        let err = bind(&request).unwrap_err();
        assert!(
            err.to_string().contains("output already exists"),
            "expected create-only rejection, got: {err}"
        );
        assert_eq!(fs::read(&request.out).unwrap(), b"pre-existing");
    }

    #[test]
    fn publish_is_create_only_second_call_fails() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        bind(&request).unwrap();
        let err = bind(&request).unwrap_err();
        assert!(err.to_string().contains("output already exists"));
    }

    #[test]
    fn no_temp_files_left_after_publish() {
        let tmp = temp_root();
        let (request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        bind(&request).unwrap();
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let tmp_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp."))
            .collect();
        assert!(tmp_entries.is_empty(), "no leftover temp files expected");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_output_parent_fails_closed() {
        let tmp = temp_root();
        let (mut request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let attacker = tmp.path().join("attacker");
        let real = tmp.path().join("real");
        fs::create_dir_all(&attacker).unwrap();
        fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        request.out = link.join("out.json");
        let err = bind(&request).unwrap_err();
        assert!(
            err.to_string().contains("real directory")
                || err.to_string().contains("private directory"),
            "unexpected: {err}"
        );
        assert!(!attacker.join("out.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_output_parent_fails_closed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = temp_root();
        let (mut request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let parent = tmp.path().join("group-writable");
        fs::create_dir_all(&parent).unwrap();
        let mut perms = fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o777);
        fs::set_permissions(&parent, perms).unwrap();
        request.out = parent.join("out.json");
        let err = bind(&request).unwrap_err();
        assert!(
            err.to_string().contains("writable directory")
                || err.to_string().contains("private directory"),
            "unexpected: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_gate_result_file_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let real = tmp.path().join("real-target.json");
        fs::write(&real, b"elsewhere").unwrap();
        fs::remove_file(&gate_path).unwrap();
        std::os::unix::fs::symlink(&real, &gate_path).unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(
            matches!(
                err,
                EvidenceBindError::GateResultNotRegular { .. }
                    | EvidenceBindError::GateResultRead { .. }
                    | EvidenceBindError::FileSymlink(_)
            ),
            "symlinked gate-result must fail closed: {err}"
        );
    }

    #[test]
    fn artifact_hashes_deduped_and_sorted() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        let h = format!("{:064x}", 5_u128);
        let h2 = format!("{:064x}", 3_u128);
        value["artifact_hashes"] = json!([
            {"path": "a", "sha256": h, "identity": {"device": 0, "inode": 0, "bytes": 0}},
            {"path": "b", "sha256": h2, "identity": {"device": 0, "inode": 0, "bytes": 0}},
            {"path": "c", "sha256": h, "identity": {"device": 0, "inode": 0, "bytes": 0}}
        ]);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let index = load_and_bind(&request).unwrap();
        let hashes = &index.records[0].artifact_sha256;
        assert_eq!(hashes, &vec![h2, h]);
    }

    #[test]
    fn run_directory_mismatch_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&gate_path).unwrap()).unwrap();
        value["evidence"]["run_directory"] = json!("run-9999-9-9");
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&gate_path, &bytes).unwrap();
        let mut plain: serde_json::Value =
            serde_json::from_slice(&fs::read(&request.plain_index).unwrap()).unwrap();
        plain["gates"][0]["sha256"] = json!(sha256_hex(&bytes));
        fs::write(
            &request.plain_index,
            serde_json::to_vec_pretty(&plain).unwrap(),
        )
        .unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(matches!(
            err,
            EvidenceBindError::GateResultRunDirectoryMismatch { .. }
        ));
    }

    #[test]
    fn missing_evidence_root_fails_closed() {
        let tmp = temp_root();
        let (mut request, _, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        request.evidence_root = tmp.path().join("does-not-exist");
        let err = load_and_bind(&request).unwrap_err();
        assert!(
            matches!(
                err,
                EvidenceBindError::Io(_) | EvidenceBindError::EvidenceRootNotDir(_)
            ),
            "missing evidence root must fail closed: {err}"
        );
    }

    #[test]
    fn multiple_gates_sorted_deterministically() {
        let tmp = temp_root();
        let (commit, tree) = current_head();
        let evidence_root = tmp.path().join("evidence");
        let provenance_path = tmp.path().join("prov.json");
        write_provenance(&provenance_path, &commit);

        let jobs = ["zeta", "alpha", "mid"];
        let mut gates = Vec::new();
        for job in jobs {
            let gate_dir = evidence_root.join(&commit).join(job).join("run-1000-1-0");
            fs::create_dir_all(&gate_dir).unwrap();
            let stdout = b"x";
            let stdout_sha = sha256_hex(stdout);
            fs::write(gate_dir.join("run-0001.stdout.log"), stdout).unwrap();
            let gv = gate_result_value(job, &commit, &tree, &stdout_sha);
            let gb = serde_json::to_vec_pretty(&gv).unwrap();
            fs::write(gate_dir.join("gate-result.json"), &gb).unwrap();
            let gs = sha256_hex(&gb);
            gates.push(json!({
                "job": job,
                "path": format!("{commit}/{job}/run-1000-1-0/gate-result.json"),
                "sha256": gs, "status": "passed", "error": ""
            }));
        }
        let plain = json!({
            "commit": commit, "gate_count": gates.len() as u32,
            "passed": gates.len() as u32, "failed": 0,
            "required_jobs": jobs, "gates": gates
        });
        let plain_path = tmp.path().join("plain.json");
        fs::write(&plain_path, serde_json::to_vec_pretty(&plain).unwrap()).unwrap();

        let request = EvidenceBindRequest {
            plain_index: plain_path,
            provenance: provenance_path,
            evidence_root,
            out: tmp.path().join("out.json"),
        };
        let index = load_and_bind(&request).unwrap();
        let bound: Vec<&str> = index.records.iter().map(|r| r.job.as_str()).collect();
        assert_eq!(bound, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn missing_gate_result_file_fails_closed() {
        let tmp = temp_root();
        let (request, commit, _, _) = build_single_gate_tree(&tmp, "portable", &[]);
        let gate_path = request
            .evidence_root
            .join(&commit)
            .join("portable")
            .join("run-1000-1-0")
            .join("gate-result.json");
        fs::remove_file(&gate_path).unwrap();
        let err = load_and_bind(&request).unwrap_err();
        assert!(
            matches!(
                err,
                EvidenceBindError::GateResultRead { .. }
                    | EvidenceBindError::GateResultNotRegular { .. }
            ),
            "missing gate-result must fail closed (never synthesized): {err}"
        );
    }

    #[test]
    fn stdout_chosen_over_stderr_canonical_log() {
        // When both stdout and stderr retained logs exist at run 1, stdout is
        // the canonical choice (lower stream rank).
        let logs = vec![
            json!({"run": 1, "stream": "stderr", "path": "err.log", "bytes": 0, "sha256": format!("{:064x}",1_u128)}),
            json!({"run": 1, "stream": "stdout", "path": "out.log", "bytes": 0, "sha256": format!("{:064x}",2_u128)}),
        ];
        let idx = pick_canonical_retained_log(&logs).unwrap();
        assert_eq!(logs[idx]["stream"], "stdout");
    }
}
