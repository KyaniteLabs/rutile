# Reader-First View Mode (Roadmap 04) — Design Decisions

Status: **locked (contract implemented; native View-menu items wired)**. Parent
issue: `.scratch/rutile-macos-roadmap/issues/04-reader-first-view-mode.md`.
Blocked by 03 (DONE).

## Resolved decisions

### V1 — Three modes: Edit, Split, View

`DocumentMode::{Edit, Split, View}`:

- **Edit** — the source editor is the sole surface. The preview is hidden. The
  editor owns selection, scroll, and keyboard focus. This is the heads-down
  writing mode.
- **Split** *(default)* — editor and preview are both visible side by side. This
  is the current baseline behavior; the source editor remains the input owner.
- **View** — the rendered preview is the sole surface (reader mode). The editor
  is hidden. The document is effectively read-only: edits arrive only through
  external change/recovery, never from a hidden editor.

Default is `Split` to preserve the existing baseline.

### V2 — Mode is a shell-level view setting, not document content

`DocumentMode` lives on `AppState` as a single field, not per-tab. It is a view
affordance, like zoom or theme. A future revision may make it per-document
(remembered per tab); the contract changes from `DocumentMode` to a slot field
without breaking the enum or messages.

### V3 — Free transitions; preservation is the shell's job

Any mode → any mode is allowed (no gating). Selection, scroll position, focus,
and accessibility context are owned by the platform surfaces, not the reducer.
The reducer records the mode; the shell, on the next view update, transfers
selection/scroll between the surfaces it already maintains. The contract
therefore duplicates no document or render logic — it only declares the mode.

### V4 — Preview/editor ownership

- In **View**, the preview is the sole owner: link clicks, scroll sync, and
  VoiceOver cursors target the preview. The shell does not route keyboard text
  into a hidden editor.
- In **Edit**, the editor is the sole owner; the preview is absent.
- In **Split**, the editor is the input owner and the preview mirrors it via the
  existing render pipeline (unchanged baseline).

### V5 — Single reducer message

`AppMessage::SetDocumentMode { mode }` is the only mode transition. The shell
sends it from menu items, keyboard shortcuts, or a future toggle button. The
reducer sets the field and emits no effects — the shell reads
`AppState::mode()` on its next render. Keeping it effect-free means mode changes
never trigger a render/autosave cycle on their own.

### V6 — Read-only enforcement is the shell's responsibility

In View mode the document is *effectively* read-only because the editor is
hidden, but the reducer does not reject edits — external changes, recovery, and
session restore can still mutate state regardless of mode. Mode is presentation,
not a permission gate. (A future hard lock — e.g. for AI boundary or trusted
review — would be a separate `DocumentLock` contract, deliberately not folded
into view mode.)

### V7 — Accessibility semantics

Each mode maps to a distinct AX surface: Edit exposes a text editor role, View
exposes a document/reader region, Split exposes both with the editor as the
keyboard-focusable peer. The shell publishes these from `AppState::mode()`; the
contract carries no AX code (kept out of the headless layer).

### V8 — macOS View menu

A View menu offers Edit / Split / View with checkmark-on-active and optional
shortcuts (⌃⌘1 / ⌃⌘2 / ⌃⌘3). Selecting an item sends `SetDocumentMode`. The
menu is rebuilt when the mode changes so the checkmark tracks the reducer state.
