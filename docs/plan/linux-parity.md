# Linux daily-driver parity

Status: **plan (not started on a Linux host).** Authority:
`docs/handoff/current-state.md`. Companion: `docs/platform-parity.md`.

macOS is the primary shipping shell. Linux is a real GTK3/WebKitGTK product
shell, not a stub — but chrome lagged while table-stakes landed on AppKit/Iced.

`linux-gtk` **does not compile on macOS** (`compile_error` in
`crates/rutile-app/src/lib.rs`). Do not implement or claim Linux chrome from a
Mac. Land every PR below on a Linux machine that can run:

```
cargo test --locked -p rutile-app --no-default-features --features linux-gtk
bash scripts/rutile-linux-gate.sh
```

## Shared already (do not reimplement)

These live in the reducer / session core and already apply on Linux:

- `DocumentSessionCore` park/swap, D7 `RequestTabCloseDecision`, last-tab no-op
- `adopt_opened_document` (D4)
- autosave store inherit onto every slot
- export print-preset splice
- find, format, save, preview, security core

`LinuxProductSession::open_via_shared_command` and
`GtkSourceEditorAdapter::install_open_snapshot` exist. `project_tabs` is the
shared strip projection. There is no second tab model.

## What Linux paints today

| Surface | Linux now |
|---|---|
| Editor + preview | GtkSourceView + WebKitGTK |
| Format | Format menu + toolbar + Ctrl+keys |
| Find | Edit ▸ Find / Ctrl+F |
| File Open / Save / Save As / Close menus | **missing** (Ctrl+S and HTML export exist; `FileChooserNative` already used for HTML) |
| CLI `rutile note.md` | **discarded** in `main.rs` (`let _ = path`) |
| Tabs | **none** (data plane only) |
| Command palette | **none** |
| View modes | **none** (toolbar toggle only) |
| Recents | **none** |
| Focus hides chrome | **no** |

Intentional forever-differences: Cmd vs Ctrl, Linux Format menu vs macOS
palette-first, Code Block shortcut. Multi-file drop still differs (macOS
rejects >1; Linux opens first).

## Sequence (one Forgejo PR each)

Parity means **same contracts, native GTK widgets**. Not iced-on-Linux. Not NSPanel.

### P0 — File is a real app

1. Stop dropping the CLI path. After GTK window build, call
   `open_via_shared_command` + `install_open_snapshot`.
2. File ▸ Open / Save / Save As / Close. Reuse `FileChooserNative`. Window
   close remains quit. Last-tab Close Tab stays a no-op.

### P1 — Tabs

3. Window ▸ New Tab / Close Tab / tab list from `project_tabs`.
4. `gtk::Box` strip above the paned view. Click → `SwitchTab` / D7 `CloseTab`.
   After every swap: `install_open_snapshot` + reschedule render (Linux
   `refresh_after_document_swap`).
5. Last tab: no ×. Dirty close: existing GTK dirty dialog, then
   `TabCloseDecided`. Never `QuitApplication`.

### P2 — Rest of the macOS day

6. View ▸ Editor / Split / Reading → `DocumentMode`.
7. Palette: `GtkWindow` or `GtkPopover` bound to `palette_snapshot()`. Do not
   re-rank. Ctrl+Shift+P. Esc closes and focuses the editor.
8. Recents from `RecentDocuments`.
9. Focus mode hides strip + toolbar.

### Leave for later / other machines

- Wayland and RPM-host evidence
- D8 restore of every open path (both platforms)
- Quality-probe GUI attestation
- GUI-stack unification

## Verification per PR

Workspace gate plus, on Linux:

```
cargo test --locked -p rutile-app --no-default-features --features linux-gtk
bash scripts/rutile-linux-gate.sh
```

Frozen files stay frozen. No fabricated lifecycle receipts.

## First PR if a Linux host is available

CLI path + File ▸ Open. Smallest user-visible hole. Then the GTK strip.
