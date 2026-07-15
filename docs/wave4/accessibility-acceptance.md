# Wave 4 — Accessibility Acceptance Contract

> **Status: contract.** This document transcribes the Wave 4 accessibility
> requirements from the remediation plan into a tracked, testable acceptance
> contract. It is the reference the release gate checks against. It does **not**
> certify current behavior; the gap analysis lives in
> [`accessibility-audit.md`](accessibility-audit.md) and the validation items not
> yet run are registered in [`evidence-debt.md`](evidence-debt.md).

## Authority

- Remediation plan, Wave 4 — "supportability, accessibility, and performance
  validation" (`rutile-full-remediation-ralplan.md` lines 269-281).
- Finding `A11Y-001` (A11y + platforms, Waves 2/4): keyboard + VoiceOver + AT-SPI
  action matrix → signed manual/automated receipt; W4/release.
- Finding `MAC-004` (macOS + A11y, Waves 2B/4): modal focus trap; edits blocked;
  stale recovery rejected → VoiceOver/native e2e; W4/release.
- Earlier shell guard: "Keep toolbar controls keyboard-focusable with accessible
  names" (plan line 240).

Verbatim requirements from the plan:

> Replace pseudo-fields/dialogs with semantic accessible controls; validate
> VoiceOver and AT-SPI manually and with available automation.
>
> Establish keyboard-only acceptance matrix for every action.
>
> Accessibility acceptance covers every registered action plus, at minimum,
> open/save/save-as, find/replace, mode/focus transfer, conflict, recovery,
> autosave error, and dirty close using keyboard-only on both platforms,
> VoiceOver on macOS, and AT-SPI/Orca on Linux. Zero keyboard traps or
> unlabeled actionable controls are allowed.

## 1. Invariants (must hold for every screen and state)

| ID | Invariant | Pass criterion |
|---|---|---|
| INV-1 | Zero keyboard traps | From any control or modal state, the user can reach every other actionable control and return to the editor using only the keyboard, without a pointing device, and without being stranded. |
| INV-2 | Zero unlabeled actionable controls | Every actionable control (button, menu item, text field, checkbox, dialog button, chooser) exposes a programmatic accessible **name** to the platform accessibility API (NSAccessibility / AT-SPI). Visible glyph-only or unnamed controls fail. |
| INV-3 | Semantic controls, not pseudo-fields | Text input is a real editable text widget (NSTextField/text editor / GtkEntry / GtkSourceView), not drawn text with a hand-rolled keystroke handler. Decisions are real modal dialogs/sheets, not window-title text plus hidden key bindings. |
| INV-4 | Status/notice announcements | Errors, recovery prompts, and autosave failures are surfaced through a live/alert region or announcement the screen reader speaks, not solely through a window-title change. |
| INV-5 | Visible focus | Keyboard focus is visually indicated at all times on the focused control. |

## 2. Platform validation requirements

| Platform | Assistive technology | How |
|---|---|---|
| macOS (`aarch64-apple-darwin`) | VoiceOver | Manual pass with VoiceOver enabled (⌘F5), plus automation where available (`Accessibility Inspector`, `xcuitest` accessibility audit, or AT-SPI-equivalent programmatic query of the NSAccessibility tree). |
| Linux (`x86_64-unknown-linux-gnu`, GTK) | AT-SPI via Orca | Manual pass with Orca enabled, plus programmatic AT-SPI queries (`accerciser`, `atspi` over D-Bus, or an `atspi`-backed test) to assert roles/names/states. |

A run under only one platform/AT does not close the matrix. Both platforms and
both AT stacks are required for every flow in §3.

## 3. Keyboard-only acceptance matrix (both platforms, no pointing device)

Each flow must be completable with the keyboard alone **and** be correctly
announced/navigable under the platform AT. `K` = keyboard-only path;
`VO` = VoiceOver; `AT` = AT-SPI/Orca.

| Flow | Keyboard path (macOS) | Keyboard path (Linux/GTK) | K | VO | AT |
|---|---|---|---|---|---|
| Open document | ⌘O / File ▸ Open… | File ▸ Open… / Ctrl+O | ☐ | ☐ | ☐ |
| Save (path-bound) | ⌘S / File ▸ Save | Ctrl+S | ☐ | ☐ | ☐ |
| Save As (untitled) | ⌘S on untitled → Save As; File ▸ Save As… | Ctrl+S on untitled → Save As dialog | ☐ | ☐ | ☐ |
| Find | ⌘F; type; ⌘G / ⌘⇧G next/prev | Ctrl+F; type; Enter / Shift+Enter next/prev | ☐ | ☐ | ☐ |
| Replace | ⌘⇧F; Tab to Replace; Enter replace; ⌘⇧Enter replace all | Find bar Replace field; Replace / All buttons | ☐ | ☐ | ☐ |
| Mode/focus transfer | Tab/Shift-Tab between editor, toolbar, find bar, preview pane | Tab/Shift-Tab between editor, menubar, toolbar, find bar, preview | ☐ | ☐ | ☐ |
| Conflict (external change) | Status announced; reload (⌘R) / save-elsewhere reachable | Status announced; Ctrl+Shift+R reload / Ctrl+Shift+K keep | ☐ | ☐ | ☐ |
| Recovery (crash journal) | ⌘Y restore / Esc dismiss, announced as a decision | Recovery dialog (Recover/Discard), modal focus | ☐ | ☐ | ☐ |
| Autosave error | Failure announced as alert; re-save (⌘S) reachable | Failure announced as alert; retry dialog reachable | ☐ | ☐ | ☐ |
| Dirty close | Save / Discard / Cancel decision, modal, focus trapped | Save / Discard / Cancel dialog, modal, focus trapped | ☐ | ☐ | ☐ |
| Every other registered format action | ⌘B/I/K/E, ⌘⇧K/H/B/L/O/X | Ctrl+B/I/K/E, Ctrl+Shift+C/H/Q/U/O/L | ☐ | ☐ | ☐ |
| Toolbar toggle | ⌘⇧T | View ▸ Formatting Toolbar (CheckMenuItem) | ☐ | ☐ | ☐ |

Every registered action beyond the minimum set (format commands, HTML export,
copy-as-HTML, undo/redo, smart paste, smart enter) must be added to the matrix
with its actual accelerator/menu path before release; "every registered action"
in the plan is the scope, the table is the minimum seed.

## 4. MAC-004 specific acceptance (recovery modal focus trap)

These must all hold on both platforms under the AT stack:

1. While a recovery decision is pending, **buffer edits are blocked** (the
   editor is not mutable) and this is announced to the AT.
2. The recovery decision is a **modal focus trap**: focus moves into the dialog
   and cannot leave it until Restore/Dismiss is chosen.
3. A **stale recovery** (journal older than the loaded document) is rejected and
   not offered; the rejection is silent-or-announced per design but never blocks
   editing.
4. The two decision keys (Restore / Dismiss) are the only accepted keys while
   pending; all other keys are consumed (no escape into the buffer).

## 5. Per-action pass/fail evidence shape

Each row of the §3 matrix is closed by an evidence record conforming to the
**`rutile.accessibility-attestation.v1`** schema. The schema file is not yet
present in `schemas/`; its absence is registered as evidence-debt (see
[`evidence-debt.md`](evidence-debt.md) item ED-A11Y-0). Until it exists, the
shape each record must satisfy is fixed here so collection is unambiguous:

```jsonc
{
  "schema": "rutile.accessibility-attestation.v1",
  "flow": "dirty-close",                       // one of the §3 flow slugs
  "action": "save",                             // granular action within the flow, or "*"
  "platform": "macos",                          // "macos" | "linux"
  "architecture": "aarch64-apple-darwin",
  "at_stack": "voiceover",                      // "voiceover" | "atspi-orca"
  "keyboard_only": true,                        // no pointing device used
  "source": {                                   // git provenance of the tested build
    "commit": "<40-hex>",
    "tree": "<40-hex>",
    "dirty": false
  },
  "result": "pass",                             // "pass" | "fail"
  "checks": {
    "keyboard_path_complete": true,             // INV-1: no trap, every step reachable
    "control_labeled": true,                    // INV-2: programmatic name present
    "semantic_control": true,                   // INV-3: real widget, not pseudo-field
    "status_announced": true,                   // INV-4: alert/live region spoken
    "visible_focus": true,                      // INV-5: focus ring/indicator shown
    "focus_trap_enforced": null                 // truthy for modal flows (MAC-004), null otherwise
  },
  "method": "manual+automation",                // "manual" | "automation" | "manual+automation"
  "artifacts": [
    {
      "kind": "atspi-dump",                     // e.g. accerciser tree, AXInspector dump, screen recording
      "path": "run-<n>/dirty-close-macos-vo.json",
      "sha256": "<64-hex>"
    }
  ],
  "operator": "<name>",
  "captured_unix_ms": 0
}
```

Validation command (once the schema is added):

```
cargo run -p xtask -- evidence validate \
  --input <attestation.json> --schema accessibility-attestation
```

A `fail` result for any `checks.*` blocks the flow. A flow with no attestation
is treated as `fail` (absence is not pass).

## 6. Exit gate (Wave 4 accessibility)

The Wave 4 accessibility exit gate is met only when **all** are true:

1. Every row of the §3 matrix is `pass` on **both** macOS/VoiceOver and
   Linux/AT-SPI-Orca, evidenced by a valid `accessibility-attestation.v1` record.
2. INV-1 through INV-5 hold on both platforms (verified by the matrix + a
   dedicated zero-trap / zero-unlabeled sweep).
3. MAC-004 §4 acceptance is satisfied with a passing VoiceOver/native receipt.
4. All pseudo-fields and pseudo-dialogs identified in the audit are either
   replaced with semantic controls or explicitly accepted with owner + expiry
   (an acceptance does not remove the AT validation requirement for the flow).
5. The `accessibility-attestation.v1` schema exists and every attestation
   validates against it.

Partial passes (one platform green, the other pending) do not satisfy the gate;
the pending platform remains evidence-debt.
