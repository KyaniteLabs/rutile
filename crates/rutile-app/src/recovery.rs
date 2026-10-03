//! Shared recovery effects for native shells. The reducer remains I/O-free.

use std::path::PathBuf;
use std::thread::JoinHandle;

use rutile_core::{
    Document, FileService, LocalFileService, MAX_DOCUMENT_BYTES, RecoveredWorkspace, Revision,
    WorkspaceRecoverySnapshot,
};

use crate::document_manager::MAX_OPEN_DOCUMENTS;
use crate::session_core::{DocumentSessionCore, RestoredDocument};

/// Workspace recovery supersedes the legacy single-document journal.
pub enum PendingRecovery {
    Legacy(rutile_core::RecoveredDocument),
    Workspace(RecoveredWorkspace),
}

impl PendingRecovery {
    #[must_use]
    pub fn needs_confirmation(&self) -> bool {
        match self {
            Self::Legacy(_) => true,
            Self::Workspace(workspace) => workspace.has_unsaved_changes(),
        }
    }
}

pub fn read_pending(core: &DocumentSessionCore) -> Result<Option<PendingRecovery>, String> {
    let Some(store) = core.app().autosave_store() else {
        return Ok(None);
    };
    if let Some(workspace) = store
        .recover_workspace()
        .map_err(|error| error.to_string())?
    {
        return Ok(Some(PendingRecovery::Workspace(workspace)));
    }
    core.app()
        .recover()
        .map(|document| document.map(PendingRecovery::Legacy))
        .map_err(|error| error.to_string())
}

#[must_use]
pub fn without_drafts(workspace: &RecoveredWorkspace) -> RecoveredWorkspace {
    RecoveredWorkspace {
        active_id: workspace.active_id,
        tabs: workspace
            .tabs
            .iter()
            .map(|tab| rutile_core::RecoveredWorkspaceTab {
                id: tab.id,
                view: tab.view,
                path: tab.path.clone(),
                document: None,
            })
            .collect(),
    }
}

#[derive(Eq, PartialEq)]
struct TabFingerprint {
    id: u64,
    view: rutile_core::WorkspaceViewState,
    path: Option<String>,
    revision: Option<Revision>,
}

#[derive(Eq, PartialEq)]
struct Fingerprint {
    document_generation: u64,
    active_id: u64,
    tabs: Vec<TabFingerprint>,
}

impl Fingerprint {
    fn new(core: &DocumentSessionCore, snapshot: &WorkspaceRecoverySnapshot) -> Self {
        Self {
            document_generation: core.document_generation(),
            active_id: snapshot.active_id,
            tabs: snapshot
                .tabs
                .iter()
                .map(|tab| TabFingerprint {
                    id: tab.id,
                    view: tab.view,
                    path: tab.path.clone(),
                    revision: tab.snapshot.as_ref().map(|snapshot| snapshot.revision),
                })
                .collect(),
        }
    }
}

/// One writer at a time, with no writes for unchanged state. Background work owns
/// immutable rope snapshots. A clean exit cannot race a stale background write.
#[derive(Default)]
pub struct WorkspaceAutosave {
    worker: Option<JoinHandle<Result<(), String>>>,
    submitted: Option<Fingerprint>,
}

impl Drop for WorkspaceAutosave {
    fn drop(&mut self) {
        // Finish an already admitted write before the native store lease is
        // released, including error exits that bypass the clean-exit checkpoint.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl WorkspaceAutosave {
    pub fn poll(&mut self) -> Result<(), String> {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let result = self
                .worker
                .take()
                .expect("finished worker exists")
                .join()
                .map_err(|_| "Autosave worker failed".to_owned())
                .and_then(|result| result);
            if result.is_err() {
                self.submitted = None;
            }
            result?;
        }
        Ok(())
    }

    pub fn start(&mut self, core: &DocumentSessionCore, timestamp: u64) -> Result<(), String> {
        self.poll()?;
        if self.worker.is_some() {
            return Ok(());
        }
        let Some(store) = core.app().autosave_store().cloned() else {
            return Ok(());
        };
        let snapshot = core.recovery_snapshot()?;
        let fingerprint = Fingerprint::new(core, &snapshot);
        if self.submitted.as_ref() == Some(&fingerprint) {
            return Ok(());
        }
        self.worker = Some(
            std::thread::Builder::new()
                .name("rutile-autosave".into())
                .spawn(move || {
                    store
                        .record_workspace(&snapshot, timestamp)
                        .map_err(|error| error.to_string())
                })
                .map_err(|error| error.to_string())?,
        );
        self.submitted = Some(fingerprint);
        Ok(())
    }

    pub fn checkpoint(
        &mut self,
        core: &DocumentSessionCore,
        timestamp: u64,
        clean_exit: bool,
    ) -> Result<(), String> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Err("Autosave is finishing; please try again in a moment".into());
        }
        self.poll()?;
        let Some(store) = core.app().autosave_store() else {
            return Ok(());
        };
        let mut snapshot = core.recovery_snapshot()?;
        if clean_exit {
            for tab in &mut snapshot.tabs {
                tab.snapshot = None;
            }
        }
        store
            .record_workspace(&snapshot, timestamp)
            .map_err(|error| error.to_string())?;
        self.submitted = if clean_exit {
            None
        } else {
            Some(Fingerprint::new(core, &snapshot))
        };
        Ok(())
    }
}

fn copy_document(document: &Document) -> Result<Document, String> {
    Document::new(&document.snapshot().to_string()).map_err(|error| error.to_string())
}

fn empty_document() -> Result<RestoredDocument, String> {
    Ok(RestoredDocument {
        view: rutile_core::WorkspaceViewState::default(),
        document: Document::new("").map_err(|error| error.to_string())?,
        path: None,
        disk: None,
        dirty: false,
    })
}

/// Prepare all file reads before atomically installing the restored ropes.
/// Missing clean files become notices; dirty snapshots are never replaced by disk.
pub fn restore_workspace(
    core: &mut DocumentSessionCore,
    workspace: &RecoveredWorkspace,
    preserve_opened: bool,
) -> Result<Vec<String>, String> {
    if !core.dirty_document_ids().is_empty() {
        return Err("Save or close unsaved documents before restoring a session".into());
    }
    let mut documents = Vec::new();
    let mut active_index = 0;
    let mut notices = Vec::new();
    for tab in &workspace.tabs {
        let path = tab.path.as_ref().map(PathBuf::from);
        let loaded = path
            .as_ref()
            .map(|path| LocalFileService::new().load(path, MAX_DOCUMENT_BYTES));
        let prepared = if let Some(document) = &tab.document {
            RestoredDocument {
                view: tab.view,
                document: copy_document(document)?,
                path,
                disk: loaded.and_then(Result::ok).map(|loaded| loaded.disk),
                dirty: true,
            }
        } else if let Some(loaded) = loaded {
            match loaded {
                Ok(loaded) => RestoredDocument {
                    view: tab.view,
                    document: loaded.document,
                    path,
                    disk: Some(loaded.disk),
                    dirty: false,
                },
                Err(error) => {
                    notices.push(format!(
                        "Could not restore {}: {error}",
                        tab.path.as_deref().unwrap_or("document")
                    ));
                    continue;
                }
            }
        } else {
            empty_document()?
        };
        if tab.id == workspace.active_id {
            active_index = documents.len();
        }
        documents.push(prepared);
    }
    if preserve_opened && let Some(path) = core.path() {
        if let Some(index) = documents
            .iter()
            .position(|doc| doc.path.as_deref() == Some(path))
        {
            active_index = index;
        } else if documents.len() < MAX_OPEN_DOCUMENTS {
            active_index = documents.len();
            documents.push(RestoredDocument {
                view: core.app().documents().active_slot().view,
                document: copy_document(core.document())?,
                path: Some(path.to_path_buf()),
                disk: core.app().saved_disk().cloned(),
                dirty: core.dirty(),
            });
        } else {
            notices.push(format!("Restored all {MAX_OPEN_DOCUMENTS} recovery tabs. Open {} after closing a tab; its file is unchanged.", path.display()));
        }
    }
    if documents.is_empty() {
        documents.push(empty_document()?);
    }
    active_index = active_index.min(documents.len() - 1);
    core.install_restored_documents(documents, active_index)?;
    Ok(notices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_tabs_do_not_spawn_writers_and_clean_exit_cannot_be_overwritten() {
        let dir =
            std::env::temp_dir().join(format!("rutile-autosave-worker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = rutile_core::AutosaveStore::new(&dir);
        let mut core = DocumentSessionCore::new_in_memory("").unwrap();
        core.app_mut().bind_autosave(store.clone()).unwrap();
        let mut autosave = WorkspaceAutosave::default();
        autosave.checkpoint(&core, 1, false).unwrap();
        autosave.start(&core, 2).unwrap();
        assert!(autosave.worker.is_none());
        core.reduce(crate::app::AppMessage::NewTab);
        autosave.start(&core, 3).unwrap();
        autosave.worker.take().unwrap().join().unwrap().unwrap();
        autosave.start(&core, 4).unwrap();
        assert!(autosave.worker.is_none());
        assert_eq!(store.recover_workspace().unwrap().unwrap().tabs.len(), 2);
        autosave.checkpoint(&core, 5, true).unwrap();
        assert!(autosave.worker.is_none());
        assert!(
            !store
                .recover_workspace()
                .unwrap()
                .unwrap()
                .has_unsaved_changes()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
