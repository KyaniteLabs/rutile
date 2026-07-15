//! Linux native gate result emitter — schema-validated `rutile.gate-result.v1`.
//!
//! Mirrors `native_smoke.rs`: bounded output (16 KiB retained logs), fail-closed
//! atomic publication, complete git provenance captured before assembly, and
//! distinct evidence run directories that never overwrite an existing file.
//!
//! Unlike the macOS native-smoke gate (which supervises the application binary
//! directly), the Linux gate delegates process supervision to an external shell
//! harness that owns the Xvfb display and D-Bus session.  This module is the
//! authoritative `rutile.gate-result.v1` producer: it reads the measured harness
//! output, captures provenance + artifact identity, bounds the retained logs,
//! and writes the schema-valid document.  The shell never assembles JSON.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::native_smoke::{self, NativeSmokeProfile, SourceIdentity, resolve_repeats};

const RETAINED_OUTPUT_BYTES: usize = 16 * 1024;
const ERROR_LINE_MAX_CHARS: usize = 160;

static EVIDENCE_RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct LinuxGateRequest {
    pub binary: PathBuf,
    pub profile: NativeSmokeProfile,
    pub cycles: usize,
    pub exit_code: i32,
    pub started_unix_ms: u128,
    pub ended_unix_ms: u128,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub evidence_dir: PathBuf,
}

pub struct LinuxGateExecution {
    pub report_path: PathBuf,
    pub passed: bool,
    pub failure: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LinuxGateError {
    #[error("linux gate cycles below profile floor: {0}")]
    Repeat(#[from] native_smoke::RepeatPolicyError),
    #[error("linux gate evidence I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not serialize linux gate result: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("could not capture git {operation} for linux gate provenance: {detail}")]
    Git {
        operation: &'static str,
        detail: String,
    },
    #[error("configured linux gate binary {path} is not usable: {detail}")]
    Artifact { path: PathBuf, detail: String },
    #[error("linux gate harness output unreadable: {detail}")]
    Harness { detail: String },
}

/// Parse the lifecycle harness summary line `ready=N closed=N failures=N`.
///
/// Scans stdout in reverse (the summary is the final line) and returns
/// `(ready, closed, failures)`, or an error if the line is absent.
fn parse_harness_summary(stdout: &str) -> Result<(usize, usize, usize), LinuxGateError> {
    for line in stdout.lines().rev() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() != 3 {
            continue;
        }
        let ready = tokens[0]
            .strip_prefix("ready=")
            .and_then(|v| v.parse::<usize>().ok());
        let closed = tokens[1]
            .strip_prefix("closed=")
            .and_then(|v| v.parse::<usize>().ok());
        let failures = tokens[2]
            .strip_prefix("failures=")
            .and_then(|v| v.parse::<usize>().ok());
        if let (Some(r), Some(c), Some(f)) = (ready, closed, failures) {
            return Ok((r, c, f));
        }
    }
    Err(LinuxGateError::Harness {
        detail: "summary line 'ready=N closed=N failures=N' not found in stdout".to_owned(),
    })
}

/// Read at most `limit` bytes from `path`, returning the truncated contents.
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, LinuxGateError> {
    let mut file = File::open(path).map_err(|source| LinuxGateError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut buf = vec![0_u8; limit];
    let read = file.read(&mut buf).map_err(|source| LinuxGateError::Io {
        path: path.to_owned(),
        source,
    })?;
    buf.truncate(read);
    Ok(buf)
}

/// Extract the first non-empty line from `stderr`, truncated to 160 chars,
/// for use as the per-run `error` field on failure.
fn error_line(stderr: &str) -> String {
    for line in stderr.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let truncated: String = trimmed.chars().take(ERROR_LINE_MAX_CHARS).collect();
            return truncated;
        }
    }
    String::new()
}

pub fn run_gate(request: LinuxGateRequest) -> Result<LinuxGateExecution, LinuxGateError> {
    // Validate the cycle floor before touching the filesystem.
    let cycles = resolve_repeats(request.profile, Some(request.cycles))?;

    // Capture git provenance before assembly (mirrors native_smoke).
    let source = native_smoke::source_identity().map_err(|error| {
        let detail = error.to_string();
        LinuxGateError::Git {
            operation: "source identity",
            detail,
        }
    })?;

    // Capture artifact identity (sha256 + device/inode/bytes).
    let artifact = native_smoke::capture_artifact(&request.binary).map_err(|error| {
        LinuxGateError::Artifact {
            path: request.binary.clone(),
            detail: error.to_string(),
        }
    })?;

    // Read and bound the harness output (16 KiB cap).
    let stdout_bytes = read_bounded(&request.stdout_log, RETAINED_OUTPUT_BYTES)?;
    let stderr_bytes = read_bounded(&request.stderr_log, RETAINED_OUTPUT_BYTES)?;
    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    let stderr_str = String::from_utf8_lossy(&stderr_bytes);

    // Parse the harness summary to derive measured test counts.
    let (ready, closed, failures) = parse_harness_summary(&stdout_str)?;

    // Derive pass/fail from the measured values, not from the exit code alone.
    let harness_passed =
        request.exit_code == 0 && failures == 0 && ready == cycles && closed == cycles;

    let (status, failed_count, err) = if harness_passed {
        ("passed", 0usize, None)
    } else {
        let fc = if failures != 0 {
            failures
        } else if ready < cycles {
            cycles - ready
        } else {
            1
        };
        ("failed", fc, Some(error_line(&stderr_str)))
    };

    let passed = harness_passed;

    // Create the evidence run directory (distinct, non-overwriting).
    fs::create_dir_all(&request.evidence_dir).map_err(|source| LinuxGateError::Io {
        path: request.evidence_dir.clone(),
        source,
    })?;
    let evidence_run_dir = create_evidence_run_dir(&request.evidence_dir)?;

    // Retain bounded logs to the evidence run dir.
    let stdout_filename = "run-0001.stdout.log";
    let stderr_filename = "run-0001.stderr.log";
    write_file(&evidence_run_dir.join(stdout_filename), &stdout_bytes)?;
    write_file(&evidence_run_dir.join(stderr_filename), &stderr_bytes)?;

    let retained_logs = vec![
        RetainedLog {
            run: 1,
            stream: "stdout",
            path: stdout_filename.to_owned(),
            bytes: stdout_bytes.len(),
            sha256: native_smoke::sha256_bytes(&stdout_bytes),
        },
        RetainedLog {
            run: 1,
            stream: "stderr",
            path: stderr_filename.to_owned(),
            bytes: stderr_bytes.len(),
            sha256: native_smoke::sha256_bytes(&stderr_bytes),
        },
    ];

    let runs = vec![GateRun {
        run: 1,
        status,
        reaped: false,
        stage_traces: 0,
        resize_traces: 0,
        error: err.clone(),
    }];

    let command_id = "linux-native-smoke";
    let report = GateResult {
        schema: "rutile.gate-result.v1",
        command_id,
        profile: request.profile.as_str(),
        source,
        evidence: EvidenceRun {
            run_directory: evidence_run_dir
                .file_name()
                .expect("generated evidence run directory has a file name")
                .to_string_lossy()
                .into_owned(),
        },
        runner: RunnerIdentity {
            platform: "linux",
            architecture: std::env::consts::ARCH,
            name: native_smoke::runner_name(),
        },
        started_unix_ms: request.started_unix_ms,
        ended_unix_ms: request.ended_unix_ms,
        exit_code: if passed { 0 } else { 1 },
        tests: TestCounts {
            total: cycles,
            passed: ready,
            failed: failed_count,
            ignored: 0,
            skipped: 0,
        },
        required_row: RequiredRow {
            name: command_id,
            required: true,
            status: if passed { "passed" } else { "failed" },
        },
        artifact_hashes: vec![ArtifactHash {
            path: request.binary.display().to_string(),
            sha256: artifact.sha256,
            identity: artifact.identity,
        }],
        retained_logs,
        runs,
    };

    let report_path = evidence_run_dir.join("gate-result.json");
    let mut json = serde_json::to_vec_pretty(&report)?;
    json.push(b'\n');
    write_file(&report_path, &json)?;

    Ok(LinuxGateExecution {
        report_path,
        passed,
        failure: err,
    })
}

// -- Serializable gate-result types (mirror native_smoke field-for-field) -----

#[derive(Serialize)]
struct GateResult<'a> {
    schema: &'a str,
    command_id: &'a str,
    profile: &'a str,
    source: SourceIdentity,
    evidence: EvidenceRun,
    runner: RunnerIdentity,
    started_unix_ms: u128,
    ended_unix_ms: u128,
    exit_code: i32,
    tests: TestCounts,
    required_row: RequiredRow<'a>,
    artifact_hashes: Vec<ArtifactHash>,
    retained_logs: Vec<RetainedLog<'a>>,
    runs: Vec<GateRun>,
}

#[derive(Serialize)]
struct EvidenceRun {
    run_directory: String,
}

#[derive(Serialize)]
struct RunnerIdentity {
    platform: &'static str,
    architecture: &'static str,
    name: String,
}

#[derive(Serialize)]
struct TestCounts {
    total: usize,
    passed: usize,
    failed: usize,
    ignored: usize,
    skipped: usize,
}

#[derive(Serialize)]
struct RequiredRow<'a> {
    name: &'a str,
    required: bool,
    status: &'a str,
}

#[derive(Serialize)]
struct ArtifactHash {
    path: String,
    sha256: String,
    identity: native_smoke::ArtifactIdentity,
}

#[derive(Serialize)]
struct RetainedLog<'a> {
    run: usize,
    stream: &'a str,
    path: String,
    bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct GateRun {
    run: usize,
    status: &'static str,
    reaped: bool,
    stage_traces: usize,
    resize_traces: usize,
    error: Option<String>,
}

// -- Filesystem helpers (mirror native_smoke: atomic, non-overwriting) --------

fn create_evidence_run_dir(evidence_dir: &Path) -> Result<PathBuf, LinuxGateError> {
    loop {
        let counter = EVIDENCE_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let run_dir = evidence_dir.join(format!(
            "run-{}-{}-{counter}",
            unix_time_ms(),
            std::process::id()
        ));
        match fs::create_dir(&run_dir) {
            Ok(()) => return Ok(run_dir),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(LinuxGateError::Io {
                    path: run_dir,
                    source,
                });
            }
        }
    }
}

/// Atomically write `contents` to `path`.  The file is created via
/// `create_new` (fails if it already exists) and then hard-linked into its
/// final name, mirroring native_smoke's fail-closed publication.
fn write_file(path: &Path, contents: &[u8]) -> Result<(), LinuxGateError> {
    let parent = path.parent().ok_or_else(|| LinuxGateError::Io {
        path: path.to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidInput, "evidence file has no parent"),
    })?;
    let filename = path
        .file_name()
        .ok_or_else(|| LinuxGateError::Io {
            path: path.to_owned(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "evidence file has no name"),
        })?
        .to_string_lossy()
        .into_owned();
    let counter = EVIDENCE_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{filename}.tmp-{}-{counter}", std::process::id()));

    let publication = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path)?;
        fs::remove_file(&temporary)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if let Err(source) = publication {
        let _ = fs::remove_file(&temporary);
        return Err(LinuxGateError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_harness_summary_extracts_three_counts() {
        let stdout = "boot\nready\nready=10 closed=10 failures=0\n";
        let (ready, closed, failures) = parse_harness_summary(stdout).unwrap();
        assert_eq!((ready, closed, failures), (10, 10, 0));
    }

    #[test]
    fn parse_harness_summary_fails_closed_when_absent() {
        let stdout = "no summary here\n";
        assert!(parse_harness_summary(stdout).is_err());
    }

    #[test]
    fn parse_harness_summary_takes_last_matching_line() {
        let stdout = "ready=5 closed=5 failures=0\nready=10 closed=10 failures=0\n";
        let (ready, _, _) = parse_harness_summary(stdout).unwrap();
        assert_eq!(ready, 10);
    }

    #[test]
    fn error_line_truncates_to_160_chars() {
        let long_line = "x".repeat(300);
        let stderr = format!("{long_line}\n");
        let extracted = error_line(&stderr);
        assert_eq!(extracted.chars().count(), ERROR_LINE_MAX_CHARS);
    }

    #[test]
    fn error_line_skips_blank_lines() {
        let stderr = "\n\n  \nactual error\n";
        assert_eq!(error_line(stderr), "actual error");
    }

    #[test]
    fn evidence_publication_never_overwrites_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.json");
        fs::write(&path, b"original").unwrap();

        let error = write_file(&path, b"replacement").expect_err("overwrite must fail closed");

        assert!(matches!(error, LinuxGateError::Io { .. }));
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn evidence_run_directory_has_schema_pattern() {
        let root = tempfile::tempdir().unwrap();
        let dir = create_evidence_run_dir(root.path()).unwrap();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("run-"),
            "evidence run dir must match run-<ms>-<pid>-<n>: {name}"
        );
        let parts: Vec<&str> = name.splitn(4, '-').collect();
        assert_eq!(parts.len(), 4, "expected 4 dash-separated segments: {name}");
        assert_eq!(parts[0], "run");
        assert!(
            parts[1].parse::<u64>().is_ok(),
            "ms segment must be numeric"
        );
        assert!(
            parts[2].parse::<u32>().is_ok(),
            "pid segment must be numeric"
        );
        assert!(
            parts[3].parse::<u64>().is_ok(),
            "counter segment must be numeric"
        );
    }

    #[test]
    fn read_bounded_truncates_to_limit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.log");
        fs::write(&path, vec![b'x'; 100_000]).unwrap();
        let data = read_bounded(&path, 256).unwrap();
        assert_eq!(data.len(), 256);
    }

    #[test]
    fn read_bounded_returns_actual_size_for_small_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("small.log");
        fs::write(&path, b"tiny").unwrap();
        let data = read_bounded(&path, 256).unwrap();
        assert_eq!(data, b"tiny");
    }
}
