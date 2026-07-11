use feathermark_core::{
    AUTOSAVE_SCHEMA_V1, AutosaveEntryV1, MAX_AUTOSAVE_ENTRY_BYTES, MAX_DOCUMENT_BYTES,
    MAX_RECENT_FILES, MAX_SESSION_PATH_BYTES, MAX_SESSION_STATE_BYTES, SESSION_SCHEMA_V1,
    SessionError, SessionSelectionV1, SessionStateV1, SessionWindowV1, decode_autosave_entry,
    decode_session_state, encode_autosave_entry, encode_session_state,
};

fn entry() -> AutosaveEntryV1 {
    AutosaveEntryV1 {
        schema: AUTOSAVE_SCHEMA_V1.into(),
        v: 1,
        sequence: 9,
        captured_at_unix_ms: 1_760_000_000_000,
        document_path: Some("/notes/todo.md".into()),
        document_revision: 42,
        snapshot_file: "autosave-9.md".into(),
        snapshot_bytes: 128,
        snapshot_blake3: "a".repeat(64),
    }
}

fn state() -> SessionStateV1 {
    SessionStateV1 {
        schema: SESSION_SCHEMA_V1.into(),
        v: 1,
        saved_at_unix_ms: 1_760_000_000_000,
        last_file: Some("/notes/todo.md".into()),
        selection: Some(SessionSelectionV1 { anchor: 4, head: 9 }),
        top_visible_byte: Some(2),
        window: Some(SessionWindowV1 {
            x: -8,
            y: 24,
            width: 1280,
            height: 900,
        }),
        recent_files: vec!["/notes/todo.md".into(), "/notes/spec.md".into()],
    }
}

#[test]
fn autosave_entry_round_trips() {
    let encoded = encode_autosave_entry(&entry()).unwrap();
    assert!(encoded.ends_with(b"\n"));
    assert!(encoded.len() <= MAX_AUTOSAVE_ENTRY_BYTES);
    let decoded = decode_autosave_entry(&encoded).unwrap();
    assert_eq!(decoded, entry());
}

#[test]
fn session_state_round_trips() {
    let encoded = encode_session_state(&state()).unwrap();
    assert!(encoded.ends_with(b"\n"));
    assert!(encoded.len() <= MAX_SESSION_STATE_BYTES);
    let decoded = decode_session_state(&encoded).unwrap();
    assert_eq!(decoded, state());
}

#[test]
fn minimal_session_state_round_trips() {
    let minimal = SessionStateV1 {
        schema: SESSION_SCHEMA_V1.into(),
        v: 1,
        saved_at_unix_ms: 0,
        last_file: None,
        selection: None,
        top_visible_byte: None,
        window: None,
        recent_files: Vec::new(),
    };
    let encoded = encode_session_state(&minimal).unwrap();
    assert_eq!(decode_session_state(&encoded).unwrap(), minimal);
}

#[test]
fn untitled_autosave_round_trips() {
    let untitled = AutosaveEntryV1 {
        document_path: None,
        ..entry()
    };
    let encoded = encode_autosave_entry(&untitled).unwrap();
    assert_eq!(decode_autosave_entry(&encoded).unwrap(), untitled);
}

#[test]
fn unsupported_versions_are_rejected() {
    let versioned = AutosaveEntryV1 { v: 2, ..entry() };
    assert!(matches!(
        encode_autosave_entry(&versioned),
        Err(SessionError::UnsupportedVersion)
    ));
    let mut wire = encode_autosave_entry(&entry()).unwrap();
    wire = String::from_utf8(wire)
        .unwrap()
        .replace("\"v\":1", "\"v\":2")
        .into_bytes();
    assert!(matches!(
        decode_autosave_entry(&wire),
        Err(SessionError::UnsupportedVersion)
    ));

    let versioned_state = SessionStateV1 { v: 3, ..state() };
    assert!(matches!(
        encode_session_state(&versioned_state),
        Err(SessionError::UnsupportedVersion)
    ));
}

#[test]
fn wrong_schema_tags_are_rejected() {
    let mislabelled = AutosaveEntryV1 {
        schema: SESSION_SCHEMA_V1.into(),
        ..entry()
    };
    assert!(matches!(
        encode_autosave_entry(&mislabelled),
        Err(SessionError::InvalidSchema)
    ));
    let mislabelled_state = SessionStateV1 {
        schema: "feathermark.metric.v1".into(),
        ..state()
    };
    assert!(matches!(
        encode_session_state(&mislabelled_state),
        Err(SessionError::InvalidSchema)
    ));
}

#[test]
fn unknown_fields_are_rejected() {
    let wire = String::from_utf8(encode_session_state(&state()).unwrap())
        .unwrap()
        .replace("\"v\":1", "\"v\":1,\"surprise\":true")
        .into_bytes();
    assert!(matches!(
        decode_session_state(&wire),
        Err(SessionError::InvalidJson(_))
    ));
}

#[test]
fn framing_violations_are_rejected() {
    assert!(matches!(
        decode_session_state(b"{}"),
        Err(SessionError::InvalidFraming)
    ));
    assert!(matches!(
        decode_session_state(b"\n"),
        Err(SessionError::InvalidFraming)
    ));
    let oversized = vec![b' '; MAX_SESSION_STATE_BYTES + 1];
    assert!(matches!(
        decode_session_state(&oversized),
        Err(SessionError::TooLarge { .. })
    ));
}

#[test]
fn autosave_snapshot_references_must_be_bare_file_names() {
    for hostile in ["../escape.md", "a/b.md", "a\\b.md", "", ".."] {
        let bad = AutosaveEntryV1 {
            snapshot_file: hostile.into(),
            ..entry()
        };
        assert!(
            matches!(
                encode_autosave_entry(&bad),
                Err(SessionError::InvalidMetadata(_))
            ),
            "snapshot_file {hostile:?} must be rejected"
        );
    }
}

#[test]
fn autosave_digest_must_be_lower_hex_blake3() {
    for hostile in ["", "ZZ", &"A".repeat(64), &"a".repeat(63)] {
        let bad = AutosaveEntryV1 {
            snapshot_blake3: (*hostile).into(),
            ..entry()
        };
        assert!(
            matches!(
                encode_autosave_entry(&bad),
                Err(SessionError::InvalidMetadata(_))
            ),
            "digest {hostile:?} must be rejected"
        );
    }
}

#[test]
fn autosave_snapshot_bytes_respect_the_document_cap() {
    let bad = AutosaveEntryV1 {
        snapshot_bytes: MAX_DOCUMENT_BYTES as u64 + 1,
        ..entry()
    };
    assert!(matches!(
        encode_autosave_entry(&bad),
        Err(SessionError::InvalidMetadata(_))
    ));
}

#[test]
fn session_offsets_respect_the_document_cap() {
    let bad_selection = SessionStateV1 {
        selection: Some(SessionSelectionV1 {
            anchor: MAX_DOCUMENT_BYTES as u64 + 1,
            head: 0,
        }),
        ..state()
    };
    assert!(matches!(
        encode_session_state(&bad_selection),
        Err(SessionError::InvalidMetadata(_))
    ));
    let bad_viewport = SessionStateV1 {
        top_visible_byte: Some(MAX_DOCUMENT_BYTES as u64 + 1),
        ..state()
    };
    assert!(matches!(
        encode_session_state(&bad_viewport),
        Err(SessionError::InvalidMetadata(_))
    ));
}

#[test]
fn session_recent_files_are_bounded_and_non_empty() {
    let overflowing = SessionStateV1 {
        recent_files: (0..MAX_RECENT_FILES + 1)
            .map(|index| format!("/notes/{index}.md"))
            .collect(),
        ..state()
    };
    assert!(matches!(
        encode_session_state(&overflowing),
        Err(SessionError::InvalidMetadata(_))
    ));
    let empty_path = SessionStateV1 {
        recent_files: vec![String::new()],
        ..state()
    };
    assert!(matches!(
        encode_session_state(&empty_path),
        Err(SessionError::InvalidMetadata(_))
    ));
    let oversized_path = SessionStateV1 {
        recent_files: vec!["p".repeat(MAX_SESSION_PATH_BYTES + 1)],
        ..state()
    };
    assert!(matches!(
        encode_session_state(&oversized_path),
        Err(SessionError::InvalidMetadata(_))
    ));
}

#[test]
fn session_window_dimensions_must_be_positive() {
    let flat = SessionStateV1 {
        window: Some(SessionWindowV1 {
            x: 0,
            y: 0,
            width: 0,
            height: 900,
        }),
        ..state()
    };
    assert!(matches!(
        encode_session_state(&flat),
        Err(SessionError::InvalidMetadata(_))
    ));
}

#[test]
fn decode_applies_the_same_validation_as_encode() {
    // A hand-crafted wire record with a traversal snapshot_file must not
    // decode, even though it is well-formed JSON of the right version.
    let wire = String::from_utf8(encode_autosave_entry(&entry()).unwrap())
        .unwrap()
        .replace("autosave-9.md", "../escape.md")
        .into_bytes();
    assert!(matches!(
        decode_autosave_entry(&wire),
        Err(SessionError::InvalidMetadata(_))
    ));
}

#[test]
fn schema_tags_are_pinned() {
    assert_eq!(AUTOSAVE_SCHEMA_V1, "feathermark.autosave.v1");
    assert_eq!(SESSION_SCHEMA_V1, "feathermark.session.v1");
}
