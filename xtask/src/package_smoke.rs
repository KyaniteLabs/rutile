//! Bounded platform package install/open/uninstall smoke engine.
//!
//! Produces a typed `rutile.package-smoke-row.v1` receipt that proves a
//! platform package was actually installed, opened, and uninstalled on the
//! current host. The engine is fail-closed: it never fakes success, never
//! interpolates into a shell, and refuses any tool outside a fixed absolute
//! allowlist. When a required tool is unavailable, the affected stage fails
//! rather than passing silently.
//!
//! # Multi-command stages
//!
//! Some package kinds require multiple commands per stage (e.g. DMG install:
//! attach → ditto → detach). [`StagePlan`] models this as an ordered command
//! sequence plus cleanup commands that always run on failure. The executor
//! trait stays single-command; the engine orchestrates the sequence.
//!
//! # Binary hash binding
//!
//! The caller supplies `expected_executable_sha256`. After install, the engine
//! hashes the installed binary (O_NOFOLLOW) and rejects any mismatch. After
//! open, it re-hashes to detect mutation. A pre-existing binary before install
//! is rejected as stale/tampering.

#![cfg(unix)]

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SMOKE_ROW_SCHEMA: &str = "rutile.package-smoke-row.v1";
pub const SMOKE_ROW_VERSION: u64 = 1;
pub const MAX_PACKAGE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
pub const DEFAULT_STAGE_DEADLINE: Duration = Duration::from_secs(90);
pub const DEFAULT_TERM_GRACE: Duration = Duration::from_secs(5);

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const KILL_SETTLE: Duration = Duration::from_millis(50);
const READ_CHUNK_BYTES: usize = 16 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 25 * 1024 * 1024;

/// Fixed absolute tool allowlist. The production executor refuses any program
/// not in this set (or not equal to the request's hash-bound `binary_path`).
const ALLOWED_PROGRAMS: &[&str] = &[
    "/usr/bin/open",
    "/usr/bin/hdiutil",
    "/usr/bin/ditto",
    "/usr/bin/codesign",
    "/bin/rm",
    "/bin/mkdir",
    "/usr/bin/mktemp",
    "/usr/bin/apt-get",
    "/usr/bin/apt",
    "/usr/bin/dpkg",
    "/usr/bin/dnf",
    "/usr/bin/rpm",
    "/usr/bin/tar",
    "/usr/bin/zstd",
    "/usr/sbin/dpkg-deb",
];

static EVIDENCE_RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Kind and platform model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    MacosAppZip,
    MacosDmg,
    LinuxDeb,
    LinuxRpm,
    LinuxTarZst,
}

impl PackageKind {
    pub const fn platform(self) -> Platform {
        match self {
            Self::MacosAppZip | Self::MacosDmg => Platform::Macos,
            Self::LinuxDeb | Self::LinuxRpm | Self::LinuxTarZst => Platform::Linux,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MacosAppZip => "macos_app_zip",
            Self::MacosDmg => "macos_dmg",
            Self::LinuxDeb => "linux_deb",
            Self::LinuxRpm => "linux_rpm",
            Self::LinuxTarZst => "linux_tar_zst",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::MacosAppZip => ".zip",
            Self::MacosDmg => ".dmg",
            Self::LinuxDeb => ".deb",
            Self::LinuxRpm => ".rpm",
            Self::LinuxTarZst => ".tar.zst",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" | "darwin" => Self::Macos,
            _ => Self::Linux,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
}

// ---------------------------------------------------------------------------
// Stages, commands, plans
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SmokeStage {
    Install,
    Open,
    Uninstall,
}

impl SmokeStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Open => "open",
            Self::Uninstall => "uninstall",
        }
    }
}

impl std::fmt::Display for SmokeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct SmokeCommand {
    stage: SmokeStage,
    program: String,
    args: Vec<OsString>,
    deadline: Duration,
    term_grace: Duration,
}

impl SmokeCommand {
    pub fn new(stage: SmokeStage, program: impl Into<String>, args: Vec<OsString>) -> Self {
        Self {
            stage,
            program: program.into(),
            args,
            deadline: DEFAULT_STAGE_DEADLINE,
            term_grace: DEFAULT_TERM_GRACE,
        }
    }

    pub fn stage(&self) -> SmokeStage {
        self.stage
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    pub fn term_grace(&self) -> Duration {
        self.term_grace
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        if deadline >= Duration::from_millis(50) {
            self.deadline = deadline;
        }
        self
    }

    pub fn with_term_grace(mut self, grace: Duration) -> Self {
        if grace >= Duration::from_millis(10) {
            self.term_grace = grace;
        }
        self
    }
}

/// A stage's execution plan: one or more commands in order, plus optional
/// cleanup commands that run on failure regardless of which step failed.
/// Single-command stages use [`StagePlan::single`].
#[derive(Clone, Debug)]
pub struct StagePlan {
    pub stage: SmokeStage,
    pub commands: Vec<SmokeCommand>,
    pub cleanup_on_failure: Vec<SmokeCommand>,
}

impl StagePlan {
    pub fn single(cmd: SmokeCommand) -> Self {
        let stage = cmd.stage();
        Self {
            stage,
            commands: vec![cmd],
            cleanup_on_failure: vec![],
        }
    }

    pub fn new(
        stage: SmokeStage,
        commands: Vec<SmokeCommand>,
        cleanup_on_failure: Vec<SmokeCommand>,
    ) -> Self {
        Self {
            stage,
            commands,
            cleanup_on_failure,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Passed,
    Failed,
    TimedOut,
    Skipped,
}

#[derive(Clone, Debug)]
pub struct StageOutcome {
    pub status: StageStatus,
    pub exit_code: Option<i32>,
    pub killed: bool,
    pub reaped: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl StageOutcome {
    pub fn skipped() -> Self {
        Self {
            status: StageStatus::Skipped,
            exit_code: None,
            killed: false,
            reaped: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request and executor trait
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SmokeRequest {
    pub package: PathBuf,
    pub package_sha256: String,
    pub kind: PackageKind,
    pub source_commit: String,
    pub install_target: PathBuf,
    pub binary_path: PathBuf,
    /// Expected SHA-256 (64-char lowercase hex) of the installed binary at
    /// `binary_path`. The engine hashes the binary after install and after
    /// open, rejecting any mismatch.
    pub expected_executable_sha256: String,
    pub evidence_dir: PathBuf,
    pub deadline: Duration,
    pub term_grace: Duration,
}

impl SmokeRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        package: PathBuf,
        package_sha256: String,
        kind: PackageKind,
        source_commit: String,
        install_target: PathBuf,
        binary_path: PathBuf,
        expected_executable_sha256: String,
        evidence_dir: PathBuf,
    ) -> Self {
        Self {
            package,
            package_sha256,
            kind,
            source_commit,
            install_target,
            binary_path,
            expected_executable_sha256,
            evidence_dir,
            deadline: DEFAULT_STAGE_DEADLINE,
            term_grace: DEFAULT_TERM_GRACE,
        }
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        if deadline >= Duration::from_millis(50) {
            self.deadline = deadline;
        }
        self
    }

    pub fn with_term_grace(mut self, grace: Duration) -> Self {
        if grace >= Duration::from_millis(10) {
            self.term_grace = grace;
        }
        self
    }
}

pub trait SmokeExecutor {
    fn execute(&self, command: &SmokeCommand) -> Result<StageOutcome, SmokeError>;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SmokeError {
    #[error("package path must be an absolute normalized path: {path}")]
    UnsafePackagePath { path: PathBuf },
    #[error("package path contains a symlink component: {path}")]
    PackageSymlink { path: PathBuf },
    #[error("install_target must be an absolute normalized path: {path}")]
    UnsafeInstallTarget { path: PathBuf },
    #[error("binary_path must be an absolute normalized path: {path}")]
    UnsafeBinaryPath { path: PathBuf },
    #[error("evidence_dir must be an absolute normalized path: {path}")]
    UnsafeEvidenceDir { path: PathBuf },
    #[error("package must be a regular file: {path}")]
    PackageNotRegular { path: PathBuf },
    #[error("package kind {kind} expects extension {expected}; got {actual}")]
    KindExtensionMismatch {
        kind: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("package platform {platform} does not match runtime platform {runtime}")]
    PlatformMismatch {
        platform: &'static str,
        runtime: &'static str,
    },
    #[error("package sha256 must be 64 lowercase hex characters")]
    InvalidPackageHash,
    #[error("expected executable sha256 must be 64 lowercase hex characters")]
    InvalidExpectedExecutableHash,
    #[error("source commit must be 40 lowercase hex characters")]
    InvalidSourceCommit,
    #[error("package exceeds {MAX_PACKAGE_BYTES} bytes: {path} ({bytes} bytes)")]
    PackageTooLarge { path: PathBuf, bytes: u64 },
    #[error("package sha256 mismatch: expected {expected}, measured {measured}")]
    PackageHashMismatch { expected: String, measured: String },
    #[error("package identity changed since capture (race)")]
    PackageIdentityRace,
    #[error("stale pre-existing binary at {path} before install")]
    StalePreExistingBinary { path: PathBuf },
    #[error("installed executable sha256 mismatch: expected {expected}, measured {measured}")]
    InstalledExecutableHashMismatch { expected: String, measured: String },
    #[error("installed executable exceeded {MAX_EXECUTABLE_BYTES} bytes: {path} ({bytes} bytes)")]
    ExecutableTooLarge { path: PathBuf, bytes: u64 },
    #[error("receipt output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("stage {stage} rejected unsafe program: {program}")]
    UnsafeCommand {
        stage: &'static str,
        program: String,
    },
    #[error("stage {stage} tool unavailable: {program}")]
    ToolUnavailable {
        stage: &'static str,
        program: String,
    },
    #[error("stage {stage} spawn failed for {program}: {error}")]
    Spawn {
        stage: &'static str,
        program: String,
        error: String,
    },
    #[error("stage {stage} wait failed: {error}")]
    Wait { stage: &'static str, error: String },
    #[error("evidence publication failed at {path}: {error}")]
    Publish { path: PathBuf, error: String },
    #[error("could not serialize package smoke receipt: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("evidence I/O failed at {path}: {error}")]
    Io { path: PathBuf, error: String },
}

// ---------------------------------------------------------------------------
// Receipt
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct PackageRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StageReceipt {
    pub status: StageStatus,
    pub exit_code: Option<i32>,
    pub reaped: bool,
    pub observed_binary: bool,
    pub executable_sha256: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvidenceRef {
    pub stage: SmokeStage,
    pub stream: &'static str,
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PackageSmokeReceipt {
    pub schema: &'static str,
    pub version: u64,
    pub kind: PackageKind,
    pub platform: Platform,
    pub architecture: &'static str,
    pub source_commit: String,
    pub expected_executable_sha256: String,
    pub package: PackageRef,
    pub install: StageReceipt,
    pub open: StageReceipt,
    pub uninstall: StageReceipt,
    pub evidence_refs: Vec<EvidenceRef>,
    pub started_unix_ms: u128,
    pub ended_unix_ms: u128,
    pub passed: bool,
}

#[derive(Debug)]
pub struct SmokeExecution {
    pub receipt_path: PathBuf,
    pub receipt: PackageSmokeReceipt,
    pub failure: Option<String>,
}

// ---------------------------------------------------------------------------
// Package and binary capture
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CapturedPackage {
    sha256: String,
    device: u64,
    inode: u64,
    bytes: u64,
}

fn capture_package(path: &Path) -> Result<CapturedPackage, SmokeError> {
    validate_absolute_normalized(path, FieldKind::Package)?;
    let file = open_no_follow(path)?;
    let metadata = file.metadata().map_err(|e| io_error(path, e))?;
    if !metadata.is_file() {
        return Err(SmokeError::PackageNotRegular {
            path: path.to_owned(),
        });
    }
    if metadata.len() > MAX_PACKAGE_BYTES {
        return Err(SmokeError::PackageTooLarge {
            path: path.to_owned(),
            bytes: metadata.len(),
        });
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(READ_CHUNK_BYTES));
    file.take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(path, e))?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(SmokeError::PackageTooLarge {
            path: path.to_owned(),
            bytes: bytes.len() as u64,
        });
    }
    Ok(CapturedPackage {
        sha256: hex_sha256(&bytes),
        device: metadata.dev(),
        inode: metadata.ino(),
        bytes: metadata.len(),
    })
}

fn reverify_package(path: &Path, expected: &CapturedPackage) -> Result<(), SmokeError> {
    let observed = capture_package(path)?;
    if observed.sha256 != expected.sha256
        || observed.device != expected.device
        || observed.inode != expected.inode
        || observed.bytes != expected.bytes
    {
        return Err(SmokeError::PackageIdentityRace);
    }
    Ok(())
}

/// Hash the installed binary at `path` with O_NOFOLLOW. Returns the hex
/// SHA-256 of the file contents. Rejects symlinks, oversized files, and
/// non-regular files.
fn hash_installed_binary(path: &Path) -> Result<String, SmokeError> {
    let file = open_no_follow(path)?;
    let metadata = file.metadata().map_err(|e| io_error(path, e))?;
    if !metadata.is_file() {
        return Err(SmokeError::Io {
            path: path.to_owned(),
            error: "binary path is not a regular file".into(),
        });
    }
    if metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(SmokeError::ExecutableTooLarge {
            path: path.to_owned(),
            bytes: metadata.len(),
        });
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(READ_CHUNK_BYTES));
    file.take(MAX_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(path, e))?;
    Ok(hex_sha256(&bytes))
}

fn io_error(path: &Path, error: io::Error) -> SmokeError {
    SmokeError::Io {
        path: path.to_owned(),
        error: error.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub fn run_smoke(
    request: SmokeRequest,
    executor: &dyn SmokeExecutor,
) -> Result<SmokeExecution, SmokeError> {
    validate_request_shape(&request)?;
    let runtime = Platform::current();
    let expected_platform = request.kind.platform();
    if runtime != expected_platform {
        return Err(SmokeError::PlatformMismatch {
            platform: expected_platform.as_str(),
            runtime: runtime.as_str(),
        });
    }

    let captured = capture_package(&request.package)?;
    if captured.sha256 != request.package_sha256 {
        return Err(SmokeError::PackageHashMismatch {
            expected: request.package_sha256.clone(),
            measured: captured.sha256.clone(),
        });
    }
    reverify_package(&request.package, &captured)?;

    // Stale binary detection: reject if binary already exists before install.
    if request.binary_path.exists() {
        return Err(SmokeError::StalePreExistingBinary {
            path: request.binary_path.clone(),
        });
    }

    fs::create_dir_all(&request.evidence_dir).map_err(|e| io_error(&request.evidence_dir, e))?;
    let started_unix_ms = unix_time_ms();
    let mut evidence_refs = Vec::new();
    let mut failure: Option<String> = None;

    let install_plan = build_install_plan(&request)?;
    let open_plan = build_open_plan(&request)?;
    let uninstall_plan = build_uninstall_plan(&request)?;

    // --- Install ---------------------------------------------------------
    let install_outcome = run_stage_plan(executor, &install_plan, &mut failure);
    let install_receipt = record_stage(
        &request.evidence_dir,
        SmokeStage::Install,
        &install_outcome,
        &mut evidence_refs,
    )?;
    let install_passed = install_outcome.status == StageStatus::Passed;

    // Binary hash verification after install.
    let mut installed_hash: Option<String> = None;
    let mut binary_observed_after_install = false;
    if install_passed {
        match hash_installed_binary(&request.binary_path) {
            Ok(measured) => {
                if measured != request.expected_executable_sha256 {
                    let msg = format!(
                        "installed executable sha256 mismatch: expected {}, measured {}",
                        request.expected_executable_sha256, measured
                    );
                    if failure.is_none() {
                        failure = Some(msg.clone());
                    }
                    // Error surfaced via failure string + receipt error field.
                } else {
                    installed_hash = Some(measured);
                    binary_observed_after_install = true;
                }
            }
            Err(e) => {
                if failure.is_none() {
                    failure = Some(e.to_string());
                }
            }
        }
    } else if failure.is_none() {
        failure = Some(first_line(install_receipt.error.as_deref()));
    }

    // --- Open ------------------------------------------------------------
    let open_outcome = if binary_observed_after_install {
        run_stage_plan(executor, &open_plan, &mut failure)
    } else {
        StageOutcome::skipped()
    };
    let open_receipt = record_stage(
        &request.evidence_dir,
        SmokeStage::Open,
        &open_outcome,
        &mut evidence_refs,
    )?;
    let open_passed = open_outcome.status == StageStatus::Passed;
    let mut binary_observed_after_open = false;
    if open_passed {
        // Re-hash binary after open to detect mutation during the open stage.
        match hash_installed_binary(&request.binary_path) {
            Ok(measured) => {
                if let Some(ref install_hash) = installed_hash {
                    if measured != *install_hash || measured != request.expected_executable_sha256 {
                        if failure.is_none() {
                            failure = Some(format!(
                                "executable sha256 changed during open: was {install_hash}, now {measured}"
                            ));
                        }
                    } else {
                        binary_observed_after_open = true;
                    }
                } else {
                    binary_observed_after_open = true;
                }
            }
            Err(e) => {
                if failure.is_none() {
                    failure = Some(e.to_string());
                }
            }
        }
    } else if failure.is_none() {
        failure = Some(first_line(open_receipt.error.as_deref()));
    }

    // --- Uninstall (always runs for cleanup) ------------------------------
    let uninstall_outcome = run_stage_plan(executor, &uninstall_plan, &mut failure);
    let uninstall_receipt = record_stage(
        &request.evidence_dir,
        SmokeStage::Uninstall,
        &uninstall_outcome,
        &mut evidence_refs,
    )?;
    if uninstall_outcome.status != StageStatus::Passed && failure.is_none() {
        failure = Some(first_line(uninstall_receipt.error.as_deref()));
    }

    // Residue check.
    let mut residue: Vec<String> = Vec::new();
    if uninstall_outcome.status == StageStatus::Passed {
        if request.binary_path.exists() {
            residue.push(request.binary_path.display().to_string());
        }
        if is_residue_candidate(&request.install_target, &request.binary_path)
            && request.install_target.exists()
        {
            residue.push(request.install_target.display().to_string());
        }
        if !residue.is_empty() && failure.is_none() {
            failure = Some(format!("uninstall left residue: {}", residue.join(", ")));
        }
    }

    let passed = failure.is_none()
        && install_passed
        && open_passed
        && binary_observed_after_install
        && binary_observed_after_open
        && uninstall_outcome.status == StageStatus::Passed
        && residue.is_empty();

    let receipt = PackageSmokeReceipt {
        schema: SMOKE_ROW_SCHEMA,
        version: SMOKE_ROW_VERSION,
        kind: request.kind,
        platform: runtime,
        architecture: std::env::consts::ARCH,
        source_commit: request.source_commit.clone(),
        expected_executable_sha256: request.expected_executable_sha256.clone(),
        package: PackageRef {
            path: request.package.display().to_string(),
            sha256: captured.sha256,
            bytes: captured.bytes,
            device: captured.device,
            inode: captured.inode,
        },
        install: stage_receipt_row(
            &install_receipt,
            binary_observed_after_install,
            &installed_hash,
        ),
        open: stage_receipt_row(&open_receipt, binary_observed_after_open, &installed_hash),
        uninstall: stage_receipt_row(&uninstall_receipt, residue.is_empty(), &None),
        evidence_refs,
        started_unix_ms,
        ended_unix_ms: unix_time_ms(),
        passed,
    };
    let receipt_path = request.evidence_dir.join("package-smoke-row.json");
    publish_receipt(&receipt_path, &receipt)?;

    Ok(SmokeExecution {
        receipt_path,
        receipt,
        failure,
    })
}

/// Execute a multi-command stage plan. Commands run in order; if any fails,
/// cleanup commands run (best-effort) and the stage is marked failed.
fn run_stage_plan(
    executor: &dyn SmokeExecutor,
    plan: &StagePlan,
    failure: &mut Option<String>,
) -> StageOutcome {
    let mut combined_stdout = Vec::new();
    let mut combined_stderr = Vec::new();
    let mut last_exit_code: Option<i32> = None;
    let mut all_reaped = true;

    for cmd in &plan.commands {
        match executor.execute(cmd) {
            Ok(outcome) => {
                append_bounded(&mut combined_stdout, &outcome.stdout);
                append_bounded(&mut combined_stderr, &outcome.stderr);
                last_exit_code = outcome.exit_code.or(last_exit_code);
                if !outcome.reaped {
                    all_reaped = false;
                }
                if outcome.status != StageStatus::Passed {
                    for cleanup in &plan.cleanup_on_failure {
                        let _ = executor.execute(cleanup);
                    }
                    return StageOutcome {
                        status: outcome.status,
                        exit_code: last_exit_code,
                        killed: outcome.killed,
                        reaped: all_reaped,
                        stdout: combined_stdout,
                        stderr: combined_stderr,
                    };
                }
            }
            Err(error) => {
                let summary = error.to_string();
                if failure.is_none() {
                    *failure = Some(summary.clone());
                }
                combined_stderr.extend_from_slice(summary.as_bytes());
                for cleanup in &plan.cleanup_on_failure {
                    let _ = executor.execute(cleanup);
                }
                return StageOutcome {
                    status: StageStatus::Failed,
                    exit_code: None,
                    killed: false,
                    reaped: all_reaped,
                    stdout: combined_stdout,
                    stderr: combined_stderr,
                };
            }
        }
    }

    StageOutcome {
        status: StageStatus::Passed,
        exit_code: last_exit_code,
        killed: false,
        reaped: all_reaped,
        stdout: combined_stdout,
        stderr: combined_stderr,
    }
}

fn append_bounded(destination: &mut Vec<u8>, source: &[u8]) {
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(destination.len());
    if remaining > 0 {
        let take = source.len().min(remaining);
        destination.extend_from_slice(&source[..take]);
    }
}

#[derive(Clone, Debug)]
struct StageReceiptInternal {
    status: StageStatus,
    exit_code: Option<i32>,
    reaped: bool,
    error: Option<String>,
}

fn stage_receipt_row(
    internal: &StageReceiptInternal,
    observed_binary: bool,
    hash: &Option<String>,
) -> StageReceipt {
    StageReceipt {
        status: internal.status,
        exit_code: internal.exit_code,
        reaped: internal.reaped,
        observed_binary,
        executable_sha256: hash.clone(),
        error: internal.error.clone(),
    }
}

fn record_stage(
    evidence_dir: &Path,
    stage: SmokeStage,
    outcome: &StageOutcome,
    evidence_refs: &mut Vec<EvidenceRef>,
) -> Result<StageReceiptInternal, SmokeError> {
    let stage_label = stage.as_str();
    let mut retained_stdout = outcome.stdout.clone();
    let mut retained_stderr = outcome.stderr.clone();
    truncate_bounded(&mut retained_stdout, MAX_OUTPUT_BYTES);
    truncate_bounded(&mut retained_stderr, MAX_OUTPUT_BYTES);

    let stdout_name = format!("{stage_label}.stdout.log");
    let stderr_name = format!("{stage_label}.stderr.log");
    write_create_only(&evidence_dir.join(&stdout_name), &retained_stdout)?;
    write_create_only(&evidence_dir.join(&stderr_name), &retained_stderr)?;

    evidence_refs.push(EvidenceRef {
        stage,
        stream: "stdout",
        path: stdout_name,
        bytes: retained_stdout.len(),
        sha256: hex_sha256(&retained_stdout),
    });
    evidence_refs.push(EvidenceRef {
        stage,
        stream: "stderr",
        path: stderr_name,
        bytes: retained_stderr.len(),
        sha256: hex_sha256(&retained_stderr),
    });

    let error = if outcome.status == StageStatus::Passed {
        None
    } else {
        Some(stage_error_summary(stage, outcome, &retained_stderr))
    };

    Ok(StageReceiptInternal {
        status: outcome.status,
        exit_code: outcome.exit_code,
        reaped: outcome.reaped,
        error,
    })
}

fn stage_error_summary(stage: SmokeStage, outcome: &StageOutcome, stderr: &[u8]) -> String {
    let suffix = match outcome.status {
        StageStatus::Passed => return String::new(),
        StageStatus::Skipped => return format!("{stage} skipped (prior stage failed)"),
        StageStatus::TimedOut => {
            if outcome.killed {
                "timed out (SIGKILL reaped)".to_string()
            } else {
                "timed out".to_string()
            }
        }
        StageStatus::Failed => match outcome.exit_code {
            Some(code) => format!("exited {code}"),
            None => "exited without status".to_string(),
        },
    };
    let stderr_first_line = std::str::from_utf8(stderr)
        .ok()
        .and_then(|s| s.lines().next())
        .filter(|line| !line.is_empty())
        .map(|line| format!(": {line}"))
        .unwrap_or_default();
    format!("{stage} {suffix}{stderr_first_line}")
}

fn first_line(value: Option<&str>) -> String {
    value
        .map(|v| v.lines().next().unwrap_or(v).to_owned())
        .unwrap_or_else(|| "stage failed".to_string())
}

fn truncate_bounded(buffer: &mut Vec<u8>, limit: usize) {
    if buffer.len() > limit {
        buffer.truncate(limit);
    }
}

// ---------------------------------------------------------------------------
// Plan builders
// ---------------------------------------------------------------------------

fn build_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    match request.kind {
        PackageKind::MacosAppZip => macos_app_zip_install_plan(request),
        PackageKind::MacosDmg => macos_dmg_install_plan(request),
        PackageKind::LinuxDeb => linux_deb_install_plan(request),
        PackageKind::LinuxRpm => linux_rpm_install_plan(request),
        PackageKind::LinuxTarZst => linux_tar_zst_install_plan(request),
    }
}

fn build_open_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    match request.kind {
        PackageKind::MacosAppZip | PackageKind::MacosDmg => macos_open_plan(request),
        PackageKind::LinuxDeb | PackageKind::LinuxRpm | PackageKind::LinuxTarZst => {
            linux_open_plan(request)
        }
    }
}

fn build_uninstall_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    match request.kind {
        PackageKind::MacosAppZip | PackageKind::MacosDmg | PackageKind::LinuxTarZst => {
            rm_uninstall_plan(request)
        }
        PackageKind::LinuxDeb => linux_deb_uninstall_plan(request),
        PackageKind::LinuxRpm => linux_rpm_uninstall_plan(request),
    }
}

fn timed(cmd: SmokeCommand, request: &SmokeRequest) -> SmokeCommand {
    cmd.with_deadline(request.deadline)
        .with_term_grace(request.term_grace)
}

fn macos_app_zip_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    expect_extension(&request.package, request.kind)?;
    // ditto -x -k <zip> <install_target's parent>: extracts the zip preserving
    // resource forks. The archive root IS Rutile.app, so extraction into the
    // parent produces <parent>/Rutile.app = install_target.
    let parent =
        request
            .install_target
            .parent()
            .ok_or_else(|| SmokeError::UnsafeInstallTarget {
                path: request.install_target.clone(),
            })?;
    let args = vec![
        "-x".into(),
        "-k".into(),
        request.package.clone().into_os_string(),
        parent.as_os_str().to_owned(),
    ];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Install, "/usr/bin/ditto", args),
        request,
    )))
}

fn macos_dmg_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    expect_extension(&request.package, request.kind)?;
    // Multi-command DMG lifecycle:
    // 1. hdiutil attach at a controlled mountpoint
    // 2. ditto Rutile.app from the mounted volume into install_target
    // 3. hdiutil detach (always, even on failure)
    let mountpoint = request.evidence_dir.join(".smoke-mount");
    let mountpoint_arg = mountpoint.as_os_str().to_owned();
    let attach = timed(
        SmokeCommand::new(
            SmokeStage::Install,
            "/usr/bin/hdiutil",
            vec![
                "attach".into(),
                "-mountpoint".into(),
                mountpoint_arg.clone(),
                "-nobrowse".into(),
                "-readonly".into(),
                "-noverify".into(),
                request.package.clone().into_os_string(),
            ],
        ),
        request,
    );
    let ditto = timed(
        SmokeCommand::new(
            SmokeStage::Install,
            "/usr/bin/ditto",
            vec![
                mountpoint.join("Rutile.app").into_os_string(),
                request.install_target.clone().into_os_string(),
            ],
        ),
        request,
    );
    let detach = timed(
        SmokeCommand::new(
            SmokeStage::Install,
            "/usr/bin/hdiutil",
            vec!["detach".into(), mountpoint_arg, "-force".into()],
        ),
        request,
    );
    Ok(StagePlan::new(
        SmokeStage::Install,
        vec![attach, ditto, detach.clone()],
        vec![detach],
    ))
}

fn macos_open_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    // `open -W -n` waits for the app to exit and forces a new instance.
    // `--args --native-smoke` passes the native-smoke flag so the app
    // self-closes after its smoke routine.
    let args = vec![
        "-W".into(),
        "-n".into(),
        request.install_target.clone().into_os_string(),
        "--args".into(),
        "--native-smoke".into(),
    ];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Open, "/usr/bin/open", args),
        request,
    )))
}

fn rm_uninstall_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    let args = vec![
        "-rf".into(),
        request.install_target.clone().into_os_string(),
    ];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Uninstall, "/bin/rm", args),
        request,
    )))
}

fn linux_deb_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    expect_extension(&request.package, request.kind)?;
    // Root-runner required: apt-get needs elevated privileges to install.
    // The production executor inherits the caller's uid; CI must run as root
    // or via sudo (documented in the plan).
    let args = vec![
        "install".into(),
        "-y".into(),
        "--no-install-recommends".into(),
        request.package.clone().into_os_string(),
    ];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Install, "/usr/bin/apt-get", args),
        request,
    )))
}

fn linux_rpm_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    expect_extension(&request.package, request.kind)?;
    // Root-runner required: dnf needs elevated privileges to install.
    let args = vec![
        "install".into(),
        "-y".into(),
        request.package.clone().into_os_string(),
    ];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Install, "/usr/bin/dnf", args),
        request,
    )))
}

fn linux_tar_zst_install_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    expect_extension(&request.package, request.kind)?;
    // Archive layout: Rutile-linux-x86_64/bin/rutile (+ manifest/sbom).
    // mkdir -p <install_target> then tar --strip-components=1 extracts
    // bin/rutile (and siblings) directly into <install_target>.
    // Binary path: <install_target>/bin/rutile.
    let mkdir = timed(
        SmokeCommand::new(
            SmokeStage::Install,
            "/bin/mkdir",
            vec!["-p".into(), request.install_target.clone().into_os_string()],
        ),
        request,
    );
    let tar = timed(
        SmokeCommand::new(
            SmokeStage::Install,
            "/usr/bin/tar",
            vec![
                "--extract".into(),
                "--zstd".into(),
                "--file".into(),
                request.package.clone().into_os_string(),
                "--directory".into(),
                request.install_target.clone().into_os_string(),
                "--strip-components=1".into(),
                "--no-same-owner".into(),
            ],
        ),
        request,
    );
    Ok(StagePlan::new(
        SmokeStage::Install,
        vec![mkdir, tar],
        vec![],
    ))
}

fn linux_open_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    // Launch the installed binary directly. Inherit the runner's display/
    // backend environment — do NOT force GDK_BACKEND so both X11 and Wayland
    // sessions work. The binary_path was hash-bound through the package and
    // is accepted by the production executor's binary-path allowlist.
    let args = vec!["--native-smoke".into(), "--single-shot".into()];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(
            SmokeStage::Open,
            request.binary_path.to_string_lossy().into_owned(),
            args,
        ),
        request,
    )))
}

fn linux_deb_uninstall_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    // Root-runner required. apt-get purge removes the package and its config.
    // Post-uninstall verification (dpkg -s returning nonzero) is not included
    // as a stage command because the executor treats nonzero exit as failure;
    // the engine's residue check (binary_path absent) provides the guarantee.
    let args = vec!["purge".into(), "-y".into(), "rutile".into()];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Uninstall, "/usr/bin/apt-get", args),
        request,
    )))
}

fn linux_rpm_uninstall_plan(request: &SmokeRequest) -> Result<StagePlan, SmokeError> {
    // Root-runner required. dnf remove uninstalls the package.
    // Post-uninstall verification (rpm -q returning nonzero) is documented
    // above; residue check provides the guarantee.
    let args = vec!["remove".into(), "-y".into(), "rutile".into()];
    Ok(StagePlan::single(timed(
        SmokeCommand::new(SmokeStage::Uninstall, "/usr/bin/dnf", args),
        request,
    )))
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum FieldKind {
    Package,
    InstallTarget,
    BinaryPath,
    EvidenceDir,
}

impl FieldKind {
    fn unsafe_error(self, path: &Path) -> SmokeError {
        match self {
            Self::Package => SmokeError::UnsafePackagePath {
                path: path.to_owned(),
            },
            Self::InstallTarget => SmokeError::UnsafeInstallTarget {
                path: path.to_owned(),
            },
            Self::BinaryPath => SmokeError::UnsafeBinaryPath {
                path: path.to_owned(),
            },
            Self::EvidenceDir => SmokeError::UnsafeEvidenceDir {
                path: path.to_owned(),
            },
        }
    }

    fn symlink_error(self, path: &Path) -> SmokeError {
        match self {
            Self::Package => SmokeError::PackageSymlink {
                path: path.to_owned(),
            },
            other => other.unsafe_error(path),
        }
    }
}

fn validate_request_shape(request: &SmokeRequest) -> Result<(), SmokeError> {
    expect_extension(&request.package, request.kind)?;
    validate_absolute_normalized(&request.package, FieldKind::Package)?;
    validate_absolute_normalized(&request.install_target, FieldKind::InstallTarget)?;
    validate_absolute_normalized(&request.binary_path, FieldKind::BinaryPath)?;
    validate_absolute_normalized(&request.evidence_dir, FieldKind::EvidenceDir)?;
    if !is_lower_hex_n(&request.package_sha256, 64) {
        return Err(SmokeError::InvalidPackageHash);
    }
    if !is_lower_hex_n(&request.expected_executable_sha256, 64) {
        return Err(SmokeError::InvalidExpectedExecutableHash);
    }
    if !is_lower_hex_n(&request.source_commit, 40) {
        return Err(SmokeError::InvalidSourceCommit);
    }
    Ok(())
}

fn is_lower_hex_n(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn expect_extension(path: &Path, kind: PackageKind) -> Result<(), SmokeError> {
    let actual = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_owned())
        .unwrap_or_default();
    let expected = kind.extension();
    if !actual.ends_with(expected) {
        return Err(SmokeError::KindExtensionMismatch {
            kind: kind.as_str(),
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_absolute_normalized(path: &Path, field: FieldKind) -> Result<(), SmokeError> {
    let ok = path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        });
    if !ok {
        return Err(field.unsafe_error(path));
    }
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            if metadata.file_type().is_symlink() {
                return Err(field.symlink_error(ancestor));
            }
        }
    }
    Ok(())
}

fn open_no_follow(path: &Path) -> Result<File, SmokeError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| io_error(path, e))
}

fn is_residue_candidate(install_target: &Path, binary_path: &Path) -> bool {
    if install_target == Path::new("/") {
        return false;
    }
    install_target != binary_path
}

// ---------------------------------------------------------------------------
// Production executor
// ---------------------------------------------------------------------------

pub struct ProductionSmokeExecutor {
    binary_path: PathBuf,
}

impl ProductionSmokeExecutor {
    pub fn new(binary_path: PathBuf) -> Self {
        Self { binary_path }
    }

    fn is_program_allowed(&self, program: &str) -> bool {
        ALLOWED_PROGRAMS.contains(&program) || Path::new(program) == self.binary_path
    }
}

impl SmokeExecutor for ProductionSmokeExecutor {
    fn execute(&self, command: &SmokeCommand) -> Result<StageOutcome, SmokeError> {
        let stage_label = command.stage().as_str();
        let program_path = Path::new(&command.program);

        if !self.is_program_allowed(&command.program) {
            return Err(SmokeError::UnsafeCommand {
                stage: stage_label,
                program: command.program.clone(),
            });
        }
        if !program_path.exists() {
            return Err(SmokeError::ToolUnavailable {
                stage: stage_label,
                program: command.program.clone(),
            });
        }

        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        unsafe {
            cmd.pre_exec(|| {
                let _ = libc::setpgid(0, 0);
                Ok(())
            });
        }

        // SAFETY: This module is the audited fixed-tool process owner for
        // platform package smoke. The program is validated against
        // ALLOWED_PROGRAMS (or the hash-bound binary_path) above, paths are
        // absolute and symlink-free, and the child runs in its own process
        // group with bounded output capture and deadline enforcement. No
        // shell interpolation occurs: arguments are passed as an explicit
        // argument vector.
        #[allow(clippy::disallowed_methods)]
        let mut child = cmd.spawn().map_err(|e| SmokeError::Spawn {
            stage: stage_label,
            program: command.program.clone(),
            error: e.to_string(),
        })?;
        let pid = child.id() as i32;

        let stdout_capture = Arc::new(OutputCapture::default());
        let stderr_capture = Arc::new(OutputCapture::default());
        let stdout_thread = child
            .stdout
            .take()
            .map(|reader| spawn_reader(reader, stdout_capture.clone()));
        let stderr_thread = child
            .stderr
            .take()
            .map(|reader| spawn_reader(reader, stderr_capture.clone()));

        let supervise_result = supervise_child(
            &mut child,
            pid,
            command.deadline,
            command.term_grace,
            stage_label,
        );

        if let Some(handle) = stdout_thread {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_thread {
            let _ = handle.join();
        }

        let stdout = stdout_capture.finalize();
        let stderr = stderr_capture.finalize();

        match supervise_result {
            Ok(()) => {
                let exit_status = child.wait().map_err(|e| SmokeError::Wait {
                    stage: stage_label,
                    error: e.to_string(),
                })?;
                Ok(build_outcome(exit_status, stdout, stderr))
            }
            Err(error) => Ok(StageOutcome {
                status: StageStatus::TimedOut,
                exit_code: None,
                killed: true,
                reaped: true,
                stdout,
                stderr: {
                    let mut s = stderr;
                    s.extend_from_slice(error.to_string().as_bytes());
                    s
                },
            }),
        }
    }
}

fn build_outcome(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> StageOutcome {
    let code = status.code();
    let mut stdout = stdout;
    let mut stderr = stderr;
    truncate_bounded(&mut stdout, MAX_OUTPUT_BYTES);
    truncate_bounded(&mut stderr, MAX_OUTPUT_BYTES);
    StageOutcome {
        status: if code == Some(0) {
            StageStatus::Passed
        } else {
            StageStatus::Failed
        },
        exit_code: code,
        killed: false,
        reaped: true,
        stdout,
        stderr,
    }
}

fn supervise_child(
    child: &mut Child,
    pid: i32,
    deadline: Duration,
    term_grace: Duration,
    stage: &'static str,
) -> Result<(), SmokeError> {
    let started = Instant::now();
    let mut signaled_term = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {
                let elapsed = started.elapsed();
                if !signaled_term && elapsed >= deadline {
                    let _ = signal_group(pid, libc::SIGTERM);
                    signaled_term = true;
                }
                if signaled_term && elapsed >= deadline + term_grace {
                    let _ = signal_group(pid, libc::SIGKILL);
                    thread::sleep(KILL_SETTLE);
                    let _ = child.wait();
                    return Err(SmokeError::Wait {
                        stage,
                        error: format!(
                            "stage exceeded deadline of {deadline:?} (term_grace {term_grace:?})"
                        ),
                    });
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                return Err(SmokeError::Wait {
                    stage,
                    error: error.to_string(),
                });
            }
        }
    }
}

fn signal_group(pid: i32, signum: i32) -> io::Result<()> {
    let rc = unsafe { libc::kill(-pid, signum) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[derive(Default)]
struct OutputCapture {
    buffer: Mutex<Vec<u8>>,
    exceeded: AtomicBool,
}

impl OutputCapture {
    fn append(&self, bytes: &[u8]) {
        let mut guard = self.buffer.lock().expect("output capture mutex poisoned");
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(guard.len());
        if remaining == 0 {
            self.exceeded.store(true, Ordering::Relaxed);
            return;
        }
        if bytes.len() <= remaining {
            guard.extend_from_slice(bytes);
        } else {
            guard.extend_from_slice(&bytes[..remaining]);
            self.exceeded.store(true, Ordering::Relaxed);
        }
    }

    fn finalize(&self) -> Vec<u8> {
        let mut guard = self.buffer.lock().expect("output capture mutex poisoned");
        std::mem::take(&mut *guard)
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    capture: Arc<OutputCapture>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => capture.append(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Publication
// ---------------------------------------------------------------------------

fn publish_receipt(path: &Path, receipt: &PackageSmokeReceipt) -> Result<(), SmokeError> {
    if path.exists() {
        return Err(SmokeError::OutputExists(path.to_owned()));
    }
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    write_create_only(path, &bytes)
}

fn write_create_only(path: &Path, contents: &[u8]) -> Result<(), SmokeError> {
    let parent = path.parent().ok_or_else(|| SmokeError::Publish {
        path: path.to_owned(),
        error: "path has no parent".into(),
    })?;
    let filename = path
        .file_name()
        .ok_or_else(|| SmokeError::Publish {
            path: path.to_owned(),
            error: "path has no file name".into(),
        })?
        .to_string_lossy()
        .into_owned();
    let counter = EVIDENCE_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{filename}.tmp-{}-{counter}", std::process::id()));

    let outcome: Result<(), (PathBuf, String)> = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|e| (temporary.clone(), e.to_string()))?;
        file.write_all(contents)
            .map_err(|e| (temporary.clone(), e.to_string()))?;
        file.sync_all()
            .map_err(|e| (temporary.clone(), e.to_string()))?;
        fs::hard_link(&temporary, path).map_err(|e| (temporary.clone(), e.to_string()))?;
        fs::remove_file(&temporary).map_err(|e| (temporary.clone(), e.to_string()))?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| (temporary.clone(), e.to_string()))?;
        Ok(())
    })();

    if let Err((temp_path, error)) = outcome {
        let _ = fs::remove_file(&temp_path);
        return Err(SmokeError::Publish {
            path: path.to_owned(),
            error,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn valid_commit() -> String {
        "a".repeat(40)
    }

    fn valid_hash() -> String {
        "b".repeat(64)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn smoke_tempdir() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate has a parent workspace dir")
            .join("target/tests/package-smoke");
        fs::create_dir_all(&root).expect("create smoke test root");
        tempfile::Builder::new()
            .prefix("smoke-")
            .tempdir_in(&root)
            .expect("create smoke tempdir")
    }

    fn write_package(temp: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = temp.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// Build a macOS zip request with the expected executable hash set to the
    /// hash of `binary_contents`.
    fn macos_zip_request(
        temp: &Path,
        package_bytes: &[u8],
        binary_path: PathBuf,
        binary_contents: &[u8],
        evidence_dir: PathBuf,
    ) -> SmokeRequest {
        let package = write_package(temp, "Rutile.app.zip", package_bytes);
        SmokeRequest::new(
            package,
            sha256_hex(package_bytes),
            PackageKind::MacosAppZip,
            valid_commit(),
            temp.join("Rutile.app"),
            binary_path,
            sha256_hex(binary_contents),
            evidence_dir,
        )
    }

    /// Test executor that receives (stage, program) so multi-command stages
    /// (e.g. DMG attach+ditto+detach) can be distinguished.
    type StageOutcomeFn =
        Box<dyn Fn(SmokeStage, &str) -> Result<StageOutcome, SmokeError> + Send + Sync>;

    struct FnExecutor {
        func: StageOutcomeFn,
    }

    impl FnExecutor {
        fn new<F>(func: F) -> Self
        where
            F: Fn(SmokeStage, &str) -> Result<StageOutcome, SmokeError> + Send + Sync + 'static,
        {
            Self {
                func: Box::new(func),
            }
        }
    }

    impl SmokeExecutor for FnExecutor {
        fn execute(&self, command: &SmokeCommand) -> Result<StageOutcome, SmokeError> {
            (self.func)(command.stage(), command.program())
        }
    }

    fn passed() -> StageOutcome {
        StageOutcome {
            status: StageStatus::Passed,
            exit_code: Some(0),
            killed: false,
            reaped: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn failed(code: i32) -> StageOutcome {
        StageOutcome {
            status: StageStatus::Failed,
            exit_code: Some(code),
            killed: false,
            reaped: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    // --- Preflight validation tests ---

    #[test]
    fn rejects_relative_package_path() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let mut request = macos_zip_request(
            temp.path(),
            b"pkg",
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            b"bin",
            evidence,
        );
        request.package = PathBuf::from("relative/Rutile.app.zip");
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(
            matches!(err, SmokeError::UnsafePackagePath { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_package_path_with_symlink_component() {
        let temp = smoke_tempdir();
        let real = temp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let package_bytes = b"bytes";
        let real_package = real.join("Rutile.app.zip");
        fs::write(&real_package, package_bytes).unwrap();
        let link = temp.path().join("linked Rutile.app.zip");
        symlink(&real_package, &link).unwrap();
        let evidence = temp.path().join("evidence");
        fs::create_dir_all(&evidence).unwrap();
        let request = SmokeRequest::new(
            link,
            sha256_hex(package_bytes),
            PackageKind::MacosAppZip,
            valid_commit(),
            temp.path().join("Rutile.app"),
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            valid_hash(),
            evidence,
        );
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(matches!(err, SmokeError::PackageSymlink { .. }), "{err:?}");
    }

    #[test]
    fn rejects_oversized_package() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let bytes = vec![0u8; (MAX_PACKAGE_BYTES as usize) + 1];
        let request = macos_zip_request(
            temp.path(),
            &bytes,
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            b"bin",
            evidence,
        );
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(matches!(err, SmokeError::PackageTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn rejects_hash_mismatch() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let mut request = macos_zip_request(
            temp.path(),
            b"bytes",
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            b"bin",
            evidence,
        );
        let mut bad = request.package_sha256.clone();
        bad.replace_range(0..1, if bad.starts_with('a') { "b" } else { "a" });
        request.package_sha256 = bad;
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(
            matches!(err, SmokeError::PackageHashMismatch { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_kind_extension_mismatch() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let package = write_package(temp.path(), "Rutile.app.tar", b"bytes");
        let request = SmokeRequest::new(
            package,
            sha256_hex(b"bytes"),
            PackageKind::MacosAppZip,
            valid_commit(),
            temp.path().join("Rutile.app"),
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            valid_hash(),
            evidence,
        );
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(
            matches!(err, SmokeError::KindExtensionMismatch { kind, expected: ".zip", .. } if kind == "macos_app_zip"),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_stale_pre_existing_binary() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary_dir = temp.path().join("Rutile.app/Contents/MacOS");
        fs::create_dir_all(&binary_dir).unwrap();
        let binary = binary_dir.join("Rutile");
        fs::write(&binary, b"stale").unwrap();
        let request = macos_zip_request(temp.path(), b"bytes", binary.clone(), b"bin", evidence);
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(
            matches!(err, SmokeError::StalePreExistingBinary { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_invalid_expected_executable_hash() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let mut request = macos_zip_request(
            temp.path(),
            b"bytes",
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            b"bin",
            evidence,
        );
        request.expected_executable_sha256 = "XYZ".to_string();
        let err = run_smoke(request, &FnExecutor::new(|_, _| Ok(passed()))).unwrap_err();
        assert!(
            matches!(err, SmokeError::InvalidExpectedExecutableHash),
            "{err:?}"
        );
    }

    #[test]
    fn validate_absolute_normalized_rejects_traversal() {
        let bad = PathBuf::from("/tmp/../etc/passwd");
        let err = validate_absolute_normalized(&bad, FieldKind::Package).unwrap_err();
        assert!(matches!(err, SmokeError::UnsafePackagePath { .. }));
    }

    // --- Stage lifecycle tests ---

    #[test]
    fn open_stage_skipped_when_install_did_not_place_binary() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let request = macos_zip_request(temp.path(), b"bytes", binary, b"bin", evidence);
        let exec = FnExecutor::new(|_, _| Ok(passed()));
        let execution = run_smoke(request, &exec).unwrap();
        assert!(!execution.receipt.passed);
        assert_eq!(execution.receipt.open.status, StageStatus::Skipped);
    }

    #[test]
    fn executable_hash_mismatch_after_install_fails() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary_dir = temp.path().join("Rutile.app/Contents/MacOS");
        fs::create_dir_all(&binary_dir).unwrap();
        let binary = binary_dir.join("Rutile");
        // Binary will be created with WRONG hash by the executor.
        let request = macos_zip_request(
            temp.path(),
            b"bytes",
            binary.clone(),
            b"correct-hash-contents",
            evidence,
        );
        let binary_clone = binary.clone();
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => {
                fs::write(&binary_clone, b"WRONG").ok();
                Ok(passed())
            }
            SmokeStage::Uninstall => {
                fs::remove_file(&binary_clone).ok();
                Ok(passed())
            }
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert!(!execution.receipt.passed);
        assert!(
            execution
                .failure
                .as_deref()
                .unwrap_or_default()
                .contains("sha256 mismatch"),
            "{:?}",
            execution.failure
        );
    }

    #[test]
    fn nonzero_install_exit_fails_and_skips_open() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let request = macos_zip_request(temp.path(), b"bytes", binary, b"bin", evidence);
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => Ok(failed(5)),
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert!(!execution.receipt.passed);
        assert_eq!(execution.receipt.install.status, StageStatus::Failed);
        assert_eq!(execution.receipt.open.status, StageStatus::Skipped);
    }

    #[test]
    fn uninstall_residue_detected_when_binary_survives() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let bin_contents = b"bin";
        let install_target = temp.path().join("Rutile.app");
        let request = SmokeRequest {
            install_target: install_target.clone(),
            ..macos_zip_request(
                temp.path(),
                b"bytes",
                binary.clone(),
                bin_contents,
                evidence,
            )
        };
        let binary_clone = binary.clone();
        let bin_owned = bin_contents.to_vec();
        // Install creates the binary; uninstall returns passed but leaves it
        // in place to trigger the residue check.
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => {
                fs::create_dir_all(binary_clone.parent().unwrap()).ok();
                fs::write(&binary_clone, &bin_owned).ok();
                Ok(passed())
            }
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert!(!execution.receipt.passed);
        assert!(
            execution
                .failure
                .as_deref()
                .unwrap_or_default()
                .contains("residue"),
            "{:?}",
            execution.failure
        );
    }

    #[test]
    fn executor_error_fails_stage_and_records_summary() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let request = macos_zip_request(temp.path(), b"bytes", binary, b"bin", evidence);
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => Err(SmokeError::ToolUnavailable {
                stage: "install",
                program: "/usr/bin/ditto".into(),
            }),
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert!(!execution.receipt.passed);
        assert_eq!(execution.receipt.install.status, StageStatus::Failed);
    }

    // --- Full-pass and receipt tests ---

    #[test]
    fn full_pass_produces_schema_valid_receipt() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("evidence");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let bin_contents = b"binary payload";
        let install_target = temp.path().join("Rutile.app");
        let request = SmokeRequest {
            install_target: install_target.clone(),
            ..macos_zip_request(
                temp.path(),
                b"bytes",
                binary.clone(),
                bin_contents,
                evidence.clone(),
            )
        };
        let binary_clone = binary.clone();
        let bin_owned = bin_contents.to_vec();
        let target_clone = install_target.clone();
        // Install creates the binary with the expected hash; uninstall removes it.
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => {
                fs::create_dir_all(binary_clone.parent().unwrap()).ok();
                fs::write(&binary_clone, &bin_owned).ok();
                Ok(passed())
            }
            SmokeStage::Uninstall => {
                fs::remove_file(&binary_clone).ok();
                fs::remove_dir_all(&target_clone).ok();
                Ok(passed())
            }
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert!(execution.receipt.passed, "{:?}", execution.failure);

        // Validate against the real registered schema.
        crate::evidence::validate_kind(&execution.receipt_path, "package-smoke-row")
            .expect("receipt validates against registered package-smoke-row schema");

        // Spot-check key fields.
        let json = serde_json::to_value(&execution.receipt).unwrap();
        assert_eq!(json["schema"], SMOKE_ROW_SCHEMA);
        assert_eq!(json["passed"].as_bool(), Some(true));
        assert_eq!(json["install"]["observed_binary"].as_bool(), Some(true));
        assert_eq!(json["open"]["observed_binary"].as_bool(), Some(true));
        assert!(json["install"]["executable_sha256"].is_string());
        assert_eq!(json["expected_executable_sha256"], sha256_hex(bin_contents));
        assert_eq!(json["evidence_refs"].as_array().unwrap().len(), 6);
        // Receipt is at evidence_dir root, not in a run subdir.
        assert_eq!(
            execution.receipt_path,
            evidence.join("package-smoke-row.json")
        );
    }

    #[test]
    fn receipt_at_evidence_dir_root() {
        let temp = smoke_tempdir();
        let evidence = temp.path().join("ev");
        let binary = temp.path().join("Rutile.app/Contents/MacOS/Rutile");
        let bin_contents = b"bin";
        let install_target = temp.path().join("Rutile.app");
        let request = SmokeRequest {
            install_target: install_target.clone(),
            ..macos_zip_request(
                temp.path(),
                b"bytes",
                binary.clone(),
                bin_contents,
                evidence.clone(),
            )
        };
        let binary_clone = binary.clone();
        let bin_owned = bin_contents.to_vec();
        let target_clone = install_target.clone();
        let exec = FnExecutor::new(move |stage, _| match stage {
            SmokeStage::Install => {
                fs::create_dir_all(binary_clone.parent().unwrap()).ok();
                fs::write(&binary_clone, &bin_owned).ok();
                Ok(passed())
            }
            SmokeStage::Uninstall => {
                fs::remove_file(&binary_clone).ok();
                fs::remove_dir_all(&target_clone).ok();
                Ok(passed())
            }
            _ => Ok(passed()),
        });
        let execution = run_smoke(request, &exec).unwrap();
        assert_eq!(
            execution.receipt_path.file_name().unwrap(),
            "package-smoke-row.json"
        );
    }

    // --- Production executor tests ---

    #[test]
    fn production_executor_rejects_unsafe_program() {
        let exec = ProductionSmokeExecutor::new(PathBuf::from("/usr/bin/rutile"));
        let cmd = SmokeCommand::new(
            SmokeStage::Install,
            "/bin/sh",
            vec!["-c".into(), "echo unsafe".into()],
        );
        let err = exec.execute(&cmd).unwrap_err();
        assert!(
            matches!(err, SmokeError::UnsafeCommand { ref program, .. } if program == "/bin/sh"),
            "{err:?}"
        );
    }

    #[test]
    fn production_executor_accepts_binary_path_for_open() {
        let temp = smoke_tempdir();
        let fake_bin = temp.path().join("rutile");
        fs::write(&fake_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_bin, fs::Permissions::from_mode(0o755)).unwrap();
        let exec = ProductionSmokeExecutor::new(fake_bin.clone());
        let cmd = SmokeCommand::new(
            SmokeStage::Open,
            fake_bin.to_string_lossy().into_owned(),
            vec!["--native-smoke".into()],
        );
        let outcome = exec.execute(&cmd).expect("binary_path accepted");
        assert_eq!(outcome.status, StageStatus::Passed);
        assert!(outcome.reaped);
    }

    // --- Plan builder tests ---

    #[test]
    fn macos_dmg_install_plan_has_attach_ditto_detach_sequence() {
        let temp = smoke_tempdir();
        let package = write_package(temp.path(), "Rutile.dmg", b"dmg");
        let evidence = temp.path().join("ev");
        let request = SmokeRequest::new(
            package,
            sha256_hex(b"dmg"),
            PackageKind::MacosDmg,
            valid_commit(),
            temp.path().join("Rutile.app"),
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            valid_hash(),
            evidence,
        );
        let plan = build_install_plan(&request).unwrap();
        assert_eq!(plan.commands.len(), 3);
        assert_eq!(plan.commands[0].program(), "/usr/bin/hdiutil");
        assert!(plan.commands[0].args().iter().any(|a| a == "-mountpoint"));
        assert_eq!(plan.commands[1].program(), "/usr/bin/ditto");
        assert_eq!(plan.commands[2].program(), "/usr/bin/hdiutil");
        assert!(plan.commands[2].args().contains(&"detach".into()));
        // Cleanup includes detach.
        assert_eq!(plan.cleanup_on_failure.len(), 1);
        assert!(plan.cleanup_on_failure[0].args().contains(&"detach".into()));
    }

    #[test]
    fn macos_open_plan_passes_native_smoke_args() {
        let temp = smoke_tempdir();
        let request = SmokeRequest::new(
            write_package(temp.path(), "Rutile.app.zip", b"zip"),
            sha256_hex(b"zip"),
            PackageKind::MacosAppZip,
            valid_commit(),
            temp.path().join("Rutile.app"),
            temp.path().join("Rutile.app/Contents/MacOS/Rutile"),
            valid_hash(),
            temp.path().join("ev"),
        );
        let plan = build_open_plan(&request).unwrap();
        assert_eq!(plan.commands.len(), 1);
        let args: Vec<&str> = plan.commands[0]
            .args()
            .iter()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert!(args.contains(&"--args"));
        assert!(args.contains(&"--native-smoke"));
        assert!(args.contains(&"-W"));
    }

    #[test]
    fn linux_tar_zst_plan_uses_mkdir_and_strip_components() {
        let temp = smoke_tempdir();
        let package = write_package(temp.path(), "Rutile.tar.zst", b"archive");
        let install_target = temp.path().join("prefix");
        let request = SmokeRequest::new(
            package,
            sha256_hex(b"archive"),
            PackageKind::LinuxTarZst,
            valid_commit(),
            install_target.clone(),
            install_target.join("bin/rutile"),
            valid_hash(),
            temp.path().join("ev"),
        );
        let plan = build_install_plan(&request).unwrap();
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[0].program(), "/bin/mkdir");
        assert!(plan.commands[0].args().contains(&"-p".into()));
        assert!(
            plan.commands[0]
                .args()
                .contains(&install_target.clone().into_os_string())
        );
        assert_eq!(plan.commands[1].program(), "/usr/bin/tar");
        assert!(
            plan.commands[1]
                .args()
                .contains(&"--strip-components=1".into())
        );
        assert!(
            plan.commands[1]
                .args()
                .contains(&install_target.into_os_string())
        );
    }

    #[test]
    fn linux_open_plan_does_not_force_gdk_backend() {
        let temp = smoke_tempdir();
        let request = SmokeRequest::new(
            write_package(temp.path(), "rutile.deb", b"deb"),
            sha256_hex(b"deb"),
            PackageKind::LinuxDeb,
            valid_commit(),
            PathBuf::from("/"),
            PathBuf::from("/usr/bin/rutile"),
            valid_hash(),
            temp.path().join("ev"),
        );
        let plan = build_open_plan(&request).unwrap();
        // The plan has no env overrides (GDK_BACKEND is not set).
        assert_eq!(plan.commands.len(), 1);
    }

    #[test]
    fn linux_deb_plan_uses_apt_get_install() {
        let temp = smoke_tempdir();
        let package = write_package(temp.path(), "rutile.deb", b"deb");
        let request = SmokeRequest::new(
            package.clone(),
            sha256_hex(b"deb"),
            PackageKind::LinuxDeb,
            valid_commit(),
            PathBuf::from("/"),
            PathBuf::from("/usr/bin/rutile"),
            valid_hash(),
            temp.path().join("ev"),
        );
        let plan = build_install_plan(&request).unwrap();
        assert_eq!(plan.commands[0].program(), "/usr/bin/apt-get");
        assert!(plan.commands[0].args().contains(&"install".into()));
        assert!(plan.commands[0].args().contains(&package.into_os_string()));
    }

    // --- Misc tests ---

    #[test]
    fn write_create_only_rejects_existing_destination() {
        let temp = smoke_tempdir();
        let target = temp.path().join("exists.json");
        fs::write(&target, b"first").unwrap();
        let err = write_create_only(&target, b"second").unwrap_err();
        assert!(matches!(err, SmokeError::Publish { .. }));
        assert_eq!(fs::read(&target).unwrap(), b"first");
    }

    #[test]
    fn is_residue_candidate_rejects_root_prefix() {
        assert!(!is_residue_candidate(
            Path::new("/"),
            Path::new("/usr/bin/rutile")
        ));
        assert!(is_residue_candidate(
            Path::new("/Applications/Rutile.app"),
            Path::new("/Applications/Rutile.app/Contents/MacOS/Rutile")
        ));
    }

    #[test]
    fn stage_plan_single_has_no_cleanup() {
        let cmd = SmokeCommand::new(SmokeStage::Install, "/bin/rm", vec!["-rf".into()]);
        let plan = StagePlan::single(cmd);
        assert_eq!(plan.commands.len(), 1);
        assert!(plan.cleanup_on_failure.is_empty());
    }
}
