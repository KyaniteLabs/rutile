use std::borrow::Cow;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use feathermark_core::{CompositionCancelReason, EditorAdapter, EditorEvent, ScrollClock};
use feathermark_protocol::{PreviewEventV1, ProtocolError};
use iced_widget::text_editor;
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::program::Program;
use iced_winit::program::core;
use iced_winit::program::graphics;
use iced_winit::program::runtime::Task;
use iced_winit::program::runtime::UserInterface;
use iced_winit::program::runtime::user_interface;
use iced_winit::winit;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
use objc2_foundation::NSString;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, ModifiersState};
use winit::window::{Window, WindowId};
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder};

use super::{
    AppKitMainThread, EditorVisualReceipt, IcedEditorAdapter, MacError, MacScrollDispatch,
    PreviewIpcFatal, PreviewIpcIngress, ProductSession, preview_ipc_channel, split_panes,
};
use crate::app::{AppEffect, CloseDecision, CloseOutcome};
use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL, STARTER_DOCUMENT, status_title};
use crate::preview_host::{
    HostError, NavigationKind, PreviewControlSink, PreviewHost, SchemeRequest, SchemeResponse,
    ScrollDelivery,
};

const WINDOW_WIDTH: u32 = 1_000;
const WINDOW_HEIGHT: u32 = 720;
const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);
const SMOKE_LIFECYCLE_CYCLES: u64 = 50;

pub(super) fn run_native(path: Option<PathBuf>, smoke: bool) -> Result<(), MacError> {
    AppKitMainThread::claim()?;
    let session = match path {
        Some(path) => ProductSession::open(&path)?,
        None => ProductSession::new_in_memory(STARTER_DOCUMENT)?,
    };
    let event_loop = EventLoop::new().map_err(|error| MacError::Native(error.to_string()))?;
    let display_handle = event_loop.owned_display_handle();
    let mut runner = ProductRunner::new(session, smoke, display_handle)?;
    event_loop
        .run_app(&mut runner)
        .map_err(|error| MacError::Native(error.to_string()))?;
    runner.finish()
}

struct ProductRunner {
    webview: Option<WebView>,
    web_context: Option<WebContext>,
    window: Option<Arc<Window>>,
    session: ProductSession,
    preview_host: Arc<Mutex<PreviewHost>>,
    ipc_receiver: Receiver<String>,
    ipc_ingress: PreviewIpcIngress,
    display_handle: OwnedDisplayHandle,
    source_pane: IcedSourcePane,
    initial_url: String,
    modifiers: ModifiersState,
    smoke: bool,
    smoke_stage: u8,
    smoke_path: Option<PathBuf>,
    deadline: Instant,
    failure: Option<String>,
    webview_first: bool,
    painted_revision: Option<u64>,
    smoke_resize_requested: bool,
    resize_proven: bool,
    preview_scroll_events: u64,
    editor_events: Arc<Mutex<VecDeque<EditorEvent>>>,
    pending_close: bool,
    started: Instant,
    preview_frame: u64,
    visibility_cycles: u64,
    compositor_resume_cycles: u64,
}

impl ProductRunner {
    fn new(
        mut session: ProductSession,
        smoke: bool,
        display_handle: OwnedDisplayHandle,
    ) -> Result<Self, MacError> {
        let snapshot = session.snapshot();
        let preview_host = session.preview_host();
        let initial_url = session.render_now()?.url;
        // A bounded burst budget holds bridge-ready, paint, and scroll frames
        // from one navigation without dropping typed evidence under an
        // optimized event loop. Frames remain individually capped and parsed.
        let (ipc_ingress, ipc_receiver) = preview_ipc_channel(8);
        let editor_events = Arc::new(Mutex::new(VecDeque::new()));
        let mut source_pane = IcedSourcePane::new(&snapshot)?;
        source_pane.set_event_queue(Arc::clone(&editor_events));
        Ok(Self {
            webview: None,
            web_context: Some(WebContext::new(None)),
            window: None,
            session,
            preview_host,
            ipc_receiver,
            ipc_ingress,
            display_handle,
            source_pane,
            initial_url,
            modifiers: ModifiersState::empty(),
            smoke,
            smoke_stage: 0,
            smoke_path: smoke.then(|| {
                std::env::temp_dir().join(format!(
                    "feathermark-native-smoke-{}.md",
                    std::process::id()
                ))
            }),
            deadline: Instant::now() + SMOKE_TIMEOUT,
            failure: None,
            webview_first: false,
            painted_revision: None,
            smoke_resize_requested: false,
            resize_proven: false,
            preview_scroll_events: 0,
            editor_events,
            pending_close: false,
            started: Instant::now(),
            preview_frame: 0,
            visibility_cycles: 0,
            compositor_resume_cycles: 0,
        })
    }

    fn process_editor_events(&mut self, event_loop: &ActiveEventLoop) -> Result<bool, MacError> {
        let events = {
            let mut queue = self
                .editor_events
                .lock()
                .map_err(|_| MacError::Native("editor event queue lock poisoned".into()))?;
            queue.drain(..).collect::<Vec<_>>()
        };
        let mut edited = false;
        for event in events {
            match event {
                EditorEvent::ViewportChanged {
                    top_visible_byte,
                    user: true,
                    ..
                } => {
                    if let MacScrollDispatch::Preview {
                        revision,
                        source_start,
                        interaction_id,
                    } = self
                        .session
                        .source_scroll(top_visible_byte, self.scroll_clock())?
                    {
                        self.deliver_preview_scroll(revision, source_start, interaction_id)?;
                    }
                }
                event => {
                    if let Some((adapter_commit_id, change)) =
                        self.session.apply_editor_event(event)?
                    {
                        self.source_pane
                            .editor_mut()
                            .acknowledge_local_commit(adapter_commit_id, &change)
                            .map_err(|error| MacError::Core(error.to_string()))?;
                        edited = true;
                    }
                }
            }
        }
        if edited {
            self.render_and_navigate()?;
        }
        let _ = event_loop;
        Ok(edited)
    }

    fn scroll_clock(&self) -> ScrollClock {
        ScrollClock {
            monotonic_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            preview_frame: self.preview_frame,
        }
    }

    fn deliver_preview_scroll(
        &mut self,
        revision: u64,
        source_start: usize,
        interaction_id: u64,
    ) -> Result<(), MacError> {
        let webview = self
            .webview
            .as_ref()
            .ok_or_else(|| MacError::Native("preview WebView is not ready".into()))?;
        self.preview_host
            .lock()
            .map_err(|_| MacError::PreviewLock)?
            .deliver_scroll_to(
                &mut WebViewScrollSink(webview),
                revision,
                source_start,
                interaction_id,
            )?;
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.failure = Some(message.into());
        event_loop.exit();
    }

    fn surface_error(&self, message: impl AsRef<str>) {
        if let Some(window) = &self.window {
            window.set_title(&status_title(message.as_ref()));
            window.request_redraw();
        }
    }

    fn finish(&mut self) -> Result<(), MacError> {
        let editor_receipt = self.source_pane.visual_receipt();
        drop(self.webview.take());
        drop(self.web_context.take());
        self.source_pane.teardown();
        self.webview_first = self.webview.is_none() && self.window.is_some();
        drop(self.window.take());
        if let Some(path) = self.smoke_path.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(failure) = self.failure.take() {
            return Err(MacError::Native(failure));
        }
        if self.smoke {
            let editor_receipt = editor_receipt?;
            if self.smoke_stage != 2
                || !self.webview_first
                || !self.resize_proven
                || self.source_pane.focus_transfers == 0
                || self.preview_scroll_events == 0
                || self.visibility_cycles < SMOKE_LIFECYCLE_CYCLES
                || self.compositor_resume_cycles < SMOKE_LIFECYCLE_CYCLES
            {
                return Err(MacError::Native(format!(
                    "native smoke proof incomplete: stage={} webview_first={} resize_50_50={} focus_transfers={} preview_scroll_events={} visibility_cycles={} compositor_resume_cycles={} target={}",
                    self.smoke_stage,
                    self.webview_first,
                    self.resize_proven,
                    self.source_pane.focus_transfers,
                    self.preview_scroll_events,
                    self.visibility_cycles,
                    self.compositor_resume_cycles,
                    SMOKE_LIFECYCLE_CYCLES,
                )));
            }
            println!(
                "feathermark-native-smoke-ok editor_frames={} editor_ink_pixels={} ime_commits={} resize_50_50=true focus_transfers={} preview_scroll_events={} hide_show_cycles={} suspend_resume_cycles={} lifecycle_cycles={}",
                editor_receipt.presented_frames,
                editor_receipt.ink_pixels,
                editor_receipt.ime_commits,
                self.source_pane.focus_transfers,
                self.preview_scroll_events,
                self.visibility_cycles,
                self.compositor_resume_cycles,
                SMOKE_LIFECYCLE_CYCLES,
            );
        }
        Ok(())
    }

    fn render_and_navigate(&mut self) -> Result<(), MacError> {
        let receipt = self.session.render_now()?;
        if let Some(webview) = &self.webview {
            webview
                .load_url(&receipt.url)
                .map_err(|error| MacError::Native(error.to_string()))?;
        }
        Ok(())
    }

    fn process_preview_event(&mut self, event_loop: &ActiveEventLoop, event: PreviewEventV1) {
        if matches!(&event, PreviewEventV1::Scroll { .. }) {
            self.preview_scroll_events = self.preview_scroll_events.saturating_add(1);
        }
        let painted = matches!(&event, PreviewEventV1::Painted { .. });
        if let PreviewEventV1::Painted { frame_seq, .. } = &event {
            self.preview_frame = *frame_seq;
        }
        let revision = match &event {
            PreviewEventV1::BridgeReady { revision }
            | PreviewEventV1::Painted { revision, .. }
            | PreviewEventV1::Scroll { revision, .. }
            | PreviewEventV1::LinkActivated { revision, .. } => *revision,
        };
        if matches!(&event, PreviewEventV1::BridgeReady { .. }) {
            let delivery = self
                .source_pane
                .editor_mut()
                .top_visible_byte(revision)
                .map_err(|error| HostError::Platform(error.to_string()))
                .and_then(|source_start| {
                    self.deliver_preview_scroll(revision, source_start, revision)
                        .map_err(|error| HostError::Platform(error.to_string()))
                });
            if let Err(error) = delivery {
                self.fail(event_loop, error.to_string());
                return;
            }
        }
        if let PreviewEventV1::Scroll {
            source_start,
            interaction_id,
            user,
            ..
        } = &event
        {
            match self.session.preview_scroll(
                *source_start,
                *interaction_id,
                *user,
                self.scroll_clock(),
            ) {
                Ok(MacScrollDispatch::Source {
                    revision,
                    source_start,
                    interaction_id,
                }) => {
                    if let Err(error) = self.source_pane.editor_mut().scroll_to_byte(
                        revision,
                        source_start,
                        interaction_id,
                    ) {
                        self.fail(event_loop, error.to_string());
                        return;
                    }
                    self.source_pane.request_redraw();
                }
                Ok(MacScrollDispatch::Suppressed) => {}
                Ok(MacScrollDispatch::Preview { .. }) => {
                    self.fail(event_loop, "preview scroll mapped to the wrong pane");
                    return;
                }
                Err(error) => {
                    self.fail(event_loop, error.to_string());
                    return;
                }
            }
        }
        for effect in self.session.accept_preview_event(event) {
            if let AppEffect::ScrollSource { .. } = effect {
                self.source_pane.request_redraw();
            }
        }

        if painted {
            self.painted_revision = Some(revision);
        }
        self.advance_smoke(event_loop);
    }

    fn advance_smoke(&mut self, event_loop: &ActiveEventLoop) {
        if !self.smoke {
            return;
        }
        if self.smoke_stage == 0
            && self.painted_revision == Some(0)
            && self.source_pane.presented_frames > 0
        {
            while self.visibility_cycles < SMOKE_LIFECYCLE_CYCLES {
                if let Some(webview) = &self.webview {
                    if let Err(error) = webview
                        .set_visible(false)
                        .and_then(|_| webview.set_visible(true))
                    {
                        self.fail(event_loop, error.to_string());
                        return;
                    }
                }
                if let Some(window) = &self.window {
                    window.set_visible(false);
                    window.set_visible(true);
                }
                self.visibility_cycles = self.visibility_cycles.saturating_add(1);
            }
            while self.compositor_resume_cycles < SMOKE_LIFECYCLE_CYCLES {
                let Some(window) = self.window.clone() else {
                    self.fail(event_loop, "native smoke window is missing");
                    return;
                };
                self.source_pane.teardown();
                if let Err(error) = self
                    .source_pane
                    .attach_window(window, self.display_handle.clone())
                {
                    self.fail(event_loop, error.to_string());
                    return;
                }
                self.compositor_resume_cycles = self.compositor_resume_cycles.saturating_add(1);
            }
            self.source_pane.request_redraw();
            if !self.smoke_resize_requested {
                self.smoke_resize_requested = true;
                if let Some(window) = &self.window {
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(1_200, 760));
                }
                return;
            }
            if !self.resize_proven {
                return;
            }
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .perform(text_editor::Action::SelectAll)
                .and_then(|_| {
                    self.source_pane
                        .editor_mut()
                        .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                            Arc::new("# Native smoke 🪶\n\n".to_owned()),
                        )))
                })
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            if let Err(error) = self.process_editor_events(event_loop) {
                self.fail(event_loop, error.to_string());
                return;
            }
            match self.source_pane.commit_ime_for_smoke("Edited.\n") {
                Ok(true) if self.source_pane.text() == "# Native smoke 🪶\n\nEdited.\n" => {}
                Ok(changed) => {
                    self.fail(
                        event_loop,
                        format!(
                            "Iced IME commit receipt mismatch: changed={changed} source={:?}",
                            self.source_pane.text()
                        ),
                    );
                    return;
                }
                Err(error) => {
                    self.fail(event_loop, error.to_string());
                    return;
                }
            }
            if let Err(error) = self.process_editor_events(event_loop) {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(undo) = self.session.undo() else {
                self.fail(event_loop, "native smoke undo had no effect");
                return;
            };
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .apply_external_change(&undo)
                .map_err(|error| MacError::Core(error.to_string()))
                .and_then(|_| self.render_and_navigate())
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(redo) = self.session.redo() else {
                self.fail(event_loop, "native smoke redo had no effect");
                return;
            };
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .apply_external_change(&redo)
                .map_err(|error| MacError::Core(error.to_string()))
                .and_then(|_| self.render_and_navigate())
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            self.smoke_stage = 1;
        } else if self.smoke_stage == 1
            && self.painted_revision == Some(self.session.snapshot().revision)
            && self.source_pane.presented_text == "# Native smoke 🪶\n\nEdited.\n"
        {
            if let Err(error) = self.source_pane.visual_receipt() {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(path) = self.smoke_path.clone() else {
                self.fail(event_loop, "native smoke path is missing");
                return;
            };
            if let Err(error) = self.session.save_as(&path) {
                self.fail(event_loop, error.to_string());
                return;
            }
            match ProductSession::open(&path) {
                Ok(reopened) if reopened.source() == "# Native smoke 🪶\n\nEdited.\n" => {
                    self.session = reopened;
                    self.smoke_stage = 2;
                    event_loop.exit();
                }
                Ok(_) => self.fail(event_loop, "reopened source did not match saved UTF-8"),
                Err(error) => self.fail(event_loop, error.to_string()),
            }
        }
    }
}

impl ApplicationHandler for ProductRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(PRODUCT_NAME)
            .with_inner_size(winit::dpi::LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        if let Err(error) = self
            .source_pane
            .attach_window(Arc::clone(&window), self.display_handle.clone())
        {
            self.fail(event_loop, error.to_string());
            return;
        }
        let physical = window.inner_size();
        let logical = physical.to_logical::<u32>(window.scale_factor());
        let panes = split_panes(logical.width, logical.height);

        let protocol_host = Arc::clone(&self.preview_host);
        let navigation_host = Arc::clone(&self.preview_host);
        let download_host = Arc::clone(&self.preview_host);
        let new_window_host = Arc::clone(&self.preview_host);
        let ipc_ingress = self.ipc_ingress.clone();
        let Some(web_context) = self.web_context.as_mut() else {
            self.fail(event_loop, "preview WebContext is missing");
            return;
        };
        let builder = WebViewBuilder::new_with_web_context(web_context)
            .with_bounds(preview_bounds(panes))
            .with_custom_protocol("feathermark".to_owned(), move |_id, request| {
                let scheme_request =
                    SchemeRequest::new(request.method().as_str(), request.uri().to_string());
                let response = protocol_host
                    .lock()
                    .map(|host| host.serve(&scheme_request))
                    .unwrap_or_else(|_| SchemeResponse {
                        status: 500,
                        headers: vec![],
                        body: Arc::from([]),
                    });
                to_wry_response(response)
            })
            .with_navigation_handler(move |url| {
                navigation_host
                    .lock()
                    .map(|mut host| host.allow_navigation(&url, NavigationKind::AppInitiated))
                    .unwrap_or(false)
            })
            .with_download_started_handler(move |url, _| {
                download_host
                    .lock()
                    .map(|host| host.allow_download(&url))
                    .unwrap_or(false)
            })
            .with_new_window_req_handler(move |url, _| {
                if new_window_host
                    .lock()
                    .map(|host| host.allow_new_window(&url))
                    .unwrap_or(false)
                {
                    NewWindowResponse::Allow
                } else {
                    NewWindowResponse::Deny
                }
            })
            .with_ipc_handler(move |request| {
                ipc_ingress.try_push(request.body().clone());
            })
            .with_url(&self.initial_url);

        match builder.build_as_child(window.as_ref()) {
            Ok(webview) => {
                self.webview = Some(webview);
                self.window = Some(window);
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
            }
            Err(error) => self.fail(event_loop, error.to_string()),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if matches!(&event, WindowEvent::CloseRequested) {
            if self.session.app_state().dirty() {
                self.pending_close = true;
                if let Some(window) = &self.window {
                    window.set_title(&status_title(
                        "Unsaved changes: ⌘S Save · ⌘D Don’t Save · Esc Cancel",
                    ));
                    window.request_redraw();
                }
            } else {
                event_loop.exit();
            }
            return;
        }

        if let WindowEvent::Resized(size) = &event
            && size.width > 0
            && size.height > 0
            && let Some(window) = &self.window
        {
            let scale_factor = window.scale_factor();
            let logical = size.to_logical::<u32>(scale_factor);
            let panes = split_panes(logical.width, logical.height);
            self.source_pane.resize(
                panes.source_width,
                size.width,
                size.height,
                scale_factor as f32,
            );
            if let Some(webview) = &self.webview
                && let Err(error) = webview.set_bounds(preview_bounds(panes))
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            if self.smoke
                && self.smoke_resize_requested
                && logical.width == 1_200
                && panes.source_width == 600
                && panes.preview_x == 600
                && panes.preview_width == 600
                && (self.source_pane.state.pane_width - 600.0).abs() < f32::EPSILON
            {
                self.resize_proven = true;
            }
        }

        if matches!(&event, WindowEvent::Focused(true))
            && let Err(error) = self.source_pane.focus_editor()
        {
            self.fail(event_loop, error.to_string());
            return;
        }

        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.modifiers = modifiers.state();
        }

        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
        {
            let command = self.modifiers.super_key();
            if self.pending_close {
                if matches!(
                    key_event.logical_key,
                    Key::Named(winit::keyboard::NamedKey::Escape)
                ) {
                    let _ = self.session.decide_close(CloseDecision::Cancel);
                    self.pending_close = false;
                    if let Some(window) = &self.window {
                        window.set_title(PRODUCT_NAME);
                    }
                    return;
                }
                if command
                    && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("d"))
                {
                    match self.session.decide_close(CloseDecision::Discard) {
                        Ok(CloseOutcome::Close) => event_loop.exit(),
                        Ok(CloseOutcome::KeepOpen) => self.pending_close = false,
                        Err(error) => self.surface_error(error.to_string()),
                    }
                    return;
                }
                if command
                    && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("s"))
                {
                    let untitled_path = if self.session.path().is_none() {
                        match choose_save_path() {
                            Ok(Some(path)) => Some(path),
                            Ok(None) => {
                                let _ = self.session.decide_close(CloseDecision::Cancel);
                                self.pending_close = false;
                                if let Some(window) = &self.window {
                                    window.set_title(PRODUCT_NAME);
                                }
                                return;
                            }
                            Err(error) => {
                                self.surface_error(error.to_string());
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    match self
                        .session
                        .decide_close(CloseDecision::Save { untitled_path })
                    {
                        Ok(CloseOutcome::Close) => event_loop.exit(),
                        Ok(CloseOutcome::KeepOpen) => self.pending_close = false,
                        Err(error) => self.surface_error(format!("Save failed: {error}")),
                    }
                    return;
                }
            }
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("s"))
            {
                if self.session.app_state().dirty() && self.session.path().is_some() {
                    if let Err(error) = self.session.save() {
                        self.fail(event_loop, error.to_string());
                    }
                }
                return;
            }
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("r"))
            {
                if self.session.path().is_some() {
                    match self.session.reload() {
                        Ok(()) => match self.session.render_now() {
                            Ok(receipt) => {
                                if let Err(error) =
                                    self.source_pane.replace(&self.session.snapshot())
                                {
                                    self.fail(event_loop, error.to_string());
                                    return;
                                }
                                if let Some(webview) = &self.webview
                                    && let Err(error) = webview.load_url(&receipt.url)
                                {
                                    self.fail(event_loop, error.to_string());
                                }
                            }
                            Err(error) => self.fail(event_loop, error.to_string()),
                        },
                        Err(error) => self.fail(event_loop, error.to_string()),
                    }
                }
                return;
            }
            if command {
                if matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("z"))
                {
                    let redo = self.modifiers.shift_key();
                    let change = if redo {
                        self.session.redo()
                    } else {
                        self.session.undo()
                    };
                    if let Some(change) = change {
                        if let Err(error) = self
                            .source_pane
                            .editor_mut()
                            .apply_external_change(&change)
                            .map_err(|error| MacError::Core(error.to_string()))
                            .and_then(|_| self.render_and_navigate())
                        {
                            self.fail(event_loop, error.to_string());
                        }
                    }
                }
                return;
            }
        }

        if matches!(&event, WindowEvent::RedrawRequested) {
            if let Err(error) = self.source_pane.draw_and_present() {
                self.fail(event_loop, error.to_string());
                return;
            }
            self.advance_smoke(event_loop);
            return;
        }

        let Some(window) = &self.window else {
            return;
        };
        match self.source_pane.handle_winit_event(
            event,
            window.scale_factor() as f32,
            self.modifiers,
        ) {
            Ok(true) => {
                if let Err(error) = self.process_editor_events(event_loop) {
                    self.fail(event_loop, error.to_string());
                }
            }
            Ok(false) => {}
            Err(error) => self.fail(event_loop, error.to_string()),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(fatal) = self.ipc_ingress.take_fatal() {
            let message = match fatal {
                PreviewIpcFatal::RequiredFrameLost => {
                    "required preview IPC frame was lost to backpressure"
                }
                PreviewIpcFatal::Disconnected => "preview IPC ingress disconnected",
            };
            self.fail(event_loop, message);
            return;
        }
        loop {
            match self.ipc_receiver.try_recv() {
                Ok(frame) => {
                    let event = self
                        .preview_host
                        .lock()
                        .map_err(|_| HostError::Platform("preview host lock poisoned".into()))
                        .and_then(|host| host.handle_ipc(frame.as_bytes()));
                    match event {
                        Ok(event) => self.process_preview_event(event_loop, event),
                        Err(
                            HostError::NoLoadedDocument
                            | HostError::Protocol(ProtocolError::StaleRevision),
                        ) => continue,
                        Err(error) => {
                            self.fail(event_loop, error.to_string());
                            return;
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fail(event_loop, "preview IPC channel disconnected");
                    return;
                }
            }
        }
        if self.smoke && Instant::now() >= self.deadline {
            self.fail(
                event_loop,
                format!(
                    "native product smoke timed out: stage={} painted={:?} frames={} resize_requested={} resize_proven={} visibility={} resume={}",
                    self.smoke_stage,
                    self.painted_revision,
                    self.source_pane.presented_frames,
                    self.smoke_resize_requested,
                    self.resize_proven,
                    self.visibility_cycles,
                    self.compositor_resume_cycles,
                ),
            );
        }
    }
}

struct WebViewScrollSink<'a>(&'a WebView);

impl PreviewControlSink for WebViewScrollSink<'_> {
    fn deliver_scroll_to(&mut self, delivery: ScrollDelivery) -> Result<(), HostError> {
        let frame = std::str::from_utf8(delivery.as_bytes())
            .map_err(|error| HostError::Platform(error.to_string()))?;
        let argument =
            serde_json::to_string(frame).map_err(|error| HostError::Platform(error.to_string()))?;
        self.0
            .evaluate_script(&format!("window.__feathermarkReceiveScrollTo({argument})"))
            .map_err(|error| HostError::Platform(error.to_string()))
    }
}

fn preview_bounds(panes: super::PaneBounds) -> Rect {
    Rect {
        position: wry::dpi::LogicalPosition::new(panes.preview_x, 0).into(),
        size: wry::dpi::LogicalSize::new(panes.preview_width, panes.height).into(),
    }
}

fn to_wry_response(response: SchemeResponse) -> Response<Cow<'static, [u8]>> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Cow::Owned(response.body.to_vec()))
        .expect("typed preview response headers are valid")
}

#[derive(Default)]
struct SourceProgram;

struct SourceState {
    editor: IcedEditorAdapter,
    error: Option<String>,
    window: Option<Arc<Window>>,
    pane_width: f32,
}

#[derive(Clone, Debug)]
enum SourceMessage {
    Edit(text_editor::Action),
}

impl Program for SourceProgram {
    type State = SourceState;
    type Message = SourceMessage;
    type Theme = core::theme::Theme;
    type Renderer = iced_renderer::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        SOURCE_EDITOR_LABEL
    }

    fn settings(&self) -> core::Settings {
        core::Settings::default()
    }

    fn window(&self) -> Option<core::window::Settings> {
        Some(core::window::Settings::default())
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            SourceState {
                editor: IcedEditorAdapter::new(),
                error: None,
                window: None,
                pane_width: WINDOW_WIDTH as f32 / 2.0,
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            SourceMessage::Edit(action) => {
                if let Err(error) = state.editor.perform(action) {
                    state.error = Some(error.to_string());
                }
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: core::window::Id,
    ) -> core::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        iced_widget::container(
            text_editor(state.editor.content())
                .id(core::widget::Id::new("feathermark-source-editor"))
                .placeholder("Write Markdown…")
                .on_action(SourceMessage::Edit),
        )
        .width(core::Length::Fixed(state.pane_width))
        .height(core::Length::Fill)
        .padding(16)
        .into()
    }
}

type IcedCompositor = iced_renderer::Compositor;
type IcedSurface = <IcedCompositor as graphics::Compositor>::Surface;

struct IcedSourcePane {
    program: SourceProgram,
    state: SourceState,
    cache: user_interface::Cache,
    renderer: Option<iced_renderer::Renderer>,
    compositor: Option<IcedCompositor>,
    surface: Option<IcedSurface>,
    viewport: Option<graphics::Viewport>,
    clipboard: Clipboard,
    cursor: core::mouse::Cursor,
    presented_frames: u64,
    ink_pixels: usize,
    ime_commits: u64,
    presented_text: String,
    focus_transfers: u64,
}

impl IcedSourcePane {
    fn new(snapshot: &feathermark_core::DocumentSnapshot) -> Result<Self, MacError> {
        let program = SourceProgram;
        let (mut state, _) = program.boot();
        state
            .editor
            .install_open_snapshot(snapshot)
            .map_err(|error| MacError::Core(error.to_string()))?;
        Ok(Self {
            program,
            state,
            cache: user_interface::Cache::new(),
            renderer: None,
            compositor: None,
            surface: None,
            viewport: None,
            clipboard: Clipboard::unconnected(),
            cursor: core::mouse::Cursor::Unavailable,
            presented_frames: 0,
            ink_pixels: 0,
            ime_commits: 0,
            presented_text: String::new(),
            focus_transfers: 0,
        })
    }

    fn set_event_queue(&mut self, events: Arc<Mutex<VecDeque<EditorEvent>>>) {
        self.state.editor.set_event_sink(Box::new(move |event| {
            if let Ok(mut queue) = events.lock() {
                queue.push_back(event);
            }
        }));
    }

    fn editor_mut(&mut self) -> &mut IcedEditorAdapter {
        &mut self.state.editor
    }

    fn attach_window(
        &mut self,
        window: Arc<Window>,
        display_handle: OwnedDisplayHandle,
    ) -> Result<(), MacError> {
        use graphics::Compositor as _;

        let physical = window.inner_size();
        let scale_factor = window.scale_factor() as f32;
        let logical = physical.to_logical::<f32>(f64::from(scale_factor));
        self.state.pane_width = logical.width / 2.0;
        let mut compositor = block_on_ready(IcedCompositor::new(
            graphics::Settings::default(),
            display_handle,
            Arc::clone(&window),
            graphics::Shell::headless(),
        ))?
        .map_err(|error| MacError::Native(error.to_string()))?;
        let renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), physical.width, physical.height);

        self.viewport = Some(graphics::Viewport::with_physical_size(
            core::Size::new(physical.width, physical.height),
            scale_factor,
        ));
        self.clipboard = Clipboard::connect(Arc::clone(&window));
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.compositor = Some(compositor);
        self.state.window = Some(Arc::clone(&window));
        window.request_redraw();
        Ok(())
    }

    fn replace(&mut self, snapshot: &feathermark_core::DocumentSnapshot) -> Result<(), MacError> {
        self.state
            .editor
            .install_open_snapshot(snapshot)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.state
            .editor
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.request_redraw();
        Ok(())
    }

    fn resize(
        &mut self,
        source_width: u32,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) {
        use graphics::Compositor as _;

        self.state.pane_width = source_width as f32;
        self.viewport = Some(graphics::Viewport::with_physical_size(
            core::Size::new(physical_width, physical_height),
            scale_factor,
        ));
        if physical_width > 0
            && physical_height > 0
            && let (Some(compositor), Some(surface)) = (&mut self.compositor, &mut self.surface)
        {
            compositor.configure_surface(surface, physical_width, physical_height);
        }
        self.request_redraw();
    }

    fn focus(&self) {
        if let Some(window) = &self.state.window {
            window.focus_window();
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.state.window {
            window.request_redraw();
        }
    }

    fn text(&self) -> String {
        self.state.editor.mirror().to_owned()
    }

    fn focus_editor(&mut self) -> Result<(), MacError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let mut operation = core::widget::operation::focusable::focus::<()>(core::widget::Id::new(
            "feathermark-source-editor",
        ));
        interface.operate(renderer, &mut operation);
        self.cache = interface.into_cache();
        self.focus();
        self.focus_transfers = self.focus_transfers.saturating_add(1);
        self.request_redraw();
        Ok(())
    }

    fn handle_winit_event(
        &mut self,
        event: WindowEvent,
        scale_factor: f32,
        modifiers: ModifiersState,
    ) -> Result<bool, MacError> {
        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                let logical = position.to_logical::<f32>(f64::from(scale_factor));
                self.cursor =
                    core::mouse::Cursor::Available(core::Point::new(logical.x, logical.y));
            }
            WindowEvent::CursorLeft { .. } => self.cursor = core::mouse::Cursor::Unavailable,
            _ => {}
        }
        let Some(event) = conversion::window_event(event, scale_factor, modifiers) else {
            return Ok(false);
        };
        self.handle_iced_events(&[event])
    }

    fn commit_ime_for_smoke(&mut self, text: &str) -> Result<bool, MacError> {
        self.focus_editor()?;
        let changed = self.handle_iced_events(&[
            core::Event::InputMethod(core::input_method::Event::Opened),
            core::Event::InputMethod(core::input_method::Event::Commit(text.to_owned())),
        ])?;
        Ok(changed)
    }

    fn handle_iced_events(&mut self, events: &[core::Event]) -> Result<bool, MacError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let mut messages = Vec::new();
        let (ui_state, _) = interface.update(
            events,
            self.cursor,
            renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.cache = interface.into_cache();

        let mut changed = false;
        let input_commit = events.iter().find_map(|event| match event {
            core::Event::InputMethod(core::input_method::Event::Commit(text)) => Some(text.clone()),
            _ => None,
        });
        for event in events {
            match event {
                core::Event::InputMethod(core::input_method::Event::Opened) => {
                    if self.state.editor.start_composition().is_err() {
                        self.state
                            .editor
                            .cancel_composition(CompositionCancelReason::StaleRevision);
                        self.state
                            .editor
                            .start_composition()
                            .map_err(|error| MacError::Core(error.to_string()))?;
                    }
                }
                core::Event::InputMethod(core::input_method::Event::Preedit(text, _)) => {
                    if self.state.editor.update_composition(text).is_err() {
                        self.state
                            .editor
                            .start_composition()
                            .and_then(|_| self.state.editor.update_composition(text))
                            .map_err(|error| MacError::Core(error.to_string()))?;
                    }
                }
                core::Event::InputMethod(core::input_method::Event::Closed) => self
                    .state
                    .editor
                    .cancel_composition(CompositionCancelReason::FocusLost),
                _ => {}
            }
        }
        let mut skipped_ime_paste = false;
        for message in messages {
            match message {
                SourceMessage::Edit(action) => {
                    if input_commit.is_some() && action.is_edit() && !skipped_ime_paste {
                        skipped_ime_paste = true;
                        continue;
                    }
                    changed |= self
                        .state
                        .editor
                        .perform(action)
                        .map_err(|error| MacError::Core(error.to_string()))?;
                }
            }
        }
        if let Some(text) = input_commit {
            changed |= self
                .state
                .editor
                .commit_composition(&text)
                .map_err(|error| MacError::Core(error.to_string()))?;
            self.ime_commits = self.ime_commits.saturating_add(1);
        }
        if let user_interface::State::Updated { input_method, .. } = ui_state
            && let Some(window) = &self.state.window
        {
            match input_method {
                core::InputMethod::Disabled => window.set_ime_allowed(false),
                core::InputMethod::Enabled { cursor, .. } => {
                    window.set_ime_allowed(true);
                    window.set_ime_cursor_area(
                        winit::dpi::LogicalPosition::new(cursor.x, cursor.y),
                        winit::dpi::LogicalSize::new(cursor.width, cursor.height),
                    );
                }
            }
        }
        self.request_redraw();
        Ok(changed)
    }

    fn draw_and_present(&mut self) -> Result<(), MacError> {
        use graphics::Compositor as _;

        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let compositor = self
            .compositor
            .as_mut()
            .ok_or_else(|| MacError::Native("editor compositor is not initialized".into()))?;
        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| MacError::Native("editor surface is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let theme = <core::theme::Theme as core::theme::Base>::default(core::theme::Mode::Light);
        let background = core::Color::from_rgb8(245, 245, 240);
        interface.draw(
            renderer,
            &theme,
            &core::renderer::Style {
                text_color: core::Color::from_rgb8(28, 28, 26),
            },
            self.cursor,
        );
        self.cache = interface.into_cache();

        let pixels = compositor.screenshot(renderer, viewport, background);
        let background_rgba = [245, 245, 240, 255];
        self.ink_pixels = pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != background_rgba)
            .count();
        let window = self
            .state
            .window
            .as_ref()
            .ok_or_else(|| MacError::Native("editor window is not initialized".into()))?;
        compositor
            .present(renderer, surface, viewport, background, || {
                window.pre_present_notify();
            })
            .map_err(|error| MacError::Native(error.to_string()))?;
        self.presented_frames = self.presented_frames.saturating_add(1);
        self.presented_text = self.state.editor.mirror().to_owned();
        Ok(())
    }

    fn visual_receipt(&self) -> Result<EditorVisualReceipt, MacError> {
        EditorVisualReceipt::new(self.presented_frames, self.ink_pixels, self.ime_commits)
    }

    fn teardown(&mut self) {
        drop(self.surface.take());
        drop(self.compositor.take());
        drop(self.renderer.take());
        self.clipboard = Clipboard::unconnected();
        drop(self.state.window.take());
    }
}

fn block_on_ready<F: Future>(future: F) -> Result<F::Output, MacError> {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(output) => Ok(output),
        std::task::Poll::Pending => Err(MacError::Native(
            "the tiny-skia compositor unexpectedly required an asynchronous executor".into(),
        )),
    }
}

fn choose_save_path() -> Result<Option<PathBuf>, MacError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| MacError::Native("save panel must run on the AppKit main thread".into()))?;
    let panel = NSSavePanel::savePanel(mtm);
    panel.setNameFieldStringValue(&NSString::from_str("Untitled.md"));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    Ok(panel
        .URL()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string())))
}
