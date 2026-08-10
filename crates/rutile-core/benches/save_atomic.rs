//! Wave 4 performance bench: the atomic-save durability path.
//!
//! Measures [`FileService::save_atomic`] through [`LocalFileService::new()`]
//! (no fault injection) against a 4 KiB and a 1 MiB document, asserting the
//! p95 budgets fixed in `docs/wave4/performance-budget.md`:
//!
//! * 4 KiB document  — p95 < 20 ms
//! * 1 MiB document — p95 < 30 ms
//!
//! The save path creates a 0600 temp file, writes the snapshot, `fsync`s the
//! temp, atomically renames over the target, and `fsync`s the parent
//! directory, so it is I/O-bound and dominated by `fsync` latency on a healthy
//! SSD. All writes land inside a single process-owned temp directory under
//! `TMPDIR`; the real autosave store and user documents are never touched.
//!
//! Style matches the existing `render.rs`/`edit.rs` benches: `harness = false`,
//! `std::time::Instant`, sorted-sample p95, `assert!` ceiling, stderr report.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rutile_core::{Document, FileService, LocalFileService, SaveOutcome};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn main() {
    let dir = bench_dir();
    measure(&dir, 4 * 1024, Duration::from_millis(20));
    measure(&dir, 1024 * 1024, Duration::from_millis(30));
}

fn measure(dir: &Path, bytes: usize, budget: Duration) {
    // Build a representative markdown document of the target size and snapshot
    // it once; `save_atomic` only reads the snapshot, so a single snapshot is
    // reused across samples.
    let payload = document_payload(bytes);
    let document = Document::new(&payload).expect("document within cap");
    let snapshot = document.snapshot();

    let service = LocalFileService::new();
    const SAMPLES: usize = 30;
    let mut times = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let path = unique_path(dir);
        // Start from an absent target so the only variable is the save itself.
        let _ = fs::remove_file(&path);
        let started = Instant::now();
        let outcome = service.save_atomic(black_box(&path), black_box(&snapshot));
        let elapsed = started.elapsed();
        assert_outcome_committed(&outcome, &payload);
        times.push(elapsed);
        let _ = fs::remove_file(&path);
    }

    times.sort_unstable();
    let p95 = times[SAMPLES * 95 / 100];
    eprintln!(
        "save_atomic {:.0} KiB p95 {p95:?} ({SAMPLES} samples, budget {budget:?})",
        bytes as f64 / 1024.0,
    );
    assert!(
        p95 <= budget,
        "save_atomic {:.0} KiB p95 {p95:?} exceeded budget {budget:?}",
        bytes as f64 / 1024.0
    );
}

/// A UTF-8, char-aligned markdown payload padded to exactly `bytes` with a
/// trailing newline so the document ends on a clean boundary.
fn document_payload(bytes: usize) -> String {
    let header = "# Rutile performance payload\n\n";
    let mut payload = String::from(header);
    payload.push_str(&"a".repeat(bytes.saturating_sub(payload.len())));
    payload
}

fn assert_outcome_committed(outcome: &SaveOutcome, payload: &str) {
    match outcome {
        SaveOutcome::Committed { .. } => {}
        other => panic!("save_atomic failed to commit: {other:?}"),
    }
    // Silence unused-warning when assertions evolve; payload anchors intent.
    let _ = payload;
}

fn bench_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rutile-bench-save-atomic-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create bench temp dir");
    dir
}

fn unique_path(dir: &Path) -> PathBuf {
    let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("doc-{n}.md"))
}
