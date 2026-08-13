//! Wave 4 performance bench: the autosave wire decoders and the autosave
//! store durability path.
//!
//! Measures three budgeted operations from `docs/wave4/performance-budget.md`:
//!
//! * [`decode_autosave_entry`] — worst-case valid record near the 4 KiB byte
//!   cap; p95 < 100 µs.
//! * [`decode_session_state`] — worst-case valid record near the 64 KiB byte
//!   cap; p95 < 500 µs.
//! * [`AutosaveStore::record`] — full journal append (snapshot write + durable
//!   journal line + prune + orphan gc) for a 4 KiB snapshot; p95 < 60 ms.
//! * [`AutosaveStore::recover`] — open/rehydrate over a full eight-snapshot
//!   journal; p95 < 50 ms.
//!
//! The decoders are CPU-bound and run per journal line during recovery, so they
//! use a high iteration count. `record` and `recover` are durability-bound
//! (lock + `fsync`) and run on the autosave timer / at launch, so they use a
//! modest sample count. All store I/O lands in a process-owned temp directory
//! under `TMPDIR`; the real autosave store is never touched.
//!
//! Style matches the existing `render.rs`/`edit.rs` benches: `harness = false`,
//! `std::time::Instant`, sorted-sample p95, `assert!` ceiling, stderr report.

use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rutile_core::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, AutosaveStore, Document, SESSION_SCHEMA_V1,
    SessionSelectionV1, SessionStateV1, SessionWindowV1, decode_autosave_entry,
    decode_session_state, encode_autosave_entry, encode_session_state,
};
use rutile_types::Revision;

fn main() {
    bench_decode_entry();
    bench_decode_session();
    bench_record();
    bench_recover();
}

// --- CPU-bound wire decoders ------------------------------------------------

fn bench_decode_entry() {
    // Worst-case valid record near the 4 KiB byte cap: a long but legitimate
    // absolute document path pushes the encoded entry toward the ceiling.
    let entry = AutosaveEntryV1 {
        schema: AUTOSAVE_SCHEMA_V1.to_owned(),
        v: 1,
        sequence: 42,
        captured_at_unix_ms: 1_750_000_000_000,
        document_path: Some(format!("/home/user/{}", "a".repeat(3_800))),
        document_revision: Revision::new(7),
        snapshot_file: "snap-42".to_owned(),
        snapshot_bytes: 4096,
        snapshot_blake3: "0".repeat(64),
    };
    let encoded = encode_autosave_entry(&entry).expect("valid entry encodes");
    assert!(
        !encoded.is_empty(),
        "constructed a representative record for decode"
    );

    const ITERS: usize = 2_000;
    let mut times = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let started = Instant::now();
        let decoded = decode_autosave_entry(black_box(&encoded)).expect("encoded record decodes");
        let elapsed = started.elapsed();
        assert_eq!(decoded, entry);
        times.push(elapsed);
    }
    report(
        "decode_autosave_entry",
        encoded.len(),
        ITERS,
        times,
        Duration::from_micros(100),
    );
}

fn bench_decode_session() {
    // Worst-case *valid* record: the session cap is 64 KiB but each path is
    // itself capped at `MAX_SESSION_PATH_BYTES` (4096), so the largest legal
    // state is `last_file` plus ten `recent_files`, each near the path cap
    // (≈45 KiB encoded). Exercise that floor.
    let long_path = format!("/home/user/{}", "a".repeat(4_080));
    let state = SessionStateV1 {
        schema: SESSION_SCHEMA_V1.to_owned(),
        v: 1,
        saved_at_unix_ms: 1_750_000_000_000,
        last_file: Some(long_path.clone()),
        selection: Some(SessionSelectionV1 {
            anchor: 100,
            head: 200,
        }),
        top_visible_byte: Some(50),
        window: Some(SessionWindowV1 {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        }),
        recent_files: (0..10).map(|_| long_path.clone()).collect(),
    };
    let encoded = encode_session_state(&state).expect("valid state encodes");

    const ITERS: usize = 1_000;
    let mut times = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let started = Instant::now();
        let decoded = decode_session_state(black_box(&encoded)).expect("encoded state decodes");
        let elapsed = started.elapsed();
        assert_eq!(decoded, state);
        times.push(elapsed);
    }
    report(
        "decode_session_state",
        encoded.len(),
        ITERS,
        times,
        Duration::from_micros(500),
    );
}

// --- Durability-bound store operations --------------------------------------

fn bench_record() {
    let dir = fresh_store_dir("record");
    let store = AutosaveStore::new(dir.clone());
    let document = Document::new(&document_payload(4 * 1024)).expect("document within cap");
    let snapshot = document.snapshot();
    let path = format!("/home/user/doc-{}.md", std::process::id());

    const SAMPLES: usize = 20;
    let mut times = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES as u64 {
        let started = Instant::now();
        let outcome = store
            .record(
                black_box(&snapshot),
                black_box(Some(&path)),
                1_750_000_000_000 + i,
            )
            .expect("record succeeds");
        times.push(started.elapsed());
        black_box(outcome);
    }
    report_path(
        "AutosaveStore::record",
        SAMPLES,
        times,
        Duration::from_millis(60),
    );
}

fn bench_recover() {
    let dir = fresh_store_dir("recover");
    let store = AutosaveStore::new(dir.clone());
    let document = Document::new(&document_payload(4 * 1024)).expect("document within cap");
    let snapshot = document.snapshot();
    let path = format!("/home/user/doc-{}.md", std::process::id());

    // Fill the store to the retention cap so recovery walks a full journal.
    for i in 0u64..8 {
        store
            .record(&snapshot, Some(&path), 1_750_000_000_000 + i)
            .expect("seed record succeeds");
    }

    const SAMPLES: usize = 15;
    let mut times = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let report = store.recover().expect("recover succeeds");
        times.push(started.elapsed());
        black_box(report);
    }
    report_path(
        "AutosaveStore::recover",
        SAMPLES,
        times,
        Duration::from_millis(50),
    );
}

// --- helpers ----------------------------------------------------------------

fn report(name: &str, bytes: usize, n: usize, mut times: Vec<Duration>, budget: Duration) {
    times.sort_unstable();
    let p95 = times[n * 95 / 100];
    eprintln!(
        "{name} {:.1} KiB p95 {p95:?} ({n} samples, budget {budget:?})",
        bytes as f64 / 1024.0,
    );
    assert!(
        p95 <= budget,
        "{name} p95 {p95:?} exceeded budget {budget:?}"
    );
}

fn report_path(name: &str, n: usize, mut times: Vec<Duration>, budget: Duration) {
    times.sort_unstable();
    let p95 = times[n * 95 / 100];
    eprintln!("{name} p95 {p95:?} ({n} samples, budget {budget:?})");
    assert!(
        p95 <= budget,
        "{name} p95 {p95:?} exceeded budget {budget:?}"
    );
}

fn document_payload(bytes: usize) -> String {
    let header = "# Rutile performance payload\n\n";
    let mut payload = String::from(header);
    payload.push_str(&"a".repeat(bytes.saturating_sub(payload.len())));
    payload
}

fn fresh_store_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rutile-bench-autosave-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create bench store dir");
    dir
}
