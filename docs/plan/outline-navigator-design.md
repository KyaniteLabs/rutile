# Outline Navigator (Roadmap 05) — Design Decisions

Status: **native dialog implemented in both shells; GTK and external macOS
selection executed. Repeated focus validation remains in progress.** Parent
issue: `.scratch/rutile-macos-roadmap/issues/05-outline-navigator.md`.
Blocked by 03 (DONE).

## Resolved decisions

### O1 — Derive headings from the existing source-anchor model

The outline does **not** introduce a second markdown parser. It calls
`rutile_core::build_source_blocks(source, revision)` — the same validated
source-anchor model the preview renderer uses — and keeps the blocks whose
`kind == SourceBlockKind::Heading`. Headings therefore match the preview
exactly, including setext (`===`/`---`) and deeply-nested cases. The outline
never imports a parser or touches the security core.

### O2 — Level + text extracted from the validated source range

Each heading block carries a validated `start..end` byte range into `source`.
The outline reads `source[start..end]` to recover the level (count of leading
`#` for ATX; `=`/`-` underline for setext) and the display text (markup
stripped). Because the range is the renderer's own range, this can never
desynchronize from the preview.

### O3 — Navigation by byte offset, never by line or DOM guess

Every `OutlineEntry` carries `source_offset` (the heading's byte start) and
`dom_id` (the renderer's anchor id). Jumping to a heading scrolls the source to
`source_offset` and the preview to `dom_id`. The shell never guesses a line
number or resolves a selector — it uses the two stable coordinates the model
already guarantees.

### O4 — Duplicate titles are allowed; offsets disambiguate

Two headings with the same text are both listed. Navigation keys on
`source_offset`, which is unique. The renderer already guarantees unique
`dom_id`s, so the preview side is unambiguous too. No title mangling.

### O5 — Bounded, flat list (no collapse in the contract)

The outline is a flat, ordered `Vec<OutlineEntry>` capped at
`MAX_OUTLINE_ENTRIES` (500). Level is exposed for indentation/collapse in the
**shell**, but the contract itself does not model collapsed groups — collapse is
ephemeral UI state the shell owns. An empty document yields an empty outline.

### O6 — Current-heading + next/prev helpers

`heading_at(offset)` returns the deepest preceding heading (the one the
viewport is currently inside), so the shell can highlight the active section as
the user scrolls. `next_after`/`prev_before` drive outline-keyboard navigation
(↑/↓ in the sidebar). All three key off `source_offset`, never off text.

### O7 — Rebuild on render, stale-safe

The native navigator calls `Outline::from_source` with the current snapshot
when opened or filtered. Activation validates that the retained snapshot still
matches before moving the caret; changed results are rejected.

### O8 — Native bounded navigator

The command palette opens a native filtered list in each shell. The shared
`navigation.rs` surface validates document identity, path, revision and snapshot
contents before installing a source selection and viewport. GTK dialog execution
and rendered inspection pass; external macOS heading selection also passes.
A persistent sidebar and active-section highlight
are separate future UI choices.
