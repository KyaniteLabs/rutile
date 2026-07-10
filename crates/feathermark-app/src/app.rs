use std::path::{Path, PathBuf};

use feathermark_core::{DiskVersion, ExternalResolution, RenderError};
use feathermark_protocol::PreviewEventV1;
use feathermark_types::{InteractionId, Revision, SafeLinkTarget};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PreviewState {
    #[default]
    Empty,
    Waiting {
        revision: Revision,
    },
    Navigating {
        revision: Revision,
    },
    Ready {
        revision: Revision,
    },
    TooLarge {
        revision: Revision,
    },
    Failed {
        revision: Revision,
        error: RenderError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppMessage {
    NewDocument,
    DocumentOpened {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    DocumentEdited {
        revision: Revision,
    },
    SaveCompleted {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    SaveFailed {
        revision: Revision,
    },
    ExternalConflictDetected {
        disk: DiskVersion,
    },
    ResolveExternalConflict(ExternalResolution),
    RenderAccepted {
        revision: Revision,
        page_bytes: usize,
    },
    RenderFailed {
        revision: Revision,
        error: RenderError,
    },
    PreviewEvent(PreviewEventV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    ScheduleRender {
        revision: Revision,
    },
    NavigatePreview {
        revision: Revision,
        page_bytes: usize,
    },
    ScrollSource {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
    },
    PresentLink(SafeLinkTarget),
    PresentExternalConflict {
        path: PathBuf,
        disk: DiskVersion,
    },
    ReloadExternal {
        path: PathBuf,
    },
    SaveExternalAs {
        path: PathBuf,
    },
    IgnoredStale {
        revision: Revision,
    },
}

#[derive(Debug, Default)]
pub struct AppState {
    revision: Revision,
    dirty: bool,
    preview: PreviewState,
    path: Option<PathBuf>,
    saved_disk: Option<DiskVersion>,
    external_conflict: Option<DiskVersion>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn preview(&self) -> &PreviewState {
        &self.preview
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn saved_disk(&self) -> Option<&DiskVersion> {
        self.saved_disk.as_ref()
    }

    pub fn external_conflict(&self) -> Option<&DiskVersion> {
        self.external_conflict.as_ref()
    }

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
            AppMessage::NewDocument => {
                self.revision = 0;
                self.dirty = false;
                self.path = None;
                self.saved_disk = None;
                self.external_conflict = None;
                self.preview = PreviewState::Waiting { revision: 0 };
                vec![AppEffect::ScheduleRender { revision: 0 }]
            }
            AppMessage::DocumentOpened {
                revision,
                path,
                disk,
            } => {
                self.revision = revision;
                self.dirty = false;
                self.path = Some(path);
                self.saved_disk = Some(disk);
                self.external_conflict = None;
                self.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::DocumentEdited { revision } if revision <= self.revision => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::DocumentEdited { revision } => {
                self.revision = revision;
                self.dirty = true;
                self.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::SaveCompleted {
                revision,
                path,
                disk,
            } if revision == self.revision => {
                self.dirty = false;
                self.path = Some(path);
                self.saved_disk = Some(disk);
                self.external_conflict = None;
                vec![]
            }
            AppMessage::SaveCompleted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::SaveFailed { revision } if revision == self.revision => {
                // A failed save leaves the document dirty and any conflict
                // unresolved; the platform shell must present the error and
                // keep the document open.
                vec![]
            }
            AppMessage::SaveFailed { revision } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::ExternalConflictDetected { disk } => {
                if self.saved_disk.as_ref() == Some(&disk) {
                    return vec![];
                }
                let Some(path) = self.path.clone() else {
                    return vec![];
                };
                self.external_conflict = Some(disk.clone());
                vec![AppEffect::PresentExternalConflict { path, disk }]
            }
            AppMessage::ResolveExternalConflict(resolution) => {
                let Some(disk) = self.external_conflict.take() else {
                    return vec![];
                };
                match resolution {
                    ExternalResolution::ReloadDisk => self
                        .path
                        .clone()
                        .map(|path| vec![AppEffect::ReloadExternal { path }])
                        .unwrap_or_default(),
                    ExternalResolution::KeepBuffer => {
                        self.saved_disk = Some(disk);
                        vec![]
                    }
                    ExternalResolution::SaveBufferAs(path) => {
                        vec![AppEffect::SaveExternalAs { path }]
                    }
                }
            }
            AppMessage::RenderAccepted {
                revision,
                page_bytes,
            } if revision == self.revision => {
                self.preview = PreviewState::Navigating { revision };
                vec![AppEffect::NavigatePreview {
                    revision,
                    page_bytes,
                }]
            }
            AppMessage::RenderAccepted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::RenderFailed { revision, error } if revision == self.revision => {
                self.preview = match error {
                    RenderError::PreviewTooLarge => PreviewState::TooLarge { revision },
                    error => PreviewState::Failed { revision, error },
                };
                vec![]
            }
            AppMessage::RenderFailed { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::PreviewEvent(event) => self.reduce_preview_event(event),
        }
    }

    fn reduce_preview_event(&mut self, event: PreviewEventV1) -> Vec<AppEffect> {
        let revision = event_revision(&event);
        if revision != self.revision {
            return vec![AppEffect::IgnoredStale { revision }];
        }

        match event {
            PreviewEventV1::BridgeReady { .. } => vec![],
            PreviewEventV1::Painted {
                revision,
                frame_seq,
            } => {
                if frame_seq >= 2 {
                    self.preview = PreviewState::Ready { revision };
                }
                vec![]
            }
            PreviewEventV1::Scroll {
                revision,
                source_start,
                interaction_id,
                user,
            } => vec![AppEffect::ScrollSource {
                revision,
                source_start,
                interaction_id,
                user,
            }],
            PreviewEventV1::LinkActivated { target, .. } => {
                vec![AppEffect::PresentLink(target)]
            }
        }
    }
}

fn event_revision(event: &PreviewEventV1) -> Revision {
    match event {
        PreviewEventV1::BridgeReady { revision }
        | PreviewEventV1::Painted { revision, .. }
        | PreviewEventV1::Scroll { revision, .. }
        | PreviewEventV1::LinkActivated { revision, .. } => *revision,
    }
}
