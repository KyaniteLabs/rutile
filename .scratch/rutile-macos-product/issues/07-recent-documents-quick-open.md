# Recent Documents and Quick Open

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 02

## What it delivers

A bounded, user-facing recent-document list and Quick Open flow for local
documents.

## Acceptance criteria

- Recent entries are capped, ordered deterministically, and persisted through
  the versioned local state contract.
- Quick Open filters the recent set without arbitrary filesystem crawling or
  network access.
- Missing, moved, duplicate, unreadable, and malformed entries degrade with
  clear recoverable outcomes.
- Opening a result uses the ordinary open, conflict, recovery, and document
  identity contracts.
- Private paths do not enter shared receipts or public output.
