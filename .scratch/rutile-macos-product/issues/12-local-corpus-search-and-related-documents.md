# Local corpus search and related documents

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 08

## What it delivers

Explicitly scoped local search over a user-selected corpus, with optional
related-document results and no hidden network boundary.

## Acceptance criteria

- Scope, roots, extensions, byte limits, refresh, and deletion behavior are
  explicit and bounded.
- Stale index entries are visible as stale and can be rebuilt or removed.
- Results carry document identity, revision, and bounded source ranges.
- Applying an edit requires revalidation against the current document.
- Relatedness is deterministic or labeled heuristic, and empty/ambiguous
  results are honest.
- A database implementation such as SQLite FTS5 is optional and does not
  authorize unbounded watchers or scanning.
