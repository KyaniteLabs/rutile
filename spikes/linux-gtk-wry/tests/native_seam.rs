use std::sync::{Arc, Mutex};

use linux_gtk_wry_spike::{
    CANDIDATE, DisplayBackend, DisplayEnvironment, LinuxShell, UiThreadToken,
};

#[cfg(feature = "linux-gtk")]
use linux_gtk_wry_spike::{BoundedIpcInbox, GtkWryControl, NativeBounds, PreviewBoundary, Route};

#[cfg(feature = "linux-gtk")]
use rutile_protocol::{PreviewEventV1, RenderUrl};

#[derive(Clone)]
struct DropSpy {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl DropSpy {
    fn new(name: &'static str, log: &Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            name,
            log: Arc::clone(log),
        }
    }
}

impl Drop for DropSpy {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(self.name);
    }
}

#[test]
fn candidate_identity_matches_the_closed_matrix() {
    assert_eq!(CANDIDATE.package, "linux-gtk-wry-spike");
    assert_eq!(CANDIDATE.bin, "linux-gtk-wry-spike");
    assert_eq!(CANDIDATE.shell_feature, "linux-gtk");
}

#[test]
fn accepts_only_provable_x11_or_native_wayland_sessions() {
    let x11 = DisplayEnvironment::new(Some("x11"), Some(":0"), None, Some("x11"));
    assert_eq!(x11.validate(), Ok(DisplayBackend::X11));

    let wayland =
        DisplayEnvironment::new(Some("wayland"), None, Some("wayland-0"), Some("wayland"));
    assert_eq!(wayland.validate(), Ok(DisplayBackend::NativeWayland));

    let xwayland = DisplayEnvironment::new(
        Some("wayland"),
        Some(":0"),
        Some("wayland-0"),
        Some("wayland"),
    );
    assert!(xwayland.validate().is_err());

    let mislabeled = DisplayEnvironment::new(Some("wayland"), None, Some("wayland-0"), Some("x11"));
    assert!(mislabeled.validate().is_err());
}

#[test]
fn ui_thread_token_rejects_calls_from_a_worker() {
    let token = UiThreadToken::claim_current();
    token.assert_current().unwrap();
    assert!(
        std::thread::spawn(move || token.assert_current())
            .join()
            .unwrap()
            .is_err()
    );
}

#[test]
fn teardown_drops_webview_before_gtk_container_window_and_application() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut shell = LinuxShell::new(
        DropSpy::new("application", &log),
        DropSpy::new("window", &log),
        DropSpy::new("container", &log),
    );
    shell.attach_webview(DropSpy::new("webview", &log)).unwrap();

    drop(shell);

    assert_eq!(
        *log.lock().unwrap(),
        ["webview", "container", "window", "application"]
    );
}

#[cfg(feature = "linux-gtk")]
#[test]
fn gtk_wry_control_tracks_resize_focus_and_bounded_ipc() {
    let mut control = GtkWryControl::new(NativeBounds::new(640, 480));
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

#[cfg(feature = "linux-gtk")]
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

#[cfg(feature = "linux-gtk")]
#[test]
fn bounded_ipc_inbox_reports_backpressure() {
    let inbox = BoundedIpcInbox::new();
    inbox.try_send("first".to_owned()).unwrap();
    assert!(inbox.try_send("second".to_owned()).is_err());
    assert_eq!(inbox.try_recv().unwrap().as_deref(), Some("first"));
}
