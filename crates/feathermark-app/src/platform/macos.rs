use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use feathermark_core::{
    AdapterCommitId, ChangeSet, DiskVersion, Document, EditorEvent, FileError, FileService,
    LocalFileService, MAX_DOCUMENT_BYTES, ScrollAnchorView, ScrollClock, ScrollGeometry, ScrollMap,
    ScrollOutcome, ScrollPosition, ScrollSynchronizer, ScrollTarget, apply_editor_commit,
};
use feathermark_protocol::{PreviewHostCommand, RenderUrl};
use feathermark_types::{InteractionId, Revision};
use thiserror::Error;

use super::PlatformAdapter;
use crate::app::{AppMessage, AppState, CloseDecision, CloseOutcome};
use crate::preview_host::{HostError, PreviewHost};
use crate::render_scheduler::{Completion, RenderRequest, RenderScheduler};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppKitMainThread;

impl AppKitMainThread {
    pub fn claim() -> Result<Self, MacError> {
        unsafe extern "C" {
            fn pthread_main_np() -> std::ffi::c_int;
        }

        // SAFETY: pthread_main_np has no arguments, returns an integer, and has
        // no side effects. It is the supported process-main-thread predicate.
        if unsafe { pthread_main_np() } == 1 {
            Ok(Self)
        } else {
            Err(MacError::WrongThread)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneBounds {
    pub source_width: u32,
    pub preview_x: u32,
    pub preview_width: u32,
    pub height: u32,
}

pub const fn split_panes(width: u32, height: u32) -> PaneBounds {
    let source_width = width / 2;
    PaneBounds {
        source_width,
        preview_x: source_width,
        preview_width: width - source_width,
        height,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewIpcStats {
    pub accepted: u64,
    pub dropped_scroll: u64,
    pub required_lost: u64,
    pub disconnected: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewIpcOutcome {
    Accepted,
    DroppedCoalescibleScroll,
    RequiredFrameLost,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewIpcFatal {
    RequiredFrameLost,
    Disconnected,
}

#[derive(Default)]
struct PreviewIpcHealth {
    stats: PreviewIpcStats,
    fatal: Option<PreviewIpcFatal>,
}

#[derive(Clone)]
pub struct PreviewIpcIngress {
    sender: SyncSender<String>,
    health: Arc<Mutex<PreviewIpcHealth>>,
}

impl PreviewIpcIngress {
    pub fn try_push(&self, frame: String) -> PreviewIpcOutcome {
        let coalescible_scroll = serde_json::from_str::<serde_json::Value>(&frame)
            .ok()
            .and_then(|value| {
                value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("scroll");
        let outcome = match self.sender.try_send(frame) {
            Ok(()) => PreviewIpcOutcome::Accepted,
            Err(TrySendError::Full(_)) if coalescible_scroll => {
                PreviewIpcOutcome::DroppedCoalescibleScroll
            }
            Err(TrySendError::Full(_)) => PreviewIpcOutcome::RequiredFrameLost,
            Err(TrySendError::Disconnected(_)) => PreviewIpcOutcome::Disconnected,
        };
        if let Ok(mut health) = self.health.lock() {
            match outcome {
                PreviewIpcOutcome::Accepted => {
                    health.stats.accepted = health.stats.accepted.saturating_add(1)
                }
                PreviewIpcOutcome::DroppedCoalescibleScroll => {
                    health.stats.dropped_scroll = health.stats.dropped_scroll.saturating_add(1)
                }
                PreviewIpcOutcome::RequiredFrameLost => {
                    health.stats.required_lost = health.stats.required_lost.saturating_add(1);
                    health.fatal = Some(PreviewIpcFatal::RequiredFrameLost);
                }
                PreviewIpcOutcome::Disconnected => {
                    health.stats.disconnected = health.stats.disconnected.saturating_add(1);
                    health.fatal = Some(PreviewIpcFatal::Disconnected);
                }
            }
        }
        outcome
    }

    pub fn stats(&self) -> PreviewIpcStats {
        self.health
            .lock()
            .map(|health| health.stats)
            .unwrap_or(PreviewIpcStats {
                required_lost: 1,
                ..PreviewIpcStats::default()
            })
    }

    pub fn take_fatal(&self) -> Option<PreviewIpcFatal> {
        self.health.lock().ok()?.fatal.take()
    }
}

pub fn preview_ipc_channel(capacity: usize) -> (PreviewIpcIngress, Receiver<String>) {
    let (sender, receiver) = sync_channel(capacity);
    (
        PreviewIpcIngress {
            sender,
            health: Arc::new(Mutex::new(PreviewIpcHealth::default())),
        },
        receiver,
    )
}

pub struct MacShell<Window, WebView, Context = ()> {
    webview: Option<WebView>,
    context: Option<Context>,
    window: Option<Window>,
}

impl<Window, WebView> MacShell<Window, WebView, ()> {
    pub fn new(window: Window) -> Self {
        Self {
            webview: None,
            context: None,
            window: Some(window),
        }
    }
}

impl<Window, WebView, Context> MacShell<Window, WebView, Context> {
    pub fn new_with_context(window: Window, context: Context) -> Self {
        Self {
            webview: None,
            context: Some(context),
            window: Some(window),
        }
    }

    pub fn attach_webview(&mut self, webview: WebView) -> Result<(), MacError> {
        if self.webview.is_some() {
            return Err(MacError::WebViewAlreadyAttached);
        }
        self.webview = Some(webview);
        Ok(())
    }
}

impl<Window, WebView, Context> Drop for MacShell<Window, WebView, Context> {
    fn drop(&mut self) {
        drop(self.webview.take());
        drop(self.context.take());
        drop(self.window.take());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderReceipt {
    pub revision: u64,
    pub page_bytes: usize,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorVisualReceipt {
    pub presented_frames: u64,
    pub ink_pixels: usize,
    pub ime_commits: u64,
}

#[derive(Clone, Copy, Debug)]
struct MacScrollAnchor {
    revision: Revision,
    start: usize,
    end: usize,
    ordinal: u32,
}

impl ScrollAnchorView for MacScrollAnchor {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn preview_top(&self) -> f64 {
        self.start as f64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacScrollDispatch {
    Preview {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
    Source {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
    Suppressed,
}

pub struct MacScrollController {
    map: ScrollMap,
    synchronizer: ScrollSynchronizer,
}

impl MacScrollController {
    pub fn new(
        revision: Revision,
        document_len: usize,
        anchors: impl IntoIterator<Item = (usize, usize, u32)>,
        first_interaction_id: InteractionId,
    ) -> Result<Self, String> {
        let anchors = anchors
            .into_iter()
            .map(|(start, end, ordinal)| MacScrollAnchor {
                revision,
                start,
                end,
                ordinal,
            })
            .collect::<Vec<_>>();
        let map = ScrollMap::new(
            ScrollGeometry {
                revision,
                document_len,
                source_max_top: document_len,
                preview_max_y: document_len as f64,
            },
            anchors,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            map,
            synchronizer: ScrollSynchronizer::new(revision, first_interaction_id),
        })
    }

    pub fn next_interaction_id(&self) -> InteractionId {
        self.synchronizer.next_interaction_id()
    }

    pub fn source_user(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, String> {
        let outcome = self
            .synchronizer
            .handle_user(
                &self.map,
                feathermark_core::Pane::Source,
                ScrollPosition::SourceByte(top_visible_byte),
                clock,
            )
            .map_err(|error| error.to_string())?;
        Ok(match outcome {
            ScrollOutcome::Command(command) => MacScrollDispatch::Preview {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::PreviewY(y) => y as usize,
                    ScrollTarget::SourceByte(byte) => byte,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => MacScrollDispatch::Suppressed,
        })
    }

    pub fn preview(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, String> {
        let revision = self.map.revision();
        let outcome = if user {
            self.synchronizer
                .handle_user(
                    &self.map,
                    feathermark_core::Pane::Preview,
                    ScrollPosition::PreviewY(source_start as f64),
                    clock,
                )
                .map_err(|error| error.to_string())?
        } else {
            self.synchronizer
                .handle_programmatic(
                    revision,
                    feathermark_core::Pane::Preview,
                    interaction_id,
                    clock,
                )
                .map_err(|error| error.to_string())?
        };
        Ok(match outcome {
            ScrollOutcome::Command(command) => MacScrollDispatch::Source {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::SourceByte(byte) => byte,
                    ScrollTarget::PreviewY(y) => y as usize,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => MacScrollDispatch::Suppressed,
        })
    }
}

impl EditorVisualReceipt {
    pub fn new(
        presented_frames: u64,
        ink_pixels: usize,
        ime_commits: u64,
    ) -> Result<Self, MacError> {
        if presented_frames == 0 {
            return Err(MacError::EditorNeverPresented);
        }
        if ink_pixels == 0 {
            return Err(MacError::EditorHasNoVisibleInk);
        }
        if ime_commits == 0 {
            return Err(MacError::EditorInputPathUntested);
        }
        Ok(Self {
            presented_frames,
            ink_pixels,
            ime_commits,
        })
    }
}

pub struct ProductSession {
    app: AppState,
    document: Document,
    scheduler: RenderScheduler,
    preview_host: Arc<Mutex<PreviewHost>>,
    scroll: Option<MacScrollController>,
    next_scroll_interaction_id: InteractionId,
}

impl ProductSession {
    pub fn new_in_memory(source: &str) -> Result<Self, MacError> {
        let document = Document::new(source).map_err(|error| MacError::Core(error.to_string()))?;
        Self::from_document(document, None)
    }

    pub fn open(path: &Path) -> Result<Self, MacError> {
        let loaded = LocalFileService::new().load(path, MAX_DOCUMENT_BYTES)?;
        Self::from_document(loaded.document, Some((path.to_path_buf(), loaded.disk)))
    }

    fn from_document(
        document: Document,
        file: Option<(PathBuf, DiskVersion)>,
    ) -> Result<Self, MacError> {
        let mut app = AppState::new();
        match file {
            Some((path, disk)) => {
                app.reduce(AppMessage::DocumentOpened {
                    revision: document.revision(),
                    path,
                    disk,
                });
            }
            None => {
                app.reduce(AppMessage::NewDocument);
            }
        }
        let mut session = Self {
            app,
            document,
            scheduler: RenderScheduler::new(),
            preview_host: Arc::new(Mutex::new(PreviewHost::new())),
            scroll: None,
            next_scroll_interaction_id: 1,
        };
        session.queue_current();
        Ok(session)
    }

    pub fn source(&self) -> String {
        self.document.snapshot().to_string()
    }

    pub fn snapshot(&self) -> feathermark_core::DocumentSnapshot {
        self.document.snapshot()
    }

    pub fn app_state(&self) -> &AppState {
        &self.app
    }

    pub fn decide_close(&mut self, decision: CloseDecision) -> Result<CloseOutcome, MacError> {
        if !self.app.dirty() {
            return Ok(CloseOutcome::Close);
        }
        match decision {
            CloseDecision::Cancel => Ok(CloseOutcome::KeepOpen),
            CloseDecision::Discard => Ok(CloseOutcome::Close),
            CloseDecision::Save { untitled_path } => {
                if self.app.path().is_some() {
                    self.save()?;
                } else {
                    let path = untitled_path.ok_or(MacError::MissingSavePath)?;
                    self.save_as(&path)?;
                }
                Ok(CloseOutcome::Close)
            }
        }
    }

    pub fn preview_host(&self) -> Arc<Mutex<PreviewHost>> {
        Arc::clone(&self.preview_host)
    }

    pub(crate) fn accept_preview_event(
        &mut self,
        event: feathermark_protocol::PreviewEventV1,
    ) -> Vec<crate::app::AppEffect> {
        self.app.reduce(AppMessage::PreviewEvent(event))
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.app.path()
    }

    pub fn apply_editor_event(
        &mut self,
        event: EditorEvent,
    ) -> Result<Option<(AdapterCommitId, ChangeSet)>, MacError> {
        let EditorEvent::CommitRequested {
            adapter_commit_id,
            commit,
        } = event
        else {
            return Ok(None);
        };
        let change = apply_editor_commit(&mut self.document, adapter_commit_id, commit)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.app.reduce(AppMessage::DocumentEdited {
            revision: self.document.revision(),
        });
        self.queue_current();
        Ok(Some((adapter_commit_id, change)))
    }

    pub(crate) fn undo(&mut self) -> Option<ChangeSet> {
        let change = self.document.undo()?;
        self.app.reduce(AppMessage::DocumentEdited {
            revision: self.document.revision(),
        });
        self.queue_current();
        Some(change)
    }

    pub(crate) fn redo(&mut self) -> Option<ChangeSet> {
        let change = self.document.redo()?;
        self.app.reduce(AppMessage::DocumentEdited {
            revision: self.document.revision(),
        });
        self.queue_current();
        Some(change)
    }

    pub fn render_now(&mut self) -> Result<RenderReceipt, MacError> {
        let permit = self
            .scheduler
            .start_ready(u64::MAX)
            .ok_or(MacError::RenderNotReady)?;
        let completion = self
            .scheduler
            .finish(permit.execute(), self.document.revision());
        let page = match completion {
            Completion::Accepted(page) => page,
            Completion::PreviewTooLarge { revision } => {
                self.app.reduce(AppMessage::RenderFailed {
                    revision,
                    error: feathermark_core::RenderError::PreviewTooLarge,
                });
                return Err(MacError::PreviewTooLarge);
            }
            Completion::Failed { error, .. } => return Err(MacError::Core(error.to_string())),
            Completion::DiscardedStale { .. } | Completion::UnknownJob { .. } => {
                return Err(MacError::StaleRender);
            }
        };

        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| MacError::Entropy(error.to_string()))?;
        let render_url = RenderUrl::new(page.revision, nonce);
        let page_bytes = page.page.len();
        let scroll = MacScrollController::new(
            page.revision,
            self.document.len_bytes(),
            page.blocks
                .iter()
                .map(|block| (block.start, block.end, block.ordinal)),
            self.next_scroll_interaction_id,
        )
        .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        self.scroll = Some(scroll);
        let command = self
            .preview_host
            .lock()
            .map_err(|_| MacError::PreviewLock)?
            .stage_document(render_url, Arc::from(page.page.into_bytes()))?;
        let PreviewHostCommand::Navigate { revision, url, .. } = command else {
            return Err(MacError::InvalidHostCommand);
        };
        self.app.reduce(AppMessage::RenderAccepted {
            revision,
            page_bytes,
        });
        Ok(RenderReceipt {
            revision,
            page_bytes,
            url: format!("feathermark://preview{}", url.document_path()),
        })
    }

    pub(crate) fn source_scroll(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, MacError> {
        let Some(scroll) = self.scroll.as_mut() else {
            return Ok(MacScrollDispatch::Suppressed);
        };
        let dispatch = scroll
            .source_user(top_visible_byte, clock)
            .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        Ok(dispatch)
    }

    pub(crate) fn preview_scroll(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, MacError> {
        let Some(scroll) = self.scroll.as_mut() else {
            return Ok(MacScrollDispatch::Suppressed);
        };
        let dispatch = scroll
            .preview(source_start, interaction_id, user, clock)
            .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        Ok(dispatch)
    }

    pub fn save(&mut self) -> Result<(), MacError> {
        let path = self
            .app
            .path()
            .map(Path::to_path_buf)
            .ok_or(MacError::MissingSavePath)?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), MacError> {
        let disk = LocalFileService::new().save_atomic(path, &self.document.snapshot())?;
        self.app.reduce(AppMessage::SaveCompleted {
            revision: self.document.revision(),
            path: path.to_path_buf(),
            disk,
        });
        Ok(())
    }

    pub fn reload(&mut self) -> Result<(), MacError> {
        let path = self
            .app
            .path()
            .map(Path::to_path_buf)
            .ok_or(MacError::MissingSavePath)?;
        let loaded = LocalFileService::new().load(&path, MAX_DOCUMENT_BYTES)?;
        self.document = loaded.document;
        self.scheduler = RenderScheduler::new();
        self.app.reduce(AppMessage::DocumentOpened {
            revision: self.document.revision(),
            path,
            disk: loaded.disk,
        });
        self.queue_current();
        Ok(())
    }

    fn queue_current(&mut self) {
        self.scheduler.submit(
            RenderRequest::new(
                self.document.revision(),
                Arc::from(self.document.snapshot().to_string()),
            ),
            0,
        );
    }
}

#[derive(Debug, Error)]
pub enum MacError {
    #[error("the macOS shell must start on the AppKit main thread")]
    WrongThread,
    #[error("a WKWebView is already attached")]
    WebViewAlreadyAttached,
    #[error("render was not ready")]
    RenderNotReady,
    #[error("render completion was stale")]
    StaleRender,
    #[error("generated preview is too large")]
    PreviewTooLarge,
    #[error("save requires a path")]
    MissingSavePath,
    #[error("preview host lock is poisoned")]
    PreviewLock,
    #[error("preview host returned a non-navigation command")]
    InvalidHostCommand,
    #[error("the native editor never presented a frame")]
    EditorNeverPresented,
    #[error("the native editor frame contained only background pixels")]
    EditorHasNoVisibleInk,
    #[error("the native editor IME path was not exercised")]
    EditorInputPathUntested,
    #[error("entropy source failed: {0}")]
    Entropy(String),
    #[error("core operation failed: {0}")]
    Core(String),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Host(#[from] HostError),
    #[error("native shell failed: {0}")]
    Native(String),
}

pub struct MacosAdapter;

impl PlatformAdapter for MacosAdapter {
    fn run() -> Result<(), String> {
        run_native(None, false).map_err(|error| error.to_string())
    }
}

pub fn run_cli(path: Option<PathBuf>, smoke: bool) -> Result<(), MacError> {
    run_native(path, smoke)
}

mod editor;
mod native;

pub use editor::{IcedAdapterStats, IcedEditorAdapter};
use native::run_native;
