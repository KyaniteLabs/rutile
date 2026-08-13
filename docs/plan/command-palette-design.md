# Command Palette and Action Registry (Roadmap 06) — Design Decisions

Status: **implemented (contract + macOS NSPanel, PR #111).** Parent
issue: `.scratch/rutile-macos-roadmap/issues/06-command-palette-and-action-registry.md`.
Blocked by 03 (DONE — `ActionRegistry` locked in `crates/rutile-app/src/actions.rs`).

## Resolved decisions

### P1 — One source of truth: the `ActionRegistry`

Every user-visible action is a `CommandDescriptor` in the `ActionRegistry`
(locked by roadmap 03). The palette, menus, keyboard shortcuts, and future AI
tools all read the **same** registry. There is no second command list. The
registry *describes* commands (id, title, category, shortcut, availability
predicate); dispatch still flows through the single reducer via `AppMessage`.

### P2 — Default command catalog

`ActionRegistry::standard()` builds the default catalog from a compile-time
static slice covering the actions the reducer already understands:

- `file.new`, `file.open`, `file.save`, `file.save-as`, `file.close`
- `edit.find`, `edit.replace`
- `format.*` (toggle bold/italic/code/code-block/heading)
- `view.next-tab`, `view.prev-tab`
- `window.new-tab`, `window.close-tab`

Platform shells may `register()` additional commands at runtime (e.g. native
quick-open) without colliding with the static set. Duplicate ids fail closed.

### P3 — Ranking: prefix > word-prefix > substring; exact id always wins

The palette ranks candidate commands against the query (case-insensitive):

1. **Exact id match** (`"file.save"` for query `"file.save"`) — top.
2. **Title prefix** (`"Save"` for query `"sa"`).
3. **Word-boundary prefix** in the title (`"Save As…"` for query `"as"`,
   matching the second word).
4. **Substring** (`"New Document"` for query `"doc"`).

Within a tier, static-catalog commands rank above dynamic ones, and ties keep
catalog declaration order (stable sort). Empty query returns all commands in
declaration order. The visible list is capped at `MAX_PALETTE_RESULTS` (50).

### P4 — Availability is live, not baked

Each `CommandDescriptor` carries `message: fn(&AppState) -> Option<AppMessage>`.
A command is **available** when that function returns `Some(..)` against the
current state, **unavailable** when it returns `None`. The palette evaluates
this per query so the same command can be enabled or greyed out as state
changes (e.g. "Save" unavailable when the document is clean). Unavailable
commands stay visible (discoverable) but are not invocable.

### P5 — Palette interaction model

`CommandPalette` is a value type the reducer drives:

- `open()` / `close()` toggle visibility; `is_open()` queries it.
- `set_query(q, &registry, &state)` recomputes the ranked, availability-tagged
  result list and resets the selection to the first available row.
- `select_next()` / `select_prev()` move within the results (wrapping is the
  shell's choice; the contract clamps to bounds).
- `submit(&registry, &state) -> Option<AppMessage>` returns the selected
  command's dispatch message and closes the palette.

The shell renders the entries from `PaletteEntry` values (id, title, category,
available, shortcut display string). The contract carries no rendering.

### P6 — Shortcut display is platform-resolved

`Shortcut` is platform-neutral (`cmd`/`shift`/`alt`/`ctrl` + key). The palette
exposes a `display()` string (`"⇧⌘P"`) for convenience, but the canonical
resolution (NSEvent ↔ Shortcut) lives in the platform shell. The registry
never imports AppKit/GTK.

### P7 — Localization readiness

Titles are plain `&'static str` for now (English). When localization lands,
`CommandDescriptor::title` becomes a translation key; the registry shape is
already a flat string per command, so the migration is mechanical and does not
change ranking (which is locale-agnostic on ids/word-boundaries).

### P8 — Dispatch never bypasses the reducer

`submit` returns an `AppMessage`, not a side effect. The shell forwards it to
`AppState::reduce`. No command constructs raw HTML/URLs or bypasses the
security core (`render.rs`, `security.rs`, `safe_link.rs`).

### P9 — Native panel UI (shipped, PR #111)

macOS presents a nonactivating `NSPanel` from `palette_snapshot()`. Rows
match `candidates()`; unavailable commands stay listed and greyed. Esc
closes and returns focus to the editor. Linux has no panel.
