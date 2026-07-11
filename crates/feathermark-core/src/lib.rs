//! FeatherMark's platform-independent document and editor contracts.

mod document;
mod editor_contract;
mod files;
mod render;
mod scroll;
mod security;

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
pub use feathermark_types::{InteractionId, Revision};
pub use files::{
    DiskVersion, ExternalChange, ExternalChangeDebouncer, ExternalResolution, FileError,
    FileService, LoadedDocument, LocalFileService, SaveFault,
};
pub use render::{
    MAX_GENERATED_BODY_BYTES, MAX_RENDER_NESTING_DEPTH, MAX_RENDERED_PAGE_BYTES,
    MAX_SOURCE_BLOCK_BYTES, RenderError, RenderLimits, RenderedPage, SourceBlock, SourceBlockKind,
    build_source_blocks, render_markdown, render_markdown_with_limits, validate_source_blocks,
};
pub use scroll::{
    InteractionLease, Pane, ScrollAnchorView, ScrollClock, ScrollCommand, ScrollError,
    ScrollGeometry, ScrollMap, ScrollOutcome, ScrollPosition, ScrollSynchronizer, ScrollTarget,
    SuppressionReason, preview_samples, source_samples,
};
pub use security::{SafeNode, SourceAttributes};
