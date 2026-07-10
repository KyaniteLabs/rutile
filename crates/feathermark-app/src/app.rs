use feathermark_core::RenderError;
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
    },
    DocumentEdited {
        revision: Revision,
    },
    SaveCompleted {
        revision: Revision,
    },
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
    IgnoredStale {
        revision: Revision,
    },
}

#[derive(Debug, Default)]
pub struct AppState {
    revision: Revision,
    dirty: bool,
    preview: PreviewState,
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

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
            AppMessage::NewDocument => {
                self.revision = 0;
                self.dirty = false;
                self.preview = PreviewState::Waiting { revision: 0 };
                vec![AppEffect::ScheduleRender { revision: 0 }]
            }
            AppMessage::DocumentOpened { revision } => {
                self.revision = revision;
                self.dirty = false;
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
            AppMessage::SaveCompleted { revision } if revision == self.revision => {
                self.dirty = false;
                vec![]
            }
            AppMessage::SaveCompleted { revision } => {
                vec![AppEffect::IgnoredStale { revision }]
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
