//! Shared document-session core for native platform shells.
//!
//! Wave 2-A requires exactly one [`DocumentSessionCore`] authority per open
//! document. Platform `ProductSession` types compose this core and never become
//! independent document authorities: adapters perform effects, the core owns
//! [`AppState`] and [`Document`].

use std::path::{Path, PathBuf};

use rutile_core::{DiskVersion, Document, DocumentSnapshot};

use crate::app::{AppEffect, AppMessage, AppState, PreviewState};
use crate::app::{NoticeSeverity, UserNotice};
use rutile_types::Revision;

/// Sole `AppState` / `Document` authority for one open document.
pub struct DocumentSessionCore {
    app: AppState,
    document: Document,
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
            open_generation: 0,
        })
    }

    /// Builds a core from an already-loaded document bound to a path/disk pair.
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
            open_generation: 0,
        }
    }

    /// Builds a core from an already-constructed document and app state
    /// (used when the platform has already driven the open reducer).
    pub fn from_parts(app: AppState, document: Document) -> Self {
        Self {
            app,
            document,
            open_generation: 0,
        }
    }

    pub fn app(&self) -> &AppState {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut AppState {
        &mut self.app
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }

    pub fn set_document(&mut self, document: Document) {
        self.document = document;
    }

    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    pub fn dirty(&self) -> bool {
        self.app.dirty()
    }

    pub fn path(&self) -> Option<&Path> {
        self.app.path()
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        self.document.snapshot()
    }

    pub fn preview(&self) -> &PreviewState {
        self.app.preview()
    }

    pub fn notices(&self) -> &[UserNotice] {
        self.app.notices()
    }

    pub fn open_generation(&self) -> u64 {
        self.open_generation
    }

    /// Allocates the next open generation for a shared open request.
    pub fn begin_open(&mut self) -> u64 {
        self.open_generation = self.open_generation.saturating_add(1);
        self.open_generation
    }

    /// Returns true when `generation` matches the latest open request.
    pub fn is_current_open(&self, generation: u64) -> bool {
        generation == self.open_generation
    }

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        self.app.reduce(message)
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
    pub fn app_and_document_mut(&mut self) -> (&mut AppState, &mut Document) {
        (&mut self.app, &mut self.document)
    }

    /// Split-borrow helper: mutable app + immutable document.
    pub fn app_mut_and_document(&mut self) -> (&mut AppState, &Document) {
        (&mut self.app, &self.document)
    }

    /// Split-borrow helper: immutable app + immutable document.
    pub fn app_and_document(&self) -> (&AppState, &Document) {
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
        let _ = core.reduce(AppMessage::DocumentEdited { revision: 1 });
        // NewDocument path starts clean; edit without document apply keeps dirty false
        // until DocumentEdited with matching revision after real edit — still owns app.
        assert!(core.app().path().is_none());
        let path = PathBuf::from("/tmp/core-open.md");
        let generation = core.begin_open();
        let effects = core.reduce(AppMessage::OpenDocument { path: path.clone() });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, AppEffect::PerformOpen { .. }))
        );
        assert_eq!(core.open_generation(), generation);
    }
}
