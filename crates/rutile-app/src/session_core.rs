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

/// Sole `AppState` / `Document` authority for one open document.
pub struct DocumentSessionCore {
    app: AppState,
    document: Document,
    /// Parked ropes for inactive tabs. Path/dirty/revision stay on `DocumentSlot`.
    parked: HashMap<DocumentId, Document>,
    /// Generation of the latest open request. Completions with a mismatched
    /// generation are ignored so stale async opens cannot roll state backward.
    open_generation: u64,
}

impl DocumentSessionCore {
    /// Builds a core around an in-memory starter document and applies
    /// [`AppMessage::NewDocument`].
    pub fn new_in_memory(source: &str) -> Result<Self, String> {
        let document = Document::new(source).map_err(|error| error.to_string())?;
        let mut app = AppState::new();
        let _ = app.reduce(AppMessage::NewDocument);
        Ok(Self {
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
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
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
        }
    }

    /// Builds a core from an already-constructed document and app state
    /// (used when the platform has already driven the open reducer).
    #[must_use]
    pub fn from_parts(app: AppState, document: Document) -> Self {
        Self {
            app,
            document,
            parked: HashMap::new(),
            open_generation: 0,
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
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    #[must_use]
    pub fn dirty(&self) -> bool {
        self.app.dirty()
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.app.path()
    }

    #[must_use]
    pub fn snapshot(&self) -> DocumentSnapshot {
        self.document.snapshot()
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

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
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

    #[test]
    fn last_tab_close_is_a_noop() {
        let mut core = DocumentSessionCore::new_in_memory("only").unwrap();
        let id = core.app().documents().active_id();
        let effects = core.reduce(AppMessage::CloseTab { id });
        assert!(effects.is_empty());
        assert_eq!(core.document().snapshot().to_string(), "only");
        assert_eq!(core.app().documents().len(), 1);
    }
}
