# Outline navigator

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 03

## What it delivers

A heading outline for the active document that supports deterministic section
navigation in View, Edit, and Split modes.

## Acceptance criteria

- The outline derives from the same bounded Markdown/render representation as
  the preview.
- Heading order, nesting, duplicate titles, malformed input, and heading-free
  documents have deterministic behavior.
- Selecting an item moves the correct pane without changing document revision
  or identity.
- The navigator has an honest empty state and an accessible tree/list model.
- Stale outline selections cannot move a newer document revision.
