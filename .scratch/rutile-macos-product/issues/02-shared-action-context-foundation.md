# Shared action and document-context foundation

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: none

## What it delivers

A shared product foundation for stable action identifiers, action metadata,
bounded preferences, recent-file records, resource budgets, and document
identity. It preserves the current single-document behavior while giving later
features one source of truth.

## Acceptance criteria

- Menus, shortcuts, the future command palette, and accessibility announcements
  can refer to the same stable action identifiers.
- Unavailable actions expose a reason and unknown identifiers cannot mutate
  application state.
- Preferences and recent-file records use versioned bounded local schemas and
  degrade safely when malformed or inaccessible.
- Document identity is distinct from path, revision, tab position, and window.
- Stale asynchronous work is rejected by identity and revision.
- Existing macOS baseline tests remain green.

## Merge boundary

Do not add tabs, palette UI, or new persistence consumers in this ticket. Land
the contracts and the smallest exercised integration surface first.
