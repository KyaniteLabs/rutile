# Foundations Contracts (node 03) — ActionRegistry + Preferences design spec

Status: **design (to be locked before roadmap waves implement)**. ralplan
`rutile-criticalpath-20260811`. Grounded in the current Elm-like reducer
(`AppMessage`→`AppState::reduce`→`Vec<AppEffect>`, `crates/rutile-app/src/app.rs`).

## `DocumentId` — LOCKED (implemented)

`crates/rutile-types/src/lib.rs`. Newtype over `u64`; `ROOT` for the single-document
baseline; shell mints fresh ids for tabs (roadmap 08). Distinct from `Revision`/
`InteractionId` at the type level. Done in this node.

## `ActionRegistry` contract (for the command palette, roadmap 06)

The palette needs a typed, declarative catalog of invocable commands — not a second
dispatch path. The registry DESCRIBES commands; dispatch still flows through the single
reducer via `AppMessage`. **No new mutation surface** — the registry is read-only state
the palette renders from, and invoking a command emits the matching `AppMessage`.

```rust
// crates/rutile-app/src/actions.rs  (extends the existing actions module)

/// Globally-stable command id (kebab-case string; never reused).
/// Example: "file.save", "format.toggle-code-block", "find.replace-all".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommandId(pub &'static str);

/// Declarative description of an invocable command for the palette/menus.
/// The shell queries enabled() against AppState; the palette greys out disabled rows.
pub struct CommandDescriptor {
    pub id: CommandId,
    pub title: &'static str,           // user-facing label
    pub category: CommandCategory,     // File / Format / Find / View / …
    pub shortcut: Option<Shortcut>,    // optional keybinding (resolved by the platform shell)
    /// Returns the AppMessage to dispatch when invoked, or None when unavailable
    /// in the current state (palette shows it disabled).
    pub message: fn(&AppState) -> Option<AppMessage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory { File, Edit, Format, Find, View, Window, Help }

/// The registry is a static catalog built at compile time (const slice) plus any
/// runtime-registered platform commands. Lookup is by CommandId; the palette
/// filters by category + free-text over `title`.
pub struct ActionRegistry { /* &'static [CommandDescriptor] + dynamic entries */ }
```

**Invariants (enforced by tests before this is marked implemented):**
1. `CommandId`s are globally unique; registration of a duplicate `id` fails closed.
2. `message()` is pure over `&AppState` — no I/O, no side effects; the reducer owns all effects.
3. Disabled commands return `None`; the palette never calls dispatch on a `None` row.
4. No command constructs raw HTML/URLs or bypasses `SafeLinkTarget`/`render.rs` (security-core fence).
5. Adding a command does not require editing the reducer's match arms — only registering a
   descriptor whose `message` returns an existing `AppMessage` variant.

**Why this shape:** it keeps the single reducer as the only state-transition path (no second
dispatch), makes the palette a pure view over the catalog, and lets features (06, 07, 08, 09)
register commands without touching reducer internals. The `fn(&AppState) -> Option<AppMessage>`
closure is chosen over a trait object to stay allocation-free and `const`-constructible.

## `Preferences` contract (for 03 + 07 + 10)

A typed, versioned preferences record persisted via the session store (single source of
truth, no ad-hoc plist/json scattered across the shell). Surfaced as `AppMessage::Preferences`
edits through the reducer.

```rust
// crates/rutile-app/src/preferences.rs  (new module)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Preferences {
    pub schema: PreferencesSchemaV1,   // versioned for forward-compatible migration
    pub appearance: Appearance,         // System / Light / Dark
    pub editor: EditorPrefs,            // font scale, tab width, wrap, spellcheck
    pub view: ViewPrefs,                // reader-first default, focus-mode default
    pub recent: RecentPrefs,            // recent-docs cap, exclude patterns
}
```

**Invariants:** schema-versioned (deny_unknown_fields + envelope peek like `session_contract.rs`);
all fields bounded (font scale within rails, recent cap ≤ MAX_RECENT_FILES); persisted atomically
through the existing session-store path (no new I/O surface). Security-core fence: preferences
never carry raw HTML/URLs/paths that bypass `validate_path`.

## Locking gate

These contracts are **LOCKED** (frozen signatures) as of the `feat/lock-03-contracts` merge.
`ActionRegistry` (`CommandId`, `CommandDescriptor`, `CommandCategory`, `Shortcut`,
`ActionRegistry`) is implemented in `crates/rutile-app/src/actions.rs`. `Preferences`
(`Preferences`, `Appearance`, `EditorPrefs`, `ViewPrefs`, `RecentPrefs`, `PreferencesSchema`)
is implemented in `crates/rutile-app/src/preferences.rs`. Both pass the per-cluster gate
(fmt + clippy + 23 unit tests covering uniqueness, purity, bounded fields, schema migration)
with no security-core edit. Consuming roadmap waves (06, 07, 08) now build against these
locked signatures.
