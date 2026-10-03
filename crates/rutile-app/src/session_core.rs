//! Shared document-session core for native platform shells.
//!
//! Wave 2-A requires exactly one [`DocumentSessionCore`] authority per open
//! document. Platform `ProductSession` types compose this core and never become
//! independent document authorities: adapters perform effects, the core owns
//! [`AppState`] and [`Document`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rutile_core::{DiskVersion, Document, DocumentSnapshot};

use crate::app::{AppEffect, AppMessage, AppState, CloseDecision, PreviewState};
use crate::app::{NoticeSeverity, UserNotice};
use rutile_types::{DocumentId, Revision};

/// A fully prepared restored tab. File reads happen in the effect executor;
/// installing the complete set below is atomic and I/O-free.
pub struct RestoredDocument {
    pub view: rutile_core::WorkspaceViewState,
    pub document: Document,
    pub path: Option<PathBuf>,
    pub disk: Option<DiskVersion>,
    pub dirty: bool,
}

/// Sole `AppState` / `Document` authority for one open document.
pub struct DocumentSessionCore {
    history: crate::revision_history::SnapshotHistory,
    app: AppState,
    document: Document,
    /// Parked ropes for inactive tabs. Path/dirty/revision stay on `DocumentSlot`.
    parked: HashMap<DocumentId, Document>,
    /// Generation of the latest open request. Completions with a mismatched
    /// generation are ignored so stale async opens cannot roll state backward.
    open_generation: u64,
    /// Advances when a document rope is replaced, including same-file revision-zero reloads.
    document_generation: u64,
}

impl DocumentSessionCore {
    pub fn checkpoint(
        &mut self,
        timestamp_ms: u64,
        source: crate::revision_history::HistorySource,
        description: &str,
    ) -> Result<u64, String> {
        self.history.record(
            self.app.documents().active_id(),
            self.snapshot(),
            timestamp_ms,
            source,
            description,
        )
    }

    pub fn checkpoints(
        &self,
    ) -> impl Iterator<Item = &crate::revision_history::SnapshotCheckpoint> {
        self.history.entries(self.app.documents().active_id())
    }

    pub fn restore_checkpoint(
        &mut self,
        id: u64,
        timestamp_ms: u64,
    ) -> Result<crate::actions::InsertApplied, String> {
        if self.app.mode() == crate::app::DocumentMode::View {
            return Err("Switch to an editing view before restoring history".into());
        }
        let snapshot = self
            .history
            .get(self.app.documents().active_id(), id)
            .ok_or("Checkpoint is no longer available for this document")?
            .snapshot
            .clone();
        let current = self.snapshot();
        let selection = rutile_core::Selection {
            anchor: 0,
            head: self.document.len_bytes(),
        };
        self.history.record(
            self.app.documents().active_id(),
            current,
            timestamp_ms,
            crate::revision_history::HistorySource::BeforeBulk,
            "Before restore",
        )?;
        let applied = self
            .app
            .insert_text(&mut self.document, selection, &snapshot.to_string())
            .map_err(|error| error.to_string())?;
        Ok(applied)
    }

    pub fn recovery_snapshot(&self) -> Result<rutile_core::WorkspaceRecoverySnapshot, String> {
        let active = self.app.documents().active_id();
        let mut tabs = Vec::new();
        for &id in self.app.documents().tab_order() {
            let slot = self
                .app
                .documents()
                .slot(id)
                .ok_or("missing document slot")?;
            let document = if id == active {
                &self.document
            } else {
                self.parked.get(&id).ok_or("missing parked document")?
            };
            let path = slot
                .path
                .as_ref()
                .map(|path| {
                    path.to_str()
                        .map(str::to_owned)
                        .ok_or("document path is not UTF-8")
                })
                .transpose()?;
            tabs.push(rutile_core::WorkspaceRecoveryTab {
                id: id.get(),
                view: slot.view.validated(&document.snapshot()),
                path,
                snapshot: slot.dirty.then(|| document.snapshot()),
            });
        }
        Ok(rutile_core::WorkspaceRecoverySnapshot {
            active_id: active.get(),
            tabs,
        })
    }

    pub fn install_restored_documents(
        &mut self,
        documents: Vec<RestoredDocument>,
        active_index: usize,
    ) -> Result<(), String> {
        use crate::document_manager::{DocumentManager, MAX_OPEN_DOCUMENTS};
        if !self.dirty_document_ids().is_empty() {
            return Err("Save or close unsaved documents before restoring a session".into());
        }
        if documents.is_empty()
            || documents.len() > MAX_OPEN_DOCUMENTS
            || active_index >= documents.len()
        {
            return Err("Invalid restored tab set".into());
        }
        let mut manager = DocumentManager::new();
        if let Some(store) = self.app.autosave_store() {
            manager.bind_autosave_store(store);
        }
        let mut ropes = HashMap::new();
        let mut paths = std::collections::HashSet::new();
        let mut active_id = DocumentId::ROOT;
        for (index, restored) in documents.into_iter().enumerate() {
            if let Some(path) = &restored.path {
                if !paths.insert(path.clone()) {
                    return Err("Duplicate restored document path".into());
                }
            }
            let id = if index == 0 {
                DocumentId::ROOT
            } else {
                manager.new_tab().map_err(|error| error.to_string())?
            };
            let slot = manager.slot_mut(id).ok_or("missing restored slot")?;
            slot.view = restored.view.validated(&restored.document.snapshot());
            slot.path = restored.path;
            slot.saved_disk = restored.disk;
            slot.dirty = restored.dirty;
            slot.revision = restored.document.revision();
            slot.preview = PreviewState::Waiting {
                revision: slot.revision,
            };
            ropes.insert(id, restored.document);
            if index == active_index {
                active_id = id;
            }
        }
        manager
            .switch_tab(active_id)
            .map_err(|error| error.to_string())?;
        let document = ropes
            .remove(&active_id)
            .ok_or("missing active restored document")?;
        self.app.replace_documents(manager);
        self.document = document;
        self.parked = ropes;
        self.history = crate::revision_history::SnapshotHistory::default();
        self.open_generation = self.open_generation.saturating_add(1);
        self.document_generation = self.document_generation.saturating_add(1);
        Ok(())
    }

    /// Builds a core around an in-memory starter document and applies
    /// [`AppMessage::NewDocument`].
    pub fn new_in_memory(source: &str) -> Result<Self, String> {
        let document = Document::new(source).map_err(|error| error.to_string())?;
        let mut app = AppState::new();
        let _ = app.reduce(AppMessage::NewDocument);
        Ok(Self {
            history: crate::revision_history::SnapshotHistory::default(),
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
            document_generation: 0,
        })
    }

    /// Builds a core from an already-loaded document bound to a path/disk pair.
    #[must_use]
    pub fn from_opened(document: Document, path: PathBuf, disk: DiskVersion) -> Self {
        let mut app = AppState::new();
        let _ = app.reduce(AppMessage::DocumentOpened {
            revision: document.revision(),
            path,
            disk,
        });
        Self {
            history: crate::revision_history::SnapshotHistory::default(),
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
            document_generation: 0,
        }
    }

    /// Builds a core from an already-constructed document and app state
    /// (used when the platform has already driven the open reducer).
    #[must_use]
    pub fn from_parts(app: AppState, document: Document) -> Self {
        Self {
            history: crate::revision_history::SnapshotHistory::default(),
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
            document_generation: 0,
        }
    }

    #[must_use]
    pub const fn app(&self) -> &AppState {
        &self.app
    }

    pub const fn app_mut(&mut self) -> &mut AppState {
        &mut self.app
    }

    #[must_use]
    pub const fn document(&self) -> &Document {
        &self.document
    }

    pub const fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }

    pub fn set_document(&mut self, document: Document) {
        self.document = document;
        self.document_generation = self.document_generation.saturating_add(1);
    }

    #[must_use]
    pub const fn document_generation(&self) -> u64 {
        self.document_generation
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    #[must_use]
    pub fn dirty(&self) -> bool {
        self.app.dirty()
    }

    /// Window close must account for unsaved background tabs as well.
    pub fn dirty_document_ids(&self) -> Vec<DocumentId> {
        self.app
            .documents()
            .tab_order()
            .iter()
            .copied()
            .filter(|id| {
                self.app
                    .documents()
                    .slot(*id)
                    .is_some_and(|slot| slot.dirty)
            })
            .collect()
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.app.path()
    }

    #[must_use]
    pub fn snapshot(&self) -> DocumentSnapshot {
        self.document.snapshot()
    }

    pub fn snapshot_for_document(&self, id: DocumentId) -> Option<DocumentSnapshot> {
        if id == self.app.documents().active_id() {
            Some(self.snapshot())
        } else {
            self.parked.get(&id).map(Document::snapshot)
        }
    }

    #[must_use]
    pub fn preview(&self) -> &PreviewState {
        self.app.preview()
    }

    #[must_use]
    pub fn notices(&self) -> &[UserNotice] {
        self.app.notices()
    }

    #[must_use]
    pub const fn open_generation(&self) -> u64 {
        self.open_generation
    }

    /// Allocates the next open generation for a shared open request.
    pub const fn begin_open(&mut self) -> u64 {
        self.open_generation = self.open_generation.saturating_add(1);
        self.open_generation
    }

    /// Returns true when `generation` matches the latest open request.
    #[must_use]
    pub const fn is_current_open(&self, generation: u64) -> bool {
        generation == self.open_generation
    }

    fn empty_document() -> Document {
        Document::new("").expect("empty document is always within MAX_DOCUMENT_BYTES")
    }

    fn park_active(&mut self) {
        let id = self.app.documents().active_id();
        let current = std::mem::replace(&mut self.document, Self::empty_document());
        self.parked.insert(id, current);
    }

    fn load_active(&mut self) {
        let id = self.app.documents().active_id();
        self.document = self.parked.remove(&id).unwrap_or_else(Self::empty_document);
    }

    /// Installs a freshly loaded file as the active tab (design D4).
    ///
    /// - Same path already open: focus it and preserve unsaved contents.
    ///   A clean buffer may adopt a refreshed disk version.
    /// - Single untitled clean tab: replace in place (startup / first open).
    /// - Otherwise: park the current tab and open a new one.
    pub fn adopt_opened_document(
        &mut self,
        document: Document,
        path: PathBuf,
        disk: DiskVersion,
    ) -> Vec<AppEffect> {
        if let Some(existing) = self.app.documents().find_by_path(&path) {
            let effects = self.reduce(AppMessage::SwitchTab { id: existing });
            if self.dirty() {
                return effects;
            }
            self.set_document(document);
            return self.app.reduce(AppMessage::DocumentOpened {
                revision: self.document.revision(),
                path,
                disk,
            });
        }

        let replace_in_place =
            self.app.documents().len() == 1 && !self.app.dirty() && self.app.path().is_none();
        if replace_in_place {
            self.history
                .remove_document(self.app.documents().active_id());
        }
        if !replace_in_place {
            self.park_active();
            let effects = self.app.reduce(AppMessage::NewTab);
            if effects
                .iter()
                .any(|effect| matches!(effect, AppEffect::PresentNotice { .. }))
            {
                self.load_active();
                return effects;
            }
        }
        self.set_document(document);
        self.app.reduce(AppMessage::DocumentOpened {
            revision: self.document.revision(),
            path,
            disk,
        })
    }

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
            AppMessage::PaletteSubmit => {
                let dispatch = self.app.selected_palette_message();
                self.app.reduce(AppMessage::CloseCommandPalette);
                dispatch.map_or_else(Vec::new, |message| self.reduce(message))
            }
            AppMessage::NewDocument if !self.dirty() => {
                self.history
                    .remove_document(self.app.documents().active_id());
                self.set_document(Self::empty_document());
                self.app.reduce(AppMessage::NewDocument)
            }
            AppMessage::NewDocument => self.surface_notice(
                NoticeSeverity::Warning,
                "Save or close the current document before creating a new document",
                "unsaved document",
            ),
            AppMessage::NewTab => {
                self.park_active();
                let effects = self.app.reduce(AppMessage::NewTab);
                self.load_active();
                effects
            }
            AppMessage::SwitchTab { id } => {
                if id == self.app.documents().active_id() {
                    return vec![];
                }
                self.park_active();
                let effects = self.app.reduce(AppMessage::SwitchTab { id });
                self.load_active();
                effects
            }
            AppMessage::CloseTab { id } => {
                if self.app.documents().len() <= 1 {
                    return vec![];
                }
                let was_active = id == self.app.documents().active_id();
                let effects = self.app.reduce(AppMessage::CloseTab { id });
                if effects
                    .iter()
                    .any(|effect| matches!(effect, AppEffect::RequestTabCloseDecision { .. }))
                {
                    return effects;
                }
                self.parked.remove(&id);
                self.history.remove_document(id);
                if was_active {
                    self.load_active();
                }
                effects
            }
            AppMessage::TabCloseDecided {
                id,
                decision: CloseDecision::Discard,
            } => {
                if self.app.documents().len() <= 1 {
                    return vec![];
                }
                let was_active = id == self.app.documents().active_id();
                let effects = self.app.reduce(AppMessage::TabCloseDecided {
                    id,
                    decision: CloseDecision::Discard,
                });
                self.parked.remove(&id);
                self.history.remove_document(id);
                if was_active {
                    self.load_active();
                }
                effects
            }
            other => self.app.reduce(other),
        }
    }

    pub fn surface_notice(
        &mut self,
        severity: NoticeSeverity,
        message: impl Into<String>,
        source_error: impl Into<String>,
    ) -> Vec<AppEffect> {
        self.app.reduce(AppMessage::SurfaceNotice {
            severity,
            message: message.into(),
            source_error: source_error.into(),
        })
    }

    /// Split-borrow helper: mutable app + mutable document.
    pub const fn app_and_document_mut(&mut self) -> (&mut AppState, &mut Document) {
        (&mut self.app, &mut self.document)
    }

    /// Split-borrow helper: mutable app + immutable document.
    pub const fn app_mut_and_document(&mut self) -> (&mut AppState, &Document) {
        (&mut self.app, &self.document)
    }

    /// Split-borrow helper: immutable app + immutable document.
    #[must_use]
    pub const fn app_and_document(&self) -> (&AppState, &Document) {
        (&self.app, &self.document)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn checkpoint_restore_is_tab_scoped_and_undoable() {
        use crate::revision_history::HistorySource;
        let mut core = super::DocumentSessionCore::new_in_memory("old café").unwrap();
        let first = core.app().documents().active_id();
        let checkpoint = core.checkpoint(1, HistorySource::Manual, "Manual").unwrap();
        core.set_document(rutile_core::Document::new("current 🪶").unwrap());
        core.reduce(crate::app::AppMessage::NewTab);
        assert!(core.restore_checkpoint(checkpoint, 2).is_err());
        core.reduce(crate::app::AppMessage::SwitchTab { id: first });
        core.restore_checkpoint(checkpoint, 3).unwrap();
        assert_eq!(core.snapshot().to_string(), "old café");
        assert_eq!(
            core.checkpoints().next().unwrap().snapshot.to_string(),
            "current 🪶"
        );
        assert!(core.dirty());
        core.document_mut().undo().unwrap();
        assert_eq!(core.snapshot().to_string(), "current 🪶");
    }

    use super::*;
    use crate::app::AppMessage;
    use std::path::PathBuf;

    #[test]
    fn open_generation_advances_and_detects_stale() {
        let mut core = DocumentSessionCore::new_in_memory("# hi\n").unwrap();
        let g1 = core.begin_open();
        assert_eq!(g1, 1);
        assert!(core.is_current_open(1));
        let g2 = core.begin_open();
        assert_eq!(g2, 2);
        assert!(!core.is_current_open(1));
        assert!(core.is_current_open(2));
    }

    #[test]
    fn core_is_sole_app_document_authority() {
        let mut core = DocumentSessionCore::new_in_memory("x").unwrap();
        assert!(!core.dirty());
        let _ = core.reduce(AppMessage::DocumentEdited {
            revision: Revision::new(1),
        });
        // NewDocument path starts clean; edit without document apply keeps dirty false
        // until DocumentEdited with matching revision after real edit — still owns app.
        assert!(core.app().path().is_none());
        let path = PathBuf::from("/tmp/core-open.md");
        let generation = core.begin_open();
        let effects = core.reduce(AppMessage::OpenDocument { path });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, AppEffect::PerformOpen { .. }))
        );
        assert_eq!(core.open_generation(), generation);
    }

    #[test]
    fn new_tab_and_switch_restore_distinct_ropes() {
        let mut core = DocumentSessionCore::new_in_memory("alpha").unwrap();
        let first = core.app().documents().active_id();
        let _ = core.reduce(AppMessage::NewTab);
        assert_eq!(core.document().snapshot().to_string(), "");
        core.set_document(Document::new("beta").unwrap());
        let second = core.app().documents().active_id();
        assert_ne!(first, second);
        let _ = core.reduce(AppMessage::SwitchTab { id: first });
        assert_eq!(core.document().snapshot().to_string(), "alpha");
        let _ = core.reduce(AppMessage::SwitchTab { id: second });
        assert_eq!(core.document().snapshot().to_string(), "beta");
    }

    #[test]
    fn new_tab_still_autosaves() {
        let dir =
            std::env::temp_dir().join(format!("rutile-session-autosave-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut core = DocumentSessionCore::new_in_memory("first").unwrap();
        core.app_mut()
            .bind_autosave(rutile_core::AutosaveStore::new(&dir))
            .unwrap();
        let _ = core.reduce(AppMessage::NewTab);
        core.set_document(Document::new("second").unwrap());
        let (app, document) = core.app_mut_and_document();
        assert!(app.autosave_tick(document, 1).unwrap().is_some());
    }

    fn sample_disk(name: &str, source: &str) -> (PathBuf, DiskVersion) {
        use rutile_core::{FileService, LocalFileService};
        let path = std::env::temp_dir().join(format!("rutile-adopt-{name}-{}", std::process::id()));
        let document = Document::new(source).unwrap();
        let disk = match LocalFileService::new().save_atomic(&path, &document.snapshot()) {
            rutile_core::SaveOutcome::Committed { disk } => disk,
            other => panic!("expected committed save, got {other:?}"),
        };
        (path, disk)
    }

    #[test]
    fn first_open_replaces_the_single_untitled_tab() {
        let mut core = DocumentSessionCore::new_in_memory("starter").unwrap();
        let (path, disk) = sample_disk("first", "from disk");
        let _ = core.adopt_opened_document(Document::new("from disk").unwrap(), path, disk);
        assert_eq!(core.app().documents().len(), 1);
        assert_eq!(core.document().snapshot().to_string(), "from disk");
    }

    #[test]
    fn open_after_edit_parks_the_current_tab() {
        let mut core = DocumentSessionCore::new_in_memory("keep me").unwrap();
        let first = core.app().documents().active_id();
        let _ = core.reduce(AppMessage::DocumentEdited {
            revision: Revision::new(1),
        });
        let (path, disk) = sample_disk("second", "opened");
        let _ = core.adopt_opened_document(Document::new("opened").unwrap(), path, disk);
        assert_eq!(core.app().documents().len(), 2);
        assert_eq!(core.document().snapshot().to_string(), "opened");
        let _ = core.reduce(AppMessage::SwitchTab { id: first });
        assert_eq!(core.document().snapshot().to_string(), "keep me");
    }

    #[test]
    fn reopen_same_path_switches_instead_of_duplicating() {
        let mut core = DocumentSessionCore::new_in_memory("").unwrap();
        let (path, disk) = sample_disk("dup", "once");
        let _ =
            core.adopt_opened_document(Document::new("once").unwrap(), path.clone(), disk.clone());
        let _ = core.reduce(AppMessage::NewTab);
        let _ = core.adopt_opened_document(Document::new("once").unwrap(), path, disk);
        assert_eq!(core.app().documents().len(), 2);
        assert_eq!(core.document().snapshot().to_string(), "once");
    }

    #[test]
    fn last_tab_close_is_a_noop() {
        let mut core = DocumentSessionCore::new_in_memory("only").unwrap();
        let id = core.app().documents().active_id();
        let effects = core.reduce(AppMessage::CloseTab { id });
        assert!(effects.is_empty());
        assert_eq!(core.document().snapshot().to_string(), "only");
        assert_eq!(core.app().documents().len(), 1);
    }

    #[test]
    fn reopening_a_dirty_tab_preserves_unsaved_text() {
        let (path, disk) = sample_disk("dirty-duplicate", "disk");
        let mut core = DocumentSessionCore::from_opened(
            Document::new("disk").unwrap(),
            path.clone(),
            disk.clone(),
        );
        let first = core.app().documents().active_id();
        core.set_document(Document::new("unsaved 🦀").unwrap());
        core.reduce(AppMessage::DocumentEdited {
            revision: Revision::new(1),
        });
        core.reduce(AppMessage::NewTab);
        core.adopt_opened_document(Document::new("disk").unwrap(), path, disk);
        assert_eq!(core.app().documents().active_id(), first);
        assert_eq!(core.snapshot().to_string(), "unsaved 🦀");
        assert!(core.dirty());
    }

    #[test]
    fn palette_new_tab_parks_the_existing_rope() {
        let mut core = DocumentSessionCore::new_in_memory("keep this text").unwrap();
        let first = core.app().documents().active_id();
        core.reduce(AppMessage::OpenCommandPalette);
        core.reduce(AppMessage::PaletteQueryChanged {
            query: "New Tab".into(),
        });
        core.reduce(AppMessage::PaletteSubmit);
        assert_eq!(core.app().documents().len(), 2);
        assert_eq!(core.snapshot().to_string(), "");
        core.reduce(AppMessage::SwitchTab { id: first });
        assert_eq!(core.snapshot().to_string(), "keep this text");
    }
}
