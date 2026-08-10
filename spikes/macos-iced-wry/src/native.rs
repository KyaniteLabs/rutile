use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rutile_protocol::{PreviewEventV1, RenderUrl};
use iced_winit::winit;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
use wry::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

use crate::iced_program::IcedProgramHost;
use crate::{
    AppKitMainThread, BoundedIpcInbox, IcedWryControl, NativeBounds, PreviewBoundary, Route,
};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";
const DOCUMENT: &[u8] = br#"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'"><script src="rutile://preview/v1/assets/bridge.js"></script></head><body><main>Rutile native seam</main></body></html>"#;
const BRIDGE: &[u8] = br#"window.ipc.postMessage('{"type":"bridge_ready","v":1,"revision":1}\n');"#;
const CSS: &[u8] = b"html { color-scheme: light dark; }";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSmokeReceipt {
    pub ipc_frames: u64,
    pub resized: bool,
    pub focused: bool,
    pub iced_program: bool,
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
    AppKitMainThread::claim()
        .map_err(|error| NativeSmokeError(format!("AppKit main-thread check failed: {error:?}")))?;

    let event_loop = EventLoop::new().map_err(|error| NativeSmokeError(error.to_string()))?;
    let mut runner = SmokeRunner::new();
    event_loop
        .run_app(&mut runner)
        .map_err(|error| NativeSmokeError(error.to_string()))?;
    runner.finish()
}

struct SmokeRunner {
    webview: Option<WebView>,
    window: Option<Arc<Window>>,
    boundary: Arc<PreviewBoundary>,
    ipc_inbox: BoundedIpcInbox,
    ipc_overflow: Arc<AtomicBool>,
    iced_program: IcedProgramHost,
    control: IcedWryControl,
    resized: bool,
    focused: bool,
    deadline: Instant,
    failure: Option<String>,
}

impl SmokeRunner {
    fn new() -> Self {
        Self {
            webview: None,
            window: None,
            boundary: Arc::new(PreviewBoundary::new(RenderUrl::new(1, [0x11; 16]))),
            ipc_inbox: BoundedIpcInbox::new(),
            ipc_overflow: Arc::new(AtomicBool::new(false)),
            iced_program: IcedProgramHost::new(),
            control: IcedWryControl::new(NativeBounds::new(640, 480)),
            resized: false,
            focused: false,
            deadline: Instant::now() + Duration::from_secs(5),
            failure: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.failure = Some(message.into());
        event_loop.exit();
    }

    fn finish(mut self) -> Result<NativeSmokeReceipt, NativeSmokeError> {
        if let Some(failure) = self.failure.take() {
            return Err(NativeSmokeError(failure));
        }

        let ipc_frames = self.control.accepted_ipc_frames();
        let iced_evidence = self.iced_program.evidence();
        drop(self.webview.take());
        let webview_dropped_before_window = self.webview.is_none() && self.window.is_some();
        drop(self.window.take());

        let receipt = NativeSmokeReceipt {
            ipc_frames,
            resized: self.resized,
            focused: self.focused,
            iced_program: iced_evidence.booted
                && iced_evidence.updated
                && iced_evidence.widget_built
                && iced_evidence.owns_native_window,
            webview_dropped_before_window,
        };
        if receipt.ipc_frames == 0
            || !receipt.resized
            || !receipt.focused
            || !receipt.iced_program
            || !receipt.webview_dropped_before_window
        {
            return Err(NativeSmokeError(
                "native seam exited without complete iced/IPC/resize/focus/teardown proof"
                    .to_owned(),
            ));
        }
        Ok(receipt)
    }
}

impl ApplicationHandler for SmokeRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("Rutile iced/Wry seam proof")
            .with_inner_size(winit::dpi::LogicalSize::new(640_u32, 480_u32));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("native window creation failed: {error}"),
                );
                return;
            }
        };

        self.iced_program.attach_window(Arc::clone(&window));

        let protocol_boundary = Arc::clone(&self.boundary);
        let navigation_boundary = Arc::clone(&self.boundary);
        let download_boundary = Arc::clone(&self.boundary);
        let new_window_boundary = Arc::clone(&self.boundary);
        let ipc_sender = self.ipc_inbox.sender();
        let ipc_overflow = Arc::clone(&self.ipc_overflow);
        let builder = WebViewBuilder::new()
            .with_bounds(full_bounds(640, 480))
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
            .with_download_started_handler(move |url, _| download_boundary.download_allowed(&url))
            .with_new_window_req_handler(move |url, _| {
                if new_window_boundary.new_window_allowed(&url) {
                    NewWindowResponse::Allow
                } else {
                    NewWindowResponse::Deny
                }
            })
            .with_ipc_handler(move |request| {
                if ipc_sender.try_send(request.body().clone()).is_err() {
                    ipc_overflow.store(true, Ordering::Release);
                }
            })
            .with_url(self.boundary.pending_url());
        let webview = match builder.build_as_child(window.as_ref()) {
            Ok(webview) => webview,
            Err(error) => {
                self.fail(event_loop, format!("WKWebView creation failed: {error}"));
                return;
            }
        };

        window.focus_window();
        self.webview = Some(webview);
        self.window = Some(window);
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                if let Some(webview) = &self.webview
                    && let Err(error) = webview.set_bounds(full_bounds(size.width, size.height))
                {
                    self.fail(event_loop, format!("WKWebView resize failed: {error}"));
                    return;
                }
                self.control
                    .resize(NativeBounds::new(size.width, size.height));
                self.iced_program.resized();
            }
            WindowEvent::Focused(focused) => {
                self.control.set_focused(focused);
                self.iced_program.focused();
            }
            WindowEvent::CloseRequested => self.fail(event_loop, "window closed before IPC proof"),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.ipc_overflow.load(Ordering::Acquire) {
            self.fail(
                event_loop,
                "WKWebView IPC channel overflowed or disconnected",
            );
            return;
        }

        if let Ok(Some(frame)) = self.ipc_inbox.try_recv() {
            match self.boundary.decode_ipc(frame.as_bytes()) {
                Ok(PreviewEventV1::BridgeReady { revision: 1 }) => {}
                Ok(event) => {
                    self.fail(
                        event_loop,
                        format!("unexpected WKWebView IPC event: {event:?}"),
                    );
                    return;
                }
                Err(error) => {
                    self.fail(
                        event_loop,
                        format!("typed WKWebView IPC rejected: {error:?}"),
                    );
                    return;
                }
            }
            if let Err(error) = self.control.accept_ipc(frame.as_bytes()) {
                self.fail(event_loop, format!("WKWebView IPC rejected: {error:?}"));
                return;
            }
            let Some(webview) = &self.webview else {
                self.fail(event_loop, "IPC arrived without a live WKWebView");
                return;
            };
            if let Err(error) = webview.set_bounds(full_bounds(900, 700)) {
                self.fail(
                    event_loop,
                    format!("WKWebView resize proof failed: {error}"),
                );
                return;
            }
            self.control.resize(NativeBounds::new(900, 700));
            self.resized = true;
            if let Err(error) = webview.focus() {
                self.fail(event_loop, format!("WKWebView focus proof failed: {error}"));
                return;
            }
            self.control.set_focused(true);
            self.iced_program.ipc_accepted();
            self.focused = true;
            event_loop.exit();
        } else if Instant::now() >= self.deadline {
            self.fail(event_loop, "timed out waiting for custom-protocol IPC");
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        }
    }
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
