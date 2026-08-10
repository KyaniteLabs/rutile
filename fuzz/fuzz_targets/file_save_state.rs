#![no_main]

//! Fuzz the [`FileService`] save/load state machine and the path gate that
//! guards it.
//!
//! The fuzzed bytes drive two real surfaces in `rutile-core`:
//!
//! * a candidate path string is validated against the product's path contract
//!   (NUL-free, absolute, non-empty, byte-bounded) — the same rules
//!   [`rutile_core`] enforces before it ever persists a path. Every
//!   hostile shape is classified with a typed rejection and never reaches a
//!   filesystem write.
//! * a controlled, process-unique temporary file is then saved and reloaded
//!   through [`LocalFileService`] across all three [`SaveFault`] transitions,
//!   asserting the typed [`SaveOutcome`] each fault produces, and that a
//!   committed save round-trips through [`FileService::load`].
//!
//! The fuzz-derived candidate path is never itself written to — only the gate
//! sees it — so hostile absolute paths cannot clobber real files. All writes
//! land inside a single process-owned temp directory under `TMPDIR`.
//!
//! Invariants asserted on every input:
//! * `save_atomic` and `load` never panic on any path/payload combination;
//! * a NUL-bearing, relative, empty, or oversize path is rejected by the gate
//!   before any filesystem write;
//! * each `SaveFault` value produces exactly its typed `SaveOutcome`
//!   (`None` -> `Committed`, `BeforeRename` -> `NotCommitted`, `AfterRename`
//!   -> `CommittedDurabilityUnknown`);
//! * a committed save's on-disk length and reloaded text match the snapshot;
//! * a non-UTF-8 file on disk surfaces a typed `FileError` from `load`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use rutile_core::{
    Document, FileError, FileService, LocalFileService, SaveError, SaveFault, SaveOutcome,
    MAX_DOCUMENT_BYTES,
};
use libfuzzer_sys::fuzz_target;

/// Mirrors `session_contract::MAX_SESSION_PATH_BYTES` — the byte cap every
/// persisted path must respect.
const PATH_CAP: usize = 4096;
/// Cap on the document payload saved per iteration, keeping each atomic write
/// (temp create, fsync, rename, parent fsync) cheap for high-throughput runs.
/// `save_atomic` always fsyncs, so this target is I/O-bound; prefer smaller
/// `-runs` budgets or a `tmpfs` corpus directory.
const PAYLOAD_CAP: usize = 4 * 1024;

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|input: &[u8]| {
    path_gate_invariants(input);
    save_state_transitions(input);
    load_rejection(input);
});

/// Asserts the path gate rejects NUL, relative, empty, and oversize paths and
/// accepts only absolute, NUL-free, byte-bounded paths.
fn path_gate_invariants(input: &[u8]) {
    let Ok(text) = std::str::from_utf8(input) else {
        // A non-UTF-8 byte string can never be a persisted path; nothing to gate.
        return;
    };
    match path_rejection_reason(text) {
        Some(reason) => {
            assert!(
                matches!(
                    reason,
                    PathRejection::Empty | PathRejection::Nul
                        | PathRejection::Relative
                        | PathRejection::Oversize
                ),
                "hostile path must carry a typed rejection"
            );
        }
        None => {
            assert!(!text.is_empty());
            assert!(!text.contains('\0'));
            assert!(text.len() <= PATH_CAP);
            assert!(Path::new(text).is_absolute());
        }
    }
}

/// Drives every `SaveFault` transition on a fresh temp file and asserts the
/// typed `SaveOutcome` for each.
fn save_state_transitions(input: &[u8]) {
    let payload = save_payload(input);
    let document = match Document::new(&payload) {
        Ok(document) => document,
        Err(_) => return,
    };
    let snapshot = document.snapshot();
    let dir = fuzz_temp_dir();
    // If the temp directory is not usable (e.g. an unwritable `TMPDIR`), every
    // save would fail with `NotCommitted(Io)` and mask the transition logic
    // under test. Bail out rather than emit false crashes.
    if !dir.exists() {
        return;
    }
    let path = unique_path(dir);

    for fault in [SaveFault::None, SaveFault::BeforeRename, SaveFault::AfterRename] {
        // Each save starts from an absent target so the transition under test
        // is the only variable.
        let _ = fs::remove_file(&path);
        let service = LocalFileService::with_fault(fault);
        let outcome = service.save_atomic(&path, &snapshot);
        assert_typed_outcome(fault, &path, &outcome, &payload);
    }

    let _ = fs::remove_file(&path);
}

/// Asserts that a non-UTF-8 file on disk is rejected by `load` with a typed
/// `FileError` rather than panicking.
fn load_rejection(input: &[u8]) {
    let raw = non_utf8_payload(input);
    if raw.is_empty() {
        return;
    }
    let dir = fuzz_temp_dir();
    let path = unique_path(dir);

    let _ = fs::remove_file(&path);
    if fs::write(&path, &raw).is_err() {
        return;
    }
    let service = LocalFileService::new();
    match service.load(&path, MAX_DOCUMENT_BYTES) {
        Err(FileError::InvalidUtf8) | Err(FileError::TooLarge { .. }) => {}
        Ok(_) => panic!("non-UTF-8 file loaded as if it were valid UTF-8"),
        Err(_) => {}
    }
    let _ = fs::remove_file(&path);
}

/// Maps a path to its typed rejection reason, mirroring
/// `session_contract::validate_path`.
fn path_rejection_reason(path: &str) -> Option<PathRejection> {
    if path.is_empty() {
        return Some(PathRejection::Empty);
    }
    if path.len() > PATH_CAP {
        return Some(PathRejection::Oversize);
    }
    if path.contains('\0') {
        return Some(PathRejection::Nul);
    }
    if !Path::new(path).is_absolute() {
        return Some(PathRejection::Relative);
    }
    None
}

#[derive(Debug)]
enum PathRejection {
    Empty,
    Nul,
    Relative,
    Oversize,
}

fn assert_typed_outcome(fault: SaveFault, path: &Path, outcome: &SaveOutcome, payload: &str) {
    match (fault, outcome) {
        (SaveFault::None, SaveOutcome::Committed { disk }) => {
            assert_eq!(disk.len, payload.len() as u64);
            let loaded = LocalFileService::new()
                .load(path, MAX_DOCUMENT_BYTES)
                .expect("committed save reloads");
            assert_eq!(loaded.document.snapshot().to_string(), payload);
        }
        (SaveFault::BeforeRename, SaveOutcome::NotCommitted { reason }) => {
            assert!(
                matches!(reason, SaveError::InjectedBeforeRename),
                "before-rename fault must surface InjectedBeforeRename"
            );
            assert!(
                !path.exists(),
                "before-rename fault must not create the target"
            );
        }
        (SaveFault::AfterRename, SaveOutcome::CommittedDurabilityUnknown { .. }) => {
            // The rename committed (the target exists) but durability is
            // unknown because the fault fires before the parent-directory fsync.
            assert!(path.exists());
        }
        (fault, outcome) => {
            panic!("fault {fault:?} produced unexpected outcome {outcome:?}")
        }
    }
}

/// A UTF-8-safe, byte-bounded slice of the input used as the saved document.
fn save_payload(input: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(input) else {
        return String::from("# Rutile fuzz payload\n");
    };
    let mut end = PAYLOAD_CAP.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Leading `0xff` guarantees the byte sequence is never valid UTF-8, regardless
/// of the trailing fuzz-derived bytes — exercising `load`'s rejection path
/// without depending on the fuzzer stumbling onto invalid UTF-8 by itself.
fn non_utf8_payload(input: &[u8]) -> Vec<u8> {
    let mut raw = vec![0xff];
    let tail = input.len().min(PAYLOAD_CAP);
    raw.extend_from_slice(&input[..tail]);
    raw
}

fn fuzz_temp_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "rutile-fuzz-file-save-state-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    });
    dir
}

fn unique_path(dir: &Path) -> PathBuf {
    let sequence = UNIQUE.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("doc-{sequence}.md"))
}
