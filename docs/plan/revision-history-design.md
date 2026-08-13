# Local Revision History (Roadmap 11) — Design Decisions

Status: **locked (contract implemented in `revision_history.rs`; native
history UI deferred).** Parent issue:
`.scratch/rutile-macos-roadmap/issues/11-local-revision-history.md`.
Blocked by 03 (LOCKED), 08 (DONE).

## Resolved decisions

### H1 — User-visible checkpoints, separate from autosave

Autosave captures crash-recovery snapshots silently. Revision history captures
**user-visible** checkpoints for compare/restore. The two are independent:
autosave is frequent and automatic; history records meaningful milestones.

### H2 — Snapshot triggers

Checkpoints are recorded on:
1. **Save** — every successful save creates a `HistorySource::OnSave` entry
2. **Before bulk operations** — format, replace-all, smart-paste creates
   `HistorySource::BeforeBulk`
3. **Manual** — user explicitly creates a checkpoint via menu/shortcut

### H3 — Bounded retention: 100 entries, oldest evicted

`MAX_HISTORY_ENTRIES = 100`. When the cap is reached, the oldest entry is
evicted. This bounds memory usage while providing substantial history depth.

### H4 — No persistent storage in the initial contract

The revision history is in-memory only. Persistence (snapshots on disk) is
deferred to a future enhancement that uses the existing autosave directory
structure. The initial contract provides the data model and API; the
platform shell decides when/how to persist.

### H5 — Restore: switch to that revision, preserve current as latest

Restoring a historical checkpoint loads that revision's content but does NOT
destroy the current state — the current content becomes a new latest
checkpoint first. This preserves the "undo restore" path.

### H6 — Diff presentation deferred to platform UI

The contract provides `HistoryEntry { revision, timestamp_ms, description }`.
Visual diff presentation is a platform-layer concern (macOS native diff view
or an iced diff widget).

### H7 — Relationship to autosave + external-disk versions

- Autosave = crash recovery (silent, frequent, bounded journal)
- Revision history = user-visible milestones (explicit, bounded list)
- External disk versions = conflict detection (DiskVersion comparison)

These three are independent concerns with separate data structures.

## Locking gate

`RevisionHistory`, `HistoryEntry`, and `HistorySource` are LOCKED. The first
PR implements these types with unit tests, no security-core edit.
