use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::command_palette::CommandPalette;
use rutile_core::{
    AutosaveEntryV1, AutosaveError, AutosaveRecordOutcome, AutosaveStore, ChangeSet, Counts,
    DiskVersion, Document, Edit, EditError, EditPlan, EditTransaction, ExportError, ExportPage,
    ExportRequest, ExportViolation, ExternalResolution, FindDirection, FindQuery, FormatCommand,
    RecoveredDocument, RenderError, ReplaceSpec, SESSION_SCHEMA_V1, Selection, SessionSelectionV1,
    SessionStateV1, SessionWindowV1, TransactionKind, apply_format, render_export_page,
    smart_enter,
};
use rutile_protocol::PreviewEventV1;
use rutile_types::{DocumentId, InteractionId, Revision, SafeLinkTarget};

use crate::actions::{
    ActionError, ActionRegistry, ExportOutput, FindSession, FormatApplied, InsertApplied,
    ReplaceApplied, SessionRestore,
};
use crate::document_manager::{DocumentManager, DocumentSlot};
use crate::publishing::PublishingPreset;
use crate::tasteroll::TasteState;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PreviewState {
    #[default]
    Empty,
    Waiting {
        revision: Revision,
    },
    Navigating {
        revision: Revision,
    },
    Ready {
        revision: Revision,
    },
    TooLarge {
        revision: Revision,
    },
    Failed {
        revision: Revision,
        error: RenderError,
    },
}
/// The shell-level document view mode (roadmap 04).
///
/// Declares which surface the platform shell shows. It is a presentation
/// affordance only — it does not gate edits or duplicate render logic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DocumentMode {
    /// Source editor only.
    Edit,
    /// Editor + preview side by side (the baseline).
    #[default]
    Split,
    /// Rendered preview only (reader mode).
    View,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloseDecision {
    Save { untitled_path: Option<PathBuf> },
    Discard,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseOutcome {
    Close,
    KeepOpen,
}

/// Severity of a [`UserNotice`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeSeverity {
    Info,
    Warning,
    Error,
}

/// A durable, user-visible notice produced by the reducer.
///
/// Notices are persisted in [`AppState::notices`] so the native shell can render
/// them on its next view update and keep them across reducer messages until the
/// user dismisses them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserNotice {
    pub id: usize,
    pub severity: NoticeSeverity,
    pub message: String,
    pub source_error: String,
    pub action_label: Option<String>,
    pub shown: bool,
    pub dismissed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppMessage {
    NewDocument,
    DocumentOpened {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    DocumentEdited {
        revision: Revision,
    },
    SaveCompleted {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    SaveDurabilityUnknown {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    SaveFailed {
        revision: Revision,
    },
    ExternalConflictDetected {
        disk: DiskVersion,
    },
    ResolveExternalConflict(ExternalResolution),
    RenderAccepted {
        revision: Revision,
        page_bytes: usize,
    },
    RenderFailed {
        revision: Revision,
        error: RenderError,
    },
    PreviewEvent(PreviewEventV1),
    // --- Wave 2-A: shared shell-integration command/status bus ---------------
    OpenDocument {
        path: PathBuf,
    },
    OpenRequestCompleted {
        result: Result<(Revision, PathBuf, DiskVersion), String>,
    },
    SaveRequested,
    SaveAsRequested {
        path: PathBuf,
    },
    CloseRequested {
        decision: CloseDecision,
    },
    AutosaveTick,
    AutosaveCompleted {
        result: Result<AutosaveRecordOutcome, String>,
    },
    RecoveryAdopted {
        document: RecoveredDocument,
    },
    RecoveryDismissed,
    SessionRestored {
        state: SessionStateV1,
    },
    NoticeDismissed {
        id: usize,
    },
    /// Generic durable notice injection for platform shells.
    ///
    /// Use this for save/restore/open-policy warnings and other non-open
    /// failures. Do **not** route those through [`AppMessage::OpenRequestCompleted`]
    /// Err — that path always prefixes `"Could not open document:"` and forces
    /// [`NoticeSeverity::Error`].
    SurfaceNotice {
        severity: NoticeSeverity,
        message: String,
        source_error: String,
    },
    MirrorFailed {
        error: String,
    },
    MirrorResyncCompleted {
        result: Result<(), String>,
    },
    /// Clears the recent-documents list (roadmap 07).
    ClearRecents,
    /// Removes a single path from the recent-documents list (roadmap 07).
    RemoveRecent {
        path: PathBuf,
    },
    NewTab,
    SwitchTab {
        id: DocumentId,
    },
    CloseTab {
        id: DocumentId,
    },
    // --- Roadmap 06: command palette ---------------------------------------
    /// Opens the command palette (clears any previous query).
    OpenCommandPalette,
    /// Closes the command palette.
    CloseCommandPalette,
    /// Updates the palette query and recomputes ranked candidates.
    PaletteQueryChanged {
        query: String,
    },
    /// Moves the palette selection down one row.
    PaletteSelectNext,
    /// Moves the palette selection up one row.
    PaletteSelectPrev,
    /// Dispatches the selected command and closes the palette.
    PaletteSubmit,
    // --- Roadmap 04: reader-first view mode ---------------------------------
    /// Sets the shell-level document view mode (Edit / Split / View).
    SetDocumentMode {
        mode: DocumentMode,
    },
    // --- Roadmap 10: focus mode --------------------------------------------
    /// Toggles distraction-free focus mode (hides shell chrome).
    ToggleFocusMode,
    // --- C8: tasteroll (chance-styled notes) -------------------------------
    /// Rolls the design dice using the stored content seed + inferred type.
    TasteRoll,
    /// Re-rolls the current design, preserving locked dimensions.
    TasteReroll,
    /// Clears the chance-styling roll entirely.
    TasteReset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    ScheduleRender {
        revision: Revision,
    },
    NavigatePreview {
        revision: Revision,
        page_bytes: usize,
    },
    ScrollSource {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
    },
    PresentLink(SafeLinkTarget),
    PresentExternalConflict {
        path: PathBuf,
        disk: DiskVersion,
    },
    ReloadExternal {
        path: PathBuf,
    },
    SaveExternalAs {
        path: PathBuf,
    },
    IgnoredStale {
        revision: Revision,
    },
    // --- Wave 2-A: shared shell-integration effects --------------------------
    PerformOpen {
        path: PathBuf,
    },
    PerformSave {
        path: PathBuf,
    },
    PerformSaveAs {
        path: PathBuf,
    },
    PerformAutosave,
    PerformRecovery,
    PerformSessionRestore,
    PresentNotice {
        notice: UserNotice,
    },
    /// Full authoritative editor/preview mirror resync after an incremental failure.
    PerformMirrorResync,
    RequestCloseDecision,
    QuitApplication,
}

/// MRU-ordered recent documents list.
///
/// Tracks file paths the user has opened, most-recently-used first, bounded
/// by [`MAX_RECENT_FILES`](rutile_core::MAX_RECENT_FILES). Duplicate paths are
/// deduplicated (moved to front on re-open). This is the headless contract
/// layer; the platform shell reads [`paths`](RecentDocuments::paths) to
/// populate its recent-documents menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentDocuments {
    paths: Vec<PathBuf>,
    cap: usize,
}

impl Default for RecentDocuments {
    fn default() -> Self {
        Self::new(rutile_core::MAX_RECENT_FILES)
    }
}

impl RecentDocuments {
    /// Creates an empty list bounded by `cap` (clamped to `MAX_RECENT_FILES`).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            paths: Vec::new(),
            cap: cap.min(rutile_core::MAX_RECENT_FILES),
        }
    }

    /// Records that `path` was opened, moving it to the front (MRU).
    /// Duplicate paths are removed first; the list is truncated to `cap`.
    pub fn touch(&mut self, path: PathBuf) {
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(self.cap);
    }

    /// Removes `path` from the list (e.g. user dismissed a missing file).
    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p.as_path() != path);
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Returns the MRU-ordered paths.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.paths.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Serializes to the `Vec<String>` wire format used by `SessionStateV1`.
    #[must_use]
    pub fn to_strings(&self) -> Vec<String> {
        self.paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// Deserializes from the `Vec<String>` wire format, clamped to `cap`.
    pub fn from_strings(strings: &[String], cap: usize) -> Self {
        let cap = cap.min(rutile_core::MAX_RECENT_FILES);
        Self {
            paths: strings.iter().take(cap).map(PathBuf::from).collect(),
            cap,
        }
    }
}

#[derive(Default)]
pub struct AppState {
    documents: DocumentManager,
    // Wave 2-A: durable user notices.
    notices: Vec<UserNotice>,
    next_notice_id: usize,
    recents: RecentDocuments,
    // Roadmap 06: command registry + palette interaction state.
    registry: ActionRegistry,
    palette: CommandPalette,
    // Roadmap 04: shell-level view mode.
    mode: DocumentMode,
    // Roadmap 10: distraction-free focus flag (orthogonal to mode).
    focused: bool,
    // C8: tasteroll chance-styling state.
    content_seed: u64,
    content_type: rutile_core::ContentType,
    taste: TasteState,
}

#[allow(clippy::missing_fields_in_debug)] // Intentional summary: full DocumentManager dump is too verbose.
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let slot = self.documents.active_slot();
        f.debug_struct("AppState")
            .field("tabs", &self.documents.len())
            .field("active_id", &self.documents.active_id())
            .field("active_revision", &slot.revision)
            .field("dirty", &slot.dirty)
            .field("path", &slot.path)
            .field("notices", &self.notices)
            .field("recents", &self.recents)
            .field("palette_open", &self.palette.is_open())
            .field("registry_len", &self.registry.len())
            .field("mode", &self.mode)
            .field("focused", &self.focused)
            .field("taste_active", &self.taste.is_active())
            .finish()
    }
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Syncs the active slot's revision to match the actual Document after
    /// a recovery that reconstructed the Document via `Document::new` (which
    /// resets revision to 0). Without this, edits after recovery would be
    /// rejected as stale because the slot retains the old revision.
    pub fn sync_active_revision(&mut self, revision: Revision) {
        let slot = self.documents.active_slot_mut();
        slot.revision = revision;
        slot.preview = PreviewState::Waiting { revision };
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.documents.active_slot().revision
    }

    #[must_use]
    pub fn dirty(&self) -> bool {
        self.documents.active_slot().dirty
    }

    #[must_use]
    pub fn preview(&self) -> &PreviewState {
        &self.documents.active_slot().preview
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.documents.active_slot().path.as_deref()
    }

    #[must_use]
    pub fn saved_disk(&self) -> Option<&DiskVersion> {
        self.documents.active_slot().saved_disk.as_ref()
    }

    #[must_use]
    pub fn external_conflict(&self) -> Option<&DiskVersion> {
        self.documents.active_slot().external_conflict.as_ref()
    }

    /// Mutably borrows the active document's slot (private helper).
    fn slot_mut(&mut self) -> &mut DocumentSlot {
        self.documents.active_slot_mut()
    }

    /// Borrows the active user notices.
    #[must_use]
    pub fn notices(&self) -> &[UserNotice] {
        &self.notices
    }

    /// Borrows the MRU-ordered recent-documents list (roadmap 07).
    #[must_use]
    pub const fn recents(&self) -> &RecentDocuments {
        &self.recents
    }

    /// Borrows the multi-document tab manager (roadmap 08).
    #[must_use]
    pub const fn documents(&self) -> &DocumentManager {
        &self.documents
    }
    /// Borrows the command registry (roadmap 06).
    #[must_use]
    pub const fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// Mutably borrows the command registry (for runtime platform commands).
    pub const fn registry_mut(&mut self) -> &mut ActionRegistry {
        &mut self.registry
    }

    /// Borrows the command palette interaction state (roadmap 06).
    #[must_use]
    pub const fn palette(&self) -> &CommandPalette {
        &self.palette
    }

    /// Projects the open palette for a native panel. `None` when closed.
    #[must_use]
    pub fn palette_snapshot(&self) -> Option<crate::command_palette::PalettePanelSnapshot> {
        if !self.palette.is_open() {
            return None;
        }
        let selected = self.palette.selected_index();
        let rows = self
            .palette
            .candidates()
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let available = self
                    .registry
                    .lookup(&candidate.id)
                    .is_some_and(|desc| (desc.message)(self).is_some());
                crate::command_palette::PaletteRowView {
                    title: candidate.title,
                    shortcut: candidate.shortcut.and_then(|shortcut| shortcut.display()),
                    available,
                    selected: selected == Some(index),
                }
            })
            .collect();
        Some(crate::command_palette::PalettePanelSnapshot {
            query: self.palette.query().to_owned(),
            rows,
        })
    }
    /// The shell-level view mode (roadmap 04).
    #[must_use]
    pub const fn mode(&self) -> DocumentMode {
        self.mode
    }
    /// Whether distraction-free focus mode is active (roadmap 10).
    #[must_use]
    pub const fn focused(&self) -> bool {
        self.focused
    }

    /// Borrows the C8 tasteroll chance-styling state.
    #[must_use]
    pub const fn taste(&self) -> &TasteState {
        &self.taste
    }

    /// Mutably borrows the C8 tasteroll state (for native lock/unlock calls).
    pub fn taste_mut(&mut self) -> &mut TasteState {
        &mut self.taste
    }

    /// Updates the content context the tasteroll reducer uses to seed rolls.
    ///
    /// The platform shell calls this on document open and after each edit
    /// (before dispatching [`AppMessage::TasteRoll`]) so the reducer can
    /// create a [`ChanceRoll`](rutile_core::ChanceRoll) without owning the
    /// document text.
    pub fn set_content_context(&mut self, text: &str) {
        self.content_seed = crate::tasteroll::content_seed(text);
        self.content_type = crate::tasteroll::infer_content_type(text);
    }

    /// Pushes a new notice and returns a clone for immediate presentation.
    fn push_notice(
        &mut self,
        severity: NoticeSeverity,
        message: impl Into<String>,
        source_error: impl Into<String>,
    ) -> UserNotice {
        let id = self.next_notice_id;
        self.next_notice_id = self.next_notice_id.saturating_add(1);
        let notice = UserNotice {
            id,
            severity,
            message: message.into(),
            source_error: source_error.into(),
            action_label: None,
            shown: false,
            dismissed: false,
        };
        self.notices.push(notice.clone());
        notice
    }

    /// Marks the notice with `id` as dismissed, if it exists.
    fn dismiss_notice(&mut self, id: usize) {
        if let Some(notice) = self.notices.iter_mut().find(|n| n.id == id) {
            notice.dismissed = true;
        }
    }

    /// Removes dismissed notices from the active list.
    fn gc_dismissed_notices(&mut self) {
        self.notices.retain(|notice| !notice.dismissed);
    }

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
            AppMessage::NewDocument => {
                let slot = self.documents.active_slot_mut();
                slot.revision = Revision::new(0);
                slot.dirty = false;
                slot.path = None;
                slot.saved_disk = None;
                slot.external_conflict = None;
                slot.preview = PreviewState::Waiting {
                    revision: Revision::new(0),
                };
                vec![AppEffect::ScheduleRender {
                    revision: Revision::new(0),
                }]
            }
            AppMessage::DocumentOpened {
                revision,
                path,
                disk,
            } => {
                self.recents.touch(path.clone());
                let slot = self.documents.active_slot_mut();
                slot.revision = revision;
                slot.dirty = false;
                slot.path = Some(path);
                slot.saved_disk = Some(disk);
                slot.external_conflict = None;
                slot.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::DocumentEdited { revision }
                if revision <= self.documents.active_slot().revision =>
            {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::DocumentEdited { revision } => {
                let slot = self.documents.active_slot_mut();
                slot.revision = revision;
                slot.dirty = true;
                slot.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::SaveCompleted {
                revision,
                path,
                disk,
            } if revision == self.documents.active_slot().revision => {
                let slot = self.documents.active_slot_mut();
                slot.dirty = false;
                slot.path = Some(path);
                slot.saved_disk = Some(disk);
                slot.external_conflict = None;
                vec![]
            }
            AppMessage::SaveCompleted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::SaveDurabilityUnknown {
                revision,
                path,
                disk,
            } if revision == self.documents.active_slot().revision => {
                // The atomic rename committed but parent-directory durability
                // could not be verified. Keep dirty set so the user can re-save
                // to flush the directory, but record the disk version/path so
                // external-change detection can proceed.
                let slot = self.documents.active_slot_mut();
                slot.dirty = true;
                slot.path = Some(path);
                slot.saved_disk = Some(disk);
                slot.external_conflict = None;
                vec![]
            }
            AppMessage::SaveDurabilityUnknown { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::SaveFailed { revision }
                if revision == self.documents.active_slot().revision =>
            {
                // A failed save leaves the document dirty and any conflict
                // unresolved; the platform shell must present the error and
                // keep the document open.
                vec![]
            }
            AppMessage::SaveFailed { revision } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::ExternalConflictDetected { disk } => {
                let path = {
                    let slot = self.documents.active_slot();
                    if slot.saved_disk.as_ref() == Some(&disk) {
                        return vec![];
                    }
                    slot.path.clone()
                };
                let Some(path) = path else {
                    return vec![];
                };
                self.documents.active_slot_mut().external_conflict = Some(disk.clone());
                vec![AppEffect::PresentExternalConflict { path, disk }]
            }
            AppMessage::ResolveExternalConflict(resolution) => {
                let Some(disk) = self.documents.active_slot_mut().external_conflict.take() else {
                    return vec![];
                };
                match resolution {
                    ExternalResolution::ReloadDisk => self
                        .documents
                        .active_slot()
                        .path
                        .clone()
                        .map(|path| vec![AppEffect::ReloadExternal { path }])
                        .unwrap_or_default(),
                    ExternalResolution::KeepBuffer => {
                        self.documents.active_slot_mut().saved_disk = Some(disk);
                        vec![]
                    }
                    ExternalResolution::SaveBufferAs(path) => {
                        vec![AppEffect::SaveExternalAs { path }]
                    }
                }
            }
            AppMessage::RenderAccepted {
                revision,
                page_bytes,
            } if revision == self.documents.active_slot().revision => {
                self.documents.active_slot_mut().preview = PreviewState::Navigating { revision };
                vec![AppEffect::NavigatePreview {
                    revision,
                    page_bytes,
                }]
            }
            AppMessage::RenderAccepted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::RenderFailed { revision, error }
                if revision == self.documents.active_slot().revision =>
            {
                let preview = match error {
                    RenderError::PreviewTooLarge => PreviewState::TooLarge { revision },
                    error => PreviewState::Failed { revision, error },
                };
                self.documents.active_slot_mut().preview = preview;
                vec![]
            }
            AppMessage::RenderFailed { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::PreviewEvent(event) => self.reduce_preview_event(event),
            // --- Wave 2-A: shared shell-integration reducer handling -----------
            AppMessage::OpenDocument { path } => {
                vec![AppEffect::PerformOpen { path }]
            }
            AppMessage::OpenRequestCompleted { result } => match result {
                Ok((revision, path, disk)) => {
                    self.recents.touch(path.clone());
                    let slot = self.documents.active_slot_mut();
                    slot.revision = revision;
                    slot.dirty = false;
                    slot.path = Some(path);
                    slot.saved_disk = Some(disk);
                    slot.external_conflict = None;
                    slot.preview = PreviewState::Waiting { revision };
                    vec![AppEffect::ScheduleRender { revision }]
                }
                Err(error) => {
                    let notice = self.push_notice(
                        NoticeSeverity::Error,
                        format!("Could not open document: {error}"),
                        &error,
                    );
                    vec![AppEffect::PresentNotice { notice }]
                }
            },
            AppMessage::SaveRequested => {
                let (path, dirty) = {
                    let slot = self.documents.active_slot();
                    (slot.path.clone(), slot.dirty)
                };
                let Some(path) = path else {
                    return vec![AppEffect::RequestCloseDecision];
                };
                if dirty {
                    vec![AppEffect::PerformSave { path }]
                } else {
                    vec![]
                }
            }
            AppMessage::SaveAsRequested { path } => {
                if self.documents.active_slot().dirty {
                    vec![AppEffect::PerformSaveAs { path }]
                } else {
                    vec![]
                }
            }
            AppMessage::CloseRequested { decision } => match decision {
                CloseDecision::Save { untitled_path } => {
                    let (path, dirty) = {
                        let slot = self.documents.active_slot();
                        (slot.path.clone(), slot.dirty)
                    };
                    let path = path.or(untitled_path);
                    let Some(path) = path else {
                        return vec![AppEffect::RequestCloseDecision];
                    };
                    if dirty {
                        vec![AppEffect::PerformSave { path }]
                    } else {
                        vec![AppEffect::QuitApplication]
                    }
                }
                CloseDecision::Discard => vec![AppEffect::QuitApplication],
                CloseDecision::Cancel => vec![],
            },
            AppMessage::AutosaveTick => {
                let slot = self.documents.active_slot();
                if slot.dirty && slot.autosave.is_some() {
                    vec![AppEffect::PerformAutosave]
                } else {
                    vec![]
                }
            }
            AppMessage::AutosaveCompleted { result } => match result {
                Ok(_outcome) => {
                    // The autosave entry has already been written by the shell.
                    // Nothing further to record in reducer state at this layer.
                    vec![]
                }
                Err(error) => {
                    let notice = self.push_notice(
                        NoticeSeverity::Warning,
                        format!("Autosave failed: {error}"),
                        &error,
                    );
                    vec![AppEffect::PresentNotice { notice }]
                }
            },
            AppMessage::RecoveryAdopted { document } => {
                let document_path = document.entry.document_path.as_ref().map(PathBuf::from);
                self.adopt_recovered(&document.document, document_path)
            }
            AppMessage::RecoveryDismissed => {
                // The recovery modal was dismissed; keep the empty/new state.
                vec![]
            }
            AppMessage::SessionRestored { state } => {
                self.documents = DocumentManager::new();
                self.recents = RecentDocuments::from_strings(
                    &state.recent_files,
                    rutile_core::MAX_RECENT_FILES,
                );
                let restore = self.restore_session(&state);
                if let Some(path) = restore.last_file {
                    vec![AppEffect::PerformOpen { path }]
                } else {
                    vec![]
                }
            }
            AppMessage::NoticeDismissed { id } => {
                self.dismiss_notice(id);
                self.gc_dismissed_notices();
                vec![]
            }
            AppMessage::SurfaceNotice {
                severity,
                message,
                source_error,
            } => {
                let notice = self.push_notice(severity, message, source_error);
                vec![AppEffect::PresentNotice { notice }]
            }
            AppMessage::MirrorFailed { error } => {
                // Contract: one full authoritative resync after incremental failure.
                // If a resync is already outstanding (or already failed without
                // clearing), surface a durable notice rather than retry forever.
                if self.documents.active_slot().mirror_resync_pending {
                    let notice = self.push_notice(
                        NoticeSeverity::Error,
                        format!("Preview mirror failed: {error}"),
                        &error,
                    );
                    vec![AppEffect::PresentNotice { notice }]
                } else {
                    self.documents.active_slot_mut().mirror_resync_pending = true;
                    vec![AppEffect::PerformMirrorResync]
                }
            }
            AppMessage::MirrorResyncCompleted { result } => {
                self.documents.active_slot_mut().mirror_resync_pending = false;
                match result {
                    Ok(()) => vec![],
                    Err(error) => {
                        let notice = self.push_notice(
                            NoticeSeverity::Error,
                            format!("Preview mirror could not resync: {error}"),
                            &error,
                        );
                        vec![AppEffect::PresentNotice { notice }]
                    }
                }
            }
            AppMessage::ClearRecents => {
                self.recents.clear();
                vec![]
            }
            AppMessage::RemoveRecent { path } => {
                self.recents.remove(&path);
                vec![]
            }
            AppMessage::NewTab => {
                if self.documents.new_tab().is_ok() {
                    vec![]
                } else {
                    let notice = self.push_notice(
                        NoticeSeverity::Warning,
                        "Too many open documents to create a new tab.".to_owned(),
                        "too many open documents",
                    );
                    vec![AppEffect::PresentNotice { notice }]
                }
            }
            AppMessage::SwitchTab { id } => {
                // Stale tab id is silently ignored — the reducer is the single
                // writer and tab ids are monotonic, so this indicates a stale
                // message, not a logic error the user can act on.
                let _ = self.documents.switch_tab(id);
                vec![]
            }
            AppMessage::CloseTab { id } => {
                let _ = self.documents.close_tab(id);
                vec![]
            }
            AppMessage::OpenCommandPalette => {
                self.palette.open();
                self.palette.set_query("", &self.registry);
                vec![]
            }
            AppMessage::CloseCommandPalette => {
                self.palette.close();
                vec![]
            }
            AppMessage::PaletteQueryChanged { query } => {
                self.palette.set_query(&query, &self.registry);
                vec![]
            }
            AppMessage::PaletteSelectNext => {
                self.palette.move_selection(1);
                vec![]
            }
            AppMessage::PaletteSelectPrev => {
                self.palette.move_selection(-1);
                vec![]
            }
            AppMessage::PaletteSubmit => {
                // Resolve the selected command's dispatch message against the
                // current state, close the palette, then forward the message
                // through the reducer so the transition is exercised exactly
                // once by the single writer.
                let dispatch: Option<AppMessage> = {
                    let id = self.palette.selected_candidate().map(|c| c.id);
                    id.and_then(|id| {
                        let desc = self.registry.lookup(&id).copied()?;
                        (desc.message)(self)
                    })
                };
                self.palette.close();
                match dispatch {
                    Some(msg) => self.reduce(msg),
                    None => vec![],
                }
            }
            AppMessage::SetDocumentMode { mode } => {
                self.mode = mode;
                vec![]
            }
            AppMessage::ToggleFocusMode => {
                self.focused = !self.focused;
                vec![]
            }
            AppMessage::TasteRoll => {
                self.taste.roll(self.content_seed, self.content_type);
                vec![]
            }
            AppMessage::TasteReroll => {
                self.taste.reroll();
                vec![]
            }
            AppMessage::TasteReset => {
                self.taste.reset();
                vec![]
            }
        }
    }

    fn reduce_preview_event(&mut self, event: PreviewEventV1) -> Vec<AppEffect> {
        let revision = event_revision(&event);
        if revision != self.documents.active_slot().revision {
            return vec![AppEffect::IgnoredStale { revision }];
        }

        match event {
            PreviewEventV1::BridgeReady { .. } => vec![],
            PreviewEventV1::Painted {
                revision,
                frame_seq,
            } => {
                if frame_seq >= 2 {
                    self.documents.active_slot_mut().preview = PreviewState::Ready { revision };
                }
                vec![]
            }
            PreviewEventV1::Scroll {
                revision,
                source_start,
                interaction_id,
                user,
            } => vec![AppEffect::ScrollSource {
                revision,
                source_start,
                interaction_id,
                user,
            }],
            PreviewEventV1::LinkActivated { target, .. } => {
                vec![AppEffect::PresentLink(target)]
            }
        }
    }
}

/// Wave 2S: the shared shell-integration action surface.
///
/// These methods are the vocabulary both platform lanes bind native input to.
/// Every mutating action takes `&mut Document` and routes its
/// [`EditPlan`]s through the existing transaction
/// path (`into_transaction` → `Document::apply`), then advances the reducer via
/// [`AppMessage::DocumentEdited`] so the render pipeline behaves exactly as it
/// does for typed edits. See [`crate::actions`] for the state-authority note.
impl AppState {
    // --- formatting ---------------------------------------------------------

    /// Applies a [`FormatCommand`] to `document` at `selection`.
    ///
    /// `SmartEnter` is routed through [`smart_enter`] so the decided
    /// [`SmartEnterAction`](rutile_core::SmartEnterAction) is reported;
    /// every other command goes through [`apply_format`]. On any
    /// [`EditPlanError`](rutile_core::EditPlanError) (including an empty
    /// plan — e.g. a list toggle over a blank line) the document is left
    /// untouched and a typed [`ActionError`] is returned: a clean no-op, never
    /// a panic and never a whole-buffer rewrite.
    pub fn apply_format_command(
        &mut self,
        document: &mut Document,
        selection: Selection,
        command: FormatCommand,
    ) -> Result<FormatApplied, ActionError> {
        let (action, plan) = if matches!(command, FormatCommand::SmartEnter) {
            let outcome = smart_enter(document, selection)?;
            (Some(outcome.action), outcome.plan)
        } else {
            (None, apply_format(document, selection, command)?)
        };
        let (selection_after, changes, effects) = self.apply_edit_plans(document, vec![plan])?;
        Ok(FormatApplied {
            action,
            selection_after,
            revision: document.revision(),
            changes,
            effects,
        })
    }

    /// Convenience wrapper: a smart-Enter keystroke. Equivalent to
    /// [`apply_format_command`](Self::apply_format_command) with
    /// [`FormatCommand::SmartEnter`].
    pub fn smart_enter(
        &mut self,
        document: &mut Document,
        selection: Selection,
    ) -> Result<FormatApplied, ActionError> {
        self.apply_format_command(document, selection, FormatCommand::SmartEnter)
    }

    // --- find / replace -----------------------------------------------------

    /// Opens (or replaces) the active find session.
    pub fn start_find(&mut self, query: FindQuery, direction: FindDirection, wrap: bool) {
        self.slot_mut().find = Some(FindSession::new(query, direction, wrap));
    }

    /// Borrows the active find session, if any.
    #[must_use]
    pub fn find_session(&self) -> Option<&FindSession> {
        self.documents.active_slot().find.as_ref()
    }

    /// Closes the active find session.
    pub fn end_find(&mut self) {
        self.slot_mut().find = None;
    }

    /// Finds the next match in the session's direction from `from_byte`,
    /// recording it as the session's current match.
    pub fn find_next(
        &mut self,
        document: &Document,
        from_byte: usize,
    ) -> Result<Option<Range<usize>>, ActionError> {
        let direction = self
            .documents
            .active_slot()
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .direction;
        Ok(self.locate(document, from_byte, direction))
    }

    /// Finds the previous match (the session's direction, inverted) from
    /// `from_byte`, recording it as the session's current match.
    pub fn find_prev(
        &mut self,
        document: &Document,
        from_byte: usize,
    ) -> Result<Option<Range<usize>>, ActionError> {
        let direction = self
            .documents
            .active_slot()
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .direction;
        let reversed = match direction {
            FindDirection::Forward => FindDirection::Backward,
            FindDirection::Backward => FindDirection::Forward,
        };
        Ok(self.locate(document, from_byte, reversed))
    }

    /// Replaces the session's current match with `replacement`. A missing
    /// current match is a clean no-op.
    pub fn replace_current(
        &mut self,
        document: &mut Document,
        replacement: String,
    ) -> Result<ReplaceApplied, ActionError> {
        let (query, current) = {
            let session = self
                .documents
                .active_slot()
                .find
                .as_ref()
                .ok_or(ActionError::NoFindSession)?;
            (session.query.clone(), session.current.clone())
        };
        let Some(current) = current else {
            return Ok(ReplaceApplied {
                replaced: 0,
                selection_after: None,
                revision: document.revision(),
                changes: Vec::new(),
                effects: Vec::new(),
            });
        };
        let spec = ReplaceSpec::new(query, replacement)?;
        let text = document.snapshot().to_string();
        let plan = rutile_core::replace_current(document.revision(), &text, &spec, current)?;
        let (selection_after, changes, effects) = self.apply_edit_plans(document, vec![plan])?;
        if let Some(session) = self.documents.active_slot_mut().find.as_mut() {
            session.current = None;
        }
        Ok(ReplaceApplied {
            replaced: 1,
            selection_after: Some(selection_after),
            revision: document.revision(),
            changes,
            effects,
        })
    }

    /// Replaces every match of the session's query with `replacement`.
    ///
    /// The engine chunks large replace-alls into a sequence of bounded
    /// [`EditPlan`]s; they are applied in order through the transaction path so
    /// a replace-all touching more than one plan still commits fully. Zero
    /// matches is a clean no-op.
    pub fn replace_all(
        &mut self,
        document: &mut Document,
        replacement: String,
    ) -> Result<ReplaceApplied, ActionError> {
        let query = self
            .documents
            .active_slot()
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .query
            .clone();
        let spec = ReplaceSpec::new(query, replacement)?;
        let text = document.snapshot().to_string();
        let plans = rutile_core::replace_all(document.revision(), &text, &spec)?;
        if plans.is_empty() {
            return Ok(ReplaceApplied {
                replaced: 0,
                selection_after: None,
                revision: document.revision(),
                changes: Vec::new(),
                effects: Vec::new(),
            });
        }
        let replaced = rutile_core::match_count(&text, spec.query());
        let (selection_after, changes, effects) = self.apply_edit_plans(document, plans)?;
        if let Some(session) = self.documents.active_slot_mut().find.as_mut() {
            session.current = None;
        }
        Ok(ReplaceApplied {
            replaced,
            selection_after: Some(selection_after),
            revision: document.revision(),
            changes,
            effects,
        })
    }

    // --- programmatic insert / paste ---------------------------------------

    /// Replaces `selection` with `text` as a single bounded programmatic edit,
    /// advancing the reducer exactly as every other shared action does and
    /// returning the [`ChangeSet`] so a shell can follow the mutation
    /// incrementally (viewport-preserving) instead of reinstalling the buffer.
    ///
    /// This is the shared smart-paste primitive both shells route through: the
    /// Linux and macOS lanes each convert clipboard HTML to markdown via the
    /// bounded core `html_to_markdown` (which enforces its 2 MiB input bound) and
    /// lower the result — or a plain-text fallback — through here. A single
    /// paste can far exceed the per-edit *formatting* budget, so this
    /// deliberately applies a raw [`EditTransaction`] rather than an
    /// [`EditPlan`]; the 20 MiB document cap is still enforced by
    /// [`Document::apply`], surfacing as [`EditError::TooLarge`].
    pub fn insert_text(
        &mut self,
        document: &mut Document,
        selection: Selection,
        text: &str,
    ) -> Result<InsertApplied, ActionError> {
        let start = selection.anchor.min(selection.head);
        let end = selection.anchor.max(selection.head);
        let id = {
            let slot = self.documents.active_slot_mut();
            let id = slot.next_transaction_id;
            slot.next_transaction_id = slot.next_transaction_id.saturating_add(1);
            id
        };
        let change = document.apply(EditTransaction {
            base_revision: document.revision(),
            id,
            kind: TransactionKind::Programmatic,
            edits: vec![Edit {
                byte_range: start..end,
                replacement: text.to_owned(),
            }],
        })?;
        let selection_after = Selection::collapsed(start.saturating_add(text.len()));
        let effects = self.reduce(AppMessage::DocumentEdited {
            revision: document.revision(),
        });
        Ok(InsertApplied {
            selection_after,
            revision: document.revision(),
            changes: vec![change],
            effects,
        })
    }

    // --- export -------------------------------------------------------------

    /// Produces a validated, self-contained export page for `document` plus a
    /// suggested file name, for both shell gestures (save-as-HTML and
    /// copy-as-HTML). The actual file write / clipboard set is the platform
    /// lane's job. When `title` is `None` the document's file stem is used.
    pub fn export_html(
        &self,
        document: &Document,
        title: Option<String>,
    ) -> Result<ExportOutput, ExportError> {
        let stem = self
            .path()
            .and_then(Path::file_stem)
            .map(|stem| stem.to_string_lossy().into_owned());
        let title = title.or_else(|| stem.clone());
        let request = ExportRequest::new(document.revision(), title)?;
        let source = document.snapshot().to_string();
        let page = render_export_page(&source, &request)?;
        let preset = PublishingPreset::print_ready();
        let html = splice_print_preset(page.as_html(), &preset)?;
        Ok(ExportOutput {
            html,
            suggested_file_name: preset.suggested_filename(stem.as_deref().unwrap_or("")),
        })
    }

    // --- counts -------------------------------------------------------------

    /// Live word/character/reading-time counts for `document`'s current text,
    /// for the native status bar.
    #[must_use]
    pub fn counts(&self, document: &Document) -> Counts {
        rutile_core::counts(&document.snapshot().to_string())
    }

    // --- autosave / session -------------------------------------------------

    /// Binds the autosave/session store this state owns. Sequence allocation
    /// is internal to the store, so recovery-then-continue keeps monotonically
    /// increasing sequences without caller bookkeeping.
    pub fn bind_autosave(&mut self, store: AutosaveStore) -> Result<(), AutosaveError> {
        self.slot_mut().autosave = Some(store);
        Ok(())
    }

    /// Borrows the bound autosave store, if any.
    #[must_use]
    pub fn autosave_store(&self) -> Option<&AutosaveStore> {
        self.documents.active_slot().autosave.as_ref()
    }

    /// Writes one autosave entry for `document`'s current snapshot. Returns
    /// `Ok(None)` when no store is bound.
    pub fn autosave_tick(
        &mut self,
        document: &Document,
        captured_at_unix_ms: u64,
    ) -> Result<Option<AutosaveEntryV1>, AutosaveError> {
        let Some(store) = self.documents.active_slot().autosave.clone() else {
            return Ok(None);
        };
        let snapshot = document.snapshot();
        let document_path = self.path().map(|path| path.to_string_lossy().into_owned());
        let outcome = store.record(&snapshot, document_path.as_deref(), captured_at_unix_ms)?;
        Ok(Some(outcome.entry))
    }

    /// Recovery-on-startup: returns the highest verifiable autosaved document,
    /// or `None` when there is nothing to recover / no store is bound.
    pub fn recover(&self) -> Result<Option<RecoveredDocument>, AutosaveError> {
        match &self.documents.active_slot().autosave {
            Some(store) => Ok(store.recover()?.recovered),
            None => Ok(None),
        }
    }

    /// Adopts a crash-recovered [`Document`] as the working buffer.
    ///
    /// The reducer takes on `document`'s own revision and is associated with the
    /// recovered `document_path` (the file the autosave entry was capturing), so
    /// saves and session-restore target the original file rather than an
    /// untitled buffer. The buffer is marked dirty because the recovered
    /// snapshot is newer than any on-disk copy, and `saved_disk` is cleared (the
    /// recovered content has never been written). Returns the render effect to
    /// schedule.
    ///
    /// This preserves what the app *can* preserve without touching the frozen
    /// core contracts: unlike inserting the recovered text as a fresh edit off a
    /// new untitled document (which advanced the revision by one, added a
    /// spurious undo entry, and dropped the path), the caller swaps the recovered
    /// document in directly. The original numeric revision recorded in the
    /// journal entry (`document_revision`) cannot be restored into the `Document`
    /// itself — core reconstructs every recovered snapshot at revision 0 and
    /// exposes no revision setter — so that part is a core follow-up; a
    /// freshly-loaded document already starts at revision 0, so this matches the
    /// open-a-file baseline.
    pub fn adopt_recovered(
        &mut self,
        document: &Document,
        document_path: Option<PathBuf>,
    ) -> Vec<AppEffect> {
        let revision = document.revision();
        let slot = self.documents.active_slot_mut();
        slot.revision = revision;
        slot.dirty = true;
        slot.path = document_path;
        slot.saved_disk = None;
        slot.external_conflict = None;
        slot.preview = PreviewState::Waiting { revision };
        vec![AppEffect::ScheduleRender { revision }]
    }

    /// Captures session-restore state from the current document path plus the
    /// platform-supplied selection, viewport, and window frame.
    #[must_use]
    pub fn capture_session_state(
        &self,
        saved_at_unix_ms: u64,
        selection: Option<Selection>,
        top_visible_byte: Option<usize>,
        window: Option<SessionWindowV1>,
    ) -> SessionStateV1 {
        let last_file = self.path().map(|path| path.to_string_lossy().into_owned());
        let recent_files = self.recents.to_strings();
        SessionStateV1 {
            schema: SESSION_SCHEMA_V1.to_owned(),
            v: 1,
            saved_at_unix_ms,
            last_file,
            selection: selection.map(|selection| SessionSelectionV1 {
                anchor: selection.anchor as u64,
                head: selection.head as u64,
            }),
            top_visible_byte: top_visible_byte.map(|byte| byte as u64),
            window,
            recent_files,
        }
    }

    /// Persists session-restore `state` through the bound store. A no-op when
    /// no store is bound.
    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), AutosaveError> {
        match &self.documents.active_slot().autosave {
            Some(store) => store.save_session_state(state),
            None => Ok(()),
        }
    }

    /// Loads and re-validates persisted session state, or `None` when absent /
    /// no store is bound.
    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, AutosaveError> {
        match &self.documents.active_slot().autosave {
            Some(store) => store.load_session_state(),
            None => Ok(None),
        }
    }

    /// Lowers a persisted [`SessionStateV1`] into the platform-actionable parts
    /// a shell restores. Offsets are advisory: the shell re-validates them
    /// against the loaded document.
    pub fn restore_session(&self, state: &SessionStateV1) -> SessionRestore {
        SessionRestore {
            last_file: state.last_file.as_ref().map(PathBuf::from),
            selection: state.selection.map(|selection| Selection {
                anchor: selection.anchor as usize,
                head: selection.head as usize,
            }),
            top_visible_byte: state.top_visible_byte.map(|byte| byte as usize),
            window: state.window,
        }
    }

    // --- shared internals ---------------------------------------------------

    /// Locates a match with the active session's query and records it as the
    /// session's current match. Returns `None` with no active session.
    fn locate(
        &mut self,
        document: &Document,
        from_byte: usize,
        direction: FindDirection,
    ) -> Option<Range<usize>> {
        let session = self.documents.active_slot_mut().find.as_mut()?;
        let text = document.snapshot().to_string();
        let found =
            rutile_core::find_next(&text, &session.query, from_byte, direction, session.wrap);
        session.current.clone_from(&found);
        found
    }

    /// Applies a non-empty sequence of edit plans through the existing
    /// transaction path and advances the reducer once at the final revision.
    /// Returns the last plan's selection, the per-plan [`ChangeSet`]s (in commit
    /// order, so a shell can follow the mutation incrementally), and the reducer
    /// effects.
    fn apply_edit_plans(
        &mut self,
        document: &mut Document,
        plans: Vec<EditPlan>,
    ) -> Result<(Selection, Vec<ChangeSet>, Vec<AppEffect>), EditError> {
        debug_assert!(
            !plans.is_empty(),
            "apply_edit_plans requires a non-empty batch"
        );
        // All-or-nothing size guard. The plans are applied in sequence and each
        // `Document::apply` rejects a post-edit document over the cap. If a
        // *middle* plan failed that way, earlier plans would already be committed
        // to the document while the reducer and adapter stayed behind — wedging
        // every later edit with a StaleRevision until reload. Project the running
        // document size across the whole batch up front and reject the entire
        // batch (before mutating anything) if it would ever cross the cap, so a
        // cap-crossing replace-all can't half-apply.
        let mut projected = document.len_bytes() as isize;
        let mut peak = projected;
        for plan in &plans {
            for edit in plan.edits() {
                let span = (edit.byte_range.end - edit.byte_range.start) as isize;
                projected += edit.replacement.len() as isize - span;
            }
            peak = peak.max(projected);
        }
        if peak > rutile_core::MAX_DOCUMENT_BYTES as isize {
            return Err(EditError::TooLarge);
        }
        let mut selection_after = Selection::collapsed(0);
        let mut changes = Vec::with_capacity(plans.len());
        for plan in plans {
            selection_after = plan.selection_after();
            let id = {
                let slot = self.documents.active_slot_mut();
                let id = slot.next_transaction_id;
                slot.next_transaction_id = slot.next_transaction_id.saturating_add(1);
                id
            };
            changes.push(document.apply(plan.into_transaction(id))?);
        }
        let effects = self.reduce(AppMessage::DocumentEdited {
            revision: document.revision(),
        });
        Ok((selection_after, changes, effects))
    }
}

/// Injects the publishing print stylesheet before `</head>` and re-validates.
fn splice_print_preset(html: &str, preset: &PublishingPreset) -> Result<String, ExportError> {
    const HEAD_END: &str = "</head>";
    let index = html.find(HEAD_END).ok_or(ExportViolation::MissingCsp)?;
    let block = preset.print_style_block();
    let mut spliced = String::with_capacity(html.len().saturating_add(block.len()));
    spliced.push_str(&html[..index]);
    spliced.push_str(&block);
    spliced.push_str(&html[index..]);
    Ok(ExportPage::from_html(spliced)?.into_html())
}

const fn event_revision(event: &PreviewEventV1) -> Revision {
    match event {
        PreviewEventV1::BridgeReady { revision }
        | PreviewEventV1::Painted { revision, .. }
        | PreviewEventV1::Scroll { revision, .. }
        | PreviewEventV1::LinkActivated { revision, .. } => *revision,
    }
}
