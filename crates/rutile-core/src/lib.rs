//! Rutile's platform-independent document and editor contracts.

mod autosave;
mod counts;
mod design_tokens;
mod document;
mod editor_contract;
mod export;
mod export_contract;
mod files;
mod find_contract;
mod find_engine;
mod format_contract;
mod format_engine;
mod html_to_markdown;
mod render;
mod scroll;
mod security;
mod session_contract;

pub use autosave::{
    AUTOSAVE_JOURNAL_FILE, AUTOSAVE_LOCK_FILE, AUTOSAVE_RETENTION, AutosaveError,
    AutosaveRecordOutcome, AutosaveStore, MAX_JOURNAL_BYTES, OrphanFailure, OrphanGcReport,
    PruneOutcome, RecoveredDocument, RecoveryReport, RejectedEntry, RejectionReason,
    SESSION_STATE_FILE,
};
pub use counts::{Counts, READING_WPM, char_count, counts, reading_time_seconds, word_count};
pub use design_tokens::{
    DESIGN_TOKENS, DesignTokenSet, DesignTokenSetError, DesignTokenValue, MAX_TOKEN_VALUE_BYTES,
    TokenValueError,
};
pub use document::{
    ChangeSet, Document, DocumentError, DocumentSnapshot, Edit, EditError, EditTransaction,
    HistoryBoundary, HistoryContext, HistoryEntry, MAX_DOCUMENT_BYTES, MAX_UNDO_BYTES, Selection,
    TransactionKind, TypingDirection,
};
pub use editor_contract::{
    AdapterCommitId, CompositionCancelReason, CompositionId, CompositionTracker, EditorAdapter,
    EditorCommit, EditorError, EditorEvent, EditorEventSink, ImeCommit, LocalCommitRejection,
    StaleRevision, ViewportState, apply_editor_commit,
};
pub use export::render_export_page;
pub use export_contract::{
    EXPORT_CSP, ExportError, ExportPage, ExportRequest, ExportViolation, MAX_EXPORT_PAGE_BYTES,
    MAX_EXPORT_TITLE_BYTES,
};
pub use files::{
    DiskVersion, DurabilityError, ExternalChange, ExternalChangeDebouncer, ExternalResolution,
    FileError, FileService, LoadedDocument, LocalFileService, SaveError, SaveFault, SaveOutcome,
};
pub use find_contract::{
    FindDirection, FindError, FindQuery, FindReplaceOp, MAX_FIND_PATTERN_BYTES,
    MAX_FIND_REPLACEMENT_BYTES, MatchMode, ReplaceSpec,
};
pub use find_engine::{
    FindReplaceOutcome, ReplaceError, execute, find_next, match_count, replace_all, replace_current,
};
pub use format_contract::{
    EditPlan, EditPlanError, FormatCommand, ListMarker, MAX_LINK_URL_BYTES, MAX_PLAN_EDITS,
    MAX_PLAN_TOTAL_BYTES, MAX_PLANNED_REPLACEMENT_BYTES, MAX_PLANNED_SPAN_BYTES, OrderedDelimiter,
    SmartEnterAction,
};
pub use format_engine::{SmartEnterOutcome, apply_format, smart_enter};
pub use html_to_markdown::{
    HtmlToMarkdownError, MAX_HTML_INPUT_BYTES, MAX_NESTING_DEPTH, MAX_OUTPUT_BYTES,
    html_to_markdown,
};
pub use render::{
    MAX_GENERATED_BODY_BYTES, MAX_RENDER_NESTING_DEPTH, MAX_RENDERED_PAGE_BYTES,
    MAX_SOURCE_BLOCK_BYTES, RenderError, RenderLimits, RenderedPage, SourceBlock, SourceBlockKind,
    build_source_blocks, render_markdown, render_markdown_with_limits, validate_source_blocks,
};
pub use rutile_types::{InteractionId, Revision};
pub use scroll::{
    InteractionLease, Pane, ScrollAnchorView, ScrollClock, ScrollCommand, ScrollError,
    ScrollGeometry, ScrollMap, ScrollOutcome, ScrollPosition, ScrollSynchronizer, ScrollTarget,
    SuppressionReason, preview_samples, source_samples,
};
pub use security::{SafeNode, SourceAttributes};
pub use session_contract::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, MAX_AUTOSAVE_ENTRY_BYTES, MAX_RECENT_FILES,
    MAX_SESSION_PATH_BYTES, MAX_SESSION_STATE_BYTES, SESSION_SCHEMA_V1, SessionError,
    SessionSelectionV1, SessionStateV1, SessionWindowV1, decode_autosave_entry,
    decode_session_state, encode_autosave_entry, encode_session_state,
};
