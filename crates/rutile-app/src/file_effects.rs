//! Filesystem-dependent identity checks shared by native effect executors.

use std::path::Path;

use rutile_core::FileService;
use rutile_types::DocumentId;

use crate::session_core::DocumentSessionCore;

/// The effect boundary supplies wall-clock time; checkpoint storage stays I/O-free.
pub fn record_checkpoint(
    core: &mut DocumentSessionCore,
    source: crate::revision_history::HistorySource,
    description: &str,
) -> Result<u64, String> {
    core.checkpoint(checkpoint_timestamp()?, source, description)
}

pub fn checkpoint_timestamp() -> Result<u64, String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    u64::try_from(millis).map_err(|error| error.to_string())
}

/// Explicit reload replaces the active buffer; ordinary duplicate Open never does.
pub fn reload_current(
    core: &mut DocumentSessionCore,
) -> Result<Vec<crate::app::AppEffect>, String> {
    let path = core
        .path()
        .ok_or("Reload requires a document path")?
        .to_path_buf();
    let loaded = rutile_core::LocalFileService::new()
        .load(&path, rutile_core::MAX_DOCUMENT_BYTES)
        .map_err(|error| error.to_string())?;
    core.set_document(loaded.document);
    Ok(core.reduce(crate::app::AppMessage::DocumentOpened {
        revision: core.revision(),
        path,
        disk: loaded.disk,
    }))
}

/// Export commits through the document writer without changing document state.
pub fn save_html(core: &DocumentSessionCore, path: &Path, html: String) -> Result<(), String> {
    let path =
        rutile_core::LocalFileService::absolute_target(path).map_err(|error| error.to_string())?;
    if open_document_at(core, &path).is_some() {
        return Err("Choose an export file that is not open in Rutile".into());
    }
    let page = rutile_core::ExportPage::from_html(html).map_err(|error| error.to_string())?;
    match rutile_core::LocalFileService::new().save_export_atomic(&path, &page) {
        rutile_core::SaveOutcome::Committed { .. } => Ok(()),
        rutile_core::SaveOutcome::CommittedDurabilityUnknown { reason, .. } => Err(format!(
            "Export was written but durability could not be confirmed: {reason}"
        )),
        rutile_core::SaveOutcome::NotCommitted { reason } => Err(reason.to_string()),
    }
}

/// Resolve already-open aliases without loading or replacing any document.
pub fn open_document_at(core: &DocumentSessionCore, path: &Path) -> Option<DocumentId> {
    let documents = core.app().documents();
    documents.find_by_path(path).or_else(|| {
        let canonical = std::fs::canonicalize(path).ok()?;
        documents.tab_order().iter().copied().find(|id| {
            documents
                .slot(*id)
                .and_then(|slot| slot.path.as_ref())
                .and_then(|path| std::fs::canonicalize(path).ok())
                .as_ref()
                == Some(&canonical)
        })
    })
}
