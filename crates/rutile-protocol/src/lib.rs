//! Versioned native/webview, GUI-control, and metric protocol contracts.

// S+-tier lint policy: enforce pedantic + nursery, allow the opinionated set.
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::Duration;

use rutile_types::{InteractionId, Revision, SafeLinkTarget};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_PREVIEW_EVENT_BYTES: usize = 1024;
pub const MAX_SCROLL_CONTROL_BYTES: usize = 256;
pub const MAX_GUI_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_METRIC_RECORD_BYTES: usize = 4 * 1024 * 1024;
pub const GUI_EVENT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderUrl {
    revision: Revision,
    nonce: [u8; 16],
}

impl RenderUrl {
    #[must_use]
    pub const fn new(revision: Revision, nonce: [u8; 16]) -> Self {
        Self { revision, nonce }
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn nonce(&self) -> &[u8; 16] {
        &self.nonce
    }

    #[must_use]
    pub fn document_path(&self) -> String {
        let mut nonce = String::with_capacity(self.nonce.len() * 2);
        for byte in &self.nonce {
            let _ = write!(nonce, "{byte:02x}");
        }
        format!("/v1/document/{}/{nonce}", self.revision)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewHostCommand {
    Navigate {
        revision: Revision,
        url: RenderUrl,
        page_bytes: usize,
    },
    ScrollTo {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewEventV1 {
    BridgeReady {
        revision: Revision,
    },
    Painted {
        revision: Revision,
        frame_seq: u64,
    },
    Scroll {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
    },
    LinkActivated {
        revision: Revision,
        target: SafeLinkTarget,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PreviewEventWireV1 {
    BridgeReady {
        v: u8,
        revision: Revision,
    },
    Painted {
        v: u8,
        revision: Revision,
        frame_seq: u64,
    },
    Scroll {
        v: u8,
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
    },
    LinkActivated {
        v: u8,
        revision: Revision,
        normalized_url: String,
    },
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("NDJSON frame exceeds {maximum} bytes")]
    TooLarge { maximum: usize },
    #[error("NDJSON must contain exactly one newline-terminated record")]
    InvalidFraming,
    #[error("invalid JSON record: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unsupported schema version")]
    UnsupportedVersion,
    #[error("stale revision")]
    StaleRevision,
    #[error("source offset exceeds the maximum document size")]
    InvalidOffset,
    #[error("unsafe or noncanonical link: {0}")]
    InvalidLink(#[from] rutile_types::SafeLinkError),
    #[error("command is not permitted on the bounded scroll-control channel")]
    InvalidControl,
    #[error("metric schema is not rutile.metric.v1")]
    InvalidMetricSchema,
    #[error("metric metadata is incomplete or malformed")]
    InvalidMetricMetadata,
}

pub type PreviewEventError = ProtocolError;

pub fn decode_preview_event(
    bytes: &[u8],
    loaded: Revision,
) -> Result<PreviewEventV1, PreviewEventError> {
    let wire: PreviewEventWireV1 = decode_ndjson(bytes, MAX_PREVIEW_EVENT_BYTES)?;
    match wire {
        PreviewEventWireV1::BridgeReady { v, revision } => {
            validate_header(v, revision, loaded)?;
            Ok(PreviewEventV1::BridgeReady { revision })
        }
        PreviewEventWireV1::Painted {
            v,
            revision,
            frame_seq,
        } => {
            validate_header(v, revision, loaded)?;
            Ok(PreviewEventV1::Painted {
                revision,
                frame_seq,
            })
        }
        PreviewEventWireV1::Scroll {
            v,
            revision,
            source_start,
            interaction_id,
            user,
        } => {
            validate_header(v, revision, loaded)?;
            if source_start > MAX_DOCUMENT_BYTES {
                return Err(ProtocolError::InvalidOffset);
            }
            Ok(PreviewEventV1::Scroll {
                revision,
                source_start,
                interaction_id,
                user,
            })
        }
        PreviewEventWireV1::LinkActivated {
            v,
            revision,
            normalized_url,
        } => {
            validate_header(v, revision, loaded)?;
            Ok(PreviewEventV1::LinkActivated {
                revision,
                target: SafeLinkTarget::parse_wire(&normalized_url)?,
            })
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ScrollControlWireV1 {
    ScrollTo {
        v: u8,
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
}

pub fn encode_scroll_control(command: &PreviewHostCommand) -> Result<Vec<u8>, ProtocolError> {
    let PreviewHostCommand::ScrollTo {
        revision,
        source_start,
        interaction_id,
    } = command
    else {
        return Err(ProtocolError::InvalidControl);
    };
    if *source_start > MAX_DOCUMENT_BYTES {
        return Err(ProtocolError::InvalidOffset);
    }
    encode_ndjson(
        &ScrollControlWireV1::ScrollTo {
            v: 1,
            revision: *revision,
            source_start: *source_start,
            interaction_id: *interaction_id,
        },
        MAX_SCROLL_CONTROL_BYTES,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuiCommandV1 {
    OpenFixture {
        request_id: u64,
        fixture: String,
    },
    Edit {
        request_id: u64,
        start: usize,
        end: usize,
        replacement: String,
    },
    BeginComposition {
        request_id: u64,
        composition_id: u64,
        start: usize,
        end: usize,
    },
    UpdateComposition {
        request_id: u64,
        composition_id: u64,
        preedit: String,
    },
    CommitComposition {
        request_id: u64,
        composition_id: u64,
        replacement: String,
    },
    CancelComposition {
        request_id: u64,
        composition_id: u64,
    },
    SetSourceViewport {
        request_id: u64,
        top_visible_byte: usize,
    },
    SetPreviewViewport {
        request_id: u64,
        y: u64,
    },
    FocusEditor {
        request_id: u64,
    },
    FocusPreview {
        request_id: u64,
    },
    Resize {
        request_id: u64,
        width: u32,
        height: u32,
    },
    HideShow {
        request_id: u64,
    },
    Close {
        request_id: u64,
    },
}

impl GuiCommandV1 {
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::OpenFixture { request_id, .. }
            | Self::Edit { request_id, .. }
            | Self::BeginComposition { request_id, .. }
            | Self::UpdateComposition { request_id, .. }
            | Self::CommitComposition { request_id, .. }
            | Self::CancelComposition { request_id, .. }
            | Self::SetSourceViewport { request_id, .. }
            | Self::SetPreviewViewport { request_id, .. }
            | Self::FocusEditor { request_id, .. }
            | Self::FocusPreview { request_id, .. }
            | Self::Resize { request_id, .. }
            | Self::HideShow { request_id, .. }
            | Self::Close { request_id, .. } => *request_id,
        }
    }
}

pub fn decode_gui_command(bytes: &[u8]) -> Result<GuiCommandV1, ProtocolError> {
    let command: GuiCommandWireV1 = decode_ndjson(bytes, MAX_GUI_COMMAND_BYTES)?;
    if command.version() != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    Ok(command.into())
}

pub fn encode_gui_command(command: &GuiCommandV1) -> Result<Vec<u8>, ProtocolError> {
    encode_ndjson(&GuiCommandWireV1::from(command), MAX_GUI_COMMAND_BYTES)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusSurface {
    Editor,
    Preview,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiErrorCode {
    InvalidCommand,
    StaleRevision,
    Timeout,
    ProcessExited,
    Protocol,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuiEventV1 {
    ControlReady {
        request_id: u64,
    },
    EditAccepted {
        request_id: u64,
        revision: Revision,
    },
    SourcePainted {
        request_id: u64,
        revision: Revision,
        frame_seq: u64,
    },
    PreviewPainted {
        request_id: u64,
        revision: Revision,
        frame_seq: u64,
    },
    Interactive {
        request_id: u64,
        revision: Revision,
    },
    FocusChanged {
        request_id: u64,
        surface: FocusSurface,
    },
    BoundsChanged {
        request_id: u64,
        width: u32,
        height: u32,
    },
    Closed {
        request_id: u64,
    },
    Error {
        request_id: u64,
        code: GuiErrorCode,
        message: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum GuiEventWireV1 {
    ControlReady {
        v: u8,
        request_id: u64,
    },
    EditAccepted {
        v: u8,
        request_id: u64,
        revision: Revision,
    },
    SourcePainted {
        v: u8,
        request_id: u64,
        revision: Revision,
        frame_seq: u64,
    },
    PreviewPainted {
        v: u8,
        request_id: u64,
        revision: Revision,
        frame_seq: u64,
    },
    Interactive {
        v: u8,
        request_id: u64,
        revision: Revision,
    },
    FocusChanged {
        v: u8,
        request_id: u64,
        surface: FocusSurface,
    },
    BoundsChanged {
        v: u8,
        request_id: u64,
        width: u32,
        height: u32,
    },
    Closed {
        v: u8,
        request_id: u64,
    },
    Error {
        v: u8,
        request_id: u64,
        code: GuiErrorCode,
        message: String,
    },
}

pub fn encode_gui_event(event: &GuiEventV1) -> Result<Vec<u8>, ProtocolError> {
    encode_ndjson(&GuiEventWireV1::from(event), MAX_GUI_COMMAND_BYTES)
}

pub fn decode_gui_event(bytes: &[u8]) -> Result<GuiEventV1, ProtocolError> {
    let event: GuiEventWireV1 = decode_ndjson(bytes, MAX_GUI_COMMAND_BYTES)?;
    if event.version() != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    Ok(event.into())
}

impl From<&GuiEventV1> for GuiEventWireV1 {
    fn from(event: &GuiEventV1) -> Self {
        match event {
            GuiEventV1::ControlReady { request_id } => Self::ControlReady {
                v: 1,
                request_id: *request_id,
            },
            GuiEventV1::EditAccepted {
                request_id,
                revision,
            } => Self::EditAccepted {
                v: 1,
                request_id: *request_id,
                revision: *revision,
            },
            GuiEventV1::SourcePainted {
                request_id,
                revision,
                frame_seq,
            } => Self::SourcePainted {
                v: 1,
                request_id: *request_id,
                revision: *revision,
                frame_seq: *frame_seq,
            },
            GuiEventV1::PreviewPainted {
                request_id,
                revision,
                frame_seq,
            } => Self::PreviewPainted {
                v: 1,
                request_id: *request_id,
                revision: *revision,
                frame_seq: *frame_seq,
            },
            GuiEventV1::Interactive {
                request_id,
                revision,
            } => Self::Interactive {
                v: 1,
                request_id: *request_id,
                revision: *revision,
            },
            GuiEventV1::FocusChanged {
                request_id,
                surface,
            } => Self::FocusChanged {
                v: 1,
                request_id: *request_id,
                surface: *surface,
            },
            GuiEventV1::BoundsChanged {
                request_id,
                width,
                height,
            } => Self::BoundsChanged {
                v: 1,
                request_id: *request_id,
                width: *width,
                height: *height,
            },
            GuiEventV1::Closed { request_id } => Self::Closed {
                v: 1,
                request_id: *request_id,
            },
            GuiEventV1::Error {
                request_id,
                code,
                message,
            } => Self::Error {
                v: 1,
                request_id: *request_id,
                code: *code,
                message: message.clone(),
            },
        }
    }
}

impl GuiEventWireV1 {
    const fn version(&self) -> u8 {
        match self {
            Self::ControlReady { v, .. }
            | Self::EditAccepted { v, .. }
            | Self::SourcePainted { v, .. }
            | Self::PreviewPainted { v, .. }
            | Self::Interactive { v, .. }
            | Self::FocusChanged { v, .. }
            | Self::BoundsChanged { v, .. }
            | Self::Closed { v, .. }
            | Self::Error { v, .. } => *v,
        }
    }
}

impl From<GuiEventWireV1> for GuiEventV1 {
    fn from(value: GuiEventWireV1) -> Self {
        match value {
            GuiEventWireV1::ControlReady { request_id, .. } => Self::ControlReady { request_id },
            GuiEventWireV1::EditAccepted {
                request_id,
                revision,
                ..
            } => Self::EditAccepted {
                request_id,
                revision,
            },
            GuiEventWireV1::SourcePainted {
                request_id,
                revision,
                frame_seq,
                ..
            } => Self::SourcePainted {
                request_id,
                revision,
                frame_seq,
            },
            GuiEventWireV1::PreviewPainted {
                request_id,
                revision,
                frame_seq,
                ..
            } => Self::PreviewPainted {
                request_id,
                revision,
                frame_seq,
            },
            GuiEventWireV1::Interactive {
                request_id,
                revision,
                ..
            } => Self::Interactive {
                request_id,
                revision,
            },
            GuiEventWireV1::FocusChanged {
                request_id,
                surface,
                ..
            } => Self::FocusChanged {
                request_id,
                surface,
            },
            GuiEventWireV1::BoundsChanged {
                request_id,
                width,
                height,
                ..
            } => Self::BoundsChanged {
                request_id,
                width,
                height,
            },
            GuiEventWireV1::Closed { request_id, .. } => Self::Closed { request_id },
            GuiEventWireV1::Error {
                request_id,
                code,
                message,
                ..
            } => Self::Error {
                request_id,
                code,
                message,
            },
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum GuiCommandWireV1 {
    OpenFixture {
        v: u8,
        request_id: u64,
        fixture: String,
    },
    Edit {
        v: u8,
        request_id: u64,
        start: usize,
        end: usize,
        replacement: String,
    },
    BeginComposition {
        v: u8,
        request_id: u64,
        composition_id: u64,
        start: usize,
        end: usize,
    },
    UpdateComposition {
        v: u8,
        request_id: u64,
        composition_id: u64,
        preedit: String,
    },
    CommitComposition {
        v: u8,
        request_id: u64,
        composition_id: u64,
        replacement: String,
    },
    CancelComposition {
        v: u8,
        request_id: u64,
        composition_id: u64,
    },
    SetSourceViewport {
        v: u8,
        request_id: u64,
        top_visible_byte: usize,
    },
    SetPreviewViewport {
        v: u8,
        request_id: u64,
        y: u64,
    },
    FocusEditor {
        v: u8,
        request_id: u64,
    },
    FocusPreview {
        v: u8,
        request_id: u64,
    },
    Resize {
        v: u8,
        request_id: u64,
        width: u32,
        height: u32,
    },
    HideShow {
        v: u8,
        request_id: u64,
    },
    Close {
        v: u8,
        request_id: u64,
    },
}

impl GuiCommandWireV1 {
    const fn version(&self) -> u8 {
        match self {
            Self::OpenFixture { v, .. }
            | Self::Edit { v, .. }
            | Self::BeginComposition { v, .. }
            | Self::UpdateComposition { v, .. }
            | Self::CommitComposition { v, .. }
            | Self::CancelComposition { v, .. }
            | Self::SetSourceViewport { v, .. }
            | Self::SetPreviewViewport { v, .. }
            | Self::FocusEditor { v, .. }
            | Self::FocusPreview { v, .. }
            | Self::Resize { v, .. }
            | Self::HideShow { v, .. }
            | Self::Close { v, .. } => *v,
        }
    }
}

impl From<&GuiCommandV1> for GuiCommandWireV1 {
    fn from(value: &GuiCommandV1) -> Self {
        match value {
            GuiCommandV1::OpenFixture {
                request_id,
                fixture,
            } => Self::OpenFixture {
                v: 1,
                request_id: *request_id,
                fixture: fixture.clone(),
            },
            GuiCommandV1::Edit {
                request_id,
                start,
                end,
                replacement,
            } => Self::Edit {
                v: 1,
                request_id: *request_id,
                start: *start,
                end: *end,
                replacement: replacement.clone(),
            },
            GuiCommandV1::BeginComposition {
                request_id,
                composition_id,
                start,
                end,
            } => Self::BeginComposition {
                v: 1,
                request_id: *request_id,
                composition_id: *composition_id,
                start: *start,
                end: *end,
            },
            GuiCommandV1::UpdateComposition {
                request_id,
                composition_id,
                preedit,
            } => Self::UpdateComposition {
                v: 1,
                request_id: *request_id,
                composition_id: *composition_id,
                preedit: preedit.clone(),
            },
            GuiCommandV1::CommitComposition {
                request_id,
                composition_id,
                replacement,
            } => Self::CommitComposition {
                v: 1,
                request_id: *request_id,
                composition_id: *composition_id,
                replacement: replacement.clone(),
            },
            GuiCommandV1::CancelComposition {
                request_id,
                composition_id,
            } => Self::CancelComposition {
                v: 1,
                request_id: *request_id,
                composition_id: *composition_id,
            },
            GuiCommandV1::SetSourceViewport {
                request_id,
                top_visible_byte,
            } => Self::SetSourceViewport {
                v: 1,
                request_id: *request_id,
                top_visible_byte: *top_visible_byte,
            },
            GuiCommandV1::SetPreviewViewport { request_id, y } => Self::SetPreviewViewport {
                v: 1,
                request_id: *request_id,
                y: *y,
            },
            GuiCommandV1::FocusEditor { request_id } => Self::FocusEditor {
                v: 1,
                request_id: *request_id,
            },
            GuiCommandV1::FocusPreview { request_id } => Self::FocusPreview {
                v: 1,
                request_id: *request_id,
            },
            GuiCommandV1::Resize {
                request_id,
                width,
                height,
            } => Self::Resize {
                v: 1,
                request_id: *request_id,
                width: *width,
                height: *height,
            },
            GuiCommandV1::HideShow { request_id } => Self::HideShow {
                v: 1,
                request_id: *request_id,
            },
            GuiCommandV1::Close { request_id } => Self::Close {
                v: 1,
                request_id: *request_id,
            },
        }
    }
}

impl From<GuiCommandWireV1> for GuiCommandV1 {
    fn from(value: GuiCommandWireV1) -> Self {
        match value {
            GuiCommandWireV1::OpenFixture {
                request_id,
                fixture,
                ..
            } => Self::OpenFixture {
                request_id,
                fixture,
            },
            GuiCommandWireV1::Edit {
                request_id,
                start,
                end,
                replacement,
                ..
            } => Self::Edit {
                request_id,
                start,
                end,
                replacement,
            },
            GuiCommandWireV1::BeginComposition {
                request_id,
                composition_id,
                start,
                end,
                ..
            } => Self::BeginComposition {
                request_id,
                composition_id,
                start,
                end,
            },
            GuiCommandWireV1::UpdateComposition {
                request_id,
                composition_id,
                preedit,
                ..
            } => Self::UpdateComposition {
                request_id,
                composition_id,
                preedit,
            },
            GuiCommandWireV1::CommitComposition {
                request_id,
                composition_id,
                replacement,
                ..
            } => Self::CommitComposition {
                request_id,
                composition_id,
                replacement,
            },
            GuiCommandWireV1::CancelComposition {
                request_id,
                composition_id,
                ..
            } => Self::CancelComposition {
                request_id,
                composition_id,
            },
            GuiCommandWireV1::SetSourceViewport {
                request_id,
                top_visible_byte,
                ..
            } => Self::SetSourceViewport {
                request_id,
                top_visible_byte,
            },
            GuiCommandWireV1::SetPreviewViewport { request_id, y, .. } => {
                Self::SetPreviewViewport { request_id, y }
            }
            GuiCommandWireV1::FocusEditor { request_id, .. } => Self::FocusEditor { request_id },
            GuiCommandWireV1::FocusPreview { request_id, .. } => Self::FocusPreview { request_id },
            GuiCommandWireV1::Resize {
                request_id,
                width,
                height,
                ..
            } => Self::Resize {
                request_id,
                width,
                height,
            },
            GuiCommandWireV1::HideShow { request_id, .. } => Self::HideShow { request_id },
            GuiCommandWireV1::Close { request_id, .. } => Self::Close { request_id },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricRecordV1 {
    pub schema: String,
    pub v: u8,
    pub scenario: String,
    pub git_commit: String,
    pub dirty: bool,
    pub rustc_version: String,
    pub toolchain: String,
    pub target_triple: String,
    pub release_profile: String,
    pub features: Vec<String>,
    pub build_kind: String,
    pub candidate_executable_sha256: String,
    pub package_sha256: Option<String>,
    pub runner_id: String,
    pub runner_lock_sha256: String,
    pub pristine_snapshot_id: String,
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub ram_bytes: u64,
    pub os: String,
    pub kernel: String,
    pub display_session: String,
    pub display_environment: BTreeMap<String, String>,
    pub webview_version: String,
    pub monitor_scale_milli: u32,
    pub monitor_refresh_millihz: u32,
    pub fixture_sha256: String,
    pub fixture_bytes: u64,
    pub captured_at_utc: String,
    pub monotonic_clock: String,
    pub warmups: usize,
    pub samples: Vec<u64>,
    pub skipped: u64,
    pub stale: u64,
    pub pid_rss_samples: Vec<PidRssSampleV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PidRssSampleV1 {
    pub monotonic_ns: u64,
    pub processes: Vec<ProcessRssV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRssV1 {
    pub pid: u32,
    pub ppid: u32,
    pub coalition: Option<u64>,
    pub rss_kib: u64,
}

pub fn decode_metric_record(bytes: &[u8]) -> Result<MetricRecordV1, ProtocolError> {
    let record: MetricRecordV1 = decode_ndjson(bytes, MAX_METRIC_RECORD_BYTES)?;
    if record.v != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    if record.schema != "rutile.metric.v1" {
        return Err(ProtocolError::InvalidMetricSchema);
    }
    validate_metric_metadata(&record)?;
    Ok(record)
}

pub fn encode_metric_record(record: &MetricRecordV1) -> Result<Vec<u8>, ProtocolError> {
    if record.v != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    if record.schema != "rutile.metric.v1" {
        return Err(ProtocolError::InvalidMetricSchema);
    }
    validate_metric_metadata(record)?;
    encode_ndjson(record, MAX_METRIC_RECORD_BYTES)
}

fn validate_metric_metadata(record: &MetricRecordV1) -> Result<(), ProtocolError> {
    let required = [
        record.scenario.as_str(),
        record.rustc_version.as_str(),
        record.toolchain.as_str(),
        record.target_triple.as_str(),
        record.release_profile.as_str(),
        record.build_kind.as_str(),
        record.runner_id.as_str(),
        record.pristine_snapshot_id.as_str(),
        record.cpu_model.as_str(),
        record.os.as_str(),
        record.kernel.as_str(),
        record.display_session.as_str(),
        record.webview_version.as_str(),
        record.captured_at_utc.as_str(),
        record.monotonic_clock.as_str(),
    ];
    if required.iter().any(|value| value.is_empty())
        || record.cpu_cores == 0
        || record.ram_bytes == 0
        || record.monitor_scale_milli == 0
        || record.monitor_refresh_millihz == 0
        || !(is_lower_hex(&record.git_commit, 40) || is_lower_hex(&record.git_commit, 64))
        || !is_lower_hex(&record.candidate_executable_sha256, 64)
        || !is_lower_hex(&record.runner_lock_sha256, 64)
        || !is_lower_hex(&record.fixture_sha256, 64)
        || record
            .package_sha256
            .as_ref()
            .is_some_and(|hash| !is_lower_hex(hash, 64))
    {
        return Err(ProtocolError::InvalidMetricMetadata);
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn validate_header(v: u8, revision: Revision, loaded: Revision) -> Result<(), ProtocolError> {
    if v != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    if revision != loaded {
        return Err(ProtocolError::StaleRevision);
    }
    Ok(())
}

fn decode_ndjson<T: DeserializeOwned>(bytes: &[u8], maximum: usize) -> Result<T, ProtocolError> {
    if bytes.len() > maximum {
        return Err(ProtocolError::TooLarge { maximum });
    }
    let Some(record) = bytes.strip_suffix(b"\n") else {
        return Err(ProtocolError::InvalidFraming);
    };
    if record.is_empty() || record.contains(&b'\n') || record.contains(&b'\r') {
        return Err(ProtocolError::InvalidFraming);
    }
    Ok(serde_json::from_slice(record)?)
}

fn encode_ndjson<T: Serialize>(value: &T, maximum: usize) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    if bytes.len() > maximum {
        return Err(ProtocolError::TooLarge { maximum });
    }
    Ok(bytes)
}
