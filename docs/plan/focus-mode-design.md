# Focus Mode (Roadmap 10) — Design Decisions

Status: **locked (contract implemented; native chrome hide/show deferred)**.
Parent issue: `.scratch/rutile-macos-roadmap/issues/10-focus-mode.md`.
Blocked by 03 (DONE), 04 (DONE).

## Resolved decisions

### F1 — Focus is a coordinated shell state, not a third view mode

`FocusMode` is a single boolean on `AppState`: `focused`. It is **orthogonal**
to `DocumentMode` (roadmap 04): the mode decides *which surfaces* show
(editor/preview/split); focus decides *whether the chrome around them shows*.
So Edit+Focus is a chrome-less editor; View+Focus is a chrome-less reader;
Split+Focus is editor+preview with no toolbar/tab bar/sidebar. This avoids a
Mode × Focus combinatorial explosion — the two flags compose freely.

### F2 — What focus hides

When `focused`, the shell hides: the toolbar, the tab strip / Window-menu
chrome, the notices banner, and any sidebar (outline/search). It keeps: the
active document surface(s), the cursor, the caret, and keyboard input. Hiding is
visual only — the reducer state (notices, tabs, recents) is unchanged; exiting
focus restores everything without loss.

### F3 — Single toggle message

`AppMessage::ToggleFocusMode` flips the flag. The reducer emits no effects — the
shell reads `AppState::focused()` on its next render and hides/shows chrome.
There is no "enter" vs "exit" message; one toggle does both, so the same shortcut
and palette command enter and leave focus. This mirrors how `SetDocumentMode` is
effect-free.

### F4 — Escape paths

The shell binds **Esc** and the toggle shortcut (and the palette command) to
`ToggleFocusMode`. Because focus is a boolean, pressing the toggle again always
exits — there is no "trapped in focus" state. The shell must ensure at least one
visible, keyboard-reachable exit at all times (Esc is always wired).

### F5 — Not persisted (v1)

Focus is ephemeral: it does not enter session state and resets to unfocused on
restore. Rationale: focus is a momentary writing preference, and persisting it
would surprise a user who launches into a chrome-less window. A future revision
may persist it behind an explicit opt-in.

### F6 — Reduced-motion behavior is the shell's job

Focus transitions (chrome slide/fade) are visual. Under reduced-motion the shell
makes them instant (no animation). The contract carries no timing or animation —
it is a boolean the shell animates (or doesn't) as accessibility requires.

### F7 — Interaction with command palette and menus

The command palette remains accessible in focus (it is keyboard-driven, not
chrome). Menu-bar menus remain functional (macOS never truly hides the menu bar
outside fullscreen). So a focused user can still invoke any command, switch
modes, or open files without leaving focus.

### F8 — Palette + macOS wiring

A `view.toggle-focus` command is added to the default catalog so the palette can
enter/exit focus. A macOS menu item (View → Focus, or a global shortcut) sends
`ToggleFocusMode`. The actual chrome hide/show animation in the tiny-skia
rendering pipeline is deferred native work — the boolean and the message path
are exercisable now.
