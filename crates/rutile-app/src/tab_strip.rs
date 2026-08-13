//! Headless tab-strip projection (roadmap 08 / ralplan C2).
//!
//! The shell renders these rows; ranking, parking, and close decisions stay
//! in [`DocumentManager`] / [`DocumentSessionCore`].

use crate::document_manager::DocumentManager;
use rutile_types::DocumentId;

/// Click from the Iced strip. The runner reduces these through the existing
/// tab messages; dirty close still goes through D7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStripCommand {
    Switch(DocumentId),
    Close(DocumentId),
}

/// One projected tab for the Iced strip and the Window ▸ Tabs menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabStripRow {
    pub id: DocumentId,
    pub label: String,
    pub dirty: bool,
    pub active: bool,
    pub close_enabled: bool,
}

impl TabStripRow {
    /// Menu / button caption. Dirty tabs get a trailing bullet.
    #[must_use]
    pub fn display_label(&self) -> String {
        if self.dirty {
            format!("{} •", self.label)
        } else {
            self.label.clone()
        }
    }
}

/// Projects the current manager into left-to-right strip rows.
#[must_use]
pub fn project_tabs(docs: &DocumentManager) -> Vec<TabStripRow> {
    let active = docs.active_id();
    let close_enabled = docs.len() > 1;
    docs.tab_order()
        .iter()
        .map(|&id| {
            let slot = docs.slot(id);
            let label = slot
                .and_then(|slot| slot.path.as_ref())
                .and_then(|path| path.file_name())
                .map_or_else(
                    || "Untitled".to_owned(),
                    |name| name.to_string_lossy().into_owned(),
                );
            TabStripRow {
                id,
                label,
                dirty: slot.is_some_and(|slot| slot.dirty),
                active: id == active,
                close_enabled,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_manager::DocumentManager;
    use rutile_types::Revision;
    use std::path::PathBuf;

    #[test]
    fn projects_untitled_and_named_with_dirty_and_active() {
        let mut docs = DocumentManager::new();
        docs.active_slot_mut().dirty = true;
        let named = docs
            .open_document(Revision::new(1), PathBuf::from("/tmp/notes.md"))
            .unwrap();
        let rows = project_tabs(&docs);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Untitled");
        assert!(rows[0].dirty);
        assert!(!rows[0].active);
        assert!(rows[0].close_enabled);
        assert_eq!(rows[1].id, named);
        assert_eq!(rows[1].label, "notes.md");
        assert!(rows[1].active);
        assert!(!rows[1].dirty);
        assert_eq!(rows[1].display_label(), "notes.md");
        assert_eq!(rows[0].display_label(), "Untitled •");
    }

    #[test]
    fn last_tab_cannot_close() {
        let docs = DocumentManager::new();
        let rows = project_tabs(&docs);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].close_enabled);
        assert!(rows[0].active);
    }

    #[test]
    fn active_index_tracks_switch() {
        let mut docs = DocumentManager::new();
        let second = docs.new_tab().unwrap();
        docs.switch_tab(second).unwrap();
        let rows = project_tabs(&docs);
        assert!(!rows[0].active);
        assert!(rows[1].active);
    }
}
