//! QA round 1 hostile protocol harness (qa/ultraqa-round1).
//!
//! Drives the public decode/encode APIs with malformed NDJSON frames, oversize
//! frames, embedded framing bytes, version/revision mismatches, out-of-range
//! offsets, and unsafe link targets. Invariant: every decoder returns a bounded
//! `ProtocolError` (or a valid value) and never panics.

use rutile_protocol::{
    GuiCommandV1, MAX_GUI_COMMAND_BYTES, MAX_METRIC_RECORD_BYTES, MAX_PREVIEW_EVENT_BYTES,
    ProtocolError, decode_gui_command, decode_gui_event, decode_metric_record,
    decode_preview_event, encode_gui_command,
};

const LOADED: u64 = 5;

// ---------- framing abuse ----------

#[test]
fn preview_event_framing_abuse() {
    let cases: &[&[u8]] = &[
        b"",                                    // empty
        b"\n",                                  // just a newline -> empty record
        b"{}",                                  // no trailing newline
        b"{}\n\n",                              // trailing double newline
        b"{}\r\n",                              // embedded CR
        b"{\"type\":\"painted\"}\n{\"x\":1}\n", // two records
        b"not json\n",                          // invalid json
        b"\x00\x01\x02\n",                      // control bytes
        b"{\"type\":\"painted\"}",              // missing newline
        b"[]\n",                                // wrong shape
        b"null\n",
        b"\"string\"\n",
    ];
    for (i, bytes) in cases.iter().enumerate() {
        let result = decode_preview_event(bytes, LOADED);
        assert!(result.is_err(), "case {i} unexpectedly decoded: {result:?}");
    }
}

#[test]
fn preview_event_oversize_frame_rejected() {
    let mut frame =
        Vec::from(&b"{\"type\":\"painted\",\"v\":1,\"revision\":5,\"frame_seq\":0,\"pad\":\""[..]);
    frame.extend(std::iter::repeat_n(b'a', MAX_PREVIEW_EVENT_BYTES));
    frame.extend_from_slice(b"\"}\n");
    match decode_preview_event(&frame, LOADED) {
        Err(ProtocolError::TooLarge { maximum }) => assert_eq!(maximum, MAX_PREVIEW_EVENT_BYTES),
        other => panic!("oversize frame not rejected as TooLarge: {other:?}"),
    }
}

#[test]
fn preview_event_version_and_revision_gating() {
    // Wrong schema version.
    let bad_v = b"{\"type\":\"painted\",\"v\":2,\"revision\":5,\"frame_seq\":0}\n";
    assert!(matches!(
        decode_preview_event(bad_v, LOADED),
        Err(ProtocolError::UnsupportedVersion)
    ));
    // Stale revision (replay from an old revision).
    let stale = b"{\"type\":\"painted\",\"v\":1,\"revision\":4,\"frame_seq\":0}\n";
    assert!(matches!(
        decode_preview_event(stale, LOADED),
        Err(ProtocolError::StaleRevision)
    ));
    // Revision far in the future (replay forward).
    let future = b"{\"type\":\"painted\",\"v\":1,\"revision\":999999,\"frame_seq\":0}\n";
    assert!(matches!(
        decode_preview_event(future, LOADED),
        Err(ProtocolError::StaleRevision)
    ));
    // Valid frame at the loaded revision decodes.
    let ok = b"{\"type\":\"painted\",\"v\":1,\"revision\":5,\"frame_seq\":0}\n";
    assert!(decode_preview_event(ok, LOADED).is_ok());
}

#[test]
fn preview_event_scroll_offset_out_of_range() {
    // source_start beyond MAX_DOCUMENT_BYTES must be rejected.
    let huge = format!(
        "{{\"type\":\"scroll\",\"v\":1,\"revision\":5,\"source_start\":{},\"interaction_id\":1,\"user\":true}}\n",
        usize::MAX
    );
    assert!(matches!(
        decode_preview_event(huge.as_bytes(), LOADED),
        Err(ProtocolError::InvalidOffset)
    ));
}

#[test]
fn preview_event_link_activated_unsafe_targets_rejected() {
    for url in [
        "javascript:alert(1)",
        "data:text/html,x",
        "vbscript:x",
        "http://ok/ evil",      // whitespace -> forbidden char
        "http://user:pw@host/", // userinfo
        "ftp://host/",          // forbidden scheme
        "HTTP://Ok/",           // noncanonical wire
    ] {
        let frame = format!(
            "{{\"type\":\"link_activated\",\"v\":1,\"revision\":5,\"normalized_url\":{}}}\n",
            serde_json::to_string(url).unwrap()
        );
        let result = decode_preview_event(frame.as_bytes(), LOADED);
        assert!(
            matches!(result, Err(ProtocolError::InvalidLink(_))),
            "unsafe link {url:?} not rejected: {result:?}"
        );
    }
    // A canonical safe link decodes.
    let good = b"{\"type\":\"link_activated\",\"v\":1,\"revision\":5,\"normalized_url\":\"https://example.com/\"}\n";
    assert!(decode_preview_event(good, LOADED).is_ok());
}

// ---------- gui command / event abuse ----------

#[test]
fn gui_command_roundtrip_and_abuse() {
    // Unknown-field / wrong-version frames must be rejected, not panic.
    let cases: &[&[u8]] = &[
        b"{\"type\":\"close\",\"request_id\":1}\n", // missing version field?
        b"{\"v\":9,\"type\":\"close\",\"request_id\":1}\n", // unsupported version
        b"garbage\n",
        b"{}\n",
        b"{\"type\":\"unknown_variant\",\"v\":1,\"request_id\":1}\n",
    ];
    for (i, bytes) in cases.iter().enumerate() {
        let _ = decode_gui_command(bytes); // must not panic
        let _ = i;
    }
    // Roundtrip a valid command.
    let cmd = GuiCommandV1::Edit {
        request_id: 42,
        start: 0,
        end: 0,
        replacement: "\u{202E}\u{0301}hostile".to_string(),
    };
    let encoded = encode_gui_command(&cmd).expect("encode");
    // Encoded frame must be newline-terminated and single-line.
    assert_eq!(*encoded.last().unwrap(), b'\n');
    assert_eq!(encoded.iter().filter(|&&b| b == b'\n').count(), 1);
    let decoded = decode_gui_command(&encoded).expect("decode");
    assert_eq!(decoded, cmd);
}

#[test]
fn gui_command_oversize_rejected() {
    // A command whose replacement is enormous must reject on encode.
    let cmd = GuiCommandV1::Edit {
        request_id: 1,
        start: 0,
        end: 0,
        replacement: "a".repeat(MAX_GUI_COMMAND_BYTES + 10),
    };
    assert!(matches!(
        encode_gui_command(&cmd),
        Err(ProtocolError::TooLarge { .. })
    ));
}

#[test]
fn gui_event_framing_abuse() {
    let cases: &[&[u8]] = &[
        b"",
        b"\n",
        b"{}\n",
        b"not json\n",
        b"{\"type\":\"control_ready\"}", // no newline
        b"{\"type\":\"control_ready\",\"request_id\":1}\r\n",
    ];
    for bytes in cases {
        let _ = decode_gui_event(bytes); // never panics
    }
}

// ---------- metric record abuse ----------

#[test]
fn metric_record_abuse() {
    let cases: &[&[u8]] = &[
        b"",
        b"{}\n",
        b"{\"schema\":\"wrong\",\"v\":1}\n",
        b"{\"schema\":\"rutile.metric.v1\",\"v\":2}\n",
        b"garbage\n",
    ];
    for bytes in cases {
        let result = decode_metric_record(bytes);
        assert!(result.is_err(), "metric decoded unexpectedly: {result:?}");
    }
    // Oversize frame.
    let mut frame = Vec::from(&b"{\"schema\":\"rutile.metric.v1\",\"pad\":\""[..]);
    frame.extend(std::iter::repeat_n(b'a', MAX_METRIC_RECORD_BYTES));
    frame.extend_from_slice(b"\"}\n");
    assert!(matches!(
        decode_metric_record(&frame),
        Err(ProtocolError::TooLarge { .. })
    ));
}
