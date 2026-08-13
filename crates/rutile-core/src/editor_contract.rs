use std::ops::Range;
use std::sync::Arc;

use rutile_types::{InteractionId, Revision};
use thiserror::Error;

use crate::{
    ChangeSet, Document, DocumentSnapshot, Edit, EditError, EditTransaction, HistoryContext,
    TransactionKind,
};

pub type AdapterCommitId = u64;
/// A composition (IME) sequence id. Distinct from `Revision` and
/// `AdapterCommitId`: the newtype wrapper prevents transposed-argument bugs.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct CompositionId(u64);

impl CompositionId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for CompositionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeCommit {
    pub composition_id: CompositionId,
    pub base_revision: Revision,
    pub byte_range: Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorCommit {
    Edit {
        transaction: EditTransaction,
        history: Option<HistoryContext>,
    },
    Ime(ImeCommit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorEvent {
    CommitRequested {
        adapter_commit_id: AdapterCommitId,
        commit: EditorCommit,
    },
    CompositionStarted {
        id: CompositionId,
        base_revision: Revision,
        byte_range: Range<usize>,
    },
    CompositionUpdated {
        id: CompositionId,
        base_revision: Revision,
        preedit: String,
    },
    CompositionCancelled {
        id: CompositionId,
        base_revision: Revision,
        reason: CompositionCancelReason,
    },
    ViewportChanged {
        revision: Revision,
        top_visible_byte: usize,
        user: bool,
    },
    SourcePainted {
        revision: Revision,
        frame_seq: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionCancelReason {
    User,
    FocusLost,
    StaleRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalCommitRejection {
    StaleRevision,
    InvalidEdit,
    TooLarge,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("revision {actual} is stale; current revision is {expected}")]
pub struct StaleRevision {
    pub expected: Revision,
    pub actual: Revision,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum EditorError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    StaleRevision(#[from] StaleRevision),
    #[error("adapter commit id {actual} does not match transaction id {expected}")]
    AdapterCommitMismatch {
        expected: AdapterCommitId,
        actual: AdapterCommitId,
    },
    #[error("viewport byte {byte} exceeds document length {document_len}")]
    InvalidViewport { byte: usize, document_len: usize },
    #[error("viewport byte {byte} is not a UTF-8 boundary")]
    InvalidViewportBoundary { byte: usize },
    #[error("platform adapter error: {0}")]
    Platform(String),
}

pub type EditorEventSink = Box<dyn FnMut(EditorEvent) + 'static>;

pub trait EditorAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink);
    fn install_open_snapshot(&mut self, snapshot: &DocumentSnapshot) -> Result<(), EditorError>;
    fn acknowledge_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        change: &ChangeSet,
    ) -> Result<(), EditorError>;
    fn reject_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        reason: LocalCommitRejection,
        authoritative: &DocumentSnapshot,
    ) -> Result<(), EditorError>;
    fn apply_external_change(&mut self, change: &ChangeSet) -> Result<(), EditorError>;
    fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision>;
    fn scroll_to_byte(
        &mut self,
        revision: Revision,
        byte: usize,
        id: InteractionId,
    ) -> Result<(), EditorError>;
    fn set_read_only_generated(
        &mut self,
        revision: Revision,
        html: Arc<str>,
    ) -> Result<(), EditorError>;
}

pub fn apply_editor_commit(
    document: &mut Document,
    adapter_commit_id: AdapterCommitId,
    commit: EditorCommit,
) -> Result<ChangeSet, EditorError> {
    match commit {
        EditorCommit::Edit {
            transaction,
            history,
        } => {
            if transaction.id != adapter_commit_id {
                return Err(EditorError::AdapterCommitMismatch {
                    expected: transaction.id,
                    actual: adapter_commit_id,
                });
            }
            match history {
                Some(history) => document.apply_with_history(transaction, history),
                None => document.apply(transaction),
            }
            .map_err(EditorError::from)
        }
        EditorCommit::Ime(ime) => document
            .apply(EditTransaction {
                base_revision: ime.base_revision,
                id: adapter_commit_id,
                kind: TransactionKind::ImeCommit,
                edits: vec![Edit {
                    byte_range: ime.byte_range,
                    replacement: ime.replacement,
                }],
            })
            .map_err(EditorError::from),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveComposition {
    id: CompositionId,
    base_revision: Revision,
    byte_range: Range<usize>,
}

#[derive(Default, Debug)]
pub struct CompositionTracker {
    active: Option<ActiveComposition>,
}

impl CompositionTracker {
    pub fn start(
        &mut self,
        id: CompositionId,
        base_revision: Revision,
        byte_range: Range<usize>,
    ) -> Option<EditorEvent> {
        if self.active.is_some() || byte_range.start > byte_range.end {
            return None;
        }
        self.active = Some(ActiveComposition {
            id,
            base_revision,
            byte_range: byte_range.clone(),
        });
        Some(EditorEvent::CompositionStarted {
            id,
            base_revision,
            byte_range,
        })
    }

    pub fn update(
        &self,
        id: CompositionId,
        base_revision: Revision,
        preedit: impl Into<String>,
    ) -> Option<EditorEvent> {
        self.matching(id, base_revision)?;
        Some(EditorEvent::CompositionUpdated {
            id,
            base_revision,
            preedit: preedit.into(),
        })
    }

    pub fn commit(
        &mut self,
        id: CompositionId,
        base_revision: Revision,
        adapter_commit_id: AdapterCommitId,
        replacement: impl Into<String>,
    ) -> Option<EditorEvent> {
        self.matching(id, base_revision)?;
        let active = self.active.take()?;
        Some(EditorEvent::CommitRequested {
            adapter_commit_id,
            commit: EditorCommit::Ime(ImeCommit {
                composition_id: id,
                base_revision,
                byte_range: active.byte_range,
                replacement: replacement.into(),
            }),
        })
    }

    pub fn cancel(&mut self, reason: CompositionCancelReason) -> Option<EditorEvent> {
        let active = self.active.take()?;
        Some(EditorEvent::CompositionCancelled {
            id: active.id,
            base_revision: active.base_revision,
            reason,
        })
    }

    pub fn invalidate_for_revision(&mut self, revision: Revision) -> Option<EditorEvent> {
        if self.active.as_ref()?.base_revision == revision {
            return None;
        }
        self.cancel(CompositionCancelReason::StaleRevision)
    }

    fn matching(&self, id: CompositionId, base_revision: Revision) -> Option<&ActiveComposition> {
        self.active
            .as_ref()
            .filter(|active| active.id == id && active.base_revision == base_revision)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportState {
    revision: Revision,
    top_visible_byte: usize,
}

impl ViewportState {
    pub fn new(snapshot: &DocumentSnapshot, top_visible_byte: usize) -> Result<Self, EditorError> {
        validate_viewport(snapshot, top_visible_byte)?;
        Ok(Self {
            revision: snapshot.revision,
            top_visible_byte,
        })
    }

    pub fn update(
        &mut self,
        snapshot: &DocumentSnapshot,
        top_visible_byte: usize,
    ) -> Result<(), EditorError> {
        validate_viewport(snapshot, top_visible_byte)?;
        self.revision = snapshot.revision;
        self.top_visible_byte = top_visible_byte;
        Ok(())
    }

    pub fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision> {
        if revision != self.revision {
            return Err(StaleRevision {
                expected: self.revision,
                actual: revision,
            });
        }
        Ok(self.top_visible_byte)
    }
}

fn validate_viewport(snapshot: &DocumentSnapshot, byte: usize) -> Result<(), EditorError> {
    if byte > snapshot.len_bytes() {
        return Err(EditorError::InvalidViewport {
            byte,
            document_len: snapshot.len_bytes(),
        });
    }
    if !snapshot.is_char_boundary(byte) {
        return Err(EditorError::InvalidViewportBoundary { byte });
    }
    Ok(())
}
