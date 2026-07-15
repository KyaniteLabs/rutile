//! The sole filesystem boundary for Markdown document contents.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::{Document, DocumentSnapshot, MAX_DOCUMENT_BYTES};

const UTF8_BOM: &[u8; 3] = b"\xef\xbb\xbf";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiskVersion {
    pub digest: blake3::Hash,
    pub modified: SystemTime,
    pub len: u64,
}

pub struct LoadedDocument {
    pub document: Document,
    pub disk: DiskVersion,
}

impl std::fmt::Debug for LoadedDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedDocument")
            .field("revision", &self.document.revision())
            .field("len_bytes", &self.document.len_bytes())
            .field("disk", &self.disk)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalResolution {
    ReloadDisk,
    KeepBuffer,
    SaveBufferAs(PathBuf),
}

#[derive(Debug)]
pub enum ExternalChange {
    Unchanged,
    Reloaded(LoadedDocument),
    Conflict { disk: DiskVersion },
}

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("file I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("document is not valid UTF-8")]
    InvalidUtf8,
    #[error("document is larger than {max} bytes")]
    TooLarge { max: usize },
    #[error("target path does not name a file")]
    InvalidTarget,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("file I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("target path does not name a file")]
    InvalidTarget,
    #[error("target is not a regular file")]
    NotARegularFile,
    #[error("target is a symlink")]
    Symlink,
    #[error("target has multiple hard links")]
    HardLinked,
    #[error("target is owned by a different user")]
    OwnerMismatch,
    #[error("failed to copy file metadata to temporary file")]
    MetadataCopyFailed(#[source] io::Error),
    #[error("injected failure before atomic rename")]
    InjectedBeforeRename,
}

#[derive(Debug, thiserror::Error)]
pub enum DurabilityError {
    #[error("file I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("injected failure after atomic rename")]
    InjectedAfterRename,
}

#[derive(Debug)]
pub enum SaveOutcome {
    NotCommitted {
        reason: SaveError,
    },
    Committed {
        disk: DiskVersion,
    },
    CommittedDurabilityUnknown {
        disk: DiskVersion,
        reason: DurabilityError,
    },
}

pub trait FileService {
    fn load(&self, path: &Path, max: usize) -> Result<LoadedDocument, FileError>;

    fn save_atomic(&self, path: &Path, snapshot: &DocumentSnapshot) -> SaveOutcome;

    fn inspect_external_change(
        &self,
        path: &Path,
        saved: &DiskVersion,
        dirty: bool,
        max: usize,
    ) -> Result<ExternalChange, FileError> {
        let loaded = self.load(path, max)?;
        if loaded.disk == *saved {
            return Ok(ExternalChange::Unchanged);
        }
        if dirty {
            return Ok(ExternalChange::Conflict { disk: loaded.disk });
        }
        Ok(ExternalChange::Reloaded(loaded))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SaveFault {
    #[default]
    None,
    BeforeRename,
    AfterRename,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileService {
    fault: SaveFault,
}

impl LocalFileService {
    pub const fn new() -> Self {
        Self {
            fault: SaveFault::None,
        }
    }

    /// Constructs a deterministic fault-injection service for atomic-save tests.
    pub const fn with_fault(fault: SaveFault) -> Self {
        Self { fault }
    }
}

impl FileService for LocalFileService {
    fn load(&self, path: &Path, max: usize) -> Result<LoadedDocument, FileError> {
        let max = max.min(MAX_DOCUMENT_BYTES);
        let mut file = File::open(path)?;
        let read_cap = max.saturating_add(UTF8_BOM.len()).saturating_add(1);
        let mut bytes = Vec::with_capacity(read_cap.min(64 * 1024));
        Read::by_ref(&mut file)
            .take(u64::try_from(read_cap).unwrap_or(u64::MAX))
            .read_to_end(&mut bytes)?;

        let source = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
        if source.len() > max {
            return Err(FileError::TooLarge { max });
        }
        let text = std::str::from_utf8(source).map_err(|_| FileError::InvalidUtf8)?;
        let document = Document::new(text).map_err(|_| FileError::TooLarge { max })?;
        let metadata = file.metadata()?;
        let disk = DiskVersion {
            digest: blake3::hash(&bytes),
            modified: metadata.modified()?,
            len: metadata.len(),
        };

        Ok(LoadedDocument { document, disk })
    }

    fn save_atomic(&self, path: &Path, snapshot: &DocumentSnapshot) -> SaveOutcome {
        let parent = normalized_parent(path);
        let file_name = match path.file_name() {
            Some(name) => name,
            None => {
                return SaveOutcome::NotCommitted {
                    reason: SaveError::InvalidTarget,
                };
            }
        };

        let target = match classify_target(path) {
            Ok(target) => target,
            Err(reason) => return SaveOutcome::NotCommitted { reason },
        };

        let (temporary_path, mut temporary_file) =
            match create_temporary_file(parent, file_name, Some(0o600)) {
                Ok(pair) => pair,
                Err(error) => {
                    return SaveOutcome::NotCommitted {
                        reason: SaveError::Io(error),
                    };
                }
            };
        let mut cleanup = TempCleanup::new(temporary_path.clone());

        let mut writer = HashingWriter::new(&mut temporary_file);
        if let Err(error) = snapshot.write_to(&mut writer) {
            return SaveOutcome::NotCommitted {
                reason: SaveError::Io(error),
            };
        }
        if let Err(error) = writer.flush() {
            return SaveOutcome::NotCommitted {
                reason: SaveError::Io(error),
            };
        }
        let digest = writer.finalize();
        if let Err(error) = temporary_file.sync_all() {
            return SaveOutcome::NotCommitted {
                reason: SaveError::Io(error),
            };
        }

        let temp_metadata = match fs::metadata(&temporary_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                return SaveOutcome::NotCommitted {
                    reason: SaveError::Io(error),
                };
            }
        };

        if let TargetClass::ExistingRegular { mode } = target {
            #[cfg(unix)]
            {
                let permissions = std::fs::Permissions::from_mode(mode);
                if let Err(error) = fs::set_permissions(&temporary_path, permissions) {
                    return SaveOutcome::NotCommitted {
                        reason: SaveError::MetadataCopyFailed(error),
                    };
                }
            }
            #[cfg(not(unix))]
            {
                let _ = mode;
            }
        }

        if self.fault == SaveFault::BeforeRename {
            return SaveOutcome::NotCommitted {
                reason: SaveError::InjectedBeforeRename,
            };
        }

        if let Err(error) = fs::rename(&temporary_path, path) {
            return SaveOutcome::NotCommitted {
                reason: SaveError::Io(error),
            };
        }
        cleanup.disarm();

        let fallback_disk = DiskVersion {
            digest,
            modified: temp_metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            len: temp_metadata.len(),
        };

        if self.fault == SaveFault::AfterRename {
            return SaveOutcome::CommittedDurabilityUnknown {
                disk: fallback_disk,
                reason: DurabilityError::InjectedAfterRename,
            };
        }

        if let Err(error) = sync_parent_directory(parent) {
            return SaveOutcome::CommittedDurabilityUnknown {
                disk: fallback_disk,
                reason: DurabilityError::Io(error),
            };
        }

        match fs::metadata(path) {
            Ok(metadata) => match disk_version(digest, &metadata) {
                Ok(disk) => SaveOutcome::Committed { disk },
                Err(error) => SaveOutcome::CommittedDurabilityUnknown {
                    disk: fallback_disk,
                    reason: DurabilityError::Io(error),
                },
            },
            Err(error) => SaveOutcome::CommittedDurabilityUnknown {
                disk: fallback_disk,
                reason: DurabilityError::Io(error),
            },
        }
    }
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `libc::geteuid` has no preconditions and is thread-safe.
    unsafe { libc::geteuid() }
}

enum TargetClass {
    NewFile,
    ExistingRegular { mode: u32 },
}

fn classify_target(path: &Path) -> Result<TargetClass, SaveError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(SaveError::Symlink);
            }
            if !metadata.file_type().is_file() {
                return Err(SaveError::NotARegularFile);
            }

            #[cfg(unix)]
            {
                if metadata.nlink() > 1 {
                    return Err(SaveError::HardLinked);
                }
                if metadata.uid() != effective_uid() {
                    return Err(SaveError::OwnerMismatch);
                }
                let mode = metadata.mode();
                Ok(TargetClass::ExistingRegular { mode })
            }
            #[cfg(not(unix))]
            {
                Ok(TargetClass::ExistingRegular { mode: 0o644 })
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(TargetClass::NewFile),
        Err(error) => Err(SaveError::Io(error)),
    }
}

fn disk_version(digest: blake3::Hash, metadata: &fs::Metadata) -> io::Result<DiskVersion> {
    Ok(DiskVersion {
        digest,
        modified: metadata.modified()?,
        len: metadata.len(),
    })
}

fn create_temporary_file(
    parent: &Path,
    file_name: &OsStr,
    mode: Option<u32>,
) -> io::Result<(PathBuf, File)> {
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            ".{}.feathermark-{}-{sequence}.tmp",
            file_name.to_string_lossy(),
            std::process::id()
        );
        let path = parent.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique atomic-save temporary file",
    ))
}

/// Coalesces noisy native watcher notifications until one complete quiet period.
#[derive(Clone, Debug)]
pub struct ExternalChangeDebouncer {
    quiet_period: Duration,
    latest_event: Option<Instant>,
}

impl ExternalChangeDebouncer {
    pub const fn new(quiet_period: Duration) -> Self {
        Self {
            quiet_period,
            latest_event: None,
        }
    }

    pub fn observe(&mut self, now: Instant) {
        self.latest_event = Some(now);
    }

    pub fn take_ready(&mut self, now: Instant) -> bool {
        let Some(latest) = self.latest_event else {
            return false;
        };
        let ready = now
            .checked_duration_since(latest)
            .is_some_and(|elapsed| elapsed >= self.quiet_period);
        if ready {
            self.latest_event = None;
        }
        ready
    }
}

/// Atomically writes `snapshot` to `dir/file_name` (same-directory temp,
/// fsync, rename, parent fsync) and returns the committed length and digest.
/// `file_name` must be a bare name; callers pass one validated by the session
/// contract. Shared by the autosave writer.
pub(crate) fn write_snapshot_atomic(
    dir: &Path,
    file_name: &str,
    snapshot: &DocumentSnapshot,
) -> io::Result<(u64, blake3::Hash)> {
    let (temporary_path, mut temporary_file) =
        create_same_directory_temp(dir, OsStr::new(file_name))?;
    let mut cleanup = TempCleanup::new(temporary_path.clone());

    let mut writer = HashingWriter::new(&mut temporary_file);
    snapshot.write_to(&mut writer)?;
    writer.flush()?;
    let digest = writer.finalize();
    temporary_file.sync_all()?;
    drop(temporary_file);

    let final_path = dir.join(file_name);
    fs::rename(&temporary_path, &final_path)?;
    cleanup.disarm();
    sync_parent_directory(dir)?;

    let len = fs::metadata(&final_path)?.len();
    Ok((len, digest))
}

/// Atomically replaces `dir/file_name` with `bytes`. Shared by session-state
/// persistence.
pub(crate) fn write_bytes_atomic(dir: &Path, file_name: &str, bytes: &[u8]) -> io::Result<()> {
    let (temporary_path, mut temporary_file) =
        create_same_directory_temp(dir, OsStr::new(file_name))?;
    let mut cleanup = TempCleanup::new(temporary_path.clone());

    temporary_file.write_all(bytes)?;
    temporary_file.flush()?;
    temporary_file.sync_all()?;
    drop(temporary_file);

    fs::rename(&temporary_path, dir.join(file_name))?;
    cleanup.disarm();
    sync_parent_directory(dir)?;
    Ok(())
}

/// Durably appends `bytes` to the `dir/file_name` journal (create-if-missing,
/// fsync file, fsync parent). Shared by the autosave journal writer.
pub(crate) fn append_bytes_durable(dir: &Path, file_name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file_name))?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    sync_parent_directory(dir)?;
    Ok(())
}

fn normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_same_directory_temp(
    parent: &Path,
    file_name: &std::ffi::OsStr,
) -> io::Result<(PathBuf, File)> {
    create_temporary_file(parent, file_name, None)
}

struct HashingWriter<'file> {
    file: &'file mut File,
    hasher: blake3::Hasher,
}

impl<'file> HashingWriter<'file> {
    fn new(file: &'file mut File) -> Self {
        Self {
            file,
            hasher: blake3::Hasher::new(),
        }
    }

    fn finalize(self) -> blake3::Hash {
        self.hasher.finalize()
    }
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct TempCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}
