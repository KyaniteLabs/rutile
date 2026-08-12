//! Shared, platform-neutral editor actions layered over the [`AppState`]
//! reducer (Wave 2S — the one-writer prelude to shell integration).
//!
//! Wave 1 froze the pure engines in `rutile-core` (format, find/replace,
//! export, autosave/session, counts). This module carries the *shared*
//! app-level surface both native shells (macOS Iced, Linux GTK) bind their
//! native input to, so the two later platform lanes cannot diverge on how a
//! [`FormatCommand`](rutile_core::FormatCommand) is applied, how a find
//! session is held, how an export page is produced, or how autosave/session
//! state is written.
//!
//! ## State authority
//!
//! [`AppState`] does not own the document text: the platform `ProductSession`
//! owns both an [`AppState`] and a `Document` as sibling fields and routes edits
//! through `apply_editor_commit`. The action methods here follow the same
//! separation — every mutating action takes `&mut Document` and applies its
//! [`EditPlan`](rutile_core::EditPlan)s through the existing transaction
//! path (`EditPlan::into_transaction` → `Document::apply`), then advances the
//! reducer through the existing `DocumentEdited` message so the render pipeline
//! coalesces exactly as it does for typed edits. `AppState` owns the doc
//! path/dirty/preview coordination and the autosave store *path*; `FileService`
//! still owns the disk; the frozen core engines still own the text logic.
//!
//! The value types below are the vocabulary the platform lanes bind to; the
//! methods that consume them live on [`AppState`] in [`crate::app`].
//!
//! [`AppState`]: crate::app::AppState

use std::ops::Range;
use std::path::PathBuf;

use rutile_core::{
    ChangeSet, EditError, EditPlanError, FindDirection, FindError, FindQuery, ReplaceError,
    Selection, SessionWindowV1, SmartEnterAction,
};
use rutile_types::Revision;
use thiserror::Error;

use crate::app::{AppEffect, AppMessage, AppState};

/// The live find/replace session held (optionally) by
/// [`AppState`](crate::app::AppState).
///
/// A shell opens a find bar with [`AppState::start_find`], drives
/// [`AppState::find_next`]/[`AppState::find_prev`] (which record the located
/// match in [`current`](FindSession::current)), and replaces with
/// [`AppState::replace_current`]/[`AppState::replace_all`]. The replacement
/// string is supplied per call; the session owns the query, direction, and
/// wrap policy so find-next and replace share one source of truth.
///
/// [`AppState::start_find`]: crate::app::AppState::start_find
/// [`AppState::find_next`]: crate::app::AppState::find_next
/// [`AppState::find_prev`]: crate::app::AppState::find_prev
/// [`AppState::replace_current`]: crate::app::AppState::replace_current
/// [`AppState::replace_all`]: crate::app::AppState::replace_all
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindSession {
    /// The validated search query.
    pub query: FindQuery,
    /// The default direction for [`AppState::find_next`]; `find_prev` inverts
    /// it.
    ///
    /// [`AppState::find_next`]: crate::app::AppState::find_next
    pub direction: FindDirection,
    /// Whether searches wrap around the ends of the buffer.
    pub wrap: bool,
    /// The most recently located match, in current-document byte coordinates.
    /// Cleared after a replace mutates the buffer (the offsets go stale).
    pub current: Option<Range<usize>>,
}

impl FindSession {
    /// Opens a session with no located match yet.
    pub fn new(query: FindQuery, direction: FindDirection, wrap: bool) -> Self {
        Self {
            query,
            direction,
            wrap,
            current: None,
        }
    }
}

/// The result of a successful [`AppState::apply_format_command`].
///
/// [`AppState::apply_format_command`]: crate::app::AppState::apply_format_command
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatApplied {
    /// The decided smart-Enter action; `Some` only for
    /// [`FormatCommand::SmartEnter`](rutile_core::FormatCommand::SmartEnter).
    pub action: Option<SmartEnterAction>,
    /// The selection the shell should install after applying the edit.
    pub selection_after: Selection,
    /// The document revision after the edit committed.
    pub revision: Revision,
    /// The applied [`ChangeSet`]s, in commit order — always exactly one for a
    /// format command (a single [`EditPlan`](rutile_core::EditPlan)). A
    /// shell replays them through the incremental
    /// [`apply_external_change`](rutile_core::EditorAdapter::apply_external_change)
    /// path (preserving its viewport) instead of re-installing the whole buffer;
    /// see [`ReplaceApplied::changes`] for why the shape is a `Vec`.
    pub changes: Vec<ChangeSet>,
    /// Reducer effects (e.g. a coalesced `ScheduleRender`) to run.
    pub effects: Vec<AppEffect>,
}

/// The result of a successful replace action
/// ([`AppState::replace_current`]/[`AppState::replace_all`]).
///
/// A zero-match replace is a clean no-op: `replaced` is `0`, `selection_after`
/// is `None`, and `effects` is empty (the document was not touched).
///
/// [`AppState::replace_current`]: crate::app::AppState::replace_current
/// [`AppState::replace_all`]: crate::app::AppState::replace_all
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceApplied {
    /// Number of matches replaced.
    pub replaced: usize,
    /// Selection to install after the edit, or `None` when nothing changed.
    pub selection_after: Option<Selection>,
    /// The document revision after the (possibly no-op) action.
    pub revision: Revision,
    /// The applied [`ChangeSet`]s, in commit order: one for
    /// [`replace_current`](crate::app::AppState::replace_current), and one per
    /// bounded plan chunk for
    /// [`replace_all`](crate::app::AppState::replace_all) (the engine chunks a
    /// large replace-all into a sequence of `EditPlan`s). Empty for a no-op.
    ///
    /// A `Vec<ChangeSet>` — not a single merged `ChangeSet` — is the shape that
    /// lets a shell follow the mutation incrementally: each [`ChangeSet`] chains
    /// `before`→`after` exactly as `Document::apply` produced it, so a shell
    /// replays the sequence through
    /// [`apply_external_change`](rutile_core::EditorAdapter::apply_external_change)
    /// (the same path it uses for undo/redo and external edits), preserving its
    /// viewport. Merging into one `ChangeSet` was rejected because the plans are
    /// applied sequentially: every plan's edit offsets live in the coordinate
    /// space of the intermediate document (after the prior plans), not a single
    /// base, so flattening them would require recomputing offsets and could
    /// introduce overlap — weakening the per-plan bounds Wave 1 froze.
    pub changes: Vec<ChangeSet>,
    /// Reducer effects to run; empty for a no-op.
    pub effects: Vec<AppEffect>,
}

/// The result of a successful [`AppState::insert_text`].
///
/// This is the shared smart-paste / programmatic-insert primitive: both shells
/// lower converted-clipboard markdown (or a plain-text fallback) through it and
/// follow the returned [`changes`](InsertApplied::changes) incrementally via
/// `apply_external_change`, preserving the viewport instead of reinstalling the
/// whole buffer.
///
/// [`AppState::insert_text`]: crate::app::AppState::insert_text
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsertApplied {
    /// The selection to install after the insert (collapsed at the end of the
    /// inserted text).
    pub selection_after: Selection,
    /// The document revision after the insert committed.
    pub revision: Revision,
    /// The applied [`ChangeSet`]s, in commit order — one for a single-edit
    /// insert. A shell replays them through
    /// [`apply_external_change`](rutile_core::EditorAdapter::apply_external_change)
    /// (the same viewport-preserving path undo/redo and external edits use).
    pub changes: Vec<ChangeSet>,
    /// Reducer effects (e.g. a coalesced `ScheduleRender`) to run.
    pub effects: Vec<AppEffect>,
}

/// A validated, self-contained export page plus a suggested file name.
///
/// The shared side computes both; the platform lane performs the actual file
/// write (save-as-HTML) or clipboard set (copy-as-HTML). Because the `html`
/// came out of `render_export_page` it is already proven scriptless and free
/// of external references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportOutput {
    /// The inert, self-contained HTML document.
    pub html: String,
    /// A default file name derived from the document path (or `untitled.html`).
    pub suggested_file_name: String,
}

/// The platform-actionable parts of a restored session
/// ([`AppState::restore_session`]).
///
/// Every field is advisory: the shell opens `last_file` through `FileService`
/// and re-validates `selection`/`top_visible_byte` against the loaded document
/// before installing them.
///
/// [`AppState::restore_session`]: crate::app::AppState::restore_session
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRestore {
    /// Path of the last open file to reopen, if any.
    pub last_file: Option<PathBuf>,
    /// Cursor/selection to install once the document is loaded.
    pub selection: Option<Selection>,
    /// Top-visible byte to scroll to once the document is loaded.
    pub top_visible_byte: Option<usize>,
    /// Window frame to restore (the platform supplies and consumes this).
    pub window: Option<SessionWindowV1>,
}

/// Why a shared editor action could not be completed.
///
/// Every variant is a clean, non-panicking rejection that leaves the document
/// and reducer untouched; the platform lane turns it into an optional status
/// message.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ActionError {
    /// A find/replace action was requested with no active find session.
    #[error("no active find session")]
    NoFindSession,
    /// The format/find engine could not build a bounded edit plan.
    #[error(transparent)]
    Plan(#[from] EditPlanError),
    /// A find/replace value (query or replacement) failed validation.
    #[error(transparent)]
    Find(#[from] FindError),
    /// A replacement could not be lowered into edit plan(s).
    #[error(transparent)]
    Replace(#[from] ReplaceError),
    /// The document rejected the transaction (stale revision, bounds, …).
    #[error(transparent)]
    Edit(#[from] EditError),
}

// ---------------------------------------------------------------------------
// ActionRegistry — declarative command catalog for the palette and menus
// (roadmap 03 / 06). The registry DESCRIBES commands; dispatch still flows
// through the single reducer via [`AppMessage`](crate::app::AppMessage).
// -----------------------------------------------------------------------

/// Globally-stable command id (kebab-case string; never reused).
///
/// Example: `"file.save"`, `"format.toggle-code-block"`, `"find.replace-all"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub &'static str);

/// Keyboard modifier flags for a [`Shortcut`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShortcutModifiers {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Platform-neutral keyboard shortcut (resolved by the platform shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Logical key name (lowercase), e.g. `"s"`, `"o"`, `"p"`, `"f1"`.
    pub key: &'static str,
    pub modifiers: ShortcutModifiers,
}

impl Shortcut {
    /// Shortcut with only the platform command modifier (⌘ on macOS).
    pub const fn cmd(key: &'static str) -> Self {
        Self {
            key,
            modifiers: ShortcutModifiers {
                cmd: true,
                shift: false,
                alt: false,
                ctrl: false,
            },
        }
    }

    /// Shortcut with command + shift.
    pub const fn cmd_shift(key: &'static str) -> Self {
        Self {
            key,
            modifiers: ShortcutModifiers {
                cmd: true,
                shift: true,
                alt: false,
                ctrl: false,
            },
        }
    }
}

/// Semantic category for palette grouping and menu placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    File,
    Edit,
    Format,
    Find,
    View,
    Window,
    Help,
}

/// Declarative description of an invocable command for the palette/menus.
///
/// The shell queries [`message`](CommandDescriptor::message) against
/// [`AppState`](AppState); the palette greys out rows that return `None`.
/// The function pointer is pure over `&AppState` — no I/O, no side effects —
/// so all state transitions still flow through the single reducer.
///
/// # Security-core fence
///
/// No command constructs raw HTML/URLs or bypasses
/// [`SafeLinkTarget`](rutile_types::SafeLinkTarget) / `render.rs`.
#[derive(Clone, Copy)]
pub struct CommandDescriptor {
    pub id: CommandId,
    /// User-facing label shown in the palette.
    pub title: &'static str,
    pub category: CommandCategory,
    /// Optional keybinding (resolved by the platform shell).
    pub shortcut: Option<Shortcut>,
    /// Returns the [`AppMessage`] to dispatch when invoked, or `None` when the
    /// command is unavailable in the current state (palette shows it disabled).
    pub message: fn(&AppState) -> Option<AppMessage>,
}

impl std::fmt::Debug for CommandDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandDescriptor")
            .field("id", &self.id.0)
            .field("title", &self.title)
            .field("category", &self.category)
            .field("shortcut", &self.shortcut)
            .finish_non_exhaustive()
    }
}

/// Error returned when a duplicate command id is registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionRegistryError {
    #[error("duplicate command id: {0}")]
    DuplicateCommand(&'static str),
}

/// Static catalog plus runtime-registered platform commands.
///
/// Lookup is by [`CommandId`]; the palette filters by [`CommandCategory`] and
/// free-text over [`title`](CommandDescriptor::title).
pub struct ActionRegistry {
    /// Compile-time catalog (const slice).
    catalog: &'static [CommandDescriptor],
    /// Runtime-registered platform commands.
    dynamic: Vec<CommandDescriptor>,
}

impl ActionRegistry {
    /// Builds a registry from a compile-time static catalog.
    ///
    /// In debug builds, panics if the catalog contains duplicate ids.
    pub fn from_static(catalog: &'static [CommandDescriptor]) -> Self {
        debug_assert!(
            ids_unique(catalog),
            "static command catalog has duplicate ids"
        );
        Self {
            catalog,
            dynamic: Vec::new(),
        }
    }

    /// Registers a runtime platform command. Fails closed on duplicate id.
    pub fn register(&mut self, descriptor: CommandDescriptor) -> Result<(), ActionRegistryError> {
        if self.lookup(&descriptor.id).is_some() {
            return Err(ActionRegistryError::DuplicateCommand(descriptor.id.0));
        }
        self.dynamic.push(descriptor);
        Ok(())
    }

    /// Total number of commands (static + dynamic).
    pub fn len(&self) -> usize {
        self.catalog.len() + self.dynamic.len()
    }

    /// Whether the registry has zero commands.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Looks up a descriptor by id across both static and dynamic sets.
    pub fn lookup(&self, id: &CommandId) -> Option<&CommandDescriptor> {
        self.catalog
            .iter()
            .chain(self.dynamic.iter())
            .find(|d| &d.id == id)
    }

    /// Iterates all descriptors (static first, then dynamic).
    pub fn iter(&self) -> impl Iterator<Item = &CommandDescriptor> {
        self.catalog.iter().chain(self.dynamic.iter())
    }

    /// Filters descriptors by category.
    pub fn by_category(
        &self,
        category: CommandCategory,
    ) -> impl Iterator<Item = &CommandDescriptor> {
        self.iter().filter(move |d| d.category == category)
    }

    /// Free-text search over titles (case-insensitive substring).
    pub fn search(&self, query: &str) -> Vec<&CommandDescriptor> {
        let q = query.to_ascii_lowercase();
        self.iter()
            .filter(|d| d.title.to_ascii_lowercase().contains(&q))
            .collect()
    }
}

/// Checks that all `CommandId`s in a slice are unique.
fn ids_unique(descriptors: &[CommandDescriptor]) -> bool {
    let mut seen = std::collections::HashSet::new();
    descriptors.iter().all(|d| seen.insert(d.id.0))
}
// ---------------------------------------------------------------------------
// Default command catalog (roadmap 06). These map AppMessage-dispatchable
// actions to palette/menu descriptors. Platform shells may register more.
// -----------------------------------------------------------------------

/// Builds the [`AppMessage`] for "New Document", always available.
fn cmd_new(_state: &AppState) -> Option<AppMessage> {
    Some(AppMessage::NewDocument)
}

/// Builds "Save", available only when the active document is dirty.
fn cmd_save(state: &AppState) -> Option<AppMessage> {
    state.dirty().then_some(AppMessage::SaveRequested)
}

/// Builds "Clear Recent Documents", available when the list is non-empty.
fn cmd_clear_recents(state: &AppState) -> Option<AppMessage> {
    (!state.recents().is_empty()).then_some(AppMessage::ClearRecents)
}

/// Builds "New Tab", always available.
fn cmd_new_tab(_state: &AppState) -> Option<AppMessage> {
    Some(AppMessage::NewTab)
}

/// Builds "Close Tab" for the active tab, available with more than one tab.
fn cmd_close_tab(state: &AppState) -> Option<AppMessage> {
    let docs = state.documents();
    (docs.len() > 1).then_some(AppMessage::CloseTab {
        id: docs.active_id(),
    })
}

/// Builds "Next/Previous Tab" by rotating the tab strip by `offset` (wrapping).
/// Available when more than one tab is open.
fn cmd_rotate_tab(state: &AppState, offset: isize) -> Option<AppMessage> {
    let docs = state.documents();
    let order = docs.tab_order();
    if order.len() < 2 {
        return None;
    }
    let pos = order.iter().position(|&id| id == docs.active_id())?;
    let next = (pos as isize + offset).rem_euclid(order.len() as isize) as usize;
    Some(AppMessage::SwitchTab { id: order[next] })
}

fn cmd_next_tab(state: &AppState) -> Option<AppMessage> {
    cmd_rotate_tab(state, 1)
}

fn cmd_prev_tab(state: &AppState) -> Option<AppMessage> {
    cmd_rotate_tab(state, -1)
}

/// The compile-time default command set (see `docs/plan/command-palette-design.md`).
pub const DEFAULT_CATALOG: &[CommandDescriptor] = &[
    CommandDescriptor {
        id: CommandId("file.new"),
        title: "New Document",
        category: CommandCategory::File,
        shortcut: Some(Shortcut::cmd("n")),
        message: cmd_new,
    },
    CommandDescriptor {
        id: CommandId("file.save"),
        title: "Save",
        category: CommandCategory::File,
        shortcut: Some(Shortcut::cmd("s")),
        message: cmd_save,
    },
    CommandDescriptor {
        id: CommandId("file.clear-recents"),
        title: "Clear Recent Documents",
        category: CommandCategory::File,
        shortcut: None,
        message: cmd_clear_recents,
    },
    CommandDescriptor {
        id: CommandId("view.next-tab"),
        title: "Show Next Tab",
        category: CommandCategory::View,
        shortcut: None,
        message: cmd_next_tab,
    },
    CommandDescriptor {
        id: CommandId("view.prev-tab"),
        title: "Show Previous Tab",
        category: CommandCategory::View,
        shortcut: None,
        message: cmd_prev_tab,
    },
    CommandDescriptor {
        id: CommandId("window.new-tab"),
        title: "New Tab",
        category: CommandCategory::Window,
        shortcut: Some(Shortcut::cmd("t")),
        message: cmd_new_tab,
    },
    CommandDescriptor {
        id: CommandId("window.close-tab"),
        title: "Close Tab",
        category: CommandCategory::Window,
        shortcut: Some(Shortcut::cmd("w")),
        message: cmd_close_tab,
    },
];

impl ActionRegistry {
    /// Builds the registry from [`DEFAULT_CATALOG`].
    pub fn standard() -> Self {
        Self::from_static(DEFAULT_CATALOG)
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

#[cfg(test)]
mod action_registry_tests {
    use super::*;
    use crate::app::AppState;

    fn always_save(_state: &AppState) -> Option<AppMessage> {
        Some(AppMessage::SaveRequested)
    }

    fn never(_state: &AppState) -> Option<AppMessage> {
        None
    }

    fn always_new(_state: &AppState) -> Option<AppMessage> {
        Some(AppMessage::NewDocument)
    }

    const SAVE_CMD: CommandDescriptor = CommandDescriptor {
        id: CommandId("file.save"),
        title: "Save",
        category: CommandCategory::File,
        shortcut: Some(Shortcut::cmd("s")),
        message: always_save,
    };

    const NEW_CMD: CommandDescriptor = CommandDescriptor {
        id: CommandId("file.new"),
        title: "New Document",
        category: CommandCategory::File,
        shortcut: Some(Shortcut::cmd("n")),
        message: always_new,
    };

    const FIND_CMD: CommandDescriptor = CommandDescriptor {
        id: CommandId("find.open"),
        title: "Find",
        category: CommandCategory::Find,
        shortcut: Some(Shortcut::cmd("f")),
        message: never,
    };

    // -- Invariant 1: CommandIds are globally unique; duplicate registration fails ---

    #[test]
    fn duplicate_registration_fails_closed() {
        let mut reg = ActionRegistry::from_static(&[SAVE_CMD]);
        let dup = CommandDescriptor {
            id: CommandId("file.save"),
            title: "Another Save",
            category: CommandCategory::File,
            shortcut: None,
            message: always_save,
        };
        assert_eq!(
            reg.register(dup),
            Err(ActionRegistryError::DuplicateCommand("file.save"))
        );
    }

    #[test]
    fn duplicate_registration_against_static_fails() {
        let mut reg = ActionRegistry::from_static(&[SAVE_CMD, NEW_CMD]);
        let dup = CommandDescriptor {
            id: CommandId("file.new"),
            title: "Override New",
            category: CommandCategory::File,
            shortcut: None,
            message: always_new,
        };
        assert!(reg.register(dup).is_err());
    }

    #[test]
    fn unique_registration_succeeds() {
        let mut reg = ActionRegistry::from_static(&[SAVE_CMD]);
        assert!(reg.register(NEW_CMD).is_ok());
        assert!(reg.register(FIND_CMD).is_ok());
        assert_eq!(reg.len(), 3);
    }

    // -- Lookup --------------------------------------------------------------

    #[test]
    fn lookup_finds_static_and_dynamic() {
        let mut reg = ActionRegistry::from_static(&[SAVE_CMD]);
        reg.register(FIND_CMD).unwrap();

        assert!(reg.lookup(&CommandId("file.save")).is_some());
        assert!(reg.lookup(&CommandId("find.open")).is_some());
        assert!(reg.lookup(&CommandId("nonexistent")).is_none());
    }

    // -- Category filter -----------------------------------------------------

    #[test]
    fn by_category_returns_only_matching() {
        let mut reg = ActionRegistry::from_static(&[SAVE_CMD, NEW_CMD]);
        reg.register(FIND_CMD).unwrap();

        let file_cmds: Vec<_> = reg.by_category(CommandCategory::File).collect();
        assert_eq!(file_cmds.len(), 2);
        assert!(
            file_cmds
                .iter()
                .all(|d| d.category == CommandCategory::File)
        );

        let find_cmds: Vec<_> = reg.by_category(CommandCategory::Find).collect();
        assert_eq!(find_cmds.len(), 1);
        assert_eq!(find_cmds[0].id.0, "find.open");
    }

    // -- Free-text search ----------------------------------------------------

    #[test]
    fn search_matches_case_insensitive_substring() {
        let reg = ActionRegistry::from_static(&[SAVE_CMD, NEW_CMD, FIND_CMD]);
        let results = reg.search("save");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "file.save");

        let results = reg.search("DOC");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "file.new");
    }

    #[test]
    fn search_empty_query_returns_all() {
        let reg = ActionRegistry::from_static(&[SAVE_CMD, NEW_CMD]);
        assert_eq!(reg.search("").len(), 2);
    }

    // -- Invariant 2: message() purity (function pointer returns correct msg) --

    #[test]
    fn message_returns_expected_app_message() {
        let reg = ActionRegistry::from_static(&[SAVE_CMD]);
        let state = AppState::new();
        let cmd = reg.lookup(&CommandId("file.save")).unwrap();
        let msg = (cmd.message)(&state);
        assert!(matches!(msg, Some(AppMessage::SaveRequested)));
    }

    #[test]
    fn disabled_command_returns_none() {
        let reg = ActionRegistry::from_static(&[FIND_CMD]);
        let state = AppState::new();
        let cmd = reg.lookup(&CommandId("find.open")).unwrap();
        let msg = (cmd.message)(&state);
        assert!(msg.is_none());
    }

    // -- Static catalog uniqueness (debug_assert) ----------------------------

    #[test]
    fn static_catalog_with_unique_ids_constructs() {
        let reg = ActionRegistry::from_static(&[SAVE_CMD, NEW_CMD, FIND_CMD]);
        assert_eq!(reg.len(), 3);
        assert!(!reg.is_empty());
    }

    #[test]
    fn empty_registry() {
        let reg = ActionRegistry::from_static(&[]);
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    // -- Shortcut constructors ----------------------------------------------

    #[test]
    fn shortcut_cmd_sets_only_cmd_modifier() {
        let s = Shortcut::cmd("s");
        assert_eq!(s.key, "s");
        assert!(s.modifiers.cmd);
        assert!(!s.modifiers.shift);
        assert!(!s.modifiers.alt);
        assert!(!s.modifiers.ctrl);
    }

    #[test]
    fn shortcut_cmd_shift_sets_cmd_and_shift() {
        let s = Shortcut::cmd_shift("p");
        assert_eq!(s.key, "p");
        assert!(s.modifiers.cmd);
        assert!(s.modifiers.shift);
        assert!(!s.modifiers.alt);
        assert!(!s.modifiers.ctrl);
    }
}
