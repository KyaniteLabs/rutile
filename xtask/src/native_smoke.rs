//! Externally supervised native-smoke process execution and gate evidence.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PARENT_DEADLINE: Duration = Duration::from_secs(30);
pub const TERM_GRACE: Duration = Duration::from_secs(5);
const RETAINED_OUTPUT_BYTES: usize = 16 * 1024;
const RETAINED_TRACE_LINES: usize = 64;
const RETAINED_TRACE_LINE_BYTES: usize = 512;
const KILL_SETTLE: Duration = Duration::from_millis(50);
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const GIT_DEADLINE: Duration = Duration::from_secs(2);
const GIT_CLEANUP_GRACE: Duration = Duration::from_secs(1);
const SUCCESS_MARKER: &str = "feathermark-native-smoke-ok";
static EVIDENCE_RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

pub const fn supervision_bound(deadline: Duration, cleanup_grace: Duration) -> Duration {
    deadline.saturating_add(cleanup_grace)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum NativeSmokeProfile {
    Pr,
    Release,
}

impl NativeSmokeProfile {
    pub const fn minimum_repeats(self) -> usize {
        match self {
            Self::Pr => 10,
            Self::Release => 50,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Release => "release",
        }
    }
}

pub fn resolve_repeats(
    profile: NativeSmokeProfile,
    requested: Option<usize>,
) -> Result<usize, RepeatPolicyError> {
    let minimum = profile.minimum_repeats();
    let repeats = requested.unwrap_or(minimum);
    if repeats < minimum {
        return Err(RepeatPolicyError {
            profile,
            requested: repeats,
            minimum,
        });
    }
    Ok(repeats)
}

#[derive(Debug, thiserror::Error)]
#[error(
    "native smoke profile {profile:?} requires at least {minimum} repeats; requested {requested}"
)]
pub struct RepeatPolicyError {
    profile: NativeSmokeProfile,
    requested: usize,
    minimum: usize,
}

#[derive(Clone, Debug)]
pub struct NativeSmokeCommand {
    program: String,
    arguments: Vec<String>,
}

impl NativeSmokeCommand {
    pub fn new<I, S>(program: impl Into<String>, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NativeSmokeDiagnostics {
    pub stdout: String,
    pub stderr: String,
    pub stage_trace: Vec<String>,
    pub resize_trace: Vec<String>,
    pub reaped: bool,
}

#[derive(Clone, Debug)]
pub struct NativeSmokeReceipt {
    diagnostics: NativeSmokeDiagnostics,
}

impl NativeSmokeReceipt {
    pub fn success(&self) -> bool {
        true
    }

    pub fn diagnostics(&self) -> &NativeSmokeDiagnostics {
        &self.diagnostics
    }

    pub fn stdout(&self) -> &str {
        &self.diagnostics.stdout
    }

    pub fn stage_trace(&self) -> &[String] {
        &self.diagnostics.stage_trace
    }

    pub fn resize_trace(&self) -> &[String] {
        &self.diagnostics.resize_trace
    }
}

#[derive(Debug)]
pub enum NativeSmokeFailure {
    Spawn {
        error: String,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    Wait {
        error: String,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    Read {
        error: String,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    Cleanup {
        cause: String,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    Exited {
        status: Option<i32>,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    TimedOut {
        killed: bool,
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    OutputLimit {
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
    ProofMissing {
        diagnostics: Box<NativeSmokeDiagnostics>,
    },
}

impl NativeSmokeFailure {
    pub fn diagnostics(&self) -> &NativeSmokeDiagnostics {
        match self {
            Self::Spawn { diagnostics, .. }
            | Self::Wait { diagnostics, .. }
            | Self::Read { diagnostics, .. }
            | Self::Cleanup { diagnostics, .. }
            | Self::Exited { diagnostics, .. }
            | Self::TimedOut { diagnostics, .. }
            | Self::OutputLimit { diagnostics }
            | Self::ProofMissing { diagnostics } => diagnostics,
        }
    }
}

impl std::fmt::Display for NativeSmokeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { error, .. } => write!(formatter, "could not spawn native smoke: {error}"),
            Self::Wait { error, .. } => {
                write!(formatter, "could not wait for native smoke: {error}")
            }
            Self::Read { error, .. } => write!(formatter, "could not read native smoke: {error}"),
            Self::Cleanup { cause, .. } => write!(
                formatter,
                "native smoke process-group cleanup could not be verified: {cause}"
            ),
            Self::Exited { status, .. } => {
                write!(formatter, "native smoke exited unsuccessfully: {status:?}")
            }
            Self::TimedOut { killed, .. } => write!(
                formatter,
                "native smoke exceeded parent deadline (SIGKILL used: {killed})"
            ),
            Self::OutputLimit { .. } => {
                write!(formatter, "native smoke exceeded retained output limit")
            }
            Self::ProofMissing { .. } => {
                write!(formatter, "native smoke exited without its proof marker")
            }
        }?;
        let diagnostics = self.diagnostics();
        write!(
            formatter,
            "\nretained stdout:\n{}\nretained stderr:\n{}\nretained stage traces:\n{}\nretained resize traces:\n{}",
            diagnostics.stdout,
            diagnostics.stderr,
            diagnostics.stage_trace.join("\n"),
            diagnostics.resize_trace.join("\n"),
        )
    }
}

impl std::error::Error for NativeSmokeFailure {}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SupervisorFaults {
    wait_once: bool,
    stderr_read_once: bool,
}

impl SupervisorFaults {
    pub const fn wait_once() -> Self {
        Self {
            wait_once: true,
            stderr_read_once: false,
        }
    }

    pub const fn stderr_read_once() -> Self {
        Self {
            wait_once: false,
            stderr_read_once: true,
        }
    }
}

#[derive(Default)]
struct FaultState {
    wait_once: bool,
    wait_injected: AtomicBool,
    stderr_read_once: bool,
}

impl From<SupervisorFaults> for FaultState {
    fn from(faults: SupervisorFaults) -> Self {
        Self {
            wait_once: faults.wait_once,
            wait_injected: AtomicBool::new(false),
            stderr_read_once: faults.stderr_read_once,
        }
    }
}

pub fn supervise(command: NativeSmokeCommand) -> Result<NativeSmokeReceipt, NativeSmokeFailure> {
    supervise_with(
        command,
        PARENT_DEADLINE,
        TERM_GRACE,
        SupervisorFaults::default(),
        Some(SUCCESS_MARKER),
    )
}

#[doc(hidden)]
pub fn supervise_for_test(
    command: NativeSmokeCommand,
    deadline: Duration,
    term_grace: Duration,
) -> Result<NativeSmokeReceipt, NativeSmokeFailure> {
    supervise_with(
        command,
        deadline,
        term_grace,
        SupervisorFaults::default(),
        Some(SUCCESS_MARKER),
    )
}

#[doc(hidden)]
pub fn supervise_for_test_with_faults(
    command: NativeSmokeCommand,
    deadline: Duration,
    term_grace: Duration,
    faults: SupervisorFaults,
) -> Result<NativeSmokeReceipt, NativeSmokeFailure> {
    supervise_with(command, deadline, term_grace, faults, Some(SUCCESS_MARKER))
}

fn supervise_with(
    command: NativeSmokeCommand,
    deadline: Duration,
    term_grace: Duration,
    faults: SupervisorFaults,
    required_marker: Option<&str>,
) -> Result<NativeSmokeReceipt, NativeSmokeFailure> {
    let output = Arc::new(OutputCapture::default());
    let fault_state = Arc::new(FaultState::from(faults));
    let mut process = Command::new(&command.program);
    process
        .args(&command.arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the child has not executed application code. Giving it its own
    // group lets this parent signal every descendant without touching its own.
    unsafe {
        process.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[allow(clippy::disallowed_methods)]
    // The native-smoke CLI is the explicit external app supervisor.
    let spawned = process.spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return Err(NativeSmokeFailure::Spawn {
                error: error.to_string(),
                diagnostics: Box::default(),
            });
        }
    };
    let started = Instant::now();
    let pid = i32::try_from(child.id()).expect("Unix pid fits in i32");
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let mut readers = ReaderTasks::new(
        stdout,
        stderr,
        Arc::clone(&output),
        Arc::clone(&fault_state),
    );

    let (pending, leader_reaped) = loop {
        if output.exceeded.load(Ordering::Acquire) {
            break (PendingFailure::OutputLimit, false);
        }
        if let Some(error) = output.read_failure() {
            break (PendingFailure::Read(error), false);
        }
        match poll_child(&mut child, &fault_state) {
            Ok(Some(status)) if status.success() => break (PendingFailure::Success, true),
            Ok(Some(status)) => break (PendingFailure::Exited(status.code()), true),
            Ok(None) if started.elapsed() >= deadline => {
                break (PendingFailure::TimedOut, false);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => break (PendingFailure::Wait(error.to_string()), false),
        }
    };

    let cleanup = cleanup_group(
        &mut child,
        pid,
        leader_reaped,
        &mut readers,
        &fault_state,
        started,
        deadline,
        term_grace,
    );
    let diagnostics = diagnostics(&output, cleanup.reaped);
    if !cleanup.reaped {
        return Err(NativeSmokeFailure::Cleanup {
            cause: cleanup.error.unwrap_or_else(|| pending.summary()),
            diagnostics: Box::new(diagnostics),
        });
    }

    match pending {
        PendingFailure::Success => {
            if output.exceeded.load(Ordering::Acquire) {
                Err(NativeSmokeFailure::OutputLimit {
                    diagnostics: Box::new(diagnostics),
                })
            } else if let Some(error) = output.read_failure() {
                Err(NativeSmokeFailure::Read {
                    error,
                    diagnostics: Box::new(diagnostics),
                })
            } else if required_marker
                .is_some_and(|required_marker| !diagnostics.stdout.contains(required_marker))
            {
                Err(NativeSmokeFailure::ProofMissing {
                    diagnostics: Box::new(diagnostics),
                })
            } else {
                Ok(NativeSmokeReceipt { diagnostics })
            }
        }
        PendingFailure::Wait(error) => Err(NativeSmokeFailure::Wait {
            error,
            diagnostics: Box::new(diagnostics),
        }),
        PendingFailure::Read(error) => Err(NativeSmokeFailure::Read {
            error,
            diagnostics: Box::new(diagnostics),
        }),
        PendingFailure::Exited(status) => Err(NativeSmokeFailure::Exited {
            status,
            diagnostics: Box::new(diagnostics),
        }),
        PendingFailure::TimedOut => Err(NativeSmokeFailure::TimedOut {
            killed: cleanup.killed,
            diagnostics: Box::new(diagnostics),
        }),
        PendingFailure::OutputLimit => Err(NativeSmokeFailure::OutputLimit {
            diagnostics: Box::new(diagnostics),
        }),
    }
}

enum PendingFailure {
    Success,
    Wait(String),
    Read(String),
    Exited(Option<i32>),
    TimedOut,
    OutputLimit,
}

impl PendingFailure {
    fn summary(&self) -> String {
        match self {
            Self::Success => "successful child teardown".to_owned(),
            Self::Wait(error) => format!("wait failure: {error}"),
            Self::Read(error) => format!("read failure: {error}"),
            Self::Exited(status) => format!("child exit {status:?}"),
            Self::TimedOut => "parent deadline".to_owned(),
            Self::OutputLimit => "output limit".to_owned(),
        }
    }
}

fn poll_child(child: &mut Child, faults: &FaultState) -> io::Result<Option<ExitStatus>> {
    if faults.wait_once && !faults.wait_injected.swap(true, Ordering::AcqRel) {
        return Err(io::Error::other("injected try_wait failure"));
    }
    child.try_wait()
}

struct Cleanup {
    killed: bool,
    reaped: bool,
    error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn cleanup_group(
    child: &mut Child,
    process_group: i32,
    mut leader_reaped: bool,
    readers: &mut ReaderTasks,
    faults: &FaultState,
    started: Instant,
    parent_deadline: Duration,
    cleanup_grace: Duration,
) -> Cleanup {
    let cleanup_started = Instant::now();
    let contract_deadline = started + supervision_bound(parent_deadline, cleanup_grace);
    let cleanup_deadline = (cleanup_started + cleanup_grace).min(contract_deadline);
    let kill_at = cleanup_deadline
        .checked_sub(KILL_SETTLE)
        .unwrap_or(cleanup_started);
    let mut error = signal_group(process_group, libc::SIGTERM)
        .err()
        .map(|value| value.to_string());
    let mut killed = false;

    loop {
        if !leader_reaped {
            match poll_child(child, faults) {
                Ok(Some(_)) => leader_reaped = true,
                Ok(None) => {}
                Err(wait_error) => error = Some(format!("try_wait during cleanup: {wait_error}")),
            }
        }
        let group_gone = match group_exists(process_group) {
            Ok(exists) => !exists,
            Err(group_error) => {
                error = Some(format!("process-group probe: {group_error}"));
                false
            }
        };
        let readers_finished = readers.finished();
        if leader_reaped && group_gone && readers_finished {
            return Cleanup {
                killed,
                reaped: true,
                error,
            };
        }

        let now = Instant::now();
        if !killed && now >= kill_at {
            killed = true;
            if let Err(kill_error) = signal_group(process_group, libc::SIGKILL) {
                error = Some(format!("SIGKILL process group: {kill_error}"));
            }
        }
        if now >= cleanup_deadline {
            return Cleanup {
                killed,
                reaped: false,
                error: Some(error.unwrap_or_else(|| {
                    format!(
                        "deadline reached (leader_reaped={leader_reaped}, group_gone={group_gone}, readers_finished={readers_finished})"
                    )
                })),
            };
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn group_exists(process_group: i32) -> io::Result<bool> {
    let result = unsafe { libc::kill(-process_group, 0) };
    if result == 0 {
        return Ok(true);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(io::Error::last_os_error()),
    }
}

fn signal_group(process_group: i32, signal: i32) -> io::Result<()> {
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[derive(Default)]
struct OutputCapture {
    stdout: Mutex<Vec<u8>>,
    stderr: Mutex<Vec<u8>>,
    stage_trace: Mutex<VecDeque<String>>,
    resize_trace: Mutex<VecDeque<String>>,
    read_failure: Mutex<Option<String>>,
    exceeded: AtomicBool,
}

impl OutputCapture {
    fn read_failure(&self) -> Option<String> {
        self.read_failure
            .lock()
            .expect("read failure lock poisoned")
            .clone()
    }

    fn record_read_failure(&self, stream: Stream, error: &io::Error) {
        let mut failure = self
            .read_failure
            .lock()
            .expect("read failure lock poisoned");
        failure.get_or_insert_with(|| format!("{}: {error}", stream.as_str()));
    }
}

struct ReaderTasks {
    stdout: Receiver<()>,
    stderr: Receiver<()>,
    stdout_finished: bool,
    stderr_finished: bool,
}

impl ReaderTasks {
    fn new(
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        output: Arc<OutputCapture>,
        faults: Arc<FaultState>,
    ) -> Self {
        Self {
            stdout: read_stream(stdout, Arc::clone(&output), Stream::Stdout, false),
            stderr: read_stream(stderr, output, Stream::Stderr, faults.stderr_read_once),
            stdout_finished: false,
            stderr_finished: false,
        }
    }

    fn finished(&mut self) -> bool {
        Self::poll(&self.stdout, &mut self.stdout_finished);
        Self::poll(&self.stderr, &mut self.stderr_finished);
        self.stdout_finished && self.stderr_finished
    }

    fn poll(receiver: &Receiver<()>, finished: &mut bool) {
        if *finished {
            return;
        }
        match receiver.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => *finished = true,
            Err(TryRecvError::Empty) => {}
        }
    }
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

fn read_stream(
    reader: impl Read + Send + 'static,
    output: Arc<OutputCapture>,
    stream: Stream,
    inject_failure: bool,
) -> Receiver<()> {
    let (finished_sender, finished_receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0_u8; 4096];
        let mut line = Vec::with_capacity(RETAINED_TRACE_LINE_BYTES);
        let mut first_read = true;
        loop {
            let read = if inject_failure && first_read {
                Err(io::Error::other("injected read failure"))
            } else {
                reader.read(&mut chunk)
            };
            first_read = false;
            let read = match read {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) => {
                    output.record_read_failure(stream, &error);
                    break;
                }
            };
            match stream {
                Stream::Stdout => retain(&output.stdout, &chunk[..read], &output.exceeded),
                Stream::Stderr => retain(&output.stderr, &chunk[..read], &output.exceeded),
            }
            for byte in &chunk[..read] {
                if *byte == b'\n' {
                    retain_trace_line(&output, &line);
                    line.clear();
                } else if line.len() < RETAINED_TRACE_LINE_BYTES {
                    line.push(*byte);
                }
            }
        }
        retain_trace_line(&output, &line);
        let _ = finished_sender.send(());
    });
    finished_receiver
}

fn retain_trace_line(output: &OutputCapture, line: &[u8]) {
    let text = String::from_utf8_lossy(line).trim_end().to_owned();
    if text.contains("SMOKE_TRACE stage=") {
        retain_trace(&output.stage_trace, text.clone());
    }
    if text.contains("event=resize") {
        retain_trace(&output.resize_trace, text);
    }
}

fn retain(destination: &Mutex<Vec<u8>>, bytes: &[u8], exceeded: &AtomicBool) {
    let mut destination = destination.lock().expect("output capture lock poisoned");
    let remaining = RETAINED_OUTPUT_BYTES.saturating_sub(destination.len());
    let retained = bytes.len().min(remaining);
    destination.extend_from_slice(&bytes[..retained]);
    if retained != bytes.len() {
        exceeded.store(true, Ordering::Release);
    }
}

fn retain_trace(destination: &Mutex<VecDeque<String>>, line: String) {
    let mut destination = destination.lock().expect("trace capture lock poisoned");
    if destination.len() == RETAINED_TRACE_LINES {
        destination.pop_front();
    }
    destination.push_back(line);
}

fn diagnostics(output: &OutputCapture, reaped: bool) -> NativeSmokeDiagnostics {
    NativeSmokeDiagnostics {
        stdout: String::from_utf8_lossy(
            &output.stdout.lock().expect("output capture lock poisoned"),
        )
        .into_owned(),
        stderr: String::from_utf8_lossy(
            &output.stderr.lock().expect("output capture lock poisoned"),
        )
        .into_owned(),
        stage_trace: output
            .stage_trace
            .lock()
            .expect("trace capture lock poisoned")
            .iter()
            .cloned()
            .collect(),
        resize_trace: output
            .resize_trace
            .lock()
            .expect("trace capture lock poisoned")
            .iter()
            .cloned()
            .collect(),
        reaped,
    }
}

pub struct NativeSmokeGateRequest {
    pub binary: PathBuf,
    pub profile: NativeSmokeProfile,
    pub repeat: Option<usize>,
    pub evidence_dir: PathBuf,
}

pub struct NativeSmokeGateExecution {
    pub report_path: PathBuf,
    pub repeats: usize,
    pub passed: bool,
    pub failure: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeSmokeGateError {
    #[error(transparent)]
    Repeat(#[from] RepeatPolicyError),
    #[error("native smoke evidence I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not serialize native smoke gate result: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("could not capture git {operation} for native smoke provenance: {detail}")]
    Git {
        operation: &'static str,
        detail: String,
    },
    #[error("configured native smoke binary {path} is not usable: {detail}")]
    Artifact { path: PathBuf, detail: String },
}

pub fn run_gate(
    request: NativeSmokeGateRequest,
) -> Result<NativeSmokeGateExecution, NativeSmokeGateError> {
    let repeats = resolve_repeats(request.profile, request.repeat)?;
    let source = source_identity()?;
    let artifact = capture_artifact(&request.binary)?;
    create_dir_all(&request.evidence_dir)?;
    let evidence_run_dir = create_evidence_run_dir(&request.evidence_dir)?;
    let started_unix_ms = unix_time_ms();
    let mut runs = Vec::new();
    let mut retained_logs = Vec::new();
    let mut failure = None;

    for run in 1..=repeats {
        let (status, diagnostics, error) = match verify_artifact(&request.binary, &artifact) {
            Ok(()) => match supervise(NativeSmokeCommand::new(
                request.binary.to_string_lossy(),
                ["--native-smoke"],
            )) {
                Ok(receipt) => ("passed", receipt.diagnostics().clone(), None),
                Err(error) => (
                    "failed",
                    error.diagnostics().clone(),
                    Some(error.to_string()),
                ),
            },
            Err(error) => {
                let error = error.to_string();
                (
                    "failed",
                    NativeSmokeDiagnostics {
                        stderr: error.clone(),
                        ..NativeSmokeDiagnostics::default()
                    },
                    Some(error),
                )
            }
        };
        for (stream, contents) in [
            ("stdout", diagnostics.stdout.as_bytes()),
            ("stderr", diagnostics.stderr.as_bytes()),
        ] {
            let filename = format!("run-{run:04}.{stream}.log");
            let path = evidence_run_dir.join(&filename);
            write_file(&path, contents)?;
            retained_logs.push(RetainedLog {
                run,
                stream,
                path: filename,
                bytes: contents.len(),
                sha256: sha256_bytes(contents),
            });
        }
        runs.push(GateRun {
            run,
            status,
            reaped: diagnostics.reaped,
            stage_traces: diagnostics.stage_trace.len(),
            resize_traces: diagnostics.resize_trace.len(),
            error: error
                .as_deref()
                .map(NativeSmokeFailure::summary_from_rendered),
        });
        if let Some(error) = error {
            failure = Some(error);
            break;
        }
    }

    let passed = failure.is_none() && runs.len() == repeats;
    let passed_count = runs.iter().filter(|run| run.status == "passed").count();
    let failed_count = runs.iter().filter(|run| run.status == "failed").count();
    let skipped_count = repeats.saturating_sub(runs.len());
    let report = GateResult {
        schema: "rutile.gate-result.v1",
        command_id: "macos-native-smoke",
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
            platform: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            name: runner_name(),
        },
        started_unix_ms,
        ended_unix_ms: unix_time_ms(),
        exit_code: if passed { 0 } else { 1 },
        tests: TestCounts {
            total: repeats,
            passed: passed_count,
            failed: failed_count,
            ignored: 0,
            skipped: skipped_count,
        },
        required_row: RequiredRow {
            name: "macos-native-smoke",
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

    Ok(NativeSmokeGateExecution {
        report_path,
        repeats,
        passed,
        failure,
    })
}

impl NativeSmokeFailure {
    fn summary_from_rendered(rendered: &str) -> String {
        rendered.lines().next().unwrap_or(rendered).to_owned()
    }
}

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
    runs: Vec<GateRun<'a>>,
}

#[derive(Serialize)]
struct SourceIdentity {
    commit: String,
    tree: String,
    dirty: bool,
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
    identity: ArtifactIdentity,
}

#[derive(Serialize)]
struct ArtifactIdentity {
    device: u64,
    inode: u64,
    bytes: u64,
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
struct GateRun<'a> {
    run: usize,
    status: &'a str,
    reaped: bool,
    stage_traces: usize,
    resize_traces: usize,
    error: Option<String>,
}

fn create_dir_all(path: &Path) -> Result<(), NativeSmokeGateError> {
    fs::create_dir_all(path).map_err(|source| NativeSmokeGateError::Io {
        path: path.to_owned(),
        source,
    })
}

fn create_evidence_run_dir(evidence_dir: &Path) -> Result<PathBuf, NativeSmokeGateError> {
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
                return Err(NativeSmokeGateError::Io {
                    path: run_dir,
                    source,
                });
            }
        }
    }
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), NativeSmokeGateError> {
    let parent = path.parent().ok_or_else(|| NativeSmokeGateError::Io {
        path: path.to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidInput, "evidence file has no parent"),
    })?;
    let filename = path
        .file_name()
        .ok_or_else(|| NativeSmokeGateError::Io {
            path: path.to_owned(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "evidence file has no name"),
        })?
        .to_string_lossy();
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
        return Err(NativeSmokeGateError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

struct CapturedArtifact {
    sha256: String,
    identity: ArtifactIdentity,
}

fn capture_artifact(path: &Path) -> Result<CapturedArtifact, NativeSmokeGateError> {
    let metadata = fs::metadata(path).map_err(|source| NativeSmokeGateError::Artifact {
        path: path.to_owned(),
        detail: source.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(NativeSmokeGateError::Artifact {
            path: path.to_owned(),
            detail: "not a regular file".to_owned(),
        });
    }
    if metadata.mode() & 0o111 == 0 {
        return Err(NativeSmokeGateError::Artifact {
            path: path.to_owned(),
            detail: "not executable".to_owned(),
        });
    }
    let mut file = File::open(path).map_err(|source| NativeSmokeGateError::Artifact {
        path: path.to_owned(),
        detail: source.to_string(),
    })?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|source| NativeSmokeGateError::Artifact {
                path: path.to_owned(),
                detail: source.to_string(),
            })?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(CapturedArtifact {
        sha256: hex::encode(digest.finalize()),
        identity: ArtifactIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
        },
    })
}

fn verify_artifact(path: &Path, expected: &CapturedArtifact) -> Result<(), NativeSmokeGateError> {
    let observed = capture_artifact(path)?;
    if observed.sha256 != expected.sha256
        || observed.identity.device != expected.identity.device
        || observed.identity.inode != expected.identity.inode
        || observed.identity.bytes != expected.identity.bytes
    {
        return Err(NativeSmokeGateError::Artifact {
            path: path.to_owned(),
            detail: "changed since preflight artifact capture".to_owned(),
        });
    }
    Ok(())
}

fn sha256_bytes(contents: &[u8]) -> String {
    hex::encode(Sha256::digest(contents))
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn source_identity() -> Result<SourceIdentity, NativeSmokeGateError> {
    Ok(SourceIdentity {
        commit: git_identity("commit", &["rev-parse", "HEAD"])?,
        tree: git_identity("tree", &["rev-parse", "HEAD^{tree}"])?,
        dirty: !git_output(
            "worktree status",
            &["status", "--porcelain", "--untracked-files=all"],
        )?
        .is_empty(),
    })
}

fn git_identity(
    operation: &'static str,
    arguments: &[&str],
) -> Result<String, NativeSmokeGateError> {
    let identity = git_output(operation, arguments)?;
    validate_git_identity(operation, &identity)?;
    Ok(identity)
}

fn validate_git_identity(
    operation: &'static str,
    identity: &str,
) -> Result<(), NativeSmokeGateError> {
    if identity.is_empty() {
        return Err(NativeSmokeGateError::Git {
            operation,
            detail: "identity was empty".to_owned(),
        });
    }
    if !(identity.len() == 40 || identity.len() == 64)
        || !identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(NativeSmokeGateError::Git {
            operation,
            detail: format!("identity was not a git object ID: {identity}"),
        });
    }
    Ok(())
}

fn git_output(operation: &'static str, arguments: &[&str]) -> Result<String, NativeSmokeGateError> {
    let command = NativeSmokeCommand::new("git", arguments.iter().copied());
    let receipt = supervise_with(
        command,
        GIT_DEADLINE,
        GIT_CLEANUP_GRACE,
        SupervisorFaults::default(),
        None,
    )
    .map_err(|error| NativeSmokeGateError::Git {
        operation,
        detail: error.to_string(),
    })?;
    Ok(receipt.stdout().trim().to_owned())
}

fn runner_name() -> String {
    ["FORGEJO_RUNNER_NAME", "RUNNER_NAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "local".to_owned())
}

#[cfg(test)]
mod gate_integrity_tests {
    use super::*;

    #[test]
    fn evidence_publication_never_overwrites_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.json");
        fs::write(&path, b"original").unwrap();

        let error = write_file(&path, b"replacement").expect_err("overwrite must fail closed");

        assert!(matches!(error, NativeSmokeGateError::Io { .. }));
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn git_object_ids_must_match_the_lowercase_schema_contract() {
        let uppercase = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";

        let error = validate_git_identity("commit", uppercase)
            .expect_err("uppercase identity must fail before execution");

        assert!(matches!(error, NativeSmokeGateError::Git { .. }));
    }
}
