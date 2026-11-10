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
    /// Editor/preview mirror recovery phase. Write only through
    /// [`DocumentSlot::begin_mirror_resync`] / [`DocumentSlot::finish_mirror_resync`].
    pub mirror: MirrorSync,
}

/// Editor ↔ preview mirror recovery lifecycle (W2-A contract: one full
/// authoritative resync after an incremental failure, never a retry loop).
///
/// ```text
///            begin_mirror_resync()
///   InSync ─────────────────────────▶ ResyncPending
///     ▲                                  │   begin_mirror_resync() → Err(AlreadyPending)
///     └──────────────────────────────────┘   (caller surfaces a durable notice)
///            finish_mirror_resync()
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MirrorSync {
    #[default]
    InSync,
    ResyncPending,
}

/// A refused [`MirrorSync`] transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MirrorSyncError {
    #[error("a full mirror resync is already outstanding")]
    AlreadyPending,
    #[error("no mirror resync is outstanding")]
    NotPending,
}

impl Default for DocumentSlot {
    fn default() -> Self {
        Self {
            revision: Revision::new(0),
            dirty: false,
            preview: PreviewState::Empty,
            path: None,
            saved_disk: None,
            external_conflict: None,
            find: None,
            autosave: None,
            next_transaction_id: 0,
            mirror: MirrorSync::InSync,
        }
    }
}

/// Edit/save lifecycle transitions.
///
/// `revision`, `dirty`, `path`, `saved_disk`, `external_conflict` and
/// `preview` together describe one document lifecycle. The reducer used to
/// write those six fields by hand in five places; every write now goes through
/// exactly one of these named moves:
///
/// ```text
///   any ──load_clean(rev, path?, disk?)──▶ Clean   (dirty=false, conflict cleared, preview Waiting)
///   any ──load_recovered(rev, path?)────▶ Dirty   (dirty=true, no disk baseline, preview Waiting)
///   any ──mark_edited(rev > current)────▶ Dirty   (stale/equal revision refused)
///   any ──record_save(path, disk, Durable)────────▶ Clean (conflict cleared)
///   any ──record_save(path, disk, DurabilityUnknown)▶ Dirty (re-save flushes the directory)
/// ```
impl DocumentSlot {
    /// Creates a slot for a freshly opened document at `revision` and `path`.
    #[must_use]
    pub fn opened(revision: Revision, path: PathBuf) -> Self {
        let mut slot = Self::default();
        slot.load_clean(revision, Some(path), None);
        slot
    }

    /// Replace the buffer with clean content (new, opened, or reopened).
    pub fn load_clean(
        &mut self,
        revision: Revision,
        path: Option<PathBuf>,
        disk: Option<DiskVersion>,
    ) {
        self.load(revision, false, path, disk);
    }

    /// Replace the buffer with recovered (unsaved) autosave content.
    pub fn load_recovered(&mut self, revision: Revision, path: Option<PathBuf>) {
        self.load(revision, true, path, None);
    }

    fn load(
        &mut self,
        revision: Revision,
        dirty: bool,
        path: Option<PathBuf>,
        disk: Option<DiskVersion>,
    ) {
        self.revision = revision;
        self.dirty = dirty;
        self.path = path;
        self.saved_disk = disk;
        self.external_conflict = None;
        self.preview = PreviewState::Waiting { revision };
    }

    /// Advance to a newer edited revision.
    ///
    /// # Errors
    ///
    /// [`StaleRevision`] when `revision` is not newer than the slot's revision;
    /// the slot is left untouched.
    pub fn mark_edited(&mut self, revision: Revision) -> Result<(), StaleRevision> {
        if revision <= self.revision {
            return Err(StaleRevision(revision));
        }
        self.revision = revision;
        self.dirty = true;
        self.preview = PreviewState::Waiting { revision };
        Ok(())
    }

    /// Record a committed save of the current revision to `path`/`disk`.
    pub fn record_save(&mut self, path: PathBuf, disk: DiskVersion, durability: SaveDurability) {
        self.dirty = match durability {
            SaveDurability::Durable => false,
            // The atomic rename committed but parent-directory durability
            // could not be verified: keep dirty so a re-save can flush it.
            SaveDurability::Unknown => true,
        };
        self.path = Some(path);
        self.saved_disk = Some(disk);
        self.external_conflict = None;
    }

    /// `InSync → ResyncPending`.
    ///
    /// # Errors
    ///
    /// [`MirrorSyncError::AlreadyPending`] when a resync is already outstanding.
    pub fn begin_mirror_resync(&mut self) -> Result<(), MirrorSyncError> {
        match self.mirror {
            MirrorSync::InSync => {
                self.mirror = MirrorSync::ResyncPending;
                Ok(())
            }
            MirrorSync::ResyncPending => Err(MirrorSyncError::AlreadyPending),
        }
    }

    /// `ResyncPending → InSync`.
    ///
    /// # Errors
    ///
    /// [`MirrorSyncError::NotPending`] when no resync was outstanding (state is
    /// unchanged).
    pub fn finish_mirror_resync(&mut self) -> Result<(), MirrorSyncError> {
        match self.mirror {
            MirrorSync::ResyncPending => {
                self.mirror = MirrorSync::InSync;
                Ok(())
            }
            MirrorSync::InSync => Err(MirrorSyncError::NotPending),
        }
    }
}

/// Whether a committed save is known to be durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDurability {
    Durable,
    Unknown,
}

/// A refused [`DocumentSlot::mark_edited`]: the revision is not newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("revision {0:?} is not newer than the slot's revision")]
pub struct StaleRevision(pub Revision);

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

    /// Clones the window autosave store onto every open slot.
    ///
    /// The store is a directory handle. Every tab must hold a clone so New Tab
    /// and closing the originally-bound slot cannot unbind crash recovery.
    pub fn bind_autosave_store(&mut self, store: &AutosaveStore) {
        for slot in self.slots.values_mut() {
            slot.autosave = Some(store.clone());
        }
    }

    fn inherit_autosave(&self) -> Option<AutosaveStore> {
        self.slots.values().find_map(|slot| slot.autosave.clone())
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
        let slot = DocumentSlot {
            autosave: self.inherit_autosave(),
            ..DocumentSlot::default()
        };
        self.slots.insert(id, slot);
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
        let mut slot = DocumentSlot::opened(revision, path);
        slot.autosave = self.inherit_autosave();
        self.slots.insert(id, slot);
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
            let fresh = DocumentSlot {
                autosave: removed.autosave.clone(),
                ..DocumentSlot::default()
            };
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
    use rutile_types::Revision;

    // --- DocumentSlot lifecycle machine ------------------------------------

    /// A distinct [`DiskVersion`] per `n` (digest from one real save; `len`
    /// and `modified` vary so versions compare unequal).
    fn disk(n: u8) -> DiskVersion {
        use rutile_core::FileService as _;
        static BASE: std::sync::OnceLock<DiskVersion> = std::sync::OnceLock::new();
        let base = BASE.get_or_init(|| {
            let path =
                std::env::temp_dir().join(format!("rutile-slot-machine-{}.md", std::process::id()));
            let document = rutile_core::Document::new("x").unwrap();
            let outcome =
                rutile_core::LocalFileService::new().save_atomic(&path, &document.snapshot());
            let _ = std::fs::remove_file(&path);
            match outcome {
                rutile_core::SaveOutcome::Committed { disk } => disk,
                other => panic!("expected committed save, got {other:?}"),
            }
        });
        DiskVersion {
            digest: base.digest,
            modified: std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(u64::from(n)),
            len: u64::from(n),
        }
    }

    /// The derived (dirty, has-baseline, conflict) table for each transition.
    #[test]
    fn slot_transition_state_table() {
        let mut slot = DocumentSlot {
            external_conflict: Some(disk(9)),
            ..DocumentSlot::default()
        };
        slot.load_clean(Revision::new(3), Some(PathBuf::from("a.md")), Some(disk(1)));
        assert_eq!(slot.revision, Revision::new(3));
        assert!(!slot.dirty);
        assert_eq!(slot.saved_disk, Some(disk(1)));
        assert_eq!(slot.external_conflict, None);
        assert_eq!(
            slot.preview,
            PreviewState::Waiting {
                revision: Revision::new(3)
            }
        );

        slot.external_conflict = Some(disk(9));
        slot.load_recovered(Revision::new(1), Some(PathBuf::from("b.md")));
        assert!(slot.dirty);
        assert_eq!(slot.path.as_deref(), Some(Path::new("b.md")));
        assert_eq!(
            slot.saved_disk, None,
            "recovered content has no disk baseline"
        );
        assert_eq!(slot.external_conflict, None);

        slot.mark_edited(Revision::new(2)).unwrap();
        assert!(slot.dirty);
        assert_eq!(
            slot.preview,
            PreviewState::Waiting {
                revision: Revision::new(2)
            }
        );

        slot.external_conflict = Some(disk(9));
        slot.record_save(PathBuf::from("c.md"), disk(2), SaveDurability::Durable);
        assert!(!slot.dirty);
        assert_eq!(slot.saved_disk, Some(disk(2)));
        assert_eq!(slot.external_conflict, None);
        assert_eq!(
            slot.revision,
            Revision::new(2),
            "a save never moves the revision"
        );

        slot.record_save(PathBuf::from("c.md"), disk(3), SaveDurability::Unknown);
        assert!(slot.dirty, "unverified durability keeps the document dirty");
        assert_eq!(slot.saved_disk, Some(disk(3)));
    }

    #[test]
    fn mark_edited_refuses_stale_and_equal_revisions() {
        let mut slot = DocumentSlot::default();
        slot.load_clean(Revision::new(5), None, None);
        let before = slot.clone();
        assert_eq!(
            slot.mark_edited(Revision::new(5)),
            Err(StaleRevision(Revision::new(5)))
        );
        assert_eq!(
            slot.mark_edited(Revision::new(4)),
            Err(StaleRevision(Revision::new(4)))
        );
        assert_eq!(slot.revision, before.revision);
        assert_eq!(slot.dirty, before.dirty);
        assert_eq!(slot.preview, before.preview);
    }

    /// Exhaustive over [`MirrorSync`] × transition.
    #[test]
    fn mirror_sync_transition_table() {
        for start in [MirrorSync::InSync, MirrorSync::ResyncPending] {
            let mut slot = DocumentSlot {
                mirror: start,
                ..DocumentSlot::default()
            };
            let begin = slot.begin_mirror_resync();
            match start {
                MirrorSync::InSync => {
                    assert_eq!(begin, Ok(()));
                    assert_eq!(slot.mirror, MirrorSync::ResyncPending);
                }
                MirrorSync::ResyncPending => {
                    assert_eq!(begin, Err(MirrorSyncError::AlreadyPending));
                    assert_eq!(slot.mirror, MirrorSync::ResyncPending);
                }
            }

            let mut slot = DocumentSlot {
                mirror: start,
                ..DocumentSlot::default()
            };
            let finish = slot.finish_mirror_resync();
            match start {
                MirrorSync::InSync => {
                    assert_eq!(finish, Err(MirrorSyncError::NotPending));
                    assert_eq!(slot.mirror, MirrorSync::InSync);
                }
                MirrorSync::ResyncPending => {
                    assert_eq!(finish, Ok(()));
                    assert_eq!(slot.mirror, MirrorSync::InSync);
                }
            }
        }
    }

    /// Stateful model test over random sequences of slot transitions.
    #[test]
    fn random_slot_transitions_preserve_invariants() {
        let mut seed: u64 = 0x0d0c_5107;
        for _run in 0..200 {
            let mut slot = DocumentSlot::default();
            let mut model_rev = 0u64;
            let mut model_dirty = false;
            let mut model_mirror = MirrorSync::InSync;
            for _step in 0..50 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let pick = seed >> 33;
                let n = (pick % 7) as u8;
                match pick % 7 {
                    0 => {
                        slot.load_clean(
                            Revision::new(pick % 4),
                            Some(PathBuf::from("x.md")),
                            Some(disk(n)),
                        );
                        model_rev = pick % 4;
                        model_dirty = false;
                    }
                    1 => {
                        slot.load_recovered(Revision::new(pick % 4), None);
                        model_rev = pick % 4;
                        model_dirty = true;
                    }
                    2 => {
                        let target = pick % 6;
                        let ok = slot.mark_edited(Revision::new(target)).is_ok();
                        assert_eq!(ok, target > model_rev);
                        if ok {
                            model_rev = target;
                            model_dirty = true;
                        }
                    }
                    3 => {
                        slot.record_save(PathBuf::from("x.md"), disk(n), SaveDurability::Durable);
                        model_dirty = false;
                    }
                    4 => {
                        slot.record_save(PathBuf::from("x.md"), disk(n), SaveDurability::Unknown);
                        model_dirty = true;
                    }
                    5 => {
                        let ok = slot.begin_mirror_resync().is_ok();
                        assert_eq!(ok, model_mirror == MirrorSync::InSync);
                        model_mirror = MirrorSync::ResyncPending;
                    }
                    _ => {
                        let ok = slot.finish_mirror_resync().is_ok();
                        assert_eq!(ok, model_mirror == MirrorSync::ResyncPending);
                        model_mirror = MirrorSync::InSync;
                    }
                }
                assert_eq!(slot.revision, Revision::new(model_rev));
                assert_eq!(slot.dirty, model_dirty);
                assert_eq!(slot.mirror, model_mirror);
                assert_eq!(
                    slot.external_conflict, None,
                    "every transition clears conflicts"
                );
                if slot.saved_disk.is_some() {
                    assert!(slot.path.is_some(), "a disk baseline always has a path");
                }
            }
        }
    }

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
        let id = mgr.open_document(Revision::new(1), path.clone()).unwrap();
        assert_ne!(id, DocumentId::ROOT);
        assert_eq!(mgr.active_id(), id);
        assert_eq!(mgr.slot(id).unwrap().path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn open_duplicate_path_switches_to_existing_tab() {
        let mut mgr = DocumentManager::new();
        let path = PathBuf::from("/tmp/dup.md");
        let id1 = mgr.open_document(Revision::new(1), path.clone()).unwrap();

        // Open a second tab
        mgr.new_tab().unwrap();

        // Reopen the same path → should switch, not create
        let id2 = mgr.open_document(Revision::new(2), path).unwrap();
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
    fn new_tab_and_open_inherit_autosave_store() {
        let mut mgr = DocumentManager::new();
        mgr.bind_autosave_store(&AutosaveStore::new("/tmp/rutile-autosave-inherit"));
        assert!(mgr.active_slot().autosave.is_some());
        let untitled = mgr.new_tab().unwrap();
        assert!(mgr.slot(untitled).unwrap().autosave.is_some());
        let named = mgr
            .open_document(Revision::new(1), PathBuf::from("/tmp/named.md"))
            .unwrap();
        assert!(mgr.slot(named).unwrap().autosave.is_some());
        mgr.switch_tab(DocumentId::ROOT).unwrap();
        assert!(mgr.active_slot().autosave.is_some());
    }

    #[test]
    fn bind_autosave_store_covers_already_open_tabs() {
        let mut mgr = DocumentManager::new();
        let second = mgr.new_tab().unwrap();
        mgr.bind_autosave_store(&AutosaveStore::new("/tmp/rutile-autosave-all"));
        assert!(mgr.slot(DocumentId::ROOT).unwrap().autosave.is_some());
        assert!(mgr.slot(second).unwrap().autosave.is_some());
    }

    #[test]
    fn closing_the_originally_bound_tab_keeps_autosave_on_neighbors() {
        let mut mgr = DocumentManager::new();
        mgr.bind_autosave_store(&AutosaveStore::new("/tmp/rutile-autosave-close"));
        let second = mgr.new_tab().unwrap();
        mgr.close_tab(DocumentId::ROOT).unwrap();
        assert!(mgr.slot(second).unwrap().autosave.is_some());
    }

    #[test]
    fn find_by_path_locates_open_document() {
        let mut mgr = DocumentManager::new();
        let path = PathBuf::from("/tmp/find.md");
        let id = mgr.open_document(Revision::new(1), path.clone()).unwrap();

        assert_eq!(mgr.find_by_path(&path), Some(id));
        assert_eq!(mgr.find_by_path(Path::new("/tmp/other.md")), None);
    }

    #[test]
    fn document_slot_default_is_empty_untitled() {
        let slot = DocumentSlot::default();
        assert_eq!(slot.revision, Revision::new(0));
        assert!(!slot.dirty);
        assert_eq!(slot.preview, PreviewState::Empty);
        assert!(slot.path.is_none());
    }

    #[test]
    fn document_slot_opened_has_path_and_revision() {
        let path = PathBuf::from("/tmp/opened.md");
        let slot = DocumentSlot::opened(Revision::new(5), path.clone());
        assert_eq!(slot.revision, Revision::new(5));
        assert!(!slot.dirty);
        assert_eq!(
            slot.preview,
            PreviewState::Waiting {
                revision: Revision::new(5)
            }
        );
        assert_eq!(slot.path.as_deref(), Some(path.as_path()));
    }
}
