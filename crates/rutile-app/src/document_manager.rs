//! Multi-document tab management contract (roadmap 08).
//!
//! Defines [`DocumentSlot`] (per-document state) and [`DocumentManager`]
//! (the collection + active-tab + ordering). This module establishes the
//! locked contract types that the full `AppState` migration will consume.
//!
//! # Design
//!
//! See `docs/plan/multi-document-design.md` for the resolved grilling
//! questions. Key decisions: `DocumentId` identity, `BTreeMap` + ordered
//! `Vec` for tab order, duplicate-open detection, `MAX_OPEN_DOCUMENTS`
//! resource bound, per-tab dirty/conflict state.
//!
//! # Security-core fence
//!
//! No field in this module constructs raw HTML, URLs, or paths that bypass
//! [`validate_path`](rutile_core::validate_path). Path operations use
//! the existing `PathBuf` types without interpretation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rutile_core::{AutosaveStore, DiskVersion};
use rutile_types::{DocumentId, Revision};

use crate::actions::FindSession;
use crate::app::PreviewState;

/// Maximum number of simultaneously open documents (resource bound, D6).
pub const MAX_OPEN_DOCUMENTS: usize = 16;

/// Per-document state — the fields that were directly on `AppState` in the
/// single-document baseline, now extracted so each tab owns its own
/// revision, dirty flag, preview coordination, path, and autosave.
#[derive(Debug, Clone)]
pub struct DocumentSlot {
    pub revision: Revision,
    pub dirty: bool,
    pub preview: PreviewState,
    pub path: Option<PathBuf>,
    pub saved_disk: Option<DiskVersion>,
    pub external_conflict: Option<DiskVersion>,
    pub find: Option<FindSession>,
    pub autosave: Option<AutosaveStore>,
    pub next_transaction_id: u64,
    pub mirror_resync_pending: bool,
}

impl Default for DocumentSlot {
    fn default() -> Self {
        Self {
            revision: 0,
            dirty: false,
            preview: PreviewState::Empty,
            path: None,
            saved_disk: None,
            external_conflict: None,
            find: None,
            autosave: None,
            next_transaction_id: 0,
            mirror_resync_pending: false,
        }
    }
}

impl DocumentSlot {
    /// Creates a slot for a freshly opened document at `revision` and `path`.
    #[must_use]
    pub fn opened(revision: Revision, path: PathBuf) -> Self {
        Self {
            revision,
            dirty: false,
            preview: PreviewState::Waiting { revision },
            path: Some(path),
            ..Self::default()
        }
    }
}

/// The result of closing a tab.
#[derive(Debug)]
pub struct CloseTabResult {
    /// The new active tab after closing (None if all tabs are closed).
    pub new_active: Option<DocumentId>,
    /// The slot that was removed (for cleanup by the platform shell).
    pub removed: DocumentSlot,
}

/// Error returned by tab operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TabError {
    #[error("unknown document id")]
    UnknownDocument,
    #[error("no tabs open")]
    NoTabsOpen,
    #[error("too many open documents (max {max})")]
    TooManyOpen { max: usize },
}

/// Multi-document tab manager (roadmap 08).
///
/// Holds a collection of [`DocumentSlot`]s keyed by [`DocumentId`], an
/// ordered tab strip, and the currently active tab. The full `AppState`
/// migration will replace `AppState`'s per-document fields with a
/// `DocumentManager`.
pub struct DocumentManager {
    slots: BTreeMap<DocumentId, DocumentSlot>,
    tab_order: Vec<DocumentId>,
    active_id: DocumentId,
    next_id: u64,
}

impl Default for DocumentManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentManager {
    /// Creates a manager with a single ROOT tab (migration entry point).
    #[must_use]
    pub fn new() -> Self {
        let mut slots = BTreeMap::new();
        slots.insert(DocumentId::ROOT, DocumentSlot::default());
        Self {
            slots,
            tab_order: vec![DocumentId::ROOT],
            active_id: DocumentId::ROOT,
            next_id: 1,
        }
    }

    /// The active document's id.
    #[must_use]
    pub const fn active_id(&self) -> DocumentId {
        self.active_id
    }

    /// Borrows the active document's slot.
    #[must_use]
    pub fn active_slot(&self) -> &DocumentSlot {
        &self.slots[&self.active_id]
    }

    /// Mutably borrows the active document's slot.
    pub fn active_slot_mut(&mut self) -> &mut DocumentSlot {
        self.slots
            .get_mut(&self.active_id)
            .expect("active_id always exists")
    }

    /// Borrows a specific document's slot.
    #[must_use]
    pub fn slot(&self, id: DocumentId) -> Option<&DocumentSlot> {
        self.slots.get(&id)
    }

    /// Mutably borrows a specific document's slot.
    pub fn slot_mut(&mut self, id: DocumentId) -> Option<&mut DocumentSlot> {
        self.slots.get_mut(&id)
    }

    /// Returns the ordered tab ids (left-to-right tab strip order).
    #[must_use]
    pub fn tab_order(&self) -> &[DocumentId] {
        &self.tab_order
    }

    /// Number of open tabs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether there are no tabs (always false after `new()`, possible after
    /// closing all tabs).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Creates a new untitled tab and makes it active. Returns its id.
    /// Fails closed if at the resource cap.
    pub fn new_tab(&mut self) -> Result<DocumentId, TabError> {
        if self.slots.len() >= MAX_OPEN_DOCUMENTS {
            return Err(TabError::TooManyOpen {
                max: MAX_OPEN_DOCUMENTS,
            });
        }
        let id = DocumentId::new(self.next_id);
        self.next_id += 1;
        self.slots.insert(id, DocumentSlot::default());
        self.tab_order.push(id);
        self.active_id = id;
        Ok(id)
    }

    /// Opens a document in a new tab (or switches to an existing tab if the
    /// path is already open — D4 duplicate detection). Returns the tab id.
    pub fn open_document(
        &mut self,
        revision: Revision,
        path: PathBuf,
    ) -> Result<DocumentId, TabError> {
        // Duplicate detection: switch to existing tab if the path matches.
        if let Some(existing_id) = self.find_by_path(&path) {
            self.active_id = existing_id;
            return Ok(existing_id);
        }
        if self.slots.len() >= MAX_OPEN_DOCUMENTS {
            return Err(TabError::TooManyOpen {
                max: MAX_OPEN_DOCUMENTS,
            });
        }
        let id = DocumentId::new(self.next_id);
        self.next_id += 1;
        self.slots.insert(id, DocumentSlot::opened(revision, path));
        self.tab_order.push(id);
        self.active_id = id;
        Ok(id)
    }

    /// Switches the active tab. Fails if `id` is not open.
    pub fn switch_tab(&mut self, id: DocumentId) -> Result<(), TabError> {
        if !self.slots.contains_key(&id) {
            return Err(TabError::UnknownDocument);
        }
        self.active_id = id;
        Ok(())
    }

    /// Closes a tab and returns the removed slot + new active id.
    /// If the closed tab was active, the next neighbor becomes active.
    /// If all tabs are closed, `new_active` is `None` and the manager
    /// re-seeds with a fresh ROOT tab.
    pub fn close_tab(&mut self, id: DocumentId) -> Result<CloseTabResult, TabError> {
        let removed = self.slots.remove(&id).ok_or(TabError::UnknownDocument)?;

        // Capture the closed tab's position BEFORE removing it from tab_order
        // so we can select the neighbor that shifts into its slot.
        let orig_pos = self.tab_order.iter().position(|&t| t == id);
        self.tab_order.retain(|&t| t != id);

        if self.slots.is_empty() {
            // Re-seed with a fresh ROOT tab (the app always has at least one).
            let fresh = DocumentSlot::default();
            self.slots.insert(DocumentId::ROOT, fresh);
            self.tab_order.push(DocumentId::ROOT);
            self.active_id = DocumentId::ROOT;
            return Ok(CloseTabResult {
                new_active: Some(DocumentId::ROOT),
                removed,
            });
        }

        if self.active_id == id {
            // Pick the neighbor at the closed tab's original position.
            // After removal, the tab at `orig_pos` is the one that shifted
            // left into the closed tab's slot (the right neighbor). Clamp
            // to the last valid index if the closed tab was at the tail.
            let pos = orig_pos.unwrap_or(0);
            let new_pos = pos.min(self.tab_order.len().saturating_sub(1));
            self.active_id = self.tab_order[new_pos];
        }

        Ok(CloseTabResult {
            new_active: Some(self.active_id),
            removed,
        })
    }

    /// Reorders a tab from `from` to `to` in the tab strip.
    pub fn reorder_tab(&mut self, from: usize, to: usize) -> Result<(), TabError> {
        if from >= self.tab_order.len() || to >= self.tab_order.len() {
            return Err(TabError::UnknownDocument);
        }
        let id = self.tab_order.remove(from);
        self.tab_order.insert(to, id);
        Ok(())
    }

    /// Finds an open tab by canonical path (for duplicate detection).
    #[must_use]
    pub fn find_by_path(&self, path: &Path) -> Option<DocumentId> {
        self.slots
            .iter()
            .find(|(_, slot)| slot.path.as_deref() == Some(path))
            .map(|(&id, _)| id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manager_has_single_root_tab() {
        let mgr = DocumentManager::new();
        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.active_id(), DocumentId::ROOT);
        assert_eq!(mgr.tab_order(), &[DocumentId::ROOT]);
    }

    #[test]
    fn new_tab_creates_and_activates() {
        let mut mgr = DocumentManager::new();
        let id = mgr.new_tab().unwrap();
        assert_ne!(id, DocumentId::ROOT);
        assert_eq!(mgr.active_id(), id);
        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.tab_order().len(), 2);
    }

    #[test]
    fn open_document_creates_new_tab() {
        let mut mgr = DocumentManager::new();
        let path = PathBuf::from("/tmp/test.md");
        let id = mgr.open_document(1, path.clone()).unwrap();
        assert_ne!(id, DocumentId::ROOT);
        assert_eq!(mgr.active_id(), id);
        assert_eq!(mgr.slot(id).unwrap().path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn open_duplicate_path_switches_to_existing_tab() {
        let mut mgr = DocumentManager::new();
        let path = PathBuf::from("/tmp/dup.md");
        let id1 = mgr.open_document(1, path.clone()).unwrap();

        // Open a second tab
        mgr.new_tab().unwrap();

        // Reopen the same path → should switch, not create
        let id2 = mgr.open_document(2, path).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(mgr.len(), 3); // ROOT + opened + new_tab, no dup created
        assert_eq!(mgr.active_id(), id1);
    }

    #[test]
    fn switch_tab_changes_active() {
        let mut mgr = DocumentManager::new();
        let id = mgr.new_tab().unwrap();
        mgr.switch_tab(DocumentId::ROOT).unwrap();
        assert_eq!(mgr.active_id(), DocumentId::ROOT);
        mgr.switch_tab(id).unwrap();
        assert_eq!(mgr.active_id(), id);
    }

    #[test]
    fn switch_to_unknown_fails() {
        let mut mgr = DocumentManager::new();
        assert!(mgr.switch_tab(DocumentId::new(999)).is_err());
    }

    #[test]
    fn close_tab_removes_and_picks_neighbor() {
        let mut mgr = DocumentManager::new();
        let id1 = mgr.new_tab().unwrap();
        let _id2 = mgr.new_tab().unwrap();
        assert_eq!(mgr.len(), 3);

        let result = mgr.close_tab(id1).unwrap();
        assert_eq!(mgr.len(), 2);
        assert!(mgr.slot(id1).is_none());
        assert!(result.new_active.is_some());
    }

    #[test]
    fn close_active_picks_neighbor() {
        let mut mgr = DocumentManager::new();
        let _id1 = mgr.new_tab().unwrap();
        let id2 = mgr.new_tab().unwrap();
        assert_eq!(mgr.active_id(), id2);

        let result = mgr.close_tab(id2).unwrap();
        assert!(result.new_active.is_some());
        assert_ne!(mgr.active_id(), id2);
    }

    #[test]
    fn close_last_tab_reseeds_root() {
        let mut mgr = DocumentManager::new();
        let result = mgr.close_tab(DocumentId::ROOT).unwrap();
        assert_eq!(result.new_active, Some(DocumentId::ROOT));
        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.active_id(), DocumentId::ROOT);
    }

    #[test]
    fn close_unknown_fails() {
        let mut mgr = DocumentManager::new();
        assert!(mgr.close_tab(DocumentId::new(999)).is_err());
    }

    #[test]
    fn reorder_tab_swaps_positions() {
        let mut mgr = DocumentManager::new();
        let id1 = mgr.new_tab().unwrap();
        let id2 = mgr.new_tab().unwrap();
        // tab_order: [ROOT, id1, id2]

        mgr.reorder_tab(2, 0).unwrap();
        assert_eq!(mgr.tab_order(), &[id2, DocumentId::ROOT, id1]);
    }

    #[test]
    fn max_open_documents_enforced() {
        let mut mgr = DocumentManager::new();
        // ROOT counts as 1; open MAX-1 more.
        for _ in 0..(MAX_OPEN_DOCUMENTS - 1) {
            mgr.new_tab().unwrap();
        }
        assert_eq!(mgr.len(), MAX_OPEN_DOCUMENTS);
        assert!(mgr.new_tab().is_err());
    }

    #[test]
    fn find_by_path_locates_open_document() {
        let mut mgr = DocumentManager::new();
        let path = PathBuf::from("/tmp/find.md");
        let id = mgr.open_document(1, path.clone()).unwrap();

        assert_eq!(mgr.find_by_path(&path), Some(id));
        assert_eq!(mgr.find_by_path(Path::new("/tmp/other.md")), None);
    }

    #[test]
    fn document_slot_default_is_empty_untitled() {
        let slot = DocumentSlot::default();
        assert_eq!(slot.revision, 0);
        assert!(!slot.dirty);
        assert_eq!(slot.preview, PreviewState::Empty);
        assert!(slot.path.is_none());
    }

    #[test]
    fn document_slot_opened_has_path_and_revision() {
        let path = PathBuf::from("/tmp/opened.md");
        let slot = DocumentSlot::opened(5, path.clone());
        assert_eq!(slot.revision, 5);
        assert!(!slot.dirty);
        assert_eq!(slot.preview, PreviewState::Waiting { revision: 5 });
        assert_eq!(slot.path.as_deref(), Some(path.as_path()));
    }
}
