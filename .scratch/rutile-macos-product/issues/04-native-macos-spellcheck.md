# Native macOS spellcheck

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 03

## What it delivers

Native macOS spelling assistance at the source-editor boundary, with explicit
user acceptance for replacements and no remote service.

## Acceptance criteria

- Native spelling underlines, menus, and replacement gestures are available
  in a real macOS interaction path.
- Spellcheck annotations never mutate saved Markdown without explicit user
  acceptance.
- Accepted replacements use the normal revisioned edit and undo contracts.
- Accessibility exposes the relevant spelling state and controls.
- The editor remains usable and explains the degraded state when the native
  service is unavailable.
