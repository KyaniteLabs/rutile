use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rutile_protocol::{PreviewEventV1, RenderUrl};
use gtk::prelude::*;
use sourceview4::prelude::*;
use wry::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtUnix};

use crate::{BoundedIpcInbox, GtkWryControl, NativeBounds, PreviewBoundary, Route};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";
const DOCUMENT: &[u8] = br#"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'"><script src="rutile://preview/v1/assets/bridge.js"></script></head><body><main>Rutile native GTK seam</main></body></html>"#;
const BRIDGE: &[u8] = br#"requestAnimationFrame(() => requestAnimationFrame(() => window.ipc.postMessage('{"type":"bridge_ready","v":1,"revision":1}\n')));"#;
const CSS: &[u8] = b"html { color-scheme: light dark; }";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSmokeReceipt {
    pub ipc_frames: u64,
    pub resized: bool,
    pub focused: bool,
    pub webview_dropped_before_window: bool,
}

#[derive(Debug)]
pub struct NativeSmokeError(String);

impl std::fmt::Display for NativeSmokeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NativeSmokeError {}

pub fn run_native_seam_smoke() -> Result<NativeSmokeReceipt, NativeSmokeError> {
    gtk::init().map_err(|error| NativeSmokeError(format!("GTK init failed: {error}")))?;

    let application = gtk::Application::new(
        Some("tech.kyanitelabs.rutile.spike"),
        gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    let webview = Rc::new(RefCell::new(None::<WebView>));
    let window = Rc::new(RefCell::new(None::<gtk::ApplicationWindow>));
    let control = Rc::new(RefCell::new(GtkWryControl::new(NativeBounds::new(
        640, 480,
    ))));
    let resized = Rc::new(Cell::new(false));
    let focused = Rc::new(Cell::new(false));
    let failure = Rc::new(RefCell::new(None::<String>));
    let boundary = Rc::new(PreviewBoundary::new(RenderUrl::new(1, [0x11; 16])));
    let ipc_inbox = Rc::new(BoundedIpcInbox::new());

    {
        let webview = Rc::clone(&webview);
        let window = Rc::clone(&window);
        let control = Rc::clone(&control);
        let resized = Rc::clone(&resized);
        let focused = Rc::clone(&focused);
        let failure = Rc::clone(&failure);
        let boundary = Rc::clone(&boundary);
        let ipc_inbox = Rc::clone(&ipc_inbox);
        application.connect_activate(move |application| {
            let native_window = gtk::ApplicationWindow::new(application);
            native_window.set_title("Rutile GTK/Wry seam proof");
            native_window.set_default_size(900, 700);

            let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
            let source = sourceview4::View::new();
            source.set_show_line_numbers(true);
            source.set_monospace(true);
            let container = gtk::Fixed::new();
            container.set_size_request(450, 700);
            paned.pack1(&source, true, false);
            paned.pack2(&container, true, false);
            native_window.add(&paned);
            native_window.show_all();

            let ipc_application = application.clone();
            let ipc_control = Rc::clone(&control);
            let ipc_failure = Rc::clone(&failure);
            let ipc_boundary = Rc::clone(&boundary);
            let ipc_inbox = Rc::clone(&ipc_inbox);
            let ipc_sender = ipc_inbox.sender();
            let protocol_boundary = Rc::clone(&boundary);
            let navigation_boundary = Rc::clone(&boundary);
            let download_boundary = Rc::clone(&boundary);
            let new_window_boundary = Rc::clone(&boundary);
            let builder = WebViewBuilder::new()
                .with_bounds(full_bounds(450, 700))
                .with_custom_protocol("rutile".to_owned(), move |_id, request| {
                    let route = protocol_boundary.route(
                        request.method().as_str(),
                        request.uri().host(),
                        request.uri().path(),
                        request.uri().query(),
                        None,
                    );
                    serve_protocol(route)
                })
                .with_navigation_handler(move |url| navigation_boundary.navigation_allowed(&url))
                .with_download_started_handler(move |url, _| {
                    download_boundary.download_allowed(&url)
                })
                .with_new_window_req_handler(move |url, _| {
                    if new_window_boundary.new_window_allowed(&url) {
                        NewWindowResponse::Allow
                    } else {
                        NewWindowResponse::Deny
                    }
                })
                .with_ipc_handler(move |request| {
                    if let Err(error) = ipc_sender.try_send(request.body().clone()) {
                        *ipc_failure.borrow_mut() =
                            Some(format!("WebKitGTK IPC backpressure failure: {error:?}"));
                        ipc_application.quit();
                        return;
                    }
                    let frame = match ipc_inbox.try_recv() {
                        Ok(Some(frame)) => frame,
                        Ok(None) => {
                            *ipc_failure.borrow_mut() =
                                Some("WebKitGTK bounded IPC frame disappeared".to_owned());
                            ipc_application.quit();
                            return;
                        }
                        Err(error) => {
                            *ipc_failure.borrow_mut() =
                                Some(format!("WebKitGTK IPC receive failed: {error:?}"));
                            ipc_application.quit();
                            return;
                        }
                    };
                    match ipc_boundary.decode_ipc(frame.as_bytes()) {
                        Ok(PreviewEventV1::BridgeReady { revision: 1 }) => {}
                        Ok(event) => {
                            *ipc_failure.borrow_mut() =
                                Some(format!("unexpected WebKitGTK IPC event: {event:?}"));
                            ipc_application.quit();
                            return;
                        }
                        Err(error) => {
                            *ipc_failure.borrow_mut() =
                                Some(format!("typed WebKitGTK IPC rejected: {error:?}"));
                            ipc_application.quit();
                            return;
                        }
                    }
                    if let Err(error) = ipc_control.borrow_mut().accept_ipc(frame.as_bytes()) {
                        *ipc_failure.borrow_mut() =
                            Some(format!("WebKitGTK IPC rejected: {error:?}"));
                    }
                    ipc_application.quit();
                })
                .with_url(boundary.pending_url());

            let native_webview = match builder.build_gtk(&container) {
                Ok(webview) => webview,
                Err(error) => {
                    *failure.borrow_mut() = Some(format!("WebKitGTK creation failed: {error}"));
                    application.quit();
                    return;
                }
            };
            if let Err(error) = native_webview.set_bounds(full_bounds(450, 700)) {
                *failure.borrow_mut() = Some(format!("WebKitGTK resize failed: {error}"));
                application.quit();
                return;
            }
            control.borrow_mut().resize(NativeBounds::new(450, 700));
            resized.set(true);
            if let Err(error) = native_webview.focus() {
                *failure.borrow_mut() = Some(format!("WebKitGTK focus failed: {error}"));
                application.quit();
                return;
            }
            control.borrow_mut().set_focused(true);
            focused.set(true);

            *webview.borrow_mut() = Some(native_webview);
            *window.borrow_mut() = Some(native_window);

            let timeout_application = application.clone();
            let timeout_control = Rc::clone(&control);
            let timeout_failure = Rc::clone(&failure);
            gtk::glib::timeout_add_local_once(Duration::from_secs(5), move || {
                if timeout_control.borrow().accepted_ipc_frames() == 0 {
                    *timeout_failure.borrow_mut() =
                        Some("timed out waiting for custom-protocol IPC".to_owned());
                    timeout_application.quit();
                }
            });
        });
    }

    application.run_with_args(&["linux-gtk-wry-spike"]);
    if let Some(failure) = failure.borrow_mut().take() {
        return Err(NativeSmokeError(failure));
    }

    let ipc_frames = control.borrow().accepted_ipc_frames();
    drop(webview.borrow_mut().take());
    let webview_dropped_before_window = webview.borrow().is_none() && window.borrow().is_some();
    drop(window.borrow_mut().take());

    if ipc_frames == 0 || !resized.get() || !focused.get() || !webview_dropped_before_window {
        return Err(NativeSmokeError(
            "native seam exited without complete IPC/resize/focus/teardown proof".to_owned(),
        ));
    }

    Ok(NativeSmokeReceipt {
        ipc_frames,
        resized: resized.get(),
        focused: focused.get(),
        webview_dropped_before_window,
    })
}

fn full_bounds(width: u32, height: u32) -> Rect {
    Rect {
        position: wry::dpi::LogicalPosition::new(0, 0).into(),
        size: wry::dpi::LogicalSize::new(width, height).into(),
    }
}

fn serve_protocol(route: Route) -> Response<Cow<'static, [u8]>> {
    match route {
        Route::Document => response(StatusCode::OK, "text/html; charset=utf-8", DOCUMENT),
        Route::Css => response(StatusCode::OK, "text/css; charset=utf-8", CSS),
        Route::Bridge => response(StatusCode::OK, "text/javascript", BRIDGE),
        Route::NotFound => response(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"not found",
        ),
    }
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: &'static [u8],
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header(CONTENT_SECURITY_POLICY, CSP)
        .body(Cow::Borrowed(body))
        .expect("fixed native seam response is valid")
}
