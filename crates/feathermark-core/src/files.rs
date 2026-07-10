//! The sole filesystem boundary for Markdown document contents.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

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
    #[error("injected failure before atomic rename")]
    InjectedBeforeRename,
}

pub trait FileService {
    fn load(&self, path: &Path, max: usize) -> Result<LoadedDocument, FileError>;

    fn save_atomic(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> Result<DiskVersion, FileError>;

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

    fn save_atomic(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> Result<DiskVersion, FileError> {
        let parent = normalized_parent(path);
        let file_name = path.file_name().ok_or(FileError::InvalidTarget)?;
        let (temporary_path, mut temporary_file) = create_same_directory_temp(parent, file_name)?;
        let mut cleanup = TempCleanup::new(temporary_path.clone());

        let mut writer = HashingWriter::new(&mut temporary_file);
        snapshot.write_to(&mut writer)?;
        writer.flush()?;
        let digest = writer.finalize();
        temporary_file.sync_all()?;

        if self.fault == SaveFault::BeforeRename {
            drop(temporary_file);
            return Err(FileError::InjectedBeforeRename);
        }

        drop(temporary_file);
        fs::rename(&temporary_path, path)?;
        cleanup.disarm();
        sync_parent_directory(parent)?;

        let metadata = fs::metadata(path)?;
        Ok(DiskVersion {
            digest,
            modified: metadata.modified()?,
            len: metadata.len(),
        })
    }
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

fn normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_same_directory_temp(
    parent: &Path,
    file_name: &std::ffi::OsStr,
) -> io::Result<(PathBuf, File)> {
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            ".{}.feathermark-{}-{sequence}.tmp",
            file_name.to_string_lossy(),
            std::process::id()
        );
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
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
