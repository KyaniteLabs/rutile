use std::sync::Arc;

use rutile_app::preview_host::{
    CONTENT_SECURITY_POLICY, HostError, NavigationKind, PreviewControlSink, PreviewHost,
    SchemeRequest, ScrollDelivery,
};
use rutile_protocol::{PreviewEventV1, RenderUrl};
use rutile_types::{InteractionId, Revision};

const NONCE: [u8; 16] = [0xab; 16];

fn document_url(revision: u64) -> String {
    format!(
        "rutile://preview{}",
        RenderUrl::new(Revision::new(revision), NONCE).document_path()
    )
}

#[test]
fn serves_only_exact_get_host_path_revision_and_nonce() {
    let mut host = PreviewHost::new();
    let render_url = RenderUrl::new(Revision::new(7), NONCE);
    host.stage_document(
        render_url.clone(),
        Arc::from(b"<!doctype html>ok".as_slice()),
    )
    .unwrap();

    let response = host.serve(&SchemeRequest::get(document_url(7)));
    assert_eq!(response.status, 200);
    assert_eq!(response.body.as_ref(), b"<!doctype html>ok");

    for request in [
        SchemeRequest::new("POST", document_url(7)),
        SchemeRequest::get(document_url(8)),
        SchemeRequest::get("rutile://other/v1/document/7/abababababababababababababababab"),
        SchemeRequest::get("rutile://preview/v1/document/7/abababababababababababababababac"),
        SchemeRequest::get(&(document_url(7) + "?query=1")),
        SchemeRequest::get(&(document_url(7) + "#fragment")),
    ] {
        assert_eq!(host.serve(&request).status, 404, "{request:?}");
    }
}

#[test]
fn fixed_assets_and_document_have_exact_security_headers() {
    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(1), NONCE),
        Arc::from(b"page".as_slice()),
    )
    .unwrap();

    let document = host.serve(&SchemeRequest::get(document_url(1)));
    assert_eq!(
        document.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(document.header("cache-control"), Some("no-store"));
    assert_eq!(document.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        document.header("content-security-policy"),
        Some(CONTENT_SECURITY_POLICY)
    );

    let css = host.serve(&SchemeRequest::get(
        "rutile://preview/v1/assets/preview.css",
    ));
    let js = host.serve(&SchemeRequest::get("rutile://preview/v1/assets/bridge.js"));
    assert_eq!(css.header("content-type"), Some("text/css"));
    assert_eq!(js.header("content-type"), Some("text/javascript"));
    assert!(
        std::str::from_utf8(&js.body)
            .unwrap()
            .contains("requestAnimationFrame")
    );
    let bridge = std::str::from_utf8(&js.body).unwrap();
    assert!(bridge.contains("__rutileReceiveScrollTo"));
    assert!(bridge.contains("TextEncoder"));
    assert!(bridge.contains("256"));
    assert!(bridge.contains("command.v!==1"));
    assert!(bridge.contains("command.revision!==revision"));
    assert!(bridge.contains("command.source_start>20971520"));
    assert!(bridge.contains("frame!==canonical"));
    assert!(bridge.contains("scrollIntoView"));
    assert!(bridge.contains("addEventListener('scroll'"));
    assert!(bridge.contains("type:'scroll',v:1,revision"));
    assert!(bridge.contains("interaction_id"));
    assert!(bridge.contains("source_start"));
    assert!(bridge.contains("user:programmatic===null"));
    assert!(bridge.contains("requestAnimationFrame(emitScroll)"));
    assert!(!bridge.contains("eval("));
    assert!(!bridge.contains("new Function"));
    assert!(!js.body.is_empty());
}

#[test]
fn navigation_downloads_and_new_windows_are_deny_by_default() {
    let mut host = PreviewHost::new();
    let url = RenderUrl::new(Revision::new(4), NONCE);
    host.stage_document(url.clone(), Arc::from(b"page".as_slice()))
        .unwrap();

    assert!(!host.allow_navigation(&document_url(4), NavigationKind::User));
    assert!(!host.allow_navigation("https://example.com", NavigationKind::AppInitiated));
    assert!(!host.allow_new_window(&document_url(4)));
    assert!(!host.allow_download(&document_url(4)));
    assert!(host.allow_navigation(&document_url(4), NavigationKind::AppInitiated));
    assert!(!host.allow_navigation(&document_url(4), NavigationKind::AppInitiated));
}

#[test]
fn ipc_is_bounded_reparsed_and_rejects_stale_paint_and_scroll() {
    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(9), NONCE),
        Arc::from(b"page".as_slice()),
    )
    .unwrap();
    assert!(host.allow_navigation(&document_url(9), NavigationKind::AppInitiated));

    let link = b"{\"type\":\"link_activated\",\"v\":1,\"revision\":9,\"normalized_url\":\"https://example.com/path\"}\n";
    assert!(matches!(
        host.handle_ipc(link).unwrap(),
        PreviewEventV1::LinkActivated { target, .. }
            if target.as_canonical_str() == "https://example.com/path"
    ));
    let noncanonical = b"{\"type\":\"link_activated\",\"v\":1,\"revision\":9,\"normalized_url\":\"HTTPS://example.com/path\"}\n";
    assert!(host.handle_ipc(noncanonical).is_err());

    let stale_paint = b"{\"type\":\"painted\",\"v\":1,\"revision\":8,\"frame_seq\":2}\n";
    let stale_scroll = b"{\"type\":\"scroll\",\"v\":1,\"revision\":8,\"source_start\":0,\"interaction_id\":1,\"user\":true}\n";
    assert!(matches!(
        host.handle_ipc(stale_paint),
        Err(HostError::Protocol(_))
    ));
    assert!(matches!(
        host.handle_ipc(stale_scroll),
        Err(HostError::Protocol(_))
    ));
    assert!(host.handle_ipc(&vec![b'x'; 1_025]).is_err());
}

#[test]
fn staging_a_new_revision_closes_the_old_documents_ipc_window() {
    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(9), NONCE),
        Arc::from(b"old".as_slice()),
    )
    .unwrap();
    assert!(host.allow_navigation(&document_url(9), NavigationKind::AppInitiated));
    host.stage_document(
        RenderUrl::new(Revision::new(10), NONCE),
        Arc::from(b"new".as_slice()),
    )
    .unwrap();

    let old_paint = b"{\"type\":\"painted\",\"v\":1,\"revision\":9,\"frame_seq\":2}\n";
    assert!(matches!(
        host.handle_ipc(old_paint),
        Err(HostError::NoLoadedDocument)
    ));
}

#[test]
fn scroll_control_is_the_only_bounded_native_to_document_channel() {
    #[derive(Default)]
    struct Sink {
        deliveries: usize,
        bytes: Vec<u8>,
    }

    impl PreviewControlSink for Sink {
        fn deliver_scroll_to(&mut self, delivery: ScrollDelivery) -> Result<(), HostError> {
            self.deliveries += 1;
            self.bytes = delivery.as_bytes().to_vec();
            Ok(())
        }
    }

    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(2), NONCE),
        Arc::from(b"page".as_slice()),
    )
    .unwrap();
    assert!(host.allow_navigation(&document_url(2), NavigationKind::AppInitiated));

    let mut sink = Sink::default();
    host.deliver_scroll_to(&mut sink, Revision::new(2), 42, InteractionId::new(3))
        .unwrap();
    assert_eq!(sink.deliveries, 1);
    assert!(sink.bytes.len() <= 256);
    assert_eq!(sink.bytes.last(), Some(&b'\n'));
    assert!(
        host.deliver_scroll_to(&mut sink, Revision::new(1), 42, InteractionId::new(3))
            .is_err()
    );
    assert_eq!(sink.deliveries, 1);
}

#[test]
fn malformed_ipc_frames_are_rejected_without_mutating_state() {
    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(5), NONCE),
        Arc::from(b"page".as_slice()),
    )
    .unwrap();
    assert!(host.allow_navigation(&document_url(5), NavigationKind::AppInitiated));

    let cases = [
        b"not json\n".as_slice(),
        b"{\"type\":\"painted\"}\n",
        b"{\"type\":\"painted\",\"v\":2,\"revision\":5,\"frame_seq\":2}\n",
        b"{\"type\":\"painted\",\"v\":1,\"revision\":5}\n",
        b"{\"type\":\"scroll\",\"v\":1,\"revision\":5,\"source_start\":20971521,\"interaction_id\":1,\"user\":true}\n",
        b"{\"type\":\"painted\",\"v\":1,\"revision\":5,\"frame_seq\":2}\n\n",
        b"{\"type\":\"painted\",\"v\":1,\"revision\":5,\"frame_seq\":2}",
    ];

    for frame in cases {
        assert!(
            host.handle_ipc(frame).is_err(),
            "frame should be rejected: {:?}",
            std::str::from_utf8(frame)
        );
    }
}

#[test]
fn forbidden_navigation_urls_are_rejected() {
    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(3), NONCE),
        Arc::from(b"page".as_slice()),
    )
    .unwrap();

    for url in [
        "https://example.com/",
        "http://example.com/",
        "file:///etc/passwd",
        "rutile://preview/v1/document/3/00000000000000000000000000000000",
    ] {
        assert!(
            !host.allow_navigation(url, NavigationKind::AppInitiated),
            "{url}"
        );
    }
}

#[test]
fn required_frame_loss_and_disconnect_are_fatal_to_old_page_authority() {
    #[derive(Default)]
    struct DisconnectedSink;

    impl PreviewControlSink for DisconnectedSink {
        fn deliver_scroll_to(&mut self, _delivery: ScrollDelivery) -> Result<(), HostError> {
            Err(HostError::Platform(
                "preview channel disconnected".to_owned(),
            ))
        }
    }

    let mut host = PreviewHost::new();
    host.stage_document(
        RenderUrl::new(Revision::new(6), NONCE),
        Arc::from(b"old".as_slice()),
    )
    .unwrap();
    assert!(host.allow_navigation(&document_url(6), NavigationKind::AppInitiated));

    // A missing required painted frame (frame_seq < 2) is accepted by the
    // protocol but the reducer will never mark the preview Ready.
    let single_frame = b"{\"type\":\"painted\",\"v\":1,\"revision\":6,\"frame_seq\":1}\n";
    assert!(matches!(
        host.handle_ipc(single_frame).unwrap(),
        PreviewEventV1::Painted {
            revision: _, // can't pattern match on newtype value
            frame_seq: 1
        }
    ));

    // Staging a new revision revokes the old page's IPC authority entirely.
    host.stage_document(
        RenderUrl::new(Revision::new(7), NONCE),
        Arc::from(b"new".as_slice()),
    )
    .unwrap();
    let old_required = b"{\"type\":\"painted\",\"v\":1,\"revision\":6,\"frame_seq\":2}\n";
    assert!(matches!(
        host.handle_ipc(old_required),
        Err(HostError::NoLoadedDocument)
    ));

    // A disconnected scroll-control sink propagates the failure.
    let mut sink = DisconnectedSink;
    assert!(
        host.deliver_scroll_to(&mut sink, Revision::new(7), 0, InteractionId::new(1))
            .is_err()
    );
}
