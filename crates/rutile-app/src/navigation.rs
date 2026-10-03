//! Bounded navigation over open document snapshots; no filesystem traversal.
use crate::app::{AppEffect, AppMessage};
use crate::session_core::DocumentSessionCore;
use rutile_core::{DocumentSnapshot, Selection, SessionSelectionV1, WorkspaceViewState};
use rutile_types::DocumentId;
use std::collections::BTreeMap;

pub const MAX_NAVIGATION_ROWS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigatorKind {
    Outline,
    Search,
    Backlinks,
}

impl NavigatorKind {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Search => "Search Open Documents",
            Self::Backlinks => "Backlinks",
        }
    }
}

#[derive(Clone)]
pub struct NavigationEntry {
    pub label: String,
    pub document_id: DocumentId,
    pub selection: Selection,
    path: Option<std::path::PathBuf>,
    snapshot: DocumentSnapshot,
}

impl DocumentSessionCore {
    pub fn navigation_entries(
        &self,
        kind: NavigatorKind,
        query: &str,
    ) -> Result<Vec<NavigationEntry>, String> {
        if query.len() > rutile_core::MAX_FIND_PATTERN_BYTES {
            return Err("Search query is too long".into());
        }
        if kind == NavigatorKind::Search && query.is_empty() {
            return Ok(Vec::new());
        }
        let active = self.app().documents().active_id();
        if kind == NavigatorKind::Outline {
            let snapshot = self.snapshot();
            let outline = crate::outline::Outline::from_source(&snapshot.to_string())
                .map_err(|error| error.to_string())?;
            let query = query.to_lowercase();
            return Ok(outline
                .entries()
                .iter()
                .filter(|entry| entry.text.to_lowercase().contains(&query))
                .map(|entry| NavigationEntry {
                    label: format!(
                        "{}{}",
                        "  ".repeat(usize::from(entry.level.saturating_sub(1))),
                        entry.text.chars().take(200).collect::<String>()
                    ),
                    document_id: active,
                    path: self.path().map(std::path::Path::to_path_buf),
                    selection: Selection::collapsed(entry.source_offset),
                    snapshot: snapshot.clone(),
                })
                .collect());
        }
        let snapshots: BTreeMap<_, _> = self
            .app()
            .documents()
            .tab_order()
            .iter()
            .filter_map(|&id| {
                self.snapshot_for_document(id)
                    .map(|snapshot| (id, snapshot))
            })
            .collect();
        if kind == NavigatorKind::Backlinks {
            return Ok(self.backlink_entries(&snapshots, query));
        }
        let mut index = crate::local_search::SearchIndex::new();
        index.set_context(active, self.app().documents().tab_order().to_vec());
        for (&id, snapshot) in &snapshots {
            index.index_document(id, &snapshot.to_string(), snapshot.revision.get());
        }
        Ok(index
            .search(query)
            .into_iter()
            .take(MAX_NAVIGATION_ROWS)
            .filter_map(|result| {
                Some(NavigationEntry {
                    label: format!(
                        "{} — {}",
                        self.navigation_name(result.document_id),
                        result.snippet
                    ),
                    document_id: result.document_id,
                    path: self
                        .app()
                        .documents()
                        .slot(result.document_id)?
                        .path
                        .clone(),
                    selection: Selection {
                        anchor: result.byte_offset,
                        head: result.byte_offset + result.byte_length,
                    },
                    snapshot: snapshots.get(&result.document_id)?.clone(),
                })
            })
            .collect())
    }

    fn navigation_name(&self, id: DocumentId) -> String {
        self.app()
            .documents()
            .slot(id)
            .and_then(|slot| slot.path.as_ref())
            .and_then(|path| path.file_name())
            .map_or_else(
                || format!("Untitled {}", id.get()),
                |name| name.to_string_lossy().chars().take(80).collect(),
            )
    }

    fn backlink_entries(
        &self,
        snapshots: &BTreeMap<DocumentId, DocumentSnapshot>,
        query: &str,
    ) -> Vec<NavigationEntry> {
        let Some(target) = self
            .path()
            .and_then(|path| url::Url::from_file_path(path).ok())
        else {
            return Vec::new();
        };
        let query = query.to_lowercase();
        let mut results = Vec::new();
        for (&id, snapshot) in snapshots {
            let Some(base) = self
                .app()
                .documents()
                .slot(id)
                .and_then(|slot| slot.path.as_ref())
                .and_then(|path| url::Url::from_file_path(path).ok())
            else {
                continue;
            };
            for link in rutile_core::document_links(&snapshot.to_string()) {
                let matches = targets_document(&base, &link.target, &target);
                if matches && link.label.to_lowercase().contains(&query) {
                    results.push(NavigationEntry {
                        label: format!("{} — {}", self.navigation_name(id), link.label),
                        document_id: id,
                        path: self
                            .app()
                            .documents()
                            .slot(id)
                            .and_then(|slot| slot.path.clone()),
                        selection: Selection::collapsed(link.byte_offset),
                        snapshot: snapshot.clone(),
                    });
                    if results.len() == MAX_NAVIGATION_ROWS {
                        return results;
                    }
                }
            }
        }
        results
    }

    pub fn activate_navigation(
        &mut self,
        entry: &NavigationEntry,
    ) -> Result<Vec<AppEffect>, String> {
        let current = self
            .snapshot_for_document(entry.document_id)
            .ok_or("The document was closed. Refresh the results.")?;
        if current.revision != entry.snapshot.revision
            || !current.same_content(&entry.snapshot)
            || self
                .app()
                .documents()
                .slot(entry.document_id)
                .and_then(|slot| slot.path.as_ref())
                != entry.path.as_ref()
        {
            return Err("The document changed. Refresh the results.".into());
        }
        if !current.is_char_boundary(entry.selection.anchor)
            || !current.is_char_boundary(entry.selection.head)
        {
            return Err("Navigation range is invalid".into());
        }
        self.reduce(AppMessage::SwitchTab {
            id: entry.document_id,
        });
        let slot = self.app().documents().active_slot();
        self.reduce(AppMessage::DocumentViewChanged {
            id: entry.document_id,
            path: slot.path.clone(),
            find_bar: slot.find_bar.clone(),
            view: WorkspaceViewState {
                selection: Some(SessionSelectionV1 {
                    anchor: entry.selection.anchor as u64,
                    head: entry.selection.head as u64,
                }),
                top_visible_byte: Some(entry.selection.anchor as u64),
            },
        });
        Ok(vec![AppEffect::ScheduleRender {
            revision: current.revision,
        }])
    }
}

fn targets_document(base: &url::Url, destination: &str, target: &url::Url) -> bool {
    if base
        .join(destination)
        .is_ok_and(|candidate| candidate == *target)
    {
        return true;
    }
    if std::path::Path::new(destination)
        .extension()
        .and_then(|suffix| suffix.to_str())
        .is_some_and(|suffix| {
            suffix.eq_ignore_ascii_case("md") || suffix.eq_ignore_ascii_case("markdown")
        })
    {
        return false;
    }
    [
        format!("{destination}.md"),
        format!("{destination}.markdown"),
    ]
    .iter()
    .any(|path| base.join(path).is_ok_and(|candidate| candidate == *target))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backlinks_resolve_relative_encoded_and_wiki_paths_without_reading_unopened_files() {
        use rutile_core::FileService;
        let dir = std::env::temp_dir().join(format!(
            "rutile-nav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::create_dir(dir.join("notes")).unwrap();
        let source = dir.join("notes/source.md");
        let target = dir.join("target withspace.md");
        std::fs::write(
            &source,
            "[go](../target%20withspace.md#part)\n\n[[../target withspace]]",
        )
        .unwrap();
        std::fs::write(&target, "# Target").unwrap();
        let loaded = rutile_core::LocalFileService::new()
            .load(&source, rutile_core::MAX_DOCUMENT_BYTES)
            .unwrap();
        let mut core = DocumentSessionCore::from_opened(loaded.document, source, loaded.disk);
        let loaded = rutile_core::LocalFileService::new()
            .load(&target, rutile_core::MAX_DOCUMENT_BYTES)
            .unwrap();
        core.adopt_opened_document(loaded.document, target, loaded.disk);
        let rows = core
            .navigation_entries(NavigatorKind::Backlinks, "")
            .unwrap();
        assert_eq!(rows.len(), 2);
        core.activate_navigation(&rows[0]).unwrap();
        assert!(core.snapshot().to_string().starts_with("[go]"));
        let base = url::Url::parse("file:///notes/source.md").unwrap();
        assert!(!targets_document(
            &base,
            "wrong.md",
            &url::Url::parse("file:///notes/wrong.md.md").unwrap()
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn outline_and_search_navigation_reject_stale_same_revision_replacements() {
        let mut core =
            DocumentSessionCore::new_in_memory("# Café\n\nneedle\n\nAgain\n=====\n").unwrap();
        let rows = core.navigation_entries(NavigatorKind::Outline, "").unwrap();
        assert_eq!(rows.len(), 2);
        core.activate_navigation(&rows[1]).unwrap();
        assert_eq!(
            core.app()
                .documents()
                .active_slot()
                .view
                .selection
                .unwrap()
                .anchor,
            17
        );
        core.reduce(AppMessage::NewTab);
        let matches = core
            .navigation_entries(NavigatorKind::Search, "needle")
            .unwrap();
        assert_eq!(matches.len(), 1);
        core.activate_navigation(&matches[0]).unwrap();
        assert_eq!(
            core.snapshot().to_string(),
            "# Café\n\nneedle\n\nAgain\n=====\n"
        );
        core.set_document(rutile_core::Document::new("different zero revision").unwrap());
        assert!(core.activate_navigation(&rows[0]).is_err());
    }
}
