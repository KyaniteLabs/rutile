# Reader, Edit, and Split modes

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 02

## What it delivers

Explicit View, Edit, and Split modes for the macOS product. The modes change
presentation and input routing while retaining one authoritative document,
revision, selection, dirty state, and preview.

## Acceptance criteria

- Each mode has an accessible name, deterministic entry/exit action, and
  keyboard path.
- View mode makes the rendered document primary and prevents accidental edits.
- Edit mode preserves existing editor, IME, undo/redo, and dirty behavior.
- Split mode composes the existing editor and preview without a second buffer.
- Scroll synchronization remains bounded, revision-aware, and free of echo
  loops.
- Mode changes preserve valid selection, scroll, document identity, and dirty
  state.
