//! Autosave journal writer and crash-recovery loader (SPEC §9, Wave 1 1Q).
//!
//! An [`AutosaveStore`] owns a directory. Each autosave [`record`] writes the
//! document snapshot to a bare-named file *atomically* (same-directory temp,
//! fsync, rename, parent fsync) and then durably appends one
//! [`AutosaveEntryV1`] NDJSON record — describing that snapshot and carrying
//! its blake3 — to the journal. Recovery reads the journal, skips any
//! corrupt/truncated lines, and returns the document referenced by the
//! highest-sequence entry whose snapshot verifies (size + blake3).
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
//! ## Atomicity
//!
//! A snapshot is only referenced by a journal entry *after* its atomic rename
//! has committed, and the entry itself is appended durably; a crash between the
//! two leaves an orphan snapshot (harmless) rather than a journal entry
//! pointing at half-written bytes. Because recovery verifies blake3, any torn
//! snapshot is rejected regardless.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::files::{append_bytes_durable, write_bytes_atomic, write_snapshot_atomic};
use crate::session_contract::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, SessionError, SessionStateV1, decode_autosave_entry,
    decode_session_state, encode_autosave_entry, encode_session_state,
};
use crate::{Document, DocumentSnapshot, MAX_DOCUMENT_BYTES};

/// File name of the append-only autosave journal within the store directory.
pub const AUTOSAVE_JOURNAL_FILE: &str = "autosave.ndjson";
/// How many autosave snapshots (and their journal entries) are retained on
/// disk. After each successful [`record`](AutosaveStore::record) the store
/// prunes down to the newest `AUTOSAVE_RETENTION` snapshots — deleting older
/// snapshot files and compacting the journal — so autosave cannot grow the
/// directory (or the journal that recovery/`next_sequence` re-read each
/// startup) without bound. Sized to keep a few crash-recovery fallbacks.
pub const AUTOSAVE_RETENTION: usize = 8;
/// File name of the persisted session-restore state within the store directory.
pub const SESSION_STATE_FILE: &str = "session.json";

/// A recoverable document plus the journal entry that produced it.
pub struct RecoveredDocument {
    /// The winning journal entry (highest verifiable sequence).
    pub entry: AutosaveEntryV1,
    /// The document reconstructed from the verified snapshot.
    pub document: Document,
}

/// Errors from the autosave writer / recovery loader.
#[derive(Debug, Error)]
pub enum AutosaveError {
    #[error("autosave I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Record(#[from] SessionError),
}

/// Owns an autosave/session directory and mediates all reads and writes to it.
#[derive(Clone, Debug)]
pub struct AutosaveStore {
    dir: PathBuf,
}

impl AutosaveStore {
    /// Binds a store to `dir`. The directory is expected to exist; writes fail
    /// with [`AutosaveError::Io`] otherwise.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory this store manages.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The next sequence number to use: one past the highest valid entry, or
    /// `0` for an empty/absent journal.
    pub fn next_sequence(&self) -> Result<u64, AutosaveError> {
        let journal = match fs::read(self.dir.join(AUTOSAVE_JOURNAL_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let highest = complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .map(|entry| entry.sequence)
            .max();
        Ok(highest.map_or(0, |seq| seq.saturating_add(1)))
    }

    /// Writes `snapshot` atomically and appends its journal entry.
    pub fn record(
        &self,
        sequence: u64,
        snapshot: &DocumentSnapshot,
        document_path: Option<&str>,
        captured_at_unix_ms: u64,
    ) -> Result<AutosaveEntryV1, AutosaveError> {
        let snapshot_file = snapshot_file_name(sequence);
        let (snapshot_bytes, digest) = write_snapshot_atomic(&self.dir, &snapshot_file, snapshot)?;

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
        // Best-effort prune: the entry is already durably committed, so a prune
        // failure must not fail the autosave. Recovery tolerates orphan
        // snapshots and re-reads the (possibly un-compacted) journal safely.
        let _ = self.prune();
        Ok(entry)
    }

    /// Retains only the newest [`AUTOSAVE_RETENTION`] snapshots: rewrites the
    /// journal to the surviving entries and deletes the dropped snapshot files.
    ///
    /// Ordering is crash-safe: the compacted journal is written atomically
    /// *before* any snapshot is removed, so a crash can only ever leave an
    /// orphan snapshot (harmless) — never a journal entry pointing at a file we
    /// already deleted. The rewritten journal keeps ascending sequence order,
    /// and recovery still selects the highest verifiable sequence from it.
    fn prune(&self) -> Result<(), AutosaveError> {
        let journal_path = self.dir.join(AUTOSAVE_JOURNAL_FILE);
        let journal = match fs::read(&journal_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut entries: Vec<AutosaveEntryV1> = complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .collect();
        if entries.len() <= AUTOSAVE_RETENTION {
            return Ok(());
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
        for entry in &dropped {
            // `snapshot_file` is a validated bare name (decode enforced it).
            let _ = fs::remove_file(self.dir.join(&entry.snapshot_file));
        }
        Ok(())
    }

    /// Reads the journal and returns the highest-sequence entry whose snapshot
    /// verifies, or `None` when there is nothing recoverable. Corrupt journal
    /// lines and entries whose snapshot fails size/blake3 verification are
    /// skipped rather than aborting recovery.
    pub fn recover(&self) -> Result<Option<RecoveredDocument>, AutosaveError> {
        let journal = match fs::read(self.dir.join(AUTOSAVE_JOURNAL_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let mut entries: Vec<AutosaveEntryV1> = complete_lines(&journal)
            .filter_map(|line| decode_autosave_entry(line).ok())
            .collect();
        // Highest sequence first; the first one that verifies wins.
        entries.sort_by(|a, b| b.sequence.cmp(&a.sequence));

        for entry in entries {
            if let Some(document) = self.load_verified_snapshot(&entry) {
                return Ok(Some(RecoveredDocument { entry, document }));
            }
        }
        Ok(None)
    }

    /// Atomically persists session-restore `state`.
    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), AutosaveError> {
        let bytes = encode_session_state(state)?;
        write_bytes_atomic(&self.dir, SESSION_STATE_FILE, &bytes)?;
        Ok(())
    }

    /// Loads and re-validates persisted session state, or `None` when absent.
    /// A present-but-corrupt record surfaces as [`AutosaveError::Record`] so a
    /// caller can log it before falling back to defaults.
    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, AutosaveError> {
        let bytes = match fs::read(self.dir.join(SESSION_STATE_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(decode_session_state(&bytes)?))
    }

    /// Loads the snapshot for `entry`, verifying size and blake3. Returns
    /// `None` for any missing/oversized/torn/mismatched/non-UTF-8 snapshot so
    /// the caller can fall back to a lower sequence.
    fn load_verified_snapshot(&self, entry: &AutosaveEntryV1) -> Option<Document> {
        // `snapshot_file` is a validated bare name (decode enforced it), so the
        // join cannot escape the store directory.
        let path = self.dir.join(&entry.snapshot_file);
        let bytes = read_bounded(&path, MAX_DOCUMENT_BYTES)?;
        if bytes.len() as u64 != entry.snapshot_bytes {
            return None;
        }
        if blake3::hash(&bytes).to_hex().as_str() != entry.snapshot_blake3 {
            return None;
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        Document::new(text).ok()
    }
}

fn snapshot_file_name(sequence: u64) -> String {
    format!("autosave-{sequence}.md")
}

/// Reads at most `max` bytes from `path`; returns `None` on any I/O error or if
/// the file is larger than `max` (a snapshot that outgrew the document cap is
/// unrecoverable and must not be trusted).
fn read_bounded(path: &Path, max: usize) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    let cap = max.saturating_add(1);
    file.take(cap as u64).read_to_end(&mut bytes).ok()?;
    if bytes.len() > max {
        return None;
    }
    Some(bytes)
}

/// Yields each newline-terminated line of `journal` (with its trailing `\n`),
/// dropping a trailing partial line left by a torn final write.
fn complete_lines(journal: &[u8]) -> impl Iterator<Item = &[u8]> {
    journal
        .split_inclusive(|&byte| byte == b'\n')
        .filter(|line| line.ends_with(b"\n"))
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
            store
                .record(seq, &snapshot(&format!("v{seq}")), None, seq + 1)
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
            store
                .record(seq, &snapshot(&format!("payload-{seq}")), None, seq + 1)
                .unwrap();
        }

        let recovered = store.recover().unwrap().expect("something recoverable");
        assert_eq!(recovered.entry.sequence, (total - 1) as u64);
        assert_eq!(
            recovered.document.snapshot().to_string(),
            format!("payload-{}", total - 1)
        );
        // next_sequence keeps advancing past the highest surviving entry.
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
            let entry = store
                .record(seq, &snapshot(&format!("s{seq}")), None, seq + 1)
                .unwrap();
            latest_file = Some(entry.snapshot_file);
        }

        // Corrupt the highest surviving snapshot; recovery falls back one step
        // (both seqs remain among the survivors), proving compaction kept them.
        fs::write(dir.0.join(latest_file.unwrap()), b"XXXX").unwrap();
        let recovered = store.recover().unwrap().unwrap();
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

        let entry = store
            .record(0, &snapshot("hello \u{1fab6}"), Some("/notes/todo.md"), 1)
            .unwrap();
        assert_eq!(entry.sequence, 0);

        let recovered = store.recover().unwrap().expect("something to recover");
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
        assert!(store.recover().unwrap().is_none());
        assert_eq!(store.next_sequence().unwrap(), 0);
    }

    #[test]
    fn highest_sequence_wins() {
        let dir = TestDir::new("highest");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(0, &snapshot("first"), None, 1).unwrap();
        store.record(1, &snapshot("second"), None, 2).unwrap();
        store.record(2, &snapshot("third"), None, 3).unwrap();

        assert_eq!(store.next_sequence().unwrap(), 3);
        let recovered = store.recover().unwrap().unwrap();
        assert_eq!(recovered.entry.sequence, 2);
        assert_eq!(recovered.document.snapshot().to_string(), "third");
    }

    #[test]
    fn blake3_mismatch_is_rejected_and_falls_back() {
        let dir = TestDir::new("mismatch");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(0, &snapshot("good"), None, 1).unwrap();
        let latest = store.record(1, &snapshot("tampered"), None, 2).unwrap();

        // Corrupt the highest snapshot's bytes so its blake3 no longer matches.
        fs::write(dir.0.join(&latest.snapshot_file), b"XXXXXXXX").unwrap();

        let recovered = store.recover().unwrap().unwrap();
        // The tampered highest entry is rejected; recovery falls back to seq 0.
        assert_eq!(recovered.entry.sequence, 0);
        assert_eq!(recovered.document.snapshot().to_string(), "good");
    }

    #[test]
    fn single_tampered_entry_recovers_nothing() {
        let dir = TestDir::new("single-tampered");
        let store = AutosaveStore::new(dir.0.clone());
        let only = store.record(0, &snapshot("payload"), None, 1).unwrap();
        fs::write(dir.0.join(&only.snapshot_file), b"different").unwrap();

        assert!(store.recover().unwrap().is_none());
    }

    #[test]
    fn truncated_final_journal_line_is_ignored() {
        let dir = TestDir::new("truncated");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(0, &snapshot("committed"), None, 1).unwrap();

        // Append a torn (newline-less) partial record, as a crash mid-append
        // would leave behind.
        let journal = dir.0.join(AUTOSAVE_JOURNAL_FILE);
        let mut bytes = fs::read(&journal).unwrap();
        bytes.extend_from_slice(b"{\"schema\":\"feathermark.autosave.v1\",\"v\":1,\"seq");
        fs::write(&journal, &bytes).unwrap();

        let recovered = store.recover().unwrap().unwrap();
        assert_eq!(recovered.entry.sequence, 0);
        assert_eq!(recovered.document.snapshot().to_string(), "committed");
    }

    #[test]
    fn corrupt_journal_lines_are_skipped() {
        let dir = TestDir::new("corrupt-lines");
        let store = AutosaveStore::new(dir.0.clone());
        store.record(0, &snapshot("alive"), None, 1).unwrap();

        let journal = dir.0.join(AUTOSAVE_JOURNAL_FILE);
        let mut bytes = fs::read(&journal).unwrap();
        // A complete but garbage JSON line between valid records.
        bytes.extend_from_slice(b"not json at all\n");
        fs::write(&journal, &bytes).unwrap();

        let recovered = store.recover().unwrap().unwrap();
        assert_eq!(recovered.document.snapshot().to_string(), "alive");
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
