# Wave 4 — Accessibility Audit of the Native Adapters (READ-ONLY)

> **Scope:** a static, read-only audit of the two production UI adapters —
> `crates/feathermark-app/src/platform/macos/native.rs` (with the macOS File
> menu in `crates/feathermark-app/src/platform/macos/open_events.rs`) and
> `crates/feathermark-app/src/platform/linux_gtk.rs`. No production code was
> changed for this audit. Each control is classified by (a) whether it exposes a
> programmatic accessible name/role, (b) whether it is keyboard-focusable, and
> (c) whether it is a semantic platform control or a pseudo-field/pseudo-dialog.
> Line numbers are against the tree checked out for Wave 4.
>
> This audit documents the gap; it does **not** validate fixes. Interactive
> VoiceOver/AT-SPI validation is registered in [`evidence-debt.md`](evidence-debt.md).

Legend: ✓ = meets the property in code; ✗ = missing/gap; △ = partial / relies on
an indirect mechanism that needs interactive confirmation.

---

## A. macOS adapter — `platform/macos/native.rs`

### Rendering model (load-bearing for the whole audit)

The macOS shell does **not** host platform-native controls. The source pane is
an **iced** program rendered through a custom tiny-skia compositor into a winit
window (`native.rs:2170` `IcedCompositor`, `native.rs:2505`-`2560`
`draw_and_present`, `native.rs:2575` `block_on_ready`). Only the AppKit **menu
bar** (`open_events.rs`) and the **open/save panels** are real AppKit objects;
every editor-pane control (editor, toolbar, find bar, status, notices) is a
drawn iced subtree with **no NSAccessibility bridging** in code.

**Consequence:** VoiceOver, which reads the NSAccessibility tree, is not
presented with the editor-pane controls at all. This single architectural gap is
the root cause behind most ✗ marks below; closing it (bridging the iced a11y
tree to NSAccessibility, or surfacing NSAccessibility proxy elements) is a
prerequisite for any macOS pass.

### A.1 Actionable controls

| Control | Location | Accessible name/role | Keyboard-focusable | Semantic control? |
|---|---|---|---|---|
| File menu: Open… / Save / Save As… / Close | `open_events.rs:110`-`125` (`NSMenuItem`, title + key equivalent) | ✓ (AppKit menu item) | ✓ (menu bar navigation; ⌘O/⌘S/⌘⇧S/⌘W equivalents at `open_events.rs:110`-`115`) | ✓ |
| Open panel (`NSOpenPanel`) | `native.rs:2625`-`2640` `choose_open_path` | ✓ (system panel) | ✓ | ✓ |
| Save panel (`NSSavePanel`) | `native.rs:2646`-`2658` `choose_save_path` | ✓ (system panel) | ✓ | ✓ |
| Source editor | `native.rs:2137`-`2141` (`text_editor`, id `feathermark-source-editor`) | ✗ — no NSAccessibility element surfaced; iced a11y tree not bridged to AppKit | ✓ within the iced focus chain (`native.rs:2355`-`2358` focus op; editor focused on window focus at `native.rs:1562`-`1567`) | △ — semantic iced widget, but rendered off the platform AT tree |
| Toolbar buttons: Bold/Italic/Code/Heading/Quote/List/Ordered/Checklist | `native.rs:2006`-`2015` (`TOOLBAR_ITEMS`); rendered `native.rs:2082`-`2090` | ✗ — no `accessibility_label` set; name would come only from the visible text, and none of it is on the NSAccessibility tree | △ — focusable in iced's focus chain, but no explicit Tab-order wiring between toolbar and editor in code | ✗ as seen by VoiceOver (iced `button` rendered via custom compositor) |
| Find/replace bar — query + replace fields | `native.rs:2094`-`2133` (rendered as `iced_widget::text` lines `"▸ Find: …"` / `"▸ Replace: …"`) | ✗ — drawn text, no field name/role | ✗ at the AT level — "focus" is internal state (`FindField::Query`/`Replace`, `native.rs:1932`-`1937`) toggled by Tab (`native.rs:1001`-`1004`), not real widget focus | **✗ PSEUDO-FIELD** — input is a hand-rolled char-by-char handler (`native.rs:810`-`826` `find_input`, `native.rs:828`-`846` `find_backspace`), not an editable text widget |
| Find/replace bar — replace-all / next / prev | No on-screen buttons; bound to keys (⌘⇧Enter replace-all, Enter/⌘G next/prev, `native.rs:967`-`1000`) | ✗ — no control to label | ✓ via keys | ✗ — no visible/AT-actionable control (key bindings only) |

### A.2 Decision flows (pseudo-dialogs)

| Flow | How it is surfaced | Dialog role? | Focus trap? |
|---|---|---|---|
| Dirty close (Save / Discard / Cancel) | window title (`native.rs:351`-`354`) + key handlers ⌘S/⌘D/Esc (`native.rs:1579`-`1641`) | **✗ pseudo-dialog** — title text only, no modal/`NSAlert`/sheet | △ — keys are consumed while `pending_close` (`native.rs:1579`), which blocks edits but is not an accessible modal focus trap |
| Recovery (Restore / Dismiss) — MAC-004 | window title (`native.rs:355`-`358`) + ⌘Y/Esc (`native.rs:1647`-`1662`); edits blocked (`native.rs:406`-`426`) | **✗ pseudo-dialog** — no accessible alert/modal | △ — decision keys are the only accepted keys (`native.rs:1661`), enforcing the trap by event swallowing, but no AT-perceivable dialog |
| Conflict (external change) | window title only (`native.rs:359`-`362`) | **✗ pseudo-notification** | n/a |
| Autosave error / notices | `surface_error` → window title (`native.rs:500`-`505`) | **✗ pseudo-alert** — no live/alert region, no announcement | n/a |

### A.3 Non-actionable chrome (informational, listed for completeness)

- Status bar (words/chars/read time): `native.rs:2147`-`2160`, `iced_widget::text`.
  Read-only; not exposed to VoiceOver. Counts change is not announced.
- Find status text ("Match at byte N" / "No matches"): `native.rs:2121`-`2126`,
  `iced_widget::text`; not announced.

---

## B. Linux adapter — `platform/linux_gtk.rs`

The GTK shell uses real GTK widgets, so most controls are already semantic and
labeled. Gaps are narrower than macOS.

### B.1 Actionable controls

| Control | Location | Accessible name/role | Keyboard-focusable | Semantic control? |
|---|---|---|---|---|
| Source editor (`sourceview4::View`) | `linux_gtk.rs:2549`-`2556`; `accessible.set_name(SOURCE_EDITOR_LABEL)` at `linux_gtk.rs:2554`-`2556` | ✓ explicit name | ✓ (`grab_focus` at `linux_gtk.rs:3256`) | ✓ (GtkSourceView text role) |
| Format toolbar buttons (8) | `linux_gtk.rs:3696`-`3725`; a11y at `linux_gtk.rs:3709`-`3719` | ✓ `accessible.set_name(label)` + `set_role(PushButton)` (`linux_gtk.rs:3716`-`3718`); `set_can_focus(true)` + `set_focus_on_click(true)` (`linux_gtk.rs:3714`-`3715`) | ✓ | ✓ — the code comment explicitly records that a prior `set_can_focus(false)` bug (which removed the buttons from keyboard navigation) was fixed (`linux_gtk.rs:3709`-`3713`) |
| Menu bar + items (File/Edit/Format/View) | `linux_gtk.rs:3730`-`3853` (`gtk::MenuItem::with_label` / `CheckMenuItem::with_label`) | ✓ (GTK menu items expose label to AT-SPI) | ✓ (menu bar navigation; accelerators noted in labels e.g. `"Bold   Ctrl+B"` at `linux_gtk.rs:3808`) | ✓ |
| Find bar — search entry | `linux_gtk.rs:3872`-`3873` (`gtk::Entry`, placeholder `"Find"`) | △ — name derived from placeholder text only; **no explicit `accessible.set_name` / label relationship** | ✓ (`Ctrl+F` → `grab_focus` at `linux_gtk.rs:2921`-`2925`) | ✓ real entry widget |
| Find bar — replace entry | `linux_gtk.rs:3874`-`3875` (`gtk::Entry`, placeholder `"Replace with"`) | △ — placeholder-only name, no explicit label | ✓ (Tab from search) | ✓ |
| Find bar — Prev / Next / Replace / All / Close buttons | `linux_gtk.rs:3876`-`3880` (`gtk::Button::with_label`) | ✓ visible text labels serve as AT-SPI name (no explicit `set_name`, but labels are present and unambiguous) | ✓ | ✓ |
| Dirty-close dialog (Cancel/Discard/Save) | `linux_gtk.rs:3518`-`3539` `prompt_dirty_close` (`gtk::MessageDialog`, modal, `set_default_response(Cancel)` at `3529`) | ✓ (system message dialog) | ✓ (modal grabs focus) | ✓ |
| Save-As chooser | `linux_gtk.rs:3541`-`3563` `prompt_save_path` (`gtk::FileChooserNative`) | ✓ | ✓ | ✓ |
| Recovery dialog (Discard/Recover) | `linux_gtk.rs:3650`-`3664` `prompt_recover` (`gtk::MessageDialog`) | ✓ | ✓ | ✓ |
| Close-save-retry dialog (autosave/save-error path) | `linux_gtk.rs:2359`-`2378` `prompt_close_save_retry` (`gtk::MessageDialog`, Error type, `set_default_response(Retry)` at `2370`) | ✓ | ✓ | ✓ |
| Conflict resolution (reload / keep buffer) | key-only: Ctrl+Shift+R reload (`linux_gtk.rs:2818`-`2833`), Ctrl+Shift+K keep (`linux_gtk.rs:2834`-`2842`) | ✗ — no on-screen control or menu item to label; status surfaced via window title | ✓ via keys | ✗ — key bindings only, no actionable control surfaced to AT-SPI |
| Status bar / notices | `linux_gtk.rs:2634`-`2642` (`gtk::Label`); notice text set via `status_bar.set_text` (`linux_gtk.rs:2129`, `2138`) | △ — `gtk::Label` is exposed to AT-SPI as text, but is **not** an `ATSPI_ROLE_NOTIFICATION_BAR`/live region, so changes are not announced by Orca | n/a (read-only) | △ |

### B.2 Menu coverage gap

The File menu contains only HTML-export items (`linux_gtk.rs:3739`-`3791`:
"Save as HTML…", "Copy as HTML"). There is **no File ▸ Open, File ▸ Save, or
File ▸ Save As** menu item, and **no `Ctrl+O` key binding**. The only `keys::o`
in the key handler is `Ctrl+Shift+O` → `ToggleOrderedList`
(`linux_gtk.rs:2947`). Markdown open is reachable only through OS delivery
(`application.connect_open` at `linux_gtk.rs:2426`) and drag-drop; in-app
keyboard open is absent.

---

## C. Consolidated gap list

| Gap | Adapter | File:line | Inv violated | Severity |
|---|---|---|---|---|
| G-1 | macOS | `native.rs:2170`, `2505`-`2560` (custom compositor, no NSAccessibility bridge) | INV-2, INV-3 (all editor-pane controls invisible to VoiceOver) | **Blocker** — every macOS AT check depends on this |
| G-2 | macOS | `native.rs:2094`-`2133`, `1932`-`1937`, `810`-`846`, `1001`-`1004` (find/replace is drawn text + char handler) | INV-3 (pseudo-field) | **Blocker** for the find/replace flow on macOS |
| G-3 | macOS | `native.rs:351`-`362`, `1579`-`1641`, `1647`-`1662` (dirty-close / recovery / conflict are title + key handlers) | INV-3, INV-4 (pseudo-dialogs, no alert) | **Blocker** for MAC-004 + dirty-close/recovery/conflict on macOS |
| G-4 | macOS | `native.rs:500`-`505` (notices/autosave-error via `surface_error` → title) | INV-4 (no live/alert region; nothing announced) | **Blocker** for the autosave-error flow on macOS |
| G-5 | macOS | `native.rs:2006`-`2015`, `2082`-`2090` (toolbar buttons: no explicit accessible name, no AT exposure) | INV-2 | High (subsumed by G-1 until the bridge exists) |
| G-6 | macOS | `native.rs:2137`-`2141`, `2355`-`2358` (editor not on NSAccessibility tree) | INV-2 | High (subsumed by G-1) |
| G-7 | macOS | whole pane — no Tab-order wiring between toolbar, editor, find bar, preview | INV-1, INV-5 | High (mode/focus-transfer flow) |
| G-8 | Linux | `linux_gtk.rs:3739`-`3791`, `2947` (no File ▸ Open / no Ctrl+O) | INV-1 (open flow has no keyboard path) | High for the open flow |
| G-9 | Linux | `linux_gtk.rs:3872`-`3875` (find/replace entries: placeholder-only name, no explicit `set_name`) | INV-2 | Medium |
| G-10 | Linux | `linux_gtk.rs:2634`-`2642`, `2129`, `2138` (status/notices are a plain `gtk::Label`, not a live/alert region) | INV-4 | Medium |
| G-11 | Linux | `linux_gtk.rs:2818`-`2842` (conflict resolve is key-only; no actionable control or menu item for AT-SPI) | INV-2, INV-4 | Medium |
| G-12 | Both | `schemas/` has no `rutile.accessibility-attestation.v1.schema.json` | (evidence shape) | Medium — tracked as ED-A11Y-0 |

### What is already correct (no action needed beyond interactive confirmation)

- macOS AppKit File menu and `NSOpenPanel`/`NSSavePanel` are native, labeled, and
  keyboard-reachable (`open_events.rs:110`-`125`; `native.rs:2625`-`2640`,
  `2646`-`2658`).
- Linux source editor is explicitly named and focusable
  (`linux_gtk.rs:2554`-`2556`, `3256`).
- Linux toolbar buttons are explicitly named, roled, and keyboard-focusable
  (`linux_gtk.rs:3709`-`3719`).
- Linux menus, dirty-close dialog, save chooser, recovery dialog, and
  close-save-retry dialog are all semantic GTK widgets
  (`linux_gtk.rs:3518`-`3539`, `3541`-`3563`, `3650`-`3664`, `2359`-`2378`,
  `3730`-`3853`).

These need **interactive confirmation only**, not code changes.
