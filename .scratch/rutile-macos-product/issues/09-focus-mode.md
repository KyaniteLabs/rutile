# Focus mode

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 03

## What it delivers

A reversible distraction-reduced presentation mode that keeps essential save,
find, recovery, conflict, and exit actions available.

## Acceptance criteria

- Focus mode changes presentation only and does not fork document state.
- Entry and exit work from keyboard and accessible controls.
- Dirty, conflict, recovery, and save notices remain discoverable.
- The prior non-focus layout is restored deterministically.
- Focus mode does not regress editor/preview synchronization or window
  lifecycle behavior.
