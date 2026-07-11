//! Shared, platform-neutral editor actions layered over the [`AppState`]
//! reducer (Wave 2S — the one-writer prelude to shell integration).
//!
//! Wave 1 froze the pure engines in `feathermark-core` (format, find/replace,
//! export, autosave/session, counts). This module carries the *shared*
//! app-level surface both native shells (macOS Iced, Linux GTK) bind their
//! native input to, so the two later platform lanes cannot diverge on how a
//! [`FormatCommand`](feathermark_core::FormatCommand) is applied, how a find
//! session is held, how an export page is produced, or how autosave/session
//! state is written.
//!
//! ## State authority
//!
//! [`AppState`] does not own the document text: the platform `ProductSession`
//! owns both an [`AppState`] and a `Document` as sibling fields and routes edits
//! through `apply_editor_commit`. The action methods here follow the same
//! separation — every mutating action takes `&mut Document` and applies its
//! [`EditPlan`](feathermark_core::EditPlan)s through the existing transaction
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

use feathermark_core::{
    EditError, EditPlanError, FindDirection, FindError, FindQuery, ReplaceError, Selection,
    SessionWindowV1, SmartEnterAction,
};
use feathermark_types::Revision;
use thiserror::Error;

use crate::app::AppEffect;

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
    /// [`FormatCommand::SmartEnter`](feathermark_core::FormatCommand::SmartEnter).
    pub action: Option<SmartEnterAction>,
    /// The selection the shell should install after applying the edit.
    pub selection_after: Selection,
    /// The document revision after the edit committed.
    pub revision: Revision,
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
    /// Reducer effects to run; empty for a no-op.
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
