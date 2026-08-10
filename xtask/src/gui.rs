use std::collections::BTreeSet;

use rutile_protocol::{
    GUI_EVENT_TIMEOUT, GuiEventV1, ProtocolError, decode_gui_command, decode_gui_event,
};
use thiserror::Error;

pub const CONTROL_TIMEOUT_SECONDS: u64 = GUI_EVENT_TIMEOUT.as_secs();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptSummary {
    pub commands: usize,
    pub events: usize,
}

#[derive(Debug, Error)]
pub enum GuiDriverError {
    #[error("GUI transcript is not newline terminated")]
    InvalidFraming,
    #[error("GUI transcript protocol failed: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("event request id {0} has no correlated command")]
    Uncorrelated(u64),
    #[error("duplicate command request id {0}")]
    DuplicateRequest(u64),
}

pub fn validate_transcript(
    command_bytes: &[u8],
    event_bytes: &[u8],
) -> Result<TranscriptSummary, GuiDriverError> {
    let command_frames = frames(command_bytes)?;
    let event_frames = frames(event_bytes)?;
    let mut request_ids = BTreeSet::new();
    for frame in &command_frames {
        let command = decode_gui_command(frame)?;
        if !request_ids.insert(command.request_id()) {
            return Err(GuiDriverError::DuplicateRequest(command.request_id()));
        }
    }
    for frame in &event_frames {
        let event = decode_gui_event(frame)?;
        let request_id = event_request_id(&event);
        if !request_ids.contains(&request_id) {
            return Err(GuiDriverError::Uncorrelated(request_id));
        }
    }
    Ok(TranscriptSummary {
        commands: command_frames.len(),
        events: event_frames.len(),
    })
}

fn frames(bytes: &[u8]) -> Result<Vec<&[u8]>, GuiDriverError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(GuiDriverError::InvalidFraming);
    }
    let mut start = 0;
    let mut frames = Vec::new();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index == start {
                return Err(GuiDriverError::InvalidFraming);
            }
            frames.push(&bytes[start..=index]);
            start = index + 1;
        }
    }
    Ok(frames)
}

fn event_request_id(event: &GuiEventV1) -> u64 {
    match event {
        GuiEventV1::ControlReady { request_id }
        | GuiEventV1::EditAccepted { request_id, .. }
        | GuiEventV1::SourcePainted { request_id, .. }
        | GuiEventV1::PreviewPainted { request_id, .. }
        | GuiEventV1::Interactive { request_id, .. }
        | GuiEventV1::FocusChanged { request_id, .. }
        | GuiEventV1::BoundsChanged { request_id, .. }
        | GuiEventV1::Closed { request_id }
        | GuiEventV1::Error { request_id, .. } => *request_id,
    }
}
