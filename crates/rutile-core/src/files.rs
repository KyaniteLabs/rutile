//! The sole filesystem boundary for Markdown document contents.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::ffi::CString;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

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

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let xattr_entries = match &target {
            TargetClass::ExistingRegular { .. } => match read_xattrs(path) {
                Ok(entries) => entries,
                Err(error) => {
                    return SaveOutcome::NotCommitted {
                        reason: SaveError::MetadataCopyFailed(error),
                    };
                }
            },
            TargetClass::NewFile => Vec::new(),
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

        if let TargetClass::ExistingRegular { mode, gid } = target {
            #[cfg(unix)]
            {
                let permissions = std::fs::Permissions::from_mode(mode);
                if let Err(error) = fs::set_permissions(&temporary_path, permissions) {
                    return SaveOutcome::NotCommitted {
                        reason: SaveError::MetadataCopyFailed(error),
                    };
                }
                // Preserve the group of the existing file. The owner (uid) is
                // left unchanged by passing the POSIX "do not change" sentinel.
                let result =
                    unsafe { libc::fchown(temporary_file.as_raw_fd(), libc::uid_t::MAX, gid) };
                if result != 0 {
                    return SaveOutcome::NotCommitted {
                        reason: SaveError::MetadataCopyFailed(io::Error::last_os_error()),
                    };
                }
                // Preserve extended attributes after mode/group, before rename.
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                {
                    if let Err(error) = write_xattrs(&temporary_path, &xattr_entries) {
                        return SaveOutcome::NotCommitted {
                            reason: SaveError::MetadataCopyFailed(error),
                        };
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (mode, gid);
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
    ExistingRegular { mode: u32, gid: u32 },
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
                let gid = metadata.gid();
                Ok(TargetClass::ExistingRegular { mode, gid })
            }
            #[cfg(not(unix))]
            {
                Ok(TargetClass::ExistingRegular {
                    mode: 0o644,
                    gid: 0,
                })
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
            ".{}.rutile-{}-{sequence}.tmp",
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

// Raw extended-attribute syscalls used to preserve xattrs on existing files.
// macOS and Linux share the names but differ in arity, so each wrapper selects
// the matching libc signature per target. Other Unix platforms have no portable
// enumeration path, so the helpers are omitted entirely there (callers must
// fail closed instead of silently dropping attributes). All pointers and the
// `path`/`name` C strings are caller-provided and must remain valid for the call.
#[cfg(any(target_os = "macos", target_os = "linux"))]
unsafe fn listxattr_raw(path: *const libc::c_char, list: *mut libc::c_char, size: usize) -> isize {
    // macOS takes a trailing `options` argument; Linux does not.
    // SAFETY: the caller guarantees `path` is a valid NUL-terminated C string and
    // that `list`/`size` describe a writable buffer (`list` may be null when
    // `size` is 0 to probe the required length).
    unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::listxattr(path, list, size, 0)
        }
        #[cfg(target_os = "linux")]
        {
            libc::listxattr(path, list, size)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
unsafe fn getxattr_raw(
    path: *const libc::c_char,
    name: *const libc::c_char,
    value: *mut libc::c_void,
    size: usize,
) -> isize {
    // macOS takes trailing `position` (0) and `options` (0); Linux takes neither.
    // SAFETY: the caller guarantees `path` and `name` are valid NUL-terminated C
    // strings and that `value`/`size` describe a writable buffer (`value` may be
    // null when `size` is 0 to probe the required length).
    unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::getxattr(path, name, value, size, 0, 0)
        }
        #[cfg(target_os = "linux")]
        {
            libc::getxattr(path, name, value, size)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
unsafe fn setxattr_raw(
    path: *const libc::c_char,
    name: *const libc::c_char,
    value: *const libc::c_void,
    size: usize,
) -> i32 {
    // macOS takes trailing `position` (0) and `options` (0); Linux takes neither.
    // SAFETY: the caller guarantees `path` and `name` are valid NUL-terminated C
    // strings and that `value`/`size` describe a readable buffer (`value` may be
    // a dangling pointer when `size` is 0 for a zero-length attribute).
    unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::setxattr(path, name, value, size, 0, 0)
        }
        #[cfg(target_os = "linux")]
        {
            libc::setxattr(path, name, value, size, 0)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_XATTR_COUNT: usize = 128;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_XATTR_NAME_BYTES: usize = 64 * 1024;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_XATTR_VALUE_BYTES: usize = 8 * 1024 * 1024;

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct XattrEntry {
    name: Vec<u8>,
    value: Vec<u8>,
}

/// Reads the bounded set of extended attributes from `path`.
///
/// The attribute-name list is probed for its exact size, capped, then read; the
/// NUL-delimited names are parsed and each value is probed and read with
/// `getxattr`. Malformed/empty names, a missing terminating NUL, size drift
/// between probe and read, and any bound violation are reported as
/// [`io::ErrorKind::InvalidData`]; raw syscall failures surface the last OS error.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_xattrs(path: &Path) -> io::Result<Vec<XattrEntry>> {
    let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "path contains an interior NUL byte",
        )
    })?;

    // Probe the exact size of the NUL-delimited name list before allocating.
    let list_probe = unsafe { listxattr_raw(path_c.as_ptr(), std::ptr::null_mut(), 0) };
    if list_probe < 0 {
        return Err(io::Error::last_os_error());
    }
    if list_probe == 0 {
        return Ok(Vec::new());
    }
    let list_size = list_probe as usize;
    if list_size > MAX_XATTR_NAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extended-attribute name list exceeds the supported bound",
        ));
    }

    // Read the name list at the probed size; reject any drift (race).
    let mut names = vec![0u8; list_size];
    let list_actual = unsafe {
        listxattr_raw(
            path_c.as_ptr(),
            names.as_mut_ptr() as *mut libc::c_char,
            list_size,
        )
    };
    if list_actual < 0 {
        return Err(io::Error::last_os_error());
    }
    if list_actual as usize != list_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extended-attribute name list size changed during read",
        ));
    }
    // The list is a sequence of NUL-terminated names; require the final NUL.
    if names.pop() != Some(0u8) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extended-attribute name list is not NUL-terminated",
        ));
    }

    let mut entries: Vec<XattrEntry> = Vec::new();
    let mut aggregate_value_bytes: usize = 0;
    for name in names.split(|&byte| byte == 0) {
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended-attribute name is empty",
            ));
        }
        if entries.len() >= MAX_XATTR_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended-attribute count exceeds the supported bound",
            ));
        }
        let name_c = CString::new(name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "extended-attribute name contains an interior NUL byte",
            )
        })?;

        // Probe the exact value size, then enforce the aggregate value cap.
        let value_probe =
            unsafe { getxattr_raw(path_c.as_ptr(), name_c.as_ptr(), std::ptr::null_mut(), 0) };
        if value_probe < 0 {
            return Err(io::Error::last_os_error());
        }
        let value_size = value_probe as usize;
        aggregate_value_bytes = match aggregate_value_bytes.checked_add(value_size) {
            Some(total) => total,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "extended-attribute value aggregate size overflowed",
                ));
            }
        };
        if aggregate_value_bytes > MAX_XATTR_VALUE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended-attribute values exceed the supported bound",
            ));
        }

        // Read the value at the probed size; zero-length values are preserved.
        let value = if value_size == 0 {
            Vec::new()
        } else {
            let mut buf = vec![0u8; value_size];
            let value_actual = unsafe {
                getxattr_raw(
                    path_c.as_ptr(),
                    name_c.as_ptr(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    value_size,
                )
            };
            if value_actual < 0 {
                return Err(io::Error::last_os_error());
            }
            if value_actual as usize != value_size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "extended-attribute value size changed during read",
                ));
            }
            buf
        };

        entries.push(XattrEntry {
            name: name.to_vec(),
            value,
        });
    }

    Ok(entries)
}

/// Writes each captured extended attribute onto `path` via `setxattr`.
/// Zero-length values are preserved by passing a size of zero; interior-NUL
/// names and any syscall failure are reported as errors.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_xattrs(path: &Path, entries: &[XattrEntry]) -> io::Result<()> {
    let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "path contains an interior NUL byte",
        )
    })?;
    for entry in entries {
        let name_c = CString::new(entry.name.as_slice()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "extended-attribute name contains an interior NUL byte",
            )
        })?;
        // SAFETY: `path_c` and `name_c` are valid NUL-terminated C strings that
        // outlive the call, and `entry.value`/`len` describe a valid buffer; a
        // zero-length value yields a dangling pointer that setxattr never reads.
        let result = unsafe {
            setxattr_raw(
                path_c.as_ptr(),
                name_c.as_ptr(),
                entry.value.as_ptr() as *const libc::c_void,
                entry.value.len(),
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
