# Local Search and Related Links (Roadmap 12) — Design Decisions

Status: **design (locked signatures pending implementation)**. Parent issue:
`.scratch/rutile-macos-roadmap/issues/12-local-search-and-related-links.md`.
Blocked by 03 (LOCKED), 08 (DONE).

## Resolved decisions

### S1 — In-memory substring index, not SQLite FTS5

For a local-first personal editor with ≤16 open documents, an in-memory
inverted index is simpler, faster (no IPC), and bounded by `MAX_OPEN_DOCUMENTS`.
SQLite FTS5 is deferred to a future "vault" mode (hundreds of documents) if
needed. The index is rebuilt on document open/edit; no persistence.

### S2 — Index triggers: on open + on idle debounce

The index updates when a document is opened and on a debounced idle timer
(~500ms after the last edit). No real-time per-keystroke reindexing.

### S3 — Ranking: recency + frequency + document-active boost

Results are ranked by: (1) active document matches first, (2) match frequency
within each document, (3) most-recently-opened document order from
`RecentDocuments`.

### S4 — Path/privacy: only open documents are indexed

The search index covers only documents currently open in tabs. It does NOT
scan the filesystem. No path traversal, no external file reading. Privacy is
preserved by construction.

### S5 — Heading extraction for result context

Each search result includes the nearest preceding heading (line starting
with `#`) as context, so the user sees where the match lives in the document
structure.

### S6 — Backlink graph from markdown + wiki-links

The backlink graph parses two link types:
- Standard markdown: `[text](relative-or-absolute-path.md)`
- Wiki-links: `[[path-or-display-name]]`

Links are resolved relative to the linking document's directory. A backlink
from A→B means "A links to B." The graph answers "which documents link to
document X?"

### S7 — Stale-index behavior: graceful degradation

If a document was edited but not yet reindexed, search results may include
stale matches. The `SearchResult` includes the `indexed_revision` so the
platform shell can show a "reindexing..." indicator if needed. Stale results
are NOT harmful — they just may not reflect the very latest edit.

### S8 — Result navigation via AppMessage

`AppMessage::SearchResultActivated { document_id, byte_offset }` switches to
the document's tab and scrolls to the match offset. The platform shell
handles the visual navigation.

## Locking gate

The `SearchIndex`, `BacklinkGraph`, `SearchResult`, and `Backlink` types are
LOCKED when implementation begins. The first PR implements these types with
unit tests, no security-core edit, and full per-cluster gate coverage.
