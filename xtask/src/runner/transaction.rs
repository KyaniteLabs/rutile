//! Crash-safe publication of the affirmative runner-lock pair.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::ProvisionedRunnerConfig;
use super::verification::{VerifiedRunnerLock, verify_runner_lock_bytes_with};
use super::{OfflineLockSummary, RunnerError};

const TRANSACTION_LOCK: &str = ".runner-lock.transaction-lock";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePoint {
    None,
    AfterIncompleteFsync,
    AfterLockFsync,
    AfterLockRename,
    AfterLockParentFsync,
    AfterJournalRewriteFsync,
    BeforeCommittedRename,
    AfterCommittedRename,
    AfterCommittedParentFsync,
    Cleanup,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TransactionRecordV1 {
    schema: String,
    output_basename: String,
    matrix_run_id: [u8; 32],
    lock_length: u64,
    lock_sha256: [u8; 32],
    state: RecordState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecordState {
    Incomplete,
    Committed,
}

pub(crate) struct CommittedRunnerLock {
    // Retaining this descriptor retains the shared transaction lock.
    _transaction_lock: File,
    verified: VerifiedRunnerLock,
}

impl CommittedRunnerLock {
    pub(crate) fn summary(&self) -> OfflineLockSummary {
        OfflineLockSummary {
            runners: self.verified.identities.len(),
            lock_sha256: self.verified.lock_sha256,
        }
    }

    pub(crate) fn lock_sha256(&self) -> [u8; 32] {
        self.verified.lock_sha256
    }

    pub(crate) fn matrix_run_id(&self) -> [u8; 32] {
        self.verified.matrix_run_id
    }

    pub(crate) fn identity(&self, index: usize) -> &super::protocol::RunnerIdentityV1 {
        &self.verified.identities[index]
    }
}

pub(crate) fn publish_runner_lock_with(
    lock_bytes: &[u8],
    out: &Path,
    config: &ProvisionedRunnerConfig,
    failure: FailurePoint,
) -> Result<(), RunnerError> {
    let verified = verify_runner_lock_bytes_with(lock_bytes, config)?;
    let (parent, basename) = checked_parent(out)?;
    let writer_lock = open_transaction_lock(&parent)?;
    lock_exclusive(&writer_lock)?;
    recover_commit_state(&parent, &basename, config)?;

    let run_hex = hex::encode(verified.matrix_run_id);
    let incomplete_name = format!(".{basename}.{run_hex}.incomplete");
    let committed_name = format!(".{basename}.{run_hex}.committed");
    let temp_name = format!(".{basename}.{run_hex}.lock.tmp");
    let incomplete_path = parent.join(&incomplete_name);
    let committed_path = parent.join(&committed_name);
    let temp_path = parent.join(&temp_name);

    let mut record_file = create_exclusive(&incomplete_path, 0o600)?;
    let incomplete_record = record(&basename, &verified, lock_bytes, RecordState::Incomplete);
    write_record(&mut record_file, &incomplete_record)?;
    sync_parent(&parent)?;
    injected_precommit(failure, FailurePoint::AfterIncompleteFsync, &parent, out)?;

    let mut temp = create_exclusive(&temp_path, 0o600)?;
    temp.write_all(lock_bytes)?;
    temp.flush()?;
    temp.seek(SeekFrom::Start(0))?;
    let mut reread = Vec::with_capacity(lock_bytes.len());
    temp.read_to_end(&mut reread)?;
    if reread != lock_bytes {
        return Err(publication("lock temp reread did not match written bytes"));
    }
    verify_runner_lock_bytes_with(&reread, config)?;
    temp.sync_all()?;
    injected_precommit(failure, FailurePoint::AfterLockFsync, &parent, out)?;
    no_replace_rename(&temp_path, out)?;
    injected_precommit(failure, FailurePoint::AfterLockRename, &parent, out)?;
    sync_parent(&parent)?;
    injected_precommit(failure, FailurePoint::AfterLockParentFsync, &parent, out)?;

    let committed_record = record(&basename, &verified, lock_bytes, RecordState::Committed);
    write_record(&mut record_file, &committed_record)?;
    injected_precommit(
        failure,
        FailurePoint::AfterJournalRewriteFsync,
        &parent,
        out,
    )?;
    if failure == FailurePoint::BeforeCommittedRename {
        quarantine_if_present(&parent, out)?;
        sync_parent(&parent)?;
        return Err(publication(
            "injected failure before committed-record rename",
        ));
    }
    no_replace_rename(&incomplete_path, &committed_path)?;
    if failure == FailurePoint::AfterCommittedRename {
        no_replace_rename(&committed_path, &incomplete_path)?;
        quarantine_if_present(&parent, out)?;
        sync_parent(&parent)?;
        return Err(publication(
            "injected failure after committed-record rename",
        ));
    }
    if let Err(error) = sync_parent(&parent) {
        if no_replace_rename(&committed_path, &incomplete_path).is_err()
            || quarantine_if_present(&parent, out).is_err()
            || sync_parent(&parent).is_err()
        {
            return Err(RunnerError::FilesystemContractLost);
        }
        return Err(error.into());
    }

    if failure == FailurePoint::AfterCommittedParentFsync {
        return Ok(());
    }

    // All work below this point is diagnostic cleanup. Failure cannot revoke the
    // already durable, affirmative pair.
    if failure != FailurePoint::Cleanup {
        let _ = fs::remove_file(&temp_path);
    }
    Ok(())
}

fn injected_precommit(
    selected: FailurePoint,
    point: FailurePoint,
    parent: &Path,
    out: &Path,
) -> Result<(), RunnerError> {
    if selected != point {
        return Ok(());
    }
    quarantine_if_present(parent, out)?;
    sync_parent(parent)?;
    Err(publication(&format!("injected failure at {point:?}")))
}

pub(crate) fn open_committed_runner_lock_with(
    out: &Path,
    config: &ProvisionedRunnerConfig,
) -> Result<CommittedRunnerLock, RunnerError> {
    let (parent, basename) = checked_parent(out)?;
    let transaction_lock = open_transaction_lock(&parent)?;
    lock_exclusive(&transaction_lock)?;
    recover_commit_state(&parent, &basename, config)?;
    let committed = committed_records(&parent, &basename)?;
    if committed.len() != 1 || !out.is_file() {
        return Err(publication(
            "authoritative committed pair is absent or ambiguous",
        ));
    }
    let record = read_record(&committed[0])?;
    let bytes = read_nofollow(out)?;
    let verified = verify_pair(&basename, &record, &bytes, config)?;
    sync_parent(&parent)?;
    lock_shared(&transaction_lock)?;
    Ok(CommittedRunnerLock {
        _transaction_lock: transaction_lock,
        verified,
    })
}

fn recover_commit_state(
    parent: &Path,
    basename: &str,
    config: &ProvisionedRunnerConfig,
) -> Result<(), RunnerError> {
    let prefix = format!(".{basename}.");
    let mut incomplete = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".incomplete") {
            incomplete.push(entry.path());
        }
    }
    if !incomplete.is_empty() {
        quarantine_if_present(parent, &parent.join(basename))?;
        for path in incomplete {
            quarantine_if_present(parent, &path)?;
        }
        sync_parent(parent)?;
    }

    let committed = committed_records(parent, basename)?;
    if committed.len() > 1 {
        quarantine_if_present(parent, &parent.join(basename))?;
        for path in committed {
            quarantine_if_present(parent, &path)?;
        }
        sync_parent(parent)?;
        return Err(publication("multiple committed records were quarantined"));
    }
    if let Some(path) = committed.first() {
        let normal = parent.join(basename);
        if !normal.is_file() {
            quarantine_if_present(parent, path)?;
            sync_parent(parent)?;
            return Err(publication("orphan committed record was quarantined"));
        }
        let result = read_record(path).and_then(|record| {
            let bytes = read_nofollow(&normal)?;
            verify_pair(basename, &record, &bytes, config).map(|_| ())
        });
        if result.is_err() {
            quarantine_if_present(parent, &normal)?;
            quarantine_if_present(parent, path)?;
            sync_parent(parent)?;
            return Err(publication("mismatched committed pair was quarantined"));
        }
        sync_parent(parent)?;
    }
    Ok(())
}

fn verify_pair(
    basename: &str,
    record: &TransactionRecordV1,
    bytes: &[u8],
    config: &ProvisionedRunnerConfig,
) -> Result<VerifiedRunnerLock, RunnerError> {
    let verified = verify_runner_lock_bytes_with(bytes, config)?;
    let length: u64 = bytes
        .len()
        .try_into()
        .map_err(|_| publication("lock is too large"))?;
    if record.schema != "feathermark.runner-lock-commit.v1"
        || record.state != RecordState::Committed
        || record.output_basename != basename
        || record.matrix_run_id != verified.matrix_run_id
        || record.lock_length != length
        || record.lock_sha256 != verified.lock_sha256
    {
        return Err(publication("committed record does not bind the lock bytes"));
    }
    Ok(verified)
}

fn record(
    basename: &str,
    verified: &VerifiedRunnerLock,
    bytes: &[u8],
    state: RecordState,
) -> TransactionRecordV1 {
    TransactionRecordV1 {
        schema: "feathermark.runner-lock-commit.v1".into(),
        output_basename: basename.into(),
        matrix_run_id: verified.matrix_run_id,
        lock_length: bytes.len() as u64,
        lock_sha256: Sha256::digest(bytes).into(),
        state,
    }
}

fn write_record(file: &mut File, record: &TransactionRecordV1) -> Result<(), RunnerError> {
    let mut bytes = serde_json::to_vec_pretty(record)?;
    bytes.push(b'\n');
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_record(path: &Path) -> Result<TransactionRecordV1, RunnerError> {
    Ok(serde_json::from_slice(&read_nofollow(path)?)?)
}

fn read_nofollow(path: &Path) -> Result<Vec<u8>, RunnerError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn checked_parent(out: &Path) -> Result<(PathBuf, String), RunnerError> {
    let parent = out
        .parent()
        .ok_or_else(|| publication("output has no parent"))?;
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(publication(
            "output parent must be an existing non-symlink directory",
        ));
    }
    let basename = out
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| publication("output basename is not valid UTF-8"))?;
    Ok((parent.to_path_buf(), basename.into()))
}

fn open_transaction_lock(parent: &Path) -> Result<File, RunnerError> {
    let path = parent.join(TRANSACTION_LOCK);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.sync_all()?;
    sync_parent(parent)?;
    Ok(file)
}

fn create_exclusive(path: &Path, mode: u32) -> Result<File, RunnerError> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

fn sync_parent(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

fn committed_records(parent: &Path, basename: &str) -> Result<Vec<PathBuf>, RunnerError> {
    let prefix = format!(".{basename}.");
    let mut records = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".committed") {
            records.push(entry.path());
        }
    }
    records.sort();
    Ok(records)
}

fn quarantine_if_present(parent: &Path, path: &Path) -> Result<(), RunnerError> {
    if !path.exists() {
        return Ok(());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| publication("quarantine basename is invalid"))?;
    for counter in 0_u64.. {
        let destination = parent.join(format!(".{name}.quarantine.{counter}"));
        match no_replace_rename(path, &destination) {
            Ok(()) => return Ok(()),
            Err(RunnerError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn lock_exclusive(file: &File) -> Result<(), RunnerError> {
    fcntl_lock(file, libc::F_WRLCK as libc::c_short)
}

fn lock_shared(file: &File) -> Result<(), RunnerError> {
    fcntl_lock(file, libc::F_RDLCK as libc::c_short)
}

fn fcntl_lock(file: &File, lock_type: libc::c_short) -> Result<(), RunnerError> {
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = lock_type;
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    // SAFETY: `lock` is a valid `flock`, and the descriptor remains owned by `file`.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLKW, &lock) };
    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn no_replace_rename(from: &Path, to: &Path) -> Result<(), RunnerError> {
    let from = c_path(from)?;
    let to = c_path(to)?;
    // SAFETY: both arguments are valid NUL-terminated path strings.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn no_replace_rename(from: &Path, to: &Path) -> Result<(), RunnerError> {
    const RENAME_NOREPLACE: libc::c_uint = 1;
    let from = c_path(from)?;
    let to = c_path(to)?;
    // SAFETY: syscall arguments are valid path strings and documented constants.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn c_path(path: &Path) -> Result<CString, RunnerError> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| publication("filesystem path contains a NUL byte"))
}

fn publication(message: &str) -> RunnerError {
    RunnerError::Publication(message.into())
}
