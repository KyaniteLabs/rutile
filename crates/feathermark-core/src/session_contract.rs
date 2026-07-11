//! Versioned autosave-journal and session-restore schemas (SPEC §9).
//!
//! Wave 0 freezes the on-disk wire shapes only; the (Wave 1) autosave writer
//! and recovery loader live in `FileService`/`feathermark-app`. Records
//! follow `feathermark-protocol`'s versioning style: a `schema` tag plus a
//! `v` field, NDJSON framing (one newline-terminated JSON record), bounded
//! record sizes, `deny_unknown_fields`, and validation applied symmetrically
//! on encode and decode.
//!
//! The autosave journal is a sequence of [`AutosaveEntryV1`] records; each
//! entry describes an atomically written snapshot file that sits next to the
//! journal (referenced by bare file name only — path traversal is
//! unrepresentable). Session restore is a single [`SessionStateV1`] record.

use feathermark_types::Revision;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::MAX_DOCUMENT_BYTES;

/// Schema tag for autosave journal entries.
pub const AUTOSAVE_SCHEMA_V1: &str = "feathermark.autosave.v1";
/// Schema tag for the session-restore record.
pub const SESSION_SCHEMA_V1: &str = "feathermark.session.v1";

/// Maximum bytes for one encoded autosave journal entry.
pub const MAX_AUTOSAVE_ENTRY_BYTES: usize = 4 * 1024;
/// Maximum bytes for the encoded session-restore record.
pub const MAX_SESSION_STATE_BYTES: usize = 16 * 1024;
/// Maximum entries in the session recent-files list (SPEC §9).
pub const MAX_RECENT_FILES: usize = 10;
/// Maximum bytes for any file path stored in these records.
pub const MAX_SESSION_PATH_BYTES: usize = 4096;

const BLAKE3_HEX_LEN: usize = 64;
const MAX_WINDOW_DIMENSION: u32 = 32_768;

/// One autosave journal record: metadata for a snapshot file written
/// alongside the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutosaveEntryV1 {
    /// Always [`AUTOSAVE_SCHEMA_V1`].
    pub schema: String,
    /// Always `1`.
    pub v: u8,
    /// Monotonically increasing per journal; the highest valid entry wins.
    pub sequence: u64,
    /// Capture time, Unix epoch milliseconds.
    pub captured_at_unix_ms: u64,
    /// The document's save path; `None` for an untitled buffer.
    pub document_path: Option<String>,
    /// Document revision captured in the snapshot.
    pub document_revision: Revision,
    /// Bare file name of the snapshot next to the journal. Never a path.
    pub snapshot_file: String,
    /// Snapshot size; must respect the document cap.
    pub snapshot_bytes: u64,
    /// blake3 of the snapshot bytes, 64 lowercase hex digits.
    pub snapshot_blake3: String,
}

/// Byte selection persisted for session restore.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSelectionV1 {
    pub anchor: u64,
    pub head: u64,
}

/// Window frame persisted for session restore. Position may be negative on
/// multi-monitor layouts; dimensions must be positive and sane.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionWindowV1 {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The session-restore record: last file, cursor, viewport, window frame,
/// and recent files (SPEC §9). Restore is best-effort — every field is
/// advisory and re-validated against reality when applied.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStateV1 {
    /// Always [`SESSION_SCHEMA_V1`].
    pub schema: String,
    /// Always `1`.
    pub v: u8,
    /// Save time, Unix epoch milliseconds.
    pub saved_at_unix_ms: u64,
    /// Absolute path of the last open file, if any.
    pub last_file: Option<String>,
    pub selection: Option<SessionSelectionV1>,
    pub top_visible_byte: Option<u64>,
    pub window: Option<SessionWindowV1>,
    /// Most-recent-first, at most [`MAX_RECENT_FILES`] absolute paths.
    pub recent_files: Vec<String>,
}

/// Errors for the session/autosave wire contracts, mirroring
/// `feathermark-protocol`'s error taxonomy.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("record exceeds {maximum} bytes")]
    TooLarge { maximum: usize },
    #[error("record must be exactly one newline-terminated JSON object")]
    InvalidFraming,
    #[error("invalid JSON record: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unsupported schema version")]
    UnsupportedVersion,
    #[error("record schema tag does not match")]
    InvalidSchema,
    #[error("invalid record field: {0}")]
    InvalidMetadata(&'static str),
}

pub fn encode_autosave_entry(entry: &AutosaveEntryV1) -> Result<Vec<u8>, SessionError> {
    validate_autosave_entry(entry)?;
    encode_ndjson(entry, MAX_AUTOSAVE_ENTRY_BYTES)
}

pub fn decode_autosave_entry(bytes: &[u8]) -> Result<AutosaveEntryV1, SessionError> {
    let entry: AutosaveEntryV1 = decode_ndjson(bytes, MAX_AUTOSAVE_ENTRY_BYTES)?;
    validate_autosave_entry(&entry)?;
    Ok(entry)
}

pub fn encode_session_state(state: &SessionStateV1) -> Result<Vec<u8>, SessionError> {
    validate_session_state(state)?;
    encode_ndjson(state, MAX_SESSION_STATE_BYTES)
}

pub fn decode_session_state(bytes: &[u8]) -> Result<SessionStateV1, SessionError> {
    let state: SessionStateV1 = decode_ndjson(bytes, MAX_SESSION_STATE_BYTES)?;
    validate_session_state(&state)?;
    Ok(state)
}

fn validate_autosave_entry(entry: &AutosaveEntryV1) -> Result<(), SessionError> {
    if entry.v != 1 {
        return Err(SessionError::UnsupportedVersion);
    }
    if entry.schema != AUTOSAVE_SCHEMA_V1 {
        return Err(SessionError::InvalidSchema);
    }
    if !is_bare_file_name(&entry.snapshot_file) {
        return Err(SessionError::InvalidMetadata(
            "snapshot_file must be a bare file name",
        ));
    }
    if entry.snapshot_bytes > MAX_DOCUMENT_BYTES as u64 {
        return Err(SessionError::InvalidMetadata(
            "snapshot_bytes exceeds the document cap",
        ));
    }
    if !is_lower_hex(&entry.snapshot_blake3, BLAKE3_HEX_LEN) {
        return Err(SessionError::InvalidMetadata(
            "snapshot_blake3 must be 64 lowercase hex digits",
        ));
    }
    if let Some(path) = &entry.document_path {
        validate_path(path)?;
    }
    Ok(())
}

fn validate_session_state(state: &SessionStateV1) -> Result<(), SessionError> {
    if state.v != 1 {
        return Err(SessionError::UnsupportedVersion);
    }
    if state.schema != SESSION_SCHEMA_V1 {
        return Err(SessionError::InvalidSchema);
    }
    if let Some(path) = &state.last_file {
        validate_path(path)?;
    }
    if let Some(selection) = state.selection
        && (selection.anchor > MAX_DOCUMENT_BYTES as u64
            || selection.head > MAX_DOCUMENT_BYTES as u64)
    {
        return Err(SessionError::InvalidMetadata(
            "selection offset exceeds the document cap",
        ));
    }
    if let Some(byte) = state.top_visible_byte
        && byte > MAX_DOCUMENT_BYTES as u64
    {
        return Err(SessionError::InvalidMetadata(
            "top_visible_byte exceeds the document cap",
        ));
    }
    if let Some(window) = state.window
        && (window.width == 0
            || window.height == 0
            || window.width > MAX_WINDOW_DIMENSION
            || window.height > MAX_WINDOW_DIMENSION)
    {
        return Err(SessionError::InvalidMetadata(
            "window dimensions must be positive and sane",
        ));
    }
    if state.recent_files.len() > MAX_RECENT_FILES {
        return Err(SessionError::InvalidMetadata(
            "recent_files exceeds its cap",
        ));
    }
    for path in &state.recent_files {
        validate_path(path)?;
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), SessionError> {
    if path.is_empty() {
        return Err(SessionError::InvalidMetadata("path must not be empty"));
    }
    if path.len() > MAX_SESSION_PATH_BYTES {
        return Err(SessionError::InvalidMetadata("path exceeds its byte cap"));
    }
    Ok(())
}

fn is_bare_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SESSION_PATH_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.contains('\0')
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_ndjson<T: DeserializeOwned>(bytes: &[u8], maximum: usize) -> Result<T, SessionError> {
    if bytes.len() > maximum {
        return Err(SessionError::TooLarge { maximum });
    }
    let Some(record) = bytes.strip_suffix(b"\n") else {
        return Err(SessionError::InvalidFraming);
    };
    if record.is_empty() || record.contains(&b'\n') || record.contains(&b'\r') {
        return Err(SessionError::InvalidFraming);
    }
    Ok(serde_json::from_slice(record)?)
}

fn encode_ndjson<T: Serialize>(value: &T, maximum: usize) -> Result<Vec<u8>, SessionError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    if bytes.len() > maximum {
        return Err(SessionError::TooLarge { maximum });
    }
    Ok(bytes)
}
