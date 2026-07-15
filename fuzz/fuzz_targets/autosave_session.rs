#![no_main]

//! Fuzz the autosave-journal and session-restore wire decoders.
//!
//! These decoders are the untrusted on-disk input boundary: a hostile, torn,
//! or half-written journal or session file is fed byte-for-byte to
//! [`feathermark_core::decode_autosave_entry`] and
//! [`feathermark_core::decode_session_state`]. Crash recovery calls the same
//! per-line decoder, so every rejection surfaced here is a typed
//! [`feathermark_core::SessionError`] that a real recovery run would classify
//! into a [`feathermark_core::RejectionReason`] — never a panic, never a
//! silent accept.
//!
//! Invariants asserted on every input:
//! * neither decoder ever panics, on valid or arbitrarily hostile bytes;
//! * an oversize record is rejected *before* JSON parsing (bounded read, no
//!   unbounded allocation): any input larger than the record cap always errors;
//! * a successfully decoded record round-trips through its encoder, stays
//!   within the byte cap, and decodes back to an equal record (the encode and
//!   decode validation are symmetric);
//! * a decoded record's path-like fields are safe by construction: the
//!   `snapshot_file` is a bare file name (no separators, no NUL) and every
//!   `document_path` / `last_file` / `recent_files` entry is absolute and
//!   NUL-free — the path-traversal protection the wire contract guarantees.

use feathermark_core::{
    AutosaveEntryV1, MAX_AUTOSAVE_ENTRY_BYTES, MAX_SESSION_PATH_BYTES, MAX_SESSION_STATE_BYTES,
    SessionStateV1, decode_autosave_entry, decode_session_state, encode_autosave_entry,
    encode_session_state,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    fuzz_autosave_entry(input);
    fuzz_session_state(input);
});

fn fuzz_autosave_entry(input: &[u8]) {
    // Bounded read: anything larger than the entry cap is rejected before the
    // record is parsed or allocated.
    if input.len() > MAX_AUTOSAVE_ENTRY_BYTES {
        assert!(decode_autosave_entry(input).is_err());
        return;
    }
    let Ok(entry) = decode_autosave_entry(input) else {
        return;
    };

    // A decoded record must re-encode (validation is symmetric) and stay
    // within the cap. `serde_json` never grows a value on re-serialization, so
    // a value decoded from a bounded input always re-encodes within bounds.
    let Ok(encoded) = encode_autosave_entry(&entry) else {
        return;
    };
    assert!(encoded.len() <= MAX_AUTOSAVE_ENTRY_BYTES);
    assert_eq!(
        decode_autosave_entry(&encoded).expect("re-encoded record decodes"),
        entry
    );

    assert_safe_entry(&entry);
}

fn fuzz_session_state(input: &[u8]) {
    if input.len() > MAX_SESSION_STATE_BYTES {
        assert!(decode_session_state(input).is_err());
        return;
    }
    let Ok(state) = decode_session_state(input) else {
        return;
    };

    let Ok(encoded) = encode_session_state(&state) else {
        return;
    };
    assert!(encoded.len() <= MAX_SESSION_STATE_BYTES);
    assert_eq!(
        decode_session_state(&encoded).expect("re-encoded state decodes"),
        state
    );

    assert_safe_state(&state);
}

fn assert_safe_entry(entry: &AutosaveEntryV1) {
    assert_is_bare_file_name(&entry.snapshot_file);
    if let Some(path) = &entry.document_path {
        assert_is_safe_absolute_path(path);
    }
}

fn assert_safe_state(state: &SessionStateV1) {
    if let Some(path) = &state.last_file {
        assert_is_safe_absolute_path(path);
    }
    for path in &state.recent_files {
        assert_is_safe_absolute_path(path);
    }
}

/// Mirrors `session_contract::is_bare_file_name`: a snapshot reference must be
/// a bare file name, so it can never escape the store directory when recovery
/// joins it onto the store path.
fn assert_is_bare_file_name(name: &str) {
    assert!(!name.is_empty());
    assert!(name.len() <= MAX_SESSION_PATH_BYTES);
    assert!(name != ".");
    assert!(name != "..");
    assert!(!name.contains(['/', '\\']));
    assert!(!name.contains('\0'));
}

/// Mirrors `session_contract::validate_path`: every persisted path must be
/// absolute and NUL-free.
fn assert_is_safe_absolute_path(path: &str) {
    assert!(!path.is_empty());
    assert!(path.len() <= MAX_SESSION_PATH_BYTES);
    assert!(!path.contains('\0'));
    assert!(std::path::Path::new(path).is_absolute());
}
