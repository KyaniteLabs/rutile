//! Atomic recovery of the complete tab set. Snapshots commit before the manifest;
//! validated old snapshots are collected only after the new manifest is durable.
//! This namespace is separate from legacy single-document autosave journals.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};

use serde::{Deserialize, Serialize};

use crate::autosave::{open_recovery_file, read_snapshot_file, with_store_lock};
use crate::files::{write_bytes_atomic, write_snapshot_atomic};
use crate::session_contract::validate_path;
use crate::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, AutosaveError, AutosaveStore, Document, DocumentSnapshot,
    MAX_DOCUMENT_BYTES, encode_autosave_entry,
};

pub const WORKSPACE_RECOVERY_FILE: &str = "workspace-recovery.json";
pub const MAX_RECOVERY_TABS: usize = 16;
const MAX_MANIFEST_BYTES: usize = 128 * 1024;
const SCHEMA: &str = "rutile.workspace-recovery.v1";

/// Advisory editor positions. Invalid or stale offsets are ignored by the
/// shell after loading the document; they never invalidate a recoverable draft.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceViewState {
    pub selection: Option<crate::SessionSelectionV1>,
    pub top_visible_byte: Option<u64>,
}

impl WorkspaceViewState {
    pub fn validated(self, snapshot: &DocumentSnapshot) -> Self {
        let valid = |offset: u64| {
            usize::try_from(offset).is_ok_and(|offset| snapshot.is_char_boundary(offset))
        };
        Self {
            selection: self
                .selection
                .filter(|selection| valid(selection.anchor) && valid(selection.head)),
            top_visible_byte: self.top_visible_byte.filter(|offset| valid(*offset)),
        }
    }
}

/// A native session holds this lease until shutdown. Other instances may edit
/// files but must not replace this session's recovery checkpoint.
pub struct WorkspaceRecoveryLease {
    _file: fs::File,
}

#[derive(Clone)]
pub struct WorkspaceRecoveryTab {
    pub id: u64,
    pub view: WorkspaceViewState,
    pub path: Option<String>,
    /// Only dirty documents need a snapshot. Clean named tabs reload from disk.
    pub snapshot: Option<DocumentSnapshot>,
}

#[derive(Clone)]
pub struct WorkspaceRecoverySnapshot {
    pub active_id: u64,
    pub tabs: Vec<WorkspaceRecoveryTab>,
}

pub struct RecoveredWorkspaceTab {
    pub id: u64,
    pub view: WorkspaceViewState,
    pub path: Option<String>,
    pub document: Option<Document>,
}

pub struct RecoveredWorkspace {
    pub active_id: u64,
    pub tabs: Vec<RecoveredWorkspaceTab>,
}

impl RecoveredWorkspace {
    pub fn has_unsaved_changes(&self) -> bool {
        self.tabs.iter().any(|tab| tab.document.is_some())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    generation: u64,
    active_id: u64,
    tabs: Vec<Tab>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tab {
    id: u64,
    #[serde(default)]
    view: WorkspaceViewState,
    path: Option<String>,
    snapshot: Option<AutosaveEntryV1>,
}

impl Manifest {
    fn validate(&self) -> Result<(), AutosaveError> {
        if self.schema != SCHEMA || self.tabs.is_empty() || self.tabs.len() > MAX_RECOVERY_TABS {
            return Err(AutosaveError::InvalidJournal);
        }
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for tab in &self.tabs {
            if !ids.insert(tab.id) {
                return Err(AutosaveError::InvalidJournal);
            }
            if let Some(path) = &tab.path {
                validate_path(path)?;
                if !paths.insert(path) {
                    return Err(AutosaveError::InvalidJournal);
                }
            }
            if let Some(entry) = &tab.snapshot {
                encode_autosave_entry(entry)?;
                if entry.document_path != tab.path
                    || entry.sequence > self.generation
                    || entry.snapshot_file != snapshot_name(entry.sequence, tab.id)
                {
                    return Err(AutosaveError::InvalidJournal);
                }
            }
        }
        if !ids.contains(&self.active_id) {
            return Err(AutosaveError::InvalidJournal);
        }
        Ok(())
    }
}

fn snapshot_name(generation: u64, id: u64) -> String {
    format!("workspace-{generation}-{id}.md")
}

impl AutosaveStore {
    pub fn acquire_workspace_lease(&self) -> Result<WorkspaceRecoveryLease, AutosaveError> {
        use fs2::FileExt;
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(self.dir().join(".workspace-session.lock"))?;
        if !file.metadata()?.is_file() {
            return Err(AutosaveError::InvalidJournal);
        }
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                AutosaveError::AutosaveBusy
            } else {
                AutosaveError::Io(error)
            }
        })?;
        Ok(WorkspaceRecoveryLease { _file: file })
    }

    /// Commit every tab together, including clean/closed state. A clean checkpoint
    /// supersedes prior dirty snapshots, preventing discarded drafts from returning.
    pub fn record_workspace(
        &self,
        workspace: &WorkspaceRecoverySnapshot,
        captured_at_unix_ms: u64,
    ) -> Result<(), AutosaveError> {
        with_store_lock(self.dir(), || {
            let previous = self.read_workspace_manifest()?;
            let generation = previous.as_ref().map_or(Ok(0), |manifest| {
                manifest
                    .generation
                    .checked_add(1)
                    .ok_or(AutosaveError::SequenceExhausted)
            })?;
            let mut manifest = Manifest {
                schema: SCHEMA.to_owned(),
                generation,
                active_id: workspace.active_id,
                tabs: workspace
                    .tabs
                    .iter()
                    .map(|tab| Tab {
                        id: tab.id,
                        view: tab.view,
                        path: tab.path.clone(),
                        snapshot: None,
                    })
                    .collect(),
            };
            // Validate identities and paths before any snapshot write.
            manifest.validate()?;
            for (tab, source) in manifest.tabs.iter_mut().zip(&workspace.tabs) {
                let Some(snapshot) = &source.snapshot else {
                    continue;
                };
                // Cursor/scroll changes update the manifest without rewriting
                // every dirty rope. Reuse only content-verified snapshots;
                // revision numbers alone are not identities across restarts.
                if let Some(entry) = previous
                    .as_ref()
                    .and_then(|manifest| {
                        manifest
                            .tabs
                            .iter()
                            .find(|old| old.id == tab.id && old.path == tab.path)
                    })
                    .and_then(|old| old.snapshot.as_ref())
                    .filter(|entry| entry.snapshot_bytes == snapshot.len_bytes() as u64)
                {
                    let mut digest = blake3::Hasher::new();
                    snapshot.write_to(&mut digest)?;
                    if digest.finalize().to_hex().as_str() == entry.snapshot_blake3 {
                        self.verified_workspace_snapshot(entry)?;
                        let mut entry = entry.clone();
                        entry.document_revision = snapshot.revision;
                        entry.captured_at_unix_ms = captured_at_unix_ms;
                        tab.snapshot = Some(entry);
                        continue;
                    }
                }
                let name = snapshot_name(generation, tab.id);
                let (snapshot_bytes, digest) = write_snapshot_atomic(self.dir(), &name, snapshot)?;
                tab.snapshot = Some(AutosaveEntryV1 {
                    schema: AUTOSAVE_SCHEMA_V1.to_owned(),
                    v: 1,
                    sequence: generation,
                    captured_at_unix_ms,
                    document_path: tab.path.clone(),
                    document_revision: snapshot.revision,
                    snapshot_file: name,
                    snapshot_bytes,
                    snapshot_blake3: digest.to_hex().to_string(),
                });
            }
            manifest.validate()?;
            let bytes = serde_json::to_vec(&manifest).map_err(|_| AutosaveError::InvalidJournal)?;
            if bytes.len() > MAX_MANIFEST_BYTES {
                return Err(AutosaveError::InvalidJournal);
            }
            write_bytes_atomic(self.dir(), WORKSPACE_RECOVERY_FILE, &bytes)?;
            let retained: HashSet<_> = manifest
                .tabs
                .iter()
                .filter_map(|tab| {
                    tab.snapshot
                        .as_ref()
                        .map(|entry| entry.snapshot_file.as_str())
                })
                .collect();
            // Restrict cleanup to our exact namespace and bound directory work.
            for entry in fs::read_dir(self.dir())?.take(1024) {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if is_workspace_snapshot(name) && !retained.contains(name) {
                    fs::remove_file(entry.path())?;
                }
            }
            Ok(())
        })
    }

    /// Read all snapshots under the same lock. Damage is an explicit error;
    /// recovery never deletes files or silently substitutes clean disk content.
    pub fn recover_workspace(&self) -> Result<Option<RecoveredWorkspace>, AutosaveError> {
        with_store_lock(self.dir(), || {
            let Some(manifest) = self.read_workspace_manifest()? else {
                return Ok(None);
            };
            let mut tabs = Vec::with_capacity(manifest.tabs.len());
            for tab in manifest.tabs {
                let document = if let Some(entry) = tab.snapshot {
                    let bytes = self.verified_workspace_snapshot(&entry)?;
                    let source =
                        std::str::from_utf8(&bytes).map_err(|_| AutosaveError::InvalidJournal)?;
                    Some(Document::new(source).map_err(|_| AutosaveError::InvalidJournal)?)
                } else {
                    None
                };
                tabs.push(RecoveredWorkspaceTab {
                    id: tab.id,
                    view: tab.view,
                    path: tab.path,
                    document,
                });
            }
            Ok(Some(RecoveredWorkspace {
                active_id: manifest.active_id,
                tabs,
            }))
        })
    }

    fn verified_workspace_snapshot(
        &self,
        entry: &AutosaveEntryV1,
    ) -> Result<Vec<u8>, AutosaveError> {
        let bytes = read_snapshot_file(&self.dir().join(&entry.snapshot_file), MAX_DOCUMENT_BYTES)
            .map_err(|_| AutosaveError::InvalidJournal)?;
        if bytes.len() as u64 != entry.snapshot_bytes
            || blake3::hash(&bytes).to_hex().as_str() != entry.snapshot_blake3
        {
            return Err(AutosaveError::InvalidJournal);
        }
        Ok(bytes)
    }

    fn read_workspace_manifest(&self) -> Result<Option<Manifest>, AutosaveError> {
        let file = match open_recovery_file(&self.dir().join(WORKSPACE_RECOVERY_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take((MAX_MANIFEST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(AutosaveError::InvalidJournal);
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| AutosaveError::InvalidJournal)?;
        manifest.validate()?;
        Ok(Some(manifest))
    }
}

fn is_workspace_snapshot(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix("workspace-")
        .and_then(|body| body.strip_suffix(".md"))
    else {
        return false;
    };
    let Some((generation, id)) = body.split_once('-') else {
        return false;
    };
    generation.parse::<u64>().is_ok()
        && id.parse::<u64>().is_ok()
        && generation.bytes().all(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit())
}
