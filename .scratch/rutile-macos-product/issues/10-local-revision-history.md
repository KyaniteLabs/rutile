# Local revision history

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 08

## What it delivers

A bounded local history browser that lets a user inspect and deliberately
restore earlier revisions for the correct document.

## Acceptance criteria

- History entries are local, bounded, versioned, and tied to document identity.
- Preview is read-only until explicit restore or copy action.
- Restore creates a normal revisioned edit with undo and dirty tracking.
- Corruption, quota exhaustion, missing snapshots, and stale selections fail
  closed without changing the active document.
- History cleanup cannot delete another document's recovery material.
