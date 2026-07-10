use std::sync::{Arc, Mutex};

use macos_iced_wry_spike::{AppKitMainThread, CANDIDATE, MacShell};

#[cfg(feature = "macos-iced")]
use macos_iced_wry_spike::{
    BoundedIpcInbox, IcedWryControl, NativeBounds, PreviewBoundary, Route,
    iced_program_lifecycle_probe,
};

#[cfg(feature = "macos-iced")]
use feathermark_protocol::{PreviewEventV1, RenderUrl};

#[derive(Clone)]
struct DropSpy {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for DropSpy {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(self.name);
    }
}

#[test]
fn candidate_identity_matches_the_closed_matrix() {
    assert_eq!(CANDIDATE.package, "macos-iced-wry-spike");
    assert_eq!(CANDIDATE.bin, "macos-iced-wry-spike");
    assert_eq!(CANDIDATE.shell_feature, "macos-iced");
}

#[cfg(target_os = "macos")]
#[test]
fn appkit_guard_rejects_a_worker() {
    assert!(
        std::thread::spawn(AppKitMainThread::claim)
            .join()
            .unwrap()
            .is_err()
    );
}

#[test]
fn teardown_drops_webview_before_iced_window() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut shell = MacShell::new(DropSpy {
        name: "window",
        log: Arc::clone(&log),
    });
    shell
        .attach_webview(DropSpy {
            name: "webview",
            log: Arc::clone(&log),
        })
        .unwrap();

    drop(shell);

    assert_eq!(*log.lock().unwrap(), ["webview", "window"]);
}

#[cfg(feature = "macos-iced")]
#[test]
fn iced_wry_control_tracks_resize_focus_and_bounded_ipc() {
    let mut control = IcedWryControl::new(NativeBounds::new(640, 480));
    control.resize(NativeBounds::new(900, 700));
    control.set_focused(true);
    control
        .accept_ipc(b"{\"type\":\"bridge_ready\"}\n")
        .unwrap();

    assert_eq!(control.bounds(), NativeBounds::new(900, 700));
    assert!(control.is_focused());
    assert_eq!(control.accepted_ipc_frames(), 1);
    assert!(control.accept_ipc(&[b'x'; 1025]).is_err());
}

#[cfg(feature = "macos-iced")]
#[test]
fn typed_boundary_rejects_adversarial_requests_and_ipc() {
    let render = RenderUrl::new(7, [0x11; 16]);
    let path = render.document_path();
    let boundary = PreviewBoundary::new(render);

    assert!(boundary.navigation_allowed(boundary.pending_url()));
    assert!(!boundary.navigation_allowed(&format!("{}?x=1", boundary.pending_url())));
    assert!(!boundary.navigation_allowed(&format!("{}#frag", boundary.pending_url())));
    assert!(!boundary.navigation_allowed("https://preview.invalid/"));
    assert!(!boundary.download_allowed(boundary.pending_url()));
    assert!(!boundary.new_window_allowed(boundary.pending_url()));

    assert_eq!(
        boundary.route("GET", Some("preview"), &path, None, None),
        Route::Document
    );
    assert_eq!(
        boundary.route("POST", Some("preview"), &path, None, None),
        Route::NotFound
    );
    assert_eq!(
        boundary.route("GET", Some("evil"), &path, None, None),
        Route::NotFound
    );
    assert_eq!(
        boundary.route("GET", Some("preview"), &path, Some("x=1"), None),
        Route::NotFound
    );
    assert_eq!(
        boundary.route("GET", Some("preview"), &path, None, Some("frag")),
        Route::NotFound
    );
    assert_eq!(
        boundary.route(
            "GET",
            Some("preview"),
            "/v1/document/7/22222222222222222222222222222222",
            None,
            None,
        ),
        Route::NotFound
    );

    let event = boundary
        .decode_ipc(b"{\"type\":\"bridge_ready\",\"v\":1,\"revision\":7}\n")
        .unwrap();
    assert!(matches!(event, PreviewEventV1::BridgeReady { revision: 7 }));
    assert!(
        boundary
            .decode_ipc(b"{\"type\":\"bridge_ready\",\"v\":1,\"revision\":6}\n")
            .is_err()
    );
    assert!(boundary.decode_ipc(b"not-json\n").is_err());
    assert!(
        boundary
            .decode_ipc(b"{\"type\":\"bridge_ready\",\"v\":1,\"revision\":7,\"revision\":7}\n")
            .is_err()
    );
    assert!(
        boundary
            .decode_ipc(b"{\"type\":\"bridge_ready\",\"v\":1,\"revision\":7,\"extra\":true}\n")
            .is_err()
    );
    assert!(boundary.decode_ipc(&vec![b'x'; 1025]).is_err());
}

#[cfg(feature = "macos-iced")]
#[test]
fn bounded_ipc_inbox_reports_backpressure() {
    let inbox = BoundedIpcInbox::new();
    inbox.try_send("first".to_owned()).unwrap();
    assert!(inbox.try_send("second".to_owned()).is_err());
    assert_eq!(inbox.try_recv().unwrap().as_deref(), Some("first"));
}

#[cfg(feature = "macos-iced")]
#[test]
fn real_iced_program_boots_updates_and_builds_a_widget() {
    let evidence = iced_program_lifecycle_probe();
    assert!(evidence.booted);
    assert!(evidence.updated);
    assert!(evidence.widget_built);
}
