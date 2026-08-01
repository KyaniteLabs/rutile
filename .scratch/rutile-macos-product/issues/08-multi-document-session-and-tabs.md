# Multi-document session manager and tabs

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 02, 03, 07

## What it delivers

Bounded multi-document ownership and tabs while preserving per-document
selection, scroll, mode, dirty state, recovery, conflicts, and preview
revision.

## Acceptance criteria

- Every tab has a stable document identity, accessible name, path display
  policy, and dirty marker.
- Switching tabs preserves valid per-document state and cannot transfer stale
  asynchronous results.
- Dirty close and replacement require an explicit save, discard, or cancel
  decision.
- Opening the same canonical local file twice has an explicit deterministic
  policy and cannot create silent write races.
- Recovery and external-change conflict decisions remain associated with the
  correct document.
- Existing single-document behavior remains a valid path.
