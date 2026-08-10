use rutile_protocol::{
    FocusSurface, GUI_EVENT_TIMEOUT, GuiCommandV1, GuiErrorCode, GuiEventV1, MAX_GUI_COMMAND_BYTES,
    MAX_PREVIEW_EVENT_BYTES, MAX_SCROLL_CONTROL_BYTES, MetricRecordV1, PreviewEventV1,
    PreviewHostCommand, RenderUrl, decode_gui_command, decode_gui_event, decode_metric_record,
    decode_preview_event, encode_gui_command, encode_gui_event, encode_scroll_control,
};
use std::time::Duration;

#[test]
fn preview_event_framing_validates_version_revision_size_and_unknown_fields() {
    let good = br#"{"type":"painted","v":1,"revision":7,"frame_seq":3}
"#;
    assert_eq!(
        decode_preview_event(good, 7).unwrap(),
        PreviewEventV1::Painted {
            revision: 7,
            frame_seq: 3
        }
    );
    assert!(decode_preview_event(good, 8).is_err());
    assert!(
        decode_preview_event(
            br#"{"type":"painted","v":2,"revision":7,"frame_seq":3}
"#,
            7
        )
        .is_err()
    );
    assert!(
        decode_preview_event(
            br#"{"type":"painted","v":1,"revision":7,"frame_seq":3,"extra":true}
"#,
            7
        )
        .is_err()
    );
    assert!(
        decode_preview_event(
            br#"{"type":"painted","v":1,"revision":7,"revision":7,"frame_seq":3}
"#,
            7
        )
        .is_err()
    );
    assert!(decode_preview_event(b"not utf8: \xff\n", 7).is_err());
    let oversized = vec![b'x'; MAX_PREVIEW_EVENT_BYTES + 1];
    assert!(decode_preview_event(&oversized, 7).is_err());
}

#[test]
fn link_activation_crosses_the_boundary_only_as_a_safe_canonical_type() {
    let good =
        br#"{"type":"link_activated","v":1,"revision":9,"normalized_url":"https://example.com/"}
"#;
    match decode_preview_event(good, 9).unwrap() {
        PreviewEventV1::LinkActivated { target, .. } => {
            assert_eq!(target.as_canonical_str(), "https://example.com/");
        }
        other => panic!("unexpected {other:?}"),
    }
    let mixed =
        br#"{"type":"link_activated","v":1,"revision":9,"normalized_url":"HTTPS://example.com/"}
"#;
    assert!(decode_preview_event(mixed, 9).is_err());
}

#[test]
fn scroll_control_is_the_only_bounded_json_control_payload() {
    assert_eq!(
        RenderUrl::new(4, [0xab; 16]).document_path(),
        "/v1/document/4/abababababababababababababababab"
    );
    let command = PreviewHostCommand::ScrollTo {
        revision: 4,
        source_start: 42,
        interaction_id: 11,
    };
    let bytes = encode_scroll_control(&command).unwrap();
    assert!(bytes.len() <= MAX_SCROLL_CONTROL_BYTES);
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert!(
        encode_scroll_control(&PreviewHostCommand::Navigate {
            revision: 4,
            url: RenderUrl::new(4, [7; 16]),
            page_bytes: 12,
        })
        .is_err()
    );
}

#[test]
fn gui_events_are_versioned_correlated_and_newline_delimited() {
    let bytes = encode_gui_event(&GuiEventV1::SourcePainted {
        request_id: 44,
        revision: 5,
        frame_seq: 9,
    })
    .unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        "{\"type\":\"source_painted\",\"v\":1,\"request_id\":44,\"revision\":5,\"frame_seq\":9}\n"
    );
    assert!(bytes.len() <= MAX_GUI_COMMAND_BYTES);
}

#[test]
fn gui_control_is_single_line_correlated_bounded_and_timed_out() {
    assert_eq!(MAX_GUI_COMMAND_BYTES, 64 * 1024);
    assert_eq!(GUI_EVENT_TIMEOUT, Duration::from_secs(5));
    let line = br#"{"type":"open_fixture","v":1,"request_id":8,"fixture":"unicode"}
"#;
    assert_eq!(
        decode_gui_command(line).unwrap(),
        GuiCommandV1::OpenFixture {
            request_id: 8,
            fixture: "unicode".into()
        }
    );
    assert!(decode_gui_command(b"{}\n{}\n").is_err());
    assert!(
        decode_gui_command(
            br#"{"type":"open_fixture","v":1,"request_id":8,"fixture":"unicode","unknown":0}
"#
        )
        .is_err()
    );
}

#[test]
fn gui_command_and_event_codecs_roundtrip_the_complete_closed_surface() {
    let commands = vec![
        GuiCommandV1::OpenFixture {
            request_id: 1,
            fixture: "unicode".into(),
        },
        GuiCommandV1::Edit {
            request_id: 2,
            start: 1,
            end: 3,
            replacement: "x".into(),
        },
        GuiCommandV1::BeginComposition {
            request_id: 3,
            composition_id: 4,
            start: 0,
            end: 0,
        },
        GuiCommandV1::UpdateComposition {
            request_id: 5,
            composition_id: 4,
            preedit: "に".into(),
        },
        GuiCommandV1::CommitComposition {
            request_id: 6,
            composition_id: 4,
            replacement: "日本".into(),
        },
        GuiCommandV1::CancelComposition {
            request_id: 7,
            composition_id: 4,
        },
        GuiCommandV1::SetSourceViewport {
            request_id: 8,
            top_visible_byte: 10,
        },
        GuiCommandV1::SetPreviewViewport {
            request_id: 9,
            y: 20,
        },
        GuiCommandV1::FocusEditor { request_id: 10 },
        GuiCommandV1::FocusPreview { request_id: 11 },
        GuiCommandV1::Resize {
            request_id: 12,
            width: 800,
            height: 600,
        },
        GuiCommandV1::HideShow { request_id: 13 },
        GuiCommandV1::Close { request_id: 14 },
    ];
    for command in commands {
        assert_eq!(
            decode_gui_command(&encode_gui_command(&command).unwrap()).unwrap(),
            command
        );
    }

    let events = vec![
        GuiEventV1::ControlReady { request_id: 1 },
        GuiEventV1::EditAccepted {
            request_id: 2,
            revision: 3,
        },
        GuiEventV1::SourcePainted {
            request_id: 4,
            revision: 3,
            frame_seq: 5,
        },
        GuiEventV1::PreviewPainted {
            request_id: 6,
            revision: 3,
            frame_seq: 7,
        },
        GuiEventV1::Interactive {
            request_id: 8,
            revision: 3,
        },
        GuiEventV1::FocusChanged {
            request_id: 9,
            surface: FocusSurface::Preview,
        },
        GuiEventV1::BoundsChanged {
            request_id: 10,
            width: 800,
            height: 600,
        },
        GuiEventV1::Closed { request_id: 11 },
        GuiEventV1::Error {
            request_id: 12,
            code: GuiErrorCode::StaleRevision,
            message: "stale".into(),
        },
    ];
    for event in events {
        assert_eq!(
            decode_gui_event(&encode_gui_event(&event).unwrap()).unwrap(),
            event
        );
    }
}

#[test]
fn gui_codecs_reject_unknown_duplicate_version_and_oversize_records() {
    assert!(
        decode_gui_event(
            br#"{"type":"focus_changed","v":1,"request_id":1,"surface":"other"}
"#
        )
        .is_err()
    );
    assert!(
        decode_gui_event(
            br#"{"type":"error","v":1,"request_id":1,"code":"other","message":"x"}
"#
        )
        .is_err()
    );
    assert!(
        decode_gui_event(
            br#"{"type":"closed","v":1,"v":1,"request_id":1}
"#
        )
        .is_err()
    );
    assert!(
        decode_gui_event(
            br#"{"type":"closed","v":2,"request_id":1}
"#
        )
        .is_err()
    );
    assert!(
        decode_gui_event(
            br#"{"type":"closed","v":1,"request_id":1,"unknown":true}
"#
        )
        .is_err()
    );
    assert!(decode_gui_event(&vec![b'x'; MAX_GUI_COMMAND_BYTES + 1]).is_err());
    assert!(
        encode_gui_command(&GuiCommandV1::Edit {
            request_id: 1,
            start: 0,
            end: 0,
            replacement: "x".repeat(MAX_GUI_COMMAND_BYTES),
        })
        .is_err()
    );
}

#[test]
fn metric_records_are_versioned_strict_and_preserve_ordered_samples() {
    let line = br#"{"schema":"rutile.metric.v1","v":1,"scenario":"paced-latency","git_commit":"0123456789012345678901234567890123456789","dirty":false,"rustc_version":"rustc 1.88.0","toolchain":"1.88.0","target_triple":"aarch64-apple-darwin","release_profile":"release","features":["test-control"],"build_kind":"instrumented","candidate_executable_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","package_sha256":null,"runner_id":"fm-macos-arm64-v1","runner_lock_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","pristine_snapshot_id":"snapshot","cpu_model":"Apple M1","cpu_cores":8,"ram_bytes":17179869184,"os":"macOS","kernel":"Darwin","display_session":"native","display_environment":{},"webview_version":"WKWebView","monitor_scale_milli":1000,"monitor_refresh_millihz":60000,"fixture_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","fixture_bytes":1048576,"captured_at_utc":"2026-07-09T00:00:00Z","monotonic_clock":"mach_continuous_time","warmups":5,"samples":[3,1,2],"skipped":0,"stale":0,"pid_rss_samples":[]}
"#;
    let record = decode_metric_record(line).unwrap();
    assert_eq!(record.samples, vec![3, 1, 2]);
    assert_eq!(record.warmups, 5);
    let bad = line.strip_suffix(b"\n").unwrap();
    let mut bad = bad.to_vec();
    let schema = b"rutile.metric.v1";
    let index = bad
        .windows(schema.len())
        .position(|window| window == schema)
        .unwrap();
    bad.splice(index..index + schema.len(), b"bad".iter().copied());
    bad.push(b'\n');
    assert!(decode_metric_record(&bad).is_err());
    let _type_check: MetricRecordV1 = record;
}
