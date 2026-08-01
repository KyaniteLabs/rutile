# Command palette and action discovery

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 02

## What it delivers

A keyboard-first command palette backed by the shared action registry.

## Acceptance criteria

- Bounded search covers stable action labels and approved aliases.
- Results show shortcuts, availability, and a reason when unavailable.
- Invocation uses the same action path as menus and keyboard shortcuts.
- Cancellation closes without mutating the active document.
- Result updates, focus, and invocation are accessible and deterministic.
- Unknown or stale actions fail without side effects.
