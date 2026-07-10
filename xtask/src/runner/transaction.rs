//! Crash-safe publication of the affirmative runner-lock pair.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
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
    AfterParentSwap,
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

struct ParentDirectory {
    file: File,
    expected_uid: u32,
    original_path: PathBuf,
}

impl ParentDirectory {
    fn open(out: &Path) -> Result<(Self, String), RunnerError> {
        let path = out
            .parent()
            .ok_or_else(|| publication("output has no parent"))?;
        let c_path = c_path(path)?;
        let fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.nlink() < 2 {
            return Err(publication("output parent is not a stable directory"));
        }
        let basename = out
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .ok_or_else(|| publication("output basename is not valid UTF-8"))?;
        #[cfg(not(test))]
        let expected_uid = 0;
        #[cfg(test)]
        let expected_uid = metadata.uid();
        Ok((
            Self {
                file,
                expected_uid,
                original_path: path.to_path_buf(),
            },
            basename.into(),
        ))
    }

    fn open_at(&self, name: &str, flags: i32, mode: u32) -> Result<File, RunnerError> {
        let name = c_name(name)?;
        let fd = unsafe { libc::openat(self.file.as_raw_fd(), name.as_ptr(), flags, mode) };
        if fd == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn create_exclusive(&self, name: &str, mode: u32) -> Result<File, RunnerError> {
        self.open_at(
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, RunnerError> {
        let mut file =
            self.open_at(name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(publication("durable-pair member is not one regular file"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn exists(&self, name: &str) -> Result<bool, RunnerError> {
        match self.open_at(name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0) {
            Ok(_) => Ok(true),
            Err(RunnerError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn entries(&self) -> Result<Vec<String>, RunnerError> {
        if unsafe { libc::lseek(self.file.as_raw_fd(), 0, libc::SEEK_SET) } == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        let duplicate = unsafe { libc::dup(self.file.as_raw_fd()) };
        if duplicate == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        let directory = unsafe { libc::fdopendir(duplicate) };
        if directory.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(std::io::Error::last_os_error().into());
        }
        let mut names = Vec::new();
        loop {
            unsafe { *errno_location() = 0 };
            let entry = unsafe { libc::readdir(directory) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                unsafe { libc::closedir(directory) };
                if error.raw_os_error() != Some(0) {
                    return Err(error.into());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            if name != "." && name != ".." {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn rename_noreplace(&self, from: &str, to: &str) -> Result<(), RunnerError> {
        no_replace_rename(self.file.as_raw_fd(), from, to)
    }

    fn remove(&self, name: &str) -> Result<(), RunnerError> {
        let name = c_name(name)?;
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) } == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    fn sync(&self) -> Result<(), RunnerError> {
        self.file.sync_all()?;
        Ok(())
    }

    fn inject_namespace_swap(&self) -> Result<(), RunnerError> {
        let retained = self.original_path.with_extension("retained");
        std::fs::rename(&self.original_path, &retained)?;
        std::fs::create_dir(&self.original_path)?;
        Ok(())
    }
}

pub(crate) struct CommittedRunnerLock {
    _parent: ParentDirectory,
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
    let (parent, basename) = ParentDirectory::open(out)?;
    if failure == FailurePoint::AfterParentSwap {
        parent.inject_namespace_swap()?;
    }
    let writer_lock = open_transaction_lock(&parent)?;
    lock_exclusive(&writer_lock)?;
    recover_commit_state(&parent, &basename, config)?;

    let run_hex = hex::encode(verified.matrix_run_id);
    let incomplete = format!(".{basename}.{run_hex}.incomplete");
    let committed = format!(".{basename}.{run_hex}.committed");
    let temp_name = format!(".{basename}.{run_hex}.lock.tmp");
    let mut record_file = parent.create_exclusive(&incomplete, 0o600)?;
    write_record(
        &mut record_file,
        &record(&basename, &verified, lock_bytes, RecordState::Incomplete),
    )?;
    parent.sync()?;
    injected_precommit(
        failure,
        FailurePoint::AfterIncompleteFsync,
        &parent,
        &basename,
    )?;

    let mut temp = parent.create_exclusive(&temp_name, 0o600)?;
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
    injected_precommit(failure, FailurePoint::AfterLockFsync, &parent, &basename)?;
    parent.rename_noreplace(&temp_name, &basename)?;
    injected_precommit(failure, FailurePoint::AfterLockRename, &parent, &basename)?;
    parent.sync()?;
    injected_precommit(
        failure,
        FailurePoint::AfterLockParentFsync,
        &parent,
        &basename,
    )?;

    write_record(
        &mut record_file,
        &record(&basename, &verified, lock_bytes, RecordState::Committed),
    )?;
    injected_precommit(
        failure,
        FailurePoint::AfterJournalRewriteFsync,
        &parent,
        &basename,
    )?;
    if failure == FailurePoint::BeforeCommittedRename {
        quarantine_if_present(&parent, &basename)?;
        parent.sync()?;
        return Err(publication(
            "injected failure before committed-record rename",
        ));
    }
    parent.rename_noreplace(&incomplete, &committed)?;
    if failure == FailurePoint::AfterCommittedRename {
        parent.rename_noreplace(&committed, &incomplete)?;
        quarantine_if_present(&parent, &basename)?;
        parent.sync()?;
        return Err(publication(
            "injected failure after committed-record rename",
        ));
    }
    if let Err(error) = parent.sync() {
        if parent.rename_noreplace(&committed, &incomplete).is_err()
            || quarantine_if_present(&parent, &basename).is_err()
            || parent.sync().is_err()
        {
            return Err(RunnerError::FilesystemContractLost);
        }
        return Err(error);
    }
    if failure == FailurePoint::AfterParentSwap {
        return Err(publication(
            "injected parent namespace swap after dirfd-bound publication",
        ));
    }
    if failure == FailurePoint::AfterCommittedParentFsync {
        return Ok(());
    }
    if failure != FailurePoint::Cleanup {
        let _ = parent.remove(&temp_name);
    }
    Ok(())
}

fn injected_precommit(
    selected: FailurePoint,
    point: FailurePoint,
    parent: &ParentDirectory,
    out: &str,
) -> Result<(), RunnerError> {
    if selected != point {
        return Ok(());
    }
    quarantine_if_present(parent, out)?;
    parent.sync()?;
    Err(publication(&format!("injected failure at {point:?}")))
}

pub(crate) fn open_committed_runner_lock_with(
    out: &Path,
    config: &ProvisionedRunnerConfig,
) -> Result<CommittedRunnerLock, RunnerError> {
    let (parent, basename) = ParentDirectory::open(out)?;
    let transaction_lock = open_transaction_lock(&parent)?;
    lock_exclusive(&transaction_lock)?;
    recover_commit_state(&parent, &basename, config)?;
    let committed = committed_records(&parent, &basename)?;
    let normal_exists = parent.exists(&basename)?;
    if committed.len() != 1 || !normal_exists {
        return Err(publication(&format!(
            "authoritative committed pair is absent or ambiguous (records={}, lock={normal_exists})",
            committed.len()
        )));
    }
    let record = read_record(&parent, &committed[0])?;
    let bytes = parent.read(&basename)?;
    let verified = verify_pair(&basename, &record, &bytes, config)?;
    parent.sync()?;
    lock_shared(&transaction_lock)?;
    Ok(CommittedRunnerLock {
        _parent: parent,
        _transaction_lock: transaction_lock,
        verified,
    })
}

fn recover_commit_state(
    parent: &ParentDirectory,
    basename: &str,
    config: &ProvisionedRunnerConfig,
) -> Result<(), RunnerError> {
    let prefix = format!(".{basename}.");
    let incomplete: Vec<_> = parent
        .entries()?
        .into_iter()
        .filter(|name| name.starts_with(&prefix) && name.ends_with(".incomplete"))
        .collect();
    if !incomplete.is_empty() {
        quarantine_if_present(parent, basename)?;
        for name in incomplete {
            quarantine_if_present(parent, &name)?;
        }
        parent.sync()?;
    }
    let committed = committed_records(parent, basename)?;
    if committed.len() > 1 {
        quarantine_if_present(parent, basename)?;
        for name in committed {
            quarantine_if_present(parent, &name)?;
        }
        parent.sync()?;
        return Err(publication("multiple committed records were quarantined"));
    }
    if let Some(name) = committed.first() {
        if !parent.exists(basename)? {
            quarantine_if_present(parent, name)?;
            parent.sync()?;
            return Err(publication("orphan committed record was quarantined"));
        }
        let result = read_record(parent, name).and_then(|record| {
            let bytes = parent.read(basename)?;
            verify_pair(basename, &record, &bytes, config).map(|_| ())
        });
        if result.is_err() {
            quarantine_if_present(parent, basename)?;
            quarantine_if_present(parent, name)?;
            parent.sync()?;
            return Err(publication("mismatched committed pair was quarantined"));
        }
        parent.sync()?;
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

fn read_record(parent: &ParentDirectory, name: &str) -> Result<TransactionRecordV1, RunnerError> {
    Ok(serde_json::from_slice(&parent.read(name)?)?)
}

fn open_transaction_lock(parent: &ParentDirectory) -> Result<File, RunnerError> {
    let file = parent.open_at(
        TRANSACTION_LOCK,
        libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0o600,
    )?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != parent.expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(publication(
            "transaction lock ownership/type/link-count/mode is invalid",
        ));
    }
    file.sync_all()?;
    parent.sync()?;
    Ok(file)
}

fn committed_records(parent: &ParentDirectory, basename: &str) -> Result<Vec<String>, RunnerError> {
    let prefix = format!(".{basename}.");
    Ok(parent
        .entries()?
        .into_iter()
        .filter(|name| name.starts_with(&prefix) && name.ends_with(".committed"))
        .collect())
}

fn quarantine_if_present(parent: &ParentDirectory, name: &str) -> Result<(), RunnerError> {
    if !parent.exists(name)? {
        return Ok(());
    }
    for counter in 0_u64.. {
        let destination = format!(".{name}.quarantine.{counter}");
        match parent.rename_noreplace(name, &destination) {
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
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLKW, &lock) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn no_replace_rename(dirfd: i32, from: &str, to: &str) -> Result<(), RunnerError> {
    let from = c_name(from)?;
    let to = c_name(to)?;
    if unsafe { libc::renameatx_np(dirfd, from.as_ptr(), dirfd, to.as_ptr(), libc::RENAME_EXCL) }
        == -1
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn no_replace_rename(dirfd: i32, from: &str, to: &str) -> Result<(), RunnerError> {
    const RENAME_NOREPLACE: libc::c_uint = 1;
    let from = c_name(from)?;
    let to = c_name(to)?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            dirfd,
            from.as_ptr(),
            dirfd,
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    } == -1
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn c_name(name: &str) -> Result<CString, RunnerError> {
    if name.is_empty() || name.contains('/') {
        return Err(publication("relative durable-pair name is invalid"));
    }
    CString::new(name).map_err(|_| publication("filesystem name contains NUL"))
}
#[cfg(target_os = "macos")]
unsafe fn errno_location() -> *mut i32 {
    unsafe { libc::__error() }
}
#[cfg(target_os = "linux")]
unsafe fn errno_location() -> *mut i32 {
    unsafe { libc::__errno_location() }
}
fn c_path(path: &Path) -> Result<CString, RunnerError> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| publication("filesystem path contains NUL"))
}
fn publication(message: &str) -> RunnerError {
    RunnerError::Publication(message.into())
}
