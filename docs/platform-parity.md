# Platform Parity — Intentional Asymmetries (M5, L8)

> **Audit refs**: M5 (menu structure divergence), L8 (keyboard shortcut divergence).
> macOS is the primary shipping platform. Linux-GTK (`linux_gtk.rs`) is a real
> native shell compiled only on Linux (`cfg(target_os="linux")`), exercised by
> the lifecycle gate. Menu/shortcut differences below are either intentional
> convention or an unimplemented chrome gap.
>
> Sequenced close-the-gap plan: [`docs/plan/linux-parity.md`](plan/linux-parity.md).

## Menu structure

### macOS (`open_events.rs`)

Native AppKit `NSMenu` hierarchy installed on the main menu bar:

| Menu   | Items                                                          |
|--------|----------------------------------------------------------------|
| File   | Open… ⌘O, Open Recent ▸, Save ⌘S, Save As… ⇧⌘S, Close ⌘W     |
| View   | Editor ⌃⌘1, Split ⌃⌘2, Reading ⃰⌘3                            |
| Window | New Tab ⌘T, Close Tab ⌃⌘W, Command Palette… ⇧⌘P, Tabs ▸      |

**No Format menu**: formatting on macOS is driven by the command palette
(⇧⌘P) and keyboard shortcuts. The format toolbar is rendered by iced inside
the source pane, not as a native menu.

### Linux-GTK (`linux_gtk.rs`)

GTK `GtkMenuBar` hierarchy:

| Menu   | Items                                                                        |
|--------|------------------------------------------------------------------------------|
| File   | Export HTML…                                                                  |
| Edit   | Find / Replace Ctrl+F                                                        |
| Format | Bold Ctrl+B, Italic Ctrl+I, Link Ctrl+K, Inline Code Ctrl+E, Code Block Ctrl+Shift+C, Cycle Heading Ctrl+Shift+H, Quote Ctrl+Shift+Q, Bullet List Ctrl+Shift+U, Numbered List Ctrl+Shift+O, Checklist Ctrl+Shift+L, Smart New Line Enter |
| View   | Formatting Toolbar (toggle)                                                   |

**No Open/Save/Save As/Close menu items**: Linux-GTK is a CI-only build.
File I/O is tested through the session/autosave contracts and the
functional test windows (save, reopen, scroll, inspect), not through
menu-driven flows. Adding these menus would require wiring native file
dialogs through GTK, which is out of scope for a CI-only target.

**No New Tab / Command Palette / View Modes**: the shared reducer owns
tabs, palette, and view mode. macOS paints them (Iced strip, `NSPanel`,
View menu). Linux-GTK still has no Window tab menu, no GTK strip, and no
palette panel. That is an unimplemented Linux chrome gap, not a second
tab model.

**No Open Recent submenu**: the recent-documents list is managed by
`NSDocumentController` on macOS. GTK has no equivalent in the CI build.

## Keyboard shortcuts

### Intentional platform-convention divergence

macOS uses ⌘ (Command) as the primary modifier. Linux uses Ctrl. This
is standard cross-platform convention — the same key position on the
physical keyboard, different modifier label.

| Action          | macOS         | Linux           |
|-----------------|---------------|-----------------|
| Bold            | ⌘B            | Ctrl+B          |
| Italic          | ⌘I            | Ctrl+I          |
| Link            | ⌘K            | Ctrl+K          |
| Inline Code     | ⌘E            | Ctrl+E          |
| Code Block      | ⌘⇧K (via palette) | Ctrl+Shift+C |
| Find            | (via palette) | Ctrl+F          |

The Code Block shortcut differs because:
- macOS: ⌘⇧K is the standard "Code Block" binding in macOS editors.
- Linux: Ctrl+Shift+C is the standard "Code Block" binding in Linux
  editors (Ctrl+Shift+K is less common and may conflict with terminal
  emulators).

These are documented divergences, not bugs. A future shared
`PlatformMenu` abstraction could unify the model, but the modifier
labels will always differ by platform convention.

## Multi-file open (M7)

| Platform | Policy                                                      |
|----------|-------------------------------------------------------------|
| macOS    | Rejects all files when >1 URL is dropped/selected.          |
| Linux    | Opens the first file, warns about ignored extras.          |

The macOS rejection is stricter because `NSOpenPanel` supports
multi-select and one drop still maps to one open (D4 then parks or
switches). The Linux behavior is more permissive for CI testing. Both
are documented; aligning requires a product decision about multi-file UX.
