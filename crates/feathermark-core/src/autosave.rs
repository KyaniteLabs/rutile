//! Autosave journal writer and crash-recovery loader (SPEC §9, Wave 1 1C).
//!
//! An [`AutosaveStore`] owns a directory and serializes all access to it through
//! an advisory exclusive lock on `<store>/.autosave.lock`. Each autosave
//! [`record`] allocates the next monotonic sequence internally, writes the
//! document snapshot to a bare-named file *atomically* (same-directory temp,
//! fsync, rename, parent fsync), durably appends one [`AutosaveEntryV1`] NDJSON
//! record, compacts the journal to the newest [`AUTOSAVE_RETENTION`] snapshots,
//! and removes validated orphan snapshot files.
//!
//! Recovery reads the journal under the same exclusive lock, skips corrupt or
//! unverifiable entries with typed [`RejectionReason`]s, deletes orphan
//! snapshots, and returns a [`RecoveryReport`] describing the winner and every
//! rejected line.
//!
//! Session restore state ([`SessionStateV1`]) is persisted with an atomic
//! whole-file replace and re-validated on load.
//!
//! ## Path safety
//!
//! Every snapshot reference is a bare file name — the session contract makes a
//! path-traversing `snapshot_file` undecodable — so recovery only ever joins a
//! bare name onto the store directory. Nothing here follows an attacker-chosen
//! path.
//!
//! ## Atomicity and locking
//!
//! A snapshot is only referenced by a journal entry *after* its atomic rename
//! has committed, and the entry itself is appended durably; a crash between the
//! two leaves an orphan snapshot (harmless) rather than a journal entry
//! pointing at half-written bytes. Because recovery verifies blake3, any torn
//! snapshot is rejected regardless.
//!
//! The advisory lock is acquired with `fs2::try_lock_exclusive` and monotonic
//! retries for up to [`LOCK_TIMEOUT_MS`]. Timeout yields [`AutosaveError::AutosaveBusy`];
//! lock-file I/O yields [`AutosaveError::AutosaveUnavailable`]. The lock is held across
//! journal read, sequence allocation, temp write/rename, journal append/sync,
//! compaction, orphan scan, and report creation.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use thiserror::Error;

use crate::files::{append_bytes_durable, write_bytes_atomic, write_snapshot_atomic};
use crate::session_contract::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, SessionError, SessionStateV1, decode_autosave_entry,
    decode_session_state, encode_autosave_entry, encode_session_state,
};
use crate::{Document, DocumentSnapshot, MAX_DOCUMENT_BYTES};

/// File name of the append-only autosave journal within the store directory.
pub const AUTOSAVE_JOURNAL_FILE: &str = "autosave.ndjson";
/// File name of the advisory lock within the store directory.
pub const AUTOSAVE_LOCK_FILE: &str = ".autosave.lock";
/// How many autosave snapshots (and their journal entries) are retained on
/// disk. After each successful [`record`](AutosaveStore::record) the store
/// prunes down to the newest `AUTOSAVE_RETENTION` snapshots — deleting older
/// snapshot files and compacting the journal — so autosave cannot grow the
/// directory (or the journal that recovery re-reads each startup) without
/// bound. Sized to keep a few crash-recovery fallbacks.
pub const AUTOSAVE_RETENTION: usize = 8;
/// File name of the persisted session-restore state within the store directory.
pub const SESSION_STATE_FILE: &str = "session.json";
/// Maximum size of the autosave journal before recovery rejects it.
pub const MAX_JOURNAL_BYTES: usize = 1024 * 1024;
/// Maximum matching entries examined by the validated orphan snapshot scanner.
pub const MAX_ORPHAN_SCAN_ENTRIES: usize = 1024;
/// Monotonic retry budget for acquiring the advisory lock.
const LOCK_TIMEOUT_MS: u64 = 250;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);

/// A recoverable document plus the journal entry that produced it.
pub struct RecoveredDocument {
    /// The winning journal entry (highest verifiable sequence).
    pub entry: AutosaveEntryV1,
    /// The document reconstructed from the verified snapshot.
    pub document: Document,
}

impl std::fmt::Debug for RecoveredDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecoveredDocument")
            .field("entry", &self.entry)
            .field("revision", &self.document.revision())
            .field("len_bytes", &self.document.len_bytes())
            .finish()
    }
}

impl Clone for RecoveredDocument {
    fn clone(&self) -> Self {
        let text = self.document.snapshot().to_string();
        Self {
            entry: self.entry.clone(),
            document: Document::new(&text).expect("recovered snapshot must be within size bounds"),
        }
    }
}

impl PartialEq for RecoveredDocument {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry
            && self.document.snapshot().to_string() == other.document.snapshot().to_string()
    }
}

impl Eq for RecoveredDocument {}

/// The result of crash recovery: the highest verifiable document, if any, plus
/// every rejected journal entry with a typed reason.
#[derive(Debug)]
pub struct RecoveryReport {
    pub recovered: Option<RecoveredDocument>,
    pub rejected: Vec<RejectedEntry>,
}

/// One journal entry that recovery could not commit, with a typed reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedEntry {
    /// Sequence when the entry could be decoded; `None` for an unparseable line.
    pub sequence: Option<u64>,
    pub reason: RejectionReason,
}

/// Why a journal entry or its snapshot was rejected during recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    CorruptLine,
    UnsupportedVersion,
    InvalidSchema,
    InvalidMetadata(&'static str),
    SnapshotMissing,
    SnapshotOversized,
    SnapshotTampered,
    SnapshotInvalid,
}

/// Outcome of a successful autosave [`record`](AutosaveStore::record).
#[derive(Debug)]
pub struct AutosaveRecordOutcome {
    pub entry: AutosaveEntryV1,
    pub prune: PruneOutcome,
    pub orphan_gc: OrphanGcReport,
}

impl Clone for AutosaveRecordOutcome {
    fn clone(&self) -> Self {
        Self {
            entry: self.entry.clone(),
            prune: self.prune,
            // The reducer does not need the per-failure error payloads, so the
            // clone is intentionally lossy for orphan-gc failures. The original
            // outcome remains available to the shell that performed the effect.
            orphan_gc: OrphanGcReport {
                scanned: self.orphan_gc.scanned,
                removed: self.orphan_gc.removed,
                failures: Vec::new(),
            },
        }
    }
}

impl PartialEq for AutosaveRecordOutcome {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry && self.prune == other.prune
    }
}

impl Eq for AutosaveRecordOutcome {}

/// Counts produced by journal compaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PruneOutcome {
    pub retained: usize,
    pub dropped: usize,
}

/// Result of the validated orphan-snapshot scanner.
#[derive(Debug, Default)]
pub struct OrphanGcReport {
    pub scanned: usize,
    pub removed: usize,
    pub failures: Vec<OrphanFailure>,
}

/// One orphan snapshot file that could not be deleted.
#[derive(Debug)]
pub struct OrphanFailure {
    pub file_name: String,
    pub error: AutosaveError,
}

/// Errors from the autosave writer / recovery loader.
#[derive(Debug, Error)]
pub enum AutosaveError {
    #[error("autosave I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Record(#[from] SessionError),
    #[error("autosave store is busy")]
    AutosaveBusy,
    #[error("autosave store unavailable: {0}")]
    AutosaveUnavailable(#[source] io::Error),
    #[error("autosave sequence exhausted")]
    SequenceExhausted,
    #[error("autosave journal exceeds {maximum} bytes")]
    OversizeJournal { maximum: usize },
    #[error("session state exceeds {maximum} bytes")]
    OversizeSessionState { maximum: usize },
}

/// Owns an autosave/session directory and mediates all reads and writes to it.
#[derive(Clone, Debug)]
pub struct AutosaveStore {
    dir: PathBuf,
}

impl AutosaveStore {
    /// Binds a store to `dir`. The directory is expected to exist; writes fail
    /// with [`AutosaveError::AutosaveUnavailable`] or [`AutosaveError::Io`] otherwise.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory this store manages.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes `snapshot` atomically and appends its journal entry.
    ///
    /// The next sequence is allocated internally under the store's advisory
    /// lock; callers must not pass a sequence number.
    pub fn record(
        &self,
        snapshot: &DocumentSnapshot,
        document_path: Option<&str>,
        captured_at_unix_ms: u64,
    ) -> Result<AutosaveRecordOutcome, AutosaveError> {
        with_store_lock(&self.dir, || {
            let journal = read_journal_bounded(&self.dir)?.unwrap_or_default();
            let sequence = Self::next_sequence_from_journal(&journal)?;

            let snapshot_file = snapshot_file_name(sequence);
            let (snapshot_bytes, digest) =
                write_snapshot_atomic(&self.dir, &snapshot_file, snapshot)?;

            let entry = AutosaveEntryV1 {
                schema: AUTOSAVE_SCHEMA_V1.to_owned(),
                v: 1,
                sequence,
                captured_at_unix_ms,
                document_path: document_path.map(str::to_owned),
                document_revision: snapshot.revision,
                snapshot_file,
                snapshot_bytes,
                snapshot_blake3: digest.to_hex().to_string(),
            };
            let record = encode_autosave_entry(&entry)?;
            append_bytes_durable(&self.dir, AUTOSAVE_JOURNAL_FILE, &record)?;

            let prune = self.prune_locked()?;
            let referenced = self.referenced_snapshot_names()?;
            let orphan_gc = self.collect_orphans(&referenced)?;

            Ok(AutosaveRecordOutcome {
                entry,
                prune,
                orphan_gc,
            })
        })
    }

    /// Reads the journal under the advisory lock and returns a report
    /// containing the highest-sequence entry whose snapshot verifies, plus a
    /// typed reason for every rejected line or unverifiable snapshot.
    pub fn recover(&self) -> Result<RecoveryReport, AutosaveError> {
        with_store_lock(&self.dir, || {
            let journal = match read_journal_bounded(&self.dir)? {
                Some(bytes) => bytes,
                None => {
                    return Ok(RecoveryReport {
                        recovered: None,
                        rejected: Vec::new(),
                    });
                }
            };

            let (entries, mut rejected) = decode_entries_with_reasons(&journal);
            let referenced: HashSet<String> = entries
                .iter()
                .map(|entry| entry.snapshot_file.clone())
                .collect();
            let _orphan_gc = self.collect_orphans(&referenced)?;

            let mut entries = entries;
            entries.sort_by(|a, b| b.sequence.cmp(&a.sequence));

            for entry in entries {
                match self.verify_snapshot(&entry) {
                    Ok(document) => {
                        return Ok(RecoveryReport {
                            recovered: Some(RecoveredDocument { entry, document }),
                            rejected,
                        });
                    }
                    Err(reason) => rejected.push(RejectedEntry {
                        sequence: Some(entry.sequence),
                        reason,
                    }),
                }
            }

            Ok(RecoveryReport {
                recovered: None,
                rejected,
            })
        })
    }

    /// Atomically persists session-restore `state`.
    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), AutosaveError> {
        with_store_lock(&self.dir, || {
            let bytes = encode_session_state(state)?;
            write_bytes_atomic(&self.dir, SESSION_STATE_FILE, &bytes)?;
            Ok(())
        })
    }

    /// Loads and re-validates persisted session state, or `None` when absent.
    /// A present-but-corrupt record surfaces as [`AutosaveError::Record`] so a
    /// caller can log it before falling back to defaults. An oversized record
    /// returns [`AutosaveError::OversizeSessionState`] without full allocation.
    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, AutosaveError> {
        with_store_lock(&self.dir, || {
            let bytes = match read_session_state_bounded(&self.dir)? {
                Some(bytes) => bytes,
                None => return Ok(None),
            };
            Ok(Some(decode_session_state(&bytes)?))
        })
    }

    /// The next sequence number to use: one past the highest valid entry, or
    /// `0` for an empty/absent journal. Sequence overflow returns
    /// [`AutosaveError::SequenceExhausted`].
    #[allow(dead_code)]
    fn next_sequence(&self) -> Result<u64, AutosaveError> {
        with_store_lock(&self.dir, || {
            let journal = read_journal_bounded(&self.dir)?.unwrap_or_default();
            Self::next_sequence_from_journal(&journal)
        })
    }

    fn next_sequence_from_journal(journal: &[u8]) -> Result<u64, AutosaveError> {
        let highest = complete_lines(journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .map(|entry| entry.sequence)
            .max();
        match highest {
            Some(u64::MAX) => Err(AutosaveError::SequenceExhausted),
            Some(seq) => Ok(seq.saturating_add(1)),
            None => Ok(0),
        }
    }

    /// Retains only the newest [`AUTOSAVE_RETENTION`] snapshots: rewrites the
    /// journal to the surviving entries. Dropped snapshot files are removed by
    /// the validated orphan scanner after compaction.
    ///
    /// Ordering is crash-safe: the compacted journal is written atomically
    /// *before* any snapshot is removed, so a crash can only ever leave an
    /// orphan snapshot (harmless) — never a journal entry pointing at a file we
    /// already deleted. The rewritten journal keeps ascending sequence order,
    /// and recovery still selects the highest verifiable sequence from it.
    fn prune_locked(&self) -> Result<PruneOutcome, AutosaveError> {
        let journal = match read_journal_bounded(&self.dir)? {
            Some(bytes) => bytes,
            None => return Ok(PruneOutcome::default()),
        };
        let mut entries: Vec<AutosaveEntryV1> = complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .collect();
        if entries.len() <= AUTOSAVE_RETENTION {
            return Ok(PruneOutcome {
                retained: entries.len(),
                dropped: 0,
            });
        }
        // Newest first; keep the first `AUTOSAVE_RETENTION`, drop the rest.
        entries.sort_by(|a, b| b.sequence.cmp(&a.sequence));
        let dropped = entries.split_off(AUTOSAVE_RETENTION);
        // Rewrite the journal in ascending sequence order before deleting.
        entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));
        let mut compacted = Vec::new();
        for entry in &entries {
            compacted.extend_from_slice(&encode_autosave_entry(entry)?);
        }
        write_bytes_atomic(&self.dir, AUTOSAVE_JOURNAL_FILE, &compacted)?;
        Ok(PruneOutcome {
            retained: entries.len(),
            dropped: dropped.len(),
        })
    }

    /// Loads the snapshot for `entry`, verifying size and blake3. Returns a
    /// typed [`RejectionReason`] for any missing/oversized/torn/mismatched/non-UTF-8
    /// snapshot so the caller can report why the entry was rejected.
    fn verify_snapshot(&self, entry: &AutosaveEntryV1) -> Result<Document, RejectionReason> {
        // `snapshot_file` is a validated bare name (decode enforced it), so the
        // join cannot escape the store directory.
        let path = self.dir.join(&entry.snapshot_file);
        let bytes = read_snapshot_file(&path, MAX_DOCUMENT_BYTES)?;
        if bytes.len() as u64 != entry.snapshot_bytes {
            return Err(RejectionReason::SnapshotTampered);
        }
        if blake3::hash(&bytes).to_hex().as_str() != entry.snapshot_blake3 {
            return Err(RejectionReason::SnapshotTampered);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| RejectionReason::SnapshotInvalid)?;
        Document::new(text).map_err(|_| RejectionReason::SnapshotInvalid)
    }

    /// Snapshot file names currently referenced by the journal.
    fn referenced_snapshot_names(&self) -> Result<HashSet<String>, AutosaveError> {
        let journal = read_journal_bounded(&self.dir)?.unwrap_or_default();
        Ok(complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .map(|entry| entry.snapshot_file)
            .collect())
    }

    /// Scans at most [`MAX_ORPHAN_SCAN_ENTRIES`] directory entries matching
    /// `autosave-[0-9]+.md`, deletes those not in `referenced`, and retains
    /// typed deletion failures for retry.
    fn collect_orphans(
        &self,
        referenced: &HashSet<String>,
    ) -> Result<OrphanGcReport, AutosaveError> {
        let mut matches: Vec<(String, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(AutosaveError::Io)? {
            if matches.len() >= MAX_ORPHAN_SCAN_ENTRIES {
                break;
            }
            let entry = entry.map_err(AutosaveError::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if is_autosave_snapshot_name(name) {
                matches.push((name.to_owned(), entry.path()));
            }
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0));

        let scanned = matches.len();
        let mut removed = 0;
        let mut failures = Vec::new();
        for (name, path) in matches {
            if referenced.contains(&name) {
                continue;
            }
            if let Err(error) = fs::remove_file(&path) {
                failures.push(OrphanFailure {
                    file_name: name,
                    error: AutosaveError::Io(error),
                });
            } else {
                removed += 1;
            }
        }
        Ok(OrphanGcReport {
            scanned,
            removed,
            failures,
        })
    }
}

fn snapshot_file_name(sequence: u64) -> String {
    format!("autosave-{sequence}.md")
}

fn is_autosave_snapshot_name(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix("autosave-")
        .and_then(|suffix| suffix.strip_suffix(".md"))
    else {
        return false;
    };
    !body.is_empty() && body.bytes().all(|byte| byte.is_ascii_digit())
}

/// Reads a snapshot file bounded at `max` bytes, distinguishing a missing
/// snapshot from one that outgrew the document cap. A TOCTOU race that grows
/// the file between the metadata check and the read is caught by the same cap.
fn read_snapshot_file(path: &Path, max: usize) -> Result<Vec<u8>, RejectionReason> {
    let metadata = fs::metadata(path).map_err(|_| RejectionReason::SnapshotMissing)?;
    if metadata.len() > max as u64 {
        return Err(RejectionReason::SnapshotOversized);
    }
    let file = fs::File::open(path).map_err(|_| RejectionReason::SnapshotMissing)?;
    let mut bytes = Vec::new();
    let cap = max.saturating_add(1);
    file.take(cap as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| RejectionReason::SnapshotMissing)?;
    if bytes.len() > max {
        return Err(RejectionReason::SnapshotOversized);
    }
    Ok(bytes)
}

/// Reads the journal bounded at [`MAX_JOURNAL_BYTES`] + 1 so an oversize
/// journal is rejected without a full allocation.
fn read_journal_bounded(dir: &Path) -> Result<Option<Vec<u8>>, AutosaveError> {
    let path = dir.join(AUTOSAVE_JOURNAL_FILE);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    let cap = MAX_JOURNAL_BYTES.saturating_add(1);
    file.take(cap as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(AutosaveError::OversizeJournal {
            maximum: MAX_JOURNAL_BYTES,
        });
    }
    Ok(Some(bytes))
}

/// Reads the session-state file bounded at [`MAX_SESSION_STATE_BYTES`] + 1 so
/// an oversize record is rejected without a full allocation.
fn read_session_state_bounded(dir: &Path) -> Result<Option<Vec<u8>>, AutosaveError> {
    use crate::session_contract::MAX_SESSION_STATE_BYTES;

    let path = dir.join(SESSION_STATE_FILE);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    let cap = MAX_SESSION_STATE_BYTES.saturating_add(1);
    file.take(cap as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SESSION_STATE_BYTES {
        return Err(AutosaveError::OversizeSessionState {
            maximum: MAX_SESSION_STATE_BYTES,
        });
    }
    Ok(Some(bytes))
}

/// Decodes all complete journal lines, partitioning valid entries from rejected
/// lines with typed reasons.
fn decode_entries_with_reasons(journal: &[u8]) -> (Vec<AutosaveEntryV1>, Vec<RejectedEntry>) {
    let mut entries = Vec::new();
    let mut rejected = Vec::new();
    for line in complete_lines(journal) {
        match decode_autosave_entry(line) {
            Ok(entry) => entries.push(entry),
            Err(error) => rejected.push(RejectedEntry {
                sequence: None,
                reason: rejection_reason_from_session_error(&error),
            }),
        }
    }
    (entries, rejected)
}

fn rejection_reason_from_session_error(error: &SessionError) -> RejectionReason {
    match error {
        SessionError::TooLarge { .. }
        | SessionError::InvalidFraming
        | SessionError::InvalidJson(_) => RejectionReason::CorruptLine,
        SessionError::UnsupportedVersion => RejectionReason::UnsupportedVersion,
        SessionError::InvalidSchema => RejectionReason::InvalidSchema,
        SessionError::InvalidMetadata(message) => RejectionReason::InvalidMetadata(message),
    }
}

/// Yields each newline-terminated line of `journal` (with its trailing `\n`),
/// dropping a trailing partial line left by a torn final write.
fn complete_lines(journal: &[u8]) -> impl Iterator<Item = &[u8]> {
    journal
        .split_inclusive(|&byte| byte == b'\n')
        .filter(|line| line.ends_with(b"\n"))
}

/// Holds the advisory lock file open. Dropping the lock releases it.
struct StoreLock {
    #[allow(dead_code)]
    file: fs::File,
}

fn acquire_store_lock(dir: &Path) -> Result<StoreLock, AutosaveError> {
    let lock_path = dir.join(AUTOSAVE_LOCK_FILE);
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(AutosaveError::AutosaveUnavailable)?;

    let start = Instant::now();
    let timeout = Duration::from_millis(LOCK_TIMEOUT_MS);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(StoreLock { file }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if start.elapsed() >= timeout {
                    return Err(AutosaveError::AutosaveBusy);
                }
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(AutosaveError::AutosaveUnavailable(error)),
        }
    }
}

fn with_store_lock<T>(
    dir: &Path,
    operation: impl FnOnce() -> Result<T, AutosaveError>,
) -> Result<T, AutosaveError> {
    let _lock = acquire_store_lock(dir)?;
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_contract::SESSION_SCHEMA_V1;
    use std::path::PathBuf;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "feathermark-autosave-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot(text: &str) -> DocumentSnapshot {
        Document::new(text).unwrap().snapshot()
    }

    fn snapshot_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("autosave-") && name.ends_with(".md"))
            })
            .collect()
    }

    fn journal_entries(dir: &Path) -> Vec<AutosaveEntryV1> {
        let journal = fs::read(dir.join(AUTOSAVE_JOURNAL_FILE)).unwrap();
        complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .collect()
    }

    #[test]
    fn prune_retains_only_the_newest_snapshots() {
        let dir = TestDir::new("prune-retain");
        let store = AutosaveStore::new(dir.0.clone());
        let total = AUTOSAVE_RETENTION + 5;
        for seq in 0..total as u64 {
            let _ = store
                .record(&snapshot(&format!("v{seq}")), None, seq + 1)
                .unwrap();
        }

        // Only the newest AUTOSAVE_RETENTION snapshot files survive on disk.
        assert_eq!(snapshot_files(&dir.0).len(), AUTOSAVE_RETENTION);

        // The compacted journal holds exactly the surviving entries, in order.
        let sequences: Vec<u64> = journal_entries(&dir.0)
            .iter()
            .map(|entry| entry.sequence)
            .collect();
        let expected: Vec<u64> = ((total - AUTOSAVE_RETENTION) as u64..total as u64).collect();
        assert_eq!(sequences, expected);
    }

    #[test]
    fn recover_after_pruning_returns_the_latest() {
        let dir = TestDir::new("prune-recover");
        let store = AutosaveStore::new(dir.0.clone());
        let total = AUTOSAVE_RETENTION + 3;
        for seq in 0..total as u64 {
            let _ = store
                .record(&snapshot(&format!("payload-{seq}")), None, seq + 1)
                .unwrap();
        }

        let recovered = store
            .recover()
            .unwrap()
            .recovered
            .expect("something recoverable");
        assert_eq!(recovered.entry.sequence, (total - 1) as u64);
        assert_eq!(
            recovered.document.snapshot().to_string(),
            format!("payload-{}", total - 1)
        );
        assert_eq!(store.next_sequence().unwrap(), total as u64);
    }

    #[test]
    fn pruned_journal_preserves_survivors_and_highest_verifiable_wins() {
        // Compaction preserves the surviving entries; the highest verifiable
        // sequence still wins, falling back when the newest snapshot is torn.
        let dir = TestDir::new("prune-fallback");
        let store = AutosaveStore::new(dir.0.clone());
        let total = AUTOSAVE_RETENTION + 2;
        let mut latest_file = None;
        for seq in 0..total as u64 {
            let outcome = store
                .record(&snapshot(&format!("s{seq}")), None, seq + 1)
                .unwrap();
            latest_file = Some(outcome.entry.snapshot_file);
        }

        // Corrupt the highest surviving snapshot; recovery falls back one step
        // (both seqs remain among the survivors), proving compaction kept them.
        fs::write(dir.0.join(latest_file.unwrap()), b"XXXX").unwrap();
        let recovered = store.recover().unwrap().recovered.unwrap();
        assert_eq!(recovered.entry.sequence, (total - 2) as u64);
        assert_eq!(
            recovered.document.snapshot().to_string(),
            format!("s{}", total - 2)
        );
    }

    #[test]
    fn record_then_recover_round_trips() {
        let dir = TestDir::new("round-trip");
        let store = AutosaveStore::new(dir.0.clone());

        let outcome = store
            .record(&snapshot("hello \u{1fab6}"), Some("/notes/todo.md"), 1)
            .unwrap();
        assert_eq!(outcome.entry.sequence, 0);

        let recovered = store
            .recover()
            .unwrap()
            .recovered
            .expect("something to recover");
        assert_eq!(recovered.entry.sequence, 0);
        assert_eq!(recovered.document.snapshot().to_string(), "hello \u{1fab6}");
        assert_eq!(
            recovered.entry.document_path.as_deref(),
            Some("/notes/todo.md")
        );
    }

    #[test]
    fn empty_or_absent_journal_recovers_nothing() {
        let dir = TestDir::new("absent");
        let store = AutosaveStore::new(dir.0.clone());
        assert!(store.recover().unwrap().recovered.is_none());
        assert_eq!(store.next_sequence().unwrap(), 0);
    }

    #[test]
    fn highest_sequence_wins() {
        let dir = TestDir::new("highest");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(&snapshot("first"), None, 1).unwrap();
        store.record(&snapshot("second"), None, 2).unwrap();
        store.record(&snapshot("third"), None, 3).unwrap();

        assert_eq!(store.next_sequence().unwrap(), 3);
        let recovered = store.recover().unwrap().recovered.unwrap();
        assert_eq!(recovered.entry.sequence, 2);
        assert_eq!(recovered.document.snapshot().to_string(), "third");
    }

    #[test]
    fn blake3_mismatch_is_rejected_and_falls_back() {
        let dir = TestDir::new("mismatch");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(&snapshot("good"), None, 1).unwrap();
        let latest = store.record(&snapshot("tampered"), None, 2).unwrap();

        // Corrupt the highest snapshot's bytes so its blake3 no longer matches.
        fs::write(dir.0.join(&latest.entry.snapshot_file), b"XXXXXXXX").unwrap();

        let report = store.recover().unwrap();
        let recovered = report.recovered.unwrap();
        // The tampered highest entry is rejected; recovery falls back to seq 0.
        assert_eq!(recovered.entry.sequence, 0);
        assert_eq!(recovered.document.snapshot().to_string(), "good");
        assert!(
            report
                .rejected
                .iter()
                .any(|r| r.sequence == Some(1) && r.reason == RejectionReason::SnapshotTampered)
        );
    }

    #[test]
    fn single_tampered_entry_recovers_nothing() {
        let dir = TestDir::new("single-tampered");
        let store = AutosaveStore::new(dir.0.clone());
        let only = store.record(&snapshot("payload"), None, 1).unwrap();
        fs::write(dir.0.join(&only.entry.snapshot_file), b"different").unwrap();

        assert!(store.recover().unwrap().recovered.is_none());
    }

    #[test]
    fn truncated_final_journal_line_is_ignored() {
        let dir = TestDir::new("truncated");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(&snapshot("committed"), None, 1).unwrap();

        // Append a torn (newline-less) partial record, as a crash mid-append
        // would leave behind.
        let journal = dir.0.join(AUTOSAVE_JOURNAL_FILE);
        let mut bytes = fs::read(&journal).unwrap();
        bytes.extend_from_slice(b"{\"schema\":\"feathermark.autosave.v1\",\"v\":1,\"seq");
        fs::write(&journal, &bytes).unwrap();

        let recovered = store.recover().unwrap().recovered.unwrap();
        assert_eq!(recovered.entry.sequence, 0);
        assert_eq!(recovered.document.snapshot().to_string(), "committed");
    }

    #[test]
    fn corrupt_journal_lines_are_skipped() {
        let dir = TestDir::new("corrupt-lines");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(&snapshot("alive"), None, 1).unwrap();

        let journal = dir.0.join(AUTOSAVE_JOURNAL_FILE);
        let mut bytes = fs::read(&journal).unwrap();
        // A complete but garbage JSON line between valid records.
        bytes.extend_from_slice(b"not json at all\n");
        fs::write(&journal, &bytes).unwrap();

        let report = store.recover().unwrap();
        let recovered = report.recovered.unwrap();
        assert_eq!(recovered.document.snapshot().to_string(), "alive");
        assert!(
            report
                .rejected
                .iter()
                .any(|r| r.sequence.is_none() && r.reason == RejectionReason::CorruptLine)
        );
    }

    #[test]
    fn session_state_round_trips() {
        let dir = TestDir::new("session");
        let store = AutosaveStore::new(dir.0.clone());
        assert!(store.load_session_state().unwrap().is_none());

        let state = SessionStateV1 {
            schema: SESSION_SCHEMA_V1.to_owned(),
            v: 1,
            saved_at_unix_ms: 42,
            last_file: Some("/notes/todo.md".to_owned()),
            selection: None,
            top_visible_byte: Some(3),
            window: None,
            recent_files: vec!["/notes/todo.md".to_owned()],
        };
        store.save_session_state(&state).unwrap();
        assert_eq!(store.load_session_state().unwrap(), Some(state));
    }

    #[test]
    fn corrupt_session_state_surfaces_an_error() {
        let dir = TestDir::new("session-corrupt");
        let store = AutosaveStore::new(dir.0.clone());
        fs::write(dir.0.join(SESSION_STATE_FILE), b"{not valid}\n").unwrap();
        assert!(matches!(
            store.load_session_state(),
            Err(AutosaveError::Record(_))
        ));
    }
}
