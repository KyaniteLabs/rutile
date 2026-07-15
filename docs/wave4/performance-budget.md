# Wave 4 — Performance Budgets

> **Status: Active budget.** Established 2026-07-14 for the Rutile 0.2.0 core.
> Verified against `feathermark-core` `0.2.0` on `codex/rutile-w4-perf`.

## BLUF

This document fixes concrete wall-clock budgets for the core operations that
sit on a user- or recovery-visible path. Each budget names the **exact public
API** it applies to, the representative input, the limit, and the operational
basis it is derived from. The budgets are **performance targets** the product
holds itself to; the bench files assert them as p95 ceilings where the
operation is benchmarked, and existing `render`/`edit`/`scroll` benches carry
looser regression ceilings that fail only on gross regressions.

## Scope and method

Budgets are **wall-clock, p95, single-record/single-call**, measured on a
development-class Apple SSD. They are deliberately split into two families:

- **CPU-bound** (`render_markdown`, `decode_autosave_entry`,
  `decode_session_state`) — limited by parsing and validation throughput; no
  `fsync`, no lock. Budgets are tight because these can sit on the input or
  per-line recovery path.
- **Durability-bound** (`save_atomic`, `AutosaveStore::record`,
  `AutosaveStore::recover`) — dominated by `fsync`/rename/lock acquisition,
  which are non-negotiable for the data-safety contract they implement. Their
  budgets are looser and assume a healthy SSD; a degraded or spinning disk can
  exceed them without indicating a code regression.

Measurement harness style (matched to the existing `benches/render.rs` and
`benches/edit.rs`): `[[bench]] harness = false` binaries using
`std::time::Instant`, a modest sample count, sorted-sample p95, and an
`assert!` ceiling so the bench fails the run on breach. No `criterion`
dependency is introduced.

## Budgets

### 1. Markdown render (CPU-bound)

| API | Input | p95 budget |
| --- | --- | --- |
| [`render_markdown`](../../crates/feathermark-core/src/render.rs) `(source, revision)` | 4 KiB (typical note) | **< 2 ms** |
| `render_markdown` | 256 KiB (large document) | **< 40 ms** |
| `render_markdown` | 1 MiB (very large document) | **< 200 ms** |

**Basis.** `render_markdown` is a single-pass parse (vendored `pulldown-cmark`)
followed by allowlisted-HTML emission and source-block mapping, then a final
`validate_source_blocks` pass. No I/O, no allocation of unbounded buffers.
Parsing throughput is hundreds of MiB/s; 256 KiB parses in well under 25 ms and
the mapping/validate passes add bounded overhead, so 40 ms is a safe p95
ceiling while keeping an interactive feel for the coalesced preview render.

**Coverage.** `benches/render.rs` measures 1 MiB and 5 MiB and asserts
regression ceilings (2 s / 10 s). Those are intentionally loose CI gates; the
budgets above are the tighter product targets. A 256 KiB input is strictly
faster than the measured 1 MiB case, so the existing bench subsumes the 256 KiB
budget.

### 2. Atomic save (durability-bound)

| API | Input | p95 budget |
| --- | --- | --- |
| [`FileService::save_atomic`](../../crates/feathermark-core/src/files.rs) `(path, snapshot)` via `LocalFileService::new()` | 4 KiB document | **< 20 ms** |
| `FileService::save_atomic` | 1 MiB document | **< 30 ms** |

**Basis.** The save path creates a 0600 temporary file, writes the snapshot,
`fsync`s the temp (`File::sync_all`, macOS `fsync`), atomically renames over the
target, then `fsync`s the parent directory — two `fsync` calls plus the create
and rename. On an Apple SSD each `fsync` runs ~3–8 ms, so the 4 KiB case is
dominated by the two `fsync` calls; the observed p95 is ~11 ms and the 20 ms
budget holds ~2× headroom for `fsync` variance (the bench fails only on a real
~2× regression). At 1 MiB the payload write adds ~1 ms at GiB/s throughput; the
observed p95 is ~17 ms, under the 30 ms ceiling. This is the user-facing
**Save** action and must feel instant; the `fsync`s are the data-safety contract
and cannot be dropped.

**Coverage.** `benches/save_atomic.rs` (added in this wave) asserts these p95
ceilings directly at 4 KiB and 1 MiB against a process-owned temp directory.

### 3. Autosave wire decode (CPU-bound)

| API | Input | p95 budget |
| --- | --- | --- |
| [`decode_autosave_entry`](../../crates/feathermark-core/src/session_contract.rs) `(bytes)` | 4 KiB record (`MAX_AUTOSAVE_ENTRY_BYTES`) | **< 100 µs** |
| [`decode_session_state`](../../crates/feathermark-core/src/session_contract.rs) `(bytes)` | 64 KiB record (`MAX_SESSION_STATE_BYTES`) | **< 500 µs** |

**Basis.** Each decoder performs an NDJSON framing check, a bounded
`serde_json` deserialize, and symmetric field validation (schema/version/path
safety). No I/O, no lock. The entry decoder observes ~3–4 µs for a worst-case
4 KiB record, so 100 µs is a generous per-entry ceiling with ~25× headroom. The
session record is larger — its 64 KiB cap is unreachable because each path is
itself capped at 4 KiB (`MAX_SESSION_PATH_BYTES`), so the largest *valid* state
is `last_file` plus ten `recent_files` near the path cap (≈44 KiB encoded); the
decoder observes ~30–42 µs there, under the 500 µs ceiling. These run per
journal line during crash recovery, so a full eight-snapshot journal decodes in
well under 1 ms.

**Coverage.** `benches/autosave.rs` (added in this wave) asserts these p95
ceilings against a worst-case *valid* record (entry at the 4 KiB cap; session at
its reachable ≈44 KiB maximum).

### 4. Autosave journal append (durability-bound)

| API | Input | p95 budget |
| --- | --- | --- |
| [`AutosaveStore::record`](../../crates/feathermark-core/src/autosave.rs) `(snapshot, document_path, captured_at_unix_ms)` | 4 KiB snapshot | **< 60 ms** |

**Basis.** `record` acquires the advisory store lock, reads the journal, writes
the snapshot atomically (temp + `fsync` + rename + parent `fsync`), encodes the
entry, durably appends the journal line (another `fsync`), prunes to the
retention window, and garbage-collects orphans — three to four `fsync` calls
plus lock acquisition and journal re-read. The observed p95 is ~31–33 ms; the
60 ms ceiling holds ~2× headroom. It runs on the autosave timer, off the
keystroke hot path, so it may be slower than `save_atomic` but must not jank the
UI.

**Coverage.** `benches/autosave.rs` measures `record` over repeated appends
into a fresh store and asserts the p95 ceiling.

### 5. Crash recovery / rehydrate (durability + CPU)

| API | Input | p95 budget |
| --- | --- | --- |
| [`AutosaveStore::recover`](../../crates/feathermark-core/src/autosave.rs) `()` | full eight-snapshot journal | **< 50 ms** |

**Basis.** `recover` acquires the store lock, decodes every journal line
(family 3, single-digit µs each), sorts by sequence, and verifies the
highest-sequence snapshot by reading and hashing it with BLAKE3. For a full
eight-snapshot journal of 4 KiB snapshots the observed p95 is sub-millisecond
(~0.5 ms); the 50 ms ceiling is the *large-snapshot* startup budget — recovery
verifies exactly one snapshot, which may be up to 20 MiB, and BLAKE3-hashing
that adds the bulk of the cost. Recovery runs exactly once at launch
(open/rehydrate), so 50 ms keeps startup snappy while bounding worst-case
large-document rehydration.

**Coverage.** `benches/autosave.rs` exercises `recover` after filling a store
to the retention cap and asserts the p95 ceiling.

## Running the benches

```sh
# Compile-check every bench target without running:
cargo bench -p feathermark-core --no-run

# Run the new Wave 4 benches (they assert their budgets and exit non-zero on breach):
cargo bench -p feathermark-core --bench save_atomic
cargo bench -p feathermark-core --bench autosave

# Existing benches (regression ceilings):
cargo bench -p feathermark-core --bench render
cargo bench -p feathermark-core --bench edit
cargo bench -p feathermark-core --bench scroll
```

Each bench prints its measured p95 to stderr. The durability-bound benches write
only into a process-owned temp directory under `TMPDIR`; they never touch the
real autosave store or user documents.

## Out of scope

- Native-shell (Iced/GTK) render, layout, and WebView paint latency — those are
  platform-shell concerns, not `feathermark-core` budgets.
- Network/cloud operations — none exist in the core (a stated non-goal).
- Memory budgets beyond the existing `edit.rs` allocation gate (ordinary edits
  must not copy the full buffer), which is unchanged.
