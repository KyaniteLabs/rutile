# Multi-Document Manager and Tabs (Roadmap 08) — Design Decisions

Status: **implemented on macOS (PRs #112–#113, #116–#118).** Linux has the
shared data plane only — no GTK strip. GTK chrome is P1 in
`docs/plan/linux-parity.md`. D8 window restore of every open path
is still last-file-only. Parent issue:
`.scratch/rutile-macos-roadmap/issues/08-multi-document-manager-and-tabs.md`.

## Question

What document/session model supports multiple open documents and a single-window tab strip on macOS while preserving revision checks, autosave, external-conflict handling, preview scheduling, recovery, and close prompts?

## Resolved decisions

### D1 — Per-document state extracted into `DocumentSlot`

The current `AppState` fields that are per-document (`revision`, `dirty`,
`preview`, `path`, `saved_disk`, `external_conflict`, `find`, `autosave`,
`next_transaction_id`, `mirror_resync_pending`) are extracted into a
`DocumentSlot` struct. `AppState` retains only shell-level state:
`documents` collection, `active_id`, tab ordering, shared notices, and
recents.

```rust
pub struct DocumentSlot {
    pub revision: Revision,
    pub dirty: bool,
    pub preview: PreviewState,
    pub path: Option<PathBuf>,
    pub saved_disk: Option<DiskVersion>,
    pub external_conflict: Option<DiskVersion>,
    pub find: Option<FindSession>,
    pub autosave: Option<AutosaveStore>,
    pub next_transaction_id: u64,
    pub mirror_resync_pending: bool,
}
```

### D2 — Tab identity = `DocumentId`

Each tab is identified by its `DocumentId` (already implemented in G003).
IDs are minted from a monotonic counter on `AppState` and never reused.
`DocumentId::ROOT` is the initial single-document tab (migration path).

### D3 — Tab ordering via `IndexMap`

`IndexMap<DocumentId, DocumentSlot>` preserves insertion order for tab
display. Users can reorder via `AppMessage::ReorderTab { from, to }`
which swaps positions in the map's order.

### D4 — Duplicate opens switch to existing tab

When opening a file, the reducer checks if any existing slot has the same
canonical path. If so, it switches to that tab (`SwitchTab`) instead of
creating a new one. This prevents duplicate opens and matches macOS HIG.

### D5 — Per-document dirty/conflict/autosave

Each `DocumentSlot` owns its own `dirty`, `saved_disk`,
`external_conflict`, and `autosave`. The reducer dispatches messages to
the active slot (or a specific slot by `DocumentId`). Only the active
document's autosave is ticked. Conflict prompts are per-tab.

### D6 — Resource limit: `MAX_OPEN_DOCUMENTS = 16`

Opening beyond the cap fails closed with a `UserNotice` (Warning
severity). The user must close a tab before opening more. This bounds
memory and preview-host resource usage.

### D7 — Close tab with dirty prompt

`AppMessage::CloseTab { id }` checks the slot's `dirty` flag. If dirty,
emits `AppEffect::RequestTabCloseDecision { id }` (the platform shows a
per-tab save/discard/cancel prompt). On `CloseDecision::Discard` or
post-save, the slot is removed and the next/previous tab becomes active.

### D8 — Window restoration

`SessionStateV1` already has `recent_files: Vec<String>`. For multi-doc,
the session stores `open_tabs: Vec<String>` (ordered paths of open tabs)
plus `active_tab: Option<String>`. On restore, each path is opened as a
tab in order, then the active tab is focused. Untitled documents are not
restored (matching current single-doc behavior).

### D9 — Migration from single-document AppState

The migration creates one `DocumentSlot` from the existing AppState
fields and assigns it `DocumentId::ROOT`. `active_id = ROOT`. This is
transparent: the first launch after the multi-doc update sees exactly one
tab, exactly as before.

### D10 — New `AppMessage` variants

- `NewTab` — creates a new untitled document in a new tab
- `SwitchTab { id: DocumentId }` — switches focus to a tab
- `CloseTab { id: DocumentId }` — initiates close (dirty prompt if needed)
- `ReorderTab { from: usize, to: usize }` — reorders tabs

Existing messages (DocumentEdited, SaveCompleted, etc.) implicitly
target the **active** slot unless they carry an explicit `DocumentId`.

## Locking gate

These signatures are LOCKED when implementation begins. The first PR
implements `DocumentSlot`, the `IndexMap` on `AppState`, tab management
methods, and the migration path — with no security-core edit and full
per-cluster gate coverage. Re-plan the macOS tab-strip UI against these
locked signatures.
