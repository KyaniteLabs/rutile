# Local Search and Related Links (Roadmap 12) — Design Decisions

Status: **in-memory contract and native search/backlink dialogs implemented;
GTK execution verified, macOS rendered acceptance pending.** Parent issue:
`.scratch/rutile-macos-roadmap/issues/12-local-search-and-related-links.md`.
Blocked by 03 (LOCKED), 08 (DONE).

## Resolved decisions

### S1 — In-memory substring index, not SQLite FTS5

For a local-first personal editor with ≤16 open documents, an in-memory
inverted index is simpler, faster (no IPC), and bounded by `MAX_OPEN_DOCUMENTS`.
SQLite FTS5 is deferred to a future "vault" mode (hundreds of documents) if
needed. The index is rebuilt on document open/edit; no persistence.

### S2 — Index triggers: on open + on idle debounce

The native navigator builds a bounded index from open snapshots when the user
submits a query. Empty queries do not index documents. Results retain their
snapshot identity so editing, closing or changing a path invalidates navigation.

### S3 — Ranking: recency + frequency + document-active boost

The index ranks the active document first, then applies its existing match
ordering. Native results are capped at 500 rows, with at most 50 matches per
document. Navigation reads only the open workspace; it does not scan directories.

### S4 — Path/privacy: only open documents are indexed

The search index covers only documents currently open in tabs. It does NOT
scan the filesystem. No path traversal, no external file reading. Privacy is
preserved by construction.

### S5 — Heading extraction for result context

Each search result includes the nearest preceding heading from the renderer's
outline model, including setext headings, as context.

### S6 — Backlink graph from markdown + wiki-links

The backlink graph parses two link types:
- Standard markdown: `[text](relative-or-absolute-path.md)`
- Wiki-links: `[[path-or-display-name]]`

Links are resolved relative to the linking document's directory. A backlink
from A→B means "A links to B." The graph answers "which documents link to
document X?"

### S7 — Stale-index behavior: graceful degradation

Native results retain their source snapshot, revision, document identity and
path. Activation rejects changed or closed documents before installing a
selection. A new query builds from the current open buffers.

### S8 — Result navigation via AppMessage

`DocumentSessionCore::activate_navigation` validates the retained result,
switches tabs through the shared reducer, and stores its selection and viewport.
The native shell then installs that shared view and refreshes the preview.

## Locking gate

The `SearchIndex`, `BacklinkGraph`, `SearchResult`, and `Backlink` types are
LOCKED when implementation begins. The first PR implements these types with
unit tests, no security-core edit, and full per-cluster gate coverage.
