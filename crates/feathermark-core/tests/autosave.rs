use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use feathermark_core::{
    AutosaveError, AutosaveStore, Document, MAX_JOURNAL_BYTES, MAX_SESSION_STATE_BYTES,
    RecoveryReport, RejectionReason, SESSION_SCHEMA_V1, SessionStateV1,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "feathermark-autosave-integration-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snap(text: &str) -> feathermark_core::DocumentSnapshot {
    Document::new(text).unwrap().snapshot()
}

fn lock_file_path(dir: &Path) -> PathBuf {
    dir.join(feathermark_core::AUTOSAVE_LOCK_FILE)
}

#[test]
fn two_stores_on_same_directory_serialize_records() {
    let dir = TestDir::new("serialize");
    let store_a = AutosaveStore::new(dir.0.clone());
    let store_b = AutosaveStore::new(dir.0.clone());

    let handle = std::thread::spawn(move || {
        store_a.record(&snap("from-thread"), None, 1).unwrap();
    });

    // Start a competing record immediately; the lock serializes it.
    let _outcome = store_b.record(&snap("from-main"), None, 2).unwrap();
    handle.join().unwrap();

    // Both records committed with unique, monotonic sequences.
    let report = store_b.recover().unwrap();
    let recovered = report.recovered.unwrap();
    assert!(
        recovered.entry.sequence <= 1,
        "recovered should be one of the two records"
    );
    let bodies: std::collections::HashSet<String> = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("autosave-") && name.ends_with(".md"))
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect();
    assert!(
        bodies.contains("from-main"),
        "from-main snapshot should exist"
    );
    assert!(
        bodies.contains("from-thread"),
        "from-thread snapshot should exist"
    );
}

#[test]
fn lock_timeout_returns_autosave_busy() {
    let dir = TestDir::new("busy");
    let store = AutosaveStore::new(dir.0.clone());

    // Hold the advisory lock externally for longer than the 250 ms retry budget.
    let lock_path = lock_file_path(&dir.0);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock_file).unwrap();

    let start = std::time::Instant::now();
    let result = store.record(&snap("blocked"), None, 1);
    let elapsed = start.elapsed();

    drop(lock_file);

    assert!(
        matches!(result, Err(AutosaveError::AutosaveBusy)),
        "expected Busy, got {result:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(250),
        "should have retried until timeout: {elapsed:?}"
    );
}

#[test]
fn all_corrupt_journal_returns_typed_rejected_reasons() {
    let dir = TestDir::new("corrupt");
    let store = AutosaveStore::new(dir.0.clone());

    fs::write(
        dir.0.join(feathermark_core::AUTOSAVE_JOURNAL_FILE),
        "not json\n{\"v\":2}\n",
    )
    .unwrap();

    let RecoveryReport {
        recovered,
        rejected,
    } = store.recover().unwrap();
    assert!(recovered.is_none());
    assert_eq!(rejected.len(), 2);
    assert_eq!(rejected[0].reason, RejectionReason::CorruptLine);
    assert_eq!(rejected[1].reason, RejectionReason::UnsupportedVersion);
}

#[test]
fn oversize_journal_is_rejected_pre_allocation() {
    let dir = TestDir::new("oversize-journal");
    let store = AutosaveStore::new(dir.0.clone());

    let huge = vec![b' '; MAX_JOURNAL_BYTES + 1];
    fs::write(dir.0.join(feathermark_core::AUTOSAVE_JOURNAL_FILE), &huge).unwrap();

    let result = store.record(&snap("too-big"), None, 1);
    assert!(
        matches!(result, Err(AutosaveError::OversizeJournal { maximum }) if maximum == MAX_JOURNAL_BYTES),
        "expected OversizeJournal, got {result:?}"
    );

    let result = store.recover();
    assert!(
        matches!(result, Err(AutosaveError::OversizeJournal { maximum }) if maximum == MAX_JOURNAL_BYTES),
        "expected OversizeJournal on recover, got {result:?}"
    );
}

#[test]
fn oversize_session_state_is_rejected_pre_allocation() {
    let dir = TestDir::new("oversize-session");
    let store = AutosaveStore::new(dir.0.clone());

    let huge = vec![b' '; MAX_SESSION_STATE_BYTES + 1];
    fs::write(dir.0.join(feathermark_core::SESSION_STATE_FILE), &huge).unwrap();

    let result = store.load_session_state();
    assert!(
        matches!(result, Err(AutosaveError::OversizeSessionState { maximum }) if maximum == MAX_SESSION_STATE_BYTES),
        "expected OversizeSessionState, got {result:?}"
    );
}

#[test]
fn relative_and_nul_paths_are_rejected() {
    let dir = TestDir::new("bad-paths");
    let store = AutosaveStore::new(dir.0.clone());

    let relative = store.record(&snap("x"), Some("notes/todo.md"), 1);
    assert!(
        matches!(relative, Err(AutosaveError::Record(_))),
        "relative document_path should be rejected: {relative:?}"
    );

    let nul = store.record(&snap("x"), Some("/notes/todo\0.md"), 1);
    assert!(
        matches!(nul, Err(AutosaveError::Record(_))),
        "NUL document_path should be rejected: {nul:?}"
    );

    let bad_state = SessionStateV1 {
        schema: SESSION_SCHEMA_V1.to_owned(),
        v: 1,
        saved_at_unix_ms: 1,
        last_file: Some("relative.md".to_owned()),
        selection: None,
        top_visible_byte: None,
        window: None,
        recent_files: vec!["/valid.md".to_owned()],
    };
    let result = store.save_session_state(&bad_state);
    assert!(
        matches!(result, Err(AutosaveError::Record(_))),
        "relative last_file should be rejected: {result:?}"
    );

    let bad_state = SessionStateV1 {
        schema: SESSION_SCHEMA_V1.to_owned(),
        v: 1,
        saved_at_unix_ms: 1,
        last_file: Some("/valid.md".to_owned()),
        selection: None,
        top_visible_byte: None,
        window: None,
        recent_files: vec!["/ok.md".to_owned(), "relative.md".to_owned()],
    };
    let result = store.save_session_state(&bad_state);
    assert!(
        matches!(result, Err(AutosaveError::Record(_))),
        "relative recent_file should be rejected: {result:?}"
    );
}

#[test]
fn orphan_snapshots_are_collected_and_deletion_failures_retained() {
    let dir = TestDir::new("orphan-gc");
    let store = AutosaveStore::new(dir.0.clone());

    // Create an orphan snapshot file that the journal does not reference.
    let orphan = dir.0.join("autosave-999.md");
    fs::write(&orphan, b"orphan").unwrap();

    let outcome = store.record(&snap("real"), None, 1).unwrap();
    // The scanner sees the just-written referenced snapshot and the orphan.
    assert_eq!(outcome.orphan_gc.scanned, 2);
    assert_eq!(outcome.orphan_gc.removed, 1);
    assert!(outcome.orphan_gc.failures.is_empty());
    assert!(!orphan.exists());

    // Create a directory with an autosave snapshot name so removal fails.
    let dir_orphan = dir.0.join("autosave-998.md");
    fs::create_dir(&dir_orphan).unwrap();

    let outcome = store.record(&snap("real-2"), None, 2).unwrap();
    assert_eq!(outcome.orphan_gc.scanned, 3);
    assert_eq!(outcome.orphan_gc.removed, 0);
    assert_eq!(outcome.orphan_gc.failures.len(), 1);
    assert_eq!(outcome.orphan_gc.failures[0].file_name, "autosave-998.md");
    assert!(dir_orphan.exists());
}

#[test]
fn sequence_overflow_returns_sequence_exhausted() {
    let dir = TestDir::new("overflow");
    let store = AutosaveStore::new(dir.0.clone());

    // Seed the journal with the maximum possible sequence.
    let entry = format!(
        "{{\"schema\":\"feathermark.autosave.v1\",\"v\":1,\"sequence\":{},\"captured_at_unix_ms\":1,\"document_path\":null,\"document_revision\":0,\"snapshot_file\":\"autosave-{}.md\",\"snapshot_bytes\":0,\"snapshot_blake3\":\"{}\"}}\n",
        u64::MAX,
        u64::MAX,
        "0".repeat(64)
    );
    fs::write(dir.0.join(feathermark_core::AUTOSAVE_JOURNAL_FILE), entry).unwrap();
    fs::write(dir.0.join(format!("autosave-{}.md", u64::MAX)), b"").unwrap();

    let result = store.record(&snap("overflow"), None, 1);
    assert!(
        matches!(result, Err(AutosaveError::SequenceExhausted)),
        "expected SequenceExhausted, got {result:?}"
    );
}

#[test]
fn oversized_snapshot_is_rejected_with_typed_reason() {
    use feathermark_core::RejectionReason;

    let dir = TestDir::new("oversize-snapshot");
    let store = AutosaveStore::new(dir.0.clone());

    // Record a valid snapshot, then grow it past the document cap so recovery
    // reports SnapshotOversized rather than SnapshotMissing.
    let outcome = store.record(&snap("valid"), None, 1).unwrap();
    let oversized = dir.0.join(&outcome.entry.snapshot_file);
    let big = vec![b'x'; feathermark_core::MAX_DOCUMENT_BYTES + 1];
    fs::write(&oversized, &big).unwrap();

    let report = store.recover().unwrap();
    assert!(report.recovered.is_none());
    assert!(
        report
            .rejected
            .iter()
            .any(|r| r.sequence == Some(0) && r.reason == RejectionReason::SnapshotOversized),
        "expected SnapshotOversized, got {report:?}"
    );
}
